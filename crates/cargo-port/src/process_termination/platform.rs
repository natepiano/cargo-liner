//! Host adapters that bind a termination signal to an observed process object.
//!
//! Linux uses `pidfd_open` and `pidfd_send_signal`, so signal delivery names
//! the process object held by the descriptor instead of resolving a PID at the
//! delivery call. The adapter revalidates lifetime and exec-image evidence
//! after binding and immediately before signaling. If `pidfd` is unavailable
//! at runtime, the capability remains observed-only; it never falls back to
//! `kill(pid, ...)`.
//!
//! macOS remains observed-only. `kill(2)` is PID-addressed, `task_for_pid`
//! requires privileges or an entitlement Cargo Port does not hold, and
//! `processkit::ProcessGroup` can bind only children it spawned or adopted.

use std::fmt;
#[cfg(target_os = "linux")]
use std::os::fd::AsRawFd as _;
#[cfg(target_os = "linux")]
use std::os::fd::FromRawFd as _;
#[cfg(target_os = "linux")]
use std::os::fd::OwnedFd;

#[cfg(test)]
use super::constants::TERMINATION_CONFIRMATION_POLL_INTERVAL;
use crate::process_observation::ProcessCapabilityMintAuthority;
use crate::process_observation::identity::InsufficientProcessIdentity;
use crate::process_observation::identity::ProcessImageContinuity;
use crate::process_observation::identity::ProcessIncarnation;
use crate::process_observation::identity::StrongProcessIdentityRevalidation;
use crate::process_observation::identity::observe_process_image_continuity;
use crate::process_observation::identity::revalidate_strong_process_identity;

/// Authority to terminate one process that Cargo Port did not start.
///
/// The fields and adapter variants are private, the value is move-only, and
/// its `Debug` implementation reveals no PID or host handle. Construction also
/// requires [`ProcessCapabilityMintAuthority`], whose private field confines
/// minting to `ProcessObserver`.
pub(crate) struct ExternalProcessTerminationCapability {
    authorized_incarnation: ProcessIncarnation,
    adapter:                ExternalTerminationAdapter,
}

impl ExternalProcessTerminationCapability {
    /// Bind the safest adapter available for immutable observer evidence.
    #[cfg(target_os = "linux")]
    pub(crate) fn from_observation(
        _: ProcessCapabilityMintAuthority,
        authorized_incarnation: ProcessIncarnation,
    ) -> Self {
        let adapter = LinuxPidFd::bind(&authorized_incarnation).map_or(
            ExternalTerminationAdapter::ObservedOnly,
            ExternalTerminationAdapter::LinuxPidFd,
        );
        Self {
            authorized_incarnation,
            adapter,
        }
    }

    /// Preserve immutable evidence as observed-only on hosts without a safe
    /// identity-bound adapter.
    #[cfg(not(target_os = "linux"))]
    pub(crate) const fn from_observation(
        _: ProcessCapabilityMintAuthority,
        authorized_incarnation: ProcessIncarnation,
    ) -> Self {
        Self {
            authorized_incarnation,
            adapter: ExternalTerminationAdapter::ObservedOnly,
        }
    }

    pub(super) const fn pid(&self) -> u32 { self.authorized_incarnation.identity().pid() }

    pub(super) fn observe_admission(&self) -> TerminationSignalAdmission {
        observe_termination_admission(&self.authorized_incarnation)
    }

    pub(super) const fn has_identity_bound_adapter(&self) -> bool {
        match &self.adapter {
            #[cfg(target_os = "linux")]
            ExternalTerminationAdapter::LinuxPidFd(_) => true,
            #[cfg(test)]
            ExternalTerminationAdapter::Fixture(_) => true,
            ExternalTerminationAdapter::ObservedOnly => false,
        }
    }

    #[cfg(target_os = "linux")]
    pub(super) fn deliver_termination_request(&self) -> BoundSignalDelivery {
        match &self.adapter {
            ExternalTerminationAdapter::LinuxPidFd(linux_pid_fd) => {
                linux_pid_fd.deliver_termination_request()
            },
            #[cfg(test)]
            ExternalTerminationAdapter::Fixture(bound_process_object) => {
                bound_process_object.deliver_termination_request()
            },
            ExternalTerminationAdapter::ObservedOnly => BoundSignalDelivery::Rejected,
        }
    }

