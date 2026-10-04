//! What the engine asks of the domains whose objects it stores (ADR 0015).
//!
//! Kinds are self-describing: every envelope carries its kind code, so the engine can refuse
//! objects no registrant knows. Everything else about a kind (what it refers to, what a write
//! must check, which index keys it adds and how its roots are stored) comes from the registry.

use std::fmt;

use narrata_storage::{Conflict, KeySpace, StorageBackend};

use crate::{HistoryError, Object, ObjectId, Op, Reader, View};

/// The engine's view of one registered kind.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KindInfo {
    /// Names the kind in diagnostics.
    pub name: &'static str,
    /// Objects of the kind never refer to other objects, so GC and the integrity scan need not
    /// read them when an edge already says what kind they are.
    pub leaf: bool,
    /// Objects of the kind are commits: Refs and checkpoint manifests may point at them.
    pub commit: bool,
}

/// An edge from one object to another.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Reference {
    Object {
        id: ObjectId,
        /// The kind the referring object requires, when its schema names one.
        kind: Option<u16>,
        /// Manifest descriptors keep their objects alive but are not part of a bundle's
        /// closure, which the manifest itself describes.
        descriptor: bool,
    },
    /// The object of `kind` that its registrant names `name`, found through the registrant's
    /// index in a store and among the carried objects in a bundle. Stage 1–5 Commits name their
    /// Program this way, by artifact.
    Named { kind: u16, name: [u8; 32] },
}

impl Reference {
    pub const fn object(id: ObjectId, kind: u16) -> Self {
        Self::Object {
            id,
            kind: Some(kind),
            descriptor: false,
        }
    }

    pub const fn untyped(id: ObjectId) -> Self {
        Self::Object {
            id,
            kind: None,
            descriptor: false,
        }
    }

    pub const fn descriptor(id: ObjectId, kind: u16) -> Self {
        Self::Object {
            id,
            kind: Some(kind),
            descriptor: true,
        }
    }
}

/// A GC root: an object a registrant's key keeps alive, with its kind when the key implies it.
pub type Root = (ObjectId, Option<u16>);

/// The kinds one engine stores, with their edges, write checks, index keys and roots.
///
/// A registry is a static type, not a set of plugins: the engine calls it directly. Kind 7, the
/// checkpoint manifest, belongs to the engine; a registry that also claims it is not asked.
pub trait Registry: Sized {
    /// Errors of the registrant's checks; history failures convert into it.
    type Error: From<HistoryError>;
    /// Says what a conflict on one of the registrant's root keys means.
    type Tag: Clone + fmt::Debug;

    fn kind(&self, code: u16) -> Option<KindInfo>;

    /// Key spaces the registrant writes; a store that holds keys in them is not empty.
    fn spaces(&self) -> &[KeySpace] {
        &[]
    }

    /// The objects `object` refers to. Called only for registered kinds; a payload that does
    /// not decode is [`HistoryError::Corrupt`].
    fn references(&self, object: &Object) -> Result<Vec<Reference>, HistoryError>;

    /// The name [`Reference::Named`] edges use for `object`, if its kind is named.
    fn name(&self, object: &Object) -> Option<[u8; 32]> {
        let _ = object;
        None
    }

    /// Finds the stored object of `kind` named `name` through the registrant's index. The index
    /// is a hint: callers that need the object check its name after reading it.
    fn resolve<B: StorageBackend>(
        &self,
        reader: &Reader<'_, B>,
        kind: u16,
        name: &[u8; 32],
    ) -> Result<Option<ObjectId>, HistoryError> {
        let _ = (reader, kind, name);
        Ok(None)
    }

    /// Checks one object a write adds, after the engine has checked that every object it refers
    /// to exists with the required kind, and returns the index keys it adds.
    fn validate<B: StorageBackend>(
        &self,
        object: &Object,
        view: &mut View<'_, B, Self>,
    ) -> Result<Vec<Op<Self::Tag>>, Self::Error>;

    /// The index keys to delete with `object` when GC removes it.
    fn unindex<B: StorageBackend>(
        &self,
        object: &Object,
        reader: &Reader<'_, B>,
    ) -> Result<Vec<Op<Self::Tag>>, HistoryError> {
        let _ = (object, reader);
        Ok(Vec::new())
    }

    /// Objects the registrant's own keys keep alive, beyond Refs and Pins.
    fn roots<B: StorageBackend>(
        &self,
        reader: &Reader<'_, B>,
        now: u64,
    ) -> Result<Vec<Root>, HistoryError> {
        let _ = (reader, now);
        Ok(Vec::new())
    }

    /// Turns a failed precondition on a key tagged `tag` into the registrant's error.
    fn conflict(&self, tag: &Self::Tag, conflict: Conflict) -> Self::Error;
}
