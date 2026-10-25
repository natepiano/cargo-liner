//! Invocation identity and continuity across scans.

use std::collections::HashMap;
use std::collections::HashSet;
use std::ffi::OsString;
use std::path::Path;

use rustix::io::Errno;
use sysinfo::Pid;
use uuid::Uuid;

use super::command_text;
use crate::birth_stamp::BirthStamp;
use crate::birth_stamp::LifetimeEvidence;
use crate::birth_stamp::ProcessLifetime;
use crate::birth_stamp::Verification;
use crate::constants::CARGO_SUBCOMMAND_PREFIX;
use crate::progress::capture::CaptureKey;
use crate::progress::capture::CaptureRootIndex;
use crate::registration::VerifiedRegistration;
use crate::root_scan::RootIncarnation;

/// An observed external tool either carries this cargo dispatch or starts another command.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DispatchedProcess {
    /// Matching forwarded arguments identify the process cargo launched.
    Matched {
        /// The process cargo launched, including an interpreted external tool.
        pid:             Pid,
        /// The executable or script begins the canonical cargo argument slice.
        arguments_start: usize,
    },
    /// The observations do not establish this dispatch.
    NotObserved,
}

/// Match the dispatched executable and complete arguments independently of cwd availability.
pub(super) fn dispatched_process(
    shim_arguments: &[OsString],
    process_arguments: &[OsString],
    pid: Pid,
) -> DispatchedProcess {
    let Ok(arguments) = command_text::cargo_split(shim_arguments) else {
        return DispatchedProcess::NotObserved;
    };
    if !arguments.follows_cargo_executable() {
        return DispatchedProcess::NotObserved;
    }
    let forwarded = &shim_arguments[arguments.start..];
    process_arguments
        .iter()
        .enumerate()
        .find_map(|(arguments_start, program)| {
            let name = Path::new(program).file_name()?.to_str()?;
            if !name.starts_with(CARGO_SUBCOMMAND_PREFIX)
                || !command_text::is_cargo_name(name.as_ref())
            {
                return None;
            }
            let subcommand = name.strip_prefix(CARGO_SUBCOMMAND_PREFIX)?;
            let dispatched = &process_arguments[arguments_start + 1..];
            (forwarded
                .first()
                .is_some_and(|argument| argument == subcommand)
                && (forwarded == dispatched
                    || (dispatched
                        .first()
                        .is_none_or(|argument| argument != subcommand)
                        && &forwarded[1..] == dispatched)))
                .then_some(DispatchedProcess::Matched {
                    pid,
                    arguments_start,
                })
        })
        .unwrap_or(DispatchedProcess::NotObserved)
}

/// Identity of an invocation, independent of its displayed process or row source.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) enum InvocationId {
    /// The verified registration directly represents this invocation.
    Captured(RunId),
    /// Uncaptured and nested invocations retain their own process lifetime.
    Process(ProcessIdentity),
}

/// Root incarnation and publication generation qualify one captured invocation.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct RunId {
    /// Startup root position alone cannot detect a replaced directory.
    pub(crate) root:        CaptureRootIndex,
    /// Descriptor identity changes when the directory itself is replaced.
    pub(crate) incarnation: RootIncarnation,
    /// The registration describes the shim, even when cargo supplies the row.
    pub(crate) shim_pid:    u32,
    /// Publication generations separate even identical pid and birth observations.
    pub(crate) generation:  String,
    /// Kernel comparison qualifies the generation without reducing its precision.
    pub(crate) birth:       BirthStamp,
}

/// Only matching registration and native birth evidence authorizes native adoption.
pub(super) struct LifetimeMatch {
    native: InvocationId,
}

/// Unknown evidence cannot authorize a join, just as contradictory evidence cannot.
pub(super) enum LifetimeContradiction {
    Different,
    Unavailable,
}

