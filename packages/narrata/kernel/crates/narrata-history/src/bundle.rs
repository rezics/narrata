//! Checkpoint bundles: a root commit and its object closure as bytes a host can store or send
//! (ADR 0006, ADR 0015). The container and its manifest (kind 7) belong to the engine; the
//! kinds a bundle may carry are whatever the registry declares.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use narrata_kernel::codec::{CborReader, CborWriter, DecodeError};
use narrata_storage::StorageBackend;
use thiserror::Error;

use crate::{
    Descriptor, History, HistoryError, KindInfo, Object, ObjectError, ObjectId, RefKey,
    RefMutation, RefRevision, RefValue, Reference, Registry, Transaction, View,
    engine::{kind_info, references},
    layout::{expect_array, expect_map, key},
    shallow::{self, ShallowManifest},
};

/// The manifest kind code, kept from Stage 2 and owned by the engine.
pub const CHECKPOINT_MANIFEST_KIND: u16 = 7;
pub const CHECKPOINT_MANIFEST_SCHEMA: u16 = 1;
pub(crate) const CHECKPOINT_MANIFEST_INFO: KindInfo = KindInfo {
    name: "Checkpoint Bundle Manifest",
    leaf: false,
    commit: false,
};
pub const CHECKPOINT_MAGIC: &[u8; 8] = b"NARCPB1\0";

