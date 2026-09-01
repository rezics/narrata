#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = narrata_core::codec::decode_canonical_value(data, &Default::default());
});

