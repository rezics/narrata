//! Thin Wasm binding. Pack decoding, state transitions, validation, save restoration, R1 save
//! migration and content resolution stay in Rust; values cross the boundary as JSON text.
//! The default `legacy-publish` feature adds the raw module's `publishGraph`; the reader
//! build omits it.

#![forbid(unsafe_code)]

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod maintenance_tests {
    use super::*;

    #[test]
    fn reader_restores_and_continues_repeated_migrations_with_exact_historic_programs() {
        let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../../../fixtures/compat/nodes-evolution-v1/republished-after-migration");
        let pack = std::fs::read(directory.join("target.narpack")).unwrap();
        let mut book = NodeBook::new(&pack, "execution:0190f2a0000070008000000000000001").unwrap();
        for file in ["source.narpack", "ancestor.narpack"] {
            book.register_history_pack(&std::fs::read(directory.join(file)).unwrap())
                .unwrap();
        }
        book.memory().unwrap();
        let save = std::fs::read(directory.join("target.checkpoint")).unwrap();
        let save = save
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        book.restore(&save).unwrap();
        let view: BookView = serde_json::from_str(&book.inspect().unwrap()).unwrap();
        if let narrata_nodes::view::GamebookInteraction::Choose {
            choice_point,
            options,
            min,
            ..
        } = view.view.interaction
        {
            let selected = options
                .into_iter()
                .filter(|o| o.enabled)
                .take(usize::from(min.max(1)))
                .map(|o| o.id.to_string())
                .collect();
            book.choose(
                &view.view.cursor.to_string(),
                &choice_point.to_string(),
                selected,
            )
            .unwrap();
            book.inspect().unwrap();
        }
    }

    #[test]
    fn gamebook_reader_rejects_visual_novel_artifacts() {
        let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../../../fixtures/compat/nodes-vn-v1");
        let pack = std::fs::read(directory.join("story.narpack")).unwrap();
        let (program, _) = Program::from_pack(&pack).unwrap();
        let error = ensure_kind(
            &program,
            ArtifactKind::Gamebook,
            "pack",
            "NodeBook accepts Gamebook artifacts only",
        )
        .unwrap_err();
        assert_eq!(error.code, "version");
    }

    #[test]
    fn vn_reader_emits_its_checked_versioned_view_and_rejects_gamebook_packs() {
        let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../../../fixtures/compat/nodes-vn-v1");
        let pack = std::fs::read(directory.join("story.narpack")).unwrap();
        let mut reader = VnBook::new(&pack, "execution:0190f2a0000070008000000000000001").unwrap();
        reader.memory().unwrap();
        let value: serde_json::Value = serde_json::from_str(&reader.inspect().unwrap()).unwrap();
        assert_eq!(value["version"], 1);
        assert!(value["view"]["scene"].is_object());
        let checked: narrata_nodes::view::VnViewV1 = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(checked.version(), 1);

        let mut unsupported = value.clone();
        unsupported["version"] = serde_json::json!(2);
        assert!(serde_json::from_value::<narrata_nodes::view::VnViewV1>(unsupported).is_err());
        let mut missing_scene = value;
        missing_scene["view"]
            .as_object_mut()
            .unwrap()
            .remove("scene");
        assert!(serde_json::from_value::<narrata_nodes::view::VnViewV1>(missing_scene).is_err());

        let gamebook = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../../../products/gamebook-demo/story.narpack");
        let gamebook = std::fs::read(gamebook).unwrap();
        let (gamebook_program, _) = Program::from_pack(&gamebook).unwrap();
        let (vn_program, _) = Program::from_pack(&pack).unwrap();
        assert!(
            ensure_kind(
                &gamebook_program,
                ArtifactKind::VisualNovel,
                "pack",
                "VnBook accepts visual novel artifacts only",
            )
            .is_err()
        );
        assert!(
            ensure_kind(
                &vn_program,
                ArtifactKind::Gamebook,
                "pack",
                "NodeBook accepts Gamebook artifacts only",
            )
            .is_err()
        );
    }
}

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use narrata_content_local::{ContentPack, ResolveRequest};
use narrata_kernel::content::ProviderId;
use narrata_nodes::{
    Analysis, AuthoredId, BookView, ChunkSource, Error, Lookup, MAX_REQUEST_BYTES, MemorySource,
    NameTable, ObjectId, Program, ProposalRequest, Session, analyze, parse_json_limited, r1,
};
use narrata_storage_host::{CacheBackend, FlushReply, HostError, Loaded, StoreId, StoreState};
use wasm_bindgen::prelude::*;

fn js_error(error: Error) -> JsError {
    JsError::new(&error.to_string())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ArtifactKind {
    Gamebook,
    VisualNovel,
}

fn ensure_kind(
    program: &Program,
    expected: ArtifactKind,
    path: &str,
    message: &str,
) -> narrata_nodes::Result<()> {
    let is_expected = match expected {
        ArtifactKind::Gamebook => !program.is_visual_novel(),
        ArtifactKind::VisualNovel => program.is_visual_novel(),
    };
    if is_expected {
        Ok(())
    } else {
        Err(Error::new("version", path, message))
    }
}

fn parse<T: std::str::FromStr>(text: &str, path: &str, expected: &str) -> Result<T, JsError> {
    text.parse().map_err(|_| {
        js_error(Error::new(
            "identifier",
            path,
            format!("expected {expected}"),
        ))
    })
}

fn json<T: serde::Serialize>(value: &T) -> Result<String, JsError> {
    serde_json::to_string(value).map_err(|error| JsError::new(&error.to_string()))
}

#[cfg(feature = "legacy-publish")]
#[derive(serde::Serialize)]
struct PublicationObject<'a> {
    object_id: String,
    bytes: &'a [u8],
}

