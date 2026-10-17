//! Invocation identity and continuity across scans.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::Path;

use sysinfo::Pid;
use uuid::Uuid;

use super::command_text;
use crate::birth_stamp::BirthStamp;
use crate::birth_stamp::LifetimeEvidence;
use crate::birth_stamp::ProcessLifetime;
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
        /// Retired once this pid disappears from a scan, even if the pid returns.
        observation: Uuid,
    },
}

/// Retain unavailable row identities only while their pids remain continuously observed.
#[derive(Default)]
pub(crate) struct ProcessIdentities {
    /// This map is replaced by each scan, so absent pids cannot retain row continuity.
    present: HashMap<Pid, ProcessIdentity>,
}

impl ProcessIdentities {
    /// Kernel evidence controls known lifetimes; presence alone retains unavailable rows.
    pub(crate) fn observe(
        &mut self,
        lifetimes: &HashMap<Pid, LifetimeEvidence>,
    ) -> HashMap<Pid, InvocationId> {
        self.present = lifetimes
            .iter()
            .map(|(&pid, lifetime)| {
                let identity = match (lifetime, self.present.get(&pid)) {
                    (
                        LifetimeEvidence::Unavailable,
                        Some(identity @ ProcessIdentity::Unavailable { .. }),
                    ) => identity.clone(),
                    _ => ProcessIdentity::observed(pid.as_u32(), lifetime.clone()),
                };
                (pid, identity)
            })
            .collect();
        self.present
            .iter()
            .map(|(&pid, identity)| (pid, InvocationId::Process(identity.clone())))
            .collect()
    }
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
