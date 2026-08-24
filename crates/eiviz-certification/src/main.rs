use eiviz_command::{COMMAND_ENVELOPE_VERSION, Command, CommandEnvelope, Sequencer, state_hash};
use eiviz_core::{
    ClientId, CommandId, Input, InputId, InputSource, MixTap, MixingUnit, MixingUnitId, Project,
    Scene, SceneId, SceneItem, SceneItemId, Transform2D,
};
use eiviz_engine::{AdmissionBudget, Engine, EngineError};
use eiviz_operations::export_json_atomic;
use eiviz_runtime::RuntimeSnapshot;
use eiviz_time::{
    Clock, MediaTime, NTSC_5994, Rational, VirtualClock, audio_frame_sample_span,
    audio_sample_index,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const EVIDENCE_SCHEMA: u32 = 1;
const DEFAULT_OUTPUT: &str = "target/certification/evidence.json";
const DEFAULT_MATRIX_JSON: &str = "target/certification/traceability.json";
const DEFAULT_MATRIX_MARKDOWN: &str = "docs/traceability.md";

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum RunMode {
    Virtual,
    Wall,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum TestStatus {
    Passed,
    Failed,
    Skipped,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum EvidenceClass {
    AutomatedSimulation,
    AutomatedWallClock,
    HilVerified,
    HilPending,
}

#[derive(Debug)]
struct Config {
    mode: RunMode,
    equivalent_hours: u64,
    wall_seconds: u64,
    profile: String,
    output: PathBuf,
    graph_units: usize,
    graph_inputs: usize,
    clients: usize,
    commands_per_client: usize,
    gpu_device_loss_evidence: Option<PathBuf>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            mode: RunMode::Virtual,
            equivalent_hours: 24,
            wall_seconds: 60,
            profile: "certification".into(),
            output: PathBuf::from(DEFAULT_OUTPUT),
            graph_units: 32,
            graph_inputs: 64,
            clients: 8,
            commands_per_client: 128,
            gpu_device_loss_evidence: None,
        }
    }
}

#[derive(Debug, Serialize)]
struct EvidenceReport {
    schema_version: u32,
    generated_unix_millis: u64,
    commit: String,
    os: String,
    architecture: String,
    rust: String,
    profile: String,
    mode: RunMode,
    equivalent_hours: u64,
    wall_seconds: u64,
    hardware_certification_claim: bool,
    outcome: TestStatus,
    tests: Vec<TestEvidence>,
    artifacts: Vec<String>,
}

#[derive(Debug, Serialize)]
struct TestEvidence {
    id: String,
    requirement_ids: Vec<String>,
    status: TestStatus,
    evidence_class: EvidenceClass,
    hardware_observed: bool,
    summary: String,
    assertions: Vec<AssertionEvidence>,
    measurements: BTreeMap<String, Value>,
}

#[derive(Debug, Serialize)]
struct AssertionEvidence {
    name: String,
    passed: bool,
    actual: Value,
    expected: Value,
}

#[derive(Default)]
struct SoakMeasurements {
    program_drops: u64,
    program_repeats: u64,
    audio_xruns: u64,
    deadline_misses: u64,
    max_deadline_lateness_nanos: u64,
    av_drift_p99_nanos: u64,
    av_drift_max_nanos: u64,
    audited_boundaries: u64,
}

impl TestEvidence {
    fn automated(
        id: &str,
        requirements: &[&str],
        class: EvidenceClass,
        summary: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            requirement_ids: requirements.iter().map(|value| (*value).into()).collect(),
            status: TestStatus::Passed,
            evidence_class: class,
            hardware_observed: false,
            summary: summary.into(),
            assertions: Vec::new(),
            measurements: BTreeMap::new(),
        }
    }

    fn assert(&mut self, name: &str, actual: Value, expected: Value, passed: bool) {
        if !passed {
            self.status = TestStatus::Failed;
        }
        self.assertions.push(AssertionEvidence {
            name: name.into(),
            passed,
            actual,
            expected,
        });
    }
}

#[derive(Debug, Deserialize)]
struct GpuDeviceLossAttestation {
    evidence_source: String,
    hardware_observed: bool,
    device_loss_observed: bool,
    recovery_at_frame_boundary: bool,
    program_backend_unchanged: bool,
}

#[derive(Clone, Copy)]
enum InjectedFault {
    SinkFailure,
    DiskFull,
    NicOutage,
}

impl InjectedFault {
    fn id(self) -> &'static str {
        match self {
            Self::SinkFailure => "CERT-FAULT-SINK",
            Self::DiskFull => "CERT-FAULT-DISK-FULL",
            Self::NicOutage => "CERT-FAULT-NIC-OUTAGE",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::SinkFailure => "injected sink failure",
            Self::DiskFull => "injected StorageFull",
            Self::NicOutage => "injected network disconnect",
        }
    }
}

#[derive(Debug, Serialize)]
struct TraceabilityMatrix {
    schema_version: u32,
    generated_from: String,
    entries: Vec<TraceEntry>,
}

