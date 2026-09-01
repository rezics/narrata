use std::collections::BTreeMap;

use narrata_core::{
    CommitId, ExecutionId, InputId, InputPayloadDigest, ObjectId, TimelineArchiveManifestId,
    TimelineCatalogEventId,
};
use thiserror::Error;

use crate::{
    CatalogRefKey, CheckedObject, RefKey, TimelineArchiveRefKey, TimelineCoverage,
    TimelineOperationId,
};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct RefRevision(u64);

impl RefRevision {
    pub const fn initial() -> Self {
        Self(1)
    }

    pub const fn from_u64(value: u64) -> Option<Self> {
        if value == 0 { None } else { Some(Self(value)) }
    }

    pub const fn get(self) -> u64 {
        self.0
    }

    pub fn next(self) -> Result<Self, StoreError> {
        self.0
            .checked_add(1)
            .and_then(Self::from_u64)
            .ok_or(StoreError::RevisionOverflow)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RefValue {
    pub revision: RefRevision,
    pub commit: CommitId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CatalogHeadRefValue {
    pub revision: RefRevision,
    pub event: TimelineCatalogEventId,
    pub coverage: TimelineCoverage,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimelineArchiveRefValue {
    pub revision: RefRevision,
    pub manifest: TimelineArchiveManifestId,
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("Ref CAS conflict for {key}: expected {expected:?}, actual {actual:?}")]
pub struct RefConflict {
    pub key: String,
    pub expected: Option<RefRevision>,
    pub actual: Option<RefValue>,
    pub proposed: Option<CommitId>,
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("Catalog Head CAS conflict for {key}: expected {expected:?}, actual {actual:?}")]
pub struct CatalogConflict {
    pub key: String,
    pub expected: Option<RefRevision>,
    pub actual: Option<CatalogHeadRefValue>,
    pub proposed: Option<TimelineCatalogEventId>,
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("Timeline Archive CAS conflict for {key}: expected {expected:?}, actual {actual:?}")]
pub struct ArchiveConflict {
    pub key: String,
    pub expected: Option<RefRevision>,
    pub actual: Option<TimelineArchiveRefValue>,
    pub proposed: Option<TimelineArchiveManifestId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InputRecord {
    pub execution: ExecutionId,
    pub input: InputId,
    pub parent: CommitId,
    pub payload: InputPayloadDigest,
    pub commit: CommitId,
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("InputId was already committed with another parent or payload")]
pub struct InputIdConflict {
    pub existing: InputRecord,
    pub proposed: InputRecord,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PutOutcome {
    Inserted,
    AlreadyPresent,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum FaultPoint {
    SnapshotWrite,
    ReceiptWrite,
    CommitWrite,
    EdgeIndexWrite,
    RefCas,
    CatalogEventWrite,
    CatalogHeadCas,
    ArchiveRefCas,
    TransactionCommit,
    GcMark,
    GcSweep,
    BundleImportRef,
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum StoreError {
    #[error("object {0} was not found")]
    MissingObject(ObjectId),
    #[error("object {0} is corrupt: {1}")]
    Corrupt(ObjectId, String),
    #[error("object kind or schema mismatch for {0}")]
    ObjectKind(ObjectId),
    #[error("object graph is invalid: {0}")]
    InvalidGraph(&'static str),
    #[error(transparent)]
    RefConflict(Box<RefConflict>),
    #[error(transparent)]
    CatalogConflict(Box<CatalogConflict>),
    #[error(transparent)]
    ArchiveConflict(Box<ArchiveConflict>),
    #[error(transparent)]
    InputConflict(Box<InputIdConflict>),
    #[error("Ref revision overflow")]
    RevisionOverflow,
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
    #[error("injected transaction fault at {0:?}")]
    InjectedFault(FaultPoint),
}

impl From<RefConflict> for StoreError {
    fn from(value: RefConflict) -> Self {
        Self::RefConflict(Box::new(value))
    }
}

impl From<CatalogConflict> for StoreError {
    fn from(value: CatalogConflict) -> Self {
        Self::CatalogConflict(Box::new(value))
    }
}

impl From<ArchiveConflict> for StoreError {
    fn from(value: ArchiveConflict) -> Self {
        Self::ArchiveConflict(Box::new(value))
    }
}

impl From<InputIdConflict> for StoreError {
    fn from(value: InputIdConflict) -> Self {
        Self::InputConflict(Box::new(value))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RefMutation {
    pub key: RefKey,
    pub expected: Option<RefRevision>,
    pub next: Option<CommitId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogMutation {
    pub key: CatalogRefKey,
    pub expected: Option<RefRevision>,
    pub next: Option<TimelineCatalogEventId>,
    pub coverage: TimelineCoverage,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArchiveMutation {
    pub key: TimelineArchiveRefKey,
    pub expected: Option<RefRevision>,
    pub next: Option<TimelineArchiveManifestId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Pin {
    pub owner: String,
    pub object: ObjectId,
    pub expires_at: Option<u64>,
}

#[derive(Clone, Debug, Default)]
pub struct CommitTransaction {
    pub objects: Vec<CheckedObject>,
    pub refs: Vec<RefMutation>,
    pub catalogs: Vec<CatalogMutation>,
    pub archives: Vec<ArchiveMutation>,
    pub inputs: Vec<InputRecord>,
    pub pins: Vec<Pin>,
    pub remove_pins: Vec<(String, ObjectId)>,
    pub observed_at: u64,
    pub import_transaction: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CommitOutcome {
    pub inserted_objects: usize,
    pub refs: BTreeMap<RefKey, Option<RefValue>>,
    pub catalogs: BTreeMap<CatalogRefKey, Option<CatalogHeadRefValue>>,
    pub archives: BTreeMap<TimelineArchiveRefKey, Option<TimelineArchiveRefValue>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RetentionPolicy {
    pub now: u64,
    pub grace_seconds: u64,
    pub dry_run: bool,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            now: u64::MAX,
            grace_seconds: 0,
            dry_run: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct GcKindReport {
    pub objects: u64,
    pub bytes: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct GcReport {
    pub roots: u64,
    pub reachable: u64,
    pub removed: BTreeMap<u16, GcKindReport>,
    pub dry_run: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IntegrityIssue {
    pub object: ObjectId,
    pub diagnostic: String,
}

pub trait SaveStore {
    fn get_object(&self, id: ObjectId) -> Result<Option<CheckedObject>, StoreError>;
    fn list_objects(&self) -> Result<Vec<CheckedObject>, StoreError>;
    fn read_ref(&self, key: &RefKey) -> Result<Option<RefValue>, StoreError>;
    fn list_refs(&self) -> Result<Vec<(RefKey, RefValue)>, StoreError>;
    fn read_catalog_head(
        &self,
        key: &CatalogRefKey,
    ) -> Result<Option<CatalogHeadRefValue>, StoreError>;
    fn list_catalog_heads(&self) -> Result<Vec<(CatalogRefKey, CatalogHeadRefValue)>, StoreError>;
    fn read_timeline_archive(
        &self,
        key: &TimelineArchiveRefKey,
    ) -> Result<Option<TimelineArchiveRefValue>, StoreError>;
    fn list_timeline_archives(
        &self,
    ) -> Result<Vec<(TimelineArchiveRefKey, TimelineArchiveRefValue)>, StoreError>;
    fn read_input(
        &self,
        execution: ExecutionId,
        input: InputId,
    ) -> Result<Option<InputRecord>, StoreError>;
    fn list_pins(&self) -> Result<Vec<Pin>, StoreError>;
    fn commit(&mut self, transaction: CommitTransaction) -> Result<CommitOutcome, StoreError>;
    fn collect(&mut self, policy: RetentionPolicy) -> Result<GcReport, StoreError>;
    fn integrity_scan(&self) -> Result<Vec<IntegrityIssue>, StoreError>;
}

pub(crate) fn check_expected_ref(
    key: &RefKey,
    expected: Option<RefRevision>,
    actual: Option<RefValue>,
    proposed: Option<CommitId>,
) -> Result<(), StoreError> {
    if actual.map(|value| value.revision) == expected {
        Ok(())
    } else {
        Err(RefConflict {
            key: key.storage_key(),
            expected,
            actual,
            proposed,
        }
        .into())
    }
}

pub(crate) fn check_expected_catalog(
    key: &CatalogRefKey,
    expected: Option<RefRevision>,
    actual: Option<CatalogHeadRefValue>,
    proposed: Option<TimelineCatalogEventId>,
) -> Result<(), StoreError> {
    if actual.map(|value| value.revision) == expected {
        Ok(())
    } else {
        Err(CatalogConflict {
            key: key.storage_key(),
            expected,
            actual,
            proposed,
        }
        .into())
    }
}

pub(crate) fn check_expected_archive(
    key: &TimelineArchiveRefKey,
    expected: Option<RefRevision>,
    actual: Option<TimelineArchiveRefValue>,
    proposed: Option<TimelineArchiveManifestId>,
) -> Result<(), StoreError> {
    if actual.map(|value| value.revision) == expected {
        Ok(())
    } else {
        Err(ArchiveConflict {
            key: key.storage_key(),
            expected,
            actual,
            proposed,
        }
        .into())
    }
}

pub(crate) fn next_revision(current: Option<RefRevision>) -> Result<RefRevision, StoreError> {
    current.map_or(Ok(RefRevision::initial()), RefRevision::next)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CatalogOperationRecord {
    pub previous: Option<TimelineCatalogEventId>,
    pub event: TimelineCatalogEventId,
}

pub(crate) type CatalogOperationIndex =
    BTreeMap<(ExecutionId, TimelineOperationId), CatalogOperationRecord>;
