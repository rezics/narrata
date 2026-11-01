//! The story, scenarios and corpus builder behind `fixtures/compat/vn-dependencies-v1/`.
//!
//! Every file of the corpus is produced from Rust definitions through the same `VnBook` entry
//! the Wasm module exports, so a frozen request or result is something an endpoint really
//! emitted. The ordinary test compares the committed bytes with a fresh build; only the
//! ignored generator writes files, and only into a new directory under `.temp`.

use std::{collections::BTreeMap, path::Path};

use narrata_nodes::{
    NodeRegistry, Pack, PackageSource, Program, ProjectManifest, ProjectSource, compile,
    plan::GraphRef,
};
use serde_json::{Value, json};

use crate::{
    VnBook,
    dependencies::{DependencyRequestV1, DependencyResultV1},
};

pub const EXECUTION: &str = "execution:0190f2a0000070008000000000000001";

fn node(n: u32) -> String {
    format!("node:{n:032x}")
}

fn point(n: u32) -> String {
    format!("choice-point:{n:032x}")
}

fn option(n: u32) -> String {
    format!("option:{n:032x}")
}

fn content(key: &str) -> Value {
    json!({"provider": "host", "key": key})
}

pub fn image(key: &str, revision: &str) -> Value {
    json!({"kind": "image", "provider": "demo", "key": key, "revision": revision})
}

pub fn audio(key: &str, revision: &str) -> Value {
    json!({"kind": "audio", "provider": "demo", "key": key, "revision": revision})
}

fn line(key: &str) -> Value {
    json!({"unit": content(key), "first": "line-start", "last": "line-end"})
}

fn segment(unit: &str, first: &str, last: &str) -> Value {
    json!({"unit": content(unit), "first": first, "last": last})
}

pub fn plan(id: u32, type_id: &str, data: Value) -> Value {
    json!({"id": node(id), "type_id": type_id, "data": data})
}