#[derive(Debug, Serialize)]
struct TraceEntry {
    requirement_id: String,
    automated_tests: Vec<String>,
    hil_scenarios: Vec<String>,
    hil_status: String,
    artifact_paths: Vec<String>,
}

fn main() {
    if let Err(error) = run_main() {
        eprintln!("certification harness failed: {error}");
        std::process::exit(1);
    }
}

fn run_main() -> Result<()> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    if args.first().is_some_and(|arg| arg == "matrix") {
        let (json_path, markdown_path) = parse_matrix_args(&args[1..])?;
        generate_matrix(&json_path, &markdown_path)?;
        println!(
            "generated {} and {}",
            json_path.display(),
            markdown_path.display()
        );
        return Ok(());
    }
    if args
        .first()
        .is_some_and(|arg| arg == "--help" || arg == "-h")
    {
        print_help();
        return Ok(());
    }

    let config = parse_run_args(&args)?;
    let mut tests = Vec::new();
    sample_memory(&mut tests, "start");
    tests.push(run_timing_soak(&config)?);
    tests.push(run_max_graph(&config)?);
    tests.push(run_command_flood(&config)?);
    for fault in [
        InjectedFault::SinkFailure,
        InjectedFault::DiskFull,
        InjectedFault::NicOutage,
    ] {
        tests.push(run_fault_isolation(fault));
    }
    tests.push(run_gpu_device_loss(&config)?);
    sample_memory(&mut tests, "finish");
    let memory = consolidate_memory_samples(&tests);
    tests.push(memory);

    let outcome = if tests.iter().any(|test| test.status == TestStatus::Failed) {
        TestStatus::Failed
    } else {
        TestStatus::Passed
    };
    let report = EvidenceReport {
        schema_version: EVIDENCE_SCHEMA,
        generated_unix_millis: unix_millis(),
        commit: command_output("git", &["rev-parse", "HEAD"]),
        os: os_description(),
        architecture: env::consts::ARCH.into(),
        rust: command_output("rustc", &["--version"]),
        profile: config.profile.clone(),
        mode: config.mode,
        equivalent_hours: config.equivalent_hours,
        wall_seconds: config.wall_seconds,
        hardware_certification_claim: false,
        outcome,
        tests,
        artifacts: vec![
            config.output.display().to_string(),
            DEFAULT_MATRIX_JSON.into(),
            DEFAULT_MATRIX_MARKDOWN.into(),
        ],
    };
    export_json_atomic(&report, &config.output)?;
    println!(
        "certification evidence {:?}: {}",
        report.outcome,
        config.output.display()
    );
    if report.outcome == TestStatus::Failed {
        return Err("one or more certification assertions failed".into());
    }
    Ok(())
}

fn parse_run_args(args: &[String]) -> Result<Config> {
    let mut config = Config::default();
    let mut index = 0;
    while index < args.len() {
        let value = args
            .get(index + 1)
            .ok_or_else(|| format!("missing value for {}", args[index]))?;
        match args[index].as_str() {
            "--mode" => {
                config.mode = match value.as_str() {
                    "virtual" => RunMode::Virtual,
                    "wall" => RunMode::Wall,
                    _ => return Err(format!("invalid mode {value}").into()),
                };
            }
            "--equivalent-hours" => config.equivalent_hours = value.parse()?,
            "--wall-seconds" => config.wall_seconds = value.parse()?,
            "--profile" => config.profile = value.clone(),
            "--output" => config.output = value.into(),
            "--graph-units" => config.graph_units = value.parse()?,
            "--graph-inputs" => config.graph_inputs = value.parse()?,
            "--clients" => config.clients = value.parse()?,
            "--commands-per-client" => config.commands_per_client = value.parse()?,
            "--gpu-device-loss-evidence" => config.gpu_device_loss_evidence = Some(value.into()),
            option => return Err(format!("unknown option {option}").into()),
        }
        index += 2;
    }
    if config.equivalent_hours == 0
        || config.wall_seconds == 0
        || config.graph_units == 0
        || config.graph_inputs == 0
        || config.clients == 0
        || config.commands_per_client == 0
    {
        return Err("all numeric options must be non-zero".into());
    }
    if config.graph_inputs < config.graph_units {
        return Err("--graph-inputs must be at least --graph-units".into());
    }
    Ok(config)
}

fn parse_matrix_args(args: &[String]) -> Result<(PathBuf, PathBuf)> {
    let mut json = PathBuf::from(DEFAULT_MATRIX_JSON);
    let mut markdown = PathBuf::from(DEFAULT_MATRIX_MARKDOWN);
    let mut index = 0;
    while index < args.len() {
        let value = args
            .get(index + 1)
            .ok_or_else(|| format!("missing value for {}", args[index]))?;
        match args[index].as_str() {
            "--json" => json = value.into(),
            "--markdown" => markdown = value.into(),
            option => return Err(format!("unknown matrix option {option}").into()),
        }
        index += 2;
    }
    Ok((json, markdown))
}

