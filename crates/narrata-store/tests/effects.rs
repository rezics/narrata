#![allow(clippy::panic, clippy::unwrap_used)]

#[macro_use]
mod support;

use std::sync::Arc;

use narrata_core::{
    CapabilityId, CapabilityVersion, DeliveryPolicy, DiagnosticId, EffectId, EffectRequestDigest,
    ExecutionId, GlobalId, InputId, RewindPolicy, Value, builtin_host_capabilities,
    program::{encode_program_artifact, load_program},
    runtime::{CheckedRuntimeInput, DraftResult},
};
use narrata_storage::{
    StorageBackend, StorageError,
    testing::{Fault, FaultInjecting, FaultPlan, Primitive, Trigger},
};
use narrata_store::{
    BranchId, CommitTransaction, CoordinatorError, EffectClaim, EffectClaimResult, EffectOutcome,
    EffectOutcomeRecord, EffectStoreError, InitialRecordingMode, LeaseId, LedgerFence,
    LedgerStatus, RecordedEffectResponseV1, RefMutation, RefName, RetentionPolicy, SaveStore,
    SessionCoordinator, Store, StoreError,
};
use narrata_testkit::generator::{barrier_command_v0, recorded_query_v0, scene_reconcile_v0};
use support::{Backend, assert_atomic, effects, refs};

fn checked_query_program() -> Arc<narrata_core::CheckedProgram> {
    load_program(
        &encode_program_artifact(&recorded_query_v0().unwrap()),
        &Default::default(),
    )
    .unwrap()
}

fn coordinator<S: SaveStore>(
    store: S,
    program: Arc<narrata_core::CheckedProgram>,
    execution: u128,
) -> SessionCoordinator<S> {
    SessionCoordinator::create_with_capabilities(
        store,
        program,
        ExecutionId::from_u128(execution),
        RefName::new(format!("effect-session-{execution}")).unwrap(),
        BranchId::from_u128(1),
        InitialRecordingMode::Standard,
        1,
        &builtin_host_capabilities().unwrap(),
    )
    .unwrap()
}

fn dispatch_pending<S: SaveStore>(
    coordinator: &mut SessionCoordinator<S>,
) -> narrata_core::EffectId {
    let committed = coordinator
        .dispatch(
            CheckedRuntimeInput::start(InputId::from_u128(1)),
            Default::default(),
            2,
        )
        .unwrap();
    let DraftResult::AwaitEffect(request) = committed.result else {
        panic!("expected pending Effect");
    };
    assert!(
        coordinator
            .store()
            .get_object(narrata_core::ObjectId::from_bytes(
                *committed.commit.as_bytes()
            ))
            .unwrap()
            .is_some(),
        "Effect may only be published with its durable pending Commit"
    );
    request.id
}

fn required_capability_is_negotiated_before_execution<B: Backend>() {
    let program = checked_query_program();
    let result = SessionCoordinator::create(
        B::store(),
        program,
        ExecutionId::from_u128(1),
        RefName::new("missing-capability").unwrap(),
        BranchId::from_u128(1),
        InitialRecordingMode::Standard,
        1,
    );
    assert!(matches!(
        result,
        Err(CoordinatorError::CapabilityNegotiation(_))
    ));
}

fn recorded_query_survives_crash_and_replay_without_redispatch<B: Backend>() {
    let program = checked_query_program();
    let host = builtin_host_capabilities().unwrap();
    let execution = ExecutionId::from_u128(2);
    let session = RefName::new("query-crash").unwrap();
    let branch = BranchId::from_u128(1);
    let mut coordinator = SessionCoordinator::create_with_capabilities(
        B::store(),
        Arc::clone(&program),
        execution,
        session.clone(),
        branch,
        InitialRecordingMode::Standard,
        1,
        &host,
    )
    .unwrap();
    let effect = dispatch_pending(&mut coordinator);
    let pending_commit = coordinator.timeline().cursor;
    let lease = LeaseId::from_u128(1);
    assert!(matches!(
        coordinator.claim_pending_effect(lease, 3, 30).unwrap(),
        EffectClaimResult::Claimed(_)
    ));
    coordinator
        .complete_pending_effect(lease, Value::I64(11), 4)
        .unwrap();
    assert_eq!(
        coordinator.store().current_ledger_fence(execution).unwrap(),
        LedgerFence::from_u64(1)
    );

    let store = coordinator.into_store();
    let mut recovered = SessionCoordinator::open_with_capabilities(
        store,
        Arc::clone(&program),
        execution,
        session,
        branch,
        &host,
    )
    .unwrap();
    let caught_up = recovered
        .recover_pending_effect(Default::default(), 5)
        .unwrap()
        .unwrap();
    assert!(matches!(caught_up.result, DraftResult::AwaitSay(_)));
    assert_eq!(
        caught_up.state.globals.get(&GlobalId::from_u128(30)),
        Some(&Value::I64(11))
    );

    recovered.rewind_to(pending_commit, 6).unwrap();
    assert!(matches!(
        recovered
            .claim_pending_effect(LeaseId::from_u128(2), 7, 40)
            .unwrap(),
        EffectClaimResult::Recorded(_)
    ));
    let replay = recovered
        .recover_pending_effect(Default::default(), 8)
        .unwrap()
        .unwrap();
    assert!(replay.reused);
    assert_eq!(effects(recovered.store(), execution).len(), 1);
    assert_eq!(
        recovered
            .store()
            .read_effect(execution, effect)
            .unwrap()
            .unwrap()
            .request_digest,
        effects(recovered.store(), execution)[0].request_digest
    );
}

