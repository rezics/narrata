//! A small R2 project that exercises every node kind, local and branch outcomes, a
//! multi-select choice point, an import binding and a `ref` parameter.

#![allow(dead_code, clippy::panic, clippy::unwrap_used)]

use std::sync::Arc;

use narrata_nodes::{
    Compilation, ExecutionId, NodeRegistry, OptionId, PackageSource, ProjectManifest,
    ProjectSource, Session, compile, plan::NameTable,
};
use serde_json::{Value, json};

pub fn node(n: u32) -> String {
    format!("node:{n:032x}")
}

pub fn point(n: u32) -> String {
    format!("choice-point:{n:032x}")
}

pub fn option(n: u32) -> String {
    format!("option:{n:032x}")
}

pub fn content(key: &str) -> Value {
    json!({"provider": "local", "key": key})
}

pub fn segment(unit: &str, first: &str, last: &str) -> Value {
    json!({"unit": content(unit), "first": first, "last": last})
}

pub fn read(scope: &str, name: &str) -> Value {
    json!({"kind": "read", "scope": scope, "name": name})
}

pub fn literal(value: Value) -> Value {
    json!({"kind": "literal", "value": value})
}

pub fn add(scope: &str, name: &str, amount: i64) -> Value {
    json!({
        "target": {"scope": scope, "name": name},
        "value": {"kind": "binary", "op": "add", "left": read(scope, name), "right": literal(json!(amount))}
    })
}

/// The project manifest of the fixture.
pub fn manifest() -> Value {
    json!({
        "format_version": 2,
        "product": {
            "id": "trial",
            "title": content("product.title"),
            "entry": {"package": "main", "graph": "start"},
            "shared": {"coins": 0, "lamp": false},
            "shared_labels": {"coins": content("shared.coins")},
            "endings": {
                "done": {"title": content("ending.done"), "body": segment("ending", "e1", "e1")},
                "lost": {"title": content("ending.lost")}
            },
            "bindings": [{
                "from": {"package": "main", "graph": "start", "port": "helper"},
                "to": {"package": "side", "graph": "visit"}
            }]
        },
        "packages": {"main": "main.json", "side": "side.json"}
    })
}

/// The entry passage: a local choice, a multi-select with `min = 0`, then a final branch.
fn gate() -> Value {
    json!({
        "title": content("main.gate.title"),
        "body": segment("main.gate", "b1", "b4"),
        "args": {"coins": read("shared", "coins")},
        "choice_points": [
            {
                "id": point(1),
                "key": "greet",
                "placement": "b1",
                "options": [
                    {
                        "id": option(1),
                        "key": "wave",
                        "label": content("main.gate.wave"),
                        "effects": [add("shared", "coins", 1)],
                        "outcome": {"kind": "local", "reply": segment("main.gate", "r-wave", "r-wave"), "rejoin": "b2"}
                    },
                    {
                        "id": option(2),
                        "key": "nod",
                        "label": content("main.gate.nod"),
                        "outcome": {"kind": "local", "reply": segment("main.gate", "r-nod", "r-nod"), "rejoin": "b2"}
                    }
                ]
            },
            {
                "id": point(2),
                "key": "pack",
                "placement": "b2",
                "min": 0,
                "max": 2,
                "options": [
                    {
                        "id": option(3),
                        "key": "rope",
                        "label": content("main.gate.rope"),
                        "effects": [add("shared", "coins", 1)],
                        "outcome": {"kind": "local", "reply": segment("main.gate", "r-rope", "r-rope"), "rejoin": "b3"}
                    },
                    {
                        "id": option(4),
                        "key": "lamp",
                        "label": content("main.gate.lamp"),
                        "effects": [{"target": {"scope": "shared", "name": "lamp"}, "value": literal(json!(true))}],
                        "outcome": {"kind": "local", "reply": segment("main.gate", "r-lamp", "r-lamp"), "rejoin": "b3"}
                    },
                    {
                        "id": option(5),
                        "key": "map",
                        "label": content("main.gate.map"),
                        "enabled_if": {"kind": "binary", "op": "ge", "left": read("shared", "coins"), "right": literal(json!(5))},
                        "reason": content("main.gate.map.reason"),
                        "outcome": {"kind": "local", "rejoin": "b3"}
                    }
                ]
            },
            {
                "id": point(3),
                "key": "route",
                "options": [
                    {
                        "id": option(6),
                        "key": "left",
                        "label": content("main.gate.left"),
                        "outcome": {"kind": "branch", "target": "visit"}
                    },
                    {
                        "id": option(7),
                        "key": "right",
                        "label": content("main.gate.right"),
                        "outcome": {"kind": "branch", "target": "count"}
                    }
                ]
            }
        ]
    })
}

