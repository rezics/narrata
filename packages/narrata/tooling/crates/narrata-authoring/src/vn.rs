//! Explicit first-party visual-novel record checking and same-candidate preview.
//!
//! A [`crate::records::CheckedRecord`] proves only its record structure. VN semantics are
//! established by checking the full assembled record set with source format 3 and the VN
//! registry.

use narrata_nodes::{NodeRegistry, Program, VISUAL_NOVEL_FORMAT_VERSION};

use crate::{
    compose::{ComposeMode, ComposedRecords, compose_records_with_budget},
    records::CheckedRecords,
    validate::{
        Diagnostic, Location, Report, Severity, ValidationBudget,
        validate_records_with_budget as validate_records_with_budget_inner,
    },
};

/// Check a complete record set using the explicit VN source format and registry.
pub fn validate_records(records: &[&[u8]]) -> Report<CheckedRecords> {
    validate_records_with_budget_inner(
        records,
        &NodeRegistry::visual_novel(),
        ValidationBudget::default(),
    )
}

/// Budgeted counterpart to [`validate_records`].
pub fn validate_records_with_budget(
    records: &[&[u8]],
    budget: ValidationBudget,
) -> Report<CheckedRecords> {
    validate_records_with_budget_inner(records, &NodeRegistry::visual_novel(), budget)
}

/// Validate and compose one record snapshot into a VN candidate. This operation is pure and
/// does not publish records or interact with reader persistence.
pub fn preview_records(
    records: &[&[u8]],
    previous: Option<&Program>,
    mode: ComposeMode<'_>,
) -> Report<ComposedRecords> {
    preview_records_with_budget(records, previous, mode, ValidationBudget::default())
}

/// Budgeted counterpart to [`preview_records`].
pub fn preview_records_with_budget(
    records: &[&[u8]],
    previous: Option<&Program>,
    mode: ComposeMode<'_>,
    budget: ValidationBudget,
) -> Report<ComposedRecords> {
    if previous.is_some_and(|program| program.source_format() != VISUAL_NOVEL_FORMAT_VERSION) {
        return Report {
            complete: true,
            diagnostics: vec![Diagnostic {
                severity: Severity::Error,
                code: "dialect_mismatch".into(),
                location: Location {
                    pointer: "/previous".into(),
                    source_path: Some("previous".into()),
                    ..Location::default()
                },
                related: Vec::new(),
                message: "a VN preview requires a VN continuity baseline".into(),
            }],
            value: None,
        };
    }
    compose_records_with_budget(
        records,
        previous,
        mode,
        &NodeRegistry::visual_novel(),
        budget,
    )
}
