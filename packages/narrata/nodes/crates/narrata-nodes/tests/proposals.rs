#![allow(clippy::panic, clippy::unwrap_used)]

mod support;

use narrata_kernel::codec::object_id;
use narrata_nodes::{
    ChoicePointId, Commit, CommitId, Compilation, Input, KIND_COMMIT, KIND_INPUT, OptionId,
    ProposalRequest, Scalar, Session, SessionExport, State, decode_input, decode_state,
    plan::NameTable,
    view::{Interaction, OptionView, Role},
};
use serde_json::{Value, json};
use support::*;

const GREET: &str = "/graphs/start/nodes/gate/data/choice_points/0";
const PACK: &str = "/graphs/start/nodes/gate/data/choice_points/1";
const ROUTE: &str = "/graphs/start/nodes/gate/data/choice_points/2";
const TALK: &str = "/graphs/visit/nodes/talk/data/choice_points/0";

fn generated(key: &str) -> Value {
    json!({"provider": "gen", "key": key})
}

/// The trial work with proposals at `greet`, `route` and the called `talk`; after the call
/// returns, the story goes back to the gate.
fn open_variant(main: &mut Value, side: &mut Value) {
    for point in [GREET, ROUTE] {
        set(main, point, "proposals", json!(true));
    }
    set(side, TALK, "proposals", json!(true));
    set(
        side,
        &format!("{TALK}/options/0"),
        "label",
        content("side.talk.continue"),
    );
    *at(main, "/graphs/start/nodes/tally/data/next") = json!("gate");
}

fn opened() -> Compilation {
    try_variant(|_, main, side| open_variant(main, side)).unwrap()
}

/// At `greet`: a local option with an effect, and a branch into a proposed cellar whose
/// options lead back to the gate or down to a proposed vault that runs on to `count`.
fn cellar() -> Value {
    json!({
        "choice_point": point(1),
        "options": [
            {
                "label": generated("hum"),
                "effects": [add("shared", "coins", 2)],
                "outcome": {"kind": "local", "reply": segment("main.gate", "r-wave", "r-wave"), "rejoin": "b2"}
            },
            {"label": generated("cellar"), "outcome": {"kind": "branch", "target": "cellar"}}
        ],
        "nodes": [
            {
                "key": "cellar",
                "title": generated("cellar.title"),
                "body": {"unit": generated("cellar")},
                "args": {"coins": read("shared", "coins")},
                "choice_points": [{
                    "options": [
                        {"label": generated("cellar.up"), "outcome": {"kind": "branch", "target": node(1)}},
                        {"label": generated("cellar.down"), "outcome": {"kind": "branch", "target": "vault"}}
                    ]
                }]
            },
            {"key": "vault", "body": {"unit": generated("vault")}, "next": node(4)}
        ]
    })
}

/// At `greet`, instead of the cellar: one local option.
fn whistle() -> Value {
    json!({
        "choice_point": point(1),
        "options": [{"label": generated("whistle"), "outcome": {"kind": "local", "rejoin": "b2"}}]
    })
}

/// At the called `talk`: a branch into a proposed aside that runs on to the graph's return.
fn aside() -> Value {
    json!({
        "choice_point": point(11),
        "options": [{"label": generated("aside"), "outcome": {"kind": "branch", "target": "aside"}}],
        "nodes": [{"key": "aside", "body": {"unit": generated("aside")}, "next": node(12)}]
    })
}

fn request(value: Value) -> ProposalRequest {
    serde_json::from_value(value).unwrap()
}

fn propose(session: &mut Session, value: Value) -> narrata_nodes::Result<CommitId> {
    let cursor = session.cursor()?;
    session.propose(&cursor, &request(value))
}

fn propose_code(session: &mut Session, value: Value) -> String {
    let before = session.cursor().unwrap();
    let code = propose(session, value)
        .err()
        .map(|error| error.code)
        .unwrap_or_else(|| "accepted".into());
    assert_eq!(session.cursor().unwrap(), before);
    code
}

