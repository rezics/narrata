//! The history engine: one implementation of object reads and writes, Refs, Pins, the children
//! and transitions indexes, GC and the integrity scan over any [`StorageBackend`], for whatever
//! kinds its [`Registry`] declares (ADR 0014, ADR 0015).
//!
//! Every object read is re-verified against its identity and every key value is decoded
//! strictly, so a backend can lose data but cannot make the engine accept data it did not write.

mod gc;
mod view;
mod write;

use std::fmt;

pub use gc::{GcKindReport, GcReport, IntegrityIssue, RetentionPolicy};
use narrata_storage::{
    Batch, Expect, KeyEntry, KeyPage, KeySpace, KeyValue, Limits, MemoryBackend, ObjectDigest,
    Revision, StorageBackend, StorageError,
};
pub use view::View;
pub use write::{
    Attempt, Op, Ops, RefMutation, Tag, Transaction, Written, expect, graph_bump, retry,
    sweep_check,
};

use crate::{
    HistoryError, KindInfo, Object, ObjectId, Page, Pin, RefKey, RefRevision, RefScope, RefValue,
    Reference, Registry, bundle, layout, shallow,
};

/// Attempts of one operation before it reports [`HistoryError::Busy`].
pub(crate) const RETRIES: usize = 8;

/// The history engine over backend `B` for the kinds `R` registers.
pub struct History<B, R> {
    backend: B,
    registry: R,
    limits: Limits,
}

impl<B: fmt::Debug, R> fmt::Debug for History<B, R> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("History")
            .field("backend", &self.backend)
            .finish_non_exhaustive()
    }
}

impl<R: Registry> History<MemoryBackend, R> {
    /// A store in a new memory backend. Writing the layout marker there cannot fail; if it ever
    /// did, the store would still work and only lack its marker.
    pub fn in_memory(registry: R) -> Self {
        let mut backend = MemoryBackend::new();
        let _ = backend.apply(&Batch::new().put(
            layout::META,
            layout::LAYOUT_KEY,
            layout::encode_layout(),
            Expect::Absent,
        ));
        Self::assemble(backend, registry)
    }
}

impl<B: StorageBackend, R: Registry> History<B, R> {
    /// Opens a store in layout v1, initialising an empty backend. A backend holding anything
    /// else, including a newer layout, is refused with a format error and left untouched.
    pub fn open(mut backend: B, registry: R) -> Result<Self, R::Error> {
        if !backend.capabilities().atomic_multi_key() {
            return Err(HistoryError::from(StorageError::Invalid(
                "the history engine needs atomic multi-key batches",
            ))
            .into());
        }
        for _ in 0..RETRIES {
            let marker = backend
                .read_key(layout::META, layout::LAYOUT_KEY)
                .map_err(HistoryError::from)?;
            match marker {
                Some(value) => {
                    let version = layout::decode_layout(&value.value)?;
                    if version != layout::LAYOUT_VERSION {
                        return Err(HistoryError::from(StorageError::Format(format!(
                            "save layout version {version} is not the supported {}",
                            layout::LAYOUT_VERSION
                        )))
                        .into());
                    }
                    return Ok(Self::assemble(backend, registry));
                }
                None if is_empty(&backend, &registry)? => {
                    let marker = Batch::new().put(
                        layout::META,
                        layout::LAYOUT_KEY,
                        layout::encode_layout(),
                        Expect::Absent,
                    );
                    match backend.apply(&marker) {
                        Ok(_) | Err(StorageError::Conflict(_)) => {}
                        Err(error) if error.outcome_unknown() => {}
                        Err(error) => return Err(HistoryError::from(error).into()),
                    }
                }
                None => {
                    return Err(HistoryError::from(StorageError::Format(
                        "backend holds data but no save layout marker".to_owned(),
                    ))
                    .into());
                }
            }
        }
        Err(HistoryError::Busy.into())
    }

    fn assemble(backend: B, registry: R) -> Self {
        let limits = backend.capabilities().limits;
        Self {
            backend,
            registry,
            limits,
        }
    }

