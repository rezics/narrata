use std::collections::BTreeSet;

use narrata_core::{
    CommitId, ExecutionId, TimelineArchiveManifestId, TimelineCatalogEventId, codec::ObjectKind,
};
use thiserror::Error;

use crate::{
    ArchiveRefName, BranchId, CheckedObject, TimelineOperationId,
    codec::{Reader, WireError, Writer, expect_array, expect_map, key},
};

pub const CATALOG_EVENT_SCHEMA_V1: u16 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimelineCoverage {
    FromBaseline {
        baseline: CommitId,
    },
    Imported {
        baseline: CommitId,
        source: TimelineArchiveManifestId,
    },
}

impl TimelineCoverage {
    pub const fn baseline(self) -> CommitId {
        match self {
            Self::FromBaseline { baseline } | Self::Imported { baseline, .. } => baseline,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TimelineRecordingMode {
    Standard,
    Complete { coverage: TimelineCoverage },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecordingStopMode {
    Seal,
    Delete,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ArchivedBranchRef {
    pub branch: BranchId,
    pub head: CommitId,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ArchivedSaveRef {
    pub name: ArchiveRefName,
    pub commit: CommitId,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ArchivedBookmark {
    pub name: ArchiveRefName,
    pub commit: CommitId,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ArchivedRefSnapshot {
    Branch(ArchivedBranchRef),
    Save(ArchivedSaveRef),
    Bookmark(ArchivedBookmark),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TimelineCatalogEventKind {
    RecordingStarted {
        baseline: CommitId,
        initial_refs: Vec<ArchivedRefSnapshot>,
    },
    SaveCreated {
        name: ArchiveRefName,
        commit: CommitId,
    },
    SaveUpdated {
        name: ArchiveRefName,
        previous_commit: CommitId,
        next_commit: CommitId,
    },
    SaveRenamed {
        from: ArchiveRefName,
        to: ArchiveRefName,
    },
    SaveDeleted {
        name: ArchiveRefName,
        previous_commit: CommitId,
    },
    BookmarkCreated {
        name: ArchiveRefName,
        commit: CommitId,
    },
    BookmarkRenamed {
        from: ArchiveRefName,
        to: ArchiveRefName,
    },
    BookmarkDeleted {
        name: ArchiveRefName,
        previous_commit: CommitId,
    },
    BranchCreated {
        branch: BranchId,
        parent: CommitId,
        head: CommitId,
    },
    BranchAdvanced {
        branch: BranchId,
        previous_head: CommitId,
        next_head: CommitId,
    },
    BranchDeleted {
        branch: BranchId,
        previous_head: CommitId,
    },
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum CatalogError {
    #[error(transparent)]
    Wire(#[from] WireError),
    #[error("catalog initial refs must be sorted and unique")]
    InitialRefs,
    #[error("catalog operation cannot rename a ref to itself")]
    RenameToSelf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimelineCatalogEventV1 {
    pub execution: ExecutionId,
    pub previous: Option<TimelineCatalogEventId>,
    pub operation: TimelineOperationId,
    pub kind: TimelineCatalogEventKind,
}

impl TimelineCatalogEventV1 {
    pub fn validate(&self) -> Result<(), CatalogError> {
        match &self.kind {
            TimelineCatalogEventKind::RecordingStarted { initial_refs, .. } => {
                let sorted = initial_refs.windows(2).all(|pair| pair[0] < pair[1]);
                let unique =
                    initial_refs.iter().collect::<BTreeSet<_>>().len() == initial_refs.len();
                if !sorted || !unique {
                    return Err(CatalogError::InitialRefs);
                }
                if self.previous.is_some() {
                    return Err(CatalogError::Wire(WireError::Schema(
                        "RecordingStarted has no previous event",
                    )));
                }
            }
            TimelineCatalogEventKind::SaveRenamed { from, to }
            | TimelineCatalogEventKind::BookmarkRenamed { from, to }
                if from == to =>
            {
                return Err(CatalogError::RenameToSelf);
            }
            _ => {}
        }
        Ok(())
    }

    pub fn to_object(&self) -> Result<CheckedObject, CatalogError> {
        self.validate()?;
        Ok(CheckedObject::new(
            ObjectKind::TimelineCatalogEvent,
            CATALOG_EVENT_SCHEMA_V1,
            &self.encode(),
        ))
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut writer = Writer::new();
        writer.map(5);
        writer.unsigned(0);
        writer.unsigned(u64::from(CATALOG_EVENT_SCHEMA_V1));
        writer.unsigned(1);
        writer.bytes(self.execution.as_bytes());
        writer.unsigned(2);
        match self.previous {
            Some(previous) => writer.bytes(previous.as_bytes()),
            None => writer.null(),
        }
        writer.unsigned(3);
        writer.bytes(self.operation.as_bytes());
        writer.unsigned(4);
        encode_kind(&mut writer, &self.kind);
        writer.into_bytes()
    }

    pub fn decode(payload: &[u8]) -> Result<Self, CatalogError> {
        let mut reader = Reader::new(payload);
        expect_map(&mut reader, 5)?;
        key(&mut reader, 0)?;
        if reader.unsigned()? != u64::from(CATALOG_EVENT_SCHEMA_V1) {
            return Err(WireError::Schema("Catalog schema").into());
        }
        key(&mut reader, 1)?;
        let execution = ExecutionId::from_bytes(reader.bytes_exact::<16>()?);
        key(&mut reader, 2)?;
        let previous = reader.optional(|reader| {
            Ok(TimelineCatalogEventId::from_bytes(
                reader.bytes_exact::<32>()?,
            ))
        })?;
        key(&mut reader, 3)?;
        let operation = TimelineOperationId::from_bytes(reader.bytes_exact::<16>()?);
        key(&mut reader, 4)?;
        let kind = decode_kind(&mut reader)?;
        reader.finish()?;
        let value = Self {
            execution,
            previous,
            operation,
            kind,
        };
        value.validate()?;
        if value.encode() != payload {
            return Err(WireError::NonCanonical.into());
        }
        Ok(value)
    }

    pub fn referenced_commits(&self) -> Vec<CommitId> {
        let mut commits = Vec::new();
        match &self.kind {
            TimelineCatalogEventKind::RecordingStarted {
                baseline,
                initial_refs,
            } => {
                commits.push(*baseline);
                for item in initial_refs {
                    commits.push(match item {
                        ArchivedRefSnapshot::Branch(value) => value.head,
                        ArchivedRefSnapshot::Save(value) => value.commit,
                        ArchivedRefSnapshot::Bookmark(value) => value.commit,
                    });
                }
            }
            TimelineCatalogEventKind::SaveCreated { commit, .. }
            | TimelineCatalogEventKind::BookmarkCreated { commit, .. } => commits.push(*commit),
            TimelineCatalogEventKind::SaveUpdated {
                previous_commit,
                next_commit,
                ..
            }
            | TimelineCatalogEventKind::BranchAdvanced {
                previous_head: previous_commit,
                next_head: next_commit,
                ..
            } => commits.extend([*previous_commit, *next_commit]),
            TimelineCatalogEventKind::SaveDeleted {
                previous_commit, ..
            }
            | TimelineCatalogEventKind::BookmarkDeleted {
                previous_commit, ..
            }
            | TimelineCatalogEventKind::BranchDeleted {
                previous_head: previous_commit,
                ..
            } => commits.push(*previous_commit),
            TimelineCatalogEventKind::BranchCreated { parent, head, .. } => {
                commits.extend([*parent, *head]);
            }
            TimelineCatalogEventKind::SaveRenamed { .. }
            | TimelineCatalogEventKind::BookmarkRenamed { .. } => {}
        }
        commits.sort_unstable();
        commits.dedup();
        commits
    }
}

fn encode_name(writer: &mut Writer, name: &ArchiveRefName) {
    writer.text(name.as_str());
}

fn decode_name(reader: &mut Reader<'_>) -> Result<ArchiveRefName, WireError> {
    ArchiveRefName::new(reader.text(128)?.to_owned())
        .map_err(|_| WireError::Schema("archive ref name"))
}

fn encode_initial(writer: &mut Writer, value: &ArchivedRefSnapshot) {
    writer.array(3);
    match value {
        ArchivedRefSnapshot::Branch(value) => {
            writer.unsigned(0);
            writer.bytes(value.branch.as_bytes());
            writer.bytes(value.head.as_bytes());
        }
        ArchivedRefSnapshot::Save(value) => {
            writer.unsigned(1);
            encode_name(writer, &value.name);
            writer.bytes(value.commit.as_bytes());
        }
        ArchivedRefSnapshot::Bookmark(value) => {
            writer.unsigned(2);
            encode_name(writer, &value.name);
            writer.bytes(value.commit.as_bytes());
        }
    }
}

fn decode_initial(reader: &mut Reader<'_>) -> Result<ArchivedRefSnapshot, WireError> {
    expect_array(reader, 3)?;
    match reader.unsigned()? {
        0 => Ok(ArchivedRefSnapshot::Branch(ArchivedBranchRef {
            branch: BranchId::from_bytes(reader.bytes_exact::<16>()?),
            head: CommitId::from_bytes(reader.bytes_exact::<32>()?),
        })),
        1 => Ok(ArchivedRefSnapshot::Save(ArchivedSaveRef {
            name: decode_name(reader)?,
            commit: CommitId::from_bytes(reader.bytes_exact::<32>()?),
        })),
        2 => Ok(ArchivedRefSnapshot::Bookmark(ArchivedBookmark {
            name: decode_name(reader)?,
            commit: CommitId::from_bytes(reader.bytes_exact::<32>()?),
        })),
        _ => Err(WireError::Schema("archived ref kind")),
    }
}

fn encode_kind(writer: &mut Writer, kind: &TimelineCatalogEventKind) {
    match kind {
        TimelineCatalogEventKind::RecordingStarted {
            baseline,
            initial_refs,
        } => {
            writer.array(3);
            writer.unsigned(0);
            writer.bytes(baseline.as_bytes());
            writer.array(initial_refs.len() as u64);
            for value in initial_refs {
                encode_initial(writer, value);
            }
        }
        TimelineCatalogEventKind::SaveCreated { name, commit } => {
            writer.array(3);
            writer.unsigned(1);
            encode_name(writer, name);
            writer.bytes(commit.as_bytes());
        }
        TimelineCatalogEventKind::SaveUpdated {
            name,
            previous_commit,
            next_commit,
        } => {
            writer.array(4);
            writer.unsigned(2);
            encode_name(writer, name);
            writer.bytes(previous_commit.as_bytes());
            writer.bytes(next_commit.as_bytes());
        }
        TimelineCatalogEventKind::SaveRenamed { from, to } => {
            writer.array(3);
            writer.unsigned(3);
            encode_name(writer, from);
            encode_name(writer, to);
        }
        TimelineCatalogEventKind::SaveDeleted {
            name,
            previous_commit,
        } => {
            writer.array(3);
            writer.unsigned(4);
            encode_name(writer, name);
            writer.bytes(previous_commit.as_bytes());
        }
        TimelineCatalogEventKind::BookmarkCreated { name, commit } => {
            writer.array(3);
            writer.unsigned(5);
            encode_name(writer, name);
            writer.bytes(commit.as_bytes());
        }
        TimelineCatalogEventKind::BookmarkRenamed { from, to } => {
            writer.array(3);
            writer.unsigned(6);
            encode_name(writer, from);
            encode_name(writer, to);
        }
        TimelineCatalogEventKind::BookmarkDeleted {
            name,
            previous_commit,
        } => {
            writer.array(3);
            writer.unsigned(7);
            encode_name(writer, name);
            writer.bytes(previous_commit.as_bytes());
        }
        TimelineCatalogEventKind::BranchCreated {
            branch,
            parent,
            head,
        } => {
            writer.array(4);
            writer.unsigned(8);
            writer.bytes(branch.as_bytes());
            writer.bytes(parent.as_bytes());
            writer.bytes(head.as_bytes());
        }
        TimelineCatalogEventKind::BranchAdvanced {
            branch,
            previous_head,
            next_head,
        } => {
            writer.array(4);
            writer.unsigned(9);
            writer.bytes(branch.as_bytes());
            writer.bytes(previous_head.as_bytes());
            writer.bytes(next_head.as_bytes());
        }
        TimelineCatalogEventKind::BranchDeleted {
            branch,
            previous_head,
        } => {
            writer.array(3);
            writer.unsigned(10);
            writer.bytes(branch.as_bytes());
            writer.bytes(previous_head.as_bytes());
        }
    }
}

fn decode_kind(reader: &mut Reader<'_>) -> Result<TimelineCatalogEventKind, WireError> {
    let length = reader.array()?;
    match (reader.unsigned()?, length) {
        (0, 3) => {
            let baseline = CommitId::from_bytes(reader.bytes_exact::<32>()?);
            let len = reader.array()?;
            if len > 100_000 {
                return Err(WireError::Limit("initial refs"));
            }
            let mut initial_refs =
                Vec::with_capacity(usize::try_from(len).map_err(|_| WireError::LengthOverflow)?);
            for _ in 0..len {
                initial_refs.push(decode_initial(reader)?);
            }
            Ok(TimelineCatalogEventKind::RecordingStarted {
                baseline,
                initial_refs,
            })
        }
        (1, 3) => Ok(TimelineCatalogEventKind::SaveCreated {
            name: decode_name(reader)?,
            commit: CommitId::from_bytes(reader.bytes_exact::<32>()?),
        }),
        (2, 4) => Ok(TimelineCatalogEventKind::SaveUpdated {
            name: decode_name(reader)?,
            previous_commit: CommitId::from_bytes(reader.bytes_exact::<32>()?),
            next_commit: CommitId::from_bytes(reader.bytes_exact::<32>()?),
        }),
        (3, 3) => Ok(TimelineCatalogEventKind::SaveRenamed {
            from: decode_name(reader)?,
            to: decode_name(reader)?,
        }),
        (4, 3) => Ok(TimelineCatalogEventKind::SaveDeleted {
            name: decode_name(reader)?,
            previous_commit: CommitId::from_bytes(reader.bytes_exact::<32>()?),
        }),
        (5, 3) => Ok(TimelineCatalogEventKind::BookmarkCreated {
            name: decode_name(reader)?,
            commit: CommitId::from_bytes(reader.bytes_exact::<32>()?),
        }),
        (6, 3) => Ok(TimelineCatalogEventKind::BookmarkRenamed {
            from: decode_name(reader)?,
            to: decode_name(reader)?,
        }),
        (7, 3) => Ok(TimelineCatalogEventKind::BookmarkDeleted {
            name: decode_name(reader)?,
            previous_commit: CommitId::from_bytes(reader.bytes_exact::<32>()?),
        }),
        (8, 4) => Ok(TimelineCatalogEventKind::BranchCreated {
            branch: BranchId::from_bytes(reader.bytes_exact::<16>()?),
            parent: CommitId::from_bytes(reader.bytes_exact::<32>()?),
            head: CommitId::from_bytes(reader.bytes_exact::<32>()?),
        }),
        (9, 4) => Ok(TimelineCatalogEventKind::BranchAdvanced {
            branch: BranchId::from_bytes(reader.bytes_exact::<16>()?),
            previous_head: CommitId::from_bytes(reader.bytes_exact::<32>()?),
            next_head: CommitId::from_bytes(reader.bytes_exact::<32>()?),
        }),
        (10, 3) => Ok(TimelineCatalogEventKind::BranchDeleted {
            branch: BranchId::from_bytes(reader.bytes_exact::<16>()?),
            previous_head: CommitId::from_bytes(reader.bytes_exact::<32>()?),
        }),
        _ => Err(WireError::Schema("Catalog event kind")),
    }
}

pub(crate) fn encode_coverage(writer: &mut Writer, coverage: TimelineCoverage) {
    match coverage {
        TimelineCoverage::FromBaseline { baseline } => {
            writer.array(2);
            writer.unsigned(0);
            writer.bytes(baseline.as_bytes());
        }
        TimelineCoverage::Imported { baseline, source } => {
            writer.array(3);
            writer.unsigned(1);
            writer.bytes(baseline.as_bytes());
            writer.bytes(source.as_bytes());
        }
    }
}

pub(crate) fn decode_coverage(reader: &mut Reader<'_>) -> Result<TimelineCoverage, WireError> {
    let len = reader.array()?;
    match (reader.unsigned()?, len) {
        (0, 2) => Ok(TimelineCoverage::FromBaseline {
            baseline: CommitId::from_bytes(reader.bytes_exact::<32>()?),
        }),
        (1, 3) => Ok(TimelineCoverage::Imported {
            baseline: CommitId::from_bytes(reader.bytes_exact::<32>()?),
            source: TimelineArchiveManifestId::from_bytes(reader.bytes_exact::<32>()?),
        }),
        _ => Err(WireError::Schema("timeline coverage")),
    }
}
