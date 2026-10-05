//! Shared effect state machine; registrations own their contract and response encoding.
use crate::effect::*;
use crate::{
    History, HistoryError, ObjectId, Op, Ops, Registry, Tag, graph_bump, layout, retry, sweep_check,
};
use narrata_storage::{Expect, Revision, StorageBackend};
fn effect_error<E: From<HistoryError>>(error: EffectStoreError) -> E {
    HistoryError::Effect(error).into()
}

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

impl<B: StorageBackend, R: Registry> History<B, R> {
    /// Pages the execution's monotonic facts, independent of its cursor and branches.
    pub fn scan_effects<L: EffectRegistration>(
        &self,
        execution: ExecutionId,
        after: Option<&EffectId>,
        limit: u32,
    ) -> Result<crate::Page<EffectLedgerEntry>, R::Error> {
        self.reader().page(
            L::EFFECTS,
            execution.as_bytes(),
            after.map(|effect| effect_key(execution, *effect)),
            limit,
            |value| L::decode(&value.key, &value.value).map_err(R::Error::from),
        )
    }

    pub fn read_effect_entry<L: EffectRegistration>(
        &self,
        execution: ExecutionId,
        effect: EffectId,
    ) -> Result<Option<(EffectLedgerEntry, Revision)>, R::Error> {
        let key = effect_key(execution, effect);
        self.reader()
            .read_key(L::EFFECTS, &key)?
            .map(|value| Ok((L::decode(&key, &value.value)?, value.revision)))
            .transpose()
    }

    pub fn read_effect_fence<L: EffectRegistration>(
        &self,
        execution: ExecutionId,
    ) -> Result<Option<(LedgerFence, Revision)>, R::Error> {
        self.reader()
            .read_key(L::FENCES, &fence_key(execution))?
            .map(|value| Ok((decode_fence(&value.value)?, value.revision)))
            .transpose()
    }

    fn put_effect_entry<L: EffectRegistration>(
        ops: &mut Ops<R::Tag>,
        entry: &EffectLedgerEntry,
        revision: Option<Revision>,
    ) -> Result<(), R::Error> {
        let key = effect_key(entry.execution, entry.effect);
        let encoded = L::encode(entry)?;
        if L::decode(&key, &encoded)? != *entry {
            return Err(HistoryError::InvalidGraph("effect contract does not round-trip").into());
        }
        ops.key(Op::put(
            L::EFFECTS,
            key,
            encoded,
            entry_expect(revision),
            Tag::Replan,
        ));
        Ok(())
    }

