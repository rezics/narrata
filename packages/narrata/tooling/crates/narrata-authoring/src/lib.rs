//! Filesystem-free authoring (ADR 0022). Checked records carry local proof; a checked
//! record set additionally proves ownership and assembly. Compilation establishes execution
//! semantics. None of these operations resolves or stores body content.

#![forbid(unsafe_code)]

pub mod compose;
pub mod ids;
pub mod records;
pub mod validate;

pub use compose::{ComposeMode, ComposedSource, compose_source};
pub use ids::{MintedEdit, Minter, mint_ids};
pub use records::{
    CheckedRecord, CheckedRecords, DraftIndexes, DraftRecord, MAX_RECORD_BYTES, assemble_records,
    derive_draft_indexes, split_source,
};
pub use validate::{
    Diagnostic, Report, Severity, ValidationBudget, validate_project, validate_record,
};

pub fn schemas() -> Vec<(&'static str, schemars::Schema)> {
    vec![("draft-record.schema.json", records::schema())]
}
