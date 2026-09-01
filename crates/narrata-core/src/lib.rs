//! Narrata deterministic narrative kernel through Stage 3.
//!
//! External bytes enter through strict wire decoders and become usable only after checked
//! construction. The core performs no I/O, clock, randomness, callbacks, or asynchronous work.

#![forbid(unsafe_code)]

pub mod codec;
pub mod content;
pub mod diagnostic;
pub mod effect;
pub mod identity;
pub mod limits;
pub mod program;
pub mod runtime;
pub mod scene;
pub mod snapshot;
pub mod value;
pub mod version;

pub use content::*;
pub use effect::*;
pub use identity::*;
pub use program::{CheckedProgram, ProgramArtifactV0, ProgramLoadError, load_program};
pub use runtime::{
    CheckedRuntimeInput, RuntimeStateV0, TransitionDraft, begin_transition,
    begin_transition_with_parent_commit, new_execution,
};
pub use scene::*;
pub use snapshot::{export_snapshot, restore_snapshot};
pub use value::{Value, ValueKindV0};
