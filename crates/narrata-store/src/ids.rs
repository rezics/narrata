use std::{fmt, str::FromStr};

use narrata_core::ExecutionId;
use thiserror::Error;

macro_rules! fixed_id {
    ($name:ident, $prefix:literal) => {
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name([u8; 16]);

        impl $name {
            pub const fn from_bytes(bytes: [u8; 16]) -> Self {
                Self(bytes)
            }

            pub const fn from_u128(value: u128) -> Self {
                Self(value.to_be_bytes())
            }

            pub const fn as_bytes(&self) -> &[u8; 16] {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, concat!($prefix, "{}"), hex::encode(self.0))
            }
        }

        impl FromStr for $name {
            type Err = NameError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                let hex = value
                    .strip_prefix($prefix)
                    .ok_or(NameError::InvalidIdentity)?;
                let bytes = hex::decode(hex).map_err(|_| NameError::InvalidIdentity)?;
                let bytes: [u8; 16] = bytes.try_into().map_err(|_| NameError::InvalidIdentity)?;
                Ok(Self(bytes))
            }
        }
    };
}

fixed_id!(BranchId, "branch:");
fixed_id!(TimelineOperationId, "timeline-op:");

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum NameError {
    #[error("name must be 1..=128 ASCII letters, digits, '.', '_' or '-'")]
    InvalidName,
    #[error("invalid fixed identity")]
    InvalidIdentity,
}

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

pub type ArchiveRefName = RefName;

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

    pub fn branch(timeline: ExecutionId, branch: BranchId) -> Result<Self, NameError> {
        Ok(Self::new(
            RefNamespace::Branch,
            RefName::new(hex::encode(timeline.as_bytes()))?,
            RefName::new(hex::encode(branch.as_bytes()))?,
        ))
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

    pub const fn namespace(&self) -> RefNamespace {
        self.namespace
    }

    pub fn owner(&self) -> &RefName {
        &self.owner
    }

    pub fn name(&self) -> &RefName {
        &self.name
    }

    pub fn storage_key(&self) -> String {
        format!("{}/{}/{}", self.namespace.as_str(), self.owner, self.name)
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CatalogRefKey {
    timeline: ExecutionId,
}

impl CatalogRefKey {
    pub const fn new(timeline: ExecutionId) -> Self {
        Self { timeline }
    }

    pub const fn timeline(&self) -> ExecutionId {
        self.timeline
    }

    pub fn storage_key(&self) -> String {
        format!(
            "catalogs/{}/complete",
            hex::encode(self.timeline.as_bytes())
        )
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TimelineArchiveRefKey {
    timeline: ExecutionId,
    name: RefName,
}

impl TimelineArchiveRefKey {
    pub const fn new(timeline: ExecutionId, name: RefName) -> Self {
        Self { timeline, name }
    }

    pub const fn timeline(&self) -> ExecutionId {
        self.timeline
    }

    pub fn name(&self) -> &RefName {
        &self.name
    }

    pub fn storage_key(&self) -> String {
        format!(
            "archives/{}/{}",
            hex::encode(self.timeline.as_bytes()),
            self.name
        )
    }
}
