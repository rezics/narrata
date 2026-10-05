//! Thin Wasm binding. Pack decoding, state transitions, validation, save restoration, R1 save
//! migration and content resolution stay in Rust; values cross the boundary as JSON text.

#![forbid(unsafe_code)]

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use narrata_content_local::{ContentPack, ResolveRequest};
use narrata_kernel::content::ProviderId;
use narrata_nodes::{
    Analysis, AuthoredId, BookView, ChunkSource, Error, Lookup, MAX_REQUEST_BYTES,
    MAX_SOURCE_BYTES, MemorySource, NameTable, ObjectId, Program, ProposalRequest, Session,
    analyze, parse_json_limited, r1,
};
use narrata_storage_host::{CacheBackend, FlushReply, HostError, Loaded, StoreId, StoreState};
use wasm_bindgen::prelude::*;

fn js_error(error: Error) -> JsError {
    JsError::new(&error.to_string())
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

#[derive(serde::Serialize)]
struct PublicationObject<'a> {
    object_id: String,
    bytes: &'a [u8],
}

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
#[wasm_bindgen(js_name = publishGraph)]
pub fn publish_graph(pack: &[u8], outlines: Option<String>) -> Result<String, JsError> {
    let (program, names) = Program::from_pack(pack).map_err(js_error)?;
    program.verify_artifact().map_err(js_error)?;
    let outlines: Vec<narrata_nodes::ContentOutline> = outlines
        .as_deref()
        .map(|text| parse_json_limited(text, MAX_SOURCE_BYTES))
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

/// A checked program and one session on it.
#[wasm_bindgen]
pub struct NodeBook {
    pack: Option<Vec<u8>>,
    names: Option<NameTable>,
    analysis: Option<Analysis>,
    chunks: Option<HostChunks>,
    program: Arc<Program>,
    execution: narrata_nodes::ExecutionId,
    cache: CacheBackend,
    session: Option<Session<CacheBackend>>,
}

#[wasm_bindgen]
impl NodeBook {
    /// Opens a pack and checks every chunk. Call open or memory to start the session.
    #[wasm_bindgen(constructor)]
    pub fn new(pack: &[u8], execution: &str) -> Result<NodeBook, JsError> {
        let (program, names) = Program::from_pack(pack).map_err(js_error)?;
        program.verify_artifact().map_err(js_error)?;
        let analysis = analyze(&program, names.as_ref()).map_err(js_error)?;
        let execution = parse(execution, "execution", "execution:<32 hex digits>")?;

        Ok(Self {
            pack: Some(pack.to_vec()),
            names,
            analysis: Some(analysis),
            chunks: None,
            program: Arc::new(program),
            execution,
            cache: CacheBackend::new(),
            session: None,
        })
    }

    /// Opens only a checked manifest. On a missing program object an operation throws
    /// `not_loaded`; takeChunkRequest identifies it, loadChunk supplies it, then retry.
    /// Retry the pending call to completion before another runtime/tool call. Supplied
    /// bytes survive its retries and are released when it succeeds.
    /// The manifest and entry chunk suffice for an ordinary first screen. Full-pack
    /// construction remains available for hosts such as the reference reader.
    #[wasm_bindgen(js_name = fromManifest)]
    pub fn from_manifest(manifest: &[u8], execution: &str) -> Result<NodeBook, JsError> {
        let chunks = HostChunks::default();
        let program =
            Program::from_manifest(manifest, Box::new(chunks.clone())).map_err(js_error)?;
        Ok(Self {
            pack: None,
            names: None,
            analysis: None,
            chunks: Some(chunks),
            program: Arc::new(program),
            execution: parse(execution, "execution", "execution:<32 hex digits>")?,
            cache: CacheBackend::new(),
            session: None,
        })
    }

    #[wasm_bindgen(js_name = takeChunkRequest)]
    pub fn take_chunk_request(&self) -> Option<String> {
        self.chunks
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
        let id: ObjectId = parse(id, "object", "object:<64 hex digits>")?;
        if id != self.program.manifest().tombstones && !self.program.manifest().chunks.contains(&id)
        {
            return Err(js_error(Error::new(
                "reference",
                id.to_string(),
                "object is not in this manifest",
            )));
        }
        let chunks = self
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
        self.names = Some(self.program.decode_names(bytes).map_err(js_error)?);
        Ok(())
    }

    /// Requires all chunks and the tombstone set, including chunks already cached.
    #[wasm_bindgen(js_name = verifyArtifact)]
    pub fn verify_artifact(&self) -> Result<(), JsError> {
        self.finish(self.program.verify_artifact().map_err(js_error))
    }

    /// Tool lookup, loading tombstones first. Live results include their structural owner.
    pub fn lookup(&self, id: &str) -> Result<String, JsError> {
        let id: AuthoredId = parse(id, "id", "an authored ID")?;
        let found = match self.program.lookup(id).map_err(js_error)? {
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
        self.program.artifact_id().to_string()
    }

    /// Opens the kernel cursor, creating it only if absent. Retried after host loads.
    pub fn open(&mut self) -> Result<(), JsError> {
        self.session = None;
        self.session = Some(
            Session::open(self.program.clone(), self.execution, self.cache.share())
                .map_err(js_error)?,
        );
        self.finish(Ok(()))
    }

    fn finish<T>(&self, result: Result<T, JsError>) -> Result<T, JsError> {
        if result.is_ok()
            && let Some(chunks) = &self.chunks
        {
            let mut pending = chunks
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            // Only the checked decoded cache/pins survive a completed operation.
            // Hosts must finish a pending retry before starting another book operation.
            pending.objects.clear();
            pending.request = None;
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
        match &self.pack {
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

    /// The text-free book view as JSON.
    pub fn inspect(&self) -> Result<String, JsError> {
        self.finish(json(&BookView {
            view: self.opened()?.view(self.names.as_ref()).map_err(js_error)?,
            page: self.opened()?.page().map_err(js_error)?,
            // Runtime inspection must not eagerly scan the whole work. Publication
            // analysis is a separate operation for hosts that need the full graph.
            graphs: self
                .analysis
                .as_ref()
                .map_or_else(Vec::new, |a| a.graphs.clone()),
            diagnostics: self
                .analysis
                .as_ref()
                .map_or_else(Vec::new, |a| a.diagnostics.clone()),
        }))
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
        let names = self.names.as_ref().ok_or_else(|| {
            js_error(Error::new(
                "names",
                "pack",
                "R1 save migration needs the pack's name table",
            ))
        })?;
        let execution = parse(execution, "execution", "execution:<32 hex digits>")?;
        let ref_text = |reference: &_| content.inner.text(reference, &[]);
        let migrated = r1::migrate_save(self.program.clone(), names, save, execution, &ref_text)
            .map_err(js_error)?;
        self.opened_mut()?
            .import_session(&migrated)
            .map_err(js_error)?;
        self.finish(Ok(()))
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
