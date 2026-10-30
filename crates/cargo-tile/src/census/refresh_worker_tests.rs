use std::cell::Cell;
use std::cell::RefCell;
use std::collections::HashSet;
use std::io::BufRead;
use std::io::Write;
use std::rc::Rc;
use std::time::Duration;
use std::time::Instant;

use sysinfo::Pid;

use super::Census;
use super::CensusCadence;
use super::InvocationResourceUse;
use super::ProcessField;
use super::ProcessObservation;
use super::ProcessObservations;
use super::ProcessRefreshSource;
use super::RefreshPace;
use super::RefreshWorker;
use super::StageAnswer;
use super::row;
use crate::census::command_text::ScannerHome;
use crate::census::direct_capture::DirectAssociation;
use crate::census::invocation_cpu_accounting::Measurement;
use crate::census::invocation_cpu_accounting::MeasurementAbsence;
use crate::census::process_identity::InvocationId;

/// Changes arrive independently of the table retained by the refresh source.
#[derive(Default)]
struct ProcessInputs {
    builds:    Cell<usize>,
    refreshes: Cell<usize>,
    live:      RefCell<HashSet<u32>>,
}

struct CountingSource {
    inputs: Rc<ProcessInputs>,
    table:  HashSet<u32>,
}

impl ProcessInputs {
    fn build(self: &Rc<Self>) -> CountingSource {
        self.builds.set(self.builds.get() + 1);
        CountingSource {
            inputs: Rc::clone(self),
            table:  HashSet::new(),
        }
    }
}

impl ProcessRefreshSource for CountingSource {
    type Snapshot = HashSet<u32>;

    fn refresh(&mut self) -> Self::Snapshot {
        self.inputs.refreshes.set(self.inputs.refreshes.get() + 1);
        self.table.clone_from(&self.inputs.live.borrow());
        self.table.clone()
    }
}

#[test]
fn five_refreshes_build_one_process_table() {
    let inputs = Rc::new(ProcessInputs::default());
    inputs.live.borrow_mut().insert(41);
    let start = Instant::now();
    let mut worker = RefreshWorker::new(|| inputs.build(), CensusCadence::default(), start);
    for step in 0..5 {
        assert!(matches!(
            worker.sample(
                start + Duration::from_millis(step * 250),
                RefreshPace::Active
            ),
            StageAnswer::Completed(_)
        ));
    }
    assert_eq!(inputs.refreshes.get(), 5);
    assert_eq!(inputs.builds.get(), 1);
}

#[test]
fn half_poll_requests_start_at_most_four_refreshes_per_second() {
    let inputs = Rc::new(ProcessInputs::default());
    inputs.live.borrow_mut().insert(41);
    let start = Instant::now();
    let mut worker = RefreshWorker::new(|| inputs.build(), CensusCadence::default(), start);
    for step in 0..8 {
        worker.sample(
            start + Duration::from_millis(step * 125),
            RefreshPace::Active,
        );
    }
    assert_eq!(inputs.refreshes.get(), 4);
}

#[test]
fn an_ended_pid_leaves_the_published_snapshot() -> Result<(), &'static str> {
    let inputs = Rc::new(ProcessInputs::default());
    inputs.live.borrow_mut().extend([41, 42]);
    let start = Instant::now();
    let mut worker = RefreshWorker::new(|| inputs.build(), CensusCadence::default(), start);
    let StageAnswer::Completed(first) = worker.sample(start, RefreshPace::Active) else {
        return Err("first refresh must publish");
    };
    inputs.live.borrow_mut().remove(&42);
    let StageAnswer::Completed(second) =
        worker.sample(start + Duration::from_millis(250), RefreshPace::Active)
    else {
        return Err("second refresh must publish");
    };
    assert_eq!(
        first,
        HashSet::from([41, 42]),
        "published snapshots stay immutable"
    );
    assert_eq!(second, HashSet::from([41]));
    Ok(())
}

