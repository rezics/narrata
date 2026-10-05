#![allow(clippy::panic, clippy::unwrap_used)]

mod support;

use std::sync::Arc;

use narrata_kernel::content::{ContentRef, Segment};
use narrata_nodes::{
    Commit, Program, Scalar, Session, SessionExport, State, ViewScalar,
    view::{Interaction, PresentationItem, Presented, Role},
};
use support::*;

fn local(key: &str) -> ContentRef {
    ContentRef::new("local", key).unwrap()
}

fn segment_of(unit: &str, first: Option<&str>, last: Option<&str>) -> Presented {
    let anchor = |text: &str| text.parse().unwrap();
    Presented::Segment(Segment {
        unit: local(unit),
        first: first.map(anchor),
        last: last.map(anchor),
    })
}

fn shown(items: &[PresentationItem]) -> Vec<(Role, Presented)> {
    items
        .iter()
        .map(|item| (item.role, item.content.clone()))
        .collect()
}

fn coins(item: &PresentationItem) -> Option<&ViewScalar> {
    item.args.get("coins")
}

#[test]
fn entering_a_passage_presents_its_title_and_the_body_up_to_the_first_choice_point() {
    let compilation = compiled();
    let (session, names) = session(&compilation);
    let view = session.view(Some(&names)).unwrap();
    assert_eq!(
        shown(&view.presentation),
        vec![
            (Role::Title, Presented::Ref(local("main.gate.title"))),
            (Role::Body, segment_of("main.gate", Some("b1"), Some("b1"))),
        ]
    );
    let Interaction::Choose {
        key,
        min,
        max,
        options,
        ..
    } = view.interaction
    else {
        panic!("expected a choice");
    };
    assert_eq!((key.as_deref(), min, max), (Some("greet"), 1, 1));
    assert_eq!(options.len(), 2);
    assert_eq!(view.presentation[0].occurrence, 0);
    assert_eq!(view.presentation[1].occurrence, 1);
}

#[test]
fn a_local_choice_shows_its_reply_then_rejoins_the_same_passage() {
    let compilation = compiled();
    let (mut session, names) = session(&compilation);
    choose(&mut session, &names, &["wave"]).unwrap();
    let view = session.view(Some(&names)).unwrap();
    assert_eq!(
        shown(&view.presentation),
        vec![
            (
                Role::Reply,
                segment_of("main.gate", Some("r-wave"), Some("r-wave"))
            ),
            (Role::Body, segment_of("main.gate", Some("b2"), Some("b2"))),
        ]
    );
    assert_eq!(
        coins(&view.presentation[0]),
        Some(&ViewScalar::Int("1".into()))
    );
    let Interaction::Choose {
        key,
        min,
        max,
        options,
        ..
    } = view.interaction
    else {
        panic!("expected a choice");
    };
    assert_eq!((key.as_deref(), min, max), (Some("pack"), 0, 2));
    let map = options
        .iter()
        .find(|option| option.key.as_deref() == Some("map"))
        .unwrap();
    assert!(!map.enabled);
    assert_eq!(map.reason, Some(local("main.gate.map.reason")));
}

#[test]
fn a_multiple_selection_presents_replies_in_option_order_after_all_effects() {
    let compilation = compiled();
    let (mut session, names) = session(&compilation);
    choose(&mut session, &names, &["nod"]).unwrap();
    choose(&mut session, &names, &["rope", "lamp"]).unwrap();
    let view = session.view(Some(&names)).unwrap();
    assert_eq!(
        shown(&view.presentation),
        vec![
            (
                Role::Reply,
                segment_of("main.gate", Some("r-rope"), Some("r-rope"))
            ),
            (
                Role::Reply,
                segment_of("main.gate", Some("r-lamp"), Some("r-lamp"))
            ),
            (Role::Body, segment_of("main.gate", Some("b3"), Some("b4"))),
        ]
    );
    // Both replies see the state after every chosen option's effects.
    assert!(
        view.presentation
            .iter()
            .all(|item| coins(item) == Some(&ViewScalar::Int("1".into())))
    );
    let state = session.state().unwrap();
    assert_eq!(state.shared.get("lamp"), Some(&Scalar::Bool(true)));
}

