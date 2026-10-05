//! A checked program that loads chunks on demand (ADR 0013 §8). It holds the manifest and
//! loads tombstones only for identity checks. A chunk is fetched from a [`ChunkSource`],
//! verified against its object ID, decoded and checked against the manifest
//! on every load, including after cache eviction.

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    num::NonZeroUsize,
    sync::{Arc, Mutex, MutexGuard, OnceLock},
};

use narrata_kernel::codec::{EnvelopeLimits, inspect_envelope};

use crate::{
    ArtifactId, AuthoredId, ChoicePointId, Error, MAX_CHUNK_BYTES, MAX_MANIFEST_BYTES,
    MAX_PACK_BYTES, NodeId, ObjectId, Result, State,
    check::{check_chunk, check_manifest},
    plan::{CallTarget, Graph, GraphRef, Manifest, NameTable, Plan, TombstoneSet},
    state::resolve_call,
    wire::{self, Pack},
};

/// Where chunk envelopes come from: a pack in memory, a kernel store, the network.
pub trait ChunkSource: Send + Sync {
    /// Returns the envelope stored under `id`. The program verifies what it gets.
    fn load(&self, id: &ObjectId) -> Result<Vec<u8>>;
}

/// Envelopes held in memory and keyed by the object ID of their payload.
#[derive(Clone, Debug, Default)]
pub struct MemorySource {
    objects: BTreeMap<ObjectId, Vec<u8>>,
}

impl MemorySource {
    pub fn insert(&mut self, envelope: Vec<u8>) -> Result<ObjectId> {
        let id = {
            let opened = inspect_envelope(
                &envelope,
                &EnvelopeLimits {
                    max_envelope_bytes: MAX_PACK_BYTES as u64,
                    max_payload_bytes: MAX_PACK_BYTES as u64,
                },
            )
            .map_err(|error| Error::new("decode", "object", error.to_string()))?;
            if opened.schema_version != wire::SCHEMA {
                return Err(Error::new("decode", "object", "unsupported object schema"));
            }
            wire::id_of(opened.kind, opened.payload)
        };
        self.objects.insert(id, envelope);
        Ok(id)
    }
}

impl ChunkSource for MemorySource {
    fn load(&self, id: &ObjectId) -> Result<Vec<u8>> {
        self.objects
            .get(id)
            .cloned()
            .ok_or_else(|| Error::new("missing_chunk", id.to_string(), "chunk is not available"))
    }
}

pub struct Program {
    artifact_id: ArtifactId,
    manifest: Manifest,
    manifest_envelope: Vec<u8>,
    tombstones: OnceLock<(TombstoneSet, Vec<u8>)>,
    source: Box<dyn ChunkSource>,
    chunks: Arc<Mutex<ChunkCache>>,
}

type ChunkGraphs = BTreeMap<GraphRef, Arc<Graph>>;

struct CachedChunk {
    graphs: Arc<ChunkGraphs>,
    pins: usize,
}

struct ChunkCache {
    capacity: usize,
    entries: BTreeMap<u32, CachedChunk>,
    /// Oldest access first; only resident chunks appear here.
    lru: VecDeque<u32>,
}

impl ChunkCache {
    fn touch(&mut self, index: u32) {
        self.lru.retain(|other| *other != index);
        self.lru.push_back(index);
    }

    fn trim(&mut self) {
        while self.entries.len() > self.capacity {
            let candidate = self.lru.iter().position(|index| {
                self.entries.get(index).is_some_and(|entry| {
                    entry.pins == 0
                        && Arc::strong_count(&entry.graphs) == 1
                        && entry
                            .graphs
                            .values()
                            .all(|graph| Arc::strong_count(graph) == 1)
                })
            });
            let Some(position) = candidate else {
                // Live frames and graph borrowers may exceed the cache budget.
                break;
            };
            if let Some(index) = self.lru.remove(position) {
                self.entries.remove(&index);
            }
        }
    }
}

/// Keeps a state's frame chunks resident until dropped. Each session needs its own guard;
/// replace it after moving the cursor, and keep the old guard while executing a choice.
/// This does not retain historical states or pin graphs entered during that choice.
/// The guard owns its cache handle and can be stored by a session.
#[must_use = "keep this guard alive while its frames are active"]
pub struct ChunkPins {
    cache: Arc<Mutex<ChunkCache>>,
    indices: BTreeSet<u32>,
}

impl Drop for ChunkPins {
    fn drop(&mut self) {
        let mut cache = self
            .cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for index in &self.indices {
            if let Some(entry) = cache.entries.get_mut(index) {
                entry.pins -= 1;
            }
        }
        cache.trim();
    }
}

impl std::fmt::Debug for Program {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Program")
            .field("artifact_id", &self.artifact_id)
            .finish_non_exhaustive()
    }
}

