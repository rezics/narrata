use std::collections::BTreeMap;

use narrata_core::{
    CommitId, CompoundSaveManifestId, EffectId, ExecutionId, InputId, InputPayloadDigest, ObjectId,
    ProgramArtifactId, TimelineArchiveManifestId, TimelineCatalogEventId,
};
use narrata_history::HistoryError;
pub use narrata_history::{
    GcKindReport, GcReport, Page, RefRevision, RefScope, RetentionPolicy, scan_all,
};
use narrata_storage::StorageError;
use thiserror::Error;

use crate::{
    CatalogRefKey, CheckedObject, CompoundSaveRefKey, EffectClaim, EffectClaimResult,
    EffectLedgerEntry, EffectOutcomeRecord, EffectStoreError, LeaseId, LedgerFence, RefKey,
    RefName, TimelineArchiveRefKey, TimelineCoverage, object::id,
};

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompoundSaveRefValue {
    pub revision: RefRevision,
    pub manifest: CompoundSaveManifestId,
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

#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("Compound Save CAS conflict for {key}: expected {expected:?}, actual {actual:?}")]
pub struct CompoundSaveConflict {
    pub key: String,
    pub expected: Option<RefRevision>,
    pub actual: Option<CompoundSaveRefValue>,
    pub proposed: Option<CompoundSaveManifestId>,
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

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum StoreError {
    #[error("object {0} was not found")]
    MissingObject(ObjectId),
    #[error("object {0} is corrupt: {1}")]
    Corrupt(ObjectId, String),
    #[error("object kind or schema mismatch for {0}")]
    ObjectKind(ObjectId),
    /// An object of a kind this store does not register; GC refuses to run past one (ADR 0015).
    #[error("object {object} has unregistered kind {kind:#06x}")]
    UnregisteredKind { object: ObjectId, kind: u16 },
    #[error("object graph is invalid: {0}")]
    InvalidGraph(&'static str),
    #[error(transparent)]
    RefConflict(Box<RefConflict>),
    #[error(transparent)]
    CatalogConflict(Box<CatalogConflict>),
    #[error(transparent)]
    ArchiveConflict(Box<ArchiveConflict>),
    #[error(transparent)]
    CompoundSaveConflict(Box<CompoundSaveConflict>),
    #[error(transparent)]
    InputConflict(Box<InputIdConflict>),
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
    #[error(transparent)]
    Storage(StorageError),
    #[error(transparent)]
    Effect(Box<EffectStoreError>),
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

impl From<CompoundSaveConflict> for StoreError {
    fn from(value: CompoundSaveConflict) -> Self {
        Self::CompoundSaveConflict(Box::new(value))
    }
}

impl From<InputIdConflict> for StoreError {
    fn from(value: InputIdConflict) -> Self {
        Self::InputConflict(Box::new(value))
    }
}

impl From<StorageError> for StoreError {
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

impl From<HistoryError> for StoreError {
    fn from(value: HistoryError) -> Self {
        match value {
            HistoryError::MissingObject(object) => Self::MissingObject(id(object)),
            HistoryError::Corrupt(object, diagnostic) => Self::Corrupt(id(object), diagnostic),
            HistoryError::ObjectKind(object) => Self::ObjectKind(id(object)),
            HistoryError::UnregisteredKind { object, kind } => Self::UnregisteredKind {
                object: id(object),
                kind,
            },
            // Commits name their Program, the only named kind of this store.
            HistoryError::Unresolved { .. } => Self::InvalidGraph("Program Artifact is missing"),
            HistoryError::InvalidGraph(reason) => Self::InvalidGraph(reason),
            HistoryError::RefConflict(conflict) => RefConflict {
                key: conflict.key.storage_key(),
                expected: conflict.expected,
                actual: conflict.actual.map(|actual| RefValue {
                    revision: actual.revision,
                    commit: CommitId::from_bytes(*actual.commit.as_bytes()),
                }),
                proposed: conflict
                    .proposed
                    .map(|commit| CommitId::from_bytes(*commit.as_bytes())),
            }
            .into(),
            // Sessions of registered domains; this store registers none.
            HistoryError::ArtifactMismatch { .. }
            | HistoryError::MissingRef(_)
            | HistoryError::Nondeterministic(_)
            | HistoryError::HeadMoved { .. } => {
                Self::InvalidGraph("generic commit operation on a Stage 1–5 store")
            }
            HistoryError::Limit(limit) => Self::Limit(limit),
            HistoryError::Busy => Self::Busy,
            HistoryError::Full => Self::Full,
            HistoryError::Io(message) => Self::Io(message),
            HistoryError::CorruptStore(message) => Self::CorruptStore(message),
            HistoryError::Storage(error) => Self::Storage(error),
        }
    }
}

impl From<EffectStoreError> for StoreError {
    fn from(value: EffectStoreError) -> Self {
        Self::Effect(Box::new(value))
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
pub struct CompoundSaveMutation {
    pub key: CompoundSaveRefKey,
    pub expected: Option<RefRevision>,
    pub next: Option<CompoundSaveManifestId>,
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
    pub compound_saves: Vec<CompoundSaveMutation>,
    pub inputs: Vec<InputRecord>,
    pub pins: Vec<Pin>,
    pub remove_pins: Vec<(String, ObjectId)>,
    pub observed_at: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CommitOutcome {
    pub inserted_objects: usize,
    pub refs: BTreeMap<RefKey, Option<RefValue>>,
    pub catalogs: BTreeMap<CatalogRefKey, Option<CatalogHeadRefValue>>,
    pub archives: BTreeMap<TimelineArchiveRefKey, Option<TimelineArchiveRefValue>>,
    pub compound_saves: BTreeMap<CompoundSaveRefKey, Option<CompoundSaveRefValue>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IntegrityIssue {
    pub object: ObjectId,
    pub diagnostic: String,
}

/// Persistent saves: immutable checked objects, revisioned roots and the Effect ledger.
///
/// Scans take a page limit, which the store lowers to what its backend reads at once.
pub trait SaveStore {
    fn get_object(&self, id: ObjectId) -> Result<Option<CheckedObject>, StoreError>;
    fn get_objects(&self, ids: &[ObjectId]) -> Result<Vec<Option<CheckedObject>>, StoreError>;
    /// The stored Program object with this artifact identity.
    fn find_program(&self, artifact: ProgramArtifactId) -> Result<Option<ObjectId>, StoreError>;
    fn read_ref(&self, key: &RefKey) -> Result<Option<RefValue>, StoreError>;
    fn scan_refs(
        &self,
        scope: &RefScope,
        after: Option<&RefKey>,
        limit: u32,
    ) -> Result<Page<(RefKey, RefValue)>, StoreError>;
    fn read_catalog_head(
        &self,
        key: &CatalogRefKey,
    ) -> Result<Option<CatalogHeadRefValue>, StoreError>;
    fn scan_catalog_heads(
        &self,
        after: Option<&CatalogRefKey>,
        limit: u32,
    ) -> Result<Page<(CatalogRefKey, CatalogHeadRefValue)>, StoreError>;
    fn read_timeline_archive(
        &self,
        key: &TimelineArchiveRefKey,
    ) -> Result<Option<TimelineArchiveRefValue>, StoreError>;
    fn scan_timeline_archives(
        &self,
        timeline: Option<ExecutionId>,
        after: Option<&TimelineArchiveRefKey>,
        limit: u32,
    ) -> Result<Page<(TimelineArchiveRefKey, TimelineArchiveRefValue)>, StoreError>;
    fn read_compound_save(
        &self,
        key: &CompoundSaveRefKey,
    ) -> Result<Option<CompoundSaveRefValue>, StoreError>;
    fn scan_compound_saves(
        &self,
        owner: Option<&RefName>,
        after: Option<&CompoundSaveRefKey>,
        limit: u32,
    ) -> Result<Page<(CompoundSaveRefKey, CompoundSaveRefValue)>, StoreError>;
    fn read_input(
        &self,
        execution: ExecutionId,
        input: InputId,
    ) -> Result<Option<InputRecord>, StoreError>;
    fn scan_pins(
        &self,
        after: Option<&(String, ObjectId)>,
        limit: u32,
    ) -> Result<Page<Pin>, StoreError>;
    fn read_effect(
        &self,
        execution: ExecutionId,
        effect: EffectId,
    ) -> Result<Option<EffectLedgerEntry>, StoreError>;
    fn scan_effects(
        &self,
        execution: ExecutionId,
        after: Option<&EffectId>,
        limit: u32,
    ) -> Result<Page<EffectLedgerEntry>, StoreError>;
    fn current_ledger_fence(&self, execution: ExecutionId) -> Result<LedgerFence, StoreError>;
    /// Commits whose parent is `parent`, by Commit ID.
    fn child_commits(
        &self,
        parent: CommitId,
        after: Option<&CommitId>,
        limit: u32,
    ) -> Result<Page<CommitId>, StoreError>;
    /// Commits of one Execution, by turn and then Commit ID.
    fn timeline_commits(
        &self,
        execution: ExecutionId,
        after: Option<&(u64, CommitId)>,
        limit: u32,
    ) -> Result<Page<(u64, CommitId)>, StoreError>;
    fn claim_effect(&mut self, claim: EffectClaim) -> Result<EffectClaimResult, StoreError>;
    fn renew_effect_lease(
        &mut self,
        execution: ExecutionId,
        effect: EffectId,
        lease: LeaseId,
        now: u64,
        expires_at: u64,
    ) -> Result<EffectLedgerEntry, StoreError>;
    fn record_effect_outcome(
        &mut self,
        record: EffectOutcomeRecord,
    ) -> Result<EffectLedgerEntry, StoreError>;
    fn mark_effect_compensated(
        &mut self,
        execution: ExecutionId,
        original: EffectId,
        by_effect: EffectId,
    ) -> Result<EffectLedgerEntry, StoreError>;
    fn commit(&mut self, transaction: CommitTransaction) -> Result<CommitOutcome, StoreError>;
    fn collect(&mut self, policy: RetentionPolicy) -> Result<GcReport, StoreError>;
    fn integrity_scan(&self) -> Result<Vec<IntegrityIssue>, StoreError>;
}

/// A whole store's domain state, for moving saves between layouts.
#[derive(Clone, Debug, Default)]
pub struct StoreContents {
    /// Objects with the time they were last observed.
    pub objects: Vec<(CheckedObject, u64)>,
    pub refs: Vec<(RefKey, CommitId)>,
    pub catalogs: Vec<(CatalogRefKey, TimelineCatalogEventId, TimelineCoverage)>,
    pub archives: Vec<(TimelineArchiveRefKey, TimelineArchiveManifestId)>,
    pub compound_saves: Vec<(CompoundSaveRefKey, CompoundSaveManifestId)>,
    pub inputs: Vec<InputRecord>,
    pub pins: Vec<Pin>,
    pub effects: Vec<EffectLedgerEntry>,
    pub ledger_fences: Vec<(ExecutionId, LedgerFence)>,
}
