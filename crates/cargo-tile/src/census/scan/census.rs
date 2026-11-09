//! Invocation attribution and assembly from one prepared census.

#[cfg(test)]
#[path = "census_tests.rs"]
mod census_tests;

use std::cell::OnceCell;
use std::collections::HashMap;
use std::collections::HashSet;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::ffi::OsStringExt;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;
use std::time::Instant;

use rustix::io::Errno;
use rustix::process::Pid as KernelPid;
use rustix::process::test_kill_process;
use sysinfo::Pid;

use super::invocation;
use super::invocation::Ancestor;
use super::invocation::AncestryWalk;
use super::invocation::CargoGroup;
use super::invocation::CargoProcess;
use super::invocation::DirectoryComparison;
use super::invocation::InvocationExecution;
use super::invocation::ProcessOwner;
use super::invocation::RowProvenance;
use super::observation::DetailPresence;
use super::observation::ProcessField;
use super::observation::ProcessObservation;
use super::observation::ProcessObservations;
use super::observation::WorkingDirectoryObservation;
use super::resource_use::Compiler;
use super::resource_use::CompilerObservation;
use super::resource_use::InvocationResourceUse;
use super::scope::CensusScope;
#[cfg(test)]
use super::tests;
#[cfg(target_os = "linux")]
use crate::birth_stamp;
use crate::birth_stamp::LifetimeEvidence;
use crate::census::command_text;
use crate::census::command_text::CargoTransport;
use crate::census::command_text::RowAbsence;
use crate::census::command_text::ScannerHome;
#[cfg(test)]
use crate::census::cpu_observation;
#[cfg(test)]
use crate::census::cpu_observation::CpuScanPublication;
use crate::census::direct_capture::DirectAssociation;
use crate::census::direct_capture::DirectCapture;
use crate::census::direct_capture::NearestRegistration;
use crate::census::direct_capture::SelectedProof;
use crate::census::invocation_cpu_accounting;
use crate::census::invocation_cpu_accounting::CacheCommand;
use crate::census::invocation_cpu_accounting::CompileMemoryShares;
use crate::census::invocation_cpu_accounting::CompileOwner;
use crate::census::invocation_cpu_accounting::CompileRequestKey;
use crate::census::invocation_cpu_accounting::CompilerCreditRetention;
use crate::census::invocation_cpu_accounting::CompilerRequest;
use crate::census::invocation_cpu_accounting::CpuAssignment;
#[cfg(any(not(target_os = "linux"), test))]
use crate::census::invocation_cpu_accounting::CpuBaseline;
use crate::census::invocation_cpu_accounting::InvocationCpuAccounting;
use crate::census::invocation_cpu_accounting::InvocationCpuContributions;
use crate::census::invocation_cpu_accounting::InvocationMeasurements;
use crate::census::invocation_cpu_accounting::Measurement;
use crate::census::invocation_cpu_accounting::MeasurementAbsence;
use crate::census::invocation_cpu_accounting::MemoryReport;
use crate::census::process_identity;
use crate::census::process_identity::CaptureMembership;
use crate::census::process_identity::DispatchedProcess;
use crate::census::process_identity::InvocationId;
use crate::census::process_identity::ProcessIdentity;
use crate::census::process_identity::RunId;
use crate::census::process_identity::VisibleParent;
use crate::constants::CARGO_DISPLAY_NAME;
use crate::constants::CARGO_PROCESS_NAMES;
use crate::constants::CARGO_SUBCOMMAND_PREFIX;
use crate::constants::COMPILER_PROCESS_NAMES;
use crate::constants::PARENT_WALK_LIMIT;
use crate::constants::ROOT_PROCESS_PID;
use crate::constants::RUSTC_BINARY;
use crate::constants::RUSTC_OUT_DIR_FLAG;
use crate::constants::SCCACHE_BINARY;
use crate::progress::capture::Capture;
use crate::progress::capture::CaptureSelection;
use crate::progress::capture_read::CaptureLookup;
use crate::registration::RegistrationCandidate;
use crate::registration::WorkingDirectoryIdentity;
use crate::sccache::SccacheServer;
use crate::settings::AssociationSelection;
use crate::settings::CaptureAssociation;
use crate::settings::UnusedCapture;
use crate::settings::UnusedCaptureReason;

/// A bounded ancestry walk establishes an owner or preserves missing evidence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CargoAncestry {
    /// An observed parent is a retained cargo invocation.
    Owner(Pid),
    /// The next parent is unavailable; an owner remains unestablished.
    ParentUnavailable,
    /// The walk exhausted its bound; an owner remains unestablished.
    WalkLimitReached,
}

/// A native transport and its forwarded worker name the same invocation.
enum CargoProcessRole {
    /// This process starts an independent command.
    Invocation(Pid),
    /// This worker executes the command whose identity stays with the runner.
    CaptureWorker { invocation: Pid },
}

impl CargoProcessRole {
    const fn invocation(self) -> Pid {
        match self {
            Self::Invocation(pid) | Self::CaptureWorker { invocation: pid } => pid,
        }
    }
}

/// One scan owns the ancestry and read order derived from its parent index.
pub(super) struct PreparedCpuTopology {
    ancestry: HashMap<Pid, Vec<Pid>>,
    ordered:  Vec<Pid>,
}
impl PreparedCpuTopology {
    fn new(parents: &HashMap<Pid, Pid>, pids: impl Iterator<Item = Pid>) -> Self {
        #[cfg(test)]
        tests::TOPOLOGY_PREPARATIONS.with(|count| count.set(count.get() + 1));
        let ancestry: HashMap<_, _> = pids.map(|pid| (pid, Self::walk(parents, pid))).collect();
        let mut ordered: Vec<_> = ancestry.keys().copied().collect();
        ordered.sort_by_key(|pid| ancestry[pid].len());
        Self { ancestry, ordered }
    }
    fn walk(parents: &HashMap<Pid, Pid>, pid: Pid) -> Vec<Pid> {
        #[cfg(test)]
        tests::CPU_ANCESTRY_WALKS.with(|count| count.set(count.get() + 1));
        let mut chain = vec![pid];
        let mut current = pid;
        for _ in 0..PARENT_WALK_LIMIT {
            if current.as_u32() <= ROOT_PROCESS_PID {
                break;
            }
            let Some(&parent) = parents.get(&current) else {
                break;
            };
            if chain.contains(&parent) {
                break;
            }
            chain.push(parent);
            if parent.as_u32() <= ROOT_PROCESS_PID {
                break;
            }
            current = parent;
        }
        chain
    }
}

/// Cache processes that carry requests or own shared resident memory.
struct CacheMemoryRoles {
    servers:  HashSet<Pid>,
    requests: HashMap<CompileRequestKey, Vec<Pid>>,
}

/// The evidence that assigns one detached compile's resident memory.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CompileMemoryOwner {
    /// Exactly one matching client names this requesting cargo.
    Requesting(Pid),
    /// No client matches, but one cargo owns the output directory.
    ByTargetDirectory(Pid),
    /// The bytes remain visible without a cargo assignment.
    Unattributed(UnattributedCause),
}

/// Why detached compiler memory cannot enter a cargo bucket.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum UnattributedCause {
    /// More than one cargo owns the output directory.
    SharedTargetDirectory,
    /// No live cargo supplies a target directory.
    NoTargetDirectory,
    /// The compiler does not supply an output directory.
    NoOutDir,
    /// The compiler or matching client has no account observation.
    NoUid,
    /// No request or matching output directory names a live cargo.
    NoRequest,
    /// More than one client carries the same compile request.
    SharedRequest,
}

/// One scan's disjoint cargo buckets and compiler memory shares.
pub(in crate::census) struct CompileMemorySample {
    /// Each row-holding cargo keeps its own resident memory bucket.
    pub(in crate::census) cargo:  HashMap<Pid, Measurement<u64>>,
    /// The summary publishes these bytes beside the cargo buckets.
    pub(in crate::census) shares: CompileMemoryShares,
}

/// What phase one learned: the parent links, the cargo processes, and the
/// compiler processes waiting to be attributed to one of them.
pub(in crate::census) struct Census {
    /// Registration eligibility is resolved even when no process row can be built.
    pub(super) registration_eligibility: HashMap<RunId, Result<(), RowAbsence>>,
    /// Invocation identities are established before attribution and row construction.
    pub(in crate::census) identities:    HashMap<Pid, InvocationId>,
    /// Origins with a worker in an unbroken command-forwarding chain.
    pub(super) executing:                HashMap<Pid, ProcessIdentity>,
    /// Preserve cargo ancestry even after a command is deliberately excluded.
    pub(super) capture_boundaries:       HashSet<Pid>,
    /// Only observed forwarding of the registered command permits walking through a wrapper.
    pub(super) capture_wrappers:         HashSet<Pid>,
    /// Observed argv naming a different command forbids direct registration fields.
    pub(super) capture_mismatches:       HashSet<Pid>,
    /// Native lifetime evidence belongs to this refresh, including unavailable reads.
    pub(super) lifetimes:                HashMap<Pid, LifetimeEvidence>,
    /// Deliberate exclusion remains distinct from unavailable command metadata.
    pub(super) eligibility:              HashMap<Pid, Result<(), RowAbsence>>,
    /// Every process's parent, for walking a compiler up to its cargo.
    pub(super) parents:                  HashMap<Pid, Pid>,
    pub(super) topology:                 OnceCell<PreparedCpuTopology>,
    /// Cargo invocations retained for row construction after selection.
    pub(super) cargo:                    Vec<Pid>,
    /// Runs carried by hidden drivers to the process that represents their command.
    pub(super) dispatched_runs:          HashMap<Pid, usize>,
    /// Hidden invocations still establish CPU ownership and competing target evidence.
    pub(super) rowless_cargo:            Vec<Pid>,
    /// Compiler processes paired with which driver they are.
    pub(super) compilers:                Vec<(Pid, &'static str)>,
    /// Per-process validation is consulted only for an invocation's own counter.
    pub(super) cpu:                      HashMap<Pid, Measurement<f32>>,
    /// Resident bytes for every process in the discovery refresh.
    pub(super) memory:                   HashMap<Pid, u64>,
    /// Own accumulated task time is the available counter on Darwin.
    pub(super) accumulated:              HashMap<Pid, Duration>,
    /// Owners of retained invocations, named from this scan's account table.
    pub(super) owners:                   HashMap<Pid, ProcessOwner>,
    /// Fixture rows enter group assembly without a process-row measurement.
    #[cfg(test)]
    pub(super) registration_rows:        Vec<CargoProcess>,
}

#[cfg(test)]
mod topology_tests {
    use super::Census;
    use super::CpuAssignment;
    use super::HashMap;
    use super::OsStr;
    use super::OsString;
    use super::Pid;
    use super::ProcessField;
    use super::ProcessObservation;
    use super::ProcessObservations;
    use super::tests::CPU_ANCESTRY_WALKS;

    #[test]
    fn repeated_assignment_uses_one_scan_topology() {
        let argv = [OsString::from("cargo"), OsString::from("build")];
        let mut child = ProcessObservation::cargo(4_100_003, &argv);
        child.parent = ProcessField::Observed(Pid::from_u32(4_100_001));
        let records =
            ProcessObservations::new([ProcessObservation::cargo(4_100_001, &argv), child]);
        let census = Census::take(records.records.values());
        let detached = HashMap::new();
        let first = census.cpu_assignment(Pid::from_u32(4_100_003), &detached);
        CPU_ANCESTRY_WALKS.with(|count| count.set(0));
        let second = census.cpu_assignment(Pid::from_u32(4_100_003), &detached);
        assert!(matches!(first, CpuAssignment::Direct(pid) if pid == Pid::from_u32(4_100_003)));
        assert!(matches!(second, CpuAssignment::Direct(pid) if pid == Pid::from_u32(4_100_003)));
        CPU_ANCESTRY_WALKS.with(|count| assert_eq!(count.get(), 0));
    }

