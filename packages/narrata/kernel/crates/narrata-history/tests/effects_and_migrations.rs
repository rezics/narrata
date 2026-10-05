#![allow(clippy::panic, clippy::unwrap_used)]

#[macro_use]
mod support;

use std::collections::BTreeSet;

use narrata_history::{
    BundleLimits, Commit, Domain, DomainKinds, History, HistoryError, Object, ObjectId, RefKey,
    RefMutation, RetentionPolicy, Session, Transaction,
    effect::{
        self, DeliveryPolicy, DiagnosticId, DomainEffects, EffectClaim, EffectClaimResult,
        EffectId, EffectOutcome, EffectOutcomeRecord, EffectRequestDigest, EffectStoreError,
        ExecutionId, LeaseId, LedgerFence, LedgerStatus, RecordedEffectResponse, RewindPolicy,
    },
    migration::{
        ArtifactRegistry, MigrationError, MigrationId, MigrationInput, MigrationRegistry,
        MigrationRegistryError, MigrationRequest,
    },
    testing::Counter,
};
use narrata_storage::{
    Batch, Expect, Limits, MemoryBackend, StorageBackend, StorageError,
    testing::{Counting, Fault, FaultInjecting, FaultPlan, Primitive, Trigger},
};
use support::{COUNTER, Fixture, name, open, step};

type Effects = DomainEffects<Counter>;

on_both_backends!(
    effects_claim_expire_reuse_and_compensate,
    effects_keep_origins_and_responses_alive,
    effects_reconcile_unknown_storage_outcomes,
    migration_dry_run_continue_and_transfer,
    migration_ref_conflict_writes_nothing,
    migration_reconciles_unknown_storage_outcomes,
);

fn claim(origin: ObjectId, n: u8) -> EffectClaim {
    EffectClaim {
        execution: ExecutionId::from_u128(1),
        effect: EffectId::from_bytes([n; 32]),
        request_digest: EffectRequestDigest::from_bytes([n; 32]),
        capability: b"query".to_vec(),
        capability_version: vec![1],
        origin_commit: origin,
        delivery: DeliveryPolicy::RecordedQuery,
        rewind: RewindPolicy::ReuseRecordedResponse,
        lease: LeaseId::from_u128(1),
        now: 2,
        expires_at: 10,
    }
}

fn record(claim: &EffectClaim, outcome: EffectOutcome, observed_at: u64) -> EffectOutcomeRecord {
    EffectOutcomeRecord {
        execution: claim.execution,
        effect: claim.effect,
        request_digest: claim.request_digest,
        lease: claim.lease,
        outcome,
        observed_at,
    }
}

fn response(claim: &EffectClaim) -> RecordedEffectResponse {
    RecordedEffectResponse {
        effect: claim.effect,
        request_digest: claim.request_digest,
        capability: claim.capability.clone(),
        capability_version: claim.capability_version.clone(),
        payload: vec![0xa0],
    }
}

