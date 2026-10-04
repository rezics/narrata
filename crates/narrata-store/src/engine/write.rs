//! Commit planning and batch application (ADR 0014).
//!
//! A write is planned from the store's current state and applied as one batch whose
//! preconditions encode everything the plan read: the CAS revisions the caller expects, absent
//! index keys and the `meta/sweep` revision. A conflict on a caller's key is the caller's
//! conflict; a conflict elsewhere means the plan read stale state, so the write is planned again.

use std::collections::{BTreeMap, BTreeSet};

use narrata_core::{
    CommitId, CompoundSaveManifestId, ExecutionId, InputId, ObjectId, TimelineArchiveManifestId,
    TimelineCatalogEventId, codec::ObjectKind,
};
use narrata_storage::{
    Applied, Batch, Conflict, Expect, KeyAction, KeyOp, KeySpace, Revision, StorageBackend,
    StorageError,
};

use super::{RETRIES, Store, digest, validate::View};
use crate::{
    ArchiveConflict, CatalogConflict, CatalogHeadRefValue, CatalogMutation, CatalogRefKey,
    CheckedObject, CommitOutcome, CommitTransaction, CompoundSaveConflict, CompoundSaveRefKey,
    CompoundSaveRefValue, EffectLedgerEntry, InputIdConflict, InputRecord, LedgerFence,
    RefConflict, RefKey, RefMutation, RefRevision, RefValue, StoreContents, StoreError,
    TimelineArchiveRefKey, TimelineArchiveRefValue, TimelineCoverage,
    graph::{Reference, references},
    layout,
};

/// What a key operation's precondition failure means.
#[derive(Clone, Debug)]
pub(super) enum Tag {
    /// The plan read state that changed since: plan again.
    Replan,
    /// An unconditional write, which cannot conflict.
    Index,
    Ref {
        key: RefKey,
        expected: Option<RefRevision>,
        proposed: Option<CommitId>,
    },
    Catalog {
        key: CatalogRefKey,
        expected: Option<RefRevision>,
        proposed: Option<TimelineCatalogEventId>,
    },
    Archive {
        key: TimelineArchiveRefKey,
        expected: Option<RefRevision>,
        proposed: Option<TimelineArchiveManifestId>,
    },
    CompoundSave {
        key: CompoundSaveRefKey,
        expected: Option<RefRevision>,
        proposed: Option<CompoundSaveManifestId>,
    },
}

pub(super) struct Op {
    op: KeyOp,
    tag: Tag,
}

impl Op {
    pub(super) fn put(
        space: KeySpace,
        key: Vec<u8>,
        value: Vec<u8>,
        expect: Expect,
        tag: Tag,
    ) -> Self {
        Self::new(space, key, expect, KeyAction::Put(value), tag)
    }

    pub(super) fn delete(space: KeySpace, key: Vec<u8>, expect: Expect, tag: Tag) -> Self {
        Self::new(space, key, expect, KeyAction::Delete, tag)
    }

    pub(super) fn check(space: KeySpace, key: Vec<u8>, expect: Expect, tag: Tag) -> Self {
        Self::new(space, key, expect, KeyAction::Check, tag)
    }

    fn new(space: KeySpace, key: Vec<u8>, expect: Expect, action: KeyAction, tag: Tag) -> Self {
        Self {
            op: KeyOp {
                space,
                key,
                expect,
                action,
            },
            tag,
        }
    }

    pub(super) fn bytes(&self) -> u64 {
        let value = match &self.op.action {
            KeyAction::Put(value) => value.len(),
            KeyAction::Delete | KeyAction::Check => 0,
        };
        (self.op.key.len() + value) as u64
    }
}

/// Why one batch did not apply.
pub(super) enum Attempt {
    Retry,
    Fail(StoreError),
}

impl From<StoreError> for Attempt {
    fn from(value: StoreError) -> Self {
        Self::Fail(value)
    }
}

/// Checks that no GC deletion ran since the plan read `sweep`.
pub(super) fn sweep_check(sweep: Option<Revision>) -> Op {
    Op::check(
        layout::META,
        layout::SWEEP_KEY.to_vec(),
        sweep.map_or(Expect::Absent, Expect::Revision),
        Tag::Replan,
    )
}