#[test]
fn idle_refreshes_slow_down_and_a_registration_restores_poll_pace() {
    let inputs = Rc::new(ProcessInputs::default());
    let start = Instant::now();
    let mut worker = RefreshWorker::new(|| inputs.build(), CensusCadence::default(), start);
    for step in 0..40 {
        worker.sample(start + Duration::from_millis(step * 125), RefreshPace::Idle);
    }
    assert_eq!(
        inputs.refreshes.get(),
        3,
        "two-second idle refreshes at 0, 2 and 4 seconds"
    );
    inputs.live.borrow_mut().insert(41);
    assert!(matches!(
        worker.sample(start + Duration::from_secs(5), RefreshPace::Active),
        StageAnswer::Completed(_)
    ));
    assert!(matches!(
        worker.sample(start + Duration::from_millis(5_125), RefreshPace::Active),
        StageAnswer::Pending
    ));
    assert!(matches!(
        worker.sample(start + Duration::from_millis(5_250), RefreshPace::Active),
        StageAnswer::Completed(_)
    ));
    assert_eq!(inputs.refreshes.get(), 5);
}

#[test]
fn idle_wakeup_and_late_requests_keep_the_cadences_minimum_start_spacing() {
    let inputs = Rc::new(ProcessInputs::default());
    let start = Instant::now();
    let cadence = CensusCadence {
        poll: Duration::from_millis(300),
        ..CensusCadence::default()
    };
    let mut worker = RefreshWorker::new(|| inputs.build(), cadence, start);
    worker.sample(start, RefreshPace::Idle);
    worker.sample(start + Duration::from_millis(125), RefreshPace::Active);
    assert_eq!(
        inputs.refreshes.get(),
        1,
        "a registration cannot refresh too soon"
    );
    worker.sample(start + Duration::from_millis(300), RefreshPace::Active);
    worker.sample(start + Duration::from_secs(5), RefreshPace::Active);
    worker.sample(start + Duration::from_millis(5_125), RefreshPace::Active);
    assert_eq!(
        inputs.refreshes.get(),
        3,
        "a late worker does not catch up in a burst"
    );
    worker.sample(start + Duration::from_millis(5_300), RefreshPace::Active);
    assert_eq!(inputs.refreshes.get(), 4);
}

#[test]
fn unaligned_idle_requests_discover_within_two_seconds() {
    let inputs = Rc::new(ProcessInputs::default());
    let start = Instant::now();
    let mut worker = RefreshWorker::new(|| inputs.build(), CensusCadence::default(), start);
    let first = Duration::from_millis(37);
    worker.sample(start + first, RefreshPace::Idle);
    let mut refreshes = vec![first];
    for step in 1..=40 {
        let elapsed = Duration::from_millis(step * 125);
        if matches!(
            worker.sample(start + elapsed, RefreshPace::Idle),
            StageAnswer::Completed(_)
        ) {
            refreshes.push(elapsed);
        }
    }
    assert_eq!(refreshes.len(), 3, "idle retains its lower refresh count");
    assert!(
        refreshes.windows(2).all(|pair| pair[1]
            .checked_sub(pair[0])
            .is_some_and(|elapsed| elapsed <= Duration::from_secs(2))),
        "unaligned publication requests must not postpone discovery: {refreshes:?}"
    );
}

