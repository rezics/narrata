use narrata_core::{
    CommitId, ExecutionId, ObjectId, TimelineCatalogEventId,
    codec::{CborReader, DecodeError, ObjectKind},
};
use narrata_history::{
    CheckpointManifest, Descriptor, decode_descriptors, decode_list, encode_descriptors,
    layout::expect_array, strictly_sorted,
};
use thiserror::Error;

use crate::{
    ArchivedBookmark, ArchivedBranchRef, ArchivedSaveRef, BranchId, CheckedObject,
    ObjectDescriptor, TimelineCoverage,
    catalog::{decode_coverage, encode_coverage},
    codec::{Reader, WireError, Writer, expect_map, key},
    object::{history_id, id},
};

pub const CHECKPOINT_MANIFEST_SCHEMA_V1: u16 = narrata_history::CHECKPOINT_MANIFEST_SCHEMA;
pub const TIMELINE_ARCHIVE_MANIFEST_SCHEMA_V1: u16 = 1;

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ManifestError {
    #[error(transparent)]
    Wire(#[from] WireError),
    #[error("object descriptors must be sorted, unique, and match the declared count")]
    Descriptors,
    #[error("archive refs must be sorted and unique")]
    Refs,
    #[error("active cursor references an undeclared branch")]
    ActiveBranch,
}

impl From<narrata_history::ManifestError> for ManifestError {
    fn from(value: narrata_history::ManifestError) -> Self {
        match value {
            narrata_history::ManifestError::Wire(error) => Self::Wire(error.into()),
            narrata_history::ManifestError::Descriptors => Self::Descriptors,
        }
    }
}

/// The history layer's checkpoint manifest with Stage 1–5 identities and kinds.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckpointBundleManifestV1 {
    pub root: CommitId,
    pub objects: Vec<ObjectDescriptor>,
    pub optional_host_manifest: Option<ObjectId>,
}

impl CheckpointBundleManifestV1 {
    pub fn validate(&self) -> Result<(), ManifestError> {
        validate_descriptors(&self.objects)
    }

    pub fn to_object(&self) -> Result<CheckedObject, ManifestError> {
        self.validate()?;
        Ok(CheckedObject::new(
            ObjectKind::CheckpointBundleManifest,
            CHECKPOINT_MANIFEST_SCHEMA_V1,
            &self.encode(),
        ))
    }

    pub fn encode(&self) -> Vec<u8> {
        self.manifest().encode()
    }

    pub fn decode(payload: &[u8]) -> Result<Self, ManifestError> {
        Self::from_manifest(CheckpointManifest::decode(payload)?)
    }

    pub(crate) fn manifest(&self) -> CheckpointManifest {
        CheckpointManifest {
            root: history_id(self.root.as_bytes()),
            objects: descriptors(&self.objects),
            host_manifest: self
                .optional_host_manifest
                .map(|manifest| history_id(manifest.as_bytes())),
        }
    }