    #[test]
    fn unrelated_compilers_receive_no_detail_request() {
        let cargo = [OsString::from("cargo"), OsString::from("build")];
        let compiler = [OsString::from("rustc"), OsString::from("outside.rs")];
        let records = ProcessObservations::new([
            ProcessObservation::cargo(4_100_001, &cargo),
            {
                let mut record = ProcessObservation::cargo(4_100_002, &compiler);
                record.name = ProcessField::Observed(OsStr::new("rustc"));
                record
            },
            {
                let mut record = ProcessObservation::cargo(4_100_003, &compiler);
                record.name = ProcessField::Observed(OsStr::new("rustc"));
                record.parent = ProcessField::Observed(Pid::from_u32(4_100_001));
                record
            },
        ]);
        let census = Census::take(records.records.values());
        let requested = census.detailed();
        assert!(requested.contains(&Pid::from_u32(4_100_001)));
        assert!(requested.contains(&Pid::from_u32(4_100_003)));
        assert!(
            !requested.contains(&Pid::from_u32(4_100_002)),
            "a compiler outside every cargo tree must receive no detail read: {requested:?}"
        );
    }
}

impl Census {
    /// Both kernel and synthetic observations use the same discovery-to-row selection.
    pub(super) fn prepare(
        &mut self,
        records: &ProcessObservations<'_>,
        capture: &Capture,
        excluded: &[String],
    ) {
        self.include_registered_processes(records, capture);
        self.include_dispatched_processes(records);
        self.identify_capture_wrappers(records, capture);
        self.collapse_shims(records, capture);
        self.identify_capture_invocations(records, capture);
        self.identify_captures(capture);
        self.select_rows(records, capture, excluded);
        self.owners = self
            .cargo
            .iter()
            .filter_map(|&pid| {
                let process = records.process(pid)?;
                Some((pid, ProcessOwner::observe(process.uid, capture)))
            })
            .collect();
    }

    /// Both capture protocols keep the origin while forwarding workers supply its work.
    fn identify_capture_invocations(
        &mut self,
        records: &ProcessObservations<'_>,
        capture: &Capture,
    ) {
        let mut seen = HashSet::new();
        let roles: Vec<_> = self
            .cargo
            .iter()
            .map(|&pid| self.cargo_process_role(records, capture, pid))
            .collect();
        for (&pid, role) in self.cargo.iter().zip(&roles) {
            if let CargoProcessRole::CaptureWorker { invocation } = role
                && let Some(runs) = self.dispatched_runs.get(&pid).copied()
            {
                self.dispatched_runs
                    .entry(*invocation)
                    .and_modify(|carried| *carried = (*carried).max(runs))
                    .or_insert(runs);
            }
        }
        self.executing = self
            .cargo
            .iter()
            .zip(&roles)
            .filter_map(|(&pid, role)| {
                let CargoProcessRole::CaptureWorker { invocation } = role else {
                    return None;
                };
                let InvocationId::Process(executor) = self.identities.get(&pid)? else {
                    return None;
                };
                Some((*invocation, executor.clone()))
            })
            .collect();
        self.cargo = roles
            .into_iter()
            .map(CargoProcessRole::invocation)
            .filter(|pid| seen.insert(*pid))
            .collect();
    }

    /// Every intervening link must forward this command; an application ends the chain.
    fn cargo_process_role(
        &self,
        records: &ProcessObservations<'_>,
        capture: &Capture,
        pid: Pid,
    ) -> CargoProcessRole {
        let Some(candidate) = records.process(pid) else {
            return CargoProcessRole::Invocation(pid);
        };
        if Self::is_native_capture(CargoTransport::from_argv(candidate.cmd())) {
            return CargoProcessRole::Invocation(pid);
        }
        let mut workers = vec![candidate];
        let mut role = CargoProcessRole::Invocation(pid);
        for &ancestor in self.ancestor_pids(pid).as_slice().iter().rev() {
            let Some(parent) = records.process(ancestor) else {
                break;
            };
            let transport = CargoTransport::from_argv(parent.cmd());
            let direct = capture.row_source(ancestor.as_u32());
            let forwards = match (&transport, &direct) {
                (
                    CargoTransport::NativeCapture { arguments }
                    | CargoTransport::ShellCapture { arguments },
                    _,
                ) => workers.iter().all(|worker| {
                    self.forwards_or_dispatches(records, worker, |argv| {
                        forwards_cargo_arguments(argv, arguments)
                    })
                }),
                (CargoTransport::Ordinary, DirectAssociation::Direct(direct))
                    if !self.capture_mismatches.contains(&ancestor) =>
                {
                    workers.iter().all(|worker| {
                        self.forwards_or_dispatches(records, worker, |argv| {
                            forwards_capture_command(argv, direct.registration().record())
                        })
                    })
                },
                (
                    CargoTransport::Ordinary,
                    DirectAssociation::None | DirectAssociation::Direct(_),
                ) => {
                    workers.push(parent);
                    continue;
                },
            };
            if !forwards {
                break;
            }
            role = CargoProcessRole::CaptureWorker {
                invocation: ancestor,
            };
            if Self::is_native_capture(transport) {
                break;
            }
            workers.push(parent);
        }
        role
    }

    /// Dispatch evidence and direct forwarding share the same ancestry boundary.
    fn forwards_or_dispatches(
        &self,
        records: &ProcessObservations<'_>,
        worker: &ProcessObservation<'_>,
        forwards: impl FnOnce(&[OsString]) -> bool,
    ) -> bool {
        forwards(worker.cmd())
            || self
                .parents
                .get(&worker.pid())
                .and_then(|pid| records.process(*pid))
                .is_some_and(|parent| Self::is_dispatched(parent, worker))
    }

    /// A cargo-launched script keeps its dispatched pid despite an interpreter's native name.
    fn include_dispatched_processes(&mut self, records: &ProcessObservations<'_>) {
        let dispatched: Vec<_> = records
            .processes()
            .iter()
            .filter_map(|(&pid, process)| {
                let parent = self.parents.get(&pid)?;
                if !self.cargo.contains(parent) {
                    return None;
                }
                let shim = records.process(*parent)?;
                match Self::dispatch(shim, process) {
                    DispatchedProcess::Matched { pid, .. } => Some(pid),
                    DispatchedProcess::NotObserved => None,
                }
            })
            .collect();
        for pid in dispatched {
            if !self.cargo.contains(&pid) {
                self.cargo.push(pid);
            }
        }
    }

    /// Exact forwarded arguments identify a dispatched child independently of its cwd.
    fn dispatch(
        shim: &ProcessObservation<'_>,
        process: &ProcessObservation<'_>,
    ) -> DispatchedProcess {
        process_identity::dispatched_process(shim.cmd(), process.cmd(), process.pid())
    }

    /// Exact dispatch evidence identifies a forwarded cargo process.
    fn is_dispatched(parent: &ProcessObservation<'_>, child: &ProcessObservation<'_>) -> bool {
        matches!(
            Self::dispatch(parent, child),
            DispatchedProcess::Matched { .. }
        )
    }

    /// The native capture process owns its invocation boundary.
    const fn is_native_capture(transport: CargoTransport<'_>) -> bool {
        matches!(transport, CargoTransport::NativeCapture { .. })
    }

