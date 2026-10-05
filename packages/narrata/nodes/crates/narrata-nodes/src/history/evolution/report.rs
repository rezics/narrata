use super::{DeletedId, MigrationId, RecoveryTarget};
#[cfg(feature = "trace")]
use crate::State;
use crate::{
    ArtifactId, AuthoredId, ChoicePointId, CommitId, Diagnostic, Error, NodeId, ObjectId, Result,
    plan::GraphRef,
};
use narrata_kernel::codec::{CborWriter, digest_bytes};
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::borrow::Cow;
#[cfg(feature = "trace")]
use std::collections::BTreeMap;

/// Decimal text retains every kernel revision across JSON/JavaScript boundaries.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CursorRevision(pub u64);
impl Serialize for CursorRevision {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.collect_str(&self.0)
    }
}
impl<'de> Deserialize<'de> for CursorRevision {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        let value: u64 = text.parse().map_err(serde::de::Error::custom)?;
        if value.to_string() != text {
            return Err(serde::de::Error::custom("noncanonical cursor revision"));
        }
        Ok(Self(value))
    }
}
impl JsonSchema for CursorRevision {
    fn schema_name() -> Cow<'static, str> {
        "CursorRevision".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({"type":"string","pattern":"^(0|[1-9][0-9]{0,19})$"})
    }
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LocationV1 {
    pub graph: GraphRef,
    pub node: NodeId,
    pub choice_point: Option<ChoicePointId>,
    pub instance: u32,
}
impl LocationV1 {
    #[cfg(feature = "trace")]
    pub(crate) fn of(program: &crate::Program, state: &State) -> Option<Self> {
        state
            .frames
            .last()
            .map(|f| Self {
                graph: f.graph.clone(),
                node: f.node,
                choice_point: f.at,
                instance: f.instance,
            })
            .or_else(|| {
                state.finished.as_ref().map(|f| Self {
                    graph: program.manifest().product.entry.clone(),
                    node: f.node,
                    choice_point: None,
                    instance: f.instance,
                })
            })
    }
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DivergenceReason {
    InitialState,
    InvalidInput,
    Execution,
    SemanticState,
    ControlTrace,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UnverifiedReason {
    MissingAncestor,
    MissingArtifact,
    MissingDescriptor,
    Limit,
    Baseline,
    TraceUnavailable,
}
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PathComparisonV1 {
    Equivalent,
    Diverged {
        source_commit: CommitId,
        input_id: Option<ObjectId>,
        reason: DivergenceReason,
        old_location: Option<LocationV1>,
        new_location: Option<LocationV1>,
        differing_fields: Vec<String>,
    },
    Unverified {
        reason: UnverifiedReason,
        baseline: Option<CommitId>,
        checked_through: Option<CommitId>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DiscardedFrameV1 {
    pub instance: u32,
    pub node: NodeId,
    pub local_keys: Vec<String>,
    pub parameter_keys: Vec<String>,
    pub overlay_ids: Vec<AuthoredId>,
    pub pending_choice: Option<ChoicePointId>,
}
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryLossV1 {
    pub tombstones: Vec<DeletedId>,
    pub discarded_frames: Vec<DiscardedFrameV1>,
    pub cleared_finished: Option<NodeId>,
    pub retained_shared_keys: Vec<String>,
    pub reset_local_keys: Vec<String>,
    pub target: RecoveryTarget,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalEffectsV1 {
    NotApplicable,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MigrationReportV1 {
    pub from_artifact: ArtifactId,
    pub to_artifact: ArtifactId,
    pub source_commit: CommitId,
    pub migration_commit: CommitId,
    pub losses: Vec<RecoveryLossV1>,
    pub path: PathComparisonV1,
    pub external_effects: ExternalEffectsV1,
}
impl MigrationReportV1 {
    /// Canonical CBOR of the versioned report data, with deterministic length-first text
    /// map keys. Presentation strings, state values and scan statistics are absent.
    pub fn encode(&self) -> Result<Vec<u8>> {
        let value = serde_json::to_value(self)
            .map_err(|e| Error::new("report", "encode", e.to_string()))?;
        let mut w = CborWriter::new();
        w.map(2);
        w.unsigned(0);
        w.unsigned(1);
        w.unsigned(1);
        encode_value(&mut w, &value)?;
        Ok(w.into_bytes())
    }
    pub fn digest(&self) -> Result<ObjectId> {
        Ok(ObjectId::from_bytes(digest_bytes(
            "narrata.nodes.evolution-report",
            1,
            &self.encode()?,
        )))
    }
}
fn encode_value(w: &mut CborWriter, value: &serde_json::Value) -> Result<()> {
    use serde_json::Value;
    match value {
        Value::Null => w.null(),
        Value::Bool(v) => w.boolean(*v),
        Value::Number(n) => w.unsigned(n.as_u64().ok_or_else(|| {
            Error::new("report", "number", "report numbers are unsigned integers")
        })?),
        Value::String(s) => w.text(s),
        Value::Array(values) => {
            w.array(values.len() as u64);
            for v in values {
                encode_value(w, v)?;
            }
        }
        Value::Object(values) => {
            let mut pairs: Vec<_> = values.iter().collect();
            pairs.sort_by_key(|(key, _)| (key.len(), key.as_bytes()));
            w.map(pairs.len() as u64);
            for (key, v) in pairs {
                w.text(key);
                encode_value(w, v)?;
            }
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SourceV1 {
    pub source_commit: CommitId,
    pub artifact_id: ArtifactId,
}
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CursorV1 {
    pub commit: CommitId,
    pub revision: CursorRevision,
}
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlanV1 {
    pub source_commit: CommitId,
    pub target_artifact: ArtifactId,
    pub migration_ids: Vec<MigrationId>,
    pub expected_cursor_revision: CursorRevision,
    pub migration_commit: CommitId,
    pub report: MigrationReportV1,
    pub report_digest: ObjectId,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfirmationReason {
    Recovery,
    Divergence,
    Unverified,
}
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AssessmentV1 {
    Exact {
        source: SourceV1,
    },
    Automatic {
        plan: Box<PlanV1>,
    },
    NeedsConfirmation {
        plan: Box<PlanV1>,
        reason: ConfirmationReason,
    },
    OldVersionOnly {
        source: SourceV1,
        target: ArtifactId,
        reason: String,
        restart_available: bool,
    },
    NeedsObjects {
        digests: Vec<ObjectId>,
        retryable: bool,
    },
    InvalidSave {
        diagnostic: Diagnostic,
        restart_available: bool,
    },
}
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ApplyV1 {
    Migrated {
        source_commit: CommitId,
        migration_commit: CommitId,
        target_artifact: ArtifactId,
        report: Box<MigrationReportV1>,
    },
    Conflict {
        actual_cursor: Option<CursorV1>,
    },
    PlanChanged,
    Failed {
        diagnostic: Diagnostic,
    },
}

/// Host limits can only tighten ADR 0023's maximums. Each replayed step also uses the
/// ordinary runtime's state/input/call/4096-node bounds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PathLimits {
    pub transitions: usize,
    pub automatic_steps: usize,
    pub object_bytes: usize,
}
impl Default for PathLimits {
    fn default() -> Self {
        Self {
            transitions: 10_000,
            automatic_steps: 100_000,
            object_bytes: 64 * 1024 * 1024,
        }
    }
}
impl PathLimits {
    #[cfg(feature = "trace")]
    pub(crate) fn bounded(self) -> Self {
        let max = Self::default();
        Self {
            transitions: self.transitions.min(max.transitions),
            automatic_steps: self.automatic_steps.min(max.automatic_steps),
            object_bytes: self.object_bytes.min(max.object_bytes),
        }
    }
}

#[cfg(feature = "trace")]
pub(crate) fn differing_fields(a: &State, b: &State) -> Vec<String> {
    let mut fields = BTreeMap::new();
    fields.insert("shared", a.shared != b.shared);
    fields.insert("next_instance", a.next_instance != b.next_instance);
    fields.insert("finished", a.finished != b.finished);
    // Compare overlay execution structure below; its content references are presentation.
    fields.insert("frames", a.frames.len() != b.frames.len());
    for (x, y) in a.frames.iter().zip(&b.frames) {
        for (name, different) in [
            (
                "frame_location",
                x.graph != y.graph || x.node != y.node || x.at != y.at || x.instance != y.instance,
            ),
            ("parameters", x.parameters != y.parameters),
            ("locals", x.locals != y.locals),
            (
                "overlay",
                semantic_overlay(&x.overlay) != semantic_overlay(&y.overlay),
            ),
        ] {
            *fields.entry(name).or_default() |= different;
        }
    }
    fields
        .into_iter()
        .filter(|(_, changed)| *changed)
        .map(|(name, _)| name.to_owned())
        .collect()
}
#[cfg(feature = "trace")]
fn semantic_overlay(overlay: &crate::Overlay) -> crate::Overlay {
    let mut result = overlay.clone();
    for passage in result.nodes.values_mut() {
        passage.title = None;
        passage.body = None;
        passage.args.clear();
        for point in &mut passage.choice_points {
            point.placement = None;
            for option in &mut point.options {
                strip_option(option);
            }
        }
    }
    for options in result.options.values_mut() {
        for option in options {
            strip_option(option);
        }
    }
    result
}
#[cfg(feature = "trace")]
fn strip_option(option: &mut crate::plan::OptionPlan) {
    option.label = None;
    option.reason = None;
    if let crate::plan::Outcome::Local { reply, rejoin } = &mut option.outcome {
        *reply = None;
        *rejoin = None;
    }
}
