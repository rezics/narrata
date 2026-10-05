//! Produces a new labels corpus; never replaces existing frozen evidence.

use std::{error::Error, fs, path::PathBuf};

use narrata_graph::encode_labels;
use narrata_kernel::codec::sha256;

#[path = "../tests/support/labels.rs"]
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
    let objects = support::tables()
        .into_iter()
        .map(|table| encode_labels(&table).map(|object| (table.cluster, object)))
        .collect::<Result<Vec<_>, _>>()?;
    fs::create_dir_all(&output)?;
    let mut files = Vec::new();
    for (cluster, object) in objects {
        fs::write(output.join(object.filename()), &object.bytes)?;
        files.push(serde_json::json!({
            "cluster": cluster, "object_id": hex::encode(object.id), "file": object.filename(),
            "bytes": object.bytes.len(), "sha256": hex::encode(sha256(&object.bytes)),
        }));
    }
    let manifest = serde_json::json!({"format_version": 1, "files": files});
    let mut bytes = serde_json::to_vec_pretty(&manifest)?;
    bytes.push(b'\n');
    fs::write(output.join("manifest.json"), bytes)?;
    Ok(())
}
