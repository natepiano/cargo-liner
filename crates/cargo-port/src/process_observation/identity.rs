#[cfg(target_os = "macos")]
use std::mem::MaybeUninit;

use processkit::process_info;

/// A process identity whose creation token distinguishes a recycled PID.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct ProcessIdentity {
    pid:            u32,
    creation_token: PlatformCreationToken,
}

impl ProcessIdentity {
    pub(crate) const fn pid(&self) -> u32 { self.pid }

    #[cfg(test)]
    pub(crate) const fn for_test(pid: u32, creation_token: u64) -> Self {
        Self {
            pid,
            creation_token: PlatformCreationToken(creation_token),
        }
    }
}

/// An opaque OS token fixed at process creation.
///
/// The token is Windows `FILETIME`, Linux `/proc/<pid>/stat` start ticks, or
/// macOS `proc_pid_rusage` monotonic start time. Its ordering only makes
/// `ProcessIdentity` a deterministic collection key.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct PlatformCreationToken(u64);

/// Identity evidence produced at the host boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
enum ObservedProcessIdentity {
    Strong(ProcessIdentity),
    Insufficient(InsufficientProcessIdentity),
}

/// Current host evidence for a previously observed strong process identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum StrongProcessIdentityRevalidation {
    Current,
    Replaced(ProcessIdentity),
    Unavailable(InsufficientProcessIdentity),
}

/// A process identity observed and revalidated as the same current lifetime.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct VerifiedProcessIdentity(ProcessIdentity);

impl VerifiedProcessIdentity {
    pub(crate) const fn into_process_identity(self) -> ProcessIdentity { self.0 }
}

/// The result of observing a PID and confirming its identity is still current.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CurrentProcessIdentityObservation {
    Verified(VerifiedProcessIdentity),
    InitialIdentityUnavailable(InsufficientProcessIdentity),
    ReplacedDuringRevalidation(ProcessIdentity),
    RevalidationUnavailable(InsufficientProcessIdentity),
}

/// Observe a strong identity and immediately revalidate it before granting
/// identity-bound authority.
pub(crate) fn observe_current_process_identity(pid: u32) -> CurrentProcessIdentityObservation {
    match observe_process_identity(pid) {
        ObservedProcessIdentity::Strong(process_identity) => {
            match revalidate_strong_process_identity(&process_identity) {
                StrongProcessIdentityRevalidation::Current => {
                    CurrentProcessIdentityObservation::Verified(VerifiedProcessIdentity(
                        process_identity,
                    ))
                },
                StrongProcessIdentityRevalidation::Replaced(replacement_identity) => {
                    CurrentProcessIdentityObservation::ReplacedDuringRevalidation(
                        replacement_identity,
                    )
                },
                StrongProcessIdentityRevalidation::Unavailable(insufficient_process_identity) => {
                    CurrentProcessIdentityObservation::RevalidationUnavailable(
                        insufficient_process_identity,
                    )
                },
            }
        },
        ObservedProcessIdentity::Insufficient(insufficient_process_identity) => {
            CurrentProcessIdentityObservation::InitialIdentityUnavailable(
                insufficient_process_identity,
            )
        },
    }
}

/// Re-observe a strong identity immediately before an identity-sensitive action.
pub(crate) fn revalidate_strong_process_identity(
    expected_identity: &ProcessIdentity,
) -> StrongProcessIdentityRevalidation {
    match observe_process_identity(expected_identity.pid()) {
        ObservedProcessIdentity::Strong(current_identity)
            if current_identity == *expected_identity =>
        {
            StrongProcessIdentityRevalidation::Current
        },
        ObservedProcessIdentity::Strong(replacement_identity) => {
            StrongProcessIdentityRevalidation::Replaced(replacement_identity)
        },
        ObservedProcessIdentity::Insufficient(insufficient_process_identity) => {
            StrongProcessIdentityRevalidation::Unavailable(insufficient_process_identity)
        },
    }
}

