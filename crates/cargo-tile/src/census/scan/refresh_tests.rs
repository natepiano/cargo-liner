#[cfg(target_os = "macos")]
use std::collections::HashMap;
#[cfg(target_os = "linux")]
use std::collections::HashSet;
#[cfg(target_os = "macos")]
use std::ffi::OsString;
use std::fs;
#[cfg(target_os = "macos")]
use std::io::BufRead;
#[cfg(target_os = "macos")]
use std::io::BufReader;
#[cfg(target_os = "macos")]
use std::io::Write;
use std::os::unix::fs::symlink;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::thread;
#[cfg(target_os = "macos")]
use std::time::Instant;

use sysinfo::Pid;
#[cfg(target_os = "linux")]
use sysinfo::ProcessesToUpdate;
#[cfg(target_os = "linux")]
use sysinfo::System;
use tempfile::tempdir;

#[cfg(target_os = "macos")]
use super::FreshProcessScan;
#[cfg(target_os = "macos")]
use super::ProcessRefreshSource;
#[cfg(target_os = "macos")]
use super::RefreshHistory;
#[cfg(target_os = "macos")]
use super::RefreshPace;
#[cfg(target_os = "macos")]
use super::RefreshWorker;
#[cfg(target_os = "macos")]
use super::ScanWorker;
#[cfg(target_os = "macos")]
use super::ScannerHomeLocation;
#[cfg(target_os = "macos")]
use super::SystemRefresh;
#[cfg(any(not(target_os = "linux"), test))]
use super::process_detail_refresh_kind;
#[cfg(any(not(target_os = "linux"), test))]
use super::process_discovery_refresh_kind;
use super::spawn_resolved_scan_loop;
use crate::birth_stamp::KernelObservation;
#[cfg(target_os = "macos")]
use crate::birth_stamp::LifetimeEvidence;
use crate::birth_stamp::Observation;
use crate::census::invocation_cpu_accounting::CensusCadence;
#[cfg(target_os = "macos")]
use crate::census::invocation_cpu_accounting::CpuBaseline;
#[cfg(target_os = "macos")]
use crate::census::invocation_cpu_accounting::InvocationCpuAccounting;
#[cfg(target_os = "macos")]
use crate::census::invocation_cpu_accounting::Measurement;
#[cfg(target_os = "macos")]
use crate::census::scan::discovery::DiscoveryCpuEvidence;
#[cfg(target_os = "macos")]
use crate::census::scan::discovery::DiscoveryProcess;
#[cfg(target_os = "macos")]
use crate::census::scan::discovery::ProcessDiscovery;
#[cfg(target_os = "macos")]
use crate::census::scan::invocation::CargoGroup;
#[cfg(target_os = "macos")]
use crate::census::scan::observation::ProcessField;
#[cfg(target_os = "macos")]
use crate::census::scan::scope::CensusScope;
#[cfg(target_os = "macos")]
use crate::census::scan::scope::ExcludedCommands;
use crate::census::scan::tests;
#[cfg(target_os = "macos")]
use crate::census::scan::tests::CENSUS_PREPARATIONS;
#[cfg(target_os = "macos")]
use crate::census::scan::tests::MetadataProcess;
#[cfg(target_os = "macos")]
use crate::census::scan::tests::TOPOLOGY_PREPARATIONS;
use crate::census::scan::tests::WORKER_REPLY_TIMEOUT;
use crate::constants::CAPTURE_LIVE_RUNS_DIR;
use crate::progress::capture::Capture;

#[cfg(target_os = "macos")]
#[test]
fn one_scan_prepares_one_census_and_one_topology() {
    let pid = Pid::from_u32(4_100_001);
    let discovery = ProcessDiscovery {
        processes: HashMap::from([(
            pid,
            DiscoveryProcess {
                name: OsString::from("cargo"),
                pid,
                parent: ProcessField::Observed(Pid::from_u32(1)),
                uid: ProcessField::Unavailable,
                cpu: DiscoveryCpuEvidence::SampledRate(0.0),
                memory: 0,
                accumulated: 0,
                started: 0,
                elapsed: 0,
            },
        )]),
        baselines: HashMap::from([(
            pid,
            CpuBaseline {
                lifetime:    LifetimeEvidence::Unavailable,
                accumulated: 0,
            },
        )]),
    };
    let cadence = CensusCadence::for_test();
    let now = Instant::now();
    let mut worker = ScanWorker {
        refresh: RefreshWorker::new(SystemRefresh::default, cadence, now),
        history: RefreshHistory::FirstObservation,
        pace: RefreshPace::Active,
        smoothing: InvocationCpuAccounting::new(cadence),
        home: ScannerHomeLocation::Unavailable,
        excluded: ExcludedCommands::new(Vec::new()),
        scope: CensusScope::default(),
        cadence,
    };
    CENSUS_PREPARATIONS.with(|count| count.set(0));
    TOPOLOGY_PREPARATIONS.with(|count| count.set(0));
    worker.scan_discovery(FreshProcessScan { discovery }, Capture::default(), now);
    CENSUS_PREPARATIONS.with(|count| assert_eq!(count.get(), 1));
    TOPOLOGY_PREPARATIONS.with(|count| assert_eq!(count.get(), 1));
}