impl LifetimeMatch {
    pub(super) fn prove(
        run: &RunId,
        evidence: &LifetimeEvidence,
    ) -> Result<Self, LifetimeContradiction> {
        match evidence {
            LifetimeEvidence::Available(lifetime) => {
                match lifetime.compare_registration(&run.birth) {
                    Verification::Confirmed => Ok(Self {
                        native: InvocationId::Process(ProcessIdentity::Known {
                            pid:      run.shim_pid,
                            lifetime: lifetime.clone(),
                        }),
                    }),
                    Verification::Ended => Err(LifetimeContradiction::Different),
                    Verification::Unknown => Err(LifetimeContradiction::Unavailable),
                }
            },
            LifetimeEvidence::Unavailable => Err(LifetimeContradiction::Unavailable),
        }
    }

    pub(super) fn into_native(self) -> InvocationId { self.native }
}

/// Process lifetime evidence never substitutes registration comparison seconds.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) enum ProcessIdentity {
    /// Native kernel precision separates process replacements without a generation.
    Known {
        /// A birth stamp belongs to the process whose kernel entry was read.
        pid:      u32,
        /// Linux start ticks or the full Darwin start timeval, qualified by boot.
        lifetime: ProcessLifetime,
    },
    /// Continuous presence permits row retention without proving a kernel lifetime.
    Unavailable {
        /// Retain the displayed process even when its lifetime cannot be read.
        pid:         u32,
        /// Retired only when an end or a contradicting native birth is proven.
        observation: Uuid,
    },
}

/// Every retired lifetime stays terminal through later cycles on the same PID.
/// Each retirement supplies the next stable replacement observation.
#[derive(Default)]
pub(super) struct TerminalIdentities(HashMap<Pid, TerminalIdentity>);

struct TerminalIdentity {
    ended:       HashSet<InvocationId>,
    replacement: InvocationId,
}

impl TerminalIdentities {
    pub(super) fn end(&mut self, pid: Pid, identity: &InvocationId) {
        let terminal = self.0.entry(pid).or_insert_with(|| TerminalIdentity {
            ended:       HashSet::new(),
            replacement: identity.clone(),
        });
        if terminal.ended.insert(identity.clone()) {
            terminal.replacement = InvocationId::Process(ProcessIdentity::observed(
                pid.as_u32(),
                LifetimeEvidence::Unavailable,
            ));
        }
    }

    pub(super) fn subsequent(&self, pid: Pid, identity: &InvocationId) -> InvocationId {
        self.0
            .get(&pid)
            .filter(|terminal| terminal.ended.contains(identity))
            .map_or_else(|| identity.clone(), |terminal| terminal.replacement.clone())
    }
}

/// Missing observations retain their identity until an end or replacement is proven.
#[derive(Default)]
pub(crate) struct ProcessIdentities {
    /// Both present and missing processes own their original identity.
    present: HashMap<Pid, ObservedProcess>,
}

/// A missing kernel read cannot contradict the last known start time.
#[derive(Clone)]
enum ProcessLifetimeObservation {
    Known(ProcessLifetime),
    Unreadable,
}

struct ObservedProcess {
    identity: ProcessIdentity,
    lifetime: ProcessLifetimeObservation,
}

/// Only a successful kernel absence probe can retire a missing identity.
pub(crate) struct ProvenProcessEnd {
    #[cfg(test)]
    pid: Pid,
}

impl ProvenProcessEnd {
    pub(super) fn probe(pid: Pid) -> Result<Self, Errno> {
        let native = rustix::process::Pid::from_raw(
            i32::try_from(pid.as_u32()).map_err(|_| rustix::io::Errno::INVAL)?,
        )
        .ok_or(rustix::io::Errno::INVAL)?;
        match rustix::process::test_kill_process(native) {
            Err(rustix::io::Errno::SRCH) => Ok(Self {
                #[cfg(test)]
                pid,
            }),
            Err(error) => Err(error),
            Ok(()) => Err(rustix::io::Errno::AGAIN),
        }
    }

    #[cfg(test)]
    pub(crate) const fn for_test(pid: Pid) -> Self { Self { pid } }
}

impl ProcessLifetimeObservation {
    fn from_evidence(evidence: &LifetimeEvidence) -> Self {
        match evidence {
            LifetimeEvidence::Available(start) => Self::Known(start.clone()),
            LifetimeEvidence::Unavailable => Self::Unreadable,
        }
    }