/// Tells a running GC that objects or roots were added after it marked.
pub(super) fn graph_bump() -> Op {
    Op::put(
        layout::META,
        layout::GRAPH_KEY.to_vec(),
        layout::encode_marker(),
        Expect::Any,
        Tag::Index,
    )
}

pub(super) fn expect(expected: Option<RefRevision>) -> Expect {
    expected
        .and_then(RefRevision::revision)
        .map_or(Expect::Absent, Expect::Revision)
}

fn oid(bytes: &[u8; 32]) -> ObjectId {
    ObjectId::from_bytes(*bytes)
}

/// A batch under construction, with the tag of each key operation.
#[derive(Default)]
pub(super) struct Ops {
    objects: Vec<CheckedObject>,
    deletes: Vec<ObjectId>,
    keys: Vec<Op>,
}

impl Ops {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) fn object(&mut self, object: CheckedObject) {
        self.objects.push(object);
    }

    pub(super) fn delete_object(&mut self, id: ObjectId) {
        self.deletes.push(id);
    }

    pub(super) fn key(&mut self, op: Op) {
        self.keys.push(op);
    }
}

struct Staged {
    object: CheckedObject,
    observed_at: u64,
    index: Vec<Op>,
}

impl Staged {
    fn ops(&self) -> u64 {
        2 + self.index.len() as u64
    }

    fn bytes(&self) -> u64 {
        self.object.bytes().len() as u64 + 48 + self.index.iter().map(Op::bytes).sum::<u64>()
    }

    fn into_ops(self, ops: &mut Ops) {
        ops.key(Op::put(
            layout::TOUCH,
            layout::touch_key(self.object.id()),
            layout::encode_touch(self.observed_at),
            Expect::Any,
            Tag::Index,
        ));
        ops.keys.extend(self.index);
        ops.object(self.object);
    }
}

/// The roots a write sets or clears, for its outcome.
#[derive(Default)]
struct Roots {
    refs: Vec<(RefKey, Option<CommitId>)>,
    catalogs: Vec<(
        CatalogRefKey,
        Option<(TimelineCatalogEventId, TimelineCoverage)>,
    )>,
    archives: Vec<(TimelineArchiveRefKey, Option<TimelineArchiveManifestId>)>,
    compound_saves: Vec<(CompoundSaveRefKey, Option<CompoundSaveManifestId>)>,
}

impl Roots {
    fn outcome(
        self,
        revision: Option<Revision>,
        inserted: u64,
    ) -> Result<CommitOutcome, StoreError> {
        let revision = || {
            revision.map(RefRevision::from_revision).ok_or_else(|| {
                StoreError::CorruptStore(
                    "backend applied root writes without a revision".to_owned(),
                )
            })
        };
        let mut outcome = CommitOutcome {
            inserted_objects: usize::try_from(inserted).unwrap_or(usize::MAX),
            ..CommitOutcome::default()
        };
        for (key, commit) in self.refs {
            let value = commit
                .map(|commit| {
                    Ok::<_, StoreError>(RefValue {
                        revision: revision()?,
                        commit,
                    })
                })
                .transpose()?;
            outcome.refs.insert(key, value);
        }
        for (key, head) in self.catalogs {
            let value = head
                .map(|(event, coverage)| {
                    Ok::<_, StoreError>(CatalogHeadRefValue {
                        revision: revision()?,
                        event,
                        coverage,
                    })
                })
                .transpose()?;
            outcome.catalogs.insert(key, value);
        }
        for (key, manifest) in self.archives {
            let value = manifest
                .map(|manifest| {
                    Ok::<_, StoreError>(TimelineArchiveRefValue {
                        revision: revision()?,
                        manifest,
                    })
                })
                .transpose()?;
            outcome.archives.insert(key, value);
        }
        for (key, manifest) in self.compound_saves {
            let value = manifest
                .map(|manifest| {
                    Ok::<_, StoreError>(CompoundSaveRefValue {
                        revision: revision()?,
                        manifest,
                    })
                })
                .transpose()?;
            outcome.compound_saves.insert(key, value);
        }
        Ok(outcome)
    }
}

struct Plan {
    sweep: Option<Revision>,
    /// Referenced objects before the objects that refer to them, so that every prefix written
    /// on its own keeps each stored object's references stored.
    objects: Vec<Staged>,
    keys: Vec<Op>,
    roots: Roots,
}

