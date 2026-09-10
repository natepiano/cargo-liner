//! Compare registrations with boot-qualified kernel process births.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;

#[cfg(any(target_os = "macos", test))]
use std::time::Duration;

use crate::constants::BIRTH_MACOS_BOOT_PREFIX;
use crate::constants::BIRTH_MACOS_BOOT_SEPARATOR;
use crate::constants::BIRTH_MACOS_BOOT_SUFFIX;
use crate::constants::BIRTH_MICROSECONDS_PER_SECOND;

/// Process identity needs both the boot and the kernel's birth counter.
/// This value is evidence to compare; it is not proof that a record is live.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BirthStamp {
    /// Boot UUID on Linux, numeric boot timeval on macOS.
    boot:  String,
    /// Linux clock ticks or macOS epoch seconds, matching the publisher.
    birth: u64,
}

impl BirthStamp {
    /// Empty identity fields and old textual macOS births remain unverifiable.
    pub(crate) fn from_fields(boot: &str, birth: &str) -> IdentityEvidence {
        let Ok(birth) = decimal(birth) else {
            return IdentityEvidence::Unavailable;
        };
        let boot = boot.trim();
        if boot.is_empty() {
            return IdentityEvidence::Unavailable;
        }
        // Darwin's trailing date is presentation; its numeric timeval identifies
        // the boot even when the writer and reader have different time zones.
        let boot = if boot.starts_with(BIRTH_MACOS_BOOT_PREFIX) {
            let Some((boot, _)) = boot.split_once(BIRTH_MACOS_BOOT_SUFFIX) else {
                return IdentityEvidence::Unavailable;
            };
            let Some((seconds, microseconds)) = boot
                .strip_prefix(BIRTH_MACOS_BOOT_PREFIX)
                .and_then(|value| value.split_once(BIRTH_MACOS_BOOT_SEPARATOR))
            else {
                return IdentityEvidence::Unavailable;
            };
            let (Ok(seconds), Ok(microseconds)) = (decimal(seconds), decimal(microseconds)) else {
                return IdentityEvidence::Unavailable;
            };
            if microseconds >= u64::from(BIRTH_MICROSECONDS_PER_SECOND) {
                return IdentityEvidence::Unavailable;
            }
            format!(
                "{BIRTH_MACOS_BOOT_PREFIX}{seconds}{BIRTH_MACOS_BOOT_SEPARATOR}{microseconds}{BIRTH_MACOS_BOOT_SUFFIX}"
            )
        } else {
            boot.to_owned()
        };
        IdentityEvidence::Available(Self { boot, birth })
    }

    /// Linux's native tick counter must retain every tick through comparison.
    #[cfg(any(target_os = "linux", test))]
    const fn linux(boot: String, ticks: u64) -> Self { Self { boot, birth: ticks } }

    /// The POSIX publisher can only recover macOS births to whole seconds.
    #[cfg(any(target_os = "macos", test))]
    const fn macos(boot: String, birth: Duration) -> Self {
        Self {
            boot,
            birth: birth.as_secs(),
        }
    }

    /// Compare complete identities; a missing observation preserves uncertainty.
    pub(crate) fn compare(&self, observation: &Observation) -> Verification {
        match observation {
            Observation::Present(live) if self == live => Verification::Confirmed,
            Observation::Present(_) | Observation::Ended => Verification::Ended,
            Observation::Unknown => Verification::Unknown,
        }
    }
}

/// Whether a record supplied enough identity information for a comparison.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum IdentityEvidence {
    /// Both fields use the publisher's normalized representation.
    Available(BirthStamp),
    /// Empty or old-format identity fields authorize neither rows nor cleanup.
    Unavailable,
}

impl IdentityEvidence {
    /// Even a missing live pid cannot authorize deletion of an unverified record.
    pub(crate) fn compare(&self, observation: &Observation) -> Verification {
        match self {
            Self::Available(stamp) => stamp.compare(observation),
            Self::Unavailable => Verification::Unknown,
        }
    }
}

/// An identity comparison input; only `KernelObservation` proves its kernel source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Observation {
    /// The process still exists and supplied a complete birth stamp.
    Present(BirthStamp),
    /// The kernel reports that this pid no longer exists.
    Ended,
    /// Access was denied or the response was incomplete.
    Unknown,
}

/// Kernel evidence bound to the pid whose state was read, independently of snapshots.
/// Private fields prevent parsed birth stamps from becoming kernel observations.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct KernelObservation {
    /// A matching stamp cannot verify a different pid from this observation's source.
    pid:         u32,
    /// Only the platform reader supplies this evidence in production.
    observation: Observation,
}

impl KernelObservation {
    /// Evidence about another pid authorizes neither admission nor removal.
    pub(crate) fn compare(&self, pid: u32, identity: &IdentityEvidence) -> Verification {
        if self.pid != pid {
            return Verification::Unknown;
        }
        identity.compare(&self.observation)
    }

    /// Sweep race tests control each observation without exposing production injection.
    #[cfg(test)]
    pub(crate) const fn for_test(pid: u32, observation: Observation) -> Self {
        Self { pid, observation }
    }
}

/// Only a completed identity comparison may authorize an active registration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Verification {
    /// The registered boot and birth match the process currently holding the pid.
    Confirmed,
    /// The writer ended, even if another process now holds its pid.
    Ended,
    /// Uncertainty authorizes neither an active row nor an artifact deletion.
    Unknown,
}

