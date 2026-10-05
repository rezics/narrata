use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

use narrata_kernel::codec::DecodeError;
use narrata_storage::{
    Applied, Batch, Capabilities, Conflict, Durability, Expect, KeyAction, KeyEntry, KeyPage,
    KeySpace, KeyValue, Limit, Limits, ObjectDigest, ObjectPage, Revision, StorageBackend,
    StorageError, WriteModel,
};
use thiserror::Error;

use crate::{
    Flush, FlushReply, LoadRequest, Loaded, Persist, Range, StoreState,
    known::{Known, prefix_end, successor},
    protocol::storage_key,
};

/// A [`StorageBackend`] over the part of a host's store it has loaded.
///
/// Reads it cannot answer fail with [`StorageError::NotLoaded`] and are recorded; the host takes
/// them with [`CacheBackend::take_request`], answers through [`CacheBackend::load`] and runs the
/// operation again. A batch is checked against what the cache knows, and fails with `NotLoaded`
/// before changing anything when the cache does not know a key it conditions on or an object it
/// puts or deletes. Accepted batches wait in [`CacheBackend::unconfirmed`] until the host has
/// persisted them and called [`CacheBackend::confirm`]; until then the host publishes nothing
/// that depends on them.
///
/// Handles made with [`CacheBackend::share`] use the same cache, so the host keeps one while an
/// engine owns another. A cache is one writer: other caches on the same store are noticed when
/// a load or a flush finds the store moved, not prevented.
#[derive(Debug, Default)]
pub struct CacheBackend {
    cache: Arc<Mutex<Cache>>,
    limits: Limits,
}

#[derive(Debug, Default)]
struct Cache {
    /// The store the cache mirrors, at the revision the host last confirmed.
    store: Option<StoreState>,
    /// The revision of the newest accepted batch.
    issued: u64,
    /// By storage key.
    keys: Known<KeyValue>,
    /// By digest bytes.
    objects: Known<ObjectEntry>,
    unconfirmed: Vec<Persist>,
    wanted: Wanted,
}

#[derive(Clone, Debug)]
enum ObjectEntry {
    Bytes(Arc<[u8]>),
    /// A digest scan listed the object; its bytes are not loaded.
    Listed,
}

#[derive(Debug, Default)]
struct Wanted {
    store: bool,
    keys: BTreeSet<Vec<u8>>,
    objects: BTreeSet<ObjectDigest>,
    key_ranges: BTreeSet<Range>,
    object_ranges: BTreeSet<Range>,
}

