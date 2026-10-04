//! R1 saves are read only to migrate them (ADR 0013 §10); R1 itself never runs. Each kept R1
//! commit's action is mapped through the name table to an Input and replayed on the R2
//! artifact migrated from the save's own R1 artifact, and every resulting state is compared
//! with the R1 snapshot.

use std::{collections::BTreeMap, sync::Arc};

use narrata_kernel::content::ContentRef;
use serde::Deserialize;

use crate::{
    CommitId, Error, ExecutionId, Program, Result, Scalar, Session,
    json::parse_json_limited,
    plan::{GraphRef, NameTable, Plan},
    source::R1ArtifactId,
    state::State,
};

pub const R1_FORMAT_VERSION: u16 = 1;
const MAX_R1_SAVE_BYTES: usize = 4 * 1024 * 1024;
const MAX_R1_COMMITS: usize = 512;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Save {
    format_version: u16,
    artifact_id: String,
    commits: Vec<SavedCommit>,
    cursor: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedCommit {
    id: String,
    parent: Option<String>,
    action: Option<String>,
    snapshot: Snapshot,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    shared: BTreeMap<String, Value>,
    frames: Vec<Frame>,
    next_instance: u32,
    finished: Option<(Address, u32, String)>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Frame {
    graph: GraphRef,
    node: String,
    instance: u32,
    parameters: BTreeMap<String, Value>,
    locals: BTreeMap<String, Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Address {
    package: String,
    graph: String,
    node: String,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Value {
    Bool(bool),
    Int(i64),
    Text(String),
}

/// Resolves a `ref` value to text, so that an R1 text value can be compared with the `ref`
/// that replaced it.
pub type RefText<'a> = &'a dyn Fn(&ContentRef) -> Option<String>;

/// The R1 artifact a save belongs to.
pub fn save_artifact(save: &str) -> Result<R1ArtifactId> {
    let save: Save = parse_json_limited(save, MAX_R1_SAVE_BYTES)?;
    R1ArtifactId::parse(&save.artifact_id).ok_or_else(|| {
        Error::new(
            "schema",
            "artifact_id",
            "R1 artifact IDs are 64 hexadecimal digits",
        )
    })
}

/// Rebuilds an R1 save's whole commit tree as an R2 session and maps its cursor.
pub fn migrate_save(
    program: Arc<Program>,
    names: &NameTable,
    save: &str,
    execution: ExecutionId,
    ref_text: RefText<'_>,
) -> Result<Session> {
    let save: Save = parse_json_limited(save, MAX_R1_SAVE_BYTES)?;
    if save.format_version != R1_FORMAT_VERSION {
        return Err(Error::new("version", "format_version", "not an R1 save"));
    }
    let artifact = R1ArtifactId::parse(&save.artifact_id);
    if artifact.is_none() || artifact != names.migrated_from_r1 {
        return Err(Error::new(
            "incompatible_save",
            "artifact_id",
            "the save's R1 artifact is not the one this artifact was migrated from",
        ));
    }
    if save.commits.is_empty() || save.commits.len() > MAX_R1_COMMITS {
        return Err(Error::new("limit", "commits", "expected 1..512 R1 commits"));
    }
    let mut session = Session::new(program.clone(), execution)?;
    let mut mapped = BTreeMap::<&str, CommitId>::new();
    let compare = Compare {
        program: &program,
        names,
        ref_text,
    };
    for (index, commit) in save.commits.iter().enumerate() {
        let path = format!("commits[{index}]");
        let id = match (&commit.parent, &commit.action) {
            (None, None) if index == 0 => session.cursor()?,
            (Some(parent), Some(action)) => {
                let parent = *mapped.get(parent.as_str()).ok_or_else(|| {
                    Error::new("save", &path, "a parent commit precedes its children")
                })?;
                session.checkout(&parent)?;
                let state = session.state()?;
                let frame = state
                    .frames
                    .last()
                    .ok_or_else(|| Error::new("migration", &path, "the parent has ended"))?;
                let at = frame
                    .at
                    .ok_or_else(|| Error::new("migration", &path, "the parent waits nowhere"))?;
                let graph = program.graph(&frame.graph)?;
                let Some(Plan::Passage(passage)) = graph.nodes.get(&frame.node) else {
                    return Err(Error::new(
                        "migration",
                        &path,
                        "the parent waits outside a passage",
                    ));
                };
                let option = passage
                    .choice_points
                    .iter()
                    .find(|point| point.id == at)
                    .and_then(|point| {
                        point.options.iter().find(|option| {
                            names.option(&frame.graph, &option.id) == Some(action.as_str())
                        })
                    })
                    .ok_or_else(|| {
                        Error::new(
                            "migration",
                            &path,
                            format!("no option is named {action} at the current choice point"),
                        )
                    })?;
                session.choose(&parent, at, vec![option.id])?
            }
            _ => return Err(Error::new("save", &path, "malformed R1 commit")),
        };
        compare.state(
            &commit.snapshot,
            session.state()?,
            &format!("{path}.snapshot"),
        )?;
        if mapped.insert(commit.id.as_str(), id).is_some() {
            return Err(Error::new("duplicate", &path, "duplicate R1 commit"));
        }
    }
    let cursor = mapped
        .get(save.cursor.as_str())
        .ok_or_else(|| Error::new("save", "cursor", "cursor is not a kept commit"))?;
    session.checkout(cursor)?;
    Ok(session)
}

struct Compare<'a> {
    program: &'a Program,
    names: &'a NameTable,
    ref_text: RefText<'a>,
}

impl Compare<'_> {
    fn value(&self, r1: &Value, r2: &Scalar) -> bool {
        match (r1, r2) {
            (Value::Bool(a), Scalar::Bool(b)) => a == b,
            (Value::Int(a), Scalar::Int(b)) => a == b,
            (Value::Text(a), Scalar::Text(b)) => a == b,
            // A text name the migrated work turned into a translatable reference.
            (Value::Text(a), Scalar::Ref(b)) => (self.ref_text)(b).as_deref() == Some(a.as_str()),
            _ => false,
        }
    }

    /// R1 variables must match; variables only the R2 work declares must still hold their
    /// initial values, which R1 play cannot have changed.
    fn variables(
        &self,
        r1: &BTreeMap<String, Value>,
        r2: &BTreeMap<String, Scalar>,
        initial: &BTreeMap<String, Scalar>,
        path: &str,
    ) -> Result<()> {
        for (name, value) in r1 {
            if !r2.get(name).is_some_and(|actual| self.value(value, actual)) {
                return Err(mismatch(format!("{path}.{name}")));
            }
        }
        for (name, value) in r2.iter().filter(|(name, _)| !r1.contains_key(*name)) {
            if initial.get(name) != Some(value) {
                return Err(mismatch(format!("{path}.{name}")));
            }
        }
        Ok(())
    }

    fn state(&self, r1: &Snapshot, r2: &State, path: &str) -> Result<()> {
        let product = &self.program.manifest().product;
        let shared_initial = product
            .shared
            .iter()
            .map(|(name, variable)| (name.clone(), variable.value.clone()))
            .collect();
        self.variables(
            &r1.shared,
            &r2.shared,
            &shared_initial,
            &format!("{path}.shared"),
        )?;
        if r1.next_instance != r2.next_instance {
            return Err(mismatch(format!("{path}.next_instance")));
        }
        match (&r1.finished, &r2.finished) {
            (None, None) => {}
            (Some((address, instance, outcome)), Some(finished)) => {
                let entry = &product.entry;
                if address.package != entry.package
                    || address.graph != entry.graph
                    || self.names.node(entry, &finished.node) != Some(address.node.as_str())
                    || *instance != finished.instance
                    || *outcome != finished.outcome
                {
                    return Err(mismatch(format!("{path}.finished")));
                }
            }
            _ => return Err(mismatch(format!("{path}.finished"))),
        }
        if r1.frames.len() != r2.frames.len() {
            return Err(mismatch(format!("{path}.frames")));
        }
        for (index, (old, new)) in r1.frames.iter().zip(&r2.frames).enumerate() {
            let path = format!("{path}.frames[{index}]");
            if old.graph != new.graph
                || self.names.node(&new.graph, &new.node) != Some(old.node.as_str())
                || old.instance != new.instance
            {
                return Err(mismatch(path));
            }
            let graph = self.program.graph(&new.graph)?;
            self.variables(
                &old.parameters,
                &new.parameters,
                &BTreeMap::new(),
                &format!("{path}.parameters"),
            )?;
            self.variables(
                &old.locals,
                &new.locals,
                &graph.header.locals,
                &format!("{path}.locals"),
            )?;
        }
        Ok(())
    }
}

fn mismatch(path: String) -> Error {
    Error::new(
        "migration",
        path,
        "the replayed R2 state differs from the R1 snapshot",
    )
}
