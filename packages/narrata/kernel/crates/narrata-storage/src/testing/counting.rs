use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use super::Primitive;
use crate::{
    Applied, Batch, Capabilities, KeyPage, KeySpace, KeyValue, ObjectDigest, ObjectPage,
    StorageBackend, StorageError,
};

/// Calls and returned items since the last reset.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Counts {
    pub get_objects: u64,
    pub scan_objects: u64,
    pub read_key: u64,
    pub scan_keys: u64,
    pub apply: u64,
    /// Objects found by `get_objects`.
    pub objects_returned: u64,
    /// Digests returned by `scan_objects`.
    pub objects_scanned: u64,
    /// Entries returned by `scan_keys`.
    pub keys_scanned: u64,
}

impl Counts {
    pub const fn calls(&self, primitive: Primitive) -> u64 {
        match primitive {
            Primitive::GetObjects => self.get_objects,
            Primitive::ScanObjects => self.scan_objects,
            Primitive::ReadKey => self.read_key,
            Primitive::ScanKeys => self.scan_keys,
            Primitive::Apply => self.apply,
        }
    }
}

/// Wraps a backend and counts calls per primitive, objects returned and entries scanned, so a
/// test can assert, for example, that loading one commit never enumerates all objects.
#[derive(Debug)]
pub struct Counting<B> {
    inner: B,
    calls: [AtomicU64; 5],
    objects_returned: AtomicU64,
    objects_scanned: AtomicU64,
    keys_scanned: AtomicU64,
}

impl<B> Counting<B> {
    pub fn new(inner: B) -> Self {
        Self {
            inner,
            calls: Default::default(),
            objects_returned: AtomicU64::new(0),
            objects_scanned: AtomicU64::new(0),
            keys_scanned: AtomicU64::new(0),
        }
    }

    pub fn counts(&self) -> Counts {
        let calls = |primitive: Primitive| self.calls[primitive.index()].load(Ordering::SeqCst);
        Counts {
            get_objects: calls(Primitive::GetObjects),
            scan_objects: calls(Primitive::ScanObjects),
            read_key: calls(Primitive::ReadKey),
            scan_keys: calls(Primitive::ScanKeys),
            apply: calls(Primitive::Apply),
            objects_returned: self.objects_returned.load(Ordering::SeqCst),
            objects_scanned: self.objects_scanned.load(Ordering::SeqCst),
            keys_scanned: self.keys_scanned.load(Ordering::SeqCst),
        }
    }

    /// Returns the counts so far and starts again from zero.
    pub fn take_counts(&self) -> Counts {
        let counts = self.counts();
        for counter in self.calls.iter().chain([
            &self.objects_returned,
            &self.objects_scanned,
            &self.keys_scanned,
        ]) {
            counter.store(0, Ordering::SeqCst);
        }
        counts
    }

    pub fn inner(&self) -> &B {
        &self.inner
    }

    pub fn into_inner(self) -> B {
        self.inner
    }

    fn count(&self, primitive: Primitive) {
        self.calls[primitive.index()].fetch_add(1, Ordering::SeqCst);
    }
}

impl<B: StorageBackend> StorageBackend for Counting<B> {
    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }

    fn get_objects(
        &self,
        digests: &[ObjectDigest],
    ) -> Result<Vec<Option<Arc<[u8]>>>, StorageError> {
        self.count(Primitive::GetObjects);
        let objects = self.inner.get_objects(digests)?;
        let found = objects.iter().filter(|object| object.is_some()).count();
        self.objects_returned
            .fetch_add(found as u64, Ordering::SeqCst);
        Ok(objects)
    }

    fn scan_objects(
        &self,
        after: Option<&ObjectDigest>,
        limit: u32,
    ) -> Result<ObjectPage, StorageError> {
        self.count(Primitive::ScanObjects);
        let page = self.inner.scan_objects(after, limit)?;
        self.objects_scanned
            .fetch_add(page.digests.len() as u64, Ordering::SeqCst);
        Ok(page)
    }

    fn read_key(&self, space: KeySpace, key: &[u8]) -> Result<Option<KeyValue>, StorageError> {
        self.count(Primitive::ReadKey);
        self.inner.read_key(space, key)
    }

    fn scan_keys(
        &self,
        space: KeySpace,
        prefix: &[u8],
        after: Option<&[u8]>,
        limit: u32,
    ) -> Result<KeyPage, StorageError> {
        self.count(Primitive::ScanKeys);
        let page = self.inner.scan_keys(space, prefix, after, limit)?;
        self.keys_scanned
            .fetch_add(page.entries.len() as u64, Ordering::SeqCst);
        Ok(page)
    }

    fn apply(&mut self, batch: &Batch) -> Result<Applied, StorageError> {
        self.count(Primitive::Apply);
        self.inner.apply(batch)
    }
}