    /// Canonical external-tool arguments leave process identity and all sampled fields intact.
    fn invocation_arguments<'records>(
        &self,
        records: &'records ProcessObservations<'_>,
        process: &'records ProcessObservation<'_>,
    ) -> &'records [OsString] {
        if let Some(parent) = self.parents.get(&process.pid())
            && let Some(shim) = records.process(*parent)
            && let DispatchedProcess::Matched {
                arguments_start, ..
            } = Self::dispatch(shim, process)
        {
            return &process.cmd()[arguments_start..];
        }
        process.cmd()
    }

    /// Classify every process the last refresh saw.
    pub(in crate::census) fn take<'scan>(
        records: impl IntoIterator<Item = &'scan ProcessObservation<'scan>>,
    ) -> Self {
        #[cfg(test)]
        tests::CENSUS_PREPARATIONS.with(|count| count.set(count.get() + 1));
        let records = records.into_iter();
        let process_count = records.size_hint().0;
        let mut census = Self {
            identities:                     HashMap::new(),
            executing:                      HashMap::new(),
            capture_boundaries:             HashSet::new(),
            capture_wrappers:               HashSet::new(),
            capture_mismatches:             HashSet::new(),
            lifetimes:                      HashMap::with_capacity(process_count),
            eligibility:                    HashMap::new(),
            registration_eligibility:       HashMap::new(),
            parents:                        HashMap::with_capacity(process_count),
            topology:                       std::cell::OnceCell::new(),
            cargo:                          Vec::new(),
            rowless_cargo:                  Vec::new(),
            dispatched_runs:                HashMap::new(),
            compilers:                      Vec::new(),
            cpu:                            HashMap::with_capacity(process_count),
            memory:                         HashMap::with_capacity(process_count),
            accumulated:                    HashMap::with_capacity(process_count),
            owners:                         HashMap::new(),
            #[cfg(test)]
            registration_rows:              Vec::new(),
        };
        for process in records {
            let pid = process.pid;
            if let ProcessField::Observed(parent) = process.parent {
                census.parents.insert(pid, parent);
            }
            census
                .lifetimes
                .insert(pid, process.lifetime.as_ref().clone());
            census
                .accumulated
                .insert(pid, Duration::from_millis(process.accumulated));
            census.cpu.insert(pid, process.cpu);
            census.memory.insert(pid, process.memory);
            let name = process.name();
            if command_text::is_cargo_name(name)
                || (invocation::is_transparent(name)
                    && command_text::cargo_split(process.cmd()).is_ok_and(|arguments| {
                        arguments.start == 2 && arguments.follows_cargo_executable()
                    }))
            {
                census.cargo.push(pid);
            } else if let Some(driver) = COMPILER_PROCESS_NAMES
                .iter()
                .find(|driver| name == OsStr::new(**driver))
            {
                census.compilers.push((pid, driver));
            }
        }
        census
    }

    /// Drop each cargo `scope` leaves out before anything reads
    /// [`Self::cargo`], so it owns no row, compiler or CPU and stands in a
    /// parent chain like any other process.
    pub(super) fn restrict_to(&mut self, scope: CensusScope) {
        let parents = &self.parents;
        self.cargo.retain(|&pid| scope.contains(pid, parents));
    }

    /// Classify sysinfo's sample before a zero can lose its availability meaning.
    ///
    /// A positive previous accumulated counter is required before trusting
    /// sysinfo's rate: without it, the rate can retain an earlier value. A failed
    /// macOS task-info read resets the counter to zero but leaves the rate intact.
    /// Quantized zero counters cannot prove either a reading or a failed read.
    /// Native lifetime evidence separates same-second replacements before counters
    /// are compared. A regression within that lifetime remains unproven.
    /// An unchanged counter cannot prove a positive rate: macOS retains the old one.
    /// Nonzero counters on both samples still support a measured zero rate.
    #[cfg(any(not(target_os = "linux"), test))]
    pub(super) fn measure_cpu(
        pid: Pid,
        cpu: f32,
        baseline: &CpuBaseline,
        previous: &HashMap<Pid, CpuBaseline>,
    ) -> Measurement<f32> {
        let LifetimeEvidence::Available(lifetime) = &baseline.lifetime else {
            return Measurement::Unavailable(MeasurementAbsence::Unproven);
        };
        let Some(previous) = previous
            .get(&pid)
            .filter(|previous| matches!(&previous.lifetime, LifetimeEvidence::Available(prior) if prior == lifetime))
        else {
            return Measurement::Unavailable(MeasurementAbsence::FirstObservation);
        };
        if !cpu.is_finite() || cpu < 0.0 {
            return Measurement::Unavailable(MeasurementAbsence::ReadFailed);
        }
        if previous.accumulated == 0 || baseline.accumulated < previous.accumulated {
            return Measurement::Unavailable(MeasurementAbsence::Unproven);
        }
        if baseline.accumulated == previous.accumulated && cpu > 0.0 {
            return Measurement::Unavailable(MeasurementAbsence::RetainedRate);
        }
        Measurement::Reading(cpu)
    }

    /// Whether phase one saw an sccache server.
    ///
    /// The server is a process named [`SCCACHE_BINARY`] like any other,
    /// so it lands in `compilers` whether or not a build is running --
    /// which is what lets this answer while the machine is idle, when a
    /// hit rate is exactly what a developer is looking at.
    pub(super) fn sccache(&self) -> SccacheServer {
        if self
            .compilers
            .iter()
            .any(|&(_, driver)| driver == SCCACHE_BINARY)
        {
            SccacheServer::Running
        } else {
            SccacheServer::Stopped
        }
    }

    /// Count each cargo invocation's compiler descendants, reporting the
    /// highest-priority driver present in [`COMPILER_PROCESS_NAMES`].
    ///
    /// `sccache` outranks `rustc` because when a wrapper is in use every
    /// `rustc` is a child of one, and reporting both would double-count
    /// the same compile.
    fn attribute_compilers(&self) -> HashMap<Pid, CompilerObservation> {
        let mut tallies: HashMap<Pid, HashMap<&'static str, usize>> = HashMap::new();
        for &(pid, driver) in &self.compilers {
            if let CargoAncestry::Owner(owner) = self.owning_cargo(pid) {
                *tallies.entry(owner).or_default().entry(driver).or_default() += 1;
            }
        }
        self.cargo
            .iter()
            .map(|&owner| {
                let compiler = tallies
                    .get(&owner)
                    .and_then(|tally| {
                        COMPILER_PROCESS_NAMES.iter().find_map(|driver| {
                            tally.get(driver).map(|&count| Compiler {
                                name: driver,
                                count,
                            })
                        })
                    })
                    .map_or(CompilerObservation::None, CompilerObservation::Running);
                (owner, compiler)
            })
            .collect()
    }

    /// Everything the census has to say about each cargo invocation.
    ///
    /// The two walks are one call because they run over the same parent
    /// chains and are both spent by the same pass over the groups.
    pub(super) fn attribute(
        &self,
        system: &ProcessObservations<'_>,
        smoothing: &mut InvocationCpuAccounting,
        now: Instant,
    ) -> InvocationMeasurements {
        self.attribute_with(system, smoothing, now, |pid| {
            let measurement = self.process_cpu_time(pid);
            system.records.get(&pid).map_or(measurement, |record| {
                record.native_cpu.set(measurement);
                record.native_cpu.get()
            })
        })
    }

    /// Kernel and fixture counters share invocation smoothing and published PID readings.
    fn attribute_with(
        &self,
        system: &ProcessObservations<'_>,
        smoothing: &mut InvocationCpuAccounting,
        now: Instant,
        read: impl FnMut(Pid) -> Measurement<Duration>,
    ) -> InvocationMeasurements {
        smoothing.observed.clone_from(&self.lifetimes);
        let sampled: HashMap<_, _> = self
            .attribute_cpu_with(system, smoothing, now, read)
            .into_iter()
            .filter_map(|(pid, cpu)| {
                self.identities
                    .get(&pid)
                    .map(|identity| (identity.clone(), cpu))
            })
            .collect();
        let identities: Vec<_> = self
            .cargo
            .iter()
            .filter_map(|pid| self.identities.get(pid).cloned())
            .collect();
        let memory_sample = self.memory_sample(system);
        let memory_sampled: HashMap<_, _> = memory_sample
            .cargo
            .into_iter()
            .filter_map(|(pid, memory)| {
                self.identities
                    .get(&pid)
                    .map(|identity| (identity.clone(), memory))
            })
            .collect();
        let memory_report = if memory_sample.shares == smoothing.reported_memory_shares {
            smoothing.memory_report(now)
        } else {
            // A share transfer publishes every bucket from the same scan.
            MemoryReport::Replace
        };
        if memory_report == MemoryReport::Replace {
            smoothing.reported_memory_shares = memory_sample.shares;
        }
        let reported = smoothing.settle(&sampled, &identities, now);
        let reported_memory = smoothing.report_memory(&memory_sampled, &identities, memory_report);
        InvocationMeasurements {
            #[cfg(test)]
            cpu_scan_publication:              smoothing.observe_cpu(
                &sampled,
                &self.identities,
                now,
            ),
            memory_shares:                     smoothing.reported_memory_shares,
            compilers:                         self.attribute_compilers(),
            cpu:                               self
                .cargo
                .iter()
                .filter_map(|pid| {
                    self.identities
                        .get(pid)
                        .and_then(|identity| reported.get(identity))
                        .map(|cpu| (*pid, *cpu))
                })
                .collect(),
            memory:                            self
                .cargo
                .iter()
                .filter_map(|pid| {
                    self.identities
                        .get(pid)
                        .and_then(|identity| reported_memory.get(identity))
                        .map(|memory| (*pid, *memory))
                })
                .collect(),
        }
    }

    /// Measure accumulated invocation work between scans, validating only its own counter.
    /// Direct descendants take precedence over output-based cache attribution.
    /// Counter reads share the same ordered accounting pass for kernel and fixture samples.
    fn attribute_cpu_with(
        &self,
        system: &ProcessObservations<'_>,
        smoothing: &mut InvocationCpuAccounting,
        now: Instant,
        read: impl FnMut(Pid) -> Measurement<Duration>,
    ) -> HashMap<Pid, Measurement<f32>> {
        smoothing.identify_owners(self);
        let identities: HashSet<_> = smoothing.owners.values().cloned().collect();
        smoothing
            .invocations
            .retain(|identity, _| identities.contains(identity));
        smoothing
            .targets
            .retain(|identity, _| identities.contains(identity));
        let targets = self.observe_targets(system, &smoothing.owners, &mut smoothing.targets);
        smoothing.cache_owners.retain(|identity, _| {
            self.compiler_credit_retention(identity) == CompilerCreditRetention::Keep
        });
        smoothing.detached = self
            .detached_compilers(system, &targets)
            .into_iter()
            .filter(|&(compiler, owner)| {
                let (Ok(compiler), Ok(process)) =
                    (self.cpu_identity(compiler), self.cpu_identity(owner))
                else {
                    return false;
                };
                let Some(owner) = smoothing.owners.get(&process) else {
                    return false;
                };
                smoothing
                    .cache_owners
                    .entry(compiler)
                    .or_insert_with(|| owner.clone())
                    == owner
            })
            .collect::<HashMap<_, _>>();
        let mut work: HashMap<Pid, InvocationCpuContributions> = self
            .live_cargo()
            .map(|pid| (pid, InvocationCpuContributions::default()))
            .collect();
        let mut evidence = self.cpu.clone();
        self.collect_cpu_work(&smoothing.detached, &mut work, &mut evidence, read);
        #[cfg(target_os = "linux")]
        self.exclude_nested_cpu(&mut work, smoothing);
        work.into_iter()
            .filter_map(|(pid, work)| {
                let process = self.cpu_identity(pid).ok()?;
                let identity = smoothing.owners.get(&process)?;
                let evidence = evidence
                    .get(&pid)
                    .copied()
                    .unwrap_or(Measurement::Unavailable(MeasurementAbsence::Unproven));
                #[cfg(test)]
                cpu_observation::work(pid, identity, &work, evidence);
                let sample = smoothing
                    .invocations
                    .entry(identity.clone())
                    .or_default()
                    .measure(work, evidence, now);
                Some((pid, sample))
            })
            .collect()
    }

    /// Assign each process once from the fields already collected for this scan.
    pub(in crate::census) fn memory_sample(
        &self,
        system: &ProcessObservations<'_>,
    ) -> CompileMemorySample {
        let cargos: HashSet<_> = self.live_cargo().collect();
        let CacheMemoryRoles { servers, requests } = self.cache_memory_roles(system);
        let targets: HashMap<_, _> = cargos
            .iter()
            .filter_map(|&cargo| {
                let process = system.process(cargo)?;
                let target = invocation_cpu_accounting::cargo_target_directory(process).ok()?;
                Some((cargo, target))
            })
            .collect();
        let mut compiles = HashMap::new();
        for &(pid, driver) in &self.compilers {
            if driver != RUSTC_BINARY {
                continue;
            }
            let boundary = self
                .cpu_ancestry(pid)
                .iter()
                .find(|ancestor| servers.contains(ancestor) || cargos.contains(ancestor));
            if boundary.is_some_and(|ancestor| cargos.contains(ancestor)) {
                continue;
            }
            let owner = self.compile_memory_owner(pid, system, &requests, &targets);
            compiles.insert(pid, owner);
        }
        let mut buckets = HashMap::<Pid, u64>::new();
        let mut shares = CompileMemoryShares::default();
        for (&pid, &bytes) in &self.memory {
            if servers.contains(&pid) {
                shares.shared_compiler_cache = shares.shared_compiler_cache.saturating_add(bytes);
                continue;
            }
            let owner = self.cpu_ancestry(pid).iter().find_map(|ancestor| {
                compiles.get(ancestor).copied().or_else(|| {
                    if servers.contains(ancestor) {
                        Some(CompileMemoryOwner::Unattributed(
                            UnattributedCause::NoRequest,
                        ))
                    } else {
                        cargos
                            .contains(ancestor)
                            .then_some(CompileMemoryOwner::Requesting(*ancestor))
                    }
                })
            });
            match owner {
                Some(
                    CompileMemoryOwner::Requesting(owner)
                    | CompileMemoryOwner::ByTargetDirectory(owner),
                ) => {
                    if self.cargo.contains(&owner) {
                        let bucket = buckets.entry(owner).or_default();
                        *bucket = bucket.saturating_add(bytes);
                    }
                },
                Some(CompileMemoryOwner::Unattributed(_)) => {
                    shares.unattributed = shares.unattributed.saturating_add(bytes);
                },
                None => {},
            }
        }
        CompileMemorySample {
            cargo: self
                .cargo
                .iter()
                .map(|&pid| {
                    let memory = match self.memory.get(&pid) {
                        Some(&bytes) if bytes > 0 => {
                            Measurement::Reading(buckets.get(&pid).copied().unwrap_or_default())
                        },
                        _ => Measurement::Unavailable(MeasurementAbsence::ReadFailed),
                    };
                    (pid, memory)
                })
                .collect(),
            shares,
        }
    }

    /// `cache_memory_roles` indexes requests and discovery servers without another process read.
    fn cache_memory_roles(&self, system: &ProcessObservations<'_>) -> CacheMemoryRoles {
        let mut servers = HashSet::new();
        let mut requests: HashMap<CompileRequestKey, Vec<Pid>> = HashMap::new();
        for &(pid, driver) in &self.compilers {
            if driver != SCCACHE_BINARY {
                continue;
            }
            let Some(process) = system.records.get(&pid) else {
                continue;
            };
            if matches!(process.argv, ProcessField::Unavailable) {
                // Discovery retains the cache server without another process read.
                if self.cpu_owners(pid).next().is_none() {
                    servers.insert(pid);
                }
                continue;
            }
            match invocation_cpu_accounting::cache_command(process.cmd()) {
                CacheCommand::CompileRequest(arguments) => {
                    requests
                        .entry(CompileRequestKey::new(arguments))
                        .or_default()
                        .push(pid);
                },
                CacheCommand::Server => {
                    servers.insert(pid);
                },
                CacheCommand::Administration => {},
            }
        }
        CacheMemoryRoles { servers, requests }
    }

    /// A matching request takes precedence over output-directory evidence.
    fn compile_memory_owner(
        &self,
        pid: Pid,
        system: &ProcessObservations<'_>,
        requests: &HashMap<CompileRequestKey, Vec<Pid>>,
        targets: &HashMap<Pid, PathBuf>,
    ) -> CompileMemoryOwner {
        let Some(process) = system.process(pid) else {
            return CompileMemoryOwner::Unattributed(UnattributedCause::NoRequest);
        };
        if process.uid == ProcessField::Unavailable {
            return CompileMemoryOwner::Unattributed(UnattributedCause::NoUid);
        }
        let arguments = process.cmd().get(1..).unwrap_or_default();
        let matching = CompilerRequest::new(arguments).matching_clients(requests);
        if !matching.is_empty() {
            let mut owners = Vec::new();
            for &client in &matching {
                let Some(client_process) = system.process(client) else {
                    continue;
                };
                if client_process.uid == ProcessField::Unavailable {
                    return CompileMemoryOwner::Unattributed(UnattributedCause::NoUid);
                }
                if client_process.uid == process.uid {
                    owners.push(client);
                }
            }
            return match owners.as_slice() {
                [client] => self.cpu_owners(*client).next().map_or(
                    CompileMemoryOwner::Unattributed(UnattributedCause::NoRequest),
                    CompileMemoryOwner::Requesting,
                ),
                [] => CompileMemoryOwner::Unattributed(UnattributedCause::NoRequest),
                [_, _, ..] => CompileMemoryOwner::Unattributed(UnattributedCause::SharedRequest),
            };
        }
        let Ok(directory) =
            invocation_cpu_accounting::process_argument_path(process, RUSTC_OUT_DIR_FLAG)
        else {
            return CompileMemoryOwner::Unattributed(UnattributedCause::NoOutDir);
        };
        if targets.is_empty() {
            return CompileMemoryOwner::Unattributed(UnattributedCause::NoTargetDirectory);
        }
        let owners = targets.iter().filter_map(|(&owner, target)| {
            system
                .process(owner)
                .filter(|cargo| cargo.uid == process.uid && directory.starts_with(target))
                .map(|_| owner)
        });
        match invocation_cpu_accounting::compile_owner(owners) {
            CompileOwner::Unique(owner) => CompileMemoryOwner::ByTargetDirectory(owner),
            CompileOwner::Ambiguous => {
                CompileMemoryOwner::Unattributed(UnattributedCause::SharedTargetDirectory)
            },
            CompileOwner::Unknown => CompileMemoryOwner::Unattributed(UnattributedCause::NoRequest),
        }
    }

    /// Row eligibility never removes a live invocation from CPU ownership.
    pub(in crate::census) fn live_cargo(&self) -> impl Iterator<Item = Pid> + '_ {
        self.cargo.iter().chain(&self.rowless_cargo).copied()
    }

    fn topology(&self) -> &PreparedCpuTopology {
        self.topology.get_or_init(|| {
            PreparedCpuTopology::new(&self.parents, self.accumulated.keys().copied())
        })
    }

    /// Every consumer borrows the same scan ancestry.
    pub(super) fn cpu_ancestry(&self, pid: Pid) -> &[Pid] {
        self.topology()
            .ancestry
            .get(&pid)
            .map_or(&[], Vec::as_slice)
    }

    /// Each live cargo in the process chain can own work, even without a displayed row.
    fn cpu_owners(&self, pid: Pid) -> impl Iterator<Item = Pid> + '_ {
        self.cpu_ancestry(pid)
            .iter()
            .copied()
            .filter(|ancestor| self.live_cargo().any(|cargo| cargo == *ancestor))
    }

    /// Ancestry and detached compiler roots establish one assignment per process.
    fn cpu_assignment(&self, pid: Pid, detached: &HashMap<Pid, Pid>) -> CpuAssignment {
        if let Some(owner) = self.cpu_owners(pid).next() {
            return CpuAssignment::Direct(owner);
        }
        self.cpu_ancestry(pid)
            .iter()
            .copied()
            .find_map(|compiler| {
                detached
                    .get(&compiler)
                    .map(|&owner| CpuAssignment::Detached { owner, compiler })
            })
            .unwrap_or(CpuAssignment::Unassigned)
    }

    /// Read parents before children so a concurrent wait cannot repeat a live counter.
    fn collect_cpu_work(
        &self,
        detached: &HashMap<Pid, Pid>,
        work: &mut HashMap<Pid, InvocationCpuContributions>,
        evidence: &mut HashMap<Pid, Measurement<f32>>,
        mut read: impl FnMut(Pid) -> Measurement<Duration>,
    ) {
        // Parents are read before children: a wait completed during this pass may
        // defer time until the next sample, but cannot charge both child and reaper.
        for &pid in &self.topology().ordered {
            let assignment = self.cpu_assignment(pid, detached);
            let owner = match assignment {
                CpuAssignment::Direct(owner) | CpuAssignment::Detached { owner, .. } => owner,
                CpuAssignment::Unassigned => continue,
            };
            let Ok(identity) = self.cpu_identity(pid) else {
                continue;
            };
            let elapsed = match read(pid) {
                Measurement::Reading(elapsed) => elapsed,
                Measurement::Unavailable(reason) => {
                    if pid == owner {
                        evidence.insert(owner, Measurement::Unavailable(reason));
                    }
                    continue;
                },
            };
            #[cfg(target_os = "linux")]
            let owner_counter_is_readable = true;
            #[cfg(not(target_os = "linux"))]
            let owner_counter_is_readable = matches!(
                evidence.get(&owner),
                Some(Measurement::Unavailable(
                    MeasurementAbsence::FirstObservation | MeasurementAbsence::RetainedRate
                ))
            ) && !elapsed.is_zero();
            if pid == owner && owner_counter_is_readable {
                // Linux's lifetime-checked stat read proves even an idle owner's
                // zero counter independently of sysinfo's unavailable rate.
                // Darwin's zero may still be a failed task-info read, while a
                // nonzero counter is a sound read whatever rate sysinfo kept:
                // the invocation's rate comes from counters, never that rate.
                // InvocationCpuHistory still reports FirstObservation for the first sample.
                evidence.insert(owner, Measurement::Reading(0.0));
            }
            match assignment {
                CpuAssignment::Detached { compiler, .. } => {
                    if let Ok(root) = self.cpu_identity(compiler)
                        && let Some(work) = work.get_mut(&owner)
                    {
                        work.detached
                            .entry(root)
                            .or_default()
                            .insert(identity, elapsed);
                    }
                },
                CpuAssignment::Direct(_) => {
                    #[cfg(target_os = "linux")]
                    for ancestor in self.cpu_owners(owner) {
                        if let Some(work) = work.get_mut(&ancestor) {
                            work.tree.insert(identity.clone(), elapsed);
                        }
                    }
                    #[cfg(not(target_os = "linux"))]
                    if let Some(work) = work.get_mut(&owner) {
                        work.tree.insert(identity, elapsed);
                    }
                },
                CpuAssignment::Unassigned => {},
            }
        }
    }

    /// Keep the manager's full tree and the nested invocations' prior credits separate.
    #[cfg(target_os = "linux")]
    fn exclude_nested_cpu(
        &self,
        work: &mut HashMap<Pid, InvocationCpuContributions>,
        smoothing: &InvocationCpuAccounting,
    ) {
        let totals: HashMap<_, _> = work
            .iter()
            .map(|(&pid, work)| {
                let retained = self
                    .cpu_identity(pid)
                    .ok()
                    .and_then(|process| smoothing.owners.get(&process))
                    .and_then(|identity| smoothing.invocations.get(identity))
                    .map_or(Duration::ZERO, |cpu| cpu.tree.total);
                (pid, retained.max(work.tree.values().sum()))
            })
            .collect();
        for (&pid, &total) in &totals {
            if let Some(parent) = self.cpu_owners(pid).nth(1)
                && let Some(work) = work.get_mut(&parent)
                && let Ok(process) = self.cpu_identity(pid)
                && let Some(identity) = smoothing.owners.get(&process)
            {
                work.nested.insert(identity.clone(), total);
            }
        }
    }

    /// Missing census metadata cannot release a still-live compiler's original owner.
    fn compiler_credit_retention(&self, identity: &ProcessIdentity) -> CompilerCreditRetention {
        let pid = match identity {
            ProcessIdentity::Known { pid, lifetime } => {
                if let Some(LifetimeEvidence::Available(current)) =
                    self.lifetimes.get(&Pid::from_u32(*pid))
                {
                    return if current == lifetime {
                        CompilerCreditRetention::Keep
                    } else {
                        CompilerCreditRetention::Retire
                    };
                }
                *pid
            },
            ProcessIdentity::Unavailable { pid, .. } => *pid,
        };
        let Some(pid) = i32::try_from(pid).ok().and_then(KernelPid::from_raw) else {
            return CompilerCreditRetention::Keep;
        };
        // Signal zero checks existence without delivering a signal. Permission and
        // inspection failures preserve the credit; only ESRCH proves process absence.
        if test_kill_process(pid) == Err(Errno::SRCH) {
            CompilerCreditRetention::Retire
        } else {
            CompilerCreditRetention::Keep
        }
    }

    /// Lifetime-qualified keys cannot transfer CPU time across pid reuse.
    pub(in crate::census) fn cpu_identity(
        &self,
        pid: Pid,
    ) -> Result<ProcessIdentity, MeasurementAbsence> {
        match self.lifetimes.get(&pid) {
            Some(LifetimeEvidence::Available(lifetime)) => Ok(ProcessIdentity::Known {
                pid:      pid.as_u32(),
                lifetime: lifetime.clone(),
            }),
            _ => Err(MeasurementAbsence::Unproven),
        }
    }

    /// Linux includes completed children; other platforms supply observed task time.
    fn process_cpu_time(&self, pid: Pid) -> Measurement<Duration> {
        #[cfg(target_os = "linux")]
        {
            let result = invocation_cpu_accounting::linux_cpu_time(pid).and_then(|elapsed| {
                if self.lifetimes.get(&pid) == Some(&birth_stamp::lifetime(pid.as_u32())) {
                    Ok(elapsed)
                } else {
                    Err(MeasurementAbsence::Unproven)
                }
            });
            match result {
                Ok(elapsed) => Measurement::Reading(elapsed),
                Err(reason) => Measurement::Unavailable(reason),
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            self.accumulated.get(&pid).copied().map_or(
                Measurement::Unavailable(MeasurementAbsence::ReadFailed),
                Measurement::Reading,
            )
        }
    }

    /// Current targets can establish competition without authorizing a counter read.
    /// Only proven process lifetimes retain destinations across scans.
    fn observe_targets(
        &self,
        system: &ProcessObservations<'_>,
        owners: &HashMap<ProcessIdentity, InvocationId>,
        retained: &mut HashMap<InvocationId, HashSet<PathBuf>>,
    ) -> HashMap<Pid, HashSet<PathBuf>> {
        let mut targets: HashMap<_, _> = self
            .live_cargo()
            .map(|pid| {
                let directories = self
                    .cpu_identity(pid)
                    .ok()
                    .and_then(|process| owners.get(&process))
                    .and_then(|identity| retained.get(identity))
                    .cloned()
                    .unwrap_or_default();
                (pid, directories)
            })
            .collect();
        for pid in self.live_cargo() {
            if let Some(process) = system.process(pid)
                && let Ok(directory) = invocation_cpu_accounting::cargo_target_directory(process)
            {
                targets.entry(pid).or_default().insert(directory);
            }
        }
        for &(pid, _) in &self.compilers {
            if let Some(owner) = self.cpu_owners(pid).next()
                && let Some(process) = system.process(pid)
                && let Ok(directory) =
                    invocation_cpu_accounting::process_argument_path(process, RUSTC_OUT_DIR_FLAG)
            {
                targets.entry(owner).or_default().insert(directory);
            }
        }
        for (&pid, directories) in &targets {
            if let Ok(process) = self.cpu_identity(pid)
                && let Some(identity) = owners.get(&process)
            {
                retained.insert(identity.clone(), directories.clone());
            }
        }
        targets
    }

    /// Only a detached rustc beneath sccache may use output-directory ownership.
    fn detached_compilers(
        &self,
        system: &ProcessObservations<'_>,
        targets: &HashMap<Pid, HashSet<PathBuf>>,
    ) -> HashMap<Pid, Pid> {
        self.compilers
            .iter()
            .filter_map(|&(pid, driver)| {
                if driver != RUSTC_BINARY
                    || self.cpu_owners(pid).next().is_some()
                    || !self
                        .cpu_ancestry(pid)
                        .iter()
                        .any(|ancestor| self.compilers.contains(&(*ancestor, SCCACHE_BINARY)))
                {
                    return None;
                }
                let process = system.process(pid)?;
                let directory =
                    invocation_cpu_accounting::process_argument_path(process, RUSTC_OUT_DIR_FLAG)
                        .ok()?;
                let owners = self.live_cargo().filter(|owner| {
                    let Some(cargo) = system.process(*owner) else {
                        return false;
                    };
                    // A shared filesystem path alone never links work from different users.
                    if process.uid == ProcessField::Unavailable || process.uid != cargo.uid {
                        return false;
                    }
                    targets.get(owner).is_some_and(|targets| {
                        targets.iter().any(|target| directory.starts_with(target))
                    })
                });
                match invocation_cpu_accounting::compile_owner(owners) {
                    CompileOwner::Unique(owner) => Some((pid, owner)),
                    CompileOwner::Unknown | CompileOwner::Ambiguous => None,
                }
            })
            .collect()
    }

    /// Drop every cargo that is only a shim in front of another cargo.
    ///
    /// A shim that wraps cargo is itself named `cargo` -- that is the
    /// whole point of a shim -- so one command can present as two
    /// processes. What separates that from a command *managing* other
    /// cargo commands is whether the child is the same command: a shim
    /// hands its line straight on, so the subcommand and the working
    /// directory both match, while `cargo mend` running a `cargo nextest`
    /// suite matches neither. The shim goes and the process doing the
    /// work stays; the manager stays and keeps its children.
    ///
    /// Looping rather than one pass because a shim can stand in front of
    /// a shim, and dropping the outer one is what reveals the next.
    fn collapse_shims(&mut self, system: &ProcessObservations<'_>, capture: &Capture) {
        // Runs are counted only after cargo-named forwarding aliases have gone.
        // Interpreted drivers admitted for dispatch naming keep their reading;
        // the aliases themselves must never become runs in that reading.
        loop {
            let children = self.cargo_children();
            let aliases: Vec<_> = self
                .cargo
                .iter()
                .copied()
                .filter(|pid| {
                    let Some(outer) = system.process(*pid) else {
                        return false;
                    };
                    if !command_text::is_cargo_name(outer.name())
                        || !self.is_shim(system, capture, &children, *pid)
                    {
                        return false;
                    }
                    let child = children[pid][0];
                    let Some(inner) = system.process(child) else {
                        return false;
                    };
                    observed_shim_match(outer.cmd(), outer.cwd(), inner.cmd(), inner.cwd())
                })
                .collect();
            if aliases.is_empty() {
                break;
            }
            self.cargo.retain(|pid| !aliases.contains(pid));
        }
        loop {
            let children = self.cargo_children();
            let shims: Vec<Pid> = self
                .cargo
                .iter()
                .copied()
                .filter(|&pid| self.is_shim(system, capture, &children, pid))
                .collect();
            if shims.is_empty() {
                return;
            }
            for &shim in &shims {
                let child = children[&shim][0];
                let Some(outer) = system.process(shim) else {
                    continue;
                };
                let mut representative = child;
                while shims.contains(&representative) {
                    representative = children[&representative][0];
                }
                let Some(inner) = system.process(representative) else {
                    continue;
                };
                // Preserve the runs of an external dispatch chain. A shim in front of
                // a built-in cargo command carries no runs of its own to preserve.
                if !Self::is_dispatched(outer, inner) && !self.dispatched_runs.contains_key(&shim) {
                    continue;
                }
                let runs = Self::descendants(&children, shim)
                    .len()
                    .max(self.dispatched_runs.get(&shim).copied().unwrap_or_default());
                self.dispatched_runs
                    .entry(representative)
                    .and_modify(|carried| *carried = (*carried).max(runs))
                    .or_insert(runs);
            }
            self.cargo.retain(|pid| !shims.contains(pid));
        }
    }

    /// Whether `pid` is a shim in front of the one cargo beneath it.
    fn is_shim(
        &self,
        system: &ProcessObservations<'_>,
        capture: &Capture,
        children: &HashMap<Pid, Vec<Pid>>,
        pid: Pid,
    ) -> bool {
        let Some(kids) = children.get(&pid) else {
            return false;
        };
        let [child] = kids[..] else {
            return false;
        };
        let (Some(outer), Some(inner)) = (system.process(pid), system.process(child)) else {
            return false;
        };
        if Self::is_native_capture(CargoTransport::from_argv(outer.cmd())) {
            return false;
        }
        if matches!(
            CargoTransport::from_argv(outer.cmd()),
            CargoTransport::ShellCapture { .. }
        ) && !matches!(
            self.cargo_process_role(system, capture, child),
            CargoProcessRole::CaptureWorker { .. }
        ) {
            return false;
        }
        if Self::is_dispatched(outer, inner) {
            return true;
        }
        if inner
            .cmd()
            .first()
            .and_then(|program| Path::new(program).file_name())
            .and_then(|program| program.to_str())
            .is_some_and(|name| {
                name.starts_with(CARGO_SUBCOMMAND_PREFIX) && !CARGO_PROCESS_NAMES.contains(&name)
            })
        {
            return false;
        }
        observed_shim_match(outer.cmd(), outer.cwd(), inner.cmd(), inner.cwd())
    }

    /// Each cargo's direct cargo children -- the tree the groups are cut
    /// from, with a process that came out as its own ancestor dropped so
    /// a walk of it cannot loop.
    fn cargo_children(&self) -> HashMap<Pid, Vec<Pid>> {
        let mut children: HashMap<Pid, Vec<Pid>> = HashMap::new();
        for &pid in &self.cargo {
            if let CargoAncestry::Owner(owner) = self.owning_cargo(pid)
                && owner != pid
            {
                children.entry(owner).or_default().push(pid);
            }
        }
        children
    }

    /// Every cargo running under `root`, at any depth.
    pub(super) fn descendants(children: &HashMap<Pid, Vec<Pid>>, root: Pid) -> Vec<Pid> {
        let mut seen: HashSet<Pid> = HashSet::from([root]);
        let mut queue = vec![root];
        let mut out = Vec::new();
        while let Some(pid) = queue.pop() {
            let Some(kids) = children.get(&pid) else {
                continue;
            };
            for &kid in kids {
                if seen.insert(kid) {
                    out.push(kid);
                    queue.push(kid);
                }
            }
        }
        out
    }

    /// Walk `pid` up its parent chain to the nearest ancestor the cell
    /// actually draws, which is what the `parent` column names.
    ///
    /// Two things are drawn: the cargo invocations, which are the rows
    /// of the table, and the chain block standing over it. The first
    /// ancestor that is one of them is the answer, and it is the only
    /// answer worth writing down -- a pid the screen shows nowhere is a
    /// number the eye cannot pair with anything.
    ///
    /// Which is why the immediate parent is never it. A captured run's
    /// parent is the pty the shim opened, whose parent is the shim,
    /// whose parent is the shell; none of the three is drawn, and the
    /// shell is the first thing above them that is. For an invocation
    /// another cargo started, the same walk stops one step sooner, at
    /// that cargo's own row.
    ///
    /// Bounded by [`PARENT_WALK_LIMIT`], like every other walk here, so
    /// a reparented chain that loops cannot spin.
    fn drawn_parent(&self, pid: Pid, ancestry: &[Ancestor]) -> VisibleParent {
        let mut current = pid;
        for _ in 0..PARENT_WALK_LIMIT {
            let Some(&parent) = self.parents.get(&current) else {
                break;
            };
            if self.cargo.contains(&parent) {
                if let Some(identity) = self.identities.get(&parent) {
                    return VisibleParent::Invocation {
                        id:  identity.clone(),
                        pid: parent.as_u32(),
                    };
                }
            } else if ancestry
                .iter()
                .any(|ancestor| ancestor.pid == parent.as_u32())
            {
                return VisibleParent::Ancestor(parent.as_u32());
            }
            current = parent;
        }
        VisibleParent::None
    }

    /// Walk `pid` up its parent chain to the cargo invocation that owns
    /// it, bounded by [`PARENT_WALK_LIMIT`] so a reparented process whose
    /// chain loops back on itself cannot spin here.
    fn owning_cargo(&self, pid: Pid) -> CargoAncestry {
        let mut current = pid;
        for _ in 0..PARENT_WALK_LIMIT {
            let Some(&parent) = self.parents.get(&current) else {
                return CargoAncestry::ParentUnavailable;
            };
            if self.cargo.contains(&parent) {
                return CargoAncestry::Owner(parent);
            }
            current = parent;
        }
        CargoAncestry::WalkLimitReached
    }

    /// Read detailed metadata for cargo, its ancestors, and compiler drivers.
    ///
    /// A superset of what any cell ends up listing. Which ancestors are
    /// the invocation's own plumbing is settled on argv, which is what
    /// this pass is about to read, so the filtering waits for
    /// [`Self::ancestry`] and the extra reads are a handful of
    /// processes.
    pub(super) fn detailed(&self) -> Vec<Pid> {
        let mut pids = self.cargo.clone();
        pids.extend(self.compilers.iter().filter_map(|&(pid, driver)| {
            let attached = self.cpu_owners(pid).next().is_some();
            let detached_candidate = !self.cargo.is_empty()
                && driver == RUSTC_BINARY
                && self
                    .cpu_ancestry(pid)
                    .iter()
                    .any(|ancestor| self.compilers.contains(&(*ancestor, SCCACHE_BINARY)));
            (attached || detached_candidate).then_some(pid)
        }));
        for (&pid, parent) in &self.parents {
            if self.cargo.contains(parent) && !pids.contains(&pid) {
                pids.push(pid);
            }
        }
        for &pid in &self.cargo {
            for &ancestor in self.ancestor_pids(pid).as_slice() {
                if !pids.contains(&ancestor) {
                    pids.push(ancestor);
                }
            }
        }
        pids
    }

    /// Every process standing above `pid`, outermost first.
    ///
    /// The walk stops short of the init process the whole tree roots
    /// at: everything on the machine descends from it, so a row naming
    /// it tells one command from no other. [`PARENT_WALK_LIMIT`] bounds
    /// it the way it bounds the compiler walk, and a pid already on the
    /// chain ends it outright, so a reparented cycle cannot spin here.
    pub(super) fn ancestor_pids(&self, pid: Pid) -> AncestryWalk<Pid> {
        let mut chain = Vec::new();
        let mut current = pid;
        while let Some(&parent) = self.parents.get(&current) {
            if parent.as_u32() <= ROOT_PROCESS_PID || chain.contains(&parent) {
                break;
            }
            if chain.len() == PARENT_WALK_LIMIT {
                chain.reverse();
                return AncestryWalk::LimitCut(chain);
            }
            chain.push(parent);
            current = parent;
        }
        chain.reverse();
        AncestryWalk::Ended(chain)
    }

    /// What stands above `pid`, as the command's own cell lists it:
    /// outermost first, each entry a pid and what that process is.
    ///
    /// One kind of process is dropped outright: the wrappers belonging
    /// to the invocation itself. A captured run reaches cargo through a
    /// shim and a pty, both running this same cargo command line, and
    /// listing them would answer "what started this" with the machinery
    /// this tool installed to watch it.
    ///
    /// The shells and login processes that merely passed the command
    /// through are marked rather than dropped -- see
    /// [`Ancestor::passes_through`].
    ///
    /// A cargo ancestor that is *not* plumbing cannot reach here:
    /// [`Self::groups`] leads a group with an invocation that has no
    /// cargo above it.
    pub(super) fn ancestry(
        &self,
        system: &ProcessObservations<'_>,
        home: ScannerHome<'_>,
        pid: Pid,
    ) -> AncestryWalk<Ancestor> {
        self.ancestor_pids(pid).filter_map(|pid| {
            let process = system.process(pid)?;
            if command_text::names_cargo(self.invocation_arguments(system, process)) {
                return None;
            }
            Some(Ancestor {
                pid:            process.pid().as_u32(),
                command:        invocation::describe(process, home),
                passes_through: invocation::is_transparent(process.name()),
            })
        })
    }

    /// What the capture behind `pid` reports, if one is behind it.
    ///
    /// The walk goes upward because the pid a capture is filed under is
    /// the shim's, and the shim is an ancestor of the cargo it started
    /// rather than the cargo itself -- two levels up when the run went
    /// through a pty, one when it did not. The same bound the compiler
    /// walk uses stops a reparented cycle here.
    fn captured_run(&self, capture: &Capture, pid: Pid) -> NearestRegistration {
        let mut walking = pid;
        for _ in 0..PARENT_WALK_LIMIT {
            match capture.select(walking.as_u32()) {
                CaptureSelection::Selected(key) => return NearestRegistration::Registered(key),
                CaptureSelection::Ambiguous(candidates) => {
                    return NearestRegistration::Ambiguous(candidates);
                },
                CaptureSelection::Unregistered => {},
            }
            let Some(parent) = self.parents.get(&walking) else {
                return NearestRegistration::Unregistered;
            };
            walking = *parent;
        }
        NearestRegistration::Unregistered
    }

    /// Report every retained selection, including a shim absent from the process table.
    pub(super) fn associate_status(&self, capture: &mut Capture, groups: &[CargoGroup]) {
        let mut associations = Vec::new();
        for row in groups
            .iter()
            .flat_map(|group| std::iter::once(&group.lead).chain(&group.rest))
        {
            // Diagnostics describe the observed source, not its projected display pid.
            let source = self
                .cargo
                .iter()
                .copied()
                .find(|pid| self.identities.get(pid) == Some(&row.invocation_id))
                .unwrap_or_else(|| Pid::from_u32(row.pid));
            associations.push((source.as_u32(), self.captured_run(capture, source)));
        }
        for pid in capture.registered_pids() {
            if !associations.iter().any(|(_, nearest)| match nearest {
                NearestRegistration::Registered(key) => key.pid == pid,
                NearestRegistration::Ambiguous(keys) => keys.iter().any(|key| key.pid == pid),
                NearestRegistration::Unregistered => false,
            }) {
                associations.push((pid, self.captured_run(capture, Pid::from_u32(pid))));
            }
        }
        for (pid, nearest) in associations {
            let (root, selection) = match nearest {
                NearestRegistration::Unregistered => continue,
                NearestRegistration::Ambiguous(candidates) => {
                    let Some(key) = candidates.first() else {
                        continue;
                    };
                    (key.root, AssociationSelection::Ambiguous { candidates })
                },
                NearestRegistration::Registered(key) => {
                    let proof = if capture
                        .confirmed()
                        .iter()
                        .any(|confirmed| confirmed.key == key)
                    {
                        SelectedProof::Confirmed
                    } else {
                        SelectedProof::Unconfirmed
                    };
                    let unused = capture
                        .confirmed()
                        .iter()
                        .filter(|confirmed| confirmed.key.pid == key.pid && confirmed.key != key)
                        .filter_map(|confirmed| {
                            let status = capture.root_status.get(confirmed.key.root.0)?;
                            let root = status.root.path.clone();
                            Some(UnusedCapture {
                                key: confirmed.key.clone(),
                                root,
                                reason: match proof {
                                    SelectedProof::Confirmed => UnusedCaptureReason::RootPrecedence,
                                    SelectedProof::Unconfirmed => {
                                        UnusedCaptureReason::SelectedUnconfirmed
                                    },
                                },
                            })
                        })
                        .collect();
                    (
                        key.root,
                        AssociationSelection::Selected { key, proof, unused },
                    )
                },
            };
            if let Some(status) = capture.root_status.get_mut(root.0) {
                status
                    .associations
                    .push(CaptureAssociation { pid, selection });
            }
        }
    }

    /// A registration can become process-readable after exec without a new lifetime.
    fn include_registered_processes(
        &mut self,
        system: &ProcessObservations<'_>,
        capture: &Capture,
    ) {
        for pid in capture.registered_pids().into_iter().map(Pid::from_u32) {
            if system
                .process(pid)
                .is_some_and(|process| command_text::names_cargo(process.cmd()))
                && !self.cargo.contains(&pid)
            {
                self.cargo.push(pid);
            }
        }
    }

    /// Record wrappers forwarding the registered command, including the shim's JSON rewrite.
    /// An exec-replaced application no longer carries that command and is a boundary.
    fn identify_capture_wrappers(&mut self, system: &ProcessObservations<'_>, capture: &Capture) {
        self.capture_mismatches = system
            .processes()
            .iter()
            .filter(|(_, process)| {
                process.details == DetailPresence::Observed && !process.cmd().is_empty()
            })
            .filter_map(|(&pid, process)| {
                let NearestRegistration::Registered(key) = self.captured_run(capture, pid) else {
                    return None;
                };
                capture
                    .confirmed()
                    .iter()
                    .find(|confirmed| confirmed.key == key)
                    .filter(|confirmed| {
                        !forwards_capture_command(
                            self.invocation_arguments(system, process),
                            confirmed.registration.record(),
                        )
                    })
                    .map(|_| pid)
            })
            .collect();
        self.capture_wrappers = system
            .processes()
            .iter()
            .filter(|(_, process)| {
                process.details == DetailPresence::Observed && !process.cmd().is_empty()
            })
            .filter_map(|(&pid, process)| {
                let NearestRegistration::Registered(key) = self.captured_run(capture, pid) else {
                    return None;
                };
                capture
                    .confirmed()
                    .iter()
                    .find(|confirmed| confirmed.key == key)
                    .filter(|confirmed| {
                        forwards_capture_command(
                            self.invocation_arguments(system, process),
                            confirmed.registration.record(),
                        )
                    })
                    .map(|_| pid)
            })
            .collect();
    }

    /// Direct ownership crosses only wrappers observed forwarding this registration.
    /// Any other intervening process establishes enclosing membership.
    pub(super) fn direct_capture(&self, capture: &Capture, pid: Pid) -> DirectAssociation {
        if self.capture_mismatches.contains(&pid) {
            return DirectAssociation::None;
        }
        let NearestRegistration::Registered(key) = self.captured_run(capture, pid) else {
            return DirectAssociation::None;
        };
        let Some(confirmed) = capture
            .confirmed()
            .iter()
            .find(|confirmed| confirmed.key == key)
        else {
            return DirectAssociation::None;
        };
        let mut walking = pid;
        for _ in 0..PARENT_WALK_LIMIT {
            if walking.as_u32() == key.pid {
                return DirectAssociation::Direct(Box::new(DirectCapture::from(confirmed)));
            }
            let Some(&parent) = self.parents.get(&walking) else {
                break;
            };
            if self.capture_mismatches.contains(&parent)
                || (parent.as_u32() != key.pid
                    && (!self.capture_wrappers.contains(&parent)
                        || self.capture_boundaries.contains(&parent)
                        || self.cargo.contains(&parent)))
            {
                break;
            }
            walking = parent;
        }
        DirectAssociation::None
    }

    /// Publication enriches an origin's identity after forwarding roles are resolved.
    fn identify_captures(&mut self, capture: &Capture) {
        self.capture_boundaries.extend(self.cargo.iter().copied());
        for &pid in &self.cargo {
            if let DirectAssociation::Direct(direct) = self.direct_capture(capture, pid) {
                if pid.as_u32() != direct.key.pid
                    && let Some(InvocationId::Process(executor)) = self.identities.get(&pid)
                {
                    self.executing.insert(pid, executor.clone());
                }
                self.identities.insert(pid, direct.invocation_id());
            }
        }
    }

    /// Exclusions prune only row ownership; parents and verified capture liveness remain intact.
    fn select_rows(
        &mut self,
        system: &ProcessObservations<'_>,
        capture: &Capture,
        excluded: &[String],
    ) {
        self.registration_eligibility = capture
            .confirmed()
            .iter()
            .map(|confirmed| {
                let argv = std::iter::once(OsString::from(CARGO_DISPLAY_NAME))
                    .chain(confirmed.registration.record().arguments().iter().cloned())
                    .collect::<Vec<_>>();
                (
                    RunId::verified(&confirmed.key, &confirmed.registration),
                    command_text::select_cargo(&argv, excluded).map(|_| ()),
                )
            })
            .collect();
        for &pid in &self.cargo {
            let process = system
                .process(pid)
                .map_or(Err(RowAbsence::ArgvUnavailable), |process| {
                    command_text::select_cargo(self.invocation_arguments(system, process), excluded)
                        .map(|_| ())
                });
            let eligibility = match self.direct_capture(capture, pid) {
                DirectAssociation::Direct(direct) => {
                    match self.registration_eligibility.get(&direct.run_id) {
                        Some(Err(
                            absence @ (RowAbsence::ProgramRejected | RowAbsence::PolicyExcluded),
                        )) => Err(*absence),
                        Some(Ok(())) if process == Err(RowAbsence::ArgvUnavailable) => Ok(()),
                        Some(Ok(()) | Err(RowAbsence::ArgvUnavailable)) | None => process,
                    }
                },
                DirectAssociation::None => process,
            };
            self.eligibility.insert(pid, eligibility);
        }
        self.rowless_cargo = self
            .cargo
            .iter()
            .copied()
            .filter(|pid| !matches!(self.eligibility.get(pid), Some(Ok(()))))
            .filter(|pid| {
                system.process(*pid).is_none_or(|process| {
                    process.cmd().is_empty()
                        || command_text::names_cargo(self.invocation_arguments(system, process))
                })
            })
            .collect();
        self.cargo
            .retain(|pid| matches!(self.eligibility.get(pid), Some(Ok(()))));
    }

    /// Only a verified direct association may supply directory annotation.
    /// Without a capture, the process owner names the row's account.
    pub(super) fn annotate_capture(
        &self,
        row: &mut CargoProcess,
        capture: &Capture,
        home: ScannerHome<'_>,
    ) {
        let pid = Pid::from_u32(row.pid);
        let owner = self
            .owners
            .get(&pid)
            .cloned()
            .unwrap_or(ProcessOwner::Unavailable);
        row.provenance = RowProvenance::Uncaptured(owner.clone());
        row.capture_membership = CaptureMembership::Outside;
        if let Some(identity) = self.identities.get(&pid) {
            row.invocation_id.clone_from(identity);
        }
        let NearestRegistration::Registered(key) = self.captured_run(capture, pid) else {
            row.state = CaptureLookup::Unregistered;
            return;
        };
        row.state = capture.read(&key);
        match self.direct_capture(capture, pid) {
            DirectAssociation::Direct(direct) => {
                row.invocation_id = direct.invocation_id();
                row.provenance = RowProvenance::direct(capture, &key, owner);
                let record = direct.registration().record();
                if let WorkingDirectoryIdentity::Absolute(path) = &row.directory_identity
                    && DirectoryComparison::between(record.directory(), path)
                        == DirectoryComparison::Same
                {
                    row.directory_identity = record.directory_identity();
                    row.path = invocation::registration_directory(record, home);
                }
            },
            DirectAssociation::None => {
                if let Some(confirmed) = capture
                    .confirmed()
                    .iter()
                    .find(|confirmed| confirmed.key == key)
                {
                    row.provenance = RowProvenance::enclosing(capture, &key, owner);
                    row.capture_membership = CaptureMembership::Enclosing(RunId::verified(
                        &key,
                        &confirmed.registration,
                    ));
                } else if pid.as_u32() != key.pid
                    && (self.capture_boundaries.contains(&Pid::from_u32(key.pid))
                        || self
                            .ancestor_pids(pid)
                            .as_slice()
                            .iter()
                            .rev()
                            .take_while(|ancestor| ancestor.as_u32() != key.pid)
                            .any(|ancestor| self.capture_boundaries.contains(ancestor)))
                {
                    row.capture_membership = CaptureMembership::EnclosingUnconfirmed;
                }
            },
        }
    }

    /// Every group the surviving cargo set forms, newest lead first.
    ///
    /// A lead ties with another when both started inside the same second
    /// -- the start time is whole seconds, since sysinfo reads the
    /// kernel's `pbi_start_tvsec` and drops the microseconds beside it.
    /// Pid breaks the tie in the same direction: macOS hands them out in
    /// order, so within one second the higher pid is the later start.
    pub(super) fn groups(
        &self,
        system: &ProcessObservations<'_>,
        attributed: &InvocationMeasurements,
        home: ScannerHome<'_>,
        capture: &Capture,
    ) -> Vec<CargoGroup> {
        let children = self.cargo_children();
        let mut candidates = self.cargo.clone();
        candidates.sort_by_key(|pid| self.ancestor_pids(*pid).as_slice().len());
        let mut rows: Vec<CargoProcess> = Vec::new();
        for pid in candidates {
            if !rows.iter().any(|row| row.pid == pid.as_u32())
                && let Ok(group) = self.group(system, attributed, home, &children, capture, pid)
            {
                rows.push(group.lead);
                rows.extend(group.rest);
            }
        }
        self.add_registration_rows(&mut rows, capture, home);
        #[cfg(test)]
        rows.extend(self.registration_rows.iter().cloned().map(|mut row| {
            row.resource_use = InvocationResourceUse::unavailable(MeasurementAbsence::Unproven);
            row
        }));
        let mut groups = self.assemble_groups(system, home, rows);
        for group in &mut groups {
            let members: Vec<_> = std::iter::once(&group.lead).chain(&group.rest).collect();
            let totals = invocation::subtree_resource_totals(&members);
            for (row, total) in std::iter::once(&mut group.lead)
                .chain(&mut group.rest)
                .zip(totals)
            {
                row.subtree_resource_use = total;
            }
        }
        groups
    }

    /// Enclosing membership never suppresses the invocation represented by a proof.
    fn add_registration_rows(
        &self,
        rows: &mut Vec<CargoProcess>,
        capture: &Capture,
        home: ScannerHome<'_>,
    ) {
        for pid in capture.registered_pids() {
            let DirectAssociation::Direct(direct) = capture.row_source(pid) else {
                continue;
            };
            if !matches!(
                self.registration_eligibility.get(&direct.run_id),
                Some(Ok(()))
            ) {
                continue;
            }
            // Root selection has already rejected unconfirmed and competing proofs.
            // Only direct representation of this invocation suppresses its source row.
            let represented = rows
                .iter()
                .any(|row| row.invocation_id == direct.invocation_id());
            if !represented && let Ok(row) = invocation::registration_row(&direct, capture, home) {
                rows.push(row);
            }
        }
    }

    /// Build one ancestry tree after both sources have supplied eligible invocations.
    pub(super) fn assemble_groups(
        &self,
        system: &ProcessObservations<'_>,
        home: ScannerHome<'_>,
        mut rows: Vec<CargoProcess>,
    ) -> Vec<CargoGroup> {
        rows.sort_by(invocation::newest_first);
        let mut seen = HashSet::new();
        rows.retain(|row| seen.insert(row.invocation_id.clone()));
        let mut visible = HashMap::new();
        for row in &rows {
            let identity = (row.invocation_id.clone(), row.pid);
            visible.insert(Pid::from_u32(row.pid), identity.clone());
            if let InvocationId::Captured(run) = &row.invocation_id {
                visible.insert(Pid::from_u32(run.shim_pid), identity);
            }
        }
        for row in &mut rows {
            row.parent = self.row_parent(row, &visible);
        }
        let parents: HashMap<_, _> = rows
            .iter()
            .filter_map(|row| match &row.parent {
                VisibleParent::Invocation { id, .. } => {
                    Some((row.invocation_id.clone(), id.clone()))
                },
                VisibleParent::Ancestor(_) | VisibleParent::None => None,
            })
            .collect();
        let mut families: HashMap<InvocationId, Vec<CargoProcess>> = HashMap::new();
        let mut assembled_descendants = HashMap::new();
        for row in rows {
            let mut root = row.invocation_id.clone();
            let mut visited = HashSet::new();
            for _ in 0..PARENT_WALK_LIMIT {
                if !visited.insert(root.clone()) {
                    break;
                }
                let Some(parent) = parents.get(&root) else {
                    break;
                };
                if !visited.contains(parent) {
                    *assembled_descendants.entry(parent.clone()).or_insert(0) += 1;
                }
                root.clone_from(parent);
            }
            families.entry(root).or_default().push(row);
        }
        let mut groups = Vec::new();
        for (id, mut rows) in families {
            for row in &mut rows {
                let managed = assembled_descendants
                    .get(&row.invocation_id)
                    .copied()
                    .unwrap_or_default();
                // Confirmed child rows count regardless of source. Without children,
                // a registration alone still cannot establish a measured zero.
                row.managed = match row.managed {
                    Measurement::Reading(dispatched) => {
                        Measurement::Reading(managed.max(dispatched))
                    },
                    Measurement::Unavailable(_) if managed > 0 => Measurement::Reading(managed),
                    unavailable @ Measurement::Unavailable(_) => unavailable,
                };
            }
            let Some(index) = rows.iter().position(|row| row.invocation_id == id) else {
                continue;
            };
            let mut lead = rows.remove(index);
            let ancestry = self.ancestry(system, home, Pid::from_u32(lead.pid));
            lead.parent = ancestry
                .as_slice()
                .last()
                .map_or(VisibleParent::None, |ancestor| {
                    VisibleParent::Ancestor(ancestor.pid)
                });
            for row in &mut rows {
                row.nested = matches!(&row.parent, VisibleParent::Invocation { id: parent, .. } if parent != &id);
            }
            rows.sort_by(invocation::newest_first);
            groups.push(CargoGroup {
                lead,
                rest: rows,
                ancestry,
            });
        }
        groups.sort_by(|left, right| invocation::newest_first(&left.lead, &right.lead));
        groups
    }

    /// Match displayed invocation identities, including a registration parent's shim alias.
    fn row_parent(
        &self,
        row: &CargoProcess,
        visible: &HashMap<Pid, (InvocationId, u32)>,
    ) -> VisibleParent {
        let mut walking = Pid::from_u32(row.pid);
        for _ in 0..PARENT_WALK_LIMIT {
            let Some(parent) = self.parents.get(&walking) else {
                break;
            };
            if let Some((id, pid)) = visible.get(parent)
                && id != &row.invocation_id
            {
                return VisibleParent::Invocation {
                    id:  id.clone(),
                    pid: *pid,
                };
            }
            walking = *parent;
        }
        VisibleParent::None
    }

    /// A capture origin needs a worker; ordinary cargo executes its own command.
    fn execution(&self, process: &ProcessObservation<'_>, origin: u32) -> InvocationExecution {
        let lifetime = if process.pid().as_u32() == origin {
            process.lifetime.as_ref().clone()
        } else {
            LifetimeEvidence::Unavailable
        };
        if let Some(executor) = self.executing.get(&process.pid()) {
            return InvocationExecution::Executing {
                origin:   lifetime,
                executor: executor.clone(),
            };
        }
        if matches!(
            CargoTransport::from_argv(process.cmd()),
            CargoTransport::Ordinary
        ) {
            InvocationExecution::Command
        } else {
            InvocationExecution::AwaitingExecutor(lifetime)
        }
    }

    /// Dispatch counts describe the physical executor before the row adopts its origin.
    fn managed_count(&self, pid: Pid, descendants: usize) -> Measurement<usize> {
        Measurement::Reading(
            descendants.max(self.dispatched_runs.get(&pid).copied().unwrap_or_default()),
        )
    }

    /// Build the group led by `root`.
    fn group(
        &self,
        system: &ProcessObservations<'_>,
        attributed: &InvocationMeasurements,
        home: ScannerHome<'_>,
        children: &HashMap<Pid, Vec<Pid>>,
        capture: &Capture,
        root: Pid,
    ) -> Result<CargoGroup, GroupAbsence> {
        if matches!(
            self.eligibility.get(&root),
            Some(Err(RowAbsence::ProgramRejected | RowAbsence::PolicyExcluded))
        ) {
            return Err(GroupAbsence::Excluded);
        }
        let managed = Self::descendants(children, root);
        let process = system.process(root).ok_or(GroupAbsence::NoProcess)?;
        let mut lead = invocation::row(
            process,
            self.invocation_arguments(system, process),
            self.identities
                .get(&root)
                .ok_or(GroupAbsence::NoIdentity)?
                .clone(),
            self.managed_count(root, managed.len()),
            home,
            InvocationResourceUse::observed(attributed, root),
            &self.direct_capture(capture, root),
        )?;
        lead.execution = self.execution(process, lead.pid);
        self.annotate_capture(&mut lead, capture, home);
        let ancestry = self.ancestry(system, home, root);
        lead.parent = self.drawn_parent(root, ancestry.as_slice());

        let mut dated: Vec<(u64, CargoProcess)> = managed
            .into_iter()
            .filter_map(|pid| {
                let process = system.process(pid)?;
                let under = Self::descendants(children, pid).len();
                let mut managed_row = invocation::row(
                    process,
                    self.invocation_arguments(system, process),
                    self.identities.get(&pid)?.clone(),
                    self.managed_count(pid, under),
                    home,
                    InvocationResourceUse::observed(attributed, pid),
                    &self.direct_capture(capture, pid),
                )
                .ok()?;
                // The same read the lead gets. An invocation the lead
                // is driving is captured in its own right where it came
                // through the shim, and where it went round the shim --
                // a cargo the enclosing run started, which the shim
                // declines to capture twice -- the enclosing capture is
                // still its own: the lock it prints about is the one
                // this row is waiting on, mirrored into the log of the
                // run it is inside.
                managed_row.execution = self.execution(process, managed_row.pid);
                self.annotate_capture(&mut managed_row, capture, home);
                managed_row.parent = self.drawn_parent(pid, ancestry.as_slice());
                managed_row.nested = !children
                    .get(&root)
                    .is_some_and(|started_by_the_lead| started_by_the_lead.contains(&pid));
                Some((process.start_time(), managed_row))
            })
            .collect();
        dated.sort_by(|left, right| right.0.cmp(&left.0).then(right.1.pid.cmp(&left.1.pid)));
        Ok(CargoGroup {
            lead,
            rest: dated.into_iter().map(|(_, process)| process).collect(),
            ancestry,
        })
    }
}

