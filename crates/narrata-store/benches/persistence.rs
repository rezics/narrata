#![allow(clippy::panic, clippy::unwrap_used)]

use criterion::{BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main};
use narrata_core::{
    ExecutionId, GlobalId, Value, ValueKindV0,
    codec::ObjectKind,
    limits::{ProgramLoadLimits, SnapshotLoadLimits},
    program::{GlobalDeclV0, encode_program_artifact, load_program},
    runtime::new_execution,
    snapshot::{export_snapshot, restore_snapshot},
};
use narrata_store::{CheckedObject, CommitTransaction, MemoryStore, RetentionPolicy, SaveStore};
use narrata_testkit::generator::branch_call_choice_v0;

fn object_batches(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("stage2/object_batches");
    for size in [1_000_usize, 10_000] {
        let objects = (0..size)
            .map(|index| {
                let mut payload = Vec::with_capacity(16);
                payload.extend_from_slice(&(index as u64).to_be_bytes());
                payload.extend_from_slice(&(size as u64).to_be_bytes());
                CheckedObject::new(ObjectKind::Value, 1, &payload)
            })
            .collect::<Vec<_>>();
        group.bench_with_input(
            BenchmarkId::new("memory_put", size),
            &objects,
            |bencher, values| {
                bencher.iter_batched(
                    MemoryStore::new,
                    |mut store| {
                        store
                            .commit(CommitTransaction {
                                objects: values.clone(),
                                observed_at: 1,
                                ..CommitTransaction::default()
                            })
                            .unwrap()
                    },
                    BatchSize::LargeInput,
                );
            },
        );
        group.bench_with_input(
            BenchmarkId::new("memory_gc", size),
            &objects,
            |bencher, values| {
                bencher.iter_batched(
                    || {
                        let mut store = MemoryStore::new();
                        store
                            .commit(CommitTransaction {
                                objects: values.clone(),
                                observed_at: 1,
                                ..CommitTransaction::default()
                            })
                            .unwrap();
                        store
                    },
                    |mut store| store.collect(RetentionPolicy::default()).unwrap(),
                    BatchSize::LargeInput,
                );
            },
        );
    }
    group.finish();
}

fn snapshot_sizes(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("stage2/full_snapshot_variables");
    group.sample_size(10);
    for size in [1_000_usize, 10_000, 100_000] {
        let mut artifact = branch_call_choice_v0();
        artifact.globals.extend((0..size).map(|index| GlobalDeclV0 {
            id: GlobalId::from_u128(10_000_u128.saturating_add(index as u128)),
            kind: ValueKindV0::I64,
            default: Value::I64(index as i64),
        }));
        let mut limits = ProgramLoadLimits::default();
        limits.program.max_globals = artifact.globals.len() as u64;
        limits.decode.max_envelope_bytes = 64 * 1024 * 1024;
        limits.decode.max_payload_bytes = 64 * 1024 * 1024;
        let program = load_program(&encode_program_artifact(&artifact), &limits).unwrap();
        let state = new_execution(&program, ExecutionId::from_u128(1)).unwrap();
        let bytes = export_snapshot(&state).unwrap();
        println!(
            "stage2/full_snapshot_variables/bytes/{size}={}",
            bytes.len()
        );
        let restore_limits = SnapshotLoadLimits {
            decode: limits.decode,
            runtime: limits.runtime,
        };
        group.bench_with_input(
            BenchmarkId::new("encode", size),
            &state,
            |bencher, value| {
                bencher.iter(|| export_snapshot(value).unwrap());
            },
        );
        group.bench_with_input(
            BenchmarkId::new("decode", size),
            &bytes,
            |bencher, value| {
                bencher.iter(|| restore_snapshot(value, &program, &restore_limits).unwrap());
            },
        );
    }
    group.finish();
}

criterion_group!(benches, object_batches, snapshot_sizes);
criterion_main!(benches);
