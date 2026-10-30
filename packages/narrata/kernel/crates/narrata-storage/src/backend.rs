use std::sync::Arc;

use crate::{
    Applied, Batch, Capabilities, KeyPage, KeySpace, KeyValue, ObjectDigest, ObjectPage,
    StorageError,
};

/// A store that knows bytes, digests and keys, and nothing about narrative types.
///
/// Calls are synchronous: embedded native hosts and in-page Wasm call storage synchronously,
/// and asynchronous hosts attach through a protocol in which the engine emits batches and the
/// host acknowledges them once persisted (ADR 0012). Reads observe every batch that returned
/// before they started; there are no read transactions across calls, so the engine reads, then
/// writes with preconditions, and re-reads on conflict.
pub trait StorageBackend {
    fn capabilities(&self) -> Capabilities;

    /// Returns object bytes aligned with `digests`; at most `max_read_items` digests.
    fn get_objects(&self, digests: &[ObjectDigest])
    -> Result<Vec<Option<Arc<[u8]>>>, StorageError>;

    fn get_object(&self, digest: &ObjectDigest) -> Result<Option<Arc<[u8]>>, StorageError> {
        Ok(self
            .get_objects(std::slice::from_ref(digest))?
            .into_iter()
            .next()
            .flatten())
    }

    /// Bounds payload materialization before reading. Legacy providers reserve their declared
    /// worst case; providers with size metadata can check exact sizes without copying values.
    fn get_objects_bounded(
        &self,
        digests: &[ObjectDigest],
        max_bytes: u64,
    ) -> Result<Vec<Option<Arc<[u8]>>>, StorageError> {
        check_read_bytes(
            (digests.len() as u64).saturating_mul(self.capabilities().limits.max_value_bytes),
            max_bytes,
        )?;
        self.get_objects(digests)
    }

    fn read_key_bounded(
        &self,
        space: KeySpace,
        key: &[u8],
        max_bytes: u64,
    ) -> Result<Option<KeyValue>, StorageError> {
        check_read_bytes(self.capabilities().limits.max_value_bytes, max_bytes)?;
        self.read_key(space, key)
    }

    /// Includes any value-bearing lookahead in the bound, even though it is not returned.
    fn scan_keys_bounded(
        &self,
        space: KeySpace,
        prefix: &[u8],
        after: Option<&[u8]>,
        limit: u32,
        max_bytes: u64,
    ) -> Result<KeyPage, StorageError> {
        let limits = self.capabilities().limits;
        check_read_bytes(
            (u64::from(limit) + 1)
                .saturating_mul(limits.max_key_bytes.saturating_add(limits.max_value_bytes)),
            max_bytes,
        )?;
        self.scan_keys(space, prefix, after, limit)
    }

    /// Lists object digests in ascending order after the `after` cursor, one bounded page at a
    /// time. Only GC sweeps and integrity checks need it.
    fn scan_objects(
        &self,
        after: Option<&ObjectDigest>,
        limit: u32,
    ) -> Result<ObjectPage, StorageError>;

    fn read_key(&self, space: KeySpace, key: &[u8]) -> Result<Option<KeyValue>, StorageError>;

    /// Lists keys of `space` that start with `prefix`, in ascending byte order, strictly after
    /// the `after` cursor (the last key of the previous page), at most `limit` entries.
    fn scan_keys(
        &self,
        space: KeySpace,
        prefix: &[u8],
        after: Option<&[u8]>,
        limit: u32,
    ) -> Result<KeyPage, StorageError>;

    /// Applies `batch` entirely or not at all. On error, [`StorageError::outcome_unknown`]
    /// tells whether the batch may nevertheless have been applied.
    fn apply(&mut self, batch: &Batch) -> Result<Applied, StorageError>;
}

pub fn check_read_bytes(requested: u64, max: u64) -> Result<(), StorageError> {
    if requested > max {
        Err(StorageError::Limit(crate::LimitExceeded {
            limit: crate::Limit::BatchBytes,
            requested,
            max,
        }))
    } else {
        Ok(())
    }
}
