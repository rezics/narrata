//! Boundary tests of the dependency exchange (ADR 0025). The native walk has its own matrix in
//! `narrata-nodes`; these cover what the Wasm entry adds: the checked DTOs, the cursor and
//! session checks, and the transport state around a query.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

mod corpus;

use std::collections::BTreeSet;

use narrata_nodes::{DependencyLimits, ObjectId, Pack, Program};
use serde_json::{Value, json};

use crate::{
    VnBook,
    dependencies::{
        ContentRoleV1, DependencyExchangeV1, DependencyRequestV1, DependencyResultV1, MediaRoleV1,
        TruncationV1, UnknownV1,
    },
};
use corpus::{EXECUTION, chunk_of, compile_pack, source};

fn pack() -> Vec<u8> {
    compile_pack(&source())
}

fn view(book: &VnBook) -> Value {
    serde_json::from_str(&book.inspect().unwrap()).unwrap()
}

fn advance(book: &mut VnBook) {
    let shown = view(book);
    let cursor = shown["view"]["cursor"].as_str().unwrap();
    let dialogue = shown["view"]["interaction"]["dialogue"].as_str().unwrap();
    book.advance(cursor, dialogue).unwrap();
}

fn choose(book: &mut VnBook, keys: &[&str]) {
    let shown = view(book);
    let interaction = &shown["view"]["interaction"];
    let ids = keys
        .iter()
        .map(|key| {
            interaction["options"]
                .as_array()
                .unwrap()
                .iter()
                .find(|option| option["key"] == *key)
                .unwrap()["id"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    let point = interaction["choice_point"].as_str().unwrap();
    book.choose(shown["view"]["cursor"].as_str().unwrap(), point, ids)
        .unwrap();
}

fn opened(pack: &[u8]) -> VnBook {
    let mut book = VnBook::new(pack, EXECUTION).unwrap();
    book.memory().unwrap();
    book
}

fn at_ask(pack: &[u8]) -> VnBook {
    let mut book = opened(pack);
    advance(&mut book);
    advance(&mut book);
    book
}

fn request(book: &VnBook, limits: DependencyLimits) -> String {
    serde_json::to_string(&DependencyRequestV1::new(
        book.cursor().unwrap().parse().unwrap(),
        limits,
    ))
    .unwrap()
}

fn query(book: &VnBook, limits: DependencyLimits) -> DependencyResultV1 {
    let text = book.dependency_result(&request(book, limits)).unwrap();
    let result = DependencyResultV1::decode(&text).unwrap();
    // The emitted bytes are the canonical spelling of what they decode to.
    assert_eq!(serde_json::to_string(&result).unwrap(), text);
    result
}

fn media(result: &DependencyResultV1) -> BTreeSet<(MediaRoleV1, String)> {
    result
        .media
        .iter()
        .map(|item| (item.role.clone(), item.reference.key.clone()))
        .collect()
}

fn layer(id: &str) -> MediaRoleV1 {
    MediaRoleV1::CharacterLayer { id: id.into() }
}

fn error_code(result: Result<String, narrata_nodes::Error>) -> String {
    result.unwrap_err().code
}

#[test]
fn an_advance_query_matches_the_native_projection_and_lists_roles() {
    let pack = pack();
    let book = opened(&pack);
    let limits = DependencyLimits::default();
    let result = query(&book, limits);
    assert!(result.complete && result.truncations.is_empty());
    let session = book.inner.session.as_ref().unwrap();
    assert_eq!(
        result,
        DependencyResultV1::new(session.dependency_projection(limits).unwrap(), limits)
    );
    assert_eq!(result.artifact_id, session.program().artifact_id());
    assert_eq!(result.cursor, session.cursor().unwrap());
    // The shown line is the View's; only the next speaker and line are additional.
    let content: BTreeSet<_> = result
        .content
        .iter()
        .map(|item| (item.role, item.reference.key.as_str().to_owned()))
        .collect();
    assert_eq!(
        content,
        BTreeSet::from([
            (ContentRoleV1::DialogueSpeaker, "speaker.friend".to_owned()),
            (ContentRoleV1::DialogueLine, "line.two".to_owned()),
        ])
    );
    assert!(result.media.is_empty() && result.graphs.is_empty() && !result.music_may_stop);
}

#[test]
fn a_choice_query_keeps_scene_roles_and_the_graph_identity_of_the_callee() {
    let pack = pack();
    let book = at_ask(&pack);
    let result = query(&book, DependencyLimits::default());
    assert!(result.complete);
    let kept: BTreeSet<_> = [
        (MediaRoleV1::MusicPlay, "music/helper"),
        // Keep lists the track already playing on each route.
        (MediaRoleV1::MusicKeep, "music/helper"),
        (MediaRoleV1::MusicKeep, "music/theme"),
        (MediaRoleV1::Background, "bg/helper"),
        (MediaRoleV1::Background, "bg/hall"),
        (layer("hero"), "portrait/hero-alert"),
        (layer("friend"), "portrait/friend"),
        (MediaRoleV1::Voice, "voice/met"),
    ]
    .into_iter()
    .map(|(role, key)| (role, key.to_owned()))
    .collect();
    assert!(kept.is_subset(&media(&result)), "{:?}", media(&result));
    // Only the helper graph is another chunk; its identity is the manifest's.
    assert_eq!(result.graphs.len(), 1);
    assert_eq!(result.graphs[0].graph.graph, "helper");
    assert_eq!(
        result.graphs[0].chunk.to_string(),
        chunk_of(&pack, "main.helper").0
    );
    // Hidden and disabled options are not offered as destinations, but their labels belong to
    // any later choice; the walk does not evaluate conditions and says so.
    assert!(result.unknowns.contains(&UnknownV1::Conditions));
}

#[test]
fn a_multiselect_query_stops_at_the_next_safe_point_and_a_later_one_reaches_the_stop() {
    let pack = pack();
    let mut book = at_ask(&pack);
    choose(&mut book, &["left"]);
    advance(&mut book);
    assert_eq!(view(&book)["view"]["interaction"]["max"], 2);
    // Combinations are never enumerated: the local replies and the later point's label.
    let result = query(&book, DependencyLimits::default());
    assert!(result.complete && !result.music_may_stop && result.media.is_empty());
    let roles: BTreeSet<_> = result.content.iter().map(|item| item.role).collect();
    assert_eq!(
        roles,
        BTreeSet::from([
            ContentRoleV1::PassageBody,
            ContentRoleV1::ChoiceLabel,
            ContentRoleV1::ChoiceReply,
        ])
    );

    // From the later point, the branch passes through a full Scene replacement that stops the
    // music, and ends at the next line.
    choose(&mut book, &["rope", "lamp"]);
    let result = query(&book, DependencyLimits::default());
    assert!(result.complete && result.music_may_stop);
    assert_eq!(
        media(&result),
        BTreeSet::from([(MediaRoleV1::Background, "bg/end".to_owned())])
    );
    assert!(
        result
            .content
            .iter()
            .any(|item| item.role == ContentRoleV1::DialogueLine
                && item.reference.key.as_str() == "line.last")
    );
}

#[test]
fn all_five_bounds_truncate_with_the_limit_that_was_asked() {
    let pack = pack();
    let book = at_ask(&pack);
    let tight = |locations, graphs, candidates, references, scans| {
        DependencyLimits::new(locations, graphs, candidates, references, scans).unwrap()
    };
    let cases = [
        (tight(3, 8, 2048, 512, 1024), "locations"),
        (tight(256, 0, 2048, 512, 1024), "graphs"),
        (tight(256, 8, 2, 512, 1024), "candidates"),
        (tight(256, 8, 2048, 2, 1024), "references"),
        (tight(256, 8, 2048, 512, 2), "scans"),
    ];
    for (limits, kind) in cases {
        let result = query(&book, limits);
        assert!(!result.complete, "{kind}");
        let value = serde_json::to_value(&result.truncations).unwrap();
        assert!(
            value
                .as_array()
                .unwrap()
                .iter()
                .any(|truncation| truncation["kind"] == kind),
            "{kind}: {value}"
        );
        // Bounds never change the answer for the same inputs.
        assert_eq!(result, query(&book, limits));
    }
}

#[test]
fn requests_are_checked_before_anything_runs() {
    let pack = pack();
    let book = at_ask(&pack);
    let good: Value = serde_json::from_str(&request(&book, DependencyLimits::default())).unwrap();
    let with = |change: &dyn Fn(&mut Value)| {
        let mut value = good.clone();
        change(&mut value);
        serde_json::to_string(&value).unwrap()
    };
    let cases = [
        (with(&|v| v["version"] = json!(2)), "schema"),
        (with(&|v| v["version"] = json!(0)), "schema"),
        (with(&|v| v["extra"] = json!(1)), "schema"),
        (with(&|v| v["limits"]["locations"] = json!(0)), "limit"),
        (with(&|v| v["limits"]["references"] = json!(4097)), "limit"),
        (with(&|v| v["limits"]["graphs"] = json!(65)), "limit"),
        (with(&|v| v["limits"]["scans"] = json!(-1)), "schema"),
        (
            with(&|v| {
                v["limits"].as_object_mut().unwrap().remove("scans");
            }),
            "schema",
        ),
        (with(&|v| v["cursor"] = json!("commit:xyz")), "schema"),
        ("{\"version\":1".to_owned(), "json"),
        (" ".repeat(5000), "limit"),
    ];
    for (text, code) in cases {
        assert_eq!(error_code(book.dependency_result(&text)), code, "{text}");
    }
    // Zero extra graphs is the one limit allowed to be zero.
    let zero = with(&|v| v["limits"]["graphs"] = json!(0));
    book.dependency_result(&zero).unwrap();

    // A cursor that is well formed but not the session's is stale, never answered.
    let moved = {
        let mut next = at_ask(&pack);
        choose(&mut next, &["left"]);
        request(&next, DependencyLimits::default())
    };
    assert_eq!(error_code(book.dependency_result(&moved)), "stale_input");
}

#[test]
fn a_closed_or_superseded_session_cannot_be_queried() {
    let pack = pack();
    let mut book = VnBook::new(&pack, EXECUTION).unwrap();
    let text = {
        let reference = opened(&pack);
        request(&reference, DependencyLimits::default())
    };
    assert_eq!(error_code(book.dependency_result(&text)), "state");
    book.memory().unwrap();
    book.dependency_result(&text).unwrap();
    book.reload();
    assert_eq!(error_code(book.dependency_result(&text)), "state");
}

#[test]
fn a_pending_candidate_cursor_is_answered_as_a_candidate_and_left_unconfirmed() {
    let pack = pack();
    let mut book = opened(&pack);
    advance(&mut book);
    let candidate = book.cursor().unwrap();
    let pending = book.unconfirmed();
    assert!(pending.is_some(), "the advance is still unconfirmed");
    let result = query(&book, DependencyLimits::default());
    assert_eq!(result.cursor.to_string(), candidate);
    // The query neither confirms, requests nor writes.
    assert_eq!(book.unconfirmed(), pending);
    assert_eq!(book.take_request(), None);
    assert_eq!(book.cursor().unwrap(), candidate);
}

/// A book that holds only the entry graph's chunk, restored at the choice.
fn cold_at_ask(pack: &[u8]) -> VnBook {
    let export = at_ask(pack).export().unwrap();
    let decoded = Pack::decode(pack).unwrap();
    let mut cold = VnBook::from_manifest(&decoded.manifest, EXECUTION).unwrap();
    cold.load_names(&decoded.names.unwrap()).unwrap();
    let (id, bytes) = chunk_of(pack, "main.start");
    cold.load_chunk(&id, &bytes).unwrap();
    cold.memory().unwrap();
    cold.restore(&export).unwrap();
    cold
}

fn pending(book: &VnBook) -> Option<ObjectId> {
    book.inner
        .active()
        .chunks
        .as_ref()
        .unwrap()
        .0
        .lock()
        .unwrap()
        .request
}

fn supplied(book: &VnBook) -> usize {
    book.inner
        .active()
        .chunks
        .as_ref()
        .unwrap()
        .0
        .lock()
        .unwrap()
        .objects
        .len()
}

/// Everything a host can observe of the book other than the query's own result.
fn observed(book: &VnBook) -> (String, String, String, Option<Vec<u8>>, Option<ObjectId>) {
    (
        book.cursor().unwrap(),
        book.export().unwrap(),
        book.inspect().unwrap(),
        book.unconfirmed(),
        pending(book),
    )
}

#[test]
fn a_cold_query_reports_the_unavailable_chunk_without_leaving_a_mandatory_request() {
    let pack = pack();
    let cold = cold_at_ask(&pack);
    let helper: ObjectId = chunk_of(&pack, "main.helper").0.parse().unwrap();
    let before = observed(&cold);
    assert_eq!(pending(&cold), None);

    let result = query(&cold, DependencyLimits::default());
    let unavailable = result
        .truncations
        .iter()
        .find_map(|truncation| match truncation {
            TruncationV1::Unavailable {
                graph,
                chunk,
                error,
            } => Some((graph, chunk, error)),
            _ => None,
        })
        .expect("the helper chunk is cold");
    assert_eq!(unavailable.0.graph, "helper");
    assert_eq!(*unavailable.1, helper);
    // The source's own error and the exact object identity, not a generic failure.
    assert_eq!(unavailable.2.code, "not_loaded");
    assert!(unavailable.2.path.contains(&helper.to_string()));
    assert!(!result.complete);
    // Other routes are still explored.
    assert!(
        result
            .content
            .iter()
            .any(|item| item.reference.key.as_str() == "line.met")
    );

    // Nothing the host sees changed, and in particular no program request exists to fetch.
    assert_eq!(observed(&cold), before);
    assert_eq!(cold.take_program_request().unwrap(), None);
    assert_eq!(cold.take_chunk_request(), None);
    // Asking again gives the same answer, and still no request.
    assert_eq!(query(&cold, DependencyLimits::default()), result);
    assert_eq!(pending(&cold), None);
}

#[test]
fn a_query_neither_steals_nor_replaces_an_earlier_ordinary_request() {
    let pack = pack();
    let mut cold = cold_at_ask(&pack);
    let helper: ObjectId = chunk_of(&pack, "main.helper").0.parse().unwrap();
    // The reader chooses `right`; the call needs the cold helper chunk, so the ordinary
    // operation fails with the mandatory request recorded.
    let shown = view(&cold);
    let interaction = &shown["view"]["interaction"];
    let right = interaction["options"]
        .as_array()
        .unwrap()
        .iter()
        .find(|option| option["key"] == "right")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let cursor = shown["view"]["cursor"].as_str().unwrap().parse().unwrap();
    let point = interaction["choice_point"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let failed = cold
        .inner
        .opened_mut()
        .unwrap()
        .choose(&cursor, point, vec![right]);
    assert_eq!(failed.unwrap_err().code, "not_loaded");
    assert_eq!(pending(&cold), Some(helper));

    let result = query(&cold, DependencyLimits::default());
    assert!(!result.complete);
    // The request that belonged to the operation survives, and the host still gets it once.
    assert_eq!(pending(&cold), Some(helper));
    assert_eq!(
        cold.take_program_request().unwrap(),
        Some(format!("[\"{}\",\"{helper}\"]", cold.artifact_id()))
    );
    assert_eq!(cold.take_program_request().unwrap(), None);

    // The host answers the operation's request; the retry succeeds.
    let (id, bytes) = chunk_of(&pack, "main.helper");
    cold.load_chunk(&id, &bytes).unwrap();
    cold.inner
        .opened_mut()
        .unwrap()
        .choose(&cursor, point, vec![right])
        .unwrap();
}

#[test]
fn a_cold_query_never_blocks_the_next_input_and_recovers_once_the_chunk_arrives() {
    let pack = pack();
    let mut cold = cold_at_ask(&pack);
    let complete = query(&at_ask(&pack), DependencyLimits::default());
    assert!(!query(&cold, DependencyLimits::default()).complete);

    // The route that stays inside the entry graph needs no helper chunk: the optional query
    // must not have queued one ahead of it.
    choose(&mut cold, &["left"]);
    assert_eq!(pending(&cold), None);

    let mut again = cold_at_ask(&pack);
    // Recovery: the host decides to supply the chunk the result named, and asks again.
    let (id, bytes) = chunk_of(&pack, "main.helper");
    again.load_chunk(&id, &bytes).unwrap();
    assert_eq!(supplied(&again), 1);
    let recovered = query(&again, DependencyLimits::default());
    assert_eq!(recovered, complete);
    // Supplied bytes belong to the operation that will use them: the query keeps them.
    assert_eq!(supplied(&again), 1);
    assert_eq!(pending(&again), None);
    choose(&mut again, &["right"]);
    assert_eq!(supplied(&again), 0);
}

#[test]
fn only_the_current_artifact_is_touched_not_registered_history() {
    let pack = pack();
    let mut cold = cold_at_ask(&pack);
    // A second registered artifact with its own pending request must be left alone.
    let other = {
        let mut source = source();
        source["manifest"]["product"]["id"] = json!("vn-deps-historic");
        compile_pack(&source)
    };
    let (historic, _) = Program::from_pack(&other).unwrap();
    let historic_pack = Pack::decode(&other).unwrap();
    cold.inner
        .register(
            crate::ReaderArtifact {
                pack: None,
                names: None,
                analysis: None,
                chunks: Some(crate::HostChunks::default()),
                program: std::sync::Arc::new(
                    Program::from_manifest(
                        &historic_pack.manifest,
                        Box::new(crate::HostChunks::default()),
                    )
                    .unwrap(),
                ),
            },
            0,
        )
        .ok()
        .unwrap();
    let marker = historic.manifest().chunks[0];
    let historic_chunks = cold.inner.artifacts[&historic.artifact_id()]
        .chunks
        .clone()
        .unwrap();
    historic_chunks.0.lock().unwrap().request = Some(marker);
    query(&cold, DependencyLimits::default());
    assert_eq!(historic_chunks.0.lock().unwrap().request, Some(marker));
}

#[test]
fn the_wasm_entry_agrees_with_checked_decode_of_every_result_it_can_emit() {
    // Every committed result is the canonical spelling of its decode.
    let directory = corpus_directory();
    for entry in std::fs::read_dir(&directory).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_str().unwrap().to_owned();
        if let Some(stem) = name.strip_suffix(".result.json") {
            let text = std::fs::read_to_string(&path).unwrap();
            let result = DependencyResultV1::decode(&text).unwrap();
            assert_eq!(serde_json::to_string(&result).unwrap(), text, "{stem}");
        }
        if name.ends_with(".request.json") {
            DependencyRequestV1::decode(&std::fs::read_to_string(&path).unwrap()).unwrap();
        }
    }
}

fn corpus_directory() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../../fixtures/compat/vn-dependencies-v1")
}

#[test]
fn inconsistent_and_unsupported_results_and_requests_are_rejected_with_their_codes() {
    let directory = corpus_directory();
    for (file, decode) in [
        (
            "rejected-requests.json",
            (|text: &str| DependencyRequestV1::decode(text).map(|_| ()))
                as fn(&str) -> narrata_nodes::Result<()>,
        ),
        ("rejected-results.json", |text: &str| {
            DependencyResultV1::decode(text).map(|_| ())
        }),
    ] {
        let cases: Value =
            serde_json::from_str(&std::fs::read_to_string(directory.join(file)).unwrap()).unwrap();
        assert!(!cases.as_array().unwrap().is_empty());
        for case in cases.as_array().unwrap() {
            let error = decode(case["text"].as_str().unwrap()).unwrap_err();
            assert_eq!(error.code, case["code"].as_str().unwrap(), "{file}: {case}");
        }
    }
}

#[test]
fn the_committed_corpus_is_exactly_what_the_endpoint_emits() {
    let expected = corpus::build();
    let directory = corpus_directory();
    let mut found = BTreeSet::new();
    for entry in std::fs::read_dir(&directory).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name().into_string().unwrap();
        let bytes = std::fs::read(entry.path()).unwrap();
        assert_eq!(
            Some(&bytes),
            expected.get(&name),
            "{name} differs from a fresh build; frozen files are never rewritten in place"
        );
        found.insert(name);
    }
    assert_eq!(found, expected.keys().cloned().collect());
    // Digests in the manifest cover every other file.
    let manifest: Value = serde_json::from_slice(&expected["manifest.json"]).unwrap();
    let artifacts = manifest["artifacts"].as_object().unwrap();
    assert_eq!(artifacts.len(), expected.len() - 1);
    for (name, entry) in artifacts {
        assert_eq!(
            entry["sha256"].as_str().unwrap(),
            narrata_kernel::codec::sha256(&expected[name])
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );
    }
}

#[test]
fn the_committed_schema_is_the_one_derived_from_the_rust_definitions() {
    let schema = schemars::schema_for!(DependencyExchangeV1);
    let mut text = serde_json::to_string_pretty(&schema).unwrap();
    text.push('\n');
    let committed = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../schemas/vn-dependencies-v1.schema.json"),
    )
    .unwrap();
    assert_eq!(committed, text, "regenerate with the ignored generator");
    // The wire version and the native ranges are constraints of the schema, not prose.
    let value = serde_json::to_value(&schema).unwrap();
    let defs = &value["$defs"];
    assert_eq!(
        defs["DependencyRequestV1"]["properties"]["version"]["maximum"],
        1
    );
    assert_eq!(defs["DependencyRequestV1"]["additionalProperties"], false);
    let limits = &defs["DependencyLimitsV1"]["properties"];
    assert_eq!(limits["graphs"]["minimum"], 0);
    assert_eq!(limits["graphs"]["maximum"], DependencyLimits::MAX_GRAPHS);
    assert_eq!(limits["locations"]["minimum"], 1);
    assert_eq!(limits["scans"]["maximum"], DependencyLimits::MAX_SCANS);
}

/// The generator: `NARRATA_CORPUS_OUT=.temp/<new directory> cargo test -p narrata-nodes-wasm
/// --lib -- --ignored write_dependency_corpus`. It also writes the derived schema beside the
/// corpus so a maintainer can review and copy both.
#[test]
#[ignore = "writes a new directory under .temp"]
fn write_dependency_corpus() {
    let out = std::path::PathBuf::from(std::env::var("NARRATA_CORPUS_OUT").unwrap());
    let out = if out.is_absolute() {
        out
    } else {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../../..")
            .join(out)
    };
    corpus::write(&out);
    let mut text =
        serde_json::to_string_pretty(&schemars::schema_for!(DependencyExchangeV1)).unwrap();
    text.push('\n');
    std::fs::write(out.join("vn-dependencies-v1.schema.json"), text).unwrap();
}
