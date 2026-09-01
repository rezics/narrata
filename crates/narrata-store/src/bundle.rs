use std::collections::{BTreeMap, BTreeSet, VecDeque};

use narrata_core::{
    CommitId, ExecutionId, ObjectId, ProgramArtifactId, TimelineArchiveManifestId,
    codec::ObjectKind, limits::ProgramLoadLimits, program::load_program,
};
use thiserror::Error;

use crate::{
    ArchiveMutation, ArchivedBookmark, ArchivedBranchRef, ArchivedSaveRef, ArchivedSessionView,
    BranchId, CatalogMutation, CatalogRefKey, CheckedObject, CheckpointBundleManifestV1,
    CommitCauseV1, CommitTransaction, CommitV1, ManifestError, ObjectDescriptor, RefKey,
    RefMutation, RefName, RefRevision, SaveStore, StoreError, TimelineArchiveManifestV1,
    TimelineArchiveRefKey, TimelineCatalogEventKind, TimelineCatalogEventV1, TimelineCoverage,
    TimelineSession, TransitionReceiptV1,
    manifest::{CHECKPOINT_MANIFEST_SCHEMA_V1, TIMELINE_ARCHIVE_MANIFEST_SCHEMA_V1},
};

const CHECKPOINT_MAGIC: &[u8; 8] = b"NARCPB1\0";
const TIMELINE_MAGIC: &[u8; 8] = b"NARTLB1\0";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BundleLimits {
    pub max_total_bytes: u64,
    pub max_object_bytes: u64,
    pub max_objects: u64,
}

impl Default for BundleLimits {
    fn default() -> Self {
        Self {
            max_total_bytes: 512 * 1024 * 1024,
            max_object_bytes: 128 * 1024 * 1024,
            max_objects: 100_000,
        }
    }
}

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

