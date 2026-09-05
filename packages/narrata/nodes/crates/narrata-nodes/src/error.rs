use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Diagnostic {
    pub code: String,
    pub path: String,
    pub message: String,
}

#[derive(Clone, Debug, thiserror::Error, Eq, PartialEq)]
#[error("{code} at {path}: {message}")]
pub struct Error {
    pub code: String,
    pub path: String,
    pub message: String,
}

impl Error {
    pub fn new(code: &str, path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            path: path.into(),
            message: message.into(),
        }
    }

    pub fn diagnostic(&self) -> Diagnostic {
        Diagnostic {
            code: self.code.clone(),
            path: self.path.clone(),
            message: self.message.clone(),
        }
    }
}
