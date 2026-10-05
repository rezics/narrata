#![allow(clippy::unwrap_used)]

use narrata_nodes::{ContentOutline, outline::diagnose_outline};
use serde_json::{Value, json};

mod support;

fn outline() -> Value {
    json!({
        "format_version": 1, "provider": "local", "units": {
            "main.gate": {
                "blocks": ["b1", "r-wave", "r-nod", "b2", "r-rope", "r-lamp", "b3", "b4"],
                "markers": {"b1": support::point(1), "b2": support::point(2)}
            },
            "side.talk": {"blocks": ["talk"]}
        }
    })
}

fn codes(value: Value) -> Vec<String> {
    let compilation = support::compiled();
    let outline: ContentOutline = narrata_nodes::parse_json(&value.to_string()).unwrap();
    diagnose_outline(&compilation.program, Some(&compilation.names), &outline)
        .unwrap()
        .into_iter()
        .map(|diagnostic| diagnostic.code)
        .collect()
}

#[test]
fn inclusive_body_boundaries_and_strict_reply_boundaries_are_valid() {
    assert!(codes(outline()).is_empty());
}

#[test]
fn missing_anchors_and_units_are_diagnostics() {
    let mut value = outline();
    value["units"]["main.gate"]["blocks"] = json!(["b1", "b2", "b3"]);
    value["units"].as_object_mut().unwrap().remove("side.talk");
    let codes = codes(value);
    assert!(codes.iter().any(|code| code == "outline_anchor_missing"));
    assert!(codes.iter().any(|code| code == "outline_unit_missing"));
}

#[test]
fn reversed_placements_replies_and_rejoins_are_diagnosed() {
    for blocks in [
        json!([
            "b2", "b1", "r-wave", "r-nod", "r-rope", "r-lamp", "b3", "b4"
        ]),
        json!([
            "r-wave", "b1", "r-nod", "b2", "r-rope", "r-lamp", "b3", "b4"
        ]),
        json!([
            "b1", "b2", "r-wave", "r-nod", "r-rope", "r-lamp", "b3", "b4"
        ]),
        json!([
            "b1", "r-wave", "r-nod", "b2", "b3", "r-rope", "r-lamp", "b4"
        ]),
        json!([
            "b4", "b1", "r-wave", "r-nod", "b2", "r-rope", "r-lamp", "b3"
        ]),
    ] {
        let mut value = outline();
        value["units"]["main.gate"]["blocks"] = blocks;
        assert!(codes(value).iter().any(|code| code == "outline_order"));
    }
}

#[test]
fn marker_duplicates_unknown_points_wrong_units_and_missing_blocks_are_diagnosed() {
    let mut value = outline();
    value["units"]["main.gate"]["markers"]["r-wave"] = json!(support::point(1));
    value["units"]["main.gate"]["markers"]["absent"] = json!(support::point(999));
    value["units"]["side.talk"]["markers"] = json!({"talk": support::point(2)});
    let codes = codes(value);
    for expected in [
        "outline_duplicate_marker",
        "outline_unknown_marker",
        "outline_marker_mismatch",
        "outline_anchor_missing",
    ] {
        assert!(codes.iter().any(|code| code == expected), "{codes:?}");
    }
}

#[test]
fn duplicate_block_ids_and_empty_units_are_diagnosed() {
    let mut value = outline();
    value["units"]["main.gate"]["blocks"][1] = json!("b1");
    value["units"]["side.talk"]["blocks"] = json!([]);
    let codes = codes(value);
    assert!(codes.iter().any(|code| code == "outline_duplicate_anchor"));
    assert!(codes.iter().any(|code| code == "outline_empty_unit"));
}

#[test]
fn replies_equal_to_placement_or_rejoin_and_rejoins_past_the_next_placement_are_rejected() {
    for (field, anchor) in [("first", "b1"), ("last", "b2")] {
        let compilation = support::try_variant(|_, main, _| {
            main.pointer_mut(&format!(
                "{}/choice_points/0/options/0/outcome/reply/{field}",
                support::GATE
            ))
            .map(|value| *value = json!(anchor))
            .unwrap();
        })
        .unwrap();
        let outline: ContentOutline = serde_json::from_value(outline()).unwrap();
        assert!(
            diagnose_outline(&compilation.program, None, &outline)
                .unwrap()
                .iter()
                .any(|d| d.code == "outline_order")
        );
    }
    let compilation = support::try_variant(|_, main, _| {
        main.pointer_mut(&format!(
            "{}/choice_points/0/options/0/outcome/rejoin",
            support::GATE
        ))
        .map(|value| *value = json!("b3"))
        .unwrap();
    })
    .unwrap();
    let outline: ContentOutline = serde_json::from_value(outline()).unwrap();
    assert!(
        diagnose_outline(&compilation.program, None, &outline)
            .unwrap()
            .iter()
            .any(|d| d.code == "outline_order")
    );
}

#[test]
fn outline_version_is_checked_and_other_providers_are_skipped() {
    let compilation = support::compiled();
    let mut outline: ContentOutline = serde_json::from_value(outline()).unwrap();
    outline.format_version = 2;
    assert_eq!(
        diagnose_outline(&compilation.program, None, &outline)
            .unwrap_err()
            .code,
        "outline_version"
    );
    outline.format_version = 1;
    outline.provider = "remote".parse().unwrap();
    outline.units.clear();
    assert!(
        diagnose_outline(&compilation.program, None, &outline)
            .unwrap()
            .is_empty()
    );
}
