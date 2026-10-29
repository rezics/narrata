#![allow(clippy::unwrap_used, clippy::panic)]

use narrata_authoring::{
    CheckedEvolution, CheckedPublication, PlannedEvolution,
    objects::Digest,
    plan_evolution,
    publication::{
        ObjectRole, publication_from_pack, read_publication, read_publication_with_source,
    },
};
use narrata_nodes::{canonical_json, history::evolution::MigrationId};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

fn corpus(case: &str, file: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../../../fixtures/compat/nodes-evolution-v1")
            .join(case)
            .join(file),
    )
    .unwrap()
}
fn publication(case: &str, file: &str) -> CheckedPublication {
    publication_from_pack(&corpus(case, file)).unwrap()
}
fn objects(p: &CheckedPublication) -> BTreeMap<Digest, Vec<u8>> {
    p.objects()
        .map(|(r, b)| (r.object_id, b.to_vec()))
        .collect()
}

#[test]
fn planning_preserves_every_frozen_descriptor_and_exact_alias_identity() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../../fixtures/compat/nodes-evolution-v1");
    for entry in std::fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if !path.is_dir() || !path.join("target.narpack").exists() {
            continue;
        }
        let case = path.file_name().unwrap().to_str().unwrap();
        let previous = publication(case, "source.narpack");
        let candidate = publication(case, "target.narpack");
        let declarations = std::fs::read_to_string(path.join("declarations.json")).ok();
        let a = plan_evolution(&previous, &candidate, declarations.as_deref()).unwrap();
        let b = plan_evolution(&previous, &candidate, declarations.as_deref()).unwrap();
        assert_eq!(
            canonical_json(&a.report()).unwrap(),
            canonical_json(&b.report()).unwrap()
        );
        match a {
            PlannedEvolution::Exact(id) => {
                assert_eq!(case, "content-alias-exact");
                assert_eq!(id, previous.program().artifact_id());
                assert_eq!(id, candidate.program().artifact_id());
            }
            PlannedEvolution::Descriptor(d) => {
                assert_eq!(d.bytes(), corpus(case, "descriptor.cbor"), "{case}");
                let read =
                    CheckedEvolution::read(&d.bytes(), d.id(), &previous, &candidate).unwrap();
                assert_eq!(read.id(), d.id());
            }
        }
    }
}

#[test]
fn publication_attachment_is_checked_addressed_idempotent_and_requires_its_source_on_read() {
    let previous = publication("unseen-rule", "source.narpack");
    let candidate = publication("unseen-rule", "target.narpack");
    let original_id = candidate.publication_id();
    let original_pack = candidate.pack().unwrap();
    let PlannedEvolution::Descriptor(d) = plan_evolution(&previous, &candidate, None).unwrap()
    else {
        panic!("descriptor")
    };
    let attached = candidate.with_evolution(&previous, &d).unwrap();
    assert_ne!(attached.publication_id(), original_id);
    assert_eq!(attached.pack().unwrap(), original_pack);
    let reference = attached
        .description()
        .objects
        .iter()
        .find(|r| r.role == ObjectRole::Evolution)
        .unwrap();
    assert_eq!(reference.object_id.as_bytes(), d.id().as_bytes());
    assert_eq!(attached.object(reference.object_id).unwrap(), d.bytes());
    let map = objects(&attached);
    assert!(
        read_publication(
            attached.description_bytes(),
            attached.publication_id(),
            |id| Ok(map[&id].clone())
        )
        .is_err()
    );
    let read = read_publication_with_source(
        attached.description_bytes(),
        attached.publication_id(),
        |id| Ok(map[&id].clone()),
        Some(&previous),
    )
    .unwrap();
    assert_eq!(read.evolution(&previous).unwrap().unwrap().id(), d.id());
    let id = read.publication_id();
    let repeated = read.with_evolution(&previous, &d).unwrap();
    assert_eq!(repeated.publication_id(), id);
    let wrong = publication("variable-type", "target.narpack");
    assert!(
        read_publication_with_source(
            attached.description_bytes(),
            attached.publication_id(),
            |id| Ok(map[&id].clone()),
            Some(&wrong)
        )
        .is_err()
    );
    assert!(
        CheckedEvolution::read(
            &d.bytes(),
            MigrationId::from_bytes([1; 32]),
            &previous,
            &attached
        )
        .is_err()
    );
    let mut corrupt = map;
    corrupt.get_mut(&reference.object_id).unwrap()[0] ^= 1;
    assert!(
        read_publication_with_source(
            attached.description_bytes(),
            attached.publication_id(),
            |id| Ok(corrupt[&id].clone()),
            Some(&previous)
        )
        .is_err()
    );
}

#[test]
fn declaration_boundaries_do_not_weaken_core_validation() {
    let previous = publication("deleted-ending", "source.narpack");
    let candidate = publication("deleted-ending", "target.narpack");
    let valid = String::from_utf8(corpus("deleted-ending", "declarations.json")).unwrap();
    let value: serde_json::Value = serde_json::from_str(&valid).unwrap();
    let mut duplicate = value.clone();
    let recoveries = duplicate["recoveries"].as_array_mut().unwrap();
    recoveries.push(recoveries[0].clone());
    assert!(plan_evolution(&previous, &candidate, Some(&duplicate.to_string())).is_err());
    let mut wrong_source = value.clone();
    wrong_source["from_artifact"] =
        serde_json::to_value(candidate.program().artifact_id()).unwrap();
    assert!(plan_evolution(&previous, &candidate, Some(&wrong_source.to_string())).is_err());
    assert!(
        plan_evolution(
            &previous,
            &candidate,
            Some(&valid.replacen(
                "\"format_version\":1",
                "\"format_version\":1,\"format_version\":1",
                1
            ))
        )
        .is_err()
    );
    assert!(plan_evolution(&previous, &candidate, Some(&" ".repeat(1024 * 1024 + 1))).is_err());
    let mut unknown = value;
    unknown["script"] = serde_json::json!("ignored?");
    assert!(plan_evolution(&previous, &candidate, Some(&unknown.to_string())).is_err());
}
