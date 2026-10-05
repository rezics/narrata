//! How a cache and its host recover from each other: conflicts with other writers, crashes
//! before confirmation, moved and evicted stores, and malformed answers.

#![allow(clippy::panic, clippy::unwrap_used)]

use narrata_storage::{
    Batch, Expect, KeySpace, KeyValue, ObjectDigest, Revision, StorageBackend, StorageError,
};
use narrata_storage_host::{
    CacheBackend, Flush, FlushReply, HostError, LoadRequest, Loaded, Persist, Range, StoreId,
    StoreState, storage_key,
    testing::{MemoryHost, flush, load},
};

const SPACE: KeySpace = KeySpace::new(4);

#[test]
fn direct_flush_object_puts_keep_existing_bytes_until_deleted() {
    let host = MemoryHost::new(id(1));
    let object = digest(1);
    let put = |base, bytes: &'static [u8]| Persist {
        base,
        revision: base + 1,
        put_objects: vec![(object, bytes.into())],
        delete_objects: vec![],
        put_keys: vec![],
        delete_keys: vec![],
    };
    let send = |batches| {
        let message = Flush {
            store: id(1),
            batches,
        };
        host.persist(&Flush::decode(&message.encode()).unwrap())
    };
    assert_eq!(
        send(vec![put(0, b"first"), put(1, b"second")]),
        FlushReply::Persisted { revision: 2 }
    );
    assert_eq!(
        send(vec![put(2, b"third")]),
        FlushReply::Persisted { revision: 3 }
    );
    let read = || {
        let loaded = host.load(&LoadRequest {
            objects: vec![object],
            ..LoadRequest::default()
        });
        Loaded::decode(&loaded.encode()).unwrap().objects[0]
            .1
            .clone()
            .unwrap()
    };
    assert_eq!(read().as_ref(), b"first");
    let mut deletion = put(3, b"");
    deletion.put_objects.clear();
    deletion.delete_objects.push(object);
    assert_eq!(
        send(vec![deletion, put(4, b"after deletion")]),
        FlushReply::Persisted { revision: 5 }
    );
    assert_eq!(read().as_ref(), b"after deletion");
}

fn id(n: u8) -> StoreId {
    StoreId::from_bytes([n; 16])
}

fn digest(n: u8) -> ObjectDigest {
    ObjectDigest::from_bytes([n; 32])
}

/// Runs one call against the cache, loading until it can answer.
fn answer<T>(
    cache: &CacheBackend,
    host: &MemoryHost,
    call: impl Fn(&CacheBackend) -> Result<T, StorageError>,
) -> T {
    for _ in 0..16 {
        match call(cache) {
            Err(StorageError::NotLoaded) => assert!(load(cache, host).unwrap()),
            other => return other.unwrap(),
        }
    }
    panic!("the call kept missing")
}

fn put(cache: &CacheBackend, host: &MemoryHost, key: &[u8], value: &[u8]) {
    let current = answer(cache, host, |cache| cache.read_key(SPACE, key));
    let expect = current.map_or(Expect::Absent, |value| Expect::Revision(value.revision));
    let batch = Batch::new().put(SPACE, key, value, expect);
    answer(cache, host, |cache| cache.share().apply(&batch));
}

fn value(cache: &CacheBackend, host: &MemoryHost, key: &[u8]) -> Option<Vec<u8>> {
    answer(cache, host, |cache| cache.read_key(SPACE, key)).map(|value| value.value)
}

fn load_store(cache: &CacheBackend, host: &MemoryHost) {
    cache.load(&host.load(&LoadRequest::default())).unwrap();
}

#[test]
fn a_second_writer_s_stale_batch_conflicts_and_drops_its_cache() {
    let host = MemoryHost::new(id(1));
    let (first, second) = (CacheBackend::new(), CacheBackend::new());
    assert_eq!(value(&first, &host, b"k"), None);
    assert_eq!(value(&second, &host, b"k"), None);

    put(&first, &host, b"k", b"first");
    flush(&first, &host).unwrap();
    put(&second, &host, b"k", b"second");
    let state = host.state();
    assert_eq!(
        flush(&second, &host),
        Err(HostError::Superseded(state)),
        "the second cache believed the key absent"
    );
    assert_eq!(second.store(), None);
    assert!(second.unconfirmed().is_none());

    // Reloading shows the first writer's value, and the second writer can build on it.
    assert_eq!(value(&second, &host, b"k"), Some(b"first".to_vec()));
    put(&second, &host, b"k", b"second");
    flush(&second, &host).unwrap();
    assert_eq!(
        value(&CacheBackend::new(), &host, b"k"),
        Some(b"second".to_vec())
    );
}

