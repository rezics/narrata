#![allow(clippy::panic, clippy::unwrap_used)]

mod support;

use narrata_kernel::codec::{CborWriter, encode_envelope};
use narrata_nodes::{
    KIND_CHUNK, KIND_INPUT, KIND_MANIFEST, KIND_STATE, MemorySource, Pack, Program, Session,
    decode_input, decode_state,
};
use proptest::prelude::*;
use serde_json::json;
use support::*;

/// A canonical CBOR tree, enough to edit encoded objects in tests.
#[derive(Clone, Debug, PartialEq)]
enum Cbor {
    Unsigned(u64),
    Negative(i64),
    Bytes(Vec<u8>),
    Text(String),
    Array(Vec<Cbor>),
    Map(Vec<(u64, Cbor)>),
    Bool(bool),
    Null,
}

fn parse(bytes: &[u8]) -> Cbor {
    fn item(bytes: &[u8], at: &mut usize) -> Cbor {
        let head = bytes[*at];
        *at += 1;
        let (major, info) = (head >> 5, head & 31);
        let mut argument = |info: u8| -> u64 {
            let width = match info {
                0..=23 => return u64::from(info),
                24 => 1,
                25 => 2,
                26 => 4,
                _ => 8,
            };
            let value = bytes[*at..*at + width]
                .iter()
                .fold(0_u64, |value, byte| (value << 8) | u64::from(*byte));
            *at += width;
            value
        };
        match major {
            0 => Cbor::Unsigned(argument(info)),
            1 => Cbor::Negative(-1 - argument(info) as i64),
            2 | 3 => {
                let length = argument(info) as usize;
                let data = bytes[*at..*at + length].to_vec();
                *at += length;
                if major == 2 {
                    Cbor::Bytes(data)
                } else {
                    Cbor::Text(String::from_utf8(data).unwrap_or_default())
                }
            }
            4 => {
                let length = argument(info);
                Cbor::Array((0..length).map(|_| item(bytes, at)).collect())
            }
            5 => {
                let length = argument(info);
                Cbor::Map(
                    (0..length)
                        .map(|_| match item(bytes, at) {
                            Cbor::Unsigned(key) => (key, item(bytes, at)),
                            other => panic!("map key {other:?}"),
                        })
                        .collect(),
                )
            }
            _ => match info {
                20 => Cbor::Bool(false),
                21 => Cbor::Bool(true),
                22 => Cbor::Null,
                _ => panic!("simple {info}"),
            },
        }
    }
    let mut at = 0;
    item(bytes, &mut at)
}

fn write(value: &Cbor) -> Vec<u8> {
    fn put(w: &mut CborWriter, value: &Cbor) {
        match value {
            Cbor::Unsigned(value) => w.unsigned(*value),
            Cbor::Negative(value) => w.signed(*value),
            Cbor::Bytes(value) => w.bytes(value),
            Cbor::Text(value) => w.text(value),
            Cbor::Array(items) => {
                w.array(items.len() as u64);
                items.iter().for_each(|item| put(w, item));
            }
            Cbor::Map(entries) => {
                w.map(entries.len() as u64);
                for (key, value) in entries {
                    w.unsigned(*key);
                    put(w, value);
                }
            }
            Cbor::Bool(value) => w.boolean(*value),
            Cbor::Null => w.null(),
        }
    }
    let mut w = CborWriter::new();
    put(&mut w, value);
    w.into_bytes()
}

fn field(value: &mut Cbor, key: u64) -> &mut Cbor {
    match value {
        Cbor::Map(entries) => entries
            .iter_mut()
            .find(|(candidate, _)| *candidate == key)
            .map(|(_, value)| value)
            .unwrap_or_else(|| panic!("no key {key}")),
        other => panic!("not a map: {other:?}"),
    }
}

fn index(value: &mut Cbor, position: usize) -> &mut Cbor {
    match value {
        Cbor::Array(items) => &mut items[position],
        other => panic!("not an array: {other:?}"),
    }
}

fn insert(value: &mut Cbor, key: u64, new: Cbor) {
    match value {
        Cbor::Map(entries) => {
            entries.push((key, new));
            entries.sort_by_key(|(key, _)| *key);
        }
        other => panic!("not a map: {other:?}"),
    }
}

fn payload(envelope: &[u8]) -> &[u8] {
    &envelope[56..]
}

