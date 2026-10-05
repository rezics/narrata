//! Write-time checks of the Stage 1–5 kinds (ADR 0014). The history engine has already checked
//! that every object a new object refers to exists with the required kind and that its Program
//! resolves; these checks decode the payloads and compare them with what they refer to.

use narrata_core::{
    CommitId, TimelineCatalogEventId,
    codec::ObjectKind,
    limits::SnapshotLoadLimits,
    snapshot::{restore_snapshot, state_digest},
};
use narrata_history::{Object, Op, Tag, View, layout::child_key};
use narrata_storage::{Expect, StorageBackend};

use super::registry::{Legacy, LegacyOp, PROGRAM, SNAPSHOT_SCHEMAS};
use crate::{
    CATALOG_EVENT_SCHEMA_V1, COMPOUND_SAVE_MANIFEST_SCHEMA_V1, CommitCauseV1, CommitV1,
    CompoundSaveManifestV1, EFFECT_RESPONSE_SCHEMA_V1, HOST_TIMELINE_MANIFEST_SCHEMA_V1,
    HostTimelineManifestV1, RecordedEffectResponseV1, STORED_RECEIPT_SCHEMA_V1, StoreError,
    TIMELINE_ARCHIVE_MANIFEST_SCHEMA_V1, TimelineArchiveManifestV1, TimelineCatalogEventV1,
    TransitionReceiptV1,
    commit::COMMIT_SCHEMA_V1,
    layout,
    object::{history_id, id},
};

type LegacyView<'v, 'a, B> = &'v mut View<'a, B, Legacy>;

fn corrupt(object: &Object, error: impl std::fmt::Display) -> StoreError {
    StoreError::Corrupt(id(object.id()), error.to_string())
}

fn commit<B: StorageBackend>(
    view: LegacyView<'_, '_, B>,
    bytes: &[u8; 32],
) -> Result<CommitV1, StoreError> {
    let object = view.require(history_id(bytes), Some(ObjectKind::Commit.code()))?;
    CommitV1::decode(object.payload()).map_err(|error| corrupt(&object, error))
}

/// Checks one new object and returns the index keys it adds.
pub(super) fn validate<B: StorageBackend>(
    legacy: &Legacy,
    object: &Object,
    view: LegacyView<'_, '_, B>,
) -> Result<Vec<LegacyOp>, StoreError> {
    let schema = |expected: u16| {
        if object.schema() == expected {
            Ok(())
        } else {
            Err(StoreError::ObjectKind(id(object.id())))
        }
    };
    let payload = object.payload();
    let Some(kind) = ObjectKind::from_code(object.kind()) else {
        return Err(StoreError::ObjectKind(id(object.id())));
    };
    match kind {
        ObjectKind::Program => return index_program(legacy, object, view),
        // A Snapshot's content is checked against its Program when a Commit names it.
        ObjectKind::Snapshot => {
            if !SNAPSHOT_SCHEMAS.contains(&object.schema()) {
                return Err(StoreError::ObjectKind(id(object.id())));
            }
        }
        // The history layer checks the checkpoint manifest, which it owns.
        ObjectKind::Value | ObjectKind::CheckpointBundleManifest => {}
        ObjectKind::Receipt => {
            schema(STORED_RECEIPT_SCHEMA_V1)?;
            TransitionReceiptV1::decode(payload).map_err(|error| corrupt(object, error))?;
        }
        ObjectKind::Commit => return validate_commit(legacy, object, view),
        ObjectKind::TimelineCatalogEvent => {
            schema(CATALOG_EVENT_SCHEMA_V1)?;
            let event =
                TimelineCatalogEventV1::decode(payload).map_err(|error| corrupt(object, error))?;
            return index_operation(
                view,
                &event,
                TimelineCatalogEventId::from_bytes(*object.id().as_bytes()),
            );
        }
        ObjectKind::TimelineArchiveManifest => {
            schema(TIMELINE_ARCHIVE_MANIFEST_SCHEMA_V1)?;
            let manifest = TimelineArchiveManifestV1::decode(payload)
                .map_err(|error| corrupt(object, error))?;
            let descriptors = manifest
                .objects
                .iter()
                .map(crate::ObjectDescriptor::descriptor)
                .collect::<Vec<_>>();
            view.check_descriptors(&descriptors)?;
        }
        ObjectKind::EffectResponse => {
            schema(EFFECT_RESPONSE_SCHEMA_V1)?;
            RecordedEffectResponseV1::decode(payload).map_err(|error| corrupt(object, error))?;
        }
        ObjectKind::CompoundSaveManifest => {
            schema(COMPOUND_SAVE_MANIFEST_SCHEMA_V1)?;
            let manifest =
                CompoundSaveManifestV1::decode(payload).map_err(|error| corrupt(object, error))?;
            let commit = commit(view, manifest.narrative.as_bytes())?;
            if commit.execution != manifest.execution
                || commit.program != manifest.program
                || commit.ledger_fence != manifest.ledger_fence.get()
            {
                return Err(StoreError::InvalidGraph("Compound Save mismatch"));
            }
        }
        ObjectKind::HostTimelineManifest => {
            schema(HOST_TIMELINE_MANIFEST_SCHEMA_V1)?;
            let manifest =
                HostTimelineManifestV1::decode(payload).map_err(|error| corrupt(object, error))?;
            for entry in &manifest.entries {
                let commit = commit(view, entry.narrative.as_bytes())?;
                if commit.execution != manifest.execution
                    || commit.ledger_fence != entry.ledger_fence.get()
                {
                    return Err(StoreError::InvalidGraph("Host Timeline mismatch"));
                }
            }
        }
    }
    Ok(Vec::new())
}

