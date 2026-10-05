//! Stage 5 persistence, migration, and debugger contracts for Narrata.
//!
//! Bytes, bundle entries, backend keys and mutable references remain untrusted until they
//! pass the checked constructors of this crate or of `narrata_history`. Immutable objects are
//! content addressed; refs are the only mutable, revisioned storage surface. [`Store`]
//! implements [`SaveStore`] once over any `narrata_storage::StorageBackend`, as the Stage 1–5
//! registrant of the kernel history layer (ADR 0015).

#![forbid(unsafe_code)]

mod bundle;
mod catalog;
mod codec;
mod commit;
mod coordinator;
mod debugger;
mod effect;
mod engine;
mod federated;
mod graph;
mod ids;
pub mod layout;
mod manifest;
mod migration;
mod object;
mod store;

pub use bundle::*;
pub use catalog::*;
pub use codec::WireError;
pub use commit::*;
pub use coordinator::*;
pub use debugger::*;
pub use effect::*;
pub use engine::{MemoryStore, Store};
pub use federated::*;
pub use ids::*;
pub use manifest::*;
pub use migration::*;
pub use object::*;
pub use store::*;
