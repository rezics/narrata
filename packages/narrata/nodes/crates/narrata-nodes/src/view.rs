//! The read model exchanged with hosts (ADR 0013 §7). It carries references and values, never
//! text; aliases appear only when a name table is loaded. It is not part of any digest.

use std::collections::BTreeMap;

use narrata_kernel::content::{ContentRef, Segment};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    ArtifactId, ChoicePointId, CommitId, Diagnostic, ExecutionId, NodeId, OptionId, ViewScalar,
    analysis::GraphAnalysis, plan::GraphRef,
};

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionView {
    pub artifact_id: ArtifactId,
    pub execution: ExecutionId,
    pub cursor: CommitId,
    pub depth: u64,
    pub product: ProductView,
    /// What to show since the previous interaction, recomputed from the parent state and the
    /// input rather than stored.
    pub presentation: Vec<PresentationItem>,
    pub interaction: Interaction,
    pub shared: Vec<VariableView>,
    pub frames: Vec<FrameView>,
    pub history: Vec<HistoryView>,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProductView {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<ContentRef>,
}

/// One item to present. Together with the execution and the commit, `occurrence` forms the
/// presentation key that generative content providers cache by.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PresentationItem {
    pub occurrence: u32,
    pub role: Role,
    pub node: NodeId,
    pub content: Presented,
    /// The passage's named arguments when the item was presented.
    pub args: BTreeMap<String, ViewScalar>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Title,
    Body,
    Reply,
}

/// A title is a reference; body and reply items are segments.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Presented {
    Segment(Segment),
    Ref(ContentRef),
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Interaction {
    Choose {
        graph: GraphRef,
        node: NodeId,
        choice_point: ChoicePointId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        key: Option<String>,
        min: u16,
        max: u16,
        /// Arguments for option labels and disabled reasons.
        args: BTreeMap<String, ViewScalar>,
        /// Visible options in order; hidden ones are left out.
        options: Vec<OptionView>,
    },
    Finished {
        outcome: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<ContentRef>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        body: Option<Segment>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OptionView {
    pub id: OptionId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<ContentRef>,
    pub enabled: bool,
    /// Only for a disabled option that declares a reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<ContentRef>,
    pub outcome: OutcomeKind,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeKind {
    Local,
    Branch,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VariableView {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<ContentRef>,
    pub value: ViewScalar,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FrameView {
    pub graph: GraphRef,
    pub node: NodeId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<ChoicePointId>,
    pub instance: u32,
    pub parameters: Vec<VariableView>,
    pub locals: Vec<VariableView>,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryView {
    pub id: CommitId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<CommitId>,
    pub depth: u64,
    /// Where the commit waits, or the node that finished the story.
    pub graph: GraphRef,
    pub node: NodeId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    pub instance: u32,
    /// The waiting passage's title, or the ending's title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<ContentRef>,
    pub finished: bool,
    pub current: bool,
}

/// What the browser reader receives: the session view, the reading page of the current
/// passage, and the text-free graph analysis.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BookView {
    pub view: SessionView,
    /// Items since the current passage was entered, across the local choices made in it.
    pub page: Vec<PresentationItem>,
    pub graphs: Vec<GraphAnalysis>,
    pub diagnostics: Vec<Diagnostic>,
}
