//! Persistent authoring identities and a round-trippable source-map sidecar.
//!
//! Relocation hints deliberately cannot be converted into a trusted migration. They are editor
//! suggestions which an author must review and register through the migration API.

use std::{collections::BTreeMap, fmt};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{SourceDocumentId, diagnostic::SourceSpan};

const SOURCE_MAP_VERSION: u16 = 1;
const MAX_SOURCE_MAP_ENTRIES: usize = 1_000_000;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StableIdKind {
    Flow,
    Instruction,
    Choice,
    Checkpoint,
    EffectSite,
    State,
    Region,
    Transition,
    History,
    Action,
    Type,
    Variant,
    Field,
    ContentOccurrence,
}

impl StableIdKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Flow => "flow",
            Self::Instruction => "instruction",
            Self::Choice => "choice",
            Self::Checkpoint => "checkpoint",
            Self::EffectSite => "effect-site",
            Self::State => "state",
            Self::Region => "region",
            Self::Transition => "transition",
            Self::History => "history",
            Self::Action => "action",
            Self::Type => "type",
            Self::Variant => "variant",
            Self::Field => "field",
            Self::ContentOccurrence => "content-occurrence",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct StableId {
    pub kind: StableIdKind,
    pub bytes: [u8; 16],
}

impl StableId {
    pub const fn new(kind: StableIdKind, bytes: [u8; 16]) -> Self {
        Self { kind, bytes }
    }
}

impl fmt::Display for StableId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}:{}",
            self.kind.label(),
            hex::encode(self.bytes)
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceLocation {
    pub document: SourceDocumentId,
    pub path: String,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceAnchor {
    pub id: StableId,
    pub location: SourceLocation,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceMapSidecar {
    entries: Vec<SourceAnchor>,
}

impl SourceMapSidecar {
    pub fn new(mut entries: Vec<SourceAnchor>) -> Result<Self, SourceMapError> {
        if entries.len() > MAX_SOURCE_MAP_ENTRIES {
            return Err(SourceMapError::Limit);
        }
        let mut seen = BTreeMap::new();
        for entry in &entries {
            validate_location(&entry.location)?;
            if let Some(first) = seen.insert(entry.id, entry.location.clone()) {
                return Err(SourceMapError::Duplicate {
                    id: entry.id,
                    first,
                    second: entry.location.clone(),
                });
            }
        }
        entries.sort_by_key(|entry| entry.id);
        Ok(Self { entries })
    }

    pub fn entries(&self) -> &[SourceAnchor] {
        &self.entries
    }

    pub fn location(&self, id: StableId) -> Option<&SourceLocation> {
        self.entries
            .binary_search_by_key(&id, |entry| entry.id)
            .ok()
            .and_then(|index| self.entries.get(index))
            .map(|entry| &entry.location)
    }

    pub fn to_json(&self) -> Result<Vec<u8>, SourceMapError> {
        let wire = SourceMapWire {
            version: SOURCE_MAP_VERSION,
            entries: self
                .entries
                .iter()
                .map(|entry| SourceAnchorWire {
                    kind: entry.id.kind,
                    id: hex::encode(entry.id.bytes),
                    document: hex::encode(entry.location.document.as_bytes()),
                    path: entry.location.path.clone(),
                    start: entry.location.span.start,
                    end: entry.location.span.end,
                })
                .collect(),
        };
        serde_json::to_vec_pretty(&wire).map_err(|error| SourceMapError::Json(error.to_string()))
    }

    pub fn from_json(bytes: &[u8]) -> Result<Self, SourceMapError> {
        let wire: SourceMapWire = serde_json::from_slice(bytes)
            .map_err(|error| SourceMapError::Json(error.to_string()))?;
        if wire.version != SOURCE_MAP_VERSION {
            return Err(SourceMapError::UnsupportedVersion(wire.version));
        }
        let entries = wire
            .entries
            .into_iter()
            .map(|entry| {
                Ok(SourceAnchor {
                    id: StableId::new(entry.kind, decode_id(&entry.id, "Stable ID")?),
                    location: SourceLocation {
                        document: SourceDocumentId::from_bytes(decode_id(
                            &entry.document,
                            "source document ID",
                        )?),
                        path: entry.path,
                        span: SourceSpan {
                            start: entry.start,
                            end: entry.end,
                        },
                    },
                })
            })
            .collect::<Result<Vec<_>, SourceMapError>>()?;
        Self::new(entries)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RelocationHintBasis {
    SameExplicitAnchor,
    SimilarStructure,
    SimilarText,
    NearbySourceSpan,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RelocationHint {
    pub from: StableId,
    pub to: StableId,
    pub basis: RelocationHintBasis,
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum SourceMapError {
    #[error("source map exceeds the configured entry limit")]
    Limit,
    #[error("unsupported source-map version {0}")]
    UnsupportedVersion(u16),
    #[error("invalid source-map JSON: {0}")]
    Json(String),
    #[error("invalid {0}")]
    InvalidId(&'static str),
    #[error("source path must be relative and non-empty")]
    InvalidPath,
    #[error("source span end precedes its start")]
    InvalidSpan,
    #[error("duplicate Stable ID {id} at {first:?} and {second:?}")]
    Duplicate {
        id: StableId,
        first: SourceLocation,
        second: SourceLocation,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceMapWire {
    version: u16,
    entries: Vec<SourceAnchorWire>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceAnchorWire {
    kind: StableIdKind,
    id: String,
    document: String,
    path: String,
    start: u32,
    end: u32,
}

fn validate_location(location: &SourceLocation) -> Result<(), SourceMapError> {
    if location.path.is_empty()
        || location.path.starts_with('/')
        || location.path.starts_with('\\')
        || location.path.contains(':')
        || location.path.split(['/', '\\']).any(|part| part == "..")
    {
        return Err(SourceMapError::InvalidPath);
    }
    if location.span.end < location.span.start {
        return Err(SourceMapError::InvalidSpan);
    }
    Ok(())
}

fn decode_id(value: &str, label: &'static str) -> Result<[u8; 16], SourceMapError> {
    let bytes = hex::decode(value).map_err(|_| SourceMapError::InvalidId(label))?;
    <[u8; 16]>::try_from(bytes).map_err(|_| SourceMapError::InvalidId(label))
}
