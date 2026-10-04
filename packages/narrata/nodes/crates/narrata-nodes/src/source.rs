//! Author source documents (ADR 0013 §1): JSON that rejects duplicate keys and unknown
//! fields. Aliases address edges here; `compile` turns them into IDs and moves them to the
//! name table, so renaming an alias never changes the artifact.

use std::collections::BTreeMap;

use narrata_kernel::content::{AnchorId, ContentRef, Segment};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    Assignment, AuthoredId, ChoicePointId, Expr, NodeId, OptionId, Scalar, ScalarType,
    plan::{CallTarget, GraphRef, ImportRef, Signature},
};

/// Resolved by tooling, never by the deterministic runtime.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectManifest {
    pub format_version: u16,
    pub product: ProductSource,
    /// Package instance alias to a path relative to the manifest.
    pub packages: BTreeMap<String, String>,
    /// The R1 artifact this work was migrated from; recorded in the name table so that R1
    /// saves can only migrate onto it (ADR 0013 §10).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub migrated_from_r1: Option<R1ArtifactId>,
}

/// The 64 lower-case hexadecimal digits of an R1 `artifact_id`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct R1ArtifactId(pub [u8; 32]);

impl JsonSchema for R1ArtifactId {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "R1ArtifactId".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({"type": "string", "pattern": "^[0-9a-f]{64}$"})
    }
}

impl Serialize for R1ArtifactId {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&hex::encode(self.0))
    }
}

impl<'de> Deserialize<'de> for R1ArtifactId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        R1ArtifactId::parse(&text).ok_or_else(|| {
            serde::de::Error::custom("R1 artifact IDs are 64 lower-case hexadecimal digits")
        })
    }
}

impl R1ArtifactId {
    pub fn parse(text: &str) -> Option<Self> {
        let mut bytes = [0_u8; 32];
        (text.len() == 64 && !text.bytes().any(|byte| byte.is_ascii_uppercase()))
            .then(|| hex::decode_to_slice(text, &mut bytes).ok())
            .flatten()
            .map(|()| Self(bytes))
    }
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProductSource {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<ContentRef>,
    pub entry: GraphRef,
    #[serde(default)]
    pub arguments: BTreeMap<String, Scalar>,
    #[serde(default)]
    pub shared: BTreeMap<String, Scalar>,
    /// Display names of shared variables.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub shared_labels: BTreeMap<String, ContentRef>,
    /// How each outcome of the entry graph is shown when the story ends.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub endings: BTreeMap<String, EndingSource>,
    #[serde(default)]
    pub bindings: Vec<Binding>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EndingSource {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<ContentRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<Segment>,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub from: ImportRef,
    pub to: GraphRef,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PackageSource {
    pub id: String,
    pub version: String,
    pub exports: Vec<String>,
    /// Deleted IDs; they are never reused, and the list only grows.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tombstones: Vec<AuthoredId>,
    pub graphs: BTreeMap<String, GraphSource>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GraphSource {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<ContentRef>,
    #[serde(default)]
    pub parameters: BTreeMap<String, ScalarType>,
    #[serde(default)]
    pub locals: BTreeMap<String, Scalar>,
    #[serde(default)]
    pub shared: BTreeMap<String, ScalarType>,
    #[serde(default)]
    pub imports: BTreeMap<String, Signature>,
    pub outcomes: Vec<String>,
    /// Alias of the entry node.
    pub entry: String,
    /// Keyed by node alias.
    pub nodes: BTreeMap<String, NodeSource>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NodeSource {
    /// Minted by `narrata-book ids`; compile rejects a missing ID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<NodeId>,
    pub type_id: String,
    pub data: serde_json::Value,
}

/// What an authoring node type lowers to; every variant is then checked in its graph.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SourcePlan {
    Passage(PassageSource),
    Branch {
        condition: Expr,
        when_true: String,
        when_false: String,
    },
    Mutate {
        #[serde(default)]
        assignments: Vec<Assignment>,
        next: String,
    },
    Call {
        target: CallTarget,
        #[serde(default)]
        arguments: BTreeMap<String, Expr>,
        on_return: BTreeMap<String, String>,
    },
    Return {
        outcome: String,
    },
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PassageSource {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<ContentRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<Segment>,
    /// Named arguments the content provider formats text with.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub args: BTreeMap<String, Expr>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub choice_points: Vec<ChoicePointSource>,
    /// Alias of the node that follows when the passage runs to its end.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ChoicePointSource {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<ChoicePointId>,
    pub key: String,
    /// The body block after which the choice point appears; only the last one may omit it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placement: Option<AnchorId>,
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub min: u16,
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub max: u16,
    /// Reserved for host proposals (ADR 0013 §5); not supported yet.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub proposals: bool,
    pub options: Vec<OptionSource>,
}

fn one() -> u16 {
    1
}

fn is_one(value: &u16) -> bool {
    *value == 1
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OptionSource {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<OptionId>,
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<ContentRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible_if: Option<Expr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled_if: Option<Expr>,
    /// Shown when the option is visible but disabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<ContentRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<Assignment>,
    pub outcome: OutcomeSource,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum OutcomeSource {
    /// Show `reply`, then continue in the same passage from `rejoin`.
    Local {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reply: Option<Segment>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rejoin: Option<AnchorId>,
    },
    /// Enter another node of the same graph, by alias.
    Branch { target: String },
}

/// Written next to the project manifest as `project.lock.json`.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CompositionLock {
    pub format_version: u16,
    pub artifact_id: crate::ArtifactId,
    pub packages: BTreeMap<String, PackageLock>,
    pub node_types: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PackageLock {
    pub id: String,
    pub version: String,
    /// `digest_bytes("narrata.nodes.package-source", 1, canonical JSON)`, hexadecimal.
    pub digest: String,
}

/// A parsed project: the manifest and its package sources by alias.
#[derive(Clone, Debug, PartialEq)]
pub struct ProjectSource {
    pub manifest: ProjectManifest,
    pub packages: BTreeMap<String, PackageSource>,
}