fn leases_expire_and_request_identity_cannot_be_rebound<B: Backend>() {
    let mut coordinator = coordinator(B::store(), checked_query_program(), 3);
    dispatch_pending(&mut coordinator);
    let first = LeaseId::from_u128(1);
    let second = LeaseId::from_u128(2);
    let claimed = coordinator.claim_pending_effect(first, 10, 20).unwrap();
    let EffectClaimResult::Claimed(entry) = claimed else {
        panic!("expected first claim");
    };
    assert!(matches!(
        coordinator.claim_pending_effect(second, 15, 25).unwrap(),
        EffectClaimResult::Leased { lease, expires_at: 20 } if lease == first
    ));
    let renewal = coordinator.renew_pending_effect(first, 20, 30);
    assert!(matches!(
        renewal,
        Err(CoordinatorError::Store(StoreError::Effect(error)))
            if *error == EffectStoreError::LeaseMismatch
    ));
    let stale_outcome = coordinator.complete_pending_effect(first, Value::I64(1), 20);
    assert!(matches!(
        stale_outcome,
        Err(CoordinatorError::Store(StoreError::Effect(error)))
            if *error == EffectStoreError::LeaseMismatch
    ));
    assert!(matches!(
        coordinator.claim_pending_effect(second, 20, 30).unwrap(),
        EffectClaimResult::Claimed(_)
    ));

    let conflict = coordinator.store_mut().claim_effect(EffectClaim {
        request_digest: EffectRequestDigest::from_bytes([99; 32]),
        lease: LeaseId::from_u128(3),
        now: 21,
        expires_at: 40,
        execution: entry.execution,
        effect: entry.effect,
        capability: entry.capability,
        capability_version: entry.capability_version,
        origin_commit: entry.origin_commit,
        delivery: entry.delivery,
        rewind: entry.rewind,
    });
    assert!(matches!(
        conflict,
        Err(StoreError::Effect(error)) if *error == EffectStoreError::RequestConflict
    ));
}

fn unknown_outcome_stops_catchup_and_all_other_input<B: Backend>() {
    let execution = ExecutionId::from_u128(4);
    let mut coordinator = coordinator(B::store(), checked_query_program(), 4);
    dispatch_pending(&mut coordinator);
    let lease = LeaseId::from_u128(1);
    coordinator.claim_pending_effect(lease, 2, 20).unwrap();
    coordinator
        .record_pending_unknown_outcome(lease, DiagnosticId::from_bytes([7; 32]), 3)
        .unwrap();
    assert_eq!(
        coordinator.store().current_ledger_fence(execution).unwrap(),
        LedgerFence::from_u64(1)
    );
    assert!(matches!(
        coordinator.recover_pending_effect(Default::default(), 4),
        Err(CoordinatorError::UnknownOutcome)
    ));
    assert!(matches!(
        coordinator.dispatch(
            CheckedRuntimeInput::start(InputId::from_u128(2)),
            Default::default(),
            5
        ),
        Err(CoordinatorError::UnknownOutcome)
    ));
}

