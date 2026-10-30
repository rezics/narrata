use std::{
    collections::{BTreeMap, btree_map::Entry},
    ops::Bound,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

use crate::{
    Applied, Batch, Capabilities, Conflict, Durability, KeyAction, KeyEntry, KeyPage, KeySpace,
    KeyValue, Limit, Limits, ObjectDigest, ObjectPage, Revision, StorageBackend, StorageError,
    WriteModel, capabilities::len,
};

/// Behaviour reference for the contract: plain ordered maps behind a mutex.
///
/// Handles made with [`MemoryBackend::share`] see the same store and may write concurrently.
/// The data lives as long as any handle does.
#[derive(Debug, Default)]
pub struct MemoryBackend {
    store: Arc<Mutex<Store>>,
    limits: Limits,
}

#[derive(Debug, Default)]
struct Store {
    objects: BTreeMap<ObjectDigest, Arc<[u8]>>,
    spaces: BTreeMap<KeySpace, BTreeMap<Vec<u8>, KeyValue>>,
    last_revision: u64,
}

impl MemoryBackend {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_limits(limits: Limits) -> Self {
        Self {
            store: Arc::default(),
            limits,
        }
    }

    /// Returns another handle to the same store.
    pub fn share(&self) -> Self {
        Self {
            store: Arc::clone(&self.store),
            limits: self.limits,
        }
    }

    fn store(&self) -> MutexGuard<'_, Store> {
        // Batches are validated before the first mutation and mutations cannot fail, so a
        // panicking holder cannot have left a half-applied batch behind.
        self.store.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl StorageBackend for MemoryBackend {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            write_model: WriteModel::AtomicConcurrent,
            durability: Durability::Memory,
            limits: self.limits,
        }
    }

    fn get_objects(
        &self,
        digests: &[ObjectDigest],
    ) -> Result<Vec<Option<Arc<[u8]>>>, StorageError> {
        self.get_objects_bounded(digests, u64::MAX)
    }

    fn get_objects_bounded(
        &self,
        digests: &[ObjectDigest],
        max_bytes: u64,
    ) -> Result<Vec<Option<Arc<[u8]>>>, StorageError> {
        self.limits.check(Limit::ReadItems, digests.len() as u64)?;
        let store = self.store();
        crate::check_read_bytes(
            digests
                .iter()
                .filter_map(|id| store.objects.get(id))
                .map(|b| b.len() as u64)
                .sum(),
            max_bytes,
        )?;
        Ok(digests
            .iter()
            .map(|digest| store.objects.get(digest).cloned())
            .collect())
    }

    fn scan_objects(
        &self,
        after: Option<&ObjectDigest>,
        limit: u32,
    ) -> Result<ObjectPage, StorageError> {
        self.limits.check_page(limit)?;
        let store = self.store();
        let lower = after.map_or(Bound::Unbounded, Bound::Excluded);
        let mut digests = store
            .objects
            .range((lower, Bound::Unbounded))
            .map(|(digest, _)| *digest);
        let page = digests.by_ref().take(limit as usize).collect();
        Ok(ObjectPage {
            digests: page,
            more: digests.next().is_some(),
        })
    }

    fn read_key(&self, space: KeySpace, key: &[u8]) -> Result<Option<KeyValue>, StorageError> {
        self.read_key_bounded(space, key, u64::MAX)
    }

    fn read_key_bounded(
        &self,
        space: KeySpace,
        key: &[u8],
        max_bytes: u64,
    ) -> Result<Option<KeyValue>, StorageError> {
        self.limits.check(Limit::KeyBytes, len(key))?;
        let store = self.store();
        let value = store.spaces.get(&space).and_then(|keys| keys.get(key));
        crate::check_read_bytes(value.map_or(0, |v| v.value.len() as u64), max_bytes)?;
        Ok(value.cloned())
    }

    fn scan_keys(
        &self,
        space: KeySpace,
        prefix: &[u8],
        after: Option<&[u8]>,
        limit: u32,
    ) -> Result<KeyPage, StorageError> {
        self.scan_keys_bounded(space, prefix, after, limit, u64::MAX)
    }

    fn scan_keys_bounded(
        &self,
        space: KeySpace,
        prefix: &[u8],
        after: Option<&[u8]>,
        limit: u32,
        max_bytes: u64,
    ) -> Result<KeyPage, StorageError> {
        self.limits.check_key_scan(prefix, limit)?;
        let store = self.store();
        let Some(keys) = store.spaces.get(&space) else {
            return Ok(KeyPage::default());
        };
        let lower = match after {
            Some(after) if after >= prefix => Bound::Excluded(after),
            _ => Bound::Included(prefix),
        };
        let mut matching = keys
            .range::<[u8], _>((lower, Bound::Unbounded))
            .take_while(|(key, _)| key.starts_with(prefix));
        let selected = matching.by_ref().take(limit as usize).collect::<Vec<_>>();
        crate::check_read_bytes(
            selected
                .iter()
                .map(|(k, v)| (k.len() + v.value.len()) as u64)
                .sum(),
            max_bytes,
        )?;
        let more = matching.next().is_some(); // Lookahead borrows, never copies a value.
        let entries = selected
            .into_iter()
            .map(|(key, value)| KeyEntry {
                key: key.clone(),
                value: value.value.clone(),
                revision: value.revision,
            })
            .collect();
        Ok(KeyPage { entries, more })
    }

    fn apply(&mut self, batch: &Batch) -> Result<Applied, StorageError> {
        batch.validate(&self.capabilities())?;
        let mut store = self.store();
        for (index, op) in batch.keys.iter().enumerate() {
            let actual = store
                .spaces
                .get(&op.space)
                .and_then(|keys| keys.get(&op.key));
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
        let revision = store
            .last_revision
            .checked_add(1)
            .and_then(Revision::new)
            .ok_or(StorageError::Full)?;

        let mut applied = Applied::default();
        if batch.puts_keys() {
            store.last_revision = revision.get();
            applied.revision = Some(revision);
        }
        for (digest, bytes) in &batch.put_objects {
            if let Entry::Vacant(entry) = store.objects.entry(*digest) {
                entry.insert(Arc::clone(bytes));
                applied.objects_inserted += 1;
            }
        }
        for digest in &batch.delete_objects {
            if store.objects.remove(digest).is_some() {
                applied.objects_deleted += 1;
            }
        }
        for op in &batch.keys {
            match &op.action {
                KeyAction::Put(value) => {
                    store.spaces.entry(op.space).or_default().insert(
                        op.key.clone(),
                        KeyValue {
                            value: value.clone(),
                            revision,
                        },
                    );
                }
                KeyAction::Delete => {
                    if let Some(keys) = store.spaces.get_mut(&op.space) {
                        keys.remove(&op.key);
                        if keys.is_empty() {
                            store.spaces.remove(&op.space);
                        }
                    }
                }
                KeyAction::Check => {}
            }
        }
        Ok(applied)
    }
}