/// The entry package: a passage with a local choice, a multi-select and a final branch.
pub fn main_package() -> Value {
    json!({
        "id": "trial.main",
        "version": "1.0.0",
        "exports": ["start"],
        "graphs": {
            "start": {
                "title": content("main.start.title"),
                "shared": {"coins": "int", "lamp": "bool"},
                "locals": {"visits": 0},
                "imports": {"helper": {"parameters": {"visitor": "ref"}, "outcomes": ["back"]}},
                "outcomes": ["done", "lost"],
                "entry": "gate",
                "nodes": {
                    "gate": {
                        "id": node(1),
                        "type_id": "narrata.passage",
                        "data": gate()
                    },
                    "visit": {
                        "id": node(2),
                        "type_id": "narrata.call",
                        "data": {
                            "target": {"kind": "import", "port": "helper"},
                            "arguments": {"visitor": literal(content("main.visitor"))},
                            "on_return": {"back": "tally"}
                        }
                    },
                    "tally": {
                        "id": node(3),
                        "type_id": "narrata.mutate",
                        "data": {"assignments": [add("local", "visits", 1)], "next": "count"}
                    },
                    "count": {
                        "id": node(4),
                        "type_id": "narrata.branch",
                        "data": {
                            "condition": {"kind": "binary", "op": "ge", "left": read("shared", "coins"), "right": literal(json!(2))},
                            "when_true": "won",
                            "when_false": "lost"
                        }
                    },
                    "won": {"id": node(5), "type_id": "narrata.return", "data": {"outcome": "done"}},
                    "lost": {"id": node(6), "type_id": "narrata.return", "data": {"outcome": "lost"}}
                }
            }
        }
    })
}

/// The imported package: a passage formatted with a `ref` parameter.
pub fn side_package() -> Value {
    json!({
        "id": "trial.side",
        "version": "1.0.0",
        "exports": ["visit"],
        "graphs": {
            "visit": {
                "parameters": {"visitor": "ref"},
                "outcomes": ["back"],
                "entry": "talk",
                "nodes": {
                    "talk": {
                        "id": node(11),
                        "type_id": "narrata.passage",
                        "data": {
                            "title": content("side.talk.title"),
                            "body": {"unit": content("side.talk")},
                            "args": {"visitor": read("parameter", "visitor")},
                            "choice_points": [{
                                "id": point(11),
                                "key": "choice",
                                "options": [{
                                    "id": option(11),
                                    "key": "continue",
                                    "outcome": {"kind": "branch", "target": "out"}
                                }]
                            }]
                        }
                    },
                    "out": {"id": node(12), "type_id": "narrata.return", "data": {"outcome": "back"}}
                }
            }
        }
    })
}

