//! The save engine: one [`SaveStore`] implementation over any [`StorageBackend`] (ADR 0014).
//!
//! Domain state lives in the key spaces of [`crate::layout`]. Every object read is re-verified
//! against its identity and every key value is decoded strictly, so a backend can lose data but
//! cannot make the engine accept data it did not write.

mod effects;
mod gc;
mod validate;
mod write;

use std::{collections::BTreeMap, fmt, sync::Arc};

use narrata_core::{
    CheckedProgram, CommitId, EffectId, ExecutionId, InputId, ObjectId, ProgramArtifactId,
    codec::{ObjectKind, inspect_envelope},
    limits::DecodeLimits,
};
use narrata_storage::{
    Batch, Expect, KeyEntry, KeyPage, KeySpace, KeyValue, Limits, MemoryBackend, ObjectDigest,
    Revision, StorageBackend, StorageError,
};

use crate::{
    CatalogHeadRefValue, CatalogRefKey, CheckedObject, CommitOutcome, CommitTransaction,
    CompoundSaveRefKey, CompoundSaveRefValue, EffectClaim, EffectClaimResult, EffectLedgerEntry,
    EffectOutcomeRecord, GcReport, InputRecord, IntegrityIssue, LeaseId, LedgerFence, Page, Pin,
    RefKey, RefName, RefRevision, RefScope, RefValue, RetentionPolicy, SaveStore, StoreContents,
    StoreError, TimelineArchiveRefKey, TimelineArchiveRefValue, layout, object_id,
};

/// Attempts of one domain operation before it reports [`StoreError::Busy`].
const RETRIES: usize = 8;
/// Loaded Programs kept per store; Programs are few and large.
const PROGRAM_CACHE: usize = 32;

/// A [`SaveStore`] over the storage backend `B`.
pub struct Store<B> {
    backend: B,
    limits: Limits,
    programs: ProgramCache,
}

/// The engine over the in-memory reference backend.
pub type MemoryStore = Store<MemoryBackend>;

/// Programs decoded from verified object bytes, keyed by object identity. An entry stays valid
/// whether or not the object is still stored; presence is always read from the backend.
#[derive(Default)]
struct ProgramCache(BTreeMap<ObjectId, Option<Arc<CheckedProgram>>>);

impl ProgramCache {
    fn load(&mut self, object: &CheckedObject) -> Option<Arc<CheckedProgram>> {
        if let Some(program) = self.0.get(&object.id()) {
            return program.clone();
        }
        let program = (object.kind() == ObjectKind::Program && object.schema() == 0)
            .then(|| narrata_core::program::load_program(object.bytes(), &Default::default()).ok())
            .flatten();
        if self.0.len() >= PROGRAM_CACHE {
            self.0.clear();
        }
        self.0.insert(object.id(), program.clone());
        program
    }
}

impl Store<MemoryBackend> {
    pub fn new() -> Self {
        let backend = MemoryBackend::new();
        // Opening a fresh in-memory backend cannot fail; if it ever did, the store would still
        // work and only lack its layout marker.
        Self::open(backend.share()).unwrap_or_else(|_| Self::assemble(backend))
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
            .field("backend", &self.backend)
            .finish_non_exhaustive()
    }
}

impl<B: StorageBackend> Store<B> {
    /// Opens a store in layout v1, initialising an empty backend. A backend holding anything
    /// else, including a newer layout, is refused with a format error and left untouched.
    pub fn open(mut backend: B) -> Result<Self, StoreError> {
        let capabilities = backend.capabilities();
        if !capabilities.atomic_multi_key() {
            return Err(
                StorageError::Invalid("the save engine needs atomic multi-key batches").into(),
            );
        }
        for _ in 0..RETRIES {
            match backend.read_key(layout::META, layout::LAYOUT_KEY)? {
                Some(value) => {
                    let version = layout::decode_layout(&value.value)?;
                    if version != layout::LAYOUT_VERSION {
                        return Err(StorageError::Format(format!(
                            "save layout version {version} is not the supported {}",
                            layout::LAYOUT_VERSION
                        ))
                        .into());
                    }
                    return Ok(Self::assemble(backend));
                }
                None if is_empty(&backend)? => {
                    let marker = Batch::new().put(
                        layout::META,
                        layout::LAYOUT_KEY,
                        layout::encode_layout(),
                        Expect::Absent,
                    );
                    match backend.apply(&marker) {
                        Ok(_) | Err(StorageError::Conflict(_)) => {}
                        Err(error) if error.outcome_unknown() => {}
                        Err(error) => return Err(error.into()),
                    }
                }
                None => {
                    return Err(StorageError::Format(
                        "backend holds data but no save layout marker".to_owned(),
                    )
                    .into());
                }
            }
        }
        Err(StoreError::Busy)
    }

