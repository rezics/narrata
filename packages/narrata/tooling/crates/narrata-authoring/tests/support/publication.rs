#![allow(dead_code)]

use narrata_authoring::{CheckedRecord, ComposeMode, publish_records, split_source};
use narrata_nodes::{Result, canonical_json};
use serde_json::json;
use std::collections::BTreeMap;

pub fn corpus_files() -> Result<BTreeMap<String, Vec<u8>>> {
    let source = super::demo()?;
    let records = split_source(&source).into_result()?;
    let inputs: Vec<_> = records.iter().map(CheckedRecord::bytes).collect();
    let published = publish_records(&inputs, &[], None, ComposeMode::Update).into_result()?;
    let publication = &published.publication;
    let mut files = BTreeMap::new();
    files.insert(
        "publication-v1/publication.json".into(),
        publication.description_bytes().to_vec(),
    );
    files.insert(
        "publication-v1/diagnostics.json".into(),
        canonical_json(
            &publish_records(&inputs, &[], None, ComposeMode::Update).diagnostic_report(),
        )?,
    );
    let mut identities = Vec::new();
    for (reference, bytes) in publication.objects() {
        let path = format!("objects/{}", reference.filename());
        identities.push(json!({"file":path,"role":reference.role,"object_id":reference.object_id,"bytes":bytes.len(),"sha256":hex::encode(narrata_kernel::codec::sha256(bytes))}));
        files.insert(format!("publication-v1/{path}"), bytes.to_vec());
        files.insert(
            format!("product/story.publication/{}", reference.filename()),
            bytes.to_vec(),
        );
    }
    files.insert(
        "product/story.publication.json".into(),
        publication.description_bytes().to_vec(),
    );
    files.insert("publication-v1/manifest.json".into(),canonical_json(&json!({"corpus_version":1,"work":"山口来信","artifact_id":publication.program().artifact_id(),"publication_id":publication.publication_id(),"objects":identities,"generation":"task goal -- slot -- cargo run -p narrata-authoring --example freeze_publication -- .temp/publication-candidate; compare twice before freezing new directories"}))?);
    let relations = publication.relations();
    files.insert(
        "relations-v1/index.json".into(),
        relations.index_object().bytes.clone(),
    );
    files.insert(
        "relations-v1/rows.json".into(),
        canonical_json(&relations.rows())?,
    );
    for part in relations.parts() {
        files.insert(
            format!("relations-v1/parts/{}.json", part.id),
            part.bytes.clone(),
        );
    }
    files.insert("relations-v1/manifest.json".into(),canonical_json(&json!({"corpus_version":1,"work":"山口来信","artifact_id":publication.program().artifact_id(),"projection_version":1,"index_id":relations.index_object().id,"parts":relations.index().parts,"counts":relations.index().counts,"row_count":relations.index().row_count}))?);
    Ok(files)
}
