#![cfg(feature = "trace")]
#![allow(clippy::unwrap_used, clippy::panic)]
#[path = "support/evolution_backend.rs"]
mod backend;
#[path = "support/evolution_corpus.rs"]
mod corpus;
mod support;

use narrata_nodes::{
    Compilation, Program, ProposalRequest, Scalar, Session, history::evolution::*,
};
use serde_json::{Value, json};
use support::*;

const POINT: &str = "/graphs/start/nodes/gate/data/choice_points/0";
const MULTI: &str = "/graphs/start/nodes/gate/data/choice_points/1";
const ROUTE: &str = "/graphs/start/nodes/gate/data/choice_points/2";

fn changed() -> Compilation {
    try_variant(|_, main, _| {
        set(
            main,
            "/graphs/start/nodes/tally/data",
            "assignments",
            json!([add("local", "visits", 2)]),
        );
    })
    .unwrap()
}
fn setup(
    old: &Compilation,
    new: &Compilation,
    recoveries: Vec<Recovery>,
) -> (EvolutionRegistry, MigrationId) {
    let mut registry = EvolutionRegistry::new();
    registry.register_program(open(&old.pack).0).unwrap();
    registry.register_program(open(&new.pack).0).unwrap();
    let id = registry
        .register_descriptor(
            EvolutionDescriptorV1::new(
                old.program.artifact_id(),
                new.program.artifact_id(),
                recoveries,
            )
            .unwrap(),
        )
        .unwrap();
    (registry, id)
}
fn plan(assessment: AssessmentV1) -> PlanV1 {
    match assessment {
        AssessmentV1::Automatic { plan } | AssessmentV1::NeedsConfirmation { plan, .. } => *plan,
        other => panic!("no plan: {other:?}"),
    }
}
fn assess(
    session: &Session,
    registry: &EvolutionRegistry,
    target: &Program,
    id: MigrationId,
) -> AssessmentV1 {
    session.assess_evolution(registry, target.artifact_id(), &[id], PathLimits::default())
}
fn destination() -> RecoveryTarget {
    RecoveryTarget {
        node: node(20).parse().unwrap(),
        choice_point: point(20).parse().unwrap(),
    }
}
fn add_recovery_passage(main: &mut Value) {
    set(
        main,
        "/graphs/start/nodes",
        "safe",
        json!({"id":node(20),"type_id":"narrata.passage","data":{"body":{"unit":content("safe")},"choice_points":[{"id":point(20),"key":"resume","options":[{"id":option(20),"key":"finish","outcome":{"kind":"branch","target":"lost"}}]}]}}),
    );
}
fn deleted_gate() -> Compilation {
    try_variant(|_, main, _| {
        add_recovery_passage(main);
        set(main, "/graphs/start", "entry", json!("safe"));
        at(main, "/graphs/start/nodes")
            .as_object_mut()
            .unwrap()
            .remove("gate");
        main["tombstones"] = json!([
            node(1),
            point(1),
            point(2),
            point(3),
            option(1),
            option(2),
            option(3),
            option(4),
            option(5),
            option(6),
            option(7)
        ]);
    })
    .unwrap()
}
fn mapping(deleted: DeletedId) -> Recovery {
    Recovery {
        deleted,
        target: destination(),
    }
}

