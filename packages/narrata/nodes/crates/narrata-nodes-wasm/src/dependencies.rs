//! The version 1 exchange of the read-only VN dependency projection (ADR 0025).
//!
//! The DTOs mirror the native [`DependencyProjection`] without adding facts to it. Requests and
//! results are decoded here, by the one authority both the Wasm entry and the tests use: a value
//! that passes [`DependencyRequestV1::decode`] or [`DependencyResultV1::decode`] has a supported
//! version, limits inside the native ranges and, for a result, internally consistent data.

use narrata_kernel::content::ContentRef;
use narrata_nodes::{
    ArtifactId, CommitId, ContentDependency, ContentRole, DependencyLimits, DependencyProjection,
    Diagnostic, GraphDependency, MediaDependency, MediaRole, ObjectId, Result, Truncation, Unknown,
    parse_json_limited,
    plan::{GraphRef, MediaRef},
};
#[cfg(test)]
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

use narrata_nodes::NodeId;

/// The only wire version this module reads and writes.
pub const VERSION: u8 = 1;
const MAX_REQUEST_BYTES: usize = 4 * 1024;
#[cfg(not(target_arch = "wasm32"))]
const MAX_RESULT_BYTES: usize = 8 * 1024 * 1024;

fn version<'de, D: Deserializer<'de>>(deserializer: D) -> std::result::Result<u8, D::Error> {
    let value = u8::deserialize(deserializer)?;
    if value == VERSION {
        Ok(value)
    } else {
        Err(serde::de::Error::custom(
            "unsupported VN dependency exchange version",
        ))
    }
}

/// The five native bounds of one projection, as exchanged.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(test, derive(JsonSchema))]
#[cfg_attr(test, schemars(rename = "DependencyLimitsV1"))]
pub struct DependencyLimitsV1 {
    #[cfg_attr(test, schemars(range(min = 1, max = DependencyLimits::MAX_LOCATIONS)))]
    locations: u32,
    #[cfg_attr(test, schemars(range(min = 0, max = DependencyLimits::MAX_GRAPHS)))]
    graphs: u32,
    #[cfg_attr(test, schemars(range(min = 1, max = DependencyLimits::MAX_CANDIDATES)))]
    candidates: u32,
    #[cfg_attr(test, schemars(range(min = 1, max = DependencyLimits::MAX_REFERENCES)))]
    references: u32,
    #[cfg_attr(test, schemars(range(min = 1, max = DependencyLimits::MAX_SCANS)))]
    scans: u32,
}

impl DependencyLimitsV1 {
    /// The native constructor is the range authority; this never restates the ranges.
    fn native(self) -> Result<DependencyLimits> {
        DependencyLimits::new(
            self.locations,
            self.graphs,
            self.candidates,
            self.references,
            self.scans,
        )
    }
}

impl From<DependencyLimits> for DependencyLimitsV1 {
    fn from(limits: DependencyLimits) -> Self {
        Self {
            locations: limits.locations(),
            graphs: limits.graphs(),
            candidates: limits.candidates(),
            references: limits.references(),
            scans: limits.scans(),
        }
    }
}

/// A projection request: the commit the caller believes is current, and the bounds to spend.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(test, derive(JsonSchema))]
#[cfg_attr(test, schemars(rename = "DependencyRequestV1"))]
pub struct DependencyRequestV1 {
    #[cfg_attr(test, schemars(range(min = 1, max = 1)))]
    #[serde(deserialize_with = "version")]
    version: u8,
    pub cursor: CommitId,
    pub limits: DependencyLimitsV1,
}

impl DependencyRequestV1 {
    pub fn new(cursor: CommitId, limits: DependencyLimits) -> Self {
        Self {
            version: VERSION,
            cursor,
            limits: limits.into(),
        }
    }

    /// The checked request: the expected cursor and the native limits.
    pub fn decode(text: &str) -> Result<(CommitId, DependencyLimits)> {
        let request: Self = parse_json_limited(text, MAX_REQUEST_BYTES)?;
        Ok((request.cursor, request.limits.native()?))
    }
}