/// A host message the cache cannot apply.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum HostError {
    /// Another writer changed the store under batches the host had not yet confirmed. The cache
    /// dropped them and everything it held; the host reloads what it shows.
    #[error("the store moved to revision {} under unconfirmed batches", .0.revision)]
    Superseded(StoreState),
    #[error("malformed host message: {0}")]
    Malformed(&'static str),
    #[error("undecodable host message: {0}")]
    Decode(#[from] DecodeError),
}

impl CacheBackend {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_limits(limits: Limits) -> Self {
        Self {
            cache: Arc::default(),
            limits,
        }
    }

    /// Returns another handle to the same cache.
    pub fn share(&self) -> Self {
        Self {
            cache: Arc::clone(&self.cache),
            limits: self.limits,
        }
    }

    fn cache(&self) -> MutexGuard<'_, Cache> {
        // Every call checks before it mutates and mutations cannot fail, so a panicking holder
        // cannot have left a half-applied change behind.
        self.cache.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The store as of the last load or confirmation; `None` before the first load.
    pub fn store(&self) -> Option<StoreState> {
        self.cache().store
    }

    /// What calls failed with [`StorageError::NotLoaded`] lacked since the last request.
    pub fn take_request(&self) -> Option<LoadRequest> {
        let wanted = std::mem::take(&mut self.cache().wanted);
        let request = LoadRequest {
            keys: wanted.keys.into_iter().collect(),
            objects: wanted.objects.into_iter().collect(),
            key_ranges: wanted.key_ranges.into_iter().collect(),
            object_ranges: wanted.object_ranges.into_iter().collect(),
        };
        (wanted.store || !request.is_empty()).then_some(request)
    }

    /// Adds what the host loaded. The host must not load while one of its flushes is in
    /// flight: a store found at another revision than the cache last confirmed has been changed
    /// by someone else.
    ///
    /// When it was, a cache without unconfirmed batches drops what it held and starts over from
    /// this answer; one with unconfirmed batches drops them too and reports
    /// [`HostError::Superseded`].
    pub fn load(&self, loaded: &Loaded) -> Result<(), HostError> {
        loaded.check().map_err(HostError::Malformed)?;
        let mut guard = self.cache();
        let cache = &mut *guard;
        match cache.store {
            Some(store) if store == loaded.store => {}
            Some(_) if !cache.unconfirmed.is_empty() => {
                *cache = Cache::default();
                return Err(HostError::Superseded(loaded.store));
            }
            Some(_) | None => {
                *cache = Cache::default();
                cache.adopt(loaded.store);
            }
        }
        for (key, value) in &loaded.keys {
            cache.keys.learn(key, value.clone());
        }
        for (digest, bytes) in &loaded.objects {
            let key = digest.as_bytes();
            match (cache.objects.get(key), bytes) {
                (None | Some(Some(ObjectEntry::Listed)), Some(bytes)) => cache
                    .objects
                    .set(key.to_vec(), Some(ObjectEntry::Bytes(Arc::clone(bytes)))),
                (None, None) => cache.objects.set(key.to_vec(), None),
                (Some(_), _) => {}
            }
        }
        for (range, entries) in &loaded.key_ranges {
            for (key, value) in entries {
                cache.keys.learn(key, Some(value.clone()));
            }
            let last = entries.last().map(|(key, _)| key.as_slice());
            cache
                .keys
                .cover(range.lower.clone(), reached(range, entries.len(), last));
        }
        for (range, digests) in &loaded.object_ranges {
            for digest in digests {
                cache
                    .objects
                    .learn(digest.as_bytes(), Some(ObjectEntry::Listed));
            }
            let last = digests.last().map(|digest| digest.as_bytes().as_slice());
            cache
                .objects
                .cover(range.lower.clone(), reached(range, digests.len(), last));
        }
        Ok(())
    }

    /// The accepted batches the host has not confirmed, oldest first. The host persists them in
    /// one transaction and answers with [`CacheBackend::confirm`]; until then it may send the
    /// same batches again, but never two flushes at once.
    pub fn unconfirmed(&self) -> Option<Flush> {
        let cache = self.cache();
        let store = cache.store?;
        (!cache.unconfirmed.is_empty()).then(|| Flush {
            store: store.id,
            batches: cache.unconfirmed.clone(),
        })
    }

    /// Applies the host's answer to a flush. A conflict drops every unconfirmed batch and
    /// everything the cache held, and reports [`HostError::Superseded`].
    pub fn confirm(&self, reply: &FlushReply) -> Result<(), HostError> {
        let mut cache = self.cache();
        match reply {
            FlushReply::Persisted { revision } => {
                let position = cache
                    .unconfirmed
                    .iter()
                    .position(|batch| batch.revision == *revision)
                    .ok_or(HostError::Malformed(
                        "confirmed revision belongs to no unconfirmed batch",
                    ))?;
                cache.unconfirmed.drain(..=position);
                if let Some(store) = &mut cache.store {
                    store.revision = *revision;
                }
                Ok(())
            }
            FlushReply::Conflict(store) => {
                *cache = Cache::default();
                Err(HostError::Superseded(*store))
            }
        }
    }

    /// Drops everything the cache holds, unconfirmed batches included, as after a conflict.
    pub fn invalidate(&self) {
        *self.cache() = Cache::default();
    }

    /// Drops what the cache holds to bound its memory or to read the store afresh, unless
    /// batches await confirmation; the next calls load again. Returns whether it did.
    pub fn forget(&self) -> bool {
        let mut cache = self.cache();
        if cache.unconfirmed.is_empty() {
            *cache = Cache::default();
            true
        } else {
            false
        }
    }

    fn apply_batch(&self, batch: &Batch) -> Result<Applied, StorageError> {
        batch.validate(&self.capabilities())?;
        let mut guard = self.cache();
        let cache = &mut *guard;
        let Some(store) = cache.store else {
            cache.wanted.store = true;
            return Err(StorageError::NotLoaded);
        };

        let mut missing = false;
        let objects = batch
            .put_objects
            .iter()
            .map(|(digest, _)| digest)
            .chain(&batch.delete_objects);
        for digest in objects {
            if cache.objects.get(digest.as_bytes()).is_none() {
                cache.wanted.objects.insert(*digest);
                missing = true;
            }
        }
        let keys = batch
            .keys
            .iter()
            .map(|op| (op, storage_key(op.space, &op.key)))
            .collect::<Vec<_>>();
        for (op, key) in &keys {
            if op.expect != Expect::Any && cache.keys.get(key).is_none() {
                cache.wanted.keys.insert(key.clone());
                missing = true;
            }
        }
        if missing {
            return Err(StorageError::NotLoaded);
        }

        for (index, (op, key)) in keys.iter().enumerate() {
            let actual = cache.keys.get(key).flatten();
            if !op.expect.holds(actual.map(|value| value.revision)) {
                return Err(Conflict {
                    index,
                    space: op.space,
                    key: op.key.clone(),
                    expected: op.expect,
                    actual: actual.cloned(),
                }
                .into());
            }
        }

        let put_objects = batch
            .put_objects
            .iter()
            .filter(|(digest, _)| matches!(cache.objects.get(digest.as_bytes()), Some(None)))
            .map(|(digest, bytes)| (*digest, Arc::clone(bytes)))
            .collect::<BTreeMap<_, _>>();
        let delete_objects = batch
            .delete_objects
            .iter()
            .filter(|digest| matches!(cache.objects.get(digest.as_bytes()), Some(Some(_))))
            .copied()
            .collect::<BTreeSet<_>>();
        let mut put_keys = BTreeMap::new();
        let mut delete_keys = BTreeSet::new();
        for (op, key) in keys {
            match &op.action {
                KeyAction::Put(value) => {
                    put_keys.insert(key, value.clone());
                }
                // Deleting a key known to be absent changes nothing.
                KeyAction::Delete if !matches!(cache.keys.get(&key), Some(None)) => {
                    delete_keys.insert(key);
                }
                KeyAction::Delete | KeyAction::Check => {}
            }
        }
        if put_objects.is_empty()
            && delete_objects.is_empty()
            && put_keys.is_empty()
            && delete_keys.is_empty()
        {
            return Ok(Applied::default());
        }

        let next = cache.issued.checked_add(1).ok_or(StorageError::Full)?;
        let revision = Revision::new(next).ok_or(StorageError::Full)?;
        for (digest, bytes) in &put_objects {
            cache.objects.set(
                digest.as_bytes().to_vec(),
                Some(ObjectEntry::Bytes(Arc::clone(bytes))),
            );
        }
        for digest in &delete_objects {
            cache.objects.set(digest.as_bytes().to_vec(), None);
        }
        for (key, value) in &put_keys {
            cache.keys.set(
                key.clone(),
                Some(KeyValue {
                    value: value.clone(),
                    revision,
                }),
            );
        }
        for key in &delete_keys {
            cache.keys.set(key.clone(), None);
        }
        let applied = Applied {
            revision: (!put_keys.is_empty()).then_some(revision),
            objects_inserted: put_objects.len() as u64,
            objects_deleted: delete_objects.len() as u64,
        };
        cache.unconfirmed.push(Persist {
            base: cache.issued,
            revision: next,
            put_objects: put_objects.into_iter().collect(),
            delete_objects: delete_objects.into_iter().collect(),
            put_keys: put_keys.into_iter().collect(),
            delete_keys: delete_keys.into_iter().collect(),
        });
        cache.issued = next;
        debug_assert_eq!(
            cache.issued,
            store.revision + cache.unconfirmed.len() as u64
        );
        Ok(applied)
    }
}

impl Cache {
    fn adopt(&mut self, store: StoreState) {
        self.store = Some(store);
        self.issued = store.revision;
        if store.revision == 0 {
            // No batch was ever persisted, so the store is empty.
            self.keys.cover_all();
            self.objects.cover_all();
        }
    }
}

/// How far a range answer with `count` entries, the last of them `last`, covers its range.
fn reached(range: &Range, count: usize, last: Option<&[u8]>) -> Option<Vec<u8>> {
    match last {
        Some(last) if count as u64 >= range.limit => Some(successor(last)),
        _ => range.upper.clone(),
    }
}

impl StorageBackend for CacheBackend {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            write_model: WriteModel::AtomicSingleWriter,
            durability: Durability::Evictable,
            limits: self.limits,
        }
    }

    fn get_objects(
        &self,
        digests: &[ObjectDigest],
    ) -> Result<Vec<Option<Arc<[u8]>>>, StorageError> {
        self.limits.check(Limit::ReadItems, digests.len() as u64)?;
        let mut guard = self.cache();
        let cache = &mut *guard;
        let mut found = Vec::with_capacity(digests.len());
        for digest in digests {
            match cache.objects.get(digest.as_bytes()) {
                Some(Some(ObjectEntry::Bytes(bytes))) => found.push(Some(Arc::clone(bytes))),
                Some(None) => found.push(None),
                Some(Some(ObjectEntry::Listed)) | None => {
                    cache.wanted.objects.insert(*digest);
                }
            }
        }
        if found.len() == digests.len() {
            Ok(found)
        } else {
            Err(StorageError::NotLoaded)
        }
    }

    fn scan_objects(
        &self,
        after: Option<&ObjectDigest>,
        limit: u32,
    ) -> Result<ObjectPage, StorageError> {
        self.limits.check_page(limit)?;
        let lower = after.map_or_else(Vec::new, |digest| successor(digest.as_bytes()));
        let need = limit as usize + 1;
        let mut guard = self.cache();
        let cache = &mut *guard;
        let walk = cache.objects.walk(&lower, None, need);
        if let Some(gap) = walk.gap {
            let remaining = need - walk.found.len();
            cache.wanted.object_ranges.insert(Range {
                lower: gap,
                upper: None,
                limit: remaining as u64,
            });
            return Err(StorageError::NotLoaded);
        }
        let mut digests = walk
            .found
            .iter()
            .map(|(key, _)| {
                <[u8; 32]>::try_from(*key)
                    .map(ObjectDigest::from_bytes)
                    .map_err(|_| StorageError::Corrupt("cached digest of the wrong length".into()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let more = digests.len() == need;
        digests.truncate(limit as usize);
        Ok(ObjectPage { digests, more })
    }

    fn read_key(&self, space: KeySpace, key: &[u8]) -> Result<Option<KeyValue>, StorageError> {
        self.limits.check(Limit::KeyBytes, key.len() as u64)?;
        let key = storage_key(space, key);
        let mut cache = self.cache();
        match cache.keys.get(&key) {
            Some(value) => Ok(value.cloned()),
            None => {
                cache.wanted.keys.insert(key);
                Err(StorageError::NotLoaded)
            }
        }
    }

    fn scan_keys(
        &self,
        space: KeySpace,
        prefix: &[u8],
        after: Option<&[u8]>,
        limit: u32,
    ) -> Result<KeyPage, StorageError> {
        self.limits.check_key_scan(prefix, limit)?;
        let start = storage_key(space, prefix);
        let end = prefix_end(&start);
        let lower = match after {
            Some(after) if after >= prefix => successor(&storage_key(space, after)),
            _ => start,
        };
        let need = limit as usize + 1;
        let mut guard = self.cache();
        let cache = &mut *guard;
        let walk = cache.keys.walk(&lower, end.as_deref(), need);
        if let Some(gap) = walk.gap {
            let remaining = need - walk.found.len();
            cache.wanted.key_ranges.insert(Range {
                lower: gap,
                upper: end,
                limit: remaining as u64,
            });
            return Err(StorageError::NotLoaded);
        }
        let more = walk.found.len() == need;
        let entries = walk
            .found
            .iter()
            .take(limit as usize)
            .map(|(key, value)| KeyEntry {
                // Every key between a space's bounds is longer than the space.
                key: key.get(2..).unwrap_or_default().to_vec(),
                value: value.value.clone(),
                revision: value.revision,
            })
            .collect();
        Ok(KeyPage { entries, more })
    }

    fn apply(&mut self, batch: &Batch) -> Result<Applied, StorageError> {
        self.apply_batch(batch)
    }
}

/// Lets a host-driving wrapper apply batches through a shared handle.
#[cfg(feature = "testing")]
impl CacheBackend {
    pub(crate) fn apply_shared(&self, batch: &Batch) -> Result<Applied, StorageError> {
        self.apply_batch(batch)
    }
}
