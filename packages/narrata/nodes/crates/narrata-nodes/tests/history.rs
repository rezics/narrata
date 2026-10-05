#![allow(clippy::unwrap_used)]

use narrata_nodes::{Commit, KIND_COMMIT, SessionExport};

/// Both formats call these "generic commits", but R2 omitted absent fields before the
/// history layer chose explicit nulls. Preserving the frozen root's identity requires the
/// kernel to accept that existing payload; resealing a five-field root changes every child.
#[test]
fn frozen_r2_roots_omit_fields_the_kernel_currently_requires() {
    let corpus = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../../fixtures/compat/nodes-r2-proposals/session.export.json");
    let export: SessionExport =
        serde_json::from_str(&std::fs::read_to_string(corpus).unwrap()).unwrap();
    let commits: Vec<_> = export
        .objects
        .iter()
        .filter_map(|text| {
            let bytes = hex::decode(text).unwrap();
            Commit::decode(&bytes).ok().map(|commit| (bytes, commit))
        })
        .collect();
    let (bytes, root) = commits
        .iter()
        .find(|(_, commit)| commit.parent.is_none())
        .unwrap();
    assert_eq!(root.depth, 0);
    assert_eq!(root.encode()[0], 0xa3);
    let object = narrata_history::Object::from_bytes(bytes, KIND_COMMIT, 1, 1024).unwrap();
    assert_eq!(object.id().as_bytes(), root.id().as_bytes());
    assert!(narrata_history::Commit::from_object(&object, KIND_COMMIT).is_err());
    let kernel_root = narrata_history::Commit::root(
        narrata_history::ArtifactId::from_bytes(*root.artifact.as_bytes()),
        narrata_history::ObjectId::from_bytes(*root.state.as_bytes()),
    )
    .to_object(KIND_COMMIT);
    assert_ne!(kernel_root.id().as_bytes(), root.id().as_bytes());
    for (bytes, commit) in commits.iter().filter(|(_, commit)| commit.parent.is_some()) {
        let object = narrata_history::Object::from_bytes(bytes, KIND_COMMIT, 1, 1024).unwrap();
        let decoded = narrata_history::Commit::from_object(&object, KIND_COMMIT).unwrap();
        assert_eq!(decoded.encode(), commit.encode());
        assert_eq!(object.id().as_bytes(), commit.id().as_bytes());
    }
}
