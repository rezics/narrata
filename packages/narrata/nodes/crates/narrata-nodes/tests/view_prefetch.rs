#![allow(clippy::unwrap_used)]

mod support;

use narrata_nodes::{Session, view::next_content_units};
use serde_json::json;
use support::{GATE, choose, compiled, content, node, option, point, session, try_variant};

fn units(session: &Session) -> Vec<String> {
    next_content_units(session.program(), session.state().unwrap())
        .unwrap()
        .into_iter()
        .map(|reference| reference.key.as_str().to_owned())
        .collect()
}

#[test]
fn local_replies_and_multiselect_stop_at_the_next_interaction_without_mutation() {
    let compilation = compiled();
    let (mut session, names) = session(&compilation);
    let before = session.export().unwrap();
    assert_eq!(units(&session), vec!["main.gate"]);
    assert_eq!(units(&session), units(&session));
    assert_eq!(session.export().unwrap(), before);
    choose(&mut session, &names, &["wave"]).unwrap();
    // Both local selections (and choosing nothing) share one unit; the later branch
    // is another interaction and must not leak into this result.
    assert_eq!(units(&session), vec!["main.gate"]);
    choose(&mut session, &names, &["rope", "lamp"]).unwrap();
    // The call route presents its callee, the other route can end the story.
    assert_eq!(units(&session), vec!["ending", "side.talk"]);
    choose(&mut session, &names, &["left"]).unwrap();
    // Returning through an active caller traverses its mutation and branch.
    assert_eq!(units(&session), vec!["ending"]);
    choose(&mut session, &names, &["continue"]).unwrap();
    assert!(units(&session).is_empty());
}

#[test]
fn hidden_and_disabled_routes_are_excluded_and_only_references_escape() {
    let compilation = try_variant(|_, main, _| {
        let gate = main.pointer_mut(GATE).unwrap();
        gate["choice_points"] = json!([{"id": point(1), "key": "route", "options": [
            {"id": option(1), "key": "end", "label": content("end"), "outcome": {"kind": "branch", "target": "count"}},
            {"id": option(2), "key": "hidden", "label": content("hidden"), "visible_if": {"kind": "literal", "value": false}, "outcome": {"kind": "branch", "target": "visit"}},
            {"id": option(3), "key": "disabled", "label": content("disabled"), "enabled_if": {"kind": "literal", "value": false}, "outcome": {"kind": "branch", "target": "visit"}}
        ]}]);
    }).unwrap();
    let (session, _) = session(&compilation);
    assert_eq!(units(&session), vec!["ending"]);
    let json = serde_json::to_value(
        next_content_units(session.program(), session.state().unwrap()).unwrap(),
    )
    .unwrap();
    assert_eq!(json, json!([{"provider": "local", "key": "ending"}]));
}

#[test]
fn automatic_callees_return_to_the_matching_continuation_and_stop_after_one_choice() {
    let compilation = try_variant(|_, main, side| {
        // Replace the helper's interaction by an automatic body passage.
        side["graphs"]["visit"]["nodes"]["talk"]["data"]["choice_points"] = json!([]);
        side["graphs"]["visit"]["nodes"]["talk"]["data"]["next"] = json!("out");
        // Continue into another interactive passage after returning.
        main["graphs"]["start"]["nodes"]["tally"]["data"]["next"] = json!("after");
        main["graphs"]["start"]["nodes"]["after"] = json!({
            "id": node(99), "type_id": "narrata.passage", "data": {
                "body": {"unit": content("after.call")},
                "choice_points": [{"id": point(99), "key": "after", "options": [{"id": option(99), "key": "end", "outcome": {"kind": "branch", "target": "count"}}]}]
            }
        });
    }).unwrap();
    let (mut session, names) = session(&compilation);
    choose(&mut session, &names, &["wave"]).unwrap();
    choose(&mut session, &names, &["rope"]).unwrap();
    assert_eq!(units(&session), vec!["after.call", "ending", "side.talk"]);
}

#[test]
fn automatic_loops_have_bounded_lookahead() {
    let compilation = try_variant(|_, main, _| {
        main["graphs"]["start"]["nodes"]["count"]["data"]["when_true"] = json!("count");
    })
    .unwrap();
    let (mut session, names) = session(&compilation);
    choose(&mut session, &names, &["wave"]).unwrap();
    choose(&mut session, &names, &["rope"]).unwrap();
    assert_eq!(units(&session), vec!["side.talk"]);
}

#[test]
fn required_multiselection_does_not_enumerate_combinations() {
    let compilation = try_variant(|_, main, _| {
        main.pointer_mut(GATE).unwrap()["choice_points"][1]["min"] = json!(2);
    })
    .unwrap();
    let (mut session, names) = session(&compilation);
    choose(&mut session, &names, &["wave"]).unwrap();
    assert_eq!(units(&session), vec!["main.gate"]);
}

#[test]
fn recursive_automatic_calls_stop_at_the_runtime_depth_limit() {
    let compilation = try_variant(|_, _, side| {
        let graph = &mut side["graphs"]["visit"];
        graph["nodes"]["talk"]["data"]["choice_points"] = json!([]);
        graph["nodes"]["talk"]["data"]["next"] = json!("fork");
        graph["nodes"]["fork"] = json!({"id": node(13), "type_id": "narrata.branch", "data": {
            "condition": {"kind": "literal", "value": true}, "when_true": "again", "when_false": "out"
        }});
        graph["nodes"]["again"] = json!({"id": node(14), "type_id": "narrata.call", "data": {
            "target": {"kind": "local", "graph": "visit"},
            "arguments": {"visitor": {"kind": "read", "scope": "parameter", "name": "visitor"}},
            "on_return": {"back": "out"}
        }});
    }).unwrap();
    let (mut session, names) = session(&compilation);
    choose(&mut session, &names, &["wave"]).unwrap();
    choose(&mut session, &names, &["rope"]).unwrap();
    assert_eq!(units(&session), vec!["ending", "side.talk"]);
}