#[cfg(target_os = "macos")]
#[test]
fn exec_into_cargo_establishes_cpu_without_losing_ancestry() {
    const FIXTURE_WORK_COUNT: u32 = 5_000;
    let root = tempdir().expect("cargo executable directory");
    let directory = root.path().canonicalize().expect("physical directory");
    let executable = directory.join("cargo-p43b-fixture");
    fs::copy("/bin/bash", &executable).expect("native external cargo fixture");
    // Darwin rejects a relocated platform shell without a new signature.
    let signature = std::process::Command::new("/usr/bin/codesign")
        .args(["--force", "--sign", "-"])
        .arg(&executable)
        .output()
        .expect("sign the owned cargo fixture");
    assert!(
        signature.status.success(),
        "{}",
        String::from_utf8_lossy(&signature.stderr)
    );
    let mut fixture = MetadataProcess {
        child: std::process::Command::new("/bin/sh")
            .args(["-c", "printf 'before\\n'; read release; exec \"$1\" -c \"$2\"", "launcher"])
            .arg(&executable)
            .arg(format!("i=0; while [ $i -lt {FIXTURE_WORK_COUNT} ]; do i=$((i+1)); done; printf 'after\\n'; read release"))
            .current_dir(&directory)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("exec fixture"),
    };
    let mut output = BufReader::new(fixture.child.stdout.take().expect("readiness pipe"));
    let mut line = String::new();
    output.read_line(&mut line).expect("shell readiness");
    assert_eq!(line, "before\n");
    let cadence = CensusCadence::for_test();
    let now = Instant::now();
    let parent = Pid::from_u32(std::process::id());
    let pid = fixture.child.id();
    let mut source = SystemRefresh::default();
    let mut worker = ScanWorker {
        refresh: RefreshWorker::new(SystemRefresh::default, cadence, now),
        history: RefreshHistory::FirstObservation,
        pace: RefreshPace::Active,
        smoothing: InvocationCpuAccounting::new(cadence),
        home: ScannerHomeLocation::Known(directory),
        excluded: ExcludedCommands::new(Vec::new()),
        scope: CensusScope::descendants_of(parent),
        cadence,
    };
    let before = worker.scan_discovery(
        FreshProcessScan {
            discovery: source.refresh(),
        },
        Capture::default(),
        now,
    );
    assert_eq!(before.groups, [] as [CargoGroup; 0]);
    fixture
        .child
        .stdin
        .as_mut()
        .expect("release pipe")
        .write_all(b"\n")
        .expect("release exec");
    line.clear();
    output.read_line(&mut line).expect("cargo readiness");
    assert_eq!(line, "after\n");
    let first = worker.scan_discovery(
        FreshProcessScan {
            discovery: source.refresh(),
        },
        Capture::default(),
        now + cadence.report,
    );
    let second = worker.scan_discovery(
        FreshProcessScan {
            discovery: source.refresh(),
        },
        Capture::default(),
        now + cadence.report * 2,
    );
    assert_eq!(first.groups.len(), 1);
    assert_eq!(second.groups.len(), 1);
    let first = &first.groups[0];
    let second = &second.groups[0];
    assert_eq!(first.lead.pid, pid);
    assert_eq!(second.lead.pid, pid);
    assert_eq!(second.lead.invocation_id, first.lead.invocation_id);
    assert_eq!(second.lead.path, "~");
    assert_eq!(
        second.ancestry.as_slice().last().expect("test parent").pid,
        parent.as_u32()
    );
    assert!(matches!(second.lead.resource_use.cpu, Measurement::Reading(cpu) if cpu.is_finite()));
}