fn completed_barrier_blocks_rewind_before_cursor_mutation<B: Backend>() {
    let program = load_program(
        &encode_program_artifact(&barrier_command_v0().unwrap()),
        &Default::default(),
    )
    .unwrap();
    let mut coordinator = coordinator(B::store(), program, 5);
    let genesis = coordinator.timeline().cursor;
    dispatch_pending(&mut coordinator);
    let lease = LeaseId::from_u128(1);
    coordinator.claim_pending_effect(lease, 2, 20).unwrap();
    coordinator
        .complete_pending_effect(lease, Value::I64(1), 3)
        .unwrap();
    coordinator
        .recover_pending_effect(Default::default(), 4)
        .unwrap();
    let before = coordinator.timeline();
    assert!(matches!(
        coordinator.rewind_to(genesis, 5),
        Err(CoordinatorError::BlockedByExternalBarrier { target, .. }) if target == genesis
    ));
    assert_eq!(coordinator.timeline(), before);
    assert_eq!(
        coordinator.inspect_commit_simulation(genesis).unwrap().id,
        genesis
    );
}

fn ledger_fault_windows_are_atomic_and_recoverable<B: Backend>() {
    let execution = ExecutionId::from_u128(6);
    let lease = LeaseId::from_u128(1);
    let pending = || {
        let mut coordinator = coordinator(B::faulty(), checked_query_program(), 6);
        dispatch_pending(&mut coordinator);
        coordinator
    };
    let claimed = || {
        let mut coordinator = pending();
        coordinator.claim_pending_effect(lease, 2, 20).unwrap();
        coordinator
    };
    let completed = || {
        let mut coordinator = claimed();
        coordinator
            .complete_pending_effect(lease, Value::I64(11), 3)
            .unwrap();
        coordinator
    };
    assert_atomic(pending, faulty_store, |coordinator| {
        coordinator.claim_pending_effect(lease, 2, 20)
    });
    assert_atomic(claimed, faulty_store, |coordinator| {
        coordinator.complete_pending_effect(lease, Value::I64(11), 3)
    });
    assert_atomic(completed, faulty_store, |coordinator| {
        coordinator.recover_pending_effect(Default::default(), 5)
    });

    // A failed outcome write leaves the claim in place, and the same call succeeds once the
    // backend recovers.
    let mut coordinator = claimed();
    let fail_writes = FaultPlan::new().at(
        Trigger::Nth(Primitive::Apply, 1),
        Fault::Before(StorageError::Io("injected".to_owned())),
    );
    coordinator
        .store_mut()
        .backend_mut()
        .set_plan(fail_writes.clone());
    assert!(
        coordinator
            .complete_pending_effect(lease, Value::I64(11), 3)
            .is_err()
    );
    assert_eq!(
        coordinator.store().current_ledger_fence(execution).unwrap(),
        LedgerFence::zero()
    );
    assert!(matches!(
        effects(coordinator.store(), execution)[0].status,
        LedgerStatus::Claimed { .. }
    ));
    coordinator
        .store_mut()
        .backend_mut()
        .set_plan(FaultPlan::new());
    coordinator
        .complete_pending_effect(lease, Value::I64(11), 4)
        .unwrap();

    coordinator.store_mut().backend_mut().set_plan(fail_writes);
    assert!(
        coordinator
            .recover_pending_effect(Default::default(), 5)
            .is_err()
    );
    assert!(coordinator.state().pending_effect().is_some());
    coordinator
        .store_mut()
        .backend_mut()
        .set_plan(FaultPlan::new());
    assert!(
        coordinator
            .recover_pending_effect(Default::default(), 6)
            .unwrap()
            .is_some()
    );
}

fn faulty_store<B: StorageBackend>(
    coordinator: &mut SessionCoordinator<Store<FaultInjecting<B>>>,
) -> &mut Store<FaultInjecting<B>> {
    coordinator.store_mut()
}

fn scene_is_reconciled_from_one_committed_snapshot<B: Backend>() {
    let program = load_program(
        &encode_program_artifact(&scene_reconcile_v0().unwrap()),
        &Default::default(),
    )
    .unwrap();
    let mut coordinator = SessionCoordinator::create(
        B::store(),
        program,
        ExecutionId::from_u128(7),
        RefName::new("scene-snapshot").unwrap(),
        BranchId::from_u128(1),
        InitialRecordingMode::Standard,
        1,
    )
    .unwrap();
    let committed = coordinator
        .dispatch(
            CheckedRuntimeInput::start(InputId::from_u128(1)),
            Default::default(),
            2,
        )
        .unwrap();
    let reconcile = committed.reconcile_scene.clone().unwrap();
    assert!(!reconcile.target.layers().is_empty());
    assert_eq!(Some(&reconcile.target), committed.state.scene.as_ref());
    let restored = coordinator
        .inspect_commit_simulation(committed.commit)
        .unwrap();
    assert_eq!(restored.state.scene, Some(reconcile.target));
}

