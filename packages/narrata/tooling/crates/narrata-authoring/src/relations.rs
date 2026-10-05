//! Rebuildable structural facts (ADR 0022 §5). No conditions or call stacks are solved.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use narrata_graph::{ClusterId, EndingClass, Id, SemanticSummary};
use narrata_kernel::content::{AnchorId, ContentKey, ContentRef, ProviderId, Segment};
use narrata_nodes::{
    ArtifactId, ChoicePointId, NodeId, OptionId, Program, Result, canonical_json,
    plan::{Outcome, Plan},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    graph::structural_graph,
    objects::{Digest, Version1, decode_json, error},
};

pub const MAX_PART_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_EXPANDED_ROWS: usize = 4_000_000;
pub const REFERENCE_DDL: &str = include_str!("../sql/relations-v1.sql");

#[derive(
    Clone, Copy, Debug, Deserialize, Eq, JsonSchema, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum RelationKind {
    Unit,
    Node,
    Edge,
    Entry,
    Ending,
    Route,
    Cluster,
    First,
}

macro_rules! row {
    ($name:ident { $($field:ident: $type:ty),* $(,)? }) => {
        #[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
        #[serde(deny_unknown_fields)]
        pub struct $name {
            pub artifact_id: ArtifactId,
            pub projection_version: Version1,
            $(pub $field: $type),*
        }
    };
}
row!(UnitRow {
    unit_id: Digest,
    provider: ProviderId,
    content_key: ContentKey
});
row!(NodeRow { node_id: NodeId, package: String, graph: String, kind: u8, unit_id: Option<Digest>, first_anchor: Option<AnchorId>, last_anchor: Option<AnchorId>, cluster_id: ClusterId });
row!(EdgeRow { edge_no: u64, source_unit: Digest, target_unit: Digest, source_node: NodeId, target_node: NodeId, edge_kind: u8, choice_point: Option<ChoicePointId>, option: Option<OptionId> });
row!(EntryRow { node_id: NodeId });
row!(EndingRow {
    node_id: NodeId,
    class: EndingClass,
    reachable: bool
});
row!(RouteRow {
    ending_node: NodeId,
    cluster_id: ClusterId
});
row!(ClusterRow { cluster_id: ClusterId, unit_id: Option<Digest>, package: Option<String>, graph: Option<String> });
row!(FirstRow {
    unit_id: Digest,
    node_id: NodeId,
    distance: u64
});

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "row_type", rename_all = "snake_case")]
pub enum RelationRow {
    Unit(UnitRow),
    Node(NodeRow),
    Edge(EdgeRow),
    Entry(EntryRow),
    Ending(EndingRow),
    Route(RouteRow),
    Cluster(ClusterRow),
    First(FirstRow),
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Key {
    Unit(Digest),
    Node(NodeId),
    Edge(
        Digest,
        Digest,
        NodeId,
        NodeId,
        u8,
        Option<ChoicePointId>,
        Option<OptionId>,
    ),
    Entry(NodeId),
    Ending(NodeId),
    Route(NodeId, ClusterId),
    Cluster(ClusterId),
    First(Digest, NodeId),
}
impl RelationRow {
    pub fn kind(&self) -> RelationKind {
        match self {
            Self::Unit(_) => RelationKind::Unit,
            Self::Node(_) => RelationKind::Node,
            Self::Edge(_) => RelationKind::Edge,
            Self::Entry(_) => RelationKind::Entry,
            Self::Ending(_) => RelationKind::Ending,
            Self::Route(_) => RelationKind::Route,
            Self::Cluster(_) => RelationKind::Cluster,
            Self::First(_) => RelationKind::First,
        }
    }
    fn artifact(&self) -> ArtifactId {
        match self {
            Self::Unit(r) => r.artifact_id,
            Self::Node(r) => r.artifact_id,
            Self::Edge(r) => r.artifact_id,
            Self::Entry(r) => r.artifact_id,
            Self::Ending(r) => r.artifact_id,
            Self::Route(r) => r.artifact_id,
            Self::Cluster(r) => r.artifact_id,
            Self::First(r) => r.artifact_id,
        }
    }
    fn key(&self) -> Key {
        match self {
            Self::Unit(r) => Key::Unit(r.unit_id),
            Self::Node(r) => Key::Node(r.node_id),
            Self::Edge(r) => Key::Edge(
                r.source_unit,
                r.target_unit,
                r.source_node,
                r.target_node,
                r.edge_kind,
                r.choice_point,
                r.option,
            ),
            Self::Entry(r) => Key::Entry(r.node_id),
            Self::Ending(r) => Key::Ending(r.node_id),
            Self::Route(r) => Key::Route(r.ending_node, r.cluster_id),
            Self::Cluster(r) => Key::Cluster(r.cluster_id),
            Self::First(r) => Key::First(r.unit_id, r.node_id),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RelationPart {
    pub artifact_id: ArtifactId,
    pub projection_version: Version1,
    pub kind: RelationKind,
    pub rows: Vec<RelationRow>,
}
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PartReference {
    pub kind: RelationKind,
    pub ordinal: u32,
    pub object_id: Digest,
    pub bytes: u64,
    pub row_count: u64,
}
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RelationIndex {
    pub artifact_id: ArtifactId,
    pub projection_version: Version1,
    pub parts: Vec<PartReference>,
    pub counts: BTreeMap<RelationKind, u64>,
    pub row_count: u64,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelationObject {
    pub id: Digest,
    pub bytes: Vec<u8>,
}

/// Checked encoding, complete part set, primary keys and cross-row references. Semantic
/// correspondence additionally requires `verify_program`, performed by publication readers.
#[derive(Debug)]
pub struct CheckedRelations {
    index: RelationIndex,
    index_object: RelationObject,
    parts: Vec<RelationObject>,
    rows: Vec<RelationRow>,
}
impl CheckedRelations {
    pub fn index(&self) -> &RelationIndex {
        &self.index
    }
    pub fn index_object(&self) -> &RelationObject {
        &self.index_object
    }
    pub fn parts(&self) -> &[RelationObject] {
        &self.parts
    }
    pub fn rows(&self) -> &[RelationRow] {
        &self.rows
    }
    pub fn verify_program(&self, program: &Program) -> Result<()> {
        let expected = project_relations(program)?;
        if expected.index_object != self.index_object || expected.parts != self.parts {
            return Err(error(
                "relation projection does not correspond to the program",
            ));
        }
        Ok(())
    }
}

pub fn unit_id(reference: &ContentRef) -> Digest {
    Digest::json("narrata.nodes.relation-unit", &reference.to_bytes())
}

pub fn project_relations(program: &Program) -> Result<CheckedRelations> {
    program.verify_artifact()?;
    let structural = structural_graph(program)?;
    let mut summary =
        narrata_graph::structural_summary(structural.graph, MAX_EXPANDED_ROWS).map_err(error)?;
    summary.artifact_id = Some(hex::encode(program.artifact_id().as_bytes()));
    project_with_summary(program, &summary)
}

pub(crate) fn project_with_summary(
    program: &Program,
    summary: &SemanticSummary,
) -> Result<CheckedRelations> {
    let artifact_id = program.artifact_id();
    let projection_version = Version1;
    let structural = structural_graph(program)?;
    let clusters: BTreeMap<_, _> = structural
        .graph
        .nodes
        .iter()
        .map(|n| (NodeId::from_bytes(n.id.0), n.cluster))
        .collect();
    let mut units = BTreeMap::<Digest, ContentRef>::new();
    let mut nodes = BTreeMap::<NodeId, NodeRow>::new();
    let mut cluster_rows = BTreeMap::new();
    let mut starts =
        BTreeMap::<NodeId, BTreeSet<(NodeId, u8, Option<ChoicePointId>, Option<OptionId>)>>::new();
    let mut add_unit = |segment: &Segment| -> Result<Digest> {
        let id = unit_id(&segment.unit);
        if let Some(previous) = units.insert(id, segment.unit.clone())
            && previous != segment.unit
        {
            return Err(error("content unit identity collision"));
        }
        if units.len() > MAX_EXPANDED_ROWS {
            return Err(error("relation unit budget"));
        }
        Ok(id)
    };
    for ending in program.manifest().product.endings.values() {
        if let Some(body) = &ending.body {
            add_unit(body)?;
        }
    }
    for reference in program.manifest().graphs.keys() {
        let graph = program.graph(reference)?;
        for (&node_id, plan) in &graph.nodes {
            let segment = match plan {
                Plan::Passage(p) => {
                    for point in &p.choice_points {
                        for option in &point.options {
                            match &option.outcome {
                                Outcome::Local {
                                    reply: Some(reply), ..
                                } => {
                                    add_unit(reply)?;
                                }
                                Outcome::Branch { target } => {
                                    starts.entry(node_id).or_default().insert((
                                        *target,
                                        0,
                                        Some(point.id),
                                        Some(option.id),
                                    ));
                                }
                                _ => {}
                            }
                        }
                    }
                    p.body.as_ref()
                }
                Plan::Return { outcome } if reference == &program.manifest().product.entry => {
                    program
                        .manifest()
                        .product
                        .endings
                        .get(outcome)
                        .and_then(|ending| ending.body.as_ref())
                }
                _ => None,
            };
            let unit = segment.map(&mut add_unit).transpose()?;
            let cluster_id = clusters[&node_id];
            // Returns retain their authored graph cluster even when they present an ending.
            let content_cluster = matches!(plan, Plan::Passage(_)) && unit.is_some();
            cluster_rows.entry(cluster_id).or_insert(ClusterRow {
                artifact_id,
                projection_version,
                cluster_id,
                unit_id: if content_cluster { unit } else { None },
                package: (!content_cluster).then(|| reference.package.clone()),
                graph: (!content_cluster).then(|| reference.graph.clone()),
            });
            let kind = match plan {
                Plan::Passage(_) => 0,
                Plan::Branch { .. } => 1,
                Plan::Mutate { .. } => 2,
                Plan::Call { .. } => 3,
                Plan::Return { .. } => 4,
            };
            nodes.insert(
                node_id,
                NodeRow {
                    artifact_id,
                    projection_version,
                    node_id,
                    package: reference.package.clone(),
                    graph: reference.graph.clone(),
                    kind,
                    unit_id: unit,
                    first_anchor: segment.and_then(|s| s.first.clone()),
                    last_anchor: segment.and_then(|s| s.last.clone()),
                    cluster_id,
                },
            );
        }
    }
    for edge in &structural.graph.edges {
        if edge.kind != narrata_graph::EdgeKind::Choice {
            starts
                .entry(NodeId::from_bytes(edge.source.0))
                .or_default()
                .insert((
                    NodeId::from_bytes(edge.target.0),
                    edge.kind as u8,
                    None,
                    None,
                ));
        }
    }
    let mut edges = BTreeMap::<Key, EdgeRow>::new();
    // Control visits are per starting transfer, not per path, so cycles terminate and
    // reconvergent paths retain all distinct originating options without path enumeration.
    for (&source_node, source) in &nodes {
        let Some(source_unit) = source.unit_id else {
            continue;
        };
        for &(target, edge_kind, choice_point, option) in
            starts.get(&source_node).into_iter().flatten()
        {
            let mut visited = BTreeSet::new();
            let mut pending = vec![target];
            while let Some(target_node) = pending.pop() {
                if !visited.insert(target_node) {
                    continue;
                }
                let target = &nodes[&target_node];
                if let Some(target_unit) = target.unit_id {
                    if source_unit != target_unit {
                        let row = EdgeRow {
                            artifact_id,
                            projection_version,
                            edge_no: 0,
                            source_unit,
                            target_unit,
                            source_node,
                            target_node,
                            edge_kind,
                            choice_point,
                            option,
                        };
                        edges
                            .entry(RelationRow::Edge(row.clone()).key())
                            .or_insert(row);
                        if edges.len() > MAX_EXPANDED_ROWS {
                            return Err(error("relation edge budget"));
                        }
                    }
                } else {
                    pending.extend(starts.get(&target_node).into_iter().flatten().map(|s| s.0));
                }
            }
        }
    }
    let mut distances = BTreeMap::new();
    let mut pending = VecDeque::new();
    for entry in &structural.graph.entries {
        let node = NodeId::from_bytes(entry.0);
        distances.insert(node, 0_u64);
        pending.push_back(node);
    }
    while let Some(node) = pending.pop_front() {
        let distance = distances[&node] + 1;
        for &(target, ..) in starts.get(&node).into_iter().flatten() {
            if let std::collections::btree_map::Entry::Vacant(entry) = distances.entry(target) {
                entry.insert(distance);
                pending.push_back(target);
            }
        }
    }
    let mut first = BTreeMap::<Digest, (u64, Vec<NodeId>)>::new();
    for (&node, row) in &nodes {
        if let (Some(unit), Some(&distance)) = (row.unit_id, distances.get(&node)) {
            let entry = first.entry(unit).or_insert((distance, Vec::new()));
            if distance < entry.0 {
                *entry = (distance, Vec::new());
            }
            if distance == entry.0 {
                entry.1.push(node);
            }
        }
    }
    let mut rows = Vec::new();
    rows.extend(units.into_iter().map(|(unit_id, reference)| {
        RelationRow::Unit(UnitRow {
            artifact_id,
            projection_version,
            unit_id,
            provider: reference.provider,
            content_key: reference.key,
        })
    }));
    rows.extend(nodes.into_values().map(RelationRow::Node));
    rows.extend(edges.into_values().enumerate().map(|(ordinal, mut r)| {
        r.edge_no = ordinal as u64;
        RelationRow::Edge(r)
    }));
    rows.extend(summary.entries.iter().map(|Id(id)| {
        RelationRow::Entry(EntryRow {
            artifact_id,
            projection_version,
            node_id: NodeId::from_bytes(*id),
        })
    }));
    let mut route_count = 0;
    for ending in &summary.endings {
        let node_id = NodeId::from_bytes(ending.node.0);
        rows.push(RelationRow::Ending(EndingRow {
            artifact_id,
            projection_version,
            node_id,
            class: ending.class,
            reachable: ending.reachable,
        }));
        for &cluster_id in &ending.clusters {
            route_count += 1;
            if route_count > MAX_EXPANDED_ROWS {
                return Err(error("relation route budget"));
            }
            rows.push(RelationRow::Route(RouteRow {
                artifact_id,
                projection_version,
                ending_node: node_id,
                cluster_id,
            }));
        }
    }
    rows.extend(cluster_rows.into_values().map(RelationRow::Cluster));
    rows.extend(first.into_iter().flat_map(|(unit_id, (distance, nodes))| {
        nodes.into_iter().map(move |node_id| {
            RelationRow::First(FirstRow {
                artifact_id,
                projection_version,
                unit_id,
                node_id,
                distance,
            })
        })
    }));
    rows.sort_by_key(RelationRow::key);
    encode_rows(artifact_id, rows)
}

fn encode_rows(artifact_id: ArtifactId, rows: Vec<RelationRow>) -> Result<CheckedRelations> {
    validate_rows(artifact_id, &rows)?;
    let mut parts = Vec::new();
    let mut references = Vec::new();
    let mut counts = BTreeMap::new();
    // Encode rows once, then count exact canonical envelope overhead and commas before
    // slicing. Repeatedly encoding an ever-growing part would make publication quadratic.
    for kind in [
        RelationKind::Unit,
        RelationKind::Node,
        RelationKind::Edge,
        RelationKind::Entry,
        RelationKind::Ending,
        RelationKind::Route,
        RelationKind::Cluster,
        RelationKind::First,
    ] {
        let selected: Vec<_> = rows
            .iter()
            .filter(|r| r.kind() == kind)
            .map(canonical_json)
            .collect::<Result<_>>()?;
        counts.insert(kind, selected.len() as u64);
        let empty = canonical_json(&RelationPart {
            artifact_id,
            projection_version: Version1,
            kind,
            rows: Vec::new(),
        })?;
        let split = empty
            .windows(2)
            .position(|s| s == b"[]")
            .ok_or_else(|| error("part array encoding"))?;
        let mut ordinal = 0;
        let mut begin = 0;
        while begin < selected.len() {
            let mut end = begin;
            let mut size = empty.len();
            while end < selected.len()
                && size + selected[end].len() + usize::from(end > begin) <= MAX_PART_BYTES
            {
                size += selected[end].len() + usize::from(end > begin);
                end += 1;
            }
            if begin == end {
                return Err(error("relation row exceeds 4 MiB"));
            }
            let mut bytes = Vec::with_capacity(size);
            bytes.extend_from_slice(&empty[..split + 1]);
            for (i, row) in selected[begin..end].iter().enumerate() {
                if i > 0 {
                    bytes.push(b',');
                }
                bytes.extend_from_slice(row);
            }
            bytes.extend_from_slice(&empty[split + 1..]);
            let id = Digest::json("narrata.nodes.relation-part", &bytes);
            references.push(PartReference {
                kind,
                ordinal,
                object_id: id,
                bytes: bytes.len() as u64,
                row_count: (end - begin) as u64,
            });
            parts.push(RelationObject { id, bytes });
            begin = end;
            ordinal += 1;
        }
    }
    let index = RelationIndex {
        artifact_id,
        projection_version: Version1,
        parts: references,
        counts,
        row_count: rows.len() as u64,
    };
    let bytes = canonical_json(&index)?;
    if bytes.len() > MAX_PART_BYTES {
        return Err(error("relation index byte budget"));
    }
    let index_object = RelationObject {
        id: Digest::json("narrata.nodes.relation-index", &bytes),
        bytes,
    };
    Ok(CheckedRelations {
        index,
        index_object,
        parts,
        rows,
    })
}

pub fn read_relations(
    index_bytes: &[u8],
    expected: Digest,
    mut load: impl FnMut(Digest) -> Result<Vec<u8>>,
) -> Result<CheckedRelations> {
    if Digest::json("narrata.nodes.relation-index", index_bytes) != expected {
        return Err(error("relation index identity"));
    }
    let index: RelationIndex = decode_json(index_bytes, MAX_PART_BYTES)?;
    let mut previous = None;
    let mut seen = BTreeSet::new();
    let mut totals = BTreeMap::new();
    for kind in [
        RelationKind::Unit,
        RelationKind::Node,
        RelationKind::Edge,
        RelationKind::Entry,
        RelationKind::Ending,
        RelationKind::Route,
        RelationKind::Cluster,
        RelationKind::First,
    ] {
        totals.insert(kind, 0_u64);
    }
    for reference in &index.parts {
        let expected_ordinal = match previous {
            Some((kind, ordinal)) if kind == reference.kind => ordinal + 1,
            _ => 0,
        };
        if previous.is_some_and(|(kind, _)| kind > reference.kind)
            || reference.ordinal != expected_ordinal
            || !seen.insert(reference.object_id)
            || reference.row_count == 0
            || reference.row_count > row_limit(reference.kind) as u64
            || reference.bytes > MAX_PART_BYTES as u64
        {
            return Err(error("relation part order, identity or budget"));
        }
        *totals
            .get_mut(&reference.kind)
            .ok_or_else(|| error("relation kind"))? += reference.row_count;
        if totals[&reference.kind] > row_limit(reference.kind) as u64 {
            return Err(error("relation row budget"));
        }
        previous = Some((reference.kind, reference.ordinal));
    }
    if totals != index.counts
        || totals.values().sum::<u64>() != index.row_count
        || totals
            .iter()
            .any(|(kind, count)| *count > row_limit(*kind) as u64)
    {
        return Err(error("relation row counts or budget"));
    }
    let mut rows = Vec::new();
    let mut parts = Vec::new();
    for reference in &index.parts {
        let bytes = load(reference.object_id)?;
        if bytes.len() as u64 != reference.bytes
            || Digest::json("narrata.nodes.relation-part", &bytes) != reference.object_id
        {
            return Err(error("relation part length or identity"));
        }
        let part: RelationPart = decode_json(&bytes, MAX_PART_BYTES)?;
        if part.artifact_id != index.artifact_id
            || part.kind != reference.kind
            || part.rows.len() as u64 != reference.row_count
            || part.rows.iter().any(|r| r.kind() != part.kind)
        {
            return Err(error("relation part ownership or type"));
        }
        rows.extend(part.rows);
        parts.push(RelationObject {
            id: reference.object_id,
            bytes,
        });
    }
    validate_rows(index.artifact_id, &rows)?;
    // Canonical splitting is part of projection v1, not an arbitrary transport choice.
    let canonical = encode_rows(index.artifact_id, rows)?;
    if canonical.index_object.bytes != index_bytes || canonical.parts != parts {
        return Err(error("noncanonical relation partition"));
    }
    Ok(canonical)
}

fn row_limit(kind: RelationKind) -> usize {
    if matches!(
        kind,
        RelationKind::Unit | RelationKind::Edge | RelationKind::Route
    ) {
        MAX_EXPANDED_ROWS
    } else {
        narrata_graph::MAX_NODES
    }
}

fn validate_rows(artifact: ArtifactId, rows: &[RelationRow]) -> Result<()> {
    let mut previous = None;
    let mut counts = BTreeMap::new();
    let mut units = BTreeMap::new();
    let mut nodes = BTreeMap::new();
    let mut clusters = BTreeMap::new();
    let mut endings = BTreeSet::new();
    for row in rows {
        let key = row.key();
        if row.artifact() != artifact || previous.as_ref().is_some_and(|p| *p >= key) {
            return Err(error("relation artifact or primary key order"));
        }
        previous = Some(key);
        let count = counts.entry(row.kind()).or_insert(0);
        *count += 1;
        if *count > row_limit(row.kind()) {
            return Err(error("relation row budget"));
        }
        match row {
            RelationRow::Unit(r) => {
                let reference = ContentRef {
                    provider: r.provider.clone(),
                    key: r.content_key.clone(),
                };
                if unit_id(&reference) != r.unit_id {
                    return Err(error("relation unit identity"));
                }
                units.insert(r.unit_id, reference);
            }
            RelationRow::Node(r) => {
                nodes.insert(r.node_id, r);
            }
            RelationRow::Cluster(r) => {
                clusters.insert(r.cluster_id, r);
            }
            RelationRow::Ending(r) => {
                endings.insert(r.node_id);
            }
            _ => {}
        }
    }
    let mut edge_no = 0;
    let mut first_distances = BTreeMap::new();
    for row in rows {
        let valid = match row {
            RelationRow::Unit(_) => true,
            RelationRow::Node(r) => {
                r.kind <= 4
                    && !r.package.is_empty()
                    && !r.graph.is_empty()
                    && r.unit_id.is_none_or(|u| units.contains_key(&u))
                    && (r.unit_id.is_some()
                        || (r.first_anchor.is_none() && r.last_anchor.is_none()))
                    && clusters.contains_key(&r.cluster_id)
            }
            RelationRow::Cluster(r) => match (r.unit_id, &r.package, &r.graph) {
                (Some(unit), None, None) => units.contains_key(&unit),
                (None, Some(package), Some(graph)) => !package.is_empty() && !graph.is_empty(),
                _ => false,
            },
            RelationRow::Edge(r) => {
                let ordinal_ok = r.edge_no == edge_no;
                edge_no += 1;
                ordinal_ok
                    && r.edge_kind <= 4
                    && r.source_unit != r.target_unit
                    && nodes
                        .get(&r.source_node)
                        .is_some_and(|n| n.unit_id == Some(r.source_unit))
                    && nodes
                        .get(&r.target_node)
                        .is_some_and(|n| n.unit_id == Some(r.target_unit))
                    && if r.edge_kind == 0 {
                        r.choice_point.is_some() && r.option.is_some()
                    } else {
                        r.choice_point.is_none() && r.option.is_none()
                    }
            }
            RelationRow::Entry(r) => nodes.contains_key(&r.node_id),
            RelationRow::Ending(r) => nodes.get(&r.node_id).is_some_and(|n| n.kind == 4),
            RelationRow::Route(r) => {
                endings.contains(&r.ending_node) && clusters.contains_key(&r.cluster_id)
            }
            RelationRow::First(r) => {
                let distance = first_distances.entry(r.unit_id).or_insert(r.distance);
                *distance == r.distance
                    && r.distance < narrata_graph::MAX_NODES as u64
                    && nodes
                        .get(&r.node_id)
                        .is_some_and(|n| n.unit_id == Some(r.unit_id))
            }
        };
        if !valid {
            return Err(error("invalid relation columns or reference"));
        }
    }
    Ok(())
}