fn waiting(session: &Session, names: &NameTable) -> (ChoicePointId, Vec<OptionView>, bool) {
    match session.view(Some(names)).unwrap().interaction {
        Interaction::Choose {
            choice_point,
            options,
            proposals,
            ..
        } => (choice_point, options, proposals),
        Interaction::Finished { .. } => panic!("the story has ended"),
    }
}

/// Chooses options by label key, which proposed options have instead of aliases.
fn choose_labelled(session: &mut Session, names: &NameTable, labels: &[&str]) {
    let (point, options, _) = waiting(session, names);
    let chosen: Vec<OptionId> = labels
        .iter()
        .map(|label| {
            options
                .iter()
                .find(|option| option.label.as_ref().map(|value| value.key.as_str()) == Some(label))
                .unwrap_or_else(|| panic!("no option labelled {label}"))
                .id
        })
        .collect();
    let cursor = session.cursor().unwrap();
    session.choose(&cursor, point, chosen).unwrap();
}

fn labels(options: &[OptionView]) -> Vec<String> {
    options
        .iter()
        .map(|option| option.label.as_ref().unwrap().key.as_str().to_owned())
        .collect()
}

#[test]
fn proposed_options_follow_the_static_ones_and_the_interaction_stays() {
    let compilation = opened();
    let (mut session, names) = session(&compilation);
    let root = session.cursor().unwrap();
    let (point, _, accepts) = waiting(&session, &names);
    assert!(accepts);
    let proposed = propose(&mut session, cellar()).unwrap();
    assert_ne!(proposed, root);
    let view = session.view(Some(&names)).unwrap();
    // Nothing new is shown, and the reading page still holds the passage.
    assert!(view.presentation.is_empty());
    assert_eq!(session.page().unwrap()[0].role, Role::Title);
    let (after, options, _) = waiting(&session, &names);
    assert_eq!(after, point);
    assert_eq!(
        labels(&options),
        ["main.gate.wave", "main.gate.nod", "hum", "cellar"]
    );
    assert_eq!(
        options
            .iter()
            .map(|option| option.key.is_some())
            .collect::<Vec<_>>(),
        [true, true, false, false]
    );
    // A proposed local option applies its effects like a static one.
    choose_labelled(&mut session, &names, &["hum"]);
    assert_eq!(
        session.state().unwrap().shared.get("coins"),
        Some(&Scalar::Int(2))
    );
}

#[test]
fn a_proposed_passage_is_entered_and_left_for_the_graph() {
    let compilation = opened();
    let (mut session, names) = session(&compilation);
    propose(&mut session, cellar()).unwrap();
    choose_labelled(&mut session, &names, &["cellar"]);
    let view = session.view(Some(&names)).unwrap();
    assert_eq!(
        view.presentation
            .iter()
            .map(|item| (item.role, item.args.contains_key("coins")))
            .collect::<Vec<_>>(),
        [(Role::Title, true), (Role::Body, true)]
    );
    let (_, options, accepts) = waiting(&session, &names);
    assert!(!accepts);
    assert_eq!(labels(&options), ["cellar.up", "cellar.down"]);
    // Back in the gate, the frame still holds what was proposed there.
    choose_labelled(&mut session, &names, &["cellar.up"]);
    let (_, options, _) = waiting(&session, &names);
    assert_eq!(options.len(), 4);
    // The vault has no choice point and runs on to a node of the graph.
    choose_labelled(&mut session, &names, &["cellar"]);
    choose_labelled(&mut session, &names, &["cellar.down"]);
    let Interaction::Finished { outcome, .. } = session.view(Some(&names)).unwrap().interaction
    else {
        panic!("expected the end");
    };
    assert_eq!(outcome, "lost");
}

