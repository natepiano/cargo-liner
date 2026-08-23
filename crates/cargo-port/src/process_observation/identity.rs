use std::ffi::OsString;
#[cfg(target_os = "macos")]
use std::mem::MaybeUninit;
use std::path::Path;

use processkit::process_info;
use sha2::Digest as _;

use super::snapshot::ProcessFieldObservation;
use super::snapshot::ProcessFieldUnavailable;
use super::snapshot::ReportedParent;

/// Whether platform evidence proves a parent's creation order relative to its child.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ParentCreationOrder {
    CreatedAfterChild,
    NotCreatedAfterChild,
    Unavailable(ProcessCreationOrderUnavailable),
}

/// Why process creation order cannot be proven from platform evidence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProcessCreationOrderUnavailable {
    EqualMonotonicCreationValue,
    #[cfg(any(test, not(any(target_os = "linux", target_os = "macos"))))]
    PlatformDoesNotExposeMonotonicCreationOrder,
    #[cfg(any(target_os = "macos", test))]
    PlatformQueryFailed,
    #[cfg(any(target_os = "macos", test))]
    PlatformValueInvalid,
    #[cfg(any(target_os = "macos", test))]
    IdentityChangedDuringPlatformQuery,
    #[cfg(any(target_os = "macos", test))]
    IdentityRevalidationUnavailable,
    ProcessIdentityInsufficient,
}

/// A process creation position in the current host's monotonic boot-time domain.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct MonotonicProcessCreationOrder(u64);

/// Platform evidence that may establish process creation order on one host.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum ProcessCreationOrderEvidence {
    Monotonic(MonotonicProcessCreationOrder),
    Unavailable(ProcessCreationOrderUnavailable),
}

impl ProcessCreationOrderEvidence {
    #[cfg(target_os = "linux")]
    const fn from_linux_creation_token(creation_token: &PlatformCreationToken) -> Self {
        Self::Monotonic(MonotonicProcessCreationOrder(creation_token.0))
    }

    pub(super) const fn parent_relative_to_child(&self, child: &Self) -> ParentCreationOrder {
        match (self, child) {
            (Self::Monotonic(parent), Self::Monotonic(child)) if parent.0 > child.0 => {
                ParentCreationOrder::CreatedAfterChild
            },
            (Self::Monotonic(parent), Self::Monotonic(child)) if parent.0 < child.0 => {
                ParentCreationOrder::NotCreatedAfterChild
            },
            (Self::Monotonic(_), Self::Monotonic(_)) => ParentCreationOrder::Unavailable(
                ProcessCreationOrderUnavailable::EqualMonotonicCreationValue,
            ),
            (Self::Unavailable(unavailable), _) | (_, Self::Unavailable(unavailable)) => {
                ParentCreationOrder::Unavailable(*unavailable)
            },
        }
    }

    #[cfg(test)]
    pub(super) const fn for_test(monotonic_process_creation_order: u64) -> Self {
        Self::Monotonic(MonotonicProcessCreationOrder(
            monotonic_process_creation_order,
        ))
    }

    #[cfg(test)]
    pub(super) const fn for_test_identity(process_identity: &ProcessIdentity) -> Self {
        Self::Monotonic(MonotonicProcessCreationOrder(
            process_identity.creation_token.0,
        ))
    }

    #[cfg(test)]
    pub(super) const fn unavailable_for_test() -> Self {
        Self::Unavailable(
            ProcessCreationOrderUnavailable::PlatformDoesNotExposeMonotonicCreationOrder,
        )
    }
}

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
            creation_token: PlatformCreationToken::for_test(creation_token),
        }
    }
}

/// An opaque OS token fixed at process creation.
///
/// The token is Windows `FILETIME`, Linux `/proc/<pid>/stat` start ticks, or
/// macOS `proc_pid_rusage` monotonic start time. Its `Ord` implementation exists
/// only so `ProcessIdentity` can be a deterministic collection key; only
/// `ProcessCreationOrderEvidence` establishes creation order.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct PlatformCreationToken(u64);

impl PlatformCreationToken {
    const fn from_platform(value: u64) -> Self { Self(value) }