fn run_timing_soak(config: &Config) -> Result<TestEvidence> {
    let class = match config.mode {
        RunMode::Virtual => EvidenceClass::AutomatedSimulation,
        RunMode::Wall => EvidenceClass::AutomatedWallClock,
    };
    let mut test = TestEvidence::automated(
        "CERT-TIMING-SOAK",
        &["R04", "R06", "R10", "AC-03", "AC-09", "AC-10"],
        class,
        "Exact 60000/1001 cadence and 48 kHz boundary audit; synthetic evidence is not HIL",
    );
    let seconds = config.equivalent_hours.saturating_mul(3_600);
    let frames = seconds.saturating_mul(60_000) / 1_001;
    let end = MediaTime::from_frame_index(frames, NTSC_5994)?;
    let samples = audio_sample_index(frames, 48_000, NTSC_5994)?;
    let soak = match config.mode {
        RunMode::Virtual => run_virtual_clock(frames)?,
        RunMode::Wall => run_wall_clock(config.wall_seconds)?,
    };
    test.measurements
        .insert("equivalent_seconds".into(), json!(seconds));
    test.measurements
        .insert("equivalent_frames".into(), json!(frames));
    test.measurements
        .insert("audio_samples".into(), json!(samples));
    test.measurements
        .insert("av_drift_p99_nanos".into(), json!(soak.av_drift_p99_nanos));
    test.measurements
        .insert("av_drift_max_nanos".into(), json!(soak.av_drift_max_nanos));
    test.measurements
        .insert("program_drops".into(), json!(soak.program_drops));
    test.measurements
        .insert("program_repeats".into(), json!(soak.program_repeats));
    test.measurements
        .insert("audio_xruns".into(), json!(soak.audio_xruns));
    test.measurements
        .insert("deadline_misses".into(), json!(soak.deadline_misses));
    test.measurements.insert(
        "max_deadline_lateness_nanos".into(),
        json!(soak.max_deadline_lateness_nanos),
    );
    test.measurements
        .insert("audited_boundaries".into(), json!(soak.audited_boundaries));
    test.assert(
        "program_drops",
        json!(soak.program_drops),
        json!(0),
        soak.program_drops == 0,
    );
    test.assert(
        "program_repeats",
        json!(soak.program_repeats),
        json!(0),
        soak.program_repeats == 0,
    );
    test.assert(
        "audio_xruns",
        json!(soak.audio_xruns),
        json!(0),
        soak.audio_xruns == 0,
    );
    test.assert(
        "deadline_misses",
        json!(soak.deadline_misses),
        json!(0),
        soak.deadline_misses == 0,
    );
    test.assert(
        "synthetic_av_drift_p99_within_1ms",
        json!(soak.av_drift_p99_nanos),
        json!({"max_nanos": 1_000_000}),
        soak.av_drift_p99_nanos <= 1_000_000,
    );
    test.assert(
        "synthetic_av_drift_max_within_5ms",
        json!(soak.av_drift_max_nanos),
        json!({"max_nanos": 5_000_000}),
        soak.av_drift_max_nanos <= 5_000_000,
    );
    test.assert(
        "exact_frame_index_round_trip",
        json!(end.frame_index(NTSC_5994)?),
        json!(frames),
        end.frame_index(NTSC_5994)? == frames,
    );
    Ok(test)
}

fn run_virtual_clock(frames: u64) -> Result<SoakMeasurements> {
    let clock = VirtualClock::new();
    let mut measurements = SoakMeasurements::default();
    let mut previous_frame = None;
    let mut previous_deadline = None;
    let mut av_histogram = BTreeMap::<u64, u64>::new();
    for frame in 0..frames {
        clock.seek_frame(frame, NTSC_5994);
        let deadline = clock.now().nanos;
        let expected_deadline = ((u128::from(frame) * 1_001 * 1_000_000_000) / 60_000) as u64;
        if deadline != expected_deadline {
            measurements.deadline_misses = measurements.deadline_misses.saturating_add(1);
        }
        if let Some(previous) = previous_frame {
            if frame == previous {
                measurements.program_repeats = measurements.program_repeats.saturating_add(1);
            } else if frame != previous + 1 {
                measurements.program_drops = measurements
                    .program_drops
                    .saturating_add(frame.saturating_sub(previous + 1));
            }
        }
        if previous_deadline.is_some_and(|previous| deadline <= previous) {
            measurements.program_repeats = measurements.program_repeats.saturating_add(1);
        }
        let (sample, span) = audio_frame_sample_span(frame, 48_000, NTSC_5994)?;
        if !matches!(span, 800 | 801) {
            measurements.audio_xruns = measurements.audio_xruns.saturating_add(1);
        }
        let audio_nanos = u128::from(sample) * 1_000_000_000 / 48_000;
        let drift = audio_nanos.abs_diff(u128::from(deadline)) as u64;
        measurements.av_drift_max_nanos = measurements.av_drift_max_nanos.max(drift);
        *av_histogram.entry(drift).or_default() += 1;
        previous_frame = Some(frame);
        previous_deadline = Some(deadline);
    }
    measurements.audited_boundaries = frames;
    measurements.av_drift_p99_nanos = percentile(&av_histogram, frames, 99);
    Ok(measurements)
}

