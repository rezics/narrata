#![allow(clippy::panic, clippy::unwrap_used)]

use std::sync::Arc;

use narrata_core::{
    ExecutionId, InputId,
    program::encode_program_artifact,
    runtime::{CheckedRuntimeInput, SliceBudget, new_execution},
};
use narrata_protocol::{
    ProtocolEngine,
    dto::{
        self, CheckpointExport, Dispatch, ProgramLoad, Request, SessionCreate, Start, request,
        response, runtime_input,
    },
};
use narrata_testkit::{
    backend::{ConformanceBackend, NativeBackend},
    generator::branch_call_choice_v1,
};
use prost::Message;

fn call(engine: &mut ProtocolEngine, request_id: u64, body: request::Body) -> response::Body {
    let bytes = Request {
        protocol_version: narrata_protocol::PROTOCOL_VERSION,
        request_id,
        body: Some(body),
    }
    .encode_to_vec();
    let response = dto::Response::decode(engine.handle_bytes(&bytes).unwrap().as_slice()).unwrap();
    assert_eq!(response.request_id, request_id);
    response.body.unwrap()
}

#[test]
fn protobuf_pull_protocol_matches_native_state_hash_and_round_trips_checkpoint() {
    let artifact = branch_call_choice_v1();
    let artifact_bytes = encode_program_artifact(&artifact);
    let checked = narrata_core::load_program(&artifact_bytes, &Default::default()).unwrap();
    let execution = ExecutionId::from_u128(70);
    let native = NativeBackend
        .transition(
            checked.clone(),
            Arc::new(new_execution(&checked, execution).unwrap()),
            CheckedRuntimeInput::start(InputId::from_u128(1)),
            Default::default(),
            SliceBudget::unlimited(),
        )
        .unwrap();

    let mut engine = ProtocolEngine::default();
    let artifact_id = match call(
        &mut engine,
        1,
        request::Body::ProgramLoad(ProgramLoad {
            artifact: artifact_bytes,
        }),
    ) {
        response::Body::ProgramLoaded(value) => value.artifact_id,
        other => panic!("unexpected response: {other:?}"),
    };
    let session = match call(
        &mut engine,
        2,
        request::Body::SessionCreate(SessionCreate {
            artifact_id: artifact_id.clone(),
            execution_id: execution.as_bytes().to_vec(),
        }),
    ) {
        response::Body::SessionCreated(value) => value.session,
        other => panic!("unexpected response: {other:?}"),
    };
    let input = dto::RuntimeInput {
        request_id: InputId::from_u128(1).as_bytes().to_vec(),
        kind: Some(runtime_input::Kind::Start(Start {})),
    };
    let mut body = call(
        &mut engine,
        3,
        request::Body::Dispatch(Dispatch {
            session,
            input: Some(input),
            slice_work: 1,
        }),
    );
    let mut request_id = 4;
    while matches!(body, response::Body::SliceYielded(_)) {
        body = call(
            &mut engine,
            request_id,
            request::Body::ContinueSlice(dto::ContinueSlice {
                session,
                slice_work: 1,
            }),
        );
        request_id += 1;
    }
    let committed = match body {
        response::Body::Committed(value) => value,
        other => panic!("unexpected response: {other:?}"),
    };
    assert_eq!(
        committed.state_digest,
        native.next_state_digest().as_bytes().to_vec()
    );

    let bundle = match call(
        &mut engine,
        request_id,
        request::Body::CheckpointExport(CheckpointExport { session }),
    ) {
        response::Body::Bundle(value) => value.bytes,
        other => panic!("unexpected response: {other:?}"),
    };
    let imported = match call(
        &mut engine,
        request_id + 1,
        request::Body::CheckpointImport(dto::CheckpointImport {
            artifact_id,
            bundle,
        }),
    ) {
        response::Body::SessionCreated(value) => value,
        other => panic!("unexpected response: {other:?}"),
    };
    assert_eq!(imported.commit_id, committed.commit_id);
    assert_eq!(imported.snapshot, committed.snapshot);

    let archive = match call(
        &mut engine,
        request_id + 2,
        request::Body::TimelineArchiveExport(dto::TimelineArchiveExport { session }),
    ) {
        response::Body::Bundle(value) => {
            assert_eq!(value.kind, dto::BundleKind::TimelineArchive as i32);
            value.bytes
        }
        other => panic!("unexpected response: {other:?}"),
    };
    let imported_archive = match call(
        &mut engine,
        request_id + 3,
        request::Body::TimelineArchiveImport(dto::TimelineArchiveImport { bundle: archive }),
    ) {
        response::Body::SessionCreated(value) => value,
        other => panic!("unexpected response: {other:?}"),
    };
    assert_eq!(imported_archive.commit_id, committed.commit_id);
    assert_eq!(imported_archive.snapshot, committed.snapshot);
}

#[test]
fn unsupported_protocol_version_is_a_typed_diagnostic() {
    let mut engine = ProtocolEngine::default();
    let response = engine.handle(Request {
        protocol_version: 99,
        request_id: 10,
        body: Some(request::Body::EngineCreate(dto::EngineCreate {
            max_message_bytes: 0,
            max_slice_work: 0,
        })),
    });
    assert!(matches!(
        response.body,
        Some(response::Body::Diagnostic(dto::Diagnostic { ref code, .. })) if code == "NAR-P0002"
    ));
}

#[test]
fn oversized_message_is_rejected_before_protobuf_decode() {
    let mut engine = ProtocolEngine::new(narrata_protocol::ProtocolLimits {
        max_message_bytes: 64,
        ..Default::default()
    });
    assert!(matches!(
        engine.handle_bytes(&[0_u8; 65]),
        Err(narrata_protocol::ProtocolBoundaryError::Oversize)
    ));
}
