//! Binding state without JavaScript values, so native tests exercise exactly what the Wasm
//! exports wrap. Inputs arrive one record, outline batch, package or object at a time; no
//! operation takes or returns a whole work as one JSON value.

use std::collections::{BTreeMap, BTreeSet};

use narrata_authoring::{
    CheckedPublication, CheckedRecord, CheckedRecords, ComposeMode, Diagnostic, DraftIndexes,
    Minter, PublicationReport, Report, compose_records, derive_draft_indexes, mint_ids,
    objects::Digest,
    publication::{
        MAX_DESCRIPTION_BYTES, ObjectReference, PublicationDescription, read_publication,
    },
    publish_records, split_source, validate_records,
};
use narrata_kernel::content::ProviderId;
use narrata_nodes::{
    AuthoredId, CompositionLock, ContentOutline, Error, MAX_PACK_BYTES, MemorySource, ObjectId,
    Pack, PackageSource, Program, ProjectManifest, ProjectSource, Result,
};

use crate::interface::{MintedIds, PreviewReport, PublishReport, RecordIndexes, RecordReport};

/// IDs minted by one call. An editor mints for one edit at a time.
pub const MAX_MINT: usize = 4096;

fn mode(lock: Option<&CompositionLock>) -> ComposeMode<'_> {
    lock.map_or(ComposeMode::Update, ComposeMode::Locked)
}

/// Checked records by index with their derived index rows and, when assembled, the source.
#[derive(Debug, Default)]
pub struct RecordSet {
    pub report: RecordReport,
    pub records: Vec<CheckedRecord>,
    indexes: BTreeMap<String, RecordIndexes>,
    pub source: Option<ProjectSource>,
}

impl Default for RecordReport {
    fn default() -> Self {
        Self {
            complete: true,
            diagnostics: Vec::new(),
            records: 0,
            minted: Vec::new(),
        }
    }
}

fn group(indexes: DraftIndexes) -> BTreeMap<String, RecordIndexes> {
    let mut grouped: BTreeMap<_, _> = indexes
        .ownership
        .into_iter()
        .map(|ownership| {
            (
                ownership.record.clone(),
                RecordIndexes {
                    ownership,
                    options: Vec::new(),
                },
            )
        })
        .collect();
    for option in indexes.options {
        if let Some(entry) = grouped.get_mut(&option.record) {
            entry.options.push(option);
        }
    }
    grouped
}

impl RecordSet {
    fn new(
        complete: bool,
        diagnostics: Vec<Diagnostic>,
        records: Vec<CheckedRecord>,
        indexes: Option<DraftIndexes>,
        source: Option<ProjectSource>,
        minted: Vec<AuthoredId>,
    ) -> Self {
        Self {
            report: RecordReport {
                complete,
                diagnostics,
                records: records.len() as u32,
                minted,
            },
            records,
            indexes: indexes.map(group).unwrap_or_default(),
            source,
        }
    }

    pub fn record(report: Report<CheckedRecord>) -> Self {
        Self::new(
            report.complete,
            report.diagnostics,
            report.value.into_iter().collect(),
            None,
            None,
            Vec::new(),
        )
    }

    pub fn checked(report: Report<CheckedRecords>, minted: Vec<AuthoredId>) -> Self {
        match report.value {
            Some(checked) => {
                let indexes = derive_draft_indexes(&checked);
                let source = checked.source().clone();
                Self::new(
                    report.complete,
                    report.diagnostics,
                    checked.records().to_vec(),
                    Some(indexes),
                    Some(source),
                    minted,
                )
            }
            None => Self::new(
                report.complete,
                report.diagnostics,
                Vec::new(),
                None,
                None,
                minted,
            ),
        }
    }

    /// Split records and their assembly both report; either failing yields no records.
    fn split(report: Report<Vec<CheckedRecord>>, minted: Vec<AuthoredId>) -> Self {
        let Some(records) = report.value else {
            return Self::checked(
                Report {
                    complete: report.complete,
                    diagnostics: report.diagnostics,
                    value: None,
                },
                minted,
            );
        };
        let mut assembled = CheckedRecords::new(records);
        let mut diagnostics = report.diagnostics;
        diagnostics.append(&mut assembled.diagnostics);
        assembled.diagnostics = diagnostics;
        assembled.complete &= report.complete;
        Self::checked(assembled, minted)
    }