    #[cfg(not(target_os = "linux"))]
    pub(super) const fn deliver_termination_request(&self) -> BoundSignalDelivery {
        match &self.adapter {
            #[cfg(test)]
            ExternalTerminationAdapter::Fixture(bound_process_object) => {
                bound_process_object.deliver_termination_request()
            },
            ExternalTerminationAdapter::ObservedOnly => BoundSignalDelivery::Rejected,
        }
    }

    #[cfg(any(target_os = "linux", test))]
    pub(super) fn confirm_process_object_gone(
        &self,
        confirmation_timeout: std::time::Duration,
    ) -> BoundProcessObjectPresence {
        match &self.adapter {
            #[cfg(target_os = "linux")]
            ExternalTerminationAdapter::LinuxPidFd(linux_pid_fd) => {
                linux_pid_fd.confirm_process_object_gone(confirmation_timeout)
            },
            #[cfg(test)]
            ExternalTerminationAdapter::Fixture(bound_process_object) => {
                bound_process_object.confirm_process_object_gone(confirmation_timeout)
            },
            ExternalTerminationAdapter::ObservedOnly => BoundProcessObjectPresence::Unavailable,
        }
    }

    #[cfg(all(not(target_os = "linux"), not(test)))]
    pub(super) const fn confirm_process_object_gone(
        &self,
        _: std::time::Duration,
    ) -> BoundProcessObjectPresence {
        match self.adapter {
            ExternalTerminationAdapter::ObservedOnly => BoundProcessObjectPresence::Unavailable,
        }
    }

    #[cfg(test)]
    pub(super) fn for_test(
        authorized_incarnation: ProcessIncarnation,
        delivery: BoundSignalDelivery,
        presence: &[BoundProcessObjectPresence],
    ) -> Self {
        Self {
            authorized_incarnation,
            adapter: ExternalTerminationAdapter::Fixture(BoundProcessObject {
                delivery,
                presence: std::sync::Mutex::new(presence.iter().copied().collect()),
            }),
        }
    }
}

impl fmt::Debug for ExternalProcessTerminationCapability {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExternalProcessTerminationCapability")
            .finish_non_exhaustive()
    }
}

enum ExternalTerminationAdapter {
    #[cfg(target_os = "linux")]
    LinuxPidFd(LinuxPidFd),
    ObservedOnly,
    #[cfg(test)]
    Fixture(BoundProcessObject),
}

/// Whether the host accepted the one graceful termination signal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum BoundSignalDelivery {
    #[cfg(any(target_os = "linux", test))]
    Accepted,
    #[cfg(any(target_os = "linux", test))]
    ProcessGone,
    Rejected,
}

/// What the identity-bound handle established after signal delivery.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum BoundProcessObjectPresence {
    #[cfg(any(target_os = "linux", test))]
    Present,
    #[cfg(any(target_os = "linux", test))]
    Gone,
    Unavailable,
}

#[cfg(target_os = "linux")]
struct LinuxPidFd {
    file_descriptor: OwnedFd,
}

#[cfg(target_os = "linux")]
impl LinuxPidFd {
    fn bind(authorized_incarnation: &ProcessIncarnation) -> Option<Self> {
        let file_descriptor = open_pid_fd(authorized_incarnation.identity().pid())?;
        let linux_pid_fd = Self { file_descriptor };
        matches!(
            observe_termination_admission(authorized_incarnation),
            TerminationSignalAdmission::SameProcessObject
        )
        .then_some(linux_pid_fd)
    }

    fn deliver_termination_request(&self) -> BoundSignalDelivery {
        send_pid_fd_termination_signal(&self.file_descriptor)
    }

    fn confirm_process_object_gone(
        &self,
        confirmation_timeout: std::time::Duration,
    ) -> BoundProcessObjectPresence {
        poll_pid_fd_presence(&self.file_descriptor, confirmation_timeout)
    }
}

#[cfg(target_os = "linux")]
#[expect(
    unsafe_code,
    reason = "pidfd_open FFI creates the identity-bound Linux process descriptor"
)]
fn open_pid_fd(pid: u32) -> Option<OwnedFd> {
    let native_pid = libc::pid_t::try_from(pid).ok()?;
    // SAFETY: `pidfd_open` receives one valid PID value and the required zero
    // flags. A nonnegative return value is a new owned file descriptor.
    let raw_file_descriptor = unsafe { libc::syscall(libc::SYS_pidfd_open, native_pid, 0_u32) };
    let raw_file_descriptor = i32::try_from(raw_file_descriptor).ok()?;
    if raw_file_descriptor < 0 {
        return None;
    }
    // SAFETY: the successful `pidfd_open` call returned a new descriptor whose
    // ownership transfers to this `OwnedFd` exactly once.
    Some(unsafe { OwnedFd::from_raw_fd(raw_file_descriptor) })
}

