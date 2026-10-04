use std::{fmt, str::FromStr};

use narrata_storage::Revision;
use thiserror::Error;

use crate::{BranchId, ObjectId};

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum NameError {
    #[error("name must be 1..=128 ASCII letters, digits, '.', '_' or '-'")]
    InvalidName,
}

/// A path segment of a Ref: never empty, never `.` or `..`, never containing the key
/// separator `0x00`, so `owner ‖ 0x00 ‖ name` keys sort as their tuples (ADR 0014).
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RefName(String);

impl RefName {
    pub fn new(value: impl Into<String>) -> Result<Self, NameError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 128
            || value == "."
            || value == ".."
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        {
            return Err(NameError::InvalidName);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RefName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FromStr for RefName {
    type Err = NameError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

/// The first byte of a Ref key; the codes are part of the stored layout (ADR 0014).
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
pub enum RefNamespace {
    Save = 0,
    Branch = 1,
    Bookmark = 2,
    Active = 3,
    Temporary = 4,
}

impl RefNamespace {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Save => "saves",
            Self::Branch => "branches",
            Self::Bookmark => "bookmarks",
            Self::Active => "active",
            Self::Temporary => "temporary",
        }
    }

    pub const fn from_code(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Save),
            1 => Some(Self::Branch),
            2 => Some(Self::Bookmark),
            3 => Some(Self::Active),
            4 => Some(Self::Temporary),
            _ => None,
        }
    }
}

/// A mutable, revisioned name for a commit: a save slot, a branch head, a bookmark or a
/// session's cursor.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RefKey {
    namespace: RefNamespace,
    owner: RefName,
    name: RefName,
}

impl RefKey {
    pub fn new(namespace: RefNamespace, owner: RefName, name: RefName) -> Self {
        Self {
            namespace,
            owner,
            name,
        }
    }

    pub fn save(owner: RefName, slot: RefName) -> Self {
        Self::new(RefNamespace::Save, owner, slot)
    }

    /// The head of `branch` among the branches `owner` keeps; the name is the branch's hex.
    pub fn branch(owner: RefName, branch: BranchId) -> Self {
        Self::new(RefNamespace::Branch, owner, branch_name(branch))
    }

    pub fn bookmark(owner: RefName, name: RefName) -> Self {
        Self::new(RefNamespace::Bookmark, owner, name)
    }

    pub fn active(session: RefName) -> Result<Self, NameError> {
        Ok(Self::new(
            RefNamespace::Active,
            session,
            RefName::new("cursor")?,
        ))
    }

    /// `active(session)` without the fallible name check: `cursor` is a valid name.
    pub(crate) fn cursor(session: RefName) -> Self {
        Self::new(RefNamespace::Active, session, RefName("cursor".to_owned()))
    }

    pub const fn namespace(&self) -> RefNamespace {
        self.namespace
    }

    pub fn owner(&self) -> &RefName {
        &self.owner
    }

    pub fn name(&self) -> &RefName {
        &self.name
    }

    /// The branch a [`RefKey::branch`] key names.
    pub fn branch_id(&self) -> Option<BranchId> {
        if self.namespace != RefNamespace::Branch {
            return None;
        }
        let mut bytes = [0_u8; 16];
        hex::decode_to_slice(self.name.as_str(), &mut bytes).ok()?;
        Some(BranchId::from_bytes(bytes))
    }

    pub fn storage_key(&self) -> String {
        format!("{}/{}/{}", self.namespace.as_str(), self.owner, self.name)
    }
}

fn branch_name(branch: BranchId) -> RefName {
    // Thirty-two lowercase hex digits always form a valid name.
    RefName(hex::encode(branch.as_bytes()))
}

/// Which Refs a scan visits.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RefScope {
    All,
    Namespace(RefNamespace),
    Owner(RefNamespace, RefName),
}

/// The backend revision at which a root key was last written.
///
/// Revisions are store-wide and never reused (ADR 0014); only their equality is meaningful.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct RefRevision(u64);

impl RefRevision {
    pub const fn from_u64(value: u64) -> Option<Self> {
        if value == 0 { None } else { Some(Self(value)) }
    }

    pub const fn get(self) -> u64 {
        self.0
    }

    pub const fn from_revision(revision: Revision) -> Self {
        Self(revision.get())
    }

    pub fn revision(self) -> Option<Revision> {
        Revision::new(self.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RefValue {
    pub revision: RefRevision,
    pub commit: ObjectId,
}

/// Keeps an object and its closure alive until `expires_at`, or for good.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Pin {
    pub owner: String,
    pub object: ObjectId,
    pub expires_at: Option<u64>,
}

/// One page of a scan, in key order. `more` is set when the scan stopped at its limit.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub more: bool,
}

/// Reads every page of a scan. `page` receives the key to resume after.
pub fn scan_all<T, K, E>(
    mut page: impl FnMut(Option<&K>) -> Result<Page<T>, E>,
    resume: impl Fn(&T) -> K,
) -> Result<Vec<T>, E> {
    let mut items = Vec::new();
    let mut after = None;
    loop {
        let next = page(after.as_ref())?;
        let more = next.more;
        after = next.items.last().map(&resume);
        items.extend(next.items);
        if !more || after.is_none() {
            return Ok(items);
        }
    }
}
