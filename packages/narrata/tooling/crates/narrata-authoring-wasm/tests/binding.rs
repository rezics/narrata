#![allow(clippy::unwrap_used, clippy::panic)]

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use narrata_authoring::{PublicationStatus, publication::ObjectReference};
use narrata_authoring_wasm::{
    core::{DraftSet, Loader, Occupancy, SourceBuilder, digest, object_id},
    interface,
};
use narrata_nodes::{
    AuthoredId, ContentOutline, Error, NodeRegistry, Program, ProjectSource, compile,
};

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../..")
}
fn demo() -> ProjectSource {
    narrata_node_tools::load_project(&repository().join("products/gamebook-demo/project.json"))
        .unwrap()
        .source
}
fn product(path: &str) -> Vec<u8> {
    std::fs::read(repository().join("products/gamebook-demo").join(path)).unwrap()
}
fn outline() -> ContentOutline {
    let text = String::from_utf8(product("content/zh-Hans.json")).unwrap();
    narrata_content_local::outline(&narrata_content_local::ContentPack::parse(&text).unwrap())
}
fn source_records() -> Vec<Vec<u8>> {
    let mut builder = SourceBuilder::new(demo().manifest);
    for (alias, package) in demo().packages {
        builder.add_package(&alias, package).unwrap();
    }
    let split = builder.split();
    assert!(split.report.complete && split.report.diagnostics.is_empty());
    split
        .records
        .iter()
        .map(|record| record.bytes().to_vec())
        .collect()
}
fn drafts(records: &[Vec<u8>]) -> DraftSet {
    let mut drafts = DraftSet::default();
    for record in records {
        drafts.add(record);
    }
    drafts
}

#[test]
fn interface_schema_is_the_committed_generated_contract() {
    let committed = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("schemas/authoring-interface.schema.json"),
    )
    .unwrap();
    assert_eq!(
        committed.replace("\r\n", "\n"),
        interface::schema_text().unwrap(),
        "regenerate with task web:build"
    );
}

#[test]
fn record_by_record_publication_matches_the_cli_objects_and_groups_indexes() {
    let records = source_records();
    let mut set = drafts(&records);
    // Outline batches of one provider assemble into the same single outline.
    let whole = outline();
    let keys: Vec<_> = whole.units.keys().cloned().collect();
    let (first, second) = keys.split_at(keys.len() / 2);
    for half in [first, second] {
        let mut batch = whole.clone();
        batch.units.retain(|key, _| half.contains(key));
        set.add_outline(batch).unwrap();
    }
    let mut repeated = whole.clone();
    repeated.units.retain(|key, _| key == &first[0]);
    assert_eq!(
        set.add_outline(repeated).unwrap_err().code,
        "outline_duplicate_unit"
    );

    let published = set.publish(None, None);
    assert_eq!(published.report.status, Some(PublicationStatus::Switchable));
    let publication = published.publication.unwrap();
    // The CLI's `publication` output for the same work, committed with the product.
    assert_eq!(
        publication.description_bytes(),
        product("story.publication.json")
    );
    for (reference, bytes) in publication.objects() {
        assert_eq!(
            product(&format!("story.publication/{}", reference.filename())),
            bytes
        );
    }
    assert_eq!(publication.pack().unwrap(), product("story.narpack"));
    let lock: narrata_nodes::CompositionLock =
        serde_json::from_slice(&product("project.lock.json")).unwrap();
    assert_eq!(published.report.lock, Some(lock));

    let check = set.check();
    assert_eq!(check.records.len(), records.len());
    let options: usize = (0..check.records.len())
        .map(|index| check.indexes(index).unwrap().options.len())
        .sum();
    let rows = narrata_authoring::derive_draft_indexes(
        &narrata_authoring::validate_records(
            &records.iter().map(Vec::as_slice).collect::<Vec<_>>(),
        )
        .value
        .unwrap(),
    );
    assert_eq!(options, rows.options.len());
    assert_eq!(check.source.unwrap(), {
        let source = demo();
        narrata_authoring::records::normalize_source(&source).unwrap()
    });
}

#[test]
fn previous_publications_restore_from_individual_objects_with_checked_identity() {
    let description = product("story.publication.json");
    let expected = digest(
        &narrata_authoring::objects::Digest::from_bytes(narrata_kernel::codec::digest_bytes(
            "narrata.nodes.publication",
            1,
            &description,
        ))
        .to_string(),
    )
    .unwrap();
    assert_eq!(
        Loader::new(&description, digest(&"0".repeat(64)).unwrap())
            .unwrap_err()
            .code,
        "publication"
    );
    let mut loader = Loader::new(&description, expected).unwrap();
    let references: Vec<ObjectReference> = loader.references().cloned().collect();
    let file = |reference: &ObjectReference| {
        product(&format!("story.publication/{}", reference.filename()))
    };
    let first = &references[0];
    assert_eq!(
        loader.supply(first.object_id, &[0]).unwrap_err().code,
        "object_length"
    );
    for reference in &references[1..] {
        loader
            .supply(reference.object_id, &file(reference))
            .unwrap();
    }
    assert_eq!(loader.missing(), vec![first.object_id]);
    let mut incomplete = Loader::new(&description, expected).unwrap();
    incomplete.supply(first.object_id, &file(first)).unwrap();
    assert_eq!(incomplete.finish().unwrap_err().code, "missing_object");
    loader.supply(first.object_id, &file(first)).unwrap();
    loader.supply(first.object_id, &file(first)).unwrap();
    let previous = loader.finish().unwrap();
    assert_eq!(previous.publication_id(), expected);

    // A later publication against the restored baseline keeps the same artifact.
    let records = source_records();
    let set = drafts(&records);
    let again = set.publish(Some(previous.program()), None);
    assert!(again.report.compared);
    assert_eq!(
        again.report.previous_artifact_id,
        Some(previous.program().artifact_id())
    );
    assert_eq!(again.report.publication_id, Some(expected));
}