/// Two lines, a scene that keeps the music, a choice with a hidden and a disabled option, two
/// routes (one through a helper graph that replaces the music) that converge on a scene, a
/// branch to one of two lines, a multi-select, and a final scene that stops the music.
pub fn source() -> Value {
    let manifest = json!({
        "format_version": narrata_nodes::VISUAL_NOVEL_FORMAT_VERSION,
        "product": {
            "id": "vn-deps",
            "entry": {"package": "main", "graph": "start"},
            "shared": {"met": false},
            "endings": {"done": {"title": content("ending.title"), "body": segment("ending.body", "e1", "e1")}}
        },
        "packages": {"main": "main.json"}
    });
    let package = json!({
        "id": "vn-deps.main",
        "version": "1.0.0",
        "exports": ["start"],
        "graphs": {
            "start": {
                "outcomes": ["done"],
                "shared": {"met": "bool"},
                "entry": "open",
                "nodes": {
                    "open": plan(1, "narrata.scene", json!({
                        "background": image("bg/room", "r1"),
                        "layers": [{"id": "hero", "order": 10, "asset": image("portrait/hero", "v1")}],
                        "music": {"kind": "play", "track": audio("music/theme", "a1")},
                        "next": "one"
                    })),
                    "one": plan(2, "narrata.dialogue", json!({
                        "speaker": content("speaker.hero"),
                        "line": line("line.one"),
                        "voice": audio("voice/one", "a1"),
                        "next": "two"
                    })),
                    "two": plan(3, "narrata.dialogue", json!({
                        "speaker": content("speaker.friend"),
                        "line": line("line.two"),
                        "next": "mid"
                    })),
                    "mid": plan(4, "narrata.scene", json!({
                        "background": image("bg/mid", "r1"),
                        "layers": [{"id": "hero", "order": 10, "asset": image("portrait/hero", "v1")}],
                        "music": {"kind": "keep"},
                        "next": "ask"
                    })),
                    "ask": plan(5, "narrata.passage", json!({
                        "title": content("ask.title"),
                        "body": {"unit": content("ask.body")},
                        "choice_points": [{
                            "id": point(1), "key": "route", "options": [
                                {"id": option(1), "key": "left", "label": content("label.left"),
                                 "outcome": {"kind": "branch", "target": "left_fx"}},
                                {"id": option(2), "key": "right", "label": content("label.right"),
                                 "outcome": {"kind": "branch", "target": "right_call"}},
                                {"id": option(3), "key": "secret", "label": content("label.secret"),
                                 "visible_if": {"kind": "literal", "value": false},
                                 "outcome": {"kind": "branch", "target": "secret"}},
                                {"id": option(4), "key": "locked", "label": content("label.locked"),
                                 "enabled_if": {"kind": "literal", "value": false},
                                 "reason": content("reason.locked"),
                                 "outcome": {"kind": "branch", "target": "vault"}}
                            ]
                        }]
                    })),
                    "left_fx": plan(6, "narrata.mutate", json!({
                        "assignments": [{
                            "target": {"scope": "shared", "name": "met"},
                            "value": {"kind": "literal", "value": true}
                        }],
                        "next": "merge"
                    })),
                    "right_call": plan(7, "narrata.call", json!({
                        "target": {"kind": "local", "graph": "helper"},
                        "on_return": {"back": "merge"}
                    })),
                    "secret": plan(8, "narrata.dialogue", json!({"line": line("line.secret"), "next": "finish"})),
                    "vault": plan(9, "narrata.dialogue", json!({"line": line("line.vault"), "next": "finish"})),
                    "merge": plan(10, "narrata.scene", json!({
                        "background": image("bg/hall", "r2"),
                        "layers": [
                            {"id": "hero", "order": 10, "asset": image("portrait/hero-alert", "v2")},
                            {"id": "friend", "order": 20, "asset": image("portrait/friend", "v3")}
                        ],
                        "music": {"kind": "keep"},
                        "next": "after"
                    })),
                    "after": plan(11, "narrata.branch", json!({
                        "condition": {"kind": "read", "scope": "shared", "name": "met"},
                        "when_true": "met",
                        "when_false": "alone"
                    })),
                    "met": plan(12, "narrata.dialogue", json!({
                        "line": line("line.met"),
                        "voice": audio("voice/met", "a2"),
                        "next": "pack"
                    })),
                    "alone": plan(13, "narrata.dialogue", json!({"line": line("line.alone"), "next": "pack"})),
                    "pack": plan(14, "narrata.passage", json!({
                        "body": {"unit": content("pack.body")},
                        "choice_points": [
                            {"id": point(2), "key": "pack", "placement": "b1", "min": 0, "max": 2, "options": [
                                {"id": option(5), "key": "rope", "label": content("label.rope"),
                                 "outcome": {"kind": "local", "reply": segment("pack.body", "r-rope", "r-rope"), "rejoin": "b2"}},
                                {"id": option(6), "key": "lamp", "label": content("label.lamp"),
                                 "outcome": {"kind": "local", "reply": segment("pack.body", "r-lamp", "r-lamp"), "rejoin": "b2"}}
                            ]},
                            {"id": point(3), "key": "go", "placement": "b2", "options": [
                                {"id": option(7), "key": "onward", "label": content("label.onward"),
                                 "outcome": {"kind": "branch", "target": "finale"}}
                            ]}
                        ]
                    })),
                    "finale": plan(15, "narrata.scene", json!({
                        "background": image("bg/end", "r3"),
                        "layers": [],
                        "music": {"kind": "stop"},
                        "next": "last"
                    })),
                    "last": plan(16, "narrata.dialogue", json!({"line": line("line.last"), "next": "finish"})),
                    "finish": plan(17, "narrata.return", json!({"outcome": "done"}))
                }
            },
            "helper": {
                "outcomes": ["back"],
                "entry": "enter",
                "nodes": {
                    "enter": plan(21, "narrata.scene", json!({
                        "background": image("bg/helper", "r1"),
                        "layers": [],
                        "music": {"kind": "play", "track": audio("music/helper", "a9")},
                        "next": "leave"
                    })),
                    "leave": plan(22, "narrata.return", json!({"outcome": "back"}))
                }
            }
        }
    });
    json!({"manifest": manifest, "packages": {"main": package}})
}

