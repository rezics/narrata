//! Stage 3 persistence and host-coordination contracts for Narrata.
//!
//! Bytes, bundle entries, database rows, and mutable references remain untrusted until they
//! pass the checked constructors in this crate. Immutable objects are content addressed;
//! refs are the only mutable, revisioned storage surface.

#![forbid(unsafe_code)]

mod bundle;
mod catalog;
mod codec;
mod commit;
mod coordinator;
mod effect;
mod federated;
mod ids;
mod manifest;
mod memory;
mod object;
mod store;

pub use bundle::*;
pub use catalog::*;
pub use codec::WireError;
pub use commit::*;
pub use coordinator::*;
pub use effect::*;
pub use federated::*;
pub use ids::*;
pub use manifest::*;
pub use memory::*;
pub use object::*;
pub use store::*;