#[cfg(target_os = "linux")]
#[expect(
    unsafe_code,
    reason = "pidfd_send_signal FFI delivers SIGTERM through the bound Linux descriptor"
)]
fn send_pid_fd_termination_signal(file_descriptor: &OwnedFd) -> BoundSignalDelivery {
    // SAFETY: the descriptor remains owned for the call, the signal is
    // `SIGTERM`, the optional siginfo pointer is null, and flags must be zero.
    let result = unsafe {
        libc::syscall(
            libc::SYS_pidfd_send_signal,
            file_descriptor.as_raw_fd(),
            libc::SIGTERM,
            std::ptr::null::<libc::siginfo_t>(),
            0_u32,
        )
    };
    if result == 0 {
        return BoundSignalDelivery::Accepted;
    }
    if std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
        BoundSignalDelivery::ProcessGone
    } else {
        BoundSignalDelivery::Rejected
    }
}

#[cfg(target_os = "linux")]
#[expect(
    unsafe_code,
    reason = "poll FFI waits for the bound Linux pidfd to report process exit"
)]
fn poll_pid_fd_presence(
    file_descriptor: &OwnedFd,
    confirmation_timeout: std::time::Duration,
) -> BoundProcessObjectPresence {
    let timeout_milliseconds = i32::try_from(confirmation_timeout.as_millis()).unwrap_or(i32::MAX);
    let mut poll_file_descriptor = libc::pollfd {
        fd:      file_descriptor.as_raw_fd(),
        events:  libc::POLLIN,
        revents: 0,
    };
    // SAFETY: `poll_file_descriptor` is a live one-element buffer for the
    // duration of the call and its descriptor remains owned by `file_descriptor`.
    let result = unsafe { libc::poll(&mut poll_file_descriptor, 1, timeout_milliseconds) };
    if result == 0 {
        return BoundProcessObjectPresence::Present;
    }
    if result < 0 {
        return BoundProcessObjectPresence::Unavailable;
    }
    if poll_file_descriptor.revents & (libc::POLLIN | libc::POLLHUP) != 0 {
        BoundProcessObjectPresence::Gone
    } else {
        BoundProcessObjectPresence::Unavailable
    }
}

/// A test-owned process-object adapter used to exercise delivery outcomes on
/// hosts that remain observed-only.
#[cfg(test)]
struct BoundProcessObject {
    delivery: BoundSignalDelivery,
    presence: std::sync::Mutex<std::collections::VecDeque<BoundProcessObjectPresence>>,
}

#[cfg(test)]
impl BoundProcessObject {
    const fn deliver_termination_request(&self) -> BoundSignalDelivery { self.delivery }

    fn confirm_process_object_gone(
        &self,
        confirmation_timeout: std::time::Duration,
    ) -> BoundProcessObjectPresence {
        let mut waited = std::time::Duration::ZERO;
        loop {
            let presence = self.presence.lock().map_or(
                BoundProcessObjectPresence::Unavailable,
                |mut scripted| {
                    scripted
                        .pop_front()
                        .unwrap_or(BoundProcessObjectPresence::Gone)
                },
            );
            match presence {
                BoundProcessObjectPresence::Gone | BoundProcessObjectPresence::Unavailable => {
                    return presence;
                },
                BoundProcessObjectPresence::Present => {},
            }
            if waited >= confirmation_timeout {
                return BoundProcessObjectPresence::Present;
            }
            std::thread::sleep(TERMINATION_CONFIRMATION_POLL_INTERVAL);
            waited += TERMINATION_CONFIRMATION_POLL_INTERVAL;
        }
    }
}

/// Whether current host evidence admits signal delivery.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TerminationSignalAdmission {
    SameProcessObject,
    PidReused,
    ProcessGone,
    ProcessImageReplaced,
    RevalidationUnavailable,
}

