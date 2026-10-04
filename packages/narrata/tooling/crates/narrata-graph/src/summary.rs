use std::collections::BTreeSet;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{Analysis, ClusterId, EndingClass, Id, MAX_NODES, model::CheckedGraph};

pub const SUMMARY_VERSION: u16 = 1;
pub const MAX_SUMMARY_BYTES: usize = 512 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BottleneckRun {
    pub cluster: ClusterId,
    pub first: Id,
    pub last: Id,
    #[schemars(range(min = 1, max = 1_000_000))]
    pub node_count: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EndingSummary {
    pub node: Id,
    pub class: EndingClass,
    pub reachable: bool,
    /// Clusters containing a reachable ancestor of this ending. These are a
    /// structural route approximation, not a proof that conditions are feasible.
    pub clusters: Vec<ClusterId>,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticSummary {
    #[schemars(range(min = 1, max = 1))]
    pub format_version: u16,
    /// Program manifest object ID, attached by a product adapter, not inferred.
    #[schemars(regex(pattern = "^[0-9a-f]{64}$"))]
    pub artifact_id: Option<String>,
    pub entries: Vec<Id>,
    pub endings: Vec<EndingSummary>,
    /// Common bottleneck sequence compacted into consecutive authored-cluster
    /// runs. Exact interior IDs remain in `Analysis::bottlenecks`.
    pub bottleneck_runs: Vec<BottleneckRun>,
    // Entity introductions deliberately have no inferred representation here:
    // they require a host-independent entity model before becoming facts.
}

#[derive(Debug, thiserror::Error)]
pub enum SummaryError {
    #[error("invalid summary JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid semantic summary: {0}")]
    Invalid(&'static str),
    #[error("semantic summary exceeds the byte budget")]
    Bytes,
}

pub fn summary_schema() -> schemars::Schema {
    schemars::schema_for!(SemanticSummary)
}

fn sorted_unique<T: Ord>(items: &[T]) -> bool {
    items.windows(2).all(|pair| pair[0] < pair[1])
}

impl SemanticSummary {
    fn validate(&self) -> Result<(), SummaryError> {
        if self.format_version != SUMMARY_VERSION {
            return Err(SummaryError::Invalid("format version"));
        }
        if self.artifact_id.as_ref().is_some_and(|id| {
            id.len() != 64
                || !id
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        }) {
            return Err(SummaryError::Invalid("artifact identity"));
        }
        if self.entries.len() > MAX_NODES
            || self.endings.len() > MAX_NODES
            || self.bottleneck_runs.len() > MAX_NODES
            || !sorted_unique(&self.entries)
            || self
                .endings
                .windows(2)
                .any(|pair| pair[0].node >= pair[1].node)
        {
            return Err(SummaryError::Invalid("row budget or identity order"));
        }
        for ending in &self.endings {
            if ending.clusters.len() > MAX_NODES
                || !sorted_unique(&ending.clusters)
                || ending.reachable == ending.clusters.is_empty()
            {
                return Err(SummaryError::Invalid("ending cluster set"));
            }
        }
        let mut boundary_ids = BTreeSet::new();
        let mut count = 0_u64;
        for run in &self.bottleneck_runs {
            count += u64::from(run.node_count);
            if run.node_count == 0
                || count > MAX_NODES as u64
                || (run.node_count == 1) != (run.first == run.last)
                || !boundary_ids.insert(run.first)
                || (run.first != run.last && !boundary_ids.insert(run.last))
            {
                return Err(SummaryError::Invalid("bottleneck run"));
            }
        }
        if self
            .bottleneck_runs
            .windows(2)
            .any(|pair| pair[0].cluster == pair[1].cluster)
        {
            return Err(SummaryError::Invalid("uncompressed bottleneck sequence"));
        }
        Ok(())
    }

    pub fn encode_json(&self) -> Result<Vec<u8>, SummaryError> {
        self.validate()?;
        let bytes = serde_json::to_vec(self)?;
        if bytes.len() > MAX_SUMMARY_BYTES {
            return Err(SummaryError::Bytes);
        }
        Ok(bytes)
    }

    /// JSON permits whitespace and key reordering; duplicate/unknown fields and
    /// unsupported versions are rejected rather than normalized into v1.
    pub fn decode_json(bytes: &[u8]) -> Result<Self, SummaryError> {
        if bytes.len() > MAX_SUMMARY_BYTES {
            return Err(SummaryError::Bytes);
        }
        let summary: Self = serde_json::from_slice(bytes)?;
        summary.validate()?;
        Ok(summary)
    }
}

pub(crate) fn summarize(graph: &CheckedGraph, analysis: &Analysis) -> SemanticSummary {
    let mut endings = Vec::new();
    // Epoch marks reuse the traversal allocation across ending routes.
    let mut visited = vec![0_usize; graph.nodes.len()];
    for (ordinal, (&ending, &class)) in graph.endings.iter().enumerate() {
        let mut clusters = BTreeSet::new();
        let mut pending = vec![ending];
        while let Some(node) = pending.pop() {
            if visited[node] != ordinal + 1 && analysis.rows[node].reachable {
                visited[node] = ordinal + 1;
                clusters.insert(graph.nodes[node].cluster);
                pending.extend(&graph.predecessors[node]);
            }
        }
        endings.push(EndingSummary {
            node: graph.nodes[ending].id,
            class,
            reachable: analysis.rows[ending].reachable,
            clusters: clusters.into_iter().collect(),
        });
    }
    let mut bottleneck_runs: Vec<BottleneckRun> = Vec::new();
    for &index in &analysis.bottlenecks {
        let node = &graph.nodes[index as usize];
        if let Some(last) = bottleneck_runs.last_mut()
            && last.cluster == node.cluster
        {
            last.last = node.id;
            last.node_count += 1;
        } else {
            bottleneck_runs.push(BottleneckRun {
                cluster: node.cluster,
                first: node.id,
                last: node.id,
                node_count: 1,
            });
        }
    }
    SemanticSummary {
        format_version: SUMMARY_VERSION,
        artifact_id: None,
        entries: graph
            .entries
            .iter()
            .map(|entry| graph.nodes[*entry].id)
            .collect(),
        endings,
        bottleneck_runs,
    }
}
