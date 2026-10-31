#![recursion_limit = "512"]
#![allow(clippy::panic, clippy::unwrap_used)]

use std::{
    cell::Cell,
    collections::{BTreeMap, BTreeSet},
    rc::Rc,
    sync::{Arc, Mutex},
};

use narrata_nodes::{
    ChunkSource, Compilation, ContentRole, DependencyLimits, DependencyProjection, Error,
    ExecutionId, MediaRole, NodeRegistry, ObjectId, Pack, PackageSource, Program, ProjectManifest,
    ProjectSource, Result, Session, Truncation, Unknown, VISUAL_NOVEL_FORMAT_VERSION, compile,
    plan::{GraphRef, NameTable},
    view::Interaction,
};
use narrata_storage::{
    Applied, Batch, Capabilities, KeyPage, KeySpace, KeyValue, MemoryBackend, ObjectDigest,
    ObjectPage, StorageBackend, StorageError,
};
use serde_json::{Value, json};

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

fn image(key: &str, revision: &str) -> Value {
    json!({"kind": "image", "provider": "demo", "key": key, "revision": revision})
}

fn audio(key: &str, revision: &str) -> Value {
    json!({"kind": "audio", "provider": "demo", "key": key, "revision": revision})
}

fn line(key: &str) -> Value {
    json!({"unit": content(key), "first": "line-start", "last": "line-end"})
}

fn segment(unit: &str, first: &str, last: &str) -> Value {
    json!({"unit": content(unit), "first": first, "last": last})
}

fn plan(id: u32, type_id: &str, data: Value) -> Value {
    json!({"id": node(id), "type_id": type_id, "data": data})
}

fn manifest() -> Value {
    json!({
        "format_version": VISUAL_NOVEL_FORMAT_VERSION,
        "product": {
            "id": "vn-deps",
            "entry": {"package": "main", "graph": "start"},
            "shared": {"met": false},
            "endings": {"done": {"title": content("ending.title"), "body": segment("ending.body", "e1", "e1")}}
        },
        "packages": {"main": "main.json"}
    })
}

/// Two dialogue lines, a scene that keeps the music, a choice with a hidden and a disabled
/// option, two routes (one through a helper graph that replaces the music) that converge on a
/// scene, a branch to one of two lines, a multi-select, and a final scene that stops the music.
fn package() -> Value {
    json!({
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
    })
}

fn compile_value(manifest: Value, package: Value) -> Compilation {
    compile(
        &ProjectSource {
            manifest: serde_json::from_value::<ProjectManifest>(manifest).unwrap(),
            packages: BTreeMap::from([(
                "main".to_owned(),
                serde_json::from_value::<PackageSource>(package).unwrap(),
            )]),
        },
        &NodeRegistry::visual_novel(),
    )
    .unwrap()
}

fn variant(edit: impl FnOnce(&mut Value, &mut Value)) -> Compilation {
    let (mut manifest, mut package) = (manifest(), package());
    edit(&mut manifest, &mut package);
    compile_value(manifest, package)
}

fn compiled() -> Compilation {
    variant(|_, _| {})
}

fn execution() -> ExecutionId {
    "execution:0190f2a0000070008000000000000001"
        .parse()
        .unwrap()
}

fn start(compilation: &Compilation) -> (Session, NameTable) {
    let (program, names) = Program::from_pack(&compilation.pack).unwrap();
    (
        Session::new(Arc::new(program), execution()).unwrap(),
        names.unwrap(),
    )
}

fn advance<B: StorageBackend>(session: &mut Session<B>, names: &NameTable) {
    let view = session.view(Some(names)).unwrap();
    let Interaction::Advance { dialogue, .. } = view.interaction else {
        panic!("not at a dialogue");
    };
    session.advance(&view.cursor, dialogue).unwrap();
}

fn choose<B: StorageBackend>(session: &mut Session<B>, names: &NameTable, keys: &[&str]) {
    let view = session.view(Some(names)).unwrap();
    let Interaction::Choose {
        choice_point,
        options,
        ..
    } = view.interaction
    else {
        panic!("not at a choice");
    };
    let chosen = keys
        .iter()
        .map(|key| {
            options
                .iter()
                .find(|option| option.key.as_deref() == Some(*key))
                .unwrap()
                .id
        })
        .collect();
    session.choose(&view.cursor, choice_point, chosen).unwrap();
}

