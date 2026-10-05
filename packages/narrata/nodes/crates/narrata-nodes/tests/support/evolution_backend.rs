#![allow(clippy::unwrap_used)]
use narrata_storage::*;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Clone, Copy)]
pub enum Failure {
    BeforeBusy,
    AfterIo,
}
#[derive(Clone)]
pub struct ObservedBackend {
    inner: Arc<Mutex<MemoryBackend>>,
    pub writes: Arc<AtomicUsize>,
    pub failure: Arc<Mutex<Option<Failure>>>,
}
impl ObservedBackend {
    pub fn new(inner: MemoryBackend) -> Self {
        Self {
            inner: Arc::new(Mutex::new(inner)),
            writes: Arc::new(AtomicUsize::new(0)),
            failure: Arc::new(Mutex::new(None)),
        }
    }
}
impl StorageBackend for ObservedBackend {
    fn capabilities(&self) -> Capabilities {
        self.inner.lock().unwrap().capabilities()
    }
    fn get_objects(&self, ids: &[ObjectDigest]) -> Result<Vec<Option<Arc<[u8]>>>, StorageError> {
        self.inner.lock().unwrap().get_objects(ids)
    }
    fn scan_objects(
        &self,
        after: Option<&ObjectDigest>,
        limit: u32,
    ) -> Result<ObjectPage, StorageError> {
        self.inner.lock().unwrap().scan_objects(after, limit)
    }
    fn read_key(&self, space: KeySpace, key: &[u8]) -> Result<Option<KeyValue>, StorageError> {
        self.inner.lock().unwrap().read_key(space, key)
    }
    fn scan_keys(
        &self,
        space: KeySpace,
        prefix: &[u8],
        after: Option<&[u8]>,
        limit: u32,
    ) -> Result<KeyPage, StorageError> {
        self.inner
            .lock()
            .unwrap()
            .scan_keys(space, prefix, after, limit)
    }
    fn apply(&mut self, batch: &Batch) -> Result<Applied, StorageError> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        let failure = self.failure.lock().unwrap().take();
        if matches!(failure, Some(Failure::BeforeBusy)) {
            return Err(StorageError::Busy);
        }
        let applied = self.inner.lock().unwrap().apply(batch)?;
        if matches!(failure, Some(Failure::AfterIo)) {
            return Err(StorageError::Io("reply lost after atomic write".into()));
        }
        Ok(applied)
    }
}
