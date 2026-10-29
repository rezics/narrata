//! Verified publication evolution planning (ADR 0023). Stable IDs alone never prove
//! compatibility; reader assessment and application use the existing session coordinator.

use narrata_nodes::history::evolution::{
    EvolutionDescriptorV1, MigrationId, RecoveryDeclarationsV1,
};
use narrata_nodes::{ArtifactId, Error, Result};
use schemars::JsonSchema;
use serde::Serialize;

use crate::CheckedPublication;

/// Fixed-domain sampling key for an authorized impact scan. The artifact is also the
/// primary grouping key; input order and duplicate Ref heads do not select the sample.
pub fn impact_sample_key(artifact: ArtifactId, commit: narrata_nodes::CommitId) -> [u8; 32] {
    let mut w = narrata_kernel::codec::CborWriter::new();
    w.array(2);
    w.bytes(artifact.as_bytes());
    w.bytes(commit.as_bytes());
    narrata_kernel::codec::digest_bytes("narrata.nodes.evolution-sample", 1, &w.into_bytes())
}

/// Meaning checked against both complete program closures. A claimed digest alone cannot
/// construct this value or attach metadata to a publication.
#[derive(Clone, Debug)]
pub struct CheckedEvolution(EvolutionDescriptorV1);

impl CheckedEvolution {
    pub fn descriptor(&self) -> &EvolutionDescriptorV1 {
        &self.0
    }
    pub fn id(&self) -> MigrationId {
        self.0.id()
    }
    pub fn bytes(&self) -> Vec<u8> {
        self.0.encode()
    }

    pub fn read(
        bytes: &[u8],
        expected: MigrationId,
        previous: &CheckedPublication,
        candidate: &CheckedPublication,
    ) -> Result<Self> {
        let descriptor = EvolutionDescriptorV1::decode(bytes)?;
        if descriptor.id() != expected {
            return Err(Error::new(
                "identity",
                "evolution",
                "descriptor identity differs",
            ));
        }
        descriptor.validate(previous.program(), candidate.program())?;
        Ok(Self(descriptor))
    }
}

#[derive(Clone, Debug, JsonSchema, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EvolutionPlanningV1 {
    Exact {
        artifact_id: ArtifactId,
    },
    Descriptor {
        migration_id: MigrationId,
        descriptor: EvolutionDescriptorV1,
    },
}

#[derive(Clone, Debug)]
pub enum PlannedEvolution {
    Exact(ArtifactId),
    Descriptor(CheckedEvolution),
}
impl PlannedEvolution {
    pub fn report(&self) -> EvolutionPlanningV1 {
        match self {
            Self::Exact(artifact_id) => EvolutionPlanningV1::Exact {
                artifact_id: *artifact_id,
            },
            Self::Descriptor(checked) => EvolutionPlanningV1::Descriptor {
                migration_id: checked.id(),
                descriptor: checked.0.clone(),
            },
        }
    }
}

/// Strict declarations retain the core's duplicate-key, sorting, budget and tombstone
/// checks. An equal artifact needs no self edge, including body revisions and alias edits.
pub fn plan_evolution(
    previous: &CheckedPublication,
    candidate: &CheckedPublication,
    declarations: Option<&str>,
) -> Result<PlannedEvolution> {
    previous.program().verify_artifact()?;
    candidate.program().verify_artifact()?;
    let declarations = declarations
        .map(RecoveryDeclarationsV1::from_json)
        .transpose()?;
    let from = previous.program().artifact_id();
    let to = candidate.program().artifact_id();
    if declarations
        .as_ref()
        .is_some_and(|d| d.source_artifact() != from)
    {
        return Err(Error::new(
            "artifact",
            "declarations",
            "declaration source differs",
        ));
    }
    let recoveries = declarations
        .map(|d| d.recoveries().to_vec())
        .unwrap_or_default();
    if from == to {
        if !recoveries.is_empty() {
            return Err(Error::new(
                "self_edge",
                "declarations",
                "exact restoration has no recoveries",
            ));
        }
        return Ok(PlannedEvolution::Exact(from));
    }
    let descriptor = EvolutionDescriptorV1::new(from, to, recoveries)?;
    descriptor.validate(previous.program(), candidate.program())?;
    Ok(PlannedEvolution::Descriptor(CheckedEvolution(descriptor)))
}
