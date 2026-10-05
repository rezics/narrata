#![allow(dead_code)]

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use narrata_authoring::{assemble_records, split_source};
use narrata_nodes::{Error, NodeRegistry, ProjectSource, Result, canonical_json, compile};
use serde_json::json;

pub fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../..")
}
pub fn demo() -> Result<ProjectSource> {
    Ok(
        narrata_node_tools::load_project(
            &repository().join("products/gamebook-demo/project.json"),
        )?
        .source,
    )
}

/// A pure generator makes reproducibility verifiable without rewriting a frozen corpus.
pub fn corpus_files(source: &ProjectSource) -> Result<BTreeMap<String, Vec<u8>>> {
    let records = split_source(source).into_result()?;
    let assembled = assemble_records(&records).into_result()?;
    let compilation = compile(&assembled, &NodeRegistry::gamebook())?;
    let mut files = BTreeMap::new();
    let mut entries = Vec::new();
    for (index, record) in records.iter().enumerate() {
        let path = format!("records/{index:04}.json");
        entries.push(json!({"file":path,"key":record.key()}));
        files.insert(path, record.bytes().to_vec());
    }
    files.insert("project.json".into(), canonical_json(&assembled.manifest)?);
    for (alias, package) in &assembled.packages {
        files.insert(format!("packages/{alias}.json"), canonical_json(package)?);
    }
    files.insert(
        "project.lock.json".into(),
        canonical_json(&compilation.lock)?,
    );
    let artifacts: BTreeMap<_, _> = files
        .iter()
        .map(|(path, bytes)| {
            let digest: String = narrata_kernel::codec::sha256(bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            (path.clone(), json!({"bytes":bytes.len(),"sha256":digest}))
        })
        .collect();
    files.insert("manifest.json".into(), canonical_json(&json!({"corpus_version":1,"format":"ADR 0022 draft records v1 and assembled R2 source","work":"山口来信","artifact_id":compilation.program.artifact_id(),"records":entries,"artifacts":artifacts,"generation":"task goal -- slot -- cargo run -p narrata-authoring --example freeze_drafts -- .temp/draft-records-v1; compare output, then copy into a new compat directory"}))?);
    Ok(files)
}

pub fn write_corpus(root: &Path) -> Result<()> {
    for (path, bytes) in corpus_files(&demo()?)? {
        narrata_node_tools::write_bytes(&root.join(path), &bytes)
            .map_err(|error| Error::new(&error.code, error.path, error.message))?;
    }
    Ok(())
}
