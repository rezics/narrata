use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Scalar {
    Bool(bool),
    Int(i64),
    Text(String),
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScalarType {
    Bool,
    Int,
    Text,
}

impl Scalar {
    pub fn kind(&self) -> ScalarType {
        match self {
            Self::Bool(_) => ScalarType::Bool,
            Self::Int(_) => ScalarType::Int,
            Self::Text(_) => ScalarType::Text,
        }
    }

    pub fn display(&self) -> String {
        match self {
            Self::Bool(v) => v.to_string(),
            Self::Int(v) => v.to_string(),
            Self::Text(v) => v.clone(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GraphRef {
    pub package: String,
    pub graph: String,
}

impl GraphRef {
    pub fn label(&self) -> String {
        format!("{}.{}", self.package, self.graph)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ImportRef {
    pub package: String,
    pub graph: String,
    pub port: String,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Bundle {
    pub format_version: u16,
    pub product: Product,
    pub packages: BTreeMap<String, NarrativePackage>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Product {
    pub id: String,
    pub title: String,
    pub entry: GraphRef,
    #[serde(default)]
    pub arguments: BTreeMap<String, Scalar>,
    #[serde(default)]
    pub shared: BTreeMap<String, Scalar>,
    #[serde(default)]
    pub bindings: Vec<Binding>,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub from: ImportRef,
    pub to: GraphRef,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NarrativePackage {
    pub id: String,
    pub version: String,
    pub exports: Vec<String>,
    #[serde(default)]
    pub content: BTreeMap<String, Content>,
    pub graphs: BTreeMap<String, Graph>,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Content {
    pub title: String,
    pub paragraphs: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Graph {
    pub title: String,
    #[serde(default)]
    pub parameters: BTreeMap<String, ScalarType>,
    #[serde(default)]
    pub locals: BTreeMap<String, Scalar>,
    #[serde(default)]
    pub shared: BTreeMap<String, ScalarType>,
    #[serde(default)]
    pub imports: BTreeMap<String, Signature>,
    pub outcomes: Vec<String>,
    pub entry: String,
    pub nodes: BTreeMap<String, NodeDefinition>,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Signature {
    #[serde(default)]
    pub parameters: BTreeMap<String, ScalarType>,
    pub outcomes: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NodeDefinition {
    pub type_id: String,
    pub data: serde_json::Value,
}

/// Checked lowering vocabulary. This is not the extensible authoring node-type registry.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum NodePlan {
    Content {
        content: String,
        label: String,
        next: String,
    },
    Decision {
        content: String,
        choices: Vec<Choice>,
    },
    Branch {
        condition: Expr,
        when_true: String,
        when_false: String,
    },
    Mutate {
        assignments: Vec<Assignment>,
        next: String,
    },
    Call {
        target: CallTarget,
        arguments: BTreeMap<String, Expr>,
        on_return: BTreeMap<String, String>,
    },
    Return {
        outcome: String,
    },
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Choice {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub visible_if: Option<Expr>,
    #[serde(default)]
    pub enabled_if: Option<Expr>,
    #[serde(default)]
    pub disabled_reason: Option<String>,
    #[serde(default)]
    pub assignments: Vec<Assignment>,
    pub target: String,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CallTarget {
    Local { graph: String },
    Import { port: String },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    Parameter,
    Local,
    Shared,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Variable {
    pub scope: Scope,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Assignment {
    pub target: Variable,
    pub value: Expr,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Expr {
    Literal {
        value: Scalar,
    },
    Read {
        scope: Scope,
        name: String,
    },
    Not {
        value: Box<Expr>,
    },
    Binary {
        op: BinaryOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BinaryOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Add,
    Sub,
    And,
    Or,
}

/// The filesystem manifest is resolved by tooling, never by the deterministic runtime.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectManifest {
    pub format_version: u16,
    pub product: Product,
    pub packages: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CompositionLock {
    pub format_version: u16,
    pub artifact_id: String,
    pub packages: BTreeMap<String, PackageLock>,
    pub node_types: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PackageLock {
    pub id: String,
    pub version: String,
    pub digest: String,
}
