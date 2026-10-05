//! Shallow checkpoints retain commit identities and certify only omitted parent edges (ADR 0019).

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use narrata_kernel::codec::{CborReader, CborWriter, DecodeError};
use narrata_storage::StorageBackend;

use crate::{
    BundleError, BundleLimits, CheckpointManifest, Commit, ContainerError, History, HistoryError,
    KindInfo, ManifestError, Object, ObjectId, ObjectSource, RefKey, RefMutation, RefRevision,
    RefValue, Reference, Registry, Transaction,
    bundle::{
        self, check_transmitted, decode_container, encode_container, gather, strictly_sorted,
    },
    engine::references,
    layout::{expect_map, key},
};

pub const SHALLOW_MAGIC: &[u8; 8] = b"NARCPB2\0";
pub const SHALLOW_MANIFEST_SCHEMA: u16 = 2;
/// Engine-owned, deterministic evidence that this parent's history was deliberately omitted.
pub const TRUNCATED_PARENT_KIND: u16 = 0x0010;
pub(crate) const TRUNCATED_PARENT_INFO: KindInfo = KindInfo {
    name: "Truncated history parent",
    leaf: true,
    commit: false,
};

pub(crate) fn truncated_parent(parent: ObjectId) -> Object {
    let mut writer = CborWriter::new();
    writer.map(1);
    writer.unsigned(0);
    writer.bytes(parent.as_bytes());
    Object::new(TRUNCATED_PARENT_KIND, 1, &writer.into_bytes())
}

pub(crate) fn validate_truncated_parent(object: &Object) -> Result<(), HistoryError> {
    let decode = || {
        let mut reader = CborReader::new(object.payload());
        expect_map(&mut reader, 1)?;
        key(&mut reader, 0)?;
        let parent = ObjectId::from_bytes(reader.bytes_exact::<32>()?);
        reader.finish()?;
        Ok::<_, DecodeError>(parent)
    };
    let parent = decode().map_err(|error| HistoryError::Corrupt(object.id(), error.to_string()))?;
    if *object != truncated_parent(parent) {
        return Err(HistoryError::ObjectKind(object.id()));
    }
    Ok(())
}

