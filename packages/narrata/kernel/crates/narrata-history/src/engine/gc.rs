//! Garbage collection and the integrity scan (ADR 0014, ADR 0015).
//!
//! GC marks from the roots and from every object still inside its grace period, then deletes
//! the rest in batches that delete an object no later than the objects it refers to. Each
//! deletion batch checks that `meta/graph` has not moved since marking, so a write that added a
//! reference in between makes GC mark again, and bumps `meta/sweep`, so a write that read an
//! object before the deletion is planned again.
//!
//! GC refuses to run when it meets an object of a kind no registrant knows: it cannot tell what
//! such an object refers to, and deleting what only that object kept alive would lose data.

use std::collections::{BTreeMap, BTreeSet};

use narrata_storage::{Expect, Revision, StorageBackend};

use super::{
    History, kind_info, references, unregistered,
    write::{Attempt, Op, Ops, Tag},
};
use crate::{HistoryError, Object, ObjectId, Reference, Registry, Root, layout, shallow};

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
    /// Removed objects and bytes by kind code.
    pub removed: BTreeMap<u16, GcKindReport>,
    pub dry_run: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IntegrityIssue {
    pub object: ObjectId,
    pub diagnostic: String,
}

impl<B: StorageBackend, R: Registry> History<B, R> {
    fn graph_references(&self, object: &Object) -> Result<Vec<Reference>, HistoryError> {
        let mut references = references(&self.registry, object)?;
        if let Some(parent) = shallow::parent(&self.registry, object)
            && self.reader().object(parent)?.is_none()
            && let Some(marker) = self.reader().truncated_parent(parent)?
        {
            for reference in &mut references {
                if matches!(reference, Reference::Object { id, descriptor: false, .. } if *id == parent)
                {
                    *reference = Reference::object(marker.id(), shallow::TRUNCATED_PARENT_KIND);
                }
            }
        }
        Ok(references)
    }

    pub fn collect(&mut self, policy: RetentionPolicy) -> Result<GcReport, R::Error> {
        let mut report = GcReport {
            dry_run: policy.dry_run,
            ..GcReport::default()
        };
        for _ in 0..super::RETRIES {
            match self.collect_once(policy, &mut report) {
                Ok(()) => return Ok(report),
                Err(Attempt::Retry) => {}
                Err(Attempt::Fail(error)) => return Err(error),
            }
        }
        Err(HistoryError::Busy.into())
    }

    fn collect_once(
        &mut self,
        policy: RetentionPolicy,
        report: &mut GcReport,
    ) -> Result<(), Attempt<R::Error>> {
        let reader = self.reader();
        let graph = reader
            .read_key(layout::META, layout::GRAPH_KEY)
            .map_err(Attempt::history)?
            .map(|value| value.revision);
        let mut touched = BTreeMap::new();
        for entry in reader
            .scan_every(layout::TOUCH, &[])
            .map_err(Attempt::history)?
        {
            let id = layout::decode_touch_key(&entry.key).map_err(Attempt::history)?;
            let observed_at = layout::decode_touch(&entry.value).map_err(Attempt::history)?;
            touched.insert(id, (observed_at, entry.revision));
        }
        let roots = self.roots(policy.now)?;
        report.roots = roots.len() as u64;
        let old_enough = |observed_at: u64| {
            observed_at
                .checked_add(policy.grace_seconds)
                .is_some_and(|deadline| deadline <= policy.now)
        };
        // Objects inside the grace period keep what they refer to, so deleting never leaves a
        // stored object with a missing reference.
        let young = touched
            .iter()
            .filter(|(_, (observed_at, _))| !old_enough(*observed_at))
            .map(|(id, _)| (*id, None));
        let marked = self.mark(roots.into_iter().chain(young).collect())?;
        report.reachable = marked.len() as u64;

        let candidates = touched
            .iter()
            .filter(|(id, _)| !marked.contains(*id))
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        let mut found = BTreeMap::new();
        let objects = self
            .reader()
            .objects(&candidates)
            .map_err(Attempt::history)?;
        for (id, object) in candidates.iter().zip(objects) {
            if let Some(object) = object {
                if kind_info(&self.registry, object.kind()).is_none() {
                    return Err(Attempt::Fail(unregistered(&object).into()));
                }
                found.insert(*id, object);
            }
        }
        let order = self.referrers_first(&candidates, &found)?;
        if policy.dry_run {
            for object in found.values() {
                count(report, object);
            }
            return Ok(());
        }

        let limits = self.limits;
        let fits =
            |ops: u64, bytes: u64| ops <= limits.max_batch_ops && bytes <= limits.max_batch_bytes;
        let mut batch = deletion_batch(graph);
        let mut counted = Vec::new();
        let (mut ops, mut bytes) = (2_u64, 64_u64);
        for id in order {
            let (_, revision) = touched.get(&id).copied().ok_or_else(|| {
                Attempt::history(HistoryError::CorruptStore(
                    "touch key vanished during GC".to_owned(),
                ))
            })?;
            let object = found.get(&id);
            let deletion = self.deletion_ops(id, revision, object)?;
            let size = deletion.len() as u64 + u64::from(object.is_some());
            let weight = deletion.iter().map(Op::bytes).sum::<u64>();
            if !counted.is_empty() && !fits(ops + size, bytes + weight) {
                let full = std::mem::replace(&mut batch, deletion_batch(graph));
                self.apply(full)?;
                for object in counted.drain(..) {
                    count(report, object);
                }
                (ops, bytes) = (2, 64);
            }
            ops += size;
            bytes += weight;
            if let Some(object) = object {
                batch.delete_object(id);
                counted.push(object);
            }
            for op in deletion {
                batch.key(op);
            }
        }
        if ops > 2 {
            self.apply(batch)?;
            for object in counted {
                count(report, object);
            }
        }
        Ok(())
    }