fn effects_claim_expire_reuse_and_compensate<F: Fixture>() {
    let fixture = F::new();
    let mut history = open(&fixture);
    let (_, root) = Session::create(&mut history, COUNTER, name("effects"), &0, 1).unwrap();
    let c = claim(root.commit, 1);
    let mut missing = c.clone();
    missing.origin_commit = ObjectId::from_bytes([99; 32]);
    assert_eq!(
        history.claim_effect::<Effects>(missing.clone()),
        Err(HistoryError::MissingObject(missing.origin_commit))
    );
    assert!(
        history
            .read_effect_entry::<Effects>(c.execution, c.effect)
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        history.claim_effect::<Effects>(c.clone()).unwrap(),
        EffectClaimResult::Claimed(_)
    ));
    let mut other = c.clone();
    other.lease = LeaseId::from_u128(2);
    other.now = 3;
    assert_eq!(
        history.claim_effect::<Effects>(other.clone()).unwrap(),
        EffectClaimResult::Leased {
            lease: c.lease,
            expires_at: 10
        }
    );
    let renewed = history
        .renew_effect_lease::<Effects>(c.execution, c.effect, c.lease, 4, 12)
        .unwrap();
    assert!(matches!(
        renewed.status,
        LedgerStatus::Claimed { expires_at: 12, .. }
    ));
    other.now = 12;
    other.expires_at = 20;
    assert!(matches!(
        history.claim_effect::<Effects>(other.clone()).unwrap(),
        EffectClaimResult::Claimed(_)
    ));
    assert_eq!(
        history.record_effect_outcome::<Effects>(record(
            &c,
            EffectOutcome::Completed(response(&c)),
            13
        )),
        Err(HistoryError::Effect(EffectStoreError::LeaseMismatch))
    );
    let mut bad_response = response(&other);
    bad_response.capability = b"different".to_vec();
    assert!(matches!(
        history.record_effect_outcome::<Effects>(record(
            &other,
            EffectOutcome::Completed(bad_response),
            13
        )),
        Err(HistoryError::Effect(EffectStoreError::InvalidResponse(_)))
    ));
    let completed = history
        .record_effect_outcome::<Effects>(record(
            &other,
            EffectOutcome::Completed(response(&other)),
            13,
        ))
        .unwrap();
    assert_eq!(completed.status.fence(), Some(LedgerFence::from_u64(1)));
    assert_eq!(
        history.claim_effect::<Effects>(other.clone()).unwrap(),
        EffectClaimResult::Recorded(completed.clone())
    );
    assert_eq!(
        history.record_effect_outcome::<Effects>(record(
            &other,
            EffectOutcome::Completed(response(&other)),
            14
        )),
        Err(HistoryError::Effect(EffectStoreError::Terminal))
    );
    let mut different = other.clone();
    different.request_digest = EffectRequestDigest::from_bytes([98; 32]);
    assert_eq!(
        history.claim_effect::<Effects>(different),
        Err(HistoryError::Effect(EffectStoreError::RequestConflict))
    );

    let mut retry = claim(root.commit, 2);
    history.claim_effect::<Effects>(retry.clone()).unwrap();
    let diagnostic = DiagnosticId::from_bytes([42; 32]);
    history
        .record_effect_outcome::<Effects>(record(
            &retry,
            EffectOutcome::RetryableFailure(diagnostic),
            3,
        ))
        .unwrap();
    retry.now = 4;
    history.claim_effect::<Effects>(retry.clone()).unwrap();
    let unknown = history
        .record_effect_outcome::<Effects>(record(
            &retry,
            EffectOutcome::UnknownOutcome(diagnostic),
            5,
        ))
        .unwrap();
    assert_eq!(unknown.status.fence(), Some(LedgerFence::from_u64(2)));
    assert_eq!(
        history.claim_effect::<Effects>(retry.clone()).unwrap(),
        EffectClaimResult::Recorded(unknown)
    );

    let mut original = claim(root.commit, 3);
    original.rewind = RewindPolicy::Compensatable {
        capability: b"undo".to_vec(),
    };
    history.claim_effect::<Effects>(original.clone()).unwrap();
    let done = history
        .record_effect_outcome::<Effects>(record(
            &original,
            EffectOutcome::Completed(response(&original)),
            3,
        ))
        .unwrap();
    let mut undo = claim(root.commit, 4);
    undo.capability = b"undo".to_vec();
    history.claim_effect::<Effects>(undo.clone()).unwrap();
    assert_eq!(
        history.mark_effect_compensated::<Effects>(
            original.execution,
            original.effect,
            undo.effect
        ),
        Err(HistoryError::Effect(EffectStoreError::InvalidCompensation))
    );
    history
        .record_effect_outcome::<Effects>(record(
            &undo,
            EffectOutcome::Completed(response(&undo)),
            4,
        ))
        .unwrap();
    let compensated = history
        .mark_effect_compensated::<Effects>(original.execution, original.effect, undo.effect)
        .unwrap();
    assert!(
        matches!(compensated.status,LedgerStatus::Compensated {original_fence,compensation_fence,..} if original_fence.get()==3 && compensation_fence.get()==4)
    );
    assert_eq!(compensated.status.response(), done.status.response());
    assert_eq!(
        history.claim_effect::<Effects>(original.clone()).unwrap(),
        EffectClaimResult::Recorded(compensated)
    );
    let page = history
        .scan_effects::<Effects>(c.execution, None, 2)
        .unwrap();
    assert_eq!(page.items.len(), 2);
    assert!(page.more);
    let rest = history
        .scan_effects::<Effects>(c.execution, Some(&page.items[1].effect), 2)
        .unwrap();
    assert_eq!(rest.items.len(), 2);
    assert!(!rest.more);
    assert!(
        history
            .scan_effects::<Effects>(ExecutionId::from_u128(99), None, 2)
            .unwrap()
            .items
            .is_empty()
    );
    assert!(history.integrity_scan().unwrap().is_empty());
}

