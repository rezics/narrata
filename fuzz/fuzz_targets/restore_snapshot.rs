#![no_main]

use libfuzzer_sys::fuzz_target;
use narrata_core::program::{encode_program_artifact, load_program};
use narrata_testkit::generator::{branch_call_choice_v1, hello_v0, scene_reconcile_v1};

// Snapshot schema 0 restores against format 0 Programs and schema 1 against format 1 ones
// (ADR 0018); the format 1 stories cover pending Choices, content indices and scenes.
fuzz_target!(|data: &[u8]| {
    let stories = [Ok(hello_v0()), Ok(branch_call_choice_v1()), scene_reconcile_v1()];
    for artifact in stories.into_iter().flatten() {
        if let Ok(program) = load_program(&encode_program_artifact(&artifact), &Default::default())
        {
            let _ = narrata_core::restore_snapshot(data, &program, &Default::default());
        }
    }
});
