#![allow(clippy::panic, clippy::unwrap_used)]

use std::{path::Path, sync::Arc};

use narrata_core::{
    CommitId, ExecutionId, InputId, ObjectId, ProgramArtifactId, Value, builtin_host_capabilities,
    codec::encode_canonical_value,
    program::{ProgramArtifactV0, encode_program_artifact, load_program},
    runtime::{
        CheckedRuntimeInput, SliceBudget, SliceOutcome, begin_transition_with_parent_commit,
    },
    snapshot::restore_snapshot,
};
use narrata_protocol::{
    ProtocolEngine,
    dto::{self, request, response, runtime_input},
};
use narrata_storage::{StorageBackend, StorageError};
use narrata_storage_sqlite::SqliteBackend;
use narrata_store::{
    BranchId, CoordinatorError, MemoryStore, RefKey, RefName, SaveStore, SessionCoordinator, Store,
};
use narrata_testkit::generator::{branch_call_choice_v1, recorded_query_v1};
use prost::Message;

fn call<B: StorageBackend>(engine: &mut ProtocolEngine<B>, body: request::Body) -> response::Body {
    let bytes = dto::Request {
        protocol_version: narrata_protocol::PROTOCOL_VERSION,
        request_id: 7,
        body: Some(body),
    }
    .encode_to_vec();
    let response = dto::Response::decode(engine.handle_bytes(&bytes).unwrap().as_slice()).unwrap();
    assert_eq!(response.request_id, 7);
    response.body.unwrap()
}

fn load<B: StorageBackend>(
    engine: &mut ProtocolEngine<B>,
    artifact: &ProgramArtifactV0,
) -> ProgramArtifactId {
    let response::Body::ProgramLoaded(value) = call(
        engine,
        request::Body::ProgramLoad(dto::ProgramLoad {
            artifact: encode_program_artifact(artifact),
        }),
    ) else {
        panic!("expected loaded Program")
    };
    ProgramArtifactId::from_bytes(value.artifact_id.try_into().unwrap())
}

fn create<B: StorageBackend>(
    engine: &mut ProtocolEngine<B>,
    artifact: ProgramArtifactId,
    execution: ExecutionId,
) -> dto::SessionCreated {
    let response::Body::SessionCreated(value) = call(
        engine,
        request::Body::SessionCreate(dto::SessionCreate {
            artifact_id: artifact.as_bytes().to_vec(),
            execution_id: execution.as_bytes().to_vec(),
        }),
    ) else {
        panic!("expected created session")
    };
    value
}

fn input(id: u128, kind: runtime_input::Kind) -> dto::RuntimeInput {
    dto::RuntimeInput {
        request_id: InputId::from_u128(id).as_bytes().to_vec(),
        kind: Some(kind),
    }
}

fn start() -> dto::RuntimeInput {
    input(1, runtime_input::Kind::Start(dto::Start {}))
}

fn advance(previous: &dto::CommittedRunResult) -> dto::RuntimeInput {
    input(
        2,
        runtime_input::Kind::Advance(dto::Advance {
            interaction_id: previous
                .result
                .as_ref()
                .unwrap()
                .interaction_or_effect_id
                .clone(),
        }),
    )
}

fn dispatch<B: StorageBackend>(
    engine: &mut ProtocolEngine<B>,
    session: u64,
    input: dto::RuntimeInput,
) -> dto::CommittedRunResult {
    let response::Body::Committed(value) = call(
        engine,
        request::Body::Dispatch(dto::Dispatch {
            session,
            input: Some(input),
            slice_work: 0,
        }),
    ) else {
        panic!("expected committed transition")
    };
    value
}

fn sqlite(path: &Path) -> ProtocolEngine<SqliteBackend> {
    let path = path.to_owned();
    ProtocolEngine::with_backend_factory(Default::default(), move |_| SqliteBackend::open(&path))
}

fn active(path: &Path) -> CommitId {
    Store::open(SqliteBackend::open(path).unwrap())
        .unwrap()
        .read_ref(&RefKey::active(RefName::new("protocol").unwrap()).unwrap())
        .unwrap()
        .unwrap()
        .commit
}