struct Request<'t> {
    transaction: &'t CommitTransaction,
    /// Per-object observation times; the transaction's time otherwise.
    observed: Option<&'t [u64]>,
    effects: &'t [EffectLedgerEntry],
    fences: &'t [(ExecutionId, LedgerFence)],
}

/// Orders `staged` so that every object follows the staged objects it refers to.
fn referents_first(
    staged: &BTreeMap<ObjectId, CheckedObject>,
    program: impl Fn(narrata_core::ProgramArtifactId) -> Option<ObjectId>,
) -> Result<Vec<ObjectId>, StoreError> {
    let mut targets = BTreeMap::new();
    for (id, object) in staged {
        let mut values = Vec::new();
        for reference in references(object)? {
            let target = match reference {
                Reference::Object { id, .. } => Some(id),
                Reference::Program(artifact) => program(artifact),
            };
            values.extend(target.filter(|target| staged.contains_key(target)));
        }
        targets.insert(*id, values);
    }
    let mut order = Vec::with_capacity(staged.len());
    let mut done = BTreeSet::new();
    let mut visiting = BTreeSet::new();
    for start in staged.keys() {
        let mut stack = vec![(*start, false)];
        while let Some((id, expanded)) = stack.pop() {
            if done.contains(&id) {
                continue;
            }
            if expanded {
                done.insert(id);
                order.push(id);
                continue;
            }
            if !visiting.insert(id) {
                continue;
            }
            stack.push((id, true));
            for target in targets.get(&id).into_iter().flatten() {
                if !done.contains(target) && !visiting.contains(target) {
                    stack.push((*target, false));
                }
            }
        }
    }
    Ok(order)
}

impl<B: StorageBackend> Store<B> {
    pub(super) fn write_transaction(
        &mut self,
        transaction: &CommitTransaction,
    ) -> Result<CommitOutcome, StoreError> {
        self.write(&Request {
            transaction,
            observed: None,
            effects: &[],
            fences: &[],
        })
    }

    pub(super) fn write_contents(&mut self, contents: StoreContents) -> Result<(), StoreError> {
        let observed = contents
            .objects
            .iter()
            .map(|(_, observed_at)| *observed_at)
            .collect::<Vec<_>>();
        let transaction = CommitTransaction {
            objects: contents
                .objects
                .into_iter()
                .map(|(object, _)| object)
                .collect(),
            refs: contents
                .refs
                .into_iter()
                .map(|(key, commit)| RefMutation {
                    key,
                    expected: None,
                    next: Some(commit),
                })
                .collect(),
            catalogs: contents
                .catalogs
                .into_iter()
                .map(|(key, event, coverage)| CatalogMutation {
                    key,
                    expected: None,
                    next: Some(event),
                    coverage,
                })
                .collect(),
            archives: contents
                .archives
                .into_iter()
                .map(|(key, manifest)| crate::ArchiveMutation {
                    key,
                    expected: None,
                    next: Some(manifest),
                })
                .collect(),
            compound_saves: contents
                .compound_saves
                .into_iter()
                .map(|(key, manifest)| crate::CompoundSaveMutation {
                    key,
                    expected: None,
                    next: Some(manifest),
                })
                .collect(),
            inputs: contents.inputs,
            pins: contents.pins,
            ..CommitTransaction::default()
        };
        self.write(&Request {
            transaction: &transaction,
            observed: Some(&observed),
            effects: &contents.effects,
            fences: &contents.ledger_fences,
        })
        .map(|_| ())
    }

    fn write(&mut self, request: &Request<'_>) -> Result<CommitOutcome, StoreError> {
        for _ in 0..RETRIES {
            let sweep = self.sweep()?;
            let Plan {
                sweep,
                objects,
                keys,
                roots,
            } = self.plan(request, sweep)?;
            if objects.is_empty() && keys.is_empty() {
                return roots.outcome(None, 0);
            }
            match self.apply_plan(sweep, objects, keys) {
                Ok((revision, inserted)) => return roots.outcome(revision, inserted),
                Err(Attempt::Retry) => {}
                Err(Attempt::Fail(error)) => return Err(error),
            }
        }
        Err(StoreError::Busy)
    }

