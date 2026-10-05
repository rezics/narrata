//! R2's publication projection. The graph is structural: conditions, proposal requests and
//! call-stack feasibility are not solved. Geometry never carries resolved content.

use std::collections::{BTreeMap, BTreeSet};
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
use std::path::Path;

use narrata_graph::{
    ClusterId, Edge, EdgeKind, Ending, EndingClass, Graph, Id, LabelTable, Node, NodeKind,
    Publication, SemanticSummary, encode_labels, prepare,
    wire::{EncodedGraph, EncodedObject, encode_publication},
};
use narrata_kernel::{
    codec::{CborWriter, digest_bytes},
    content::ContentRef,
};
use narrata_nodes::{
    Analysis, ArtifactId, ContentOutline, Diagnostic, Error, NameTable, NodeId, Program, Result,
    plan::{GraphRef, Outcome, Plan},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
use crate::{pretty, write_bytes, write_text};

#[derive(Debug)]
pub struct StructuralGraph {
    pub graph: Graph,
    pub labels: Vec<LabelTable>,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LabelObject {
    pub cluster: ClusterId,
    #[schemars(regex(pattern = "^[0-9a-f]{64}$"))]
    pub object_id: String,
}

/// Bootstrap references for object-id-named files in the companion `.graph/` directory.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GraphFiles {
    #[schemars(range(min = 1, max = 1))]
    pub format_version: u16,
    pub artifact_id: ArtifactId,
    #[schemars(regex(pattern = "^[0-9a-f]{64}$"))]
    pub index_object_id: String,
    pub labels: Vec<LabelObject>,
}

pub fn graph_files_schema() -> schemars::Schema {
    schemars::schema_for!(GraphFiles)
}

impl GraphFiles {
    fn validate(&self) -> Result<()> {
        let digest = |id: &str| {
            id.len() == 64
                && id
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        };
        if self.format_version != 1
            || !digest(&self.index_object_id)
            || self.labels.len() > narrata_graph::MAX_NODES
            || self
                .labels
                .windows(2)
                .any(|pair| pair[0].cluster >= pair[1].cluster)
        {
            return Err(graph_error("invalid publication references or version"));
        }
        let mut objects = BTreeSet::from([self.index_object_id.as_str()]);
        for label in &self.labels {
            if !digest(&label.object_id) || !objects.insert(label.object_id.as_str()) {
                return Err(graph_error("invalid or repeated label object identity"));
            }
        }
        Ok(())
    }

    /// JSON exchange permits whitespace, but not unknown/duplicate fields, invalid object
    /// identities, versions, unsorted or duplicate clusters, or repeated object references.
    pub fn decode_json(bytes: &[u8]) -> Result<Self> {
        let text = std::str::from_utf8(bytes)
            .map_err(|_| graph_error("publication references are not UTF-8"))?;
        let files: Self = narrata_nodes::parse_json_limited(
            text,
            narrata_graph::wire::MAX_OBJECT_BYTES as usize,
        )?;
        files.validate()?;
        Ok(files)
    }
}

#[derive(Debug)]
pub struct PublishedGraph {
    pub geometry: EncodedGraph,
    pub labels: Vec<EncodedObject>,
    pub files: GraphFiles,
    pub summary: SemanticSummary,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ClusterSource {
    Content(ContentRef),
    Graph(GraphRef),
}

fn cluster_id(source: &ClusterSource) -> ClusterId {
    let (domain, bytes) = match source {
        ClusterSource::Content(reference) => {
            ("narrata.graph.content-cluster", reference.to_bytes())
        }
        ClusterSource::Graph(reference) => {
            let mut writer = CborWriter::new();
            writer.array(2);
            writer.text(&reference.package);
            writer.text(&reference.graph);
            ("narrata.graph.graph-cluster", writer.into_bytes())
        }
    };
    let digest = digest_bytes(domain, 1, &bytes);
    let mut bytes = [0; 16];
    bytes.copy_from_slice(&digest[..16]);
    ClusterId(bytes)
}

/// Includes every static node. Return edges start at the matching callee return node, so
/// called graphs connect to their continuations rather than appearing as dead ends.
pub fn structural_graph(program: &Program) -> Result<StructuralGraph> {
    let manifest = program.manifest();
    let mut graphs = BTreeMap::new();
    for reference in manifest.graphs.keys() {
        graphs.insert(reference.clone(), program.graph(reference)?);
    }
    let mut node_count = 0_usize;
    for graph in graphs.values() {
        node_count = node_count
            .checked_add(graph.nodes.len())
            .ok_or_else(|| graph_error("node budget"))?;
        if node_count > narrata_graph::MAX_NODES {
            return Err(graph_error("node budget"));
        }
    }
    let mut returns = BTreeMap::<(GraphRef, String), Vec<NodeId>>::new();
    for (reference, graph) in &graphs {
        for (id, plan) in &graph.nodes {
            if let Plan::Return { outcome } = plan {
                returns
                    .entry((reference.clone(), outcome.clone()))
                    .or_default()
                    .push(*id);
            }
        }
    }
    let mut out = Graph::default();
    let entry = graphs
        .get(&manifest.product.entry)
        .ok_or_else(|| Error::new("reference", "product.entry", "entry graph is missing"))?;
    out.entries.push(Id(*entry.header.entry.as_bytes()));
    let mut sources = BTreeMap::new();
    let mut labels = BTreeMap::<_, LabelTable>::new();
    for (reference, graph) in &graphs {
        for (id, plan) in &graph.nodes {
            let source = match plan {
                Plan::Passage(passage) => passage
                    .body
                    .as_ref()
                    .map(|body| ClusterSource::Content(body.unit.clone()))
                    .unwrap_or_else(|| ClusterSource::Graph(reference.clone())),
                _ => ClusterSource::Graph(reference.clone()),
            };
            let cluster = cluster_id(&source);
            if let Some(previous) = sources.insert(cluster, source.clone())
                && previous != source
            {
                return Err(Error::new(
                    "graph_cluster_collision",
                    id.to_string(),
                    "distinct author clusters have the same identity",
                ));
            }
            let table = labels.entry(cluster).or_insert_with(|| LabelTable {
                cluster,
                titles: BTreeMap::new(),
            });
            let node = Id(*id.as_bytes());
            let mut edge = |target: NodeId, kind| -> Result<()> {
                if out.edges.len() >= narrata_graph::MAX_EDGES {
                    return Err(graph_error("edge budget"));
                }
                out.edges.push(Edge {
                    source: node,
                    target: Id(*target.as_bytes()),
                    kind,
                });
                Ok(())
            };
            let kind = match plan {
                Plan::Passage(passage) => {
                    if let Some(title) = &passage.title {
                        table.titles.insert(node, title.clone());
                    }
                    for point in &passage.choice_points {
                        for option in &point.options {
                            if let Outcome::Branch { target } = option.outcome {
                                edge(target, EdgeKind::Choice)?;
                            }
                        }
                    }
                    if let Some(next) = passage.next {
                        edge(next, EdgeKind::Next)?;
                    }
                    NodeKind::Passage
                }
                Plan::Branch {
                    when_true,
                    when_false,
                    ..
                } => {
                    edge(*when_true, EdgeKind::Conditional)?;
                    edge(*when_false, EdgeKind::Conditional)?;
                    NodeKind::Branch
                }
                Plan::Mutate { next, .. } => {
                    edge(*next, EdgeKind::Next)?;
                    NodeKind::State
                }
                Plan::Call {
                    target, on_return, ..
                } => {
                    let callee = program.resolve_call(reference, target)?;
                    let callee_graph = graphs.get(&callee).ok_or_else(|| {
                        Error::new("reference", id.to_string(), "callee graph is missing")
                    })?;
                    edge(callee_graph.header.entry, EdgeKind::Call)?;
                    for (outcome, continuation) in on_return {
                        if let Some(nodes) = returns.get(&(callee.clone(), outcome.clone())) {
                            if nodes.len() > narrata_graph::MAX_EDGES - out.edges.len() {
                                return Err(graph_error("edge budget"));
                            }
                            out.edges.extend(nodes.iter().map(|id| Edge {
                                source: Id(*id.as_bytes()),
                                target: Id(*continuation.as_bytes()),
                                kind: EdgeKind::Return,
                            }));
                        }
                    }
                    NodeKind::Call
                }
                Plan::Return { outcome } => {
                    if reference == &manifest.product.entry {
                        out.endings.push(Ending {
                            node,
                            class: EndingClass::Unspecified,
                        });
                        if let Some(title) = manifest
                            .product
                            .endings
                            .get(outcome)
                            .and_then(|ending| ending.title.as_ref())
                        {
                            table.titles.insert(node, title.clone());
                        }
                    }
                    NodeKind::Return
                }
            };
            out.nodes.push(Node {
                id: node,
                kind,
                cluster,
                author_order: None,
            });
        }
    }
    Ok(StructuralGraph {
        graph: out,
        labels: labels.into_values().collect(),
    })
}

fn graph_error(error: impl std::fmt::Display) -> Error {
    Error::new("graph_publication", "graph", error.to_string())
}

fn diagnostics(
    program: &Program,
    names: Option<&NameTable>,
    publication: &Publication,
) -> Result<Vec<Diagnostic>> {
    let mut paths = BTreeMap::new();
    for reference in program.manifest().graphs.keys() {
        for id in program.graph(reference)?.nodes.keys() {
            let path = match names.and_then(|table| table.node(reference, id)) {
                Some(alias) => format!(
                    "packages.{}.graphs.{}.nodes.{alias}",
                    reference.package, reference.graph
                ),
                None => format!("{}/{id}", reference.label()),
            };
            paths.insert(Id(*id.as_bytes()), path);
        }
    }
    let mut out = Vec::new();
    for (node, row) in publication.nodes.iter().zip(&publication.analysis.rows) {
        let path = paths
            .get(&node.id)
            .ok_or_else(|| graph_error("unknown projected node"))?;
        for (present, code, message) in [
            (
                !row.reachable,
                "structurally_unreachable",
                "No structural path from the product entry (conditions and call stacks are not solved).",
            ),
            (
                row.dead_end,
                "structural_dead_end",
                "This node has no structural continuation and is not a product ending.",
            ),
            (
                row.bottleneck,
                "structural_bottleneck",
                "Every finite structural entry-to-ending route passes through this node.",
            ),
        ] {
            if present {
                out.push(Diagnostic {
                    code: code.into(),
                    path: path.clone(),
                    message: message.into(),
                });
            }
        }
    }
    for component in &publication.analysis.components {
        if let Some(&first) = component.first()
            && publication.analysis.rows[first as usize].cyclic
        {
            let node = &publication.nodes[first as usize];
            out.push(Diagnostic {
                code: "structural_cycle".into(),
                path: paths.get(&node.id).ok_or_else(|| graph_error("unknown cycle node"))?.clone(),
                message: format!("A structural cycle contains {} node(s); conditions and call stacks are not solved.", component.len()),
            });
        }
    }
    Ok(out)
}

pub fn publish(program: &Program, names: Option<&NameTable>) -> Result<PublishedGraph> {
    let input = structural_graph(program)?;
    let mut publication = prepare(input.graph).map_err(graph_error)?;
    publication.summary.artifact_id = Some(hex::encode(program.artifact_id().as_bytes()));
    // Validate the JSON projection before any publication files are written.
    publication.summary.encode_json().map_err(graph_error)?;
    let diagnostics = diagnostics(program, names, &publication)?;
    let geometry = encode_publication(&publication).map_err(graph_error)?;
    let mut labels = Vec::new();
    let mut label_objects = Vec::new();
    for table in input.labels {
        let object = encode_labels(&table).map_err(graph_error)?;
        label_objects.push(LabelObject {
            cluster: table.cluster,
            object_id: hex::encode(object.id),
        });
        labels.push(object);
    }
    let files = GraphFiles {
        format_version: 1,
        artifact_id: program.artifact_id(),
        index_object_id: hex::encode(geometry.index.id),
        labels: label_objects,
    };
    files.validate()?;
    Ok(PublishedGraph {
        geometry,
        labels,
        files,
        summary: publication.summary,
        diagnostics,
    })
}

/// Writes object files first, then the JSON references. Each file is atomic; the set is not
/// a transaction. Existing immutable objects can remain available to readers of old indexes.
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
pub fn write_publication(output: &Path, publication: &PublishedGraph) -> Result<()> {
    publication.files.validate()?;
    let objects = output.with_extension("graph");
    for object in std::iter::once(&publication.geometry.index)
        .chain(&publication.geometry.tiles)
        .chain(&publication.labels)
    {
        write_bytes(&objects.join(object.filename()), &object.bytes)?;
    }
    let summary = publication.summary.encode_json().map_err(graph_error)?;
    write_bytes(&output.with_extension("summary.json"), &summary)?;
    write_text(
        &output.with_extension("graph.json"),
        &pretty(&publication.files)?,
    )
}

/// The same publication diagnostics for CLI and Wasm callers. Outline defects are
/// diagnostics; duplicate providers are invalid input. No content text is resolved.
pub fn publication_analysis(
    mut analysis: Analysis,
    program: &Program,
    names: Option<&NameTable>,
    publication: &PublishedGraph,
    outlines: &[ContentOutline],
) -> Result<Analysis> {
    analysis
        .diagnostics
        .retain(|diagnostic| diagnostic.code != "structurally_unreachable");
    analysis
        .diagnostics
        .extend(publication.diagnostics.iter().cloned());
    let mut providers = BTreeSet::new();
    for outline in outlines {
        if !providers.insert(outline.provider.clone()) {
            return Err(Error::new(
                "outline_provider",
                format!("content.{}", outline.provider),
                "supply one original-language outline per provider",
            ));
        }
        analysis
            .diagnostics
            .extend(narrata_nodes::outline::diagnose_outline(
                program, names, outline,
            )?);
    }
    let mut required = BTreeSet::new();
    for reference in program.manifest().graphs.keys() {
        for plan in program.graph(reference)?.nodes.values() {
            if let Plan::Passage(passage) = plan
                && let Some(body) = &passage.body
            {
                required.insert(body.unit.provider.clone());
            }
        }
    }
    for provider in required.difference(&providers) {
        analysis.diagnostics.push(Diagnostic {
            code: "outline_checks_skipped".into(),
            path: format!("content.{provider}"),
            message:
                "No content outline was supplied; anchor, order and marker checks were skipped."
                    .into(),
        });
    }
    Ok(analysis)
}
