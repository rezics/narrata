//! Typed authoring nodes and explicit subgraph composition, independent of the legacy Flow VM.
//!
//! Source documents are untrusted. Only [`compile`] constructs a [`CheckedProduct`].
//! Node type extensions lower to the checked execution vocabulary; they cannot bypass validation.

#![forbid(unsafe_code)]

mod compile;
mod error;
mod expr;
mod json;
mod model;
mod registry;
mod runtime;

pub use compile::{
    CheckedProduct, Compilation, EdgeAnalysis, GraphAnalysis, NodeAddress, NodeAnalysis, compile,
};
pub use error::{Diagnostic, Error, Result};
pub use json::parse_json;
pub use model::*;
pub use registry::{NodeCompiler, NodeRegistry};
pub use runtime::{
    ActionView, FrameView, HistoryView, SaveArchive, SavedCommit, Session, SessionView,
    VariableView,
};

/// This is an alpha authoring/transport format, separate from the legacy CBOR Program format.
pub const FORMAT_VERSION: u16 = 1;
pub const MAX_DOCUMENT_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_NODES: usize = 4096;
pub const MAX_TEXT_BYTES: usize = 64 * 1024;

pub fn bundle_schema() -> schemars::Schema {
    schemars::schema_for!(Bundle)
}

pub fn view_schema() -> schemars::Schema {
    schemars::schema_for!(SessionView)
}
