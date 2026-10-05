#![allow(clippy::unwrap_used, clippy::panic)]

use narrata_authoring::{
    CheckedRecord, ComposeMode,
    objects::Digest,
    publication::read_publication,
    publish_records,
    relations::{MAX_PART_BYTES, read_relations},
    split_source,
};
use narrata_nodes::{NodeId, NodeRegistry, ProjectSource, compile_objects};
use serde_json::json;
use std::collections::BTreeMap;

#[test]
#[ignore = "allocates more than 64 MiB; run through a heavy Goal slot"]
fn independent_objects_publish_and_reopen_when_a_single_file_pack_is_too_large() {
    let mut packages = BTreeMap::new();
    let mut paths = BTreeMap::new();
    let padding = "x".repeat(60_000);
    for package in 0..19 {
        let alias = format!("p{package:02}");
        let mut nodes = BTreeMap::new();
        for index in 0..60 {
            // A large legal scalar argument exercises the existing object budgets. It is
            // synthetic input, not a host's inline body or content-label field.
            nodes.insert(format!("n{index}"),json!({"id":NodeId::from_bytes(((package*100+index+1) as u128).to_be_bytes()),"type_id":"narrata.passage","data":{"args":{"padding":{"kind":"literal","value":padding}},"next": if index == 59 { "end".to_owned() } else { format!("n{}",index+1) }}}));
        }
        nodes.insert("end".into(),json!({"id":NodeId::from_bytes(((package*100+99) as u128).to_be_bytes()),"type_id":"narrata.return","data":{"outcome":"done"}}));
        packages.insert(alias.clone(),serde_json::from_value(json!({"id":alias,"version":"1","exports":["main"],"graphs":{"main":{"entry":"n0","outcomes":["done"],"nodes":nodes}}})).unwrap());
        paths.insert(alias.clone(), format!("{alias}.json"));
    }
    let source = ProjectSource { manifest: serde_json::from_value(json!({"format_version":2,"product":{"id":"large","entry":{"package":"p00","graph":"main"},"endings":{"done":{}}},"packages":paths})).unwrap(), packages };
    let compiled = compile_objects(&source, &NodeRegistry::gamebook()).unwrap();
    let object_bytes: usize = compiled.objects.chunks.iter().map(Vec::len).sum();
    assert!(object_bytes > narrata_nodes::MAX_PACK_BYTES);
    assert!(
        compiled
            .objects
            .chunks
            .iter()
            .all(|c| c.len() <= narrata_nodes::MAX_CHUNK_BYTES)
    );
    assert_eq!(compiled.into_packed().unwrap_err().code, "decode");
    let records = split_source(&source).into_result().unwrap();
    let inputs: Vec<_> = records.iter().map(CheckedRecord::bytes).collect();
    let report = publish_records(&inputs, &[], None, ComposeMode::Update);
    assert!(report.complete);
    let published = report.into_result().unwrap();
    assert!(
        published
            .publication
            .objects()
            .map(|(_, b)| b.len())
            .sum::<usize>()
            > narrata_nodes::MAX_PACK_BYTES
    );
    assert!(published.publication.pack().is_err());
    let reopened = read_publication(
        published.publication.description_bytes(),
        published.publication.publication_id(),
        |id| Ok(published.publication.object(id).unwrap().to_vec()),
    )
    .unwrap();
    assert_eq!(
        reopened.program().artifact_id(),
        published.candidate_artifact_id
    );
    println!(
        "program_chunk_bytes={object_bytes}; publication_objects={}",
        reopened.description().objects.len()
    );
}

#[test]
#[ignore = "100k-choice projection and checked reload; run through a heavy Goal slot"]
fn the_100k_choice_projection_splits_at_four_mib_and_reloads_all_rows() {
    let compilation =
        narrata_node_bench::generate(narrata_node_bench::WorkSize::new(100_000).unwrap()).unwrap();
    let projection = narrata_authoring::project_relations(&compilation.program).unwrap();
    assert!(
        projection
            .parts()
            .iter()
            .all(|p| p.bytes.len() <= MAX_PART_BYTES)
    );
    assert!(projection.index().parts.iter().any(|r| r.ordinal > 0));
    let objects: BTreeMap<Digest, Vec<u8>> = projection
        .parts()
        .iter()
        .map(|p| (p.id, p.bytes.clone()))
        .collect();
    let checked = read_relations(
        &projection.index_object().bytes,
        projection.index_object().id,
        |id| Ok(objects[&id].clone()),
    )
    .unwrap();
    assert_eq!(checked.rows(), projection.rows());
}