fn effects_keep_origins_and_responses_alive<F: Fixture>() {
    let fixture = F::new();
    let mut history = open(&fixture);
    let (session, root) = Session::create(&mut history, COUNTER, name("effects"), &0, 1).unwrap();
    let c = claim(root.commit, 1);
    history.claim_effect::<Effects>(c.clone()).unwrap();
    let entry = history
        .record_effect_outcome::<Effects>(record(&c, EffectOutcome::Rejected(response(&c)), 3))
        .unwrap();
    let cursor = history
        .read_ref(&RefKey::active(name("effects")).unwrap())
        .unwrap()
        .unwrap();
    let branch = RefKey::branch(name("effects"), session.branch().unwrap());
    let branch_value = history.read_ref(&branch).unwrap().unwrap();
    history
        .write(
            &Transaction {
                refs: vec![
                    RefMutation {
                        key: RefKey::active(name("effects")).unwrap(),
                        expected: Some(cursor.revision),
                        next: None,
                    },
                    RefMutation {
                        key: branch,
                        expected: Some(branch_value.revision),
                        next: None,
                    },
                ],
                ..Transaction::default()
            },
            |_| Ok(vec![]),
        )
        .unwrap();
    history
        .collect(RetentionPolicy {
            now: 100,
            grace_seconds: 0,
            ..Default::default()
        })
        .unwrap();
    assert!(history.get_object(root.commit).unwrap().is_some());
    assert!(
        history
            .get_object(entry.status.response().unwrap())
            .unwrap()
            .is_some()
    );
    assert!(
        history
            .get_object(effect::ledger_guard().id())
            .unwrap()
            .is_some()
    );
}

fn effects_reconcile_unknown_storage_outcomes<F: Fixture>() {
    let fixture = F::new();
    let mut history = History::open(
        FaultInjecting::new(fixture.backend(), FaultPlan::new()),
        DomainKinds::<Counter>::new(),
    )
    .unwrap();
    let (_, root) = Session::create(&mut history, COUNTER, name("effects"), &0, 1).unwrap();
    let c = claim(root.commit, 1);
    history
        .backend_mut()
        .set_plan(FaultPlan::new().at(Trigger::Nth(Primitive::Apply, 1), Fault::After));
    history.claim_effect::<Effects>(c.clone()).unwrap();
    history
        .backend_mut()
        .set_plan(FaultPlan::new().at(Trigger::Nth(Primitive::Apply, 1), Fault::After));
    let entry = history
        .record_effect_outcome::<Effects>(record(&c, EffectOutcome::Completed(response(&c)), 3))
        .unwrap();
    assert_eq!(entry.status.fence().unwrap().get(), 1);
    assert_eq!(
        history.claim_effect::<Effects>(c).unwrap(),
        EffectClaimResult::Recorded(entry)
    );
}

