//! Wasm binding over the exact same protocol engine used by the C ABI.

#![forbid(unsafe_code)]

use narrata_protocol::{PROTOCOL_ABI_VERSION, ProtocolEngine, ProtocolLimits};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct WasmEngine {
    inner: ProtocolEngine,
}

#[wasm_bindgen]
impl WasmEngine {
    #[wasm_bindgen(constructor)]
    pub fn new(max_message_bytes: u32, max_slice_work: u32) -> Self {
        let defaults = ProtocolLimits::default();
        Self {
            inner: ProtocolEngine::new(ProtocolLimits {
                max_message_bytes: if max_message_bytes == 0 {
                    defaults.max_message_bytes
                } else {
                    u64::from(max_message_bytes).min(defaults.max_message_bytes)
                },
                max_slice_work: if max_slice_work == 0 {
                    defaults.max_slice_work
                } else {
                    u64::from(max_slice_work).min(defaults.max_slice_work)
                },
                ..defaults
            }),
        }
    }

    pub fn call(&mut self, request: &[u8]) -> Result<Vec<u8>, JsError> {
        self.inner
            .handle_bytes(request)
            .map_err(|error| JsError::new(&error.to_string()))
    }
}

#[wasm_bindgen]
pub fn narrata_abi_version() -> u32 {
    PROTOCOL_ABI_VERSION
}

pub fn call_protocol_for_conformance(
    engine: &mut ProtocolEngine,
    request: &[u8],
) -> Result<Vec<u8>, narrata_protocol::ProtocolBoundaryError> {
    engine.handle_bytes(request)
}
