//! Immutable object collections. Hosts upload and verify a candidate before atomically
//! switching their current publication reference; these functions do no I/O or switching.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use narrata_kernel::codec::{EnvelopeLimits, inspect_envelope, object_id};
use narrata_nodes::plan::Plan;
use narrata_nodes::{
    ArtifactId, CompositionLock, ContentOutline, MemorySource, NameTable, NodeRegistry, Pack,
    Program, Result, canonical_json,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    ComposeMode,
    compose::compose_objects_with_registry,
    evolution::CheckedEvolution,
    graph::{GraphFiles, PublishedGraph, publication_analysis, publish_collection},
    objects::{Digest, Version1, decode_json, error},
    records::{CheckedRecord, DraftIndexes, DraftRecord, derive_draft_indexes, split_source},
    relations::{CheckedRelations, MAX_PART_BYTES, project_with_summary, read_relations},
    validate::{Collector, Report, ValidationBudget, validate_records_with_budget, warning},
};

pub const MAX_DESCRIPTION_BYTES: usize = 4 * 1024 * 1024;

/// Lexical role order is the descriptor order, including ADR 0023 evolution metadata.
#[derive(
    Clone, Copy, Debug, Deserialize, Eq, JsonSchema, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum ObjectRole {
    Chunk,
    Evolution,
    GraphIndex,
    GraphTile,
    Labels,
    Manifest,
    Names,
    RelationIndex,
    RelationPart,
    Summary,
    Tombstones,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Encoding {
    Cbor,
    Json,
}

impl ObjectRole {
    fn encoding(self) -> Encoding {
        if matches!(
            self,
            Self::RelationIndex | Self::RelationPart | Self::Summary
        ) {
            Encoding::Json
        } else {
            Encoding::Cbor
        }
    }
    fn budget(self) -> usize {
        match self {
            Self::Evolution => narrata_nodes::history::evolution::MAX_DESCRIPTOR_BYTES,
            Self::Manifest => narrata_nodes::MAX_MANIFEST_BYTES,
            Self::Chunk => narrata_nodes::MAX_CHUNK_BYTES,
            Self::Names | Self::Tombstones => narrata_nodes::MAX_PACK_BYTES,
            Self::RelationIndex | Self::RelationPart => MAX_PART_BYTES,
            Self::Summary => narrata_graph::MAX_SUMMARY_BYTES,
            _ => narrata_graph::wire::MAX_OBJECT_BYTES as usize,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectReference {
    pub role: ObjectRole,
    pub object_id: Digest,
    pub bytes: u64,
    pub encoding: Encoding,
}
impl ObjectReference {
    pub fn filename(&self) -> String {
        format!(
            "{}.{}",
            self.object_id,
            match self.encoding {
                Encoding::Cbor => "cbor",
                Encoding::Json => "json",
            }
        )
    }
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PublicationDescription {
    pub format_version: Version1,
    pub artifact_id: ArtifactId,
    pub graph: GraphFiles,
    pub summary: Digest,
    pub relation_index: Digest,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub names: Option<Digest>,
    /// ADR 0023's full MigrationId digest, addressed by the evolution role.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evolution: Option<Digest>,
    pub objects: Vec<ObjectReference>,
}

/// Private bytes and proof-bearing accessors prevent a host from silently replacing a
/// checked candidate's objects. Binary objects are retrieved individually, never serialized
/// as an all-work JSON array. The description itself is addressed by `publication_id`.
#[derive(Debug)]
pub struct CheckedPublication {
    description: PublicationDescription,
    description_bytes: Vec<u8>,
    publication_id: Digest,
    objects: BTreeMap<Digest, Vec<u8>>,
    program: Arc<Program>,
    names: Option<NameTable>,
    relations: CheckedRelations,
}
impl CheckedPublication {
    /// Attaches a complete checked object after publishing, without changing the artifact.
    /// Replacing metadata creates a new publication identity; the host retains old ones.
    pub fn with_evolution(
        mut self,
        previous: &CheckedPublication,
        evolution: &CheckedEvolution,
    ) -> Result<Self> {
        evolution
            .descriptor()
            .validate(previous.program(), self.program())?;
        if let Some(old) = self.description.evolution {
            self.objects.remove(&old);
        }
        self.description
            .objects
            .retain(|r| r.role != ObjectRole::Evolution);
        let bytes = evolution.bytes();
        let id = Digest::from_bytes(*evolution.id().as_bytes());
        self.description.objects.push(ObjectReference {
            role: ObjectRole::Evolution,
            object_id: id,
            bytes: bytes.len() as u64,
            encoding: Encoding::Cbor,
        });
        self.description
            .objects
            .sort_by_key(|r| (r.role, r.object_id));
        self.description.evolution = Some(id);
        self.objects.insert(id, bytes);
        self.description_bytes = canonical_json(&self.description)?;
        if self.description_bytes.len() > MAX_DESCRIPTION_BYTES {
            return Err(error("publication description byte budget"));
        }
        self.publication_id = Digest::json("narrata.nodes.publication", &self.description_bytes);
        Ok(self)
    }

    /// Rechecks the source using the exact historic publication, never reader-supplied metadata.
    pub fn evolution(&self, previous: &CheckedPublication) -> Result<Option<CheckedEvolution>> {
        self.description
            .evolution
            .map(|id| {
                CheckedEvolution::read(
                    self.object(id)
                        .ok_or_else(|| error("missing evolution object"))?,
                    narrata_nodes::history::evolution::MigrationId::from_bytes(*id.as_bytes()),
                    previous,
                    self,
                )
            })
            .transpose()
    }
    pub fn description(&self) -> &PublicationDescription {
        &self.description
    }
    pub fn description_bytes(&self) -> &[u8] {
        &self.description_bytes
    }
    pub fn publication_id(&self) -> Digest {
        self.publication_id
    }
    pub fn program(&self) -> &Program {
        &self.program
    }
    pub fn shared_program(&self) -> Arc<Program> {
        self.program.clone()
    }
    pub fn names(&self) -> Option<&NameTable> {
        self.names.as_ref()
    }
    pub fn relations(&self) -> &CheckedRelations {
        &self.relations
    }
    pub fn object(&self, id: Digest) -> Option<&[u8]> {
        self.objects.get(&id).map(Vec::as_slice)
    }
    pub fn objects(&self) -> impl Iterator<Item = (&ObjectReference, &[u8])> {
        self.description
            .objects
            .iter()
            .map(|r| (r, self.objects[&r.object_id].as_slice()))
    }
    pub fn pack(&self) -> Result<Vec<u8>> {
        let get = |id: Digest| {
            self.objects
                .get(&id)
                .cloned()
                .ok_or_else(|| error("missing program object"))
        };
        let pack = Pack {
            manifest: get(Digest::from_bytes(*self.program.artifact_id().as_bytes()))?,
            chunks: self
                .program
                .manifest()
                .chunks
                .iter()
                .map(|id| get(Digest::from_bytes(*id.as_bytes())))
                .collect::<Result<_>>()?,
            tombstones: get(Digest::from_bytes(
                *self.program.manifest().tombstones.as_bytes(),
            ))?,
            names: self.description.names.map(get).transpose()?,
        }
        .encode();
        Program::from_pack(&pack)?.0.verify_artifact()?;
        Ok(pack)
    }
}

/// Compiled candidates can be retained and inspected even when their outlines prevent
/// switching. Topology warnings do not prevent switching; missing/defective outlines do.
#[derive(Clone, Copy, Debug, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicationStatus {
    Compiled,
    Switchable,
}

#[derive(Debug)]
pub struct PublishedRecords {
    pub status: PublicationStatus,
    pub publication: CheckedPublication,
    pub records: Vec<CheckedRecord>,
    pub tombstones: Vec<DraftRecord>,
    pub indexes: DraftIndexes,
    pub lock: CompositionLock,
    pub compared: bool,
    pub previous_artifact_id: Option<ArtifactId>,
    pub candidate_artifact_id: ArtifactId,
}
pub type PublicationReport = Report<PublishedRecords>;

pub fn publish_records(
    records: &[&[u8]],
    outlines: &[ContentOutline],
    previous: Option<&Program>,
    mode: ComposeMode<'_>,
) -> PublicationReport {
    publish_records_with_budget(
        records,
        outlines,
        previous,
        mode,
        ValidationBudget::default(),
    )
}

pub fn publish_records_with_budget(
    records: &[&[u8]],
    outlines: &[ContentOutline],
    previous: Option<&Program>,
    mode: ComposeMode<'_>,
    budget: ValidationBudget,
) -> PublicationReport {
    let registry = NodeRegistry::gamebook();
    let report = validate_records_with_budget(records, &registry, budget);
    let mut out = Collector::new(budget.max_diagnostics);
    out.complete = report.complete;
    out.diagnostics = report.diagnostics;
    let Some(checked) = report.value else {
        return out.finish(None);
    };
    let result = (|| -> Result<PublishedRecords> {
        let composed = compose_objects_with_registry(checked.source(), previous, mode, &registry)?;
        let compilation = composed.compilation;
        let graph = publish_collection(&compilation.program, Some(&compilation.names))?;
        let analysis = publication_analysis(
            compilation.analysis,
            &compilation.program,
            Some(&compilation.names),
            &graph,
            outlines,
        )?;
        let mut diagnostics = analysis.diagnostics.iter().map(warning).collect::<Vec<_>>();
        // The legacy analysis checks passage placement. Companion publication also checks
        // body slices presented by local replies and product endings, without changing it.
        diagnose_other_segments(
            &compilation.program,
            Some(&compilation.names),
            outlines,
            &mut diagnostics,
        )?;
        crate::records::locate_diagnostics(checked.records(), &mut diagnostics);
        let status = if diagnostics.iter().any(|d| d.code.starts_with("outline_")) {
            PublicationStatus::Compiled
        } else {
            PublicationStatus::Switchable
        };
        for diagnostic in diagnostics {
            out.push(diagnostic);
        }
        let mut updated = checked.source().clone();
        for (package, id) in &composed.appended {
            updated
                .packages
                .get_mut(package)
                .ok_or_else(|| error("missing tombstone owner"))?
                .tombstones
                .push(*id);
        }
        let records = split_source(&updated).into_result()?;
        let checked_records = crate::records::CheckedRecords::new(records.clone()).into_result()?;
        let indexes = derive_draft_indexes(&checked_records);
        let candidate_artifact_id = compilation.program.artifact_id();
        let publication = build_collection(
            compilation.objects,
            compilation.program,
            Some(compilation.names),
            graph,
        )?;
        Ok(PublishedRecords {
            status,
            publication,
            records,
            tombstones: composed.tombstones,
            indexes,
            lock: compilation.lock,
            compared: composed.compared,
            previous_artifact_id: previous.map(Program::artifact_id),
            candidate_artifact_id,
        })
    })();
    match result {
        Ok(value) => out.finish(Some(value)),
        Err(error) => {
            if error.code == "limit" {
                out.limit(&error.message);
            } else {
                out.error(error, None);
                crate::compose::locate(checked.records(), &mut out.diagnostics);
            }
            out.finish(None)
        }
    }
}

fn open_objects(objects: &Pack) -> Result<(Program, Option<NameTable>)> {
    let mut source = MemorySource::default();
    let mut chunks = BTreeSet::new();
    for chunk in &objects.chunks {
        if !chunks.insert(source.insert(chunk.clone())?) {
            return Err(error("duplicate program chunk"));
        }
    }
    source.insert(objects.tombstones.clone())?;
    let program = Program::from_manifest(&objects.manifest, Box::new(source))?;
    if chunks != program.manifest().chunks.iter().copied().collect() {
        return Err(error("program chunk closure"));
    }
    let names = objects
        .names
        .as_deref()
        .map(|b| program.decode_names(b))
        .transpose()?;
    program.verify_artifact()?;
    Ok((program, names))
}

pub fn publication_from_pack(bytes: &[u8]) -> Result<CheckedPublication> {
    publication_from_objects(Pack::decode(bytes)?)
}
pub fn publication_from_objects(objects: Pack) -> Result<CheckedPublication> {
    let (program, names) = open_objects(&objects)?;
    let graph = publish_collection(&program, names.as_ref())?;
    build_collection(objects, program, names, graph)
}

fn cbor_id(role: ObjectRole, bytes: &[u8]) -> Result<Digest> {
    let envelope = inspect_envelope(
        bytes,
        &EnvelopeLimits {
            max_envelope_bytes: role.budget() as u64,
            max_payload_bytes: role.budget() as u64,
        },
    )
    .map_err(error)?;
    let kind_matches = match role {
        ObjectRole::Manifest => envelope.kind == narrata_nodes::KIND_MANIFEST,
        ObjectRole::Chunk => envelope.kind == narrata_nodes::KIND_CHUNK,
        ObjectRole::Tombstones => envelope.kind == narrata_nodes::KIND_TOMBSTONES,
        ObjectRole::Names => envelope.kind == narrata_nodes::KIND_NAMES,
        ObjectRole::GraphIndex => envelope.kind == narrata_graph::wire::INDEX_KIND,
        ObjectRole::GraphTile => matches!(
            envelope.kind,
            narrata_graph::wire::WORK_KIND
                | narrata_graph::wire::CLUSTER_KIND
                | narrata_graph::wire::NODE_KIND
        ),
        ObjectRole::Labels => envelope.kind == narrata_graph::wire::LABEL_KIND,
        _ => false,
    };
    if !kind_matches || envelope.schema_version != 1 {
        return Err(error("object role or schema"));
    }
    Ok(Digest::from_bytes(object_id(
        envelope.kind,
        1,
        envelope.payload,
    )))
}
fn identity(role: ObjectRole, bytes: &[u8]) -> Result<Digest> {
    if bytes.len() > role.budget() {
        return Err(error("publication object byte budget"));
    }
    match role {
        ObjectRole::Summary => Ok(Digest::json("narrata.nodes.summary", bytes)),
        ObjectRole::RelationIndex => Ok(Digest::json("narrata.nodes.relation-index", bytes)),
        ObjectRole::RelationPart => Ok(Digest::json("narrata.nodes.relation-part", bytes)),
        ObjectRole::Evolution => Ok(Digest::from_bytes(
            *narrata_nodes::history::evolution::EvolutionDescriptorV1::decode(bytes)?
                .id()
                .as_bytes(),
        )),
        _ => cbor_id(role, bytes),
    }
}

fn build_collection(
    program_objects: Pack,
    program: Program,
    names: Option<NameTable>,
    graph: PublishedGraph,
) -> Result<CheckedPublication> {
    let relations = project_with_summary(&program, &graph.summary)?;
    let mut objects = BTreeMap::new();
    let mut references = BTreeSet::new();
    let mut add = |role: ObjectRole, bytes: Vec<u8>| -> Result<Digest> {
        let id = identity(role, &bytes)?;
        references.insert((role, id));
        if let Some(previous) = objects.insert(id, bytes.clone())
            && previous != bytes
        {
            return Err(error("object identity collision"));
        }
        Ok(id)
    };
    add(ObjectRole::Manifest, program_objects.manifest)?;
    for chunk in program_objects.chunks {
        add(ObjectRole::Chunk, chunk)?;
    }
    add(ObjectRole::Tombstones, program_objects.tombstones)?;
    let names_id = program_objects
        .names
        .map(|b| add(ObjectRole::Names, b))
        .transpose()?;
    add(ObjectRole::GraphIndex, graph.geometry.index.bytes)?;
    for object in graph.geometry.tiles {
        add(ObjectRole::GraphTile, object.bytes)?;
    }
    for object in graph.labels {
        add(ObjectRole::Labels, object.bytes)?;
    }
    let summary = add(ObjectRole::Summary, canonical_json(&graph.summary)?)?;
    let relation_index = add(
        ObjectRole::RelationIndex,
        relations.index_object().bytes.clone(),
    )?;
    for object in relations.parts() {
        add(ObjectRole::RelationPart, object.bytes.clone())?;
    }
    let description = PublicationDescription {
        format_version: Version1,
        artifact_id: program.artifact_id(),
        graph: graph.files,
        summary,
        relation_index,
        names: names_id,
        evolution: None,
        objects: references
            .into_iter()
            .map(|(role, object_id)| ObjectReference {
                role,
                object_id,
                bytes: objects[&object_id].len() as u64,
                encoding: role.encoding(),
            })
            .collect(),
    };
    let description_bytes = canonical_json(&description)?;
    if description_bytes.len() > MAX_DESCRIPTION_BYTES {
        return Err(error("publication description byte budget"));
    }
    let publication_id = Digest::json("narrata.nodes.publication", &description_bytes);
    Ok(CheckedPublication {
        description,
        description_bytes,
        publication_id,
        objects,
        program: Arc::new(program),
        names,
        relations,
    })
}

/// Checks a description's identity, encoding and every object reference's role, order,
/// encoding and role byte budget, before any object is requested. Object bytes, reference
/// closure and correspondence are proven by `read_publication`.
pub fn read_description(
    description_bytes: &[u8],
    expected: Digest,
) -> Result<PublicationDescription> {
    if Digest::json("narrata.nodes.publication", description_bytes) != expected {
        return Err(error("publication identity"));
    }
    let description: PublicationDescription =
        decode_json(description_bytes, MAX_DESCRIPTION_BYTES)?;
    GraphFiles::decode_json(&canonical_json(&description.graph)?)?;
    if description.graph.artifact_id != description.artifact_id {
        return Err(error("artifact ownership"));
    }
    let mut previous = None;
    let mut ids = BTreeSet::new();
    for reference in &description.objects {
        let key = (reference.role, reference.object_id);
        if previous.is_some_and(|p| p >= key)
            || reference.encoding != reference.role.encoding()
            || reference.bytes > reference.role.budget() as u64
            || reference.bytes == 0
            || !ids.insert(reference.object_id)
        {
            return Err(error("object descriptor order, duplication or encoding"));
        }
        previous = Some(key);
    }
    Ok(description)
}

pub fn read_publication(
    description_bytes: &[u8],
    expected: Digest,
    load: impl FnMut(Digest) -> Result<Vec<u8>>,
) -> Result<CheckedPublication> {
    read_publication_with_source(description_bytes, expected, load, None)
}

/// Evolution metadata additionally requires its verified source publication. Legacy
/// publications need no source and preserve their original bytes and reading path.
pub fn read_publication_with_source(
    description_bytes: &[u8],
    expected: Digest,
    mut load: impl FnMut(Digest) -> Result<Vec<u8>>,
    previous: Option<&CheckedPublication>,
) -> Result<CheckedPublication> {
    let description = read_description(description_bytes, expected)?;
    let mut objects = BTreeMap::new();
    let mut roles = BTreeMap::new();
    for reference in &description.objects {
        let bytes = load(reference.object_id)?;
        if bytes.len() as u64 != reference.bytes
            || identity(reference.role, &bytes)? != reference.object_id
        {
            return Err(error("object identity or length"));
        }
        roles
            .entry(reference.role)
            .or_insert_with(BTreeSet::new)
            .insert(reference.object_id);
        objects.insert(reference.object_id, bytes);
    }
    let get = |id: Digest| {
        objects
            .get(&id)
            .cloned()
            .ok_or_else(|| error("missing publication object"))
    };
    let exact = |role: ObjectRole, ids: BTreeSet<Digest>| -> Result<()> {
        if roles.get(&role).cloned().unwrap_or_default() != ids {
            return Err(error(format!("{role:?} reference closure")));
        }
        Ok(())
    };
    let manifest_id = Digest::from_bytes(*description.artifact_id.as_bytes());
    exact(ObjectRole::Manifest, BTreeSet::from([manifest_id]))?;
    let mut source = MemorySource::default();
    for reference in &description.objects {
        if matches!(reference.role, ObjectRole::Chunk | ObjectRole::Tombstones) {
            source.insert(get(reference.object_id)?)?;
        }
    }
    let program = Program::from_manifest(&get(manifest_id)?, Box::new(source))?;
    program.verify_artifact()?;
    exact(
        ObjectRole::Chunk,
        program
            .manifest()
            .chunks
            .iter()
            .map(|id| Digest::from_bytes(*id.as_bytes()))
            .collect(),
    )?;
    exact(
        ObjectRole::Tombstones,
        BTreeSet::from([Digest::from_bytes(
            *program.manifest().tombstones.as_bytes(),
        )]),
    )?;
    exact(ObjectRole::Names, description.names.into_iter().collect())?;
    let names = description
        .names
        .map(|id| program.decode_names(&get(id)?))
        .transpose()?;
    let graph_index_id =
        Digest::try_from(description.graph.index_object_id.clone()).map_err(error)?;
    exact(ObjectRole::GraphIndex, BTreeSet::from([graph_index_id]))?;
    let graph_index =
        narrata_graph::wire::decode_index(&get(graph_index_id)?, graph_index_id.as_bytes())
            .map_err(error)?;
    exact(
        ObjectRole::GraphTile,
        graph_index
            .tiles()
            .iter()
            .map(|t| Digest::from_bytes(t.object_id))
            .collect(),
    )?;
    for (ordinal, tile) in graph_index.tiles().iter().enumerate() {
        narrata_graph::wire::decode_tile(
            &get(Digest::from_bytes(tile.object_id))?,
            &graph_index,
            ordinal as u32,
        )
        .map_err(error)?;
    }
    let mut label_ids = BTreeSet::new();
    for label in &description.graph.labels {
        let id = Digest::try_from(label.object_id.clone()).map_err(error)?;
        label_ids.insert(id);
        narrata_graph::decode_labels(&get(id)?, id.as_bytes(), label.cluster).map_err(error)?;
    }
    exact(ObjectRole::Labels, label_ids)?;
    exact(ObjectRole::Summary, BTreeSet::from([description.summary]))?;
    let summary: narrata_graph::SemanticSummary =
        decode_json(&get(description.summary)?, narrata_graph::MAX_SUMMARY_BYTES)?;
    narrata_graph::SemanticSummary::decode_json(&get(description.summary)?).map_err(error)?;
    if summary.artifact_id != Some(hex::encode(program.artifact_id().as_bytes())) {
        return Err(error("summary artifact ownership"));
    }
    exact(
        ObjectRole::RelationIndex,
        BTreeSet::from([description.relation_index]),
    )?;
    let relations = read_relations(
        &get(description.relation_index)?,
        description.relation_index,
        get,
    )?;
    exact(
        ObjectRole::RelationPart,
        relations.parts().iter().map(|p| p.id).collect(),
    )?;
    exact(
        ObjectRole::Evolution,
        description.evolution.into_iter().collect(),
    )?;
    if let Some(id) = description.evolution {
        let descriptor =
            narrata_nodes::history::evolution::EvolutionDescriptorV1::decode(&get(id)?)?;
        let source = previous.ok_or_else(|| {
            narrata_nodes::Error::new(
                "needs_artifact",
                descriptor.source_artifact().to_string(),
                "evolution requires its checked source publication",
            )
        })?;
        descriptor.validate(source.program(), &program)?;
    }
    if relations.index().artifact_id != description.artifact_id {
        return Err(error("relation artifact ownership"));
    }
    // Geometry, labels and summaries lack an internal artifact field (ADR 0016). Rebuild
    // their existing v1 identities to establish correspondence, not merely valid CBOR.
    let rebuilt = publish_collection(&program, names.as_ref())?;
    if rebuilt.files != description.graph || rebuilt.summary != summary {
        return Err(error("graph or summary does not correspond to the program"));
    }
    let rebuilt_relations = project_with_summary(&program, &rebuilt.summary)?;
    if rebuilt_relations.index_object() != relations.index_object()
        || rebuilt_relations.parts() != relations.parts()
    {
        return Err(error("relations do not correspond to the program"));
    }
    Ok(CheckedPublication {
        description,
        description_bytes: description_bytes.to_vec(),
        publication_id: expected,
        objects,
        program: Arc::new(program),
        names,
        relations,
    })
}

fn diagnose_other_segments(
    program: &Program,
    names: Option<&NameTable>,
    outlines: &[ContentOutline],
    diagnostics: &mut Vec<crate::Diagnostic>,
) -> Result<()> {
    let mut segments = Vec::new();
    for reference in program.manifest().graphs.keys() {
        for (&id, plan) in &program.graph(reference)?.nodes {
            if let Plan::Passage(p) = plan {
                for (point_index, point) in p.choice_points.iter().enumerate() {
                    for (option_index, option) in point.options.iter().enumerate() {
                        if let narrata_nodes::plan::Outcome::Local {
                            reply: Some(segment),
                            ..
                        } = &option.outcome
                        {
                            let path = names.and_then(|n| n.node(reference, &id)).map_or_else(
                                || format!("{}/{id}", reference.label()),
                                |alias| {
                                    format!(
                                        "packages.{}.graphs.{}.nodes.{alias}",
                                        reference.package, reference.graph
                                    )
                                },
                            );
                            segments.push((segment.clone(),format!("{path}.choice_points[{point_index}].options[{option_index}].outcome.reply"),Some(id),Some(point.id),Some(option.id)));
                        }
                    }
                }
            }
        }
    }
    for (outcome, ending) in &program.manifest().product.endings {
        if let Some(segment) = &ending.body {
            segments.push((
                segment.clone(),
                format!("product.endings.{outcome}.body"),
                None,
                None,
                None,
            ));
        }
    }
    for (segment, path, node, choice_point, option) in segments {
        let outline = outlines
            .iter()
            .find(|o| o.provider == segment.unit.provider);
        let unit = outline.and_then(|o| o.units.get(&segment.unit.key));
        let mut issues = Vec::new();
        if outline.is_none() {
            issues.push(("outline_checks_skipped", None));
        } else if let Some(unit) = unit {
            let position = |anchor: &Option<narrata_kernel::content::AnchorId>, default| {
                anchor
                    .as_ref()
                    .map_or(default, |a| unit.blocks.iter().position(|b| b == a))
            };
            let first = position(&segment.first, (!unit.blocks.is_empty()).then_some(0));
            let last = position(&segment.last, unit.blocks.len().checked_sub(1));
            for anchor in [&segment.first, &segment.last].into_iter().flatten() {
                if !unit.blocks.contains(anchor) {
                    issues.push(("outline_anchor_missing", Some(anchor.clone())));
                }
            }
            if unit.blocks.is_empty() {
                issues.push(("outline_empty_unit", None));
            }
            if first.zip(last).is_some_and(|(first, last)| first > last) {
                issues.push(("outline_order", None));
            }
        } else {
            issues.push(("outline_unit_missing", None));
        }
        for (code, anchor) in issues {
            diagnostics.push(crate::Diagnostic {
                severity: crate::Severity::Warning,
                code: code.into(),
                location: crate::validate::Location {
                    pointer: crate::validate::source_pointer(&path),
                    source_path: Some(path.clone()),
                    node,
                    choice_point,
                    option,
                    unit: Some(segment.unit.clone()),
                    anchor,
                    ..Default::default()
                },
                related: Vec::new(),
                message: format!("The content outline does not verify this body slice ({code})."),
            });
        }
    }
    Ok(())
}