#[test]
fn a_shell_execing_cargo_enters_the_retained_discoverys_census()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let executable = directory.path().join("cargo");
    std::fs::copy("/bin/cat", &executable)?;
    // A relocated Apple platform binary needs a local signature before exec.
    #[cfg(target_os = "macos")]
    if !std::process::Command::new("/usr/bin/codesign")
        .args(["--force", "--sign", "-", "--timestamp=none"])
        .arg(&executable)
        .status()?
        .success()
    {
        return Err("signing the cargo exec fixture failed".into());
    }
    let mut child = std::process::Command::new("/bin/sh")
        .args([
            "-c",
            "printf 'shell\\n'; read value; exec \"$1\"",
            "refresh-exec",
        ])
        .arg(&executable)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()?;
    let pid = Pid::from_u32(child.id());
    let mut input = child.stdin.take().ok_or("child input pipe")?;
    let mut output = std::io::BufReader::new(child.stdout.take().ok_or("child output pipe")?);
    let mut line = String::new();
    output.read_line(&mut line)?;
    let mut source = super::SystemRefresh {
        system: sysinfo::System::new(),
    };
    let shell = source.refresh();
    writeln!(input, "exec\nafter-exec")?;
    line.clear();
    output.read_line(&mut line)?;
    let cargo = source.refresh();
    drop(input);
    let status = child.wait()?;
    if !status.success() {
        return Err(format!("cargo exec fixture exited: {status}").into());
    }
    assert_eq!(
        line, "after-exec\n",
        "the replacement executable acknowledged its input"
    );
    assert_ne!(
        shell.processes[&pid].name, "cargo",
        "discovery first saw the shell"
    );
    assert_eq!(
        cargo.processes[&pid].name, "cargo",
        "exec must replace the discovery name"
    );
    let observations = cargo.observations(&shell);
    let census = Census::take(observations.records.values());
    assert!(
        census.live_cargo().any(|live| live == pid),
        "uncaptured cargo must restore active discovery"
    );
    assert_ne!(
        shell.processes[&pid].name, "cargo",
        "the earlier snapshot remains a shell"
    );
    Ok(())
}

#[test]
fn the_retained_system_removes_a_reaped_child_without_mutating_its_old_snapshot()
-> Result<(), Box<dyn std::error::Error>> {
    let mut child = std::process::Command::new("/bin/sh")
        .args(["-c", "read value"])
        .stdin(std::process::Stdio::piped())
        .spawn()?;
    let pid = sysinfo::Pid::from_u32(child.id());
    let mut source = super::SystemRefresh {
        system: sysinfo::System::new(),
    };
    let first = source.refresh();
    drop(child.stdin.take());
    child.wait()?;
    let second = source.refresh();
    assert!(
        first.processes.contains_key(&pid),
        "the child was live at discovery"
    );
    assert!(
        !second.processes.contains_key(&pid),
        "a reaped child leaves the retained OS table"
    );
    assert!(
        !second.baselines.contains_key(&pid),
        "ended counters leave with the process"
    );
    assert!(
        first.baselines.contains_key(&pid),
        "the published evidence remains immutable"
    );
    Ok(())
}

/// Late eligible session rows can regroup already observed children in one publication.
#[test]
fn late_session_rows_regroup_foreign_commands_without_a_process_refresh() -> Result<(), &'static str>
{
    let session = ["cargo", "fix"].map(std::ffi::OsString::from);
    let command = ["cargo", "check"].map(std::ffi::OsString::from);
    let mut processes = Vec::new();
    for parent in [1000, 2000, 3000, 4000] {
        processes.push(ProcessObservation::cargo(parent, &session));
        for pid in parent + 1..=parent + 3 {
            let mut child = ProcessObservation::cargo(pid, &command);
            child.parent = ProcessField::Observed(Pid::from_u32(parent));
            processes.push(child);
        }
    }
    let records = ProcessObservations::new(processes);
    let census = Census::take(records.records.values());
    let mut early = Vec::new();
    let mut late = Vec::new();
    for process in records.records.values() {
        let row = row(
            process,
            process.cmd(),
            InvocationId::for_test(process.pid.as_u32()),
            Measurement::Reading(0),
            ScannerHome::Unavailable,
            InvocationResourceUse::unavailable(MeasurementAbsence::Unproven),
            &DirectAssociation::None,
        )
        .map_err(|_| "fixture row must be eligible")?;
        if [1000, 2000, 3000, 4000].contains(&process.pid.as_u32()) {
            late.push(row);
        } else {
            early.push(row);
        }
    }
    let before = census.assemble_groups(&records, ScannerHome::Unavailable, early.clone());
    assert_eq!(
        before.len(),
        12,
        "unrepresented parents leave individual command families"
    );
    early.extend(late);
    let after = census.assemble_groups(&records, ScannerHome::Unavailable, early);
    assert_eq!(
        after.len(),
        4,
        "one publication groups all newly represented sessions"
    );
    assert!(after.iter().all(|group| group.rest.len() == 3));
    Ok(())
}