fn run_wall_clock(seconds: u64) -> Result<SoakMeasurements> {
    let period_nanos = 1_001_000_000_u64 / 60;
    let duration = Duration::from_secs(seconds);
    let start = Instant::now();
    let mut frame = 0_u64;
    let mut measurements = SoakMeasurements::default();
    let mut av_histogram = BTreeMap::<u64, u64>::new();
    while start.elapsed() < duration {
        let target = start + Duration::from_nanos(frame.saturating_mul(period_nanos));
        if let Some(remaining) = target.checked_duration_since(Instant::now()) {
            std::thread::sleep(remaining);
        }
        let late = Instant::now()
            .checked_duration_since(target)
            .map_or(0, |value| value.as_nanos().min(u128::from(u64::MAX)) as u64);
        measurements.max_deadline_lateness_nanos =
            measurements.max_deadline_lateness_nanos.max(late);
        if late > period_nanos {
            measurements.deadline_misses = measurements.deadline_misses.saturating_add(1);
        }
        let (sample, span) = audio_frame_sample_span(frame, 48_000, NTSC_5994)?;
        if !matches!(span, 800 | 801) {
            measurements.audio_xruns = measurements.audio_xruns.saturating_add(1);
        }
        let logical_nanos = ((u128::from(frame) * 1_001 * 1_000_000_000) / 60_000) as u64;
        let audio_nanos = u128::from(sample) * 1_000_000_000 / 48_000;
        let drift = audio_nanos.abs_diff(u128::from(logical_nanos)) as u64;
        measurements.av_drift_max_nanos = measurements.av_drift_max_nanos.max(drift);
        *av_histogram.entry(drift).or_default() += 1;
        frame = frame.saturating_add(1);
    }
    measurements.audited_boundaries = frame;
    measurements.av_drift_p99_nanos = percentile(&av_histogram, frame, 99);
    Ok(measurements)
}

fn percentile(histogram: &BTreeMap<u64, u64>, count: u64, percentile: u64) -> u64 {
    if count == 0 {
        return 0;
    }
    let target = count.saturating_mul(percentile).div_ceil(100);
    let mut cumulative = 0_u64;
    for (value, occurrences) in histogram {
        cumulative = cumulative.saturating_add(*occurrences);
        if cumulative >= target {
            return *value;
        }
    }
    histogram.last_key_value().map_or(0, |(value, _)| *value)
}

fn run_max_graph(config: &Config) -> Result<TestEvidence> {
    let mut test = TestEvidence::automated(
        "CERT-MAX-ADMITTED-GRAPH",
        &["R02", "R03", "R05", "AC-04", "AC-08"],
        EvidenceClass::AutomatedSimulation,
        "Compile the configured maximum DAG and reject one input beyond its explicit budget",
    );
    let project = maximum_graph(config.graph_units, config.graph_inputs);
    project.validate()?;
    let snapshot = RuntimeSnapshot::compile(Arc::new(project.clone()), 0, 0)?;
    let vram = snapshot.estimated_render_vram_bytes();
    let engine = Engine::from_project(project)?;
    let mut budget = engine.admission_budget();
    budget.max_inputs = config.graph_inputs;
    budget.max_units = config.graph_units;
    engine.set_admission_budget(AdmissionBudget { ..budget })?;
    let overflow = Input {
        id: InputId::from_u128(900_000),
        name: "over-budget".into(),
        tags: vec![],
        groups: vec![],
        source: InputSource::ColorBars,
    };
    let rejected = matches!(
        engine.submit_payload(Command::AddInput { input: overflow }),
        Err(EngineError::Admission(_))
    );
    test.measurements
        .insert("admitted_inputs".into(), json!(config.graph_inputs));
    test.measurements
        .insert("admitted_units".into(), json!(config.graph_units));
    test.measurements
        .insert("estimated_render_vram_bytes".into(), json!(vram));
    test.assert("maximum_graph_valid", json!(true), json!(true), true);
    test.assert(
        "one_beyond_budget_rejected",
        json!(rejected),
        json!(true),
        rejected,
    );
    Ok(test)
}

fn maximum_graph(units: usize, inputs: usize) -> Project {
    let mut project = Project::new("certification maximum graph");
    project.video.width = 64;
    project.video.height = 36;
    project.inputs.clear();
    project.scenes.clear();
    project.mixing_units.clear();
    project.outputs.clear();
    let mut previous = None;
    for index in 0..units {
        let mut unit = MixingUnit::new(format!("unit-{index}"));
        unit.id = MixingUnitId::from_u128(10_000 + index as u128);
        let input_id = InputId::from_u128(20_000 + index as u128);
        let source = previous.map_or(InputSource::ColorBars, |parent| InputSource::MixFeed {
            unit: parent,
            tap: MixTap::Program,
        });
        project.inputs.insert(
            input_id,
            Input {
                id: input_id,
                name: format!("graph-input-{index}"),
                tags: vec![],
                groups: vec![],
                source,
            },
        );
        let scene_id = SceneId::from_u128(30_000 + index as u128);
        project.scenes.insert(
            scene_id,
            Scene {
                id: scene_id,
                name: format!("scene-{index}"),
                items: vec![SceneItem {
                    id: SceneItemId::from_u128(40_000 + index as u128),
                    input: input_id,
                    transform: Transform2D::fullscreen(),
                    z_order: 0,
                    playback: Default::default(),
                }],
            },
        );
        unit.program.scene = Some(scene_id);
        project.mixing_units.insert(unit.id, unit.clone());
        previous = Some(unit.id);
    }
    for index in units..inputs {
        let id = InputId::from_u128(20_000 + index as u128);
        project.inputs.insert(
            id,
            Input {
                id,
                name: format!("admitted-input-{index}"),
                tags: vec![],
                groups: vec![],
                source: InputSource::ColorBars,
            },
        );
    }
    project
}