fn code<T>(result: narrata_nodes::Result<T>) -> String {
    result
        .err()
        .map(|error| error.code)
        .unwrap_or_else(|| "accepted".into())
}

#[test]
fn a_pack_reencodes_byte_for_byte() {
    let compilation = compiled();
    let pack = Pack::decode(&compilation.pack).unwrap();
    assert_eq!(pack.encode(), compilation.pack);
    let (program, names) = Program::from_pack(&compilation.pack).unwrap();
    assert_eq!(
        program.pack(names.as_ref()).ok(),
        Some(compilation.pack.clone())
    );
    // Without the name table the artifact is the same.
    let bare = program.pack(None).unwrap();
    let (reopened, none) = Program::from_pack(&bare).unwrap();
    assert_eq!(reopened.artifact_id(), compilation.program.artifact_id());
    assert!(none.is_none());
}

#[test]
fn the_artifact_id_is_the_manifest_object_id() {
    let compilation = compiled();
    let pack = Pack::decode(&compilation.pack).unwrap();
    let id = narrata_kernel::codec::object_id(KIND_MANIFEST, 1, payload(&pack.manifest));
    assert_eq!(compilation.program.artifact_id().as_bytes(), &id);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn flipping_any_pack_byte_is_rejected(position in any::<prop::sample::Index>(), bit in 0_u8..8) {
        let compilation = compiled();
        let mut bytes = compilation.pack.clone();
        let at = position.index(bytes.len());
        bytes[at] ^= 1 << bit;
        prop_assert!(Program::from_pack(&bytes).is_err());
    }

    #[test]
    fn flipping_any_state_byte_is_rejected(position in any::<prop::sample::Index>(), bit in 0_u8..8) {
        let compilation = compiled();
        let (session, _) = session(&compilation);
        let mut bytes = session.state().unwrap().envelope();
        let at = position.index(bytes.len());
        bytes[at] ^= 1 << bit;
        prop_assert!(decode_state(session.program(), &bytes).is_err());
    }

    /// Alias renames never reach the artifact, its chunks, the commits or the states.
    #[test]
    fn alias_renames_keep_artifact_chunk_commit_and_state_ids(
        node in "[a-z][a-z0-9_]{0,12}",
        point in "[a-z][a-z0-9-]{0,12}",
        option in "[a-z][a-z0-9_]{0,12}",
    ) {
        prop_assume!(!["visit", "tally", "count", "won", "lost", "gate"].contains(&node.as_str()));
        prop_assume!(!["pack", "route"].contains(&point.as_str()));
        prop_assume!(option != "nod");
        let base = compiled();
        let renamed = try_variant(|_, main, _| {
            let nodes = at(main, "/graphs/start/nodes").as_object_mut().unwrap();
            let gate = nodes.remove("gate").unwrap();
            nodes.insert(node.clone(), gate);
            *at(main, "/graphs/start/entry") = json!(node);
            let data = format!("/graphs/start/nodes/{node}/data");
            *at(main, &format!("{data}/choice_points/0/key")) = json!(point);
            *at(main, &format!("{data}/choice_points/0/options/0/key")) = json!(option);
        })
        .unwrap();
        prop_assert_eq!(base.program.artifact_id(), renamed.program.artifact_id());
        prop_assert_eq!(&base.program.manifest().chunks, &renamed.program.manifest().chunks);
        let play = |compilation: &narrata_nodes::Compilation, first: &str| {
            let (mut session, names) = session(compilation);
            choose(&mut session, &names, &[first]).unwrap();
            choose(&mut session, &names, &["rope", "lamp"]).unwrap();
            choose(&mut session, &names, &["left"]).unwrap();
            session
                .commits().unwrap()
                .map(|(id, commit)| (id, commit.state))
                .collect::<Vec<_>>()
        };
        prop_assert_eq!(play(&base, "wave"), play(&renamed, &option));
    }
}