/// Recognize both argv forwarding and the shim's single-quoted POSIX command string.
/// PTY helpers and their shells carry this command; an application launched by cargo does not.
fn forwards_capture_command(argv: &[OsString], record: &RegistrationCandidate) -> bool {
    forwards_cargo_arguments(argv, record.arguments())
}

/// Compare the command after its transport boundary, preserving each original argv byte.
fn forwards_cargo_arguments(argv: &[OsString], registered: &[OsString]) -> bool {
    if argv
        .first()
        .is_some_and(|program| Path::new(program).file_name() == Some(OsStr::new("script")))
        && (0..argv.len()).any(|start| {
            matches!(CargoTransport::from_argv(&argv[start..]),
                CargoTransport::NativeCapture { arguments } if arguments == registered)
        })
    {
        return true;
    }
    if let Ok(arguments) = command_text::cargo_split(argv)
        && (argv[arguments.start..] == *registered
            || invocation::forwards_json_capture_arguments(&argv[arguments.start..], registered))
    {
        return true;
    }
    let mut expected_arguments = Vec::new();
    for argument in registered {
        expected_arguments.push(b'\'');
        for byte in argument.as_bytes() {
            if *byte == b'\'' {
                expected_arguments.extend_from_slice(b"'\\''");
            } else {
                expected_arguments.push(*byte);
            }
        }
        expected_arguments.extend_from_slice(b"' ");
    }
    argv.iter().any(|argument| {
        if native_capture_shell_forwards(argument, registered) {
            return true;
        }
        argument
            .as_bytes()
            .strip_suffix(expected_arguments.as_slice())
            .and_then(|binary| binary.strip_prefix(b"'"))
            .and_then(|binary| binary.strip_suffix(b"' "))
            .is_some_and(quoted_cargo_binary)
    })
}

