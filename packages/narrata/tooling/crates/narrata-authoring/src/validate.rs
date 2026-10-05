//! Version 1 host diagnostics. Message text is explanatory; codes and identity locations
//! are the machine interface. Failed prerequisites are reported, never silently omitted.

use narrata_kernel::content::{AnchorId, ContentRef};
use narrata_nodes::{ChoicePointId, Error, NodeId, NodeRegistry, OptionId, Result};
use serde::{Deserialize, Serialize};

use crate::records::{CheckedRecord, CheckedRecords, DraftRecord, MAX_RECORD_BYTES};

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Location {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record: Option<String>,
    pub pointer: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node: Option<NodeId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub choice_point: Option<ChoicePointId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub option: Option<OptionId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<ContentRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor: Option<AnchorId>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Diagnostic {
    pub severity: Severity,
    pub code: String,
    #[serde(flatten)]
    pub location: Location,
    pub related: Vec<Location>,
    pub message: String,
}

#[derive(Clone, Debug)]
pub struct Report<T> {
    pub complete: bool,
    pub diagnostics: Vec<Diagnostic>,
    pub value: Option<T>,
}

#[derive(Clone, Debug, Serialize)]
pub struct DiagnosticReport<'a> {
    pub complete: bool,
    pub diagnostics: &'a [Diagnostic],
}

