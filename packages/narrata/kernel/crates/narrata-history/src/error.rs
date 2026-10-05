use narrata_storage::StorageError;
use thiserror::Error;

use crate::{ArtifactId, ObjectId, RefKey, RefRevision, RefValue};

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum HistoryError {
    #[error("object {0} was not found")]
    MissingObject(ObjectId),
    #[error("history was truncated before parent commit {0}")]
    HistoryTruncated(ObjectId),
    #[error("object {0} is corrupt: {1}")]
    Corrupt(ObjectId, String),
    #[error("object kind or schema mismatch for {0}")]
    ObjectKind(ObjectId),
    /// The store holds an object whose kind no registrant of this engine knows, so the engine
    /// cannot tell what it refers to.
    #[error("object {object} has kind {kind:#06x}, which is not registered")]
    UnregisteredKind { object: ObjectId, kind: u16 },
    /// An object refers to another by registrant-defined name and the registrant's index has no
    /// entry for it.
    #[error("referenced object of kind {kind:#06x} is not indexed")]
    Unresolved { kind: u16 },
    #[error("object graph is invalid: {0}")]
    InvalidGraph(&'static str),
    #[error("commit {commit} belongs to artifact {actual}, not {expected}")]
    ArtifactMismatch {
        commit: ObjectId,
        expected: ArtifactId,
        actual: ArtifactId,
    },
    #[error("Ref {} does not exist", .0.storage_key())]
    MissingRef(RefKey),
    #[error(transparent)]
    RefConflict(Box<RefConflict>),
    #[error(transparent)]
    Nondeterministic(Box<Nondeterminism>),
    /// A session operation expected its head at one commit and found it elsewhere.
    #[error("session head moved: expected {expected}, found {actual:?}")]
    HeadMoved {
        expected: ObjectId,
        actual: Option<ObjectId>,
    },
    #[error("configured store limit exceeded: {0}")]
    Limit(&'static str),
    #[error("storage is busy")]
    Busy,
    #[error("storage is full")]
    Full,
    #[error("storage I/O failed: {0}")]
    Io(String),
    #[error("storage database is corrupt: {0}")]
    CorruptStore(String),
    /// Limits, malformed requests, unreadable formats, and [`StorageError::NotLoaded`], which
    /// stays itself so that a caching backend's host can load what is missing and run the
    /// operation again.
    #[error(transparent)]
    Storage(StorageError),
}

impl From<StorageError> for HistoryError {
    fn from(value: StorageError) -> Self {
        match value {
            StorageError::Busy | StorageError::Conflict(_) => Self::Busy,
            StorageError::Full => Self::Full,
            StorageError::Io(message) => Self::Io(message),
            StorageError::Corrupt(message) => Self::CorruptStore(message),
            other @ (StorageError::Limit(_)
            | StorageError::Invalid(_)
            | StorageError::Format(_)
            | StorageError::NotLoaded) => Self::Storage(other),
        }
    }
}

impl From<RefConflict> for HistoryError {
    fn from(value: RefConflict) -> Self {
        Self::RefConflict(Box::new(value))
    }
}

impl From<Nondeterminism> for HistoryError {
    fn from(value: Nondeterminism) -> Self {
        Self::Nondeterministic(Box::new(value))
    }
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("Ref CAS conflict for {}: expected {expected:?}, actual {actual:?}", key.storage_key())]
pub struct RefConflict {
    pub key: RefKey,
    pub expected: Option<RefRevision>,
    pub actual: Option<RefValue>,
    pub proposed: Option<ObjectId>,
}

/// One parent and one input led to two different commits: the step that produced them is not
/// deterministic, or the store was given a commit computed elsewhere.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("input {input} after commit {parent} is recorded as {recorded} but produced {proposed}")]
pub struct Nondeterminism {
    pub parent: ObjectId,
    pub input: ObjectId,
    pub recorded: ObjectId,
    pub proposed: ObjectId,
}