#[cfg(feature = "legacy-publish")]
#[derive(serde::Serialize)]
struct GraphPublication<'a> {
    index: PublicationObject<'a>,
    tiles: Vec<PublicationObject<'a>>,
    labels: Vec<PublicationObject<'a>>,
    files_json: String,
    summary_json: String,
    analysis_json: String,
    diagnostics: &'a [narrata_nodes::Diagnostic],
}

/// Publication from a complete pack, using exactly the CLI's Rust projection and
/// diagnostics. `outlines` is an optional JSON array of provider outlines. Object bytes
/// are arrays of u8; JSON files are strings preserving the CLI's exact output bytes.
/// This is a host API value, not a new stored artifact format.
#[cfg(feature = "legacy-publish")]
#[wasm_bindgen(js_name = publishGraph)]
pub fn publish_graph(pack: &[u8], outlines: Option<String>) -> Result<String, JsError> {
    let (program, names) = Program::from_pack(pack).map_err(js_error)?;
    ensure_kind(
        &program,
        ArtifactKind::Gamebook,
        "pack",
        "publishGraph accepts Gamebook artifacts only",
    )
    .map_err(js_error)?;
    program.verify_artifact().map_err(js_error)?;
    let outlines: Vec<narrata_nodes::ContentOutline> = outlines
        .as_deref()
        .map(|text| parse_json_limited(text, narrata_nodes::MAX_SOURCE_BYTES))
        .transpose()
        .map_err(js_error)?
        .unwrap_or_default();
    let publication =
        narrata_node_tools::publish::publish(&program, names.as_ref()).map_err(js_error)?;
    let analysis = narrata_node_tools::publish::publication_analysis(
        analyze(&program, names.as_ref()).map_err(js_error)?,
        &program,
        names.as_ref(),
        &publication,
        &outlines,
    )
    .map_err(js_error)?;
    let object = |id: [u8; 32], bytes| PublicationObject {
        object_id: hex::encode(id),
        bytes,
    };
    let files_json = serde_json::to_string_pretty(&publication.files)
        .map(|text| text + "\n")
        .map_err(|e| JsError::new(&e.to_string()))?;
    let analysis_json = serde_json::to_string_pretty(&analysis)
        .map(|text| text + "\n")
        .map_err(|e| JsError::new(&e.to_string()))?;
    let summary = publication
        .summary
        .encode_json()
        .map_err(|e| JsError::new(&e.to_string()))?;
    let result = GraphPublication {
        index: object(
            publication.geometry.index.id,
            &publication.geometry.index.bytes,
        ),
        tiles: publication
            .geometry
            .tiles
            .iter()
            .map(|o| object(o.id, o.bytes.as_slice()))
            .collect(),
        labels: publication
            .labels
            .iter()
            .map(|o| object(o.id, o.bytes.as_slice()))
            .collect(),
        files_json,
        summary_json: String::from_utf8(summary).map_err(|e| JsError::new(&e.to_string()))?,
        analysis_json,
        diagnostics: &analysis.diagnostics,
    };
    json(&result)
}

#[derive(Default)]
struct PendingChunks {
    objects: BTreeMap<ObjectId, Vec<u8>>,
    request: Option<ObjectId>,
}

#[derive(Clone, Default)]
struct HostChunks(Arc<Mutex<PendingChunks>>);

impl ChunkSource for HostChunks {
    fn load(&self, id: &ObjectId) -> narrata_nodes::Result<Vec<u8>> {
        let mut pending = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // A retried operation can span more than the decoded cache budget. Retain its
        // supplied bytes until success so retries make progress after decoded eviction.
        if let Some(bytes) = pending.objects.get(id) {
            return Ok(bytes.clone());
        }
        pending.request = Some(*id);
        Err(Error::new(
            "not_loaded",
            id.to_string(),
            "host must load this program object and retry",
        ))
    }
}

/// Read-only save discovery before the host has loaded an artifact.
#[wasm_bindgen]
pub struct SaveLocator {
    cache: CacheBackend,
    execution: narrata_nodes::ExecutionId,
    proof: Option<narrata_nodes::history::locator::RetentionProof>,
}

