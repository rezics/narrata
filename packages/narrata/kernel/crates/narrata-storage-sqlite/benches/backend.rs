//! Single-operation cost on stores holding 1K and 100K keys and objects. Row-level access
//! should make applying a batch, reading a key, reading an object and scanning a page cost
//! about the same at both sizes.

#![allow(clippy::unwrap_used)]

use std::{hint::black_box, path::Path, sync::Arc, time::Duration};

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use narrata_storage::{Batch, Expect, KeySpace, ObjectDigest, Revision, StorageBackend};
use narrata_storage_sqlite::SqliteBackend;

const SPACE: KeySpace = KeySpace::new(1);
const SIZES: [u64; 2] = [1_000, 100_000];
const FILL_BATCH: u64 = 1_000;
const PAGE: u32 = 64;

fn key(n: u64) -> [u8; 8] {
    n.to_be_bytes()
}

fn digest(n: u64) -> ObjectDigest {
    let mut bytes = [0; 32];
    bytes[..8].copy_from_slice(&n.wrapping_mul(0x9E37_79B9_7F4A_7C15).to_be_bytes());
    bytes[8..16].copy_from_slice(&n.to_be_bytes());
    ObjectDigest::from_bytes(bytes)
}

fn populated(dir: &Path, size: u64) -> SqliteBackend {
    let mut backend = SqliteBackend::open(dir.join(format!("store-{size}.sqlite"))).unwrap();
    let object: Arc<[u8]> = vec![0xAB; 256].into();
    for start in (0..size).step_by(FILL_BATCH as usize) {
        let batch = (start..(start + FILL_BATCH).min(size)).fold(Batch::new(), |batch, n| {
            batch.put_object(digest(n), Arc::clone(&object)).put(
                SPACE,
                key(n),
                [0; 64],
                Expect::Any,
            )
        });
        backend.apply(&batch).unwrap();
    }
    backend
}

fn operations(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    let mut group = c.benchmark_group("sqlite");
    group
        .sample_size(20)
        .measurement_time(Duration::from_secs(5));
    for size in SIZES {
        let mut backend = populated(dir.path(), size);
        let middle = size / 2;

        let object: Arc<[u8]> = vec![0xCD; 256].into();
        let mut next = size;
        let mut head: Option<Revision> = None;
        group.bench_function(BenchmarkId::new("apply_batch", size), |bench| {
            bench.iter(|| {
                // One commit's shape: a new object, its index key and a CAS on a head key.
                let expect = head.map_or(Expect::Absent, Expect::Revision);
                let batch = Batch::new()
                    .put_object(digest(next), Arc::clone(&object))
                    .put(SPACE, key(next), [1; 64], Expect::Absent)
                    .put(KeySpace::new(2), b"head", key(next), expect);
                head = backend.apply(&batch).unwrap().revision;
                next += 1;
            });
        });
        group.bench_function(BenchmarkId::new("read_key", size), |bench| {
            bench.iter(|| black_box(backend.read_key(SPACE, &key(middle)).unwrap()));
        });
        group.bench_function(BenchmarkId::new("get_object", size), |bench| {
            bench.iter(|| black_box(backend.get_object(&digest(middle)).unwrap()));
        });
        group.bench_function(BenchmarkId::new("scan_page", size), |bench| {
            bench.iter(|| {
                let page = backend
                    .scan_keys(SPACE, b"", Some(&key(middle)), PAGE)
                    .unwrap();
                assert_eq!(page.entries.len(), PAGE as usize);
                black_box(page)
            });
        });
    }
    group.finish();
}

criterion_group!(benches, operations);
criterion_main!(benches);