    /// Refs, unexpired Pins and the registrant's roots, each object once.
    fn roots(&self, now: u64) -> Result<Vec<Root>, R::Error> {
        let reader = self.reader();
        let mut roots = Vec::new();
        for entry in reader.scan_every(layout::REFS, &[])? {
            roots.push((layout::decode_ref_target(&entry.value)?, None));
        }
        for entry in reader.scan_every(layout::PINS, &[])? {
            let (_, object) = layout::decode_pin_key(&entry.key)?;
            let expires_at = layout::decode_pin_value(&entry.value)?;
            if expires_at.is_none_or(|expires_at| expires_at > now) {
                roots.push((object, None));
            }
        }
        roots.extend(crate::effect::generic_roots(&reader)?);
        roots.extend(self.registry.roots(&reader, now)?);
        roots.sort_unstable();
        roots.dedup_by_key(|(id, _)| *id);
        Ok(roots)
    }

    fn resolve_named(&self, kind: u16, name: &[u8; 32]) -> Result<ObjectId, R::Error> {
        self.registry
            .resolve(&self.reader(), kind, name)?
            .ok_or_else(|| HistoryError::Unresolved { kind }.into())
    }

    /// Everything reachable from `start`. Only objects whose kind can refer to others are read;
    /// an object of an unregistered kind stops the walk.
    fn mark(&self, start: Vec<Root>) -> Result<BTreeSet<ObjectId>, R::Error> {
        let mut marked = BTreeSet::new();
        let mut named = BTreeMap::new();
        let mut frontier = start;
        while !frontier.is_empty() {
            let mut read = Vec::new();
            for (id, kind) in std::mem::take(&mut frontier) {
                let leaf = kind
                    .and_then(|kind| kind_info(&self.registry, kind))
                    .is_some_and(|info| info.leaf);
                if marked.insert(id) && !leaf {
                    read.push(id);
                }
            }
            for (id, object) in read.iter().zip(self.reader().objects(&read)?) {
                let object = object.ok_or(HistoryError::MissingObject(*id))?;
                for reference in self.graph_references(&object)? {
                    frontier.push(match reference {
                        Reference::Object { id, kind, .. } => (id, kind),
                        Reference::Named { kind, name } => {
                            let id = match named.get(&(kind, name)) {
                                Some(id) => *id,
                                None => {
                                    let id = self.resolve_named(kind, &name)?;
                                    named.insert((kind, name), id);
                                    id
                                }
                            };
                            (id, Some(kind))
                        }
                    });
                }
            }
        }
        Ok(marked)
    }

    /// Orders unreachable objects so that each comes before every candidate it refers to.
    fn referrers_first(
        &self,
        candidates: &[ObjectId],
        found: &BTreeMap<ObjectId, Object>,
    ) -> Result<Vec<ObjectId>, R::Error> {
        let reader = self.reader();
        let mut referents = BTreeMap::<ObjectId, Vec<ObjectId>>::new();
        let mut referrers = candidates
            .iter()
            .map(|id| (*id, 0_usize))
            .collect::<BTreeMap<_, _>>();
        for (id, object) in found {
            let mut targets = BTreeSet::new();
            for reference in self.graph_references(object)? {
                let target = match reference {
                    Reference::Object { id, .. } => Some(id),
                    Reference::Named { kind, name } => {
                        self.registry.resolve(&reader, kind, &name)?
                    }
                };
                targets.extend(target.filter(|target| referrers.contains_key(target)));
            }
            for target in &targets {
                if let Some(count) = referrers.get_mut(target) {
                    *count += 1;
                }
            }
            referents.insert(*id, targets.into_iter().collect());
        }
        let mut ready = referrers
            .iter()
            .filter(|(_, count)| **count == 0)
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        let mut order = Vec::with_capacity(candidates.len());
        while let Some(id) = ready.pop() {
            order.push(id);
            for target in referents.get(&id).into_iter().flatten() {
                if let Some(count) = referrers.get_mut(target) {
                    *count -= 1;
                    if *count == 0 {
                        ready.push(*target);
                    }
                }
            }
        }
        Ok(order)
    }