#[test]
fn options_out_of_choice_point_order_are_rejected_and_leave_the_session_unchanged() {
    let compilation = compiled();
    let (mut session, names) = session(&compilation);
    choose(&mut session, &names, &["wave"]).unwrap();
    let before = session.cursor().unwrap();
    let error = choose(&mut session, &names, &["lamp", "rope"])
        .err()
        .unwrap();
    assert_eq!(error.code, "action");
    let error = choose(&mut session, &names, &["rope", "rope"])
        .err()
        .unwrap();
    assert_eq!(error.code, "action");
    assert_eq!(session.cursor().unwrap(), before);
}

#[test]
fn a_disabled_option_cannot_be_chosen() {
    let compilation = compiled();
    let (mut session, names) = session(&compilation);
    choose(&mut session, &names, &["wave"]).unwrap();
    let error = choose(&mut session, &names, &["map"]).err().unwrap();
    assert_eq!(error.code, "unavailable");
}

#[test]
fn choosing_nothing_takes_the_shared_rejoin() {
    let compilation = compiled();
    let (mut session, names) = session(&compilation);
    choose(&mut session, &names, &["wave"]).unwrap();
    choose(&mut session, &names, &[]).unwrap();
    let view = session.view(Some(&names)).unwrap();
    assert_eq!(
        shown(&view.presentation),
        vec![(Role::Body, segment_of("main.gate", Some("b3"), Some("b4")))]
    );
}

#[test]
fn a_choice_point_with_min_zero_and_no_available_option_is_skipped() {
    let compilation = try_variant(|_, main, _| {
        for option in 0..2 {
            set(
                main,
                &format!("{GATE}/choice_points/1/options/{option}"),
                "visible_if",
                literal(serde_json::json!(false)),
            );
        }
    })
    .unwrap();
    let (mut session, names) = session(&compilation);
    choose(&mut session, &names, &["wave"]).unwrap();
    let view = session.view(Some(&names)).unwrap();
    assert_eq!(
        shown(&view.presentation),
        vec![
            (
                Role::Reply,
                segment_of("main.gate", Some("r-wave"), Some("r-wave"))
            ),
            (Role::Body, segment_of("main.gate", Some("b2"), Some("b2"))),
            (Role::Body, segment_of("main.gate", Some("b3"), Some("b4"))),
        ]
    );
    let Interaction::Choose { key, .. } = view.interaction else {
        panic!("expected a choice");
    };
    assert_eq!(key.as_deref(), Some("route"));
}

#[test]
fn too_few_available_options_report_no_actions() {
    let compilation = try_variant(|_, main, _| {
        *at(main, &format!("{GATE}/choice_points/1/min")) = serde_json::json!(1);
        for option in 0..3 {
            set(
                main,
                &format!("{GATE}/choice_points/1/options/{option}"),
                "visible_if",
                literal(serde_json::json!(false)),
            );
        }
    })
    .unwrap();
    let (mut session, names) = session(&compilation);
    let error = choose(&mut session, &names, &["wave"]).err().unwrap();
    assert_eq!(error.code, "no_actions");
}

#[test]
fn the_page_collects_everything_since_the_passage_was_entered() {
    let compilation = compiled();
    let (mut session, names) = session(&compilation);
    let root = session.cursor().unwrap();
    choose(&mut session, &names, &["wave"]).unwrap();
    let waved = session.cursor().unwrap();
    choose(&mut session, &names, &["rope"]).unwrap();
    let packed = session.cursor().unwrap();
    let page = session.page().unwrap();
    assert_eq!(page.len(), 6);
    assert_eq!(page[0].role, Role::Title);
    // Each item keeps its own presentation key.
    assert_eq!(
        page.iter()
            .map(|item| (item.commit, item.occurrence))
            .collect::<Vec<_>>(),
        vec![
            (root, 0),
            (root, 1),
            (waved, 0),
            (waved, 1),
            (packed, 0),
            (packed, 1)
        ]
    );
    assert_eq!(session.presentation(&waved).unwrap(), page[2..4].to_vec());
}