#[test]
fn unseen_rule_change_is_read_only_automatic_and_cursor_only_application() {
    let old = compiled();
    let new = changed();
    let (registry, id) = setup(&old, &new, vec![]);
    let (mut s, names) = session(&old);
    choose(&mut s, &names, &["wave"]).unwrap();
    let source = s.cursor().unwrap();
    let state = s.state().unwrap().clone();
    let slot = narrata_history::RefName::new("before").unwrap();
    s.save(slot.clone()).unwrap();
    let before = s.export().unwrap();
    let assessment = assess(&s, &registry, &new.program, id);
    assert!(matches!(assessment, AssessmentV1::Automatic { .. }));
    let p = plan(assessment);
    assert_eq!(p.report.path, PathComparisonV1::Equivalent);
    assert!(p.report.losses.is_empty());
    assert_eq!(s.export().unwrap(), before);
    assert_eq!(s.cursor().unwrap(), source);
    assert!(matches!(
        s.apply_evolution(&registry, &p, PathLimits::default()),
        ApplyV1::Migrated { .. }
    ));
    assert_eq!(s.cursor().unwrap(), p.migration_commit);
    assert_eq!(s.state().unwrap(), &state);
    assert_eq!(s.program().artifact_id(), new.program.artifact_id());
    let migration = s
        .commits()
        .unwrap()
        .find(|(id, _)| *id == p.migration_commit)
        .unwrap()
        .1;
    let input = s
        .history()
        .reader()
        .require(
            narrata_history::ObjectId::from_bytes(*migration.input.unwrap().as_bytes()),
            None,
        )
        .unwrap();
    assert_eq!(
        input.kind(),
        narrata_history::migration::MIGRATION_INPUT_KIND
    );
    assert_eq!(s.view(None).unwrap().history.len(), 3);
    assert!(!s.page().unwrap().is_empty());
    let shown = s.page().unwrap();
    let body = shown
        .iter()
        .find_map(|item| match &item.content {
            narrata_nodes::view::Presented::Segment(segment) => Some(segment),
            _ => None,
        })
        .unwrap();
    assert_eq!(body.last.as_ref().unwrap().as_str(), "b2");
    let migrated = s.export().unwrap();
    let restored = Session::restore_with_registry(&registry, &migrated).unwrap();
    assert_eq!(restored.cursor().unwrap(), p.migration_commit);
    // Exact restore never asks for evolution metadata.
    let mut programs = EvolutionRegistry::new();
    programs.register_program(open(&new.pack).0).unwrap();
    assert_eq!(
        Session::restore_with_registry(&programs, &migrated)
            .unwrap()
            .cursor()
            .unwrap(),
        p.migration_commit
    );
    s.load_save(slot).unwrap();
    assert_eq!(s.cursor().unwrap(), source);
    assert_eq!(s.program().artifact_id(), old.program.artifact_id());
    s.checkout(&p.migration_commit).unwrap();
    s.choose(&p.migration_commit, point(2).parse().unwrap(), vec![])
        .unwrap();
    assert_eq!(s.program().artifact_id(), new.program.artifact_id());
}

#[test]
fn changed_control_target_is_first_divergence_even_after_same_input_rejoins() {
    let old = compiled();
    let new = try_variant(|_, main, _| { set(main, &format!("{ROUTE}/options/1/outcome"), "target", json!("detour")); set(main, "/graphs/start/nodes", "detour", json!({"id":node(30),"type_id":"narrata.mutate","data":{"assignments":[],"next":"count"}})); }).unwrap();
    let (registry, id) = setup(&old, &new, vec![]);
    let (mut s, names) = session(&old);
    for choices in [&["wave"][..], &["rope"], &["right"]] {
        choose(&mut s, &names, choices).unwrap();
    }
    let expected = s.cursor().unwrap();
    let assessment = assess(&s, &registry, &new.program, id);
    assert!(matches!(
        assessment,
        AssessmentV1::NeedsConfirmation {
            reason: ConfirmationReason::Divergence,
            ..
        }
    ));
    assert!(
        matches!(plan(assessment).report.path, PathComparisonV1::Diverged { source_commit, reason: DivergenceReason::ControlTrace, .. } if source_commit == expected)
    );
    assert!(s.state().unwrap().finished.is_some());
}

#[test]
fn multi_selection_uses_target_order_and_preserves_recorded_input() {
    let old = compiled();
    let new = try_variant(|_, main, _| {
        at(main, &format!("{MULTI}/options"))
            .as_array_mut()
            .unwrap()
            .swap(0, 1);
    })
    .unwrap();
    let (registry, id) = setup(&old, &new, vec![]);
    let (mut s, names) = session(&old);
    choose(&mut s, &names, &["wave"]).unwrap();
    choose(&mut s, &names, &["rope", "lamp"]).unwrap();
    let before = s.export().unwrap();
    let p = plan(assess(&s, &registry, &new.program, id));
    assert!(matches!(
        p.report.path,
        PathComparisonV1::Diverged {
            reason: DivergenceReason::ControlTrace,
            ..
        }
    ));
    assert_eq!(s.export().unwrap(), before);
    assert!(matches!(
        s.apply_evolution(&registry, &p, PathLimits::default()),
        ApplyV1::Migrated { .. }
    ));
    assert_eq!(s.state().unwrap().shared["coins"], Scalar::Int(2));
}