/// Decode only the native entry's quoted command, including raw Unix path bytes.
fn native_capture_shell_forwards(command: &OsStr, registered: &[OsString]) -> bool {
    let mut remaining = command.as_bytes();
    if let Some(command) = remaining.strip_prefix(b"exec ") {
        remaining = command;
    }
    let mut words = Vec::new();
    while !remaining.is_empty() {
        let Some(quoted) = remaining.strip_prefix(b"'") else {
            return false;
        };
        remaining = quoted;
        let mut word = Vec::new();
        loop {
            if let Some(escaped) = remaining.strip_prefix(b"'\\''") {
                word.push(b'\'');
                remaining = escaped;
            } else if let Some(rest) = remaining.strip_prefix(b"'") {
                remaining = rest;
                break;
            } else if let Some((byte, rest)) = remaining.split_first() {
                word.push(*byte);
                remaining = rest;
            } else {
                return false;
            }
        }
        words.push(OsString::from_vec(word));
        if !remaining.is_empty() {
            let Some(rest) = remaining.strip_prefix(b" ") else {
                return false;
            };
            remaining = rest;
        }
    }
    matches!(CargoTransport::from_argv(&words),
        CargoTransport::NativeCapture { arguments } if arguments == registered)
}

/// Inside one quoted executable word, an apostrophe must use the shim's escape sequence.
fn quoted_cargo_binary(binary: &[u8]) -> bool {
    let mut remaining = binary;
    while let Some(index) = remaining.iter().position(|byte| *byte == b'\'') {
        let Some(rest) = remaining[index..].strip_prefix(b"'\\''") else {
            return false;
        };
        remaining = rest;
    }
    Path::new(OsStr::from_bytes(binary))
        .file_name()
        .is_some_and(|name| {
            CARGO_PROCESS_NAMES
                .iter()
                .any(|cargo| name == OsStr::new(cargo))
        })
}