    fn plan(&mut self, request: &Request<'_>, sweep: Option<Revision>) -> Result<Plan, StoreError> {
        let transaction = request.transaction;
        let mut staged = BTreeMap::new();
        let mut observed = BTreeMap::new();
        for (index, object) in transaction.objects.iter().enumerate() {
            let observed_at = request
                .observed
                .and_then(|values| values.get(index))
                .copied()
                .unwrap_or(transaction.observed_at);
            observed.entry(object.id()).or_insert(observed_at);
            staged.entry(object.id()).or_insert_with(|| object.clone());
        }
        let read_items = self.read_items();
        let mut view = View::new(&self.backend, read_items, &mut self.programs, &staged);
        let order = referents_first(&staged, |artifact| view.staged_program(artifact))?;

        let mut wanted = Vec::new();
        for object in staged.values() {
            for reference in references(object)? {
                if let Reference::Object { id, .. } = reference {
                    wanted.push(id);
                }
            }
        }
        wanted.extend(
            transaction
                .refs
                .iter()
                .filter_map(|value| value.next)
                .map(|id| oid(id.as_bytes())),
        );
        wanted.extend(
            transaction
                .catalogs
                .iter()
                .filter_map(|value| value.next)
                .map(|id| oid(id.as_bytes())),
        );
        wanted.extend(
            transaction
                .archives
                .iter()
                .filter_map(|value| value.next)
                .map(|id| oid(id.as_bytes())),
        );
        wanted.extend(
            transaction
                .compound_saves
                .iter()
                .filter_map(|value| value.next)
                .map(|id| oid(id.as_bytes())),
        );
        wanted.extend(transaction.pins.iter().map(|pin| pin.object));
        view.prefetch(wanted)?;

        let mut objects = Vec::with_capacity(order.len());
        for id in order {
            let Some(object) = staged.get(&id) else {
                continue;
            };
            let index = view.validate(object)?;
            objects.push(Staged {
                object: object.clone(),
                observed_at: observed
                    .get(&id)
                    .copied()
                    .unwrap_or(transaction.observed_at),
                index,
            });
        }

        let mut keys = vec![];
        let mut inputs = BTreeMap::<(ExecutionId, InputId), InputRecord>::new();
        for record in &transaction.inputs {
            match inputs.get(&(record.execution, record.input)) {
                Some(existing) if existing == record => continue,
                Some(existing) => {
                    return Err(InputIdConflict {
                        existing: *existing,
                        proposed: *record,
                    }
                    .into());
                }
                None => {}
            }
            inputs.insert((record.execution, record.input), *record);
            let key = layout::input_key(record.execution, record.input);
            match view.backend().read_key(layout::INPUTS, &key)? {
                Some(value) => {
                    let (parent, payload, commit) = layout::decode_input(&value.value)?;
                    let existing = InputRecord {
                        execution: record.execution,
                        input: record.input,
                        parent,
                        payload,
                        commit,
                    };
                    if existing != *record {
                        return Err(InputIdConflict {
                            existing,
                            proposed: *record,
                        }
                        .into());
                    }
                }
                None => keys.push(Op::put(
                    layout::INPUTS,
                    key,
                    layout::encode_input(record.parent, record.payload, record.commit),
                    Expect::Absent,
                    Tag::Replan,
                )),
            }
        }

        let mut roots = Roots::default();
        let mut named = BTreeSet::new();
        for mutation in &transaction.refs {
            let key = layout::ref_key(&mutation.key);
            if !named.insert((layout::REFS, key.clone())) {
                return Err(StoreError::InvalidGraph(
                    "Ref named twice in one transaction",
                ));
            }
            let tag = Tag::Ref {
                key: mutation.key.clone(),
                expected: mutation.expected,
                proposed: mutation.next,
            };
            keys.push(match mutation.next {
                Some(commit) => {
                    view.require(oid(commit.as_bytes()), Some(ObjectKind::Commit))?;
                    Op::put(
                        layout::REFS,
                        key,
                        layout::encode_commit_target(commit),
                        expect(mutation.expected),
                        tag,
                    )
                }
                None => Op::delete(layout::REFS, key, expect(mutation.expected), tag),
            });
            roots.refs.push((mutation.key.clone(), mutation.next));
        }
        for mutation in &transaction.catalogs {
            let key = layout::catalog_key(&mutation.key);
            if !named.insert((layout::CATALOG_HEADS, key.clone())) {
                return Err(StoreError::InvalidGraph(
                    "Catalog Head named twice in one transaction",
                ));
            }
            let tag = Tag::Catalog {
                key: mutation.key.clone(),
                expected: mutation.expected,
                proposed: mutation.next,
            };
            keys.push(match mutation.next {
                Some(event) => {
                    view.require(
                        oid(event.as_bytes()),
                        Some(ObjectKind::TimelineCatalogEvent),
                    )?;
                    Op::put(
                        layout::CATALOG_HEADS,
                        key,
                        layout::encode_catalog_head(event, mutation.coverage),
                        expect(mutation.expected),
                        tag,
                    )
                }
                None => Op::delete(layout::CATALOG_HEADS, key, expect(mutation.expected), tag),
            });
            roots.catalogs.push((
                mutation.key.clone(),
                mutation.next.map(|event| (event, mutation.coverage)),
            ));
        }
        for mutation in &transaction.archives {
            let key = layout::archive_key(&mutation.key);
            if !named.insert((layout::ARCHIVES, key.clone())) {
                return Err(StoreError::InvalidGraph(
                    "archive named twice in one transaction",
                ));
            }
            let tag = Tag::Archive {
                key: mutation.key.clone(),
                expected: mutation.expected,
                proposed: mutation.next,
            };
            keys.push(match mutation.next {
                Some(manifest) => {
                    view.require(
                        oid(manifest.as_bytes()),
                        Some(ObjectKind::TimelineArchiveManifest),
                    )?;
                    Op::put(
                        layout::ARCHIVES,
                        key,
                        layout::encode_archive(manifest),
                        expect(mutation.expected),
                        tag,
                    )
                }
                None => Op::delete(layout::ARCHIVES, key, expect(mutation.expected), tag),
            });
            roots.archives.push((mutation.key.clone(), mutation.next));
        }
        for mutation in &transaction.compound_saves {
            let key = layout::compound_save_key(&mutation.key);
            if !named.insert((layout::COMPOUND_SAVES, key.clone())) {
                return Err(StoreError::InvalidGraph(
                    "Compound Save named twice in one transaction",
                ));
            }
            let tag = Tag::CompoundSave {
                key: mutation.key.clone(),
                expected: mutation.expected,
                proposed: mutation.next,
            };
            keys.push(match mutation.next {
                Some(manifest) => {
                    view.require(
                        oid(manifest.as_bytes()),
                        Some(ObjectKind::CompoundSaveManifest),
                    )?;
                    Op::put(
                        layout::COMPOUND_SAVES,
                        key,
                        layout::encode_compound_save(manifest),
                        expect(mutation.expected),
                        tag,
                    )
                }
                None => Op::delete(layout::COMPOUND_SAVES, key, expect(mutation.expected), tag),
            });
            roots
                .compound_saves
                .push((mutation.key.clone(), mutation.next));
        }

        // A Pin added and removed in one transaction ends removed; the last of repeated Pins wins.
        let removed = transaction
            .remove_pins
            .iter()
            .map(|(owner, object)| layout::pin_key(owner, *object))
            .collect::<Result<BTreeSet<_>, _>>()?;
        let mut pins = BTreeMap::new();
        for pin in &transaction.pins {
            view.require(pin.object, None)?;
            let key = layout::pin_key(&pin.owner, pin.object)?;
            if !removed.contains(&key) {
                pins.insert(key, pin.expires_at);
            }
        }
        for (key, expires_at) in pins {
            keys.push(Op::put(
                layout::PINS,
                key,
                layout::encode_pin(expires_at),
                Expect::Any,
                Tag::Index,
            ));
        }
        for key in removed {
            keys.push(Op::delete(layout::PINS, key, Expect::Any, Tag::Index));
        }

        for entry in request.effects {
            view.require(
                oid(entry.origin_commit.as_bytes()),
                Some(ObjectKind::Commit),
            )?;
            if let Some(response) = entry.status.response() {
                view.require(response, Some(ObjectKind::EffectResponse))?;
            }
            if entry.status.fence().is_some_and(|fence| fence.get() == 0) {
                return Err(StoreError::InvalidGraph("terminal ledger fence is zero"));
            }
            keys.push(Op::put(
                layout::EFFECTS,
                layout::effect_key(entry.execution, entry.effect),
                layout::encode_effect(entry),
                Expect::Absent,
                Tag::Replan,
            ));
        }
        for (execution, fence) in request.fences {
            keys.push(Op::put(
                layout::LEDGER_FENCES,
                layout::fence_key(*execution),
                layout::encode_fence(*fence),
                Expect::Absent,
                Tag::Replan,
            ));
        }
        Ok(Plan {
            sweep,
            objects,
            keys,
            roots,
        })
    }