#[test]
fn the_overlay_vanishes_when_its_frame_returns() {
    let compilation = opened();
    let (mut session, names) = session(&compilation);
    propose(&mut session, cellar()).unwrap();
    choose(&mut session, &names, &["nod"]).unwrap();
    choose(&mut session, &names, &[]).unwrap();
    choose(&mut session, &names, &["left"]).unwrap();
    propose(&mut session, aside()).unwrap();
    let state = session.state().unwrap();
    assert_eq!(state.frames.len(), 2);
    assert_eq!(state.frames[1].overlay.nodes.len(), 1);
    choose_labelled(&mut session, &names, &["aside"]);
    // The aside ran on to the call's return; the gate is waiting again with its own overlay.
    let state = session.state().unwrap();
    assert_eq!(state.frames.len(), 1);
    assert_eq!(state.frames[0].overlay.nodes.len(), 2);
    let (_, options, _) = waiting(&session, &names);
    assert_eq!(options.len(), 4);
}

#[test]
fn a_different_proposal_after_rewinding_keeps_both_branches_with_different_ids() {
    let compilation = opened();
    let (mut session, names) = session(&compilation);
    let root = session.cursor().unwrap();
    let first = propose(&mut session, cellar()).unwrap();
    let (_, first_options, _) = waiting(&session, &names);
    session.checkout(&root).unwrap();
    let second = propose(&mut session, whistle()).unwrap();
    let (_, second_options, _) = waiting(&session, &names);
    assert_ne!(first, second);
    assert_ne!(first_options[2].id, second_options[2].id);
    let parents: Vec<_> = session
        .commits()
        .unwrap()
        .filter(|(id, _)| [first, second].contains(id))
        .map(|(_, commit)| commit.parent)
        .collect();
    assert_eq!(parents, [Some(root), Some(root)]);
    session.checkout(&first).unwrap();
    assert_eq!(waiting(&session, &names).1, first_options);
}

#[test]
fn the_same_request_after_the_same_commit_reuses_its_commit() {
    let compilation = opened();
    let (mut session, _) = session(&compilation);
    let root = session.cursor().unwrap();
    let first = propose(&mut session, cellar()).unwrap();
    session.checkout(&root).unwrap();
    // Request-local keys are not part of the request's identity.
    let mut renamed = cellar();
    renamed["nodes"][1]["key"] = json!("strongroom");
    renamed["nodes"][0]["choice_points"][0]["options"][1]["outcome"]["target"] =
        json!("strongroom");
    assert_eq!(propose(&mut session, renamed).unwrap(), first);
    assert_eq!(session.commits().unwrap().count(), 2);
}

#[test]
fn proposals_from_another_commit_derive_other_ids() {
    let compilation = opened();
    let (mut session, names) = session(&compilation);
    choose(&mut session, &names, &["wave"]).unwrap();
    choose(&mut session, &names, &["rope"]).unwrap();
    let request = json!({
        "choice_point": point(3),
        "options": [{"label": generated("cellar"), "outcome": {"kind": "branch", "target": node(4)}}]
    });
    propose(&mut session, request.clone()).unwrap();
    let (_, here, _) = waiting(&session, &names);
    let (mut other, _) = support::session(&compilation);
    choose(&mut other, &names, &["nod"]).unwrap();
    choose(&mut other, &names, &["rope"]).unwrap();
    propose(&mut other, request).unwrap();
    let (_, there, _) = waiting(&other, &names);
    assert_ne!(here[2].id, there[2].id);
}

#[test]
fn derived_ids_are_uuidv8() {
    let compilation = opened();
    let (mut session, _) = session(&compilation);
    propose(&mut session, cellar()).unwrap();
    let overlay = &session.state().unwrap().frames[0].overlay;
    let greet: ChoicePointId = point(1).parse().unwrap();
    let ids = overlay.options[&greet]
        .iter()
        .map(|option| *option.id.as_bytes())
        .chain(overlay.nodes.keys().map(|node| *node.as_bytes()));
    for id in ids {
        assert_eq!(id[6] >> 4, 8);
        assert_eq!(id[8] >> 6, 0b10);
    }
}

