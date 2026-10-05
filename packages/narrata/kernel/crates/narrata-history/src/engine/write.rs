//! Write planning and batch application (ADR 0014).
//!
//! A write is planned from the store's current state and applied as one batch whose
//! preconditions encode everything the plan read: the CAS revisions the caller expects, absent
//! index keys and the `meta/sweep` revision. A conflict on a caller's key is the caller's
//! conflict; a conflict elsewhere means the plan read stale state, so the write is planned again.

use std::collections::{BTreeMap, BTreeSet};

use narrata_storage::{
    Applied, Batch, Conflict, Expect, KeyAction, KeyOp, KeySpace, Revision, StorageBackend,
    StorageError,
};

use super::{History, RETRIES, View, digest, kind_info, references, unregistered};
use crate::{
    HistoryError, Object, ObjectId, Pin, RefConflict, RefKey, RefRevision, RefValue, Reference,
    Registry, bundle, layout, shallow,
};

/// What a key operation's precondition failure means.
#[derive(Clone, Debug)]
pub enum Tag<T> {
    /// The plan read state that changed since: plan again.
    Replan,
    /// An unconditional write, which cannot conflict.
    Index,
    Ref {
        key: RefKey,
        expected: Option<RefRevision>,
        proposed: Option<ObjectId>,
    },
    /// One of the registrant's root keys; [`Registry::conflict`] says what it means.
    Root(T),
}

/// One key operation of a batch with the meaning of its precondition failing.
#[derive(Clone, Debug)]
pub struct Op<T> {
    op: KeyOp,
    tag: Tag<T>,
}

impl<T> Op<T> {
    pub fn put(space: KeySpace, key: Vec<u8>, value: Vec<u8>, expect: Expect, tag: Tag<T>) -> Self {
        Self::new(space, key, expect, KeyAction::Put(value), tag)
    }

    pub fn delete(space: KeySpace, key: Vec<u8>, expect: Expect, tag: Tag<T>) -> Self {
        Self::new(space, key, expect, KeyAction::Delete, tag)
    }

    pub fn check(space: KeySpace, key: Vec<u8>, expect: Expect, tag: Tag<T>) -> Self {
        Self::new(space, key, expect, KeyAction::Check, tag)
    }

    fn new(space: KeySpace, key: Vec<u8>, expect: Expect, action: KeyAction, tag: Tag<T>) -> Self {
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

    pub fn bytes(&self) -> u64 {
        let value = match &self.op.action {
            KeyAction::Put(value) => value.len(),
            KeyAction::Delete | KeyAction::Check => 0,
        };
        (self.op.key.len() + value) as u64
    }
}

/// Why one batch did not apply.
#[derive(Debug)]
pub enum Attempt<E> {
    /// The batch read stale state; read again and retry.
    Retry,
    Fail(E),
}

impl<E: From<HistoryError>> Attempt<E> {
    /// A history failure as the registrant's error.
    pub fn history(error: HistoryError) -> Self {
        Self::Fail(error.into())
    }
}

impl<E> From<E> for Attempt<E> {
    fn from(value: E) -> Self {
        Self::Fail(value)
    }
}

/// Runs `attempt` until it succeeds or fails rather than asking to read again.
pub fn retry<T, E: From<HistoryError>>(
    mut attempt: impl FnMut() -> Result<T, Attempt<E>>,
) -> Result<T, E> {
    for _ in 0..RETRIES {
        match attempt() {
            Ok(value) => return Ok(value),
            Err(Attempt::Retry) => {}
            Err(Attempt::Fail(error)) => return Err(error),
        }
    }
    Err(HistoryError::Busy.into())
}

/// Checks that no GC deletion ran since the plan read `sweep`.
pub fn sweep_check<T>(sweep: Option<Revision>) -> Op<T> {
    Op::check(
        layout::META,
        layout::SWEEP_KEY.to_vec(),
        sweep.map_or(Expect::Absent, Expect::Revision),
        Tag::Replan,
    )
}

/// Tells a running GC that objects or roots were added after it marked.
pub fn graph_bump<T>() -> Op<T> {
    Op::put(
        layout::META,
        layout::GRAPH_KEY.to_vec(),
        layout::encode_marker(),
        Expect::Any,
        Tag::Index,
    )
}

/// The precondition for a root key the caller saw at `expected`, or saw absent.
pub fn expect(expected: Option<RefRevision>) -> Expect {
    expected
        .and_then(RefRevision::revision)
        .map_or(Expect::Absent, Expect::Revision)
}

/// A batch under construction, with the tag of each key operation.
#[derive(Debug)]
pub struct Ops<T> {
    objects: Vec<Object>,
    deletes: Vec<ObjectId>,
    keys: Vec<Op<T>>,
}

impl<T> Default for Ops<T> {
    fn default() -> Self {
        Self {
            objects: Vec::new(),
            deletes: Vec::new(),
            keys: Vec::new(),
        }
    }
}

impl<T> Ops<T> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn object(&mut self, object: Object) {
        self.objects.push(object);
    }

