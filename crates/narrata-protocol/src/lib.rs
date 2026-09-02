//! Versioned, size-bounded Protobuf protocol for all non-Rust bindings.
//!
//! DTOs remain untrusted until `ProtocolEngine` parses identities, canonical Values, Programs,
//! Snapshots, and bundles through their checked constructors. Protobuf bytes are transport-only
//! and never enter Narrata content hashes.

#![forbid(unsafe_code)]

pub mod dto;
mod engine;

pub use engine::{PROTOCOL_ABI_VERSION, ProtocolBoundaryError, ProtocolEngine, ProtocolLimits};
