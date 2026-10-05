#![allow(clippy::unwrap_used)]

mod support;

use std::{
    collections::BTreeMap,
    num::NonZeroUsize,
    sync::{Arc, Mutex},
};

use narrata_nodes::{ChunkSource, ExecutionId, ObjectId, Pack, Program, Session, plan::GraphRef};
use serde_json::json;
use support::{content, node, option, point, try_variant};

#[derive(Default)]
struct SourceState {
    bytes: BTreeMap<ObjectId, Vec<u8>>,
    reads: BTreeMap<ObjectId, usize>,
}

struct Source(Arc<Mutex<SourceState>>);

impl ChunkSource for Source {
    fn load(&self, id: &ObjectId) -> narrata_nodes::Result<Vec<u8>> {
        let mut state = self.0.lock().unwrap();
        *state.reads.entry(*id).or_default() += 1;
        state.bytes.get(id).cloned().ok_or_else(|| {
            narrata_nodes::Error::new("missing_chunk", id.to_string(), "missing object")
        })
    }
}

fn fixture(count: usize) -> Vec<u8> {
    try_variant(|manifest, main, _| {
        manifest["product"]["entry"]["graph"] = json!("g0");
        manifest["product"]["bindings"] = json!([]);
        manifest["product"]["endings"] = json!({});
        main["exports"] = json!(["g0"]);
        let mut graphs = serde_json::Map::new();
        for index in 0..count {
            let base = 100 + index as u32 * 3;
            let target = if index + 1 < count { "call" } else { "out" };
            let mut nodes = json!({
                "gate": {"id": node(base), "type_id": "narrata.passage", "data": {
                    "choice_points": [{"id": point(base), "key": "route", "options": [
                        {"id": option(base), "key": "descend", "label": content("descend"), "outcome": {"kind": "branch", "target": target}},
                        {"id": option(base + 1), "key": "leave", "label": content("leave"), "outcome": {"kind": "branch", "target": "out"}}
                    ]}]
                }},
                "out": {"id": node(base + 1), "type_id": "narrata.return", "data": {"outcome": "done"}}
            });
            if index + 1 < count {
                nodes["call"] = json!({"id": node(base + 2), "type_id": "narrata.call", "data": {
                    "target": {"kind": "local", "graph": format!("g{}", index + 1)}, "on_return": {"done": "out"}
                }});
            }
            graphs.insert(format!("g{index}"), json!({"entry": "gate", "outcomes": ["done"], "nodes": nodes}));
        }
        main["graphs"] = graphs.into();
    }).unwrap().pack
}

fn opened(bytes: &[u8], capacity: usize) -> (Arc<Program>, Arc<Mutex<SourceState>>) {
    let pack = Pack::decode(bytes).unwrap();
    let (reference, _) = Program::from_pack(bytes).unwrap();
    let state = Arc::new(Mutex::new(SourceState {
        bytes: reference
            .manifest()
            .chunks
            .iter()
            .copied()
            .zip(pack.chunks)
            .collect(),
        ..SourceState::default()
    }));
    let program = Program::open(
        &pack.manifest,
        &pack.tombstones,
        Box::new(Source(state.clone())),
    )
    .unwrap();
    program.set_chunk_capacity(NonZeroUsize::new(capacity).unwrap());
    (Arc::new(program), state)
}

fn graph(index: usize) -> GraphRef {
    GraphRef {
        package: "main".into(),
        graph: format!("g{index}"),
    }
}

fn descend(session: &mut Session) {
    let frame = session.state().unwrap().frames.last().unwrap();
    let graph = session.program().graph(&frame.graph).unwrap();
    let narrata_nodes::plan::Plan::Passage(passage) = &graph.nodes[&frame.node] else {
        unreachable!()
    };
    let point = &passage.choice_points[0];
    session
        .choose(
            &session.cursor().unwrap(),
            point.id,
            vec![point.options[0].id],
        )
        .unwrap();
}

#[test]
fn opening_and_first_screen_read_only_the_needed_chunk() {
    let (program, source) = opened(&fixture(20), 2);
    assert_eq!(program.loaded_chunks(), 0);
    assert!(source.lock().unwrap().reads.is_empty());
    let session = Session::new(program.clone(), ExecutionId::from_bytes([1; 16])).unwrap();
    assert_eq!(session.state().unwrap().frames.len(), 1);
    assert_eq!(source.lock().unwrap().reads.values().sum::<usize>(), 1);
    assert_eq!(program.loaded_chunks(), 1);
}