#[test]
fn a_page_starts_with_the_step_that_entered_the_passage() {
    let compilation = compiled();
    let (mut session, names) = session(&compilation);
    choose(&mut session, &names, &["nod"]).unwrap();
    choose(&mut session, &names, &[]).unwrap();
    choose(&mut session, &names, &["left"]).unwrap();
    let page = session.page().unwrap();
    // Leaving the gate by a branch presents nothing more of it; the called passage follows.
    assert_eq!(
        shown(&page),
        vec![
            (Role::Title, Presented::Ref(local("side.talk.title"))),
            (Role::Body, segment_of("side.talk", None, None)),
        ]
    );
}

#[test]
fn a_branch_ends_the_story_with_the_product_ending() {
    let compilation = compiled();
    let (mut session, names) = session(&compilation);
    choose(&mut session, &names, &["wave"]).unwrap();
    choose(&mut session, &names, &["rope"]).unwrap();
    choose(&mut session, &names, &["right"]).unwrap();
    let view = session.view(Some(&names)).unwrap();
    let Interaction::Finished {
        outcome,
        title,
        body,
    } = view.interaction
    else {
        panic!("expected the end");
    };
    assert_eq!(outcome, "done");
    assert_eq!(title, Some(local("ending.done")));
    assert!(body.is_some());
    assert!(view.presentation.is_empty());
    assert!(
        view.history
            .last()
            .is_some_and(|entry| entry.finished && entry.current)
    );
}

#[test]
fn a_call_passes_a_ref_argument_and_returns_to_its_continuation() {
    let compilation = compiled();
    let (mut session, names) = session(&compilation);
    choose(&mut session, &names, &["nod"]).unwrap();
    choose(&mut session, &names, &[]).unwrap();
    choose(&mut session, &names, &["left"]).unwrap();
    let view = session.view(Some(&names)).unwrap();
    assert_eq!(view.frames.len(), 2);
    assert_eq!(
        view.presentation[1].args.get("visitor"),
        Some(&ViewScalar::Ref(local("main.visitor")))
    );
    assert_eq!(
        shown(&view.presentation)[1],
        (Role::Body, segment_of("side.talk", None, None))
    );
    choose(&mut session, &names, &["continue"]).unwrap();
    let state = session.state().unwrap();
    assert!(state.frames.is_empty());
    assert_eq!(
        state.finished.as_ref().map(|f| f.outcome.as_str()),
        Some("lost")
    );
    assert_eq!(state.next_instance, 3);
}

#[test]
fn a_stale_expected_commit_is_rejected() {
    let compilation = compiled();
    let (mut session, names) = session(&compilation);
    let root = session.cursor().unwrap();
    let view = session.view(Some(&names)).unwrap();
    choose(&mut session, &names, &["wave"]).unwrap();
    let Interaction::Choose {
        choice_point,
        options,
        ..
    } = view.interaction
    else {
        panic!("expected a choice");
    };
    let error = session
        .choose(&root, choice_point, vec![options[0].id])
        .err()
        .unwrap();
    assert_eq!(error.code, "stale_input");
}

#[test]
fn the_same_input_from_the_same_parent_reuses_the_commit() {
    let compilation = compiled();
    let (mut session, names) = session(&compilation);
    let root = session.cursor().unwrap();
    choose(&mut session, &names, &["wave"]).unwrap();
    let first = session.cursor().unwrap();
    session.checkout(&root).unwrap();
    choose(&mut session, &names, &["wave"]).unwrap();
    assert_eq!(session.cursor().unwrap(), first);
    assert_eq!(session.commits().count(), 2);
}