/// The parent edge of a registered commit in the generic Commit format. Other formats cannot
/// opt into truncation accidentally merely by referring to a missing object.
pub(crate) fn parent<R: Registry>(registry: &R, object: &Object) -> Option<ObjectId> {
    registry.kind(object.kind()).filter(|info| info.commit)?;
    Commit::from_object(object, object.kind()).ok()?.parent
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShallowManifest {
    pub checkpoint: CheckpointManifest,
    pub boundary_parents: Vec<ObjectId>,
}

impl ShallowManifest {
    pub fn validate(&self) -> Result<(), ManifestError> {
        self.checkpoint.validate()?;
        if !strictly_sorted(&self.boundary_parents)
            || self.boundary_parents.iter().any(|parent| {
                self.checkpoint
                    .objects
                    .iter()
                    .any(|object| object.id == *parent)
            })
        {
            return Err(DecodeError::Schema("shallow boundary parents").into());
        }
        Ok(())
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut writer = CborWriter::new();
        writer.map(5);
        writer.unsigned(0);
        writer.unsigned(2);
        writer.unsigned(1);
        writer.bytes(self.checkpoint.root.as_bytes());
        writer.unsigned(2);
        bundle::encode_descriptors(&mut writer, &self.checkpoint.objects);
        writer.unsigned(3);
        match self.checkpoint.host_manifest {
            Some(id) => writer.bytes(id.as_bytes()),
            None => writer.null(),
        }
        writer.unsigned(4);
        writer.array(self.boundary_parents.len() as u64);
        for parent in &self.boundary_parents {
            writer.bytes(parent.as_bytes());
        }
        writer.into_bytes()
    }

    pub fn decode(payload: &[u8]) -> Result<Self, ManifestError> {
        let mut reader = CborReader::new(payload);
        expect_map(&mut reader, 5)?;
        key(&mut reader, 0)?;
        if reader.unsigned()? != 2 {
            return Err(DecodeError::Schema("shallow manifest schema").into());
        }
        key(&mut reader, 1)?;
        let root = ObjectId::from_bytes(reader.bytes_exact::<32>()?);
        key(&mut reader, 2)?;
        let objects = bundle::decode_descriptors(&mut reader)?;
        key(&mut reader, 3)?;
        let host_manifest =
            reader.optional(|reader| Ok(ObjectId::from_bytes(reader.bytes_exact::<32>()?)))?;
        key(&mut reader, 4)?;
        let boundary_parents = bundle::decode_list(&mut reader, |reader| {
            Ok(ObjectId::from_bytes(reader.bytes_exact::<32>()?))
        })?;
        reader.finish()?;
        let value = Self {
            checkpoint: CheckpointManifest {
                root,
                objects,
                host_manifest,
            },
            boundary_parents,
        };
        value.validate()?;
        if value.encode() != payload {
            return Err(DecodeError::NonCanonical("round-trip mismatch").into());
        }
        Ok(value)
    }

    pub fn to_object(&self) -> Result<Object, ManifestError> {
        self.validate()?;
        Ok(Object::new(
            bundle::CHECKPOINT_MANIFEST_KIND,
            SHALLOW_MANIFEST_SCHEMA,
            &self.encode(),
        ))
    }
}

#[derive(Clone, Debug)]
pub struct ShallowBundle {
    pub manifest: ShallowManifest,
    pub objects: Vec<Object>,
}

impl ShallowBundle {
    /// Carries the target plus at most `ancestors` parents and all their non-parent dependencies.
    pub fn export<S: ObjectSource, R: Registry>(
        source: &S,
        registry: &R,
        root: ObjectId,
        ancestors: u64,
        receiver_has: &BTreeSet<ObjectId>,
    ) -> Result<Self, BundleError<S::Error>> {
        let mut boundary = BTreeSet::new();
        let mut current = root;
        let mut remaining = ancestors;
        loop {
            let object = source
                .objects(&[current])
                .map_err(BundleError::Store)?
                .into_iter()
                .next()
                .flatten()
                .ok_or(BundleError::MissingObject(current))?;
            if !registry.kind(object.kind()).is_some_and(|info| info.commit) {
                return Err(BundleError::Store(HistoryError::ObjectKind(current).into()));
            }
            let commit = Commit::from_object(&object, object.kind())
                .map_err(|error| BundleError::Store(error.into()))?;
            let Some(parent) = commit.parent else { break };
            if remaining == 0 {
                boundary.insert(parent);
                break;
            }
            if source
                .objects(&[parent])
                .map_err(BundleError::Store)?
                .into_iter()
                .next()
                .flatten()
                .is_none()
            {
                let marker = truncated_parent(parent);
                let stored = source.objects(&[marker.id()]).map_err(BundleError::Store)?;
                if stored.first().and_then(Option::as_ref) != Some(&marker) {
                    return Err(BundleError::MissingObject(parent));
                }
                boundary.insert(parent);
                break;
            }
            current = parent;
            remaining -= 1;
        }
        let mut objects = BTreeMap::new();
        let mut queue = VecDeque::from([root]);
        while let Some(id) = queue.pop_front() {
            if objects.contains_key(&id) {
                continue;
            }
            if objects.len() >= 1_000_000 {
                return Err(ContainerError::Limit("closure objects").into());
            }
            let object = source
                .objects(&[id])
                .map_err(BundleError::Store)?
                .into_iter()
                .next()
                .flatten()
                .ok_or(BundleError::MissingObject(id))?;
            for reference in
                references(registry, &object).map_err(|error| BundleError::Store(error.into()))?
            {
                match reference {
                    Reference::Object {
                        descriptor: true, ..
                    } => {}
                    Reference::Object { id: target, .. } => {
                        if parent(registry, &object) == Some(target) && boundary.contains(&target) {
                            continue;
                        }
                        queue.push_back(target);
                    }
                    Reference::Named { kind, name } => {
                        let target = source
                            .resolve(kind, &name)
                            .map_err(BundleError::Store)?
                            .ok_or(BundleError::Unresolved { kind })?;
                        let named = source.objects(&[target]).map_err(BundleError::Store)?;
                        if !named
                            .first()
                            .and_then(Option::as_ref)
                            .is_some_and(|object| {
                                object.kind() == kind && registry.name(object) == Some(name)
                            })
                        {
                            return Err(BundleError::Unresolved { kind });
                        }
                        queue.push_back(target);
                    }
                }
            }
            objects.insert(id, object);
        }
        let checkpoint = CheckpointManifest {
            root,
            objects: objects.values().map(crate::Descriptor::from).collect(),
            host_manifest: None,
        };
        let bundle = Self {
            manifest: ShallowManifest {
                checkpoint,
                boundary_parents: boundary.into_iter().collect(),
            },
            objects: objects
                .into_values()
                .filter(|object| !receiver_has.contains(&object.id()))
                .collect(),
        };
        bundle.check_closure(source, registry)?;
        Ok(bundle)
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, ContainerError> {
        encode_container(SHALLOW_MAGIC, &self.manifest.to_object()?, &self.objects)
    }

    pub fn from_bytes(
        bytes: &[u8],
        limits: BundleLimits,
        known: impl Fn(u16) -> bool,
    ) -> Result<Self, ContainerError> {
        let (manifest, objects) = decode_container(
            bytes,
            SHALLOW_MAGIC,
            bundle::CHECKPOINT_MANIFEST_KIND,
            SHALLOW_MANIFEST_SCHEMA,
            limits,
            &known,
        )?;
        let manifest = ShallowManifest::decode(manifest.payload())?;
        if manifest
            .checkpoint
            .objects
            .iter()
            .any(|descriptor| !known(descriptor.kind))
        {
            return Err(ContainerError::DescriptorMismatch);
        }
        check_transmitted(&manifest.checkpoint.objects, &objects)?;
        Ok(Self { manifest, objects })
    }

    pub fn check_closure<S: ObjectSource, R: Registry>(
        &self,
        source: &S,
        registry: &R,
    ) -> Result<BTreeMap<ObjectId, Object>, BundleError<S::Error>> {
        self.manifest.validate().map_err(ContainerError::from)?;
        let available = gather(source, &self.manifest.checkpoint.objects, &self.objects)?;
        let mut names = BTreeMap::new();
        for object in available.values() {
            if let Some(name) = registry.name(object) {
                names.entry((object.kind(), name)).or_insert(object.id());
            }
        }
        let declared_boundaries = self
            .manifest
            .boundary_parents
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        let mut used_boundaries = BTreeSet::new();
        let mut marked = BTreeSet::new();
        let mut queue = VecDeque::from([self.manifest.checkpoint.root]);
        while let Some(id) = queue.pop_front() {
            if !marked.insert(id) {
                continue;
            }
            let object = available.get(&id).ok_or(BundleError::MissingObject(id))?;
            for reference in
                references(registry, object).map_err(|error| BundleError::Store(error.into()))?
            {
                match reference {
                    Reference::Object {
                        descriptor: true, ..
                    } => {}
                    Reference::Object { id: target, .. } => {
                        if declared_boundaries.contains(&target)
                            && parent(registry, object) == Some(target)
                        {
                            used_boundaries.insert(target);
                        } else {
                            queue.push_back(target);
                        }
                    }
                    Reference::Named { kind, name } => queue.push_back(
                        *names
                            .get(&(kind, name))
                            .ok_or(BundleError::Unresolved { kind })?,
                    ),
                }
            }
        }
        if used_boundaries != declared_boundaries || marked != available.keys().copied().collect() {
            return Err(BundleError::ClosureMismatch);
        }
        Ok(available)
    }

    pub fn import<B: StorageBackend, R: Registry>(
        self,
        history: &mut History<B, R>,
        target: RefKey,
        expected: Option<RefRevision>,
        observed_at: u64,
    ) -> Result<RefValue, BundleError<R::Error>> {
        self.check_closure(history, history.registry())?;
        self.write(history, target, expected, observed_at)
    }

    pub(crate) fn write<B: StorageBackend, R: Registry>(
        self,
        history: &mut History<B, R>,
        target: RefKey,
        expected: Option<RefRevision>,
        observed_at: u64,
    ) -> Result<RefValue, BundleError<R::Error>> {
        let root = self.manifest.checkpoint.root;
        let mut objects = self.objects;
        objects.extend(
            self.manifest
                .boundary_parents
                .iter()
                .copied()
                .map(truncated_parent),
        );
        objects.push(self.manifest.to_object().map_err(ContainerError::from)?);
        let written = history
            .write(
                &Transaction {
                    objects,
                    refs: vec![RefMutation {
                        key: target,
                        expected,
                        next: Some(root),
                    }],
                    observed_at,
                    ..Transaction::default()
                },
                |_| Ok(Vec::new()),
            )
            .map_err(BundleError::Store)?;
        written
            .ref_value(Some(root))
            .map_err(|error| BundleError::Store(error.into()))?
            .ok_or(BundleError::MissingObject(root))
    }
}
