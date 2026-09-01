use std::collections::BTreeSet;

use narrata_core::{CommitId, ExecutionId, ObjectId, TimelineCatalogEventId, codec::ObjectKind};
use thiserror::Error;

use crate::{
    ArchivedBookmark, ArchivedBranchRef, ArchivedSaveRef, BranchId, CheckedObject,
    ObjectDescriptor, TimelineCoverage,
    catalog::{decode_coverage, encode_coverage},
    codec::{Reader, WireError, Writer, expect_array, expect_map, key},
};

pub const CHECKPOINT_MANIFEST_SCHEMA_V1: u16 = 1;
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
        let mut writer = Writer::new();
        writer.map(4);
        writer.unsigned(0);
        writer.unsigned(u64::from(CHECKPOINT_MANIFEST_SCHEMA_V1));
        writer.unsigned(1);
        writer.bytes(self.root.as_bytes());
        writer.unsigned(2);
        encode_descriptors(&mut writer, &self.objects);
        writer.unsigned(3);
        match self.optional_host_manifest {
            Some(id) => writer.bytes(id.as_bytes()),
            None => writer.null(),
        }
        writer.into_bytes()
    }

    pub fn decode(payload: &[u8]) -> Result<Self, ManifestError> {
        let mut reader = Reader::new(payload);
        expect_map(&mut reader, 4)?;
        key(&mut reader, 0)?;
        if reader.unsigned()? != u64::from(CHECKPOINT_MANIFEST_SCHEMA_V1) {
            return Err(WireError::Schema("Checkpoint manifest schema").into());
        }
        key(&mut reader, 1)?;
        let root = CommitId::from_bytes(reader.bytes_exact::<32>()?);
        key(&mut reader, 2)?;
        let objects = decode_descriptors(&mut reader)?;
        key(&mut reader, 3)?;
        let optional_host_manifest =
            reader.optional(|reader| Ok(ObjectId::from_bytes(reader.bytes_exact::<32>()?)))?;
        reader.finish()?;
        let value = Self {
            root,
            objects,
            optional_host_manifest,
        };
        value.validate()?;
        if value.encode() != payload {
            return Err(WireError::NonCanonical.into());
        }
        Ok(value)
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
        encode_descriptors(&mut writer, &self.objects);
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
        let branch_heads = decode_list(&mut reader, |reader| {
            expect_array(reader, 2)?;
            Ok(ArchivedBranchRef {
                branch: BranchId::from_bytes(reader.bytes_exact::<16>()?),
                head: CommitId::from_bytes(reader.bytes_exact::<32>()?),
            })
        })?;
        key(&mut reader, 4)?;
        let save_refs = decode_list(&mut reader, |reader| {
            expect_array(reader, 2)?;
            Ok(ArchivedSaveRef {
                name: crate::ArchiveRefName::new(reader.text(128)?.to_owned())
                    .map_err(|_| WireError::Schema("save name"))?,
                commit: CommitId::from_bytes(reader.bytes_exact::<32>()?),
            })
        })?;
        key(&mut reader, 5)?;
        let bookmarks = decode_list(&mut reader, |reader| {
            expect_array(reader, 2)?;
            Ok(ArchivedBookmark {
                name: crate::ArchiveRefName::new(reader.text(128)?.to_owned())
                    .map_err(|_| WireError::Schema("bookmark name"))?,
                commit: CommitId::from_bytes(reader.bytes_exact::<32>()?),
            })
        })?;
        key(&mut reader, 6)?;
        let catalog_head = TimelineCatalogEventId::from_bytes(reader.bytes_exact::<32>()?);
        key(&mut reader, 7)?;
        let active = reader.optional(|reader| {
            expect_array(reader, 2)?;
            Ok(ArchivedSessionView {
                selected_branch: BranchId::from_bytes(reader.bytes_exact::<16>()?),
                cursor: CommitId::from_bytes(reader.bytes_exact::<32>()?),
            })
        })?;
        key(&mut reader, 8)?;
        let objects = decode_descriptors(&mut reader)?;
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

fn strictly_sorted<T: Ord>(values: &[T]) -> bool {
    values.windows(2).all(|pair| pair[0] < pair[1])
        && values.iter().collect::<BTreeSet<_>>().len() == values.len()
}

fn validate_descriptors(values: &[ObjectDescriptor]) -> Result<(), ManifestError> {
    if strictly_sorted(values) {
        Ok(())
    } else {
        Err(ManifestError::Descriptors)
    }
}

fn encode_descriptors(writer: &mut Writer, descriptors: &[ObjectDescriptor]) {
    writer.array(descriptors.len() as u64);
    for descriptor in descriptors {
        writer.array(4);
        writer.bytes(descriptor.id.as_bytes());
        writer.unsigned(u64::from(descriptor.kind.code()));
        writer.unsigned(u64::from(descriptor.schema));
        writer.unsigned(descriptor.bytes);
    }
}

fn decode_descriptors(reader: &mut Reader<'_>) -> Result<Vec<ObjectDescriptor>, WireError> {
    decode_list(reader, |reader| {
        expect_array(reader, 4)?;
        let id = ObjectId::from_bytes(reader.bytes_exact::<32>()?);
        let kind_code =
            u16::try_from(reader.unsigned()?).map_err(|_| WireError::IntegerOverflow)?;
        let kind =
            ObjectKind::from_code(kind_code).ok_or(WireError::Schema("ObjectDescriptor kind"))?;
        let schema = u16::try_from(reader.unsigned()?).map_err(|_| WireError::IntegerOverflow)?;
        let bytes = reader.unsigned()?;
        Ok(ObjectDescriptor {
            id,
            kind,
            schema,
            bytes,
        })
    })
}

fn decode_list<T>(
    reader: &mut Reader<'_>,
    mut decode: impl FnMut(&mut Reader<'_>) -> Result<T, WireError>,
) -> Result<Vec<T>, WireError> {
    let length = reader.array()?;
    if length > 1_000_000 {
        return Err(WireError::Limit("manifest list"));
    }
    let mut values =
        Vec::with_capacity(usize::try_from(length).map_err(|_| WireError::LengthOverflow)?);
    for _ in 0..length {
        values.push(decode(reader)?);
    }
    Ok(values)
}
