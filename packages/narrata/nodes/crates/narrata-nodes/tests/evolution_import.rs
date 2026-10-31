#![allow(clippy::unwrap_used)]

mod support;

use narrata_nodes::{Session, history::evolution::EvolutionRegistry};
use support::*;

#[test]
fn checkpoint_import_requests_the_exact_missing_artifact_and_accepts_registered_history() {
    let source = compiled();
    let (source_session, _) = session(&source);
    let save = source_session.export().unwrap();
    let target = try_variant(|_, main, _| {
        *at(main, "/graphs/start/locals/visits") = serde_json::json!(7);
    })
    .unwrap();
    let (mut target_session, _) = session(&target);
    let before = target_session.export().unwrap();

    let error = target_session.import(&save).unwrap_err();
    assert_eq!(error.code, "needs_artifact");
    assert_eq!(error.path, source.program.artifact_id().to_string());
    assert_eq!(target_session.export().unwrap(), before);

    target_session
        .register_history_program(open(&source.pack).0)
        .unwrap();
    target_session.import(&save).unwrap();
    assert_eq!(
        target_session.program().artifact_id(),
        source.program.artifact_id()
    );
    assert_eq!(
        target_session.state().unwrap(),
        source_session.state().unwrap()
    );
    assert_eq!(
        target_session.cursor().unwrap(),
        source_session.cursor().unwrap()
    );
}

#[test]
fn registry_restore_still_reports_the_exact_missing_artifact() {
    let source = compiled();
    let (source_session, _) = session(&source);
    let save = source_session.export().unwrap();
    let mut registry = EvolutionRegistry::new();

    let error = Session::restore_with_registry(&registry, &save)
        .err()
        .unwrap();
    assert_eq!(error.code, "needs_artifact");
    assert_eq!(error.path, source.program.artifact_id().to_string());

    registry.register_program(open(&source.pack).0).unwrap();
    let restored = Session::restore_with_registry(&registry, &save).unwrap();
    assert_eq!(restored.cursor().unwrap(), source_session.cursor().unwrap());
}