fn played() -> (
    narrata_nodes::Compilation,
    Session,
    narrata_nodes::plan::NameTable,
) {
    let compilation = compiled();
    let (mut session, names) = session(&compilation);
    choose(&mut session, &names, &["wave"]).unwrap();
    choose(&mut session, &names, &["rope", "lamp"]).unwrap();
    let rope = session.cursor().unwrap();
    choose(&mut session, &names, &["right"]).unwrap();
    session.checkout(&rope).unwrap();
    // The last commit waits inside the called graph.
    choose(&mut session, &names, &["left"]).unwrap();
    session.checkout(&rope).unwrap();
    (compilation, session, names)
}

#[test]
fn restoring_an_export_reproduces_the_view_without_replay() {
    let (compilation, session, names) = played();
    let text = session.export().unwrap();
    let (program, _) = open(&compilation.pack);
    let restored = Session::restore(program, &text).unwrap();
    assert_eq!(
        restored.view(Some(&names)).unwrap(),
        session.view(Some(&names)).unwrap()
    );
    assert_eq!(restored.page().ok(), session.page().ok());
    assert_eq!(restored.export().ok(), Some(text));
}

#[test]
fn every_commit_follows_its_input_and_state_in_the_export() {
    let (_, session, _) = played();
    let export: SessionExport = serde_json::from_str(&session.export().unwrap()).unwrap();
    assert_eq!(export.format_version, 2);
    let mut seen = std::collections::BTreeSet::new();
    for object in &export.objects {
        let bytes = hex::decode(object).unwrap();
        if u16::from_be_bytes([bytes[10], bytes[11]]) == narrata_nodes::KIND_COMMIT {
            let commit = Commit::decode(&bytes).unwrap();
            assert!(seen.contains(&commit.state));
            if let Some(input) = commit.input {
                assert!(seen.contains(&input));
            }
        } else {
            let payload = &bytes[56..];
            let kind = u16::from_be_bytes([bytes[10], bytes[11]]);
            seen.insert(narrata_nodes::ObjectId::from_bytes(
                narrata_kernel::codec::object_id(kind, 1, payload),
            ));
        }
    }
}

#[test]
fn restore_rejects_an_export_of_another_artifact() {
    let (_, session, _) = played();
    let text = session.export().unwrap();
    let other = try_variant(|_, main, _| {
        *at(main, "/graphs/start/locals/visits") = serde_json::json!(7);
    })
    .unwrap();
    let (program, _) = open(&other.pack);
    let error = Session::restore(program, &text).err().unwrap();
    assert_eq!(error.code, "incompatible_save");
}

/// Replaces the state of the last commit (a leaf) with `change(state)`, re-seals that commit
/// and moves the cursor to it.
fn tampered(session: &Session, change: impl FnOnce(&mut State)) -> String {
    let mut export: SessionExport = serde_json::from_str(&session.export().unwrap()).unwrap();
    let position = export.objects.len() - 1;
    let mut commit =
        Commit::decode(&hex::decode(&export.objects[position]).unwrap_or_default()).unwrap();
    let (index, mut state) = export
        .objects
        .iter()
        .enumerate()
        .find_map(|(index, object)| {
            let bytes = hex::decode(object).ok()?;
            narrata_nodes::decode_state(session.program(), &bytes)
                .ok()
                .filter(|(_, id)| *id == commit.state)
                .map(|(state, _)| (index, state))
        })
        .unwrap();
    change(&mut state);
    commit.state = state.id();
    // Only the leaf references its state, so it is replaced in place.
    export.objects[index] = hex::encode(state.envelope());
    export.objects[position] = hex::encode(commit.envelope());
    export.cursor = commit.id();
    serde_json::to_string(&export).unwrap()
}

