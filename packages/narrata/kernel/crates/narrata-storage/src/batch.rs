use std::{collections::BTreeSet, sync::Arc};

use crate::{
    Capabilities, KeySpace, Limit, ObjectDigest, Revision, StorageError, WriteModel,
    capabilities::len,
};

/// Writes that commit together or not at all (see [`WriteModel`] for backends that cannot).
///
/// Every key and every object digest appears at most once, so each precondition is evaluated
/// against the state before the batch and the outcome does not depend on operation order.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Batch {
    /// Put-if-absent: an existing digest is skipped without comparing bytes.
    pub put_objects: Vec<(ObjectDigest, Arc<[u8]>)>,
    /// Reserved for GC; deleting an absent object is not an error.
    pub delete_objects: Vec<ObjectDigest>,
    pub keys: Vec<KeyOp>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeyOp {
    pub space: KeySpace,
    pub key: Vec<u8>,
    pub expect: Expect,
    pub action: KeyAction,
}

/// Precondition on a key's revision before the batch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Expect {
    Any,
    Absent,
    Revision(Revision),
}

impl Expect {
    pub fn holds(self, actual: Option<Revision>) -> bool {
        match self {
            Self::Any => true,
            Self::Absent => actual.is_none(),
            Self::Revision(expected) => actual == Some(expected),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum KeyAction {
    Put(Vec<u8>),
    /// Deleting an absent key is not an error; use [`Expect`] to require presence.
    Delete,
    /// Only evaluates the precondition, so a decision based on a key joins the batch.
    Check,
}

/// What a successful batch did.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Applied {
    /// Revision of every key the batch put; `None` when it put no key.
    pub revision: Option<Revision>,
    pub objects_inserted: u64,
    pub objects_deleted: u64,
}

impl Batch {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.put_objects.is_empty() && self.delete_objects.is_empty() && self.keys.is_empty()
    }

    pub fn put_object(mut self, digest: ObjectDigest, bytes: impl Into<Arc<[u8]>>) -> Self {
        self.put_objects.push((digest, bytes.into()));
        self
    }

    pub fn delete_object(mut self, digest: ObjectDigest) -> Self {
        self.delete_objects.push(digest);
        self
    }

    pub fn put(
        self,
        space: KeySpace,
        key: impl Into<Vec<u8>>,
        value: impl Into<Vec<u8>>,
        expect: Expect,
    ) -> Self {
        self.key(space, key, expect, KeyAction::Put(value.into()))
    }

    pub fn delete(self, space: KeySpace, key: impl Into<Vec<u8>>, expect: Expect) -> Self {
        self.key(space, key, expect, KeyAction::Delete)
    }

    pub fn check(self, space: KeySpace, key: impl Into<Vec<u8>>, expect: Expect) -> Self {
        self.key(space, key, expect, KeyAction::Check)
    }

    fn key(
        mut self,
        space: KeySpace,
        key: impl Into<Vec<u8>>,
        expect: Expect,
        action: KeyAction,
    ) -> Self {
        self.keys.push(KeyOp {
            space,
            key: key.into(),
            expect,
            action,
        });
        self
    }

    pub fn puts_keys(&self) -> bool {
        self.keys
            .iter()
            .any(|op| matches!(op.action, KeyAction::Put(_)))
    }

    /// Checks shape and limits before a backend touches storage. Backends call this first so
    /// that rejected batches never have a partial effect.
    pub fn validate(&self, capabilities: &Capabilities) -> Result<(), StorageError> {
        let limits = capabilities.limits;
        let ops = self.put_objects.len() + self.delete_objects.len() + self.keys.len();
        limits.check(Limit::BatchOps, ops as u64)?;
        if capabilities.write_model == WriteModel::RootKeyOnly && self.keys.len() > 1 {
            return Err(StorageError::Invalid(
                "backend applies at most one key operation per batch",
            ));
        }

        let mut bytes = 0_u64;
        let mut digests = BTreeSet::new();
        for (digest, object) in &self.put_objects {
            limits.check(Limit::ValueBytes, len(object))?;
            bytes = bytes.saturating_add(len(object));
            if !digests.insert(digest) {
                return Err(StorageError::Invalid("object named twice in one batch"));
            }
        }
        for digest in &self.delete_objects {
            if !digests.insert(digest) {
                return Err(StorageError::Invalid("object named twice in one batch"));
            }
        }

        let mut keys = BTreeSet::new();
        for op in &self.keys {
            limits.check(Limit::KeyBytes, len(&op.key))?;
            bytes = bytes.saturating_add(len(&op.key));
            match &op.action {
                KeyAction::Put(value) => {
                    limits.check(Limit::ValueBytes, len(value))?;
                    bytes = bytes.saturating_add(len(value));
                }
                KeyAction::Delete => {}
                KeyAction::Check if op.expect == Expect::Any => {
                    return Err(StorageError::Invalid(
                        "check needs an Absent or Revision precondition",
                    ));
                }
                KeyAction::Check => {}
            }
            if !keys.insert((op.space, op.key.as_slice())) {
                return Err(StorageError::Invalid("key named twice in one batch"));
            }
        }
        limits.check(Limit::BatchBytes, bytes)
    }
}
