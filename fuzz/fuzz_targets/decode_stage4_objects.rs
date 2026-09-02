#![no_main]

use libfuzzer_sys::fuzz_target;
use narrata_core::{program::validate_program, snapshot::restore_snapshot};
use narrata_store::TransitionReceiptV1;

fuzz_target!(|data: &[u8]| {
    if let Ok(artifact) = narrata_testkit::generator::statechart_parallel_history_v0()
        && let Ok(program) = validate_program(artifact, &Default::default())
    {
        let _ = restore_snapshot(data, &program, &Default::default());
    }
    let _ = TransitionReceiptV1::decode(data);
});
