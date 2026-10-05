//! A host that answers a cache synchronously from memory, as the IndexedDB adapter does
//! asynchronously, and drivers that run the load-and-retry loop around a cache.
//!
//! Messages pass through their CBOR encoding on the way, so everything driven here also
//! exercises the codec.

use std::{
    collections::BTreeMap,
    ops::Bound,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

use narrata_storage::{
    Applied, Batch, Capabilities, KeyPage, KeySpace, KeyValue, ObjectDigest, ObjectPage, Revision,
    StorageBackend, StorageError,
};

use crate::{
    CacheBackend, Flush, FlushReply, HostError, LoadRequest, Loaded, Range, StoreId, StoreState,
};

/// Loads and retries one operation may take before a driver gives up.
const ATTEMPTS: usize = 64;

/// Lookups a host has served: requested keys and objects, and entries returned by ranges.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct HostReads {
    pub loads: u64,
    pub keys: u64,
    pub objects: u64,
    pub key_entries: u64,
    pub object_entries: u64,
}

impl HostReads {
    /// Items read from the store, whatever their kind.
    pub const fn items(&self) -> u64 {
        self.keys + self.objects + self.key_entries + self.object_entries
    }
}

/// The reference host: one store in memory, read and written the way the IndexedDB adapter
/// reads and writes its object stores. Handles made with [`MemoryHost::share`] reach the same
/// store, like two browser tabs.
#[derive(Debug)]
pub struct MemoryHost {
    store: Arc<Mutex<HostStore>>,
}

#[derive(Debug)]
struct HostStore {
    state: StoreState,
    objects: BTreeMap<ObjectDigest, Arc<[u8]>>,
    keys: BTreeMap<Vec<u8>, KeyValue>,
    reads: HostReads,
}

impl MemoryHost {
    pub fn new(id: StoreId) -> Self {
        Self {
            store: Arc::new(Mutex::new(HostStore {
                state: StoreState { id, revision: 0 },
                objects: BTreeMap::new(),
                keys: BTreeMap::new(),
                reads: HostReads::default(),
            })),
        }
    }

    pub fn share(&self) -> Self {
        Self {
            store: Arc::clone(&self.store),
        }
    }

    fn store(&self) -> MutexGuard<'_, HostStore> {
        self.store.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn state(&self) -> StoreState {
        self.store().state
    }

    pub fn reads(&self) -> HostReads {
        self.store().reads
    }

    /// Returns the reads so far and starts counting again from zero.
    pub fn take_reads(&self) -> HostReads {
        std::mem::take(&mut self.store().reads)
    }

    /// Replaces the store with an empty one under another id, as when the browser evicts it.
    pub fn evict(&self, id: StoreId) {
        let mut store = self.store();
        store.state = StoreState { id, revision: 0 };
        store.objects.clear();
        store.keys.clear();
    }

    /// Answers a request from one consistent state.
    pub fn load(&self, request: &LoadRequest) -> Loaded {
        let mut guard = self.store();
        let store = &mut *guard;
        store.reads.loads += 1;
        store.reads.keys += request.keys.len() as u64;
        store.reads.objects += request.objects.len() as u64;
        let keys = request
            .keys
            .iter()
            .map(|key| (key.clone(), store.keys.get(key).cloned()))
            .collect();
        let objects = request
            .objects
            .iter()
            .map(|digest| (*digest, store.objects.get(digest).cloned()))
            .collect();
        let key_ranges = request
            .key_ranges
            .iter()
            .map(|range| {
                let entries = store
                    .keys
                    .range::<[u8], _>(bounds(range))
                    .take(limit(range))
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect::<Vec<_>>();
                store.reads.key_entries += entries.len() as u64;
                (range.clone(), entries)
            })
            .collect();
        let object_ranges = request
            .object_ranges
            .iter()
            .map(|range| {
                let digests = store
                    .objects
                    .keys()
                    .filter(|digest| range.contains(digest.as_bytes()))
                    .take(limit(range))
                    .copied()
                    .collect::<Vec<_>>();
                store.reads.object_entries += digests.len() as u64;
                (range.clone(), digests)
            })
            .collect();
        Loaded {
            store: store.state,
            keys,
            objects,
            key_ranges,
            object_ranges,
        }
    }

    /// Writes every batch of `flush`, or nothing when the store is not where the first batch
    /// expects it.
    pub fn persist(&self, flush: &Flush) -> FlushReply {
        let mut store = self.store();
        let (Some(first), Some(revision)) = (flush.batches.first(), flush.revision()) else {
            return FlushReply::Persisted {
                revision: store.state.revision,
            };
        };
        if flush.store != store.state.id || first.base != store.state.revision {
            return FlushReply::Conflict(store.state);
        }
        for batch in &flush.batches {
            for (digest, bytes) in &batch.put_objects {
                store
                    .objects
                    .entry(*digest)
                    .or_insert_with(|| Arc::clone(bytes));
            }
            for digest in &batch.delete_objects {
                store.objects.remove(digest);
            }
            if let Some(stamp) = Revision::new(batch.revision) {
                for (key, value) in &batch.put_keys {
                    let value = KeyValue {
                        value: value.clone(),
                        revision: stamp,
                    };
                    store.keys.insert(key.clone(), value);
                }
            }
            for key in &batch.delete_keys {
                store.keys.remove(key);
            }
        }
        store.state.revision = revision;
        FlushReply::Persisted { revision }
    }
}

