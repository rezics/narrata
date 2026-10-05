//! The save engine: one [`SaveStore`] implementation over any [`StorageBackend`] (ADR 0014),
//! built on the history engine with the Stage 1–5 kinds as its registrant (ADR 0015).
//!
//! Objects, Refs, Pins, the children index, GC and the integrity scan are the history layer's;
//! this module adds the Stage 1–5 roots, inputs and the Effect ledger in their own key spaces.
//! Every key value is decoded strictly, so a backend can lose data but cannot make the engine
//! accept data it did not write.

mod effects;
mod registry;
mod validate;
mod write;

use std::fmt;

use narrata_core::{CommitId, EffectId, ExecutionId, InputId, ObjectId, ProgramArtifactId};
use narrata_history::{History, Reader};
use narrata_storage::{KeyEntry, KeySpace, KeyValue, MemoryBackend, Revision, StorageBackend};
pub(crate) use registry::{Legacy, PROGRAM};

use crate::{
    CatalogHeadRefValue, CatalogRefKey, CheckedObject, CommitOutcome, CommitTransaction,
    CompoundSaveRefKey, CompoundSaveRefValue, EffectClaim, EffectClaimResult, EffectLedgerEntry,
    EffectOutcomeRecord, GcReport, InputRecord, IntegrityIssue, LeaseId, LedgerFence, Page, Pin,
    RefKey, RefName, RefRevision, RefScope, RefValue, RetentionPolicy, SaveStore, StoreContents,
    StoreError, TimelineArchiveRefKey, TimelineArchiveRefValue, layout,
    object::{history_id, id},
};

/// A [`SaveStore`] over the storage backend `B`.
pub struct Store<B> {
    history: History<B, Legacy>,
}

/// The engine over the in-memory reference backend.
pub type MemoryStore = Store<MemoryBackend>;

impl Store<MemoryBackend> {
    pub fn new() -> Self {
        Self {
            history: History::in_memory(Legacy::default()),
        }
    }
}

impl Default for Store<MemoryBackend> {
    fn default() -> Self {
        Self::new()
    }
}

impl<B: fmt::Debug> fmt::Debug for Store<B> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Store")
            .field("history", &self.history)
            .finish_non_exhaustive()
    }
}

impl<B: StorageBackend> Store<B> {
    /// Opens a store in layout v1, initialising an empty backend. A backend holding anything
    /// else, including a newer layout, is refused with a format error and left untouched.
    pub fn open(backend: B) -> Result<Self, StoreError> {
        Ok(Self {
            history: History::open(backend, Legacy::default())?,
        })
    }

    /// Writes `contents` into an empty backend and opens it. Objects keep their bytes and
    /// observation times; revisions are new.
    pub fn restore(backend: B, contents: StoreContents) -> Result<Self, StoreError> {
        let mut store = Self::open(backend)?;
        if !store.history.is_empty_except_meta()? {
            return Err(
                narrata_storage::StorageError::Invalid("restore needs an empty store").into(),
            );
        }
        store.write_contents(contents)?;
        Ok(store)
    }

    pub fn backend(&self) -> &B {
        self.history.backend()
    }

    pub fn backend_mut(&mut self) -> &mut B {
        self.history.backend_mut()
    }

    pub fn into_backend(self) -> B {
        self.history.into_backend()
    }

    fn reader(&self) -> Reader<'_, B> {
        self.history.reader()
    }

    fn read_key(&self, space: KeySpace, key: &[u8]) -> Result<Option<KeyValue>, StoreError> {
        Ok(self.reader().read_key(space, key)?)
    }

    /// The revision of `meta/sweep` that writers check.
    fn sweep(&self) -> Result<Option<Revision>, StoreError> {
        Ok(self.reader().sweep()?)
    }

    fn page<T>(
        &self,
        space: KeySpace,
        prefix: &[u8],
        after: Option<Vec<u8>>,
        limit: u32,
        map: impl Fn(&KeyEntry) -> Result<T, StoreError>,
    ) -> Result<Page<T>, StoreError> {
        self.reader().page(space, prefix, after, limit, map)
    }
}

fn commit_id(id: narrata_history::ObjectId) -> CommitId {
    CommitId::from_bytes(*id.as_bytes())
}

fn ref_value(value: narrata_history::RefValue) -> RefValue {
    RefValue {
        revision: value.revision,
        commit: commit_id(value.commit),
    }
}

fn catalog_value(entry: &KeyEntry) -> Result<(CatalogRefKey, CatalogHeadRefValue), StoreError> {
    let (event, coverage) = layout::decode_catalog_head(&entry.value)?;
    Ok((
        layout::decode_catalog_key(&entry.key)?,
        CatalogHeadRefValue {
            revision: RefRevision::from_revision(entry.revision),
            event,
            coverage,
        },
    ))
}

