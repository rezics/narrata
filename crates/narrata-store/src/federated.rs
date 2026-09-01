use std::sync::Arc;

use narrata_core::{
    CapabilityId, CommitId, CompoundSaveManifestId, ContentLockId, ExecutionId, HostSnapshotDigest,
    HostTimelineManifestId, ProgramArtifactId, TimelineArchiveManifestId, codec::ObjectKind,
};
use thiserror::Error;

use crate::{
    CheckedObject, LedgerFence, TimelineCoverage, WireError,
    codec::{Reader, Writer, expect_array, expect_map, key},
};

pub const COMPOUND_SAVE_MANIFEST_SCHEMA_V1: u16 = 1;
pub const HOST_TIMELINE_MANIFEST_SCHEMA_V1: u16 = 1;

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum HostSnapshotRefError {
    #[error("host snapshot reference must be 1..=256 visible ASCII characters")]
    InvalidReference,
    #[error("host snapshot format version must be nonzero")]
    InvalidVersion,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostSnapshotRef {
    provider: CapabilityId,
    reference: Arc<str>,
    format_version: u16,
    digest: HostSnapshotDigest,
}

impl HostSnapshotRef {
    pub fn new(
        provider: CapabilityId,
        reference: impl AsRef<str>,
        format_version: u16,
        digest: HostSnapshotDigest,
    ) -> Result<Self, HostSnapshotRefError> {
        let reference = reference.as_ref();
        if reference.is_empty()
            || reference.len() > 256
            || !reference.bytes().all(|byte| byte.is_ascii_graphic())
        {
            return Err(HostSnapshotRefError::InvalidReference);
        }
        if format_version == 0 {
            return Err(HostSnapshotRefError::InvalidVersion);
        }
        Ok(Self {
            provider,
            reference: Arc::from(reference),
            format_version,
            digest,
        })
    }

    pub fn provider(&self) -> &CapabilityId {
        &self.provider
    }

    pub fn reference(&self) -> &str {
        &self.reference
    }

    pub const fn format_version(&self) -> u16 {
        self.format_version
    }

    pub const fn digest(&self) -> HostSnapshotDigest {
        self.digest
    }
}

pub trait HostSnapshotVerifier {
    fn verify(&self, snapshot: &HostSnapshotRef) -> Result<(), String>;
}

pub trait FederatedSaveVerifier: HostSnapshotVerifier {
    fn verify_content_lock(&self, content_lock: ContentLockId) -> Result<(), String>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompoundSaveManifestV1 {
    pub execution: ExecutionId,
    pub narrative: CommitId,
    pub host: HostSnapshotRef,
    pub program: ProgramArtifactId,
    pub content_lock: ContentLockId,
    pub ledger_fence: LedgerFence,
}

impl CompoundSaveManifestV1 {
    pub fn to_object(&self) -> CheckedObject {
        CheckedObject::new(
            ObjectKind::CompoundSaveManifest,
            COMPOUND_SAVE_MANIFEST_SCHEMA_V1,
            &self.encode(),
        )
    }

    pub fn id(&self) -> CompoundSaveManifestId {
        let object = self.to_object();
        CompoundSaveManifestId::from_bytes(*object.id().as_bytes())
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut writer = Writer::new();
        writer.map(7);
        writer.unsigned(0);
        writer.unsigned(u64::from(COMPOUND_SAVE_MANIFEST_SCHEMA_V1));
        writer.unsigned(1);
        writer.bytes(self.execution.as_bytes());
        writer.unsigned(2);
        writer.bytes(self.narrative.as_bytes());
        writer.unsigned(3);
        encode_host_snapshot(&mut writer, &self.host);
        writer.unsigned(4);
        writer.bytes(self.program.as_bytes());
        writer.unsigned(5);
        writer.bytes(self.content_lock.as_bytes());
        writer.unsigned(6);
        writer.unsigned(self.ledger_fence.get());
        writer.into_bytes()
    }

    pub fn decode(encoded: &[u8]) -> Result<Self, WireError> {
        let mut reader = Reader::new(encoded);
        expect_map(&mut reader, 7)?;
        key(&mut reader, 0)?;
        schema(&mut reader, COMPOUND_SAVE_MANIFEST_SCHEMA_V1)?;
        key(&mut reader, 1)?;
        let execution = ExecutionId::from_bytes(reader.bytes_exact::<16>()?);
        key(&mut reader, 2)?;
        let narrative = CommitId::from_bytes(reader.bytes_exact::<32>()?);
        key(&mut reader, 3)?;
        let host = decode_host_snapshot(&mut reader)?;
        key(&mut reader, 4)?;
        let program = ProgramArtifactId::from_bytes(reader.bytes_exact::<32>()?);
        key(&mut reader, 5)?;
        let content_lock = ContentLockId::from_bytes(reader.bytes_exact::<32>()?);
        key(&mut reader, 6)?;
        let ledger_fence = LedgerFence::from_u64(reader.unsigned()?);
        reader.finish()?;
        let value = Self {
            execution,
            narrative,
            host,
            program,
            content_lock,
            ledger_fence,
        };
        canonical(&value.encode(), encoded)?;
        Ok(value)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostTimelineEntry {
    pub narrative: CommitId,
    pub host: HostSnapshotRef,
    pub ledger_fence: LedgerFence,
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum HostTimelineError {
    #[error("Host Timeline entries must be unique and sorted by Narrative Commit ID")]
    EntryOrder,
    #[error("Host Timeline must include its coverage baseline")]
    MissingBaseline,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostTimelineManifestV1 {
    pub(crate) execution: ExecutionId,
    pub(crate) coverage: TimelineCoverage,
    pub(crate) entries: Vec<HostTimelineEntry>,
}

impl HostTimelineManifestV1 {
    pub fn checked(
        execution: ExecutionId,
        coverage: TimelineCoverage,
        entries: Vec<HostTimelineEntry>,
    ) -> Result<Self, HostTimelineError> {
        if entries
            .windows(2)
            .any(|pair| pair[0].narrative >= pair[1].narrative)
        {
            return Err(HostTimelineError::EntryOrder);
        }
        if !entries
            .iter()
            .any(|entry| entry.narrative == coverage.baseline())
        {
            return Err(HostTimelineError::MissingBaseline);
        }
        Ok(Self {
            execution,
            coverage,
            entries,
        })
    }

    pub fn entry(&self, narrative: CommitId) -> Option<&HostTimelineEntry> {
        self.entries
            .binary_search_by_key(&narrative, |entry| entry.narrative)
            .ok()
            .and_then(|index| self.entries.get(index))
    }

    pub const fn execution(&self) -> ExecutionId {
        self.execution
    }

    pub const fn coverage(&self) -> TimelineCoverage {
        self.coverage
    }

    pub fn entries(&self) -> &[HostTimelineEntry] {
        &self.entries
    }

    pub fn to_object(&self) -> CheckedObject {
        CheckedObject::new(
            ObjectKind::HostTimelineManifest,
            HOST_TIMELINE_MANIFEST_SCHEMA_V1,
            &self.encode(),
        )
    }

    pub fn id(&self) -> HostTimelineManifestId {
        let object = self.to_object();
        HostTimelineManifestId::from_bytes(*object.id().as_bytes())
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut writer = Writer::new();
        writer.map(4);
        writer.unsigned(0);
        writer.unsigned(u64::from(HOST_TIMELINE_MANIFEST_SCHEMA_V1));
        writer.unsigned(1);
        writer.bytes(self.execution.as_bytes());
        writer.unsigned(2);
        encode_coverage(&mut writer, self.coverage);
        writer.unsigned(3);
        writer.array(self.entries.len() as u64);
        for entry in &self.entries {
            writer.array(3);
            writer.bytes(entry.narrative.as_bytes());
            encode_host_snapshot(&mut writer, &entry.host);
            writer.unsigned(entry.ledger_fence.get());
        }
        writer.into_bytes()
    }

    pub fn decode(encoded: &[u8]) -> Result<Self, WireError> {
        let mut reader = Reader::new(encoded);
        expect_map(&mut reader, 4)?;
        key(&mut reader, 0)?;
        schema(&mut reader, HOST_TIMELINE_MANIFEST_SCHEMA_V1)?;
        key(&mut reader, 1)?;
        let execution = ExecutionId::from_bytes(reader.bytes_exact::<16>()?);
        key(&mut reader, 2)?;
        let coverage = decode_coverage(&mut reader)?;
        key(&mut reader, 3)?;
        let length = reader.array()?;
        if length > 1_000_000 {
            return Err(WireError::Limit("Host Timeline entries"));
        }
        let mut entries =
            Vec::with_capacity(usize::try_from(length).map_err(|_| WireError::LengthOverflow)?);
        for _ in 0..length {
            expect_array(&mut reader, 3)?;
            entries.push(HostTimelineEntry {
                narrative: CommitId::from_bytes(reader.bytes_exact::<32>()?),
                host: decode_host_snapshot(&mut reader)?,
                ledger_fence: LedgerFence::from_u64(reader.unsigned()?),
            });
        }
        reader.finish()?;
        let value = Self::checked(execution, coverage, entries)
            .map_err(|_| WireError::Schema("Host Timeline invariants"))?;
        canonical(&value.encode(), encoded)?;
        Ok(value)
    }
}

fn encode_host_snapshot(writer: &mut Writer, snapshot: &HostSnapshotRef) {
    writer.array(4);
    writer.text(snapshot.provider.as_str());
    writer.text(&snapshot.reference);
    writer.unsigned(u64::from(snapshot.format_version));
    writer.bytes(snapshot.digest.as_bytes());
}

fn decode_host_snapshot(reader: &mut Reader<'_>) -> Result<HostSnapshotRef, WireError> {
    expect_array(reader, 4)?;
    let provider = CapabilityId::new(reader.text(64)?)
        .map_err(|_| WireError::Schema("Host Snapshot provider"))?;
    let reference = reader.text(256)?;
    let version = u16::try_from(reader.unsigned()?).map_err(|_| WireError::IntegerOverflow)?;
    let digest = HostSnapshotDigest::from_bytes(reader.bytes_exact::<32>()?);
    HostSnapshotRef::new(provider, reference, version, digest)
        .map_err(|_| WireError::Schema("Host Snapshot reference"))
}

fn encode_coverage(writer: &mut Writer, coverage: TimelineCoverage) {
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

fn decode_coverage(reader: &mut Reader<'_>) -> Result<TimelineCoverage, WireError> {
    let length = reader.array()?;
    match (reader.unsigned()?, length) {
        (0, 2) => Ok(TimelineCoverage::FromBaseline {
            baseline: CommitId::from_bytes(reader.bytes_exact::<32>()?),
        }),
        (1, 3) => Ok(TimelineCoverage::Imported {
            baseline: CommitId::from_bytes(reader.bytes_exact::<32>()?),
            source: TimelineArchiveManifestId::from_bytes(reader.bytes_exact::<32>()?),
        }),
        _ => Err(WireError::Schema("Timeline coverage")),
    }
}

fn schema(reader: &mut Reader<'_>, expected: u16) -> Result<(), WireError> {
    if reader.unsigned()? == u64::from(expected) {
        Ok(())
    } else {
        Err(WireError::Schema("manifest schema"))
    }
}

fn canonical(encoded: &[u8], original: &[u8]) -> Result<(), WireError> {
    if encoded == original {
        Ok(())
    } else {
        Err(WireError::NonCanonical)
    }
}