/// Where a tool-side lookup found an authored ID (ADR 0013 §3).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Lookup {
    Live {
        graph: GraphRef,
        node: NodeId,
        choice_point: Option<ChoicePointId>,
    },
    Deleted,
    Unknown,
}

impl Program {
    /// Convenience path with an already available, checked tombstone set.
    pub fn open(
        manifest_envelope: &[u8],
        tombstones_envelope: &[u8],
        source: Box<dyn ChunkSource>,
    ) -> Result<Self> {
        let program = Self::from_manifest(manifest_envelope, source)?;
        let tombstones = program.decode_tombstones(tombstones_envelope)?;
        let _ = program.tombstones.set(tombstones);
        Ok(program)
    }

    /// Opens only the manifest. Chunks and tombstones come from `source` on demand.
    /// Chunk-local checks need no tombstones (ADR 0013 §8); lookup, proposals and full
    /// artifact verification must fetch and check the manifest's tombstone object.
    pub fn from_manifest(manifest_envelope: &[u8], source: Box<dyn ChunkSource>) -> Result<Self> {
        let (manifest, manifest_id) = wire::open(
            manifest_envelope,
            wire::KIND_MANIFEST,
            MAX_MANIFEST_BYTES,
            "manifest",
            wire::decode_manifest,
            wire::encode_manifest,
        )?;
        check_manifest(&manifest)?;
        let chunks = Arc::new(Mutex::new(ChunkCache {
            capacity: Self::DEFAULT_CHUNK_CAPACITY,
            entries: BTreeMap::new(),
            lru: VecDeque::new(),
        }));
        Ok(Self {
            artifact_id: ArtifactId::from_bytes(*manifest_id.as_bytes()),
            manifest,
            manifest_envelope: manifest_envelope.to_vec(),
            tombstones: OnceLock::new(),
            source,
            chunks,
        })
    }

    /// Opens a single-file pack. It must hold exactly the manifest's chunks; its optional
    /// name table must name this artifact.
    pub fn from_pack(bytes: &[u8]) -> Result<(Self, Option<NameTable>)> {
        let pack = Pack::decode(bytes)?;
        let mut source = MemorySource::default();
        let mut ids = BTreeSet::new();
        for chunk in &pack.chunks {
            ids.insert(source.insert(chunk.clone())?);
        }
        let program = Self::open(&pack.manifest, &pack.tombstones, Box::new(source))?;
        if ids != program.manifest.chunks.iter().copied().collect()
            || ids.len() != pack.chunks.len()
        {
            return Err(Error::new(
                "artifact",
                "pack",
                "pack chunks differ from the manifest",
            ));
        }
        let names = pack
            .names
            .as_deref()
            .map(|bytes| program.decode_names(bytes))
            .transpose()?;
        Ok((program, names))
    }

    pub fn decode_names(&self, envelope: &[u8]) -> Result<NameTable> {
        let (names, _) = wire::open(
            envelope,
            wire::KIND_NAMES,
            MAX_PACK_BYTES,
            "names",
            wire::decode_names,
            wire::encode_names,
        )?;
        if names.artifact != self.artifact_id {
            return Err(Error::new(
                "artifact",
                "names",
                "name table belongs to another artifact",
            ));
        }
        Ok(names)
    }

    pub fn artifact_id(&self) -> ArtifactId {
        self.artifact_id
    }

    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn decode_tombstones(&self, envelope: &[u8]) -> Result<(TombstoneSet, Vec<u8>)> {
        let (tombstones, id) = wire::open(
            envelope,
            wire::KIND_TOMBSTONES,
            MAX_PACK_BYTES,
            "tombstones",
            wire::decode_tombstones,
            wire::encode_tombstones,
        )?;
        if id != self.manifest.tombstones {
            return Err(Error::new(
                "artifact",
                "tombstones",
                "tombstone set differs from the manifest",
            ));
        }
        Ok((tombstones, envelope.to_vec()))
    }

    fn loaded_tombstones(&self) -> Result<&(TombstoneSet, Vec<u8>)> {
        if let Some(loaded) = self.tombstones.get() {
            return Ok(loaded);
        }
        // Failed loads are never cached. Concurrent loads check the same object ID;
        // whichever wins publishes the same immutable value.
        let loaded = self.decode_tombstones(&self.source.load(&self.manifest.tombstones)?)?;
        let _ = self.tombstones.set(loaded);
        self.tombstones
            .get()
            .ok_or_else(|| Error::new("state", "tombstones", "checked tombstones not retained"))
    }

    pub fn tombstones(&self) -> Result<&TombstoneSet> {
        Ok(&self.loaded_tombstones()?.0)
    }