#[test]
fn recovery_requires_all_positions_agree_and_reports_only_keys_and_ids() {
    let old = compiled();
    let new = deleted_gate();
    let recoveries = vec![
        mapping(DeletedId::Node(node(1).parse().unwrap())),
        mapping(DeletedId::ChoicePoint(point(1).parse().unwrap())),
    ];
    let (registry, id) = setup(&old, &new, recoveries);
    let (mut s, _) = session(&old);
    let p = plan(assess(&s, &registry, &new.program, id));
    assert_eq!(p.report.losses.len(), 1);
    assert_eq!(p.report.losses[0].tombstones.len(), 2);
    assert_eq!(
        p.report.losses[0].discarded_frames[0].local_keys,
        ["visits"]
    );
    assert_eq!(p.report.losses[0].target, destination());
    assert!(matches!(
        s.apply_evolution(&registry, &p, PathLimits::default()),
        ApplyV1::Migrated { .. }
    ));
    assert_eq!(s.state().unwrap().frames[0].instance, 2);
    assert_eq!(s.state().unwrap().next_instance, 3);
    assert_eq!(s.state().unwrap().frames[0].node, destination().node);
    let (registry, id) = setup(&old, &new, vec![]);
    let (s, _) = session(&old);
    assert!(
        matches!(assess(&s, &registry, &new.program, id), AssessmentV1::OldVersionOnly { reason, .. } if reason == "unmapped_continuation")
    );
}

#[test]
fn deleting_a_lower_call_recovers_the_entire_stack() {
    let old = compiled();
    let new = try_variant(|_, main, _| {
        add_recovery_passage(main);
        at(main, "/graphs/start/nodes")
            .as_object_mut()
            .unwrap()
            .remove("visit");
        set(
            main,
            &format!("{ROUTE}/options/0/outcome"),
            "target",
            json!("safe"),
        );
        main["tombstones"] = json!([node(2)]);
    })
    .unwrap();
    let (registry, id) = setup(
        &old,
        &new,
        vec![mapping(DeletedId::Node(node(2).parse().unwrap()))],
    );
    let (mut s, names) = session(&old);
    for choices in [&["wave"][..], &["rope"], &["left"]] {
        choose(&mut s, &names, choices).unwrap();
    }
    assert_eq!(s.state().unwrap().frames.len(), 2);
    let p = plan(assess(&s, &registry, &new.program, id));
    assert_eq!(p.report.losses[0].discarded_frames.len(), 2);
    assert_eq!(
        p.report.losses[0].discarded_frames[1].parameter_keys,
        ["visitor"]
    );
    assert!(matches!(
        s.apply_evolution(&registry, &p, PathLimits::default()),
        ApplyV1::Migrated { .. }
    ));
    assert_eq!(s.state().unwrap().frames.len(), 1);
    assert_eq!(s.state().unwrap().shared["coins"], Scalar::Int(2));
}

#[test]
fn confirmation_changes_and_cursor_aba_are_rejected_without_partial_write() {
    let old = compiled();
    let new = changed();
    let (registry, id) = setup(&old, &new, vec![]);
    let (mut s, names) = session(&old);
    let root = s.cursor().unwrap();
    let p = plan(assess(&s, &registry, &new.program, id));
    let mut forged = p.clone();
    forged.report.path = PathComparisonV1::Unverified {
        reason: UnverifiedReason::Limit,
        baseline: None,
        checked_through: None,
    };
    assert_eq!(
        s.apply_evolution(&registry, &forged, PathLimits::default()),
        ApplyV1::PlanChanged
    );
    choose(&mut s, &names, &["wave"]).unwrap();
    s.checkout(&root).unwrap();
    let before = s.export().unwrap();
    assert!(matches!(
        s.apply_evolution(&registry, &p, PathLimits::default()),
        ApplyV1::Conflict { .. }
    ));
    assert_eq!(s.export().unwrap(), before);
}

