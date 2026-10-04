use crate::{Limit, LimitExceeded, StorageError};

/// What a backend guarantees. The engine reads it to choose a write pattern or to refuse to run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Capabilities {
    pub write_model: WriteModel,
    pub durability: Durability,
    pub limits: Limits,
}

impl Capabilities {
    pub const fn atomic_multi_key(&self) -> bool {
        matches!(
            self.write_model,
            WriteModel::AtomicConcurrent | WriteModel::AtomicSingleWriter
        )
    }

    pub const fn concurrent_writers(&self) -> bool {
        matches!(self.write_model, WriteModel::AtomicConcurrent)
    }
}

/// Multi-key atomicity and writer concurrency as one choice, so that a backend cannot declare
/// concurrent writers without atomic batches.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WriteModel {
    /// Every batch commits entirely or not at all, and several handles may write; the backend
    /// serializes their batches.
    AtomicConcurrent,
    /// Every batch commits entirely or not at all; only one handle may write at a time.
    AtomicSingleWriter,
    /// Objects land one by one, then at most one key operation per batch applies atomically.
    /// The engine writes all objects first and finishes with a CAS on a single root key; a
    /// failed batch may leave unreferenced objects behind for GC. Single writer only.
    RootKeyOnly,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Durability {
    /// A successful batch survives process crashes, as far as the filesystem honours fsync.
    Durable,
    /// Data survives restarts, but the platform may delete the whole store (browser storage).
    Evictable,
    /// Data is lost when the last handle is dropped.
    Memory,
}

/// Size bounds a backend enforces. Requests over a bound fail before anything is written.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Limits {
    pub max_key_bytes: u64,
    /// Applies to object bytes and to key values alike.
    pub max_value_bytes: u64,
    /// Object puts, object deletes and key operations together.
    pub max_batch_ops: u64,
    /// Key, value and object bytes together; digests are not counted.
    pub max_batch_bytes: u64,
    /// Entries per scan page and digests per object read.
    pub max_read_items: u64,
}

impl Limits {
    /// Bounds the in-tree backends declare unless configured otherwise.
    pub const DEFAULT: Self = Self {
        max_key_bytes: 1024,
        max_value_bytes: 16 * 1024 * 1024,
        max_batch_ops: 10_000,
        max_batch_bytes: 64 * 1024 * 1024,
        max_read_items: 1024,
    };

    pub fn check(self, limit: Limit, requested: u64) -> Result<(), StorageError> {
        let max = match limit {
            Limit::KeyBytes => self.max_key_bytes,
            Limit::ValueBytes => self.max_value_bytes,
            Limit::BatchOps => self.max_batch_ops,
            Limit::BatchBytes => self.max_batch_bytes,
            Limit::ReadItems => self.max_read_items,
        };
        if requested <= max {
            Ok(())
        } else {
            Err(StorageError::Limit(LimitExceeded {
                limit,
                requested,
                max,
            }))
        }
    }

    /// Validates the arguments every key scan shares.
    pub fn check_key_scan(self, prefix: &[u8], limit: u32) -> Result<(), StorageError> {
        self.check(Limit::KeyBytes, len(prefix))?;
        self.check_page(limit)
    }

    /// Validates a page size for key or object scans.
    pub fn check_page(self, limit: u32) -> Result<(), StorageError> {
        if limit == 0 {
            return Err(StorageError::Invalid("scan limit must be positive"));
        }
        self.check(Limit::ReadItems, u64::from(limit))
    }
}

impl Default for Limits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

pub(crate) const fn len(bytes: &[u8]) -> u64 {
    bytes.len() as u64
}