#[test]
fn sqlite_session_reopens_and_continues_with_memory_identical_commits() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("session.db");
    let execution = ExecutionId::from_u128(90);
    let artifact = branch_call_choice_v1();
    let mut persistent = sqlite(&path);
    let mut memory = ProtocolEngine::default();
    let id = load(&mut persistent, &artifact);
    assert_eq!(id, load(&mut memory, &artifact));
    let saved = create(&mut persistent, id, execution);
    assert_eq!(saved, create(&mut memory, id, execution));
    let first = dispatch(&mut persistent, saved.session, start());
    assert_eq!(first, dispatch(&mut memory, saved.session, start()));
    drop(persistent);

    let mut reopened = sqlite(&path);
    load(&mut reopened, &artifact);
    let restored = reopened
        .reopen_session(id, execution, BranchId::from_bytes(*execution.as_bytes()))
        .unwrap();
    assert_eq!(restored.commit_id, first.commit_id);
    assert_eq!(restored.snapshot, first.snapshot);
    assert_eq!(
        dispatch(&mut reopened, restored.session, advance(&first)),
        dispatch(&mut memory, saved.session, advance(&first))
    );
    assert_eq!(
        active(&path).as_bytes().as_slice(),
        dispatch_checkpoint(&mut reopened, restored.session)
            .manifest
            .root
            .as_bytes()
    );
}

fn dispatch_checkpoint<B: StorageBackend>(
    engine: &mut ProtocolEngine<B>,
    session: u64,
) -> narrata_store::CheckpointBundle {
    let response::Body::Bundle(bundle) = call(
        engine,
        request::Body::CheckpointExport(dto::CheckpointExport { session }),
    ) else {
        panic!("expected Checkpoint")
    };
    narrata_store::CheckpointBundle::from_bytes(&bundle.bytes, Default::default()).unwrap()
}

#[test]
fn sqlite_slices_publish_only_on_completion_and_conflicts_leave_refs_unchanged() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("session.db");
    let mut engine = sqlite(&path);
    let artifact = load(&mut engine, &branch_call_choice_v1());
    let created = create(&mut engine, artifact, ExecutionId::from_u128(91));
    let genesis = active(&path);
    let mut body = call(
        &mut engine,
        request::Body::Dispatch(dto::Dispatch {
            session: created.session,
            input: Some(start()),
            slice_work: 1,
        }),
    );
    assert!(matches!(body, response::Body::SliceYielded(_)));
    for body in [
        request::Body::CheckpointExport(dto::CheckpointExport {
            session: created.session,
        }),
        request::Body::TimelineArchiveExport(dto::TimelineArchiveExport {
            session: created.session,
        }),
        request::Body::Dispatch(dto::Dispatch {
            session: created.session,
            input: Some(start()),
            slice_work: 1,
        }),
    ] {
        assert!(
            matches!(call(&mut engine, body), response::Body::Diagnostic(dto::Diagnostic { ref code, .. }) if code == "NAR-P0004")
        );
    }
    while matches!(body, response::Body::SliceYielded(_)) {
        assert_eq!(active(&path), genesis);
        body = call(
            &mut engine,
            request::Body::ContinueSlice(dto::ContinueSlice {
                session: created.session,
                slice_work: 1,
            }),
        );
    }
    let response::Body::Committed(committed) = body else {
        panic!("expected commit")
    };
    let head = active(&path);
    assert_eq!(head.as_bytes().as_slice(), committed.commit_id);
    assert_ne!(head, genesis);
    assert!(
        matches!(call(&mut engine, request::Body::Dispatch(dto::Dispatch { session: created.session, input: Some(start()), slice_work: 0 })), response::Body::Diagnostic(dto::Diagnostic { ref code, .. }) if code == "NAR-P0004")
    );
    assert_eq!(active(&path), head);
}

#[test]
fn sqlite_replay_uses_coordinator_input_index_and_rejects_changed_payload() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("session.db");
    let execution = ExecutionId::from_u128(92);
    let mut engine = sqlite(&path);
    let artifact = branch_call_choice_v1();
    let id = load(&mut engine, &artifact);
    let created = create(&mut engine, id, execution);
    let first = dispatch(&mut engine, created.session, start());
    drop(engine);
    let program = load_program(&encode_program_artifact(&artifact), &Default::default()).unwrap();
    let mut coordinator = SessionCoordinator::open_with_capabilities(
        Store::open(SqliteBackend::open(&path).unwrap()).unwrap(),
        program,
        execution,
        RefName::new("protocol").unwrap(),
        BranchId::from_bytes(*execution.as_bytes()),
        &builtin_host_capabilities().unwrap(),
    )
    .unwrap();
    let genesis = CommitId::from_bytes(created.commit_id.try_into().unwrap());
    coordinator.rewind_to(genesis, 0).unwrap();
    drop(coordinator);
    let mut engine = sqlite(&path);
    load(&mut engine, &artifact);
    let restored = engine
        .reopen_session(id, execution, BranchId::from_bytes(*execution.as_bytes()))
        .unwrap();
    let changed = input(
        1,
        runtime_input::Kind::Event(dto::Event {
            event_type_id: vec![0; 16],
        }),
    );
    assert!(
        matches!(call(&mut engine, request::Body::Dispatch(dto::Dispatch { session: restored.session, input: Some(changed), slice_work: 0 })), response::Body::Diagnostic(dto::Diagnostic { ref code, .. }) if code == "NAR-P0004")
    );
    assert_eq!(active(&path), genesis);
    let reused = dispatch(&mut engine, restored.session, start());
    assert!(reused.reused);
    assert_eq!(reused.commit_id, first.commit_id);
    assert_eq!(reused.receipt_id, first.receipt_id);
    assert_eq!(reused.snapshot, first.snapshot);
    assert_eq!(reused.result, first.result);
    assert_eq!(active(&path).as_bytes().as_slice(), first.commit_id);
}