#[test]
fn a_choice_point_that_accepts_proposals_waits_even_without_available_options() {
    let compilation = try_variant(|_, main, side| {
        open_variant(main, side);
        for option in 0..2 {
            set(
                main,
                &format!("{GREET}/options/{option}"),
                "visible_if",
                literal(json!(false)),
            );
        }
    })
    .unwrap();
    let (mut session, names) = session(&compilation);
    let (_, options, accepts) = waiting(&session, &names);
    assert!(accepts && options.is_empty());
    propose(&mut session, cellar()).unwrap();
    choose_labelled(&mut session, &names, &["hum"]);
    let (_, options, _) = waiting(&session, &names);
    assert_eq!(options.len(), 3);
}

#[test]
fn a_choice_point_that_accepts_proposals_is_not_skipped_when_nothing_is_available() {
    let compilation = try_variant(|_, main, side| {
        open_variant(main, side);
        set(main, PACK, "proposals", json!(true));
        for option in 0..3 {
            set(
                main,
                &format!("{PACK}/options/{option}"),
                "visible_if",
                literal(json!(false)),
            );
        }
    })
    .unwrap();
    let (mut session, names) = session(&compilation);
    choose(&mut session, &names, &["wave"]).unwrap();
    let (point, options, accepts) = waiting(&session, &names);
    assert_eq!(point, point_id(2));
    assert!(accepts && options.is_empty());
    // Choosing nothing is still possible.
    let cursor = session.cursor().unwrap();
    session.choose(&cursor, point, Vec::new()).unwrap();
    assert_eq!(waiting(&session, &names).0, point_id(3));
}

fn point_id(n: u32) -> ChoicePointId {
    point(n).parse().unwrap()
}

#[test]
fn only_choice_points_that_accept_proposals_take_them() {
    let (mut session, _) = session(&compiled());
    assert_eq!(propose_code(&mut session, cellar()), "proposals");
    let compilation = opened();
    let (mut session, names) = support::session(&compilation);
    let mut elsewhere = whistle();
    elsewhere["choice_point"] = json!(point(2));
    assert_eq!(propose_code(&mut session, elsewhere), "action");
    choose(&mut session, &names, &["wave"]).unwrap();
    let mut pack = whistle();
    pack["choice_point"] = json!(point(2));
    assert_eq!(propose_code(&mut session, pack), "proposals");
    let mut stale = whistle();
    stale["choice_point"] = json!(point(1));
    assert_eq!(propose_code(&mut session, stale), "action");
}

#[test]
fn requests_are_bounded() {
    let compilation = opened();
    let (mut session, _) = session(&compilation);
    let option = json!({"label": generated("x"), "outcome": {"kind": "local", "rejoin": "b2"}});
    let many = json!({"choice_point": point(1), "options": vec![option; 17]});
    assert_eq!(propose_code(&mut session, many), "limit");
    let passage = |index: usize| {
        json!({
            "key": format!("n{index}"),
            "choice_points": [{"options": [{"outcome": {"kind": "branch", "target": node(4)}}]}]
        })
    };
    let crowded =
        json!({"choice_point": point(1), "nodes": (0..17).map(passage).collect::<Vec<_>>()});
    assert_eq!(propose_code(&mut session, crowded), "limit");
    let empty = json!({"choice_point": point(1)});
    assert_eq!(propose_code(&mut session, empty), "proposal");
    // A frame holds 256 proposed nodes.
    for round in 0..16 {
        let nodes: Vec<_> = (0..16).map(|index| passage(round * 16 + index)).collect();
        propose(
            &mut session,
            json!({"choice_point": point(1), "nodes": nodes}),
        )
        .unwrap();
    }
    assert_eq!(session.state().unwrap().frames[0].overlay.nodes.len(), 256);
    let one = json!({"choice_point": point(1), "nodes": [passage(999)]});
    assert_eq!(propose_code(&mut session, one), "limit");
}

