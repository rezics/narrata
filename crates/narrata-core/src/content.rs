use thiserror::Error;

use crate::{CapabilityId, EntityId, ObjectId, Value};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StructureOccurrence {
    pub structure: EntityId,
    pub occurrence: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContentPermission {
    Public,
    Authenticated,
    Entitled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContentLifecycle {
    Active,
    Retired,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RevisionPolicy {
    Pinned(ObjectId),
    RecordFirstResolution,
    LivePresentationOnly,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContentQuery {
    pub provider: CapabilityId,
    pub occurrence: StructureOccurrence,
    pub permission: ContentPermission,
    pub revision: RevisionPolicy,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedContent {
    pub provider: CapabilityId,
    pub occurrence: StructureOccurrence,
    pub permission: ContentPermission,
    pub lifecycle: ContentLifecycle,
    pub revision: ObjectId,
    pub value: Value,
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ContentResolutionError {
    #[error("external content was not found")]
    NotFound,
    #[error("external content is forbidden")]
    Forbidden,
    #[error("external content is retired")]
    Retired,
    #[error("external content provider or structure occurrence is incompatible")]
    Incompatible,
    #[error("external content revision does not match the pinned revision")]
    RevisionMismatch,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CheckedContentResolution {
    Branching {
        recorded_value: Value,
        revision: ObjectId,
    },
    LivePresentationOnly {
        value: Value,
        revision: ObjectId,
    },
}

impl CheckedContentResolution {
    pub fn check(
        query: &ContentQuery,
        resolved: ResolvedContent,
    ) -> Result<Self, ContentResolutionError> {
        if resolved.provider != query.provider || resolved.occurrence != query.occurrence {
            return Err(ContentResolutionError::Incompatible);
        }
        if resolved.permission != query.permission {
            return Err(ContentResolutionError::Forbidden);
        }
        if resolved.lifecycle == ContentLifecycle::Retired {
            return Err(ContentResolutionError::Retired);
        }
        if let RevisionPolicy::Pinned(expected) = query.revision
            && resolved.revision != expected
        {
            return Err(ContentResolutionError::RevisionMismatch);
        }
        Ok(match query.revision {
            RevisionPolicy::Pinned(_) | RevisionPolicy::RecordFirstResolution => Self::Branching {
                recorded_value: resolved.value,
                revision: resolved.revision,
            },
            RevisionPolicy::LivePresentationOnly => Self::LivePresentationOnly {
                value: resolved.value,
                revision: resolved.revision,
            },
        })
    }

    pub fn branching_value(&self) -> Option<&Value> {
        match self {
            Self::Branching { recorded_value, .. } => Some(recorded_value),
            Self::LivePresentationOnly { .. } => None,
        }
    }
}

pub trait ExternalContentResolver {
    fn resolve(&self, query: &ContentQuery) -> Result<ResolvedContent, ContentResolutionError>;
}