#[test]
fn shallow_history_and_limits_cannot_prove_equivalence() {
    let old = compiled();
    let new = changed();
    let (registry, id) = setup(&old, &new, vec![]);
    let (mut s, names) = session(&old);
    choose(&mut s, &names, &["wave"]).unwrap();
    let bundle = s
        .history()
        .export_shallow(
            narrata_history::ObjectId::from_bytes(*s.cursor().unwrap().as_bytes()),
            0,
            &Default::default(),
        )
        .unwrap()
        .to_bytes()
        .unwrap();
    let shallow = Session::restore_with_registry(&registry, &hex::encode(bundle)).unwrap();
    let assessment = assess(&shallow, &registry, &new.program, id);
    assert!(
        matches!(
            assessment,
            AssessmentV1::NeedsConfirmation {
                reason: ConfirmationReason::Unverified,
                ..
            }
        ),
        "{assessment:?}"
    );
    let limited = s.assess_evolution(
        &registry,
        new.program.artifact_id(),
        &[id],
        PathLimits {
            transitions: 0,
            ..PathLimits::default()
        },
    );
    assert!(matches!(
        limited,
        AssessmentV1::NeedsConfirmation {
            reason: ConfirmationReason::Unverified,
            ..
        }
    ));
}

#[test]
fn previous_migrations_are_audited_and_new_segment_uses_their_baseline() {
    let old = compiled();
    let new = changed();
    let next = try_variant(|_, main, _| {
        set(
            main,
            "/graphs/start/nodes/tally/data",
            "assignments",
            json!([add("local", "visits", 3)]),
        );
    })
    .unwrap();
    let (mut registry, first) = setup(&old, &new, vec![]);
    registry.register_program(open(&next.pack).0).unwrap();
    let second = registry
        .register_descriptor(
            EvolutionDescriptorV1::new(
                new.program.artifact_id(),
                next.program.artifact_id(),
                vec![],
            )
            .unwrap(),
        )
        .unwrap();
    let (mut s, _) = session(&old);
    let p = plan(assess(&s, &registry, &new.program, first));
    assert!(matches!(
        s.apply_evolution(&registry, &p, PathLimits::default()),
        ApplyV1::Migrated { .. }
    ));
    let p = plan(assess(&s, &registry, &next.program, second));
    assert_eq!(p.report.path, PathComparisonV1::Equivalent);
    assert!(matches!(
        s.apply_evolution(&registry, &p, PathLimits::default()),
        ApplyV1::Migrated { .. }
    ));
    assert_eq!(s.view(None).unwrap().history.len(), 3);
}

#[test]
fn proposal_ids_are_retained_and_new_static_collisions_reject_the_candidate() {
    let old = try_variant(|_, main, _| {
        set(main, POINT, "proposals", json!(true));
    })
    .unwrap();
    let new = try_variant(|_, main, _| {
        set(main, POINT, "proposals", json!(true));
        set(
            main,
            "/graphs/start/nodes/tally/data",
            "assignments",
            json!([add("local", "visits", 2)]),
        );
    })
    .unwrap();
    let (registry, id) = setup(&old, &new, vec![]);
    let (mut s, _) = session(&old);
    let parent = s.cursor().unwrap();
    let request: ProposalRequest = serde_json::from_value(json!({"choice_point":point(1),"options":[{"label":content("proposed"),"outcome":{"kind":"local","rejoin":"b2"}}]})).unwrap();
    s.propose(&parent, &request).unwrap();
    let state = s.state().unwrap().clone();
    let p = plan(assess(&s, &registry, &new.program, id));
    assert_eq!(p.report.path, PathComparisonV1::Equivalent);
    assert!(matches!(
        s.apply_evolution(&registry, &p, PathLimits::default()),
        ApplyV1::Migrated { .. }
    ));
    assert_eq!(s.state().unwrap(), &state);
    let dynamic_id = state.frames[0].overlay.options[&point(1).parse().unwrap()][0].id;
    let collision = try_variant(|_, main, _| {
        set(main, POINT, "proposals", json!(true));
        let mut extra = at(main, &format!("{POINT}/options/0")).clone();
        extra["id"] = json!(dynamic_id);
        extra["key"] = json!("collision");
        at(main, &format!("{POINT}/options"))
            .as_array_mut()
            .unwrap()
            .push(extra);
    })
    .unwrap();
    let (registry, id) = setup(&old, &collision, vec![]);
    let (mut original, _) = session(&old);
    original.propose(&parent, &request).unwrap();
    assert!(matches!(
        assess(&original, &registry, &collision.program, id),
        AssessmentV1::OldVersionOnly { .. }
    ));
    assert!(matches!(
        registry
            .compare_path(
                original.history(),
                original.cursor().unwrap(),
                collision.program.artifact_id(),
                PathLimits::default()
            )
            .unwrap(),
        PathComparisonV1::Diverged { .. }
    ));
}

