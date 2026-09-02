#![allow(clippy::panic, clippy::unwrap_used)]

use narrata_core::{ExecutionId, InputId, program::encode_program_artifact};
use narrata_protocol::{
    ProtocolEngine,
    dto::{
        self, Dispatch, ProgramLoad, Request, SessionCreate, Start, request, response,
        runtime_input,
    },
};
use narrata_testkit::generator::branch_call_choice_v0;
use narrata_wasm::call_protocol_for_conformance;
use prost::Message;

fn request(id: u64, body: request::Body) -> Vec<u8> {
    Request {
        protocol_version: 1,
        request_id: id,
        body: Some(body),
    }
    .encode_to_vec()
}

fn call(engine: &mut ProtocolEngine, bytes: &[u8]) -> response::Body {
    dto::Response::decode(
        call_protocol_for_conformance(engine, bytes)
            .unwrap()
            .as_slice(),
    )
    .unwrap()
    .body
    .unwrap()
}

#[test]
fn wasm_export_path_produces_the_same_canonical_hashes_as_native_protocol() {
    let artifact = encode_program_artifact(&branch_call_choice_v0());
    let mut native = ProtocolEngine::default();
    let mut wasm_path = ProtocolEngine::default();
    let load = request(1, request::Body::ProgramLoad(ProgramLoad { artifact }));
    let native_artifact = match call(&mut native, &load) {
        response::Body::ProgramLoaded(value) => value.artifact_id,
        other => panic!("unexpected {other:?}"),
    };
    let wasm_artifact = match call(&mut wasm_path, &load) {
        response::Body::ProgramLoaded(value) => value.artifact_id,
        other => panic!("unexpected {other:?}"),
    };
    assert_eq!(native_artifact, wasm_artifact);
    let create = |artifact_id| {
        request(
            2,
            request::Body::SessionCreate(SessionCreate {
                artifact_id,
                execution_id: ExecutionId::from_u128(91).as_bytes().to_vec(),
            }),
        )
    };
    let native_session = match call(&mut native, &create(native_artifact)) {
        response::Body::SessionCreated(value) => value.session,
        other => panic!("unexpected {other:?}"),
    };
    let wasm_session = match call(&mut wasm_path, &create(wasm_artifact)) {
        response::Body::SessionCreated(value) => value.session,
        other => panic!("unexpected {other:?}"),
    };
    let dispatch = |session| {
        request(
            3,
            request::Body::Dispatch(Dispatch {
                session,
                input: Some(dto::RuntimeInput {
                    request_id: InputId::from_u128(1).as_bytes().to_vec(),
                    kind: Some(runtime_input::Kind::Start(Start {})),
                }),
                slice_work: 100_000,
            }),
        )
    };
    let native_result = match call(&mut native, &dispatch(native_session)) {
        response::Body::Committed(value) => value,
        other => panic!("unexpected {other:?}"),
    };
    let wasm_result = match call(&mut wasm_path, &dispatch(wasm_session)) {
        response::Body::Committed(value) => value,
        other => panic!("unexpected {other:?}"),
    };
    assert_eq!(native_result.state_digest, wasm_result.state_digest);
    assert_eq!(native_result.receipt_id, wasm_result.receipt_id);
    assert_eq!(native_result.commit_id, wasm_result.commit_id);
    assert_eq!(native_result.snapshot, wasm_result.snapshot);
}
