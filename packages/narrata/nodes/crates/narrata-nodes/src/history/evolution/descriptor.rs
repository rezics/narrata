use crate::{
    ArtifactId, AuthoredId, ChoicePointId, Error, Lookup, NodeId, Program, Result,
    json::parse_json_limited, plan::Plan,
};
use narrata_kernel::codec::{CborReader, CborWriter, DecodeError, digest_bytes};
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{borrow::Cow, collections::BTreeSet};

pub const MAX_DESCRIPTOR_BYTES: usize = 1024 * 1024;
pub const MAX_RECOVERIES: usize = 4096;

narrata_kernel::derived_id!(MigrationId, "migration:");
impl Serialize for MigrationId {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}
impl<'de> Deserialize<'de> for MigrationId {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        let id: Self = text.parse().map_err(serde::de::Error::custom)?;
        if id.to_string() != text {
            return Err(serde::de::Error::custom("noncanonical migration ID"));
        }
        Ok(id)
    }
}
impl JsonSchema for MigrationId {
    fn schema_name() -> Cow<'static, str> {
        "MigrationId".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({"type":"string","pattern":"^migration:[0-9a-f]{64}$"})
    }
}

/// Only continuation positions can declare recovery; options cannot relocate a state.
#[derive(
    Clone, Copy, Debug, Deserialize, Eq, JsonSchema, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(untagged)]
