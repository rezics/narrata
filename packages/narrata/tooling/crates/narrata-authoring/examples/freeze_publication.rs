//! Writes a candidate under .temp; it never updates a frozen compat directory.
#[path = "../tests/support/publication.rs"]
mod publication;
#[path = "../tests/support/mod.rs"]
mod support;
use support::demo;

fn main() -> narrata_nodes::Result<()> {
    let output = std::env::args().nth(1).ok_or_else(|| {
        narrata_nodes::Error::new(
            "arguments",
            "output",
            "supply a temporary candidate directory",
        )
    })?;
    let output = std::path::Path::new(&output);
    if !output.components().any(|c| c.as_os_str() == ".temp") {
        return Err(narrata_nodes::Error::new(
            "arguments",
            "output",
            "candidate output must be under .temp",
        ));
    }
    for (path, bytes) in publication::corpus_files()? {
        narrata_node_tools::write_bytes(&output.join(path), &bytes)?;
    }
    Ok(())
}
