//! Checked publication metadata and explicit, host-directed artifact evolution (ADR 0023).
//! Descriptors are trusted only after registration against both verified programs.

#[cfg(feature = "trace")]
mod audit;
mod descriptor;
mod report;
mod transform;

pub use descriptor::{
    DeletedId, EvolutionDescriptorV1, MAX_DESCRIPTOR_BYTES, MAX_RECOVERIES, MigrationId, Recovery,
    RecoveryDeclarationsV1, RecoveryTarget,
};
pub use report::*;

use crate::{ArtifactId, Error, Program, Result, history::NodeDomain};
use narrata_history::migration::{ArtifactRegistry, MigrationRegistry};
use std::{collections::BTreeMap, sync::Arc};

/// Only programs and descriptors supplied by the trusted publication source belong here.
/// Checkpoint import never registers executable metadata from a save.
#[derive(Default)]
pub struct EvolutionRegistry {
    pub(crate) programs: BTreeMap<ArtifactId, Arc<Program>>,
    pub(crate) artifacts: ArtifactRegistry<narrata_history::ArtifactId, NodeDomain>,
    pub(crate) migrations: MigrationRegistry<
        narrata_history::migration::MigrationId,
        narrata_history::ArtifactId,
        EvolutionDescriptorV1,
    >,
}

impl EvolutionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_program(&mut self, program: Arc<Program>) -> Result<()> {
        program.verify_artifact()?;
        self.programs.insert(program.artifact_id(), program.clone());
        self.artifacts
            .register(artifact(program.artifact_id()), NodeDomain(program));
        Ok(())
    }

    pub fn program(&self, id: ArtifactId) -> Result<&Arc<Program>> {
        self.artifacts
            .get(artifact(id))
            .map(|d| &d.0)
            .ok_or_else(|| {
                Error::new(
                    "needs_artifact",
                    id.to_string(),
                    "exact artifact is not registered",
                )
            })
    }

    pub fn register_descriptor(
        &mut self,
        descriptor: EvolutionDescriptorV1,
    ) -> Result<MigrationId> {
        descriptor.validate(
            self.program(descriptor.source_artifact())?,
            self.program(descriptor.target_artifact())?,
        )?;
        let id = descriptor.id();
        if self.migrations.get(migration(id)).is_some() {
            return Ok(id);
        }
        self.migrations
            .register(
                migration(id),
                artifact(descriptor.source_artifact()),
                artifact(descriptor.target_artifact()),
                descriptor,
            )
            .map_err(|e| Error::new("migration", "descriptor", e.to_string()))?;
        Ok(id)
    }

    pub fn descriptor(&self, id: MigrationId) -> Option<&EvolutionDescriptorV1> {
        self.migrations.get(migration(id))
    }
}

pub(crate) fn artifact(id: ArtifactId) -> narrata_history::ArtifactId {
    narrata_history::ArtifactId::from_bytes(*id.as_bytes())
}
pub(crate) fn migration(id: MigrationId) -> narrata_history::migration::MigrationId {
    narrata_history::migration::MigrationId::from_bytes(*id.as_bytes())
}

/// Kept with the owning types; host binding/schema catalog registration is independent.
pub fn schemas() -> Vec<(&'static str, schemars::Schema)> {
    use schemars::schema_for;
    vec![
        (
            "evolution-declarations.schema.json",
            schema_for!(RecoveryDeclarationsV1),
        ),
        (
            "evolution-descriptor.schema.json",
            schema_for!(EvolutionDescriptorV1),
        ),
        (
            "evolution-assessment.schema.json",
            schema_for!(AssessmentV1),
        ),
        ("evolution-apply.schema.json", schema_for!(ApplyV1)),
    ]
}