#[wasm_bindgen]
impl SaveLocator {
    #[wasm_bindgen(constructor)]
    pub fn new(execution: &str) -> Result<Self, JsError> {
        Ok(Self {
            cache: CacheBackend::new(),
            execution: parse(execution, "execution", "execution:<32 hex digits>")?,
            proof: None,
        })
    }
    pub fn locate(&self) -> Result<Option<String>, JsError> {
        narrata_nodes::history::locator::locate(&self.cache, self.execution)
            .map_err(|error| JsError::new(&error.to_string()))?
            .map(|location| json(&(location.cursor, location.artifact)))
            .transpose()
    }
    pub fn retained(&mut self, candidate: &str, roots: u32, commits: u32) -> Result<bool, JsError> {
        use narrata_nodes::history::locator::{Limits, ProofScope, RetentionProof};
        let candidate = parse(candidate, "candidate", "commit:<64 hex digits>")?;
        let limits = Limits {
            roots: roots as usize,
            commits: commits as usize,
        };
        if self
            .proof
            .as_ref()
            .is_none_or(|proof| !proof.matches(candidate, limits))
        {
            self.proof = Some(RetentionProof::new(self.execution, candidate, limits));
        }
        let Some(store) = self.cache.store() else {
            // Record a normal cache miss without manufacturing an unscoped proof snapshot.
            self.locate()?;
            return Err(JsError::new("load the store before verifying retention"));
        };
        self.proof
            .as_mut()
            .ok_or_else(|| JsError::new("missing retention query"))?
            .poll(
                &self.cache,
                ProofScope {
                    store: *store.id.as_bytes(),
                    revision: store.revision,
                },
            )
            .map_err(|error| JsError::new(&error.to_string()))
    }
    #[wasm_bindgen(js_name = takeRequest)]
    pub fn take_request(&self) -> Option<Vec<u8>> {
        self.cache.take_request().map(|request| request.encode())
    }
    pub fn load(&self, bytes: &[u8]) -> Result<bool, JsError> {
        self.cache.load(&Loaded::decode(bytes)?)?;
        Ok(true)
    }
    pub fn unconfirmed(&self) -> Option<Vec<u8>> {
        None
    }
    pub fn confirm(&self, _bytes: &[u8]) -> Result<bool, JsError> {
        Err(JsError::new("read-only locator cannot confirm writes"))
    }
}

struct ReaderArtifact {
    pack: Option<Vec<u8>>,
    names: Option<NameTable>,
    analysis: Option<Analysis>,
    chunks: Option<HostChunks>,
    program: Arc<Program>,
}

/// A checked artifact registry and one session. The cursor selects its active context.
#[wasm_bindgen]
pub struct NodeBook {
    kind: ArtifactKind,
    artifacts: BTreeMap<narrata_nodes::ArtifactId, ReaderArtifact>,
    initial: narrata_nodes::ArtifactId,
    execution: narrata_nodes::ExecutionId,
    cache: CacheBackend,
    session: Option<Session<CacheBackend>>,
    history_bytes: usize,
}

impl NodeBook {
    fn from_pack(pack: &[u8], execution: &str, kind: ArtifactKind) -> Result<NodeBook, JsError> {
        let (program, names) = Program::from_pack(pack).map_err(js_error)?;
        ensure_kind(
            &program,
            kind,
            "pack",
            match kind {
                ArtifactKind::Gamebook => "NodeBook accepts Gamebook artifacts only",
                ArtifactKind::VisualNovel => "VnBook accepts visual novel artifacts only",
            },
        )
        .map_err(js_error)?;
        program.verify_artifact().map_err(js_error)?;
        let analysis = match kind {
            ArtifactKind::Gamebook => Some(analyze(&program, names.as_ref()).map_err(js_error)?),
            ArtifactKind::VisualNovel => None,
        };
        let execution = parse(execution, "execution", "execution:<32 hex digits>")?;
        let initial = program.artifact_id();
        Ok(Self {
            kind,
            artifacts: [(
                initial,
                ReaderArtifact {
                    pack: Some(pack.to_vec()),
                    names,
                    analysis,
                    chunks: None,
                    program: Arc::new(program),
                },
            )]
            .into(),
            initial,
            execution,
            cache: CacheBackend::new(),
            session: None,
            history_bytes: 0,
        })
    }

    fn from_manifest_kind(
        manifest: &[u8],
        execution: &str,
        kind: ArtifactKind,
    ) -> Result<NodeBook, JsError> {
        let chunks = HostChunks::default();
        let program =
            Program::from_manifest(manifest, Box::new(chunks.clone())).map_err(js_error)?;
        ensure_kind(
            &program,
            kind,
            "manifest",
            match kind {
                ArtifactKind::Gamebook => "NodeBook accepts Gamebook artifacts only",
                ArtifactKind::VisualNovel => "VnBook accepts visual novel artifacts only",
            },
        )
        .map_err(js_error)?;
        let initial = program.artifact_id();
        Ok(Self {
            kind,
            artifacts: [(
                initial,
                ReaderArtifact {
                    pack: None,
                    names: None,
                    analysis: None,
                    chunks: Some(chunks),
                    program: Arc::new(program),
                },
            )]
            .into(),
            initial,
            execution: parse(execution, "execution", "execution:<32 hex digits>")?,
            cache: CacheBackend::new(),
            session: None,
            history_bytes: 0,
        })
    }