    fn contradicts(&self, other: &Self) -> bool {
        matches!((self, other), (Self::Known(a), Self::Known(b)) if a != b)
    }
}

impl ProcessIdentities {
    /// Kernel evidence controls known lifetimes; presence alone retains unavailable rows.
    pub(crate) fn observe(
        &mut self,
        lifetimes: &HashMap<Pid, LifetimeEvidence>,
    ) -> HashMap<Pid, InvocationId> {
        let mut observed = HashMap::new();
        for (&pid, lifetime) in lifetimes {
            let reading = ProcessLifetimeObservation::from_evidence(lifetime);
            let previous = self
                .present
                .get(&pid)
                .filter(|old| !old.lifetime.contradicts(&reading));
            let identity = previous.map_or_else(
                || ProcessIdentity::observed(pid.as_u32(), lifetime.clone()),
                |old| old.identity.clone(),
            );
            let lifetime = match reading {
                ProcessLifetimeObservation::Unreadable => {
                    previous.map_or_else(|| reading.clone(), |old| old.lifetime.clone())
                },
                known @ ProcessLifetimeObservation::Known(_) => known,
            };
            observed.insert(pid, InvocationId::Process(identity.clone()));
            self.present
                .insert(pid, ObservedProcess { identity, lifetime });
        }
        observed
    }

    /// The caller must carry the end proof for the exact PID being retired.
    #[cfg(test)]
    pub(crate) fn finish(&mut self, ended: &ProvenProcessEnd) { self.present.remove(&ended.pid); }
}

impl InvocationId {
    /// Stable synthetic lifetime for fixtures that are not kernel observations.
    #[cfg(test)]
    pub(crate) fn for_test(pid: u32) -> Self {
        Self::Process(ProcessIdentity::Known {
            pid,
            lifetime: crate::birth_stamp::ProcessLifetime::for_test(u64::from(pid)),
        })
    }
}

/// Capture membership supplies progress without granting registration row fields.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CaptureMembership {
    /// This invocation runs inside the named capture and keeps its own identity.
    Enclosing(RunId),
    /// An ancestor's selected capture supplies progress without verified metadata.
    EnclosingUnconfirmed,
    /// No enclosing capture supplies this invocation's progress.
    Outside,
}

/// A visible parent is either an invocation, a chain entry, or absent from the view.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum VisibleParent {
    /// Family matching uses invocation identity rather than the displayed pid.
    Invocation {
        /// Family continuity follows the invocation across changes of row source.
        id:  InvocationId,
        /// Display the cargo parent pid even when the identity names its shim.
        pid: u32,
    },
    /// A non-cargo ancestor is drawn in the command's ancestry chain.
    Ancestor(u32),
    /// No ancestor is drawn for this invocation.
    None,
}

impl RunId {
    /// Bind the verified generation to the actual directory scanned this time.
    pub(super) fn verified(key: &CaptureKey, registration: &VerifiedRegistration) -> Self {
        Self {
            root:        key.root,
            incarnation: key.incarnation,
            shim_pid:    registration.pid(),
            generation:  registration.record().generation().to_owned(),
            birth:       registration.birth().clone(),
        }
    }
}

