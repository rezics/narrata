use std::{fmt, num::NonZeroU64};

/// Names an immutable object. The caller computes it; backends neither compute nor trust it,
/// and the engine re-verifies the bytes it reads back.
#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ObjectDigest([u8; 32]);

impl ObjectDigest {
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Display for ObjectDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0
            .iter()
            .try_for_each(|byte| write!(formatter, "{byte:02x}"))
    }
}

impl fmt::Debug for ObjectDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "ObjectDigest({self})")
    }
}

/// Separates independent key ranges. Which space holds what belongs to the engine's layout.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct KeySpace(u16);

impl KeySpace {
    pub const fn new(value: u16) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u16 {
        self.0
    }
}

/// Store-wide version stamped on every key a batch puts.
///
/// Each batch that puts at least one key receives a revision greater than every revision the
/// store issued before, so a key deleted and recreated never repeats an old revision. Revisions
/// are strictly increasing but need not be consecutive.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Revision(NonZeroU64);

impl Revision {
    pub const fn new(value: u64) -> Option<Self> {
        match NonZeroU64::new(value) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeyValue {
    pub value: Vec<u8>,
    pub revision: Revision,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeyEntry {
    pub key: Vec<u8>,
    pub value: Vec<u8>,
    pub revision: Revision,
}

/// One page of an ordered key scan.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct KeyPage {
    /// Entries in ascending key order.
    pub entries: Vec<KeyEntry>,
    /// Whether more entries match after the last one; a page with more pending is full.
    pub more: bool,
}

impl KeyPage {
    /// Cursor for the next page, if there is one.
    pub fn resume_after(&self) -> Option<&[u8]> {
        if self.more {
            self.entries.last().map(|entry| entry.key.as_slice())
        } else {
            None
        }
    }
}

/// One page of the object digest scan used by GC sweeps and integrity checks.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ObjectPage {
    /// Digests in ascending order.
    pub digests: Vec<ObjectDigest>,
    /// Whether more digests follow the last one; a page with more pending is full.
    pub more: bool,
}

impl ObjectPage {
    /// Cursor for the next page, if there is one.
    pub fn resume_after(&self) -> Option<&ObjectDigest> {
        if self.more { self.digests.last() } else { None }
    }
}