    fn load_chunk(&self, index: u32) -> Result<BTreeMap<GraphRef, Arc<Graph>>> {
        let id = self
            .manifest
            .chunks
            .get(index as usize)
            .ok_or_else(|| Error::new("reference", "chunks", "chunk index out of range"))?;
        let envelope = self.source.load(id)?;
        let (chunk, actual) = wire::open(
            &envelope,
            wire::KIND_CHUNK,
            MAX_CHUNK_BYTES,
            "chunk",
            wire::decode_chunk,
            wire::encode_chunk,
        )?;
        if actual != *id {
            return Err(Error::new(
                "artifact",
                id.to_string(),
                "chunk content differs from its object ID",
            ));
        }
        // Global identity continuity belongs to compilation/verify_artifact. Still
        // check deletions here when a caller supplied or already loaded tombstones.
        let empty = TombstoneSet::default();
        let tombstones = self.tombstones.get().map_or(&empty, |loaded| &loaded.0);
        check_chunk(&self.manifest, index, &chunk, tombstones, None)?;
        Ok(chunk
            .graphs
            .into_iter()
            .map(|(key, graph)| (key, Arc::new(graph)))
            .collect())
    }

    fn cache(&self) -> MutexGuard<'_, ChunkCache> {
        self.chunks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn chunk_graphs(&self, index: u32) -> Result<Arc<ChunkGraphs>> {
        {
            let mut cache = self.cache();
            if let Some(entry) = cache.entries.get(&index) {
                let graphs = entry.graphs.clone();
                cache.touch(index);
                cache.trim();
                return Ok(graphs);
            }
        }
        // Never call an external source while holding the cache lock. A concurrent miss
        // may load twice; only checked data is published and both name the same object.
        let loaded = Arc::new(self.load_chunk(index)?);
        let mut cache = self.cache();
        let graphs = cache
            .entries
            .entry(index)
            .or_insert_with(|| CachedChunk {
                graphs: loaded,
                pins: 0,
            })
            .graphs
            .clone();
        cache.touch(index);
        cache.trim();
        Ok(graphs)
    }

    pub fn graph(&self, key: &GraphRef) -> Result<Arc<Graph>> {
        let entry = self
            .manifest
            .graphs
            .get(key)
            .ok_or_else(|| Error::new("reference", key.label(), "unknown graph"))?;
        self.chunk_graphs(entry.chunk)?
            .get(key)
            .cloned()
            .ok_or_else(|| Error::new("state", key.label(), "graph missing from its chunk"))
    }

    /// Default resident chunk budget. Live pins/borrowers can exceed this budget.
    pub const DEFAULT_CHUNK_CAPACITY: usize = 16;

    /// Changes the resident chunk budget, immediately evicting eligible LRU entries.
    /// Encoded bytes retained by a source (for example `MemorySource`) are not in this budget.
    pub fn set_chunk_capacity(&self, capacity: NonZeroUsize) {
        let mut cache = self.cache();
        cache.capacity = capacity.get();
        cache.trim();
    }

    pub fn chunk_capacity(&self) -> usize {
        self.cache().capacity
    }

    /// Pins all distinct chunks referenced by the state's frames. Every miss is checked
    /// again, including after eviction. On failure all pins acquired by this call release.
    /// This pins frame references; it does not validate the rest of the state's contents.
    pub fn pin_state(&self, state: &State) -> Result<ChunkPins> {
        let indices = state
            .frames
            .iter()
            .map(|frame| {
                self.manifest
                    .graphs
                    .get(&frame.graph)
                    .map(|entry| entry.chunk)
                    .ok_or_else(|| Error::new("reference", frame.graph.label(), "unknown graph"))
            })
            .collect::<Result<BTreeSet<_>>>()?;
        let mut pins = ChunkPins {
            cache: self.chunks.clone(),
            indices: BTreeSet::new(),
        };
        // Protect the whole resident subset before any miss can trigger trimming. In
        // particular, transferring guards must not evict a newly entered frame while
        // walking the older frames first.
        {
            let mut cache = self.cache();
            for index in &indices {
                if let Some(entry) = cache.entries.get_mut(index) {
                    entry.pins = entry
                        .pins
                        .checked_add(1)
                        .ok_or_else(|| Error::new("limit", "chunks", "too many chunk pins"))?;
                    pins.indices.insert(*index);
                }
            }
        }
        for index in indices {
            if pins.indices.contains(&index) {
                continue;
            }
            let _graphs = self.chunk_graphs(index)?;
            let mut cache = self.cache();
            let entry = cache
                .entries
                .get_mut(&index)
                .ok_or_else(|| Error::new("state", "chunks", "pinned chunk is missing"))?;
            entry.pins = entry
                .pins
                .checked_add(1)
                .ok_or_else(|| Error::new("limit", "chunks", "too many chunk pins"))?;
            pins.indices.insert(index);
        }
        Ok(pins)
    }