impl ProcessIdentity {
    /// Unavailable evidence receives only a row token, never a claimed lifetime.
    pub(super) fn observed(pid: u32, evidence: LifetimeEvidence) -> Self {
        match evidence {
            LifetimeEvidence::Available(lifetime) => Self::Known { pid, lifetime },
            LifetimeEvidence::Unavailable => Self::Unavailable {
                pid,
                observation: uuid::Uuid::now_v7(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    use sysinfo::Pid;

    use super::DispatchedProcess;
    use super::dispatched_process;

    #[test]
    fn an_external_dispatch_matches_its_forwarded_arguments() {
        let shim = [
            OsString::from("cargo"),
            "p20driver".into(),
            "--jobs".into(),
            "2".into(),
        ];
        let driver = [
            OsString::from("/tools/cargo-p20driver"),
            "p20driver".into(),
            "--jobs".into(),
            "2".into(),
        ];
        assert_eq!(
            dispatched_process(&shim, &driver, Pid::from_u32(200)),
            DispatchedProcess::Matched {
                pid:             Pid::from_u32(200),
                arguments_start: 0,
            }
        );
    }

    #[test]
    fn an_interpreted_dispatch_retains_the_external_script_arguments() {
        let shim = [OsString::from("cargo"), "p20driver".into()];
        let driver = [
            OsString::from("python3"),
            "/tools/cargo-p20driver".into(),
            "p20driver".into(),
        ];
        assert_eq!(
            dispatched_process(&shim, &driver, Pid::from_u32(200)),
            DispatchedProcess::Matched {
                pid:             Pid::from_u32(200),
                arguments_start: 1,
            }
        );
    }

    #[test]
    fn an_installed_shell_shim_identifies_its_dispatched_script() {
        let shim = [
            OsString::from("/bin/sh"),
            "/home/u/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin/cargo".into(),
            "p20bdrv".into(),
        ];
        let driver = [
            OsString::from("/bin/sh"),
            "/tools/drvbin/cargo-p20bdrv".into(),
            "p20bdrv".into(),
        ];
        assert_eq!(
            dispatched_process(&shim, &driver, Pid::from_u32(2_929_162)),
            DispatchedProcess::Matched {
                pid:             Pid::from_u32(2_929_162),
                arguments_start: 1,
            }
        );
        let external = [OsString::from("cargo-nextest"), "p20bdrv".into()];
        assert_eq!(
            dispatched_process(&external, &driver, Pid::from_u32(2_929_162)),
            DispatchedProcess::NotObserved
        );
        let unrelated = [
            OsString::from("/bin/sh"),
            "/tools/build".into(),
            "p20bdrv".into(),
        ];
        assert_eq!(
            dispatched_process(&unrelated, &driver, Pid::from_u32(2_929_162)),
            DispatchedProcess::NotObserved
        );
    }

    #[test]
    fn interpreter_flags_do_not_hide_the_dispatched_script() {
        let shim = [OsString::from("cargo"), "p20driver".into()];
        let driver = [
            OsString::from("python3"),
            "-u".into(),
            "/tools/cargo-p20driver".into(),
            "p20driver".into(),
        ];
        assert_eq!(
            dispatched_process(&shim, &driver, Pid::from_u32(200)),
            DispatchedProcess::Matched {
                pid:             Pid::from_u32(200),
                arguments_start: 2,
            }
        );
    }

    #[test]
    fn dispatch_matching_keeps_non_utf8_argument_bytes_distinct() {
        let shim = [
            OsString::from("cargo"),
            "p20driver".into(),
            OsString::from_vec(vec![0xff]),
        ];
        let driver = [
            OsString::from("cargo-p20driver"),
            "p20driver".into(),
            OsString::from_vec(vec![0xfe]),
        ];
        assert_eq!(
            dispatched_process(&shim, &driver, Pid::from_u32(200)),
            DispatchedProcess::NotObserved
        );
    }

    #[test]
    fn a_nested_command_with_different_arguments_is_not_a_dispatch() {
        let shim = [
            OsString::from("cargo"),
            "p20driver".into(),
            "--jobs".into(),
            "2".into(),
        ];
        let changed = [
            OsString::from("cargo-p20driver"),
            "p20driver".into(),
            "--jobs".into(),
            "4".into(),
        ];
        let other = [
            OsString::from("cargo-nextest"),
            "nextest".into(),
            "run".into(),
        ];
        let nested = [
            OsString::from("cargo"),
            "p20driver".into(),
            "--jobs".into(),
            "2".into(),
        ];
        for arguments in [changed.as_slice(), other.as_slice(), nested.as_slice(), &[]] {
            assert_eq!(
                dispatched_process(&shim, arguments, Pid::from_u32(200)),
                DispatchedProcess::NotObserved
            );
        }
    }
}