pub enum DeletedId {
    Node(NodeId),
    ChoicePoint(ChoicePointId),
}
impl DeletedId {
    pub(crate) fn authored(self) -> AuthoredId {
        match self {
            Self::Node(id) => AuthoredId::Node(id),
            Self::ChoicePoint(id) => AuthoredId::ChoicePoint(id),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryTarget {
    pub node: NodeId,
    pub choice_point: ChoicePointId,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Recovery {
    pub deleted: DeletedId,
    pub target: RecoveryTarget,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(rename = "RecoveryDeclarationsV1")]
struct DeclarationsWire {
    #[schemars(range(min = 1, max = 1))]
    format_version: u16,
    from_artifact: ArtifactId,
    #[schemars(length(max = 4096))]
    recoveries: Vec<Recovery>,
}
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(rename = "EvolutionDescriptorV1")]
struct DescriptorWire {
    #[schemars(range(min = 1, max = 1))]
    format_version: u16,
    from_artifact: ArtifactId,
    to_artifact: ArtifactId,
    #[schemars(length(max = 4096))]
    recoveries: Vec<Recovery>,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(try_from = "DeclarationsWire")]
#[schemars(with = "DeclarationsWire")]
pub struct RecoveryDeclarationsV1 {
    format_version: u16,
    from_artifact: ArtifactId,
    recoveries: Vec<Recovery>,
}
impl TryFrom<DeclarationsWire> for RecoveryDeclarationsV1 {
    type Error = Error;
    fn try_from(w: DeclarationsWire) -> Result<Self> {
        version(w.format_version)?;
        Self::new(w.from_artifact, w.recoveries)
    }
}
impl RecoveryDeclarationsV1 {
    pub fn new(from_artifact: ArtifactId, recoveries: Vec<Recovery>) -> Result<Self> {
        Ok(Self {
            format_version: 1,
            from_artifact,
            recoveries: sorted(recoveries)?,
        })
    }
    pub fn from_json(text: &str) -> Result<Self> {
        parse_json_limited(text, MAX_DESCRIPTOR_BYTES)
    }
    pub fn source_artifact(&self) -> ArtifactId {
        self.from_artifact
    }
    pub fn recoveries(&self) -> &[Recovery] {
        &self.recoveries
    }
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(try_from = "DescriptorWire")]
#[schemars(with = "DescriptorWire")]
pub struct EvolutionDescriptorV1 {
    format_version: u16,
    from_artifact: ArtifactId,
    to_artifact: ArtifactId,
    recoveries: Vec<Recovery>,
}
impl TryFrom<DescriptorWire> for EvolutionDescriptorV1 {
    type Error = Error;
    fn try_from(w: DescriptorWire) -> Result<Self> {
        version(w.format_version)?;
        Self::new(w.from_artifact, w.to_artifact, w.recoveries)
    }
}
fn version(v: u16) -> Result<()> {
    if v != 1 {
        return Err(Error::new(
            "version",
            "format_version",
            "unsupported evolution version",
        ));
    }
    Ok(())
}
fn sorted(mut values: Vec<Recovery>) -> Result<Vec<Recovery>> {
    if values.len() > MAX_RECOVERIES {
        return Err(Error::new("limit", "recoveries", "more than 4096 mappings"));
    }
    values.sort_by_key(|r| r.deleted);
    if values.windows(2).any(|w| w[0].deleted == w[1].deleted) {
        return Err(Error::new(
            "duplicate",
            "recoveries",
            "duplicate deleted ID",
        ));
    }
    Ok(values)
}

impl EvolutionDescriptorV1 {
    /// Checks the data form. `validate` additionally proves its meaning against artifacts.
    pub fn new(
        from_artifact: ArtifactId,
        to_artifact: ArtifactId,
        recoveries: Vec<Recovery>,
    ) -> Result<Self> {
        if from_artifact == to_artifact {
            return Err(Error::new(
                "self_edge",
                "descriptor",
                "identical artifacts use exact restoration",
            ));
        }
        Ok(Self {
            format_version: 1,
            from_artifact,
            to_artifact,
            recoveries: sorted(recoveries)?,
        })
    }
    pub fn from_json(text: &str) -> Result<Self> {
        parse_json_limited(text, MAX_DESCRIPTOR_BYTES)
    }
    pub fn source_artifact(&self) -> ArtifactId {
        self.from_artifact
    }
    pub fn target_artifact(&self) -> ArtifactId {
        self.to_artifact
    }
    pub fn recoveries(&self) -> &[Recovery] {
        &self.recoveries
    }
    pub fn id(&self) -> MigrationId {
        MigrationId::from_bytes(digest_bytes("narrata.nodes.evolution", 1, &self.encode()))
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut w = CborWriter::new();
        w.map(4);
        w.unsigned(0);
        w.unsigned(1);
        w.unsigned(1);
        w.bytes(self.from_artifact.as_bytes());
        w.unsigned(2);
        w.bytes(self.to_artifact.as_bytes());
        w.unsigned(3);
        w.array(self.recoveries.len() as u64);
        for recovery in &self.recoveries {
            w.array(4);
            match recovery.deleted {
                DeletedId::Node(id) => {
                    w.unsigned(0);
                    w.bytes(id.as_bytes());
                }
                DeletedId::ChoicePoint(id) => {
                    w.unsigned(1);
                    w.bytes(id.as_bytes());
                }
            }
            w.bytes(recovery.target.node.as_bytes());
            w.bytes(recovery.target.choice_point.as_bytes());
        }
        w.into_bytes()
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_DESCRIPTOR_BYTES {
            return Err(Error::new(
                "limit",
                "descriptor",
                "descriptor exceeds 1 MiB",
            ));
        }
        let parse = || -> std::result::Result<_, DecodeError> {
            let mut r = CborReader::new(bytes);
            narrata_history::layout::expect_map(&mut r, 4)?;
            narrata_history::layout::key(&mut r, 0)?;
            if r.unsigned()? != 1 {
                return Err(DecodeError::Schema("evolution version"));
            }
            narrata_history::layout::key(&mut r, 1)?;
            let from = ArtifactId::from_bytes(r.bytes_exact()?);
            narrata_history::layout::key(&mut r, 2)?;
            let to = ArtifactId::from_bytes(r.bytes_exact()?);
            narrata_history::layout::key(&mut r, 3)?;
            let count = r.array_len()?;
            if count > MAX_RECOVERIES as u64 {
                return Err(DecodeError::Schema("recovery limit"));
            }
            let mut recoveries = Vec::new();
            for _ in 0..count {
                if r.array_len()? != 4 {
                    return Err(DecodeError::Schema("recovery tuple"));
                }
                let kind = r.unsigned()?;
                let deleted = r.bytes_exact()?;
                let deleted = match kind {
                    0 => DeletedId::Node(NodeId::from_bytes(deleted)),
                    1 => DeletedId::ChoicePoint(ChoicePointId::from_bytes(deleted)),
                    _ => return Err(DecodeError::Schema("deleted kind")),
                };
                recoveries.push(Recovery {
                    deleted,
                    target: RecoveryTarget {
                        node: NodeId::from_bytes(r.bytes_exact()?),
                        choice_point: ChoicePointId::from_bytes(r.bytes_exact()?),
                    },
                });
            }
            r.finish()?;
            Ok((from, to, recoveries))
        };
        let (from, to, values) =
            parse().map_err(|e| Error::new("decode", "descriptor", e.to_string()))?;
        let result = Self::new(from, to, values)?;
        if result.encode() != bytes {
            return Err(Error::new(
                "canonical",
                "descriptor",
                "noncanonical descriptor",
            ));
        }
        Ok(result)
    }

    pub fn validate(&self, source: &Program, target: &Program) -> Result<()> {
        if source.artifact_id() != self.from_artifact || target.artifact_id() != self.to_artifact {
            return Err(Error::new(
                "artifact",
                "descriptor",
                "descriptor artifacts differ",
            ));
        }
        source.verify_artifact()?;
        target.verify_artifact()?;
        let old = source.owners()?;
        let new = target.owners()?;
        let tombstones = target.tombstones()?;
        for (id, owner) in &old {
            match new.get(id) {
                Some(next) if next == owner => {}
                Some(_) => {
                    return Err(Error::new(
                        "ownership",
                        id.to_string(),
                        "stable identity changed owner",
                    ));
                }
                None if tombstones.contains(id) => {}
                None => {
                    return Err(Error::new(
                        "unknown_id",
                        id.to_string(),
                        "disappearing identity lacks a tombstone",
                    ));
                }
            }
        }
        let old_dead = source.tombstones()?;
        if !old_dead.nodes.is_subset(&tombstones.nodes)
            || !old_dead.choice_points.is_subset(&tombstones.choice_points)
            || !old_dead.options.is_subset(&tombstones.options)
        {
            return Err(Error::new(
                "tombstone",
                "descriptor",
                "previous tombstones must remain deleted",
            ));
        }
        let graph = target.graph(&target.manifest().product.entry)?;
        let mut seen = BTreeSet::new();
        for recovery in &self.recoveries {
            let id = recovery.deleted.authored();
            if !seen.insert(id)
                || !matches!(source.lookup(id)?, Lookup::Live { .. })
                || target.lookup(id)? != Lookup::Deleted
            {
                return Err(Error::new(
                    "recovery",
                    id.to_string(),
                    "recovery must name a live source and deleted target position",
                ));
            }
            let Some(Plan::Passage(passage)) = graph.nodes.get(&recovery.target.node) else {
                return Err(Error::new(
                    "recovery",
                    "target",
                    "recovery target must be an entry-graph passage",
                ));
            };
            if !passage
                .choice_points
                .iter()
                .any(|p| p.id == recovery.target.choice_point)
            {
                return Err(Error::new(
                    "recovery",
                    "target",
                    "recovery choice point is not in its passage",
                ));
            }
        }
        Ok(())
    }
}
