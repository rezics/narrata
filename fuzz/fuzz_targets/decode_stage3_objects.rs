#![no_main]

use libfuzzer_sys::fuzz_target;
use narrata_store::{CompoundSaveManifestV1, HostTimelineManifestV1, RecordedEffectResponseV1};

fuzz_target!(|data: &[u8]| {
    let _ = RecordedEffectResponseV1::decode(data);
    let _ = CompoundSaveManifestV1::decode(data);
    let _ = HostTimelineManifestV1::decode(data);
});