#[test]
fn checkpoint_and_timeline_imports_use_injected_sqlite_backend() {
    let mut memory = ProtocolEngine::default();
    let artifact = branch_call_choice_v1();
    let execution = ExecutionId::from_u128(93);
    let id = load(&mut memory, &artifact);
    let created = create(&mut memory, id, execution);
    let first = dispatch(&mut memory, created.session, start());
    let expected = dispatch(&mut memory, created.session, advance(&first));
    // Export after the first transition in a separate, identical session.
    let mut source = ProtocolEngine::default();
    load(&mut source, &artifact);
    let source_session = create(&mut source, id, execution);
    dispatch(&mut source, source_session.session, start());
    for archive in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("import.db");
        let request = if archive {
            request::Body::TimelineArchiveExport(dto::TimelineArchiveExport {
                session: source_session.session,
            })
        } else {
            request::Body::CheckpointExport(dto::CheckpointExport {
                session: source_session.session,
            })
        };
        let response::Body::Bundle(bundle) = call(&mut source, request) else {
            panic!("expected bundle")
        };
        let mut target = sqlite(&path);
        if !archive {
            load(&mut target, &artifact);
        }
        let request = if archive {
            request::Body::TimelineArchiveImport(dto::TimelineArchiveImport {
                bundle: bundle.bytes,
            })
        } else {
            request::Body::SessionLoad(dto::SessionLoad {
                artifact_id: id.as_bytes().to_vec(),
                checkpoint_bundle: bundle.bytes,
            })
        };
        let response::Body::SessionCreated(imported) = call(&mut target, request) else {
            panic!("expected imported session")
        };
        assert_eq!(imported.commit_id, first.commit_id);
        assert_eq!(imported.snapshot, first.snapshot);
        drop(target);
        let mut reopened = sqlite(&path);
        load(&mut reopened, &artifact);
        let restored = reopened
            .reopen_session(id, execution, BranchId::from_bytes(*execution.as_bytes()))
            .unwrap();
        let next = dispatch(&mut reopened, restored.session, advance(&first));
        assert_eq!(next, expected);
        assert!(matches!(
            call(
                &mut reopened,
                request::Body::TimelineArchiveExport(dto::TimelineArchiveExport {
                    session: restored.session
                })
            ),
            response::Body::Bundle(_)
        ));
    }
}

#[test]
fn malformed_bundles_are_rejected_before_opening_backend_and_backend_errors_are_typed() {
    let mut engine = ProtocolEngine::with_backend_factory(
        Default::default(),
        |_| -> Result<SqliteBackend, StorageError> { panic!("bad input must not open storage") },
    );
    let id = load(&mut engine, &branch_call_choice_v1());
    for body in [
        request::Body::CheckpointImport(dto::CheckpointImport {
            artifact_id: id.as_bytes().to_vec(),
            bundle: vec![0; 12],
        }),
        request::Body::TimelineArchiveImport(dto::TimelineArchiveImport {
            bundle: vec![0; 12],
        }),
        request::Body::SessionCreate(dto::SessionCreate {
            artifact_id: id.as_bytes().to_vec(),
            execution_id: vec![0; 15],
        }),
    ] {
        assert!(
            matches!(call(&mut engine, body), response::Body::Diagnostic(dto::Diagnostic { ref code, .. }) if code == "NAR-P0001")
        );
    }
    let mut engine = ProtocolEngine::with_backend_factory(
        Default::default(),
        |_| -> Result<SqliteBackend, StorageError> { Err(StorageError::Busy) },
    );
    let id = load(&mut engine, &branch_call_choice_v1());
    assert!(
        matches!(call(&mut engine, request::Body::SessionCreate(dto::SessionCreate { artifact_id: id.as_bytes().to_vec(), execution_id: vec![0; 16] })), response::Body::Diagnostic(dto::Diagnostic { ref code, retryable: true, .. }) if code == "NAR-P0008")
    );
}