pub fn compile_pack(source: &Value) -> Vec<u8> {
    let project = ProjectSource {
        manifest: serde_json::from_value::<ProjectManifest>(source["manifest"].clone()).unwrap(),
        packages: source["packages"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(name, package)| {
                (
                    name.clone(),
                    serde_json::from_value::<PackageSource>(package.clone()).unwrap(),
                )
            })
            .collect(),
    };
    compile(&project, &NodeRegistry::visual_novel())
        .unwrap()
        .pack
}

fn limits(locations: u32, graphs: u32, candidates: u32, references: u32, scans: u32) -> Value {
    json!({
        "locations": locations, "graphs": graphs, "candidates": candidates,
        "references": references, "scans": scans
    })
}

/// How the frozen requests were produced: the steps that bring a fresh session to the
/// interaction, and the queries asked there. A `manifest` scenario restores that interaction
/// onto a book that holds the entry graph's chunk alone, supplying the listed graphs' chunks
/// before the query.
pub fn scenarios() -> Value {
    let default = limits(256, 8, 2048, 512, 1024);
    json!({
        "corpus_version": 1,
        "execution": EXECUTION,
        "scenarios": [
            {
                "name": "advance",
                "mode": "pack",
                "steps": [],
                "queries": [{"name": "default", "limits": default}]
            },
            {
                "name": "choice",
                "mode": "pack",
                "steps": ["advance", "advance"],
                "queries": [
                    {"name": "default", "limits": default},
                    {"name": "few-locations", "limits": limits(3, 8, 2048, 512, 1024)},
                    {"name": "no-extra-graphs", "limits": limits(256, 0, 2048, 512, 1024)}
                ]
            },
            {
                "name": "multiselect",
                "mode": "pack",
                "steps": ["advance", "advance", {"choose": ["left"]}, "advance"],
                "queries": [{"name": "default", "limits": default}]
            },
            {
                "name": "scene-stop",
                "mode": "pack",
                "steps": ["advance", "advance", {"choose": ["left"]}, "advance", {"choose": ["rope", "lamp"]}],
                "queries": [{"name": "default", "limits": default}]
            },
            {
                "name": "cold-chunk",
                "mode": "manifest",
                "steps": ["advance", "advance"],
                "queries": [
                    {"name": "unavailable", "limits": default, "supply": []},
                    {"name": "recovered", "limits": default, "supply": ["main.helper"]}
                ]
            }
        ]
    })
}

/// A hand-listed set of documents a decoder must refuse, each with the error code it must give.
fn rejected(good_request: &str, good_result: &str) -> (Value, Value) {
    let request: Value = serde_json::from_str(good_request).unwrap();
    let result: Value = serde_json::from_str(good_result).unwrap();
    let edit = |base: &Value, change: &dyn Fn(&mut Value)| {
        let mut value = base.clone();
        change(&mut value);
        serde_json::to_string(&value).unwrap()
    };
    let requests = json!([
        {"name": "version-2", "code": "schema", "text": edit(&request, &|v| v["version"] = json!(2))},
        {"name": "version-0", "code": "schema", "text": edit(&request, &|v| v["version"] = json!(0))},
        {"name": "unknown-field", "code": "schema", "text": edit(&request, &|v| v["prefetch"] = json!(true))},
        {"name": "zero-locations", "code": "limit", "text": edit(&request, &|v| v["limits"]["locations"] = json!(0))},
        {"name": "too-many-graphs", "code": "limit", "text": edit(&request, &|v| v["limits"]["graphs"] = json!(65))},
        {"name": "cursor-not-a-commit", "code": "schema", "text": edit(&request, &|v| v["cursor"] = json!("node:00000000000000000000000000000001"))}
    ]);
    let results = json!([
        {"name": "version-2", "code": "schema", "text": edit(&result, &|v| v["version"] = json!(2))},
        {"name": "unknown-media-role", "code": "schema", "text": edit(&result, &|v| v["media"][0]["role"]["kind"] = json!("sound_effect"))},
        {"name": "unknown-truncation", "code": "schema", "text": edit(&result, &|v| v["truncations"] = json!([{"kind": "timeout", "limit": 1}]))},
        {"name": "complete-with-truncation", "code": "dependencies", "text": edit(&result, &|v| v["truncations"] = json!([{"kind": "locations", "limit": v["limits"]["locations"]}]))},
        {"name": "truncation-for-another-limit", "code": "dependencies", "text": edit(&result, &|v| {
            v["complete"] = json!(false);
            v["truncations"] = json!([{"kind": "locations", "limit": 1}]);
        })},
        {"name": "unsorted-content", "code": "dependencies", "text": edit(&result, &|v| v["content"].as_array_mut().unwrap().reverse())},
        {"name": "work-beyond-limits", "code": "dependencies", "text": edit(&result, &|v| v["work"]["scans"] = json!(100_000))},
        {"name": "unknown-field", "code": "schema", "text": edit(&result, &|v| v["bytes"] = json!([1, 2, 3]))}
    ]);
    (requests, results)
}