#[test]
fn request_keys_and_targets_are_checked() {
    let compilation = opened();
    let (mut session, _) = session(&compilation);
    let mut unknown = cellar();
    unknown["options"][1]["outcome"]["target"] = json!("attic");
    assert_eq!(propose_code(&mut session, unknown), "reference");
    let mut twice = cellar();
    twice["nodes"][1]["key"] = json!("cellar");
    assert_eq!(propose_code(&mut session, twice), "duplicate");
    let mut malformed = cellar();
    malformed["nodes"][1]["next"] = json!("node:zz");
    assert_eq!(propose_code(&mut session, malformed), "identifier");
    let mut bad_key = cellar();
    bad_key["nodes"][1]["key"] = json!("a key");
    assert_eq!(propose_code(&mut session, bad_key), "identifier");
}

#[test]
fn proposals_pass_the_compile_time_rules() {
    let compilation = opened();
    let (mut session, names) = session(&compilation);
    // A branch target outside the graph and its overlay.
    let mut foreign = cellar();
    foreign["nodes"][0]["choice_points"][0]["options"][0]["outcome"]["target"] = json!(node(11));
    assert_eq!(propose_code(&mut session, foreign), "reference");
    // Effects and conditions are typed against the graph's declarations.
    let mut typed = whistle();
    typed["options"][0]["effects"] =
        json!([{"target": {"scope": "shared", "name": "lamp"}, "value": literal(json!(3))}]);
    assert_eq!(propose_code(&mut session, typed), "type");
    // A reply must stay in the passage body's unit.
    let mut reply = whistle();
    reply["options"][0]["outcome"]["reply"] = segment("elsewhere", "r1", "r1");
    assert_eq!(propose_code(&mut session, reply), "local");
    // With two or more options, every option needs a label.
    let mut unlabelled = whistle();
    unlabelled["options"][0]
        .as_object_mut()
        .unwrap()
        .remove("label");
    assert_eq!(propose_code(&mut session, unlabelled), "label");
    // A proposed passage that can run to its end needs a next node.
    let mut endless = cellar();
    endless["nodes"][1].as_object_mut().unwrap().remove("next");
    assert_eq!(propose_code(&mut session, endless), "contract");
    // The last choice point only branches and the gate has no next node, so a local option
    // there would run off the passage's end.
    choose(&mut session, &names, &["wave"]).unwrap();
    choose(&mut session, &names, &[]).unwrap();
    let mut local = whistle();
    local["choice_point"] = json!(point(3));
    local["options"][0]["outcome"] = json!({"kind": "local"});
    assert_eq!(propose_code(&mut session, local), "contract");
}

#[test]
fn a_called_graph_checks_the_whole_choice_point_after_the_append() {
    // Without a label on `continue`, adding a second option breaks the label rule.
    let compilation = try_variant(|_, main, side| {
        open_variant(main, side);
        at(side, &format!("{TALK}/options/0"))
            .as_object_mut()
            .unwrap()
            .remove("label");
    })
    .unwrap();
    let (mut session, names) = session(&compilation);
    choose(&mut session, &names, &["nod"]).unwrap();
    choose(&mut session, &names, &[]).unwrap();
    choose(&mut session, &names, &["left"]).unwrap();
    assert_eq!(propose_code(&mut session, aside()), "label");
}

/// The envelope of the input that `commit` records, from the session export.
fn recorded_input(session: &Session, commit: &CommitId) -> Vec<u8> {
    let export: SessionExport = legacy_export(session);
    let objects: Vec<Vec<u8>> = export
        .objects
        .iter()
        .map(|object| hex::decode(object).unwrap())
        .collect();
    let kind = |bytes: &[u8]| u16::from_be_bytes([bytes[10], bytes[11]]);
    let input = objects
        .iter()
        .filter(|bytes| kind(bytes) == KIND_COMMIT)
        .map(|bytes| Commit::decode(bytes).unwrap())
        .find(|candidate| candidate.id() == *commit)
        .and_then(|candidate| candidate.input)
        .unwrap();
    objects
        .into_iter()
        .find(|bytes| {
            kind(bytes) == KIND_INPUT && object_id(KIND_INPUT, 1, &bytes[56..]) == *input.as_bytes()
        })
        .unwrap()
}

