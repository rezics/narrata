//! The R2 text-free node format (ADR 0013): typed authoring nodes, explicit subgraph
//! composition, content-addressed artifacts and portable Gamebook sessions.
//!
//! Narrata holds structure only. Titles, bodies and labels are content references that the
//! host resolves. Source documents, packs and session objects are untrusted: only [`compile`]
//! builds an artifact, and every decoded object passes checked decode and the structural
//! checks before it runs.

#![forbid(unsafe_code)]

pub mod analysis;
mod check;
mod compile;
mod error;
mod expr;
pub mod history;
mod ids;
mod json;
pub mod outline;
pub mod plan;
mod program;
mod proposal;
pub mod r1;
mod registry;
mod runtime;
mod session;
pub mod source;
mod state;
mod value;
pub mod view;
mod wire;

pub use analysis::{Analysis, analyze};
pub use compile::{Compilation, canonical_json, compile, package_digest};
pub use error::{Diagnostic, Error, Result};
pub use expr::{Assignment, BinaryOp, Expr, Scope, Variable};
pub use ids::{
    ArtifactId, AuthoredId, ChoicePointId, CommitId, ExecutionId, NodeId, ObjectId, OptionId,
};
pub use json::{parse_json, parse_json_limited};
pub use outline::ContentOutline;
pub use plan::NameTable;
pub use program::{ChunkPins, ChunkSource, Lookup, MemorySource, Owner, Program};
pub use proposal::{
    ChoicePointProposal, MAX_OVERLAY_NODES, MAX_PROPOSED_NODES, MAX_PROPOSED_OPTIONS,
    MAX_REQUEST_BYTES, NodeProposal, OptionProposal, ProposalRequest,
};
pub use registry::{GAMEBOOK_REVISION, NodeCompiler, NodeRegistry, lower_builtin};
pub use session::{
    EXPORT_FORMAT_VERSION, MAX_COMMITS, MAX_EXPORT_BYTES, MAX_RETAINED_STATE_BYTES, Session,
    SessionExport,
};
pub use source::{
    CompositionLock, PackageLock, PackageSource, ProjectManifest, ProjectSource, SourcePlan,
};
pub use state::{Commit, Finished, Frame, Input, Overlay, State, decode_input, decode_state};
pub use value::{Scalar, ScalarType, ViewScalar};
pub use view::{BookView, SessionView};
pub use wire::{
    KIND_CHUNK, KIND_COMMIT, KIND_INPUT, KIND_MANIFEST, KIND_NAMES, KIND_PACK, KIND_STATE,
    KIND_TOMBSTONES, Pack, SCHEMA,
};

/// The source and composition format revision.
pub const FORMAT_VERSION: u16 = 2;
pub const MAX_SOURCE_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_MANIFEST_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_CHUNK_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_PACK_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_STATE_BYTES: usize = 128 * 1024;
pub const MAX_GRAPH_NODES: usize = 4096;
pub const MAX_CHOICE_POINTS: usize = 64;
pub const MAX_OPTIONS: usize = 128;
pub const MAX_ARGS: usize = 256;
pub const MAX_ASSIGNMENTS: usize = 256;
pub const MAX_VARIABLES: usize = 256;
pub const MAX_SHARED: usize = 4096;
pub const MAX_STEPS: u32 = 4096;
pub const MAX_CALL_DEPTH: usize = 64;
pub const MAX_TEXT_BYTES: usize = 64 * 1024;
pub const MAX_NAME_BYTES: usize = 80;

pub(crate) fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME_BYTES
        && name
            .bytes()
            .all(|v| v.is_ascii_alphanumeric() || matches!(v, b'_' | b'-'))
}

/// JSON Schemas of the exchanged JSON forms, keyed by file name.
pub fn schemas() -> Vec<(&'static str, schemars::Schema)> {
    use schemars::schema_for;
    vec![
        ("analysis.schema.json", schema_for!(Analysis)),
        ("book-view.schema.json", schema_for!(BookView)),
        ("content-outline.schema.json", schema_for!(ContentOutline)),
        ("lock.schema.json", schema_for!(CompositionLock)),
        ("node-plan.schema.json", schema_for!(SourcePlan)),
        ("package.schema.json", schema_for!(PackageSource)),
        ("project.schema.json", schema_for!(ProjectManifest)),
        ("proposal-request.schema.json", schema_for!(ProposalRequest)),
        ("session-export.schema.json", schema_for!(SessionExport)),
        ("view.schema.json", schema_for!(SessionView)),
    ]
}