    #[cfg(test)]
    const fn for_test(value: u64) -> Self { Self(value) }
}

/// Identity evidence produced at the host boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum ObservedProcessIdentity {
    Strong(ProcessIdentity),
    Insufficient(InsufficientProcessIdentity),
}

impl ObservedProcessIdentity {
    pub(super) const fn pid(&self) -> u32 {
        match self {
            Self::Strong(process_identity) => process_identity.pid(),
            Self::Insufficient(insufficient_identity) => insufficient_identity.pid(),
        }
    }
}

/// Current host evidence for a previously observed strong process identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum StrongProcessIdentityRevalidation {
    Current,
    Replaced(ProcessIdentity),
    Unavailable(InsufficientProcessIdentity),
}

/// A process identity observed and revalidated as the same current lifetime.
///
/// This is the boundary for operations that need to bind authority to a live
/// process. Its inner [`ProcessIdentity`] remains private so a caller cannot
/// claim verification from a PID alone.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct VerifiedProcessIdentity(ProcessIdentity);

impl VerifiedProcessIdentity {
    pub(crate) const fn into_process_identity(self) -> ProcessIdentity { self.0 }
}

/// The result of observing a PID and confirming that its strong identity is
/// still current immediately afterward.
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
    match PlatformProcessObservation::observe_lifetime(pid)
        .identity()
        .clone()
    {
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
    classify_strong_process_identity_revalidation(
        expected_identity,
        PlatformProcessObservation::observe_lifetime(expected_identity.pid())
            .identity()
            .clone(),
    )
}

/// Classify raw identity lookup evidence against a previously observed
/// process lifetime.
fn classify_strong_process_identity_revalidation(
    expected_identity: &ProcessIdentity,
    current_identity_observation: ObservedProcessIdentity,
) -> StrongProcessIdentityRevalidation {
    match current_identity_observation {
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

impl InsufficientProcessIdentity {
    const fn pid(&self) -> u32 {
        match self {
            Self::ProcessExitedBeforeIdentityLookup { pid }
            | Self::PlatformIdentityLookupFailed { pid } => *pid,
            #[cfg(any(target_os = "macos", test))]
            Self::ProcessLifetimeAnchorInvalid { pid }
            | Self::ProcessLifetimeAnchorUnavailable { pid }
            | Self::PlatformIdentityChangedDuringLookup { pid }
            | Self::PlatformMonotonicCreationQueryFailed { pid }
            | Self::PlatformMonotonicCreationValueInvalid { pid } => *pid,
            #[cfg(any(test, not(target_os = "macos")))]
            Self::PlatformCreationTokenUnavailable { pid } => *pid,
        }
    }
}

/// Child identity and reported-parent identity captured at one observation boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PlatformProcessObservation {
    pub(super) lifetime: PlatformProcessLifetimeEvidence,
    pub(super) parent:   ProcessFieldObservation<ReportedParent>,
}

/// Identity and creation-order evidence bound to the same process lifetime.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PlatformProcessLifetimeEvidence {
    identity:                ObservedProcessIdentity,
    creation_order_evidence: ProcessCreationOrderEvidence,
}

impl PlatformProcessLifetimeEvidence {
    const fn strong(
        identity: ProcessIdentity,
        creation_order_evidence: ProcessCreationOrderEvidence,
    ) -> Self {
        Self {
            identity: ObservedProcessIdentity::Strong(identity),
            creation_order_evidence,
        }
    }

    const fn insufficient(
        insufficient_process_identity: InsufficientProcessIdentity,
        process_creation_order_unavailable: ProcessCreationOrderUnavailable,
    ) -> Self {
        Self {
            identity:                ObservedProcessIdentity::Insufficient(
                insufficient_process_identity,
            ),
            creation_order_evidence: ProcessCreationOrderEvidence::Unavailable(
                process_creation_order_unavailable,
            ),
        }
    }

    pub(super) const fn identity(&self) -> &ObservedProcessIdentity { &self.identity }

    pub(super) const fn creation_order_evidence(&self) -> &ProcessCreationOrderEvidence {
        &self.creation_order_evidence
    }