fn bounds(range: &Range) -> (Bound<&[u8]>, Bound<&[u8]>) {
    let upper = range
        .upper
        .as_deref()
        .map_or(Bound::Unbounded, Bound::Excluded);
    (Bound::Included(range.lower.as_slice()), upper)
}

fn limit(range: &Range) -> usize {
    usize::try_from(range.limit).unwrap_or(usize::MAX)
}

/// Answers the cache's outstanding request; returns whether there was one.
pub fn load(cache: &CacheBackend, host: &MemoryHost) -> Result<bool, HostError> {
    let Some(request) = cache.take_request() else {
        return Ok(false);
    };
    let request = LoadRequest::decode(&request.encode())?;
    let loaded = Loaded::decode(&host.load(&request).encode())?;
    cache.load(&loaded)?;
    Ok(true)
}

/// Persists the cache's unconfirmed batches and confirms them.
pub fn flush(cache: &CacheBackend, host: &MemoryHost) -> Result<(), HostError> {
    let Some(flush) = cache.unconfirmed() else {
        return Ok(());
    };
    let flush = Flush::decode(&flush.encode())?;
    let reply = FlushReply::decode(&host.persist(&flush).encode())?;
    cache.confirm(&reply)
}

/// Why a driven operation did not return a published result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RunError<E> {
    /// The operation failed for its own reasons.
    Failed(E),
    /// Loading or flushing failed; after [`HostError::Superseded`] the caller reloads.
    Host(HostError),
    /// The operation kept needing more after every load.
    Stuck,
}

/// Runs `operation` the way a host does: when it fails and the cache lacks something, load it
/// and run the operation again; when it succeeds, persist and confirm its batches before
/// returning its result.
pub fn run<T, E>(
    cache: &CacheBackend,
    host: &MemoryHost,
    mut operation: impl FnMut() -> Result<T, E>,
) -> Result<T, RunError<E>> {
    for _ in 0..ATTEMPTS {
        match operation() {
            Ok(value) => {
                // A miss the operation recovered from needs no load.
                let _ = cache.take_request();
                flush(cache, host).map_err(RunError::Host)?;
                return Ok(value);
            }
            Err(error) => {
                if !load(cache, host).map_err(RunError::Host)? {
                    return Err(RunError::Failed(error));
                }
            }
        }
    }
    Err(RunError::Stuck)
}

/// A cache whose host answers inside every call, so the conformance suite can run against it:
/// each call loads what it lacks and retries, and each batch is persisted and confirmed before
/// `apply` returns. A cold driver forgets everything before each call, so every call loads.
#[derive(Debug)]
pub struct Driven {
    cache: CacheBackend,
    host: MemoryHost,
    cold: bool,
}

impl Driven {
    /// Keeps what it loaded between calls.
    pub fn warm(host: MemoryHost) -> Self {
        Self {
            cache: CacheBackend::new(),
            host,
            cold: false,
        }
    }

    /// Loads everything each call reads.
    pub fn cold(host: MemoryHost) -> Self {
        Self {
            cold: true,
            ..Self::warm(host)
        }
    }

    pub fn cache(&self) -> &CacheBackend {
        &self.cache
    }

    pub fn host(&self) -> &MemoryHost {
        &self.host
    }

    pub fn into_host(self) -> MemoryHost {
        self.host
    }

    fn call<T>(
        &self,
        call: impl Fn(&CacheBackend) -> Result<T, StorageError>,
    ) -> Result<T, StorageError> {
        let failed = |error: HostError| StorageError::Io(error.to_string());
        if self.cold {
            self.cache.forget();
        }
        for _ in 0..ATTEMPTS {
            match call(&self.cache) {
                Err(StorageError::NotLoaded) => {
                    if !load(&self.cache, &self.host).map_err(failed)? {
                        return Err(StorageError::Io("NotLoaded without a request".to_owned()));
                    }
                }
                other => return other,
            }
        }
        Err(StorageError::Busy)
    }
}

impl StorageBackend for Driven {
    fn capabilities(&self) -> Capabilities {
        self.cache.capabilities()
    }

    fn get_objects(
        &self,
        digests: &[ObjectDigest],
    ) -> Result<Vec<Option<Arc<[u8]>>>, StorageError> {
        self.call(|cache| cache.get_objects(digests))
    }

    fn scan_objects(
        &self,
        after: Option<&ObjectDigest>,
        limit: u32,
    ) -> Result<ObjectPage, StorageError> {
        self.call(|cache| cache.scan_objects(after, limit))
    }

    fn read_key(&self, space: KeySpace, key: &[u8]) -> Result<Option<KeyValue>, StorageError> {
        self.call(|cache| cache.read_key(space, key))
    }

    fn scan_keys(
        &self,
        space: KeySpace,
        prefix: &[u8],
        after: Option<&[u8]>,
        limit: u32,
    ) -> Result<KeyPage, StorageError> {
        self.call(|cache| cache.scan_keys(space, prefix, after, limit))
    }

    fn apply(&mut self, batch: &Batch) -> Result<Applied, StorageError> {
        let applied = self.call(|cache| cache.apply_shared(batch))?;
        flush(&self.cache, &self.host).map_err(|error| StorageError::Io(error.to_string()))?;
        Ok(applied)
    }
}