const TARGET: Counter = Counter::new(2000);
fn artifacts() -> ArtifactRegistry<narrata_history::ArtifactId, Counter> {
    let mut artifacts = ArtifactRegistry::new();
    artifacts.register(COUNTER.artifact_id(), COUNTER);
    artifacts.register(TARGET.artifact_id(), TARGET);
    artifacts
}
fn migrations() -> MigrationRegistry<MigrationId, narrata_history::ArtifactId, ()> {
    let mut registry = MigrationRegistry::new();
    registry
        .register(
            MigrationId::from_bytes([1; 32]),
            COUNTER.artifact_id(),
            TARGET.artifact_id(),
            (),
        )
        .unwrap();
    registry
}
fn migrate(_: &(), _: &Counter, _: &Counter, state: &u64) -> Result<(u64, ()), &'static str> {
    Ok((state + 10, ()))
}
fn request(source: ObjectId) -> MigrationRequest {
    MigrationRequest {
        source,
        target: TARGET.artifact_id(),
        explicit_path: None,
        target_ref: RefKey::save(name("player"), name("migrated")),
        expected_ref: None,
        observed_at: 3,
    }
}

fn migration_dry_run_continue_and_transfer<F: Fixture>() {
    let fixture = F::new();
    let mut history = History::open(
        Counting::new(fixture.backend()),
        DomainKinds::<Counter>::new(),
    )
    .unwrap();
    let (_, root) = Session::create(&mut history, COUNTER, name("player"), &1, 1).unwrap();
    let request = request(root.commit);
    let artifacts = artifacts();
    let migrations = migrations();
    history.backend().take_counts();
    let dry = history
        .dry_run_migration(&artifacts, &migrations, &request, migrate)
        .unwrap();
    assert_eq!(history.backend().take_counts().apply, 0);
    assert_eq!(dry.state, 11);
    assert!(history.read_ref(&request.target_ref).unwrap().is_none());
    let applied = history
        .apply_migration(&artifacts, &migrations, &request, migrate)
        .unwrap();
    assert_eq!(history.backend().take_counts().apply, 1);
    assert_eq!(dry.commit, applied.commit);
    assert_eq!(dry.commits, applied.commits);
    assert_eq!(history.load(&COUNTER, root.commit).unwrap().state, 1);
    assert_eq!(history.load(&TARGET, applied.commit).unwrap().state, 11);
    let bytes = history
        .export(applied.commit, &BTreeSet::new())
        .unwrap()
        .to_bytes()
        .unwrap();
    let fresh = F::new();
    let mut imported = open(&fresh);
    imported
        .import(
            &TARGET,
            &bytes,
            BundleLimits::default(),
            RefKey::active(name("imported")).unwrap(),
            None,
            4,
        )
        .unwrap();
    let (mut session, _) = Session::open(&imported, TARGET, name("imported")).unwrap();
    let advanced = session
        .advance(&mut imported, applied.commit, &1, step, 5)
        .unwrap();
    assert_eq!(advanced.state, 12);
    let input = Object::new(
        Counter::INPUT_KIND,
        Counter::INPUT_SCHEMA,
        &TARGET.encode_input(&1),
    );
    let state = Object::new(
        Counter::STATE_KIND,
        Counter::STATE_SCHEMA,
        &TARGET.encode_state(&12),
    );
    let illegal = Commit {
        artifact: TARGET.artifact_id(),
        parent: Some(root.commit),
        input: Some(input.id()),
        state: state.id(),
        depth: 1,
    }
    .to_object(Counter::COMMIT_KIND);
    assert!(matches!(
        history.write(
            &Transaction {
                objects: vec![input, state, illegal],
                ..Default::default()
            },
            |_| Ok(vec![])
        ),
        Err(HistoryError::InvalidGraph(_))
    ));
    assert!(imported.integrity_scan().unwrap().is_empty());
}

fn migration_ref_conflict_writes_nothing<F: Fixture>() {
    let fixture = F::new();
    let mut history = History::open(
        Counting::new(fixture.backend()),
        DomainKinds::<Counter>::new(),
    )
    .unwrap();
    let (_, root) = Session::create(&mut history, COUNTER, name("player"), &1, 1).unwrap();
    let mut req = request(root.commit);
    req.target_ref = RefKey::active(name("player")).unwrap();
    history.backend().take_counts();
    let dry = history
        .dry_run_migration(&artifacts(), &migrations(), &req, migrate)
        .unwrap();
    assert!(matches!(
        history.apply_migration(&artifacts(), &migrations(), &req, migrate),
        Err(MigrationError::History(HistoryError::RefConflict(_)))
    ));
    assert_eq!(history.backend().take_counts().apply, 1);
    assert!(history.get_object(dry.commit).unwrap().is_none());
    assert_eq!(
        history.read_ref(&req.target_ref).unwrap().unwrap().commit,
        root.commit
    );
    let bad = |_: &(), _: &Counter, _: &Counter, _: &u64| -> Result<(u64, ()), &'static str> {
        Ok((3000, ()))
    };
    assert!(matches!(
        history.dry_run_migration(&artifacts(), &migrations(), &req, bad),
        Err(MigrationError::History(HistoryError::Corrupt(..)))
    ));
    assert_eq!(history.backend().take_counts().apply, 0);
}