    pub fn delete_object(&mut self, id: ObjectId) {
        self.deletes.push(id);
    }

    pub fn key(&mut self, op: Op<T>) {
        self.keys.push(op);
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RefMutation {
    pub key: RefKey,
    pub expected: Option<RefRevision>,
    pub next: Option<ObjectId>,
}

impl RefMutation {
    /// The key operation that applies this mutation; a target must be a commit-role object.
    /// [`History::write`] plans its transaction's Refs with it, and registrants that order their
    /// own keys around Refs call it from the planning closure.
    pub fn plan<B: StorageBackend, R: Registry>(
        &self,
        view: &mut View<'_, B, R>,
    ) -> Result<Op<R::Tag>, HistoryError> {
        let key = layout::ref_key(&self.key);
        let tag = Tag::Ref {
            key: self.key.clone(),
            expected: self.expected,
            proposed: self.next,
        };
        Ok(match self.next {
            Some(commit) => {
                let target = view.require(commit, None)?;
                if !view.kind(target.kind()).is_some_and(|info| info.commit) {
                    return Err(HistoryError::ObjectKind(commit));
                }
                Op::put(
                    layout::REFS,
                    key,
                    layout::encode_ref_target(commit),
                    expect(self.expected),
                    tag,
                )
            }
            None => Op::delete(layout::REFS, key, expect(self.expected), tag),
        })
    }
}

/// Objects, Refs and Pins one write adds or changes. Registrants add their own keys through
/// the planning closure of [`History::write`].
#[derive(Clone, Debug, Default)]
pub struct Transaction {
    pub objects: Vec<Object>,
    /// Observation time of each object, aligned with `objects`; when empty or short, the rest
    /// take `observed_at`.
    pub observed: Vec<u64>,
    pub observed_at: u64,
    pub refs: Vec<RefMutation>,
    pub pins: Vec<Pin>,
    pub remove_pins: Vec<(String, ObjectId)>,
    /// Stored objects the planning closure will require, read together with the references of
    /// the new objects.
    pub prefetch: Vec<ObjectId>,
}

/// What a write did.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Written {
    /// The revision of every key the write put; `None` when it put none.
    pub revision: Option<Revision>,
    pub inserted: u64,
}

impl Written {
    /// The revision a root key the write put now has.
    pub fn root_revision(&self) -> Result<RefRevision, HistoryError> {
        self.revision
            .map(RefRevision::from_revision)
            .ok_or_else(|| {
                HistoryError::CorruptStore(
                    "backend applied root writes without a revision".to_owned(),
                )
            })
    }

    /// The value a Ref the write set to `next` now has.
    pub fn ref_value(&self, next: Option<ObjectId>) -> Result<Option<RefValue>, HistoryError> {
        next.map(|commit| {
            Ok(RefValue {
                revision: self.root_revision()?,
                commit,
            })
        })
        .transpose()
    }
}

struct Staged<T> {
    object: Object,
    observed_at: u64,
    index: Vec<Op<T>>,
}

impl<T> Staged<T> {
    fn ops(&self) -> u64 {
        2 + self.index.len() as u64
    }

    fn bytes(&self) -> u64 {
        self.object.bytes().len() as u64 + 48 + self.index.iter().map(Op::bytes).sum::<u64>()
    }