    fn active(&self) -> &ReaderArtifact {
        let id = self
            .session
            .as_ref()
            .map_or(self.initial, |s| s.program().artifact_id());
        &self.artifacts[&id]
    }
    fn artifact(&self, id: narrata_nodes::ArtifactId) -> Result<&ReaderArtifact, JsError> {
        self.artifacts.get(&id).ok_or_else(|| {
            js_error(Error::new(
                "needs_artifact",
                id.to_string(),
                "exact historic program is unavailable",
            ))
        })
    }
    fn register(&mut self, artifact: ReaderArtifact, bytes: usize) -> Result<(), JsError> {
        let id = artifact.program.artifact_id();
        if self.artifacts.contains_key(&id) {
            return Ok(());
        }
        let total = self
            .history_bytes
            .checked_add(bytes)
            .filter(|n| *n <= 64 * 1024 * 1024)
            .ok_or_else(|| {
                js_error(Error::new(
                    "limit",
                    "history",
                    "historical metadata/packs exceed 64 MiB",
                ))
            })?;
        if self.artifacts.len() >= 65 {
            return Err(js_error(Error::new(
                "limit",
                "history",
                "at most 64 historical artifacts",
            )));
        }
        if let Some(session) = &mut self.session {
            session
                .register_history_program(artifact.program.clone())
                .map_err(js_error)?;
        }
        self.artifacts.insert(id, artifact);
        self.history_bytes = total;
        Ok(())
    }
}

#[wasm_bindgen]
impl NodeBook {
    /// Opens a pack and checks every chunk. Call open or memory to start the session.
    #[wasm_bindgen(constructor)]
    pub fn new(pack: &[u8], execution: &str) -> Result<NodeBook, JsError> {
        Self::from_pack(pack, execution, ArtifactKind::Gamebook)
    }

    /// Opens a checked manifest; missing chunks are requested and checked on demand.
    /// Finish retrying the pending operation before starting another runtime/tool call.
    #[wasm_bindgen(js_name = fromManifest)]
    pub fn from_manifest(manifest: &[u8], execution: &str) -> Result<NodeBook, JsError> {
        Self::from_manifest_kind(manifest, execution, ArtifactKind::Gamebook)
    }

    /// Registers only metadata. No graph bodies are fetched to register an old program.
    #[wasm_bindgen(js_name = registerHistoryManifest)]
    pub fn register_history_manifest(
        &mut self,
        expected: &str,
        manifest: &[u8],
        names: Option<Vec<u8>>,
    ) -> Result<(), JsError> {
        let expected = parse(expected, "artifact", "artifact:<64 hex digits>")?;
        let chunks = HostChunks::default();
        let program =
            Program::from_manifest(manifest, Box::new(chunks.clone())).map_err(js_error)?;
        ensure_kind(
            &program,
            self.kind,
            "history.manifest",
            "historical artifact format does not match this reader",
        )
        .map_err(js_error)?;
        if program.artifact_id() != expected {
            return Err(js_error(Error::new(
                "artifact",
                expected.to_string(),
                "manifest differs from requested artifact",
            )));
        }
        let bytes = manifest.len() + names.as_ref().map_or(0, Vec::len);
        let names = names
            .as_deref()
            .map(|b| program.decode_names(b))
            .transpose()
            .map_err(js_error)?;
        self.register(
            ReaderArtifact {
                pack: None,
                names,
                analysis: None,
                chunks: Some(chunks),
                program: Arc::new(program),
            },
            bytes,
        )
    }

    /// Includes artifact identity so a historical miss never uses the current chunk loader.
    #[wasm_bindgen(js_name = takeProgramRequest)]
    pub fn take_program_request(&self) -> Result<Option<String>, JsError> {
        for (artifact, context) in &self.artifacts {
            if let Some(chunks) = &context.chunks {
                let request = chunks
                    .0
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .request
                    .take();
                if let Some(id) = request {
                    return json(&(*artifact, id)).map(Some);
                }
            }
        }
        Ok(None)
    }

    #[wasm_bindgen(js_name = takeChunkRequest)]
    pub fn take_chunk_request(&self) -> Option<String> {
        self.active()
            .chunks
            .as_ref()?
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .request
            .take()
            .map(|id| id.to_string())
    }

    /// Accepts only a manifest-referenced object with a checked envelope and matching
    /// object ID. Program checks its canonical payload and semantics when used.
    #[wasm_bindgen(js_name = loadChunk)]
    pub fn load_chunk(&self, id: &str, bytes: &[u8]) -> Result<(), JsError> {
        self.load_artifact_chunk(&self.artifact_id(), id, bytes)
    }

    #[wasm_bindgen(js_name = loadArtifactChunk)]
    pub fn load_artifact_chunk(
        &self,
        artifact: &str,
        id: &str,
        bytes: &[u8],
    ) -> Result<(), JsError> {
        let artifact = parse(artifact, "artifact", "artifact:<64 hex digits>")?;
        let context = self.artifact(artifact)?;
        let id: ObjectId = parse(id, "object", "object:<64 hex digits>")?;
        if id != context.program.manifest().tombstones
            && !context.program.manifest().chunks.contains(&id)
        {
            return Err(js_error(Error::new(
                "reference",
                id.to_string(),
                "object is not in this manifest",
            )));
        }
        let chunks = context
            .chunks
            .as_ref()
            .ok_or_else(|| JsError::new("loadChunk requires fromManifest"))?;
        let actual = MemorySource::default()
            .insert(bytes.to_vec())
            .map_err(js_error)?;
        if id != actual {
            return Err(js_error(Error::new(
                "artifact",
                id.to_string(),
                "object content differs from its object ID",
            )));
        }
        let pending_bytes: usize = self
            .artifacts
            .values()
            .filter_map(|a| a.chunks.as_ref())
            .map(|chunks| {
                chunks
                    .0
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .objects
                    .values()
                    .map(Vec::len)
                    .sum::<usize>()
            })
            .sum();
        if pending_bytes.saturating_add(bytes.len()) > 64 * 1024 * 1024 {
            return Err(js_error(Error::new(
                "limit",
                "program_objects",
                "pending program objects exceed 64 MiB",
            )));
        }
        chunks
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .objects
            .insert(id, bytes.to_vec());
        Ok(())
    }

