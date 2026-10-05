mod allocator;

use std::{
    collections::BTreeSet,
    fs,
    num::NonZeroUsize,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering::Relaxed},
    },
    time::{Duration, Instant},
};

use allocator::{Counting, Interval, Measurement};
use narrata_node_bench::{SEED, WorkSize, generate};
use narrata_nodes::{
    ChunkSource, Error, ExecutionId, ObjectId, Pack, Program, Result, Session, plan::Plan,
};
use serde::{Deserialize, Serialize};

#[global_allocator]
static ALLOCATOR: Counting = Counting;

#[derive(Debug, Deserialize, Serialize)]
struct ArtifactInfo {
    seed: u64,
    choices: usize,
    units: usize,
    packages: usize,
    graphs: usize,
    artifact_id: String,
    pack_bytes: usize,
    manifest_bytes: usize,
    tombstones_bytes: usize,
    chunk_bytes: usize,
    max_chunk_bytes: usize,
}

#[derive(Default)]
struct Reads {
    count: AtomicUsize,
    bytes: AtomicUsize,
    elapsed_ns: AtomicU64,
}

struct DirectorySource {
    directory: PathBuf,
    reads: Arc<Reads>,
}

impl ChunkSource for DirectorySource {
    fn load(&self, id: &ObjectId) -> Result<Vec<u8>> {
        let started = Instant::now();
        let bytes = fs::read(
            self.directory
                .join(format!("{}.cbor", hex::encode(id.as_bytes()))),
        )
        .map_err(io)?;
        self.reads.elapsed_ns.fetch_add(
            u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX),
            Relaxed,
        );
        self.reads.count.fetch_add(1, Relaxed);
        self.reads.bytes.fetch_add(bytes.len(), Relaxed);
        Ok(bytes)
    }
}

fn io(error: std::io::Error) -> Error {
    Error::new("benchmark", "io", error.to_string())
}
fn json(error: serde_json::Error) -> Error {
    Error::new("benchmark", "json", error.to_string())
}
fn fail(message: &str) -> Error {
    Error::new("benchmark", "arguments", message)
}
fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    fs::write(path, serde_json::to_vec_pretty(value).map_err(json)?).map_err(io)
}

fn publish(directory: &Path, size: WorkSize) -> Result<()> {
    fs::create_dir_all(directory.join("chunks")).map_err(io)?;
    let interval = Interval::start();
    let compilation = generate(size)?;
    let measurement = interval.finish();
    let pack = Pack::decode(&compilation.pack)?;
    let info = ArtifactInfo {
        seed: SEED,
        choices: size.choices(),
        units: size.units(),
        packages: compilation.program.manifest().packages.len(),
        graphs: compilation.program.manifest().graphs.len(),
        artifact_id: compilation.program.artifact_id().to_string(),
        pack_bytes: compilation.pack.len(),
        manifest_bytes: pack.manifest.len(),
        tombstones_bytes: pack.tombstones.len(),
        chunk_bytes: pack.chunks.iter().map(Vec::len).sum(),
        max_chunk_bytes: pack.chunks.iter().map(Vec::len).max().unwrap_or(0),
    };
    fs::write(directory.join("work.pack.cbor"), &compilation.pack).map_err(io)?;
    fs::write(directory.join("manifest.cbor"), pack.manifest).map_err(io)?;
    fs::write(directory.join("tombstones.cbor"), pack.tombstones).map_err(io)?;
    for (id, bytes) in compilation
        .program
        .manifest()
        .chunks
        .iter()
        .zip(pack.chunks)
    {
        fs::write(
            directory
                .join("chunks")
                .join(format!("{}.cbor", hex::encode(id.as_bytes()))),
            bytes,
        )
        .map_err(io)?;
    }
    write_json(&directory.join("artifact.json"), &info)?;
    write_json(&directory.join("generation.json"), &measurement)?;
    println!("{}", serde_json::to_string(&info).map_err(json)?);
    Ok(())
}

#[derive(Debug, Serialize)]
struct Latencies {
    samples: usize,
    p50_us: f64,
    p99_us: f64,
    max_us: f64,
}

fn latencies(samples: &mut [Duration]) -> Latencies {
    samples.sort_unstable();
    let us = |index: usize| {
        samples
            .get(index)
            .map_or(0.0, |d| d.as_secs_f64() * 1_000_000.0)
    };
    let percentile = |p: usize| (samples.len() * p).div_ceil(100).saturating_sub(1);
    Latencies {
        samples: samples.len(),
        p50_us: us(percentile(50)),
        p99_us: us(percentile(99)),
        max_us: us(samples.len().saturating_sub(1)),
    }
}