#[test]
fn dry_run_makes_zero_backend_writes_and_unknown_apply_is_read_back() {
    use std::sync::atomic::Ordering;
    let old = compiled();
    let new = changed();
    let (registry, id) = setup(&old, &new, vec![]);
    let backend = backend::ObservedBackend::new(session(&old).0.into_backend());
    let mut s = Session::open_with_registry(&registry, execution(), backend.clone()).unwrap();
    let before = backend.writes.load(Ordering::SeqCst);
    let p = plan(s.assess_evolution(
        &registry,
        new.program.artifact_id(),
        &[id],
        PathLimits::default(),
    ));
    assert_eq!(backend.writes.load(Ordering::SeqCst), before);
    *backend.failure.lock().unwrap() = Some(backend::Failure::BeforeBusy);
    assert!(matches!(
        s.apply_evolution(&registry, &p, PathLimits::default()),
        ApplyV1::Failed { .. }
    ));
    assert_eq!(s.cursor().unwrap(), p.source_commit);
    *backend.failure.lock().unwrap() = Some(backend::Failure::AfterIo);
    assert!(
        matches!(s.apply_evolution(&registry, &p, PathLimits::default()), ApplyV1::Migrated { migration_commit, .. } if migration_commit == p.migration_commit)
    );
    assert_eq!(backend.writes.load(Ordering::SeqCst), before + 2);
    assert_eq!(
        Session::open_with_registry(&registry, execution(), backend)
            .unwrap()
            .cursor()
            .unwrap(),
        p.migration_commit
    );
}

#[test]
fn another_session_choice_invalidates_confirmation_without_overwriting_it() {
    let old = compiled();
    let new = changed();
    let (registry, id) = setup(&old, &new, vec![]);
    let backend = backend::ObservedBackend::new(session(&old).0.into_backend());
    let mut first = Session::open_with_registry(&registry, execution(), backend.clone()).unwrap();
    let p = plan(first.assess_evolution(
        &registry,
        new.program.artifact_id(),
        &[id],
        PathLimits::default(),
    ));
    let mut second = Session::open_with_registry(&registry, execution(), backend.clone()).unwrap();
    let selected = second
        .choose(
            &second.cursor().unwrap(),
            point(1).parse().unwrap(),
            vec![option(1).parse().unwrap()],
        )
        .unwrap();
    assert!(
        matches!(first.apply_evolution(&registry, &p, PathLimits::default()), ApplyV1::Conflict { actual_cursor: Some(CursorV1 { commit, .. }) } if commit == selected)
    );
    assert_eq!(
        Session::open_with_registry(&registry, execution(), backend)
            .unwrap()
            .cursor()
            .unwrap(),
        selected
    );
}