fn proposed_at_root() -> (Session, CommitId, State, Vec<u8>) {
    let compilation = opened();
    let (mut session, _) = session(&compilation);
    let root = session.cursor().unwrap();
    let parent = session.state().unwrap().clone();
    let proposed = propose(&mut session, cellar()).unwrap();
    let envelope = recorded_input(&session, &proposed);
    (session, root, parent, envelope)
}

fn input_code(session: &Session, parent: (&CommitId, &State), input: &Input) -> String {
    decode_input(session.program(), parent.0, parent.1, &input.envelope())
        .err()
        .map(|error| error.code)
        .unwrap_or_else(|| "accepted".into())
}

#[test]
fn a_recorded_proposal_round_trips_byte_for_byte() {
    let (session, root, parent, envelope) = proposed_at_root();
    let (input, _) = decode_input(session.program(), &root, &parent, &envelope).unwrap();
    assert_eq!(input.envelope(), envelope);
    let Input::Propose { options, nodes, .. } = &input else {
        panic!("expected a proposal");
    };
    // New nodes keep the request's order: the cellar, then the vault.
    assert_eq!((options.len(), nodes.len()), (2, 2));
    assert!(nodes[0].1.title.is_some() && nodes[1].1.title.is_none());
    let state = session.state().unwrap();
    let (decoded, id) = decode_state(session.program(), &state.envelope()).unwrap();
    assert_eq!(&decoded, state);
    assert_eq!(id, state.id());
    assert_eq!(decoded.encode(), state.encode());
}

#[test]
fn decode_input_rederives_every_id() {
    let (session, root, parent, envelope) = proposed_at_root();
    let (input, _) = decode_input(session.program(), &root, &parent, &envelope).unwrap();
    let parent = (&root, &parent);
    let mut renamed = input.clone();
    if let Input::Propose { options, .. } = &mut renamed {
        options[0].id = OptionId::from_u128(0x77);
    }
    assert_eq!(input_code(&session, parent, &renamed), "derived_id");
    // The IDs are bound to the structure: changing a recorded field without new IDs fails.
    let mut retitled = input.clone();
    if let Input::Propose { nodes, .. } = &mut retitled {
        nodes[0].1.title = None;
    }
    assert_eq!(input_code(&session, parent, &retitled), "derived_id");
    // So are they to the parent commit.
    let elsewhere = session.cursor().unwrap();
    assert_eq!(
        input_code(&session, (&elsewhere, parent.1), &input),
        "derived_id"
    );
}

#[test]
fn decode_input_rejects_proposals_beyond_the_limits() {
    let (session, root, parent, envelope) = proposed_at_root();
    let (input, _) = decode_input(session.program(), &root, &parent, &envelope).unwrap();
    let Input::Propose {
        choice_point,
        options,
        nodes,
    } = input
    else {
        panic!("expected a proposal");
    };
    let many = Input::Propose {
        choice_point,
        options: vec![options[0].clone(); 17],
        nodes: nodes.clone(),
    };
    assert_eq!(input_code(&session, (&root, &parent), &many), "decode");
    let crowded = Input::Propose {
        choice_point,
        options,
        nodes: vec![nodes[0].clone(); 17],
    };
    assert_eq!(input_code(&session, (&root, &parent), &crowded), "decode");
}

