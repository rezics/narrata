//! Compare full and shallow generic checkpoints at depth 10K on the same store.
#![allow(clippy::panic, clippy::unwrap_used)]

use narrata_history::{DomainKinds, History, RefName, Session, testing::Counter};
use narrata_storage::{MemoryBackend, StorageBackend};
use narrata_storage_sqlite::SqliteBackend;
use serde_json::json;
use std::{collections::BTreeSet, hint::black_box, time::Instant};

fn measure<B: StorageBackend>(backend: B, label: &str) {
    let counter = Counter::new(1_000_000);
    let mut history = History::open(backend, DomainKinds::<Counter>::new()).unwrap();
    let start = Instant::now();
    let (mut session, _) =
        Session::create(&mut history, counter, RefName::new("bench").unwrap(), &0, 1).unwrap();
    for time in 2..=10_001 {
        let head = session.head();
        session
            .advance(
                &mut history,
                head,
                &1,
                |counter, state, input| counter.step(state, input),
                time,
            )
            .unwrap();
    }
    let build_seconds = start.elapsed().as_secs_f64();
    let mut results = Vec::new();
    for budget in [Some(0), Some(100), None] {
        let mut times = Vec::new();
        let mut size = 0;
        for sample in 0..12 {
            let start = Instant::now();
            let bytes = match budget {
                Some(ancestors) => history
                    .export_shallow(session.head(), ancestors, &BTreeSet::new())
                    .unwrap()
                    .to_bytes()
                    .unwrap(),
                None => history
                    .export(session.head(), &BTreeSet::new())
                    .unwrap()
                    .to_bytes()
                    .unwrap(),
            };
            let elapsed = start.elapsed().as_secs_f64() * 1000.0;
            size = bytes.len();
            black_box(bytes);
            if sample >= 2 {
                times.push(elapsed);
            }
        }
        times.sort_by(f64::total_cmp);
        results.push(
            json!({ "ancestors": budget, "bytes": size, "median_ms": (times[4] + times[5]) / 2.0 }),
        );
    }
    println!(
        "{}",
        json!({ "backend": label, "depth": 10000, "target": session.head().to_string(), "build_seconds": build_seconds, "exports": results })
    );
}

fn main() {
    measure(MemoryBackend::new(), "memory");
    // A unique system-temporary database prevents accidentally reopening an earlier benchmark.
    let directory = tempfile::tempdir().unwrap();
    measure(
        SqliteBackend::open(directory.path().join("history.sqlite3")).unwrap(),
        "sqlite",
    );
}
