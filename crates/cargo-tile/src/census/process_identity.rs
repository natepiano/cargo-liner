//! Invocation identity and continuity across scans.

use std::collections::HashMap;

use sysinfo::Pid;
use uuid::Uuid;

use crate::birth_stamp::BirthStamp;
use crate::birth_stamp::LifetimeEvidence;
use crate::birth_stamp::ProcessLifetime;
use crate::progress::capture::CaptureKey;
use crate::progress::capture::CaptureRootIndex;
use crate::registration::VerifiedRegistration;
use crate::root_scan::RootIncarnation;

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