    pub fn backend(&self) -> &B {
        &self.backend
    }

    pub fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }

    pub fn into_backend(self) -> B {
        self.backend
    }

    pub fn registry(&self) -> &R {
        &self.registry
    }

    pub fn reader(&self) -> Reader<'_, B> {
        Reader {
            backend: &self.backend,
            read_items: read_items(self.limits),
        }
    }

    /// Whether the store holds no object and no key outside `meta`.
    pub fn is_empty_except_meta(&self) -> Result<bool, R::Error> {
        Ok(is_empty_except_meta(&self.backend, &self.registry)?)
    }

    /// What the engine knows about kind `code`: the checkpoint manifest, or a registered kind.
    pub fn kind(&self, code: u16) -> Option<KindInfo> {
        kind_info(&self.registry, code)
    }

    pub fn get_object(&self, id: ObjectId) -> Result<Option<Object>, R::Error> {
        Ok(self.reader().object(id)?)
    }

    pub fn get_objects(&self, ids: &[ObjectId]) -> Result<Vec<Option<Object>>, R::Error> {
        Ok(self.reader().objects(ids)?)
    }

    pub fn read_ref(&self, key: &RefKey) -> Result<Option<RefValue>, R::Error> {
        let reader = self.reader();
        Ok(reader
            .read_key(layout::REFS, &layout::ref_key(key))?
            .map(|value| {
                Ok::<_, HistoryError>(RefValue {
                    revision: RefRevision::from_revision(value.revision),
                    commit: layout::decode_ref_target(&value.value)?,
                })
            })
            .transpose()?)
    }

    pub fn scan_refs(
        &self,
        scope: &RefScope,
        after: Option<&RefKey>,
        limit: u32,
    ) -> Result<Page<(RefKey, RefValue)>, R::Error> {
        let prefix = match scope {
            RefScope::All => layout::ref_prefix(None, None),
            RefScope::Namespace(namespace) => layout::ref_prefix(Some(*namespace), None),
            RefScope::Owner(namespace, owner) => layout::ref_prefix(Some(*namespace), Some(owner)),
        };
        self.reader().page(
            layout::REFS,
            &prefix,
            after.map(layout::ref_key),
            limit,
            |entry| Ok::<_, R::Error>(ref_entry(entry)?),
        )
    }

    pub fn scan_pins(
        &self,
        after: Option<&(String, ObjectId)>,
        limit: u32,
    ) -> Result<Page<Pin>, R::Error> {
        let after = after
            .map(|(owner, object)| layout::pin_key(owner, *object))
            .transpose()?;
        self.reader()
            .page(layout::PINS, &[], after, limit, |entry| {
                Ok::<_, R::Error>(pin_entry(entry)?)
            })
    }

    /// Commits whose parent is `parent`, by commit ID.
    pub fn children(
        &self,
        parent: ObjectId,
        after: Option<&ObjectId>,
        limit: u32,
    ) -> Result<Page<ObjectId>, R::Error> {
        self.reader().page(
            layout::CHILDREN,
            parent.as_bytes(),
            after.map(|child| layout::child_key(parent, *child)),
            limit,
            |entry| {
                layout::decode_marker("child index value", &entry.value)?;
                Ok::<_, R::Error>(layout::decode_child_key(&entry.key)?.1)
            },
        )
    }

    /// The commit recorded for `input` applied after `parent`, if any.
    pub fn transition(
        &self,
        parent: ObjectId,
        input: ObjectId,
    ) -> Result<Option<ObjectId>, R::Error> {
        Ok(self
            .reader()
            .read_key(layout::TRANSITIONS, &layout::transition_key(parent, input))?
            .map(|value| layout::decode_transition(&value.value))
            .transpose()?)
    }
}

/// Read access to a store for registrants: every object comes back verified and every scan is
/// checked against the request, because the backend is not trusted.
#[derive(Clone, Copy, Debug)]
pub struct Reader<'a, B> {
    backend: &'a B,
    read_items: u32,
}

