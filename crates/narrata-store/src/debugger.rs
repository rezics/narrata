use std::sync::Arc;

use narrata_core::{
    CommitId, ExecutionId, ObjectId, ProgramArtifactId, ReceiptId, SnapshotId, StateDigest,
    codec::ObjectKind,
    limits::{MacrostepLimits, SnapshotLoadLimits},
    runtime::{
        CheckedRuntimeInput, SliceBudget, SliceOutcome, begin_transition_with_parent_commit,
    },
    snapshot::{export_snapshot, restore_snapshot, state_digest},
};
use thiserror::Error;

use crate::{
    CheckedObject, CommitCauseV1, CommitV1, RefKey, RefScope, SaveStore, StoreError,
    TimelineCoverage, TransitionReceiptV1, scan_all,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimelineDebugNode {
    pub id: CommitId,
    pub parent: Option<CommitId>,
    pub program: ProgramArtifactId,
    pub turn: u64,
    pub cause: &'static str,
    pub ledger_fence: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimelineDebugView {
    pub execution: ExecutionId,
    pub coverage: Option<TimelineCoverage>,
    pub nodes: Vec<TimelineDebugNode>,
    pub refs: Vec<(RefKey, CommitId)>,
}

pub fn inspect_timeline(
    store: &impl SaveStore,
    execution: ExecutionId,
) -> Result<TimelineDebugView, StoreError> {
    let indexed = scan_all(
        |after| store.timeline_commits(execution, after, u32::MAX),
        |entry| *entry,
    )?;
    let ids = indexed
        .iter()
        .map(|(_, commit)| ObjectId::from_bytes(*commit.as_bytes()))
        .collect::<Vec<_>>();
    let mut nodes = Vec::new();
    for (id, object) in ids.iter().zip(store.get_objects(&ids)?) {
        let object = object.ok_or(StoreError::MissingObject(*id))?;
        if object.kind() != ObjectKind::Commit {
            return Err(StoreError::ObjectKind(*id));
        }
        let commit = CommitV1::decode(object.payload())
            .map_err(|error| StoreError::Corrupt(object.id(), error.to_string()))?;
        // The index is a hint; its entries must name Commits of this Execution.
        if commit.execution == execution {
            nodes.push(TimelineDebugNode {
                id: CommitId::from_bytes(*object.id().as_bytes()),
                parent: commit.parent,
                program: commit.program,
                turn: commit.turn.0,
                cause: match commit.cause {
                    CommitCauseV1::Genesis => "genesis",
                    CommitCauseV1::RuntimeTransition(_) => "transition",
                    CommitCauseV1::Migration(_) => "migration",
                },
                ledger_fence: commit.ledger_fence,
            });
        }
    }
    nodes.sort_by_key(|node| (node.turn, node.id));
    let mut refs = Vec::new();
    let stored = scan_all(
        |after| store.scan_refs(&RefScope::All, after, u32::MAX),
        |(key, _)| key.clone(),
    )?;
    for (key, value) in stored {
        let object = store.get_object(ObjectId::from_bytes(*value.commit.as_bytes()))?;
        if object
            .and_then(|object| CommitV1::decode(object.payload()).ok())
            .is_some_and(|commit| commit.execution == execution)
        {
            refs.push((key, value.commit));
        }
    }
    refs.sort_by_key(|entry| entry.0.storage_key());
    let coverage = store
        .read_catalog_head(&crate::CatalogRefKey::new(execution))?
        .map(|value| value.coverage);
    Ok(TimelineDebugView {
        execution,
        coverage,
        nodes,
        refs,
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReceiptVerification {
    pub receipt: ReceiptId,
    pub parent_state: StateDigest,
    pub next_state: StateDigest,
}

#[derive(Debug, Error)]
pub enum ReceiptVerificationError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("Receipt or Commit relationship is invalid: {0}")]
    Invalid(&'static str),
    #[error("Snapshot is invalid: {0}")]
    Snapshot(String),
    #[error("Receipt replay failed: {0}")]
    Replay(String),
}

pub fn verify_receipt_chain(
    store: &impl SaveStore,
    receipt_id: ReceiptId,
    program: &narrata_core::CheckedProgram,
) -> Result<ReceiptVerification, ReceiptVerificationError> {
    let receipt_object = store
        .get_object(ObjectId::from_bytes(*receipt_id.as_bytes()))?
        .ok_or(StoreError::MissingObject(ObjectId::from_bytes(
            *receipt_id.as_bytes(),
        )))?;
    if receipt_object.kind() != ObjectKind::Receipt {
        return Err(ReceiptVerificationError::Invalid("object kind"));
    }
    let receipt = TransitionReceiptV1::decode(receipt_object.payload())
        .map_err(|_| ReceiptVerificationError::Invalid("Receipt encoding"))?;
    let parent_object = store
        .get_object(ObjectId::from_bytes(*receipt.parent.as_bytes()))?
        .ok_or(StoreError::MissingObject(ObjectId::from_bytes(
            *receipt.parent.as_bytes(),
        )))?;
    let parent = CommitV1::decode(parent_object.payload())
        .map_err(|_| ReceiptVerificationError::Invalid("parent Commit"))?;
    let parent_snapshot = store
        .get_object(ObjectId::from_bytes(*parent.snapshot.as_bytes()))?
        .ok_or(StoreError::MissingObject(ObjectId::from_bytes(
            *parent.snapshot.as_bytes(),
        )))?;
    let parent_state = restore_snapshot(
        parent_snapshot.bytes(),
        program,
        &SnapshotLoadLimits::default(),
    )
    .map_err(|error| ReceiptVerificationError::Snapshot(error.to_string()))?;
    let next_snapshot = store
        .get_object(ObjectId::from_bytes(*receipt.next_snapshot.as_bytes()))?
        .ok_or(StoreError::MissingObject(ObjectId::from_bytes(
            *receipt.next_snapshot.as_bytes(),
        )))?;
    let next_state = restore_snapshot(
        next_snapshot.bytes(),
        program,
        &SnapshotLoadLimits::default(),
    )
    .map_err(|error| ReceiptVerificationError::Snapshot(error.to_string()))?;
    let parent_digest = state_digest(&parent_state);
    let next_digest = state_digest(&next_state);
    if receipt.program != program.artifact_id()
        || receipt.parent_state != parent_digest
        || receipt.next_state != next_digest
    {
        return Err(ReceiptVerificationError::Invalid("state digest"));
    }
    let children = scan_all(
        |after| store.child_commits(receipt.parent, after, u32::MAX),
        |child| *child,
    )?
    .into_iter()
    .map(|child| ObjectId::from_bytes(*child.as_bytes()))
    .collect::<Vec<_>>();
    let matching_commits = store
        .get_objects(&children)?
        .into_iter()
        .flatten()
        .filter(|object| object.kind() == ObjectKind::Commit)
        .filter_map(|object| CommitV1::decode(object.payload()).ok())
        .filter(|commit| {
            commit.parent == Some(receipt.parent)
                && commit.program == receipt.program
                && commit.execution == receipt.execution
                && commit.snapshot == receipt.next_snapshot
                && commit.cause == CommitCauseV1::RuntimeTransition(receipt_id)
        })
        .count();
    if matching_commits != 1 {
        return Err(ReceiptVerificationError::Invalid("child Commit"));
    }
    Ok(ReceiptVerification {
        receipt: receipt_id,
        parent_state: parent_digest,
        next_state: next_digest,
    })
}

pub fn verify_receipt_replay(
    store: &impl SaveStore,
    receipt_id: ReceiptId,
    program: Arc<narrata_core::CheckedProgram>,
    input: CheckedRuntimeInput,
    limits: MacrostepLimits,
) -> Result<ReceiptVerification, ReceiptVerificationError> {
    let verified = verify_receipt_chain(store, receipt_id, &program)?;
    let receipt_object = store
        .get_object(ObjectId::from_bytes(*receipt_id.as_bytes()))?
        .ok_or(StoreError::MissingObject(ObjectId::from_bytes(
            *receipt_id.as_bytes(),
        )))?;
    let receipt = TransitionReceiptV1::decode(receipt_object.payload())
        .map_err(|_| ReceiptVerificationError::Invalid("Receipt encoding"))?;
    if input.request_id() != receipt.request_id
        || input.payload_digest() != receipt.input_payload_digest
    {
        return Err(ReceiptVerificationError::Invalid("replay input"));
    }
    let parent_object = store
        .get_object(ObjectId::from_bytes(*receipt.parent.as_bytes()))?
        .ok_or(StoreError::MissingObject(ObjectId::from_bytes(
            *receipt.parent.as_bytes(),
        )))?;
    let parent = CommitV1::decode(parent_object.payload())
        .map_err(|_| ReceiptVerificationError::Invalid("parent Commit"))?;
    let parent_snapshot = store
        .get_object(ObjectId::from_bytes(*parent.snapshot.as_bytes()))?
        .ok_or(StoreError::MissingObject(ObjectId::from_bytes(
            *parent.snapshot.as_bytes(),
        )))?;
    let parent_state = restore_snapshot(
        parent_snapshot.bytes(),
        &program,
        &SnapshotLoadLimits::default(),
    )
    .map_err(|error| ReceiptVerificationError::Snapshot(error.to_string()))?;
    let outcome = begin_transition_with_parent_commit(
        program.clone(),
        Arc::new(parent_state),
        receipt.parent,
        input,
        limits,
    )
    .map_err(|error| ReceiptVerificationError::Replay(error.to_string()))?
    .run_slice(SliceBudget::unlimited());
    let draft = match outcome {
        SliceOutcome::Completed(draft) => draft,
        SliceOutcome::Faulted(error) => {
            return Err(ReceiptVerificationError::Replay(error.to_string()));
        }
        SliceOutcome::Yielded { .. } => {
            return Err(ReceiptVerificationError::Replay(
                "unlimited replay yielded".to_owned(),
            ));
        }
    };
    let snapshot_bytes = export_snapshot(draft.next_state())
        .map_err(|error| ReceiptVerificationError::Snapshot(error.to_string()))?;
    let snapshot_object = CheckedObject::from_bytes(
        &snapshot_bytes,
        ObjectKind::Snapshot,
        0,
        SnapshotLoadLimits::default().decode.max_envelope_bytes,
    )
    .map_err(|error| ReceiptVerificationError::Snapshot(error.to_string()))?;
    let replayed_snapshot = SnapshotId::from_bytes(*snapshot_object.id().as_bytes());
    let replayed_receipt = TransitionReceiptV1::from_draft(
        receipt.execution,
        receipt.program,
        receipt.parent,
        replayed_snapshot,
        &draft,
    );
    if replayed_receipt != receipt {
        return Err(ReceiptVerificationError::Invalid("replayed Receipt"));
    }
    let stored_snapshot = store
        .get_object(ObjectId::from_bytes(*receipt.next_snapshot.as_bytes()))?
        .ok_or(StoreError::MissingObject(ObjectId::from_bytes(
            *receipt.next_snapshot.as_bytes(),
        )))?;
    if stored_snapshot.bytes() != snapshot_bytes {
        return Err(ReceiptVerificationError::Invalid("replayed Snapshot"));
    }
    Ok(verified)
}