/// Read kernel state afresh; process-table snapshot absence is never enough.
pub(crate) fn observe(pid: u32) -> KernelObservation {
    KernelObservation {
        pid,
        observation: observe_process(pid),
    }
}

/// Keep platform comparison inputs behind the pid-binding observation constructor.
fn observe_process(pid: u32) -> Observation {
    #[cfg(target_os = "linux")]
    {
        linux::observe(pid)
    }
    #[cfg(target_os = "macos")]
    {
        macos::observe(pid)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = pid;
        Observation::Unknown
    }
}

/// Reject signs, whitespace, and old locale-dependent date text at the boundary.
fn decimal(value: &str) -> Result<u64, ()> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(());
    }
    value.parse().map_err(|_| ())
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::time::Duration;

    use super::BirthStamp;
    use super::IdentityEvidence;
    use super::Observation;
    use super::Verification;
    use super::observe;
    use crate::constants::BIRTH_MICROSECONDS_PER_SECOND;
    use crate::registration::Registration;
    use crate::registration::RegistrationVerification;

    #[test]
    fn kernel_observation_verifies_only_the_pid_it_read() {
        let pid = std::process::id();
        let observation = observe(pid);
        let stamp = match &observation.observation {
            Observation::Present(stamp) => Ok(stamp),
            Observation::Ended | Observation::Unknown => {
                Err("current process birth is unavailable")
            },
        }
        .expect("kernel must report this test process birth");
        let bytes = [
            "cargo-tile-v2",
            "generation",
            &stamp.boot,
            &stamp.birth.to_string(),
            "exact.log",
            "/work",
            "/home/writer",
            "0",
            "",
        ]
        .join("\0");
        let record = match Registration::parse(bytes.as_bytes()).expect("valid live record") {
            Registration::Versioned(record) => Ok(record),
            Registration::Legacy(_) => Err("expected versioned record"),
        }
        .expect("fixture supplies a versioned record");
        assert!(matches!(
            record.verify_observation(pid, &observation),
            RegistrationVerification::Confirmed(verified) if verified.pid() == pid && verified.record() == &record
        ));
        assert_eq!(
            record.verify_observation(pid.wrapping_add(1), &observation),
            RegistrationVerification::Unknown
        );
    }

    #[test]
    fn mismatched_live_birth_is_ended() {
        let record = BirthStamp::from_fields("boot", "101");
        let live = Observation::Present(BirthStamp::linux("boot".to_owned(), 102));
        assert_eq!(record.compare(&live), Verification::Ended);
    }

    #[test]
    fn boot_change_ends_an_otherwise_identical_birth() {
        let record = BirthStamp::from_fields("previous", "101");
        let live = Observation::Present(BirthStamp::linux("current".to_owned(), 101));
        assert_eq!(record.compare(&live), Verification::Ended);
    }

    #[test]
    fn missing_or_nonnumeric_identity_remains_unknown_even_when_pid_ended() {
        for (boot, birth) in [
            ("", "101"),
            ("boot", ""),
            ("boot", "+101"),
            ("boot", "101 "),
            ("boot", "Wed Sep 9 20:00:00 2026"),
            ("boot", "18446744073709551616"),
        ] {
            let record = BirthStamp::from_fields(boot, birth);
            assert_eq!(record, IdentityEvidence::Unavailable);
            assert_eq!(record.compare(&Observation::Ended), Verification::Unknown);
        }
    }

    #[test]
    fn denied_observation_neither_confirms_nor_ends_a_registration() {
        assert_eq!(
            BirthStamp::from_fields("boot", "101").compare(&Observation::Unknown),
            Verification::Unknown
        );
    }

    #[test]
    fn macos_discards_fractional_seconds_but_linux_keeps_individual_ticks() {
        let record = BirthStamp::from_fields("boot", "101");
        let birth = Duration::from_secs(101)
            + Duration::from_micros(u64::from(BIRTH_MICROSECONDS_PER_SECOND - 1));
        assert_eq!(
            record.compare(&Observation::Present(BirthStamp::macos(
                "boot".to_owned(),
                birth
            ))),
            Verification::Confirmed
        );
        assert_eq!(
            record.compare(&Observation::Present(BirthStamp::linux(
                "boot".to_owned(),
                102
            ))),
            Verification::Ended
        );
    }

    #[test]
    fn macos_boot_identity_ignores_localized_date_suffix_only() {
        let writer =
            BirthStamp::from_fields("{ sec = 100, usec = 23 } Wed Sep 9 20:00:00 2026", "101");
        let reader = BirthStamp::from_fields("{ sec = 100, usec = 23 } jeudi", "101");
        assert_eq!(writer, reader);
        assert_ne!(
            writer,
            BirthStamp::from_fields("{ sec = 100, usec = 24 } jeudi", "101")
        );
    }

    #[test]
    fn incomplete_or_invalid_macos_boot_time_is_unknown() {
        for boot in [
            "{ sec = 100",
            "{ sec = 100, usec = -1 }",
            "{ sec = 100, usec = 1000000 }",
        ] {
            assert_eq!(
                BirthStamp::from_fields(boot, "101"),
                IdentityEvidence::Unavailable
            );
        }
    }
}
