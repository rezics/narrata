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
#[derive(Clone, Copy, Debug)]
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

/// Proves a candidate belongs to a durable root's ancestry in this execution. An orphan or
/// another execution's root is insufficient, even when the store's revision has advanced.
pub fn retained<B: StorageBackend>(
    backend: &B,
    execution: ExecutionId,
    candidate: CommitId,
    limits: Limits,
) -> Result<bool, HistoryError> {
    layout(backend)?;
    let owner = RefName::hex(execution.as_bytes());
    let mut roots = BTreeSet::new();
    let mut count = 0;
    for namespace in [
        RefNamespace::Active,
        RefNamespace::Branch,
        RefNamespace::Bookmark,
        RefNamespace::Save,
    ] {
        let prefix = layout::ref_prefix(Some(namespace), Some(&owner));
        let mut after: Option<Vec<u8>> = None;
        loop {
            let page = backend.scan_keys(layout::REFS, &prefix, after.as_deref(), 128)?;
            if page.entries.len() > 128 || (page.more && page.entries.is_empty()) {
                return Err(corrupt("invalid root scan"));
            }
            let mut previous = after.clone();
            for entry in page.entries {
                count += 1;
                if count > limits.roots {
                    return Err(corrupt("retention root budget exceeded"));
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
                roots.insert(layout::decode_ref_target(&entry.value)?);
                previous = Some(entry.key);
            }
            if !page.more {
                break;
            }
            after = previous;
        }
    }
    let candidate = ObjectId::from_bytes(*candidate.as_bytes());
    let mut seen = BTreeMap::<ObjectId, u64>::new();
    for root in roots {
        let mut next = Some(root);
        let mut child_depth: Option<u64> = None;
        while let Some(id) = next {
            if let Some(depth) = seen.get(&id) {
                if child_depth.is_some_and(|child| depth.checked_add(1) != Some(child)) {
                    return Err(corrupt("history depth mismatch"));
                }
                break;
            }
            if seen.len() >= limits.commits {
                return Err(corrupt("retention commit budget exceeded"));
            }
            let commit = header(backend, id)?;
            if child_depth.is_some_and(|child| commit.depth.checked_add(1) != Some(child)) {
                return Err(corrupt("history depth mismatch"));
            }
            if id == candidate {
                return Ok(true);
            }
            seen.insert(id, commit.depth);
            child_depth = Some(commit.depth);
            next = commit.parent;
        }
    }
    Ok(false)
}