/// What a content reference is for.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[cfg_attr(not(target_arch = "wasm32"), derive(Deserialize))]
#[serde(rename_all = "snake_case")]
#[cfg_attr(test, derive(JsonSchema))]
#[cfg_attr(test, schemars(rename = "DependencyContentRoleV1"))]
pub enum ContentRoleV1 {
    DialogueLine,
    DialogueSpeaker,
    PassageTitle,
    PassageBody,
    ChoiceLabel,
    ChoiceReason,
    ChoiceReply,
    EndingTitle,
    EndingBody,
}

impl From<ContentRole> for ContentRoleV1 {
    fn from(role: ContentRole) -> Self {
        match role {
            ContentRole::DialogueLine => Self::DialogueLine,
            ContentRole::DialogueSpeaker => Self::DialogueSpeaker,
            ContentRole::PassageTitle => Self::PassageTitle,
            ContentRole::PassageBody => Self::PassageBody,
            ContentRole::ChoiceLabel => Self::ChoiceLabel,
            ContentRole::ChoiceReason => Self::ChoiceReason,
            ContentRole::ChoiceReply => Self::ChoiceReply,
            ContentRole::EndingTitle => Self::EndingTitle,
            ContentRole::EndingBody => Self::EndingBody,
        }
    }
}

/// How a media reference would be used by the scene node it came from.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[cfg_attr(not(target_arch = "wasm32"), derive(Deserialize))]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[cfg_attr(test, derive(JsonSchema))]
#[cfg_attr(test, schemars(rename = "DependencyMediaRoleV1"))]
pub enum MediaRoleV1 {
    Background,
    CharacterLayer { id: String },
    MusicPlay,
    MusicKeep,
    Voice,
}

impl From<MediaRole> for MediaRoleV1 {
    fn from(role: MediaRole) -> Self {
        match role {
            MediaRole::Background => Self::Background,
            MediaRole::CharacterLayer { id } => Self::CharacterLayer { id },
            MediaRole::MusicPlay => Self::MusicPlay,
            MediaRole::MusicKeep => Self::MusicKeep,
            MediaRole::Voice => Self::Voice,
        }
    }
}

