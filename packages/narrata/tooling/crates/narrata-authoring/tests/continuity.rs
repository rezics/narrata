#![allow(clippy::unwrap_used, clippy::panic)]

use std::path::Path;

use narrata_authoring::{ComposeMode, Minter, compose_source, ids::uuid_v7, mint_ids};
use narrata_nodes::{AuthoredId, Error, NodeId, NodeRegistry, Program, ProjectSource, compile};

fn source() -> ProjectSource {
    narrata_node_tools::load_project(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../../../products/gamebook-demo/project.json"),
    )
    .unwrap()
    .source
}
fn old(source: &ProjectSource) -> Program {
    Program::from_pack(&compile(source, &NodeRegistry::gamebook()).unwrap().pack)
        .unwrap()
        .0
}

#[test]
fn deterministic_minting_preserves_existing_ids_and_avoids_cross_kind_tombstone_collisions() {
    let mut edit = source();
    let node = edit
        .packages
        .get_mut("road")
        .unwrap()
        .graphs
        .get_mut("crossing")
        .unwrap()
        .nodes
        .get_mut("rocks")
        .unwrap();
    node.id = None;
    node.data["choice_points"][0]
        .as_object_mut()
        .unwrap()
        .remove("id");
    node.data["choice_points"][0]["options"][0]
        .as_object_mut()
        .unwrap()
        .remove("id");
    let occupied = AuthoredId::Option(narrata_nodes::OptionId::from_bytes(
        uuid_v7(&mut || Ok(42), &mut |bytes| {
            bytes.fill(1);
            Ok(())
        })
        .unwrap(),
    ));
    let generate = || {
        let mut counter = 0_u8;
        mint_ids(
            &edit,
            [occupied],
            || Ok(42),
            move |bytes| {
                counter += 1;
                bytes.fill(counter);
                Ok(())
            },
        )
        .unwrap()
    };
    let first = generate();
    let second = generate();
    assert_eq!(first.source, second.source);
    assert_eq!(first.minted, second.minted);
    assert_eq!(first.minted.len(), 3);
    assert!(
        first
            .minted
            .iter()
            .all(|id| id.to_string().contains("00000000002a7"))
    );
    let kept = edit.packages["camp"].graphs["visit"].nodes["fire"].id;
    assert_eq!(
        first.source.packages["camp"].graphs["visit"].nodes["fire"].id,
        kept
    );
    let again = mint_ids(
        &first.source,
        [],
        || Err(Error::new("clock", "ids", "must not call")),
        |_| Err(Error::new("random", "ids", "must not call")),
    )
    .unwrap();
    assert!(again.minted.is_empty());
    assert_eq!(again.source, first.source);
    assert!(compile(&first.source, &NodeRegistry::gamebook()).is_ok());
    assert_eq!(
        edit.packages["road"].graphs["crossing"].nodes["rocks"].id,
        None
    );
}

#[test]
fn minting_failure_never_mutates_the_edit_and_broken_platform_sources_are_bounded() {
    let mut edit = source();
    edit.packages
        .get_mut("camp")
        .unwrap()
        .graphs
        .get_mut("visit")
        .unwrap()
        .nodes
        .get_mut("fire")
        .unwrap()
        .id = None;
    let before = edit.clone();
    assert!(
        mint_ids(
            &edit,
            [],
            || Ok(42),
            |_| Err(Error::new("random", "ids", "offline"))
        )
        .is_err()
    );
    assert_eq!(edit, before);
    assert!(uuid_v7(&mut || Ok(1 << 48), &mut |_| Ok(())).is_err());
    let id = uuid_v7(&mut || Ok(42), &mut |bytes| {
        bytes.fill(1);
        Ok(())
    })
    .unwrap();
    let mut minter = Minter::new(
        [AuthoredId::Node(NodeId::from_bytes(id))],
        || Ok(42),
        |bytes: &mut [u8]| {
            bytes.fill(1);
            Ok(())
        },
    );
    assert!(minter.option().is_err());
    assert_eq!(minter.minted, 0);
}

