//! Checkpoint bundles and timeline archives of Stage 1–5 saves.
//!
//! The container, the closure walk and the checks against descriptors are the history layer's
//! (ADR 0015); this module adds the timeline archive, whose manifest names a catalog, branches,
//! saves and bookmarks, and converts between Stage 1–5 identities and history objects.

use std::collections::{BTreeMap, BTreeSet};

use narrata_core::{
    CommitId, ExecutionId, ObjectId, ProgramArtifactId, TimelineArchiveManifestId,
    codec::ObjectKind,
};
pub use narrata_history::BundleLimits;
use narrata_history::{CHECKPOINT_MAGIC, ContainerError, Object, ObjectSource};
use thiserror::Error;

use crate::{
    ArchiveMutation, ArchivedBookmark, ArchivedBranchRef, ArchivedSaveRef, ArchivedSessionView,
    BranchId, CatalogMutation, CatalogRefKey, CheckedObject, CheckpointBundleManifestV1,
    CommitTransaction, CommitV1, ManifestError, ObjectDescriptor, RefKey, RefMutation, RefName,
    RefRevision, RefScope, SaveStore, StoreError, TimelineArchiveManifestV1, TimelineArchiveRefKey,
    TimelineCatalogEventKind, TimelineCatalogEventV1, TimelineCoverage, TimelineSession,
    engine::{Legacy, PROGRAM},
    manifest::TIMELINE_ARCHIVE_MANIFEST_SCHEMA_V1,
    object::{history_id, id},
    scan_all, timeline_branch,
};

