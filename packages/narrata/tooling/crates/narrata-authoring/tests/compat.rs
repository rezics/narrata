#![allow(clippy::unwrap_used, clippy::panic)]

mod support;

use narrata_authoring::validate_project;
use narrata_nodes::{NodeRegistry, compile};

#[test]
fn frozen_records_source_and_lock_regenerate_twice_to_the_same_bytes() {
    let source = support::demo().unwrap();
    let first = support::corpus_files(&source).unwrap();
    let second = support::corpus_files(&source).unwrap();
    assert_eq!(first, second);
    let corpus = support::repository().join("fixtures/compat/draft-records-v1");
    for (path, bytes) in &first {
        assert_eq!(&std::fs::read(corpus.join(path)).unwrap(), bytes, "{path}");
    }
    let inputs: Vec<_> = first
        .iter()
        .filter(|(path, _)| path.starts_with("records/"))
        .map(|(_, bytes)| bytes.as_slice())
        .collect();
    let checked = validate_project(&inputs).into_result().unwrap();
    assert_eq!(
        checked.compilation.pack,
        std::fs::read(support::repository().join("products/gamebook-demo/story.narpack")).unwrap()
    );
    assert_eq!(
        checked.compilation.lock,
        compile(&source, &NodeRegistry::gamebook()).unwrap().lock
    );
}
