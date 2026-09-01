#![allow(clippy::panic, clippy::unwrap_used)]

use std::{collections::BTreeMap, sync::Arc};

use narrata_core::{
    ExecutionId, InputId, builtin_host_capabilities,
    program::{encode_program_artifact, load_program},
    runtime::CheckedRuntimeInput,
};
use narrata_store::{
    BranchId, CommitTransaction, EffectClaim, EffectClaimResult, InitialRecordingMode, LeaseId,
    MemoryStore, RefKey, RefMutation, RefName, RefRevision, SaveStore, SessionCoordinator,
    StoreError,
};
use narrata_testkit::generator::{branch_call_choice_v0, recorded_query_v0};
use proptest::prelude::*;

proptest! {
    #[test]
    fn ref_command_sequences_match_an_independent_model(commands in prop::collection::vec((0_u8..4, 0_u8..8), 0..200)) {
        let program = load_program(
            &encode_program_artifact(&branch_call_choice_v0()),
            &Default::default(),
        ).unwrap();
        let mut coordinator = SessionCoordinator::create(
            MemoryStore::new(),
            Arc::clone(&program),
            ExecutionId::from_u128(1),
            RefName::new("session").unwrap(),
            BranchId::from_u128(1),
            InitialRecordingMode::Standard,
            1,
        ).unwrap();
        let commit = coordinator.dispatch(
            CheckedRuntimeInput::start(InputId::from_u128(1)),
            Default::default(),
            2,
        ).unwrap().commit;
        let mut store = coordinator.into_store();
        let owner = RefName::new("model").unwrap();
        let mut model = BTreeMap::<u8, u64>::new();

        for (operation, slot_number) in commands {
            let key = RefKey::save(
                owner.clone(),
                RefName::new(format!("slot-{slot_number}")).unwrap(),
            );
            let current = model.get(&slot_number).copied();
            let transaction = match operation {
                0 => CommitTransaction {
                    refs: vec![RefMutation {
                        key: key.clone(),
                        expected: current.and_then(RefRevision::from_u64),
                        next: Some(commit),
                    }],
                    ..CommitTransaction::default()
                },
                1 => CommitTransaction {
                    refs: vec![RefMutation {
                        key: key.clone(),
                        expected: None,
                        next: Some(commit),
                    }],
                    ..CommitTransaction::default()
                },
                2 => CommitTransaction {
                    refs: vec![RefMutation {
                        key: key.clone(),
                        expected: current.and_then(RefRevision::from_u64),
                        next: None,
                    }],
                    ..CommitTransaction::default()
                },
                _ => CommitTransaction {
                    refs: vec![RefMutation {
                        key: key.clone(),
                        expected: RefRevision::from_u64(current.unwrap_or(1).saturating_add(17)),
                        next: Some(commit),
                    }],
                    ..CommitTransaction::default()
                },
            };
            let result = store.commit(transaction);
            match operation {
                0 => {
                    let next = current.unwrap_or(0).saturating_add(1);
                    model.insert(slot_number, next);
                    prop_assert!(result.is_ok());
                }
                1 if current.is_none() => {
                    model.insert(slot_number, 1);
                    prop_assert!(result.is_ok());
                }
                2 if current.is_some() => {
                    model.remove(&slot_number);
                    prop_assert!(result.is_ok());
                }
                2 => prop_assert!(result.is_ok()),
                _ => prop_assert!(matches!(result, Err(StoreError::RefConflict(_)))),
            }
            let actual = store.read_ref(&key).unwrap().map(|value| value.revision.get());
            prop_assert_eq!(actual, model.get(&slot_number).copied());
        }
    }

    #[test]
    fn lease_claims_match_expiry_model(
        commands in prop::collection::vec((0_u8..4, 0_u64..100, 1_u64..20), 1..200)
    ) {
        let execution = ExecutionId::from_u128(90);
        let program = load_program(
            &encode_program_artifact(&recorded_query_v0().unwrap()),
            &Default::default(),
        ).unwrap();
        let mut coordinator = SessionCoordinator::create_with_capabilities(
            MemoryStore::new(),
            program,
            execution,
            RefName::new("lease-model").unwrap(),
            BranchId::from_u128(1),
            InitialRecordingMode::Standard,
            1,
            &builtin_host_capabilities().unwrap(),
        ).unwrap();
        coordinator.dispatch(
            CheckedRuntimeInput::start(InputId::from_u128(1)),
            Default::default(),
            2,
        ).unwrap();
        let request = coordinator.state().pending_effect().unwrap().request.clone();
        let origin_commit = coordinator.timeline().cursor;
        let mut store = coordinator.into_store();
        let mut model: Option<(LeaseId, u64)> = None;

        for (claimant, now, ttl) in commands {
            let lease = LeaseId::from_u128(u128::from(claimant) + 1);
            let expires_at = now + ttl;
            let result = store.claim_effect(EffectClaim {
                execution,
                effect: request.id,
                request_digest: request.request_digest,
                capability: request.capability.clone(),
                capability_version: request.capability_version,
                origin_commit,
                delivery: request.delivery,
                rewind: request.rewind.clone(),
                lease,
                now,
                expires_at,
            }).unwrap();
            match model {
                Some((owner, expiry)) if owner != lease && expiry > now => {
                    prop_assert_eq!(
                        result,
                        EffectClaimResult::Leased {
                            lease: owner,
                            expires_at: expiry,
                        }
                    );
                }
                _ => {
                    prop_assert!(matches!(result, EffectClaimResult::Claimed(_)));
                    model = Some((lease, expires_at));
                }
            }
        }
    }
}