    pub fn indexes(&self, index: usize) -> Option<&RecordIndexes> {
        self.indexes.get(&self.records.get(index)?.key())
    }
}

/// Persisted records and outline batches for one check, preview or publication.
#[derive(Debug, Default)]
pub struct DraftSet {
    records: Vec<Vec<u8>>,
    outlines: BTreeMap<ProviderId, ContentOutline>,
}

impl DraftSet {
    pub fn add(&mut self, bytes: &[u8]) {
        self.records.push(bytes.to_vec());
    }

    /// Outlines arrive in unit batches and assemble into one outline per provider. A unit
    /// arriving twice is rejected rather than silently replaced.
    pub fn add_outline(&mut self, batch: ContentOutline) -> Result<()> {
        let Some(outline) = self.outlines.get_mut(&batch.provider) else {
            self.outlines.insert(batch.provider.clone(), batch);
            return Ok(());
        };
        if outline.format_version != batch.format_version {
            return Err(Error::new(
                "outline_version",
                batch.provider.to_string(),
                "outline batches of one provider must share a format version",
            ));
        }
        if let Some(key) = batch
            .units
            .keys()
            .find(|key| outline.units.contains_key(*key))
        {
            return Err(Error::new(
                "outline_duplicate_unit",
                format!("{}/{key}", batch.provider),
                "this content unit's outline was already supplied",
            ));
        }
        outline.units.extend(batch.units);
        Ok(())
    }

    fn inputs(&self) -> Vec<&[u8]> {
        self.records.iter().map(Vec::as_slice).collect()
    }

    pub fn check(&self) -> RecordSet {
        RecordSet::checked(validate_records(&self.inputs()), Vec::new())
    }

    pub fn compose(
        &self,
        previous: Option<&Program>,
        lock: Option<&CompositionLock>,
    ) -> Result<PreviewObjects> {
        let report = compose_records(&self.inputs(), previous, mode(lock));
        let mut preview = PreviewObjects {
            report: PreviewReport {
                complete: report.complete,
                diagnostics: report.diagnostics,
                artifact_id: None,
                lock: None,
                compared: previous.is_some(),
                tombstones: Vec::new(),
                objects: Vec::new(),
            },
            pack: None,
            positions: BTreeMap::new(),
        };
        let Some(composed) = report.value else {
            return Ok(preview);
        };
        let composed = composed.composed;
        let pack = composed.compilation.objects;
        for (position, bytes) in pack
            .chunks
            .iter()
            .chain(std::iter::once(&pack.tombstones))
            .enumerate()
        {
            let id = MemorySource::default().insert(bytes.clone())?;
            preview.positions.insert(id, position);
            preview.report.objects.push(id);
        }
        preview.report.artifact_id = Some(composed.compilation.program.artifact_id());
        preview.report.lock = Some(composed.compilation.lock);
        preview.report.compared = composed.compared;
        preview.report.tombstones = composed.tombstones;
        preview.pack = Some(pack);
        Ok(preview)
    }

    pub fn publish(&self, previous: Option<&Program>, lock: Option<&CompositionLock>) -> Published {
        let outlines: Vec<_> = self.outlines.values().cloned().collect();
        published(publish_records(
            &self.inputs(),
            &outlines,
            previous,
            mode(lock),
        ))
    }
}

/// A preview's program objects. The manifest and entry chunk suffice for a first screen;
/// further chunks are read individually by object ID.
#[derive(Debug)]
pub struct PreviewObjects {
    pub report: PreviewReport,
    pack: Option<Pack>,
    positions: BTreeMap<ObjectId, usize>,
}