#[test]
fn oversized_genesis_snapshot_is_rejected_before_opening_backend() {
    let mut limits = narrata_protocol::ProtocolLimits::default();
    limits.snapshot.decode.max_envelope_bytes = 1;
    let mut engine =
        ProtocolEngine::with_backend_factory(limits, |_| -> Result<SqliteBackend, StorageError> {
            panic!("oversized Genesis must not open storage")
        });
    let id = load(&mut engine, &branch_call_choice_v1());
    assert!(
        matches!(call(&mut engine, request::Body::SessionCreate(dto::SessionCreate {
        artifact_id: id.as_bytes().to_vec(), execution_id: vec![0; 16],
    })), response::Body::Diagnostic(dto::Diagnostic { ref code, .. }) if code == "NAR-P0005")
    );
}

#[test]
fn pull_protocol_effect_response_remains_checked_without_host_ledger_callbacks() {
    let artifact = recorded_query_v1().unwrap();
    let mut engine = ProtocolEngine::default();
    let id = load(&mut engine, &artifact);
    let created = create(&mut engine, id, ExecutionId::from_u128(94));
    let first = dispatch(&mut engine, created.session, start());
    let program = load_program(&encode_program_artifact(&artifact), &Default::default()).unwrap();
    let state = restore_snapshot(&first.snapshot, &program, &Default::default()).unwrap();
    let effect = &state.pending_effect().unwrap().request;
    let mut response = dto::EffectResponse {
        effect_id: effect.id.as_bytes().to_vec(),
        request_digest: effect.request_digest.as_bytes().to_vec(),
        capability: effect.capability.to_string(),
        capability_version: u32::from(effect.capability_version.get()),
        canonical_value: encode_canonical_value(&Value::I64(42)),
    };
    response.request_digest[0] ^= 1;
    assert!(matches!(
        call(
            &mut engine,
            request::Body::Dispatch(dto::Dispatch {
                session: created.session,
                input: Some(input(
                    2,
                    runtime_input::Kind::EffectResponse(response.clone())
                )),
                slice_work: 0
            })
        ),
        response::Body::Diagnostic(_)
    ));
    response.request_digest[0] ^= 1;
    let next = dispatch(
        &mut engine,
        created.session,
        input(2, runtime_input::Kind::EffectResponse(response)),
    );
    assert_eq!(next.result.unwrap().kind, dto::RunResultKind::Say as i32);
}

#[test]
fn coordinator_rejects_stale_and_foreign_completed_drafts_without_writes() {
    let program = load_program(
        &encode_program_artifact(&branch_call_choice_v1()),
        &Default::default(),
    )
    .unwrap();
    let execution = ExecutionId::from_u128(95);
    let mut coordinator = SessionCoordinator::create(
        MemoryStore::new(),
        program.clone(),
        execution,
        RefName::new("session").unwrap(),
        BranchId::from_u128(95),
        narrata_store::InitialRecordingMode::Complete,
        0,
    )
    .unwrap();
    let genesis = coordinator.timeline().cursor;
    let state = coordinator.state().clone();
    let make_draft = |state: Arc<_>, parent| {
        let runner = begin_transition_with_parent_commit(
            program.clone(),
            state,
            parent,
            CheckedRuntimeInput::start(InputId::from_u128(1)),
            Default::default(),
        )
        .unwrap();
        let SliceOutcome::Completed(draft) = runner.run_slice(SliceBudget::unlimited()) else {
            panic!("expected completed draft")
        };
        draft
    };
    let draft = make_draft(state.clone(), genesis);
    let foreign = Arc::new(
        narrata_core::runtime::new_execution(&program, ExecutionId::from_u128(96)).unwrap(),
    );
    assert!(matches!(
        coordinator.commit_draft(genesis, make_draft(foreign, genesis), 0),
        Err(CoordinatorError::IncompatibleCommit)
    ));
    assert_eq!(coordinator.timeline().cursor, genesis);
    let committed = coordinator.commit_draft(genesis, draft.clone(), 0).unwrap();
    assert!(matches!(
        coordinator.commit_draft(genesis, draft, 0),
        Err(CoordinatorError::IncompatibleCommit)
    ));
    assert_eq!(coordinator.timeline().cursor, committed.commit);
    assert!(
        coordinator
            .store()
            .get_object(ObjectId::from_bytes(*committed.commit.as_bytes()))
            .unwrap()
            .is_some()
    );
}