impl<T> Report<T> {
    pub fn diagnostic_report(&self) -> DiagnosticReport<'_> {
        DiagnosticReport {
            complete: self.complete,
            diagnostics: &self.diagnostics,
        }
    }

    pub fn into_result(self) -> Result<T> {
        self.value.ok_or_else(|| {
            self.diagnostics
                .iter()
                .find(|diagnostic| diagnostic.severity == Severity::Error)
                .map(|diagnostic| {
                    Error::new(
                        &diagnostic.code,
                        diagnostic
                            .location
                            .source_path
                            .clone()
                            .unwrap_or_else(|| diagnostic.location.pointer.clone()),
                        &diagnostic.message,
                    )
                })
                .unwrap_or_else(|| Error::new("limit", "$", "validation did not complete"))
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ValidationBudget {
    pub max_records: usize,
    pub max_input_bytes: usize,
    pub max_checks: usize,
    pub max_diagnostics: usize,
}

impl Default for ValidationBudget {
    fn default() -> Self {
        Self {
            max_records: 1_000_000,
            max_input_bytes: 256 * 1024 * 1024,
            max_checks: 4_000_000,
            max_diagnostics: 100_000,
        }
    }
}

pub(crate) struct Collector {
    pub diagnostics: Vec<Diagnostic>,
    pub complete: bool,
    remaining: usize,
    work: usize,
}

impl Collector {
    pub fn new(max_diagnostics: usize) -> Self {
        Self {
            diagnostics: Vec::new(),
            complete: true,
            remaining: max_diagnostics,
            work: usize::MAX,
        }
    }
    pub fn set_work(&mut self, work: usize) {
        self.work = work;
    }
    pub fn remaining_work(&self) -> usize {
        self.work
    }
    pub fn step(&mut self) -> bool {
        if !self.complete {
            return false;
        }
        if self.work == 0 {
            self.limit("assembly work budget exhausted");
            return false;
        }
        self.work -= 1;
        true
    }

    pub fn error(&mut self, error: Error, record: Option<&DraftRecord>) {
        let location = record
            .map(|record| record.location(&error.path))
            .unwrap_or_else(|| Location {
                source_path: Some(error.path.clone()),
                pointer: source_pointer(&error.path),
                ..Location::default()
            });
        self.push(Diagnostic {
            severity: Severity::Error,
            code: error.code,
            location,
            related: Vec::new(),
            message: error.message,
        });
    }

    pub fn push(&mut self, diagnostic: Diagnostic) {
        if self.remaining == 0 {
            self.limit("diagnostic budget exhausted");
        } else if self.complete {
            self.remaining -= 1;
            self.diagnostics.push(diagnostic);
        }
    }

    pub fn limit(&mut self, message: &str) {
        if self.complete {
            self.complete = false;
            self.diagnostics.push(Diagnostic {
                severity: Severity::Error,
                code: "limit".into(),
                location: Location::default(),
                related: Vec::new(),
                message: message.into(),
            });
        }
    }

    pub fn finish<T>(mut self, value: Option<T>) -> Report<T> {
        self.diagnostics.sort_by(|a, b| {
            (
                &a.location.record,
                &a.location.pointer,
                &a.code,
                &a.location,
                &a.related,
                &a.message,
            )
                .cmp(&(
                    &b.location.record,
                    &b.location.pointer,
                    &b.code,
                    &b.location,
                    &b.related,
                    &b.message,
                ))
        });
        self.diagnostics.dedup();
        let success = self.complete
            && !self
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.severity == Severity::Error);
        Report {
            complete: self.complete,
            diagnostics: self.diagnostics,
            value: if success { value } else { None },
        }
    }
}

pub(crate) fn escape(value: &str) -> String {
    value.replace('~', "~0").replace('/', "~1")
}

pub(crate) fn source_pointer(path: &str) -> String {
    if path == "$" {
        return String::new();
    }
    if path.starts_with('/') {
        return path.into();
    }
    path.replace('[', ".")
        .replace(']', "")
        .split('.')
        .filter(|part| !part.is_empty())
        .map(|part| format!("/{}", escape(part)))
        .collect()
}

/// JSON field visits and shared semantic rule evaluations consume the work budget. Parsing
/// has its separate byte limit; counting stops before processing any over-budget subtree.
pub(crate) fn value_work(value: &serde_json::Value, limit: usize) -> Option<usize> {
    let mut pending = vec![value];
    let mut count = 0;
    while let Some(value) = pending.pop() {
        if count == limit {
            return None;
        }
        count += 1;
        match value {
            serde_json::Value::Array(items) => pending.extend(items),
            serde_json::Value::Object(fields) => pending.extend(fields.values()),
            _ => {}
        }
    }
    Some(count)
}

pub fn validate_record(bytes: &[u8]) -> Report<CheckedRecord> {
    validate_record_with_budget(bytes, ValidationBudget::default())
}

pub fn validate_record_with_budget(
    bytes: &[u8],
    budget: ValidationBudget,
) -> Report<CheckedRecord> {
    let mut out = Collector::new(budget.max_diagnostics);
    if bytes.len() > MAX_RECORD_BYTES {
        out.error(
            Error::new("record_too_large", "$", "draft record exceeds 1 MiB"),
            None,
        );
        return out.finish(None);
    }
    if bytes.len() > budget.max_input_bytes || budget.max_checks == 0 {
        out.limit("record input or work budget exhausted");
        return out.finish(None);
    }
    let text = match std::str::from_utf8(bytes) {
        Ok(text) => text,
        Err(error) => {
            out.error(Error::new("json", "$", error.to_string()), None);
            return out.finish(None);
        }
    };
    let raw: serde_json::Value = match narrata_nodes::parse_json_limited(text, MAX_RECORD_BYTES) {
        Ok(record) => record,
        Err(error) => {
            out.error(error, None);
            return out.finish(None);
        }
    };
    if value_work(&raw, budget.max_checks).is_none() {
        out.limit("record work budget exhausted");
        return out.finish(None);
    }
    let record: DraftRecord = match serde_json::from_value(raw) {
        Ok(record) => record,
        Err(error) => {
            out.error(Error::new("schema", "$", error.to_string()), None);
            return out.finish(None);
        }
    };
    record.check(&mut out);
    let value = CheckedRecord::from_checked(record);
    match value {
        Ok(value) => {
            if value.work() > budget.max_checks {
                out.limit("record work budget exhausted");
            }
            out.finish(Some(value))
        }
        Err(error) => {
            out.error(error, None);
            out.finish(None)
        }
    }
}

#[derive(Debug)]
pub struct ValidatedProject {
    pub records: CheckedRecords,
    pub compilation: narrata_nodes::Compilation,
}

/// Parses every independent record, assembles the valid subset for diagnostics and shares
/// semantic rules with the compiler. Invalid dependencies are explicitly marked skipped.
pub fn validate_project(records: &[&[u8]]) -> Report<ValidatedProject> {
    validate_project_with_budget(
        records,
        &NodeRegistry::gamebook(),
        ValidationBudget::default(),
    )
}

pub fn validate_project_with_budget(
    records: &[&[u8]],
    registry: &NodeRegistry,
    budget: ValidationBudget,
) -> Report<ValidatedProject> {
    let mut out = Collector::new(budget.max_diagnostics);
    let mut checked = Vec::new();
    let mut bytes = 0_usize;
    let mut remaining_checks = budget.max_checks;
    for (index, input) in records.iter().enumerate() {
        bytes = bytes.saturating_add(input.len());
        if index >= budget.max_records || bytes > budget.max_input_bytes || !out.complete {
            out.limit("project input budget exhausted");
            break;
        }
        let report = validate_record_with_budget(
            input,
            ValidationBudget {
                max_checks: remaining_checks,
                ..budget
            },
        );
        if !report.complete {
            out.limit("record validation budget exhausted");
        }
        for mut diagnostic in report.diagnostics {
            if diagnostic.location.record.is_none() || diagnostic.location.source_path.is_none() {
                diagnostic.location.source_path = Some(format!("records[{index}]"));
            }
            out.push(diagnostic);
        }
        if report.value.is_none() {
            out.push(Diagnostic {
                severity: Severity::Warning,
                code: "checks_skipped".into(),
                location: Location {
                    source_path: Some(format!("records[{index}]")),
                    ..Location::default()
                },
                related: Vec::new(),
                message: "checks depending on this invalid record require a valid payload".into(),
            });
        }
        if let Some(record) = report.value {
            if record.work() > remaining_checks {
                out.limit("project work budget exhausted");
                break;
            }
            remaining_checks -= record.work();
            checked.push(record);
        } else {
            remaining_checks = remaining_checks.saturating_sub(1);
        }
    }
    if !out.complete {
        return out.finish(None);
    }
    out.set_work(remaining_checks);
    let assembled = crate::records::assemble_partial(&checked, &mut out);
    if let Some(source) = assembled {
        let semantic = narrata_nodes::validate_source(
            &source,
            registry,
            out.remaining_work(),
            budget.max_diagnostics,
        );
        if !semantic.complete {
            out.limit("source validation budget exhausted");
        }
        for error in semantic.errors {
            out.error(error, None);
        }
        for skipped in semantic.skipped {
            out.push(Diagnostic {
                severity: Severity::Warning,
                code: skipped.code,
                location: Location {
                    pointer: source_pointer(&skipped.path),
                    source_path: Some(skipped.path),
                    ..Location::default()
                },
                related: Vec::new(),
                message: skipped.message,
            });
        }
        crate::records::locate_diagnostics(&checked, &mut out.diagnostics);
        if out.complete
            && !out
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.severity == Severity::Error)
        {
            match narrata_nodes::compile(&source, registry) {
                Ok(compilation) => {
                    for diagnostic in &compilation.diagnostics {
                        out.push(Diagnostic {
                            severity: Severity::Warning,
                            code: diagnostic.code.clone(),
                            location: Location {
                                pointer: source_pointer(&diagnostic.path),
                                source_path: Some(diagnostic.path.clone()),
                                ..Location::default()
                            },
                            related: Vec::new(),
                            message: diagnostic.message.clone(),
                        });
                    }
                    crate::records::locate_diagnostics(&checked, &mut out.diagnostics);
                    return out.finish(Some(ValidatedProject {
                        records: CheckedRecords::from_assembled(checked, source),
                        compilation,
                    }));
                }
                Err(error) => out.error(error, None),
            }
        }
    }
    out.finish(None)
}
