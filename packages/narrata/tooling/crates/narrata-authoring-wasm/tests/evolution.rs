#![allow(clippy::unwrap_used, clippy::panic)]

use narrata_authoring::{
    CheckedPublication, PlannedEvolution, plan_evolution, publication::publication_from_pack,
};
use narrata_authoring_wasm::evolution::{ComparisonBudgetV1, Engine};
use narrata_nodes::{ExecutionId, Session, history::evolution::*};
use narrata_storage_host::{
    CacheBackend, StoreId,
    testing::{self, MemoryHost},
};
use std::path::Path;

fn corpus(case: &str, file: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../../../fixtures/compat/nodes-evolution-v1")
            .join(case)
            .join(file),
    )
    .unwrap()
}
fn publication(case: &str, file: &str) -> CheckedPublication {
    publication_from_pack(&corpus(case, file)).unwrap()
}
fn attach(previous: &CheckedPublication, candidate: CheckedPublication) -> CheckedPublication {
    let PlannedEvolution::Descriptor(d) = plan_evolution(previous, &candidate, None).unwrap()
    else {
        panic!("descriptor")
    };
    candidate.with_evolution(previous, &d).unwrap()
}
fn plan(result: AssessmentV1) -> PlanV1 {
    match result {
        AssessmentV1::Automatic { plan } | AssessmentV1::NeedsConfirmation { plan, .. } => *plan,
        other => panic!("{other:?}"),
    }
}
fn budget() -> ComparisonBudgetV1 {
    ComparisonBudgetV1::default()
}
fn engine(case: &str) -> (Engine, narrata_nodes::ArtifactId, Vec<MigrationId>) {
    let previous = publication(case, "source.narpack");
    let candidate = publication(case, "target.narpack");
    let declarations = String::from_utf8(corpus(case, "declarations.json")).unwrap();
    let PlannedEvolution::Descriptor(d) =
        plan_evolution(&previous, &candidate, Some(&declarations)).unwrap()
    else {
        panic!("descriptor")
    };
    let candidate = candidate.with_evolution(&previous, &d).unwrap();
    let mut engine = Engine::default();
    let id = engine.register_evolution(&previous, &candidate).unwrap();
    if case == "republished-after-migration" {
        let ancestor = publication(case, "ancestor.narpack");
        let intermediate = attach(&ancestor, publication(case, "source.narpack"));
        engine.register_evolution(&ancestor, &intermediate).unwrap();
    }
    engine
        .restore(&hex::encode(corpus(case, "source.checkpoint")))
        .unwrap();
    (engine, candidate.program().artifact_id(), vec![id])
}

#[test]
fn public_driver_reuses_frozen_assessments_reports_and_migration_bytes() {
    for case in [
        "unseen-rule",
        "target-rejoins",
        "multi-reordered",
        "deleted-ending",
        "deleted-lower-call",
        "conflicting-declarations",
        "no-declarations",
        "variable-type",
        "shallow-history",
        "proposal-retained",
        "proposal-collision",
        "content-reference",
        "republished-after-migration",
    ] {
        let (mut engine, target, ids) = engine(case);
        let before = if case == "shallow-history" {
            None
        } else {
            Some(engine.export().unwrap())
        };
        let assessment = engine.assess(target, &ids, budget()).unwrap();
        let actual = serde_json::to_value(&assessment).unwrap();
        let mut expected: serde_json::Value =
            serde_json::from_slice(&corpus(case, "assessment.json")).unwrap();
        // Restoring a checkpoint creates a new cursor revision; commit/report identities are
        // identical. Do not use the original process's revision as a restore promise.
        if let Some(p) = expected.get_mut("plan") {
            p["expected_cursor_revision"] = actual["plan"]["expected_cursor_revision"].clone();
        }
        assert_eq!(actual, expected, "{case}");
        if let Some(before) = before {
            assert_eq!(engine.export().unwrap(), before);
        }
        if matches!(
            assessment,
            AssessmentV1::Automatic { .. } | AssessmentV1::NeedsConfirmation { .. }
        ) {
            let p = plan(assessment);
            assert_eq!(p.report.encode().unwrap(), corpus(case, "report.cbor"));
            let applied = engine.apply(&p, budget()).unwrap();
            assert_eq!(
                serde_json::to_value(applied).unwrap(),
                serde_json::from_slice::<serde_json::Value>(&corpus(case, "apply.json")).unwrap(),
                "{case}"
            );
            {
                let bytes = if case == "shallow-history" {
                    engine.export_shallow(1).unwrap()
                } else {
                    hex::decode(engine.export().unwrap()).unwrap()
                };
                assert_eq!(bytes, corpus(case, "target.checkpoint"), "{case}");
            }
        }
    }
}

#[test]
fn driver_binds_confirmation_to_target_budget_registry_and_snapshot() {
    let (mut e, target, ids) = engine("target-rejoins");
    let p = plan(e.assess(target, &ids, budget()).unwrap());
    let tighter = ComparisonBudgetV1 {
        transitions: 0,
        ..budget()
    };
    assert_eq!(e.apply(&p, tighter).unwrap(), ApplyV1::PlanChanged);
    let _ = e.assess(target, &ids, tighter).unwrap();
    assert_eq!(e.apply(&p, budget()).unwrap(), ApplyV1::PlanChanged);
    let p = plan(e.assess(target, &ids, budget()).unwrap());
    let source = publication("target-rejoins", "source.narpack");
    assert!(matches!(
        e.assess(source.program().artifact_id(), &[], budget())
            .unwrap(),
        AssessmentV1::Exact { .. }
    ));
    assert_eq!(e.apply(&p, budget()).unwrap(), ApplyV1::PlanChanged);
    let p = plan(e.assess(target, &ids, budget()).unwrap());
    e.restore(&hex::encode(corpus("target-rejoins", "source.checkpoint")))
        .unwrap();
    assert_eq!(e.apply(&p, budget()).unwrap(), ApplyV1::PlanChanged);
    let p = plan(e.assess(target, &ids, budget()).unwrap());
    e.register_publication(&source).unwrap();
    assert_eq!(e.apply(&p, budget()).unwrap(), ApplyV1::PlanChanged);
}