impl PreviewObjects {
    pub fn manifest(&self) -> Option<&[u8]> {
        Some(&self.pack.as_ref()?.manifest)
    }
    pub fn names(&self) -> Option<&[u8]> {
        self.pack.as_ref()?.names.as_deref()
    }
    pub fn object(&self, id: &ObjectId) -> Option<&[u8]> {
        let pack = self.pack.as_ref()?;
        let position = *self.positions.get(id)?;
        Some(pack.chunks.get(position).unwrap_or(&pack.tombstones))
    }
    /// The legacy single-file form, limited to 64 MiB like every pack.
    pub fn pack(&self) -> Result<Vec<u8>> {
        let pack = self
            .pack
            .as_ref()
            .ok_or_else(|| Error::new("preview", "$", "the preview did not compile"))?
            .encode();
        if pack.len() > MAX_PACK_BYTES {
            return Err(Error::new(
                "pack_too_large",
                "pack",
                "the program exceeds the 64 MiB single-file pack; read its objects instead",
            ));
        }
        Ok(pack)
    }
}

#[derive(Debug)]
pub struct Published {
    pub report: PublishReport,
    pub publication: Option<CheckedPublication>,
    pub records: RecordSet,
}

fn published(report: PublicationReport) -> Published {
    let mut out = Published {
        report: PublishReport {
            complete: report.complete,
            diagnostics: report.diagnostics,
            status: None,
            publication_id: None,
            candidate_artifact_id: None,
            previous_artifact_id: None,
            compared: false,
            lock: None,
            tombstones: Vec::new(),
        },
        publication: None,
        records: RecordSet::default(),
    };
    if let Some(value) = report.value {
        out.report.status = Some(value.status);
        out.report.publication_id = Some(value.publication.publication_id());
        out.report.candidate_artifact_id = Some(value.candidate_artifact_id);
        out.report.previous_artifact_id = value.previous_artifact_id;
        out.report.compared = value.compared;
        out.report.lock = Some(value.lock);
        out.report.tombstones = value.tombstones;
        out.records = RecordSet::new(
            true,
            Vec::new(),
            value.records,
            Some(value.indexes),
            None,
            Vec::new(),
        );
        out.publication = Some(value.publication);
    }
    out
}

/// Accepts a host ID as `object:<hex>` (the reader's chunk requests) or bare hex (the
/// publication description and object file names).
pub fn object_id(text: &str) -> Result<ObjectId> {
    let hex = text.strip_prefix("object:").unwrap_or(text);
    format!("object:{hex}")
        .parse()
        .map_err(|_| Error::new("identifier", text, "expected a 64-digit object ID"))
}

pub fn digest(text: &str) -> Result<Digest> {
    Ok(Digest::from_bytes(*object_id(text)?.as_bytes()))
}

/// Restores a checked publication from its description and individually supplied objects,
/// so a previous publication is not limited to the 64 MiB single-file pack.
#[derive(Debug)]
pub struct Loader {
    description: Vec<u8>,
    expected: Digest,
    references: BTreeMap<Digest, ObjectReference>,
    objects: BTreeMap<Digest, Vec<u8>>,
}

impl Loader {
    pub fn new(description: &[u8], expected: Digest) -> Result<Self> {
        let actual = Digest::from_bytes(narrata_kernel::codec::digest_bytes(
            "narrata.nodes.publication",
            1,
            description,
        ));
        if actual != expected {
            return Err(Error::new(
                "publication",
                expected.to_string(),
                "description bytes do not have this publication ID",
            ));
        }
        let text = std::str::from_utf8(description)
            .map_err(|error| Error::new("json", "$", error.to_string()))?;
        let decoded: PublicationDescription =
            narrata_nodes::parse_json_limited(text, MAX_DESCRIPTION_BYTES)?;
        Ok(Self {
            description: description.to_vec(),
            expected,
            references: decoded
                .objects
                .into_iter()
                .map(|reference| (reference.object_id, reference))
                .collect(),
            objects: BTreeMap::new(),
        })
    }

    pub fn references(&self) -> impl Iterator<Item = &ObjectReference> {
        self.references.values()
    }

    pub fn missing(&self) -> Vec<Digest> {
        self.references
            .keys()
            .filter(|id| !self.objects.contains_key(id))
            .copied()
            .collect()
    }