/// The session waiting at `ask` (after two lines and a scene).
fn at_ask<B: StorageBackend>(session: &mut Session<B>, names: &NameTable) {
    advance(session, names);
    advance(session, names);
}

fn project<B: StorageBackend>(session: &Session<B>) -> DependencyProjection {
    session
        .dependency_projection(DependencyLimits::default())
        .unwrap()
}

fn content_of(projection: &DependencyProjection) -> BTreeSet<(ContentRole, &str)> {
    projection
        .content
        .iter()
        .map(|dependency| (dependency.role, dependency.reference.key.as_str()))
        .collect()
}

fn media_of(projection: &DependencyProjection) -> BTreeSet<(MediaRole, &str)> {
    projection
        .media
        .iter()
        .map(|dependency| (dependency.role.clone(), dependency.reference.key.as_str()))
        .collect()
}

fn expected<R: Ord + Clone>(items: &[(R, &'static str)]) -> BTreeSet<(R, &'static str)> {
    items.iter().cloned().collect()
}

fn graph(name: &str) -> GraphRef {
    GraphRef {
        package: "main".into(),
        graph: name.into(),
    }
}

fn layer(id: &str) -> MediaRole {
    MediaRole::CharacterLayer { id: id.into() }
}

#[test]
fn advance_lists_the_next_line_with_speaker_and_voice_and_nothing_further() {
    let compilation = compiled();
    let (mut session, names) = start(&compilation);
    let first = project(&session);
    assert!(first.is_complete());
    assert_eq!(first.artifact_id, session.program().artifact_id());
    assert_eq!(first.cursor, session.cursor().unwrap());
    // The current line (`line.one`) is already in the View; only `two` is potential.
    assert_eq!(
        content_of(&first),
        expected(&[
            (ContentRole::DialogueSpeaker, "speaker.friend"),
            (ContentRole::DialogueLine, "line.two"),
        ])
    );
    assert!(first.media.is_empty() && first.graphs.is_empty());
    assert!(first.unknowns.is_empty());
    assert!(!first.music_may_stop);

    advance(&mut session, &names);
    // `two` leads through a scene that keeps the theme into the choice passage. The hidden and
    // disabled options are static references of a *future* interaction: listed, with
    // `Unknown::Conditions` saying their visibility was not evaluated.
    let second = project(&session);
    assert!(second.is_complete());
    assert_eq!(
        content_of(&second),
        expected(&[
            (ContentRole::PassageTitle, "ask.title"),
            (ContentRole::PassageBody, "ask.body"),
            (ContentRole::ChoiceLabel, "label.left"),
            (ContentRole::ChoiceLabel, "label.right"),
            (ContentRole::ChoiceLabel, "label.secret"),
            (ContentRole::ChoiceLabel, "label.locked"),
            (ContentRole::ChoiceReason, "reason.locked"),
        ])
    );
    assert_eq!(
        media_of(&second),
        expected(&[
            (MediaRole::Background, "bg/mid"),
            (layer("hero"), "portrait/hero"),
            (MediaRole::MusicKeep, "music/theme"),
        ])
    );
    assert_eq!(second.unknowns, BTreeSet::from([Unknown::Conditions]));
}

#[test]
fn a_choice_follows_only_enabled_options_through_calls_branches_and_convergence() {
    let compilation = compiled();
    let (mut session, names) = start(&compilation);
    at_ask(&mut session, &names);
    let projection = project(&session);
    assert!(projection.is_complete());
    // Hidden `secret` and disabled `locked` routes contribute nothing.
    assert_eq!(
        content_of(&projection),
        expected(&[
            (ContentRole::DialogueLine, "line.met"),
            (ContentRole::DialogueLine, "line.alone"),
        ])
    );
    assert_eq!(
        media_of(&projection),
        expected(&[
            // Both routes meet at the same replacing scene.
            (MediaRole::Background, "bg/hall"),
            (layer("hero"), "portrait/hero-alert"),
            (layer("friend"), "portrait/friend"),
            // The helper's scene replaces the visuals and plays its own track.
            (MediaRole::Background, "bg/helper"),
            (MediaRole::MusicPlay, "music/helper"),
            // `Keep` at the merge scene carries whichever track the path arrived with.
            (MediaRole::MusicKeep, "music/theme"),
            (MediaRole::MusicKeep, "music/helper"),
            (MediaRole::Voice, "voice/met"),
        ])
    );
    let helper = &projection.graphs;
    assert_eq!(helper.len(), 1);
    assert_eq!(helper[0].graph, graph("helper"));
    assert_eq!(
        helper[0].chunk,
        session.program().manifest().chunks
            [session.program().manifest().graphs[&graph("helper")].chunk as usize]
    );
    // The mutation and the branch were not evaluated: the result is a union, and says so.
    assert_eq!(
        projection.unknowns,
        BTreeSet::from([Unknown::Assignments, Unknown::Conditions])
    );
    assert!(!projection.music_may_stop);
}

#[test]
fn local_options_multiselect_and_empty_selection_unite_without_enumerating_combinations() {
    let compilation = compiled();
    let (mut session, names) = start(&compilation);
    at_ask(&mut session, &names);
    choose(&mut session, &names, &["left"]);
    advance(&mut session, &names); // met -> pack
    let projection = project(&session);
    assert!(projection.is_complete());
    // The replies of both enabled options, the later choice's label (the next certain interaction), and the body
    // that resumes at the rejoin anchor; nothing beyond that safe point such as the finale.
    assert_eq!(
        content_of(&projection),
        expected(&[
            // A reply is a segment of the passage's own unit.
            (ContentRole::ChoiceReply, "pack.body"),
            (ContentRole::PassageBody, "pack.body"),
            (ContentRole::ChoiceLabel, "label.onward"),
        ])
    );
    assert!(projection.media.is_empty());
    // No automatic place was visited, and nothing was done per subset of the options.
    assert_eq!(projection.work.locations, 0);
    assert_eq!(projection, project(&session));

    // After choosing locally, the branch option of the next point reaches the finale scene.
    choose(&mut session, &names, &["rope", "lamp"]);
    let finale = project(&session);
    assert!(finale.is_complete());
    assert_eq!(
        content_of(&finale),
        expected(&[(ContentRole::DialogueLine, "line.last")])
    );
    assert_eq!(
        media_of(&finale),
        expected(&[(MediaRole::Background, "bg/end")])
    );
    assert!(
        finale.music_may_stop,
        "a Stop directive has no reference but is reported"
    );
}

#[test]
fn the_last_line_projects_the_ending_and_a_finished_story_projects_nothing() {
    let compilation = compiled();
    let (mut session, names) = start(&compilation);
    at_ask(&mut session, &names);
    choose(&mut session, &names, &["left"]);
    advance(&mut session, &names);
    choose(&mut session, &names, &[]);
    choose(&mut session, &names, &["onward"]);
    let at_last_line = project(&session);
    assert!(at_last_line.is_complete());
    assert_eq!(
        content_of(&at_last_line),
        expected(&[
            (ContentRole::EndingTitle, "ending.title"),
            (ContentRole::EndingBody, "ending.body"),
        ])
    );
    advance(&mut session, &names);
    let finished = project(&session);
    // Complete and empty is a real ending, which the other cases never mimic.
    assert!(finished.is_complete());
    assert!(finished.content.is_empty() && finished.media.is_empty() && finished.graphs.is_empty());
    assert!(finished.unknowns.is_empty());
}

#[test]
fn small_limits_stop_before_the_bound_and_say_what_was_cut() {
    let compilation = compiled();
    let (mut session, names) = start(&compilation);
    at_ask(&mut session, &names);
    let limits = |locations, graphs, candidates, references| {
        DependencyLimits::new(locations, graphs, candidates, references).unwrap()
    };

    let few_places = session
        .dependency_projection(limits(3, 8, 100, 100))
        .unwrap();
    assert!(!few_places.is_complete());
    assert_eq!(few_places.work.locations, 3);
    assert_eq!(
        few_places.truncations,
        vec![Truncation::Locations { limit: 3 }]
    );

    let no_graphs = session
        .dependency_projection(limits(256, 0, 100, 100))
        .unwrap();
    assert_eq!(
        no_graphs.truncations,
        vec![Truncation::Graphs {
            graph: graph("helper"),
            limit: 0
        }]
    );
    // The graph is still named, from the manifest alone, but never loaded or explored.
    assert_eq!(no_graphs.graphs.len(), 1);
    assert_eq!(no_graphs.work.graphs, 0);
    assert!(!media_of(&no_graphs).contains(&(MediaRole::MusicPlay, "music/helper")));
    // The other route is still explored to its safe points.
    assert!(content_of(&no_graphs).contains(&(ContentRole::DialogueLine, "line.met")));

    let few_candidates = session
        .dependency_projection(limits(256, 8, 2, 100))
        .unwrap();
    assert_eq!(
        few_candidates.truncations,
        vec![Truncation::Candidates { limit: 2 }]
    );
    assert_eq!(few_candidates.work.candidates, 2);

    let few_references = session
        .dependency_projection(limits(256, 8, 100, 2))
        .unwrap();
    assert_eq!(
        few_references.truncations,
        vec![Truncation::References { limit: 2 }]
    );
    assert_eq!(
        few_references.graphs.len() + few_references.content.len() + few_references.media.len(),
        2
    );

    for projection in [&few_places, &no_graphs, &few_candidates, &few_references] {
        assert!(!projection.is_complete());
    }
    // A bound never changes the answer for the same inputs.
    assert_eq!(
        few_places,
        session
            .dependency_projection(limits(3, 8, 100, 100))
            .unwrap()
    );
}

#[test]
fn limits_are_checked() {
    let new = DependencyLimits::new;
    assert!(new(1, 0, 1, 1).is_ok());
    assert!(new(0, 0, 1, 1).is_err());
    assert!(new(1, 0, 0, 1).is_err());
    assert!(new(1, 0, 1, 0).is_err());
    assert!(new(DependencyLimits::MAX_LOCATIONS + 1, 0, 1, 1).is_err());
    assert!(new(1, DependencyLimits::MAX_GRAPHS + 1, 1, 1).is_err());
    assert!(new(1, 0, DependencyLimits::MAX_CANDIDATES + 1, 1).is_err());
    assert!(new(1, 0, 1, DependencyLimits::MAX_REFERENCES + 1).is_err());
    assert_eq!(new(1, 0, 1, 1).unwrap().locations(), 1);
    assert_eq!(new(1, 0, 1, 1).unwrap().graphs(), 0);
    assert_eq!(new(1, 0, 1, 1).unwrap().candidates(), 1);
    assert_eq!(new(1, 0, 1, 1).unwrap().references(), 1);
}

#[test]
fn recursion_reaches_the_call_depth_and_is_not_mistaken_for_an_ending() {
    let compilation = variant(|_, package| {
        package["graphs"]["start"]["nodes"]["two"]["data"]["next"] = json!("rec");
        package["graphs"]["start"]["nodes"]["rec"] = plan(
            30,
            "narrata.call",
            json!({"target": {"kind": "local", "graph": "recur"}, "on_return": {"back": "ask"}}),
        );
        package["graphs"]["recur"] = json!({
            "outcomes": ["back"],
            "entry": "again",
            "nodes": {
                "again": plan(31, "narrata.call", json!({
                    "target": {"kind": "local", "graph": "recur"},
                    "on_return": {"back": "out"}
                })),
                "out": plan(32, "narrata.return", json!({"outcome": "back"}))
            }
        });
    });
    let (mut session, names) = start(&compilation);
    advance(&mut session, &names);
    let projection = project(&session);
    assert!(!projection.is_complete());
    assert_eq!(
        projection.truncations,
        vec![Truncation::CallDepth {
            graph: graph("recur"),
            node: node(31).parse().unwrap(),
        }]
    );
    // No call ever returns within the bound, so nothing after it (`ask`) is claimed.
    assert!(projection.content.is_empty());
    assert_eq!(projection.graphs.len(), 1);
    // The runtime's own limit is 64 frames, and the walk visited one place per level.
    assert_eq!(projection.work.locations, 64);
}

#[test]
fn automatic_cycles_are_reported_even_when_an_exit_exists_or_not() {
    let looping = |exit: &'static str| {
        variant(|_, package| {
            package["graphs"]["start"]["nodes"]["two"]["data"]["next"] = json!("spin_a");
            package["graphs"]["start"]["nodes"]["spin_a"] = plan(
                30,
                "narrata.mutate",
                json!({"assignments": [], "next": "spin_b"}),
            );
            package["graphs"]["start"]["nodes"]["spin_b"] = plan(
                31,
                "narrata.branch",
                json!({
                    "condition": {"kind": "literal", "value": true},
                    "when_true": "spin_a",
                    "when_false": exit
                }),
            );
        })
    };
    let spin_a = node(30).parse().unwrap();
    let expected_cycle = Truncation::AutomaticCycle {
        graph: graph("start"),
        node: spin_a,
    };

    let (mut session, names) = start(&looping("ask"));
    advance(&mut session, &names);
    let with_exit = project(&session);
    assert_eq!(with_exit.truncations, vec![expected_cycle.clone()]);
    assert!(content_of(&with_exit).contains(&(ContentRole::PassageTitle, "ask.title")));

    let (mut session, names) = start(&looping("spin_a"));
    advance(&mut session, &names);
    let no_exit = project(&session);
    assert_eq!(no_exit.truncations, vec![expected_cycle]);
    assert!(no_exit.content.is_empty() && no_exit.media.is_empty());
    assert!(
        !no_exit.is_complete(),
        "a loop with no exit is not an ending"
    );
}

/// Serves a pack's chunks and can withhold or corrupt some, counting every read.
#[derive(Clone, Default)]
struct Chunks(Arc<Mutex<ChunkState>>);

#[derive(Default)]
struct ChunkState {
    bytes: BTreeMap<ObjectId, Vec<u8>>,
    withheld: BTreeSet<ObjectId>,
    corrupt: BTreeSet<ObjectId>,
    reads: BTreeMap<ObjectId, usize>,
}

impl ChunkSource for Chunks {
    fn load(&self, id: &ObjectId) -> Result<Vec<u8>> {
        let mut state = self.0.lock().unwrap();
        *state.reads.entry(*id).or_default() += 1;
        if state.withheld.contains(id) {
            return Err(Error::new("missing_chunk", id.to_string(), "not delivered"));
        }
        let mut bytes = state
            .bytes
            .get(id)
            .cloned()
            .ok_or_else(|| Error::new("missing_chunk", id.to_string(), "unknown object"))?;
        if state.corrupt.contains(id) {
            let middle = bytes.len() / 2;
            bytes[middle] ^= 0xff;
        }
        Ok(bytes)
    }
}

struct Lazy {
    session: Session,
    names: NameTable,
    chunks: Chunks,
    program: Arc<Program>,
}

impl Lazy {
    fn chunk(&self, name: &str) -> ObjectId {
        let manifest = self.program.manifest();
        manifest.chunks[manifest.graphs[&graph(name)].chunk as usize]
    }

    fn state(&self) -> std::sync::MutexGuard<'_, ChunkState> {
        self.chunks.0.lock().unwrap()
    }
}

