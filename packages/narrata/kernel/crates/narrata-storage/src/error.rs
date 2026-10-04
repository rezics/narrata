use thiserror::Error;

use crate::{Expect, KeySpace, KeyValue};

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum StorageError {
    /// A key precondition failed; the batch had no effect.
    #[error(transparent)]
    Conflict(Box<Conflict>),
    /// A request exceeded a declared limit; it had no effect.
    #[error(transparent)]
    Limit(LimitExceeded),
    /// A malformed request, such as a key named twice in one batch; it had no effect.
    #[error("invalid storage request: {0}")]
    Invalid(&'static str),
    /// Another writer holds the store; the call had no effect and may be retried.
    #[error("storage is busy")]
    Busy,
    /// The store has no room left; the call had no effect.
    #[error("storage is full")]
    Full,
    /// The store was written in a format this backend does not read; nothing was changed.
    #[error("unsupported storage format: {0}")]
    Format(String),
    /// I/O failed. A batch may or may not have been applied.
    #[error("storage I/O failed: {0}")]
    Io(String),
    /// The store is damaged. A batch may or may not have been applied.
    #[error("storage is corrupt: {0}")]
    Corrupt(String),
}

impl StorageError {
    /// Whether a failed `apply` may nevertheless have taken effect. The engine then re-reads the
    /// affected keys and objects instead of retrying blindly.
    pub const fn outcome_unknown(&self) -> bool {
        matches!(self, Self::Io(_) | Self::Corrupt(_))
    }

    /// Whether the same call may succeed if retried later.
    pub const fn retryable(&self) -> bool {
        matches!(self, Self::Busy)
    }
}

impl From<Conflict> for StorageError {
    fn from(value: Conflict) -> Self {
        Self::Conflict(Box::new(value))
    }
}

/// The first key operation of a batch whose precondition failed, with the key's current state.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error(
    "precondition {expected:?} failed for key operation {index} in space {}: current revision {:?}",
    space.get(),
    actual.as_ref().map(|value| value.revision.get())
)]
pub struct Conflict {
    /// Position in `Batch::keys`.
    pub index: usize,
    pub space: KeySpace,
    pub key: Vec<u8>,
    pub expected: Expect,
    pub actual: Option<KeyValue>,
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[error("{limit:?} limit exceeded: {requested} > {max}")]
pub struct LimitExceeded {
    pub limit: Limit,
    pub requested: u64,
    pub max: u64,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Limit {
    KeyBytes,
    ValueBytes,
    BatchOps,
    BatchBytes,
    ReadItems,
}