    fn into_ops(self, ops: &mut Ops<T>) {
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

struct Plan<T> {
    sweep: Option<Revision>,
    /// Referenced objects before the objects that refer to them, so that every prefix written
    /// on its own keeps each stored object's references stored.
    objects: Vec<Staged<T>>,
    keys: Vec<Op<T>>,
}

/// Orders `staged` so that every object follows the staged objects it refers to.
fn referents_first<B: StorageBackend, R: Registry>(
    view: &mut View<'_, B, R>,
) -> Result<Vec<ObjectId>, R::Error> {
    let staged = view.staged();
    let mut targets = BTreeMap::new();
    for (id, object) in staged {
        let mut values = Vec::new();
        for reference in view.references(object)? {
            let target = match reference {
                Reference::Object { id, .. } => Some(id),
                Reference::Named { kind, name } => view.staged_named(kind, &name),
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

/// Checks one new object: the objects it refers to exist with the kinds it requires, and the
/// registrant's own checks pass. Returns the index keys it adds.
pub(super) fn validate<B: StorageBackend, R: Registry>(
    view: &mut View<'_, B, R>,
    object: &Object,
) -> Result<Vec<Op<R::Tag>>, R::Error> {
    for reference in view.references(object)? {
        match reference {
            Reference::Object {
                id,
                kind,
                descriptor,
            } => {
                // Descriptors are compared field by field by the manifest's own check.
                view.require(id, if descriptor { None } else { kind })?;
            }
            Reference::Named { kind, name } => {
                if view.resolve(kind, &name)?.is_none() {
                    return Err(HistoryError::Unresolved { kind }.into());
                }
            }
        }
    }
    if object.kind() == shallow::TRUNCATED_PARENT_KIND {
        shallow::validate_truncated_parent(object)?;
        return Ok(Vec::new());
    }
    if object.kind() == bundle::CHECKPOINT_MANIFEST_KIND {
        bundle::validate_manifest(view, object)?;
        return Ok(Vec::new());
    }
    if object.kind() == crate::effect::EFFECT_LEDGER_GUARD_KIND {
        crate::effect::validate_guard(object)?;
        return Ok(Vec::new());
    }
    if object.kind() == crate::effect::EFFECT_RESPONSE_KIND {
        crate::effect::validate_response_object(object)?;
        return Ok(Vec::new());
    }
    if object.kind() == crate::migration::MIGRATION_INPUT_KIND {
        crate::migration::MigrationInput::from_object(object)?;
        return Ok(Vec::new());
    }
    let registry = view.registry();
    registry.validate(object, view)
}

impl<B: StorageBackend, R: Registry> History<B, R> {
    /// Writes `transaction` and the keys `extra` plans, as one batch where the backend's limits
    /// allow. `extra` runs once per attempt, against the state that attempt reads, so its reads
    /// and the preconditions it sets stay consistent.
    pub fn write(
        &mut self,
        transaction: &Transaction,
        mut extra: impl FnMut(&mut View<'_, B, R>) -> Result<Vec<Op<R::Tag>>, R::Error>,
    ) -> Result<Written, R::Error> {
        for _ in 0..RETRIES {
            let sweep = self.reader().sweep()?;
            let Plan {
                sweep,
                objects,
                keys,
            } = self.plan(transaction, sweep, &mut extra)?;
            if objects.is_empty() && keys.is_empty() {
                return Ok(Written::default());
            }
            match self.apply_plan(sweep, objects, keys) {
                Ok((revision, inserted)) => return Ok(Written { revision, inserted }),
                Err(Attempt::Retry) => {}
                Err(Attempt::Fail(error)) => return Err(error),
            }
        }
        Err(HistoryError::Busy.into())
    }

    /// Plans a single batch without writing it. Migration dry-runs use the same checks/limits
    /// as application; the actual CAS is still checked when the batch is applied.
    pub fn validate_atomic(&self, transaction: &Transaction) -> Result<(), R::Error> {
        let plan = self.plan(transaction, self.reader().sweep()?, &mut |_| Ok(Vec::new()))?;
        self.atomic_ops(plan).map(|_| ())
    }
    /// Writes exactly one atomic batch; oversized operations fail before writing any object.
    pub fn write_atomic(
        &mut self,
        transaction: &Transaction,
        mut extra: impl FnMut(&mut View<'_, B, R>) -> Result<Vec<Op<R::Tag>>, R::Error>,
    ) -> Result<Written, R::Error> {
        retry::<_, R::Error>(|| {
            let plan = self.plan(
                transaction,
                self.reader().sweep().map_err(R::Error::from)?,
                &mut extra,
            )?;
            let ops = self.atomic_ops(plan)?;
            let applied = self.apply(ops)?;
            Ok(Written {
                revision: applied.revision,
                inserted: applied.objects_inserted,
            })
        })
    }
    fn atomic_ops(&self, plan: Plan<R::Tag>) -> Result<Ops<R::Tag>, R::Error> {
        for staged in &plan.objects {
            self.limits
                .check(
                    narrata_storage::Limit::ValueBytes,
                    staged.object.bytes().len() as u64,
                )
                .map_err(HistoryError::from)?;
        }
        for key in plan
            .keys
            .iter()
            .chain(plan.objects.iter().flat_map(|staged| &staged.index))
        {
            self.limits
                .check(narrata_storage::Limit::KeyBytes, key.op.key.len() as u64)
                .map_err(HistoryError::from)?;
            if let KeyAction::Put(value) = &key.op.action {
                self.limits
                    .check(narrata_storage::Limit::ValueBytes, value.len() as u64)
                    .map_err(HistoryError::from)?;
            }
        }
        let count = 2 + plan.objects.iter().map(Staged::ops).sum::<u64>() + plan.keys.len() as u64;
        let bytes = 64
            + plan.objects.iter().map(Staged::bytes).sum::<u64>()
            + plan.keys.iter().map(Op::bytes).sum::<u64>();
        if count > self.limits.max_batch_ops || bytes > self.limits.max_batch_bytes {
            return Err(HistoryError::Limit("atomic batch").into());
        }
        let mut ops = Ops::new();
        ops.key(sweep_check(plan.sweep));
        ops.keys.extend(plan.keys);
        for object in plan.objects {
            object.into_ops(&mut ops);
        }
        ops.key(graph_bump());
        Ok(ops)
    }

    fn plan(
        &self,
        transaction: &Transaction,
        sweep: Option<Revision>,
        extra: &mut impl FnMut(&mut View<'_, B, R>) -> Result<Vec<Op<R::Tag>>, R::Error>,
    ) -> Result<Plan<R::Tag>, R::Error> {
        let mut staged = BTreeMap::new();
        let mut observed = BTreeMap::new();
        for (index, object) in transaction.objects.iter().enumerate() {
            if kind_info(&self.registry, object.kind()).is_none() {
                return Err(unregistered(object).into());
            }
            let observed_at = transaction
                .observed
                .get(index)
                .copied()
                .unwrap_or(transaction.observed_at);
            observed.entry(object.id()).or_insert(observed_at);
            staged.entry(object.id()).or_insert_with(|| object.clone());
        }
        let mut view = View::new(self.reader(), &self.registry, &staged);

        let mut wanted = Vec::new();
        for object in staged.values() {
            for reference in references(&self.registry, object)? {
                if let Reference::Object { id, .. } = reference {
                    wanted.push(id);
                }
            }
        }
        wanted.extend(transaction.refs.iter().filter_map(|value| value.next));
        wanted.extend(transaction.pins.iter().map(|pin| pin.object));
        wanted.extend(transaction.prefetch.iter().copied());
        view.prefetch(wanted)?;
        let order = referents_first(&mut view)?;

        let mut objects = Vec::with_capacity(order.len());
        for id in order {
            let Some(object) = staged.get(&id) else {
                continue;
            };
            let index = validate(&mut view, object)?;
            objects.push(Staged {
                object: object.clone(),
                observed_at: observed
                    .get(&id)
                    .copied()
                    .unwrap_or(transaction.observed_at),
                index,
            });
        }

        let mut keys = Vec::new();
        let mut named = BTreeSet::new();
        for mutation in &transaction.refs {
            if !named.insert(layout::ref_key(&mutation.key)) {
                return Err(
                    HistoryError::InvalidGraph("Ref named twice in one transaction").into(),
                );
            }
            keys.push(mutation.plan(&mut view)?);
        }
        keys.extend(extra(&mut view)?);

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
        Ok(Plan {
            sweep,
            objects,
            keys,
        })
    }

    /// Applies a plan as one batch, or, when it exceeds the backend's batch limits, as object
    /// batches in reference order followed by one batch with every other key.
    fn apply_plan(
        &mut self,
        sweep: Option<Revision>,
        objects: Vec<Staged<R::Tag>>,
        keys: Vec<Op<R::Tag>>,
    ) -> Result<(Option<Revision>, u64), Attempt<R::Error>> {
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
        let applied = self.apply(batch)?;
        Ok((applied.revision, inserted + applied.objects_inserted))
    }

    fn apply_objects(
        &mut self,
        sweep: Option<Revision>,
        objects: Vec<Staged<R::Tag>>,
    ) -> Result<Applied, Attempt<R::Error>> {
        let mut batch = Ops::new();
        batch.key(sweep_check(sweep));
        for object in objects {
            object.into_ops(&mut batch);
        }
        batch.key(graph_bump());
        self.apply(batch)
    }

    /// Applies one batch. A conflict is mapped through its key's tag; an unknown outcome is
    /// reconciled by reading back what the batch would have written.
    pub fn apply(&mut self, ops: Ops<R::Tag>) -> Result<Applied, Attempt<R::Error>> {
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
            Err(StorageError::Conflict(conflict)) => Err(match tags.get(conflict.index) {
                Some(tag) => self.conflict(tag, *conflict),
                None => Attempt::Retry,
            }),
            Err(error) if error.outcome_unknown() => match self.reconcile(&batch) {
                Ok(Some(applied)) => Ok(applied),
                // The host loads what the read-back lacks and runs the operation again, which
                // decides better than reporting the outcome as unknown.
                Err(StorageError::NotLoaded) => Err(Attempt::Fail(
                    HistoryError::from(StorageError::NotLoaded).into(),
                )),
                Ok(None) | Err(_) => Err(Attempt::Fail(HistoryError::from(error).into())),
            },
            Err(error) => Err(Attempt::Fail(HistoryError::from(error).into())),
        }
    }

    fn conflict(&self, tag: &Tag<R::Tag>, conflict: Conflict) -> Attempt<R::Error> {
        match tag {
            Tag::Replan | Tag::Index => Attempt::Retry,
            Tag::Ref {
                key,
                expected,
                proposed,
            } => {
                let actual = conflict
                    .actual
                    .as_ref()
                    .map(|value| {
                        Ok::<_, HistoryError>(RefValue {
                            revision: RefRevision::from_revision(value.revision),
                            commit: layout::decode_ref_target(&value.value)?,
                        })
                    })
                    .transpose();
                Attempt::Fail(match actual {
                    Ok(actual) => HistoryError::from(RefConflict {
                        key: key.clone(),
                        expected: *expected,
                        actual,
                        proposed: *proposed,
                    })
                    .into(),
                    Err(error) => error.into(),
                })
            }
            Tag::Root(tag) => Attempt::Fail(self.registry.conflict(tag, conflict)),
        }
    }

    /// Decides whether a batch with an unknown outcome was applied: its objects are present or
    /// absent as it asked, every key it put holds its value, the conditional ones at one shared
    /// revision, and every key it deleted is absent. Anything else counts as not applied. The
    /// read-back cannot tell which objects existed before, so all of them count as inserted.
    fn reconcile(&self, batch: &Batch) -> Result<Option<Applied>, StorageError> {
        let read_items = self.reader().read_items() as usize;
        let puts = batch
            .put_objects
            .iter()
            .map(|(digest, _)| *digest)
            .collect::<Vec<_>>();
        for chunk in puts.chunks(read_items) {
            if self.backend.get_objects(chunk)?.iter().any(Option::is_none) {
                return Ok(None);
            }
        }
        for chunk in batch.delete_objects.chunks(read_items) {
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