    /// The key operations that remove one object's touch key and the index keys derived from it.
    fn deletion_ops(
        &self,
        id: ObjectId,
        touch: Revision,
        object: Option<&Object>,
    ) -> Result<Vec<Op<R::Tag>>, R::Error> {
        let mut ops = vec![Op::delete(
            layout::TOUCH,
            layout::touch_key(id),
            Expect::Revision(touch),
            Tag::Replan,
        )];
        if let Some(object) = object
            && object.kind() != crate::bundle::CHECKPOINT_MANIFEST_KIND
            && object.kind() != shallow::TRUNCATED_PARENT_KIND
            && object.kind() != crate::effect::EFFECT_RESPONSE_KIND
            && object.kind() != crate::effect::EFFECT_LEDGER_GUARD_KIND
            && object.kind() != crate::migration::MIGRATION_INPUT_KIND
        {
            ops.extend(self.registry.unindex(object, &self.reader())?);
        }
        Ok(ops)
    }

    pub fn integrity_scan(&self) -> Result<Vec<IntegrityIssue>, R::Error> {
        let reader = self.reader();
        let mut issues = Vec::new();
        let mut present = BTreeSet::new();
        let mut pending = Vec::new();
        let mut after = None;
        loop {
            let page = self
                .backend
                .scan_objects(after.as_ref(), reader.read_items())
                .map_err(HistoryError::from)?;
            let found = self
                .backend
                .get_objects(&page.digests)
                .map_err(HistoryError::from)?;
            for (digest, bytes) in page.digests.iter().zip(found) {
                let id = ObjectId::from_bytes(*digest.as_bytes());
                let Some(bytes) = bytes else {
                    continue;
                };
                match Object::verify(id, bytes) {
                    Ok(object) => {
                        present.insert(id);
                        match kind_info(&self.registry, object.kind()) {
                            None => issues.push(IntegrityIssue {
                                object: id,
                                diagnostic: unregistered(&object).to_string(),
                            }),
                            Some(info)
                                if info.leaf && object.kind() != shallow::TRUNCATED_PARENT_KIND => {
                            }
                            Some(_) => match self.graph_references(&object) {
                                Ok(references) => pending.push((id, references)),
                                Err(error) => issues.push(IntegrityIssue {
                                    object: id,
                                    diagnostic: diagnostic(error),
                                }),
                            },
                        }
                    }
                    Err(error @ HistoryError::Corrupt(..)) => issues.push(IntegrityIssue {
                        object: id,
                        diagnostic: diagnostic(error),
                    }),
                    Err(error) => return Err(error.into()),
                }
            }
            match page.resume_after() {
                Some(last) => after = Some(*last),
                None => break,
            }
        }
        for (id, references) in pending {
            for reference in references {
                let target = match reference {
                    Reference::Object { id, .. } => Some(id),
                    Reference::Named { kind, name } => {
                        match self.registry.resolve(&reader, kind, &name)? {
                            Some(target) => Some(target),
                            None => {
                                let name = kind_info(&self.registry, kind)
                                    .map_or("object", |info| info.name);
                                issues.push(IntegrityIssue {
                                    object: id,
                                    diagnostic: format!("referenced {name} is not indexed"),
                                });
                                None
                            }
                        }
                    }
                };
                if let Some(target) = target
                    && !present.contains(&target)
                {
                    issues.push(IntegrityIssue {
                        object: id,
                        diagnostic: format!("missing referenced object {target}"),
                    });
                }
            }
        }
        Ok(issues)
    }
}

fn diagnostic(error: HistoryError) -> String {
    match error {
        HistoryError::Corrupt(_, diagnostic) => diagnostic,
        other => other.to_string(),
    }
}

fn count(report: &mut GcReport, object: &Object) {
    let entry = report.removed.entry(object.kind()).or_default();
    entry.objects = entry.objects.saturating_add(1);
    entry.bytes = entry.bytes.saturating_add(object.bytes().len() as u64);
}

fn deletion_batch<T>(graph: Option<Revision>) -> Ops<T> {
    let mut ops = Ops::new();
    ops.key(Op::check(
        layout::META,
        layout::GRAPH_KEY.to_vec(),
        graph.map_or(Expect::Absent, Expect::Revision),
        Tag::Replan,
    ));
    ops.key(Op::put(
        layout::META,
        layout::SWEEP_KEY.to_vec(),
        layout::encode_marker(),
        Expect::Any,
        Tag::Index,
    ));
    ops
}