    /// Applies a plan as one batch, or, when it exceeds the backend's batch limits, as object
    /// batches in reference order followed by one batch with every other key.
    fn apply_plan(
        &mut self,
        sweep: Option<Revision>,
        objects: Vec<Staged>,
        keys: Vec<Op>,
    ) -> Result<(Option<Revision>, u64), Attempt> {
        let limits = self.limits;
        let fits =
            |ops: u64, bytes: u64| ops <= limits.max_batch_ops && bytes <= limits.max_batch_bytes;
        let fence_ops = 2;
        let fence_bytes = 64;
        let total_ops =
            fence_ops + objects.iter().map(Staged::ops).sum::<u64>() + keys.len() as u64;
        let total_bytes = fence_bytes
            + objects.iter().map(Staged::bytes).sum::<u64>()
            + keys.iter().map(Op::bytes).sum::<u64>();
        let mut inserted = 0;
        let mut pending = Vec::new();
        if !fits(total_ops, total_bytes) {
            let (mut ops, mut bytes) = (fence_ops, fence_bytes);
            for object in objects {
                if !pending.is_empty() && !fits(ops + object.ops(), bytes + object.bytes()) {
                    inserted += self
                        .apply_objects(sweep, std::mem::take(&mut pending))?
                        .objects_inserted;
                    (ops, bytes) = (fence_ops, fence_bytes);
                }
                ops += object.ops();
                bytes += object.bytes();
                pending.push(object);
            }
            if !pending.is_empty()
                && !fits(
                    ops + keys.len() as u64,
                    bytes + keys.iter().map(Op::bytes).sum::<u64>(),
                )
            {
                inserted += self
                    .apply_objects(sweep, std::mem::take(&mut pending))?
                    .objects_inserted;
            }
        } else {
            pending = objects;
        }
        let mut batch = Ops::new();
        batch.key(sweep_check(sweep));
        batch.keys.extend(keys);
        for object in pending {
            object.into_ops(&mut batch);
        }
        batch.key(graph_bump());
        let applied = self.apply_ops(batch)?;
        Ok((applied.revision, inserted + applied.objects_inserted))
    }