/// Compiles a variant of the fixture produced by `change`, which edits the manifest and the
/// two package documents.
pub fn try_variant(
    change: impl FnOnce(&mut Value, &mut Value, &mut Value),
) -> narrata_nodes::Result<Compilation> {
    let (mut manifest, mut main, mut side) = (manifest(), main_package(), side_package());
    change(&mut manifest, &mut main, &mut side);
    let schema =
        |path: &str, e: serde_json::Error| narrata_nodes::Error::new("schema", path, e.to_string());
    let manifest: ProjectManifest =
        serde_json::from_value(manifest).map_err(|e| schema("manifest", e))?;
    let main: PackageSource = serde_json::from_value(main).map_err(|e| schema("main", e))?;
    let side: PackageSource = serde_json::from_value(side).map_err(|e| schema("side", e))?;
    compile(
        &ProjectSource {
            manifest,
            packages: [("main".to_owned(), main), ("side".to_owned(), side)].into(),
        },
        &NodeRegistry::gamebook(),
    )
}

/// The error code of a variant that must not compile.
pub fn rejected(change: impl FnOnce(&mut Value, &mut Value, &mut Value)) -> String {
    match try_variant(change) {
        Ok(_) => panic!("the variant compiled"),
        Err(error) => error.code,
    }
}

pub fn compiled() -> Compilation {
    try_variant(|_, _, _| {}).unwrap()
}

pub fn execution() -> ExecutionId {
    "execution:0190f2a0000070008000000000000001"
        .parse()
        .unwrap()
}

pub fn open(pack: &[u8]) -> (Arc<narrata_nodes::Program>, NameTable) {
    let (program, names) = narrata_nodes::Program::from_pack(pack).unwrap();
    (Arc::new(program), names.unwrap())
}

pub fn session(compilation: &Compilation) -> (Session, NameTable) {
    let (program, names) = open(&compilation.pack);
    let session = Session::new(program, execution()).unwrap();
    (session, names)
}

/// Chooses the options with the given keys at the current choice point.
pub fn choose(
    session: &mut Session,
    names: &NameTable,
    keys: &[&str],
) -> narrata_nodes::Result<()> {
    let view = session.view(Some(names))?;
    let narrata_nodes::view::Interaction::Choose {
        choice_point,
        options,
        ..
    } = view.interaction
    else {
        panic!("no choice at {}", view.cursor);
    };
    let chosen: Vec<OptionId> = keys
        .iter()
        .map(|key| {
            options
                .iter()
                .find(|option| option.key.as_deref() == Some(*key))
                .map(|option| option.id)
                .unwrap_or_else(|| panic!("no option {key}"))
        })
        .collect();
    session.choose(&view.cursor, choice_point, chosen)?;
    Ok(())
}

pub fn at<'a>(value: &'a mut Value, pointer: &str) -> &'a mut Value {
    value
        .pointer_mut(pointer)
        .unwrap_or_else(|| panic!("no {pointer}"))
}

/// Sets `field` of the object at `pointer`.
pub fn set(value: &mut Value, pointer: &str, field: &str, new: Value) {
    at(value, pointer)
        .as_object_mut()
        .unwrap_or_else(|| panic!("{pointer} is not an object"))
        .insert(field.into(), new);
}

pub const GATE: &str = "/graphs/start/nodes/gate/data";

/// Rebuilds the old container only to exercise its read-only import path.
pub fn legacy_export(session: &Session) -> narrata_nodes::SessionExport {
    let mut seen = std::collections::BTreeSet::new();
    let mut objects = Vec::new();
    for (_, commit) in session.commits().unwrap() {
        for id in [commit.input, Some(commit.state)].into_iter().flatten() {
            if seen.insert(id) {
                let object = session
                    .history()
                    .reader()
                    .require(narrata_history::ObjectId::from_bytes(*id.as_bytes()), None)
                    .unwrap();
                objects.push(hex::encode(object.bytes()));
            }
        }
        objects.push(hex::encode(commit.envelope()));
    }
    narrata_nodes::SessionExport {
        format_version: 2,
        artifact_id: session.program().artifact_id(),
        execution: session.execution(),
        cursor: session.cursor().unwrap(),
        objects,
    }
}