/// Most objects one closure may hold.
const MAX_CLOSURE: usize = 1_000_000;

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
pub enum ManifestError {
    #[error(transparent)]
    Wire(#[from] DecodeError),
    #[error("object descriptors must be sorted, unique, and match the declared count")]
    Descriptors,
}

/// What is wrong with bundle bytes on their own, before any store is consulted.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ContainerError {
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    #[error("bundle is truncated")]
    Truncated,
    #[error("wrong bundle discriminator")]
    WrongKind,
    #[error("bundle exceeds configured limit: {0}")]
    Limit(&'static str),
    #[error("bundle object is duplicated")]
    DuplicateObject,
    #[error("bundle object does not match its descriptor")]
    DescriptorMismatch,
    #[error(transparent)]
    Object(#[from] ObjectError),
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum BundleError<E> {
    #[error(transparent)]
    Container(#[from] ContainerError),
    #[error(transparent)]
    Store(E),
    #[error("bundle is missing required object {0}")]
    MissingObject(ObjectId),
    #[error("bundle closure does not exactly match the manifest")]
    ClosureMismatch,
    #[error("bundle object of kind {kind:#06x} named by another object is missing")]
    Unresolved { kind: u16 },
}

impl<E: From<HistoryError>> BundleError<E> {
    fn history(error: HistoryError) -> Self {
        Self::Store(error.into())
    }
}

/// The manifest of a checkpoint bundle: its root commit and every object of the root's closure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckpointManifest {
    pub root: ObjectId,
    pub objects: Vec<Descriptor>,
    pub host_manifest: Option<ObjectId>,
}

impl CheckpointManifest {
    pub fn validate(&self) -> Result<(), ManifestError> {
        validate_descriptors(&self.objects)
    }

    pub fn to_object(&self) -> Result<Object, ManifestError> {
        self.validate()?;
        Ok(Object::new(
            CHECKPOINT_MANIFEST_KIND,
            CHECKPOINT_MANIFEST_SCHEMA,
            &self.encode(),
        ))
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut writer = CborWriter::new();
        writer.map(4);
        writer.unsigned(0);
        writer.unsigned(u64::from(CHECKPOINT_MANIFEST_SCHEMA));
        writer.unsigned(1);
        writer.bytes(self.root.as_bytes());
        writer.unsigned(2);
        encode_descriptors(&mut writer, &self.objects);
        writer.unsigned(3);
        match self.host_manifest {
            Some(id) => writer.bytes(id.as_bytes()),
            None => writer.null(),
        }
        writer.into_bytes()
    }

    /// Decodes a manifest payload. Descriptor kinds are not checked here; the bundle reader
    /// checks them against the registry.
    pub fn decode(payload: &[u8]) -> Result<Self, ManifestError> {
        let mut reader = CborReader::new(payload);
        expect_map(&mut reader, 4)?;
        key(&mut reader, 0)?;
        if reader.unsigned()? != u64::from(CHECKPOINT_MANIFEST_SCHEMA) {
            return Err(DecodeError::Schema("Checkpoint manifest schema").into());
        }
        key(&mut reader, 1)?;
        let root = ObjectId::from_bytes(reader.bytes_exact::<32>()?);
        key(&mut reader, 2)?;
        let objects = decode_descriptors(&mut reader)?;
        key(&mut reader, 3)?;
        let host_manifest =
            reader.optional(|reader| Ok(ObjectId::from_bytes(reader.bytes_exact::<32>()?)))?;
        reader.finish()?;
        let value = Self {
            root,
            objects,
            host_manifest,
        };
        value.validate()?;
        if value.encode() != payload {
            return Err(DecodeError::NonCanonical("round-trip mismatch").into());
        }
        Ok(value)
    }
}

pub fn strictly_sorted<T: Ord>(values: &[T]) -> bool {
    values.windows(2).all(|pair| pair[0] < pair[1])
}

pub fn validate_descriptors(values: &[Descriptor]) -> Result<(), ManifestError> {
    if strictly_sorted(values) {
        Ok(())
    } else {
        Err(ManifestError::Descriptors)
    }
}

pub fn encode_descriptors(writer: &mut CborWriter, descriptors: &[Descriptor]) {
    writer.array(descriptors.len() as u64);
    for descriptor in descriptors {
        writer.array(4);
        writer.bytes(descriptor.id.as_bytes());
        writer.unsigned(u64::from(descriptor.kind));
        writer.unsigned(u64::from(descriptor.schema));
        writer.unsigned(descriptor.bytes);
    }
}

pub fn decode_descriptors(reader: &mut CborReader<'_>) -> Result<Vec<Descriptor>, DecodeError> {
    decode_list(reader, |reader| {
        expect_array(reader, 4)?;
        let id = ObjectId::from_bytes(reader.bytes_exact::<32>()?);
        let kind = u16::try_from(reader.unsigned()?).map_err(|_| DecodeError::IntegerOverflow)?;
        let schema = u16::try_from(reader.unsigned()?).map_err(|_| DecodeError::IntegerOverflow)?;
        let bytes = reader.unsigned()?;
        Ok(Descriptor {
            id,
            kind,
            schema,
            bytes,
        })
    })
}

/// Decodes a CBOR array of at most a million items.
pub fn decode_list<'a, T>(
    reader: &mut CborReader<'a>,
    mut decode: impl FnMut(&mut CborReader<'a>) -> Result<T, DecodeError>,
) -> Result<Vec<T>, DecodeError> {
    let length = reader.array_len()?;
    if length > 1_000_000 {
        return Err(DecodeError::Limit("manifest list"));
    }
    let mut values =
        Vec::with_capacity(usize::try_from(length).map_err(|_| DecodeError::LengthOverflow)?);
    for _ in 0..length {
        values.push(decode(reader)?);
    }
    Ok(values)
}

pub(crate) fn manifest_references(object: &Object) -> Result<Vec<Reference>, HistoryError> {
    if object.schema() == shallow::SHALLOW_MANIFEST_SCHEMA {
        let manifest = ShallowManifest::decode(object.payload())
            .map_err(|error| HistoryError::Corrupt(object.id(), error.to_string()))?;
        let mut values = vec![Reference::untyped(manifest.checkpoint.root)];
        values.extend(
            manifest
                .checkpoint
                .objects
                .iter()
                .map(|descriptor| Reference::descriptor(descriptor.id, descriptor.kind)),
        );
        values.extend(manifest.checkpoint.host_manifest.map(Reference::untyped));
        values.extend(manifest.boundary_parents.into_iter().map(|parent| {
            Reference::object(
                shallow::truncated_parent(parent).id(),
                shallow::TRUNCATED_PARENT_KIND,
            )
        }));
        return Ok(values);
    }
    let manifest = CheckpointManifest::decode(object.payload())
        .map_err(|error| HistoryError::Corrupt(object.id(), error.to_string()))?;
    let mut values = vec![Reference::untyped(manifest.root)];
    values.extend(
        manifest
            .objects
            .iter()
            .map(|descriptor| Reference::descriptor(descriptor.id, descriptor.kind)),
    );
    values.extend(manifest.host_manifest.map(Reference::untyped));
    Ok(values)
}

pub(crate) fn validate_manifest<B: StorageBackend, R: Registry>(
    view: &mut View<'_, B, R>,
    object: &Object,
) -> Result<(), HistoryError> {
    let id = object.id();
    if object.schema() != CHECKPOINT_MANIFEST_SCHEMA
        && object.schema() != shallow::SHALLOW_MANIFEST_SCHEMA
    {
        return Err(HistoryError::ObjectKind(id));
    }
    let manifest = if object.schema() == shallow::SHALLOW_MANIFEST_SCHEMA {
        ShallowManifest::decode(object.payload()).map(|manifest| manifest.checkpoint)
    } else {
        CheckpointManifest::decode(object.payload())
    }
    .map_err(|error| HistoryError::Corrupt(id, error.to_string()))?;
    let root = view.require(manifest.root, None)?;
    if !view.kind(root.kind()).is_some_and(|info| info.commit) {
        return Err(HistoryError::ObjectKind(manifest.root));
    }
    view.check_descriptors(&manifest.objects)
}

/// Where a bundle's objects come from when it is exported or checked against a store.
pub trait ObjectSource {
    type Error: From<HistoryError>;

    fn objects(&self, ids: &[ObjectId]) -> Result<Vec<Option<Object>>, Self::Error>;

    /// The stored object of `kind` named `name`, through the registrant's index.
    fn resolve(&self, kind: u16, name: &[u8; 32]) -> Result<Option<ObjectId>, Self::Error>;
}

impl<B: StorageBackend, R: Registry> ObjectSource for History<B, R> {
    type Error = R::Error;

    fn objects(&self, ids: &[ObjectId]) -> Result<Vec<Option<Object>>, R::Error> {
        self.get_objects(ids)
    }

    fn resolve(&self, kind: u16, name: &[u8; 32]) -> Result<Option<ObjectId>, R::Error> {
        Ok(self.registry().resolve(&self.reader(), kind, name)?)
    }
}

/// `magic ‖ manifest length ‖ manifest ‖ count ‖ (id ‖ kind ‖ schema ‖ length ‖ bytes)*`, big
/// endian, objects in ascending id order.
pub fn encode_container<'a>(
    magic: &[u8; 8],
    manifest: &Object,
    objects: impl IntoIterator<Item = &'a Object>,
) -> Result<Vec<u8>, ContainerError> {
    let mut sorted = objects.into_iter().collect::<Vec<_>>();
    sorted.sort_by_key(|object| object.id());
    if sorted.windows(2).any(|pair| pair[0].id() == pair[1].id()) {
        return Err(ContainerError::DuplicateObject);
    }
    let mut bytes = Vec::new();
    bytes.extend_from_slice(magic);
    push_u64(&mut bytes, manifest.bytes().len() as u64);
    bytes.extend_from_slice(manifest.bytes());
    push_u64(&mut bytes, sorted.len() as u64);
    for object in sorted {
        bytes.extend_from_slice(object.id().as_bytes());
        bytes.extend_from_slice(&object.kind().to_be_bytes());
        bytes.extend_from_slice(&object.schema().to_be_bytes());
        push_u64(&mut bytes, object.bytes().len() as u64);
        bytes.extend_from_slice(object.bytes());
    }
    Ok(bytes)
}

/// Splits container bytes into a checked manifest object and checked objects of known kinds.
pub fn decode_container(
    bytes: &[u8],
    magic: &[u8; 8],
    manifest_kind: u16,
    manifest_schema: u16,
    limits: BundleLimits,
    known: impl Fn(u16) -> bool,
) -> Result<(Object, Vec<Object>), ContainerError> {
    if bytes.len() as u64 > limits.max_total_bytes {
        return Err(ContainerError::Limit("total bytes"));
    }
    let mut cursor = BinaryCursor::new(bytes);
    if cursor.take(8)? != magic {
        return Err(ContainerError::WrongKind);
    }
    let manifest_len = cursor.u64()?;
    if manifest_len > limits.max_object_bytes {
        return Err(ContainerError::Limit("manifest bytes"));
    }
    let manifest = Object::from_bytes(
        cursor.take(to_usize(manifest_len)?)?,
        manifest_kind,
        manifest_schema,
        limits.max_object_bytes,
    )?;
    let count = cursor.u64()?;
    if count > limits.max_objects {
        return Err(ContainerError::Limit("object count"));
    }
    let mut objects = Vec::with_capacity(to_usize(count)?);
    let mut previous = None;
    for _ in 0..count {
        let declared_id = ObjectId::from_bytes(cursor.array::<32>()?);
        let kind = cursor.u16()?;
        if !known(kind) {
            return Err(ContainerError::DescriptorMismatch);
        }
        let schema = cursor.u16()?;
        let length = cursor.u64()?;
        if length > limits.max_object_bytes {
            return Err(ContainerError::Limit("object bytes"));
        }
        let object = Object::from_bytes(
            cursor.take(to_usize(length)?)?,
            kind,
            schema,
            limits.max_object_bytes,
        )?;
        if object.id() != declared_id {
            return Err(ContainerError::DescriptorMismatch);
        }
        if previous.is_some_and(|id| id >= object.id()) {
            return Err(ContainerError::DuplicateObject);
        }
        previous = Some(object.id());
        objects.push(object);
    }
    if !cursor.finished() {
        return Err(ContainerError::DescriptorMismatch);
    }
    Ok((manifest, objects))
}

/// Every transmitted object must be declared, with its kind, schema and size.
pub fn check_transmitted(
    descriptors: &[Descriptor],
    objects: &[Object],
) -> Result<(), ContainerError> {
    let declared = descriptors
        .iter()
        .map(|descriptor| (descriptor.id, descriptor))
        .collect::<BTreeMap<_, _>>();
    for object in objects {
        match declared.get(&object.id()) {
            Some(descriptor) if **descriptor == Descriptor::from(object) => {}
            _ => return Err(ContainerError::DescriptorMismatch),
        }
    }
    Ok(())
}

/// Every declared object, from the bundle or else from `source`, checked against its descriptor.
pub fn gather<S: ObjectSource>(
    source: &S,
    descriptors: &[Descriptor],
    transmitted: &[Object],
) -> Result<BTreeMap<ObjectId, Object>, BundleError<S::Error>> {
    let incoming = transmitted
        .iter()
        .map(|object| (object.id(), object))
        .collect::<BTreeMap<_, _>>();
    let stored_ids = descriptors
        .iter()
        .map(|descriptor| descriptor.id)
        .filter(|id| !incoming.contains_key(id))
        .collect::<Vec<_>>();
    let mut stored = stored_ids
        .iter()
        .copied()
        .zip(source.objects(&stored_ids).map_err(BundleError::Store)?)
        .collect::<BTreeMap<_, _>>();
    let mut available = BTreeMap::new();
    for descriptor in descriptors {
        let object = match incoming.get(&descriptor.id) {
            Some(object) => (*object).clone(),
            None => stored
                .remove(&descriptor.id)
                .flatten()
                .ok_or(BundleError::MissingObject(descriptor.id))?,
        };
        if Descriptor::from(&object) != *descriptor {
            return Err(ContainerError::DescriptorMismatch.into());
        }
        available.insert(descriptor.id, object);
    }
    Ok(available)
}

/// References that belong to a bundle closure. Manifest descriptors describe a closure and are
/// not part of one.
fn closure_references<R: Registry>(
    registry: &R,
    object: &Object,
) -> Result<Vec<Reference>, HistoryError> {
    Ok(references(registry, object)?
        .into_iter()
        .filter(|reference| {
            !matches!(
                reference,
                Reference::Object {
                    descriptor: true,
                    ..
                }
            )
        })
        .collect())
}

/// The closure of `roots` read from `source`, one reference level per read.
pub fn stored_closure<S: ObjectSource, R: Registry>(
    source: &S,
    registry: &R,
    roots: &[ObjectId],
) -> Result<BTreeMap<ObjectId, Object>, BundleError<S::Error>> {
    let mut objects = BTreeMap::new();
    let mut named = BTreeMap::new();
    let mut frontier = roots.to_vec();
    while !frontier.is_empty() {
        let wanted = frontier
            .drain(..)
            .filter(|id| !objects.contains_key(id))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        if objects.len() + wanted.len() > MAX_CLOSURE {
            return Err(ContainerError::Limit("closure objects").into());
        }
        let found = source.objects(&wanted).map_err(BundleError::Store)?;
        for (id, object) in wanted.iter().zip(found) {
            let object = match object {
                Some(object) => object,
                None => {
                    let marker = shallow::truncated_parent(*id);
                    let stored = source.objects(&[marker.id()]).map_err(BundleError::Store)?;
                    if stored.first().and_then(Option::as_ref) == Some(&marker) {
                        return Err(BundleError::history(HistoryError::HistoryTruncated(*id)));
                    }
                    return Err(BundleError::MissingObject(*id));
                }
            };
            // The registrant's index is a hint: the object it names must carry the name.
            if let Some((kind, name)) = named.get(id)
                && (object.kind() != *kind || registry.name(&object).as_ref() != Some(name))
            {
                return Err(BundleError::Unresolved { kind: *kind });
            }
            for reference in closure_references(registry, &object).map_err(BundleError::history)? {
                frontier.push(match reference {
                    Reference::Object { id, .. } => id,
                    Reference::Named { kind, name } => {
                        let target = source
                            .resolve(kind, &name)
                            .map_err(BundleError::Store)?
                            .ok_or(BundleError::Unresolved { kind })?;
                        named.insert(target, (kind, name));
                        target
                    }
                });
            }
            objects.insert(*id, object);
        }
    }
    Ok(objects)
}

/// The closure of `roots` among `objects`, which must hold all of it.
pub fn closure<E: From<HistoryError>, R: Registry>(
    registry: &R,
    objects: &BTreeMap<ObjectId, Object>,
    roots: &[ObjectId],
) -> Result<BTreeSet<ObjectId>, BundleError<E>> {
    let mut names = BTreeMap::new();
    for (id, object) in objects {
        if let Some(name) = registry.name(object) {
            names.entry((object.kind(), name)).or_insert(*id);
        }
    }
    let mut marked = BTreeSet::new();
    let mut queue = VecDeque::from(roots.to_vec());
    while let Some(id) = queue.pop_front() {
        if !marked.insert(id) {
            continue;
        }
        if marked.len() > MAX_CLOSURE {
            return Err(ContainerError::Limit("closure objects").into());
        }
        let object = objects.get(&id).ok_or(BundleError::MissingObject(id))?;
        for reference in closure_references(registry, object).map_err(BundleError::history)? {
            queue.push_back(match reference {
                Reference::Object { id, .. } => id,
                Reference::Named { kind, name } => *names
                    .get(&(kind, name))
                    .ok_or(BundleError::Unresolved { kind })?,
            });
        }
    }
    Ok(marked)
}

pub fn descriptors<E>(
    objects: &BTreeMap<ObjectId, Object>,
    ids: &BTreeSet<ObjectId>,
) -> Result<Vec<Descriptor>, BundleError<E>> {
    ids.iter()
        .map(|id| {
            objects
                .get(id)
                .map(Descriptor::from)
                .ok_or(BundleError::MissingObject(*id))
        })
        .collect()
}

/// One root commit with its closure; objects the receiver already has are declared, not sent.
#[derive(Clone, Debug)]
pub struct CheckpointBundle {
    pub manifest: CheckpointManifest,
    pub objects: Vec<Object>,
}

impl CheckpointBundle {
    pub fn export<S: ObjectSource, R: Registry>(
        source: &S,
        registry: &R,
        root: ObjectId,
        receiver_has: &BTreeSet<ObjectId>,
    ) -> Result<Self, BundleError<S::Error>> {
        let objects = stored_closure(source, registry, &[root])?;
        let closure = objects.keys().copied().collect::<BTreeSet<_>>();
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
            manifest: CheckpointManifest {
                root,
                objects: descriptors,
                host_manifest: None,
            },
            objects: transmitted,
        })
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, ContainerError> {
        encode_container(CHECKPOINT_MAGIC, &self.manifest.to_object()?, &self.objects)
    }

    /// Reads untrusted bundle bytes. Every object must have a kind `known` accepts, a valid
    /// envelope that hashes to its declared id, and a matching descriptor.
    pub fn from_bytes(
        bytes: &[u8],
        limits: BundleLimits,
        known: impl Fn(u16) -> bool,
    ) -> Result<Self, ContainerError> {
        let (manifest_object, objects) = decode_container(
            bytes,
            CHECKPOINT_MAGIC,
            CHECKPOINT_MANIFEST_KIND,
            CHECKPOINT_MANIFEST_SCHEMA,
            limits,
            &known,
        )?;
        let manifest = CheckpointManifest::decode(manifest_object.payload())?;
        if manifest
            .objects
            .iter()
            .any(|descriptor| !known(descriptor.kind))
        {
            return Err(ContainerError::DescriptorMismatch);
        }
        check_transmitted(&manifest.objects, &objects)?;
        Ok(Self { manifest, objects })
    }

    /// Checks that the declared objects, from the bundle or from `source`, are exactly the
    /// root's closure, and returns them.
    pub fn check_closure<S: ObjectSource, R: Registry>(
        &self,
        source: &S,
        registry: &R,
    ) -> Result<BTreeMap<ObjectId, Object>, BundleError<S::Error>> {
        let available = gather(source, &self.manifest.objects, &self.objects)?;
        let closure = closure(registry, &available, &[self.manifest.root])?;
        let declared = self
            .manifest
            .objects
            .iter()
            .map(|descriptor| descriptor.id)
            .collect::<BTreeSet<_>>();
        if closure != declared {
            return Err(BundleError::ClosureMismatch);
        }
        Ok(available)
    }

    /// Checks the bundle against `history` and writes it, pointing `target` at the root. The
    /// engine validates every carried object as for any other write.
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

    /// Writes a bundle whose closure was checked.
    pub(crate) fn write<B: StorageBackend, R: Registry>(
        self,
        history: &mut History<B, R>,
        target: RefKey,
        expected: Option<RefRevision>,
        observed_at: u64,
    ) -> Result<RefValue, BundleError<R::Error>> {
        let manifest = self.manifest.to_object().map_err(ContainerError::from)?;
        let root = self.manifest.root;
        let mut objects = self.objects;
        objects.push(manifest);
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
            .map_err(BundleError::history)?
            .ok_or(BundleError::MissingObject(root))
    }
}

/// Whether `history` can store objects of kind `code`.
pub fn registered<B: StorageBackend, R: Registry>(history: &History<B, R>, code: u16) -> bool {
    kind_info(history.registry(), code).is_some()
}

fn push_u64(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&value.to_be_bytes());
}

fn to_usize(value: u64) -> Result<usize, ContainerError> {
    usize::try_from(value).map_err(|_| ContainerError::Limit("platform length"))
}

struct BinaryCursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> BinaryCursor<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], ContainerError> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or(ContainerError::Truncated)?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or(ContainerError::Truncated)?;
        self.offset = end;
        Ok(value)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], ContainerError> {
        self.take(N)?
            .try_into()
            .map_err(|_| ContainerError::Truncated)
    }

    fn u16(&mut self) -> Result<u16, ContainerError> {
        Ok(u16::from_be_bytes(self.array()?))
    }

    fn u64(&mut self) -> Result<u64, ContainerError> {
        Ok(u64::from_be_bytes(self.array()?))
    }

    fn finished(&self) -> bool {
        self.offset == self.bytes.len()
    }
}