#[test]
fn a_crash_before_confirmation_reopens_at_the_last_confirmed_batch() {
    let host = MemoryHost::new(id(2));
    let cache = CacheBackend::new();
    put(&cache, &host, b"a", b"1");
    flush(&cache, &host).unwrap();
    let confirmed = host.state();
    put(&cache, &host, b"a", b"2");
    put(&cache, &host, b"b", b"1");
    assert_eq!(
        cache.unconfirmed().map(|flush| flush.batches.len()),
        Some(2)
    );
    assert!(!cache.forget(), "unconfirmed batches are kept");
    drop(cache);

    let reopened = CacheBackend::new();
    assert_eq!(value(&reopened, &host, b"a"), Some(b"1".to_vec()));
    assert_eq!(value(&reopened, &host, b"b"), None);
    assert_eq!(reopened.store(), Some(confirmed));

    // A flush the host persisted but never confirmed is found after reopening.
    let cache = CacheBackend::new();
    put(&cache, &host, b"a", b"3");
    let unconfirmed = cache.unconfirmed().unwrap();
    assert!(matches!(
        host.persist(&unconfirmed),
        FlushReply::Persisted { .. }
    ));
    drop(cache);
    assert_eq!(
        value(&CacheBackend::new(), &host, b"a"),
        Some(b"3".to_vec())
    );
}

#[test]
fn a_moved_store_resyncs_a_clean_cache_and_supersedes_unconfirmed_batches() {
    let host = MemoryHost::new(id(3));
    let writer = CacheBackend::new();
    put(&writer, &host, b"k", b"1");
    flush(&writer, &host).unwrap();

    let reader = CacheBackend::new();
    assert_eq!(value(&reader, &host, b"k"), Some(b"1".to_vec()));
    put(&writer, &host, b"k", b"2");
    flush(&writer, &host).unwrap();

    // The reader still answers from what it loaded until a load finds the store moved, then
    // drops it and reads the key again.
    assert_eq!(value(&reader, &host, b"k"), Some(b"1".to_vec()));
    assert_eq!(value(&reader, &host, b"other"), None);
    assert_eq!(reader.store(), Some(host.state()));
    assert_eq!(value(&reader, &host, b"k"), Some(b"2".to_vec()));

    put(&reader, &host, b"mine", b"x");
    put(&writer, &host, b"k", b"3");
    flush(&writer, &host).unwrap();
    assert_eq!(
        reader.read_key(SPACE, b"unread"),
        Err(StorageError::NotLoaded)
    );
    assert_eq!(
        load(&reader, &host),
        Err(HostError::Superseded(host.state()))
    );
    assert!(reader.unconfirmed().is_none());
    assert_eq!(value(&reader, &host, b"mine"), None);
}

#[test]
fn an_evicted_store_never_takes_batches_counted_in_the_old_one() {
    let host = MemoryHost::new(id(4));
    let stale = CacheBackend::new();
    put(&stale, &host, b"k", b"old");
    flush(&stale, &host).unwrap();
    put(&stale, &host, b"k", b"lost");

    host.evict(id(5));
    let fresh = CacheBackend::new();
    put(&fresh, &host, b"k", b"new");
    flush(&fresh, &host).unwrap();
    assert_eq!(
        host.state().revision,
        stale.unconfirmed().unwrap().batches[0].base,
        "the new store reached the revision the stale batch expects"
    );
    assert!(matches!(
        flush(&stale, &host),
        Err(HostError::Superseded(_))
    ));
    assert_eq!(value(&fresh, &host, b"k"), Some(b"new".to_vec()));
}

#[test]
fn a_store_never_written_is_known_empty_after_one_load() {
    let host = MemoryHost::new(id(6));
    let mut cache = CacheBackend::new();
    assert_eq!(cache.read_key(SPACE, b"k"), Err(StorageError::NotLoaded));
    assert!(load(&cache, &host).unwrap());
    assert_eq!(cache.read_key(SPACE, b"k"), Ok(None));
    assert_eq!(cache.get_objects(&[digest(1)]), Ok(vec![None]));
    assert!(
        cache
            .scan_keys(SPACE, b"", None, 5)
            .unwrap()
            .entries
            .is_empty()
    );
    assert!(cache.scan_objects(None, 5).unwrap().digests.is_empty());
    cache
        .apply(
            &Batch::new()
                .put_object(digest(1), vec![1])
                .put(SPACE, b"k", b"v", Expect::Absent),
        )
        .unwrap();
    assert_eq!(host.reads().loads, 1);
}

