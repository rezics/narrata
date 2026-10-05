//! Deterministic counter evidence for the shared effect and migration formats (ADR 0020).
#![allow(dead_code, clippy::unwrap_used)]

use narrata_history::{
    Domain, ObjectId, RefKey, Session,
    effect::{
        DeliveryPolicy, DiagnosticId, DomainEffects, EffectClaim, EffectId, EffectOutcome,
        EffectOutcomeRecord, EffectRequestDigest, ExecutionId, LeaseId, RecordedEffectResponse,
        RewindPolicy,
    },
    migration::{ArtifactRegistry, MigrationId, MigrationRegistry, MigrationRequest},
    testing::Counter,
};
use narrata_storage::StorageBackend;

#[path = "../corpus/mod.rs"]
mod base;
pub use base::{Counters, image, name, step};
pub fn fixture(name: &str) -> std::path::PathBuf {
    base::fixture(name)
}

pub const SOURCE: Counter = Counter::new(1_000);
pub const TARGET: Counter = Counter::new(2_000);
pub type Effects = DomainEffects<Counter>;

pub fn claim(origin: ObjectId, n: u8) -> EffectClaim {
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
        expires_at: 100,
    }
}
pub fn response(claim: &EffectClaim) -> RecordedEffectResponse {
    RecordedEffectResponse {
        effect: claim.effect,
        request_digest: claim.request_digest,
        capability: claim.capability.clone(),
        capability_version: claim.capability_version.clone(),
        payload: vec![0xa0],
    }
}
pub fn outcome<B: StorageBackend>(
    history: &mut Counters<B>,
    claim: &EffectClaim,
    outcome: EffectOutcome,
) {
    history
        .record_effect_outcome::<Effects>(EffectOutcomeRecord {
            execution: claim.execution,
            effect: claim.effect,
            request_digest: claim.request_digest,
            lease: claim.lease,
            outcome,
            observed_at: 3,
        })
        .unwrap();
}
pub struct Recorded {
    pub source: ObjectId,
    pub migrated: ObjectId,
    pub continued: ObjectId,
}
pub fn record<B: StorageBackend>(history: &mut Counters<B>) -> Recorded {
    let (_, root) = Session::create(history, SOURCE, name("source"), &1, 1).unwrap();
    let diagnostic = DiagnosticId::from_bytes([9; 32]);
    for n in 1..=6 {
        let mut c = claim(root.commit, n);
        if n == 1 {
            c.rewind = RewindPolicy::Compensatable {
                capability: b"undo".to_vec(),
            };
        }
        if n == 2 {
            c.capability = b"undo".to_vec();
        }
        if n == 4 {
            c.rewind = RewindPolicy::Barrier;
        }
        history.claim_effect::<Effects>(c.clone()).unwrap();
        match n {
            1 | 2 => outcome(history, &c, EffectOutcome::Completed(response(&c))),
            3 => outcome(history, &c, EffectOutcome::Rejected(response(&c))),
            4 => outcome(history, &c, EffectOutcome::UnknownOutcome(diagnostic)),
            5 => outcome(history, &c, EffectOutcome::RetryableFailure(diagnostic)),
            _ => {
                history
                    .renew_effect_lease::<Effects>(c.execution, c.effect, c.lease, 3, 200)
                    .unwrap();
            }
        }
    }
    history
        .mark_effect_compensated::<Effects>(
            ExecutionId::from_u128(1),
            EffectId::from_bytes([1; 32]),
            EffectId::from_bytes([2; 32]),
        )
        .unwrap();
    let mut artifacts = ArtifactRegistry::new();
    artifacts.register(SOURCE.artifact_id(), SOURCE);
    artifacts.register(TARGET.artifact_id(), TARGET);
    let mut migrations = MigrationRegistry::new();
    migrations
        .register(
            MigrationId::from_bytes([1; 32]),
            SOURCE.artifact_id(),
            TARGET.artifact_id(),
            (),
        )
        .unwrap();
    let request = MigrationRequest {
        source: root.commit,
        target: TARGET.artifact_id(),
        explicit_path: None,
        target_ref: RefKey::active(name("target")).unwrap(),
        expected_ref: None,
        observed_at: 4,
    };
    let migrated = history
        .apply_migration(&artifacts, &migrations, &request, |_, _, _, state| {
            Ok::<_, std::convert::Infallible>((state + 10, ()))
        })
        .unwrap();
    let (mut session, _) = Session::open(history, TARGET, name("target")).unwrap();
    let continued = session
        .advance(history, migrated.commit, &1, step, 5)
        .unwrap();
    Recorded {
        source: root.commit,
        migrated: migrated.commit,
        continued: continued.commit,
    }
}
