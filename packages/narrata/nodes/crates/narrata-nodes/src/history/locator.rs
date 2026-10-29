//! Read-only discovery and retention proofs before a host has loaded a program.
use std::collections::{BTreeMap, BTreeSet};

use narrata_history::{
    Commit, HistoryError, Object, ObjectId, RefKey, RefName, RefNamespace, layout,
};
use narrata_storage::{ObjectDigest, StorageBackend};

use crate::{ArtifactId, CommitId, ExecutionId, wire};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Location {
    pub cursor: CommitId,
    pub artifact: ArtifactId,
}

/// Exhausting either budget is a failed verification, never evidence that a write was lost.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Limits {
    pub roots: usize,
    pub commits: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            roots: 4096,
            commits: 65_536,
        }
    }
}

fn corrupt(message: &str) -> HistoryError {
    HistoryError::CorruptStore(message.to_owned())
}

fn layout<B: StorageBackend>(backend: &B) -> Result<(), HistoryError> {
    if let Some(marker) = backend.read_key(layout::META, layout::LAYOUT_KEY)? {
        if layout::decode_layout(&marker.value)? != layout::LAYOUT_VERSION {
            return Err(corrupt("unsupported save layout"));
        }
    } else {
        // Unlike History::open this path neither initializes a store nor repairs missing metadata.
        if !backend.scan_objects(None, 1)?.digests.is_empty() {
            return Err(corrupt("objects without a save layout"));
        }
        for (space, _) in layout::SPACES {
            if !backend.scan_keys(space, &[], None, 1)?.entries.is_empty() {
                return Err(corrupt("keys without a save layout"));
            }
        }
    }
    Ok(())
}

fn header<B: StorageBackend>(backend: &B, id: ObjectId) -> Result<Commit, HistoryError> {
    let bytes = backend
        .get_object(&ObjectDigest::from_bytes(*id.as_bytes()))?
        .ok_or(HistoryError::MissingObject(id))?;
    // Node commits have fixed-size fields. Bound the envelope before checked decoding.
    if bytes.len() > 512 {
        return Err(corrupt("oversized node commit"));
    }
    Commit::from_object(&Object::verify(id, bytes)?, wire::KIND_COMMIT)
}

/// Only absence of the active cursor means a new journey. Bad/missing objects remain errors.
pub fn locate<B: StorageBackend>(
    backend: &B,
    execution: ExecutionId,
) -> Result<Option<Location>, HistoryError> {
    layout(backend)?;
    let key = RefKey::active(RefName::hex(execution.as_bytes()))
        .map_err(|error| corrupt(&error.to_string()))?;
    let Some(value) = backend.read_key(layout::REFS, &layout::ref_key(&key))? else {
        return Ok(None);
    };
    let id = layout::decode_ref_target(&value.value)?;
    let commit = header(backend, id)?;
    Ok(Some(Location {
        cursor: CommitId::from_bytes(*id.as_bytes()),
        artifact: ArtifactId::from_bytes(*commit.artifact.as_bytes()),
    }))
}

/// Identity of the snapshot supplying a retryable proof. Root mutations must advance revision.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProofScope {
    pub store: [u8; 16],
    pub revision: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ProofWork {
    pub headers: usize,
    pub roots: usize,
    pub polls: usize,
    pub resets: usize,
}

const NAMESPACES: [RefNamespace; 4] = [
    RefNamespace::Active,
    RefNamespace::Branch,
    RefNamespace::Bookmark,
    RefNamespace::Save,
];

#[derive(Default)]
struct Progress {
    namespace: usize,
    after: Option<Vec<u8>>,
    roots: BTreeSet<ObjectId>,
    root_count: usize,
    seen: BTreeMap<ObjectId, u64>,
    next: Option<ObjectId>,
    child_depth: Option<u64>,
    result: Option<bool>,
}

/// A resumable, read-only proof. A cache miss leaves its scan/ancestry cursor at the missing
/// item, so a cold chain verifies each header once rather than replaying its prefix per load.
/// Snapshot changes discard all evidence; cumulative work survives resets to bound churn.
pub struct RetentionProof {
    execution: ExecutionId,
    candidate: CommitId,
    limits: Limits,
    scope: Option<ProofScope>,
    progress: Progress,
    work: ProofWork,
}

impl RetentionProof {
    pub fn new(execution: ExecutionId, candidate: CommitId, limits: Limits) -> Self {
        Self {
            execution,
            candidate,
            limits,
            scope: None,
            progress: Progress::default(),
            work: ProofWork::default(),
        }
    }

    pub fn matches(&self, candidate: CommitId, limits: Limits) -> bool {
        self.candidate == candidate && self.limits == limits
    }

    pub fn work(&self) -> ProofWork {
        self.work
    }

