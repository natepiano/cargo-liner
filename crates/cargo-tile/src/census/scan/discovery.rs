//! Current process discovery and native lifetime evidence.

use std::borrow::Cow;
use std::cell::Cell;
use std::collections::HashMap;
#[cfg(target_os = "macos")]
use std::collections::HashSet;
use std::ffi::OsStr;
use std::ffi::OsString;
#[cfg(target_os = "linux")]
use std::fs;
#[cfg(target_os = "linux")]
use std::io::Read;
#[cfg(target_os = "macos")]
use std::path::Path;

use sysinfo::Pid;
#[cfg(not(target_os = "linux"))]
use sysinfo::Process;
#[cfg(target_os = "linux")]
use sysinfo::System;
#[cfg(not(target_os = "linux"))]
use tui_pane::kernel_parent;

#[cfg(not(target_os = "linux"))]
use super::census::Census;
#[cfg(target_os = "linux")]
use super::linux_details;
use super::observation::DetailPresence;
use super::observation::ProcessField;
use super::observation::ProcessObservation;
use super::observation::ProcessObservations;
#[cfg(target_os = "macos")]
use super::refresh::RefreshHistory;
#[cfg(all(test, target_os = "linux"))]
use super::tests;
#[cfg(target_os = "macos")]
use crate::birth_stamp;
#[cfg(target_os = "linux")]
use crate::birth_stamp::ProcessLifetime;
#[cfg(target_os = "linux")]
use crate::census::command_text;
use crate::census::invocation_cpu_accounting::CpuBaseline;
use crate::census::invocation_cpu_accounting::Measurement;
use crate::census::invocation_cpu_accounting::MeasurementAbsence;
#[cfg(target_os = "linux")]
use crate::constants::COMPILER_PROCESS_NAMES;
#[cfg(target_os = "linux")]
use crate::linux_process_stat::LinuxProcessStat;
#[cfg(target_os = "macos")]
use crate::progress::capture::Capture;

/// `ProcessDiscovery` owns current fields; baselines.lifetime holds the selected native lifetime
/// evidence.
pub(super) struct ProcessDiscovery {
    pub(super) processes: HashMap<Pid, DiscoveryProcess>,
    pub(super) baselines: HashMap<Pid, CpuBaseline>,
}

impl ProcessDiscovery {
    /// One stat observation supplies discovery and its precise birth together.
    #[cfg(target_os = "linux")]
    pub(super) fn linux() -> Self {
        let mut discovery = Self {
            processes: HashMap::new(),
            baselines: HashMap::new(),
        };
        let ticks_per_second = rustix::param::clock_ticks_per_second();
        let page_size = rustix::param::page_size() as u64;
        let boot = System::boot_time();
        let uptime = System::uptime();
        let Ok(entries) = fs::read_dir("/proc") else {
            return discovery;
        };
        let mut bytes = Vec::new();
        for entry in entries.filter_map(Result::ok) {
            let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
                continue;
            };
            bytes.clear();
            let Ok(mut stat_file) = fs::File::open(entry.path().join("stat")) else {
                continue;
            };
            if stat_file.read_to_end(&mut bytes).is_err() {
                continue;
            }
            let Ok(stat) = LinuxDiscoveryStat::parse(&bytes) else {
                continue;
            };
            if stat.pid != Pid::from_u32(pid) {
                continue;
            }
            let Some(started) = stat.start_ticks.checked_div(ticks_per_second) else {
                continue;
            };
            let Some(accumulated) = stat
                .own_ticks
                .saturating_mul(1_000)
                .checked_div(ticks_per_second)
            else {
                continue;
            };
            #[cfg(test)]
            tests::NATIVE_DISCOVERY_READS.with(|count| count.set(count.get() + 1));
            discovery.baselines.insert(
                stat.pid,
                CpuBaseline {
                    lifetime: ProcessLifetime::linux(stat.start_ticks),
                    #[cfg(test)]
                    accumulated,
                },
            );
            let arguments = if command_text::is_cargo_name(&stat.name)
                || COMPILER_PROCESS_NAMES
                    .iter()
                    .any(|name| stat.name == OsStr::new(name))
            {
                Vec::new()
            } else {
                linux_details::candidate_arguments(&entry.path(), &stat.name)
            };
            discovery.processes.insert(
                stat.pid,
                DiscoveryProcess {
                    name: stat.name,
                    arguments,
                    pid: stat.pid,
                    parent: stat.parent,
                    uid: ProcessField::Unavailable,
                    cpu: DiscoveryCpuEvidence::NativeCounter,
                    memory: stat.resident_pages.saturating_mul(page_size),
                    accumulated,
                    started: boot.saturating_add(started),
                    elapsed: uptime.saturating_sub(started),
                },
            );
        }
        discovery
    }

    #[cfg(test)]
    pub(super) fn observations(&self, previous: &Self) -> ProcessObservations<'_> {
        self.observations_since(&previous.baselines)
    }

    pub(super) fn observations_since(
        &self,
        previous: &HashMap<Pid, CpuBaseline>,
    ) -> ProcessObservations<'_> {
        ProcessObservations {
            records: self
                .processes
                .iter()
                .filter_map(|(&pid, process)| {
                    self.baselines
                        .get(&pid)
                        .map(|baseline| (pid, process.observation(baseline, previous)))
                })
                .collect(),
        }
    }
}

