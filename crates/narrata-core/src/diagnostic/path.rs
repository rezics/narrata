use std::fmt;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum PathSegment {
    Field(&'static str),
    Index(usize),
    Object(String),
}

#[derive(Clone, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct DiagnosticPath(Vec<PathSegment>);

impl DiagnosticPath {
    pub fn root() -> Self {
        Self::default()
    }

    pub fn field(mut self, field: &'static str) -> Self {
        self.0.push(PathSegment::Field(field));
        self
    }

    pub fn index(mut self, index: usize) -> Self {
        self.0.push(PathSegment::Index(index));
        self
    }

    pub fn object(mut self, object: impl ToString) -> Self {
        self.0.push(PathSegment::Object(object.to_string()));
        self
    }

    pub fn segments(&self) -> &[PathSegment] {
        &self.0
    }
}

impl fmt::Display for DiagnosticPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.is_empty() {
            return formatter.write_str("$");
        }
        formatter.write_str("$")?;
        for segment in &self.0 {
            match segment {
                PathSegment::Field(field) => write!(formatter, ".{field}")?,
                PathSegment::Index(index) => write!(formatter, "[{index}]")?,
                PathSegment::Object(object) => write!(formatter, "[{object}]")?,
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct SourceSpan {
    pub start: u32,
    pub end: u32,
}