    /// Early length checks reject a wrong object before the whole set is read; identity,
    /// encoding and correspondence are proven when finishing.
    pub fn supply(&mut self, id: Digest, bytes: &[u8]) -> Result<()> {
        let reference = self.references.get(&id).ok_or_else(|| {
            Error::new(
                "reference",
                id.to_string(),
                "object is not in this publication",
            )
        })?;
        if reference.bytes != bytes.len() as u64 {
            return Err(Error::new(
                "object_length",
                id.to_string(),
                "object length differs from the publication description",
            ));
        }
        if let Some(existing) = self.objects.get(&id) {
            if existing != bytes {
                return Err(Error::new(
                    "object_identity",
                    id.to_string(),
                    "a different object was already supplied for this ID",
                ));
            }
            return Ok(());
        }
        self.objects.insert(id, bytes.to_vec());
        Ok(())
    }

    pub fn finish(mut self) -> Result<CheckedPublication> {
        let objects = &mut self.objects;
        read_publication(&self.description, self.expected, |id| {
            objects.remove(&id).ok_or_else(|| {
                Error::new(
                    "missing_object",
                    id.to_string(),
                    "supply every referenced object before finishing",
                )
            })
        })
    }
}

/// Live and deleted IDs that minting must avoid, shared across authored kinds.
#[derive(Debug, Default)]
pub struct Occupancy {
    taken: BTreeSet<AuthoredId>,
}

impl Occupancy {
    pub fn occupy(&mut self, ids: impl IntoIterator<Item = AuthoredId>) {
        self.taken.extend(ids);
    }

    pub fn mint(
        &mut self,
        nodes: usize,
        choice_points: usize,
        options: usize,
        clock: impl FnMut() -> Result<u64>,
        random: impl FnMut(&mut [u8]) -> Result<()>,
    ) -> Result<MintedIds> {
        if nodes.saturating_add(choice_points).saturating_add(options) > MAX_MINT {
            return Err(Error::new(
                "limit",
                "ids",
                format!("mint at most {MAX_MINT} IDs per call"),
            ));
        }
        let mut minter = Minter::new(self.taken.iter().copied(), clock, random);
        let mut minted = MintedIds::default();
        for _ in 0..nodes {
            minted.nodes.push(minter.node()?);
        }
        for _ in 0..choice_points {
            minted.choice_points.push(minter.choice_point()?);
        }
        for _ in 0..options {
            minted.options.push(minter.option()?);
        }
        self.taken.extend(minter.allocations);
        Ok(minted)
    }

    /// Fills missing IDs of an R2 edit, keeping every existing ID, and splits the result.
    pub fn fill(
        &mut self,
        source: &ProjectSource,
        clock: impl FnMut() -> Result<u64>,
        random: impl FnMut(&mut [u8]) -> Result<()>,
    ) -> Result<RecordSet> {
        let edit = mint_ids(source, self.taken.iter().copied(), clock, random)?;
        self.taken.extend(edit.minted.iter().copied());
        Ok(RecordSet::split(
            Report {
                complete: true,
                diagnostics: Vec::new(),
                value: Some(edit.records),
            },
            edit.minted,
        ))
    }
}

/// An R2 project supplied as its manifest and one package source at a time.
#[derive(Debug)]
pub struct SourceBuilder {
    pub source: ProjectSource,
}

impl SourceBuilder {
    pub fn new(manifest: ProjectManifest) -> Self {
        Self {
            source: ProjectSource {
                manifest,
                packages: BTreeMap::new(),
            },
        }
    }

    pub fn add_package(&mut self, alias: &str, package: PackageSource) -> Result<()> {
        if self.source.packages.contains_key(alias) {
            return Err(Error::new(
                "duplicate_package",
                format!("packages.{alias}"),
                "this package alias was already supplied",
            ));
        }
        self.source.packages.insert(alias.to_owned(), package);
        Ok(())
    }

    pub fn split(&self) -> RecordSet {
        RecordSet::split(split_source(&self.source), Vec::new())
    }
}

/// Program objects of a checked publication by object ID, for reader chunk requests.
pub fn publication_object<'a>(
    publication: &'a CheckedPublication,
    id: &str,
) -> Result<Option<&'a [u8]>> {
    Ok(publication.object(digest(id)?))
}