/// `InvocationLifetimeReads` selects processes that supply invocation rows, ancestry, or direct and
/// detached work.
#[cfg(target_os = "macos")]
pub(super) struct InvocationLifetimeReads {
    pids: HashSet<Pid>,
}

#[cfg(target_os = "macos")]
impl InvocationLifetimeReads {
    pub(super) fn for_census(census: &Census, capture: &Capture) -> Self {
        let mut roots: HashSet<_> = census
            .live_cargo()
            .chain(capture.registered_pids().into_iter().map(Pid::from_u32))
            .collect();
        if roots.is_empty() {
            return Self { pids: roots };
        }
        // Detached compiler descendants can supply work even without a cargo ancestor.
        roots.extend(census.compilers.iter().map(|&(pid, _)| pid));
        let mut pids: HashSet<_> = census.detailed().into_iter().collect();
        for &root in &roots {
            pids.extend(census.cpu_ancestry(root));
        }
        for &pid in census.accumulated.keys() {
            if census
                .cpu_ancestry(pid)
                .iter()
                .any(|pid| roots.contains(pid))
            {
                pids.insert(pid);
            }
        }
        Self { pids }
    }

    /// `InvocationLifetimeReads` adds fresh lifetimes without changing the prepared parent
    /// topology.
    pub(super) fn apply(
        self,
        census: &mut Census,
        observations: &mut ProcessObservations<'_>,
        discovery: &ProcessDiscovery,
        history: &RefreshHistory,
    ) {
        for pid in self.pids {
            let (Some(record), Some(process)) = (
                observations.records.get_mut(&pid),
                discovery.processes.get(&pid),
            ) else {
                continue;
            };
            let baseline = CpuBaseline {
                lifetime:    birth_stamp::lifetime(pid.as_u32()),
                accumulated: process.accumulated,
            };
            let DiscoveryCpuEvidence::SampledRate(rate) = process.cpu;
            record.cpu = history.measure(pid, rate, &baseline);
            census.cpu.insert(pid, record.cpu);
            census.lifetimes.insert(pid, baseline.lifetime.clone());
            record.lifetime = Cow::Owned(baseline.lifetime);
        }
    }
}

/// Fields from one Linux stat read; a partial record cannot enter discovery.
#[cfg(target_os = "linux")]
pub(super) struct LinuxDiscoveryStat {
    pub(super) pid:            Pid,
    pub(super) parent:         ProcessField<Pid>,
    pub(super) name:           OsString,
    own_ticks:                 u64,
    pub(super) start_ticks:    u64,
    pub(super) resident_pages: u64,
}

