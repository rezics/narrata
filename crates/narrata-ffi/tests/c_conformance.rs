#![allow(unsafe_code, clippy::panic, clippy::unwrap_used)]

use narrata_core::{ExecutionId, InputId, program::encode_program_artifact};
use narrata_ffi::{
    NarBuffer, NarStatus, nar_buffer_free, nar_engine_call, nar_engine_create, nar_engine_free,
};
use narrata_protocol::{
    ProtocolEngine,
    dto::{
        self, Dispatch, EngineCreate, ProgramLoad, Request, SessionCreate, Start, request,
        response, runtime_input,
    },
};
use narrata_testkit::generator::branch_call_choice_v1;
use prost::Message;

unsafe extern "C" {
    fn narrata_c_harness_roundtrip(
        create_engine: unsafe extern "C" fn(*mut u64) -> NarStatus,
        call_engine: unsafe extern "C" fn(u64, *const u8, usize, *mut NarBuffer) -> NarStatus,
        free_buffer: extern "C" fn(u64) -> NarStatus,
        free_engine: extern "C" fn(u64) -> NarStatus,
        request: *const u8,
        request_len: usize,
        response: *mut NarBuffer,
    ) -> NarStatus;
    fn narrata_c_harness_trace(
        create_engine: unsafe extern "C" fn(*mut u64) -> NarStatus,
        call_engine: unsafe extern "C" fn(u64, *const u8, usize, *mut NarBuffer) -> NarStatus,
        free_buffer: extern "C" fn(u64) -> NarStatus,
        free_engine: extern "C" fn(u64) -> NarStatus,
        load_request: *const u8,
        load_request_len: usize,
        create_request: *const u8,
        create_request_len: usize,
        dispatch_request: *const u8,
        dispatch_request_len: usize,
        responses: *mut NarBuffer,
    ) -> NarStatus;
}

fn request(id: u64, body: request::Body) -> Vec<u8> {
    Request {
        protocol_version: narrata_protocol::PROTOCOL_VERSION,
        request_id: id,
        body: Some(body),
    }
    .encode_to_vec()
}

#[test]
fn c_harness_calls_protocol_and_ownership_is_single_use() {
    let request = Request {
        protocol_version: narrata_protocol::PROTOCOL_VERSION,
        request_id: 7,
        body: Some(request::Body::EngineCreate(EngineCreate {
            max_message_bytes: 0,
            max_slice_work: 0,
        })),
    }
    .encode_to_vec();
    let mut output = NarBuffer {
        data: std::ptr::null(),
        len: 0,
        token: 0,
    };
    // SAFETY: Function pointers and request/output storage match the C harness contract.
    let status = unsafe {
        narrata_c_harness_roundtrip(
            nar_engine_create,
            nar_engine_call,
            nar_buffer_free,
            nar_engine_free,
            request.as_ptr(),
            request.len(),
            &mut output,
        )
    };
    assert_eq!(status, NarStatus::Ok);
    // SAFETY: The returned token owns a live buffer of exactly `len` bytes until freed below.
    let bytes = unsafe { std::slice::from_raw_parts(output.data, output.len) };
    let response = dto::Response::decode(bytes).unwrap();
    assert!(matches!(
        response.body,
        Some(response::Body::EngineCreated(_))
    ));
    assert_eq!(nar_buffer_free(output.token), NarStatus::Ok);
    assert_eq!(nar_buffer_free(output.token), NarStatus::InvalidHandle);
}

#[test]
fn invalid_handles_and_arguments_are_rejected() {
    let mut output = NarBuffer {
        data: std::ptr::null(),
        len: 0,
        token: 0,
    };
    // SAFETY: The output pointer is valid; the invalid engine handle is intentional.
    assert_eq!(
        unsafe { nar_engine_call(u64::MAX, std::ptr::null(), 0, &mut output) },
        NarStatus::InvalidHandle
    );
    assert_eq!(nar_engine_free(u64::MAX), NarStatus::InvalidHandle);
    // SAFETY: A null output pointer intentionally exercises argument validation.
    assert_eq!(
        unsafe { nar_engine_create(std::ptr::null_mut()) },
        NarStatus::InvalidArgument
    );
}

#[test]
fn c_harness_runs_the_shared_program_session_dispatch_trace() {
    let artifact = encode_program_artifact(&branch_call_choice_v1());
    let program = narrata_core::load_program(&artifact, &Default::default()).unwrap();
    let load = request(1, request::Body::ProgramLoad(ProgramLoad { artifact }));
    let create = request(
        2,
        request::Body::SessionCreate(SessionCreate {
            artifact_id: program.artifact_id().as_bytes().to_vec(),
            execution_id: ExecutionId::from_u128(901).as_bytes().to_vec(),
        }),
    );
    let dispatch = request(
        3,
        request::Body::Dispatch(Dispatch {
            session: 1,
            input: Some(dto::RuntimeInput {
                request_id: InputId::from_u128(1).as_bytes().to_vec(),
                kind: Some(runtime_input::Kind::Start(Start {})),
            }),
            slice_work: 100_000,
        }),
    );
    let mut native = ProtocolEngine::default();
    let expected = [
        native.handle_bytes(&load).unwrap(),
        native.handle_bytes(&create).unwrap(),
        native.handle_bytes(&dispatch).unwrap(),
    ];
    let empty = NarBuffer {
        data: std::ptr::null(),
        len: 0,
        token: 0,
    };
    let mut outputs = [empty; 3];
    // SAFETY: Every input slice and the three-element output array remain live for the C call.
    let status = unsafe {
        narrata_c_harness_trace(
            nar_engine_create,
            nar_engine_call,
            nar_buffer_free,
            nar_engine_free,
            load.as_ptr(),
            load.len(),
            create.as_ptr(),
            create.len(),
            dispatch.as_ptr(),
            dispatch.len(),
            outputs.as_mut_ptr(),
        )
    };
    assert_eq!(status, NarStatus::Ok);
    for (output, expected) in outputs.into_iter().zip(expected) {
        // SAFETY: Each successful C call returned one live owned buffer.
        let actual = unsafe { std::slice::from_raw_parts(output.data, output.len) };
        assert_eq!(actual, expected);
        assert_eq!(nar_buffer_free(output.token), NarStatus::Ok);
    }
}

#[test]
fn ffi_rejects_oversized_protocol_payload_without_returning_a_buffer() {
    let mut handle = 0;
    // SAFETY: `handle` is valid writable storage.
    assert_eq!(unsafe { nar_engine_create(&mut handle) }, NarStatus::Ok);
    let bytes = vec![0_u8; 16 * 1024 * 1024 + 1];
    let mut output = NarBuffer {
        data: std::ptr::null(),
        len: 0,
        token: 0,
    };
    // SAFETY: The input slice and output storage are live for the call.
    assert_eq!(
        unsafe { nar_engine_call(handle, bytes.as_ptr(), bytes.len(), &mut output) },
        NarStatus::ProtocolError
    );
    assert_eq!(output.token, 0);
    assert_eq!(nar_engine_free(handle), NarStatus::Ok);
}
