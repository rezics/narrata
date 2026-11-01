#![allow(clippy::unwrap_used, clippy::panic)]

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use narrata_authoring::{PublicationStatus, publication::ObjectReference};
use narrata_authoring_wasm::{
    VnDrafts,
    core::{DraftSet, Loader, Occupancy, SourceBuilder, digest, object_id},
    interface,
};
use narrata_nodes::{
    AuthoredId, ContentOutline, Error, NodeRegistry, Program, ProjectSource,
    VISUAL_NOVEL_FORMAT_VERSION, compile,
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

fn vn_records() -> Vec<Vec<u8>> {
    let mut value: serde_json::Value = serde_json::from_slice(
        &std::fs::read(repository().join("fixtures/compat/nodes-vn-v1/source.json")).unwrap(),
    )
    .unwrap();
    let source = ProjectSource {
        manifest: serde_json::from_value(value["manifest"].take()).unwrap(),
        packages: BTreeMap::from([(
            "main".into(),
            serde_json::from_value(value["package"].take()).unwrap(),
        )]),
    };
    narrata_authoring::split_source(&source)
        .into_result()
        .unwrap()
        .into_iter()
        .map(|record| record.bytes().to_vec())
        .collect()
}

#[test]
fn vn_wasm_entry_checks_and_previews_only_vn_snapshots() {
    let records = vn_records();
    let input = records.iter().map(Vec::as_slice).collect::<Vec<_>>();
    let native = narrata_authoring::vn::preview_records(
        &input,
        None,
        narrata_authoring::ComposeMode::Update,
    )
    .value
    .unwrap();
    let native_pack = &native.composed.compilation.objects;
    let mut drafts = VnDrafts::new();
    for record in &records {
        drafts.add(record);
    }
    let checked = drafts.check();
    assert!(serde_json::from_str::<serde_json::Value>(&checked.report().unwrap()).unwrap()
        ["diagnostics"]
        .as_array()
        .unwrap()
        .is_empty());
    let preview = drafts.compose(None).unwrap();
    let report: serde_json::Value = serde_json::from_str(&preview.report().unwrap()).unwrap();
    let old_report = report.clone();
    let old_manifest = preview.manifest().unwrap();
    let old_pack = preview.pack().unwrap();
    assert_eq!(
        report["artifact_id"],
        native
            .composed
            .compilation
            .program
            .artifact_id()
            .to_string()
    );
    assert_eq!(
        report["lock"],
        serde_json::to_value(&native.composed.compilation.lock).unwrap()
    );
    assert_eq!(old_manifest, native_pack.manifest);
    assert_eq!(VISUAL_NOVEL_FORMAT_VERSION, 3);

    let objects = report["objects"].as_array().unwrap();
    let old_objects = objects
        .iter()
        .map(|id| preview.object(id.as_str().unwrap()).unwrap().unwrap())
        .collect::<Vec<_>>();
    for (id, expected) in objects.iter().zip(
        native_pack
            .chunks
            .iter()
            .chain(std::iter::once(&native_pack.tombstones)),
    ) {
        let id = id.as_str().unwrap();
        assert_eq!(preview.object(id).unwrap().unwrap(), *expected);
    }
    assert_eq!(
        preview
            .object(&format!("object:{}", "00".repeat(32)))
            .unwrap(),
        None
    );
    assert!(preview.pack().is_ok());
    let repeated = drafts.compose(None).unwrap();
    assert_eq!(repeated.pack().unwrap(), old_pack);
    assert_eq!(repeated.manifest().unwrap(), old_manifest);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&repeated.report().unwrap()).unwrap(),
        old_report
    );

    let mut changed = vn_records();
    let dialogue = changed
        .iter_mut()
        .find(|bytes| {
            serde_json::from_slice::<serde_json::Value>(bytes).unwrap()["payload"]["type_id"]
                == "narrata.dialogue"
        })
        .unwrap();
    let mut record: serde_json::Value = serde_json::from_slice(dialogue).unwrap();
    record["payload"]["data"]["speaker"]["key"] =
        serde_json::Value::String("speaker.edited".into());
    *dialogue = narrata_nodes::canonical_json(&record).unwrap();
    let changed_input = changed.iter().map(Vec::as_slice).collect::<Vec<_>>();
    let changed_native = narrata_authoring::vn::preview_records(
        &changed_input,
        None,
        narrata_authoring::ComposeMode::Update,
    )
    .value
    .unwrap();
    let mut changed_drafts = VnDrafts::new();
    for record in &changed {
        changed_drafts.add(record);
    }
    let next = changed_drafts.compose(None).unwrap();
    let next_report: serde_json::Value = serde_json::from_str(&next.report().unwrap()).unwrap();
    assert!(next_report["complete"].as_bool().unwrap());
    assert_eq!(
        next_report["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|d| d["severity"] == "error")
            .count(),
        0
    );
    assert!(next_report["artifact_id"].is_string());
    assert_eq!(
        next_report["artifact_id"],
        changed_native
            .composed
            .compilation
            .program
            .artifact_id()
            .to_string()
    );
    assert_eq!(
        next.pack().unwrap(),
        changed_native.composed.compilation.objects.encode()
    );
    assert_ne!(old_report["artifact_id"], next_report["artifact_id"]);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&preview.report().unwrap()).unwrap(),
        old_report
    );
    assert_eq!(preview.manifest().unwrap(), old_manifest);
    assert_eq!(preview.pack().unwrap(), old_pack);
    for (id, expected) in old_report["objects"]
        .as_array()
        .unwrap()
        .iter()
        .zip(old_objects)
    {
        assert_eq!(
            preview.object(id.as_str().unwrap()).unwrap().unwrap(),
            expected
        );
    }
}

