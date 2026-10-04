//! The Effect ledger: each operation is one batch conditioned on the ledger entry's revision.

use narrata_core::{EffectId, ExecutionId, ObjectId, RewindPolicy, codec::ObjectKind};
use narrata_storage::{Expect, Revision, StorageBackend};

use super::{
    RETRIES, Store,
    write::{Attempt, Op, Ops, Tag, graph_bump, sweep_check},
};
use crate::{
    EffectClaim, EffectClaimResult, EffectLedgerEntry, EffectOutcome, EffectOutcomeRecord,
    EffectStoreError, LeaseId, LedgerFence, LedgerStatus, RecordedEffectResponseV1, StoreError,
    layout,
};

fn same_claim_contract(entry: &EffectLedgerEntry, claim: &EffectClaim) -> bool {
    entry.request_digest == claim.request_digest
        && entry.capability == claim.capability
        && entry.capability_version == claim.capability_version
        && entry.origin_commit == claim.origin_commit
        && entry.delivery == claim.delivery
        && entry.rewind == claim.rewind
}

fn entry_expect(revision: Option<Revision>) -> Expect {
    revision.map_or(Expect::Absent, Expect::Revision)
}

/// Runs `attempt` until it neither succeeds nor fails but asks to read again.
fn retry<T>(mut attempt: impl FnMut() -> Result<T, Attempt>) -> Result<T, StoreError> {
    for _ in 0..RETRIES {
        match attempt() {
            Ok(value) => return Ok(value),
            Err(Attempt::Retry) => {}
            Err(Attempt::Fail(error)) => return Err(error),
        }
    }
    Err(StoreError::Busy)
}

impl<B: StorageBackend> Store<B> {
    pub(super) fn read_entry(
        &self,
        execution: ExecutionId,
        effect: EffectId,
    ) -> Result<Option<(EffectLedgerEntry, Revision)>, StoreError> {
        let key = layout::effect_key(execution, effect);
        self.read_key(layout::EFFECTS, &key)?
            .map(|value| Ok((layout::decode_effect(&key, &value.value)?, value.revision)))
            .transpose()
    }

    pub(super) fn read_fence(
        &self,
        execution: ExecutionId,
    ) -> Result<Option<(LedgerFence, Revision)>, StoreError> {
        self.read_key(layout::LEDGER_FENCES, &layout::fence_key(execution))?
            .map(|value| Ok((layout::decode_fence(&value.value)?, value.revision)))
            .transpose()
    }

    fn put_entry(ops: &mut Ops, entry: &EffectLedgerEntry, revision: Option<Revision>) {
        ops.key(Op::put(
            layout::EFFECTS,
            layout::effect_key(entry.execution, entry.effect),
            layout::encode_effect(entry),
            entry_expect(revision),
            Tag::Replan,
        ));
    }

