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
    generator::{branch_call_choice_v1, hello_v0},
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

#[test]
fn version_1_requests_and_format_0_programs_are_refused() {
    let mut engine = ProtocolEngine::default();
    let response = engine.handle(Request {
        protocol_version: 1,
        request_id: 11,
        body: Some(request::Body::EngineCreate(dto::EngineCreate {
            max_message_bytes: 0,
            max_slice_work: 0,
        })),
    });
    assert_eq!(response.protocol_version, 2);
    assert!(matches!(
        response.body,
        Some(response::Body::Diagnostic(dto::Diagnostic { ref code, .. })) if code == "NAR-P0002"
    ));
    assert!(matches!(
        call(
            &mut engine,
            12,
            request::Body::ProgramLoad(ProgramLoad {
                artifact: encode_program_artifact(&hello_v0()),
            }),
        ),
        response::Body::Diagnostic(dto::Diagnostic { ref code, .. }) if code == "NAR-P0002"
    ));
}

fn reference(key: &str) -> dto::ContentRef {
    dto::ContentRef {
        provider: "local".to_owned(),
        key: key.to_owned(),
    }
}

/// Results name content references, the speaker and the occurrence (ADR 0018); the session
/// reports its Execution so hosts can form presentation keys.
#[test]
fn results_carry_content_references_speaker_and_occurrence() {
    let mut engine = ProtocolEngine::default();
    let artifact_id = match call(
        &mut engine,
        1,
        request::Body::ProgramLoad(ProgramLoad {
            artifact: encode_program_artifact(&branch_call_choice_v1()),
        }),
    ) {
        response::Body::ProgramLoaded(value) => value.artifact_id,
        other => panic!("unexpected response: {other:?}"),
    };
    let execution = ExecutionId::from_u128(71);
    let created = match call(
        &mut engine,
        2,
        request::Body::SessionCreate(SessionCreate {
            artifact_id,
            execution_id: execution.as_bytes().to_vec(),
        }),
    ) {
        response::Body::SessionCreated(value) => value,
        other => panic!("unexpected response: {other:?}"),
    };
    assert_eq!(created.execution_id, execution.as_bytes().to_vec());
    let mut dispatch = |request_id: u64, kind: runtime_input::Kind| match call(
        &mut engine,
        request_id,
        request::Body::Dispatch(Dispatch {
            session: created.session,
            input: Some(dto::RuntimeInput {
                request_id: InputId::from_u128(u128::from(request_id))
                    .as_bytes()
                    .to_vec(),
                kind: Some(kind),
            }),
            slice_work: 0,
        }),
    ) {
        response::Body::Committed(value) => value.result.unwrap(),
        other => panic!("unexpected response: {other:?}"),
    };

    let say = dispatch(3, runtime_input::Kind::Start(Start {}));
    assert_eq!(say.kind, dto::RunResultKind::Say as i32);
    assert_eq!(
        say.body,
        Some(dto::Segment {
            unit: Some(reference("choose-a-path")),
            first: None,
            last: None,
        })
    );
    assert_eq!((say.speaker.as_ref(), say.occurrence), (None, 0));
    let choice = dispatch(
        4,
        runtime_input::Kind::Advance(dto::Advance {
            interaction_id: say.interaction_or_effect_id,
        }),
    );
    assert_eq!(choice.prompt, Some(reference("choose-a-path")));
    assert_eq!(choice.occurrence, 1);
    assert_eq!(choice.choices.len(), 1);
    assert_eq!(choice.choices[0].label, Some(reference("continue")));
    let callee = dispatch(
        5,
        runtime_input::Kind::Select(dto::Select {
            interaction_id: choice.interaction_or_effect_id,
            choice_id: choice.choices[0].id.clone(),
        }),
    );
    assert_eq!(callee.speaker, Some(reference("narrator")));
    assert_eq!(
        callee.body.and_then(|body| body.unit),
        Some(reference("inside-callee"))
    );
    assert_eq!(callee.occurrence, 2);
}

/// The hand-written Rust DTOs and the TypeScript code generated from `narrata.proto` agree on
/// these bytes; `bindings/typescript/tests/conformance.test.ts` decodes the same vector.
#[test]
fn result_wire_vector_matches_the_typescript_binding() {
    let result = dto::RunResult {
        kind: dto::RunResultKind::Choice as i32,
        interaction_or_effect_id: vec![0xaa],
        choices: vec![dto::Choice {
            id: vec![1],
            label: Some(reference("continue")),
        }],
        canonical_value: Vec::new(),
        active_states: Vec::new(),
        speaker: Some(reference("narrator")),
        body: Some(dto::Segment {
            unit: Some(reference("hello")),
            first: Some("p1".to_owned()),
            last: None,
        }),
        prompt: Some(reference("choose")),
        capability: "host.query".to_owned(),
        occurrence: 3,
    };
    assert_eq!(hex::encode(result.encode_to_vec()), RESULT_VECTOR);
    assert_eq!(
        dto::RunResult::decode(hex::decode(RESULT_VECTOR).unwrap().as_slice()).unwrap(),
        result
    );
}

const RESULT_VECTOR: &str = "08011201aa22160a01011a110a056c6f63616c1208636f6e74696e75653a110a056c6f63616c12086e61727261746f7242140a0e0a056c6f63616c120568656c6c6f120270314a0f0a056c6f63616c120663686f6f7365520a686f73742e71756572795803";
