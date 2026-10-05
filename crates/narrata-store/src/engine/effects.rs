//! Stage 1–5 registration of the shared effect ledger; ADR 0014 bytes stay unchanged.
use super::Store;
use crate::{
    EffectClaim, EffectClaimResult, EffectLedgerEntry, EffectOutcome, EffectOutcomeRecord, LeaseId,
    LedgerFence, LedgerStatus, RecordedEffectResponseV1, StoreError, layout, object::history_id,
};
use narrata_core::{
    CapabilityId, CapabilityVersion, EffectId, ExecutionId, RewindPolicy, codec::ObjectKind,
};
use narrata_history::{HistoryError, Object, effect as h};
use narrata_storage::{Revision, StorageBackend};

pub(super) struct LegacyEffects;
impl h::EffectRegistration for LegacyEffects {
    const EFFECTS: narrata_storage::KeySpace = layout::EFFECTS;
    const FENCES: narrata_storage::KeySpace = layout::LEDGER_FENCES;
    const COMMIT_KIND: u16 = ObjectKind::Commit.code();
    type Response = RecordedEffectResponseV1;
    fn encode(entry: &h::EffectLedgerEntry) -> Result<Vec<u8>, HistoryError> {
        Ok(layout::encode_effect(&from_history(entry.clone())?))
    }
    fn decode(key: &[u8], value: &[u8]) -> Result<h::EffectLedgerEntry, HistoryError> {
        Ok(to_history(layout::decode_effect(key, value)?))
    }
    fn response(
        entry: &h::EffectLedgerEntry,
        response: &Self::Response,
    ) -> Result<Object, h::EffectStoreError> {
        if response.effect.as_bytes() != entry.effect.as_bytes()
            || response.request_digest.as_bytes() != entry.request_digest.as_bytes()
            || response.capability.as_str().as_bytes() != entry.capability
            || response.capability_version.get().to_be_bytes().as_slice()
                != entry.capability_version
        {
            return Err(h::EffectStoreError::InvalidResponse(
                "contract mismatch".to_owned(),
            ));
        }
        Ok(response.to_object().into_object())
    }
}
fn capability(bytes: &[u8]) -> Result<CapabilityId, HistoryError> {
    std::str::from_utf8(bytes)
        .ok()
        .and_then(|s| CapabilityId::new(s).ok())
        .ok_or(HistoryError::InvalidGraph("legacy capability is invalid"))
}
fn rewind_to_history(rewind: RewindPolicy) -> h::RewindPolicy {
    match rewind {
        RewindPolicy::Reapply => h::RewindPolicy::Reapply,
        RewindPolicy::ReuseRecordedResponse => h::RewindPolicy::ReuseRecordedResponse,
        RewindPolicy::Barrier => h::RewindPolicy::Barrier,
        RewindPolicy::Compensatable { capability } => h::RewindPolicy::Compensatable {
            capability: capability.as_str().as_bytes().to_vec(),
        },
    }
}
fn rewind_from_history(rewind: h::RewindPolicy) -> Result<RewindPolicy, HistoryError> {
    Ok(match rewind {
        h::RewindPolicy::Reapply => RewindPolicy::Reapply,
        h::RewindPolicy::ReuseRecordedResponse => RewindPolicy::ReuseRecordedResponse,
        h::RewindPolicy::Barrier => RewindPolicy::Barrier,
        h::RewindPolicy::Compensatable { capability: bytes } => RewindPolicy::Compensatable {
            capability: capability(&bytes)?,
        },
    })
}
fn status_to_history(status: LedgerStatus) -> h::LedgerStatus {
    match status {
        LedgerStatus::Claimed { lease, expires_at } => h::LedgerStatus::Claimed {
            lease: h::LeaseId::from_bytes(*lease.as_bytes()),
            expires_at,
        },
        LedgerStatus::Completed { response, fence } => h::LedgerStatus::Completed {
            response: history_id(response.as_bytes()),
            fence,
        },
        LedgerStatus::Rejected { response, fence } => h::LedgerStatus::Rejected {
            response: history_id(response.as_bytes()),
            fence,
        },
        LedgerStatus::RetryableFailure { diagnostic } => h::LedgerStatus::RetryableFailure {
            diagnostic: h::DiagnosticId::from_bytes(*diagnostic.as_bytes()),
        },
        LedgerStatus::UnknownOutcome { diagnostic, fence } => h::LedgerStatus::UnknownOutcome {
            diagnostic: h::DiagnosticId::from_bytes(*diagnostic.as_bytes()),
            fence,
        },
        LedgerStatus::Compensated {
            response,
            original_fence,
            by_effect,
            compensation_fence,
        } => h::LedgerStatus::Compensated {
            response: history_id(response.as_bytes()),
            original_fence,
            by_effect: h::EffectId::from_bytes(*by_effect.as_bytes()),
            compensation_fence,
        },
    }
}
fn status_from_history(status: h::LedgerStatus) -> LedgerStatus {
    match status {
        h::LedgerStatus::Claimed { lease, expires_at } => LedgerStatus::Claimed {
            lease: crate::LeaseId::from_bytes(*lease.as_bytes()),
            expires_at,
        },
        h::LedgerStatus::Completed { response, fence } => LedgerStatus::Completed {
            response: narrata_core::ObjectId::from_bytes(*response.as_bytes()),
            fence,
        },
        h::LedgerStatus::Rejected { response, fence } => LedgerStatus::Rejected {
            response: narrata_core::ObjectId::from_bytes(*response.as_bytes()),
            fence,
        },
        h::LedgerStatus::RetryableFailure { diagnostic } => LedgerStatus::RetryableFailure {
            diagnostic: narrata_core::DiagnosticId::from_bytes(*diagnostic.as_bytes()),
        },
        h::LedgerStatus::UnknownOutcome { diagnostic, fence } => LedgerStatus::UnknownOutcome {
            diagnostic: narrata_core::DiagnosticId::from_bytes(*diagnostic.as_bytes()),
            fence,
        },
        h::LedgerStatus::Compensated {
            response,
            original_fence,
            by_effect,
            compensation_fence,
        } => LedgerStatus::Compensated {
            response: narrata_core::ObjectId::from_bytes(*response.as_bytes()),
            original_fence,
            by_effect: narrata_core::EffectId::from_bytes(*by_effect.as_bytes()),
            compensation_fence,
        },
    }
}
fn delivery_to_history(value: narrata_core::DeliveryPolicy) -> h::DeliveryPolicy {
    match value {
        narrata_core::DeliveryPolicy::Reconcile => h::DeliveryPolicy::Reconcile,
        narrata_core::DeliveryPolicy::RecordedQuery => h::DeliveryPolicy::RecordedQuery,
        narrata_core::DeliveryPolicy::AtLeastOnceIdempotent => {
            h::DeliveryPolicy::AtLeastOnceIdempotent
        }
        narrata_core::DeliveryPolicy::HostTransactional => h::DeliveryPolicy::HostTransactional,
    }
}
fn delivery_from_history(value: h::DeliveryPolicy) -> narrata_core::DeliveryPolicy {
    match value {
        h::DeliveryPolicy::Reconcile => narrata_core::DeliveryPolicy::Reconcile,
        h::DeliveryPolicy::RecordedQuery => narrata_core::DeliveryPolicy::RecordedQuery,
        h::DeliveryPolicy::AtLeastOnceIdempotent => {
            narrata_core::DeliveryPolicy::AtLeastOnceIdempotent
        }
        h::DeliveryPolicy::HostTransactional => narrata_core::DeliveryPolicy::HostTransactional,
    }
}

