//! Read-only, checked execution evidence. A report describes presentation by a machine,
//! never human attention, authorization or entitlement. Only ancestor replay constructs
//! `CheckedVisits`; deserializing a host report cannot establish that proof.

use crate::{
    ArtifactId, ChoicePointId, CommitId, Error, NodeId, ObjectId, OptionId, Result,
    history::{
        NodeDomain,
        evolution::{EvolutionRegistry, PathComparisonV1, PathLimits, UnverifiedReason},
    },
    plan::GraphRef,
    runtime::Machine,
    view::{Interaction, Role},
};
use narrata_history::{DomainKinds, History};
use narrata_kernel::content::{ContentRef, Segment};
use narrata_storage::StorageBackend;
use schemars::JsonSchema;
use serde::Serialize;

pub const MAX_VISIT_EVENTS: usize = 100_000;
pub const MAX_VISIT_CURSORS: usize = 256;

#[derive(Clone, Debug, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VisitActionV1 {
    Visit {
        static_node: bool,
    },
    Segment {
        segment: Segment,
        role: Option<Role>,
    },
    Selected {
        choice_point: ChoicePointId,
        option: OptionId,
        branch: bool,
    },
    Branch {
        target: NodeId,
    },
    Call {
        graph: GraphRef,
        node: NodeId,
        instance: u32,
    },
    Return {
        outcome: String,
        continuation: Option<NodeId>,
    },
    Finished {
        outcome: String,
    },
}

#[derive(Clone, Debug, Eq, JsonSchema, PartialEq, Serialize)]
pub struct VisitEventV1 {
    pub commit: CommitId,
    pub graph: GraphRef,
    pub node: NodeId,
    pub instance: u32,
    pub action: VisitActionV1,
}

/// The only information about an unvisited choice target that may leave the map core.
#[derive(Clone, Debug, Eq, JsonSchema, PartialEq, Serialize)]
pub struct VisitFrontierV1 {
    pub option: OptionId,
    pub label: Option<ContentRef>,
    pub enabled: bool,
}

#[derive(Clone, Debug, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VisitMissingV1 {
    Object { id: ObjectId },
    Artifact { at: CommitId },
    Descriptor { at: CommitId },
    Limit,
}

/// Segments remain separate even if a later migration returns to the same artifact.
#[derive(Clone, Debug, Eq, JsonSchema, PartialEq, Serialize)]
pub struct ArtifactVisitsV1 {
    pub artifact_id: ArtifactId,
    pub events: Vec<VisitEventV1>,
}

#[derive(Clone, Debug, Eq, JsonSchema, PartialEq, Serialize)]
pub struct VisitSourceV1 {
    pub cursor: CommitId,
    pub complete: bool,
    pub missing: Vec<VisitMissingV1>,
    pub segments: Vec<ArtifactVisitsV1>,
    pub frontier: Vec<VisitFrontierV1>,
}

#[derive(Clone, Debug, Eq, JsonSchema, PartialEq, Serialize)]
pub struct VisitReportV1 {
    pub format_version: u16,
    pub sources: Vec<VisitSourceV1>,
}

#[derive(Debug)]
pub struct CheckedVisits {
    report: VisitReportV1,
}

impl CheckedVisits {
    pub fn report(&self) -> &VisitReportV1 {
        &self.report
    }

    /// Explicit ever-read mode keeps each cursor's evidence and incompleteness separate.
    pub fn merge(reports: impl IntoIterator<Item = CheckedVisits>) -> Result<Self> {
        let mut sources = Vec::new();
        for report in reports {
            if sources.len() + report.report.sources.len() > MAX_VISIT_CURSORS {
                return Err(Error::new(
                    "visits_limit",
                    "cursors",
                    "too many visit sources",
                ));
            }
            sources.extend(report.report.sources);
        }
        Ok(Self {
            report: VisitReportV1 {
                format_version: 1,
                sources,
            },
        })
    }
}

