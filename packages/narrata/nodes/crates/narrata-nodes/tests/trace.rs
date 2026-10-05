#![cfg(feature = "trace")]
#![allow(clippy::panic, clippy::unwrap_used)]

mod support;

use narrata_nodes::{
    Commit, Input, OptionId, ProposalRequest, decode_input,
    plan::Outcome,
    view::{Interaction, Role},
};
use serde_json::json;
use support::*;

// Keep the public API consumable through Program without needing a runtime module export.
// A trace does not allocate commits: reconstruct their existing bytes from its result.
macro_rules! assert_observation {
    ($session:expr, $trace:expr, $parent:expr, $input:expr, $depth:expr) => {{
        let session = &$session;
        let trace = &$trace;
        let state = session.state().unwrap();
        assert_eq!(trace.state.encode(), state.encode());
        assert_eq!(trace.state.envelope(), state.envelope());
        assert_eq!(trace.state.id(), state.id());
        let commit = Commit {
            artifact: session.program().artifact_id(),
            parent: $parent,
            input: $input,
            state: trace.state.id(),
            depth: $depth,
        };
        assert_eq!(commit.id(), session.cursor().unwrap());
        let ordinary = session
            .commits()
            .unwrap()
            .find(|(id, _)| *id == commit.id())
            .unwrap()
            .1;
        assert_eq!(commit.envelope(), ordinary.envelope());
        let view = session.view(None).unwrap();
        assert_eq!(
            serde_json::to_vec(&trace.presentation(commit.id())).unwrap(),
            serde_json::to_vec(&view.presentation).unwrap(),
        );
        assert_eq!(
            serde_json::to_vec(&trace.interaction(session.program(), None).unwrap()).unwrap(),
            serde_json::to_vec(&view.interaction).unwrap(),
        );
    }};
}

macro_rules! labels {
    ($trace:expr) => {
        $trace
            .events
            .iter()
            .map(|event| {
                if event.is_visit() {
                    "visit"
                } else if event.selection().is_some() {
                    "selected"
                } else if event.branch_target().is_some() {
                    "branch"
                } else if event.call_target().is_some() {
                    "call"
                } else if event.returned().is_some() {
                    "return"
                } else if event.ending().is_some() {
                    "finished"
                } else if event.role() == Some(Role::Reply) {
                    "reply"
                } else if event.role() == Some(Role::Body) {
                    "body"
                } else {
                    panic!("unknown event: {event:?}")
                }
            })
            .collect::<Vec<_>>()
    };
}