    /// Writes `contents` into an empty backend and opens it. Objects keep their bytes and
    /// observation times; revisions are new.
    pub fn restore(backend: B, contents: StoreContents) -> Result<Self, StoreError> {
        let mut store = Self::open(backend)?;
        if !is_empty_except_meta(&store.backend)? {
            return Err(StorageError::Invalid("restore needs an empty store").into());
        }
        store.write_contents(contents)?;
        Ok(store)
    }

    pub fn backend(&self) -> &B {
        &self.backend
    }

    pub fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }

    pub fn into_backend(self) -> B {
        self.backend
    }

    fn assemble(backend: B) -> Self {
        let limits = backend.capabilities().limits;
        Self {
            backend,
            limits,
            programs: ProgramCache::default(),
        }
    }

    fn read_items(&self) -> u32 {
        u32::try_from(self.limits.max_read_items)
            .unwrap_or(u32::MAX)
            .max(1)
    }

    fn page_limit(&self, limit: u32) -> u32 {
        limit.clamp(1, self.read_items())
    }

    fn read_key(&self, space: KeySpace, key: &[u8]) -> Result<Option<KeyValue>, StoreError> {
        Ok(self.backend.read_key(space, key)?)
    }

    fn load_objects(&self, ids: &[ObjectId]) -> Result<Vec<Option<CheckedObject>>, StoreError> {
        load_objects(&self.backend, self.read_items(), ids)
    }

    fn load_object(&self, id: ObjectId) -> Result<Option<CheckedObject>, StoreError> {
        Ok(self.load_objects(&[id])?.into_iter().next().flatten())
    }

    /// Reads a stored object of the given kind; the error names the first missing or mismatched
    /// object as validation always has.
    fn require_stored(&self, id: ObjectId, kind: ObjectKind) -> Result<CheckedObject, StoreError> {
        let object = self.load_object(id)?.ok_or(StoreError::MissingObject(id))?;
        if object.kind() == kind {
            Ok(object)
        } else {
            Err(StoreError::ObjectKind(id))
        }
    }

    /// The revision of `meta/sweep` that writers check, so a GC deletion between their reads
    /// and their batch makes the batch conflict.
    fn sweep(&self) -> Result<Option<Revision>, StoreError> {
        self.read_key(layout::META, layout::SWEEP_KEY)?
            .map(|value| {
                layout::decode_marker("sweep marker", &value.value)?;
                Ok(value.revision)
            })
            .transpose()
    }

    fn page<T>(
        &self,
        space: KeySpace,
        prefix: &[u8],
        after: Option<Vec<u8>>,
        limit: u32,
        map: impl Fn(&KeyEntry) -> Result<T, StoreError>,
    ) -> Result<Page<T>, StoreError> {
        let page = scan_keys(
            &self.backend,
            space,
            prefix,
            after.as_deref(),
            self.page_limit(limit),
        )?;
        Ok(Page {
            items: page.entries.iter().map(map).collect::<Result<_, _>>()?,
            more: page.more,
        })
    }

    fn scan_every(&self, space: KeySpace, prefix: &[u8]) -> Result<Vec<KeyEntry>, StoreError> {
        scan_every(&self.backend, self.read_items(), space, prefix)
    }
}

fn is_empty<B: StorageBackend>(backend: &B) -> Result<bool, StoreError> {
    Ok(backend
        .read_key(layout::META, layout::LAYOUT_KEY)?
        .is_none()
        && is_empty_except_meta(backend)?
        && backend
            .scan_keys(layout::META, &[], None, 1)?
            .entries
            .is_empty())
}

