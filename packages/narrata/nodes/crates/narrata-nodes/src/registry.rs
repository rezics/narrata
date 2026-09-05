use std::collections::BTreeMap;

use crate::{Error, NodePlan, Result};

/// An authoring extension lowers to a plan that is subsequently checked in its graph context.
/// Compilers are trusted build-time code, not callbacks executed by the story runtime.
pub type NodeCompiler = fn(&serde_json::Value) -> Result<NodePlan>;

#[derive(Clone)]
struct Registration {
    revision: String,
    compiler: NodeCompiler,
}

#[derive(Clone, Default)]
pub struct NodeRegistry {
    entries: BTreeMap<String, Registration>,
}

impl NodeRegistry {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn gamebook() -> Self {
        let mut entries = BTreeMap::new();
        for (name, compiler) in [
            ("content", content as NodeCompiler),
            ("decision", decision as NodeCompiler),
            ("branch", branch as NodeCompiler),
            ("mutate", mutate as NodeCompiler),
            ("call", call as NodeCompiler),
            ("return", return_node as NodeCompiler),
        ] {
            entries.insert(
                format!("narrata.{name}"),
                Registration {
                    revision: "1".into(),
                    compiler,
                },
            );
        }
        Self { entries }
    }

    pub fn register(&mut self, id: &str, revision: &str, compiler: NodeCompiler) -> Result<()> {
        if id.len() > 128
            || !id.split('.').all(crate::compile::valid_name)
            || revision.is_empty()
            || revision.len() > 64
        {
            return Err(Error::new(
                "identifier",
                "node_types",
                "invalid type identifier or semantic revision",
            ));
        }
        if self.entries.contains_key(id) {
            return Err(Error::new(
                "duplicate",
                "node_types",
                format!("node type {id} is already registered"),
            ));
        }
        self.entries.insert(
            id.into(),
            Registration {
                revision: revision.into(),
                compiler,
            },
        );
        Ok(())
    }

    pub(crate) fn lower(
        &self,
        id: &str,
        data: &serde_json::Value,
        path: &str,
    ) -> Result<(NodePlan, String)> {
        let entry = self.entries.get(id).ok_or_else(|| {
            Error::new(
                "node_type",
                path,
                format!("node type {id} is not installed"),
            )
        })?;
        (entry.compiler)(data)
            .map(|plan| (plan, entry.revision.clone()))
            .map_err(|e| Error::new(&e.code, path, e.message))
    }
}

fn lower(kind: &str, data: &serde_json::Value) -> Result<NodePlan> {
    let mut fields = data
        .as_object()
        .cloned()
        .ok_or_else(|| Error::new("schema", "data", "node data must be an object"))?;
    if fields.contains_key("kind") {
        return Err(Error::new(
            "schema",
            "data.kind",
            "kind is defined by type_id",
        ));
    }
    fields.insert("kind".into(), serde_json::Value::String(kind.into()));
    if kind == "content" {
        fields
            .entry("label")
            .or_insert_with(|| serde_json::Value::String("继续".into()));
    }
    if kind == "mutate" {
        fields
            .entry("assignments")
            .or_insert_with(|| serde_json::json!([]));
    }
    if kind == "call" {
        fields
            .entry("arguments")
            .or_insert_with(|| serde_json::json!({}));
    }
    serde_json::from_value(serde_json::Value::Object(fields))
        .map_err(|e| Error::new("schema", "data", e.to_string()))
}

fn content(v: &serde_json::Value) -> Result<NodePlan> {
    lower("content", v)
}
fn decision(v: &serde_json::Value) -> Result<NodePlan> {
    lower("decision", v)
}
fn branch(v: &serde_json::Value) -> Result<NodePlan> {
    lower("branch", v)
}
fn mutate(v: &serde_json::Value) -> Result<NodePlan> {
    lower("mutate", v)
}
fn call(v: &serde_json::Value) -> Result<NodePlan> {
    lower("call", v)
}
fn return_node(v: &serde_json::Value) -> Result<NodePlan> {
    lower("return", v)
}
