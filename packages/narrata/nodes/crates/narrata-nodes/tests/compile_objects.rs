#![allow(clippy::unwrap_used)]
mod support;

use narrata_nodes::{MemorySource, NodeRegistry, Program, compile_objects};

#[test]
fn object_compilation_reuses_the_existing_envelopes_lock_and_checked_readers() {
    let old = support::compiled();
    let source = narrata_nodes::ProjectSource {
        manifest: serde_json::from_value(support::manifest()).unwrap(),
        packages: [
            (
                "main".to_owned(),
                serde_json::from_value(support::main_package()).unwrap(),
            ),
            (
                "side".to_owned(),
                serde_json::from_value(support::side_package()).unwrap(),
            ),
        ]
        .into(),
    };
    let objects = compile_objects(&source, &NodeRegistry::gamebook()).unwrap();
    assert_eq!(objects.objects.encode(), old.pack);
    assert_eq!(objects.lock, old.lock);
    let mut storage = MemorySource::default();
    for object in objects
        .objects
        .chunks
        .iter()
        .chain(std::iter::once(&objects.objects.tombstones))
    {
        storage.insert(object.clone()).unwrap();
    }
    let reopened = Program::from_manifest(&objects.objects.manifest, Box::new(storage)).unwrap();
    reopened.verify_artifact().unwrap();
    assert_eq!(reopened.artifact_id(), old.program.artifact_id());
    assert_eq!(objects.into_packed().unwrap().pack, old.pack);
}
