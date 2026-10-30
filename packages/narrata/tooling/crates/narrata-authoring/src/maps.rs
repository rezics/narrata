//! Reader maps contain executed, revealed facts only. Coordinates do not depend on the
//! author's layout, hidden node counts, structural first rows or ending route sets.

use crate::{
    objects::{Digest, error},
    relations::{CheckedRelations, RelationRow, unit_id},
};
use narrata_kernel::content::{ContentRef, Segment};
use narrata_nodes::{
    ArtifactId, ChoicePointId, CommitId, NodeId, OptionId, Result,
    history::visits::{CheckedVisits, VisitActionV1, VisitFrontierV1, VisitMissingV1},
};
use schemars::JsonSchema;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReaderUnitV1 {
    pub unit_id: Digest,
    pub content: ContentRef,
    pub segments: Vec<Segment>,
    // JSON Schema's int32 format is only an annotation in transport validators.
    #[schemars(range(min = i32::MIN, max = i32::MAX))]
    pub x: i32,
    #[schemars(range(min = i32::MIN, max = i32::MAX))]
    pub y: i32,
}

#[derive(Clone, Debug, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReaderNodeV1 {
    pub node: NodeId,
    pub unit_id: Option<Digest>,
    #[schemars(range(min = i32::MIN, max = i32::MAX))]
    pub x: i32,
    #[schemars(range(min = i32::MIN, max = i32::MAX))]
    pub y: i32,
}

#[derive(Clone, Debug, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReaderEdgeV1 {
    pub source: NodeId,
    pub target: NodeId,
    pub choice_point: Option<ChoicePointId>,
    pub option: Option<OptionId>,
}

#[derive(Clone, Debug, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReaderMapSourceV1 {
    pub cursor: CommitId,
    pub complete: bool,
    pub missing: Vec<VisitMissingV1>,
    pub frontier: Vec<VisitFrontierV1>,
}

#[derive(Clone, Debug, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReaderMapV1 {
    pub format_version: u16,
    pub artifact_id: ArtifactId,
    pub projection_version: crate::objects::Version1,
    pub sources: Vec<ReaderMapSourceV1>,
    pub units: Vec<ReaderUnitV1>,
    pub nodes: Vec<ReaderNodeV1>,
    pub edges: Vec<ReaderEdgeV1>,
    pub endings: Vec<NodeId>,
}

/// One map belongs to exactly one artifact. Earlier migration segments require their own
/// projection and map; they are never relabelled with the current artifact identity.
pub fn reader_map(projection: &CheckedRelations, visits: &CheckedVisits) -> Result<ReaderMapV1> {
    map_rows(projection.index().artifact_id, projection.rows(), visits)
}

pub fn reader_map_from_subset(
    projection: &crate::relations::CheckedReaderRelations,
    visits: &CheckedVisits,
) -> Result<ReaderMapV1> {
    map_rows(projection.artifact_id(), projection.rows(), visits)
}

fn map_rows(
    artifact: ArtifactId,
    rows: &[RelationRow],
    visits: &CheckedVisits,
) -> Result<ReaderMapV1> {
    let node_rows: BTreeMap<_, _> = rows
        .iter()
        .filter_map(|r| match r {
            RelationRow::Node(node) => Some((node.node_id, node)),
            _ => None,
        })
        .collect();
    let mut map = ReaderMapV1 {
        format_version: 1,
        artifact_id: artifact,
        projection_version: crate::objects::Version1,
        sources: Vec::new(),
        units: Vec::new(),
        nodes: Vec::new(),
        edges: Vec::new(),
        endings: Vec::new(),
    };
    let mut nodes = BTreeSet::new();
    let mut units = BTreeMap::new();
    let mut edges = BTreeSet::new();
    let mut endings = BTreeSet::new();
    let mut matched = false;
    for source in &visits.report().sources {
        let relevant: Vec<_> = source
            .segments
            .iter()
            .filter(|s| s.artifact_id == artifact)
            .collect();
        if relevant.is_empty() && source.artifact_id != Some(artifact) {
            continue;
        }
        matched = true;
        let current = source.artifact_id == Some(artifact);
        map.sources.push(ReaderMapSourceV1 {
            cursor: source.cursor,
            complete: source.complete,
            missing: source.missing.clone(),
            frontier: if current {
                source.frontier.clone()
            } else {
                Vec::new()
            },
        });
        for segment in relevant {
            let mut previous = None;
            let mut selection = None;
            for event in &segment.events {
                match &event.action {
                    VisitActionV1::Visit { static_node } => {
                        if *static_node {
                            if !node_rows.contains_key(&event.node) {
                                return Err(error("visited node absent from exact projection"));
                            }
                            if nodes.insert(event.node) {
                                let (x, y) = coordinate(map.nodes.len())?;
                                map.nodes.push(ReaderNodeV1 {
                                    node: event.node,
                                    unit_id: None,
                                    x,
                                    y,
                                });
                            }
                            if let Some(from) = previous {
                                let (choice_point, option) =
                                    selection.map_or((None, None), |(c, o)| (Some(c), Some(o)));
                                if edges.insert((from, event.node, choice_point, option)) {
                                    map.edges.push(ReaderEdgeV1 {
                                        source: from,
                                        target: event.node,
                                        choice_point,
                                        option,
                                    });
                                }
                            }
                            previous = Some(event.node);
                        } else {
                            // Proposed nodes contribute presented references, never static topology.
                            previous = None;
                        }
                        selection = None;
                    }
                    VisitActionV1::Selected {
                        choice_point,
                        option,
                        branch: true,
                    } => selection = Some((*choice_point, *option)),
                    VisitActionV1::Segment { segment, .. } => {
                        let id = unit_id(&segment.unit);
                        let index = if let Some(index) = units.get(&id) {
                            *index
                        } else {
                            let index = map.units.len();
                            let (x, y) = coordinate(index)?;
                            map.units.push(ReaderUnitV1 {
                                unit_id: id,
                                content: segment.unit.clone(),
                                segments: Vec::new(),
                                x,
                                y,
                            });
                            units.insert(id, index);
                            index
                        };
                        let unit = &mut map.units[index];
                        if unit.content != segment.unit {
                            return Err(error("visited content identity collision"));
                        }
                        if !unit.segments.contains(segment) {
                            unit.segments.push(segment.clone());
                        }
                    }
                    VisitActionV1::Finished { .. } => {
                        if !node_rows.get(&event.node).is_some_and(|n| n.kind == 4) {
                            return Err(error("visited ending absent from projection"));
                        }
                        if endings.insert(event.node) {
                            map.endings.push(event.node);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    if !matched {
        return Err(error("visit evidence does not contain this artifact"));
    }
    for node in &mut map.nodes {
        node.unit_id = node_rows[&node.node]
            .unit_id
            .filter(|id| units.contains_key(id));
    }
    Ok(map)
}

fn coordinate(index: usize) -> Result<(i32, i32)> {
    let index = i32::try_from(index).map_err(|_| error("reader coordinate budget"))?;
    Ok((index % 8, index / 8))
}