/// Decide whether a signal may be delivered from lifetime and image evidence.
pub(super) const fn admit_termination_signal(
    identity_revalidation: &StrongProcessIdentityRevalidation,
    process_image_continuity: ProcessImageContinuity,
) -> TerminationSignalAdmission {
    match (identity_revalidation, process_image_continuity) {
        (StrongProcessIdentityRevalidation::Replaced(_), _) => {
            TerminationSignalAdmission::PidReused
        },
        (
            StrongProcessIdentityRevalidation::Unavailable(
                InsufficientProcessIdentity::ProcessExitedBeforeIdentityLookup { .. },
            ),
            _,
        )
        | (_, ProcessImageContinuity::ProcessGone) => TerminationSignalAdmission::ProcessGone,
        (StrongProcessIdentityRevalidation::Unavailable(_), _)
        | (StrongProcessIdentityRevalidation::Current, ProcessImageContinuity::Unobservable) => {
            TerminationSignalAdmission::RevalidationUnavailable
        },
        (StrongProcessIdentityRevalidation::Current, ProcessImageContinuity::SameImage) => {
            TerminationSignalAdmission::SameProcessObject
        },
        (StrongProcessIdentityRevalidation::Current, ProcessImageContinuity::ReplacedImage) => {
            TerminationSignalAdmission::ProcessImageReplaced
        },
    }
}

