//! Filesystem-free authoring (ADR 0022). Checked records carry local proof; a checked
//! record set additionally proves ownership and assembly. Compilation establishes execution
//! semantics. None of these operations resolves or stores body content.

#![forbid(unsafe_code)]

pub mod compose;
pub mod evolution;
pub mod graph;
pub mod ids;
pub mod objects;
pub mod publication;
pub mod records;
pub mod relations;
pub mod validate;

pub use compose::{
    ComposeMode, ComposedObjects, ComposedRecords, ComposedSource, compose_records, compose_source,
};
pub use evolution::{CheckedEvolution, EvolutionPlanningV1, PlannedEvolution, plan_evolution};
pub use ids::{MintedEdit, Minter, mint_ids};
pub use publication::{
    CheckedPublication, PublicationReport, PublicationStatus, PublishedRecords, publish_records,
};
pub use records::{
    CheckedRecord, CheckedRecords, DraftIndexes, DraftRecord, MAX_RECORD_BYTES, assemble_records,
    derive_draft_indexes, split_source,
};
pub use relations::{CheckedRelations, project_relations};
pub use validate::{
    Diagnostic, Report, Severity, ValidationBudget, validate_project, validate_record,
    validate_records,
};

pub fn schemas() -> Vec<(&'static str, schemars::Schema)> {
    vec![
        ("draft-record.schema.json", records::schema()),
        (
            "publication.schema.json",
            schemars::schema_for!(publication::PublicationDescription),
        ),
        (
            "publication-diagnostics.schema.json",
            schemars::schema_for!(validate::DiagnosticReport<'static>),
        ),
        (
            "relation-index.schema.json",
            schemars::schema_for!(relations::RelationIndex),
        ),
        (
            "relation-part.schema.json",
            schemars::schema_for!(relations::RelationPart),
        ),
    ]
}