#[derive(Debug, Serialize)]
struct Report {
    artifact: ArtifactInfo,
    capacity: usize,
    open: Measurement,
    open_transfer_bytes: usize,
    open_chunk_reads: usize,
    first_screen: Measurement,
    first_screen_transfer_bytes: usize,
    first_screen_chunk_reads: usize,
    requested_choices: usize,
    completed_choices: usize,
    stop_reason: String,
    choose: Latencies,
    prefetch: Latencies,
    prefetch_max_loaded_chunks: usize,
    choose_with_chunk_load: Latencies,
    choose_without_chunk_load: Latencies,
    play_chunk_read_us: f64,
    play: Measurement,
    checkpoint: Measurement,
    checkpoint_bytes: usize,
    play_chunk_reads: usize,
    play_max_loaded_chunks: usize,
    play_max_frame_chunks: usize,
    final_commit: String,
    final_state: String,
    scan: Measurement,
    scan_chunk_reads: usize,
    scan_max_loaded_chunks: usize,
}

fn read(directory: &Path, capacity: NonZeroUsize, steps: usize, output: &Path) -> Result<()> {
    let info: ArtifactInfo =
        serde_json::from_slice(&fs::read(directory.join("artifact.json")).map_err(io)?)
            .map_err(json)?;
    let reads = Arc::new(Reads::default());
    let interval = Interval::start();
    let manifest = fs::read(directory.join("manifest.cbor")).map_err(io)?;
    let tombstones = fs::read(directory.join("tombstones.cbor")).map_err(io)?;
    let open_transfer_bytes = manifest.len() + tombstones.len();
    let program = Arc::new(Program::open(
        &manifest,
        &tombstones,
        Box::new(DirectorySource {
            directory: directory.join("chunks"),
            reads: reads.clone(),
        }),
    )?);
    program.set_chunk_capacity(capacity);
    drop(manifest);
    drop(tombstones);
    let open = interval.finish();
    let open_chunk_reads = reads.count.load(Relaxed);
    let interval = Interval::start();
    let mut session = Session::new(
        program.clone(),
        ExecutionId::from_bytes(
            SEED.to_be_bytes()
                .repeat(2)
                .try_into()
                .map_err(|_| fail("execution ID length"))?,
        ),
    )?;
    let first_screen = interval.finish();
    let first_screen_chunk_reads = reads.count.load(Relaxed);
    let first_screen_transfer_bytes = open_transfer_bytes + reads.bytes.load(Relaxed);
    let mut samples = Vec::with_capacity(steps);
    let mut cold_samples = Vec::new();
    let mut warm_samples = Vec::new();
    let mut prefetch_samples = Vec::with_capacity(steps);
    let mut prefetch_max_loaded_chunks = 0;
    let before_reads = reads.count.load(Relaxed);
    let before_read_ns = reads.elapsed_ns.load(Relaxed);
    let interval = Interval::start();
    let mut stop_reason = "requested_choices_completed".to_owned();
    let mut max_loaded = program.loaded_chunks();
    let mut max_frame_chunks = 0;
    for _ in 0..steps {
        let started = Instant::now();
        session.prefetch()?;
        prefetch_samples.push(started.elapsed());
        prefetch_max_loaded_chunks = prefetch_max_loaded_chunks.max(program.loaded_chunks());
        let state = session.state()?;
        if state.finished.is_some() {
            stop_reason = "finished".into();
            break;
        }
        let frame = state.frames.last().ok_or_else(|| fail("no frame"))?;
        let graph = program.graph(&frame.graph)?;
        let Some(Plan::Passage(passage)) = graph.nodes.get(&frame.node) else {
            return Err(fail("not a passage"));
        };
        let point = passage
            .choice_points
            .iter()
            .find(|point| Some(point.id) == frame.at)
            .ok_or_else(|| fail("missing point"))?;
        // Deterministic varied local replies and detours, including chapter call/return.
        let index = (samples.len() as u64 ^ SEED) as usize % point.options.len();
        let (point, option) = (point.id, point.options[index].id);
        drop(graph);
        let expected = session.cursor()?;
        let choice_reads = reads.count.load(Relaxed);
        let started = Instant::now();
        match session.choose(&expected, point, vec![option]) {
            Ok(_) => {
                let elapsed = started.elapsed();
                samples.push(elapsed);
                if reads.count.load(Relaxed) == choice_reads {
                    warm_samples.push(elapsed);
                } else {
                    cold_samples.push(elapsed);
                }
                max_loaded = max_loaded.max(program.loaded_chunks());
                let frame_chunks = session
                    .state()?
                    .frames
                    .iter()
                    .map(|frame| {
                        program
                            .manifest()
                            .graphs
                            .get(&frame.graph)
                            .map(|entry| entry.chunk)
                            .ok_or_else(|| fail("frame graph is missing"))
                    })
                    .collect::<Result<BTreeSet<_>>>()?;
                max_frame_chunks = max_frame_chunks.max(frame_chunks.len());
            }
            Err(error) if error.code == "history_limit" => {
                stop_reason = error.code;
                break;
            }
            Err(error) => return Err(error),
        }
    }
    let play = interval.finish();
    let completed_choices = samples.len();
    let play_chunk_reads = reads.count.load(Relaxed) - before_reads;
    let play_chunk_read_us = (reads.elapsed_ns.load(Relaxed) - before_read_ns) as f64 / 1000.0;
    let final_commit = session.cursor()?.to_string();
    let final_state = session.state()?.id().to_string();
    let interval = Interval::start();
    let exported = session.export()?;
    let checkpoint_bytes = exported.len() / 2;
    let restored = Session::restore(program.clone(), &exported)?;
    if restored.cursor()? != session.cursor()? || restored.state()? != session.state()? {
        return Err(fail("checkpoint changed the long-play state"));
    }
    let checkpoint = interval.finish();
    drop(restored);
    drop(exported);
    drop(session);
    program.set_chunk_capacity(capacity);
    let before_reads = reads.count.load(Relaxed);
    let interval = Interval::start();
    let mut scan_max_loaded_chunks = 0;
    // A separate cache-only full scan verifies that old chunks do not accumulate. It is
    // not counted as play; session-owned pins have been released.
    for key in program.manifest().graphs.keys() {
        drop(program.graph(key)?);
        program.set_chunk_capacity(capacity);
        scan_max_loaded_chunks = scan_max_loaded_chunks.max(program.loaded_chunks());
    }
    let scan = interval.finish();
    let report = Report {
        artifact: info,
        capacity: capacity.get(),
        open,
        open_transfer_bytes,
        open_chunk_reads,
        first_screen,
        first_screen_transfer_bytes,
        first_screen_chunk_reads,
        requested_choices: steps,
        completed_choices,
        stop_reason,
        choose: latencies(&mut samples),
        prefetch: latencies(&mut prefetch_samples),
        prefetch_max_loaded_chunks,
        choose_with_chunk_load: latencies(&mut cold_samples),
        choose_without_chunk_load: latencies(&mut warm_samples),
        play_chunk_read_us,
        play,
        checkpoint,
        checkpoint_bytes,
        play_chunk_reads,
        play_max_loaded_chunks: max_loaded,
        play_max_frame_chunks: max_frame_chunks,
        final_commit,
        final_state,
        scan,
        scan_chunk_reads: reads.count.load(Relaxed) - before_reads,
        scan_max_loaded_chunks,
    };
    write_json(output, &report)?;
    println!("{}", serde_json::to_string(&report).map_err(json)?);
    // Give the Windows parent time to sample the process's historical working-set peak.
    std::thread::sleep(Duration::from_millis(200));
    Ok(())
}