/// Missing command or cwd fields never count as observed equality between wrappers.
fn observed_shim_match(
    outer: &[OsString],
    outer_cwd: WorkingDirectoryObservation<'_>,
    inner: &[OsString],
    inner_cwd: WorkingDirectoryObservation<'_>,
) -> bool {
    matches!((command_text::subcommand(outer), command_text::subcommand(inner), outer_cwd, inner_cwd),
        (Ok(outer), Ok(inner), WorkingDirectoryObservation::Observed(outer_cwd), WorkingDirectoryObservation::Observed(inner_cwd))
        if outer == inner && DirectoryComparison::between(outer_cwd, inner_cwd) == DirectoryComparison::Same)
}

/// Group construction preserves the reason a candidate cannot lead a tile.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GroupAbsence {
    /// The detailed process snapshot contains no such pid.
    NoProcess,
    /// The complete census supplied no invocation identity.
    NoIdentity,
    /// Required process fields remain unavailable after direct registration merging.
    Unavailable,
    /// Observed arguments or configured policy deliberately reject this command.
    Excluded,
}

impl From<RowAbsence> for GroupAbsence {
    fn from(absence: RowAbsence) -> Self {
        match absence {
            RowAbsence::ArgvUnavailable => Self::Unavailable,
            RowAbsence::ProgramRejected | RowAbsence::PolicyExcluded => Self::Excluded,
        }
    }
}