    /// Optional aliases are checked against this artifact without loading graph bodies.
    #[wasm_bindgen(js_name = loadNames)]
    pub fn load_names(&mut self, bytes: &[u8]) -> Result<(), JsError> {
        let names = self
            .active()
            .program
            .decode_names(bytes)
            .map_err(js_error)?;
        let id = self.active().program.artifact_id();
        if let Some(context) = self.artifacts.get_mut(&id) {
            context.names = Some(names);
        }
        Ok(())
    }

    /// Exact historical artifacts for rendering/continuing a restored migrated save.
    /// This adds no migration evaluator, publication compiler or trace to the reader.
    #[wasm_bindgen(js_name = registerHistoryPack)]
    pub fn register_history_pack(&mut self, bytes: &[u8]) -> Result<(), JsError> {
        let (program, names) = Program::from_pack(bytes).map_err(js_error)?;
        ensure_kind(
            &program,
            self.kind,
            "pack",
            "historical artifact format does not match this reader",
        )
        .map_err(js_error)?;
        program.verify_artifact().map_err(js_error)?;
        let analysis = analyze(&program, names.as_ref()).map_err(js_error)?;
        self.register(
            ReaderArtifact {
                pack: Some(bytes.to_vec()),
                names,
                analysis: Some(analysis),
                chunks: None,
                program: Arc::new(program),
            },
            bytes.len(),
        )
    }

    /// Requires all chunks and the tombstone set, including chunks already cached.
    #[wasm_bindgen(js_name = verifyArtifact)]
    pub fn verify_artifact(&self) -> Result<(), JsError> {
        self.finish(self.active().program.verify_artifact().map_err(js_error))
    }

    /// Tool lookup, loading tombstones first. Live results include their structural owner.
    pub fn lookup(&self, id: &str) -> Result<String, JsError> {
        let id: AuthoredId = parse(id, "id", "an authored ID")?;
        let found = match self.active().program.lookup(id).map_err(js_error)? {
            Lookup::Deleted => serde_json::json!({"kind": "deleted"}),
            Lookup::Unknown => serde_json::json!({"kind": "unknown"}),
            Lookup::Live {
                graph,
                node,
                choice_point,
            } => serde_json::json!({
                "kind": "live", "graph": graph, "node": node, "choice_point": choice_point
            }),
        };
        self.finish(json(&found))
    }

    #[wasm_bindgen(getter)]
    pub fn artifact_id(&self) -> String {
        self.active().program.artifact_id().to_string()
    }

    /// Candidate identity is available to the host before persistence is acknowledged.
    pub fn cursor(&self) -> Result<String, JsError> {
        Ok(self.opened()?.cursor().map_err(js_error)?.to_string())
    }

    /// Opens the kernel cursor, creating it only if absent. Retried after host loads.
    pub fn open(&mut self) -> Result<(), JsError> {
        // Discover before dropping the old session. Reload and checkout may have moved
        // the durable cursor to another registered artifact.
        let location = narrata_nodes::history::locator::locate(&self.cache, self.execution)
            .map_err(|error| JsError::new(&error.to_string()))?;
        let id = location.map_or(self.initial, |location| location.artifact);
        let program = self.artifact(id)?.program.clone();
        ensure_kind(
            &program,
            self.kind,
            "history.artifact",
            "saved artifact format does not match this reader",
        )
        .map_err(js_error)?;
        self.session = None;
        let mut session =
            Session::open(program, self.execution, self.cache.share()).map_err(js_error)?;
        for context in self.artifacts.values() {
            session
                .register_history_program(context.program.clone())
                .map_err(js_error)?;
        }
        self.session = Some(session);
        self.finish(Ok(()))
    }

    /// Restores an existing execution without creating a replacement cursor if it disappears.
    /// Repeating discovery on every cache retry prevents a miss from turning resume into create.
    pub fn resume(&mut self) -> Result<(), JsError> {
        if narrata_nodes::history::locator::locate(&self.cache, self.execution)
            .map_err(|error| JsError::new(&error.to_string()))?
            .is_none()
        {
            return Err(JsError::new("the saved execution has no active cursor"));
        }
        self.open()
    }

    fn finish<T>(&self, result: Result<T, JsError>) -> Result<T, JsError> {
        if result.is_ok() {
            for context in self.artifacts.values() {
                if let Some(chunks) = &context.chunks {
                    let mut pending = chunks
                        .0
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    pending.objects.clear();
                    pending.request = None;
                }
            }
        }
        result
    }

