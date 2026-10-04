use std::collections::BTreeMap;

use narrata_core::{
    CommitId, CompoundSaveManifestId, EffectId, ExecutionId, InputId, InputPayloadDigest, ObjectId,
    ProgramArtifactId, TimelineArchiveManifestId, TimelineCatalogEventId,
};
use narrata_storage::{Revision, StorageError};
use thiserror::Error;

use crate::{
    CatalogRefKey, CheckedObject, CompoundSaveRefKey, EffectClaim, EffectClaimResult,
    EffectLedgerEntry, EffectOutcomeRecord, EffectStoreError, LeaseId, LedgerFence, RefKey,
    RefName, RefNamespace, TimelineArchiveRefKey, TimelineCoverage,
};

/// The backend revision at which a Ref, Catalog Head, archive or Compound Save was last written.
///
/// Revisions are store-wide and never reused (ADR 0014); only their equality is meaningful.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct RefRevision(u64);

impl RefRevision {
    pub const fn from_u64(value: u64) -> Option<Self> {
        if value == 0 { None } else { Some(Self(value)) }
    }

    pub const fn get(self) -> u64 {
        self.0
    }

    pub(crate) const fn from_revision(revision: Revision) -> Self {
        Self(revision.get())
    }

    pub(crate) fn revision(self) -> Option<Revision> {
        Revision::new(self.0)
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
            | StorageError::Format(_)) => Self::Storage(other),
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

/// One page of a scan, in key order. `more` is set when the scan stopped at its limit.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub more: bool,
}

/// Which Refs a scan visits.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RefScope {
    All,
    Namespace(RefNamespace),
    Owner(RefNamespace, RefName),
}

/// Reads every page of a scan. `page` receives the key to resume after.
pub fn scan_all<T, K>(
    mut page: impl FnMut(Option<&K>) -> Result<Page<T>, StoreError>,
    resume: impl Fn(&T) -> K,
) -> Result<Vec<T>, StoreError> {
    let mut items = Vec::new();
    let mut after = None;
    loop {
        let next = page(after.as_ref())?;
        let more = next.more;
        after = next.items.last().map(&resume);
        items.extend(next.items);
        if !more || after.is_none() {
            return Ok(items);
        }
    }
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