    #[cfg(test)]
    pub(super) const fn for_test(
        identity: ObservedProcessIdentity,
        creation_order_evidence: ProcessCreationOrderEvidence,
    ) -> Self {
        Self {
            identity,
            creation_order_evidence,
        }
    }
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
struct MacosMonotonicProcessStart(u64);

#[cfg(any(target_os = "macos", test))]
#[derive(Clone, Debug, Eq, PartialEq)]
enum MacosMonotonicProcessStartObservation {
    Observed(MacosMonotonicProcessStart),
    QueryFailed,
    ZeroValue,
}

#[cfg(any(target_os = "macos", test))]
impl MacosMonotonicProcessStartObservation {
    const fn validate_successful_query(value: u64) -> Self {
        match value {
            0 => Self::ZeroValue,
            value => Self::Observed(MacosMonotonicProcessStart(value)),
        }
    }
}

impl PlatformProcessObservation {
    pub(super) fn observe(pid: u32) -> Self {
        match process_info(pid) {
            Ok(Some(member_info)) => {
                let initial_anchor =
                    Self::processkit_lifetime_anchor(pid, member_info.start_time());
                let lifetime = Self::bind_initial_anchor(pid, initial_anchor);
                let parent = match member_info.ppid() {
                    Some(0) => ProcessFieldObservation::Observed(ReportedParent::Root),
                    Some(parent_pid) => {
                        ProcessFieldObservation::Observed(Self::observe_reported_parent(parent_pid))
                    },
                    None => ProcessFieldObservation::Unavailable(
                        ProcessFieldUnavailable::PlatformDidNotReport,
                    ),
                };
                Self { lifetime, parent }
            },
            Ok(None) => Self {
                lifetime: PlatformProcessLifetimeEvidence::insufficient(
                    InsufficientProcessIdentity::ProcessExitedBeforeIdentityLookup { pid },
                    ProcessCreationOrderUnavailable::ProcessIdentityInsufficient,
                ),
                parent:   ProcessFieldObservation::Unavailable(
                    ProcessFieldUnavailable::ProcessExited,
                ),
            },
            Err(_) => Self {
                lifetime: PlatformProcessLifetimeEvidence::insufficient(
                    InsufficientProcessIdentity::PlatformIdentityLookupFailed { pid },
                    ProcessCreationOrderUnavailable::ProcessIdentityInsufficient,
                ),
                parent:   ProcessFieldObservation::Unavailable(
                    ProcessFieldUnavailable::PlatformLookupFailed,
                ),
            },
        }
    }

    fn observe_reported_parent(parent_pid: u32) -> ReportedParent {
        match Self::observe_lifetime(parent_pid).identity {
            ObservedProcessIdentity::Strong(parent_identity) => {
                ReportedParent::Identified(parent_identity)
            },
            ObservedProcessIdentity::Insufficient(insufficient_identity) => {
                ReportedParent::IdentityUnavailable(insufficient_identity)
            },
        }
    }

    pub(super) fn observe_lifetime(pid: u32) -> PlatformProcessLifetimeEvidence {
        match process_info(pid) {
            Ok(Some(process_info)) => {
                let initial_anchor =
                    Self::processkit_lifetime_anchor(pid, process_info.start_time());
                Self::bind_initial_anchor(pid, initial_anchor)
            },
            Ok(None) => PlatformProcessLifetimeEvidence::insufficient(
                InsufficientProcessIdentity::ProcessExitedBeforeIdentityLookup { pid },
                ProcessCreationOrderUnavailable::ProcessIdentityInsufficient,
            ),
            Err(_) => PlatformProcessLifetimeEvidence::insufficient(
                InsufficientProcessIdentity::PlatformIdentityLookupFailed { pid },
                ProcessCreationOrderUnavailable::ProcessIdentityInsufficient,
            ),
        }
    }

    #[cfg(test)]
    pub(super) const fn for_test(
        identity: ObservedProcessIdentity,
        creation_order_evidence: ProcessCreationOrderEvidence,
        parent: ProcessFieldObservation<ReportedParent>,
    ) -> Self {
        Self {
            lifetime: PlatformProcessLifetimeEvidence::for_test(identity, creation_order_evidence),
            parent,
        }
    }

