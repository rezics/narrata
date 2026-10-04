//! Garbage collection and the integrity scan (ADR 0014).
//!
//! GC marks from the roots and from every object still inside its grace period, then deletes
//! the rest in batches that delete an object no later than the objects it refers to. Each
//! deletion batch checks that `meta/graph` has not moved since marking, so a write that added a
//! reference in between makes GC mark again, and bumps `meta/sweep`, so a write that read an
//! object before the deletion is planned again.

use std::collections::{BTreeMap, BTreeSet};

use narrata_core::{
    CommitId, ObjectId, ProgramArtifactId, TimelineCatalogEventId, codec::ObjectKind,
};
use narrata_storage::{Expect, Revision, StorageBackend};

use super::{
    RETRIES, Store, verify,
    write::{Attempt, Op, Ops, Tag},
};
use crate::{
    CheckedObject, CommitV1, GcKindReport, GcReport, IntegrityIssue, RetentionPolicy, SaveStore,
    StoreError, TimelineCatalogEventV1,
    graph::{Reference, is_leaf, references},
    layout,
};

type Root = (ObjectId, Option<ObjectKind>);

impl<B: StorageBackend> Store<B> {
    pub(super) fn collect_garbage(
        &mut self,
        policy: RetentionPolicy,
    ) -> Result<GcReport, StoreError> {
        let mut report = GcReport {
            dry_run: policy.dry_run,
            ..GcReport::default()
        };
        for _ in 0..RETRIES {
            match self.collect_once(policy, &mut report) {
                Ok(()) => return Ok(report),
                Err(Attempt::Retry) => {}
                Err(Attempt::Fail(error)) => return Err(error),
            }
        }
        Err(StoreError::Busy)
    }