fn index_program<B: StorageBackend>(
    legacy: &Legacy,
    object: &Object,
    view: LegacyView<'_, '_, B>,
) -> Result<Vec<LegacyOp>, StoreError> {
    // Objects that do not load as a Program stay stored but unindexed; no Commit can name them.
    let Some(program) = legacy.program(object) else {
        return Ok(Vec::new());
    };
    let key = layout::program_key(program.artifact_id());
    if view.planned(layout::PROGRAMS, &key).is_some() {
        return Ok(Vec::new());
    }
    if let Some(value) = view.reader().read_key(layout::PROGRAMS, &key)? {
        // The first stored object for an artifact stays its entry.
        layout::decode_program(&value.value)?;
        return Ok(Vec::new());
    }
    let value = layout::encode_program(object.id());
    view.plan(layout::PROGRAMS, key.clone(), value.clone());
    Ok(vec![Op::put(
        layout::PROGRAMS,
        key,
        value,
        Expect::Absent,
        Tag::Replan,
    )])
}

fn index_operation<B: StorageBackend>(
    view: LegacyView<'_, '_, B>,
    event: &TimelineCatalogEventV1,
    id: TimelineCatalogEventId,
) -> Result<Vec<LegacyOp>, StoreError> {
    let record = (event.previous, id);
    let key = layout::catalog_operation_key(event.execution, event.operation);
    let recorded = match view.planned(layout::CATALOG_OPERATIONS, &key) {
        Some(value) => Some(layout::decode_catalog_operation(value)?),
        None => view
            .reader()
            .read_key(layout::CATALOG_OPERATIONS, &key)?
            .map(|value| layout::decode_catalog_operation(&value.value))
            .transpose()?,
    };
    match recorded {
        Some(recorded) if recorded == record => Ok(Vec::new()),
        Some(_) => Err(StoreError::InvalidGraph("TimelineOperationId conflict")),
        None => {
            let value = layout::encode_catalog_operation(event.previous, id);
            view.plan(layout::CATALOG_OPERATIONS, key.clone(), value.clone());
            Ok(vec![Op::put(
                layout::CATALOG_OPERATIONS,
                key,
                value,
                Expect::Absent,
                Tag::Replan,
            )])
        }
    }
}

