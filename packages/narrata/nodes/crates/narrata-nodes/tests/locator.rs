#![allow(clippy::unwrap_used)]
mod support;

use narrata_history::{RefKey, RefName, layout};
use narrata_nodes::{
    Session,
    history::locator::{self, Limits},
};
use narrata_storage::{Batch, Expect, MemoryBackend, ObjectDigest, StorageBackend};

use locator::{ProofScope, RetentionProof};
use narrata_storage::{
    Applied, Capabilities, KeyPage, KeySpace, KeyValue, ObjectPage, StorageError,
};
use narrata_storage_host::{
    CacheBackend, LoadRequest, StoreId,
    testing::{Driven, MemoryHost},
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
};

struct Observed<B> {
    inner: B,
    headers: Rc<Cell<usize>>,
    writes: Rc<Cell<usize>>,
    after_active: RefCell<Option<Box<dyn FnOnce()>>>,
    active: Vec<u8>,
}
impl<B> Observed<B> {
    fn new(inner: B) -> Self {
        Self {
            inner,
            headers: Rc::new(Cell::new(0)),
            writes: Rc::new(Cell::new(0)),
            after_active: RefCell::new(None),
            active: layout::ref_key(
                &RefKey::active(RefName::hex(support::execution().as_bytes())).unwrap(),
            ),
        }
    }
}
impl<B: StorageBackend> StorageBackend for Observed<B> {
    fn capabilities(&self) -> Capabilities {
        self.inner.capabilities()
    }
    fn get_objects(&self, ids: &[ObjectDigest]) -> Result<Vec<Option<Arc<[u8]>>>, StorageError> {
        let result = self.inner.get_objects(ids)?;
        self.headers
            .set(self.headers.get() + result.iter().filter(|bytes| bytes.is_some()).count());
        Ok(result)
    }
    fn scan_objects(
        &self,
        after: Option<&ObjectDigest>,
        limit: u32,
    ) -> Result<ObjectPage, StorageError> {
        self.inner.scan_objects(after, limit)
    }
    fn read_key(&self, space: KeySpace, key: &[u8]) -> Result<Option<KeyValue>, StorageError> {
        let result = self.inner.read_key(space, key)?;
        if space == layout::REFS
            && key == self.active
            && let Some(action) = self.after_active.borrow_mut().take()
        {
            action();
        }
        Ok(result)
    }
    fn scan_keys(
        &self,
        space: KeySpace,
        prefix: &[u8],
        after: Option<&[u8]>,
        limit: u32,
    ) -> Result<KeyPage, StorageError> {
        self.inner.scan_keys(space, prefix, after, limit)
    }
    fn apply(&mut self, batch: &Batch) -> Result<Applied, StorageError> {
        self.writes.set(self.writes.get() + 1);
        self.inner.apply(batch)
    }
}

fn cold_chain(
    length: usize,
) -> (
    MemoryHost,
    Driven,
    narrata_nodes::CommitId,
    narrata_history::ObjectId,
) {
    let host = MemoryHost::new(StoreId::from_bytes([1; 16]));
    let mut writer = Driven::warm(host.share());
    let mut parent = None;
    let mut first = None;
    for start in (0..length).step_by(128) {
        let mut batch = Batch::new();
        for depth in start..length.min(start + 128) {
            let object = narrata_history::Commit {
                artifact: narrata_history::ArtifactId::from_bytes([1; 32]),
                parent,
                input: parent.map(|_| narrata_history::ObjectId::from_bytes([2; 32])),
                state: narrata_history::ObjectId::from_bytes([3; 32]),
                depth: depth as u64,
            }
            .to_object(narrata_nodes::KIND_COMMIT);
            first.get_or_insert(object.id());
            parent = Some(object.id());
            batch = batch.put_object(
                ObjectDigest::from_bytes(*object.id().as_bytes()),
                object.shared_bytes(),
            );
        }
        writer.apply(&batch).unwrap();
    }
    let head = parent.unwrap();
    let active =
        layout::ref_key(&RefKey::active(RefName::hex(support::execution().as_bytes())).unwrap());
    writer
        .apply(
            &Batch::new()
                .put(
                    layout::META,
                    layout::LAYOUT_KEY,
                    layout::encode_layout(),
                    Expect::Any,
                )
                .put(
                    layout::REFS,
                    active,
                    layout::encode_ref_target(head),
                    Expect::Any,
                ),
        )
        .unwrap();
    (
        host,
        writer,
        narrata_nodes::CommitId::from_bytes(*first.unwrap().as_bytes()),
        head,
    )
}

