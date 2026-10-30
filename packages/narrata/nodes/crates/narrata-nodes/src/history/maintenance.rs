//! Host-authorized maintenance of a node library. No storage or bundle format is added.

use narrata_history::{
    DomainKinds, History, HistoryError, MaintenanceBackend, MaintenanceBudget, RefMutation,
    RetentionPolicy, Transaction,
};
use narrata_storage::StorageBackend;

use super::{NodeDomain, history_error};
use crate::{Error, Result};

pub type NodeHistory<B> = History<MaintenanceBackend<B>, DomainKinds<NodeDomain>>;

pub fn open<B: StorageBackend>(backend: B, budget: MaintenanceBudget) -> Result<NodeHistory<B>> {
    History::open_existing(
        MaintenanceBackend::new(backend, budget).map_err(history_error)?,
        DomainKinds::new(),
    )
    .map_err(history_error)
}

/// Deletes precisely the observed names in one atomic transaction. Never re-read a newer
/// revision and silently turn a stale deletion into authorization to delete that newer root.
pub fn delete_roots<B: StorageBackend>(
    history: &mut NodeHistory<B>,
    roots: Vec<RefMutation>,
) -> Result<()> {
    if roots.is_empty()
        || roots.len() > 256
        || roots
            .iter()
            .any(|r| r.expected.is_none() || r.next.is_some())
    {
        return Err(Error::new(
            "limit",
            "roots",
            "select 1..=256 observed roots for deletion",
        ));
    }
    history
        .write_atomic(
            &Transaction {
                refs: roots,
                ..Transaction::default()
            },
            |_| Ok(Vec::new()),
        )
        .map_err(history_error)?;
    Ok(())
}

/// The trusted host supplies Unix seconds and an explicit grace period. Integrity must be
/// complete before sweep: a limited scan, unknown kind or bad reference cannot authorize GC.
pub fn collect<B: StorageBackend>(
    history: &mut NodeHistory<B>,
    policy: RetentionPolicy,
) -> Result<narrata_history::GcReport> {
    if policy.now > 9_007_199_254_740_991 || policy.grace_seconds > 9_007_199_254_740_991 {
        return Err(Error::new(
            "limit",
            "clock",
            "maintenance requires finite safe Unix seconds and grace",
        ));
    }
    let issues = history.integrity_scan().map_err(history_error)?;
    if !issues.is_empty() {
        for issue in &issues {
            if let Some(object) = history.get_object(issue.object).map_err(history_error)?
                && history.kind(object.kind()).is_none()
            {
                return Err(history_error(HistoryError::UnregisteredKind {
                    object: object.id(),
                    kind: object.kind(),
                }));
            }
        }
        return Err(history_error(HistoryError::CorruptStore(
            "integrity scan found invalid objects or references; collection refused".into(),
        )));
    }
    history.collect_atomic(policy).map_err(history_error)
}
