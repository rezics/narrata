//! Produces a new corpus directory; refuses to overwrite frozen evidence.

use std::{error::Error, fs, path::PathBuf};

use narrata_graph::{prepare, wire::encode_publication};
use narrata_kernel::codec::sha256;

#[path = "../tests/support/mod.rs"]
mod support;

fn main() -> Result<(), Box<dyn Error>> {
    let output = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("supply a new output directory")?,
    );
    if output.exists() {
        return Err("refusing to overwrite a corpus directory".into());
    }
    let publication = prepare(support::compatibility_graph())?;
    let encoded = encode_publication(&publication)?;
    let summary = publication.summary.encode_json()?;
    let mut files = Vec::new();
    fs::create_dir_all(&output)?;
    for object in std::iter::once(&encoded.index).chain(&encoded.tiles) {
        let name = object.filename();
        fs::write(output.join(&name), &object.bytes)?;
        files.push(serde_json::json!({ "file": name, "bytes": object.bytes.len(), "sha256": hex::encode(sha256(&object.bytes)) }));
    }
    fs::write(output.join("summary.json"), &summary)?;
    files.push(serde_json::json!({ "file": "summary.json", "bytes": summary.len(), "sha256": hex::encode(sha256(&summary)) }));
    let manifest = serde_json::json!({
        "format_version": 1,
        "index_object_id": hex::encode(encoded.index.id),
        "files": files,
    });
    let mut bytes = serde_json::to_vec_pretty(&manifest)?;
    bytes.push(b'\n');
    fs::write(output.join("manifest.json"), bytes)?;
    Ok(())
}