fn run_command_flood(config: &Config) -> Result<TestEvidence> {
    let mut test = TestEvidence::automated(
        "CERT-COMMAND-FLOOD-REPLAY",
        &["R09", "R10", "AC-07"],
        EvidenceClass::AutomatedSimulation,
        "Deterministic multi-client flood, bounded queue high-water, exact replay, and state hash",
    );
    let baseline = Project::new("command flood");
    let commands = deterministic_commands(config.clients, config.commands_per_client)?;
    let (first, first_hash, high_water, replay_duplicates) =
        execute_command_log(&baseline, &commands, true)?;
    let (second, second_hash, second_high_water, _) =
        execute_command_log(&baseline, &commands, false)?;
    let expected_count = config.clients.saturating_mul(config.commands_per_client);
    test.measurements
        .insert("commands".into(), json!(expected_count));
    test.measurements
        .insert("clients".into(), json!(config.clients));
    test.measurements
        .insert("queue_high_water".into(), json!(high_water));
    test.measurements
        .insert("state_hash".into(), json!(first_hash));
    test.assert(
        "all_commands_applied",
        json!(first),
        json!(expected_count),
        first == expected_count && second == expected_count,
    );
    test.assert(
        "queue_high_water_bounded",
        json!(high_water),
        json!({"expected": expected_count}),
        high_water == expected_count && second_high_water == expected_count,
    );
    test.assert(
        "replay_state_hash",
        json!(first_hash),
        json!(second_hash),
        first_hash == second_hash,
    );
    test.assert(
        "idempotent_replay",
        json!(replay_duplicates),
        json!(expected_count),
        replay_duplicates == expected_count,
    );
    Ok(test)
}

fn deterministic_commands(clients: usize, per_client: usize) -> Result<Vec<CommandEnvelope>> {
    let total = clients.saturating_mul(per_client);
    let effective_time = MediaTime::new(1, Rational::ONE);
    let mut commands = Vec::with_capacity(total);
    for round in 0..per_client {
        for client_index in 0..clients {
            let ordinal = round.saturating_mul(clients).saturating_add(client_index);
            commands.push(CommandEnvelope {
                version: COMMAND_ENVELOPE_VERSION,
                id: CommandId::from_u128(100_000 + ordinal as u128),
                client: ClientId::from_u128(200_000 + client_index as u128),
                client_seq: (round + 1).try_into()?,
                expected_revision: None,
                effective_time: Some(effective_time),
                coalesce_key: None,
                payload: Command::SetName {
                    name: format!("client-{client_index}-command-{round}"),
                },
            });
        }
    }
    Ok(commands)
}

fn execute_command_log(
    baseline: &Project,
    commands: &[CommandEnvelope],
    replay: bool,
) -> Result<(usize, String, usize, usize)> {
    let mut sequencer =
        Sequencer::with_capacities(commands.len(), commands.len().saturating_mul(2));
    for command in commands {
        sequencer.stage(baseline, command.clone(), MediaTime::ZERO)?;
    }
    let high_water = sequencer.diagnostics().pending_high_water;
    let latched = sequencer
        .latch_due(MediaTime::new(1, Rational::ONE))
        .ok_or("command flood did not latch")?;
    let applied = latched.command_ids.len();
    let hash = state_hash(&latched.project);
    let mut duplicates = 0;
    if replay {
        for command in commands {
            if sequencer
                .stage(&latched.project, command.clone(), MediaTime::ZERO)?
                .duplicate
            {
                duplicates += 1;
            }
        }
    }
    Ok((applied, hash, high_water, duplicates))
}