impl<'a, B: StorageBackend> Reader<'a, B> {
    pub fn backend(&self) -> &'a B {
        self.backend
    }

    /// Digests per object read and entries per scan page the backend serves at once.
    pub fn read_items(&self) -> u32 {
        self.read_items
    }

    pub fn read_key(&self, space: KeySpace, key: &[u8]) -> Result<Option<KeyValue>, HistoryError> {
        Ok(self.backend.read_key(space, key)?)
    }

    /// One page of keys, refused if the backend returns keys outside the request.
    pub fn scan(
        &self,
        space: KeySpace,
        prefix: &[u8],
        after: Option<&[u8]>,
        limit: u32,
    ) -> Result<KeyPage, HistoryError> {
        let page = self.backend.scan_keys(space, prefix, after, limit)?;
        let ordered = page
            .entries
            .windows(2)
            .all(|pair| pair[0].key < pair[1].key);
        let bounded = page.entries.iter().all(|entry| {
            entry.key.starts_with(prefix) && after.is_none_or(|after| entry.key.as_slice() > after)
        });
        if !ordered || !bounded || page.entries.len() > limit as usize {
            return Err(HistoryError::CorruptStore(
                "backend returned keys outside the requested scan".to_owned(),
            ));
        }
        Ok(page)
    }

    pub fn scan_every(
        &self,
        space: KeySpace,
        prefix: &[u8],
    ) -> Result<Vec<KeyEntry>, HistoryError> {
        let mut entries = Vec::new();
        let mut after: Option<Vec<u8>> = None;
        loop {
            let page = self.scan(space, prefix, after.as_deref(), self.read_items)?;
            let resume = page.resume_after().map(<[u8]>::to_vec);
            entries.extend(page.entries);
            match resume {
                Some(key) => after = Some(key),
                None => return Ok(entries),
            }
        }
    }

    /// A page of decoded entries; the limit is lowered to what the backend reads at once.
    pub fn page<T, E: From<HistoryError>>(
        &self,
        space: KeySpace,
        prefix: &[u8],
        after: Option<Vec<u8>>,
        limit: u32,
        map: impl Fn(&KeyEntry) -> Result<T, E>,
    ) -> Result<Page<T>, E> {
        let page = self.scan(
            space,
            prefix,
            after.as_deref(),
            limit.clamp(1, self.read_items),
        )?;
        Ok(Page {
            items: page.entries.iter().map(map).collect::<Result<_, _>>()?,
            more: page.more,
        })
    }

    pub fn objects(&self, ids: &[ObjectId]) -> Result<Vec<Option<Object>>, HistoryError> {
        let mut objects = Vec::with_capacity(ids.len());
        for chunk in ids.chunks(self.read_items as usize) {
            let digests = chunk.iter().copied().map(digest).collect::<Vec<_>>();
            let found = self.backend.get_objects(&digests)?;
            if found.len() != chunk.len() {
                return Err(HistoryError::CorruptStore(
                    "backend returned a misaligned object read".to_owned(),
                ));
            }
            for (id, bytes) in chunk.iter().zip(found) {
                objects.push(bytes.map(|bytes| Object::verify(*id, bytes)).transpose()?);
            }
        }
        Ok(objects)
    }

    pub fn object(&self, id: ObjectId) -> Result<Option<Object>, HistoryError> {
        Ok(self.objects(&[id])?.into_iter().next().flatten())
    }

    /// Reads a stored object of the given kind; the error names the missing or mismatched
    /// object.
    pub fn require(&self, id: ObjectId, kind: Option<u16>) -> Result<Object, HistoryError> {
        let object = match self.object(id)? {
            Some(object) => object,
            None if self.truncated_parent(id)?.is_some() => {
                return Err(HistoryError::HistoryTruncated(id));
            }
            None => return Err(HistoryError::MissingObject(id)),
        };
        if kind.is_none_or(|kind| object.kind() == kind) {
            Ok(object)
        } else {
            Err(HistoryError::ObjectKind(id))
        }
    }

    pub(crate) fn truncated_parent(
        &self,
        parent: ObjectId,
    ) -> Result<Option<Object>, HistoryError> {
        let expected = shallow::truncated_parent(parent);
        match self.object(expected.id())? {
            Some(object) if object == expected => Ok(Some(object)),
            Some(object) => Err(HistoryError::ObjectKind(object.id())),
            None => Ok(None),
        }
    }

    /// The revision of `meta/sweep` that writers check, so a GC deletion between their reads
    /// and their batch makes the batch conflict.
    pub fn sweep(&self) -> Result<Option<Revision>, HistoryError> {
        self.read_key(layout::META, layout::SWEEP_KEY)?
            .map(|value| {
                layout::decode_marker("sweep marker", &value.value)?;
                Ok(value.revision)
            })
            .transpose()
    }
}

