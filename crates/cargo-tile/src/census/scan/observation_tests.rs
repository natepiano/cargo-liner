use std::collections::HashMap;
use std::fs;
use std::io::BufRead;
use std::io::BufReader;
use std::io::Write;
use std::thread;
use std::time::Instant;

use sysinfo::Pid;
#[cfg(any(not(target_os = "linux"), test))]
use sysinfo::ProcessesToUpdate;
use tempfile::tempdir;

#[cfg(any(target_os = "linux", test))]
use super::ProcessField;
#[cfg(any(target_os = "linux", test))]
use super::ProcessObservations;
use crate::birth_stamp::LifetimeEvidence;
use crate::census::invocation_cpu_accounting;
use crate::census::invocation_cpu_accounting::CpuBaseline;
use crate::census::invocation_cpu_accounting::Measurement;
use crate::census::invocation_cpu_accounting::MeasurementAbsence;
use crate::census::scan::census::Census;
#[cfg(any(not(target_os = "linux"), test))]
use crate::census::scan::refresh;
use crate::census::scan::tests::MetadataProcess;
use crate::census::scan::tests::WORKER_REPLY_TIMEOUT;

#[test]
fn discovery_lifetime_and_parent_survive_fresh_detail_overlay() {
    let pid = Pid::from_u32(std::process::id());
    let discovery_system = refresh::process_details(&[pid]);
    let process = discovery_system
        .process(pid)
        .expect("current process discovery");
    let discovery = [(process, CpuBaseline::from(process))];
    let mut observations = ProcessObservations::from_discovery(&discovery, &HashMap::new());
    assert!(
        observations.process(pid).is_none(),
        "discovery alone is not a detail observation"
    );
    let original_parent = ProcessField::Observed(Pid::from_u32(42));
    let record = observations.records.get_mut(&pid).expect("discovered PID");
    record.parent = original_parent;
    record.argv = ProcessField::Unavailable;
    record.directory = ProcessField::Unavailable;
    record.executable = ProcessField::Unavailable;
    let accumulated = record.accumulated;
    let cpu = record.cpu;
    assert!(std::ptr::eq(
        record.lifetime.as_ref(),
        &raw const discovery[0].1.lifetime
    ));
    let census = Census::take(observations.records.values());
    assert_eq!(census.parents.get(&pid), Some(&Pid::from_u32(42)));
    assert_eq!(census.lifetimes.get(&pid), Some(&discovery[0].1.lifetime));

    let details = refresh::process_details(&[pid]);
    let process = details.process(pid).expect("fresh process details");
    observations.overlay_details(&details);
    assert_eq!(observations.records.len(), 1);
    let record = observations
        .process(pid)
        .expect("detail overlay retains the PID");
    assert_eq!(record.parent, original_parent);
    assert!(std::ptr::eq(
        record.lifetime.as_ref(),
        &raw const discovery[0].1.lifetime
    ));
    assert_eq!(record.accumulated, accumulated);
    assert_eq!(record.cpu, cpu);
    assert!(std::ptr::eq(record.cmd(), process.cmd()));
    assert!(std::ptr::eq(record.environ(), process.environ()));
    assert_eq!(record.cwd(), process.cwd().into());
    assert_eq!(record.exe(), process.exe().into());
}

#[test]
fn late_details_borrow_metadata_without_inventing_discovery_evidence() {
    let pid = Pid::from_u32(std::process::id());
    let details = refresh::process_details(&[pid]);
    let process = details.process(pid).expect("current process details");
    let census = Census::take([]);
    let mut observations = ProcessObservations::from_discovery(&[], &HashMap::new());
    observations.overlay_details(&details);
    let record = observations.process(pid).expect("late detail PID retained");
    assert!(std::ptr::eq(record.cmd(), process.cmd()));
    assert!(std::ptr::eq(record.environ(), process.environ()));
    assert!(std::ptr::eq(record.name(), process.name()));
    assert_eq!(record.cwd(), process.cwd().into());
    assert_eq!(record.exe(), process.exe().into());
    assert_eq!(record.lifetime.as_ref(), &LifetimeEvidence::Unavailable);
    assert_eq!(
        record.cpu,
        Measurement::Unavailable(MeasurementAbsence::Unproven)
    );
    assert!(census.lifetimes.is_empty());
}

#[test]
fn fresh_details_replace_cached_command_directory_and_executable() {
    let root = tempdir().expect("metadata directories");
    let before = root.path().join("before");
    let after = root.path().join("after");
    fs::create_dir(&before).expect("initial directory");
    fs::create_dir(&after).expect("replacement directory");
    let before = before.canonicalize().expect("physical initial directory");
    let after = after
        .canonicalize()
        .expect("physical replacement directory");
    let mut fixture = MetadataProcess {
        child: std::process::Command::new(std::env::current_exe().expect("test executable"))
            .args([
                "--exact",
                "census::scan::tests::metadata_exec_child",
                "--nocapture",
            ])
            .env("CARGO_TILE_METADATA_EXEC_DIRECTORY", &after)
            .current_dir(&before)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("metadata fixture"),
    };
    let mut reader = BufReader::new(fixture.child.stdout.take().expect("ready pipe"));
    loop {
        let mut ready = String::new();
        assert_ne!(reader.read_line(&mut ready).expect("fixture handshake"), 0);
        if ready.trim() == "metadata ready" {
            break;
        }
    }
    let pid = Pid::from_u32(fixture.child.id());
    let mut cached = refresh::process_details(&[pid]);
    let original = cached.process(pid).expect("initial process");
    let command = original.cmd().to_vec();
    let executable = original.exe().expect("initial executable").to_path_buf();
    assert_eq!(original.cwd(), Some(before.as_path()));
    fixture
        .child
        .stdin
        .take()
        .expect("release pipe")
        .write_all(b"\n")
        .expect("release exec");
    let deadline = Instant::now() + WORKER_REPLY_TIMEOUT;
    loop {
        let refreshed = refresh::process_details(&[pid]);
        if let Some(process) = refreshed.process(pid)
            && process.cwd() == Some(after.as_path())
            && process.exe().is_some_and(|exe| exe != executable)
        {
            assert_ne!(process.cmd(), command);
            break;
        }
        assert!(
            Instant::now() < deadline,
            "fixture must exec with new metadata"
        );
        thread::sleep(invocation_cpu_accounting::poll());
    }
    cached.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        false,
        refresh::process_detail_refresh_kind(),
    );
    let retained = cached.process(pid).expect("cached process");
    assert_eq!(retained.cmd(), command);
    assert_eq!(retained.cwd(), Some(before.as_path()));
    assert_eq!(retained.exe(), Some(executable.as_path()));
    drop(fixture);
    assert!(refresh::process_details(&[pid]).process(pid).is_none());
}