fn migration_reconciles_unknown_storage_outcomes<F: Fixture>() {
    let fixture = F::new();
    let mut history = History::open(
        FaultInjecting::new(fixture.backend(), FaultPlan::new()),
        DomainKinds::<Counter>::new(),
    )
    .unwrap();
    let (_, root) = Session::create(&mut history, COUNTER, name("player"), &1, 1).unwrap();
    let req = request(root.commit);
    history.backend_mut().set_plan(FaultPlan::new().at(
        Trigger::Nth(Primitive::Apply, 1),
        Fault::Before(StorageError::Io("before".into())),
    ));
    assert!(
        history
            .apply_migration(&artifacts(), &migrations(), &req, migrate)
            .is_err()
    );
    assert!(history.read_ref(&req.target_ref).unwrap().is_none());
    history
        .backend_mut()
        .set_plan(FaultPlan::new().at(Trigger::Nth(Primitive::Apply, 1), Fault::After));
    let applied = history
        .apply_migration(&artifacts(), &migrations(), &req, migrate)
        .unwrap();
    assert_eq!(
        history.read_ref(&req.target_ref).unwrap().unwrap().commit,
        applied.commit
    );
}

#[test]
fn migration_registry_requires_unambiguous_connected_paths() {
    let mut registry = MigrationRegistry::new();
    registry.register(1, 10, 20, ()).unwrap();
    registry.register(2, 20, 30, ()).unwrap();
    registry.register(3, 10, 30, ()).unwrap();
    registry.register(4, 20, 10, ()).unwrap();
    assert_eq!(
        registry.inspect(10, 30, None),
        Err(MigrationRegistryError::Ambiguous { from: 10, to: 30 })
    );
    assert_eq!(
        registry.inspect(10, 30, Some(&[1, 2])).unwrap().artifacts,
        vec![10, 20, 30]
    );
    assert_eq!(
        registry.inspect(10, 30, Some(&[2])),
        Err(MigrationRegistryError::Disconnected(2))
    );
    assert_eq!(
        registry.register(5, 30, 30, ()),
        Err(MigrationRegistryError::SelfEdge(5))
    );
    assert_eq!(
        registry.register(1, 30, 40, ()),
        Err(MigrationRegistryError::Duplicate(1))
    );
    assert_eq!(
        registry.inspect(30, 10, None),
        Err(MigrationRegistryError::NoPath { from: 30, to: 10 })
    );
}

#[test]
fn migration_rejects_large_atomic_batches_and_wrong_links() {
    let backend = MemoryBackend::with_limits(Limits {
        max_batch_ops: 10,
        ..Limits::default()
    });
    let mut history = History::open(Counting::new(backend), DomainKinds::<Counter>::new()).unwrap();
    let (_, root) = Session::create(&mut history, COUNTER, name("player"), &1, 1).unwrap();
    history.backend().take_counts();
    assert!(matches!(
        history.apply_migration(&artifacts(), &migrations(), &request(root.commit), migrate),
        Err(MigrationError::History(HistoryError::Limit("atomic batch")))
    ));
    assert_eq!(history.backend().take_counts().apply, 0);
    let mut history = History::in_memory(DomainKinds::<Counter>::new());
    let (_, root) = Session::create(&mut history, COUNTER, name("player"), &1, 1).unwrap();
    let state = Object::new(Counter::STATE_KIND, 1, &TARGET.encode_state(&11));
    let input = MigrationInput {
        id: MigrationId::from_bytes([1; 32]),
        from: TARGET.artifact_id(),
        to: COUNTER.artifact_id(),
    }
    .to_object();
    let child = Commit {
        artifact: TARGET.artifact_id(),
        parent: Some(root.commit),
        input: Some(input.id()),
        state: state.id(),
        depth: 1,
    }
    .to_object(Counter::COMMIT_KIND);
    assert!(matches!(
        history.write_atomic(
            &Transaction {
                objects: vec![state, input, child],
                ..Default::default()
            },
            |_| Ok(vec![])
        ),
        Err(HistoryError::InvalidGraph(_))
    ));
}

