use super::{DiagnosticClass, DiagnosticCode, DiagnosticPath, SourceSpan};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Diagnostic {
    pub class: DiagnosticClass,
    pub code: DiagnosticCode,
    pub path: DiagnosticPath,
    pub message: String,
    pub relevant_ids: Vec<String>,
    pub source_span: Option<SourceSpan>,
}

impl Diagnostic {
    pub fn new(
        class: DiagnosticClass,
        code: DiagnosticCode,
        path: DiagnosticPath,
        message: impl Into<String>,
    ) -> Self {
        Self {
            class,
            code,
            path,
            message: message.into(),
            relevant_ids: Vec::new(),
            source_span: None,
        }
    }

    pub fn with_id(mut self, id: impl ToString) -> Self {
        self.relevant_ids.push(id.to_string());
        self
    }

    pub fn with_span(mut self, source_span: Option<SourceSpan>) -> Self {
        self.source_span = source_span;
        self
    }
}

impl std::fmt::Display for Diagnostic {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{} {:?} at {}: {}",
            self.code, self.class, self.path, self.message
        )
    }
}