#[test]
fn malformed_source_is_invalid_save_and_missing_target_is_retryable() {
    use narrata_history::{
        DomainKinds, History, Object, RefKey, RefMutation, RefName, Transaction,
    };
    use narrata_nodes::history::NodeDomain;
    let old = compiled();
    let new = changed();
    let (registry, id) = setup(&old, &new, vec![]);
    let mut state = session(&old).0.state().unwrap().clone();
    state.shared.insert("coins".into(), Scalar::Int(12345));
    let state = Object::new(
        narrata_nodes::KIND_STATE,
        narrata_nodes::SCHEMA,
        &state.encode(),
    );
    let commit = narrata_history::Commit::root(
        narrata_history::ArtifactId::from_bytes(*old.program.artifact_id().as_bytes()),
        state.id(),
    )
    .to_object(narrata_nodes::KIND_COMMIT);
    let mut history = History::in_memory(DomainKinds::<NodeDomain>::new());
    let key = RefKey::active(RefName::hex(execution().as_bytes())).unwrap();
    history
        .write(
            &Transaction {
                objects: vec![state, commit.clone()],
                refs: vec![RefMutation {
                    key,
                    expected: None,
                    next: Some(commit.id()),
                }],
                ..Transaction::default()
            },
            |_| Ok(Vec::new()),
        )
        .unwrap();
    let s = Session::open_with_registry(&registry, execution(), history.into_backend()).unwrap();
    assert!(matches!(
        assess(&s, &registry, &new.program, id),
        AssessmentV1::InvalidSave { .. }
    ));
    let (s, _) = session(&old);
    let missing = s.assess_evolution(
        &registry,
        narrata_nodes::ArtifactId::from_bytes([99; 32]),
        &[id],
        PathLimits::default(),
    );
    assert!(matches!(
        missing,
        AssessmentV1::NeedsObjects {
            retryable: true,
            ..
        }
    ));
}

#[test]
fn unknown_positions_and_variable_schema_changes_do_not_act_as_recovery() {
    let old = compiled();
    let new = try_variant(|_, main, _| {
        add_recovery_passage(main);
        set(main, "/graphs/start", "entry", json!("safe"));
        at(main, "/graphs/start/nodes")
            .as_object_mut()
            .unwrap()
            .remove("gate");
    })
    .unwrap();
    let descriptor =
        EvolutionDescriptorV1::new(old.program.artifact_id(), new.program.artifact_id(), vec![])
            .unwrap();
    assert_eq!(
        descriptor
            .validate(&old.program, &new.program)
            .unwrap_err()
            .code,
        "unknown_id"
    );
    let added = try_variant(|manifest, _, _| {
        manifest["product"]["shared"]["added"] = json!(0);
    })
    .unwrap();
    let (registry, id) = setup(&old, &added, vec![]);
    let (s, _) = session(&old);
    assert!(
        matches!(assess(&s, &registry, &added.program, id), AssessmentV1::OldVersionOnly { reason, .. } if reason == "invalid_state")
    );
}

#[test]
fn explicit_multihop_keeps_every_migration_edge_and_missing_prior_descriptor_is_unverified() {
    let old = compiled();
    let middle = changed();
    let target = try_variant(|_, main, _| {
        set(
            main,
            "/graphs/start/nodes/tally/data",
            "assignments",
            json!([add("local", "visits", 3)]),
        );
    })
    .unwrap();
    let (mut registry, first) = setup(&old, &middle, vec![]);
    registry.register_program(open(&target.pack).0).unwrap();
    let second = registry
        .register_descriptor(
            EvolutionDescriptorV1::new(
                middle.program.artifact_id(),
                target.program.artifact_id(),
                vec![],
            )
            .unwrap(),
        )
        .unwrap();
    let (mut s, _) = session(&old);
    let p = plan(s.assess_evolution(
        &registry,
        target.program.artifact_id(),
        &[first, second],
        PathLimits::default(),
    ));
    assert_eq!(p.migration_ids, [first, second]);
    assert!(matches!(
        s.apply_evolution(&registry, &p, PathLimits::default()),
        ApplyV1::Migrated { .. }
    ));
    assert_eq!(s.commits().unwrap().count(), 3);
    s.checkout(&p.source_commit).unwrap();
    let first_plan = plan(assess(&s, &registry, &middle.program, first));
    assert!(matches!(
        s.apply_evolution(&registry, &first_plan, PathLimits::default()),
        ApplyV1::Migrated { .. }
    ));
    let mut incomplete = EvolutionRegistry::new();
    for program in [
        open(&old.pack).0,
        open(&middle.pack).0,
        open(&target.pack).0,
    ] {
        incomplete.register_program(program).unwrap();
    }
    incomplete
        .register_descriptor(registry.descriptor(second).unwrap().clone())
        .unwrap();
    assert!(matches!(
        assess(&s, &incomplete, &target.program, second),
        AssessmentV1::NeedsConfirmation {
            reason: ConfirmationReason::Unverified,
            ..
        }
    ));
}

