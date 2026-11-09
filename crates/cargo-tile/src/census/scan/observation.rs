//! Independent process fields and fresh detail observations.

#[cfg(test)]
#[path = "observation_tests.rs"]
mod observation_tests;

use std::borrow::Cow;
use std::cell::Cell;
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::path::Path;
use std::time::Duration;

use sysinfo::Pid;
#[cfg(any(not(target_os = "linux"), test))]
use sysinfo::Process;
#[cfg(any(not(target_os = "linux"), test))]
use sysinfo::System;
#[cfg(any(not(target_os = "linux"), test))]
use tui_pane::kernel_parent;

#[cfg(test)]
use super::census::Census;
use crate::birth_stamp::LifetimeEvidence;
#[cfg(test)]
use crate::birth_stamp::ProcessLifetime;
#[cfg(test)]
use crate::census::invocation_cpu_accounting::CpuBaseline;
use crate::census::invocation_cpu_accounting::Measurement;
#[cfg(any(not(target_os = "linux"), test))]
use crate::census::invocation_cpu_accounting::MeasurementAbsence;
#[cfg(test)]
use crate::constants::CARGO_DISPLAY_NAME;

/// Missing cwd cannot establish equality or be formatted before metadata merging.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::census) enum WorkingDirectoryObservation<'directory> {
    /// The process API supplied its own working directory.
    Observed(&'directory Path),
    /// A direct registration may fill this field before display conversion.
    Unavailable,
}

impl<'directory> From<Option<&'directory Path>> for WorkingDirectoryObservation<'directory> {
    fn from(directory: Option<&'directory Path>) -> Self {
        directory.map_or(Self::Unavailable, Self::Observed)
    }
}

/// An independently readable process field never substitutes for another field.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProcessField<T> {
    Observed(T),
    Unavailable,
}

impl<T> From<Option<T>> for ProcessField<T> {
    fn from(value: Option<T>) -> Self { value.map_or(Self::Unavailable, Self::Observed) }
}

/// A PID can disappear between discovery and the detailed OS refresh.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::census) enum DetailPresence {
    Observed,
    Absent,
}

/// One scan's independent metadata and CPU evidence, borrowed from its OS snapshots.
#[derive(Clone, Debug)]
pub(crate) struct ProcessObservation<'scan> {
    pub(in crate::census) pid:         Pid,
    pub(crate) parent:                 ProcessField<Pid>,
    pub(crate) name:                   ProcessField<&'scan OsStr>,
    pub(in crate::census) argv:        ProcessField<&'scan [OsString]>,
    pub(crate) directory:              ProcessField<&'scan Path>,
    pub(in crate::census) executable:  ProcessField<&'scan Path>,
    pub(crate) uid:                    ProcessField<u32>,
    pub(super) environment:            &'scan [OsString],
    pub(in crate::census) lifetime:    Cow<'scan, LifetimeEvidence>,
    pub(in crate::census) cpu:         Measurement<f32>,
    pub(in crate::census) memory:      u64,
    pub(in crate::census) accumulated: u64,
    /// Ordered accounting stores the native read while metadata remains shared.
    pub(in crate::census) native_cpu:  Cell<Measurement<Duration>>,
    pub(in crate::census) started:     u64,
    pub(in crate::census) elapsed:     u64,
    pub(in crate::census) details:     DetailPresence,
}

#[cfg(any(not(target_os = "linux"), test))]
impl<'scan> ProcessObservation<'scan> {
    #[cfg(test)]
    fn from_process(
        process: &'scan Process,
        baseline: &'scan CpuBaseline,
        previous: &HashMap<Pid, CpuBaseline>,
    ) -> Self {
        let cpu = Census::measure_cpu(process.pid(), process.cpu_usage(), baseline, previous);
        Self::metadata(process, &baseline.lifetime, cpu)
    }

    fn metadata(
        process: &'scan Process,
        lifetime: &'scan LifetimeEvidence,
        cpu: Measurement<f32>,
    ) -> Self {
        Self {
            pid: process.pid(),
            parent: process
                .parent()
                .or_else(|| kernel_parent(process.pid()))
                .into(),
            name: (!process.name().is_empty()).then(|| process.name()).into(),
            argv: (!process.cmd().is_empty()).then(|| process.cmd()).into(),
            directory: process.cwd().into(),
            executable: process.exe().into(),
            uid: process.user_id().map(|uid| **uid).into(),
            environment: process.environ(),
            cpu,
            memory: process.memory(),
            lifetime: Cow::Borrowed(lifetime),
            accumulated: process.accumulated_cpu_time(),
            native_cpu: Cell::new(Measurement::Unavailable(MeasurementAbsence::ReadFailed)),
            started: process.start_time(),
            elapsed: process.run_time(),
            details: DetailPresence::Absent,
        }
    }