fn lazy(compilation: &Compilation, capacity: usize) -> Lazy {
    let pack = Pack::decode(&compilation.pack).unwrap();
    let (complete, names) = Program::from_pack(&compilation.pack).unwrap();
    let chunks = Chunks::default();
    chunks.0.lock().unwrap().bytes = complete
        .manifest()
        .chunks
        .iter()
        .copied()
        .zip(pack.chunks)
        .collect();
    let program =
        Arc::new(Program::from_manifest(&pack.manifest, Box::new(chunks.clone())).unwrap());
    program.set_chunk_capacity(std::num::NonZeroUsize::new(capacity).unwrap());
    Lazy {
        session: Session::new(program.clone(), execution()).unwrap(),
        names: names.unwrap(),
        chunks,
        program,
    }
}

#[test]
fn unavailable_and_corrupt_chunks_are_reported_with_their_errors_and_recover() {
    let compilation = compiled();
    let mut lazy = lazy(&compilation, 16);
    let names = lazy.names.clone();
    at_ask(&mut lazy.session, &names);
    let baseline = project(&lazy.session);
    assert!(baseline.is_complete());
    let helper = graph("helper");
    let chunk = lazy.chunk("helper");

    // Withheld: the source's own error comes back with the exact object identity, the other
    // route is still explored, and the result is not complete.
    // The complete query left the chunk cached; evict it so the next load reaches the source.
    lazy.program
        .set_chunk_capacity(std::num::NonZeroUsize::new(1).unwrap());
    lazy.state().withheld.insert(chunk);
    let missing = project(&lazy.session);
    let Some(Truncation::Unavailable {
        graph: failed,
        chunk: failed_chunk,
        error,
    }) = missing.truncations.first()
    else {
        panic!("expected an unavailable chunk: {:?}", missing.truncations);
    };
    assert_eq!((failed, *failed_chunk), (&helper, chunk));
    assert_eq!(error.code, "missing_chunk");
    assert_eq!(missing.truncations.len(), 1);
    assert!(!missing.is_complete());
    assert!(content_of(&missing).contains(&(ContentRole::DialogueLine, "line.met")));
    // Nothing downstream of the unavailable callee is claimed.
    assert!(!media_of(&missing).contains(&(MediaRole::MusicPlay, "music/helper")));
    // Repeating the failing query gives the same answer and the failure is not cached.
    assert_eq!(missing, project(&lazy.session));

    // Corrupt: the checked decode rejects it, and that rejection is what is reported.
    {
        let mut state = lazy.state();
        state.withheld.clear();
        state.corrupt.insert(chunk);
    }
    let corrupt = project(&lazy.session);
    let Some(Truncation::Unavailable { error, .. }) = corrupt.truncations.first() else {
        panic!("expected a rejected chunk");
    };
    assert_ne!(error.code, "missing_chunk");
    assert_eq!(Err(error.clone()), lazy.program.graph(&helper).map(|_| ()));
    assert!(!corrupt.is_complete());

    // Recovery: once the object is delivered intact, the projection is the complete one.
    lazy.state().corrupt.clear();
    assert_eq!(project(&lazy.session), baseline);
}

