//! Generate a candidate corpus in a temporary directory; frozen compat data is read-only.

#[path = "../tests/support/mod.rs"]
mod support;

fn main() -> narrata_nodes::Result<()> {
    let output = std::env::args().nth(1).ok_or_else(|| {
        narrata_nodes::Error::new(
            "arguments",
            "output",
            "provide a new temporary output directory",
        )
    })?;
    support::write_corpus(std::path::Path::new(&output))
}