    const fn processkit_lifetime_anchor(
        pid: u32,
        start_time: Option<u64>,
    ) -> ProcessLifetimeAnchorObservation {
        #[cfg(target_os = "macos")]
        {
            Self::macos_processkit_lifetime_anchor(pid, start_time)
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

    #[cfg(target_os = "linux")]
    const fn bind_initial_anchor(
        pid: u32,
        initial_anchor: ProcessLifetimeAnchorObservation,
    ) -> PlatformProcessLifetimeEvidence {
        match initial_anchor {
            ProcessLifetimeAnchorObservation::Present(ProcessLifetimeAnchor(value)) => {
                let creation_token = PlatformCreationToken::from_platform(value);
                let creation_order_evidence =
                    ProcessCreationOrderEvidence::from_linux_creation_token(&creation_token);
                PlatformProcessLifetimeEvidence::strong(
                    ProcessIdentity {
                        pid,
                        creation_token,
                    },
                    creation_order_evidence,
                )
            },
            ProcessLifetimeAnchorObservation::Insufficient(insufficient_process_identity) => {
                PlatformProcessLifetimeEvidence::insufficient(
                    insufficient_process_identity,
                    ProcessCreationOrderUnavailable::ProcessIdentityInsufficient,
                )
            },
        }
    }

    #[cfg(target_os = "macos")]
    fn bind_initial_anchor(
        pid: u32,
        initial_anchor: ProcessLifetimeAnchorObservation,
    ) -> PlatformProcessLifetimeEvidence {
        if let ProcessLifetimeAnchorObservation::Insufficient(insufficient_process_identity) =
            initial_anchor
        {
            return PlatformProcessLifetimeEvidence::insufficient(
                insufficient_process_identity,
                ProcessCreationOrderUnavailable::ProcessIdentityInsufficient,
            );
        }
        let monotonic_process_start = query_macos_monotonic_process_start(pid);
        let revalidated_anchor = observe_process_lifetime_anchor(pid);
        Self::bind_macos_process_lifetime(
            pid,
            initial_anchor,
            &monotonic_process_start,
            revalidated_anchor,
        )
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    fn bind_initial_anchor(
        pid: u32,
        initial_anchor: ProcessLifetimeAnchorObservation,
    ) -> PlatformProcessLifetimeEvidence {
        match initial_anchor {
            ProcessLifetimeAnchorObservation::Present(ProcessLifetimeAnchor(value)) => {
                PlatformProcessLifetimeEvidence::strong(
                    ProcessIdentity {
                        pid,
                        creation_token: PlatformCreationToken::from_platform(value),
                    },
                    ProcessCreationOrderEvidence::Unavailable(
                        ProcessCreationOrderUnavailable::PlatformDoesNotExposeMonotonicCreationOrder,
                    ),
                )
            },
            ProcessLifetimeAnchorObservation::Insufficient(insufficient_process_identity) => {
                PlatformProcessLifetimeEvidence::insufficient(
                    insufficient_process_identity,
                    ProcessCreationOrderUnavailable::ProcessIdentityInsufficient,
                )
            },
        }
    }

    #[cfg(any(target_os = "macos", test))]
    fn bind_macos_process_lifetime(
        pid: u32,
        initial_anchor: ProcessLifetimeAnchorObservation,
        monotonic_process_start: &MacosMonotonicProcessStartObservation,
        revalidated_anchor: ProcessLifetimeAnchorObservation,
    ) -> PlatformProcessLifetimeEvidence {
        match (initial_anchor, revalidated_anchor) {
            (
                ProcessLifetimeAnchorObservation::Present(before),
                ProcessLifetimeAnchorObservation::Present(after),
            ) if before == after => match monotonic_process_start {
                MacosMonotonicProcessStartObservation::Observed(MacosMonotonicProcessStart(
                    value,
                )) => PlatformProcessLifetimeEvidence::strong(
                    ProcessIdentity {
                        pid,
                        creation_token: PlatformCreationToken::from_platform(*value),
                    },
                    ProcessCreationOrderEvidence::Monotonic(MonotonicProcessCreationOrder(*value)),
                ),
                MacosMonotonicProcessStartObservation::QueryFailed => {
                    PlatformProcessLifetimeEvidence::insufficient(
                        InsufficientProcessIdentity::PlatformMonotonicCreationQueryFailed { pid },
                        ProcessCreationOrderUnavailable::PlatformQueryFailed,
                    )
                },
                MacosMonotonicProcessStartObservation::ZeroValue => {
                    PlatformProcessLifetimeEvidence::insufficient(
                        InsufficientProcessIdentity::PlatformMonotonicCreationValueInvalid { pid },
                        ProcessCreationOrderUnavailable::PlatformValueInvalid,
                    )
                },
            },
            (
                ProcessLifetimeAnchorObservation::Present(_),
                ProcessLifetimeAnchorObservation::Present(_),
            ) => PlatformProcessLifetimeEvidence::insufficient(
                InsufficientProcessIdentity::PlatformIdentityChangedDuringLookup { pid },
                ProcessCreationOrderUnavailable::IdentityChangedDuringPlatformQuery,
            ),
            (
                ProcessLifetimeAnchorObservation::Present(_),
                ProcessLifetimeAnchorObservation::Insufficient(insufficient_process_identity),
            ) => PlatformProcessLifetimeEvidence::insufficient(
                insufficient_process_identity,
                ProcessCreationOrderUnavailable::IdentityRevalidationUnavailable,
            ),
            (ProcessLifetimeAnchorObservation::Insufficient(insufficient_process_identity), _) => {
                PlatformProcessLifetimeEvidence::insufficient(
                    insufficient_process_identity,
                    ProcessCreationOrderUnavailable::ProcessIdentityInsufficient,
                )
            },
        }
    }
}

#[cfg(target_os = "macos")]
fn observe_process_lifetime_anchor(pid: u32) -> ProcessLifetimeAnchorObservation {
    match process_info(pid) {
        Ok(Some(process_info)) => {
            PlatformProcessObservation::processkit_lifetime_anchor(pid, process_info.start_time())
        },
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
    // SAFETY: `rusage_info` is a writable `rusage_info_v0` buffer and the V0
    // flavor writes that supported structure without retaining the pointer.
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
    // SAFETY: `proc_pid_rusage` returned success after initializing the V0 buffer.
    let process_rusage = unsafe { rusage_info.assume_init() };
    MacosMonotonicProcessStartObservation::validate_successful_query(
        process_rusage.ri_proc_start_abstime,
    )
}

/// One process lifetime plus the executable and argument identity active in it.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(super) struct ProcessIncarnation {
    identity:                    ProcessIdentity,
    executable_argv_fingerprint: ProcessFingerprint,
}

impl ProcessIncarnation {
    pub(super) const fn new(
        identity: ProcessIdentity,
        executable_argv_fingerprint: ProcessFingerprint,
    ) -> Self {
        Self {
            identity,
            executable_argv_fingerprint,
        }
    }

    pub(super) const fn identity(&self) -> &ProcessIdentity { &self.identity }

    #[cfg(test)]
    pub(super) const fn executable_argv_fingerprint(&self) -> &ProcessFingerprint {
        &self.executable_argv_fingerprint
    }
}

/// A stable digest of observed executable and argument evidence.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(super) struct ProcessFingerprint([u8; 32]);

impl ProcessFingerprint {
    pub(super) fn from_observed_fields(executable: &Path, argv: &[OsString]) -> Self {
        let mut digest = sha2::Sha256::new();
        Self::write_hash_input(executable, argv, |bytes| digest.update(bytes));
        Self(digest.finalize().into())
    }

    fn write_hash_input(executable: &Path, argv: &[OsString], mut write: impl FnMut(&[u8])) {
        let executable_bytes = executable.as_os_str().as_encoded_bytes();
        let executable_length = executable_bytes.len().to_le_bytes();
        write(&executable_length);
        write(executable_bytes);

        let argument_count = argv.len().to_le_bytes();
        write(&argument_count);
        for argument in argv {
            let argument_bytes = argument.as_encoded_bytes();
            let argument_length = argument_bytes.len().to_le_bytes();
            write(&argument_length);
            write(argument_bytes);
        }
    }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::os::unix::ffi::OsStringExt as _;

    use super::InsufficientProcessIdentity;
    use super::MacosMonotonicProcessStartObservation;
    use super::ObservedProcessIdentity;
    use super::ParentCreationOrder;
    use super::PlatformProcessObservation;
    use super::ProcessCreationOrderEvidence;
    use super::ProcessCreationOrderUnavailable;
    use super::ProcessFingerprint;
    use super::ProcessIdentity;
    use super::ProcessLifetimeAnchor;
    use super::ProcessLifetimeAnchorObservation;

    fn lifetime_anchor(value: u64) -> ProcessLifetimeAnchorObservation {
        ProcessLifetimeAnchorObservation::Present(ProcessLifetimeAnchor(value))
    }

    #[test]
    fn reused_pid_has_a_different_strong_identity() {
        let prior = ProcessIdentity::for_test(42, 100);
        let replacement = ProcessIdentity::for_test(42, 101);

        assert_ne!(prior, replacement);
    }

    #[test]
    fn insufficient_identity_never_becomes_strong_identity() {
        let observed = ObservedProcessIdentity::Insufficient(
            InsufficientProcessIdentity::PlatformCreationTokenUnavailable { pid: 42 },
        );

        assert!(matches!(
            observed,
            ObservedProcessIdentity::Insufficient(..)
        ));
    }

    #[test]
    fn macos_native_start_binds_identity_and_order_to_the_same_value() {
        let lifetime = PlatformProcessObservation::bind_macos_process_lifetime(
            42,
            lifetime_anchor(100),
            &MacosMonotonicProcessStartObservation::validate_successful_query(700),
            lifetime_anchor(100),
        );

        assert_eq!(
            lifetime.identity(),
            &ObservedProcessIdentity::Strong(ProcessIdentity::for_test(42, 700))
        );
        assert_eq!(
            lifetime.creation_order_evidence(),
            &ProcessCreationOrderEvidence::for_test(700)
        );
    }

    #[test]
    fn macos_missing_initial_processkit_anchor_returns_insufficient_lifetime_evidence() {
        let lifetime = PlatformProcessObservation::bind_macos_process_lifetime(
            42,
            PlatformProcessObservation::macos_processkit_lifetime_anchor(42, None),
            &MacosMonotonicProcessStartObservation::validate_successful_query(700),
            lifetime_anchor(100),
        );

        assert_eq!(
            lifetime.identity(),
            &ObservedProcessIdentity::Insufficient(
                InsufficientProcessIdentity::ProcessLifetimeAnchorUnavailable { pid: 42 }
            )
        );
        assert_eq!(
            lifetime.creation_order_evidence(),
            &ProcessCreationOrderEvidence::Unavailable(
                ProcessCreationOrderUnavailable::ProcessIdentityInsufficient
            )
        );
    }

    #[test]
    fn macos_missing_revalidated_processkit_anchor_returns_insufficient_lifetime_evidence() {
        let lifetime = PlatformProcessObservation::bind_macos_process_lifetime(
            42,
            lifetime_anchor(100),
            &MacosMonotonicProcessStartObservation::validate_successful_query(700),
            PlatformProcessObservation::macos_processkit_lifetime_anchor(42, None),
        );

        assert_eq!(
            lifetime.identity(),
            &ObservedProcessIdentity::Insufficient(
                InsufficientProcessIdentity::ProcessLifetimeAnchorUnavailable { pid: 42 }
            )
        );
        assert_eq!(
            lifetime.creation_order_evidence(),
            &ProcessCreationOrderEvidence::Unavailable(
                ProcessCreationOrderUnavailable::IdentityRevalidationUnavailable
            )
        );
    }

    #[test]
    fn monotonic_creation_evidence_distinguishes_all_order_comparisons() {
        let child = ProcessCreationOrderEvidence::for_test(100);
        let earlier_parent = ProcessCreationOrderEvidence::for_test(99);
        let equal_parent = ProcessCreationOrderEvidence::for_test(100);
        let later_parent = ProcessCreationOrderEvidence::for_test(101);

        assert_eq!(
            earlier_parent.parent_relative_to_child(&child),
            ParentCreationOrder::NotCreatedAfterChild
        );
        assert_eq!(
            equal_parent.parent_relative_to_child(&child),
            ParentCreationOrder::Unavailable(
                ProcessCreationOrderUnavailable::EqualMonotonicCreationValue
            )
        );
        assert_eq!(
            later_parent.parent_relative_to_child(&child),
            ParentCreationOrder::CreatedAfterChild
        );
    }

    #[test]
    fn unavailable_creation_evidence_never_orders_processes() {
        let child = ProcessCreationOrderEvidence::for_test(100);
        let parent = ProcessCreationOrderEvidence::unavailable_for_test();

        assert_eq!(
            parent.parent_relative_to_child(&child),
            ParentCreationOrder::Unavailable(
                ProcessCreationOrderUnavailable::PlatformDoesNotExposeMonotonicCreationOrder
            )
        );
    }

    #[test]
    fn macos_identity_change_rejects_native_lifetime_evidence() {
        let lifetime = PlatformProcessObservation::bind_macos_process_lifetime(
            42,
            lifetime_anchor(100),
            &MacosMonotonicProcessStartObservation::validate_successful_query(700),
            lifetime_anchor(101),
        );

        assert_eq!(
            lifetime.identity(),
            &ObservedProcessIdentity::Insufficient(
                InsufficientProcessIdentity::PlatformIdentityChangedDuringLookup { pid: 42 }
            )
        );
        assert_eq!(
            lifetime.creation_order_evidence(),
            &ProcessCreationOrderEvidence::Unavailable(
                ProcessCreationOrderUnavailable::IdentityChangedDuringPlatformQuery
            )
        );
    }

    #[test]
    fn macos_exit_during_revalidation_rejects_native_lifetime_evidence() {
        let lifetime = PlatformProcessObservation::bind_macos_process_lifetime(
            42,
            lifetime_anchor(100),
            &MacosMonotonicProcessStartObservation::validate_successful_query(700),
            ProcessLifetimeAnchorObservation::Insufficient(
                InsufficientProcessIdentity::ProcessExitedBeforeIdentityLookup { pid: 42 },
            ),
        );

        assert_eq!(
            lifetime.identity(),
            &ObservedProcessIdentity::Insufficient(
                InsufficientProcessIdentity::ProcessExitedBeforeIdentityLookup { pid: 42 }
            )
        );
        assert_eq!(
            lifetime.creation_order_evidence(),
            &ProcessCreationOrderEvidence::Unavailable(
                ProcessCreationOrderUnavailable::IdentityRevalidationUnavailable
            )
        );
    }

    #[test]
    fn macos_native_query_failure_returns_insufficient_lifetime_evidence() {
        let lifetime = PlatformProcessObservation::bind_macos_process_lifetime(
            42,
            lifetime_anchor(100),
            &MacosMonotonicProcessStartObservation::QueryFailed,
            lifetime_anchor(100),
        );

        assert_eq!(
            lifetime.identity(),
            &ObservedProcessIdentity::Insufficient(
                InsufficientProcessIdentity::PlatformMonotonicCreationQueryFailed { pid: 42 }
            )
        );
        assert_eq!(
            lifetime.creation_order_evidence(),
            &ProcessCreationOrderEvidence::Unavailable(
                ProcessCreationOrderUnavailable::PlatformQueryFailed
            )
        );
    }

    #[test]
    fn macos_zero_native_start_value_returns_insufficient_lifetime_evidence() {
        let lifetime = PlatformProcessObservation::bind_macos_process_lifetime(
            42,
            lifetime_anchor(100),
            &MacosMonotonicProcessStartObservation::validate_successful_query(0),
            lifetime_anchor(100),
        );

        assert_eq!(
            lifetime.identity(),
            &ObservedProcessIdentity::Insufficient(
                InsufficientProcessIdentity::PlatformMonotonicCreationValueInvalid { pid: 42 }
            )
        );
        assert_eq!(
            lifetime.creation_order_evidence(),
            &ProcessCreationOrderEvidence::Unavailable(
                ProcessCreationOrderUnavailable::PlatformValueInvalid
            )
        );
    }

    #[test]
    fn macos_zero_initial_processkit_anchor_returns_insufficient_lifetime_evidence() {
        let lifetime = PlatformProcessObservation::bind_macos_process_lifetime(
            42,
            PlatformProcessObservation::macos_processkit_lifetime_anchor(42, Some(0)),
            &MacosMonotonicProcessStartObservation::validate_successful_query(700),
            lifetime_anchor(100),
        );

        assert_eq!(
            lifetime.identity(),
            &ObservedProcessIdentity::Insufficient(
                InsufficientProcessIdentity::ProcessLifetimeAnchorInvalid { pid: 42 }
            )
        );
        assert_eq!(
            lifetime.creation_order_evidence(),
            &ProcessCreationOrderEvidence::Unavailable(
                ProcessCreationOrderUnavailable::ProcessIdentityInsufficient
            )
        );
    }

    #[test]
    fn macos_zero_revalidated_processkit_anchor_returns_insufficient_lifetime_evidence() {
        let lifetime = PlatformProcessObservation::bind_macos_process_lifetime(
            42,
            lifetime_anchor(100),
            &MacosMonotonicProcessStartObservation::validate_successful_query(700),
            PlatformProcessObservation::macos_processkit_lifetime_anchor(42, Some(0)),
        );

        assert_eq!(
            lifetime.identity(),
            &ObservedProcessIdentity::Insufficient(
                InsufficientProcessIdentity::ProcessLifetimeAnchorInvalid { pid: 42 }
            )
        );
        assert_eq!(
            lifetime.creation_order_evidence(),
            &ProcessCreationOrderEvidence::Unavailable(
                ProcessCreationOrderUnavailable::IdentityRevalidationUnavailable
            )
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_current_process_has_monotonic_creation_order_evidence() {
        let observation = PlatformProcessObservation::observe(std::process::id());

        assert!(matches!(
            observation.lifetime.identity(),
            ObservedProcessIdentity::Strong(_)
        ));
        assert!(matches!(
            observation.lifetime.creation_order_evidence(),
            ProcessCreationOrderEvidence::Monotonic(_)
        ));
    }

    #[cfg(unix)]
    fn fingerprint_input(executable: &[u8], argv: &[&[u8]]) -> Vec<u8> {
        let executable =
            std::path::PathBuf::from(std::ffi::OsString::from_vec(executable.to_vec()));
        let argv: Vec<_> = argv
            .iter()
            .map(|argument| std::ffi::OsString::from_vec(argument.to_vec()))
            .collect();
        let mut input = Vec::new();
        ProcessFingerprint::write_hash_input(&executable, &argv, |bytes| {
            input.extend_from_slice(bytes);
        });
        input
    }

    #[cfg(unix)]
    #[test]
    fn executable_and_argv_separator_bytes_have_distinct_fingerprint_inputs() {
        let first_input = fingerprint_input(b"a", &[b"\xffb"]);
        let second_input = fingerprint_input(b"a\xff", &[b"b"]);

        assert_ne!(first_input, second_input);

        let first_fingerprint = ProcessFingerprint::from_observed_fields(
            &std::path::PathBuf::from(std::ffi::OsString::from_vec(b"a".to_vec())),
            &[std::ffi::OsString::from_vec(b"\xffb".to_vec())],
        );
        let second_fingerprint = ProcessFingerprint::from_observed_fields(
            &std::path::PathBuf::from(std::ffi::OsString::from_vec(b"a\xff".to_vec())),
            &[std::ffi::OsString::from_vec(b"b".to_vec())],
        );
        assert_ne!(first_fingerprint, second_fingerprint);
    }

    #[cfg(unix)]
    #[test]
    fn empty_and_non_utf8_argv_boundaries_have_distinct_fingerprint_inputs() {
        assert_ne!(
            fingerprint_input(b"a", &[b"\xff", b"b"]),
            fingerprint_input(b"a\xff", &[b"", b"b"])
        );
    }
}
