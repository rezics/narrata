#![allow(clippy::panic, clippy::unwrap_used)]

use narrata_content_local::{
    Content, ContentPack, LocalContent, Payload, Resolution, ResolveContext, ResolveItem,
    ResolveRequest,
};
use narrata_kernel::content::{ContentRef, Segment};
use narrata_nodes::ViewScalar;
use proptest::prelude::*;
use serde_json::json;

fn pack(language: &str, greeting: &str) -> ContentPack {
    serde_json::from_value(json!({
        "format_version": 1,
        "provider": "local",
        "language": language,
        "entries": {
            "title": {"text": greeting},
            "visitor": {"text": "you"},
            "count": {"text": "{name} has {coins} coins, {{literally}}"},
            "unit": {"blocks": [
                {"id": "b1", "text": "One."},
                {"id": "m1", "choice_point": "choice-point:0000000000000000000000000000000a"},
                {"id": "b2", "text": "Two, {visitor}."},
                {"id": "b3", "text": "Three."}
            ]}
        }
    }))
    .unwrap()
}

fn content() -> LocalContent {
    let mut content = LocalContent::new();
    content.add(pack("en", "Hello")).unwrap();
    content.add(pack("zh-Hans", "你好")).unwrap();
    content
}

fn reference(key: &str) -> ContentRef {
    ContentRef::new("local", key).unwrap()
}

fn item(content: Content) -> ResolveItem {
    ResolveItem {
        content,
        args: Default::default(),
    }
}

fn segment(first: Option<&str>, last: Option<&str>) -> Content {
    Content::Segment(Segment {
        unit: reference("unit"),
        first: first.map(|anchor| anchor.parse().unwrap()),
        last: last.map(|anchor| anchor.parse().unwrap()),
    })
}

fn resolve(content: &LocalContent, languages: &[&str], items: Vec<ResolveItem>) -> Vec<Resolution> {
    content
        .resolve(&ResolveRequest {
            context: ResolveContext {
                languages: languages
                    .iter()
                    .map(|language| (*language).to_owned())
                    .collect(),
                ..ResolveContext::default()
            },
            items,
        })
        .unwrap()
}

#[test]
fn optional_host_context_round_trips_and_is_ignored_locally() {
    let old: ResolveRequest = serde_json::from_value(json!({
        "context": {"languages": ["zh-Hans"]},
        "items": [{"content": {"provider": "local", "key": "title"}}]
    }))
    .unwrap();
    assert_eq!(old.context.realization, None);
    assert_eq!(old.context.viewer, None);
    let mut remote = old.clone();
    remote.context.realization = Some("expression:translated-revision".into());
    remote.context.viewer = Some("host-reader:opaque".into());
    assert_eq!(
        content().resolve(&old).unwrap(),
        content().resolve(&remote).unwrap()
    );
    assert_eq!(
        serde_json::from_str::<ResolveRequest>(&serde_json::to_string(&remote).unwrap()).unwrap(),
        remote
    );
    assert!(
        serde_json::from_value::<ResolveRequest>(json!({"context": {"viewer": 3}, "items": []}))
            .is_err()
    );
}

#[test]
fn resolve_schemas_match_the_rust_exchange_types() {
    for (name, schema) in narrata_content_local::schemas() {
        if name == "content-resolve-request.schema.json" || name == "content-resolution.schema.json"
        {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../../nodes/schemas")
                .join(name);
            assert_eq!(
                std::fs::read_to_string(path).unwrap(),
                serde_json::to_string_pretty(&schema).unwrap() + "\n"
            );
        }
    }
}

fn text(resolution: &Resolution) -> String {
    match resolution {
        Resolution::Ok {
            payload: Payload::Text { text },
            ..
        } => text.clone(),
        Resolution::Ok {
            payload: Payload::Blocks { blocks },
            ..
        } => blocks
            .iter()
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>()
            .join("|"),
        other => panic!("not resolved: {other:?}"),
    }
}

#[test]
fn a_segment_resolves_the_text_blocks_between_its_anchors() {
    let content = content();
    let mut visitor = item(segment(Some("b1"), Some("b2")));
    visitor
        .args
        .insert("visitor".into(), ViewScalar::Ref(reference("visitor")));
    let results = resolve(&content, &["en"], vec![visitor]);
    // The choice point marker has no text and is skipped.
    assert_eq!(text(&results[0]), "One.|Two, you.");
}

#[test]
fn an_omitted_anchor_means_the_start_or_end_of_the_unit() {
    let content = content();
    let mut whole = item(segment(None, None));
    whole
        .args
        .insert("visitor".into(), ViewScalar::Text("me".into()));
    let mut tail = item(segment(Some("b2"), None));
    tail.args
        .insert("visitor".into(), ViewScalar::Text("me".into()));
    let results = resolve(
        &content,
        &["en"],
        vec![whole, tail, item(segment(None, Some("b1")))],
    );
    assert_eq!(text(&results[0]), "One.|Two, me.|Three.");
    assert_eq!(text(&results[1]), "Two, me.|Three.");
    assert_eq!(text(&results[2]), "One.");
}