#[test]
fn batches_load_what_they_condition_on_and_count() {
    let host = MemoryHost::new(id(7));
    let seed = CacheBackend::new();
    put(&seed, &host, b"k", b"v");
    flush(&seed, &host).unwrap();
    let revision = seed.read_key(SPACE, b"k").unwrap().unwrap().revision;

    let mut cache = CacheBackend::new();
    let batch = Batch::new()
        .put_object(digest(1), vec![1])
        .delete_object(digest(2))
        .put(SPACE, b"k", b"w", Expect::Revision(revision))
        .put(SPACE, b"index", b"i", Expect::Any)
        .delete(SPACE, b"gone", Expect::Any);
    assert_eq!(cache.apply(&batch), Err(StorageError::NotLoaded));
    assert_eq!(
        cache.take_request(),
        Some(LoadRequest::default()),
        "first the store"
    );
    load_store(&cache, &host);
    assert_eq!(cache.apply(&batch), Err(StorageError::NotLoaded));
    let request = LoadRequest {
        keys: vec![storage_key(SPACE, b"k")],
        objects: vec![digest(1), digest(2)],
        ..LoadRequest::default()
    };
    assert_eq!(
        cache.take_request(),
        Some(request.clone()),
        "unconditional keys need no load"
    );
    assert!(cache.unconfirmed().is_none());
    cache.load(&host.load(&request)).unwrap();
    let applied = cache.apply(&batch).unwrap();
    assert_eq!((applied.objects_inserted, applied.objects_deleted), (1, 0));
    let flushed = cache.unconfirmed().unwrap();
    assert_eq!(flushed.batches.len(), 1);
    assert_eq!(
        flushed.batches[0].delete_keys,
        vec![storage_key(SPACE, b"gone")],
        "a key of unknown state is deleted on the host"
    );

    // A batch that changes nothing is not sent to the host.
    let now = cache.read_key(SPACE, b"k").unwrap().unwrap().revision;
    cache
        .apply(
            &Batch::new()
                .check(SPACE, b"k", Expect::Revision(now))
                .delete(SPACE, b"gone", Expect::Any),
        )
        .unwrap();
    assert_eq!(cache.unconfirmed().unwrap().batches.len(), 1);
}

#[test]
fn scans_load_again_while_unconfirmed_deletes_hide_entries() {
    let host = MemoryHost::new(id(8));
    let seed = CacheBackend::new();
    for key in [b"a", b"b", b"c", b"d"] {
        put(&seed, &host, key, key);
    }
    flush(&seed, &host).unwrap();

    let mut cache = CacheBackend::new();
    load_store(&cache, &host);
    cache
        .apply(
            &Batch::new()
                .delete(SPACE, b"b", Expect::Any)
                .delete(SPACE, b"c", Expect::Any),
        )
        .unwrap();
    host.take_reads();
    let page = answer(&cache, &host, |cache| cache.scan_keys(SPACE, b"", None, 1));
    let keys = page
        .entries
        .iter()
        .map(|entry| entry.key.clone())
        .collect::<Vec<_>>();
    assert_eq!((keys, page.more), (vec![b"a".to_vec()], true));
    assert_eq!(
        host.reads().loads,
        3,
        "each load found one more entry hidden by a delete"
    );
    let page = answer(&cache, &host, |cache| {
        cache.scan_keys(SPACE, b"", Some(b"a"), 5)
    });
    let keys = page
        .entries
        .iter()
        .map(|entry| entry.key.clone())
        .collect::<Vec<_>>();
    assert_eq!((keys, page.more), (vec![b"d".to_vec()], false));
    assert_eq!(
        host.reads().loads,
        4,
        "the rest of the space needed one more load"
    );
}

#[test]
fn malformed_answers_change_nothing() {
    let host = MemoryHost::new(id(9));
    let seed = CacheBackend::new();
    put(&seed, &host, b"k", b"v");
    flush(&seed, &host).unwrap();
    let state = host.state();

    let cache = CacheBackend::new();
    let range = Range {
        lower: storage_key(SPACE, b"a"),
        upper: Some(storage_key(SPACE, b"c")),
        limit: 1,
    };
    let entry = |key: &[u8]| {
        let value = KeyValue {
            value: Vec::new(),
            revision: Revision::new(1).unwrap(),
        };
        (storage_key(SPACE, key), value)
    };
    let answers = [
        (vec![entry(b"z")], "outside the range"),
        (vec![entry(b"a"), entry(b"b")], "beyond the limit"),
    ];
    for (entries, case) in answers {
        let loaded = Loaded {
            store: state,
            keys: Vec::new(),
            objects: Vec::new(),
            key_ranges: vec![(range.clone(), entries)],
            object_ranges: Vec::new(),
        };
        assert!(
            matches!(cache.load(&loaded), Err(HostError::Malformed(_))),
            "{case}"
        );
        assert_eq!(cache.store(), None, "{case}");
    }
    let ahead = Loaded {
        store: StoreState {
            revision: 0,
            ..state
        },
        keys: vec![(storage_key(SPACE, b"k"), Some(entry(b"k").1))],
        objects: Vec::new(),
        key_ranges: Vec::new(),
        object_ranges: Vec::new(),
    };
    assert!(matches!(cache.load(&ahead), Err(HostError::Malformed(_))));
    assert_eq!(
        cache.confirm(&FlushReply::Persisted { revision: 3 }),
        Err(HostError::Malformed(
            "confirmed revision belongs to no unconfirmed batch"
        ))
    );
    assert_eq!(value(&cache, &host, b"k"), Some(b"v".to_vec()));
}
