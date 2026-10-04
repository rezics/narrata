//! Narrow storage backend contract (ADR 0012).
//!
//! A backend stores immutable objects named by caller-supplied digests and mutable keys with
//! store-wide revisions, applies atomic batches with per-key preconditions, serves bounded
//! ordered scans, and declares its capabilities. Effect ledgers, catalogs, GC, integrity checks
//! and migration are written once on top of these primitives, so the crate depends on no
//! narrative types and both engine stacks can use it.

#![forbid(unsafe_code)]

mod backend;
mod batch;
mod capabilities;
mod error;
mod memory;
#[cfg(feature = "testing")]
pub mod testing;
mod types;

pub use backend::StorageBackend;
pub use batch::{Applied, Batch, Expect, KeyAction, KeyOp};
pub use capabilities::{Capabilities, Durability, Limits, WriteModel};
pub use error::{Conflict, Limit, LimitExceeded, StorageError};
pub use memory::MemoryBackend;
pub use types::{KeyEntry, KeyPage, KeySpace, KeyValue, ObjectDigest, ObjectPage, Revision};