fn archive_value(
    entry: &KeyEntry,
) -> Result<(TimelineArchiveRefKey, TimelineArchiveRefValue), StoreError> {
    Ok((
        layout::decode_archive_key(&entry.key)?,
        TimelineArchiveRefValue {
            revision: RefRevision::from_revision(entry.revision),
            manifest: layout::decode_archive(&entry.value)?,
        },
    ))
}

fn compound_save_value(
    entry: &KeyEntry,
) -> Result<(CompoundSaveRefKey, CompoundSaveRefValue), StoreError> {
    Ok((
        layout::decode_compound_save_key(&entry.key)?,
        CompoundSaveRefValue {
            revision: RefRevision::from_revision(entry.revision),
            manifest: layout::decode_compound_save(&entry.value)?,
        },
    ))
}

fn entry(key: Vec<u8>, value: KeyValue) -> KeyEntry {
    KeyEntry {
        key,
        value: value.value,
        revision: value.revision,
    }
}

impl<B: StorageBackend> SaveStore for Store<B> {
    fn get_object(&self, id: ObjectId) -> Result<Option<CheckedObject>, StoreError> {
        Ok(self.get_objects(&[id])?.into_iter().next().flatten())
    }

    fn get_objects(&self, ids: &[ObjectId]) -> Result<Vec<Option<CheckedObject>>, StoreError> {
        let ids = ids
            .iter()
            .map(|id| history_id(id.as_bytes()))
            .collect::<Vec<_>>();
        self.history
            .get_objects(&ids)?
            .into_iter()
            .map(|object| object.map(CheckedObject::from_object).transpose())
            .collect()
    }

    fn find_program(&self, artifact: ProgramArtifactId) -> Result<Option<ObjectId>, StoreError> {
        Ok(self
            .read_key(layout::PROGRAMS, &layout::program_key(artifact))?
            .map(|value| layout::decode_program(&value.value).map(id))
            .transpose()?)
    }

    fn read_ref(&self, key: &RefKey) -> Result<Option<RefValue>, StoreError> {
        Ok(self.history.read_ref(key)?.map(ref_value))
    }

    fn scan_refs(
        &self,
        scope: &RefScope,
        after: Option<&RefKey>,
        limit: u32,
    ) -> Result<Page<(RefKey, RefValue)>, StoreError> {
        let page = self.history.scan_refs(scope, after, limit)?;
        Ok(Page {
            items: page
                .items
                .into_iter()
                .map(|(key, value)| (key, ref_value(value)))
                .collect(),
            more: page.more,
        })
    }

    fn read_catalog_head(
        &self,
        key: &CatalogRefKey,
    ) -> Result<Option<CatalogHeadRefValue>, StoreError> {
        let bytes = layout::catalog_key(key);
        self.read_key(layout::CATALOG_HEADS, &bytes)?
            .map(|value| catalog_value(&entry(bytes, value)).map(|(_, value)| value))
            .transpose()
    }

    fn scan_catalog_heads(
        &self,
        after: Option<&CatalogRefKey>,
        limit: u32,
    ) -> Result<Page<(CatalogRefKey, CatalogHeadRefValue)>, StoreError> {
        self.page(
            layout::CATALOG_HEADS,
            &[],
            after.map(layout::catalog_key),
            limit,
            catalog_value,
        )
    }

    fn read_timeline_archive(
        &self,
        key: &TimelineArchiveRefKey,
    ) -> Result<Option<TimelineArchiveRefValue>, StoreError> {
        let bytes = layout::archive_key(key);
        self.read_key(layout::ARCHIVES, &bytes)?
            .map(|value| archive_value(&entry(bytes, value)).map(|(_, value)| value))
            .transpose()
    }

    fn scan_timeline_archives(
        &self,
        timeline: Option<ExecutionId>,
        after: Option<&TimelineArchiveRefKey>,
        limit: u32,
    ) -> Result<Page<(TimelineArchiveRefKey, TimelineArchiveRefValue)>, StoreError> {
        let prefix = timeline.map_or_else(Vec::new, |timeline| timeline.as_bytes().to_vec());
        self.page(
            layout::ARCHIVES,
            &prefix,
            after.map(layout::archive_key),
            limit,
            archive_value,
        )
    }

    fn read_compound_save(
        &self,
        key: &CompoundSaveRefKey,
    ) -> Result<Option<CompoundSaveRefValue>, StoreError> {
        let bytes = layout::compound_save_key(key);
        self.read_key(layout::COMPOUND_SAVES, &bytes)?
            .map(|value| compound_save_value(&entry(bytes, value)).map(|(_, value)| value))
            .transpose()
    }