#[test]
fn local_multiselect_call_return_and_ending_are_ordered_and_read_only() {
    let compilation = compiled();
    let (mut session, names) = session(&compilation);
    let program = session.program().clone();
    let initial = program.trace_initial().unwrap();
    assert_eq!(initial.events, program.trace_initial().unwrap().events);
    assert_eq!(labels!(initial), ["visit", "body"]);
    assert_eq!(initial.events[0].location.node.to_string(), node(1));
    assert_eq!(initial.events[0].location.instance, 1);
    assert!(initial.entered);
    assert_observation!(session, initial, None, None, 0);

    for (index, keys) in [
        vec!["wave"],
        vec!["rope", "lamp"],
        vec!["left"],
        vec!["continue"],
    ]
    .into_iter()
    .enumerate()
    {
        let parent = session.cursor().unwrap();
        let state = session.state().unwrap().clone();
        let before = session.export().unwrap();
        let Interaction::Choose {
            choice_point,
            options,
            ..
        } = session.view(Some(&names)).unwrap().interaction
        else {
            panic!("expected choice")
        };
        let chosen: Vec<OptionId> = keys
            .iter()
            .map(|key| {
                options
                    .iter()
                    .find(|option| option.key.as_deref() == Some(key))
                    .unwrap()
                    .id
            })
            .collect();
        let input = Input::Choose {
            choice_point,
            options: chosen.clone(),
        };
        let trace = program.trace_apply(&parent, &state, &input).unwrap();
        assert_eq!(
            trace.events,
            program.trace_apply(&parent, &state, &input).unwrap().events
        );
        assert_eq!(session.export().unwrap(), before);
        assert_eq!(session.state().unwrap(), &state);
        session
            .choose(&parent, choice_point, chosen.clone())
            .unwrap();
        assert_observation!(
            session,
            trace,
            Some(parent),
            Some(input.id()),
            (index + 1) as u64
        );
        let selections: Vec<_> = trace
            .events
            .iter()
            .filter_map(|event| event.selection())
            .collect();
        assert_eq!(
            selections
                .iter()
                .map(|(_, option, _)| *option)
                .collect::<Vec<_>>(),
            chosen
        );
        assert!(
            selections
                .iter()
                .all(|(point, _, _)| *point == choice_point)
        );
        match index {
            0 => {
                assert_eq!(labels!(trace), ["selected", "reply", "body"]);
                assert!(!trace.entered);
                assert!(matches!(
                    selections[0].2,
                    Outcome::Local {
                        reply: Some(_),
                        rejoin: Some(_)
                    }
                ));
                let segments: Vec<_> = trace
                    .events
                    .iter()
                    .filter_map(|event| event.segment())
                    .collect();
                assert_eq!(segments[0].first.as_ref().unwrap().as_str(), "r-wave");
                assert_eq!(segments[1].first.as_ref().unwrap().as_str(), "b2");
                assert_eq!(segments[1].last.as_ref().unwrap().as_str(), "b2");
            }
            1 => {
                assert_eq!(
                    labels!(trace),
                    ["selected", "selected", "reply", "reply", "body"]
                );
                let segments: Vec<_> = trace
                    .events
                    .iter()
                    .filter_map(|event| event.segment())
                    .collect();
                assert_eq!(segments[0].first.as_ref().unwrap().as_str(), "r-rope");
                assert_eq!(segments[1].first.as_ref().unwrap().as_str(), "r-lamp");
                assert_eq!(segments[2].first.as_ref().unwrap().as_str(), "b3");
                assert_eq!(segments[2].last.as_ref().unwrap().as_str(), "b4");
            }
            2 => {
                assert_eq!(
                    labels!(trace),
                    ["selected", "visit", "call", "visit", "body"]
                );
                assert_eq!(
                    selections[0].2,
                    &Outcome::Branch {
                        target: node(2).parse().unwrap()
                    }
                );
                let target = trace.events[2].call_target().unwrap();
                assert_eq!(
                    (target.graph.package.as_str(), target.graph.graph.as_str()),
                    ("side", "visit")
                );
                assert_eq!(target.node.to_string(), node(11));
                assert_eq!(target.instance, 2);
                assert_eq!(trace.events[3].location, *target);
            }
            3 => {
                assert_eq!(
                    labels!(trace),
                    [
                        "selected", "visit", "return", "visit", "visit", "branch", "visit",
                        "return", "finished"
                    ]
                );
                let visits: Vec<_> = trace
                    .events
                    .iter()
                    .filter(|event| event.is_visit())
                    .map(|event| event.location.node.to_string())
                    .collect();
                assert_eq!(visits, [node(12), node(3), node(4), node(5)]);
                let (outcome, continuation) = trace.events[2].returned().unwrap();
                assert_eq!(outcome, "back");
                let continuation = continuation.unwrap();
                assert_eq!(continuation.node.to_string(), node(3));
                assert_eq!(continuation.instance, 1);
                assert_eq!(
                    trace.events[5].branch_target().unwrap().to_string(),
                    node(5)
                );
                assert_eq!(trace.events[7].returned(), Some(("done", None)));
                let (outcome, body) = trace.events[8].ending().unwrap();
                assert_eq!(outcome, "done");
                let body = body.unwrap();
                assert_eq!(body.unit.key.as_str(), "ending");
                assert_eq!(body.first.as_ref().unwrap().as_str(), "e1");
                assert_eq!(trace.events[8].segment(), Some(body));
                assert_eq!(trace.events[8].role(), None);
            }
            _ => unreachable!(),
        }
    }
}

#[test]
fn initial_records_all_automatic_nodes_and_the_false_branch_without_an_ending_body() {
    let compilation = try_variant(|_, main, side| {
        set(main, "/graphs/start", "entry", json!("visit"));
        set(
            side,
            "/graphs/visit/nodes/talk/data",
            "choice_points",
            json!([]),
        );
        set(side, "/graphs/visit/nodes/talk/data", "next", json!("out"));
    })
    .unwrap();
    let (session, _) = session(&compilation);
    let trace = session.program().trace_initial().unwrap();
    assert_observation!(session, trace, None, None, 0);
    assert_eq!(
        trace.events,
        session.program().trace_initial().unwrap().events
    );
    assert_eq!(
        labels!(trace),
        [
            "visit", "call", "visit", "body", "visit", "return", "visit", "visit", "branch",
            "visit", "return", "finished"
        ]
    );
    assert_eq!(
        trace.events[8].branch_target().unwrap().to_string(),
        node(6)
    );
    assert_eq!(trace.events.last().unwrap().ending(), Some(("lost", None)));
    assert_eq!(trace.events[2].location.instance, 2);
}

