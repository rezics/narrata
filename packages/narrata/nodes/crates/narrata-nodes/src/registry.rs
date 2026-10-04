use std::collections::BTreeMap;

use crate::{Error, Result, SourcePlan};

/// The semantic revision of the built-in `narrata.*` node types (ADR 0013 §4).
pub const GAMEBOOK_REVISION: &str = "2";

/// An authoring extension lowers to a plan that is subsequently checked in its graph context.
/// Compilers are trusted build-time code, not callbacks executed by the story runtime.
pub type NodeCompiler = fn(&serde_json::Value) -> Result<SourcePlan>;

#[derive(Clone)]
struct Registration {
    revision: String,
    compiler: NodeCompiler,
}

#[derive(Clone, Default)]
pub struct NodeRegistry {
    entries: BTreeMap<String, Registration>,
}

pub(crate) fn valid_type_id(id: &str) -> bool {
    id.len() <= 128 && id.split('.').all(crate::valid_name)
}

impl NodeRegistry {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn gamebook() -> Self {
        let mut entries = BTreeMap::new();
        for (name, compiler) in [
            ("passage", passage as NodeCompiler),
            ("branch", branch as NodeCompiler),
            ("mutate", mutate as NodeCompiler),
            ("call", call as NodeCompiler),
            ("return", return_node as NodeCompiler),
        ] {
            entries.insert(
                format!("narrata.{name}"),
                Registration {
                    revision: GAMEBOOK_REVISION.into(),
                    compiler,
                },
            );
        }
        Self { entries }
    }

    pub fn register(&mut self, id: &str, revision: &str, compiler: NodeCompiler) -> Result<()> {
        if !valid_type_id(id) || revision.is_empty() || revision.len() > 64 {
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
    ) -> Result<(SourcePlan, String)> {
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

/// The built-in types take their plan's fields as data; the kind comes from `type_id`.
pub fn lower_builtin(kind: &str, data: &serde_json::Value) -> Result<SourcePlan> {
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
    serde_json::from_value(serde_json::Value::Object(fields))
        .map_err(|e| Error::new("schema", "data", e.to_string()))
}

fn passage(v: &serde_json::Value) -> Result<SourcePlan> {
    lower_builtin("passage", v)
}
fn branch(v: &serde_json::Value) -> Result<SourcePlan> {
    lower_builtin("branch", v)
}
fn mutate(v: &serde_json::Value) -> Result<SourcePlan> {
    lower_builtin("mutate", v)
}
fn call(v: &serde_json::Value) -> Result<SourcePlan> {
    lower_builtin("call", v)
}
fn return_node(v: &serde_json::Value) -> Result<SourcePlan> {
    lower_builtin("return", v)
}