#[test]
fn generic_effect_and_migration_decoders_reject_bad_bytes() {
    let c = claim(ObjectId::from_bytes([9; 32]), 1);
    let entry = narrata_history::effect::EffectLedgerEntry {
        execution: c.execution,
        effect: c.effect,
        request_digest: c.request_digest,
        capability: c.capability.clone(),
        capability_version: vec![1],
        origin_commit: c.origin_commit,
        delivery: c.delivery,
        rewind: c.rewind.clone(),
        status: LedgerStatus::Completed {
            response: ObjectId::from_bytes([8; 32]),
            fence: LedgerFence::zero(),
        },
    };
    let key = effect::effect_key(c.execution, c.effect);
    assert!(effect::decode_effect(&key, &effect::encode_effect(&entry)).is_err());
    assert!(effect::decode_effect(&key[..47], &effect::encode_effect(&entry)).is_err());
    let mut response = response(&c).encode();
    response.push(0);
    assert!(RecordedEffectResponse::decode(&response).is_err());
    let input = MigrationInput {
        id: MigrationId::from_bytes([1; 32]),
        from: COUNTER.artifact_id(),
        to: TARGET.artifact_id(),
    };
    let mut bytes = input.encode();
    bytes.push(0);
    assert!(MigrationInput::decode(&bytes).is_err());
    let self_edge = MigrationInput {
        to: input.from,
        ..input
    };
    assert!(MigrationInput::decode(&self_edge.encode()).is_err());
}

#[test]
fn atomic_dry_run_respects_value_limits_before_writing() {
    let backend = MemoryBackend::with_limits(Limits {
        max_value_bytes: 160,
        ..Limits::default()
    });
    let mut history = History::open(Counting::new(backend), DomainKinds::<Counter>::new()).unwrap();
    let (_, root) = Session::create(&mut history, COUNTER, name("player"), &1, 1).unwrap();
    history.backend().take_counts();
    assert!(matches!(
        history.apply_migration(&artifacts(), &migrations(), &request(root.commit), migrate),
        Err(MigrationError::History(HistoryError::Storage(
            StorageError::Limit(_)
        )))
    ));
    assert_eq!(history.backend().take_counts().apply, 0);
}

#[test]
fn fence_overflow_preserves_the_claim_and_does_not_store_response() {
    let mut history = History::in_memory(DomainKinds::<Counter>::new());
    let (_, root) = Session::create(&mut history, COUNTER, name("player"), &1, 1).unwrap();
    let c = claim(root.commit, 1);
    history.claim_effect::<Effects>(c.clone()).unwrap();
    history
        .backend_mut()
        .apply(&Batch::new().put(
            effect::LEDGER_FENCES,
            effect::fence_key(c.execution),
            effect::encode_fence(LedgerFence::from_u64(u64::MAX)),
            Expect::Absent,
        ))
        .unwrap();
    let response = response(&c);
    let id = response.to_object().id();
    assert_eq!(
        history.record_effect_outcome::<Effects>(record(&c, EffectOutcome::Completed(response), 3)),
        Err(HistoryError::Effect(EffectStoreError::FenceOverflow))
    );
    assert!(history.get_object(id).unwrap().is_none());
    assert!(matches!(
        history
            .read_effect_entry::<Effects>(c.execution, c.effect)
            .unwrap()
            .unwrap()
            .0
            .status,
        LedgerStatus::Claimed { .. }
    ));
}