#[test]
fn compose_returns_tombstone_records_and_a_lock_without_mutating_sources() {
    let source = source();
    let previous = old(&source);
    let mut edit = source.clone();
    let removed = edit
        .packages
        .get_mut("road")
        .unwrap()
        .graphs
        .get_mut("crossing")
        .unwrap()
        .nodes
        .get_mut("rocks")
        .unwrap()
        .data["choice_points"][0]["options"]
        .as_array_mut()
        .unwrap()
        .remove(1);
    let before = edit.clone();
    let composed = compose_source(&edit, Some(&previous), ComposeMode::Update).unwrap();
    assert_eq!(edit, before);
    assert!(composed.compared);
    assert_eq!(composed.appended.len(), 1);
    assert_eq!(
        composed.appended[0].1.to_string(),
        removed["id"].as_str().unwrap()
    );
    assert_eq!(composed.tombstones.len(), 1);
    let mut persisted = edit.clone();
    for (alias, id) in &composed.appended {
        persisted
            .packages
            .get_mut(alias)
            .unwrap()
            .tombstones
            .push(*id);
    }
    let locked = compose_source(
        &persisted,
        Some(&previous),
        ComposeMode::Locked(&composed.compilation.lock),
    )
    .unwrap();
    assert_eq!(locked.compilation.pack, composed.compilation.pack);
    assert!(locked.appended.is_empty());
    assert_eq!(
        compose_source(
            &edit,
            Some(&previous),
            ComposeMode::Locked(&composed.compilation.lock)
        )
        .unwrap_err()
        .code,
        "tombstones_needed"
    );
    // Reusing a tombstoned option in the same authored kind is rejected by compilation.
    let option = &mut persisted
        .packages
        .get_mut("road")
        .unwrap()
        .graphs
        .get_mut("crossing")
        .unwrap()
        .nodes
        .get_mut("rocks")
        .unwrap()
        .data["choice_points"][0]["options"][0]["id"];
    *option = removed["id"].clone();
    assert_eq!(
        compose_source(&persisted, Some(&previous), ComposeMode::Update)
            .unwrap_err()
            .code,
        "tombstone"
    );
}

#[test]
fn continuity_refuses_owner_changes_removed_tombstones_and_deleted_packages() {
    let source = source();
    let previous = old(&source);
    let mut moved = source.clone();
    let nodes = &mut moved
        .packages
        .get_mut("road")
        .unwrap()
        .graphs
        .get_mut("crossing")
        .unwrap()
        .nodes;
    let left = nodes["fog"].data["choice_points"][0]["options"][0]["id"].clone();
    let right = nodes["trail"].data["choice_points"][0]["options"][0]["id"].clone();
    nodes.get_mut("fog").unwrap().data["choice_points"][0]["options"][0]["id"] = right;
    nodes.get_mut("trail").unwrap().data["choice_points"][0]["options"][0]["id"] = left;
    assert_eq!(
        compose_source(&moved, Some(&previous), ComposeMode::Update)
            .unwrap_err()
            .code,
        "owner_changed"
    );
    let mut deleted = source.clone();
    deleted
        .packages
        .get_mut("main")
        .unwrap()
        .tombstones
        .push(AuthoredId::Node(NodeId::from_bytes([99; 16])));
    let with_tombstone = old(&deleted);
    assert_eq!(
        compose_source(&source, Some(&with_tombstone), ComposeMode::Update)
            .unwrap_err()
            .code,
        "tombstone_removed"
    );
    let mut no_camp = source.clone();
    no_camp.packages.remove("camp");
    no_camp.manifest.packages.remove("camp");
    // Use a minimal valid replacement project to exercise deletion of the original owners.
    let one = source.packages["road"].clone();
    no_camp.packages = std::collections::BTreeMap::from([("road".into(), one)]);
    no_camp.manifest.packages =
        std::collections::BTreeMap::from([("road".into(), "road.json".into())]);
    no_camp.manifest.product.entry = narrata_nodes::plan::GraphRef {
        package: "road".into(),
        graph: "crossing".into(),
    };
    no_camp.manifest.product.bindings.clear();
    no_camp.manifest.product.endings.clear();
    assert_eq!(
        compose_source(&no_camp, Some(&previous), ComposeMode::Update)
            .unwrap_err()
            .code,
        "tombstone"
    );
}

#[test]
fn first_publication_and_locked_source_semantics_are_explicit() {
    let source = source();
    let composed = compose_source(&source, None, ComposeMode::Update).unwrap();
    assert!(!composed.compared);
    assert!(composed.appended.is_empty());
    assert!(
        !compose_source(
            &source,
            None,
            ComposeMode::Locked(&composed.compilation.lock)
        )
        .unwrap()
        .compared
    );
    let mut expected = composed.compilation.lock.clone();
    expected
        .node_types
        .insert("narrata.passage".into(), "future".into());
    assert_eq!(
        compose_source(&source, None, ComposeMode::Locked(&expected))
            .unwrap_err()
            .code,
        "lock_mismatch"
    );
    let mut renamed = source.clone();
    renamed.packages.get_mut("road").unwrap().version = "future".into();
    assert_eq!(
        compose_source(
            &renamed,
            None,
            ComposeMode::Locked(&composed.compilation.lock)
        )
        .unwrap_err()
        .code,
        "lock_mismatch"
    );
}