fn prove(
    proof: &mut RetentionProof,
    cache: &Observed<CacheBackend>,
    host: &MemoryHost,
) -> Result<bool, narrata_history::HistoryError> {
    for _ in 0..100_000 {
        let state = cache.inner.store().unwrap();
        match proof.poll(
            cache,
            ProofScope {
                store: *state.id.as_bytes(),
                revision: state.revision,
            },
        ) {
            Ok(result) => return Ok(result),
            Err(error) => {
                let Some(request) = cache.inner.take_request() else {
                    return Err(error);
                };
                cache.inner.load(&host.load(&request)).unwrap();
            }
        }
    }
    unreachable!("proof did not settle")
}

#[test]
fn cold_proof_verifies_a_deep_chain_once_across_cache_retries() {
    for length in [2048, Limits::default().commits] {
        let (host, _, candidate, _) = cold_chain(length);
        let cache = Observed::new(CacheBackend::new());
        cache
            .inner
            .load(&host.load(&LoadRequest::default()))
            .unwrap();
        let mut proof = RetentionProof::new(support::execution(), candidate, Limits::default());
        assert!(prove(&mut proof, &cache, &host).unwrap());
        assert_eq!(
            cache.headers.get(),
            length,
            "count every successful header read, including warm prefix rereads"
        );
        assert_eq!(proof.work().headers, cache.headers.get());
        assert!(proof.work().polls <= length + 10);
        assert_eq!(proof.work().roots, 1);
        assert_eq!(cache.writes.get(), 0);
    }
}

#[test]
fn revision_root_and_store_changes_discard_progress_without_resetting_total_work() {
    let (host, mut writer, candidate, head) = cold_chain(1024);
    let cache = Observed::new(CacheBackend::new());
    cache
        .inner
        .load(&host.load(&LoadRequest::default()))
        .unwrap();
    let mut proof = RetentionProof::new(support::execution(), candidate, Limits::default());
    loop {
        let state = cache.inner.store().unwrap();
        let result = proof.poll(
            &cache,
            ProofScope {
                store: *state.id.as_bytes(),
                revision: state.revision,
            },
        );
        if proof.work().headers >= 128 {
            break;
        }
        assert!(result.is_err());
        cache
            .inner
            .load(&host.load(&cache.inner.take_request().unwrap()))
            .unwrap();
    }
    writer
        .apply(&Batch::new().delete(layout::REFS, cache.active.clone(), Expect::Any))
        .unwrap();
    cache
        .inner
        .load(&host.load(&cache.inner.take_request().unwrap()))
        .unwrap();
    assert!(!prove(&mut proof, &cache, &host).unwrap());
    assert_eq!(proof.work().headers, 128);
    writer
        .apply(&Batch::new().put(
            layout::REFS,
            cache.active.clone(),
            layout::encode_ref_target(head),
            Expect::Any,
        ))
        .unwrap();
    cache
        .inner
        .load(&host.load(&LoadRequest::default()))
        .unwrap();
    assert!(prove(&mut proof, &cache, &host).unwrap());
    assert_eq!(cache.headers.get(), 1024 + 128);
    host.evict(StoreId::from_bytes([9; 16]));
    cache
        .inner
        .load(&host.load(&LoadRequest::default()))
        .unwrap();
    assert!(!prove(&mut proof, &cache, &host).unwrap());
    assert_eq!(proof.work().headers, cache.headers.get());
    assert_eq!(proof.work().resets, 4);
}

#[test]
fn registry_reopening_cannot_resurrect_a_deleted_or_moved_cursor() {
    for change in ["delete", "move", "evict"] {
        let backend = MemoryBackend::new();
        let (program, names) = support::open(&support::compiled().pack);
        let mut session =
            Session::open(program.clone(), support::execution(), backend.share()).unwrap();
        let original = session.cursor().unwrap();
        support::choose(&mut session, &names, &["wave"]).unwrap();
        let mut registry = narrata_nodes::history::evolution::EvolutionRegistry::new();
        registry.register_program(program).unwrap();
        let observed = Observed::new(backend.share());
        let mut shared = backend.share();
        let key = observed.active.clone();
        *observed.after_active.borrow_mut() = Some(Box::new(move || {
            if change == "evict" {
                let mut batch = Batch::new();
                for (space, _) in layout::SPACES {
                    for entry in shared.scan_keys(space, &[], None, 1024).unwrap().entries {
                        batch = batch.delete(space, entry.key, Expect::Any);
                    }
                }
                for id in shared.scan_objects(None, 1024).unwrap().digests {
                    batch = batch.delete_object(id);
                }
                shared.apply(&batch).unwrap();
                return;
            }
            let batch = if change == "move" {
                Batch::new().put(
                    layout::REFS,
                    key,
                    layout::encode_ref_target(narrata_history::ObjectId::from_bytes(
                        *original.as_bytes(),
                    )),
                    Expect::Any,
                )
            } else {
                Batch::new().delete(layout::REFS, key, Expect::Any)
            };
            shared.apply(&batch).unwrap();
        }));
        let writes = observed.writes.clone();
        let active_key = observed.active.clone();
        let result = Session::open_with_registry(&registry, support::execution(), observed);
        assert!(result.is_err());
        assert_eq!(writes.get(), 0);
        let active = backend.read_key(layout::REFS, &active_key).unwrap();
        if change == "move" {
            assert_eq!(
                layout::decode_ref_target(&active.unwrap().value)
                    .unwrap()
                    .as_bytes(),
                original.as_bytes()
            );
        } else {
            assert!(active.is_none());
            if change == "evict" {
                assert!(
                    backend
                        .read_key(layout::META, layout::LAYOUT_KEY)
                        .unwrap()
                        .is_none()
                );
            }
        }
    }
}