#[cfg(target_os = "linux")]
impl LinuxDiscoveryStat {
    pub(super) fn parse(bytes: &[u8]) -> Result<Self, ()> {
        let stat = LinuxProcessStat::parse(bytes)?;
        let parent = stat.parent()?;
        Ok(Self {
            pid:            Pid::from_u32(stat.pid()?),
            parent:         if parent == 0 {
                ProcessField::Unavailable
            } else {
                ProcessField::Observed(Pid::from_u32(parent))
            },
            name:           stat.name().to_os_string(),
            own_ticks:      stat.own_ticks()?,
            start_ticks:    stat.start_ticks()?,
            resident_pages: stat.resident_pages()?,
        })
    }
}

/// Linux rates come from ordered native counter reads; other platforms supply a rate.
pub(super) enum DiscoveryCpuEvidence {
    #[cfg(target_os = "linux")]
    NativeCounter,
    #[cfg(not(target_os = "linux"))]
    SampledRate(f32),
}
/// Cheap process fields copied out while the refresh worker retains its OS table.
/// Detailed metadata still comes only from the independent detail readers.
pub(super) struct DiscoveryProcess {
    #[cfg(target_os = "linux")]
    arguments:              Vec<OsString>,
    pub(super) name:        OsString,
    pub(super) pid:         Pid,
    pub(super) parent:      ProcessField<Pid>,
    pub(super) uid:         ProcessField<u32>,
    pub(super) cpu:         DiscoveryCpuEvidence,
    pub(super) memory:      u64,
    pub(super) accumulated: u64,
    pub(super) started:     u64,
    pub(super) elapsed:     u64,
}

impl DiscoveryProcess {
    #[cfg(not(target_os = "linux"))]
    pub(super) fn from_process(process: &Process) -> Self {
        Self {
            name:        Self::current_name(process),
            pid:         process.pid(),
            parent:      process
                .parent()
                .or_else(|| kernel_parent(process.pid()))
                .into(),
            uid:         process.user_id().map(|uid| **uid).into(),
            cpu:         DiscoveryCpuEvidence::SampledRate(process.cpu_usage()),
            memory:      process.memory(),
            accumulated: process.accumulated_cpu_time(),
            started:     process.start_time(),
            elapsed:     process.run_time(),
        }
    }

    /// sysinfo retains its first name across exec; discovery needs the current image.
    /// Linux reads comm afresh; macOS requests an executable-path refresh each pass.
    #[cfg(not(target_os = "linux"))]
    fn current_name(process: &Process) -> OsString {
        #[cfg(target_os = "macos")]
        {
            // The discovery refresh always rereads this path on macOS.
            process
                .exe()
                .and_then(Path::file_name)
                .map_or_else(OsString::new, OsStr::to_os_string)
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        process.name().to_os_string()
    }

    fn observation<'scan>(
        &'scan self,
        baseline: &'scan CpuBaseline,
        previous: &HashMap<Pid, CpuBaseline>,
    ) -> ProcessObservation<'scan> {
        ProcessObservation {
            pid:         self.pid,
            parent:      self.parent,
            name:        (!self.name.is_empty())
                .then_some(self.name.as_os_str())
                .into(),
            argv:        {
                #[cfg(target_os = "linux")]
                {
                    (!self.arguments.is_empty())
                        .then_some(self.arguments.as_slice())
                        .into()
                }
                #[cfg(not(target_os = "linux"))]
                {
                    ProcessField::Unavailable
                }
            },
            directory:   ProcessField::Unavailable,
            executable:  ProcessField::Unavailable,
            uid:         self.uid,
            environment: &[],
            lifetime:    Cow::Borrowed(&baseline.lifetime),
            cpu:         match self.cpu {
                #[cfg(target_os = "linux")]
                DiscoveryCpuEvidence::NativeCounter => {
                    let _ = previous;
                    Measurement::Unavailable(MeasurementAbsence::Unproven)
                },
                #[cfg(not(target_os = "linux"))]
                DiscoveryCpuEvidence::SampledRate(rate) => {
                    Census::measure_cpu(self.pid, rate, baseline, previous)
                },
            },
            memory:      self.memory,
            accumulated: self.accumulated,
            native_cpu:  Cell::new(Measurement::Unavailable(MeasurementAbsence::ReadFailed)),
            started:     self.started,
            elapsed:     self.elapsed,
            details:     DetailPresence::Absent,
        }
    }
}