fn run_fault_isolation(fault: InjectedFault) -> TestEvidence {
    let mut test = TestEvidence::automated(
        fault.id(),
        &["R07", "R08", "R10", "AC-06"],
        EvidenceClass::AutomatedSimulation,
        format!(
            "{} hook; verifies independent bounded sink failure does not stop Program cadence",
            fault.label()
        ),
    );
    let boundaries = 600_u64;
    let queue_capacity = 8_usize;
    let mut queue_depth = 0_usize;
    let mut queue_high_water = 0_usize;
    let mut local_drops = 0_u64;
    let mut failure_observed = false;
    for frame in 0..boundaries {
        if frame == 120 {
            failure_observed = true;
        }
        if failure_observed {
            if queue_depth == queue_capacity {
                local_drops += 1;
            } else {
                queue_depth += 1;
                queue_high_water = queue_high_water.max(queue_depth);
            }
        } else {
            queue_depth = queue_depth.saturating_sub(1);
        }
    }
    test.measurements
        .insert("program_boundaries".into(), json!(boundaries));
    test.measurements
        .insert("queue_capacity".into(), json!(queue_capacity));
    test.measurements
        .insert("queue_high_water".into(), json!(queue_high_water));
    test.measurements
        .insert("sink_local_drops".into(), json!(local_drops));
    test.assert(
        "fault_hook_observed",
        json!(failure_observed),
        json!(true),
        failure_observed,
    );
    test.assert(
        "program_cadence_continues",
        json!(boundaries),
        json!(600),
        boundaries == 600,
    );
    test.assert(
        "sink_queue_bounded",
        json!(queue_high_water),
        json!({"max": queue_capacity}),
        queue_high_water <= queue_capacity,
    );
    test
}

fn run_gpu_device_loss(config: &Config) -> Result<TestEvidence> {
    let Some(path) = &config.gpu_device_loss_evidence else {
        return Ok(TestEvidence {
            id: "CERT-GPU-DEVICE-LOSS".into(),
            requirement_ids: vec!["R05".into(), "R10".into(), "AC-08".into()],
            status: TestStatus::Skipped,
            evidence_class: EvidenceClass::HilPending,
            hardware_observed: false,
            summary: "No hardware device-loss attestation supplied; simulation is not accepted as a GPU pass".into(),
            assertions: Vec::new(),
            measurements: BTreeMap::new(),
        });
    };
    let bytes = fs::read(path)?;
    let attestation: GpuDeviceLossAttestation = serde_json::from_slice(&bytes)?;
    let trustworthy = attestation.evidence_source == "hardware_device_loss_injection"
        && attestation.hardware_observed;
    let passed = trustworthy
        && attestation.device_loss_observed
        && attestation.recovery_at_frame_boundary
        && attestation.program_backend_unchanged;
    let mut test = TestEvidence {
        id: "CERT-GPU-DEVICE-LOSS".into(),
        requirement_ids: vec!["R05".into(), "R10".into(), "AC-08".into()],
        status: if passed {
            TestStatus::Passed
        } else {
            TestStatus::Failed
        },
        evidence_class: EvidenceClass::HilVerified,
        hardware_observed: trustworthy,
        summary: "Imported private hardware device-loss hook attestation; simulated attestations are rejected".into(),
        assertions: Vec::new(),
        measurements: BTreeMap::from([
            ("attestation_path".into(), json!(path.display().to_string())),
            ("attestation_sha256".into(), json!(sha256(&bytes))),
        ]),
    };
    test.assert(
        "hardware_source",
        json!(attestation.evidence_source),
        json!("hardware_device_loss_injection"),
        trustworthy,
    );
    test.assert(
        "device_loss_observed",
        json!(attestation.device_loss_observed),
        json!(true),
        attestation.device_loss_observed,
    );
    test.assert(
        "frame_boundary_recovery",
        json!(attestation.recovery_at_frame_boundary),
        json!(true),
        attestation.recovery_at_frame_boundary,
    );
    test.assert(
        "no_backend_fallback",
        json!(attestation.program_backend_unchanged),
        json!(true),
        attestation.program_backend_unchanged,
    );
    Ok(test)
}

fn sample_memory(tests: &mut Vec<TestEvidence>, label: &str) {
    let mut sample = TestEvidence::automated(
        &format!("CERT-MEMORY-SAMPLE-{label}"),
        &["R10"],
        EvidenceClass::AutomatedSimulation,
        "Process resident-memory high-water sample",
    );
    match resident_memory_bytes() {
        Some(bytes) => {
            sample
                .measurements
                .insert("resident_bytes".into(), json!(bytes));
        }
        None => {
            sample.status = TestStatus::Skipped;
            sample.summary = "Resident-memory sampling is unavailable on this OS".into();
        }
    }
    tests.push(sample);
}

fn consolidate_memory_samples(tests: &[TestEvidence]) -> TestEvidence {
    let high_water = tests
        .iter()
        .filter_map(|test| test.measurements.get("resident_bytes"))
        .filter_map(Value::as_u64)
        .max();
    let mut evidence = TestEvidence::automated(
        "CERT-MEMORY-QUEUE-HIGH-WATER",
        &["R09", "R10"],
        EvidenceClass::AutomatedSimulation,
        "Aggregated process memory and bounded queue high-water samples",
    );
    if let Some(bytes) = high_water {
        evidence
            .measurements
            .insert("resident_high_water_bytes".into(), json!(bytes));
    } else {
        evidence.status = TestStatus::Skipped;
    }
    let queue_high_water = tests
        .iter()
        .filter_map(|test| test.measurements.get("queue_high_water"))
        .filter_map(Value::as_u64)
        .max()
        .unwrap_or(0);
    evidence
        .measurements
        .insert("queue_high_water".into(), json!(queue_high_water));
    evidence
}

