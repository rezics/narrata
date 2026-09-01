//! Narrata Stage 1 deterministic in-memory narrative kernel.
//!
//! External bytes enter through strict wire decoders and become usable only after checked
//! construction. The core performs no I/O, clock, randomness, callbacks, or asynchronous work.

#![forbid(unsafe_code)]

pub mod codec;
pub mod diagnostic;
pub mod identity;
pub mod limits;
pub mod program;
pub mod runtime;
pub mod snapshot;
pub mod value;
pub mod version;

pub use identity::*;
pub use program::{CheckedProgram, ProgramArtifactV0, ProgramLoadError, load_program};
pub use runtime::{
    CheckedRuntimeInput, RuntimeStateV0, TransitionDraft, begin_transition, new_execution,
};
pub use snapshot::{export_snapshot, restore_snapshot};
pub use value::{Value, ValueKindV0};