fn restore_error(session: &Session, change: impl FnOnce(&mut State)) -> String {
    let text = tampered(session, change);
    let program = session.program().clone();
    Session::restore(program, &text)
        .err()
        .map(|e| e.code)
        .unwrap_or_else(|| "accepted".into())
}

#[test]
fn restore_rejects_states_with_unknown_ids() {
    let (_, session, _) = played();
    let unknown_node = restore_error(&session, |state| {
        state.frames[1].node = node(99).parse().unwrap();
    });
    assert_eq!(unknown_node, "invalid_state");
    let unknown_point = restore_error(&session, |state| {
        state.frames[1].at = Some(point(99).parse().unwrap());
    });
    assert_eq!(unknown_point, "invalid_state");
}

#[test]
fn restore_rejects_states_outside_the_declarations() {
    let (_, session, _) = played();
    let wrong_type = restore_error(&session, |state| {
        state.shared.insert("coins".into(), Scalar::Bool(true));
    });
    assert_eq!(wrong_type, "invalid_state");
    let undeclared = restore_error(&session, |state| {
        state.shared.insert("gold".into(), Scalar::Int(1));
    });
    assert_eq!(undeclared, "invalid_state");
    let instance = restore_error(&session, |state| {
        state.next_instance = 1;
    });
    assert_eq!(instance, "invalid_state");
}

#[test]
fn restore_rejects_tampered_object_bytes() {
    let (_, session, _) = played();
    let mut export: SessionExport = serde_json::from_str(&session.export().unwrap()).unwrap();
    let mut bytes = hex::decode(&export.objects[0]).unwrap_or_default();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    export.objects[0] = hex::encode(bytes);
    let text = serde_json::to_string(&export).unwrap();
    let error = Session::restore(session.program().clone(), &text)
        .err()
        .unwrap();
    assert!(error.path.starts_with("objects[0]."), "{error}");
}

#[test]
fn verify_path_detects_a_well_formed_but_unreachable_state() {
    let (_, session, names) = played();
    let text = tampered(&session, |state| {
        state.shared.insert("coins".into(), Scalar::Int(99));
    });
    let restored = Session::restore(session.program().clone(), &text).unwrap();
    let cursor = restored.cursor().unwrap();
    let error = restored.verify_path(&cursor).err().unwrap();
    assert_eq!(error.code, "unreachable_state");
    assert_eq!(
        restored.view(Some(&names)).err().map(|e| e.code).as_deref(),
        Some("unreachable_state")
    );
    let root = restored.commits().next().map(|(id, _)| id).unwrap();
    restored.verify_path(&root).unwrap();
}

#[test]
fn chunks_load_only_when_a_frame_enters_their_graph() {
    let compilation = compiled();
    let (program, names) = open(&compilation.pack);
    assert_eq!(program.loaded_chunks(), 0);
    let mut session = Session::new(program.clone(), execution()).unwrap();
    assert_eq!(program.loaded_chunks(), 1);
    choose(&mut session, &names, &["nod"]).unwrap();
    choose(&mut session, &names, &[]).unwrap();
    choose(&mut session, &names, &["left"]).unwrap();
    assert_eq!(program.loaded_chunks(), 2);
}

#[test]
fn a_program_reads_only_the_manifest_and_requested_chunks_from_its_source() {
    let compilation = compiled();
    let pack = narrata_nodes::Pack::decode(&compilation.pack).unwrap();
    let mut source = narrata_nodes::MemorySource::default();
    for chunk in &pack.chunks {
        source.insert(chunk.clone()).unwrap();
    }
    let program = Program::open(&pack.manifest, &pack.tombstones, Box::new(source)).unwrap();
    assert_eq!(program.artifact_id(), compilation.program.artifact_id());
    let session = Session::new(Arc::new(program), execution()).unwrap();
    assert!(session.view(None).is_ok());
}