    fn apply_objects(
        &mut self,
        sweep: Option<Revision>,
        objects: Vec<Staged>,
    ) -> Result<Applied, Attempt> {
        let mut batch = Ops::new();
        batch.key(sweep_check(sweep));
        for object in objects {
            object.into_ops(&mut batch);
        }
        batch.key(graph_bump());
        self.apply_ops(batch)
    }

    /// Applies one batch. A conflict is mapped through its key's tag; an unknown outcome is
    /// reconciled by reading back what the batch would have written.
    pub(super) fn apply_ops(&mut self, ops: Ops) -> Result<Applied, Attempt> {
        let Ops {
            objects,
            deletes,
            keys,
        } = ops;
        let (keys, tags): (Vec<_>, Vec<_>) = keys.into_iter().map(|op| (op.op, op.tag)).unzip();
        let batch = Batch {
            put_objects: objects
                .iter()
                .map(|object| (digest(object.id()), object.shared_bytes()))
                .collect(),
            delete_objects: deletes.into_iter().map(digest).collect(),
            keys,
        };
        match self.backend.apply(&batch) {
            Ok(applied) => Ok(applied),
            Err(StorageError::Conflict(conflict)) => Err(tags
                .get(conflict.index)
                .map_or(Attempt::Retry, |tag| conflict_error(tag, *conflict))),
            Err(error) if error.outcome_unknown() => match self.reconcile(&batch) {
                Ok(Some(applied)) => Ok(applied),
                Ok(None) | Err(_) => Err(Attempt::Fail(error.into())),
            },
            Err(error) => Err(Attempt::Fail(error.into())),
        }
    }