    /// Resident decoded chunks, including those kept by live pins or graph borrowers.
    /// Released borrowers are reclaimed on the next access, budget change or pin drop.
    pub fn loaded_chunks(&self) -> usize {
        self.cache().entries.len()
    }

    pub fn resolve_call(&self, graph: &GraphRef, target: &CallTarget) -> Result<GraphRef> {
        resolve_call(&self.manifest, graph, target)
    }

    /// Loads every chunk and confirms work-wide ID uniqueness, which per-chunk decoding
    /// cannot see.
    pub fn verify_artifact(&self) -> Result<()> {
        let tombstones = self.tombstones()?;
        let live = |id: AuthoredId, key: &GraphRef| {
            if tombstones.contains(&id) {
                Err(Error::new(
                    "tombstone",
                    key.label(),
                    format!("{id} is deleted"),
                ))
            } else {
                Ok(())
            }
        };
        let mut nodes = BTreeSet::new();
        let mut points = BTreeSet::new();
        let mut options = BTreeSet::new();
        for index in 0..self.manifest.chunks.len() {
            for (key, graph) in self.chunk_graphs(index as u32)?.iter() {
                for (id, plan) in &graph.nodes {
                    live(AuthoredId::Node(*id), key)?;
                    if !nodes.insert(*id) {
                        return Err(Error::new(
                            "duplicate",
                            key.label(),
                            format!("duplicate {id}"),
                        ));
                    }
                    if let Plan::Passage(passage) = plan {
                        for point in &passage.choice_points {
                            live(AuthoredId::ChoicePoint(point.id), key)?;
                            if !points.insert(point.id) {
                                return Err(Error::new(
                                    "duplicate",
                                    key.label(),
                                    format!("duplicate {}", point.id),
                                ));
                            }
                            for option in &point.options {
                                live(AuthoredId::Option(option.id), key)?;
                                if !options.insert(option.id) {
                                    return Err(Error::new(
                                        "duplicate",
                                        key.label(),
                                        format!("duplicate {}", option.id),
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Scans the artifact for an authored ID.
    pub fn lookup(&self, id: AuthoredId) -> Result<Lookup> {
        if self.tombstones()?.contains(&id) {
            return Ok(Lookup::Deleted);
        }
        for index in 0..self.manifest.chunks.len() {
            for (key, graph) in self.chunk_graphs(index as u32)?.iter() {
                for (node, plan) in &graph.nodes {
                    let live = |choice_point| Lookup::Live {
                        graph: key.clone(),
                        node: *node,
                        choice_point,
                    };
                    if id == AuthoredId::Node(*node) {
                        return Ok(live(None));
                    }
                    let Plan::Passage(passage) = plan else {
                        continue;
                    };
                    for point in &passage.choice_points {
                        if id == AuthoredId::ChoicePoint(point.id) {
                            return Ok(live(None));
                        }
                        if point
                            .options
                            .iter()
                            .any(|option| id == AuthoredId::Option(option.id))
                        {
                            return Ok(live(Some(point.id)));
                        }
                    }
                }
            }
        }
        Ok(Lookup::Unknown)
    }

    /// Every live authored ID with its owner: a node's graph, a choice point's node, an
    /// option's choice point. `compose` compares these with the previous artifact.
    pub fn owners(&self) -> Result<BTreeMap<AuthoredId, Owner>> {
        let mut owners = BTreeMap::new();
        for index in 0..self.manifest.chunks.len() {
            for (key, graph) in self.chunk_graphs(index as u32)?.iter() {
                for (node, plan) in &graph.nodes {
                    owners.insert(AuthoredId::Node(*node), Owner::Graph(key.clone()));
                    let Plan::Passage(passage) = plan else {
                        continue;
                    };
                    for point in &passage.choice_points {
                        owners.insert(AuthoredId::ChoicePoint(point.id), Owner::Node(*node));
                        for option in &point.options {
                            owners.insert(
                                AuthoredId::Option(option.id),
                                Owner::ChoicePoint(point.id),
                            );
                        }
                    }
                }
            }
        }
        Ok(owners)
    }

    /// Rebuilds the single-file pack, loading every chunk envelope from the source.
    pub fn pack(&self, names: Option<&NameTable>) -> Result<Vec<u8>> {
        let chunks = self
            .manifest
            .chunks
            .iter()
            .map(|id| self.source.load(id))
            .collect::<Result<Vec<_>>>()?;
        Ok(Pack {
            manifest: self.manifest_envelope.clone(),
            chunks,
            tombstones: self.loaded_tombstones()?.1.clone(),
            names: names.map(|names| wire::seal(wire::KIND_NAMES, &wire::encode_names(names)).1),
        }
        .encode())
    }
}

/// The structural owner of an authored ID.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Owner {
    Graph(GraphRef),
    Node(NodeId),
    ChoicePoint(ChoicePointId),
}
