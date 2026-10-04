#![allow(clippy::panic, clippy::unwrap_used)]

#[macro_use]
mod support;

use std::sync::Arc;

use narrata_core::{
    CapabilityId, ContentLockId, ExecutionId, HostSnapshotDigest,
    program::{encode_program_artifact, load_program},
};
use narrata_store::{
    BranchId, CompoundSaveRefKey, CoordinatorError, FederatedRestoreAvailability,
    FederatedSaveVerifier, HostSnapshotRef, HostSnapshotVerifier, HostTimelineEntry,
    HostTimelineManifestV1, InitialRecordingMode, LedgerFence, RefName, SaveStore,
    SessionCoordinator, Store, TimelineCoverage,
};
use narrata_testkit::generator::hello_v0;
use support::Backend;

struct ExactVerifier {
    digest: HostSnapshotDigest,
    format_version: u16,
    content_lock: ContentLockId,
}

impl HostSnapshotVerifier for ExactVerifier {
    fn verify(&self, snapshot: &HostSnapshotRef) -> Result<(), String> {
        if snapshot.digest() != self.digest {
            return Err("host snapshot digest mismatch".to_owned());
        }
        if snapshot.format_version() != self.format_version {
            return Err("unsupported host snapshot format".to_owned());
        }
        Ok(())
    }
}

impl FederatedSaveVerifier for ExactVerifier {
    fn verify_content_lock(&self, content_lock: ContentLockId) -> Result<(), String> {
        if content_lock == self.content_lock {
            Ok(())
        } else {
            Err("content lock is unavailable".to_owned())
        }
    }
}

fn snapshot(reference: &str, digest: u8) -> HostSnapshotRef {
    HostSnapshotRef::new(
        CapabilityId::new("host.snapshot").unwrap(),
        reference,
        1,
        HostSnapshotDigest::from_bytes([digest; 32]),
    )
    .unwrap()
}

fn coordinator<B: Backend>() -> SessionCoordinator<Store<B::Inner>> {
    let program = load_program(&encode_program_artifact(&hello_v0()), &Default::default()).unwrap();
    SessionCoordinator::create(
        B::store(),
        Arc::clone(&program),
        ExecutionId::from_u128(20),
        RefName::new("federated-session").unwrap(),
        BranchId::from_u128(1),
        InitialRecordingMode::Standard,
        1,
    )
    .unwrap()
}

fn compound_save_validates_host_before_atomic_ref_publish<B: Backend>() {
    let mut coordinator = coordinator::<B>();
    let owner = RefName::new("player").unwrap();
    let slot = RefName::new("slot").unwrap();
    let key = CompoundSaveRefKey::new(owner.clone(), slot.clone());
    let host = snapshot("snapshot-1", 1);
    let wrong = ExactVerifier {
        digest: HostSnapshotDigest::from_bytes([2; 32]),
        format_version: 1,
        content_lock: ContentLockId::from_bytes([3; 32]),
    };
    assert!(matches!(
        coordinator.save_compound(
            owner.clone(),
            slot.clone(),
            host.clone(),
            ContentLockId::from_bytes([3; 32]),
            None,
            &wrong,
            2,
        ),
        Err(CoordinatorError::HostSnapshot(_))
    ));
    assert!(
        coordinator
            .store()
            .read_compound_save(&key)
            .unwrap()
            .is_none()
    );

    let wrong_lock = ExactVerifier {
        digest: host.digest(),
        format_version: 1,
        content_lock: ContentLockId::from_bytes([99; 32]),
    };
    assert!(matches!(
        coordinator.save_compound(
            owner.clone(),
            slot.clone(),
            host.clone(),
            ContentLockId::from_bytes([3; 32]),
            None,
            &wrong_lock,
            2,
        ),
        Err(CoordinatorError::ContentLock(_))
    ));
    assert!(
        coordinator
            .store()
            .read_compound_save(&key)
            .unwrap()
            .is_none()
    );

    let verifier = ExactVerifier {
        digest: host.digest(),
        format_version: 1,
        content_lock: ContentLockId::from_bytes([3; 32]),
    };
    let saved = coordinator
        .save_compound(
            owner.clone(),
            slot.clone(),
            host.clone(),
            ContentLockId::from_bytes([3; 32]),
            None,
            &verifier,
            3,
        )
        .unwrap();
    let validated = coordinator
        .validate_compound_save(owner.clone(), slot.clone(), &verifier)
        .unwrap();
    assert_eq!(validated.host_snapshot(), &host);
    assert_eq!(validated.narrative(), coordinator.timeline().cursor);

    let replacement = snapshot("snapshot-2", 4);
    let replacement_verifier = ExactVerifier {
        digest: replacement.digest(),
        format_version: 1,
        content_lock: ContentLockId::from_bytes([5; 32]),
    };
    coordinator
        .save_compound(
            owner,
            slot,
            replacement,
            ContentLockId::from_bytes([5; 32]),
            Some(saved.revision),
            &replacement_verifier,
            4,
        )
        .unwrap();
    let before = coordinator.timeline();
    assert!(matches!(
        coordinator.activate_compound_save(validated, 5),
        Err(CoordinatorError::CompoundSaveChanged)
    ));
    assert_eq!(coordinator.timeline(), before);
}

fn host_timeline_only_advertises_verified_joint_restore_points<B: Backend>() {
    let coordinator = coordinator::<B>();
    let baseline = coordinator.timeline().cursor;
    let host = snapshot("baseline", 8);
    let manifest = HostTimelineManifestV1::checked(
        ExecutionId::from_u128(20),
        TimelineCoverage::FromBaseline { baseline },
        vec![HostTimelineEntry {
            narrative: baseline,
            host: host.clone(),
            ledger_fence: LedgerFence::zero(),
        }],
    )
    .unwrap();
    let verifier = ExactVerifier {
        digest: host.digest(),
        format_version: 1,
        content_lock: ContentLockId::from_bytes([0; 32]),
    };
    coordinator
        .validate_host_timeline(&manifest, &verifier)
        .unwrap();
    assert_eq!(
        coordinator.federated_restore_availability(&manifest, baseline),
        FederatedRestoreAvailability::Joint(host)
    );
    assert_eq!(
        coordinator.federated_restore_availability(
            &manifest,
            narrata_core::CommitId::from_bytes([99; 32])
        ),
        FederatedRestoreAvailability::NarrativeOnly
    );
}

backend_tests!(
    compound_save_validates_host_before_atomic_ref_publish,
    host_timeline_only_advertises_verified_joint_restore_points,
);