    fn scan_compound_saves(
        &self,
        owner: Option<&RefName>,
        after: Option<&CompoundSaveRefKey>,
        limit: u32,
    ) -> Result<Page<(CompoundSaveRefKey, CompoundSaveRefValue)>, StoreError> {
        self.page(
            layout::COMPOUND_SAVES,
            &layout::compound_save_prefix(owner),
            after.map(layout::compound_save_key),
            limit,
            compound_save_value,
        )
    }

    fn read_input(
        &self,
        execution: ExecutionId,
        input: InputId,
    ) -> Result<Option<InputRecord>, StoreError> {
        self.read_key(layout::INPUTS, &layout::input_key(execution, input))?
            .map(|value| {
                let (parent, payload, commit) = layout::decode_input(&value.value)?;
                Ok(InputRecord {
                    execution,
                    input,
                    parent,
                    payload,
                    commit,
                })
            })
            .transpose()
    }

    fn scan_pins(
        &self,
        after: Option<&(String, ObjectId)>,
        limit: u32,
    ) -> Result<Page<Pin>, StoreError> {
        let after = after.map(|(owner, object)| (owner.clone(), history_id(object.as_bytes())));
        let page = self.history.scan_pins(after.as_ref(), limit)?;
        Ok(Page {
            items: page
                .items
                .into_iter()
                .map(|pin| Pin {
                    owner: pin.owner,
                    object: id(pin.object),
                    expires_at: pin.expires_at,
                })
                .collect(),
            more: page.more,
        })
    }

    fn read_effect(
        &self,
        execution: ExecutionId,
        effect: EffectId,
    ) -> Result<Option<EffectLedgerEntry>, StoreError> {
        Ok(self.read_entry(execution, effect)?.map(|(entry, _)| entry))
    }

    fn scan_effects(
        &self,
        execution: ExecutionId,
        after: Option<&EffectId>,
        limit: u32,
    ) -> Result<Page<EffectLedgerEntry>, StoreError> {
        self.page(
            layout::EFFECTS,
            execution.as_bytes(),
            after.map(|effect| layout::effect_key(execution, *effect)),
            limit,
            |entry| Ok(layout::decode_effect(&entry.key, &entry.value)?),
        )
    }

    fn current_ledger_fence(&self, execution: ExecutionId) -> Result<LedgerFence, StoreError> {
        Ok(self
            .read_fence(execution)?
            .map_or_else(LedgerFence::zero, |(fence, _)| fence))
    }

    fn child_commits(
        &self,
        parent: CommitId,
        after: Option<&CommitId>,
        limit: u32,
    ) -> Result<Page<CommitId>, StoreError> {
        let after = after.map(|child| history_id(child.as_bytes()));
        let page = self
            .history
            .children(history_id(parent.as_bytes()), after.as_ref(), limit)?;
        Ok(Page {
            items: page.items.into_iter().map(commit_id).collect(),
            more: page.more,
        })
    }

    fn timeline_commits(
        &self,
        execution: ExecutionId,
        after: Option<&(u64, CommitId)>,
        limit: u32,
    ) -> Result<Page<(u64, CommitId)>, StoreError> {
        self.page(
            layout::COMMITS,
            execution.as_bytes(),
            after.map(|(turn, commit)| layout::commit_key(execution, *turn, *commit)),
            limit,
            |entry| {
                narrata_history::layout::decode_marker("commit index value", &entry.value)?;
                let (_, turn, commit) = layout::decode_commit_key(&entry.key)?;
                Ok((turn, commit))
            },
        )
    }

    fn claim_effect(&mut self, claim: EffectClaim) -> Result<EffectClaimResult, StoreError> {
        self.claim(claim)
    }

    fn renew_effect_lease(
        &mut self,
        execution: ExecutionId,
        effect: EffectId,
        lease: LeaseId,
        now: u64,
        expires_at: u64,
    ) -> Result<EffectLedgerEntry, StoreError> {
        self.renew(execution, effect, lease, now, expires_at)
    }

    fn record_effect_outcome(
        &mut self,
        record: EffectOutcomeRecord,
    ) -> Result<EffectLedgerEntry, StoreError> {
        self.record_outcome(record)
    }

    fn mark_effect_compensated(
        &mut self,
        execution: ExecutionId,
        original: EffectId,
        by_effect: EffectId,
    ) -> Result<EffectLedgerEntry, StoreError> {
        self.compensate(execution, original, by_effect)
    }

    fn commit(&mut self, transaction: CommitTransaction) -> Result<CommitOutcome, StoreError> {
        self.write_transaction(&transaction)
    }

    fn collect(&mut self, policy: RetentionPolicy) -> Result<GcReport, StoreError> {
        self.history.collect(policy)
    }

    fn integrity_scan(&self) -> Result<Vec<IntegrityIssue>, StoreError> {
        Ok(self
            .history
            .integrity_scan()?
            .into_iter()
            .map(|issue| IntegrityIssue {
                object: id(issue.object),
                diagnostic: issue.diagnostic,
            })
            .collect())
    }
}