#[test]
fn decode_state_rejects_dangling_and_foreign_overlay_references() {
    let (session, _, _, _) = proposed_at_root();
    let state = session.state().unwrap();
    let code = |change: &dyn Fn(&mut State)| {
        let mut tampered = state.clone();
        change(&mut tampered);
        decode_state(session.program(), &tampered.envelope())
            .err()
            .map(|error| error.code)
            .unwrap_or_else(|| "accepted".into())
    };
    let vault = |state: &mut State| {
        let overlay = &mut state.frames[0].overlay;
        let id = overlay
            .nodes
            .iter()
            .find(|(_, passage)| passage.title.is_none())
            .map(|(id, _)| *id)
            .unwrap();
        overlay.nodes.get_mut(&id).unwrap().next = Some(node(99).parse().unwrap());
    };
    assert_eq!(code(&vault), "invalid_state");
    // A node of the called graph is not a node of this frame.
    let foreign = |state: &mut State| {
        let overlay = &mut state.frames[0].overlay;
        for passage in overlay.nodes.values_mut() {
            passage.next = passage.next.map(|_| node(11).parse().unwrap());
        }
    };
    assert_eq!(code(&foreign), "invalid_state");
    // Options appended to a choice point that does not accept proposals.
    let closed = |state: &mut State| {
        let overlay = &mut state.frames[0].overlay;
        let options = overlay.options.values().next().unwrap().clone();
        overlay.options.insert(point(2).parse().unwrap(), options);
    };
    assert_eq!(code(&closed), "invalid_state");
    // A proposed node that reuses an authored ID.
    let shadow = |state: &mut State| {
        let overlay = &mut state.frames[0].overlay;
        let (_, passage) = overlay.nodes.pop_first().unwrap();
        overlay.nodes.insert(node(6).parse().unwrap(), passage);
    };
    assert_eq!(code(&shadow), "invalid_state");
    // Proposed options must be new IDs.
    let reused = |state: &mut State| {
        let overlay = &mut state.frames[0].overlay;
        let options = overlay.options.values_mut().next().unwrap();
        options[0].id = option(3).parse().unwrap();
    };
    assert_eq!(code(&reused), "invalid_state");
}

#[test]
fn an_overlay_beyond_256_nodes_does_not_decode() {
    let (session, _, _, _) = proposed_at_root();
    let mut state = session.state().unwrap().clone();
    let overlay = &mut state.frames[0].overlay;
    let (_, vault) = overlay
        .nodes
        .iter()
        .find(|(_, passage)| passage.title.is_none())
        .map(|(id, passage)| (*id, passage.clone()))
        .unwrap();
    for index in 0..256 {
        overlay.nodes.insert(
            format!("node:ff{index:030x}").parse().unwrap(),
            vault.clone(),
        );
    }
    let error = decode_state(session.program(), &state.envelope())
        .err()
        .unwrap();
    assert_eq!(error.code, "decode");
}

/// Plays the scenario the frozen corpus records: a proposal at the gate, into the proposed
/// cellar and back, a proposed local option, a proposal inside the called graph and its
/// return, and a second proposal from the root.
fn play(compilation: &Compilation, requests: &dyn Fn(&str) -> Value) -> Session {
    let (mut session, names) = session(compilation);
    let root = session.cursor().unwrap();
    propose(&mut session, requests("cellar")).unwrap();
    choose_labelled(&mut session, &names, &["cellar"]);
    choose_labelled(&mut session, &names, &["cellar.up"]);
    choose_labelled(&mut session, &names, &["hum"]);
    choose(&mut session, &names, &[]).unwrap();
    choose(&mut session, &names, &["left"]).unwrap();
    propose(&mut session, requests("aside")).unwrap();
    choose_labelled(&mut session, &names, &["aside"]);
    let back = session.cursor().unwrap();
    session.checkout(&root).unwrap();
    propose(&mut session, requests("whistle")).unwrap();
    choose_labelled(&mut session, &names, &["whistle"]);
    session.checkout(&back).unwrap();
    session
}

fn played() -> (Compilation, Session) {
    let compilation = opened();
    let session = play(&compilation, &|name| match name {
        "cellar" => cellar(),
        "aside" => aside(),
        _ => whistle(),
    });
    (compilation, session)
}