#[test]
fn lru_reloads_evicted_chunks_and_preserves_borrowed_graphs() {
    let (program, source) = opened(&fixture(5), 2);
    program.graph(&graph(0)).unwrap();
    program.graph(&graph(1)).unwrap();
    program.graph(&graph(0)).unwrap(); // g1 is now oldest.
    program.graph(&graph(2)).unwrap();
    let id0 = program.manifest().chunks[program.manifest().graphs[&graph(0)].chunk as usize];
    let id1 = program.manifest().chunks[program.manifest().graphs[&graph(1)].chunk as usize];
    program.graph(&graph(0)).unwrap();
    assert_eq!(source.lock().unwrap().reads[&id0], 1);
    program.graph(&graph(1)).unwrap();
    assert_eq!(source.lock().unwrap().reads[&id1], 2);
    program.set_chunk_capacity(NonZeroUsize::new(1).unwrap());
    let borrowed = program.graph(&graph(0)).unwrap();
    program.graph(&graph(3)).unwrap();
    assert_eq!(program.loaded_chunks(), 2);
    assert!(!borrowed.nodes.is_empty());
    drop(borrowed);
    program.set_chunk_capacity(NonZeroUsize::new(1).unwrap());
    assert_eq!(program.loaded_chunks(), 1);
}

#[test]
fn every_reload_is_checked_and_failed_loads_do_not_enter_the_cache() {
    let bytes = fixture(4);
    let (program, source) = opened(&bytes, 1);
    let before = program.graph(&graph(0)).unwrap().as_ref().clone();
    program.graph(&graph(1)).unwrap();
    let id = program.manifest().chunks[program.manifest().graphs[&graph(0)].chunk as usize];
    let original = source.lock().unwrap().bytes[&id].clone();
    let mut bad = original.clone();
    let last = bad.len() - 1;
    bad[last] ^= 1;
    source.lock().unwrap().bytes.insert(id, bad);
    assert!(program.graph(&graph(0)).is_err());
    assert_eq!(program.loaded_chunks(), 1);
    let other_id = program.manifest().chunks[program.manifest().graphs[&graph(2)].chunk as usize];
    let other = source.lock().unwrap().bytes[&other_id].clone();
    source.lock().unwrap().bytes.insert(id, other);
    assert_eq!(program.graph(&graph(0)).unwrap_err().code, "artifact");
    source.lock().unwrap().bytes.insert(id, original);
    assert_eq!(*program.graph(&graph(0)).unwrap(), before);
    assert_eq!(source.lock().unwrap().reads[&id], 4);
}

#[test]
fn deep_frame_pins_survive_a_smaller_budget_and_release_independently() {
    let (program, source) = opened(&fixture(64), 1);
    let mut session = Session::new(program.clone(), ExecutionId::from_bytes([2; 16])).unwrap();
    let mut pins = program.pin_state(session.state().unwrap()).unwrap();
    for _ in 1..64 {
        descend(&mut session);
        let next = program.pin_state(session.state().unwrap()).unwrap();
        pins = next;
    }
    assert_eq!(session.state().unwrap().frames.len(), 64);
    assert_eq!(program.loaded_chunks(), 64);
    let second = program.pin_state(session.state().unwrap()).unwrap();
    drop(pins);
    program
        .graph(&GraphRef {
            package: "side".into(),
            graph: "visit".into(),
        })
        .unwrap();
    program.set_chunk_capacity(NonZeroUsize::new(1).unwrap());
    assert_eq!(program.loaded_chunks(), 64);
    for index in 0..64 {
        program.graph(&graph(index)).unwrap();
        let id = program.manifest().chunks[program.manifest().graphs[&graph(index)].chunk as usize];
        assert_eq!(source.lock().unwrap().reads[&id], 1);
    }
    drop(second);
    assert_eq!(program.loaded_chunks(), 1);
}

#[test]
fn failed_pinning_releases_partial_pins() {
    let (program, source) = opened(&fixture(4), 1);
    let mut session = Session::new(program.clone(), ExecutionId::from_bytes([3; 16])).unwrap();
    descend(&mut session);
    let id = program.manifest().chunks[program.manifest().graphs[&graph(1)].chunk as usize];
    source.lock().unwrap().bytes.remove(&id);
    program.graph(&graph(2)).unwrap();
    assert!(program.pin_state(session.state().unwrap()).is_err());
    program.graph(&graph(3)).unwrap();
    assert_eq!(program.loaded_chunks(), 1);
}

#[test]
fn eviction_changes_neither_states_commits_replay_nor_restore() {
    let bytes = fixture(12);
    let (small, _) = opened(&bytes, 1);
    let (large, _) = opened(&bytes, 64);
    let execution = ExecutionId::from_bytes([4; 16]);
    let mut a = Session::new(small.clone(), execution).unwrap();
    let mut b = Session::new(large, execution).unwrap();
    for _ in 0..12 {
        assert_eq!(a.cursor().unwrap(), b.cursor().unwrap());
        assert_eq!(a.state().unwrap().id(), b.state().unwrap().id());
        assert_eq!(a.view(None).unwrap(), b.view(None).unwrap());
        descend(&mut a);
        descend(&mut b);
    }
    assert_eq!(a.export().unwrap(), b.export().unwrap());
    a.verify_path(&a.cursor().unwrap()).unwrap();
    let restored = Session::restore(small, &a.export().unwrap()).unwrap();
    assert_eq!(restored.state().unwrap(), a.state().unwrap());
    assert_eq!(restored.cursor().unwrap(), a.cursor().unwrap());
    restored.verify_path(&restored.cursor().unwrap()).unwrap();
    assert_eq!(restored.state().unwrap().frames.len(), 0);
}
