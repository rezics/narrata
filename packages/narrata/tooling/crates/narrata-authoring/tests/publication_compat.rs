#![allow(clippy::unwrap_used, clippy::panic)]
#[path = "support/publication.rs"]
mod publication;
mod support;
use support::demo;

#[test]
fn frozen_publication_and_relations_regenerate_twice_byte_for_byte() {
    let first = publication::corpus_files().unwrap();
    assert_eq!(first, publication::corpus_files().unwrap());
    for (path, bytes) in first {
        let checked = if let Some(path) = path.strip_prefix("product/") {
            support::repository()
                .join("products/gamebook-demo")
                .join(path)
        } else {
            support::repository().join("fixtures/compat").join(&path)
        };
        assert_eq!(std::fs::read(checked).unwrap(), bytes, "{path}");
    }
}