fn validate_commit<B: StorageBackend>(
    legacy: &Legacy,
    object: &Object,
    view: LegacyView<'_, '_, B>,
) -> Result<Vec<LegacyOp>, StoreError> {
    if object.schema() != COMMIT_SCHEMA_V1 {
        return Err(StoreError::ObjectKind(id(object.id())));
    }
    let commit = CommitV1::decode(object.payload()).map_err(|error| corrupt(object, error))?;
    // The engine resolved the Program before calling; this reads the resolution back.
    let program_id = view
        .resolve(PROGRAM, commit.program.as_bytes())?
        .ok_or(StoreError::InvalidGraph("Program Artifact is missing"))?;
    let program_object = view.require(program_id, Some(PROGRAM))?;
    let program = legacy.program(&program_object).ok_or_else(|| {
        StoreError::CorruptStore(format!(
            "Program index entry {program_id} does not hold artifact {}",
            commit.program
        ))
    })?;
    let snapshot_object = view.require(
        history_id(commit.snapshot.as_bytes()),
        Some(ObjectKind::Snapshot.code()),
    )?;
    let limits = SnapshotLoadLimits::default();
    let state = restore_snapshot(snapshot_object.bytes(), &program, &limits)
        .map_err(|error| corrupt(&snapshot_object, error))?;
    if state.execution_id != commit.execution || state.turn != commit.turn {
        return Err(StoreError::InvalidGraph("Commit/Snapshot mismatch"));
    }
    match (commit.parent, commit.cause) {
        (None, CommitCauseV1::Genesis) if commit.turn.0 == 0 && commit.ledger_fence == 0 => {}
        (Some(parent_id), CommitCauseV1::RuntimeTransition(receipt_id)) => {
            let parent = self::commit(view, parent_id.as_bytes())?;
            if parent.execution != commit.execution
                || parent.program != commit.program
                || parent.turn.0.checked_add(1) != Some(commit.turn.0)
                || commit.ledger_fence < parent.ledger_fence
            {
                return Err(StoreError::InvalidGraph("Commit parent mismatch"));
            }
            let receipt_object = view.require(
                history_id(receipt_id.as_bytes()),
                Some(ObjectKind::Receipt.code()),
            )?;
            let receipt = TransitionReceiptV1::decode(receipt_object.payload())
                .map_err(|error| corrupt(&receipt_object, error))?;
            if receipt.execution != commit.execution
                || receipt.program != commit.program
                || receipt.parent != parent_id
                || receipt.next_snapshot != commit.snapshot
                || receipt.next_state != state_digest(&state)
            {
                return Err(StoreError::InvalidGraph("Commit/Receipt mismatch"));
            }
            let parent_snapshot = view.require(
                history_id(parent.snapshot.as_bytes()),
                Some(ObjectKind::Snapshot.code()),
            )?;
            let parent_state = restore_snapshot(parent_snapshot.bytes(), &program, &limits)
                .map_err(|error| corrupt(&parent_snapshot, error))?;
            if receipt.parent_state != state_digest(&parent_state) {
                return Err(StoreError::InvalidGraph("Receipt parent state mismatch"));
            }
            if state
                .pending_effect()
                .is_some_and(|pending| pending.origin_parent_commit != parent_id)
            {
                return Err(StoreError::InvalidGraph(
                    "pending Effect parent Commit mismatch",
                ));
            }
        }
        (Some(parent_id), CommitCauseV1::Migration(_)) => {
            let parent = self::commit(view, parent_id.as_bytes())?;
            if parent.execution != commit.execution
                || parent.turn != commit.turn
                || commit.ledger_fence != parent.ledger_fence
                || parent.program == commit.program
            {
                return Err(StoreError::InvalidGraph("migration Commit parent mismatch"));
            }
        }
        _ => return Err(StoreError::InvalidGraph("Commit cause/parent shape")),
    }
    let commit_id = CommitId::from_bytes(*object.id().as_bytes());
    let mut index = vec![Op::put(
        layout::COMMITS,
        layout::commit_key(commit.execution, commit.turn.0, commit_id),
        narrata_history::layout::encode_marker(),
        Expect::Any,
        Tag::Index,
    )];
    if let Some(parent) = commit.parent {
        index.push(Op::put(
            layout::CHILDREN,
            child_key(history_id(parent.as_bytes()), object.id()),
            narrata_history::layout::encode_marker(),
            Expect::Any,
            Tag::Index,
        ));
    }
    Ok(index)
}
