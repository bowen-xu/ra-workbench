//! Explicit isolated-process synthetic I/O matrix, never application/user data.
//! Parent reads JSON reports only; it never preopens a crashed SQLite/Raft pair.
use super::{Database, SharedDatabase, journal};
use serde::{Deserialize, Serialize};
use std::{path::Path, process::Command};
#[path = "benchmark/metrics.rs"]
pub(crate) mod metrics;
#[path = "benchmark/recovery.rs"]
mod recovery;
#[path = "benchmark/runtime_metrics.rs"]
mod runtime_metrics;
#[path = "benchmark/scenario.rs"]
mod scenario;
#[path = "benchmark/verification.rs"]
mod verification;
#[path = "benchmark/workload.rs"]
mod workload;
use scenario::{Job, Mode, Shape};
const WRITE_CHILD: &str = "data_interface::benchmark::synthetic_io_child";
const RECOVERY_CHILD: &str = "data_interface::benchmark::synthetic_recovery_child";
const MARKER: &str = ".synthetic-agent-io-fixture";
#[derive(Debug, Serialize, Deserialize)]
struct Execution {
    repeat: usize,
    ordinal: usize,
    job: Job,
}
#[derive(Debug, Serialize)]
struct Report {
    schema_version: u32,
    platform: &'static str,
    profile: &'static str,
    revision: Option<String>,
    runs: usize,
    test_binary_sha256: String,
    measurement_scope: &'static str,
    coverage: serde_json::Value,
    execution_order: Vec<Execution>,
    measurements: Vec<workload::Measurement>,
    process_exit_recovery: Vec<recovery::Recovery>,
}
fn child(path: &Path, job: &Job, recover: bool, crash: bool) {
    if !recover {
        std::fs::create_dir_all(path).unwrap();
        std::fs::write(path.join(MARKER), b"research-assistant-synthetic-io-v2").unwrap();
    }
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            if recover { RECOVERY_CHILD } else { WRITE_CHILD },
            "--nocapture",
            "--test-threads=1",
        ])
        .env("RESEARCH_AGENT_IO_CHILD_PATH", path)
        .env(
            "RESEARCH_AGENT_IO_CHILD_JOB",
            serde_json::to_string(job).unwrap(),
        )
        .env(
            "RESEARCH_AGENT_IO_CHILD_CRASH",
            if crash { "1" } else { "0" },
        )
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "synthetic child failed: {} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
fn assert_fixture(path: &Path) {
    assert!(path.is_absolute());
    assert_eq!(
        std::fs::read(path.join(MARKER)).unwrap(),
        b"research-assistant-synthetic-io-v2"
    );
}
fn child_context() -> Option<(std::path::PathBuf, Job, bool)> {
    let path = std::env::var_os("RESEARCH_AGENT_IO_CHILD_PATH")?;
    Some((
        path.into(),
        serde_json::from_str(&std::env::var("RESEARCH_AGENT_IO_CHILD_JOB").unwrap()).unwrap(),
        std::env::var("RESEARCH_AGENT_IO_CHILD_CRASH").unwrap() == "1",
    ))
}
#[test]
#[ignore = "isolated synthetic writer child, invoked by explicit benchmark"]
fn synthetic_io_child() {
    let Some((path, job, crash)) = child_context() else {
        return;
    };
    if let Some(result) = workload::run(&path, job, crash) {
        std::fs::write(
            path.join("measurement.json"),
            serde_json::to_vec_pretty(&result).unwrap(),
        )
        .unwrap();
    }
}
#[test]
#[ignore = "isolated recovery child: all sampling precedes first SQLite/Raft open"]
fn synthetic_recovery_child() {
    let Some((path, _, crash)) = child_context() else {
        return;
    };
    let result = recovery::run(&path, crash);
    std::fs::write(
        path.join("recovery.json"),
        serde_json::to_vec_pretty(&result).unwrap(),
    )
    .unwrap();
}
fn selected(shape: Shape) -> bool {
    std::env::var("RESEARCH_AGENT_IO_BENCH_SHAPES").map_or(true, |list| {
        list.split(',').any(|value| value == shape.name())
    })
}
fn binary_sha256() -> String {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut file = std::fs::File::open(std::env::current_exe().unwrap()).unwrap();
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let count = file.read(&mut buffer).unwrap();
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    format!("{:x}", digest.finalize())
}
#[test]
#[ignore = "full synthetic disk matrix; explicit, isolated, one test thread, no preview/server"]
fn compares_fragment_transactions_with_actual_automatic_projection() {
    let runs = std::env::var("RESEARCH_AGENT_IO_BENCH_RUNS")
        .map_or(3, |value| value.parse::<usize>().unwrap());
    let fragments = std::env::var("RESEARCH_AGENT_IO_BENCH_FRAGMENTS")
        .map_or(1024, |value| value.parse::<usize>().unwrap());
    assert!((1..=3).contains(&runs));
    assert!((128..=4096).contains(&fragments));
    if let Ok(list) = std::env::var("RESEARCH_AGENT_IO_BENCH_SHAPES") {
        assert!(
            list.split(',')
                .all(|name| Shape::all().iter().any(|shape| shape.name() == name)),
            "unknown synthetic workload selection"
        );
    }
    let shapes = Shape::all()
        .into_iter()
        .filter(|shape| selected(*shape))
        .collect::<Vec<_>>();
    assert!(!shapes.is_empty());
    let test_binary_sha256 = binary_sha256();
    let directory = tempfile::tempdir().unwrap();
    let mut measurements = vec![];
    let mut execution_order = vec![];
    let total_groups = runs * shapes.len() * 3 * Mode::all().len();
    for repeat in 0..runs {
        let mut modes = Mode::all();
        if repeat % 2 == 1 {
            modes.reverse();
        } else {
            modes.rotate_left(repeat % 4);
        }
        for shape in &shapes {
            for query_every in [None, Some(128), Some(16)] {
                for mode in modes {
                    let job = Job {
                        mode,
                        shape: *shape,
                        query_every,
                        fragments,
                        fragment_bytes: 128,
                    };
                    let path = directory.path().join(format!(
                        "run-{repeat}-{}-{}-q{}",
                        shape.name(),
                        mode.name(),
                        query_every.unwrap_or(0)
                    ));
                    execution_order.push(Execution {
                        repeat,
                        ordinal: execution_order.len(),
                        job: job.clone(),
                    });
                    child(&path, &job, false, false);
                    let mut measurement: workload::Measurement = serde_json::from_slice(
                        &std::fs::read(path.join("measurement.json")).unwrap(),
                    )
                    .unwrap();
                    child(&path, &job, true, false);
                    measurement.clean_recovery = Some(
                        serde_json::from_slice(&std::fs::read(path.join("recovery.json")).unwrap())
                            .unwrap(),
                    );
                    measurements.push(measurement);
                    std::fs::remove_dir_all(path).unwrap();
                    eprintln!(
                        "synthetic journal {}/{} repeat={}/{} shape={} mode={} query={}",
                        measurements.len(),
                        total_groups,
                        repeat + 1,
                        runs,
                        shape.name(),
                        mode.name(),
                        query_every.map_or_else(|| "none".into(), |every| every.to_string())
                    );
                }
            }
        }
    }
    // All modes/query points/repeats must preserve exactly the same facts for a
    // shape, including canonical items, original IDs, complete attempts and turns.
    for shape in &shapes {
        let same = measurements
            .iter()
            .filter(|m| m.job.shape == *shape)
            .collect::<Vec<_>>();
        for measurement in &same {
            assert_eq!(measurement.facts, same[0].facts);
        }
    }
    let mut recovery = vec![];
    for repeat in 0..runs {
        let mut modes = Mode::all();
        if repeat % 2 == 1 {
            modes.reverse();
        }
        for mode in modes {
            let job = Job {
                mode,
                shape: Shape::FineLowCompression,
                query_every: None,
                fragments: 64,
                fragment_bytes: 128,
            };
            let path = directory
                .path()
                .join(format!("crash-{repeat}-{}", mode.name()));
            child(&path, &job, false, true);
            // Never open or inspect either engine in the parent here.
            child(&path, &job, true, true);
            recovery.push(
                serde_json::from_slice::<recovery::Recovery>(
                    &std::fs::read(path.join("recovery.json")).unwrap(),
                )
                .unwrap(),
            );
            std::fs::remove_dir_all(path).unwrap();
            eprintln!(
                "synthetic abrupt recovery {}/{} repeat={}/{} mode={}",
                recovery.len(),
                runs * Mode::all().len(),
                repeat + 1,
                runs,
                mode.name()
            );
        }
    }
    for result in &recovery {
        assert_eq!(result.facts, recovery[0].facts);
    }
    let report = Report {
        schema_version: 4,
        platform: std::env::consts::OS,
        profile: if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        revision: std::env::var("RESEARCH_AGENT_IO_BENCH_REVISION").ok(),
        runs,
        test_binary_sha256,
        measurement_scope: "counterfactual per-event repository::append_batch versus current Database::write; initialization separate; complete write and clean/abrupt recovery windows include verification, checkpoint, sync and close",
        coverage: serde_json::json!({"selectedShapes":shapes,"queries":[null,128,16],"queryUnit":"measured journal commands","fragmentParameters":"base profile controls; short fixes 16x64, eight conversations divides requested fragments; actual delta count/bytes are recorded","durabilityModes":Mode::all(),"threeAlternatingRepeats":runs==3,"allJournalShapes":shapes.len()==Shape::all().len(),"fullFComplete":false,"revisionProvided":std::env::var("RESEARCH_AGENT_IO_BENCH_REVISION").is_ok(),"revisionBinding":"caller-supplied revision; testBinarySha256 identifies the exact executed artifact",
            "notMeasured":["control/metadata I/O and truncate/allocate/flush outside managed log write/sync delegator","subscription/render first-word latency","isolated serialization CPU","workspace first-copy/retained-lease/local-intent persistence","host power loss"],
            "engineIo":"per-log delegated write/sync attempts and errors; reported_write_bytes sums Ok(n), excludes unknown partial bytes after Err; not physical disk bytes; legacy has no Raft engine and returns null",
            "engineIoWindows":"observer precedes engine construction; initialization ends at ready, append-loop ends before final verification, complete-write/recovery include sync and owner Drop; handle does not retain owner/files",
            "pending":"actual coordinator accepted-minus-materialized and retained record::encode(Stored<Accepted>) JSON payload bytes sampled after each ACK; excludes outer Raft protobuf/compression/allocator capacity and intra-write peaks",
            "serialization":"per-coordinator successful codec calls and actual produced/consumed JSON bytes; command encodes inside record validation are included, so categories overlap; serialized_command_bytes is complete-write command_encoded_bytes, not unique payload or physical I/O; recovery reports open and full windows; legacy and unsupported CPU remain null; observer adds atomic-counter overhead",
            "rss":"sampled process RSS at each 32 writes and recovery boundaries, not a proven instantaneous maximum","occupancy":"logical file lengths at sampled points, not allocated blocks or cumulative disk writes","cpuAndOsIo":"calling child process lifetime-counter differences over each stated full window, including real encoding, projection, validation and instrumentation; not isolated serialization CPU; unsupported platforms return null"}),
        execution_order,
        measurements,
        process_exit_recovery: recovery,
    };
    let encoded = serde_json::to_string_pretty(&report).unwrap();
    if let Some(path) = std::env::var_os("RESEARCH_AGENT_IO_BENCH_REPORT") {
        std::fs::write(path, &encoded).unwrap();
    }
    println!("{encoded}");
}
#[test]
fn all_matrix_workloads_reach_canonical_terminal_facts_before_any_database_is_opened() {
    for shape in Shape::all() {
        let job = Job {
            mode: Mode::AutoProcess,
            shape,
            query_every: Some(16),
            fragments: 128,
            fragment_bytes: 128,
        };
        let data = scenario::dataset(&job);
        let facts = verification::expected(&data);
        assert!(facts.canonical_items > 0);
        assert_eq!(
            facts.terminal_turns,
            match shape {
                Shape::LongHistory => 33,
                Shape::EightConversations => 8,
                _ => 1,
            }
        );
        assert_eq!(
            facts.events,
            data.setup.len() as u64 + data.measured.len() as u64
        );
    }
}
