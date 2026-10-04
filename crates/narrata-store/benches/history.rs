#![allow(clippy::panic, clippy::unwrap_used)]

use std::{
    collections::BTreeSet,
    hint::black_box,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use criterion::{BenchmarkId, Criterion, SamplingMode, criterion_group, criterion_main};
use narrata_core::{
    CheckedProgram, ExecutionId, InputId,
    program::{OpV0, encode_program_artifact, load_program},
    runtime::{CheckedRuntimeInput, Turn},
};
use narrata_storage::{MemoryBackend, StorageBackend};
use narrata_storage_sqlite::SqliteBackend;
use narrata_store::{
    BranchId, CheckpointBundle, InitialRecordingMode, RefName, SaveStore, SessionCoordinator, Store,
};
use narrata_testkit::generator::hello_v0;

const DEPTHS: [u64; 3] = [100, 1_000, 10_000];
/// Input identities of measured commits, apart from the turns that build the history.
const MEASURED: u128 = 1 << 64;

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

fn open<S: SaveStore>(
    store: S,
    program: &Arc<CheckedProgram>,
    branch: BranchId,
) -> SessionCoordinator<S> {
    SessionCoordinator::open(
        store,
        Arc::clone(program),
        ExecutionId::from_u128(1),
        session_name(),
        branch,
    )
    .unwrap()
}

fn input<S: SaveStore>(session: &SessionCoordinator<S>, id: u128) -> CheckedRuntimeInput {
    session.state().pending().map_or_else(
        || CheckedRuntimeInput::start(InputId::from_u128(id)),
        |pending| CheckedRuntimeInput::advance(InputId::from_u128(id), pending.interaction_id()),
    )
}

fn advance<S: SaveStore>(session: &mut SessionCoordinator<S>, turn: u64) {
    let input = input(session, u128::from(turn));
    let result = session
        .dispatch(input, Default::default(), turn + 1)
        .unwrap();
    assert!(!result.reused);
    assert_eq!(session.state().turn, Turn(turn));
    assert!(session.state().pending().is_some());
}

fn history<B: StorageBackend>(criterion: &mut Criterion, name: &str, backend: B) {
    let program = program();
    let mut group = criterion.benchmark_group(format!("history/{name}"));
    group.sample_size(10);
    group.sampling_mode(SamplingMode::Flat);
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(1));

    let started = Instant::now();
    let mut session = SessionCoordinator::create(
        Store::open(backend).unwrap(),
        Arc::clone(&program),
        ExecutionId::from_u128(1),
        session_name(),
        BranchId::from_u128(1),
        InitialRecordingMode::Standard,
        1,
    )
    .unwrap();
    let mut build_time = started.elapsed();
    let mut built = 0;
    let mut measured = 0_u128;

    for depth in DEPTHS {
        let started = Instant::now();
        for turn in built + 1..=depth {
            advance(&mut session, turn);
        }
        build_time += started.elapsed();
        built = depth;
        println!(
            "history/{name}/build_seconds/{depth}={:.6}",
            build_time.as_secs_f64()
        );
        let root = session.timeline().cursor;
        let bundle = CheckpointBundle::export(session.store(), root, &BTreeSet::new()).unwrap();
        let bytes = bundle.to_bytes().unwrap();
        println!("history/{name}/checkpoint_bytes/{depth}={}", bytes.len());
        println!("history/{name}/commit/{depth}={root}");

        // Each sample rewinds to the head at depth N, untimed, and commits one new input from
        // it, so every measured Commit is at depth N + 1.
        group.bench_function(BenchmarkId::new("commit", depth), |b| {
            b.iter_custom(|iterations| {
                let mut elapsed = Duration::ZERO;
                for _ in 0..iterations {
                    if session.timeline().cursor != root {
                        session.rewind_to(root, depth + 2).unwrap();
                    }
                    measured += 1;
                    let input = input(&session, MEASURED | measured);
                    let started = Instant::now();
                    let result = session
                        .dispatch(input, Default::default(), depth + 2)
                        .unwrap();
                    elapsed += started.elapsed();
                    assert!(!result.reused);
                    assert_eq!(session.state().turn, Turn(depth + 1));
                }
                elapsed
            });
        });
        if session.timeline().cursor != root {
            session.rewind_to(root, depth + 2).unwrap();
        }
        let branch = session.timeline().selected_branch;

        let mut store = Some(session.into_store());
        group.bench_with_input(BenchmarkId::new("open", depth), &depth, |b, &depth| {
            b.iter_custom(|iterations| {
                let mut elapsed = Duration::ZERO;
                for _ in 0..iterations {
                    let backend = store.take().unwrap();
                    let started = Instant::now();
                    let loaded = open(backend, &program, branch);
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
        session = open(store.take().unwrap(), &program, branch);
    }
    println!("history/{name}/measured_commits={measured}");
    group.finish();
}

fn sqlite_history(criterion: &mut Criterion) {
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../.temp/storage-history")
        .join(std::process::id().to_string());
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("history.db");
    history(criterion, "sqlite", SqliteBackend::open(&path).unwrap());
    // The store has closed; only remove files created in this process's scratch directory.
    for suffix in ["", "-wal", "-shm"] {
        let file = PathBuf::from(format!("{}{suffix}", path.display()));
        match std::fs::remove_file(&file) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => panic!("cannot remove {}: {error}", file.display()),
        }
    }
    std::fs::remove_dir(directory).unwrap();
}

fn benchmarks(criterion: &mut Criterion) {
    history(criterion, "memory", MemoryBackend::new());
    sqlite_history(criterion);
}

criterion_group!(benches, benchmarks);
criterion_main!(benches);
