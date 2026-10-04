#![allow(clippy::panic, clippy::unwrap_used)]

use std::{
    cell::Cell,
    collections::BTreeSet,
    hint::black_box,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use criterion::{BatchSize, BenchmarkId, Criterion, SamplingMode, criterion_group, criterion_main};
use narrata_core::{
    CheckedProgram, ExecutionId, InputId,
    program::{OpV0, encode_program_artifact, load_program},
    runtime::{CheckedRuntimeInput, Turn},
};
use narrata_store::{
    BranchId, CheckpointBundle, InitialRecordingMode, MemoryStore, RefKey, RefName, SaveStore,
    SessionCoordinator,
};
use narrata_store_sqlite::SqliteStore;
use narrata_testkit::generator::hello_v0;

const DEPTHS: [u64; 3] = [100, 1_000, 10_000];
const BUILD_BUDGET: Duration = Duration::from_secs(600);

fn program() -> Arc<CheckedProgram> {
    let mut artifact = hello_v0();
    let say = artifact.flows[0].entry;
    // Yield on every input, indefinitely, without growing the runtime state.
    let OpV0::Say { next, .. } = &mut artifact.flows[0].instructions[0].op else {
        panic!("hello generator must begin with Say");
    };
    *next = say;
    artifact.flows[0].instructions.truncate(1);
    load_program(&encode_program_artifact(&artifact), &Default::default()).unwrap()
}

fn session_name() -> RefName {
    RefName::new("history").unwrap()
}

fn open<S: SaveStore>(store: S, program: &Arc<CheckedProgram>) -> SessionCoordinator<S> {
    SessionCoordinator::open(
        store,
        Arc::clone(program),
        ExecutionId::from_u128(1),
        session_name(),
        BranchId::from_u128(1),
    )
    .unwrap()
}

fn advance<S: SaveStore>(session: &mut SessionCoordinator<S>, turn: u64) {
    let input = session.state().pending().map_or_else(
        || CheckedRuntimeInput::start(InputId::from_u128(u128::from(turn))),
        |pending| {
            CheckedRuntimeInput::advance(
                InputId::from_u128(u128::from(turn)),
                pending.interaction_id(),
            )
        },
    );
    let result = session
        .dispatch(input, Default::default(), turn + 1)
        .unwrap();
    assert!(!result.reused);
    assert_eq!(session.state().turn, Turn(turn));
    assert!(session.state().pending().is_some());
}

fn restored<S: SaveStore>(
    create: &impl Fn() -> S,
    bundle: &CheckpointBundle,
    program: &Arc<CheckedProgram>,
) -> SessionCoordinator<S> {
    let mut store = create();
    // Public bundle import restores the same immutable history for every commit sample.
    // It adds a manifest and fresh refs, and does not restore the prior input index.
    bundle
        .clone()
        .import(
            &mut store,
            RefKey::branch(ExecutionId::from_u128(1), BranchId::from_u128(1)).unwrap(),
            None,
            1,
        )
        .unwrap();
    bundle
        .clone()
        .import(&mut store, RefKey::active(session_name()).unwrap(), None, 1)
        .unwrap();
    open(store, program)
}

fn history<S: SaveStore>(criterion: &mut Criterion, backend: &str, create: impl Fn() -> S) {
    let program = program();
    let mut group = criterion.benchmark_group(format!("history/{backend}"));
    group.sample_size(10);
    group.sampling_mode(SamplingMode::Flat);
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(1));

    let started = Instant::now();
    let mut session = SessionCoordinator::create(
        create(),
        Arc::clone(&program),
        ExecutionId::from_u128(1),
        session_name(),
        BranchId::from_u128(1),
        InitialRecordingMode::Standard,
        1,
    )
    .unwrap();
    for turn in 1..=10 {
        advance(&mut session, turn);
    }
    let mut built = 10;
    let mut build_time = started.elapsed();
    println!(
        "history/{backend}/build_seconds/10={:.6}",
        build_time.as_secs_f64()
    );

    for depth in DEPTHS {
        // Existing full-history validation makes total setup approximately quadratic.
        // Re-evaluate from the last measured depth before attempting the next one.
        let projected = build_time.mul_f64((depth as f64 / built as f64).powi(2));
        if projected > BUILD_BUDGET {
            println!(
                "history/{backend}/skipped/{depth}: projected_build_seconds={:.3}; basis_depth={built}; basis_seconds={:.6}",
                projected.as_secs_f64(),
                build_time.as_secs_f64(),
            );
            continue;
        }
        let started = Instant::now();
        for turn in built + 1..=depth {
            advance(&mut session, turn);
            if build_time + started.elapsed() > BUILD_BUDGET {
                println!(
                    "history/{backend}/skipped/{depth}: build exceeded 600 seconds at turn={turn}"
                );
                return;
            }
        }
        build_time += started.elapsed();
        built = depth;
        println!(
            "history/{backend}/build_seconds/{depth}={:.6}",
            build_time.as_secs_f64()
        );
        let root = session.timeline().cursor;
        let bundle = CheckpointBundle::export(session.store(), root, &BTreeSet::new()).unwrap();
        let bytes = bundle.to_bytes().unwrap();
        println!("history/{backend}/checkpoint_bytes/{depth}={}", bytes.len());
        println!("history/{backend}/commit/{depth}={root}");

        group.bench_with_input(BenchmarkId::new("commit", depth), &depth, |b, &depth| {
            b.iter_batched_ref(
                || restored(&create, &bundle, &program),
                |session| {
                    assert_eq!(session.state().turn, Turn(depth));
                    advance(session, depth + 1);
                    black_box(session.timeline());
                },
                BatchSize::PerIteration,
            );
        });

        let mut store = Some(session.into_store());
        group.bench_with_input(BenchmarkId::new("open", depth), &depth, |b, &depth| {
            b.iter_custom(|iterations| {
                let mut elapsed = Duration::ZERO;
                for _ in 0..iterations {
                    let backend = store.take().unwrap();
                    let started = Instant::now();
                    let loaded = open(backend, &program);
                    elapsed += started.elapsed();
                    assert_eq!(loaded.state().turn, Turn(depth));
                    assert!(loaded.state().pending().is_some());
                    store = Some(loaded.into_store());
                }
                elapsed
            });
        });
        group.bench_with_input(BenchmarkId::new("checkpoint", depth), &depth, |b, _| {
            b.iter(|| {
                black_box(
                    CheckpointBundle::export(store.as_ref().unwrap(), root, &BTreeSet::new())
                        .unwrap()
                        .to_bytes()
                        .unwrap(),
                )
            });
        });
        session = open(store.take().unwrap(), &program);
    }
    group.finish();
}