impl EvolutionRegistry {
    /// Derives evidence from this cursor's ancestors only. Missing history or trusted
    /// publication metadata gives an explicit incomplete report; malformed objects fail.
    pub fn visited_from_save<B: StorageBackend>(
        &self,
        history: &History<B, DomainKinds<NodeDomain>>,
        cursor: CommitId,
        limits: PathLimits,
    ) -> Result<CheckedVisits> {
        let mut source = VisitSourceV1 {
            cursor,
            complete: false,
            missing: Vec::new(),
            segments: Vec::new(),
            frontier: Vec::new(),
        };
        let mut event_count = 0;
        let outcome = self.audit_path(history, cursor, None, limits, |step| {
            let program = self.program(step.header.artifact)?;
            // Ending bodies have both a Segment observation and a Finished observation.
            let count = step
                .events
                .iter()
                .map(|event| 1 + usize::from(event.ending().is_some() && event.segment().is_some()))
                .sum::<usize>();
            if event_count + count > MAX_VISIT_EVENTS {
                return Err(Error::new(
                    "visits_limit",
                    "events",
                    "visit report event limit",
                ));
            }
            let mut events = Vec::with_capacity(count);
            for event in step.events {
                let mut push = |action| {
                    events.push(VisitEventV1 {
                        commit: step.id,
                        graph: event.location.graph.clone(),
                        node: event.location.node,
                        instance: event.location.instance,
                        action,
                    })
                };
                if event.is_visit() {
                    let graph = program.graph(&event.location.graph)?;
                    push(VisitActionV1::Visit {
                        static_node: graph.nodes.contains_key(&event.location.node),
                    });
                } else if let Some((choice_point, option, outcome)) = event.selection() {
                    push(VisitActionV1::Selected {
                        choice_point,
                        option,
                        branch: matches!(outcome, crate::plan::Outcome::Branch { .. }),
                    });
                } else if let Some(target) = event.branch_target() {
                    push(VisitActionV1::Branch { target });
                } else if let Some(target) = event.call_target() {
                    push(VisitActionV1::Call {
                        graph: target.graph.clone(),
                        node: target.node,
                        instance: target.instance,
                    });
                } else if let Some((outcome, continuation)) = event.returned() {
                    push(VisitActionV1::Return {
                        outcome: outcome.into(),
                        continuation: continuation.map(|c| c.node),
                    });
                }
                if let Some(segment) = event.segment() {
                    push(VisitActionV1::Segment {
                        segment: segment.clone(),
                        role: event.role(),
                    });
                }
                if let Some((outcome, _)) = event.ending() {
                    push(VisitActionV1::Finished {
                        outcome: outcome.into(),
                    });
                }
            }
            if step.migration || source.segments.is_empty() {
                source.segments.push(ArtifactVisitsV1 {
                    artifact_id: step.header.artifact,
                    events: Vec::new(),
                });
            }
            source
                .segments
                .last_mut()
                .ok_or_else(|| Error::new("visits", "segment", "missing audit segment"))?
                .events
                .extend(events);
            event_count += count;
            if step.id == cursor
                && let Interaction::Choose { options, .. } =
                    (Machine { program }).interaction(step.state, None)?
            {
                source.frontier = options
                    .into_iter()
                    .map(|option| VisitFrontierV1 {
                        option: option.id,
                        label: option.label,
                        enabled: option.enabled,
                    })
                    .collect();
            }
            Ok(())
        });
        match outcome {
            Ok(PathComparisonV1::Equivalent) => source.complete = true,
            Ok(PathComparisonV1::Unverified {
                reason, baseline, ..
            }) => {
                let at = baseline.unwrap_or(cursor);
                source.missing.push(match reason {
                    UnverifiedReason::MissingAncestor => VisitMissingV1::Object {
                        id: ObjectId::from_bytes(*at.as_bytes()),
                    },
                    UnverifiedReason::MissingArtifact => VisitMissingV1::Artifact { at },
                    UnverifiedReason::MissingDescriptor => VisitMissingV1::Descriptor { at },
                    _ => VisitMissingV1::Limit,
                });
            }
            Err(error) if error.code == "needs_object" => {
                source.missing.push(VisitMissingV1::Object {
                    id: error.path.parse().map_err(|_| error.clone())?,
                });
            }
            Err(error) if matches!(error.code.as_str(), "path_limit" | "visits_limit") => {
                source.missing.push(VisitMissingV1::Limit)
            }
            Err(error) => return Err(error),
            Ok(PathComparisonV1::Diverged { .. }) => {
                return Err(Error::new(
                    "visits",
                    "audit",
                    "unexpected comparison in source audit",
                ));
            }
        }
        Ok(CheckedVisits {
            report: VisitReportV1 {
                format_version: 1,
                sources: vec![source],
            },
        })
    }
}
