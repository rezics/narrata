//! Commit planning for the Stage 1–5 roots (ADR 0014).
//!
//! The history engine stages objects, checks them and applies the batch; this module plans the
//! keys only this registrant has, in the order its batches have always used: inputs, Refs,
//! Catalog Heads, archives and Compound Saves, then the engine's Pins, and, for a restore, the
//! Effect ledger and its fences. A conflict on a caller's key is the caller's conflict; a
//! conflict elsewhere means the plan read stale state, so the engine plans again.

use std::collections::{BTreeMap, BTreeSet};

use narrata_core::{
    CommitId, CompoundSaveManifestId, ExecutionId, InputId, TimelineArchiveManifestId,
    TimelineCatalogEventId, codec::ObjectKind,
};
use narrata_history::{Op, Tag, Transaction, View, Written, expect};
use narrata_storage::{Expect, KeySpace, StorageBackend};

use super::{
    Store,
    registry::{Legacy, LegacyOp, RootKey},
};
use crate::{
    CatalogHeadRefValue, CatalogMutation, CatalogRefKey, CommitOutcome, CommitTransaction,
    CompoundSaveRefKey, CompoundSaveRefValue, EffectLedgerEntry, InputIdConflict, InputRecord,
    LedgerFence, RefKey, RefMutation, RefValue, StoreContents, StoreError, TimelineArchiveRefKey,
    TimelineArchiveRefValue, TimelineCoverage, layout, object::history_id,
};

/// The roots a write sets or clears, for its outcome.
#[derive(Default)]
struct Roots {
    refs: Vec<(RefKey, Option<CommitId>)>,
    catalogs: Vec<(
        CatalogRefKey,
        Option<(TimelineCatalogEventId, TimelineCoverage)>,
    )>,
    archives: Vec<(TimelineArchiveRefKey, Option<TimelineArchiveManifestId>)>,
    compound_saves: Vec<(CompoundSaveRefKey, Option<CompoundSaveManifestId>)>,
}

impl Roots {
    fn of(transaction: &CommitTransaction) -> Self {
        Self {
            refs: transaction
                .refs
                .iter()
                .map(|mutation| (mutation.key.clone(), mutation.next))
                .collect(),
            catalogs: transaction
                .catalogs
                .iter()
                .map(|mutation| {
                    (
                        mutation.key.clone(),
                        mutation.next.map(|event| (event, mutation.coverage)),
                    )
                })
                .collect(),
            archives: transaction
                .archives
                .iter()
                .map(|mutation| (mutation.key.clone(), mutation.next))
                .collect(),
            compound_saves: transaction
                .compound_saves
                .iter()
                .map(|mutation| (mutation.key.clone(), mutation.next))
                .collect(),
        }
    }

    fn outcome(self, written: Written) -> Result<CommitOutcome, StoreError> {
        let revision = || written.root_revision();
        let mut outcome = CommitOutcome {
            inserted_objects: usize::try_from(written.inserted).unwrap_or(usize::MAX),
            ..CommitOutcome::default()
        };
        for (key, commit) in self.refs {
            let value = commit
                .map(|commit| {
                    Ok::<_, StoreError>(RefValue {
                        revision: revision()?,
                        commit,
                    })
                })
                .transpose()?;
            outcome.refs.insert(key, value);
        }
        for (key, head) in self.catalogs {
            let value = head
                .map(|(event, coverage)| {
                    Ok::<_, StoreError>(CatalogHeadRefValue {
                        revision: revision()?,
                        event,
                        coverage,
                    })
                })
                .transpose()?;
            outcome.catalogs.insert(key, value);
        }
        for (key, manifest) in self.archives {
            let value = manifest
                .map(|manifest| {
                    Ok::<_, StoreError>(TimelineArchiveRefValue {
                        revision: revision()?,
                        manifest,
                    })
                })
                .transpose()?;
            outcome.archives.insert(key, value);
        }
        for (key, manifest) in self.compound_saves {
            let value = manifest
                .map(|manifest| {
                    Ok::<_, StoreError>(CompoundSaveRefValue {
                        revision: revision()?,
                        manifest,
                    })
                })
                .transpose()?;
            outcome.compound_saves.insert(key, value);
        }
        Ok(outcome)
    }
}

struct Request<'t> {
    atomic: bool,
    transaction: &'t CommitTransaction,
    /// Per-object observation times; the transaction's time otherwise.
    observed: &'t [u64],
    effects: &'t [EffectLedgerEntry],
    fences: &'t [(ExecutionId, LedgerFence)],
}

impl<B: StorageBackend> Store<B> {
    pub(super) fn write_transaction(
        &mut self,
        transaction: &CommitTransaction,
        atomic: bool,
    ) -> Result<CommitOutcome, StoreError> {
        self.write(&Request {
            transaction,
            atomic,
            observed: &[],
            effects: &[],
            fences: &[],
        })
    }