/// A graph a continuation may enter and the program chunk that holds it.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[cfg_attr(not(target_arch = "wasm32"), derive(Deserialize))]
#[serde(deny_unknown_fields)]
#[cfg_attr(test, derive(JsonSchema))]
#[cfg_attr(test, schemars(rename = "DependencyGraphV1"))]
pub struct GraphObjectV1 {
    pub graph: GraphRef,
    pub chunk: ObjectId,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[cfg_attr(not(target_arch = "wasm32"), derive(Deserialize))]
#[serde(deny_unknown_fields)]
#[cfg_attr(test, derive(JsonSchema))]
#[cfg_attr(test, schemars(rename = "DependencyContentV1"))]
pub struct ContentItemV1 {
    pub role: ContentRoleV1,
    pub reference: ContentRef,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[cfg_attr(not(target_arch = "wasm32"), derive(Deserialize))]
#[serde(deny_unknown_fields)]
#[cfg_attr(test, derive(JsonSchema))]
#[cfg_attr(test, schemars(rename = "DependencyMediaV1"))]
pub struct MediaItemV1 {
    pub role: MediaRoleV1,
    pub reference: MediaRef,
}

/// Why the result may be missing references. Each is a fact about this query: an unavailable
/// object may exist and become loadable later.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[cfg_attr(not(target_arch = "wasm32"), derive(Deserialize))]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[cfg_attr(test, derive(JsonSchema))]
#[cfg_attr(test, schemars(rename = "DependencyTruncationV1"))]
pub enum TruncationV1 {
    Locations {
        limit: u32,
    },
    Scans {
        limit: u32,
    },
    Candidates {
        limit: u32,
    },
    References {
        limit: u32,
    },
    Graphs {
        graph: GraphRef,
        limit: u32,
    },
    CallDepth {
        graph: GraphRef,
        node: NodeId,
    },
    AutomaticCycle {
        graph: GraphRef,
        node: NodeId,
    },
    /// The chunk could not be loaded or failed its checked decode; `error` is the source's own.
    Unavailable {
        graph: GraphRef,
        chunk: ObjectId,
        error: Diagnostic,
    },
    Unresolved {
        graph: GraphRef,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        node: Option<NodeId>,
        error: Diagnostic,
    },
}

impl From<Truncation> for TruncationV1 {
    fn from(truncation: Truncation) -> Self {
        match truncation {
            Truncation::Locations { limit } => Self::Locations { limit },
            Truncation::Scans { limit } => Self::Scans { limit },
            Truncation::Candidates { limit } => Self::Candidates { limit },
            Truncation::References { limit } => Self::References { limit },
            Truncation::Graphs { graph, limit } => Self::Graphs { graph, limit },
            Truncation::CallDepth { graph, node } => Self::CallDepth { graph, node },
            Truncation::AutomaticCycle { graph, node } => Self::AutomaticCycle { graph, node },
            Truncation::Unavailable {
                graph,
                chunk,
                error,
            } => Self::Unavailable {
                graph,
                chunk,
                error: error.diagnostic(),
            },
            Truncation::Unresolved { graph, node, error } => Self::Unresolved {
                graph,
                node,
                error: error.diagnostic(),
            },
        }
    }
}

/// What a result still did not evaluate. These never hide references.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[cfg_attr(not(target_arch = "wasm32"), derive(Deserialize))]
#[serde(rename_all = "snake_case")]
#[cfg_attr(test, derive(JsonSchema))]
#[cfg_attr(test, schemars(rename = "DependencyUnknownV1"))]
pub enum UnknownV1 {
    Conditions,
    Assignments,
    CallArguments,
    HostProposals,
}

impl From<Unknown> for UnknownV1 {
    fn from(unknown: Unknown) -> Self {
        match unknown {
            Unknown::Conditions => Self::Conditions,
            Unknown::Assignments => Self::Assignments,
            Unknown::CallArguments => Self::CallArguments,
            Unknown::HostProposals => Self::HostProposals,
        }
    }
}

/// The work the query did, for hosts that budget or test it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[cfg_attr(not(target_arch = "wasm32"), derive(Deserialize))]
#[serde(deny_unknown_fields)]
#[cfg_attr(test, derive(JsonSchema))]
#[cfg_attr(test, schemars(rename = "DependencyWorkV1"))]
pub struct WorkV1 {
    pub locations: u32,
    pub scans: u32,
    pub graphs: u32,
    pub candidates: u32,
}

/// Potential additional dependencies of the next safe point, tied to the exact artifact and
/// commit they were computed from. A conservative union, not a prediction: the current View's
/// scene resources stay with the caller, and nothing here is bytes, a URL or a credential.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[cfg_attr(not(target_arch = "wasm32"), derive(Deserialize))]
#[serde(deny_unknown_fields)]
#[cfg_attr(test, derive(JsonSchema))]
#[cfg_attr(test, schemars(rename = "DependencyResultV1"))]
pub struct DependencyResultV1 {
    #[cfg_attr(test, schemars(range(min = 1, max = 1)))]
    #[serde(deserialize_with = "version")]
    version: u8,
    pub artifact_id: ArtifactId,
    pub cursor: CommitId,
    pub limits: DependencyLimitsV1,
    /// The bounded enumeration reached every safe point and ending: `truncations` is empty.
    pub complete: bool,
    /// Graphs of the current frames are not listed. Sorted, without repeats.
    pub graphs: Vec<GraphObjectV1>,
    pub content: Vec<ContentItemV1>,
    pub media: Vec<MediaItemV1>,
    /// Some continuation stops the music before its safe point. Stopping has no reference.
    pub music_may_stop: bool,
    /// In discovery order.
    pub truncations: Vec<TruncationV1>,
    /// Sorted, without repeats.
    pub unknowns: Vec<UnknownV1>,
    pub work: WorkV1,
}

#[cfg(not(target_arch = "wasm32"))]
fn inconsistent(path: &str, message: &str) -> narrata_nodes::Error {
    narrata_nodes::Error::new("dependencies", path, message)
}

#[cfg(not(target_arch = "wasm32"))]
fn strictly_increasing<T: Ord>(items: &[T]) -> bool {
    items.windows(2).all(|pair| pair[0] < pair[1])
}

impl DependencyResultV1 {
    pub fn new(projection: DependencyProjection, limits: DependencyLimits) -> Self {
        Self {
            version: VERSION,
            artifact_id: projection.artifact_id,
            cursor: projection.cursor,
            limits: limits.into(),
            complete: projection.is_complete(),
            graphs: projection
                .graphs
                .into_iter()
                .map(|GraphDependency { graph, chunk }| GraphObjectV1 { graph, chunk })
                .collect(),
            content: projection
                .content
                .into_iter()
                .map(|ContentDependency { role, reference }| ContentItemV1 {
                    role: role.into(),
                    reference,
                })
                .collect(),
            media: projection
                .media
                .into_iter()
                .map(|MediaDependency { role, reference }| MediaItemV1 {
                    role: role.into(),
                    reference,
                })
                .collect(),
            music_may_stop: projection.music_may_stop,
            truncations: projection.truncations.into_iter().map(Into::into).collect(),
            unknowns: projection.unknowns.into_iter().map(Into::into).collect(),
            work: WorkV1 {
                locations: projection.work.locations,
                scans: projection.work.scans,
                graphs: projection.work.graphs,
                candidates: projection.work.candidates,
            },
        }
    }
}

/// The Wasm module only emits results; reading one back, with the consistency checks that
/// make a result trustworthy, is for Rust consumers and the tests, so the reader build carries
/// no decoder for it.
#[cfg(not(target_arch = "wasm32"))]
impl DependencyResultV1 {
    /// The checked result. A value whose parts contradict each other is rejected, not trusted.
    pub fn decode(text: &str) -> Result<Self> {
        let result: Self = parse_json_limited(text, MAX_RESULT_BYTES)?;
        result.check()?;
        Ok(result)
    }