#[test]
fn spawn_returns_while_root_resolution_waits_and_resolves_once_across_scans() {
    let parent = tempdir().expect("isolated capture parent");
    let capture = parent.path().join("capture");
    fs::create_dir(&capture).expect("capture directory");
    let caller = thread::current().id();
    let resolutions = Arc::new(AtomicUsize::new(0));
    let worker_resolutions = Arc::clone(&resolutions);
    let (started, resolution_started) = mpsc::channel();
    let (release, resolution_release) = mpsc::channel();
    let worker_capture = capture.clone();
    let (scans, worker) = spawn_resolved_scan_loop(
        CensusCadence::for_test().poll,
        move || {
            assert_ne!(thread::current().id(), caller);
            worker_resolutions.fetch_add(1, Ordering::SeqCst);
            started.send(()).expect("startup observer is alive");
            resolution_release
                .recv_timeout(WORKER_REPLY_TIMEOUT)
                .expect("spawn must return before resolution is released");
            tests::resolved_test_roots(&[&worker_capture])
        },
        move || {
            let mut generation = ["first", "second"].into_iter();
            move |roots| {
                let generation = generation.next().unwrap_or("later");
                tests::write_versioned_capture(
                    &capture,
                    4_000_000,
                    generation,
                    "/writer/project",
                    "/writer",
                    "build",
                    "",
                );
                Capture::take_roots(roots, &|pid| {
                    KernelObservation::for_test(pid, Observation::Ended)
                });
                assert!(
                    !capture
                        .join(CAPTURE_LIVE_RUNS_DIR)
                        .join(format!("4000000.{generation}"))
                        .exists()
                );
                generation
            }
        },
    );

    resolution_started
        .recv_timeout(WORKER_REPLY_TIMEOUT)
        .expect("worker must reach root resolution");
    assert!(matches!(scans.try_recv(), Err(mpsc::TryRecvError::Empty)));
    release.send(()).expect("release worker root resolution");
    assert_eq!(
        scans
            .recv_timeout(WORKER_REPLY_TIMEOUT)
            .expect("worker publishes its first scan"),
        "first"
    );
    assert_eq!(
        scans
            .recv_timeout(WORKER_REPLY_TIMEOUT)
            .expect("worker publishes its second scan"),
        "second"
    );
    drop(scans);
    worker
        .join()
        .expect("scanner must exit after receiver drop");
    assert_eq!(resolutions.load(Ordering::SeqCst), 1);
}

#[test]
fn repeated_scans_reuse_roots_resolved_before_an_ancestor_alias_changes() {
    let directory = tempdir().expect("fixture directory");
    let original = directory.path().join("original");
    let replacement = directory.path().join("replacement");
    let alias = directory.path().join("alias");
    let root = original.join("capture");
    let other = replacement.join("capture");
    let pid = std::process::id();
    tests::write_versioned_capture(
        &root,
        pid,
        "first",
        "/writer/project",
        "/writer",
        "build",
        "",
    );
    tests::write_versioned_capture(
        &other,
        pid,
        "other",
        "/runner/project",
        "/runner",
        "test",
        "",
    );
    symlink(&original, &alias).expect("original ancestor alias");
    let roots = tests::resolved_test_roots(&[&alias.join("capture")]);
    Capture::take_roots(&roots, &|observed_pid| {
        KernelObservation::for_test(observed_pid, Observation::Ended)
    });
    assert!(
        !root
            .join(CAPTURE_LIVE_RUNS_DIR)
            .join(format!("{pid}.first"))
            .exists()
    );

    fs::remove_file(&alias).expect("remove original ancestor alias");
    symlink(&replacement, &alias).expect("retarget ancestor alias");
    tests::write_versioned_capture(
        &root,
        pid,
        "next",
        "/writer/project",
        "/writer",
        "build",
        "",
    );
    Capture::take_roots(&roots, &|observed_pid| {
        KernelObservation::for_test(observed_pid, Observation::Ended)
    });

    assert!(
        !root
            .join(CAPTURE_LIVE_RUNS_DIR)
            .join(format!("{pid}.next"))
            .exists()
    );
    assert!(
        other
            .join(CAPTURE_LIVE_RUNS_DIR)
            .join(format!("{pid}.other"))
            .exists()
    );
    assert_ne!(roots, tests::resolved_test_roots(&[&alias.join("capture")]));
}

#[test]
fn scanner_process_refreshes_exclude_tasks() {
    assert!(!process_discovery_refresh_kind().tasks());
    assert!(!process_detail_refresh_kind().tasks());
}

#[cfg(target_os = "linux")]
#[test]
fn scanner_discovery_excludes_linux_tasks() -> std::io::Result<()> {
    let process_pid = std::process::id();
    let task_pids = fs::read_dir(format!("/proc/{process_pid}/task"))?
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().to_string_lossy().parse::<u32>().ok())
        .filter(|pid| *pid != process_pid)
        .map(Pid::from_u32)
        .collect::<HashSet<_>>();
    let mut system = System::new();

    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        process_discovery_refresh_kind(),
    );

    let discovered_pids = system.processes().keys().copied().collect::<HashSet<_>>();
    assert!(task_pids.is_disjoint(&discovered_pids));
    assert!(
        system
            .process(Pid::from_u32(process_pid))
            .is_some_and(|process| process.tasks().is_none())
    );
    Ok(())
}