#[test]
fn snapshot_failures_are_redacted_and_missing_objects_are_retryable_without_hiding_bad_budgets() {
    let mut e = Engine::default();
    let source = publication("unseen-rule", "source.narpack");
    let target = source.program().artifact_id();
    assert!(matches!(
        e.assess_save(
            &hex::encode(corpus("unseen-rule", "source.checkpoint")),
            target,
            &[],
            budget()
        )
        .unwrap(),
        AssessmentV1::NeedsObjects {
            retryable: true,
            ..
        }
    ));
    e.register_publication(&source).unwrap();
    let invalid = e
        .assess_save("private-invalid-payload", target, &[], budget())
        .unwrap();
    assert!(matches!(invalid, AssessmentV1::InvalidSave { .. }));
    assert!(
        !serde_json::to_string(&invalid)
            .unwrap()
            .contains("private-invalid-payload")
    );
    assert!(
        e.assess_save(
            "private-invalid-payload",
            target,
            &[],
            ComparisonBudgetV1 {
                transitions: 10_001,
                ..budget()
            }
        )
        .is_err()
    );
}

#[test]
fn assessment_forks_share_trust_but_never_approvals_or_mutable_cursors() {
    let (mut e, target, ids) = engine("unseen-rule");
    let p = plan(e.assess(target, &ids, budget()).unwrap());
    let mut fork = e.fork();
    assert_eq!(fork.apply(&p, budget()).unwrap(), ApplyV1::PlanChanged);
    let source = publication("unseen-rule", "source.narpack");
    assert!(e.register_publication(&source).is_err());
    fork.restore(&hex::encode(corpus("unseen-rule", "source.checkpoint")))
        .unwrap();
    assert!(matches!(
        fork.assess(target, &ids, budget()).unwrap(),
        AssessmentV1::Automatic { .. }
    ));
    drop(fork);
    e.register_publication(&source).unwrap();
}

fn drive<T>(
    e: &mut Engine,
    host: &MemoryHost,
    mut operation: impl FnMut(&mut Engine) -> narrata_nodes::Result<T>,
) -> T {
    for _ in 0..1000 {
        match operation(e) {
            Ok(value) => return value,
            Err(error) => {
                let Some(request) = e.take_request() else {
                    panic!("{error}")
                };
                e.cache.load(&host.load(&request)).unwrap();
            }
        }
    }
    panic!("load loop did not settle")
}

#[test]
fn stored_assessment_is_read_only_and_flush_cas_preserves_the_winning_cursor() {
    let previous = publication("content-reference", "source.narpack");
    let candidate = attach(
        &previous,
        publication("content-reference", "target.narpack"),
    );
    let host = MemoryHost::new(StoreId::from_bytes([3; 16]));
    let cache = CacheBackend::new();
    let execution = ExecutionId::from_bytes([4; 16]);
    let source = testing::run(&cache, &host, || {
        Session::open(previous.shared_program(), execution, cache.share())
    })
    .unwrap();
    let source_cursor = source.cursor().unwrap();
    let old_save = source.export().unwrap();
    let before = host.state();
    let mut a = Engine::default();
    let id = a.register_evolution(&previous, &candidate).unwrap();
    a.connect(execution);
    let p = plan(drive(&mut a, &host, |e| {
        e.assess(candidate.program().artifact_id(), &[id], budget())
    }));
    assert_eq!(host.state(), before);
    assert!(a.cache.unconfirmed().is_none());
    let mut b = Engine::default();
    b.register_evolution(&previous, &candidate).unwrap();
    b.connect(execution);
    let q = plan(drive(&mut b, &host, |e| {
        e.assess(candidate.program().artifact_id(), &[id], budget())
    }));
    assert_eq!(p, q);
    assert!(matches!(
        drive(&mut a, &host, |e| e.apply(&p, budget())),
        ApplyV1::Migrated { .. }
    ));
    assert!(matches!(
        drive(&mut b, &host, |e| e.apply(&q, budget())),
        ApplyV1::Migrated { .. }
    ));
    // Neither result is published before flush confirmation. Both old and new objects are
    // immutable; only one active-cursor CAS may win in the durable host.
    assert_eq!(host.state(), before);
    let flush = a.cache.unconfirmed().unwrap();
    let reply = host.persist(&flush);
    a.cache.confirm(&reply).unwrap();
    assert!(
        b.cache
            .confirm(&host.persist(&b.cache.unconfirmed().unwrap()))
            .is_err()
    );
    let restored =
        Session::open_with_registry(&a.registry, execution, testing::Driven::cold(host.share()))
            .unwrap();
    assert_eq!(restored.cursor().unwrap(), p.migration_commit);
    assert_ne!(restored.cursor().unwrap(), source_cursor);
    // Migration does not invalidate the old checkpoint or its exact artifact.
    let old = Session::restore(previous.shared_program(), &old_save).unwrap();
    assert_eq!(old.cursor().unwrap(), source_cursor);
}