/// Successive snapshots retain invocation identity and cumulative CPU history.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct CensusSequence {
    pub(super) smoothing:                    InvocationCpuAccounting,
    previous:                                HashMap<Pid, CpuBaseline>,
    pub(in crate::census) registration_rows: Vec<CargoProcess>,
}

#[cfg(test)]
impl CensusSequence {
    /// Supplied rates exercise selection and assembly without invoking OS accounting.
    pub(crate) fn sample(&mut self, records: &ProcessObservations<'_>) -> Vec<CargoGroup> {
        self.sample_with_capture(records, &Capture::default(), &[], ScannerHome::Unavailable)
    }

    /// App scenarios feed verified captures through the production row-selection path.
    pub(crate) fn sample_capture(
        &mut self,
        records: &ProcessObservations<'_>,
        capture: &Capture,
    ) -> Vec<CargoGroup> {
        self.sample_with_capture(records, capture, &[], ScannerHome::Unavailable)
    }

    pub(super) fn sample_with_capture(
        &mut self,
        records: &ProcessObservations<'_>,
        capture: &Capture,
        excluded: &[String],
        home: ScannerHome<'_>,
    ) -> Vec<CargoGroup> {
        self.sample_attributed(
            records,
            capture,
            excluded,
            home,
            records
                .records
                .iter()
                .map(|(&pid, record)| (pid, record.cpu))
                .collect(),
        )
    }