#[test]
fn named_arguments_format_ints_texts_and_escaped_braces() {
    let content = content();
    let mut count = item(Content::Ref(reference("count")));
    count
        .args
        .insert("name".into(), ViewScalar::Text("Ann".into()));
    count
        .args
        .insert("coins".into(), ViewScalar::Int("-3".into()));
    let results = resolve(&content, &["en"], vec![count]);
    assert_eq!(text(&results[0]), "Ann has -3 coins, {literally}");
}

#[test]
fn missing_entries_are_unavailable() {
    let content = content();
    let results = resolve(
        &content,
        &["en"],
        vec![
            item(Content::Ref(reference("absent"))),
            item(Content::Ref(ContentRef::new("rezics", "title").unwrap())),
        ],
    );
    assert_eq!(
        results,
        vec![Resolution::Unavailable, Resolution::Unavailable]
    );
}

#[test]
fn missing_anchors_arguments_and_reversed_segments_are_incompatible() {
    let content = content();
    let results = resolve(
        &content,
        &["en"],
        vec![
            item(segment(Some("b9"), None)),
            item(segment(Some("b3"), Some("b1"))),
            item(segment(Some("b2"), Some("b2"))),
            item(Content::Segment(Segment {
                unit: reference("title"),
                first: Some("b1".parse().unwrap()),
                last: None,
            })),
        ],
    );
    assert!(
        results
            .iter()
            .all(|result| matches!(result, Resolution::Incompatible { .. })),
        "{results:?}"
    );
}

#[test]
fn one_language_is_chosen_for_the_whole_batch_with_lookup_and_fallback() {
    let content = content();
    let title = || item(Content::Ref(reference("title")));
    assert_eq!(
        text(&resolve(&content, &["zh-Hans-CN"], vec![title()])[0]),
        "你好"
    );
    assert_eq!(
        text(&resolve(&content, &["fr", "zh-hans"], vec![title()])[0]),
        "你好"
    );
    // No match falls back to the original language, the first pack added.
    assert_eq!(text(&resolve(&content, &["fr"], vec![title()])[0]), "Hello");
    assert_eq!(text(&resolve(&content, &[], vec![title()])[0]), "Hello");
}

#[test]
fn the_revision_changes_only_when_the_entry_text_changes() {
    let revision = |content: &LocalContent, key: &str| match &resolve(
        content,
        &["en"],
        vec![item(Content::Ref(reference(key)))],
    )[0]
    {
        Resolution::Ok { revision, .. } => revision.clone(),
        other => panic!("{other:?}"),
    };
    let base = content();
    let mut edited = LocalContent::new();
    edited.add(pack("en", "Hi")).unwrap();
    assert_ne!(revision(&base, "title"), revision(&edited, "title"));
    assert_eq!(revision(&base, "visitor"), revision(&edited, "visitor"));
}

#[test]
fn the_outline_lists_blocks_and_choice_point_markers_without_text() {
    let content = content();
    let outline = content
        .outline(&"local".parse().unwrap(), &["en".into()])
        .unwrap();
    let unit = outline.units.get(&"unit".parse().unwrap()).unwrap();
    assert_eq!(
        unit.blocks
            .iter()
            .map(|block| block.as_str())
            .collect::<Vec<_>>(),
        vec!["b1", "m1", "b2", "b3"]
    );
    assert_eq!(unit.markers.len(), 1);
    let json = serde_json::to_string(&outline).unwrap();
    assert!(!json.contains("One.") && !json.contains("Hello"));
}

#[test]
fn packs_reject_duplicate_blocks_languages_and_bad_tags() {
    let mut duplicate = pack("en", "Hello");
    duplicate.entries.insert(
        "twice".parse().unwrap(),
        serde_json::from_value(
            json!({"blocks": [{"id": "a", "text": "1"}, {"id": "a", "text": "2"}]}),
        )
        .unwrap(),
    );
    assert_eq!(
        duplicate.check().err().map(|e| e.code).as_deref(),
        Some("duplicate")
    );
    let mut content = content();
    assert_eq!(
        content
            .add(pack("EN", "x"))
            .err()
            .map(|e| e.code)
            .as_deref(),
        Some("duplicate")
    );
    assert_eq!(
        content
            .add(pack("not a tag", "x"))
            .err()
            .map(|e| e.code)
            .as_deref(),
        Some("language")
    );
    assert!(
        ContentPack::parse(
            r#"{"format_version":1,"provider":"local","language":"en","entries":{},"extra":1}"#
        )
        .is_err()
    );
}

proptest! {
    /// Text never contains braces after formatting unless they were escaped.
    #[test]
    fn formatting_round_trips_escaped_literal_text(literal in "[^{}]{0,40}") {
        let mut content = LocalContent::new();
        let escaped = format!("{{{{{literal}}}}}");
        content
            .add(serde_json::from_value(json!({
                "format_version": 1,
                "provider": "local",
                "language": "en",
                "entries": {"t": {"text": escaped}}
            }))
            .unwrap())
            .unwrap();
        let results = resolve(&content, &["en"], vec![item(Content::Ref(reference("t")))]);
        prop_assert_eq!(text(&results[0]), format!("{{{literal}}}"));
    }
}