    #[cfg(test)]
    pub(crate) fn cargo(pid: u32, argv: &'scan [OsString]) -> Self {
        Self {
            pid:         Pid::from_u32(pid),
            parent:      ProcessField::Unavailable,
            name:        ProcessField::Observed(OsStr::new(CARGO_DISPLAY_NAME)),
            argv:        ProcessField::Observed(argv),
            directory:   ProcessField::Observed(Path::new("/work")),
            executable:  ProcessField::Unavailable,
            uid:         ProcessField::Observed(1000),
            environment: &[],
            lifetime:    Cow::Owned(LifetimeEvidence::Available(ProcessLifetime::for_test(
                u64::from(pid),
            ))),
            cpu:         Measurement::Unavailable(MeasurementAbsence::Unproven),
            memory:      0,
            accumulated: 0,
            native_cpu:  Cell::new(Measurement::Unavailable(MeasurementAbsence::ReadFailed)),
            started:     1,
            elapsed:     0,
            details:     DetailPresence::Observed,
        }
    }
}

impl ProcessObservation<'_> {
    pub(super) const fn pid(&self) -> Pid { self.pid }
    pub(super) fn name(&self) -> &OsStr {
        match self.name {
            ProcessField::Observed(name) => name,
            ProcessField::Unavailable => OsStr::new(""),
        }
    }
    pub(in crate::census) const fn cmd(&self) -> &[OsString] {
        match self.argv {
            ProcessField::Observed(argv) => argv,
            ProcessField::Unavailable => &[],
        }
    }
    pub(in crate::census) const fn cwd(&self) -> WorkingDirectoryObservation<'_> {
        match self.directory {
            ProcessField::Observed(directory) => WorkingDirectoryObservation::Observed(directory),
            ProcessField::Unavailable => WorkingDirectoryObservation::Unavailable,
        }
    }
    pub(super) const fn exe(&self) -> ProcessField<&Path> { self.executable }
    pub(in crate::census) const fn environ(&self) -> &[OsString] { self.environment }
    pub(super) const fn start_time(&self) -> u64 { self.started }
    pub(super) const fn run_time(&self) -> u64 { self.elapsed }
}

/// A single PID index holds both discovery and fresh detail observations.
#[derive(Default)]
pub(crate) struct ProcessObservations<'scan> {
    pub(super) records: HashMap<Pid, ProcessObservation<'scan>>,
}

impl<'scan> ProcessObservations<'scan> {
    #[cfg(test)]
    pub(super) fn from_discovery(
        discovery: &'scan [(&'scan Process, CpuBaseline)],
        previous: &HashMap<Pid, CpuBaseline>,
    ) -> Self {
        Self {
            records: discovery
                .iter()
                .map(|(process, baseline)| {
                    (
                        process.pid(),
                        ProcessObservation::from_process(process, baseline, previous),
                    )
                })
                .collect(),
        }
    }

    /// Established discovery parent and lifetime evidence remain authoritative.
    #[cfg(any(not(target_os = "linux"), test))]
    pub(super) fn overlay_details(&mut self, details: &'scan System) {
        for process in details.processes().values() {
            self.overlay_record(ProcessObservation::metadata(
                process,
                &LifetimeEvidence::Unavailable,
                Measurement::Unavailable(MeasurementAbsence::Unproven),
            ));
        }
    }

    /// Both OS detail readers preserve the discovery evidence in the same PID index.
    pub(super) fn overlay_record(&mut self, mut detail: ProcessObservation<'scan>) {
        detail.details = DetailPresence::Observed;
        match self.records.entry(detail.pid) {
            Entry::Vacant(entry) => {
                entry.insert(detail);
            },
            Entry::Occupied(mut entry) => {
                let record = entry.get_mut();
                if matches!(record.parent, ProcessField::Unavailable) {
                    record.parent = detail.parent;
                }
                record.details = detail.details;
                record.name = detail.name;
                record.argv = detail.argv;
                record.directory = detail.directory;
                record.executable = detail.executable;
                record.uid = detail.uid;
                record.environment = detail.environment;
                record.started = detail.started;
                record.elapsed = detail.elapsed;
            },
        }
    }

    pub(super) fn process(&self, pid: Pid) -> Option<&ProcessObservation<'scan>> {
        self.records
            .get(&pid)
            .filter(|record| record.details == DetailPresence::Observed)
    }
    pub(super) const fn processes(&self) -> &HashMap<Pid, ProcessObservation<'scan>> {
        &self.records
    }

    #[cfg(test)]
    pub(crate) fn new(records: impl IntoIterator<Item = ProcessObservation<'scan>>) -> Self {
        Self {
            records: records
                .into_iter()
                .map(|record| (record.pid, record))
                .collect(),
        }
    }
}