fn compensation_preserves_the_original_fact_and_links_a_new_effect<B: Backend>() {
    let execution = ExecutionId::from_u128(8);
    let coordinator = coordinator(B::store(), checked_query_program(), 8);
    let origin_commit = coordinator.timeline().cursor;
    let mut store = coordinator.into_store();
    let command = CapabilityId::new("host.command").unwrap();
    let compensation = CapabilityId::new("host.compensate").unwrap();
    let version = CapabilityVersion::new(1).unwrap();
    let original = EffectId::from_bytes([1; 32]);
    let compensator = EffectId::from_bytes([2; 32]);

    let original_response = {
        let mut complete = |effect: EffectId,
                            digest: EffectRequestDigest,
                            capability: CapabilityId,
                            rewind: RewindPolicy,
                            lease: LeaseId,
                            observed_at: u64| {
            store
                .claim_effect(EffectClaim {
                    execution,
                    effect,
                    request_digest: digest,
                    capability: capability.clone(),
                    capability_version: version,
                    origin_commit,
                    delivery: DeliveryPolicy::AtLeastOnceIdempotent,
                    rewind,
                    lease,
                    now: observed_at,
                    expires_at: observed_at + 10,
                })
                .unwrap();
            store
                .record_effect_outcome(EffectOutcomeRecord {
                    execution,
                    effect,
                    request_digest: digest,
                    lease,
                    outcome: EffectOutcome::Completed(RecordedEffectResponseV1 {
                        effect,
                        request_digest: digest,
                        capability,
                        capability_version: version,
                        payload: Value::Null,
                    }),
                    observed_at: observed_at + 1,
                })
                .unwrap()
        };
        let original_entry = complete(
            original,
            EffectRequestDigest::from_bytes([11; 32]),
            command,
            RewindPolicy::Compensatable {
                capability: compensation.clone(),
            },
            LeaseId::from_u128(1),
            2,
        );
        let original_response = original_entry.status.response().unwrap();
        complete(
            compensator,
            EffectRequestDigest::from_bytes([12; 32]),
            compensation,
            RewindPolicy::Reapply,
            LeaseId::from_u128(2),
            4,
        );
        original_response
    };
    let compensated = store
        .mark_effect_compensated(execution, original, compensator)
        .unwrap();
    assert!(matches!(
        compensated.status,
        LedgerStatus::Compensated {
            response,
            original_fence,
            by_effect,
            compensation_fence,
        } if response == original_response
            && original_fence == LedgerFence::from_u64(1)
            && by_effect == compensator
            && compensation_fence == LedgerFence::from_u64(2)
    ));
    assert_eq!(effects(&store, execution).len(), 2);
}

fn ledger_origin_and_response_remain_gc_roots_without_timeline_refs<B: Backend>() {
    let execution = ExecutionId::from_u128(9);
    let mut coordinator = coordinator(B::store(), checked_query_program(), 9);
    let effect = dispatch_pending(&mut coordinator);
    let origin = coordinator.timeline().cursor;
    let lease = LeaseId::from_u128(1);
    coordinator.claim_pending_effect(lease, 2, 20).unwrap();
    let completed = coordinator
        .complete_pending_effect(lease, Value::I64(11), 3)
        .unwrap();
    let response = completed.status.response().unwrap();
    let mut store = coordinator.into_store();
    let refs = refs(&store);
    store
        .commit(CommitTransaction {
            refs: refs
                .into_iter()
                .map(|(key, value)| RefMutation {
                    key,
                    expected: Some(value.revision),
                    next: None,
                })
                .collect(),
            observed_at: 4,
            ..CommitTransaction::default()
        })
        .unwrap();
    store
        .collect(RetentionPolicy {
            now: u64::MAX,
            grace_seconds: 0,
            dry_run: false,
        })
        .unwrap();
    assert!(
        store
            .get_object(narrata_core::ObjectId::from_bytes(*origin.as_bytes()))
            .unwrap()
            .is_some()
    );
    assert!(store.get_object(response).unwrap().is_some());
    assert!(store.read_effect(execution, effect).unwrap().is_some());
}

backend_tests!(
    required_capability_is_negotiated_before_execution,
    recorded_query_survives_crash_and_replay_without_redispatch,
    leases_expire_and_request_identity_cannot_be_rebound,
    unknown_outcome_stops_catchup_and_all_other_input,
    completed_barrier_blocks_rewind_before_cursor_mutation,
    ledger_fault_windows_are_atomic_and_recoverable,
    scene_is_reconciled_from_one_committed_snapshot,
    compensation_preserves_the_original_fact_and_links_a_new_effect,
    ledger_origin_and_response_remain_gc_roots_without_timeline_refs,
);