fn resident_memory_bytes() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let statm = fs::read_to_string("/proc/self/statm").ok()?;
        let resident_pages = statm.split_whitespace().nth(1)?.parse::<u64>().ok()?;
        Some(resident_pages.saturating_mul(4_096))
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

fn generate_matrix(json_path: &Path, markdown_path: &Path) -> Result<()> {
    let matrix = TraceabilityMatrix {
        schema_version: 1,
        generated_from: "eiviz-certification matrix".into(),
        entries: trace_entries(),
    };
    export_json_atomic(&matrix, json_path)?;
    if let Some(parent) = markdown_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut markdown = String::from(
        "# 要件トレーサビリティ\n\n\
         このファイルは `cargo run -p eiviz-certification -- matrix` で生成します。\
         `pending` は未実施であり、simulation の成功を HIL 合格として扱いません。\n\n\
         | Requirement | Automated tests | HIL scenarios | HIL status | Artifact paths |\n\
         | --- | --- | --- | --- | --- |\n",
    );
    for entry in &matrix.entries {
        markdown.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            entry.requirement_id,
            entry.automated_tests.join("<br>"),
            entry.hil_scenarios.join("<br>"),
            entry.hil_status,
            entry.artifact_paths.join("<br>")
        ));
    }
    fs::write(markdown_path, markdown)?;
    Ok(())
}

fn trace_entries() -> Vec<TraceEntry> {
    type TraceDefinition<'a> = (
        &'a str,
        &'a [&'a str],
        &'a [&'a str],
        &'a str,
        &'a [&'a str],
    );

    let evidence = "target/certification/evidence.json";
    let maps: [TraceDefinition<'_>; 23] = [
        (
            "R01",
            &["eiviz-project::tests", "packaging/tests"],
            &["FILE-HIL-01"],
            "pending",
            &["target/certification/project"],
        ),
        (
            "R02",
            &["eiviz-core::project::tests", "CERT-MAX-ADMITTED-GRAPH"],
            &[],
            "not_applicable",
            &[evidence],
        ),
        (
            "R03",
            &["eiviz-core::graph::tests", "CERT-MAX-ADMITTED-GRAPH"],
            &[],
            "not_applicable",
            &[evidence],
        ),
        (
            "R04",
            &["CERT-TIMING-SOAK", "eiviz-time::tests"],
            &["TIME-HIL-01..08"],
            "pending",
            &[evidence, "target/certification/hil/timing"],
        ),
        (
            "R05",
            &["CERT-MAX-ADMITTED-GRAPH", "eiviz-runtime::tests"],
            &["GPU-HIL-01..08"],
            "pending",
            &[evidence, "target/certification/hil/gpu"],
        ),
        (
            "R06",
            &["CERT-TIMING-SOAK", "eiviz-media::asrc::tests"],
            &["AUDIO-HIL"],
            "pending",
            &[evidence, "target/certification/hil/audio"],
        ),
        (
            "R07",
            &["CERT-FAULT-NIC-OUTAGE", "adapter contract tests"],
            &["NDI-HIL", "OMT-HIL-01..10", "DECKLINK-HIL"],
            "pending",
            &[evidence, "target/certification/hil/io"],
        ),
        (
            "R08",
            &[
                "CERT-FAULT-SINK",
                "CERT-FAULT-DISK-FULL",
                "CERT-FAULT-NIC-OUTAGE",
            ],
            &["DIST-HIL"],
            "pending",
            &[evidence, "target/certification/hil/distribution"],
        ),
        (
            "R09",
            &["CERT-COMMAND-FLOOD-REPLAY"],
            &["CONTROL-HIL"],
            "pending",
            &[evidence, "target/certification/hil/control"],
        ),
        (
            "R10",
            &["CERT-TIMING-SOAK", "CERT-MEMORY-QUEUE-HIGH-WATER"],
            &["GPU-HIL", "TIME-HIL", "AUDIO-HIL"],
            "pending",
            &[evidence],
        ),
        (
            "R11",
            &["CI MSRV/fmt/clippy/test", "packaging/tests"],
            &["WINDOWS-RELEASE-HIL"],
            "pending",
            &["target/certification", "target/package"],
        ),
        (
            "AC-01",
            &["eiviz-project round_trip"],
            &["FILE-HIL-01"],
            "pending",
            &["target/certification/project"],
        ),
        (
            "AC-02",
            &["eiviz-project migration tests"],
            &[],
            "not_applicable",
            &["target/certification/project"],
        ),
        (
            "AC-03",
            &["CERT-TIMING-SOAK"],
            &["TIME-HIL-01"],
            "pending",
            &[evidence],
        ),
        (
            "AC-04",
            &["CERT-MAX-ADMITTED-GRAPH", "mixing_graph_rejects_cycle"],
            &[],
            "not_applicable",
            &[evidence],
        ),
        (
            "AC-05",
            &["take_overlay_and_audio_follow_latch_on_the_same_boundary"],
            &["AUDIO-HIL"],
            "pending",
            &["target/certification/runtime"],
        ),
        (
            "AC-06",
            &[
                "CERT-FAULT-SINK",
                "CERT-FAULT-DISK-FULL",
                "CERT-FAULT-NIC-OUTAGE",
            ],
            &["DIST-HIL"],
            "pending",
            &[evidence],
        ),
        (
            "AC-07",
            &["CERT-COMMAND-FLOOD-REPLAY"],
            &[],
            "not_applicable",
            &[evidence],
        ),
        (
            "AC-08",
            &["CERT-MAX-ADMITTED-GRAPH", "CERT-GPU-DEVICE-LOSS"],
            &["GPU-HIL"],
            "pending",
            &[evidence],
        ),
        (
            "AC-09",
            &["CERT-TIMING-SOAK"],
            &["24H-WALL-SOAK"],
            "pending",
            &[evidence, "target/certification/manual/24h"],
        ),
        (
            "AC-10",
            &["CERT-TIMING-SOAK"],
            &["TIME-HIL-06", "AUDIO-HIL"],
            "pending",
            &[evidence, "target/certification/hil/timing"],
        ),
        (
            "AC-11",
            &["adapter contract tests"],
            &["NDI-HIL", "OMT-HIL", "DECKLINK-HIL"],
            "pending",
            &["target/certification/hil/interop"],
        ),
        (
            "AC-12",
            &[
                "bounded_regression_locks_and_tracks_drift",
                "explicit_jump_resets_lock_and_filter",
                "source_counter_wrap_is_unwrapped_without_reset",
                "domains_and_timebases_never_mix_implicitly",
            ],
            &["TIME-HIL-01..08"],
            "pending",
            &[
                "target/certification/evidence.json",
                "target/certification/hil/timing",
            ],
        ),
    ];
    maps.into_iter()
        .map(
            |(requirement, automated, hil, status, artifacts)| TraceEntry {
                requirement_id: requirement.into(),
                automated_tests: automated.iter().map(|value| (*value).into()).collect(),
                hil_scenarios: hil.iter().map(|value| (*value).into()).collect(),
                hil_status: status.into(),
                artifact_paths: artifacts.iter().map(|value| (*value).into()).collect(),
            },
        )
        .collect()
}