    #[wasm_bindgen(js_name = takeRequest)]
    pub fn take_request(&self) -> Option<Vec<u8>> {
        self.cache.take_request().map(|r| r.encode())
    }
    pub fn load(&mut self, bytes: &[u8]) -> Result<bool, JsError> {
        let result = self.cache.load(&Loaded::decode(bytes)?);
        self.settle(result)
    }
    pub fn unconfirmed(&self) -> Option<Vec<u8>> {
        self.cache.unconfirmed().map(|r| r.encode())
    }
    pub fn confirm(&mut self, bytes: &[u8]) -> Result<bool, JsError> {
        let result = self.cache.confirm(&FlushReply::decode(bytes)?);
        self.settle(result)
    }
    fn settle(&mut self, result: Result<(), HostError>) -> Result<bool, JsError> {
        match result {
            Ok(()) => Ok(true),
            Err(HostError::Superseded(_)) => {
                self.session = None;
                Ok(false)
            }
            Err(e) => Err(e.into()),
        }
    }
    pub fn reload(&mut self) {
        self.session = None;
        self.cache.invalidate();
    }
    /// Starts an isolated, fully known cache when browser persistence is unavailable.
    pub fn memory(&mut self) -> Result<(), JsError> {
        self.session = None;
        self.cache.invalidate();
        self.cache.load(&Loaded {
            store: StoreState {
                id: StoreId::from_bytes(*self.execution.as_bytes()),
                revision: 0,
            },
            keys: vec![],
            objects: vec![],
            key_ranges: vec![],
            object_ranges: vec![],
        })?;
        self.open()
    }
    /// Acknowledges memory-only batches so the fallback retains no pending flush history.
    pub fn confirm_memory(&mut self) -> Result<(), JsError> {
        if let Some(flush) = self.cache.unconfirmed()
            && let Some(batch) = flush.batches.last()
        {
            self.cache.confirm(&FlushReply::Persisted {
                revision: batch.revision,
            })?;
        }
        Ok(())
    }
    fn opened(&self) -> Result<&Session<CacheBackend>, JsError> {
        self.session
            .as_ref()
            .ok_or_else(|| JsError::new("open the session first"))
    }
    fn opened_mut(&mut self) -> Result<&mut Session<CacheBackend>, JsError> {
        self.session
            .as_mut()
            .ok_or_else(|| JsError::new("open the session first"))
    }

    /// The original full pack. Manifest-only books leave pack assembly to the host.
    pub fn pack(&self) -> Result<Vec<u8>, JsError> {
        match &self.active().pack {
            Some(pack) => Ok(pack.clone()),
            None => Err(JsError::new(
                "pack bytes are unavailable on a manifest-only book",
            )),
        }
    }

    /// Prepares possible next graph chunks between interactions, without advancing history.
    pub fn prefetch(&mut self) -> Result<(), JsError> {
        let result = self.opened_mut()?.prefetch().map_err(js_error);
        self.finish(result)
    }

    /// Possible body-unit references after one choice, without changing history or BookView.
    #[wasm_bindgen(js_name = nextContentUnits)]
    pub fn next_content_units(&self) -> Result<String, JsError> {
        let units = narrata_nodes::view::next_content_units(
            &self.active().program,
            self.opened()?.state().map_err(js_error)?,
        )
        .map_err(js_error)?;
        self.finish(json(&units))
    }

    /// The text-free book view as JSON.
    pub fn inspect(&self) -> Result<String, JsError> {
        let graphs = self
            .active()
            .analysis
            .as_ref()
            .map_or_else(Vec::new, |analysis| analysis.graphs.clone())
            .into_iter()
            .map(TryInto::try_into)
            .collect::<narrata_nodes::Result<Vec<_>>>()
            .map_err(js_error)?;
        self.finish(json(&BookView {
            view: self
                .opened()?
                .view_with_names(|id| self.artifacts.get(&id).and_then(|a| a.names.as_ref()))
                .map_err(js_error)?
                .try_into()
                .map_err(js_error)?,
            page: self.opened()?.page().map_err(js_error)?,
            // Runtime inspection must not eagerly scan the whole work. Publication
            // analysis is a separate operation for hosts that need the full graph.
            graphs,
            diagnostics: self
                .active()
                .analysis
                .as_ref()
                .map_or_else(Vec::new, |a| a.diagnostics.clone()),
        }))
    }

    /// Derived provenance for host content contexts; this changes no stored/read-model format.
    #[wasm_bindgen(js_name = historyArtifacts)]
    pub fn history_artifacts(&self, expected: &str) -> Result<String, JsError> {
        let session = self.opened()?;
        let expected = parse(expected, "expected", "commit:<64 hex digits>")?;
        if session.cursor().map_err(js_error)? != expected {
            return Err(js_error(Error::new(
                "stale_input",
                "cursor",
                "cursor changed before content provenance read",
            )));
        }
        let artifacts: BTreeMap<_, _> = session
            .commits()
            .map_err(js_error)?
            .map(|(id, commit)| (id, commit.artifact))
            .collect();
        self.finish(json(&artifacts))
    }