    fn check(&self) -> Result<()> {
        let limits = self.limits.native()?;
        if self.complete != self.truncations.is_empty() {
            return Err(inconsistent(
                "complete",
                "complete must hold exactly when there are no truncations",
            ));
        }
        if !strictly_increasing(&self.graphs)
            || !strictly_increasing(&self.content)
            || !strictly_increasing(&self.media)
            || !strictly_increasing(&self.unknowns)
        {
            return Err(inconsistent(
                "order",
                "references and unknowns must be sorted without repeats",
            ));
        }
        let distinct = self.graphs.len() + self.content.len() + self.media.len();
        if distinct > limits.references() as usize {
            return Err(inconsistent(
                "references",
                "more distinct references than the limit allows",
            ));
        }
        let work = self.work;
        if work.locations > limits.locations()
            || work.scans > limits.scans()
            || work.graphs > limits.graphs()
            || work.candidates > limits.candidates()
            || distinct > work.candidates as usize
        {
            return Err(inconsistent("work", "work exceeds the limits it ran under"));
        }
        let mut seen = std::collections::BTreeSet::new();
        for (index, truncation) in self.truncations.iter().enumerate() {
            let path = format!("truncations[{index}]");
            let bound = match truncation {
                TruncationV1::Locations { limit } => Some((*limit, limits.locations())),
                TruncationV1::Scans { limit } => Some((*limit, limits.scans())),
                TruncationV1::Candidates { limit } => Some((*limit, limits.candidates())),
                TruncationV1::References { limit } => Some((*limit, limits.references())),
                TruncationV1::Graphs { limit, .. } => Some((*limit, limits.graphs())),
                _ => None,
            };
            if bound.is_some_and(|(reported, requested)| reported != requested) {
                return Err(inconsistent(
                    &path,
                    "a truncation must report the limit the query ran under",
                ));
            }
            let key = serde_json::to_string(truncation)
                .map_err(|error| inconsistent(&path, &error.to_string()))?;
            if !seen.insert(key) {
                return Err(inconsistent(&path, "truncations must not repeat"));
            }
        }
        Ok(())
    }
}

/// The schema document: both halves of the exchange, so one file describes the whole boundary.
#[cfg(test)]
#[derive(JsonSchema)]
#[schemars(rename = "VnDependenciesV1")]
#[allow(dead_code)]
pub struct DependencyExchangeV1 {
    request: DependencyRequestV1,
    result: DependencyResultV1,
}