fn is_empty_except_meta<B: StorageBackend>(backend: &B) -> Result<bool, StoreError> {
    if !backend.scan_objects(None, 1)?.digests.is_empty() {
        return Ok(false);
    }
    for (space, _) in &layout::SPACES[1..] {
        if !backend.scan_keys(*space, &[], None, 1)?.entries.is_empty() {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(crate) const fn digest(id: ObjectId) -> ObjectDigest {
    ObjectDigest::from_bytes(*id.as_bytes())
}

/// Rebuilds a checked object from backend bytes and refuses bytes that do not hash to `id`.
pub(crate) fn verify(id: ObjectId, bytes: Arc<[u8]>) -> Result<CheckedObject, StoreError> {
    let length = bytes.len() as u64;
    let limits = DecodeLimits {
        max_envelope_bytes: length,
        max_payload_bytes: length,
        ..DecodeLimits::default()
    };
    let envelope = inspect_envelope(&bytes, &limits)
        .map_err(|error| StoreError::Corrupt(id, error.to_string()))?;
    let actual = object_id(envelope.kind, envelope.schema_version, envelope.payload);
    if actual != id {
        return Err(StoreError::Corrupt(id, "ObjectId mismatch".to_owned()));
    }
    let (kind, schema) = (envelope.kind, envelope.schema_version);
    Ok(CheckedObject::from_verified(id, kind, schema, bytes))
}

pub(crate) fn load_objects<B: StorageBackend>(
    backend: &B,
    read_items: u32,
    ids: &[ObjectId],
) -> Result<Vec<Option<CheckedObject>>, StoreError> {
    let mut objects = Vec::with_capacity(ids.len());
    for chunk in ids.chunks(read_items as usize) {
        let digests = chunk.iter().copied().map(digest).collect::<Vec<_>>();
        let found = backend.get_objects(&digests)?;
        if found.len() != chunk.len() {
            return Err(StoreError::CorruptStore(
                "backend returned a misaligned object read".to_owned(),
            ));
        }
        for (id, bytes) in chunk.iter().zip(found) {
            objects.push(bytes.map(|bytes| verify(*id, bytes)).transpose()?);
        }
    }
    Ok(objects)
}

pub(crate) fn scan_keys<B: StorageBackend>(
    backend: &B,
    space: KeySpace,
    prefix: &[u8],
    after: Option<&[u8]>,
    limit: u32,
) -> Result<KeyPage, StoreError> {
    let page = backend.scan_keys(space, prefix, after, limit)?;
    let ordered = page
        .entries
        .windows(2)
        .all(|pair| pair[0].key < pair[1].key);
    let bounded = page.entries.iter().all(|entry| {
        entry.key.starts_with(prefix) && after.is_none_or(|after| entry.key.as_slice() > after)
    });
    if !ordered || !bounded || page.entries.len() > limit as usize {
        return Err(StoreError::CorruptStore(
            "backend returned keys outside the requested scan".to_owned(),
        ));
    }
    Ok(page)
}

pub(crate) fn scan_every<B: StorageBackend>(
    backend: &B,
    read_items: u32,
    space: KeySpace,
    prefix: &[u8],
) -> Result<Vec<KeyEntry>, StoreError> {
    let mut entries = Vec::new();
    let mut after: Option<Vec<u8>> = None;
    loop {
        let page = scan_keys(backend, space, prefix, after.as_deref(), read_items)?;
        let resume = page.resume_after().map(<[u8]>::to_vec);
        entries.extend(page.entries);
        match resume {
            Some(key) => after = Some(key),
            None => return Ok(entries),
        }
    }
}

fn ref_value(entry: &KeyEntry) -> Result<(RefKey, RefValue), StoreError> {
    Ok((
        layout::decode_ref_key(&entry.key)?,
        RefValue {
            revision: RefRevision::from_revision(entry.revision),
            commit: layout::decode_commit_target(&entry.value)?,
        },
    ))
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
        self.load_object(id)
    }

    fn get_objects(&self, ids: &[ObjectId]) -> Result<Vec<Option<CheckedObject>>, StoreError> {
        self.load_objects(ids)
    }

    fn find_program(&self, artifact: ProgramArtifactId) -> Result<Option<ObjectId>, StoreError> {
        self.read_key(layout::PROGRAMS, &layout::program_key(artifact))?
            .map(|value| layout::decode_program(&value.value))
            .transpose()
    }

    fn read_ref(&self, key: &RefKey) -> Result<Option<RefValue>, StoreError> {
        let bytes = layout::ref_key(key);
        self.read_key(layout::REFS, &bytes)?
            .map(|value| ref_value(&entry(bytes, value)).map(|(_, value)| value))
            .transpose()
    }

    fn scan_refs(
        &self,
        scope: &RefScope,
        after: Option<&RefKey>,
        limit: u32,
    ) -> Result<Page<(RefKey, RefValue)>, StoreError> {
        let prefix = match scope {
            RefScope::All => layout::ref_prefix(None, None),
            RefScope::Namespace(namespace) => layout::ref_prefix(Some(*namespace), None),
            RefScope::Owner(namespace, owner) => layout::ref_prefix(Some(*namespace), Some(owner)),
        };
        self.page(
            layout::REFS,
            &prefix,
            after.map(layout::ref_key),
            limit,
            ref_value,
        )
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
        let after = after
            .map(|(owner, object)| layout::pin_key(owner, *object))
            .transpose()?;
        self.page(layout::PINS, &[], after, limit, |entry| {
            layout::decode_pin(&entry.key, &entry.value)
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
            |entry| layout::decode_effect(&entry.key, &entry.value),
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
        self.page(
            layout::CHILDREN,
            parent.as_bytes(),
            after.map(|child| layout::child_key(parent, *child)),
            limit,
            |entry| {
                layout::decode_marker("child index value", &entry.value)?;
                Ok(layout::decode_child_key(&entry.key)?.1)
            },
        )
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
                layout::decode_marker("commit index value", &entry.value)?;
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
        self.collect_garbage(policy)
    }

    fn integrity_scan(&self) -> Result<Vec<IntegrityIssue>, StoreError> {
        self.scan_integrity()
    }
}