    pub(super) fn write_contents(&mut self, contents: StoreContents) -> Result<(), StoreError> {
        let observed = contents
            .objects
            .iter()
            .map(|(_, observed_at)| *observed_at)
            .collect::<Vec<_>>();
        let transaction = CommitTransaction {
            objects: contents
                .objects
                .into_iter()
                .map(|(object, _)| object)
                .collect(),
            refs: contents
                .refs
                .into_iter()
                .map(|(key, commit)| RefMutation {
                    key,
                    expected: None,
                    next: Some(commit),
                })
                .collect(),
            catalogs: contents
                .catalogs
                .into_iter()
                .map(|(key, event, coverage)| CatalogMutation {
                    key,
                    expected: None,
                    next: Some(event),
                    coverage,
                })
                .collect(),
            archives: contents
                .archives
                .into_iter()
                .map(|(key, manifest)| crate::ArchiveMutation {
                    key,
                    expected: None,
                    next: Some(manifest),
                })
                .collect(),
            compound_saves: contents
                .compound_saves
                .into_iter()
                .map(|(key, manifest)| crate::CompoundSaveMutation {
                    key,
                    expected: None,
                    next: Some(manifest),
                })
                .collect(),
            inputs: contents.inputs,
            pins: contents.pins,
            ..CommitTransaction::default()
        };
        self.write(&Request {
            transaction: &transaction,
            atomic: false,
            observed: &observed,
            effects: &contents.effects,
            fences: &contents.ledger_fences,
        })
        .map(|_| ())
    }

    fn write(&mut self, request: &Request<'_>) -> Result<CommitOutcome, StoreError> {
        let transaction = request.transaction;
        let target = |bytes: &[u8; 32]| history_id(bytes);
        let mut prefetch = Vec::new();
        prefetch.extend(
            transaction
                .refs
                .iter()
                .filter_map(|value| value.next)
                .map(|id| target(id.as_bytes())),
        );
        prefetch.extend(
            transaction
                .catalogs
                .iter()
                .filter_map(|value| value.next)
                .map(|id| target(id.as_bytes())),
        );
        prefetch.extend(
            transaction
                .archives
                .iter()
                .filter_map(|value| value.next)
                .map(|id| target(id.as_bytes())),
        );
        prefetch.extend(
            transaction
                .compound_saves
                .iter()
                .filter_map(|value| value.next)
                .map(|id| target(id.as_bytes())),
        );
        let history_transaction = Transaction {
            objects: transaction
                .objects
                .iter()
                .map(|object| object.object().clone())
                .collect(),
            observed: request.observed.to_vec(),
            observed_at: transaction.observed_at,
            refs: Vec::new(),
            pins: transaction
                .pins
                .iter()
                .map(|pin| narrata_history::Pin {
                    owner: pin.owner.clone(),
                    object: target(pin.object.as_bytes()),
                    expires_at: pin.expires_at,
                })
                .collect(),
            remove_pins: transaction
                .remove_pins
                .iter()
                .map(|(owner, object)| (owner.clone(), target(object.as_bytes())))
                .collect(),
            prefetch,
        };
        let written = if request.atomic {
            self.history
                .write_atomic(&history_transaction, |view| plan(view, request))?
        } else {
            self.history
                .write(&history_transaction, |view| plan(view, request))?
        };
        Roots::of(transaction).outcome(written)
    }
}

/// Checks that a root key is named once per transaction.
fn once(
    named: &mut BTreeSet<(KeySpace, Vec<u8>)>,
    space: KeySpace,
    key: &[u8],
    what: &'static str,
) -> Result<(), StoreError> {
    if named.insert((space, key.to_vec())) {
        Ok(())
    } else {
        Err(StoreError::InvalidGraph(what))
    }
}