fn sqlite_history(criterion: &mut Criterion) {
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../.temp/storage-history")
        .join(std::process::id().to_string());
    std::fs::create_dir_all(&directory).unwrap();
    let sequence = Cell::new(0_u64);
    let create = || {
        let index = sequence.get();
        sequence.set(index + 1);
        let path = directory.join(format!("{index}.db"));
        for suffix in ["", "-wal", "-shm"] {
            let file = PathBuf::from(format!("{}{suffix}", path.display()));
            match std::fs::remove_file(&file) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => panic!("cannot remove {}: {error}", file.display()),
            }
        }
        SqliteStore::open(&path).unwrap()
    };
    history(criterion, "sqlite", create);
    // All stores have closed; only remove files created in this process's scratch directory.
    for index in 0..sequence.get() {
        for suffix in ["", "-wal", "-shm"] {
            let file = directory.join(format!("{index}.db{suffix}"));
            match std::fs::remove_file(&file) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => panic!("cannot remove {}: {error}", file.display()),
            }
        }
    }
    std::fs::remove_dir(directory).unwrap();
}

fn benchmarks(criterion: &mut Criterion) {
    history(criterion, "memory", MemoryStore::new);
    sqlite_history(criterion);
}

criterion_group!(benches, benchmarks);
criterion_main!(benches);