#[test]
fn a_small_projection_reads_only_the_chunks_it_needs_in_a_large_work() {
    let compilation = variant(|_, package| {
        let mut exports = vec![json!("start")];
        for index in 0..150_u32 {
            let name = format!("unrelated{index}");
            package["graphs"][name.as_str()] = json!({
                "outcomes": ["done"],
                "entry": "end",
                "nodes": {"end": plan(1000 + index, "narrata.return", json!({"outcome": "done"}))}
            });
            exports.push(json!(name));
        }
        package["exports"] = Value::Array(exports);
    });
    let mut lazy = lazy(&compilation, 16);
    assert!(lazy.program.manifest().chunks.len() > 150);
    let names = lazy.names.clone();
    at_ask(&mut lazy.session, &names);
    lazy.state().reads.clear();
    let projection = project(&lazy.session);
    assert!(projection.is_complete());
    assert_eq!(projection.graphs.len(), 1);
    let reads = lazy.state().reads.clone();
    // Only the callee's chunk is fetched: the current frame's chunk is already resident, and
    // none of the 150 unrelated graphs is touched.
    assert_eq!(reads, BTreeMap::from([(lazy.chunk("helper"), 1)]));
    assert_eq!(projection.work.graphs, 1);
    // Resident chunks answer a repeat without reading the source again.
    lazy.state().reads.clear();
    assert_eq!(project(&lazy.session), projection);
    assert!(lazy.state().reads.is_empty());
}