    fn collect_once(
        &mut self,
        policy: RetentionPolicy,
        report: &mut GcReport,
    ) -> Result<(), Attempt> {
        let graph = self
            .read_key(layout::META, layout::GRAPH_KEY)?
            .map(|value| value.revision);
        let mut touched = BTreeMap::new();
        for entry in self.scan_every(layout::TOUCH, &[])? {
            touched.insert(
                layout::decode_touch_key(&entry.key)?,
                (layout::decode_touch(&entry.value)?, entry.revision),
            );
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
        let mut programs = BTreeMap::new();
        let marked = self.mark(roots.into_iter().chain(young).collect(), &mut programs)?;
        report.reachable = marked.len() as u64;

        let candidates = touched
            .iter()
            .filter(|(id, _)| !marked.contains(*id))
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        let objects = self.load_objects(&candidates)?;
        let mut found = BTreeMap::new();
        for (id, object) in candidates.iter().zip(objects) {
            if let Some(object) = object {
                found.insert(*id, object);
            }
        }
        let order = self.referrers_first(&candidates, &found, &mut programs)?;
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
            let (_, revision) = touched.get(&id).copied().ok_or(StoreError::CorruptStore(
                "touch key vanished during GC".to_owned(),
            ))?;
            let object = found.get(&id);
            let deletion = self.deletion_ops(id, revision, object)?;
            let size = deletion.len() as u64 + u64::from(object.is_some());
            let weight = deletion.iter().map(|op| op.bytes()).sum::<u64>();
            if !counted.is_empty() && !fits(ops + size, bytes + weight) {
                let full = std::mem::replace(&mut batch, deletion_batch(graph));
                self.apply_ops(full)?;
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
        if !batch_is_trivial(ops) {
            self.apply_ops(batch)?;
            for object in counted {
                count(report, object);
            }
        }
        Ok(())
    }

    fn roots(&self, now: u64) -> Result<Vec<Root>, StoreError> {
        let mut roots = Vec::new();
        for entry in self.scan_every(layout::REFS, &[])? {
            let commit = layout::decode_commit_target(&entry.value)?;
            roots.push((
                ObjectId::from_bytes(*commit.as_bytes()),
                Some(ObjectKind::Commit),
            ));
        }
        for entry in self.scan_every(layout::CATALOG_HEADS, &[])? {
            let (event, _) = layout::decode_catalog_head(&entry.value)?;
            roots.push((
                ObjectId::from_bytes(*event.as_bytes()),
                Some(ObjectKind::TimelineCatalogEvent),
            ));
        }
        for entry in self.scan_every(layout::ARCHIVES, &[])? {
            let manifest = layout::decode_archive(&entry.value)?;
            roots.push((
                ObjectId::from_bytes(*manifest.as_bytes()),
                Some(ObjectKind::TimelineArchiveManifest),
            ));
        }
        for entry in self.scan_every(layout::PINS, &[])? {
            let pin = layout::decode_pin(&entry.key, &entry.value)?;
            if pin.expires_at.is_none_or(|expires_at| expires_at > now) {
                roots.push((pin.object, None));
            }
        }
        for entry in self.scan_every(layout::EFFECTS, &[])? {
            let effect = layout::decode_effect(&entry.key, &entry.value)?;
            roots.push((
                ObjectId::from_bytes(*effect.origin_commit.as_bytes()),
                Some(ObjectKind::Commit),
            ));
            if let Some(response) = effect.status.response() {
                roots.push((response, Some(ObjectKind::EffectResponse)));
            }
        }
        for entry in self.scan_every(layout::COMPOUND_SAVES, &[])? {
            let manifest = layout::decode_compound_save(&entry.value)?;
            roots.push((
                ObjectId::from_bytes(*manifest.as_bytes()),
                Some(ObjectKind::CompoundSaveManifest),
            ));
        }
        roots.sort_unstable();
        roots.dedup_by_key(|(id, _)| *id);
        Ok(roots)
    }

    fn program_object(
        &self,
        artifact: ProgramArtifactId,
        programs: &mut BTreeMap<ProgramArtifactId, ObjectId>,
    ) -> Result<ObjectId, StoreError> {
        if let Some(id) = programs.get(&artifact) {
            return Ok(*id);
        }
        let id = self
            .find_program(artifact)?
            .ok_or(StoreError::InvalidGraph("Program Artifact is missing"))?;
        programs.insert(artifact, id);
        Ok(id)
    }

    /// Everything reachable from `start`. Only objects whose kind can refer to others are read.
    fn mark(
        &self,
        start: Vec<Root>,
        programs: &mut BTreeMap<ProgramArtifactId, ObjectId>,
    ) -> Result<BTreeSet<ObjectId>, StoreError> {
        let mut marked = BTreeSet::new();
        let mut frontier = start;
        while !frontier.is_empty() {
            let mut read = Vec::new();
            for (id, kind) in std::mem::take(&mut frontier) {
                if marked.insert(id) && kind.is_none_or(|kind| !is_leaf(kind)) {
                    read.push(id);
                }
            }
            for (id, object) in read.iter().zip(self.load_objects(&read)?) {
                let object = object.ok_or(StoreError::MissingObject(*id))?;
                for reference in references(&object)? {
                    frontier.push(match reference {
                        Reference::Object { id, kind, .. } => (id, kind),
                        Reference::Program(artifact) => (
                            self.program_object(artifact, programs)?,
                            Some(ObjectKind::Program),
                        ),
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
        found: &BTreeMap<ObjectId, CheckedObject>,
        programs: &mut BTreeMap<ProgramArtifactId, ObjectId>,
    ) -> Result<Vec<ObjectId>, StoreError> {
        let mut referents = BTreeMap::<ObjectId, Vec<ObjectId>>::new();
        let mut referrers = candidates
            .iter()
            .map(|id| (*id, 0_usize))
            .collect::<BTreeMap<_, _>>();
        for (id, object) in found {
            let mut targets = BTreeSet::new();
            for reference in references(object)? {
                let target = match reference {
                    Reference::Object { id, .. } => Some(id),
                    Reference::Program(artifact) => {
                        self.find_program(artifact)?.inspect(|target| {
                            programs.insert(artifact, *target);
                        })
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
        object: Option<&CheckedObject>,
    ) -> Result<Vec<Op>, StoreError> {
        let mut ops = vec![Op::delete(
            layout::TOUCH,
            layout::touch_key(id),
            Expect::Revision(touch),
            Tag::Replan,
        )];
        let Some(object) = object else {
            return Ok(ops);
        };
        match object.kind() {
            ObjectKind::Commit => {
                if let Ok(commit) = CommitV1::decode(object.payload()) {
                    let commit_id = CommitId::from_bytes(*id.as_bytes());
                    ops.push(Op::delete(
                        layout::COMMITS,
                        layout::commit_key(commit.execution, commit.turn.0, commit_id),
                        Expect::Any,
                        Tag::Index,
                    ));
                    if let Some(parent) = commit.parent {
                        ops.push(Op::delete(
                            layout::CHILDREN,
                            layout::child_key(parent, commit_id),
                            Expect::Any,
                            Tag::Index,
                        ));
                    }
                }
            }
            ObjectKind::Program => {
                if let Ok(program) =
                    narrata_core::program::load_program(object.bytes(), &Default::default())
                {
                    ops.extend(self.index_deletion(
                        layout::PROGRAMS,
                        layout::program_key(program.artifact_id()),
                        |value| Ok(layout::decode_program(value)? == id),
                    )?);
                }
            }
            ObjectKind::TimelineCatalogEvent => {
                if let Ok(event) = TimelineCatalogEventV1::decode(object.payload()) {
                    let event_id = TimelineCatalogEventId::from_bytes(*id.as_bytes());
                    ops.extend(self.index_deletion(
                        layout::CATALOG_OPERATIONS,
                        layout::catalog_operation_key(event.execution, event.operation),
                        |value| Ok(layout::decode_catalog_operation(value)?.1 == event_id),
                    )?);
                }
            }
            _ => {}
        }
        Ok(ops)
    }

    /// Deletes an index entry if it names the object being deleted.
    fn index_deletion(
        &self,
        space: narrata_storage::KeySpace,
        key: Vec<u8>,
        names_object: impl FnOnce(&[u8]) -> Result<bool, StoreError>,
    ) -> Result<Option<Op>, StoreError> {
        match self.read_key(space, &key)? {
            Some(value) if names_object(&value.value)? => Ok(Some(Op::delete(
                space,
                key,
                Expect::Revision(value.revision),
                Tag::Replan,
            ))),
            _ => Ok(None),
        }
    }

    pub(super) fn scan_integrity(&self) -> Result<Vec<IntegrityIssue>, StoreError> {
        let mut issues = Vec::new();
        let mut present = BTreeSet::new();
        let mut pending = Vec::new();
        let mut after = None;
        loop {
            let page = self
                .backend
                .scan_objects(after.as_ref(), self.read_items())?;
            let found = self.backend.get_objects(&page.digests)?;
            for (digest, bytes) in page.digests.iter().zip(found) {
                let id = ObjectId::from_bytes(*digest.as_bytes());
                let Some(bytes) = bytes else {
                    continue;
                };
                match verify(id, bytes) {
                    Ok(object) => {
                        present.insert(id);
                        if !is_leaf(object.kind()) {
                            match references(&object) {
                                Ok(references) => pending.push((id, references)),
                                Err(error) => issues.push(issue(id, error)),
                            }
                        }
                    }
                    Err(error @ StoreError::Corrupt(..)) => issues.push(issue(id, error)),
                    Err(error) => return Err(error),
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
                    Reference::Program(artifact) => self.find_program(artifact)?,
                };
                match target {
                    Some(target) if present.contains(&target) => {}
                    Some(target) => issues.push(IntegrityIssue {
                        object: id,
                        diagnostic: format!("missing referenced object {target}"),
                    }),
                    None => issues.push(IntegrityIssue {
                        object: id,
                        diagnostic: "referenced Program is not indexed".to_owned(),
                    }),
                }
            }
        }
        Ok(issues)
    }
}

fn issue(object: ObjectId, error: StoreError) -> IntegrityIssue {
    let diagnostic = match error {
        StoreError::Corrupt(_, diagnostic) => diagnostic,
        other => other.to_string(),
    };
    IntegrityIssue { object, diagnostic }
}

fn count(report: &mut GcReport, object: &CheckedObject) {
    let entry: &mut GcKindReport = report.removed.entry(object.kind().code()).or_default();
    entry.objects = entry.objects.saturating_add(1);
    entry.bytes = entry.bytes.saturating_add(object.bytes().len() as u64);
}

fn deletion_batch(graph: Option<Revision>) -> Ops {
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

const fn batch_is_trivial(ops: u64) -> bool {
    ops <= 2
}
