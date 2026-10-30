//! Shared native/Wasm driver. The only migration implementation is Session's coordinator.

use narrata_authoring::CheckedPublication;
use narrata_nodes::history::evolution::{
    ApplyV1, AssessmentV1, EvolutionRegistry, MigrationId, PathLimits, PlanV1,
};
use narrata_nodes::{ArtifactId, Error, ExecutionId, Result, Session};
use narrata_storage_host::{CacheBackend, LoadRequest};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Per-assessment limits, also bound to the confirmation in this driver. Values may only
/// tighten the core's limits. JSON integers fit exactly in JavaScript.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComparisonBudgetV1 {
    #[schemars(range(max = 10000))]
    pub transitions: u32,
    #[schemars(range(max = 100000))]
    pub automatic_steps: u32,
    #[schemars(range(max = 67108864))]
    pub object_bytes: u32,
}
impl Default for ComparisonBudgetV1 {
    fn default() -> Self {
        Self {
            transitions: 10_000,
            automatic_steps: 100_000,
            object_bytes: 64 * 1024 * 1024,
        }
    }
}
impl ComparisonBudgetV1 {
    fn limits(self) -> Result<PathLimits> {
        let max = Self::default();
        if self.transitions > max.transitions
            || self.automatic_steps > max.automatic_steps
            || self.object_bytes > max.object_bytes
        {
            return Err(Error::new(
                "limit",
                "comparison_budget",
                "comparison budget exceeds core maximum",
            ));
        }
        Ok(PathLimits {
            transitions: self.transitions as usize,
            automatic_steps: self.automatic_steps as usize,
            object_bytes: self.object_bytes as usize,
        })
    }
}

#[derive(Default)]
pub struct Engine {
    pub registry: Arc<EvolutionRegistry>,
    pub cache: CacheBackend,
    snapshot: Option<Session>,
    execution: Option<ExecutionId>,
    approved: Option<(PlanV1, ComparisonBudgetV1)>,
    request: Option<LoadRequest>,
}
impl Engine {
    pub fn visits(
        &mut self,
        budget: ComparisonBudgetV1,
    ) -> Result<narrata_nodes::history::visits::CheckedVisits> {
        let limits = budget.limits()?;
        let result = if let Some(execution) = self.execution {
            Session::open_with_registry(&self.registry, execution, self.cache.share()).and_then(
                |s| {
                    self.registry
                        .visited_from_save(s.history(), s.cursor()?, limits)
                },
            )
        } else {
            self.snapshot
                .as_ref()
                .ok_or_else(|| Error::new("save", "visits", "open a save first"))
                .and_then(|s| {
                    self.registry
                        .visited_from_save(s.history(), s.cursor()?, limits)
                })
        };
        self.settle(result)
    }
    /// Read-only jobs share verified programs and descriptors, while cursors, approvals and
    /// storage caches remain isolated. No full-work cloning or re-verification per reader.
    pub fn fork(&self) -> Self {
        Self {
            registry: self.registry.clone(),
            ..Self::default()
        }
    }
    fn registry_mut(&mut self) -> Result<&mut EvolutionRegistry> {
        Arc::get_mut(&mut self.registry).ok_or_else(|| {
            Error::new(
                "registry_in_use",
                "evolution",
                "close assessment forks before changing trusted publications",
            )
        })
    }
    pub fn register_publication(&mut self, publication: &CheckedPublication) -> Result<()> {
        self.approved = None;
        self.registry_mut()?
            .register_program(publication.shared_program())
    }
    /// The host must obtain this publication from its trusted publication collection.
    /// Saves cannot register their own descriptors.
    pub fn register_evolution(
        &mut self,
        previous: &CheckedPublication,
        candidate: &CheckedPublication,
    ) -> Result<MigrationId> {
        let checked = candidate.evolution(previous)?.ok_or_else(|| {
            Error::new(
                "descriptor",
                "evolution",
                "publication has no evolution object",
            )
        })?;
        self.register_publication(previous)?;
        self.register_publication(candidate)?;
        self.registry_mut()?
            .register_descriptor(checked.descriptor().clone())
    }
    pub fn restore(&mut self, save: &str) -> Result<()> {
        self.approved = None;
        self.execution = None;
        self.snapshot = None;
        self.snapshot = Some(Session::restore_with_registry(&self.registry, save)?);
        Ok(())
    }
    pub fn connect(&mut self, execution: ExecutionId) {
        self.approved = None;
        self.snapshot = None;
        self.execution = Some(execution);
        self.refresh();
    }
    /// Discard cached durable reads between operations. A persisted CAS is still required
    /// after the last read. The StorageHost prevents refresh while a write is unknown.
    pub fn refresh(&mut self) {
        self.cache.invalidate();
        self.request = None;
    }