/// The keys of one attempt, read against the state that attempt sees.
fn plan<B: StorageBackend>(
    view: &mut View<'_, B, Legacy>,
    request: &Request<'_>,
) -> Result<Vec<LegacyOp>, StoreError> {
    let transaction = request.transaction;
    let target = |bytes: &[u8; 32]| history_id(bytes);
    let mut keys = Vec::new();
    let mut inputs = BTreeMap::<(ExecutionId, InputId), InputRecord>::new();
    for record in &transaction.inputs {
        match inputs.get(&(record.execution, record.input)) {
            Some(existing) if existing == record => continue,
            Some(existing) => {
                return Err(InputIdConflict {
                    existing: *existing,
                    proposed: *record,
                }
                .into());
            }
            None => {}
        }
        inputs.insert((record.execution, record.input), *record);
        let key = layout::input_key(record.execution, record.input);
        match view.reader().read_key(layout::INPUTS, &key)? {
            Some(value) => {
                let (parent, payload, commit) = layout::decode_input(&value.value)?;
                let existing = InputRecord {
                    execution: record.execution,
                    input: record.input,
                    parent,
                    payload,
                    commit,
                };
                if existing != *record {
                    return Err(InputIdConflict {
                        existing,
                        proposed: *record,
                    }
                    .into());
                }
            }
            None => keys.push(Op::put(
                layout::INPUTS,
                key,
                layout::encode_input(record.parent, record.payload, record.commit),
                Expect::Absent,
                Tag::Replan,
            )),
        }
    }

    let mut named = BTreeSet::new();
    for mutation in &transaction.refs {
        let key = narrata_history::layout::ref_key(&mutation.key);
        once(
            &mut named,
            layout::REFS,
            &key,
            "Ref named twice in one transaction",
        )?;
        let mutation = narrata_history::RefMutation {
            key: mutation.key.clone(),
            expected: mutation.expected,
            next: mutation.next.map(|commit| target(commit.as_bytes())),
        };
        keys.push(mutation.plan(view)?);
    }
    for mutation in &transaction.catalogs {
        let key = layout::catalog_key(&mutation.key);
        once(
            &mut named,
            layout::CATALOG_HEADS,
            &key,
            "Catalog Head named twice in one transaction",
        )?;
        let tag = Tag::Root(RootKey::Catalog {
            key: mutation.key.clone(),
            expected: mutation.expected,
            proposed: mutation.next,
        });
        keys.push(match mutation.next {
            Some(event) => {
                view.require(
                    target(event.as_bytes()),
                    Some(ObjectKind::TimelineCatalogEvent.code()),
                )?;
                Op::put(
                    layout::CATALOG_HEADS,
                    key,
                    layout::encode_catalog_head(event, mutation.coverage),
                    expect(mutation.expected),
                    tag,
                )
            }
            None => Op::delete(layout::CATALOG_HEADS, key, expect(mutation.expected), tag),
        });
    }
    for mutation in &transaction.archives {
        let key = layout::archive_key(&mutation.key);
        once(
            &mut named,
            layout::ARCHIVES,
            &key,
            "archive named twice in one transaction",
        )?;
        let tag = Tag::Root(RootKey::Archive {
            key: mutation.key.clone(),
            expected: mutation.expected,
            proposed: mutation.next,
        });
        keys.push(match mutation.next {
            Some(manifest) => {
                view.require(
                    target(manifest.as_bytes()),
                    Some(ObjectKind::TimelineArchiveManifest.code()),
                )?;
                Op::put(
                    layout::ARCHIVES,
                    key,
                    layout::encode_archive(manifest),
                    expect(mutation.expected),
                    tag,
                )
            }
            None => Op::delete(layout::ARCHIVES, key, expect(mutation.expected), tag),
        });
    }
    for mutation in &transaction.compound_saves {
        let key = layout::compound_save_key(&mutation.key);
        once(
            &mut named,
            layout::COMPOUND_SAVES,
            &key,
            "Compound Save named twice in one transaction",
        )?;
        let tag = Tag::Root(RootKey::CompoundSave {
            key: mutation.key.clone(),
            expected: mutation.expected,
            proposed: mutation.next,
        });
        keys.push(match mutation.next {
            Some(manifest) => {
                view.require(
                    target(manifest.as_bytes()),
                    Some(ObjectKind::CompoundSaveManifest.code()),
                )?;
                Op::put(
                    layout::COMPOUND_SAVES,
                    key,
                    layout::encode_compound_save(manifest),
                    expect(mutation.expected),
                    tag,
                )
            }
            None => Op::delete(layout::COMPOUND_SAVES, key, expect(mutation.expected), tag),
        });
    }

    for entry in request.effects {
        view.require(
            target(entry.origin_commit.as_bytes()),
            Some(ObjectKind::Commit.code()),
        )?;
        if let Some(response) = entry.status.response() {
            view.require(
                target(response.as_bytes()),
                Some(ObjectKind::EffectResponse.code()),
            )?;
        }
        if entry.status.fence().is_some_and(|fence| fence.get() == 0) {
            return Err(StoreError::InvalidGraph("terminal ledger fence is zero"));
        }
        keys.push(Op::put(
            layout::EFFECTS,
            layout::effect_key(entry.execution, entry.effect),
            layout::encode_effect(entry),
            Expect::Absent,
            Tag::Replan,
        ));
    }
    for (execution, fence) in request.fences {
        keys.push(Op::put(
            layout::LEDGER_FENCES,
            layout::fence_key(*execution),
            layout::encode_fence(*fence),
            Expect::Absent,
            Tag::Replan,
        ));
    }
    Ok(keys)
}