#[test]
fn borrowed_graphs_are_released_so_a_wide_query_leaves_the_cache_at_its_budget() {
    let compilation = compiled();
    let mut lazy = lazy(&compilation, 1);
    let names = lazy.names.clone();
    at_ask(&mut lazy.session, &names);
    assert_eq!(lazy.program.loaded_chunks(), 1);
    let first = project(&lazy.session);
    // The callee was loaded for the query but belongs to nobody afterwards.
    assert_eq!(lazy.program.loaded_chunks(), 1);
    // Eviction between queries changes work, never the answer.
    lazy.state().reads.clear();
    assert_eq!(project(&lazy.session), first);
    assert_eq!(lazy.state().reads.get(&lazy.chunk("helper")), Some(&1));
    assert_eq!(lazy.program.loaded_chunks(), 1);
}

/// Counts writes (applied batches) so a test can prove a query writes nothing.
struct Writes<B> {
    inner: B,
    applied: Rc<Cell<usize>>,
}

impl<B: StorageBackend> StorageBackend for Writes<B> {
    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }
    fn get_objects(
        &self,
        digests: &[ObjectDigest],
    ) -> std::result::Result<Vec<Option<Arc<[u8]>>>, StorageError> {
        self.inner.get_objects(digests)
    }
    fn scan_objects(
        &self,
        after: Option<&ObjectDigest>,
        limit: u32,
    ) -> std::result::Result<ObjectPage, StorageError> {
        self.inner.scan_objects(after, limit)
    }
    fn read_key(
        &self,
        space: KeySpace,
        key: &[u8],
    ) -> std::result::Result<Option<KeyValue>, StorageError> {
        self.inner.read_key(space, key)
    }
    fn scan_keys(
        &self,
        space: KeySpace,
        prefix: &[u8],
        after: Option<&[u8]>,
        limit: u32,
    ) -> std::result::Result<KeyPage, StorageError> {
        self.inner.scan_keys(space, prefix, after, limit)
    }
    fn apply(&mut self, batch: &Batch) -> std::result::Result<Applied, StorageError> {
        self.applied.set(self.applied.get() + 1);
        self.inner.apply(batch)
    }
}