    // The session core reports logical failures as data. Cache misses are transport work:
    // surface them for load-and-retry instead of misclassifying an unloaded save as invalid.
    fn settle<T>(&mut self, value: Result<T>) -> Result<T> {
        if let Some(request) = self.cache.take_request() {
            self.request = Some(request);
            return Err(Error::new(
                "storage_pending",
                "evolution",
                "load requested storage objects and retry",
            ));
        }
        value
    }
    pub fn take_request(&mut self) -> Option<LoadRequest> {
        self.request.take().or_else(|| self.cache.take_request())
    }

    pub fn assess(
        &mut self,
        target: ArtifactId,
        ids: &[MigrationId],
        budget: ComparisonBudgetV1,
    ) -> Result<AssessmentV1> {
        self.approved = None;
        let limits = budget.limits()?;
        let result = if let Some(execution) = self.execution {
            Session::open_with_registry(&self.registry, execution, self.cache.share())
                .map(|session| session.assess_evolution(&self.registry, target, ids, limits))
        } else {
            self.snapshot
                .as_ref()
                .ok_or_else(|| Error::new("save", "evolution", "open a save first"))
                .map(|session| session.assess_evolution(&self.registry, target, ids, limits))
        };
        let result = match self.settle(result) {
            Ok(result) => result,
            Err(error) if error.code == "storage_pending" => return Err(error),
            Err(error) => return Ok(unavailable(error)),
        };
        if let AssessmentV1::Automatic { plan } | AssessmentV1::NeedsConfirmation { plan, .. } =
            &result
        {
            self.approved = Some((plan.as_ref().clone(), budget));
        }
        Ok(result)
    }
    /// Snapshot import errors are assessment data; invalid host budgets remain call errors.
    pub fn assess_save(
        &mut self,
        save: &str,
        target: ArtifactId,
        ids: &[MigrationId],
        budget: ComparisonBudgetV1,
    ) -> Result<AssessmentV1> {
        budget.limits()?;
        if self.execution.is_some() {
            return Err(Error::new(
                "mode",
                "evolution",
                "use a separate snapshot context",
            ));
        }
        if let Err(error) = self.restore(save) {
            return Ok(unavailable(error));
        }
        self.assess(target, ids, budget)
    }
    pub fn apply(&mut self, plan: &PlanV1, budget: ComparisonBudgetV1) -> Result<ApplyV1> {
        let limits = budget.limits()?;
        if self.approved.as_ref() != Some(&(plan.clone(), budget)) {
            return Ok(ApplyV1::PlanChanged);
        }
        let result = if let Some(execution) = self.execution {
            Session::open_with_registry(&self.registry, execution, self.cache.share())
                .map(|mut session| session.apply_evolution(&self.registry, plan, limits))
        } else {
            self.snapshot
                .as_mut()
                .ok_or_else(|| Error::new("save", "evolution", "open a save first"))
                .map(|session| session.apply_evolution(&self.registry, plan, limits))
        };
        let result = self.settle(result)?;
        self.approved = None;
        Ok(result)
    }
    pub fn export(&mut self) -> Result<String> {
        let result = if let Some(execution) = self.execution {
            Session::open_with_registry(&self.registry, execution, self.cache.share())
                .and_then(|s| s.export())
        } else {
            self.snapshot
                .as_ref()
                .ok_or_else(|| Error::new("save", "evolution", "open a save first"))
                .and_then(Session::export)
        };
        self.settle(result)
    }
    /// Explicitly preserve a shallow history boundary using the existing kernel container.
    pub fn export_shallow(&mut self, ancestors: u32) -> Result<Vec<u8>> {
        if ancestors > 10_000 {
            return Err(Error::new("limit", "checkpoint", "at most 10000 ancestors"));
        }
        let result = if let Some(execution) = self.execution {
            Session::open_with_registry(&self.registry, execution, self.cache.share())
                .and_then(|session| shallow(&session, ancestors))
        } else {
            self.snapshot
                .as_ref()
                .ok_or_else(|| Error::new("save", "evolution", "open a save first"))
                .and_then(|session| shallow(session, ancestors))
        };
        self.settle(result)
    }
}

fn shallow<B: narrata_storage::StorageBackend>(
    session: &Session<B>,
    ancestors: u32,
) -> Result<Vec<u8>> {
    session.export_shallow(ancestors)
}

fn unavailable(error: Error) -> AssessmentV1 {
    use narrata_nodes::{ArtifactId, CommitId, ObjectId};
    if matches!(error.code.as_str(), "needs_artifact" | "needs_object") {
        let digest = error
            .path
            .parse::<ObjectId>()
            .map(|id| *id.as_bytes())
            .or_else(|_| error.path.parse::<ArtifactId>().map(|id| *id.as_bytes()))
            .or_else(|_| error.path.parse::<CommitId>().map(|id| *id.as_bytes()));
        return AssessmentV1::NeedsObjects {
            digests: digest.map(ObjectId::from_bytes).into_iter().collect(),
            retryable: true,
        };
    }
    AssessmentV1::InvalidSave {
        diagnostic: narrata_nodes::Diagnostic {
            code: error.code,
            path: "evolution".into(),
            message: "save could not be assessed".into(),
        },
        restart_available: true,
    }
}