fn read_items(limits: Limits) -> u32 {
    u32::try_from(limits.max_read_items)
        .unwrap_or(u32::MAX)
        .max(1)
}

pub(crate) const fn digest(id: ObjectId) -> ObjectDigest {
    ObjectDigest::from_bytes(*id.as_bytes())
}

pub(crate) fn kind_info<R: Registry>(registry: &R, code: u16) -> Option<KindInfo> {
    if code == bundle::CHECKPOINT_MANIFEST_KIND {
        Some(bundle::CHECKPOINT_MANIFEST_INFO)
    } else if code == shallow::TRUNCATED_PARENT_KIND {
        Some(shallow::TRUNCATED_PARENT_INFO)
    } else {
        registry.kind(code)
    }
}

/// The edges of a registered object, the engine's own kinds included.
pub(crate) fn references<R: Registry>(
    registry: &R,
    object: &Object,
) -> Result<Vec<Reference>, HistoryError> {
    let mut values = if object.kind() == shallow::TRUNCATED_PARENT_KIND {
        shallow::validate_truncated_parent(object)?;
        Vec::new()
    } else if object.kind() == bundle::CHECKPOINT_MANIFEST_KIND {
        bundle::manifest_references(object)?
    } else if registry.kind(object.kind()).is_some() {
        registry.references(object)?
    } else {
        return Err(unregistered(object));
    };
    values.sort_unstable();
    values.dedup();
    Ok(values)
}

pub(crate) const fn unregistered(object: &Object) -> HistoryError {
    HistoryError::UnregisteredKind {
        object: object.id(),
        kind: object.kind(),
    }
}

fn ref_entry(entry: &KeyEntry) -> Result<(RefKey, RefValue), HistoryError> {
    Ok((
        layout::decode_ref_key(&entry.key)?,
        RefValue {
            revision: RefRevision::from_revision(entry.revision),
            commit: layout::decode_ref_target(&entry.value)?,
        },
    ))
}

fn pin_entry(entry: &KeyEntry) -> Result<Pin, HistoryError> {
    let (owner, object) = layout::decode_pin_key(&entry.key)?;
    Ok(Pin {
        owner,
        object,
        expires_at: layout::decode_pin_value(&entry.value)?,
    })
}

fn is_empty<B: StorageBackend, R: Registry>(
    backend: &B,
    registry: &R,
) -> Result<bool, HistoryError> {
    Ok(backend
        .read_key(layout::META, layout::LAYOUT_KEY)?
        .is_none()
        && is_empty_except_meta(backend, registry)?
        && backend
            .scan_keys(layout::META, &[], None, 1)?
            .entries
            .is_empty())
}

fn is_empty_except_meta<B: StorageBackend, R: Registry>(
    backend: &B,
    registry: &R,
) -> Result<bool, HistoryError> {
    if !backend.scan_objects(None, 1)?.digests.is_empty() {
        return Ok(false);
    }
    let spaces = layout::SPACES[1..]
        .iter()
        .map(|(space, _)| *space)
        .chain(registry.spaces().iter().copied());
    for space in spaces {
        if !backend.scan_keys(space, &[], None, 1)?.entries.is_empty() {
            return Ok(false);
        }
    }
    Ok(true)
}
