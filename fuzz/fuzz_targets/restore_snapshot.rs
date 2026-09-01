#![no_main]

use libfuzzer_sys::fuzz_target;
use narrata_core::program::{encode_program_artifact, load_program};

fuzz_target!(|data: &[u8]| {
    if let Ok(program) = load_program(
        &encode_program_artifact(&narrata_testkit::generator::hello_v0()),
        &Default::default(),
    ) {
        let _ = narrata_core::restore_snapshot(data, &program, &Default::default());
    }
});
