//! Filesystem adapter for the shared, Wasm-compatible graph publication core.

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
use crate::{pretty, write_bytes, write_text};
pub use narrata_authoring::graph::*;
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
use narrata_nodes::{Error, Result};
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
use std::path::Path;

/// Writes object files first, then the JSON references. Each file is atomic; the set is not
/// a transaction. Existing immutable objects can remain available to readers of old indexes.
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
pub fn write_publication(output: &Path, publication: &PublishedGraph) -> Result<()> {
    GraphFiles::decode_json(&narrata_nodes::canonical_json(&publication.files)?)?;
    let objects = output.with_extension("graph");
    for object in std::iter::once(&publication.geometry.index)
        .chain(&publication.geometry.tiles)
        .chain(&publication.labels)
    {
        write_bytes(&objects.join(object.filename()), &object.bytes)?;
    }
    let summary = publication.summary.encode_json().map_err(graph_error)?;
    write_bytes(&output.with_extension("summary.json"), &summary)?;
    write_text(
        &output.with_extension("graph.json"),
        &pretty(&publication.files)?,
    )
}

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
fn graph_error(error: impl std::fmt::Display) -> Error {
    Error::new("graph_publication", "graph", error.to_string())
}

/// Immutable object files precede the description. Repeated writes accept only identical
/// bytes; a failed write never changes which description the caller's host considers current.
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
pub fn write_object_collection(
    description: &Path,
    objects: &Path,
    publication: &narrata_authoring::CheckedPublication,
) -> Result<()> {
    for (reference, bytes) in publication.objects() {
        let path = objects.join(reference.filename());
        crate::files::write_immutable(
            &path,
            bytes,
            narrata_graph::wire::MAX_OBJECT_BYTES as usize,
        )?;
    }
    let references: std::collections::BTreeMap<_, _> = publication
        .description()
        .objects
        .iter()
        .map(|r| (r.object_id, r))
        .collect();
    narrata_authoring::publication::read_publication(
        publication.description_bytes(),
        publication.publication_id(),
        |id| {
            let reference = references.get(&id).ok_or_else(|| {
                Error::new(
                    "missing_object",
                    id.to_string(),
                    "unknown publication object",
                )
            })?;
            crate::files::read_limited(
                &objects.join(reference.filename()),
                reference.bytes as usize,
            )
        },
    )?;
    write_bytes(description, publication.description_bytes())
}