#[test]
fn a_proposals_field_written_as_false_is_not_canonical() {
    let compilation = compiled();
    let pack = Pack::decode(&compilation.pack).unwrap();
    let chunk_index = compilation
        .program
        .manifest()
        .graphs
        .iter()
        .find(|(graph, _)| graph.graph == "start")
        .map(|(_, entry)| entry.chunk as usize)
        .unwrap();
    // chunk {0: [[graph, {.., 7: [[node, plan]..]}]]}; plan [0, passage]; passage {3: [cp..]}
    let mut chunk = parse(payload(&pack.chunks[chunk_index]));
    let graph = index(index(field(&mut chunk, 0), 0), 1);
    let nodes = field(graph, 7);
    let Cbor::Array(entries) = nodes else {
        panic!("nodes")
    };
    let passage = entries
        .iter_mut()
        .map(|entry| index(entry, 1))
        .find(|plan| matches!(plan, Cbor::Array(items) if items[0] == Cbor::Unsigned(0)))
        .unwrap();
    let point = index(field(index(passage, 1), 3), 0);
    // Absent means false, so `false` has no encoding of its own.
    insert(point, 5, Cbor::Bool(false));
    let chunk_envelope = encode_envelope(KIND_CHUNK, 1, &write(&chunk));
    let chunk_id = narrata_kernel::codec::object_id(KIND_CHUNK, 1, payload(&chunk_envelope));
    let mut manifest = parse(payload(&pack.manifest));
    *index(field(&mut manifest, 3), chunk_index) = Cbor::Bytes(chunk_id.to_vec());
    let manifest_envelope = encode_envelope(KIND_MANIFEST, 1, &write(&manifest));
    let mut source = MemorySource::default();
    for (position, chunk) in pack.chunks.iter().enumerate() {
        let envelope = if position == chunk_index {
            chunk_envelope.clone()
        } else {
            chunk.clone()
        };
        source.insert(envelope).unwrap();
    }
    let program = Program::open(&manifest_envelope, &pack.tombstones, Box::new(source)).unwrap();
    let start = narrata_nodes::plan::GraphRef {
        package: "main".into(),
        graph: "start".into(),
    };
    assert_eq!(code(program.graph(&start)), "decode");
}

#[test]
fn an_empty_overlay_is_not_canonical() {
    let compilation = compiled();
    let (session, _) = session(&compilation);
    let envelope = session.state().unwrap().envelope();
    let mut state = parse(payload(&envelope));
    insert(index(field(&mut state, 1), 0), 6, Cbor::Map(Vec::new()));
    let tampered = encode_envelope(KIND_STATE, 1, &write(&state));
    assert_eq!(code(decode_state(session.program(), &tampered)), "decode");
}

#[test]
fn a_proposal_at_a_choice_point_without_proposals_is_rejected() {
    let compilation = compiled();
    let (session, _) = session(&compilation);
    let parent = session.state().unwrap();
    let at = parent.frames[0].at.unwrap();
    let input = Cbor::Array(vec![
        Cbor::Unsigned(1),
        Cbor::Bytes(at.as_bytes().to_vec()),
        Cbor::Array(Vec::new()),
        Cbor::Array(Vec::new()),
    ]);
    let envelope = encode_envelope(KIND_INPUT, 1, &write(&input));
    let cursor = session.cursor().unwrap();
    assert_eq!(
        code(decode_input(session.program(), &cursor, parent, &envelope)),
        "proposals"
    );
}

#[test]
fn unknown_map_keys_are_rejected() {
    let compilation = compiled();
    let (session, _) = session(&compilation);
    let envelope = session.state().unwrap().envelope();
    let mut state = parse(payload(&envelope));
    insert(&mut state, 9, Cbor::Null);
    let tampered = encode_envelope(KIND_STATE, 1, &write(&state));
    assert_eq!(code(decode_state(session.program(), &tampered)), "decode");
}

#[test]
fn a_state_round_trips_through_its_canonical_encoding() {
    let compilation = compiled();
    let (mut session, names) = session(&compilation);
    choose(&mut session, &names, &["wave"]).unwrap();
    let state = session.state().unwrap().clone();
    let (decoded, id) = decode_state(session.program(), &state.envelope()).unwrap();
    assert_eq!(decoded, state);
    assert_eq!(id, state.id());
    assert_eq!(write(&parse(&state.encode())), state.encode());
}

#[test]
fn a_session_never_needs_the_name_table() {
    let compilation = compiled();
    let (program, _) = open(&compilation.pack);
    let bare = program.pack(None).unwrap();
    let (program, names) = Program::from_pack(&bare).unwrap();
    assert!(names.is_none());
    let session = Session::new(std::sync::Arc::new(program), execution()).unwrap();
    let view = session.view(None).unwrap();
    let text = serde_json::to_string(&view).unwrap();
    assert!(!text.contains("greet") && !text.contains("\"gate\""));
}