    /// Decides whether a batch with an unknown outcome was applied: its objects are present or
    /// absent as it asked, every key it put holds its value, the conditional ones at one shared
    /// revision, and every key it deleted is absent. Anything else counts as not applied. The
    /// read-back cannot tell which objects existed before, so all of them count as inserted.
    fn reconcile(&self, batch: &Batch) -> Result<Option<Applied>, StoreError> {
        let puts = batch
            .put_objects
            .iter()
            .map(|(digest, _)| *digest)
            .collect::<Vec<_>>();
        for chunk in puts.chunks(self.read_items() as usize) {
            if self.backend.get_objects(chunk)?.iter().any(Option::is_none) {
                return Ok(None);
            }
        }
        for chunk in batch.delete_objects.chunks(self.read_items() as usize) {
            if self.backend.get_objects(chunk)?.iter().any(Option::is_some) {
                return Ok(None);
            }
        }
        let mut conditional = None;
        let mut any = None;
        for op in &batch.keys {
            let current = self.backend.read_key(op.space, &op.key)?;
            match (&op.action, current) {
                (KeyAction::Put(value), Some(current)) if current.value == *value => {
                    if op.expect != Expect::Any {
                        if conditional.is_some_and(|revision| revision != current.revision) {
                            return Ok(None);
                        }
                        conditional = Some(current.revision);
                    }
                    any.get_or_insert(current.revision);
                }
                (KeyAction::Put(_), _) | (KeyAction::Delete, Some(_)) => return Ok(None),
                (KeyAction::Delete, None) | (KeyAction::Check, _) => {}
            }
        }
        Ok(Some(Applied {
            revision: conditional.or(any),
            objects_inserted: batch.put_objects.len() as u64,
            objects_deleted: batch.delete_objects.len() as u64,
        }))
    }
}

fn conflict_error(tag: &Tag, conflict: Conflict) -> Attempt {
    let revision = conflict
        .actual
        .as_ref()
        .map(|value| RefRevision::from_revision(value.revision));
    let value = conflict.actual.as_ref().map(|value| value.value.as_slice());
    let error: Result<StoreError, StoreError> = match tag {
        Tag::Replan | Tag::Index => return Attempt::Retry,
        Tag::Ref {
            key,
            expected,
            proposed,
        } => value
            .map(layout::decode_commit_target)
            .transpose()
            .map(|commit| {
                RefConflict {
                    key: key.storage_key(),
                    expected: *expected,
                    actual: commit
                        .zip(revision)
                        .map(|(commit, revision)| RefValue { revision, commit }),
                    proposed: *proposed,
                }
                .into()
            }),
        Tag::Catalog {
            key,
            expected,
            proposed,
        } => value
            .map(layout::decode_catalog_head)
            .transpose()
            .map(|head| {
                CatalogConflict {
                    key: key.storage_key(),
                    expected: *expected,
                    actual: head.zip(revision).map(|((event, coverage), revision)| {
                        CatalogHeadRefValue {
                            revision,
                            event,
                            coverage,
                        }
                    }),
                    proposed: *proposed,
                }
                .into()
            }),
        Tag::Archive {
            key,
            expected,
            proposed,
        } => value
            .map(layout::decode_archive)
            .transpose()
            .map(|manifest| {
                ArchiveConflict {
                    key: key.storage_key(),
                    expected: *expected,
                    actual: manifest
                        .zip(revision)
                        .map(|(manifest, revision)| TimelineArchiveRefValue { revision, manifest }),
                    proposed: *proposed,
                }
                .into()
            }),
        Tag::CompoundSave {
            key,
            expected,
            proposed,
        } => value
            .map(layout::decode_compound_save)
            .transpose()
            .map(|manifest| {
                CompoundSaveConflict {
                    key: key.storage_key(),
                    expected: *expected,
                    actual: manifest
                        .zip(revision)
                        .map(|(manifest, revision)| CompoundSaveRefValue { revision, manifest }),
                    proposed: *proposed,
                }
                .into()
            }),
    };
    Attempt::Fail(error.unwrap_or_else(|error| error))
}