    /// Claims only a stored origin commit. Dispatch starts after this operation succeeds;
    /// a recorded terminal result is returned for reuse without a new dispatch.
    pub fn claim_effect<L: EffectRegistration>(
        &mut self,
        claim: EffectClaim,
    ) -> Result<EffectClaimResult, R::Error> {
        if claim.expires_at <= claim.now {
            return Err(HistoryError::from(EffectStoreError::InvalidLease).into());
        }
        let origin = claim.origin_commit;
        retry::<_, R::Error>(|| {
            let sweep = self.reader().sweep().map_err(R::Error::from)?;
            self.reader()
                .require(origin, Some(L::COMMIT_KIND))
                .map_err(R::Error::from)?;
            let existing = self.read_effect_entry::<L>(claim.execution, claim.effect)?;
            if let Some((entry, _)) = &existing {
                if !same_claim_contract(entry, &claim) {
                    return Err(effect_error::<R::Error>(EffectStoreError::RequestConflict).into());
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
                capability_version: claim.capability_version.clone(),
                origin_commit: claim.origin_commit,
                delivery: claim.delivery,
                rewind: claim.rewind.clone(),
                status: LedgerStatus::Claimed {
                    lease: claim.lease,
                    expires_at: claim.expires_at,
                },
            };
            let mut ops = Ops::<R::Tag>::new();
            ops.key(sweep_check(sweep));
            if L::EFFECTS == EFFECTS {
                let guard = ledger_guard();
                ops.key(Op::put(
                    layout::TOUCH,
                    layout::touch_key(guard.id()),
                    layout::encode_touch(claim.now),
                    Expect::Any,
                    Tag::Index,
                ));
                ops.object(guard);
            }
            Self::put_effect_entry::<L>(&mut ops, &entry, existing.map(|(_, revision)| revision))?;
            ops.key(graph_bump());
            self.apply(ops)?;
            Ok(EffectClaimResult::Claimed(entry))
        })
    }

    pub fn renew_effect_lease<L: EffectRegistration>(
        &mut self,
        execution: ExecutionId,
        effect: EffectId,
        lease: LeaseId,
        now: u64,
        expires_at: u64,
    ) -> Result<EffectLedgerEntry, R::Error> {
        if expires_at <= now {
            return Err(HistoryError::from(EffectStoreError::InvalidLease).into());
        }
        retry::<_, R::Error>(|| {
            let (mut entry, revision) = self
                .read_effect_entry::<L>(execution, effect)?
                .ok_or(effect_error::<R::Error>(EffectStoreError::Missing))?;
            match entry.status {
                LedgerStatus::Claimed {
                    lease: actual,
                    expires_at: actual_expiry,
                } if actual == lease && actual_expiry > now => {}
                _ => {
                    return Err(effect_error::<R::Error>(EffectStoreError::LeaseMismatch).into());
                }
            }
            entry.status = LedgerStatus::Claimed { lease, expires_at };
            let mut ops = Ops::<R::Tag>::new();
            Self::put_effect_entry::<L>(&mut ops, &entry, Some(revision))?;
            self.apply(ops)?;
            Ok(entry)
        })
    }

    pub fn record_effect_outcome<L: EffectRegistration>(
        &mut self,
        record: EffectOutcomeRecord<L::Response>,
    ) -> Result<EffectLedgerEntry, R::Error> {
        retry::<_, R::Error>(|| {
            let sweep = self.reader().sweep().map_err(R::Error::from)?;
            let (existing, revision) = self
                .read_effect_entry::<L>(record.execution, record.effect)?
                .ok_or(effect_error::<R::Error>(EffectStoreError::Missing))?;
            if existing.request_digest != record.request_digest {
                return Err(effect_error::<R::Error>(EffectStoreError::RequestConflict).into());
            }
            match existing.status {
                LedgerStatus::Claimed { lease, expires_at }
                    if lease == record.lease && expires_at > record.observed_at => {}
                LedgerStatus::Claimed { .. } | LedgerStatus::RetryableFailure { .. } => {
                    return Err(effect_error::<R::Error>(EffectStoreError::LeaseMismatch).into());
                }
                LedgerStatus::Completed { .. }
                | LedgerStatus::Rejected { .. }
                | LedgerStatus::UnknownOutcome { .. }
                | LedgerStatus::Compensated { .. } => {
                    return Err(effect_error::<R::Error>(EffectStoreError::Terminal).into());
                }
            }
            let mut ops = Ops::<R::Tag>::new();
            ops.key(sweep_check(sweep));
            let allocate = |ops: &mut Ops<R::Tag>| -> Result<LedgerFence, R::Error> {
                let current = self.read_effect_fence::<L>(record.execution)?;
                let next = current
                    .map_or_else(LedgerFence::zero, |(fence, _)| fence)
                    .next()
                    .map_err(HistoryError::from)?;
                ops.key(Op::put(
                    L::FENCES,
                    fence_key(record.execution),
                    encode_fence(next),
                    entry_expect(current.map(|(_, revision)| revision)),
                    Tag::Replan,
                ));
                Ok(next)
            };
            let status = match &record.outcome {
                EffectOutcome::Completed(response) => {
                    let response = stage_response::<L, B, R>(
                        self,
                        &mut ops,
                        &existing,
                        response,
                        record.observed_at,
                    )?;
                    LedgerStatus::Completed {
                        response,
                        fence: allocate(&mut ops)?,
                    }
                }
                EffectOutcome::Rejected(response) => {
                    let response = stage_response::<L, B, R>(
                        self,
                        &mut ops,
                        &existing,
                        response,
                        record.observed_at,
                    )?;
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
            Self::put_effect_entry::<L>(&mut ops, &entry, Some(revision))?;
            ops.key(graph_bump());
            self.apply(ops)?;
            Ok(entry)
        })
    }

    pub fn mark_effect_compensated<L: EffectRegistration>(
        &mut self,
        execution: ExecutionId,
        original: EffectId,
        by_effect: EffectId,
    ) -> Result<EffectLedgerEntry, R::Error> {
        retry::<_, R::Error>(|| {
            let (compensator, compensator_revision) = self
                .read_effect_entry::<L>(execution, by_effect)?
                .ok_or(effect_error::<R::Error>(EffectStoreError::Missing))?;
            let compensation_fence = match compensator.status {
                LedgerStatus::Completed { fence, .. } => fence,
                _ => {
                    return Err(
                        effect_error::<R::Error>(EffectStoreError::InvalidCompensation).into(),
                    );
                }
            };
            let (entry, revision) = self
                .read_effect_entry::<L>(execution, original)?
                .ok_or(effect_error::<R::Error>(EffectStoreError::Missing))?;
            let RewindPolicy::Compensatable { capability } = &entry.rewind else {
                return Err(effect_error::<R::Error>(EffectStoreError::InvalidCompensation).into());
            };
            if capability != &compensator.capability {
                return Err(effect_error::<R::Error>(EffectStoreError::InvalidCompensation).into());
            }
            let (response, original_fence) = match entry.status {
                LedgerStatus::Completed { response, fence } => (response, fence),
                _ => {
                    return Err(
                        effect_error::<R::Error>(EffectStoreError::InvalidCompensation).into(),
                    );
                }
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
            let mut ops = Ops::<R::Tag>::new();
            if original != by_effect {
                ops.key(Op::check(
                    L::EFFECTS,
                    effect_key(execution, by_effect),
                    Expect::Revision(compensator_revision),
                    Tag::Replan,
                ));
            }
            Self::put_effect_entry::<L>(&mut ops, &entry, Some(revision))?;
            self.apply(ops)?;
            Ok(entry)
        })
    }
}

fn stage_response<L: EffectRegistration, B: StorageBackend, R: Registry>(
    history: &History<B, R>,
    ops: &mut Ops<R::Tag>,
    entry: &EffectLedgerEntry,
    response: &L::Response,
    observed_at: u64,
) -> Result<ObjectId, R::Error> {
    let object = L::response(entry, response).map_err(HistoryError::from)?;
    let id = object.id();
    if history.kind(object.kind()).is_none() {
        return Err(HistoryError::UnregisteredKind {
            object: id,
            kind: object.kind(),
        }
        .into());
    }
    let staged = std::collections::BTreeMap::from([(id, object.clone())]);
    let mut view = crate::View::new(history.reader(), history.registry(), &staged);
    for op in super::write::validate(&mut view, &object)? {
        ops.key(op);
    }
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