fn run() -> Result<()> {
    let mut directory = None;
    let mut output = None;
    let mut choices = 100_000;
    let mut capacity = NonZeroUsize::new(16).ok_or_else(|| fail("zero capacity"))?;
    let mut steps = 10_000;
    let mut generate_mode = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let value = args
            .next()
            .ok_or_else(|| fail("every argument needs a value"))?;
        match arg.as_str() {
            "--generate" => {
                directory = Some(PathBuf::from(value));
                generate_mode = true;
            }
            "--read" => directory = Some(PathBuf::from(value)),
            "--output" => output = Some(PathBuf::from(value)),
            "--choices" => choices = value.parse().map_err(|_| fail("invalid choices"))?,
            "--capacity" => {
                capacity = value
                    .parse()
                    .map_err(|_| fail("capacity must be positive"))?
            }
            "--steps" => {
                steps = value.parse().map_err(|_| fail("invalid steps"))?;
                if !(1..=100_000).contains(&steps) {
                    return Err(fail("steps must be in 1..=100000"));
                }
            }
            _ => return Err(fail("unknown argument")),
        }
    }
    let directory = directory.ok_or_else(|| fail("use --generate DIR or --read DIR"))?;
    if generate_mode {
        publish(&directory, WorkSize::new(choices)?)
    } else {
        read(
            &directory,
            capacity,
            steps,
            &output.ok_or_else(|| fail("--read requires --output"))?,
        )
    }
}

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            std::process::ExitCode::FAILURE
        }
    }
}
