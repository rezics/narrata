use std::collections::{BTreeMap, BTreeSet};

use narrata_storage::{KeySpace, StorageBackend};

use super::{Reader, kind_info};
use crate::{Descriptor, HistoryError, KindInfo, Object, ObjectId, Registry};

/// The objects a write can see: its own batch first, then the store.
///
/// Write validation reads only what the new objects refer to. Objects already stored passed the
/// same checks, so every stored object's references are stored too (ADR 0014, invariant I), and
/// no write reads further than one step.
pub struct View<'a, B, R> {
    reader: Reader<'a, B>,
    registry: &'a R,
    staged: &'a BTreeMap<ObjectId, Object>,
    staged_names: BTreeMap<(u16, [u8; 32]), ObjectId>,
    fetched: BTreeMap<ObjectId, Option<Object>>,
    resolved: BTreeMap<(u16, [u8; 32]), Option<ObjectId>>,
    planned: BTreeMap<(KeySpace, Vec<u8>), Vec<u8>>,
}

impl<'a, B: StorageBackend, R: Registry> View<'a, B, R> {
    pub(super) fn new(
        reader: Reader<'a, B>,
        registry: &'a R,
        staged: &'a BTreeMap<ObjectId, Object>,
    ) -> Self {
        let mut staged_names = BTreeMap::new();
        for object in staged.values() {
            if let Some(name) = registry.name(object) {
                staged_names
                    .entry((object.kind(), name))
                    .or_insert(object.id());
            }
        }
        Self {
            reader,
            registry,
            staged,
            staged_names,
            fetched: BTreeMap::new(),
            resolved: BTreeMap::new(),
            planned: BTreeMap::new(),
        }
    }

    pub fn reader(&self) -> &Reader<'a, B> {
        &self.reader
    }

    pub fn registry(&self) -> &'a R {
        self.registry
    }

    pub fn kind(&self, code: u16) -> Option<KindInfo> {
        kind_info(self.registry, code)
    }

    /// The objects the write adds, by identity.
    pub fn staged(&self) -> &'a BTreeMap<ObjectId, Object> {
        self.staged
    }

    /// The staged object of `kind` its registrant names `name`.
    pub fn staged_named(&self, kind: u16, name: &[u8; 32]) -> Option<ObjectId> {
        self.staged_names.get(&(kind, *name)).copied()
    }

    /// Reads the stored objects among `ids` in as few backend calls as the read limit allows.
    pub fn prefetch(
        &mut self,
        ids: impl IntoIterator<Item = ObjectId>,
    ) -> Result<(), HistoryError> {
        let wanted = ids
            .into_iter()
            .filter(|id| !self.staged.contains_key(id) && !self.fetched.contains_key(id))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let found = self.reader.objects(&wanted)?;
        self.fetched.extend(wanted.into_iter().zip(found));
        Ok(())
    }

    pub fn object(&mut self, id: ObjectId) -> Result<Option<Object>, HistoryError> {
        if let Some(object) = self.staged.get(&id) {
            return Ok(Some(object.clone()));
        }
        self.prefetch([id])?;
        Ok(self.fetched.get(&id).cloned().flatten())
    }

    pub fn require(&mut self, id: ObjectId, kind: Option<u16>) -> Result<Object, HistoryError> {
        let object = self.object(id)?.ok_or(HistoryError::MissingObject(id))?;
        if kind.is_none_or(|kind| object.kind() == kind) {
            Ok(object)
        } else {
            Err(HistoryError::ObjectKind(id))
        }
    }

    /// Finds the object of `kind` named `name`: in the batch, or through the registrant's index,
    /// whose entry is checked against the object it names because the index is only a hint.
    pub fn resolve(&mut self, kind: u16, name: &[u8; 32]) -> Result<Option<ObjectId>, R::Error> {
        if let Some(id) = self.staged_named(kind, name) {
            return Ok(Some(id));
        }
        if let Some(resolved) = self.resolved.get(&(kind, *name)) {
            return Ok(*resolved);
        }
        let resolved = match self.registry.resolve(&self.reader, kind, name)? {
            Some(id) => {
                let object = self.object(id)?.ok_or(HistoryError::Unresolved { kind })?;
                if object.kind() != kind || self.registry.name(&object).as_ref() != Some(name) {
                    return Err(HistoryError::CorruptStore(format!(
                        "index entry {id} does not hold the object of kind {kind:#06x} named {}",
                        hex::encode(name)
                    ))
                    .into());
                }
                Some(id)
            }
            None => None,
        };
        self.resolved.insert((kind, *name), resolved);
        Ok(resolved)
    }

    /// The value this write already puts under an index key, if another object of the batch
    /// planned it.
    pub fn planned(&self, space: KeySpace, key: &[u8]) -> Option<&[u8]> {
        self.planned.get(&(space, key.to_vec())).map(Vec::as_slice)
    }

    /// Records that this write puts `value` under an index key.
    pub fn plan(&mut self, space: KeySpace, key: Vec<u8>, value: Vec<u8>) {
        self.planned.insert((space, key), value);
    }

    /// Checks manifest descriptors against the objects they describe.
    pub fn check_descriptors(&mut self, descriptors: &[Descriptor]) -> Result<(), HistoryError> {
        for descriptor in descriptors {
            let object = self.require(descriptor.id, None)?;
            if Descriptor::from(&object) != *descriptor {
                return Err(HistoryError::InvalidGraph("manifest descriptor mismatch"));
            }
        }
        Ok(())
    }
}