#[test]
fn replacement_declarations_invalidate_confirmation_and_instance_overflow_is_rejected() {
    let old = compiled();
    let target = deleted_gate();
    let recoveries = vec![
        mapping(DeletedId::Node(node(1).parse().unwrap())),
        mapping(DeletedId::ChoicePoint(point(1).parse().unwrap())),
    ];
    let (registry, first) = setup(&old, &target, recoveries.clone());
    let (mut s, _) = session(&old);
    let p = plan(assess(&s, &registry, &target.program, first));
    let mut replacements = recoveries;
    replacements.push(mapping(DeletedId::ChoicePoint(point(2).parse().unwrap())));
    let (replacement_registry, second) = setup(&old, &target, replacements);
    assert_ne!(first, second);
    let before = s.export().unwrap();
    assert_eq!(
        s.apply_evolution(&replacement_registry, &p, PathLimits::default()),
        ApplyV1::PlanChanged
    );
    assert_eq!(s.export().unwrap(), before);
    let mut state = s.state().unwrap().clone();
    state.next_instance = u32::MAX;
    assert_eq!(
        registry
            .descriptor(first)
            .unwrap()
            .migrate_state(&old.program, &target.program, &state)
            .unwrap_err()
            .code,
        "instance_limit"
    );
}

#[test]
fn a_new_interaction_without_actions_is_not_delivered() {
    let old = compiled();
    let target = try_variant(|_, main, _| {
        for index in 0..2 {
            set(
                main,
                &format!("{POINT}/options/{index}"),
                "enabled_if",
                literal(json!(false)),
            );
        }
    })
    .unwrap();
    let (registry, id) = setup(&old, &target, vec![]);
    let (s, _) = session(&old);
    assert!(
        matches!(assess(&s, &registry, &target.program, id), AssessmentV1::OldVersionOnly { reason, .. } if reason == "unavailable_interaction")
    );
}

#[test]
fn interim_proposal_parents_remain_valid_during_shadow_replay() {
    use std::{collections::BTreeMap, path::Path, sync::Arc};
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../../fixtures/compat/nodes-r2-proposals");
    let source = Arc::new(
        Program::from_pack(&std::fs::read(root.join("story.narpack")).unwrap())
            .unwrap()
            .0,
    );
    let mut project = narrata_nodes::ProjectSource {
        manifest: serde_json::from_str(
            &std::fs::read_to_string(root.join("project.json")).unwrap(),
        )
        .unwrap(),
        packages: BTreeMap::from([
            (
                "main".into(),
                serde_json::from_str(&std::fs::read_to_string(root.join("main.json")).unwrap())
                    .unwrap(),
            ),
            (
                "side".into(),
                serde_json::from_str(&std::fs::read_to_string(root.join("side.json")).unwrap())
                    .unwrap(),
            ),
        ]),
    };
    project.packages.get_mut("main").unwrap().version = "1.0.1".into();
    let target =
        narrata_nodes::compile(&project, &narrata_nodes::NodeRegistry::gamebook()).unwrap();
    let mut s = Session::restore(
        source.clone(),
        &std::fs::read_to_string(root.join("session.export.json")).unwrap(),
    )
    .unwrap();
    let mut registry = EvolutionRegistry::new();
    registry.register_program(source.clone()).unwrap();
    registry.register_program(open(&target.pack).0).unwrap();
    let id = registry
        .register_descriptor(
            EvolutionDescriptorV1::new(source.artifact_id(), target.program.artifact_id(), vec![])
                .unwrap(),
        )
        .unwrap();
    let p = plan(assess(&s, &registry, &target.program, id));
    assert_eq!(p.report.path, PathComparisonV1::Equivalent);
    let state = s.state().unwrap().clone();
    assert!(matches!(
        s.apply_evolution(&registry, &p, PathLimits::default()),
        ApplyV1::Migrated { .. }
    ));
    assert_eq!(s.state().unwrap(), &state);
}