#[test]
fn preview_objects_open_by_reader_ids_without_a_pack() {
    let records = source_records();
    let preview = drafts(&records).compose(None, None).unwrap();
    assert!(preview.report.complete);
    let program = Program::from_pack(&product("story.narpack")).unwrap().0;
    assert_eq!(preview.report.artifact_id, Some(program.artifact_id()));
    let manifest = program.manifest();
    for id in manifest.chunks.iter().chain([&manifest.tombstones]) {
        let text = id.to_string();
        assert_eq!(
            preview.object(&object_id(&text).unwrap()),
            preview.object(&object_id(text.trim_start_matches("object:")).unwrap())
        );
        assert!(preview.object(id).is_some());
    }
    assert_eq!(preview.report.objects.len(), manifest.chunks.len() + 1);
    assert!(preview.manifest().is_some() && preview.names().is_some());
    assert_eq!(preview.pack().unwrap(), product("story.narpack"));
    assert_eq!(object_id("object:xyz").unwrap_err().code, "identifier");

    let mut broken = records.clone();
    broken.push(b"not json".to_vec());
    let failed = drafts(&broken).compose(None, None).unwrap();
    assert!(failed.manifest().is_none());
    assert_eq!(failed.pack().unwrap_err().code, "preview");
    assert!(failed.report.diagnostics.iter().any(
        |d| d.location.source_path.as_deref() == Some(&*format!("records[{}]", records.len()))
    ));
}

#[test]
fn minting_avoids_occupied_ids_and_fill_keeps_existing_ids() {
    let clock = || Ok(1_700_000_000_000);
    let fixed = |bytes: &mut [u8]| {
        bytes.fill(7);
        Ok(())
    };
    let mut occupancy = Occupancy::default();
    let first = occupancy.mint(1, 0, 0, clock, fixed).unwrap();
    // The same deterministic source cannot produce a second ID: occupancy prevents reuse.
    assert_eq!(
        occupancy.mint(1, 0, 0, clock, fixed).unwrap_err().code,
        "random"
    );
    let mut fresh = Occupancy::default();
    fresh.occupy([AuthoredId::Node(first.nodes[0])]);
    assert!(fresh.mint(1, 0, 0, clock, fixed).is_err());
    let mut counter = 0_u8;
    let minted = fresh
        .mint(1, 1, 2, clock, |bytes: &mut [u8]| {
            counter += 1;
            bytes.fill(counter);
            Ok(())
        })
        .unwrap();
    assert_eq!(
        (
            minted.nodes.len(),
            minted.choice_points.len(),
            minted.options.len()
        ),
        (1, 1, 2)
    );
    assert_eq!(
        fresh.mint(4097, 0, 0, clock, fixed).unwrap_err().code,
        "limit"
    );
    assert_eq!(
        fresh
            .mint(
                1,
                0,
                0,
                || Err(Error::new("clock", "ids", "offline")),
                fixed
            )
            .unwrap_err()
            .code,
        "clock"
    );

    let mut edit = demo();
    let node = edit
        .packages
        .get_mut("road")
        .unwrap()
        .graphs
        .get_mut("crossing")
        .unwrap()
        .nodes
        .get_mut("fog")
        .unwrap();
    node.id = None;
    let kept = edit.packages["road"].graphs["crossing"].nodes["rocks"].id;
    let mut counter = 10_u8;
    let filled = Occupancy::default()
        .fill(&edit, clock, |bytes: &mut [u8]| {
            counter += 1;
            bytes.fill(counter);
            Ok(())
        })
        .unwrap();
    assert_eq!(filled.report.minted.len(), 1);
    let source = filled.source.unwrap();
    assert!(
        source.packages["road"].graphs["crossing"].nodes["fog"]
            .id
            .is_some()
    );
    assert_eq!(
        source.packages["road"].graphs["crossing"].nodes["rocks"].id,
        kept
    );
    assert!(compile(&source, &NodeRegistry::gamebook()).is_ok());
    let records: BTreeMap<_, _> = filled
        .records
        .iter()
        .map(|record| (record.key(), record.bytes().len()))
        .collect();
    assert_eq!(records.len(), filled.records.len());
}