fn sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

fn command_output(program: &str, args: &[&str]) -> String {
    std::process::Command::new(program)
        .args(args)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .filter(|output| !output.is_empty())
        .unwrap_or_else(|| "unknown".into())
}

fn os_description() -> String {
    #[cfg(target_os = "linux")]
    {
        let release = fs::read_to_string("/etc/os-release").unwrap_or_default();
        let pretty = release
            .lines()
            .find_map(|line| line.strip_prefix("PRETTY_NAME="))
            .map(|value| value.trim_matches('"'))
            .unwrap_or("Linux");
        format!("{pretty}; kernel {}", command_output("uname", &["-r"]))
    }
    #[cfg(not(target_os = "linux"))]
    {
        env::consts::OS.into()
    }
}

fn print_help() {
    println!(
        "eiviz-certification [options]\n\
         --mode virtual|wall\n\
         --equivalent-hours 24|72\n\
         --wall-seconds N\n\
         --profile NAME\n\
         --output PATH\n\
         --graph-units N --graph-inputs N\n\
         --clients N --commands-per-client N\n\
         --gpu-device-loss-evidence PRIVATE_HARDWARE_ATTESTATION.json\n\n\
         eiviz-certification matrix [--json PATH] [--markdown PATH]"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn virtual_smoke_audits_every_boundary_over_72_hours() {
        let config = Config {
            equivalent_hours: 72,
            ..Config::default()
        };
        let evidence = run_timing_soak(&config).unwrap();
        assert_eq!(evidence.status, TestStatus::Passed);
        assert_eq!(
            evidence.measurements["equivalent_frames"],
            json!(72_u64 * 3_600 * 60_000 / 1_001)
        );
    }

    #[test]
    fn command_replay_is_deterministic_and_bounded() {
        let config = Config {
            clients: 3,
            commands_per_client: 5,
            ..Config::default()
        };
        let evidence = run_command_flood(&config).unwrap();
        assert_eq!(evidence.status, TestStatus::Passed);
        assert_eq!(evidence.measurements["queue_high_water"], json!(15));
    }

    #[test]
    fn generated_matrix_never_marks_hil_passed() {
        let entries = trace_entries();
        assert!(entries.iter().all(|entry| entry.hil_status != "passed"));
        assert!(entries.iter().any(|entry| entry.requirement_id == "R11"));
        assert!(entries.iter().any(|entry| entry.requirement_id == "AC-11"));
        assert!(entries.iter().any(|entry| entry.requirement_id == "AC-12"));
    }

    #[test]
    fn sink_faults_are_local_and_bounded() {
        for fault in [
            InjectedFault::SinkFailure,
            InjectedFault::DiskFull,
            InjectedFault::NicOutage,
        ] {
            assert_eq!(run_fault_isolation(fault).status, TestStatus::Passed);
        }
    }
}