/// Why an observed PID cannot identify one process lifetime.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum InsufficientProcessIdentity {
    ProcessExitedBeforeIdentityLookup {
        pid: u32,
    },
    #[cfg(any(target_os = "macos", test))]
    ProcessLifetimeAnchorInvalid {
        pid: u32,
    },
    #[cfg(any(target_os = "macos", test))]
    ProcessLifetimeAnchorUnavailable {
        pid: u32,
    },
    #[cfg(any(test, not(target_os = "macos")))]
    PlatformCreationTokenUnavailable {
        pid: u32,
    },
    #[cfg(any(target_os = "macos", test))]
    PlatformIdentityChangedDuringLookup {
        pid: u32,
    },
    PlatformIdentityLookupFailed {
        pid: u32,
    },
    #[cfg(any(target_os = "macos", test))]
    PlatformMonotonicCreationQueryFailed {
        pid: u32,
    },
    #[cfg(any(target_os = "macos", test))]
    PlatformMonotonicCreationValueInvalid {
        pid: u32,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ProcessLifetimeAnchor(u64);

#[derive(Clone, Debug, Eq, PartialEq)]
enum ProcessLifetimeAnchorObservation {
    Present(ProcessLifetimeAnchor),
    Insufficient(InsufficientProcessIdentity),
}

#[cfg(any(target_os = "macos", test))]
#[derive(Clone, Debug, Eq, PartialEq)]
enum MacosMonotonicProcessStartObservation {
    Observed(u64),
    QueryFailed,
    ZeroValue,
}

#[cfg(any(target_os = "macos", test))]
impl MacosMonotonicProcessStartObservation {
    const fn validate_successful_query(value: u64) -> Self {
        match value {
            0 => Self::ZeroValue,
            value => Self::Observed(value),
        }
    }
}

fn observe_process_identity(pid: u32) -> ObservedProcessIdentity {
    match process_info(pid) {
        Ok(Some(info)) => {
            bind_initial_anchor(pid, processkit_lifetime_anchor(pid, info.start_time()))
        },
        Ok(None) => ObservedProcessIdentity::Insufficient(
            InsufficientProcessIdentity::ProcessExitedBeforeIdentityLookup { pid },
        ),
        Err(_) => ObservedProcessIdentity::Insufficient(
            InsufficientProcessIdentity::PlatformIdentityLookupFailed { pid },
        ),
    }
}

const fn processkit_lifetime_anchor(
    pid: u32,
    start_time: Option<u64>,
) -> ProcessLifetimeAnchorObservation {
    #[cfg(target_os = "macos")]
    {
        macos_processkit_lifetime_anchor(pid, start_time)
    }
    #[cfg(not(target_os = "macos"))]
    {
        match start_time {
            Some(start_time) => {
                ProcessLifetimeAnchorObservation::Present(ProcessLifetimeAnchor(start_time))
            },
            None => ProcessLifetimeAnchorObservation::Insufficient(
                InsufficientProcessIdentity::PlatformCreationTokenUnavailable { pid },
            ),
        }
    }
}

#[cfg(any(target_os = "macos", test))]
const fn macos_processkit_lifetime_anchor(
    pid: u32,
    start_time: Option<u64>,
) -> ProcessLifetimeAnchorObservation {
    match start_time {
        Some(0) => ProcessLifetimeAnchorObservation::Insufficient(
            InsufficientProcessIdentity::ProcessLifetimeAnchorInvalid { pid },
        ),
        Some(start_time) => {
            ProcessLifetimeAnchorObservation::Present(ProcessLifetimeAnchor(start_time))
        },
        None => ProcessLifetimeAnchorObservation::Insufficient(
            InsufficientProcessIdentity::ProcessLifetimeAnchorUnavailable { pid },
        ),
    }
}

#[cfg(not(target_os = "macos"))]
const fn bind_initial_anchor(
    pid: u32,
    initial_anchor: ProcessLifetimeAnchorObservation,
) -> ObservedProcessIdentity {
    match initial_anchor {
        ProcessLifetimeAnchorObservation::Present(ProcessLifetimeAnchor(value)) => {
            ObservedProcessIdentity::Strong(ProcessIdentity {
                pid,
                creation_token: PlatformCreationToken(value),
            })
        },
        ProcessLifetimeAnchorObservation::Insufficient(reason) => {
            ObservedProcessIdentity::Insufficient(reason)
        },
    }
}

#[cfg(target_os = "macos")]
fn bind_initial_anchor(
    pid: u32,
    initial_anchor: ProcessLifetimeAnchorObservation,
) -> ObservedProcessIdentity {
    if let ProcessLifetimeAnchorObservation::Insufficient(reason) = initial_anchor {
        return ObservedProcessIdentity::Insufficient(reason);
    }
    let monotonic_process_start = query_macos_monotonic_process_start(pid);
    let revalidated_anchor = observe_process_lifetime_anchor(pid);
    bind_macos_process_lifetime(
        pid,
        initial_anchor,
        &monotonic_process_start,
        revalidated_anchor,
    )
}

#[cfg(any(target_os = "macos", test))]
fn bind_macos_process_lifetime(
    pid: u32,
    initial_anchor: ProcessLifetimeAnchorObservation,
    monotonic_process_start: &MacosMonotonicProcessStartObservation,
    revalidated_anchor: ProcessLifetimeAnchorObservation,
) -> ObservedProcessIdentity {
    match (initial_anchor, revalidated_anchor) {
        (
            ProcessLifetimeAnchorObservation::Present(before),
            ProcessLifetimeAnchorObservation::Present(after),
        ) if before == after => match monotonic_process_start {
            MacosMonotonicProcessStartObservation::Observed(value) => {
                ObservedProcessIdentity::Strong(ProcessIdentity {
                    pid,
                    creation_token: PlatformCreationToken(*value),
                })
            },
            MacosMonotonicProcessStartObservation::QueryFailed => {
                ObservedProcessIdentity::Insufficient(
                    InsufficientProcessIdentity::PlatformMonotonicCreationQueryFailed { pid },
                )
            },
            MacosMonotonicProcessStartObservation::ZeroValue => {
                ObservedProcessIdentity::Insufficient(
                    InsufficientProcessIdentity::PlatformMonotonicCreationValueInvalid { pid },
                )
            },
        },
        (
            ProcessLifetimeAnchorObservation::Present(_),
            ProcessLifetimeAnchorObservation::Present(_),
        ) => ObservedProcessIdentity::Insufficient(
            InsufficientProcessIdentity::PlatformIdentityChangedDuringLookup { pid },
        ),
        (
            ProcessLifetimeAnchorObservation::Present(_),
            ProcessLifetimeAnchorObservation::Insufficient(reason),
        )
        | (ProcessLifetimeAnchorObservation::Insufficient(reason), _) => {
            ObservedProcessIdentity::Insufficient(reason)
        },
    }
}

#[cfg(target_os = "macos")]
fn observe_process_lifetime_anchor(pid: u32) -> ProcessLifetimeAnchorObservation {
    match process_info(pid) {
        Ok(Some(info)) => processkit_lifetime_anchor(pid, info.start_time()),
        Ok(None) => ProcessLifetimeAnchorObservation::Insufficient(
            InsufficientProcessIdentity::ProcessExitedBeforeIdentityLookup { pid },
        ),
        Err(_) => ProcessLifetimeAnchorObservation::Insufficient(
            InsufficientProcessIdentity::PlatformIdentityLookupFailed { pid },
        ),
    }
}

#[cfg(target_os = "macos")]
#[expect(
    unsafe_code,
    reason = "proc_pid_rusage FFI reads the macOS monotonic process start time"
)]
fn query_macos_monotonic_process_start(pid: u32) -> MacosMonotonicProcessStartObservation {
    let Ok(native_pid) = libc::pid_t::try_from(pid) else {
        return MacosMonotonicProcessStartObservation::QueryFailed;
    };
    let mut rusage_info = MaybeUninit::<libc::rusage_info_v0>::uninit();
    // SAFETY: `rusage_info` is a writable V0 buffer that the query does not retain.
    let query_result = unsafe {
        libc::proc_pid_rusage(
            native_pid,
            libc::RUSAGE_INFO_V0,
            rusage_info.as_mut_ptr().cast::<libc::rusage_info_t>(),
        )
    };
    if query_result != 0 {
        return MacosMonotonicProcessStartObservation::QueryFailed;
    }
    // SAFETY: a successful query initialized the V0 buffer.
    let process_rusage = unsafe { rusage_info.assume_init() };
    MacosMonotonicProcessStartObservation::validate_successful_query(
        process_rusage.ri_proc_start_abstime,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reused_pid_has_a_different_strong_identity() {
        assert_ne!(
            ProcessIdentity::for_test(42, 100),
            ProcessIdentity::for_test(42, 101)
        );
    }

    #[test]
    fn macos_native_start_binds_identity_to_same_lifetime() {
        let observed = bind_macos_process_lifetime(
            42,
            ProcessLifetimeAnchorObservation::Present(ProcessLifetimeAnchor(100)),
            &MacosMonotonicProcessStartObservation::validate_successful_query(700),
            ProcessLifetimeAnchorObservation::Present(ProcessLifetimeAnchor(100)),
        );
        assert_eq!(
            observed,
            ObservedProcessIdentity::Strong(ProcessIdentity::for_test(42, 700))
        );
    }

    #[test]
    fn macos_missing_anchor_rejects_lifetime() {
        for (start_time, reason) in [
            (
                None,
                InsufficientProcessIdentity::ProcessLifetimeAnchorUnavailable { pid: 42 },
            ),
            (
                Some(0),
                InsufficientProcessIdentity::ProcessLifetimeAnchorInvalid { pid: 42 },
            ),
        ] {
            let initial_anchor = macos_processkit_lifetime_anchor(42, start_time);
            assert_eq!(
                initial_anchor,
                ProcessLifetimeAnchorObservation::Insufficient(reason.clone())
            );
            let observed = bind_macos_process_lifetime(
                42,
                initial_anchor,
                &MacosMonotonicProcessStartObservation::validate_successful_query(700),
                ProcessLifetimeAnchorObservation::Present(ProcessLifetimeAnchor(100)),
            );
            assert_eq!(observed, ObservedProcessIdentity::Insufficient(reason));
        }
    }

    #[test]
    fn macos_query_failure_rejects_lifetime() {
        let observed = bind_macos_process_lifetime(
            42,
            ProcessLifetimeAnchorObservation::Present(ProcessLifetimeAnchor(100)),
            &MacosMonotonicProcessStartObservation::QueryFailed,
            ProcessLifetimeAnchorObservation::Present(ProcessLifetimeAnchor(100)),
        );
        assert_eq!(
            observed,
            ObservedProcessIdentity::Insufficient(
                InsufficientProcessIdentity::PlatformMonotonicCreationQueryFailed { pid: 42 }
            )
        );
    }

    #[test]
    fn macos_identity_change_rejects_lifetime() {
        let observed = bind_macos_process_lifetime(
            42,
            ProcessLifetimeAnchorObservation::Present(ProcessLifetimeAnchor(100)),
            &MacosMonotonicProcessStartObservation::validate_successful_query(700),
            ProcessLifetimeAnchorObservation::Present(ProcessLifetimeAnchor(101)),
        );
        assert_eq!(
            observed,
            ObservedProcessIdentity::Insufficient(
                InsufficientProcessIdentity::PlatformIdentityChangedDuringLookup { pid: 42 }
            )
        );
    }
}