    /// Chooses a set of option IDs at the cursor, which must still be `expected`, and returns
    /// the new cursor.
    pub fn choose(
        &mut self,
        expected: &str,
        choice_point: &str,
        options: Vec<String>,
    ) -> Result<String, JsError> {
        let expected = parse(expected, "expected", "commit:<64 hex digits>")?;
        let choice_point = parse(choice_point, "choice_point", "choice-point:<32 hex digits>")?;
        let options = options
            .iter()
            .map(|option| parse(option, "options", "option:<32 hex digits>"))
            .collect::<Result<Vec<_>, _>>()?;
        let commit = self
            .opened_mut()?
            .choose(&expected, choice_point, options)
            .map_err(js_error)?;
        self.finish(Ok(commit.to_string()))
    }

    /// Records a host proposal (a proposal request as JSON) at the cursor, which must still be
    /// `expected`, and returns the new cursor. The host inspects the view after confirming
    /// the pending batches. The interaction stays at the same choice point.
    pub fn propose(&mut self, expected: &str, request: &str) -> Result<String, JsError> {
        let expected = parse(expected, "expected", "commit:<64 hex digits>")?;
        let request: ProposalRequest =
            parse_json_limited(request, MAX_REQUEST_BYTES).map_err(js_error)?;
        let commit = self
            .opened_mut()?
            .propose(&expected, &request)
            .map_err(js_error)?;
        self.finish(Ok(commit.to_string()))
    }

    pub fn checkout(&mut self, commit: &str) -> Result<(), JsError> {
        let commit = parse(commit, "commit", "commit:<64 hex digits>")?;
        let result = self.opened_mut()?.checkout(&commit).map_err(js_error);
        self.finish(result)
    }

    /// The session export JSON.
    pub fn export(&self) -> Result<String, JsError> {
        self.finish(self.opened()?.export().map_err(js_error))
    }

    /// Replaces the session with a checked export of this artifact; the current session stays
    /// on failure.
    pub fn restore(&mut self, export: &str) -> Result<(), JsError> {
        self.opened_mut()?.import(export).map_err(js_error)?;
        self.finish(Ok(()))
    }

    /// Rebuilds an R1 save on this artifact, which must have been migrated from the save's R1
    /// artifact. `content` resolves the references that replaced R1 text values. The current
    /// session stays on failure.
    pub fn migrate_r1(
        &mut self,
        save: &str,
        content: &LocalContent,
        execution: &str,
    ) -> Result<(), JsError> {
        let names = self.active().names.as_ref().ok_or_else(|| {
            js_error(Error::new(
                "names",
                "pack",
                "R1 save migration needs the pack's name table",
            ))
        })?;
        let execution = parse(execution, "execution", "execution:<32 hex digits>")?;
        let ref_text = |reference: &_| content.inner.text(reference, &[]);
        let migrated = r1::migrate_save(
            self.active().program.clone(),
            names,
            save,
            execution,
            &ref_text,
        )
        .map_err(js_error)?;
        self.opened_mut()?
            .import_session(&migrated)
            .map_err(js_error)?;
        self.finish(Ok(()))
    }
}

/// The VN-only Wasm reader facade. It shares the checked session and cache transport with
/// `NodeBook`, while never serializing a Gamebook `BookView`.
#[wasm_bindgen]
pub struct VnBook {
    inner: NodeBook,
}

#[wasm_bindgen]
impl VnBook {
    #[wasm_bindgen(constructor)]
    pub fn new(pack: &[u8], execution: &str) -> Result<VnBook, JsError> {
        Ok(Self {
            inner: NodeBook::from_pack(pack, execution, ArtifactKind::VisualNovel)?,
        })
    }

    #[wasm_bindgen(js_name = fromManifest)]
    pub fn from_manifest(manifest: &[u8], execution: &str) -> Result<VnBook, JsError> {
        Ok(Self {
            inner: NodeBook::from_manifest_kind(manifest, execution, ArtifactKind::VisualNovel)?,
        })
    }

    #[wasm_bindgen(js_name = registerHistoryManifest)]
    pub fn register_history_manifest(
        &mut self,
        expected: &str,
        manifest: &[u8],
        names: Option<Vec<u8>>,
    ) -> Result<(), JsError> {
        self.inner
            .register_history_manifest(expected, manifest, names)
    }

    #[wasm_bindgen(js_name = takeProgramRequest)]
    pub fn take_program_request(&self) -> Result<Option<String>, JsError> {
        self.inner.take_program_request()
    }

    #[wasm_bindgen(js_name = takeChunkRequest)]
    pub fn take_chunk_request(&self) -> Option<String> {
        self.inner.take_chunk_request()
    }

    #[wasm_bindgen(js_name = loadChunk)]
    pub fn load_chunk(&self, id: &str, bytes: &[u8]) -> Result<(), JsError> {
        self.inner.load_chunk(id, bytes)
    }

    #[wasm_bindgen(js_name = loadArtifactChunk)]
    pub fn load_artifact_chunk(
        &self,
        artifact: &str,
        id: &str,
        bytes: &[u8],
    ) -> Result<(), JsError> {
        self.inner.load_artifact_chunk(artifact, id, bytes)
    }