fn observe_termination_admission(
    authorized_incarnation: &ProcessIncarnation,
) -> TerminationSignalAdmission {
    let identity_revalidation =
        revalidate_strong_process_identity(authorized_incarnation.identity());
    admit_termination_signal(
        &identity_revalidation,
        observe_process_image_continuity(authorized_incarnation),
    )
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    #[cfg(unix)]
    use std::io::BufRead as _;
    #[cfg(unix)]
    use std::io::Write as _;
    #[cfg(unix)]
    use std::process::Command;
    #[cfg(unix)]
    use std::process::Stdio;

    use super::*;
    use crate::process_observation::PlatformTerminationCapabilityObservation;
    #[cfg(unix)]
    use crate::process_observation::identity::CurrentProcessIdentityObservation;
    use crate::process_observation::identity::ObservedProcessIdentity;
    use crate::process_observation::identity::ProcessIdentity;
    use crate::process_observation::identity::classify_strong_process_identity_revalidation;
    #[cfg(unix)]
    use crate::process_observation::identity::observe_current_process_identity;
    use crate::process_observation::observe_platform_termination_capability_for_test;

    const ADMISSION_POLL_ATTEMPTS: usize = 100;

    #[cfg(unix)]
    struct ExecProcessFixture {
        child: std::process::Child,
    }

    #[cfg(unix)]
    impl ExecProcessFixture {
        fn spawn() -> Self {
            let mut child = Command::new("/bin/sh")
                .arg("-c")
                .arg("printf 'ready\\n'; IFS= read -r ready; exec /bin/sleep 5")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .expect("the exec fixture should spawn");
            let mut ready = String::new();
            std::io::BufReader::new(
                child
                    .stdout
                    .take()
                    .expect("the exec fixture should have stdout"),
            )
            .read_line(&mut ready)
            .expect("the exec fixture should report readiness");
            assert_eq!(ready, "ready\n");
            Self { child }
        }

        fn capability(&self) -> ExternalProcessTerminationCapability {
            match observe_platform_termination_capability_for_test(self.child.id()) {
                PlatformTerminationCapabilityObservation::Available(capability) => capability,
                PlatformTerminationCapabilityObservation::InsufficientIncarnationEvidence => {
                    panic!("the live exec fixture should produce strong immutable evidence")
                },
            }
        }

        fn exec_sleep(&mut self) {
            self.child
                .stdin
                .take()
                .expect("the exec fixture should have stdin")
                .write_all(b"go\n")
                .expect("the exec fixture should accept its exec command");
        }
    }

    #[cfg(unix)]
    impl Drop for ExecProcessFixture {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    #[cfg(unix)]
    struct ReplacementProcessTableFixture {
        expected:          ProcessIdentity,
        replacement:       ProcessIdentity,
        replacement_child: std::process::Child,
    }

    #[cfg(unix)]
    impl ReplacementProcessTableFixture {
        fn spawn() -> Self {
            let mut original_child = Command::new("/bin/sleep")
                .arg("5")
                .spawn()
                .expect("the original PID fixture should spawn");
            let replacement_child = Command::new("/bin/sleep")
                .arg("5")
                .spawn()
                .expect("the replacement PID fixture should spawn");
            let expected = Self::strong_identity(&original_child);
            let replacement =
                Self::strong_identity(&replacement_child).with_pid_for_test(expected.pid());
            original_child
                .kill()
                .expect("the original PID fixture should stop");
            original_child
                .wait()
                .expect("the original PID fixture should be reaped");
            Self {
                expected,
                replacement,
                replacement_child,
            }
        }

        fn strong_identity(child: &std::process::Child) -> ProcessIdentity {
            match observe_current_process_identity(child.id()) {
                CurrentProcessIdentityObservation::Verified(verified_process_identity) => {
                    verified_process_identity.into_process_identity()
                },
                observation => {
                    panic!("the PID fixture should have a verified identity: {observation:?}")
                },
            }
        }

        fn admission(&self) -> TerminationSignalAdmission {
            let identity_revalidation = classify_strong_process_identity_revalidation(
                &self.expected,
                ObservedProcessIdentity::Strong(self.replacement.clone()),
            );
            admit_termination_signal(&identity_revalidation, ProcessImageContinuity::SameImage)
        }
    }

    #[cfg(unix)]
    impl Drop for ReplacementProcessTableFixture {
        fn drop(&mut self) {
            let _ = self.replacement_child.kill();
            let _ = self.replacement_child.wait();
        }
    }

    #[cfg(unix)]
    #[test]
    fn real_process_table_fixture_exercises_pid_replacement_admission() {
        let process_table_fixture = ReplacementProcessTableFixture::spawn();

        assert_eq!(
            process_table_fixture.admission(),
            TerminationSignalAdmission::PidReused
        );
    }

    #[cfg(unix)]
    #[test]
    fn real_same_pid_exec_fixture_is_rejected_by_the_admission_path() {
        let mut process_fixture = ExecProcessFixture::spawn();
        let capability = process_fixture.capability();
        assert_eq!(
            capability.observe_admission(),
            TerminationSignalAdmission::SameProcessObject
        );

        process_fixture.exec_sleep();
        for _ in 0..ADMISSION_POLL_ATTEMPTS {
            if capability.observe_admission() == TerminationSignalAdmission::ProcessImageReplaced {
                return;
            }
            std::thread::sleep(TERMINATION_CONFIRMATION_POLL_INTERVAL);
        }
        panic!("the real same-PID exec fixture was not rejected before the deadline");
    }

    #[cfg(unix)]
    #[test]
    fn definite_disappearance_from_identity_lookup_is_preserved() {
        let mut process_fixture = ExecProcessFixture::spawn();
        let capability = process_fixture.capability();
        process_fixture.exec_sleep();
        process_fixture
            .child
            .kill()
            .expect("the disappearance fixture should accept termination");
        process_fixture
            .child
            .wait()
            .expect("the disappearance fixture should be reaped");

        assert_eq!(
            observe_process_image_continuity(&capability.authorized_incarnation),
            ProcessImageContinuity::ProcessGone
        );
        assert_eq!(
            capability.observe_admission(),
            TerminationSignalAdmission::ProcessGone
        );
    }

    #[test]
    fn unavailable_identity_and_image_evidence_remain_distinct_from_disappearance() {
        assert_eq!(
            admit_termination_signal(
                &StrongProcessIdentityRevalidation::Unavailable(
                    InsufficientProcessIdentity::PlatformIdentityLookupFailed { pid: 4242 },
                ),
                ProcessImageContinuity::SameImage,
            ),
            TerminationSignalAdmission::RevalidationUnavailable
        );
        assert_eq!(
            admit_termination_signal(
                &StrongProcessIdentityRevalidation::Current,
                ProcessImageContinuity::Unobservable,
            ),
            TerminationSignalAdmission::RevalidationUnavailable
        );
        assert_eq!(
            admit_termination_signal(
                &StrongProcessIdentityRevalidation::Current,
                ProcessImageContinuity::ProcessGone,
            ),
            TerminationSignalAdmission::ProcessGone
        );
    }

    #[test]
    fn replaced_identity_takes_precedence_over_every_image_observation() {
        let replacement_identity = ProcessIdentity::for_test(4242, 8);
        let cases = [
            (
                ProcessImageContinuity::SameImage,
                TerminationSignalAdmission::PidReused,
            ),
            (
                ProcessImageContinuity::ReplacedImage,
                TerminationSignalAdmission::PidReused,
            ),
            (
                ProcessImageContinuity::ProcessGone,
                TerminationSignalAdmission::PidReused,
            ),
            (
                ProcessImageContinuity::Unobservable,
                TerminationSignalAdmission::PidReused,
            ),
        ];

        for (process_image_continuity, expected_admission) in cases {
            let identity_revalidation =
                StrongProcessIdentityRevalidation::Replaced(replacement_identity.clone());
            assert_eq!(
                admit_termination_signal(&identity_revalidation, process_image_continuity),
                expected_admission,
                "unexpected admission for {identity_revalidation:?} and {process_image_continuity:?}"
            );
        }
    }

    #[test]
    fn exited_identity_evidence_takes_precedence_over_image_observations() {
        let cases = [
            (
                ProcessImageContinuity::SameImage,
                TerminationSignalAdmission::ProcessGone,
            ),
            (
                ProcessImageContinuity::ReplacedImage,
                TerminationSignalAdmission::ProcessGone,
            ),
            (
                ProcessImageContinuity::ProcessGone,
                TerminationSignalAdmission::ProcessGone,
            ),
            (
                ProcessImageContinuity::Unobservable,
                TerminationSignalAdmission::ProcessGone,
            ),
        ];

        for (process_image_continuity, expected_admission) in cases {
            let identity_revalidation = StrongProcessIdentityRevalidation::Unavailable(
                InsufficientProcessIdentity::ProcessExitedBeforeIdentityLookup { pid: 4242 },
            );
            assert_eq!(
                admit_termination_signal(&identity_revalidation, process_image_continuity),
                expected_admission,
                "unexpected admission for {identity_revalidation:?} and {process_image_continuity:?}"
            );
        }
    }

    #[test]
    fn unavailable_identity_evidence_defers_to_process_disappearance() {
        let cases = [
            (
                ProcessImageContinuity::SameImage,
                TerminationSignalAdmission::RevalidationUnavailable,
            ),
            (
                ProcessImageContinuity::ReplacedImage,
                TerminationSignalAdmission::RevalidationUnavailable,
            ),
            (
                ProcessImageContinuity::ProcessGone,
                TerminationSignalAdmission::ProcessGone,
            ),
            (
                ProcessImageContinuity::Unobservable,
                TerminationSignalAdmission::RevalidationUnavailable,
            ),
        ];

        for (process_image_continuity, expected_admission) in cases {
            let identity_revalidation = StrongProcessIdentityRevalidation::Unavailable(
                InsufficientProcessIdentity::PlatformIdentityLookupFailed { pid: 4242 },
            );
            assert_eq!(
                admit_termination_signal(&identity_revalidation, process_image_continuity),
                expected_admission,
                "unexpected admission for {identity_revalidation:?} and {process_image_continuity:?}"
            );
        }
    }

    #[test]
    fn current_identity_evidence_defers_to_image_observation() {
        let cases = [
            (
                ProcessImageContinuity::SameImage,
                TerminationSignalAdmission::SameProcessObject,
            ),
            (
                ProcessImageContinuity::ReplacedImage,
                TerminationSignalAdmission::ProcessImageReplaced,
            ),
            (
                ProcessImageContinuity::ProcessGone,
                TerminationSignalAdmission::ProcessGone,
            ),
            (
                ProcessImageContinuity::Unobservable,
                TerminationSignalAdmission::RevalidationUnavailable,
            ),
        ];

        for (process_image_continuity, expected_admission) in cases {
            let identity_revalidation = StrongProcessIdentityRevalidation::Current;
            assert_eq!(
                admit_termination_signal(&identity_revalidation, process_image_continuity),
                expected_admission,
                "unexpected admission for {identity_revalidation:?} and {process_image_continuity:?}"
            );
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_real_process_capability_uses_pidfd() {
        let process_fixture = ExecProcessFixture::spawn();
        let capability = process_fixture.capability();
        assert!(capability.has_identity_bound_adapter());
    }

    #[cfg(all(unix, not(target_os = "linux")))]
    #[test]
    fn hosts_without_a_safe_adapter_produce_observed_only_capabilities() {
        let process_fixture = ExecProcessFixture::spawn();
        let capability = process_fixture.capability();
        assert!(!capability.has_identity_bound_adapter());
    }

    #[test]
    fn bound_fixture_reports_a_survivor_without_escalation() {
        let capability = ExternalProcessTerminationCapability::for_test(
            ProcessIncarnation::for_test(ProcessIdentity::for_test(4242, 7), "/usr/bin/cargo"),
            BoundSignalDelivery::Accepted,
            &[BoundProcessObjectPresence::Present; 8],
        );
        assert_eq!(
            capability.confirm_process_object_gone(TERMINATION_CONFIRMATION_POLL_INTERVAL * 2),
            BoundProcessObjectPresence::Present
        );
    }
}