fn sha256(bytes: &[u8]) -> String {
    narrata_kernel::codec::sha256(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn pretty(value: &Value) -> Vec<u8> {
    let mut text = serde_json::to_string_pretty(value).unwrap();
    text.push('\n');
    text.into_bytes()
}

fn view(book: &VnBook) -> Value {
    serde_json::from_str(&book.inspect().unwrap()).unwrap()
}

fn apply(book: &mut VnBook, step: &Value) {
    let shown = view(book);
    let (cursor, interaction) = (
        shown["view"]["cursor"].as_str().unwrap().to_owned(),
        &shown["view"]["interaction"],
    );
    match step {
        Value::String(name) if name == "advance" => {
            let dialogue = interaction["dialogue"].as_str().unwrap();
            book.advance(&cursor, dialogue).unwrap();
        }
        Value::Object(choose) => {
            let keys: Vec<&str> = choose["choose"]
                .as_array()
                .unwrap()
                .iter()
                .map(|key| key.as_str().unwrap())
                .collect();
            let options = interaction["options"].as_array().unwrap();
            let ids = keys
                .iter()
                .map(|key| {
                    options.iter().find(|option| option["key"] == *key).unwrap()["id"]
                        .as_str()
                        .unwrap()
                        .to_owned()
                })
                .collect();
            let point = interaction["choice_point"].as_str().unwrap();
            book.choose(&cursor, point, ids).unwrap();
        }
        other => panic!("unknown step {other}"),
    }
}

/// The chunk bytes of `graph` ("package.graph") and its object ID.
pub fn chunk_of(pack: &[u8], graph: &str) -> (String, Vec<u8>) {
    let (program, _) = Program::from_pack(pack).unwrap();
    let (package, name) = graph.split_once('.').unwrap();
    let reference = GraphRef {
        package: package.into(),
        graph: name.into(),
    };
    let index = program.manifest().graphs[&reference].chunk as usize;
    (
        program.manifest().chunks[index].to_string(),
        Pack::decode(pack).unwrap().chunks[index].clone(),
    )
}

/// Runs a scenario, returning its `(query name, request, result)` documents.
pub fn run(pack: &[u8], scenario: &Value) -> Vec<(String, String, String)> {
    let mut full = VnBook::new(pack, EXECUTION).unwrap();
    full.memory().unwrap();
    for step in scenario["steps"].as_array().unwrap() {
        apply(&mut full, step);
    }
    let book = if scenario["mode"] == "manifest" {
        let export = full.export().unwrap();
        let mut cold =
            VnBook::from_manifest(&Pack::decode(pack).unwrap().manifest, EXECUTION).unwrap();
        let (id, bytes) = chunk_of(pack, "main.start");
        cold.load_chunk(&id, &bytes).unwrap();
        cold.memory().unwrap();
        cold.restore(&export).unwrap();
        cold
    } else {
        full
    };
    scenario["queries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|query| {
            for graph in query["supply"].as_array().into_iter().flatten() {
                let (id, bytes) = chunk_of(pack, graph.as_str().unwrap());
                book.load_chunk(&id, &bytes).unwrap();
            }
            let limits: crate::dependencies::DependencyLimitsV1 =
                serde_json::from_value(query["limits"].clone()).unwrap();
            let request = serde_json::to_string(&json!({
                "version": 1,
                "cursor": book.cursor().unwrap(),
                "limits": limits,
            }))
            .unwrap();
            // The Wasm entry's own checked decode must accept what it is given and emit.
            DependencyRequestV1::decode(&request).unwrap();
            let result = book.dependency_result(&request).unwrap();
            DependencyResultV1::decode(&result).unwrap();
            (query["name"].as_str().unwrap().to_owned(), request, result)
        })
        .collect()
}

/// Every file of the corpus, by name, as the generator writes it.
pub fn build() -> BTreeMap<String, Vec<u8>> {
    let source = source();
    let scenarios = scenarios();
    let pack = compile_pack(&source);
    let (program, _) = Program::from_pack(&pack).unwrap();
    let mut files = BTreeMap::new();
    let mut results = BTreeMap::new();
    for scenario in scenarios["scenarios"].as_array().unwrap() {
        let name = scenario["name"].as_str().unwrap();
        for (query, request, result) in run(&pack, scenario) {
            results.insert(format!("{name}.{query}"), (request.clone(), result.clone()));
            files.insert(format!("{name}.{query}.request.json"), request.into_bytes());
            files.insert(format!("{name}.{query}.result.json"), result.into_bytes());
        }
    }
    // Rejections start from the useful partial result so every tag and field is in play.
    let (request, result) = &results["choice.default"];
    let (rejected_requests, rejected_results) = rejected(request, result);
    files.insert("rejected-requests.json".into(), pretty(&rejected_requests));
    files.insert("rejected-results.json".into(), pretty(&rejected_results));
    files.insert("source.json".into(), pretty(&source));
    files.insert("scenarios.json".into(), pretty(&scenarios));
    // The cold scenario's objects, split out so a host without a pack reader can replay it.
    let decoded = Pack::decode(&pack).unwrap();
    let mut objects = serde_json::Map::new();
    for graph in ["main.start", "main.helper"] {
        let (id, bytes) = chunk_of(&pack, graph);
        let file = format!("object.{graph}.bin");
        objects.insert(graph.into(), json!({"id": id, "file": file}));
        files.insert(file, bytes);
    }
    files.insert("objects.json".into(), pretty(&Value::Object(objects)));
    files.insert("story.manifest".into(), decoded.manifest);
    files.insert("story.narpack".into(), pack);
    let artifacts: serde_json::Map<String, Value> = files
        .iter()
        .map(|(name, bytes)| (name.clone(), json!({"sha256": sha256(bytes)})))
        .collect();
    files.insert(
        "manifest.json".into(),
        pretty(&json!({
            "corpus_version": 1,
            "format": "ADR 0025 VN dependency exchange version 1: VnBook.dependencies request and result JSON",
            "execution": EXECUTION,
            "artifact_id": program.artifact_id(),
            "inputs": ["source.json", "scenarios.json", "story.narpack"],
            "derived_objects": "story.manifest, objects.json and object.<graph>.bin are the pack's manifest and chunks, for hosts that replay the cold scenario without reading a pack",
            "generation": "cargo test -p narrata-nodes-wasm --lib -- --ignored write_dependency_corpus, with NARRATA_CORPUS_OUT naming a new directory under .temp; story.narpack compiles from source.json and every other file is produced by running scenarios.json through VnBook",
            "artifacts": artifacts,
        })),
    );
    files
}

/// Writes the corpus into a new directory under `.temp`; never into an existing one.
pub fn write(directory: &Path) {
    assert!(
        directory
            .components()
            .any(|part| part.as_os_str() == ".temp"),
        "the corpus is generated under .temp only"
    );
    std::fs::create_dir(directory).expect("output directory must be new");
    for (name, bytes) in build() {
        std::fs::write(directory.join(name), bytes).unwrap();
    }
}