    /// `scope` must describe the backend's current snapshot, including on every cache retry.
    pub fn poll<B: StorageBackend>(
        &mut self,
        backend: &B,
        scope: ProofScope,
    ) -> Result<bool, HistoryError> {
        self.work.polls = self.work.polls.saturating_add(1);
        if self.work.polls
            > self
                .limits
                .commits
                .saturating_add(self.limits.roots)
                .saturating_mul(4)
                .saturating_add(32)
        {
            return Err(corrupt("cumulative retention retry budget exceeded"));
        }
        if self.scope != Some(scope) {
            self.scope = Some(scope);
            self.progress = Progress::default();
            self.work.resets = self.work.resets.saturating_add(1);
        }
        layout(backend)?;
        if let Some(result) = self.progress.result {
            return Ok(result);
        }
        let owner = RefName::hex(self.execution.as_bytes());
        let page_size = backend.capabilities().limits.max_read_items.clamp(1, 128) as u32;
        while self.progress.namespace < NAMESPACES.len() {
            let namespace = NAMESPACES[self.progress.namespace];
            let prefix = layout::ref_prefix(Some(namespace), Some(&owner));
            let page = backend.scan_keys(
                layout::REFS,
                &prefix,
                self.progress.after.as_deref(),
                page_size,
            )?;
            if page.entries.len() > page_size as usize || (page.more && page.entries.is_empty()) {
                return Err(corrupt("invalid root scan"));
            }
            // Validate the complete page before retaining its cursor or roots.
            let mut previous = self.progress.after.clone();
            let mut roots = Vec::new();
            for entry in page.entries {
                self.work.roots = self.work.roots.saturating_add(1);
                if self.work.roots > self.limits.roots.saturating_mul(4) {
                    return Err(corrupt("cumulative retention root budget exceeded"));
                }
                if !entry.key.starts_with(&prefix)
                    || previous.as_ref().is_some_and(|key| entry.key <= *key)
                {
                    return Err(corrupt("unordered or out-of-scope root"));
                }
                let key = layout::decode_ref_key(&entry.key)?;
                if key.owner() != &owner || key.namespace() != namespace {
                    return Err(corrupt("root owner mismatch"));
                }
                roots.push(layout::decode_ref_target(&entry.value)?);
                previous = Some(entry.key);
            }
            self.progress.root_count += roots.len();
            if self.progress.root_count > self.limits.roots {
                return Err(corrupt("retention root budget exceeded"));
            }
            self.progress.roots.extend(roots);
            if page.more {
                self.progress.after = previous;
            } else {
                self.progress.namespace += 1;
                self.progress.after = None;
            }
        }
        loop {
            if self.progress.next.is_none() {
                self.progress.next = self.progress.roots.pop_first();
                self.progress.child_depth = None;
            }
            let Some(id) = self.progress.next else {
                self.progress.result = Some(false);
                return Ok(false);
            };
            if let Some(depth) = self.progress.seen.get(&id) {
                if self
                    .progress
                    .child_depth
                    .is_some_and(|child| depth.checked_add(1) != Some(child))
                {
                    return Err(corrupt("history depth mismatch"));
                }
                self.progress.next = None;
                continue;
            }
            if self.progress.seen.len() >= self.limits.commits
                || self.work.headers >= self.limits.commits.saturating_mul(4)
            {
                return Err(corrupt("retention commit budget exceeded"));
            }
            // A NotLoaded read changes no progress. Count actual checked headers, not loads.
            let bytes = backend
                .get_object(&ObjectDigest::from_bytes(*id.as_bytes()))?
                .ok_or(HistoryError::MissingObject(id))?;
            self.work.headers += 1;
            if bytes.len() > 512 {
                return Err(corrupt("oversized node commit"));
            }
            let commit = Commit::from_object(&Object::verify(id, bytes)?, wire::KIND_COMMIT)?;
            if self
                .progress
                .child_depth
                .is_some_and(|child| commit.depth.checked_add(1) != Some(child))
            {
                return Err(corrupt("history depth mismatch"));
            }
            if id.as_bytes() == self.candidate.as_bytes() {
                self.progress.result = Some(true);
                return Ok(true);
            }
            self.progress.seen.insert(id, commit.depth);
            self.progress.child_depth = Some(commit.depth);
            self.progress.next = commit.parent;
        }
    }
}

/// Single-call native convenience. Cache hosts keep a RetentionProof between retries instead.
pub fn retained<B: StorageBackend>(
    backend: &B,
    execution: ExecutionId,
    candidate: CommitId,
    limits: Limits,
) -> Result<bool, HistoryError> {
    RetentionProof::new(execution, candidate, limits).poll(
        backend,
        ProofScope {
            store: [0; 16],
            revision: 0,
        },
    )
}