#[test]
fn gamebook_and_vn_draft_routes_reject_the_other_dialect() {
    let mut vn = VnDrafts::new();
    for record in source_records() {
        vn.add(&record);
    }
    let vn_report: serde_json::Value = serde_json::from_str(&vn.check().report().unwrap()).unwrap();
    assert!(!vn_report["diagnostics"].as_array().unwrap().is_empty());

    let mut gamebook = DraftSet::default();
    for record in vn_records() {
        gamebook.add(&record);
    }
    assert!(gamebook.check().report.complete);
    assert!(gamebook.check().records.is_empty());
}

#[test]
fn vn_draft_check_returns_located_dialogue_diagnostics() {
    let mut records = vn_records();
    let dialogue = records
        .iter_mut()
        .find(|bytes| {
            serde_json::from_slice::<serde_json::Value>(bytes).unwrap()["payload"]["type_id"]
                == "narrata.dialogue"
        })
        .unwrap();
    let mut record: serde_json::Value = serde_json::from_slice(dialogue).unwrap();
    record["payload"]["data"]
        .as_object_mut()
        .unwrap()
        .remove("line");
    *dialogue = narrata_nodes::canonical_json(&record).unwrap();
    let mut drafts = VnDrafts::new();
    for record in records {
        drafts.add(&record);
    }
    let report: serde_json::Value =
        serde_json::from_str(&drafts.check().report().unwrap()).unwrap();
    let diagnostic = report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["severity"] == "error")
        .unwrap();
    assert_eq!(
        diagnostic["source_path"],
        "packages.main.graphs.start.nodes.line_four"
    );
}