    pub(super) fn claim(&mut self, claim: EffectClaim) -> Result<EffectClaimResult, StoreError> {
        if claim.expires_at <= claim.now {
            return Err(EffectStoreError::InvalidLease.into());
        }
        let origin = ObjectId::from_bytes(*claim.origin_commit.as_bytes());
        retry(|| {
            let sweep = self.sweep()?;
            self.require_stored(origin, ObjectKind::Commit)?;
            let existing = self.read_entry(claim.execution, claim.effect)?;
            if let Some((entry, _)) = &existing {
                if !same_claim_contract(entry, &claim) {
                    return Err(StoreError::from(EffectStoreError::RequestConflict).into());
                }
                match entry.status {
                    LedgerStatus::Claimed { lease, expires_at }
                        if lease != claim.lease && expires_at > claim.now =>
                    {
                        return Ok(EffectClaimResult::Leased { lease, expires_at });
                    }
                    LedgerStatus::Claimed { .. } | LedgerStatus::RetryableFailure { .. } => {}
                    LedgerStatus::Completed { .. }
                    | LedgerStatus::Rejected { .. }
                    | LedgerStatus::UnknownOutcome { .. }
                    | LedgerStatus::Compensated { .. } => {
                        return Ok(EffectClaimResult::Recorded(entry.clone()));
                    }
                }
            }
            let entry = EffectLedgerEntry {
                execution: claim.execution,
                effect: claim.effect,
                request_digest: claim.request_digest,
                capability: claim.capability.clone(),
                capability_version: claim.capability_version,
                origin_commit: claim.origin_commit,
                delivery: claim.delivery,
                rewind: claim.rewind.clone(),
                status: LedgerStatus::Claimed {
                    lease: claim.lease,
                    expires_at: claim.expires_at,
                },
            };
            let mut ops = Ops::new();
            ops.key(sweep_check(sweep));
            Self::put_entry(&mut ops, &entry, existing.map(|(_, revision)| revision));
            ops.key(graph_bump());
            self.apply_ops(ops)?;
            Ok(EffectClaimResult::Claimed(entry))
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
        if expires_at <= now {
            return Err(EffectStoreError::InvalidLease.into());
        }
        retry(|| {
            let (mut entry, revision) = self
                .read_entry(execution, effect)?
                .ok_or(StoreError::from(EffectStoreError::Missing))?;
            match entry.status {
                LedgerStatus::Claimed {
                    lease: actual,
                    expires_at: actual_expiry,
                } if actual == lease && actual_expiry > now => {}
                _ => return Err(StoreError::from(EffectStoreError::LeaseMismatch).into()),
            }
            entry.status = LedgerStatus::Claimed { lease, expires_at };
            let mut ops = Ops::new();
            Self::put_entry(&mut ops, &entry, Some(revision));
            self.apply_ops(ops)?;
            Ok(entry)
        })
    }

    pub(super) fn record_outcome(
        &mut self,
        record: EffectOutcomeRecord,
    ) -> Result<EffectLedgerEntry, StoreError> {
        retry(|| {
            let sweep = self.sweep()?;
            let (existing, revision) = self
                .read_entry(record.execution, record.effect)?
                .ok_or(StoreError::from(EffectStoreError::Missing))?;
            if existing.request_digest != record.request_digest {
                return Err(StoreError::from(EffectStoreError::RequestConflict).into());
            }
            match existing.status {
                LedgerStatus::Claimed { lease, expires_at }
                    if lease == record.lease && expires_at > record.observed_at => {}
                LedgerStatus::Claimed { .. } | LedgerStatus::RetryableFailure { .. } => {
                    return Err(StoreError::from(EffectStoreError::LeaseMismatch).into());
                }
                LedgerStatus::Completed { .. }
                | LedgerStatus::Rejected { .. }
                | LedgerStatus::UnknownOutcome { .. }
                | LedgerStatus::Compensated { .. } => {
                    return Err(StoreError::from(EffectStoreError::Terminal).into());
                }
            }
            let mut ops = Ops::new();
            ops.key(sweep_check(sweep));
            let allocate = |ops: &mut Ops| -> Result<LedgerFence, StoreError> {
                let current = self.read_fence(record.execution)?;
                let next = current
                    .map_or_else(LedgerFence::zero, |(fence, _)| fence)
                    .next()?;
                ops.key(Op::put(
                    layout::LEDGER_FENCES,
                    layout::fence_key(record.execution),
                    layout::encode_fence(next),
                    entry_expect(current.map(|(_, revision)| revision)),
                    Tag::Replan,
                ));
                Ok(next)
            };
            let status = match &record.outcome {
                EffectOutcome::Completed(response) => {
                    let response =
                        stage_response(&mut ops, &existing, response, record.observed_at)?;
                    LedgerStatus::Completed {
                        response,
                        fence: allocate(&mut ops)?,
                    }
                }
                EffectOutcome::Rejected(response) => {
                    let response =
                        stage_response(&mut ops, &existing, response, record.observed_at)?;
                    LedgerStatus::Rejected {
                        response,
                        fence: allocate(&mut ops)?,
                    }
                }
                EffectOutcome::RetryableFailure(diagnostic) => LedgerStatus::RetryableFailure {
                    diagnostic: *diagnostic,
                },
                EffectOutcome::UnknownOutcome(diagnostic) => LedgerStatus::UnknownOutcome {
                    diagnostic: *diagnostic,
                    fence: allocate(&mut ops)?,
                },
            };
            let entry = EffectLedgerEntry { status, ..existing };
            Self::put_entry(&mut ops, &entry, Some(revision));
            ops.key(graph_bump());
            self.apply_ops(ops)?;
            Ok(entry)
        })
    }

    pub(super) fn compensate(
        &mut self,
        execution: ExecutionId,
        original: EffectId,
        by_effect: EffectId,
    ) -> Result<EffectLedgerEntry, StoreError> {
        retry(|| {
            let (compensator, compensator_revision) = self
                .read_entry(execution, by_effect)?
                .ok_or(StoreError::from(EffectStoreError::Missing))?;
            let compensation_fence = match compensator.status {
                LedgerStatus::Completed { fence, .. } => fence,
                _ => return Err(StoreError::from(EffectStoreError::InvalidCompensation).into()),
            };
            let (entry, revision) = self
                .read_entry(execution, original)?
                .ok_or(StoreError::from(EffectStoreError::Missing))?;
            let RewindPolicy::Compensatable { capability } = &entry.rewind else {
                return Err(StoreError::from(EffectStoreError::InvalidCompensation).into());
            };
            if capability != &compensator.capability {
                return Err(StoreError::from(EffectStoreError::InvalidCompensation).into());
            }
            let (response, original_fence) = match entry.status {
                LedgerStatus::Completed { response, fence } => (response, fence),
                _ => return Err(StoreError::from(EffectStoreError::InvalidCompensation).into()),
            };
            let entry = EffectLedgerEntry {
                status: LedgerStatus::Compensated {
                    response,
                    original_fence,
                    by_effect,
                    compensation_fence,
                },
                ..entry
            };
            let mut ops = Ops::new();
            if original != by_effect {
                ops.key(Op::check(
                    layout::EFFECTS,
                    layout::effect_key(execution, by_effect),
                    Expect::Revision(compensator_revision),
                    Tag::Replan,
                ));
            }
            Self::put_entry(&mut ops, &entry, Some(revision));
            self.apply_ops(ops)?;
            Ok(entry)
        })
    }
}

/// Adds the recorded response object and its touch key to `ops`.
fn stage_response(
    ops: &mut Ops,
    entry: &EffectLedgerEntry,
    response: &RecordedEffectResponseV1,
    observed_at: u64,
) -> Result<ObjectId, StoreError> {
    if response.effect != entry.effect
        || response.request_digest != entry.request_digest
        || response.capability != entry.capability
        || response.capability_version != entry.capability_version
    {
        return Err(EffectStoreError::InvalidResponse("contract mismatch".to_owned()).into());
    }
    let object = response.to_object();
    let id = object.id();
    ops.key(Op::put(
        layout::TOUCH,
        layout::touch_key(id),
        layout::encode_touch(observed_at),
        Expect::Any,
        Tag::Index,
    ));
    ops.object(object);
    Ok(id)
}
