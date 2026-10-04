#![allow(clippy::panic, clippy::unwrap_used)]

#[macro_use]
mod support;

use std::collections::{BTreeMap, BTreeSet};

use narrata_core::{ExecutionId, InputId, builtin_host_capabilities, runtime::CheckedRuntimeInput};
use narrata_store::{
    BranchId, CommitTransaction, EffectClaim, EffectClaimResult, InitialRecordingMode, LeaseId,
    RefKey, RefMutation, RefName, RefRevision, SaveStore, SessionCoordinator, StoreError,
};
use narrata_testkit::generator::{branch_call_choice_v0, recorded_query_v0};
use proptest::{prelude::*, test_runner::TestCaseError};
use support::{Backend, Memory, Sqlite, program};

/// A revision no store issues within a test.
fn never_issued() -> Option<RefRevision> {
    RefRevision::from_u64(u64::MAX)
}

/// Ref commands against a model that only knows which revision each slot was last given:
/// 0 compares and sets, 1 creates, 2 compares and deletes, 3 sets against a stale revision.
fn ref_commands_match_model<B: Backend>(commands: &[(u8, u8)]) -> Result<(), TestCaseError> {
    let mut coordinator = SessionCoordinator::create(
        B::store(),
        program(&branch_call_choice_v0()),
        ExecutionId::from_u128(1),
        RefName::new("session").unwrap(),
        BranchId::from_u128(1),
        InitialRecordingMode::Standard,
        1,
    )
    .unwrap();
    let commit = coordinator
        .dispatch(
            CheckedRuntimeInput::start(InputId::from_u128(1)),
            Default::default(),
            2,
        )
        .unwrap()
        .commit;
    let mut store = coordinator.into_store();
    let owner = RefName::new("model").unwrap();
    let mut model = BTreeMap::<u8, RefRevision>::new();
    let mut issued = BTreeSet::<RefRevision>::new();

    for &(operation, slot_number) in commands {
        let key = RefKey::save(
            owner.clone(),
            RefName::new(format!("slot-{slot_number}")).unwrap(),
        );
        let current = model.get(&slot_number).copied();
        let (expected, next) = match operation {
            0 => (current, Some(commit)),
            1 => (None, Some(commit)),
            2 => (current, None),
            _ => (never_issued(), Some(commit)),
        };
        let result = store.commit(CommitTransaction {
            refs: vec![RefMutation {
                key: key.clone(),
                expected,
                next,
            }],
            ..CommitTransaction::default()
        });
        match (operation, current) {
            (0, _) | (1, None) => {
                let revision = result.unwrap().refs[&key].unwrap().revision;
                prop_assert!(
                    issued.insert(revision),
                    "revision {} was reused",
                    revision.get()
                );
                model.insert(slot_number, revision);
            }
            (2, _) => {
                prop_assert_eq!(result.unwrap().refs[&key], None);
                model.remove(&slot_number);
            }
            _ => prop_assert!(matches!(result, Err(StoreError::RefConflict(_)))),
        }
        let actual = store.read_ref(&key).unwrap().map(|value| value.revision);
        prop_assert_eq!(actual, model.get(&slot_number).copied());
    }
    Ok(())
}

fn lease_claims_match_model<B: Backend>(commands: &[(u8, u64, u64)]) -> Result<(), TestCaseError> {
    let execution = ExecutionId::from_u128(90);
    let mut coordinator = SessionCoordinator::create_with_capabilities(
        B::store(),
        program(&recorded_query_v0().unwrap()),
        execution,
        RefName::new("lease-model").unwrap(),
        BranchId::from_u128(1),
        InitialRecordingMode::Standard,
        1,
        &builtin_host_capabilities().unwrap(),
    )
    .unwrap();
    coordinator
        .dispatch(
            CheckedRuntimeInput::start(InputId::from_u128(1)),
            Default::default(),
            2,
        )
        .unwrap();
    let request = coordinator
        .state()
        .pending_effect()
        .unwrap()
        .request
        .clone();
    let origin_commit = coordinator.timeline().cursor;
    let mut store = coordinator.into_store();
    let mut model: Option<(LeaseId, u64)> = None;

    for &(claimant, now, ttl) in commands {
        let lease = LeaseId::from_u128(u128::from(claimant) + 1);
        let expires_at = now + ttl;
        let result = store
            .claim_effect(EffectClaim {
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
            })
            .unwrap();
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
    Ok(())
}

fn ref_commands() -> impl Strategy<Value = Vec<(u8, u8)>> {
    prop::collection::vec((0_u8..4, 0_u8..8), 0..200)
}

fn lease_commands() -> impl Strategy<Value = Vec<(u8, u64, u64)>> {
    prop::collection::vec((0_u8..4, 0_u64..100, 1_u64..20), 1..200)
}

proptest! {
    #[test]
    fn memory_ref_commands_match_model(commands in ref_commands()) {
        ref_commands_match_model::<Memory>(&commands)?;
    }

    #[test]
    fn sqlite_ref_commands_match_model(commands in ref_commands()) {
        ref_commands_match_model::<Sqlite>(&commands)?;
    }

    #[test]
    fn memory_lease_claims_match_model(commands in lease_commands()) {
        lease_claims_match_model::<Memory>(&commands)?;
    }

    #[test]
    fn sqlite_lease_claims_match_model(commands in lease_commands()) {
        lease_claims_match_model::<Sqlite>(&commands)?;
    }
}