    /// An omitted share stays absent until production row and subtree assembly handle it.
    pub(in crate::census) fn sample_cpu(
        &mut self,
        records: &ProcessObservations<'_>,
        shares: &[(u32, Measurement<f32>)],
    ) -> Vec<CargoGroup> {
        self.sample_attributed(
            records,
            &Capture::default(),
            &[],
            ScannerHome::Unavailable,
            shares
                .iter()
                .map(|&(pid, cpu)| (Pid::from_u32(pid), cpu))
                .collect(),
        )
    }

    fn sample_attributed(
        &mut self,
        records: &ProcessObservations<'_>,
        capture: &Capture,
        excluded: &[String],
        home: ScannerHome<'_>,
        cpu: HashMap<Pid, Measurement<f32>>,
    ) -> Vec<CargoGroup> {
        let mut census = Census::take(records.records.values());
        census.identities = self.smoothing.identities.observe(&census.lifetimes);
        census.registration_rows.clone_from(&self.registration_rows);
        census.prepare(records, capture, excluded);
        let memory = census.memory_sample(records);
        let attributed = InvocationMeasurements {
            cpu_scan_publication: CpuScanPublication::Unobserved,
            memory_shares: memory.shares,
            compilers: census.attribute_compilers(),
            cpu,
            memory: memory.cargo,
        };
        census.groups(records, &attributed, home, capture)
    }

    /// Each fresh snapshot goes through own-counter validation and invocation accounting.
    pub(in crate::census) fn sample_counters(
        &mut self,
        records: &mut ProcessObservations<'_>,
        now: Instant,
    ) -> Vec<CargoGroup> {
        self.sample_counters_with_capture(records, now, &Capture::default(), &[])
    }

    /// Retained CPU accounting sees the same captures and exclusions as row selection.
    pub(super) fn sample_counters_with_capture(
        &mut self,
        records: &mut ProcessObservations<'_>,
        now: Instant,
        capture: &Capture,
        excluded: &[String],
    ) -> Vec<CargoGroup> {
        for (&pid, record) in &mut records.records {
            let baseline = CpuBaseline {
                lifetime:    record.lifetime.as_ref().clone(),
                accumulated: record.accumulated,
            };
            record.cpu = Census::measure_cpu(pid, 0.0, &baseline, &self.previous);
            self.previous.insert(pid, baseline);
        }
        self.previous
            .retain(|pid, _| records.records.contains_key(pid));
        let mut census = Census::take(records.records.values());
        census.identities = self.smoothing.identities.observe(&census.lifetimes);
        census.registration_rows.clone_from(&self.registration_rows);
        census.prepare(records, capture, excluded);
        let attributed = census.attribute_with(records, &mut self.smoothing, now, |pid| {
            records.process(pid).map_or(
                Measurement::Unavailable(MeasurementAbsence::ReadFailed),
                |record| record.native_cpu.get(),
            )
        });
        census.groups(records, &attributed, ScannerHome::Unavailable, capture)
    }

    /// Capture tests can retain deliberately partial ancestry while sharing row assembly.
    pub(super) fn assemble(
        census: &mut Census,
        records: &ProcessObservations<'_>,
        capture: &Capture,
        excluded: &[String],
    ) -> Vec<CargoGroup> {
        census.prepare(records, capture, excluded);
        census.groups(
            records,
            &InvocationMeasurements {
                cpu_scan_publication: CpuScanPublication::Unobserved,
                cpu:                  HashMap::new(),
                memory:               HashMap::new(),
                memory_shares:        CompileMemoryShares::default(),
                compilers:            HashMap::new(),
            },
            ScannerHome::Known(Path::new("/writer")),
            capture,
        )
    }
}