#[test]
fn cumulative_header_budget_bounds_revision_churn_across_all_host_retries() {
    let (host, mut writer, candidate, _) = cold_chain(64);
    let cache = Observed::new(CacheBackend::new());
    cache
        .inner
        .load(&host.load(&LoadRequest::default()))
        .unwrap();
    let mut proof = RetentionProof::new(
        support::execution(),
        candidate,
        Limits {
            roots: 4096,
            commits: 128,
        },
    );
    let mut epoch = 0;
    loop {
        let state = cache.inner.store().unwrap();
        let error = proof
            .poll(
                &cache,
                ProofScope {
                    store: *state.id.as_bytes(),
                    revision: state.revision,
                },
            )
            .unwrap_err();
        let Some(request) = cache.inner.take_request() else {
            assert!(error.to_string().contains("budget"));
            break;
        };
        if proof.work().headers >= (epoch + 1) * 16 {
            epoch += 1;
            writer
                .apply(&Batch::new().put(
                    layout::META,
                    b"proof-observation",
                    epoch.to_be_bytes().to_vec(),
                    Expect::Any,
                ))
                .unwrap();
        }
        cache.inner.load(&host.load(&request)).unwrap();
    }
    assert_eq!(cache.headers.get(), 512);
    assert_eq!(proof.work().headers, 512);
    assert!(proof.work().resets > 4);
}

#[test]
fn discovery_is_read_only_and_distinguishes_missing_corrupt_and_new() {
    let mut backend = MemoryBackend::new();
    let execution = support::execution();
    assert_eq!(locator::locate(&backend, execution).unwrap(), None);
    assert!(
        backend
            .read_key(layout::META, layout::LAYOUT_KEY)
            .unwrap()
            .is_none()
    );
    let (program, _) = support::open(&support::compiled().pack);
    let session = Session::open(program.clone(), execution, backend.share()).unwrap();
    let located = locator::locate(&backend, execution).unwrap().unwrap();
    assert_eq!(located.cursor, session.cursor().unwrap());
    assert_eq!(located.artifact, program.artifact_id());
    backend
        .apply(&Batch::new().delete_object(ObjectDigest::from_bytes(*located.cursor.as_bytes())))
        .unwrap();
    assert!(locator::locate(&backend, execution).is_err());
    let key = layout::ref_key(&RefKey::active(RefName::hex(execution.as_bytes())).unwrap());
    backend
        .apply(&Batch::new().put(layout::REFS, key, vec![0xff], Expect::Any))
        .unwrap();
    assert!(locator::locate(&backend, execution).is_err());
}

#[test]
fn proof_requires_same_execution_roots_and_follows_descendants_with_a_budget() {
    let mut backend = MemoryBackend::new();
    let (program, names) = support::open(&support::compiled().pack);
    let execution = support::execution();
    let mut session = Session::open(program, execution, backend.share()).unwrap();
    support::choose(&mut session, &names, &["wave"]).unwrap();
    let candidate = session.cursor().unwrap();
    support::choose(&mut session, &names, &[]).unwrap();
    assert!(locator::retained(&backend, execution, candidate, Limits::default()).unwrap());
    assert!(
        locator::retained(
            &backend,
            execution,
            candidate,
            Limits {
                roots: 0,
                commits: 0
            }
        )
        .is_err()
    );
    let other = "execution:0190f2a0000070008000000000000002"
        .parse()
        .unwrap();
    assert!(!locator::retained(&backend, other, candidate, Limits::default()).unwrap());
    // Keep every object, remove only this execution's roots: object existence proves nothing.
    let entries = backend
        .scan_keys(layout::REFS, &[], None, 128)
        .unwrap()
        .entries;
    for entry in entries {
        backend
            .apply(&Batch::new().delete(layout::REFS, entry.key, Expect::Any))
            .unwrap();
    }
    assert!(!locator::retained(&backend, execution, candidate, Limits::default()).unwrap());
}