    #[wasm_bindgen(js_name = loadNames)]
    pub fn load_names(&mut self, bytes: &[u8]) -> Result<(), JsError> {
        self.inner.load_names(bytes)
    }

    #[wasm_bindgen(js_name = registerHistoryPack)]
    pub fn register_history_pack(&mut self, bytes: &[u8]) -> Result<(), JsError> {
        self.inner.register_history_pack(bytes)
    }

    #[wasm_bindgen(js_name = verifyArtifact)]
    pub fn verify_artifact(&self) -> Result<(), JsError> {
        self.inner.verify_artifact()
    }

    #[wasm_bindgen(getter)]
    pub fn artifact_id(&self) -> String {
        self.inner.artifact_id()
    }

    pub fn cursor(&self) -> Result<String, JsError> {
        self.inner.cursor()
    }

    pub fn open(&mut self) -> Result<(), JsError> {
        self.inner.open()
    }

    pub fn resume(&mut self) -> Result<(), JsError> {
        self.inner.resume()
    }

    #[wasm_bindgen(js_name = takeRequest)]
    pub fn take_request(&self) -> Option<Vec<u8>> {
        self.inner.take_request()
    }

    pub fn load(&mut self, bytes: &[u8]) -> Result<bool, JsError> {
        self.inner.load(bytes)
    }

    pub fn unconfirmed(&self) -> Option<Vec<u8>> {
        self.inner.unconfirmed()
    }

    pub fn confirm(&mut self, bytes: &[u8]) -> Result<bool, JsError> {
        self.inner.confirm(bytes)
    }

    pub fn reload(&mut self) {
        self.inner.reload();
    }

    pub fn memory(&mut self) -> Result<(), JsError> {
        self.inner.memory()
    }

    pub fn confirm_memory(&mut self) -> Result<(), JsError> {
        self.inner.confirm_memory()
    }

    pub fn pack(&self) -> Result<Vec<u8>, JsError> {
        self.inner.pack()
    }

    /// The versioned VN view carries a required checked scene target and its own closed
    /// interaction union. A Gamebook artifact is rejected before it reaches this conversion.
    pub fn inspect(&self) -> Result<String, JsError> {
        let view = self
            .inner
            .opened()?
            .view_with_names(|id| self.inner.artifacts.get(&id).and_then(|a| a.names.as_ref()))
            .map_err(js_error)?;
        let view = narrata_nodes::view::VnViewV1::try_from(view).map_err(js_error)?;
        self.inner.finish(json(&view))
    }

    #[wasm_bindgen(js_name = historyArtifacts)]
    pub fn history_artifacts(&self, expected: &str) -> Result<String, JsError> {
        self.inner.history_artifacts(expected)
    }

    /// Advances only the currently shown dialogue at the expected commit.
    pub fn advance(&mut self, expected: &str, dialogue: &str) -> Result<String, JsError> {
        let expected = parse(expected, "expected", "commit:<64 hex digits>")?;
        let dialogue = parse(dialogue, "dialogue", "node:<32 hex digits>")?;
        let commit = self
            .inner
            .opened_mut()?
            .advance(&expected, dialogue)
            .map_err(js_error)?;
        self.inner.finish(Ok(commit.to_string()))
    }

    /// Chooses options at the current choice point; it remains a separate operation from Advance.
    pub fn choose(
        &mut self,
        expected: &str,
        choice_point: &str,
        options: Vec<String>,
    ) -> Result<String, JsError> {
        self.inner.choose(expected, choice_point, options)
    }

    pub fn checkout(&mut self, commit: &str) -> Result<(), JsError> {
        self.inner.checkout(commit)
    }

    pub fn export(&self) -> Result<String, JsError> {
        self.inner.export()
    }

    pub fn restore(&mut self, export: &str) -> Result<(), JsError> {
        self.inner.restore(export)
    }
}

/// The local content provider: content packs of one work, the first being the original
/// language that lookups fall back to.
#[wasm_bindgen]
#[derive(Default)]
pub struct LocalContent {
    inner: narrata_content_local::LocalContent,
}

#[wasm_bindgen]
impl LocalContent {
    #[wasm_bindgen(constructor)]
    pub fn new() -> LocalContent {
        Self::default()
    }

    /// Adds a content pack given as JSON text.
    pub fn add(&mut self, pack: &str) -> Result<(), JsError> {
        self.inner
            .add(ContentPack::parse(pack).map_err(js_error)?)
            .map_err(js_error)
    }

    /// Resolves a batch: a resolve request as JSON in, the resolutions in order as JSON out.
    pub fn resolve(&self, request: &str) -> Result<String, JsError> {
        let request: ResolveRequest =
            parse_json_limited(request, 8 * 1024 * 1024).map_err(js_error)?;
        json(&self.inner.resolve(&request).map_err(js_error)?)
    }

    /// The content outline of `provider` in the best matching language, as JSON, if any pack
    /// serves the provider.
    pub fn outline(
        &self,
        provider: &str,
        languages: Vec<String>,
    ) -> Result<Option<String>, JsError> {
        let provider: ProviderId = parse(provider, "provider", "a provider ID")?;
        self.inner
            .outline(&provider, &languages)
            .map(|outline| json(&outline))
            .transpose()
    }
}