impl CheckpointBundle {
    pub fn export(
        store: &impl SaveStore,
        root: CommitId,
        receiver_has: &BTreeSet<ObjectId>,
    ) -> Result<Self, BundleError> {
        let objects = object_map(store)?;
        let closure = closure(&objects, &[object_id(root.as_bytes())])?;
        let descriptors = descriptors(&objects, &closure)?;
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
            manifest: CheckpointBundleManifestV1 {
                root,
                objects: descriptors,
                optional_host_manifest: None,
            },
            objects: transmitted,
        })
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, BundleError> {
        self.manifest.validate()?;
        encode_bundle(CHECKPOINT_MAGIC, &self.manifest.to_object()?, &self.objects)
    }

    pub fn from_bytes(bytes: &[u8], limits: BundleLimits) -> Result<Self, BundleError> {
        let (manifest_object, objects) = decode_bundle(
            bytes,
            CHECKPOINT_MAGIC,
            ObjectKind::CheckpointBundleManifest,
            CHECKPOINT_MANIFEST_SCHEMA_V1,
            limits,
        )?;
        let manifest = CheckpointBundleManifestV1::decode(manifest_object.payload())?;
        validate_transmitted(&manifest.objects, &objects)?;
        Ok(Self { manifest, objects })
    }

    pub fn import(
        self,
        store: &mut impl SaveStore,
        target: RefKey,
        expected: Option<RefRevision>,
        observed_at: u64,
    ) -> Result<crate::RefValue, BundleError> {
        let available = gather_available(store, &self.manifest.objects, &self.objects)?;
        let closure = closure(&available, &[object_id(self.manifest.root.as_bytes())])?;
        let declared = self
            .manifest
            .objects
            .iter()
            .map(|descriptor| descriptor.id)
            .collect::<BTreeSet<_>>();
        if closure != declared {
            return Err(BundleError::ClosureMismatch);
        }
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
            import_transaction: true,
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
        let objects = object_map(store)?;
        let mut branch_heads = Vec::new();
        let mut save_refs = Vec::new();
        let mut bookmarks = Vec::new();
        let mut roots = vec![
            object_id(catalog.event.as_bytes()),
            object_id(catalog.coverage.baseline().as_bytes()),
        ];
        for (key, value) in store.list_refs()? {
            let commit = match objects.get(&object_id(value.commit.as_bytes())) {
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
        let closure = closure(&objects, &roots)?;
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
                key: RefKey::branch(
                    self.manifest.execution,
                    *mapping
                        .branch_ids
                        .get(&value.branch)
                        .ok_or(BundleError::Mapping)?,
                )
                .map_err(|_| BundleError::Mapping)?,
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
            import_transaction: true,
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
    let mut sorted = objects.to_vec();
    sorted.sort_by_key(CheckedObject::id);
    if sorted.windows(2).any(|pair| pair[0].id() == pair[1].id()) {
        return Err(BundleError::DuplicateObject);
    }
    let mut bytes = Vec::new();
    bytes.extend_from_slice(magic);
    push_u64(&mut bytes, manifest.bytes().len() as u64);
    bytes.extend_from_slice(manifest.bytes());
    push_u64(&mut bytes, sorted.len() as u64);
    for object in sorted {
        bytes.extend_from_slice(object.id().as_bytes());
        bytes.extend_from_slice(&object.kind().code().to_be_bytes());
        bytes.extend_from_slice(&object.schema().to_be_bytes());
        push_u64(&mut bytes, object.bytes().len() as u64);
        bytes.extend_from_slice(object.bytes());
    }
    Ok(bytes)
}

fn decode_bundle(
    bytes: &[u8],
    magic: &[u8; 8],
    manifest_kind: ObjectKind,
    manifest_schema: u16,
    limits: BundleLimits,
) -> Result<(CheckedObject, Vec<CheckedObject>), BundleError> {
    if bytes.len() as u64 > limits.max_total_bytes {
        return Err(BundleError::Limit("total bytes"));
    }
    let mut cursor = BinaryCursor::new(bytes);
    if cursor.take(8)? != magic {
        return Err(BundleError::WrongKind);
    }
    let manifest_len = cursor.u64()?;
    if manifest_len > limits.max_object_bytes {
        return Err(BundleError::Limit("manifest bytes"));
    }
    let manifest_bytes = cursor.take(to_usize(manifest_len)?)?;
    let manifest = CheckedObject::from_bytes(
        manifest_bytes,
        manifest_kind,
        manifest_schema,
        limits.max_object_bytes,
    )
    .map_err(|error| BundleError::Object(error.to_string()))?;
    let count = cursor.u64()?;
    if count > limits.max_objects {
        return Err(BundleError::Limit("object count"));
    }
    let mut objects = Vec::with_capacity(to_usize(count)?);
    let mut previous = None;
    for _ in 0..count {
        let declared_id = ObjectId::from_bytes(cursor.array::<32>()?);
        let kind = ObjectKind::from_code(cursor.u16()?).ok_or(BundleError::DescriptorMismatch)?;
        let schema = cursor.u16()?;
        let length = cursor.u64()?;
        if length > limits.max_object_bytes {
            return Err(BundleError::Limit("object bytes"));
        }
        let object = CheckedObject::from_bytes(
            cursor.take(to_usize(length)?)?,
            kind,
            schema,
            limits.max_object_bytes,
        )
        .map_err(|error| BundleError::Object(error.to_string()))?;
        if object.id() != declared_id {
            return Err(BundleError::DescriptorMismatch);
        }
        if previous.is_some_and(|id| id >= object.id()) {
            return Err(BundleError::DuplicateObject);
        }
        previous = Some(object.id());
        objects.push(object);
    }
    if !cursor.finished() {
        return Err(BundleError::DescriptorMismatch);
    }
    Ok((manifest, objects))
}

fn validate_transmitted(
    descriptors: &[ObjectDescriptor],
    objects: &[CheckedObject],
) -> Result<(), BundleError> {
    let declared = descriptors
        .iter()
        .map(|descriptor| (descriptor.id, descriptor))
        .collect::<BTreeMap<_, _>>();
    for object in objects {
        let descriptor = declared
            .get(&object.id())
            .ok_or(BundleError::DescriptorMismatch)?;
        if descriptor.kind != object.kind()
            || descriptor.schema != object.schema()
            || descriptor.bytes != object.bytes().len() as u64
        {
            return Err(BundleError::DescriptorMismatch);
        }
    }
    Ok(())
}

fn gather_available(
    store: &impl SaveStore,
    descriptors: &[ObjectDescriptor],
    transmitted: &[CheckedObject],
) -> Result<BTreeMap<ObjectId, CheckedObject>, BundleError> {
    let incoming = transmitted
        .iter()
        .cloned()
        .map(|object| (object.id(), object))
        .collect::<BTreeMap<_, _>>();
    let mut available = BTreeMap::new();
    for descriptor in descriptors {
        let object = match incoming.get(&descriptor.id).cloned() {
            Some(object) => object,
            None => store
                .get_object(descriptor.id)?
                .ok_or(BundleError::MissingObject(descriptor.id))?,
        };
        if object.kind() != descriptor.kind
            || object.schema() != descriptor.schema
            || object.bytes().len() as u64 != descriptor.bytes
        {
            return Err(BundleError::DescriptorMismatch);
        }
        available.insert(descriptor.id, object);
    }
    Ok(available)
}

fn object_map(store: &impl SaveStore) -> Result<BTreeMap<ObjectId, CheckedObject>, BundleError> {
    Ok(store
        .list_objects()?
        .into_iter()
        .map(|object| (object.id(), object))
        .collect())
}

fn closure(
    objects: &BTreeMap<ObjectId, CheckedObject>,
    roots: &[ObjectId],
) -> Result<BTreeSet<ObjectId>, BundleError> {
    let mut marked = BTreeSet::new();
    let mut queue = VecDeque::from(roots.to_vec());
    while let Some(id) = queue.pop_front() {
        if !marked.insert(id) {
            continue;
        }
        if marked.len() > 1_000_000 {
            return Err(BundleError::Limit("closure objects"));
        }
        let object = objects.get(&id).ok_or(BundleError::MissingObject(id))?;
        queue.extend(edges(objects, object)?);
    }
    Ok(marked)
}

fn edges(
    objects: &BTreeMap<ObjectId, CheckedObject>,
    object: &CheckedObject,
) -> Result<Vec<ObjectId>, BundleError> {
    let mut values = Vec::new();
    match object.kind() {
        ObjectKind::Commit => {
            let commit = CommitV1::decode(object.payload())
                .map_err(|_| BundleError::Timeline("invalid Commit"))?;
            values.extend(commit.parent.map(|id| object_id(id.as_bytes())));
            values.push(object_id(commit.snapshot.as_bytes()));
            if let CommitCauseV1::RuntimeTransition(receipt) = commit.cause {
                values.push(object_id(receipt.as_bytes()));
            }
            values.push(find_program(objects, commit.program)?);
        }
        ObjectKind::TimelineCatalogEvent => {
            let event = TimelineCatalogEventV1::decode(object.payload())
                .map_err(|_| BundleError::Timeline("invalid Catalog Event"))?;
            values.extend(event.previous.map(|id| object_id(id.as_bytes())));
            values.extend(
                event
                    .referenced_commits()
                    .iter()
                    .map(|id| object_id(id.as_bytes())),
            );
        }
        ObjectKind::CheckpointBundleManifest => {
            let manifest = CheckpointBundleManifestV1::decode(object.payload())?;
            values.push(object_id(manifest.root.as_bytes()));
            values.extend(manifest.optional_host_manifest);
        }
        ObjectKind::TimelineArchiveManifest => {
            let manifest = TimelineArchiveManifestV1::decode(object.payload())?;
            values.push(object_id(manifest.catalog_head.as_bytes()));
            values.push(object_id(manifest.coverage.baseline().as_bytes()));
            values.extend(manifest.host_timeline);
        }
        ObjectKind::Program | ObjectKind::Snapshot | ObjectKind::Receipt | ObjectKind::Value => {}
    }
    Ok(values)
}

fn find_program(
    objects: &BTreeMap<ObjectId, CheckedObject>,
    id: ProgramArtifactId,
) -> Result<ObjectId, BundleError> {
    objects
        .iter()
        .filter(|(_, object)| object.kind() == ObjectKind::Program)
        .find_map(|(object_id, object)| {
            load_program(object.bytes(), &ProgramLoadLimits::default())
                .ok()
                .filter(|program| program.artifact_id() == id)
                .map(|_| *object_id)
        })
        .ok_or(BundleError::Timeline("Program Artifact is missing"))
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

fn push_u64(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&value.to_be_bytes());
}

fn to_usize(value: u64) -> Result<usize, BundleError> {
    usize::try_from(value).map_err(|_| BundleError::Limit("platform length"))
}

struct BinaryCursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> BinaryCursor<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], BundleError> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or(BundleError::Truncated)?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or(BundleError::Truncated)?;
        self.offset = end;
        Ok(value)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], BundleError> {
        self.take(N)?.try_into().map_err(|_| BundleError::Truncated)
    }

    fn u16(&mut self) -> Result<u16, BundleError> {
        Ok(u16::from_be_bytes(self.array()?))
    }

    fn u64(&mut self) -> Result<u64, BundleError> {
        Ok(u64::from_be_bytes(self.array()?))
    }

    fn finished(&self) -> bool {
        self.offset == self.bytes.len()
    }
}

#[allow(dead_code)]
fn _receipt_is_checked(_: &TransitionReceiptV1) {}