#[test]
fn queries_change_no_commit_state_history_export_or_storage_even_when_they_fail() {
    let compilation = compiled();
    let pack = Pack::decode(&compilation.pack).unwrap();
    let (complete, names) = Program::from_pack(&compilation.pack).unwrap();
    let names = names.unwrap();
    let chunks = Chunks::default();
    chunks.0.lock().unwrap().bytes = complete
        .manifest()
        .chunks
        .iter()
        .copied()
        .zip(pack.chunks)
        .collect();
    let program =
        Arc::new(Program::from_manifest(&pack.manifest, Box::new(chunks.clone())).unwrap());
    let applied = Rc::new(Cell::new(0));
    let mut session = Session::open(
        program.clone(),
        execution(),
        Writes {
            inner: MemoryBackend::new(),
            applied: applied.clone(),
        },
    )
    .unwrap();
    at_ask(&mut session, &names);

    let snapshot = |session: &Session<Writes<MemoryBackend>>| {
        (
            session.cursor().unwrap(),
            session.state().unwrap().clone(),
            session.export().unwrap(),
            session.commits().unwrap().collect::<Vec<_>>(),
            applied.get(),
        )
    };
    let before = snapshot(&session);

    let ok = project(&session);
    assert!(ok.is_complete());
    assert_eq!(snapshot(&session), before);

    // A truncated query and a failing one are just as pure.
    let helper =
        complete.manifest().chunks[complete.manifest().graphs[&graph("helper")].chunk as usize];
    program.set_chunk_capacity(std::num::NonZeroUsize::new(1).unwrap());
    chunks.0.lock().unwrap().withheld.insert(helper);
    let failed = project(&session);
    assert!(!failed.is_complete());
    let truncated = session
        .dependency_projection(DependencyLimits::new(2, 1, 4, 4).unwrap())
        .unwrap();
    assert!(!truncated.is_complete());
    assert_eq!(snapshot(&session), before);

    // The session still plays on after the queries, from the same cursor.
    chunks.0.lock().unwrap().withheld.clear();
    choose(&mut session, &names, &["right"]);
    assert!(applied.get() > before.4);
}