#[test]
fn an_export_with_proposals_restores_without_replay_and_verifies_by_replay() {
    let (compilation, session) = played();
    let text = session.export().unwrap();
    let (program, names) = open(&compilation.pack);
    let restored = Session::restore(program, &text).unwrap();
    assert_eq!(restored.state().unwrap(), session.state().unwrap());
    assert_eq!(
        restored.view(Some(&names)).unwrap().interaction,
        session.view(Some(&names)).unwrap().interaction
    );
    assert_eq!(restored.page().ok(), session.page().ok());
    assert_eq!(restored.export().ok(), Some(text));
    for (id, _) in restored.commits().unwrap() {
        restored.verify_path(&id).unwrap();
    }
}

fn corpus() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../../fixtures/compat/nodes-r2-proposals")
}

fn frozen(name: &str) -> Vec<u8> {
    std::fs::read(corpus().join(name)).unwrap()
}

fn frozen_text(name: &str) -> String {
    String::from_utf8(frozen(name)).unwrap()
}

fn frozen_compilation() -> Compilation {
    let manifest: narrata_nodes::ProjectManifest =
        narrata_nodes::parse_json(&frozen_text("project.json")).unwrap();
    let packages = manifest
        .packages
        .iter()
        .map(|(alias, path)| {
            (
                alias.clone(),
                narrata_nodes::parse_json(&frozen_text(path)).unwrap(),
            )
        })
        .collect();
    narrata_nodes::compile(
        &narrata_nodes::ProjectSource { manifest, packages },
        &narrata_nodes::NodeRegistry::gamebook(),
    )
    .unwrap()
}

fn frozen_manifest() -> Value {
    serde_json::from_str(&frozen_text("manifest.json")).unwrap()
}

#[test]
fn every_frozen_file_matches_its_recorded_digest() {
    let manifest = frozen_manifest();
    let artifacts = manifest["artifacts"].as_object().unwrap();
    assert_eq!(artifacts.len(), 8);
    for (name, entry) in artifacts {
        assert_eq!(
            hex::encode(narrata_kernel::codec::sha256(&frozen(name))),
            entry["sha256"].as_str().unwrap(),
            "{name}"
        );
    }
}

#[test]
fn the_frozen_source_still_compiles_to_the_frozen_pack() {
    let compilation = frozen_compilation();
    assert_eq!(compilation.pack, frozen("story.narpack"));
    assert_eq!(
        compilation.program.artifact_id().to_string(),
        frozen_manifest()["artifact_id"].as_str().unwrap()
    );
}

#[test]
fn the_frozen_export_restores_without_replay_and_replays_from_the_root() {
    let (program, names) = open(&frozen("story.narpack"));
    let text = frozen_text("session.export.json");
    let session = Session::restore(program, &text).unwrap();
    assert_eq!(session.commits().unwrap().count(), 11);
    for (id, _) in session.commits().unwrap() {
        session.verify_path(&id).unwrap();
    }
    assert_ne!(
        session.cursor().unwrap().to_string(),
        serde_json::from_str::<Value>(&text).unwrap()["cursor"]
    );
    let state = session.state().unwrap();
    assert_eq!(state.frames.len(), 1);
    assert_eq!(state.frames[0].overlay.nodes.len(), 2);
    assert_eq!(waiting(&session, &names).1.len(), 4);
}

#[test]
fn the_frozen_requests_still_derive_the_frozen_session() {
    let compilation = frozen_compilation();
    let session = play(&compilation, &|name| {
        serde_json::from_str(&frozen_text(&format!("{name}.request.json"))).unwrap()
    });
    let repeated = play(&compilation, &|name| {
        serde_json::from_str(&frozen_text(&format!("{name}.request.json"))).unwrap()
    });
    assert_eq!(session.export().unwrap(), repeated.export().unwrap());
    session.verify_path(&session.cursor().unwrap()).unwrap();
    // The frozen requests are the ones the other tests propose.
    assert_eq!(
        serde_json::from_str::<Value>(&frozen_text("cellar.request.json")).unwrap(),
        cellar()
    );
}
