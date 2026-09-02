#![no_main]

use libfuzzer_sys::fuzz_target;
use narrata_protocol::{ProtocolEngine, ProtocolLimits};

fuzz_target!(|data: &[u8]| {
    let mut engine = ProtocolEngine::new(ProtocolLimits {
        max_message_bytes: 64 * 1024,
        ..ProtocolLimits::default()
    });
    let _ = engine.handle_bytes(data);
});