#[test]
fn vn_draft_check_returns_located_scene_diagnostics() {
    let mut records = vn_records();
    let scene = records
        .iter_mut()
        .find(|bytes| {
            serde_json::from_slice::<serde_json::Value>(bytes).unwrap()["payload"]["type_id"]
                == "narrata.scene"
        })
        .unwrap();
    let mut record: serde_json::Value = serde_json::from_slice(scene).unwrap();
    let duplicate = record["payload"]["data"]["layers"][0].clone();
    record["payload"]["data"]["layers"]
        .as_array_mut()
        .unwrap()
        .push(duplicate);
    *scene = narrata_nodes::canonical_json(&record).unwrap();
    let mut drafts = VnDrafts::new();
    for record in records {
        drafts.add(&record);
    }
    let report: serde_json::Value =
        serde_json::from_str(&drafts.check().report().unwrap()).unwrap();
    let diagnostic = report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["severity"] == "error")
        .unwrap();
    assert_eq!(
        diagnostic["source_path"],
        "packages.main.graphs.start.nodes.scene_one.layers[1]"
    );
}

#[test]
fn failed_gamebook_preview_keeps_compared_when_a_baseline_was_supplied() {
    let source = demo();
    let previous = compile(&source, &NodeRegistry::gamebook()).unwrap().program;
    let mut invalid = DraftSet::default();
    invalid.add(b"{}");
    let failed = invalid.compose(Some(&previous), None).unwrap();
    assert!(failed.report.artifact_id.is_none());
    assert!(failed.report.compared);

    let valid = drafts(&source_records());
    let successful = valid.compose(Some(&previous), None).unwrap();
    assert!(successful.report.artifact_id.is_some());
    assert!(successful.report.compared);
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
fn loader_rejects_over_budget_descriptors_before_requesting_any_objects() {
    use narrata_authoring::{objects::Digest, publication::PublicationDescription};
    let original: PublicationDescription =
        serde_json::from_slice(&product("story.publication.json")).unwrap();
    for index in 0..original.objects.len() {
        let mut description = original.clone();
        // Includes a 128 MiB Chunk claim; no object payload is allocated or supplied.
        description.objects[index].bytes = if description.objects[index].role
            == narrata_authoring::publication::ObjectRole::Chunk
        {
            128 * 1024 * 1024
        } else {
            u64::MAX
        };
        let bytes = narrata_nodes::canonical_json(&description).unwrap();
        let expected = Digest::from_bytes(narrata_kernel::codec::digest_bytes(
            "narrata.nodes.publication",
            1,
            &bytes,
        ));
        assert!(Loader::new(&bytes, expected).is_err());
        let mut fetched = false;
        assert!(
            narrata_authoring::publication::read_publication(&bytes, expected, |_| {
                fetched = true;
                Ok(Vec::new())
            })
            .is_err()
        );
        assert!(!fetched, "metadata rejection precedes object loading");
    }
}

#[test]
#[ignore = "allocates more than 64 MiB; run through a heavy Goal slot"]
fn binding_publishes_and_reopens_individual_objects_above_the_pack_limit() {
    use narrata_nodes::NodeId;
    use serde_json::json;

    // Same legal scalar-argument scale pattern as the authoring core's publication test.
    let padding = "x".repeat(60_000);
    let mut paths = BTreeMap::new();
    let mut packages = Vec::new();
    for package in 0..19 {
        let alias = format!("p{package:02}");
        paths.insert(alias.clone(), format!("{alias}.json"));
        let mut nodes = BTreeMap::new();
        for index in 0..60 {
            nodes.insert(format!("n{index}"), json!({
                "id": NodeId::from_bytes(((package * 100 + index + 1) as u128).to_be_bytes()),
                "type_id": "narrata.passage",
                "data": {"args": {"padding": {"kind": "literal", "value": padding}},
                    "next": if index == 59 { "end".to_owned() } else { format!("n{}", index + 1) }}
            }));
        }
        nodes.insert(
            "end".into(),
            json!({
                "id": NodeId::from_bytes(((package * 100 + 99) as u128).to_be_bytes()),
                "type_id": "narrata.return", "data": {"outcome": "done"}
            }),
        );
        packages.push((
            alias.clone(),
            serde_json::from_value(json!({
                "id": alias, "version": "1", "exports": ["main"],
                "graphs": {"main": {"entry": "n0", "outcomes": ["done"], "nodes": nodes}}
            }))
            .unwrap(),
        ));
    }
    let mut builder = SourceBuilder::new(serde_json::from_value(json!({
        "format_version": 2, "packages": paths,
        "product": {"id": "large", "entry": {"package": "p00", "graph": "main"}, "endings": {"done": {}}}
    })).unwrap());
    for (alias, package) in packages {
        builder.add_package(&alias, package).unwrap();
    }
    let split = builder.split();
    drop(builder);
    assert!(split.report.complete && split.report.diagnostics.is_empty());
    let mut drafts = DraftSet::default();
    for record in &split.records {
        drafts.add(record.bytes());
    }
    drop(split);
    let published = drafts.publish(None, None);
    drop(drafts);
    let publication = published.publication.unwrap();
    let total = publication
        .objects()
        .map(|(_, bytes)| bytes.len())
        .sum::<usize>();
    assert!(total > narrata_nodes::MAX_PACK_BYTES);
    assert!(publication.pack().is_err());
    let mut loader = Loader::new(
        publication.description_bytes(),
        publication.publication_id(),
    )
    .unwrap();
    for (reference, bytes) in publication.objects() {
        loader.supply(reference.object_id, bytes).unwrap();
    }
    let reopened = loader.finish().unwrap();
    assert_eq!(reopened.publication_id(), publication.publication_id());
    assert_eq!(
        reopened.program().artifact_id(),
        publication.program().artifact_id()
    );
    println!(
        "binding_publication_bytes={total}; objects={}",
        reopened.description().objects.len()
    );
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

#[test]
fn fill_keeps_live_and_deleted_source_ids_occupied_for_later_mints() {
    let clock = || Ok(1_700_000_000_000);
    let entropy = |byte: u8| {
        move |bytes: &mut [u8]| {
            bytes.fill(byte);
            Ok(())
        }
    };
    let never = || Err(Error::new("clock", "ids", "must not mint"));
    let refuse = |_: &mut [u8]| Err(Error::new("random", "ids", "must not mint"));
    // IDs a repeated entropy source would produce, placed into an already-minted source.
    let live = Occupancy::default()
        .mint(1, 0, 0, clock, entropy(7))
        .unwrap()
        .nodes[0];
    let deleted = Occupancy::default()
        .mint(0, 0, 1, clock, entropy(8))
        .unwrap()
        .options[0];
    let mut source = demo();
    let road = source.packages.get_mut("road").unwrap();
    road.graphs
        .get_mut("crossing")
        .unwrap()
        .nodes
        .get_mut("fog")
        .unwrap()
        .id = Some(live);
    road.tombstones.push(AuthoredId::Option(deleted));

    let mut occupancy = Occupancy::default();
    for _ in 0..2 {
        let filled = occupancy.fill(&source, never, refuse).unwrap();
        assert!(filled.report.minted.is_empty());
        assert!(filled.report.diagnostics.is_empty());
        // Neither kind may reuse the live node's bytes or the tombstoned option's bytes.
        for (byte, counts) in [
            (7, (1, 0, 0)),
            (7, (0, 1, 0)),
            (8, (0, 0, 1)),
            (8, (1, 0, 0)),
        ] {
            let (nodes, points, options) = counts;
            assert_eq!(
                occupancy
                    .mint(nodes, points, options, clock, entropy(byte))
                    .unwrap_err()
                    .code,
                "random"
            );
        }
    }
    let fresh = occupancy.mint(1, 1, 1, clock, entropy(9)).unwrap_err();
    assert_eq!(
        fresh.code, "random",
        "one fixed source yields one ID per call"
    );
    let minted = occupancy.mint(0, 0, 1, clock, entropy(9)).unwrap();
    assert_ne!(
        AuthoredId::Option(minted.options[0]),
        AuthoredId::Option(deleted)
    );
}
