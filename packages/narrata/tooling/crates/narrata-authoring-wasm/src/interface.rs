//! JSON shapes crossing the authoring Wasm boundary. These are host API values; the stored
//! formats they carry (draft records, publication descriptions, outlines, locks) keep their
//! own ADR versions. Binary records and objects cross separately as bytes.

use narrata_authoring::{
    Diagnostic, DraftRecord, PublicationStatus,
    objects::Digest,
    publication::{ObjectReference, PublicationDescription},
    records::{OptionIndex, OwnershipIndex},
};
use narrata_nodes::{
    ArtifactId, AuthoredId, ChoicePointId, CompositionLock, ContentOutline, NodeId, ObjectId,
    OptionId, PackageSource, ProjectManifest,
};
use schemars::{JsonSchema, Schema};
use serde::Serialize;

/// A checked record set, a single checked record or a split source. `records` counts the
/// records available by index; it is zero whenever an error prevented a checked value.
#[derive(Debug, JsonSchema, Serialize)]
pub struct RecordReport {
    pub complete: bool,
    pub diagnostics: Vec<Diagnostic>,
    pub records: u32,
    /// IDs this operation minted, in allocation order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub minted: Vec<AuthoredId>,
}

/// The derived index rows of one record, replaced together with its payload.
#[derive(Debug, JsonSchema, Serialize)]
pub struct RecordIndexes {
    pub ownership: OwnershipIndex,
    pub options: Vec<OptionIndex>,
}

/// Preview compilation of records into program objects. A compiled preview is never a
/// publication: switching requires `publish` with outlines.
#[derive(Debug, JsonSchema, Serialize)]
pub struct PreviewReport {
    pub complete: bool,
    pub diagnostics: Vec<Diagnostic>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact_id: Option<ArtifactId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lock: Option<CompositionLock>,
    pub compared: bool,
    /// Tombstone records the host persists atomically with the edit before publishing.
    pub tombstones: Vec<DraftRecord>,
    /// Program chunks and the tombstone set, retrievable one at a time.
    pub objects: Vec<ObjectId>,
}

#[derive(Debug, JsonSchema, Serialize)]
pub struct PublishReport {
    pub complete: bool,
    pub diagnostics: Vec<Diagnostic>,
    /// `compiled` candidates can be retained and inspected; only `switchable` ones pass the
    /// outline policy for becoming the work's current publication.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<PublicationStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publication_id: Option<Digest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidate_artifact_id: Option<ArtifactId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_artifact_id: Option<ArtifactId>,
    pub compared: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lock: Option<CompositionLock>,
    pub tombstones: Vec<DraftRecord>,
}

/// Newly minted IDs; none of them occurred in the minter's occupied set.
#[derive(Debug, Default, JsonSchema, Serialize)]
pub struct MintedIds {
    pub nodes: Vec<NodeId>,
    pub choice_points: Vec<ChoicePointId>,
    pub options: Vec<OptionId>,
}

/// Every exchanged shape, so one generated declaration file shares its definitions.
#[derive(JsonSchema)]
pub struct AuthoringInterface {
    pub record: DraftRecord,
    pub record_report: RecordReport,
    pub record_indexes: RecordIndexes,
    pub preview_report: PreviewReport,
    pub publish_report: PublishReport,
    pub minted_ids: MintedIds,
    pub publication: PublicationDescription,
    pub object_reference: ObjectReference,
    pub outline: ContentOutline,
    pub manifest: ProjectManifest,
    pub package: PackageSource,
    pub lock: CompositionLock,
    pub authored_id: AuthoredId,
}

pub fn schema() -> Schema {
    schemars::schema_for!(AuthoringInterface)
}

/// The committed `schemas/authoring-interface.schema.json` text.
pub fn schema_text() -> serde_json::Result<String> {
    serde_json::to_string_pretty(&schema()).map(|text| text + "\n")
}