    /// Narrows a history manifest to the Stage 1–5 kinds.
    pub(crate) fn from_manifest(manifest: CheckpointManifest) -> Result<Self, ManifestError> {
        Ok(Self {
            root: CommitId::from_bytes(*manifest.root.as_bytes()),
            objects: narrow(manifest.objects)?,
            optional_host_manifest: manifest.host_manifest.map(id),
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ArchivedSessionView {
    pub selected_branch: BranchId,
    pub cursor: CommitId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimelineArchiveManifestV1 {
    pub execution: ExecutionId,
    pub coverage: TimelineCoverage,
    pub branch_heads: Vec<ArchivedBranchRef>,
    pub save_refs: Vec<ArchivedSaveRef>,
    pub bookmarks: Vec<ArchivedBookmark>,
    pub catalog_head: TimelineCatalogEventId,
    pub active: Option<ArchivedSessionView>,
    pub objects: Vec<ObjectDescriptor>,
    pub host_timeline: Option<ObjectId>,
}

impl TimelineArchiveManifestV1 {
    pub fn validate(&self) -> Result<(), ManifestError> {
        validate_descriptors(&self.objects)?;
        if !strictly_sorted(&self.branch_heads)
            || !strictly_sorted(&self.save_refs)
            || !strictly_sorted(&self.bookmarks)
        {
            return Err(ManifestError::Refs);
        }
        if self.active.is_some_and(|active| {
            !self
                .branch_heads
                .iter()
                .any(|branch| branch.branch == active.selected_branch)
        }) {
            return Err(ManifestError::ActiveBranch);
        }
        Ok(())
    }

    pub fn to_object(&self) -> Result<CheckedObject, ManifestError> {
        self.validate()?;
        Ok(CheckedObject::new(
            ObjectKind::TimelineArchiveManifest,
            TIMELINE_ARCHIVE_MANIFEST_SCHEMA_V1,
            &self.encode(),
        ))
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut writer = Writer::new();
        writer.map(10);
        writer.unsigned(0);
        writer.unsigned(u64::from(TIMELINE_ARCHIVE_MANIFEST_SCHEMA_V1));
        writer.unsigned(1);
        writer.bytes(self.execution.as_bytes());
        writer.unsigned(2);
        encode_coverage(&mut writer, self.coverage);
        writer.unsigned(3);
        writer.array(self.branch_heads.len() as u64);
        for value in &self.branch_heads {
            writer.array(2);
            writer.bytes(value.branch.as_bytes());
            writer.bytes(value.head.as_bytes());
        }
        writer.unsigned(4);
        writer.array(self.save_refs.len() as u64);
        for value in &self.save_refs {
            writer.array(2);
            writer.text(value.name.as_str());
            writer.bytes(value.commit.as_bytes());
        }
        writer.unsigned(5);
        writer.array(self.bookmarks.len() as u64);
        for value in &self.bookmarks {
            writer.array(2);
            writer.text(value.name.as_str());
            writer.bytes(value.commit.as_bytes());
        }
        writer.unsigned(6);
        writer.bytes(self.catalog_head.as_bytes());
        writer.unsigned(7);
        match self.active {
            Some(active) => {
                writer.array(2);
                writer.bytes(active.selected_branch.as_bytes());
                writer.bytes(active.cursor.as_bytes());
            }
            None => writer.null(),
        }
        writer.unsigned(8);
        encode_descriptors(&mut writer, &descriptors(&self.objects));
        writer.unsigned(9);
        match self.host_timeline {
            Some(id) => writer.bytes(id.as_bytes()),
            None => writer.null(),
        }
        writer.into_bytes()
    }

    pub fn decode(payload: &[u8]) -> Result<Self, ManifestError> {
        let mut reader = Reader::new(payload);
        expect_map(&mut reader, 10)?;
        key(&mut reader, 0)?;
        if reader.unsigned()? != u64::from(TIMELINE_ARCHIVE_MANIFEST_SCHEMA_V1) {
            return Err(WireError::Schema("Timeline manifest schema").into());
        }
        key(&mut reader, 1)?;
        let execution = ExecutionId::from_bytes(reader.bytes_exact::<16>()?);
        key(&mut reader, 2)?;
        let coverage = decode_coverage(&mut reader)?;
        key(&mut reader, 3)?;
        let branch_heads = decode_list(reader.kernel(), |reader| {
            expect_array(reader, 2)?;
            Ok(ArchivedBranchRef {
                branch: BranchId::from_bytes(reader.bytes_exact::<16>()?),
                head: CommitId::from_bytes(reader.bytes_exact::<32>()?),
            })
        })
        .map_err(WireError::from)?;
        key(&mut reader, 4)?;
        let save_refs = decode_list(reader.kernel(), |reader| {
            expect_array(reader, 2)?;
            Ok(ArchivedSaveRef {
                name: archive_name(reader, "save name")?,
                commit: CommitId::from_bytes(reader.bytes_exact::<32>()?),
            })
        })
        .map_err(WireError::from)?;
        key(&mut reader, 5)?;
        let bookmarks = decode_list(reader.kernel(), |reader| {
            expect_array(reader, 2)?;
            Ok(ArchivedBookmark {
                name: archive_name(reader, "bookmark name")?,
                commit: CommitId::from_bytes(reader.bytes_exact::<32>()?),
            })
        })
        .map_err(WireError::from)?;
        key(&mut reader, 6)?;
        let catalog_head = TimelineCatalogEventId::from_bytes(reader.bytes_exact::<32>()?);
        key(&mut reader, 7)?;
        let active = reader.optional(|reader| {
            crate::codec::expect_array(reader, 2)?;
            Ok(ArchivedSessionView {
                selected_branch: BranchId::from_bytes(reader.bytes_exact::<16>()?),
                cursor: CommitId::from_bytes(reader.bytes_exact::<32>()?),
            })
        })?;
        key(&mut reader, 8)?;
        let objects = narrow(decode_descriptors(reader.kernel()).map_err(WireError::from)?)?;
        key(&mut reader, 9)?;
        let host_timeline =
            reader.optional(|reader| Ok(ObjectId::from_bytes(reader.bytes_exact::<32>()?)))?;
        reader.finish()?;
        let value = Self {
            execution,
            coverage,
            branch_heads,
            save_refs,
            bookmarks,
            catalog_head,
            active,
            objects,
            host_timeline,
        };
        value.validate()?;
        if value.encode() != payload {
            return Err(WireError::NonCanonical.into());
        }
        Ok(value)
    }
}

fn archive_name(
    reader: &mut CborReader<'_>,
    what: &'static str,
) -> Result<crate::ArchiveRefName, DecodeError> {
    crate::ArchiveRefName::new(reader.text(128)?.to_owned()).map_err(|_| DecodeError::Schema(what))
}

fn descriptors(values: &[ObjectDescriptor]) -> Vec<Descriptor> {
    values.iter().map(ObjectDescriptor::descriptor).collect()
}

/// Descriptors of the Stage 1–5 kinds only.
fn narrow(values: Vec<Descriptor>) -> Result<Vec<ObjectDescriptor>, ManifestError> {
    values
        .into_iter()
        .map(|descriptor| {
            ObjectDescriptor::from_descriptor(descriptor).ok_or(ManifestError::Wire(
                WireError::Schema("ObjectDescriptor kind"),
            ))
        })
        .collect()
}

fn validate_descriptors(values: &[ObjectDescriptor]) -> Result<(), ManifestError> {
    if strictly_sorted(values) {
        Ok(())
    } else {
        Err(ManifestError::Descriptors)
    }
}
