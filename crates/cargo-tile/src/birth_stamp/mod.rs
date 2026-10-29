//! Compare registrations with boot-qualified kernel process births.

mod kernel_observation;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
mod process_lifetime;

#[cfg(any(target_os = "macos", test))]
use std::time::Duration;

pub(crate) use kernel_observation::IdentityEvidence;
pub(crate) use kernel_observation::KernelObservation;
pub(crate) use kernel_observation::Observation;
pub(crate) use kernel_observation::Verification;
pub(crate) use kernel_observation::boot_verification;
pub(crate) use kernel_observation::observe;
pub(crate) use process_lifetime::LifetimeEvidence;
pub(crate) use process_lifetime::ProcessLifetime;
pub(crate) use process_lifetime::lifetime;

use crate::constants::BIRTH_MACOS_BOOT_PREFIX;
use crate::constants::BIRTH_MACOS_BOOT_SEPARATOR;
use crate::constants::BIRTH_MACOS_BOOT_SUFFIX;
use crate::constants::BIRTH_MICROSECONDS_PER_SECOND;

/// Process identity needs both the boot and the kernel's birth counter.
/// This value is evidence to compare; it is not proof that a record is live.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct BirthStamp {
    /// Boot-session UUID, or a legacy macOS calendar-derived boot timeval.
    boot:  String,
    /// Linux clock ticks or macOS epoch seconds, matching the publisher.
    birth: u64,
}

impl BirthStamp {
    /// Empty identity fields and old textual macOS births remain unverifiable.
    pub(crate) fn from_fields(boot: &str, birth: &str) -> IdentityEvidence {
        let Ok(birth) = Self::decimal(birth) else {
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
            let (Ok(seconds), Ok(microseconds)) =
                (Self::decimal(seconds), Self::decimal(microseconds))
            else {
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
    fn compare(&self, observation: &Observation) -> Verification {
        match observation {
            Observation::Present(live) if self == live => Verification::Confirmed,
            Observation::Present(live)
                if self.boot.starts_with(BIRTH_MACOS_BOOT_PREFIX)
                    || live.boot.starts_with(BIRTH_MACOS_BOOT_PREFIX) =>
            {
                // Legacy Darwin boot time follows clock corrections. A mismatch
                // with a live process therefore proves neither reuse nor exit.
                Verification::Unknown
            },
            Observation::Present(_) | Observation::Ended => Verification::Ended,
            Observation::Unknown => Verification::Unknown,
        }
    }

    /// Reject signs, whitespace, and old locale-dependent date text at the boundary.
    fn decimal(value: &str) -> Result<u64, ()> {
        if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(());
        }
        value.parse().map_err(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::BirthStamp;
    use super::IdentityEvidence;

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
