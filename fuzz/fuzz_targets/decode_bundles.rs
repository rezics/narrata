#![no_main]

use libfuzzer_sys::fuzz_target;
use narrata_store::{BundleLimits, CheckpointBundle, TimelineArchiveBundle};

fuzz_target!(|data: &[u8]| {
    let limits = BundleLimits {
        max_total_bytes: 1024 * 1024,
        max_object_bytes: 256 * 1024,
        max_objects: 1024,
    };
    let _ = CheckpointBundle::from_bytes(data, limits);
    let _ = TimelineArchiveBundle::from_bytes(data, limits);
});

