#![allow(clippy::unwrap_used)]
mod support;

use narrata_history::{BundleLimits, ShallowBundle};
use narrata_nodes::{Program, Session};
use std::{collections::BTreeSet, sync::Arc};

fn restore(session: &Session, ancestors: u32) -> Session {
    let bytes = session.export_shallow(ancestors).unwrap();
    assert_eq!(bytes, session.export_shallow(ancestors).unwrap());
    let restored = Session::restore(session.program().clone(), &hex::encode(&bytes)).unwrap();
    assert_eq!(restored.cursor().unwrap(), session.cursor().unwrap());
    assert_eq!(restored.page().unwrap(), session.page().unwrap());
    assert_eq!(restored.state().unwrap(), session.state().unwrap());
    assert_eq!(
        restored.interaction(None).unwrap(),
        session.interaction(None).unwrap()
    );
    restored.view(None).unwrap();
    restored
}

#[test]
fn zero_ancestors_retains_the_exact_page_local_arguments_multiselect_call_and_ending() {
    let compilation = support::compiled();
    let (mut session, names) = support::session(&compilation);
    restore(&session, 0);
    for options in [
        &["wave"][..],
        &["rope", "lamp"][..],
        &["left"][..],
        &["continue"][..],
    ] {
        support::choose(&mut session, &names, options).unwrap();
        let mut restored = restore(&session, 0);
        // The same next choice must produce exactly the same commit identity and state.
        if let narrata_nodes::view::Interaction::Choose {
            choice_point,
            options,
            ..
        } = session.interaction(None).unwrap()
        {
            let selected: Vec<_> = options
                .iter()
                .filter(|o| o.enabled)
                .take(1)
                .map(|o| o.id)
                .collect();
            let expected = session.cursor().unwrap();
            let actual = restored
                .choose(&expected, choice_point, selected.clone())
                .unwrap();
            let mut reference =
                Session::restore(session.program().clone(), &session.export().unwrap()).unwrap();
            assert_eq!(
                actual,
                reference.choose(&expected, choice_point, selected).unwrap()
            );
            assert_eq!(restored.page().unwrap(), reference.page().unwrap());
        }
    }
}

#[test]
fn a_truly_shallow_page_can_continue_but_full_export_and_replay_refuse_the_boundary() {
    let (mut session, names) = support::session(&support::compiled());
    support::choose(&mut session, &names, &["wave"]).unwrap();
    support::choose(&mut session, &names, &["rope"]).unwrap();
    support::choose(&mut session, &names, &["left"]).unwrap();
    let bytes = session.export_shallow(0).unwrap();
    let bundle = ShallowBundle::from_bytes(&bytes, BundleLimits::default(), |_| true).unwrap();
    assert!(!bundle.manifest.boundary_parents.is_empty());
    assert_eq!(
        bundle
            .objects
            .iter()
            .filter(|o| o.kind() == narrata_nodes::KIND_COMMIT)
            .count(),
        2
    );
    let mut restored = restore(&session, 0);
    assert!(restored.export().is_err());
    assert_eq!(
        restored
            .verify_path(&restored.cursor().unwrap())
            .unwrap_err()
            .code,
        "history_truncated"
    );
    support::choose(&mut restored, &names, &["continue"]).unwrap();
    support::choose(&mut session, &names, &["continue"]).unwrap();
    assert_eq!(restored.cursor().unwrap(), session.cursor().unwrap());
    assert_eq!(restored.page().unwrap(), session.page().unwrap());
}

#[test]
fn insufficient_page_context_is_rejected_before_an_existing_durable_cursor_moves() {
    let (mut source, names) = support::session(&support::compiled());
    support::choose(&mut source, &names, &["wave"]).unwrap();
    support::choose(&mut source, &names, &["rope"]).unwrap();
    let invalid = source
        .history()
        .export_shallow(
            narrata_history::ObjectId::from_bytes(*source.cursor().unwrap().as_bytes()),
            0,
            &BTreeSet::new(),
        )
        .unwrap()
        .to_bytes()
        .unwrap();
    let text = hex::encode(invalid);
    assert_eq!(
        Session::restore(source.program().clone(), &text)
            .unwrap()
            .page()
            .unwrap_err()
            .code,
        "history_truncated"
    );
    let (mut target, _) = support::session(&support::compiled());
    let cursor = target.cursor().unwrap();
    let backend = target.history().backend().share();
    let execution = target.execution();
    assert_eq!(target.import(&text).unwrap_err().code, "history_truncated");
    assert_eq!(target.cursor().unwrap(), cursor);
    let reopened = Session::resume(source.program().clone(), execution, backend).unwrap();
    assert_eq!(reopened.cursor().unwrap(), cursor);
}

#[test]
fn interim_proposals_retain_exact_shallow_context() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../../fixtures/compat/nodes-r2-proposals");
    let (program, _) =
        Program::from_pack(&std::fs::read(root.join("story.narpack")).unwrap()).unwrap();
    let session = Session::restore(
        Arc::new(program),
        &std::fs::read_to_string(root.join("session.export.json")).unwrap(),
    )
    .unwrap();
    let bytes = session.export_shallow(0).unwrap();
    let restored = Session::restore(session.program().clone(), &hex::encode(bytes)).unwrap();
    assert_eq!(restored.page().unwrap(), session.page().unwrap());
    assert_eq!(
        restored.interaction(None).unwrap(),
        session.interaction(None).unwrap()
    );
}
