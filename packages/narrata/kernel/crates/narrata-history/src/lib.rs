//! Commit history for Narrata's narrative domains (ADR 0015).
//!
//! The engine stores immutable, content-addressed objects and revisioned Refs over any
//! `narrata_storage::StorageBackend`, once for every domain: writes are checked against a
//! [`Registry`] of kinds, GC keeps what Refs, Pins and registrant roots reach, and checkpoint
//! bundles move a commit's closure between stores. Domains implement [`Domain`] and use the
//! shared [`Commit`] format and [`Session`] API; the Stage 1–5 stack registers its own kinds.
//!
//! Bytes, bundle entries and backend values stay untrusted until they pass the checked
//! constructors here: an [`Object`] always hashes to its id.

#![forbid(unsafe_code)]

mod bundle;
mod commit;
mod domain;
mod engine;
mod error;
mod ids;
pub mod layout;
mod names;
mod object;
mod registry;
mod session;
#[cfg(feature = "testing")]
pub mod testing;

pub use bundle::{
    BundleError, BundleLimits, CHECKPOINT_MAGIC, CHECKPOINT_MANIFEST_KIND,
    CHECKPOINT_MANIFEST_SCHEMA, CheckpointBundle, CheckpointManifest, ContainerError,
    ManifestError, ObjectSource, check_transmitted, closure, decode_container, decode_descriptors,
    decode_list, descriptors, encode_container, encode_descriptors, gather, registered,
    stored_closure, strictly_sorted, validate_descriptors,
};
pub use commit::{COMMIT_SCHEMA, Commit, DomainKinds};
pub use domain::Domain;
pub use engine::{
    Attempt, GcKindReport, GcReport, History, IntegrityIssue, Op, Ops, Reader, RefMutation,
    RetentionPolicy, Tag, Transaction, View, Written, graph_bump, retry, sweep_check,
};
pub use error::{HistoryError, Nondeterminism, RefConflict};
pub use ids::{ArtifactId, BranchId, ObjectId, object_id};
pub use names::{
    NameError, Page, Pin, RefKey, RefName, RefNamespace, RefRevision, RefScope, RefValue, scan_all,
};
pub use object::{Descriptor, Object, ObjectError};
pub use registry::{KindInfo, Reference, Registry, Root};
pub use session::{AdvanceError, Advanced, AuditError, Imported, Loaded, Session};

/// Functions a registry that stores a domain's sessions alongside other kinds composes.
pub mod domain_kinds {
    pub use crate::commit::{kind, references, unindex, validate};
}