fn to_history(entry: EffectLedgerEntry) -> h::EffectLedgerEntry {
    h::EffectLedgerEntry {
        execution: h::ExecutionId::from_bytes(*entry.execution.as_bytes()),
        effect: h::EffectId::from_bytes(*entry.effect.as_bytes()),
        request_digest: h::EffectRequestDigest::from_bytes(*entry.request_digest.as_bytes()),
        capability: entry.capability.as_str().as_bytes().to_vec(),
        capability_version: entry.capability_version.get().to_be_bytes().to_vec(),
        origin_commit: history_id(entry.origin_commit.as_bytes()),
        delivery: delivery_to_history(entry.delivery),
        rewind: rewind_to_history(entry.rewind),
        status: status_to_history(entry.status),
    }
}
fn from_history(entry: h::EffectLedgerEntry) -> Result<EffectLedgerEntry, HistoryError> {
    Ok(EffectLedgerEntry {
        execution: ExecutionId::from_bytes(*entry.execution.as_bytes()),
        effect: EffectId::from_bytes(*entry.effect.as_bytes()),
        request_digest: narrata_core::EffectRequestDigest::from_bytes(
            *entry.request_digest.as_bytes(),
        ),
        capability: capability(&entry.capability)?,
        capability_version: CapabilityVersion::new(u16::from_be_bytes(
            narrata_history::layout::fixed("legacy capability version", &entry.capability_version)?,
        ))
        .ok_or(HistoryError::InvalidGraph(
            "legacy capability version is zero",
        ))?,
        origin_commit: narrata_core::CommitId::from_bytes(*entry.origin_commit.as_bytes()),
        delivery: delivery_from_history(entry.delivery),
        rewind: rewind_from_history(entry.rewind)?,
        status: status_from_history(entry.status),
    })
}
impl<B: StorageBackend> Store<B> {
    pub(super) fn read_entry(
        &self,
        execution: ExecutionId,
        effect: EffectId,
    ) -> Result<Option<(EffectLedgerEntry, Revision)>, StoreError> {
        self.history
            .read_effect_entry::<LegacyEffects>(
                h::ExecutionId::from_bytes(*execution.as_bytes()),
                h::EffectId::from_bytes(*effect.as_bytes()),
            )?
            .map(|(entry, revision)| Ok((from_history(entry)?, revision)))
            .transpose()
    }
    pub(super) fn read_fence(
        &self,
        execution: ExecutionId,
    ) -> Result<Option<(LedgerFence, Revision)>, StoreError> {
        self.history
            .read_effect_fence::<LegacyEffects>(h::ExecutionId::from_bytes(*execution.as_bytes()))
    }
    pub(super) fn claim(&mut self, claim: EffectClaim) -> Result<EffectClaimResult, StoreError> {
        let result = self.history.claim_effect::<LegacyEffects>(h::EffectClaim {
            execution: h::ExecutionId::from_bytes(*claim.execution.as_bytes()),
            effect: h::EffectId::from_bytes(*claim.effect.as_bytes()),
            request_digest: h::EffectRequestDigest::from_bytes(*claim.request_digest.as_bytes()),
            capability: claim.capability.as_str().as_bytes().to_vec(),
            capability_version: claim.capability_version.get().to_be_bytes().to_vec(),
            origin_commit: history_id(claim.origin_commit.as_bytes()),
            delivery: delivery_to_history(claim.delivery),
            rewind: rewind_to_history(claim.rewind),
            lease: h::LeaseId::from_bytes(*claim.lease.as_bytes()),
            now: claim.now,
            expires_at: claim.expires_at,
        })?;
        Ok(match result {
            h::EffectClaimResult::Claimed(entry) => {
                EffectClaimResult::Claimed(from_history(entry)?)
            }
            h::EffectClaimResult::Recorded(entry) => {
                EffectClaimResult::Recorded(from_history(entry)?)
            }
            h::EffectClaimResult::Leased { lease, expires_at } => EffectClaimResult::Leased {
                lease: LeaseId::from_bytes(*lease.as_bytes()),
                expires_at,
            },
        })
    }
    pub(super) fn renew(
        &mut self,
        execution: ExecutionId,
        effect: EffectId,
        lease: LeaseId,
        now: u64,
        expires_at: u64,
    ) -> Result<EffectLedgerEntry, StoreError> {
        Ok(from_history(
            self.history.renew_effect_lease::<LegacyEffects>(
                h::ExecutionId::from_bytes(*execution.as_bytes()),
                h::EffectId::from_bytes(*effect.as_bytes()),
                h::LeaseId::from_bytes(*lease.as_bytes()),
                now,
                expires_at,
            )?,
        )?)
    }
    pub(super) fn record_outcome(
        &mut self,
        record: EffectOutcomeRecord,
    ) -> Result<EffectLedgerEntry, StoreError> {
        let outcome = match record.outcome {
            EffectOutcome::Completed(response) => h::EffectOutcome::Completed(response),
            EffectOutcome::Rejected(response) => h::EffectOutcome::Rejected(response),
            EffectOutcome::RetryableFailure(diagnostic) => h::EffectOutcome::RetryableFailure(
                h::DiagnosticId::from_bytes(*diagnostic.as_bytes()),
            ),
            EffectOutcome::UnknownOutcome(diagnostic) => h::EffectOutcome::UnknownOutcome(
                h::DiagnosticId::from_bytes(*diagnostic.as_bytes()),
            ),
        };
        Ok(from_history(
            self.history
                .record_effect_outcome::<LegacyEffects>(h::EffectOutcomeRecord {
                    execution: h::ExecutionId::from_bytes(*record.execution.as_bytes()),
                    effect: h::EffectId::from_bytes(*record.effect.as_bytes()),
                    request_digest: h::EffectRequestDigest::from_bytes(
                        *record.request_digest.as_bytes(),
                    ),
                    lease: h::LeaseId::from_bytes(*record.lease.as_bytes()),
                    outcome,
                    observed_at: record.observed_at,
                })?,
        )?)
    }
    pub(super) fn compensate(
        &mut self,
        execution: ExecutionId,
        original: EffectId,
        by_effect: EffectId,
    ) -> Result<EffectLedgerEntry, StoreError> {
        Ok(from_history(
            self.history.mark_effect_compensated::<LegacyEffects>(
                h::ExecutionId::from_bytes(*execution.as_bytes()),
                h::EffectId::from_bytes(*original.as_bytes()),
                h::EffectId::from_bytes(*by_effect.as_bytes()),
            )?,
        )?)
    }
}