#[test]
fn proposals_do_not_visit_unselected_nodes_and_their_selected_ids_are_observed() {
    let compilation = try_variant(|_, main, _| {
        set(
            main,
            &format!("{GATE}/choice_points/0"),
            "proposals",
            json!(true),
        );
    })
    .unwrap();
    let (mut session, names) = session(&compilation);
    let program = session.program().clone();
    let request: ProposalRequest = serde_json::from_value(json!({
        "choice_point": point(1),
        "options": [{"label": content("detour"), "outcome": {"kind": "branch", "target": "detour"}}],
        "nodes": [{"key": "detour", "body": {"unit": content("detour")}, "next": node(4)}]
    })).unwrap();
    let parent = session.cursor().unwrap();
    let state = session.state().unwrap().clone();
    let before = session.export().unwrap();
    session.propose(&parent, &request).unwrap();
    let commit = session
        .commits()
        .unwrap()
        .find(|(id, _)| *id == session.cursor().unwrap())
        .unwrap()
        .1;
    let input_object = session
        .history()
        .reader()
        .require(
            narrata_history::ObjectId::from_bytes(*commit.input.unwrap().as_bytes()),
            None,
        )
        .unwrap();
    let (input, _) = decode_input(&program, &parent, &state, input_object.bytes()).unwrap();
    let trace = program.trace_apply(&parent, &state, &input).unwrap();
    assert!(trace.events.is_empty());
    assert!(!trace.entered);
    assert_observation!(session, trace, Some(parent), Some(input.id()), 1);
    assert_eq!(
        trace.events,
        program.trace_apply(&parent, &state, &input).unwrap().events
    );
    let Input::Propose { options, nodes, .. } = &input else {
        panic!("expected proposal")
    };
    let proposed_node = nodes[0].0;
    let proposed_option = options[0].id;
    // The original parent identity is needed to verify the proposal's derived identities.
    assert!(
        program
            .trace_apply(&session.cursor().unwrap(), &state, &input)
            .is_err()
    );
    let restored = narrata_nodes::Session::restore(program.clone(), &before).unwrap();
    assert_eq!(restored.state().unwrap(), &state);

    let parent = session.cursor().unwrap();
    let state = session.state().unwrap().clone();
    let input = Input::Choose {
        choice_point: point(1).parse().unwrap(),
        options: vec![proposed_option],
    };
    let trace = program.trace_apply(&parent, &state, &input).unwrap();
    session
        .choose(&parent, point(1).parse().unwrap(), vec![proposed_option])
        .unwrap();
    assert_observation!(session, trace, Some(parent), Some(input.id()), 2);
    assert_eq!(
        trace.events,
        program.trace_apply(&parent, &state, &input).unwrap().events
    );
    assert_eq!(
        labels!(trace),
        [
            "selected", "visit", "body", "visit", "branch", "visit", "return", "finished"
        ]
    );
    assert_eq!(trace.events[0].selection().unwrap().1, proposed_option);
    assert_eq!(trace.events[1].location.node, proposed_node);
    assert_eq!(
        trace.events[2].segment().unwrap().unit.key.as_str(),
        "detour"
    );
    assert!(
        session
            .view(Some(&names))
            .unwrap()
            .presentation
            .iter()
            .all(|item| item.args.is_empty())
    );
}

#[test]
fn failed_choices_and_invalid_parent_states_leave_the_session_unchanged() {
    let compilation = compiled();
    let (session, _) = session(&compilation);
    let before = session.export().unwrap();
    let state = session.state().unwrap().clone();
    let input = Input::Choose {
        choice_point: point(1).parse().unwrap(),
        options: vec![option(3).parse().unwrap()],
    };
    assert!(
        session
            .program()
            .trace_apply(&session.cursor().unwrap(), &state, &input)
            .is_err()
    );
    let mut invalid = state.clone();
    invalid.frames[0].instance = 0;
    assert!(
        session
            .program()
            .trace_apply(&session.cursor().unwrap(), &invalid, &input)
            .is_err()
    );
    assert_eq!(session.state().unwrap(), &state);
    assert_eq!(session.export().unwrap(), before);
}

#[test]
fn an_automatic_detour_is_observable_even_when_it_rejoins_in_the_same_input() {
    let direct = compiled();
    let detour = try_variant(|_, main, _| {
        set(
            main,
            "/graphs/start/nodes/count/data",
            "when_true",
            json!("detour"),
        );
        set(
            main,
            "/graphs/start/nodes",
            "detour",
            json!({
                "id": node(7), "type_id": "narrata.mutate",
                "data": {"assignments": [], "next": "won"}
            }),
        );
    })
    .unwrap();
    let (mut direct, direct_names) = session(&direct);
    let (mut detour, detour_names) = session(&detour);
    for keys in [&["wave"][..], &["rope", "lamp"][..]] {
        choose(&mut direct, &direct_names, keys).unwrap();
        choose(&mut detour, &detour_names, keys).unwrap();
    }
    let input = Input::Choose {
        choice_point: point(3).parse().unwrap(),
        options: vec![option(7).parse().unwrap()],
    };
    let direct = direct
        .program()
        .trace_apply(&direct.cursor().unwrap(), direct.state().unwrap(), &input)
        .unwrap();
    let detour = detour
        .program()
        .trace_apply(&detour.cursor().unwrap(), detour.state().unwrap(), &input)
        .unwrap();
    assert_eq!(direct.state.encode(), detour.state.encode());
    assert_ne!(direct.events, detour.events);
    assert_eq!(
        direct
            .events
            .iter()
            .filter_map(|event| event.branch_target())
            .collect::<Vec<_>>(),
        vec![node(5).parse().unwrap()]
    );
    assert_eq!(
        detour
            .events
            .iter()
            .filter_map(|event| event.branch_target())
            .collect::<Vec<_>>(),
        vec![node(7).parse().unwrap()]
    );
    assert!(
        detour
            .events
            .iter()
            .any(|event| event.is_visit() && event.location.node.to_string() == node(7))
    );
}
