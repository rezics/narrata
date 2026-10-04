//! A checked program that loads chunks on demand (ADR 0013 §8). It holds the manifest and
//! the tombstone set; a chunk is fetched by object ID from a [`ChunkSource`], verified
//! against that ID, decoded with the checked decoder and checked against the manifest
//! before first use.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, OnceLock},
};

use narrata_kernel::codec::{EnvelopeLimits, inspect_envelope};

use crate::{
    ArtifactId, AuthoredId, ChoicePointId, Error, MAX_CHUNK_BYTES, MAX_MANIFEST_BYTES,
    MAX_PACK_BYTES, NodeId, ObjectId, Result,
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
    tombstones: TombstoneSet,
    tombstones_envelope: Vec<u8>,
    source: Box<dyn ChunkSource>,
    chunks: Vec<OnceLock<BTreeMap<GraphRef, Arc<Graph>>>>,
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
    /// Opens the manifest and tombstone set; chunks load from `source` when first used.
    pub fn open(
        manifest_envelope: &[u8],
        tombstones_envelope: &[u8],
        source: Box<dyn ChunkSource>,
    ) -> Result<Self> {
        let (manifest, manifest_id) = wire::open(
            manifest_envelope,
            wire::KIND_MANIFEST,
            MAX_MANIFEST_BYTES,
            "manifest",
            wire::decode_manifest,
            wire::encode_manifest,
        )?;
        check_manifest(&manifest)?;
        let (tombstones, tombstones_id) = wire::open(
            tombstones_envelope,
            wire::KIND_TOMBSTONES,
            MAX_PACK_BYTES,
            "tombstones",
            wire::decode_tombstones,
            wire::encode_tombstones,
        )?;
        if tombstones_id != manifest.tombstones {
            return Err(Error::new(
                "artifact",
                "tombstones",
                "tombstone set differs from the manifest",
            ));
        }
        let chunks = manifest.chunks.iter().map(|_| OnceLock::new()).collect();
        Ok(Self {
            artifact_id: ArtifactId::from_bytes(*manifest_id.as_bytes()),
            manifest,
            manifest_envelope: manifest_envelope.to_vec(),
            tombstones,
            tombstones_envelope: tombstones_envelope.to_vec(),
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

    pub fn tombstones(&self) -> &TombstoneSet {
        &self.tombstones
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
        check_chunk(&self.manifest, index, &chunk, &self.tombstones, None)?;
        Ok(chunk
            .graphs
            .into_iter()
            .map(|(key, graph)| (key, Arc::new(graph)))
            .collect())
    }

    fn chunk_graphs(&self, index: u32) -> Result<&BTreeMap<GraphRef, Arc<Graph>>> {
        let cell = self
            .chunks
            .get(index as usize)
            .ok_or_else(|| Error::new("reference", "chunks", "chunk index out of range"))?;
        if let Some(graphs) = cell.get() {
            return Ok(graphs);
        }
        let graphs = self.load_chunk(index)?;
        // A concurrent loader may have won; both verified the same bytes.
        let _ = cell.set(graphs);
        cell.get()
            .ok_or_else(|| Error::new("state", "chunks", "chunk cache is empty"))
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

    /// How many chunks are loaded; the rest stay unread.
    pub fn loaded_chunks(&self) -> usize {
        self.chunks
            .iter()
            .filter(|cell| cell.get().is_some())
            .count()
    }

    pub fn resolve_call(&self, graph: &GraphRef, target: &CallTarget) -> Result<GraphRef> {
        resolve_call(&self.manifest, graph, target)
    }

    /// Loads every chunk and confirms work-wide ID uniqueness, which per-chunk decoding
    /// cannot see.
    pub fn verify_artifact(&self) -> Result<()> {
        let mut nodes = BTreeSet::new();
        let mut points = BTreeSet::new();
        let mut options = BTreeSet::new();
        for index in 0..self.manifest.chunks.len() {
            for (key, graph) in self.chunk_graphs(index as u32)? {
                for (id, plan) in &graph.nodes {
                    if !nodes.insert(*id) {
                        return Err(Error::new(
                            "duplicate",
                            key.label(),
                            format!("duplicate {id}"),
                        ));
                    }
                    if let Plan::Passage(passage) = plan {
                        for point in &passage.choice_points {
                            if !points.insert(point.id) {
                                return Err(Error::new(
                                    "duplicate",
                                    key.label(),
                                    format!("duplicate {}", point.id),
                                ));
                            }
                            for option in &point.options {
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
        if self.tombstones.contains(&id) {
            return Ok(Lookup::Deleted);
        }
        for index in 0..self.manifest.chunks.len() {
            for (key, graph) in self.chunk_graphs(index as u32)? {
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
            for (key, graph) in self.chunk_graphs(index as u32)? {
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
            tombstones: self.tombstones_envelope.clone(),
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