const TIMELINE_MAGIC: &[u8; 8] = b"NARTLB1\0";

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum BundleError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    #[error("bundle is truncated")]
    Truncated,
    #[error("wrong bundle discriminator")]
    WrongKind,
    #[error("bundle exceeds configured limit: {0}")]
    Limit(&'static str),
    #[error("bundle descriptor is duplicated or unsorted")]
    DescriptorOrder,
    #[error("bundle object is duplicated")]
    DuplicateObject,
    #[error("bundle object does not match its descriptor")]
    DescriptorMismatch,
    #[error("bundle is missing required object {0}")]
    MissingObject(ObjectId),
    #[error("bundle closure does not exactly match the manifest")]
    ClosureMismatch,
    #[error("timeline archive is internally inconsistent: {0}")]
    Timeline(&'static str),
    #[error("timeline import mapping is incomplete or ambiguous")]
    Mapping,
    #[error("object envelope is invalid: {0}")]
    Object(String),
}

impl From<ContainerError> for BundleError {
    fn from(value: ContainerError) -> Self {
        match value {
            ContainerError::Manifest(error) => Self::Manifest(error.into()),
            ContainerError::Truncated => Self::Truncated,
            ContainerError::WrongKind => Self::WrongKind,
            ContainerError::Limit(limit) => Self::Limit(limit),
            ContainerError::DuplicateObject => Self::DuplicateObject,
            ContainerError::DescriptorMismatch => Self::DescriptorMismatch,
            ContainerError::Object(error) => Self::Object(error.to_string()),
        }
    }
}

impl From<narrata_history::BundleError<StoreError>> for BundleError {
    fn from(value: narrata_history::BundleError<StoreError>) -> Self {
        match value {
            narrata_history::BundleError::Container(error) => error.into(),
            narrata_history::BundleError::Store(error) => Self::Store(error),
            narrata_history::BundleError::MissingObject(object) => Self::MissingObject(id(object)),
            narrata_history::BundleError::ClosureMismatch => Self::ClosureMismatch,
            // Commits name their Program, the only named kind of a Stage 1–5 store.
            narrata_history::BundleError::Unresolved { .. } => {
                Self::Timeline("Program Artifact is missing")
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct CheckpointBundle {
    pub manifest: CheckpointBundleManifestV1,
    pub objects: Vec<CheckedObject>,
}

#[derive(Clone, Debug)]
pub struct TimelineArchiveBundle {
    pub manifest: TimelineArchiveManifestV1,
    pub objects: Vec<CheckedObject>,
}

#[derive(Clone, Debug)]
pub struct TimelineImportMapping {
    pub archive_name: RefName,
    pub save_owner: RefName,
    pub bookmark_owner: RefName,
    pub branch_ids: BTreeMap<BranchId, BranchId>,
    pub save_names: BTreeMap<RefName, RefName>,
    pub bookmark_names: BTreeMap<RefName, RefName>,
    pub session_name: Option<RefName>,
}

/// A [`SaveStore`] as the source of a bundle's objects; Programs resolve through its index.
struct Saves<'a, S>(&'a S);

impl<S: SaveStore> ObjectSource for Saves<'_, S> {
    type Error = StoreError;

    fn objects(
        &self,
        ids: &[narrata_history::ObjectId],
    ) -> Result<Vec<Option<Object>>, StoreError> {
        let ids = ids.iter().copied().map(id).collect::<Vec<_>>();
        Ok(self
            .0
            .get_objects(&ids)?
            .into_iter()
            .map(|object| object.map(CheckedObject::into_object))
            .collect())
    }

    fn resolve(
        &self,
        kind: u16,
        name: &[u8; 32],
    ) -> Result<Option<narrata_history::ObjectId>, StoreError> {
        if kind != PROGRAM {
            return Ok(None);
        }
        Ok(self
            .0
            .find_program(ProgramArtifactId::from_bytes(*name))?
            .map(|program| history_id(program.as_bytes())))
    }
}

fn known(kind: u16) -> bool {
    ObjectKind::from_code(kind).is_some()
}

fn history_objects(objects: &[CheckedObject]) -> Vec<Object> {
    objects
        .iter()
        .map(|object| object.object().clone())
        .collect()
}

fn checked(objects: Vec<Object>) -> Result<Vec<CheckedObject>, BundleError> {
    Ok(objects
        .into_iter()
        .map(CheckedObject::from_object)
        .collect::<Result<_, _>>()?)
}

fn checked_map(
    objects: BTreeMap<narrata_history::ObjectId, Object>,
) -> Result<BTreeMap<ObjectId, CheckedObject>, BundleError> {
    objects
        .into_iter()
        .map(|(key, object)| Ok((id(key), CheckedObject::from_object(object)?)))
        .collect()
}

fn history_ids(ids: impl IntoIterator<Item = ObjectId>) -> Vec<narrata_history::ObjectId> {
    ids.into_iter()
        .map(|object| history_id(object.as_bytes()))
        .collect()
}

impl CheckpointBundle {
    pub fn export(
        store: &impl SaveStore,
        root: CommitId,
        receiver_has: &BTreeSet<ObjectId>,
    ) -> Result<Self, BundleError> {
        let receiver_has = history_ids(receiver_has.iter().copied())
            .into_iter()
            .collect();
        let bundle = narrata_history::CheckpointBundle::export(
            &Saves(store),
            &Legacy::default(),
            history_id(root.as_bytes()),
            &receiver_has,
        )?;
        Ok(Self {
            manifest: CheckpointBundleManifestV1::from_manifest(bundle.manifest)?,
            objects: checked(bundle.objects)?,
        })
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, BundleError> {
        self.manifest.validate()?;
        encode_bundle(CHECKPOINT_MAGIC, &self.manifest.to_object()?, &self.objects)
    }

    pub fn from_bytes(bytes: &[u8], limits: BundleLimits) -> Result<Self, BundleError> {
        let bundle = narrata_history::CheckpointBundle::from_bytes(bytes, limits, known)?;
        Ok(Self {
            manifest: CheckpointBundleManifestV1::from_manifest(bundle.manifest)?,
            objects: checked(bundle.objects)?,
        })
    }

    pub fn import(
        self,
        store: &mut impl SaveStore,
        target: RefKey,
        expected: Option<RefRevision>,
        observed_at: u64,
    ) -> Result<crate::RefValue, BundleError> {
        narrata_history::CheckpointBundle {
            manifest: self.manifest.manifest(),
            objects: history_objects(&self.objects),
        }
        .check_closure(&Saves(&*store), &Legacy::default())?;
        let manifest_object = self.manifest.to_object()?;
        let outcome = store.commit(CommitTransaction {
            objects: self
                .objects
                .into_iter()
                .chain(std::iter::once(manifest_object))
                .collect(),
            refs: vec![RefMutation {
                key: target.clone(),
                expected,
                next: Some(self.manifest.root),
            }],
            observed_at,
            ..CommitTransaction::default()
        })?;
        outcome
            .refs
            .get(&target)
            .and_then(|value| *value)
            .ok_or(BundleError::Timeline("import Ref was not created"))
    }
}

impl TimelineArchiveBundle {
    pub fn export(
        store: &impl SaveStore,
        execution: ExecutionId,
        active: Option<TimelineSession>,
        receiver_has: &BTreeSet<ObjectId>,
    ) -> Result<Self, BundleError> {
        let catalog = store
            .read_catalog_head(&CatalogRefKey::new(execution))?
            .ok_or(BundleError::Timeline("complete recording is not enabled"))?;
        let mut branch_heads = Vec::new();
        let mut save_refs = Vec::new();
        let mut bookmarks = Vec::new();
        let mut roots = vec![
            object_id(catalog.event.as_bytes()),
            object_id(catalog.coverage.baseline().as_bytes()),
        ];
        let refs = scan_all(
            |after| store.scan_refs(&RefScope::All, after, u32::MAX),
            |(key, _)| key.clone(),
        )?;
        let targets = refs
            .iter()
            .map(|(_, value)| object_id(value.commit.as_bytes()))
            .collect::<Vec<_>>();
        for ((key, value), target) in refs.into_iter().zip(store.get_objects(&targets)?) {
            let commit = match target {
                Some(object) if object.kind() == ObjectKind::Commit => {
                    CommitV1::decode(object.payload())
                        .map_err(|_| BundleError::Timeline("invalid Commit"))?
                }
                _ => continue,
            };
            if commit.execution != execution {
                continue;
            }
            match key.namespace() {
                crate::RefNamespace::Branch => {
                    let bytes = hex::decode(key.name().as_str())
                        .map_err(|_| BundleError::Timeline("branch Ref name"))?;
                    let bytes = <[u8; 16]>::try_from(bytes)
                        .map_err(|_| BundleError::Timeline("branch Ref name"))?;
                    branch_heads.push(ArchivedBranchRef {
                        branch: BranchId::from_bytes(bytes),
                        head: value.commit,
                    });
                    roots.push(object_id(value.commit.as_bytes()));
                }
                crate::RefNamespace::Save => {
                    save_refs.push(ArchivedSaveRef {
                        name: key.name().clone(),
                        commit: value.commit,
                    });
                    roots.push(object_id(value.commit.as_bytes()));
                }
                crate::RefNamespace::Bookmark => {
                    bookmarks.push(ArchivedBookmark {
                        name: key.name().clone(),
                        commit: value.commit,
                    });
                    roots.push(object_id(value.commit.as_bytes()));
                }
                crate::RefNamespace::Active | crate::RefNamespace::Temporary => {}
            }
        }
        branch_heads.sort();
        branch_heads.dedup();
        save_refs.sort();
        save_refs.dedup();
        bookmarks.sort();
        bookmarks.dedup();
        let objects = stored_closure(store, &roots)?;
        let closure = objects.keys().copied().collect::<BTreeSet<_>>();
        let object_descriptors = descriptors(&objects, &closure)?;
        let active = active.map(|value| ArchivedSessionView {
            selected_branch: value.selected_branch,
            cursor: value.cursor,
        });
        let manifest = TimelineArchiveManifestV1 {
            execution,
            coverage: catalog.coverage,
            branch_heads,
            save_refs,
            bookmarks,
            catalog_head: catalog.event,
            active,
            objects: object_descriptors,
            host_timeline: None,
        };
        validate_timeline(&objects, &manifest)?;
        let transmitted = closure
            .iter()
            .filter(|id| !receiver_has.contains(id))
            .map(|id| {
                objects
                    .get(id)
                    .cloned()
                    .ok_or(BundleError::MissingObject(*id))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            manifest,
            objects: transmitted,
        })
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, BundleError> {
        self.manifest.validate()?;
        encode_bundle(TIMELINE_MAGIC, &self.manifest.to_object()?, &self.objects)
    }

    pub fn from_bytes(bytes: &[u8], limits: BundleLimits) -> Result<Self, BundleError> {
        let (manifest_object, objects) = decode_bundle(
            bytes,
            TIMELINE_MAGIC,
            ObjectKind::TimelineArchiveManifest,
            TIMELINE_ARCHIVE_MANIFEST_SCHEMA_V1,
            limits,
        )?;
        let manifest = TimelineArchiveManifestV1::decode(manifest_object.payload())?;
        validate_transmitted(&manifest.objects, &objects)?;
        Ok(Self { manifest, objects })
    }

    pub fn import(
        self,
        store: &mut impl SaveStore,
        mapping: TimelineImportMapping,
        observed_at: u64,
    ) -> Result<TimelineArchiveManifestId, BundleError> {
        validate_mapping(&self.manifest, &mapping)?;
        let available = gather_available(store, &self.manifest.objects, &self.objects)?;
        validate_timeline(&available, &self.manifest)?;
        let declared = self
            .manifest
            .objects
            .iter()
            .map(|descriptor| descriptor.id)
            .collect::<BTreeSet<_>>();
        let mut roots = self
            .manifest
            .branch_heads
            .iter()
            .map(|value| object_id(value.head.as_bytes()))
            .chain(
                self.manifest
                    .save_refs
                    .iter()
                    .map(|value| object_id(value.commit.as_bytes())),
            )
            .chain(
                self.manifest
                    .bookmarks
                    .iter()
                    .map(|value| object_id(value.commit.as_bytes())),
            )
            .collect::<Vec<_>>();
        roots.push(object_id(self.manifest.catalog_head.as_bytes()));
        roots.push(object_id(self.manifest.coverage.baseline().as_bytes()));
        if closure(&available, &roots)? != declared {
            return Err(BundleError::ClosureMismatch);
        }
        let manifest_object = self.manifest.to_object()?;
        let manifest_id = TimelineArchiveManifestId::from_bytes(*manifest_object.id().as_bytes());
        let imported_coverage = TimelineCoverage::Imported {
            baseline: self.manifest.coverage.baseline(),
            source: manifest_id,
        };
        let mut refs = Vec::new();
        for value in &self.manifest.branch_heads {
            refs.push(RefMutation {
                key: timeline_branch(
                    self.manifest.execution,
                    *mapping
                        .branch_ids
                        .get(&value.branch)
                        .ok_or(BundleError::Mapping)?,
                ),
                expected: None,
                next: Some(value.head),
            });
        }
        for value in &self.manifest.save_refs {
            refs.push(RefMutation {
                key: RefKey::save(
                    mapping.save_owner.clone(),
                    mapping
                        .save_names
                        .get(&value.name)
                        .cloned()
                        .ok_or(BundleError::Mapping)?,
                ),
                expected: None,
                next: Some(value.commit),
            });
        }
        for value in &self.manifest.bookmarks {
            refs.push(RefMutation {
                key: RefKey::bookmark(
                    mapping.bookmark_owner.clone(),
                    mapping
                        .bookmark_names
                        .get(&value.name)
                        .cloned()
                        .ok_or(BundleError::Mapping)?,
                ),
                expected: None,
                next: Some(value.commit),
            });
        }
        if let (Some(active), Some(session_name)) = (self.manifest.active, mapping.session_name) {
            refs.push(RefMutation {
                key: RefKey::active(session_name).map_err(|_| BundleError::Mapping)?,
                expected: None,
                next: Some(active.cursor),
            });
        }
        store.commit(CommitTransaction {
            objects: self
                .objects
                .into_iter()
                .chain(std::iter::once(manifest_object))
                .collect(),
            refs,
            catalogs: vec![CatalogMutation {
                key: CatalogRefKey::new(self.manifest.execution),
                expected: None,
                next: Some(self.manifest.catalog_head),
                coverage: imported_coverage,
            }],
            archives: vec![ArchiveMutation {
                key: TimelineArchiveRefKey::new(self.manifest.execution, mapping.archive_name),
                expected: None,
                next: Some(manifest_id),
            }],
            observed_at,
            ..CommitTransaction::default()
        })?;
        Ok(manifest_id)
    }
}

fn validate_mapping(
    manifest: &TimelineArchiveManifestV1,
    mapping: &TimelineImportMapping,
) -> Result<(), BundleError> {
    let branches = manifest
        .branch_heads
        .iter()
        .map(|value| value.branch)
        .collect::<BTreeSet<_>>();
    let saves = manifest
        .save_refs
        .iter()
        .map(|value| value.name.clone())
        .collect::<BTreeSet<_>>();
    let bookmarks = manifest
        .bookmarks
        .iter()
        .map(|value| value.name.clone())
        .collect::<BTreeSet<_>>();
    let mapped_branches = mapping.branch_ids.keys().copied().collect::<BTreeSet<_>>();
    let mapped_saves = mapping.save_names.keys().cloned().collect::<BTreeSet<_>>();
    let mapped_bookmarks = mapping
        .bookmark_names
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    let unique_branch_targets = mapping
        .branch_ids
        .values()
        .copied()
        .collect::<BTreeSet<_>>()
        .len()
        == mapping.branch_ids.len();
    let unique_save_targets = mapping
        .save_names
        .values()
        .cloned()
        .collect::<BTreeSet<_>>()
        .len()
        == mapping.save_names.len();
    let unique_bookmark_targets = mapping
        .bookmark_names
        .values()
        .cloned()
        .collect::<BTreeSet<_>>()
        .len()
        == mapping.bookmark_names.len();
    if branches != mapped_branches
        || saves != mapped_saves
        || bookmarks != mapped_bookmarks
        || !unique_branch_targets
        || !unique_save_targets
        || !unique_bookmark_targets
        || (manifest.active.is_some() != mapping.session_name.is_some())
    {
        return Err(BundleError::Mapping);
    }
    Ok(())
}

fn validate_timeline(
    objects: &BTreeMap<ObjectId, CheckedObject>,
    manifest: &TimelineArchiveManifestV1,
) -> Result<(), BundleError> {
    manifest.validate()?;
    let mut event_id = Some(manifest.catalog_head);
    let mut seen = BTreeSet::new();
    let mut baseline = None;
    while let Some(id) = event_id {
        if !seen.insert(id) || seen.len() > 1_000_000 {
            return Err(BundleError::Timeline("Catalog cycle or limit"));
        }
        let object = objects
            .get(&object_id(id.as_bytes()))
            .ok_or(BundleError::MissingObject(object_id(id.as_bytes())))?;
        let event = TimelineCatalogEventV1::decode(object.payload())
            .map_err(|_| BundleError::Timeline("invalid Catalog Event"))?;
        if event.execution != manifest.execution {
            return Err(BundleError::Timeline("mixed Execution in Catalog"));
        }
        if let TimelineCatalogEventKind::RecordingStarted {
            baseline: value, ..
        } = event.kind
        {
            if event.previous.is_some() {
                return Err(BundleError::Timeline("RecordingStarted previous"));
            }
            baseline = Some(value);
        }
        event_id = event.previous;
    }
    if baseline != Some(manifest.coverage.baseline()) {
        return Err(BundleError::Timeline("coverage baseline"));
    }
    for root in manifest
        .branch_heads
        .iter()
        .map(|value| value.head)
        .chain(manifest.save_refs.iter().map(|value| value.commit))
        .chain(manifest.bookmarks.iter().map(|value| value.commit))
    {
        let object = objects
            .get(&object_id(root.as_bytes()))
            .ok_or(BundleError::MissingObject(object_id(root.as_bytes())))?;
        let commit = CommitV1::decode(object.payload())
            .map_err(|_| BundleError::Timeline("invalid root Commit"))?;
        if commit.execution != manifest.execution {
            return Err(BundleError::Timeline("mixed Execution in roots"));
        }
    }
    if let Some(active) = manifest.active {
        let head = manifest
            .branch_heads
            .iter()
            .find(|branch| branch.branch == active.selected_branch)
            .map(|branch| branch.head)
            .ok_or(BundleError::Timeline("active branch"))?;
        ensure_ancestor_in(objects, active.cursor, head)?;
    }
    Ok(())
}

fn ensure_ancestor_in(
    objects: &BTreeMap<ObjectId, CheckedObject>,
    ancestor: CommitId,
    mut current: CommitId,
) -> Result<(), BundleError> {
    let mut seen = BTreeSet::new();
    while current != ancestor {
        if !seen.insert(current) {
            return Err(BundleError::Timeline("Commit cycle"));
        }
        let object = objects
            .get(&object_id(current.as_bytes()))
            .ok_or(BundleError::MissingObject(object_id(current.as_bytes())))?;
        current = CommitV1::decode(object.payload())
            .map_err(|_| BundleError::Timeline("invalid Commit"))?
            .parent
            .ok_or(BundleError::Timeline("active cursor is not an ancestor"))?;
    }
    Ok(())
}

fn encode_bundle(
    magic: &[u8; 8],
    manifest: &CheckedObject,
    objects: &[CheckedObject],
) -> Result<Vec<u8>, BundleError> {
    Ok(narrata_history::encode_container(
        magic,
        manifest.object(),
        &history_objects(objects),
    )?)
}

fn decode_bundle(
    bytes: &[u8],
    magic: &[u8; 8],
    manifest_kind: ObjectKind,
    manifest_schema: u16,
    limits: BundleLimits,
) -> Result<(CheckedObject, Vec<CheckedObject>), BundleError> {
    let (manifest, objects) = narrata_history::decode_container(
        bytes,
        magic,
        manifest_kind.code(),
        manifest_schema,
        limits,
        known,
    )?;
    Ok((CheckedObject::from_object(manifest)?, checked(objects)?))
}

fn validate_transmitted(
    descriptors: &[ObjectDescriptor],
    objects: &[CheckedObject],
) -> Result<(), BundleError> {
    let descriptors = descriptors
        .iter()
        .map(ObjectDescriptor::descriptor)
        .collect::<Vec<_>>();
    Ok(narrata_history::check_transmitted(
        &descriptors,
        &history_objects(objects),
    )?)
}

fn gather_available(
    store: &impl SaveStore,
    descriptors: &[ObjectDescriptor],
    transmitted: &[CheckedObject],
) -> Result<BTreeMap<ObjectId, CheckedObject>, BundleError> {
    let descriptors = descriptors
        .iter()
        .map(ObjectDescriptor::descriptor)
        .collect::<Vec<_>>();
    checked_map(narrata_history::gather(
        &Saves(store),
        &descriptors,
        &history_objects(transmitted),
    )?)
}

/// The closure of `roots` read from the store, one reference level per read.
fn stored_closure(
    store: &impl SaveStore,
    roots: &[ObjectId],
) -> Result<BTreeMap<ObjectId, CheckedObject>, BundleError> {
    checked_map(narrata_history::stored_closure(
        &Saves(store),
        &Legacy::default(),
        &history_ids(roots.iter().copied()),
    )?)
}

fn closure(
    objects: &BTreeMap<ObjectId, CheckedObject>,
    roots: &[ObjectId],
) -> Result<BTreeSet<ObjectId>, BundleError> {
    let objects = objects
        .iter()
        .map(|(key, object)| (history_id(key.as_bytes()), object.object().clone()))
        .collect();
    let closure = narrata_history::closure::<StoreError, _>(
        &Legacy::default(),
        &objects,
        &history_ids(roots.iter().copied()),
    )?;
    Ok(closure.into_iter().map(id).collect())
}

fn descriptors(
    objects: &BTreeMap<ObjectId, CheckedObject>,
    ids: &BTreeSet<ObjectId>,
) -> Result<Vec<ObjectDescriptor>, BundleError> {
    ids.iter()
        .map(|id| {
            objects
                .get(id)
                .map(ObjectDescriptor::from)
                .ok_or(BundleError::MissingObject(*id))
        })
        .collect()
}

fn object_id(bytes: &[u8; 32]) -> ObjectId {
    ObjectId::from_bytes(*bytes)
}
