//! Retain native process birth precision independently of registration comparison.

#[cfg(any(target_os = "macos", test))]
use std::time::Duration;

use super::BirthStamp;
#[cfg(target_os = "linux")]
use super::Observation;
use super::Verification;
#[cfg(target_os = "linux")]
use super::linux;
#[cfg(target_os = "macos")]
use super::macos;
#[cfg(target_os = "macos")]
use crate::constants::BIRTH_MICROSECONDS_PER_SECOND;

/// Kernel birth at its native precision, separate from publisher comparison fields.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct ProcessLifetime {
    /// A reboot starts a different process namespace.
    boot:  String,
    /// Linux ticks or Darwin epoch microseconds, without truncating to seconds.
    birth: u64,
}

impl ProcessLifetime {
    /// Compare native precision with the registration format on this platform.
    pub(crate) fn compare_registration(&self, registered: &BirthStamp) -> Verification {
        #[cfg(target_os = "macos")]
        let birth = self.birth / u64::from(BIRTH_MICROSECONDS_PER_SECOND);
        #[cfg(not(target_os = "macos"))]
        let birth = self.birth;
        registered.compare(&super::Observation::Present(BirthStamp {
            boot: self.boot.clone(),
            birth,
        }))
    }
    /// Preserve the entire timeval where registration comparison keeps seconds only.
    #[cfg(any(target_os = "macos", test))]
    pub(super) fn macos(boot: String, birth: Duration) -> LifetimeEvidence {
        u64::try_from(birth.as_micros()).map_or(LifetimeEvidence::Unavailable, |birth| {
            LifetimeEvidence::Available(Self { boot, birth })
        })
    }

    /// Fixtures explicitly supply precise births without implying kernel proof.
    #[cfg(test)]
    pub(crate) fn for_test(birth: u64) -> Self {
        Self {
            boot: String::from("test-boot"),
            birth,
        }
    }

    /// Supply native evidence for a fixture's independently verified registration.
    #[cfg(test)]
    pub(crate) fn matching_registration_for_test(registered: &BirthStamp) -> Self {
        #[cfg(target_os = "macos")]
        let birth = registered.birth * u64::from(BIRTH_MICROSECONDS_PER_SECOND);
        #[cfg(not(target_os = "macos"))]
        let birth = registered.birth;
        Self {
            boot: registered.boot.clone(),
            birth,
        }
    }
}

/// A failed lifetime read permits no comparison between process observations.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum LifetimeEvidence {
    /// Native kernel birth precision is available for this process.
    Available(ProcessLifetime),
    /// Absence must never be interpreted as a distinct or matching lifetime.
    Unavailable,
}

/// Read native lifetime precision independently of the shim's comparison format.
pub(crate) fn lifetime(pid: u32) -> LifetimeEvidence {
    #[cfg(target_os = "linux")]
    {
        match linux::observe(pid) {
            Observation::Present(stamp) => LifetimeEvidence::Available(ProcessLifetime {
                boot:  stamp.boot,
                birth: stamp.birth,
            }),
            Observation::Ended | Observation::Unknown => LifetimeEvidence::Unavailable,
        }
    }
    #[cfg(target_os = "macos")]
    {
        macos::lifetime(pid)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = pid;
        LifetimeEvidence::Unavailable
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::birth_stamp::BirthStamp;
    use crate::birth_stamp::Verification;

    #[test]
    fn a_native_lifetime_join_rejects_a_different_boot_or_birth() {
        let registered = BirthStamp {
            boot:  String::from("boot"),
            birth: 100,
        };
        let matching = super::ProcessLifetime::matching_registration_for_test(&registered);
        assert_eq!(
            matching.compare_registration(&registered),
            Verification::Confirmed
        );
        let other_boot = BirthStamp {
            boot: String::from("another boot"),
            ..registered
        };
        let other_birth = BirthStamp {
            birth: 101,
            ..registered
        };
        assert_eq!(
            matching.compare_registration(&other_boot),
            Verification::Ended
        );
        assert_eq!(
            matching.compare_registration(&other_birth),
            Verification::Ended
        );
    }

    #[test]
    fn macos_births_inside_one_second_share_comparison_but_not_process_lifetime() {
        let first = Duration::from_secs(100) + Duration::from_micros(1);
        let second = Duration::from_secs(100) + Duration::from_micros(2);
        assert_eq!(
            BirthStamp::macos("boot".to_owned(), first),
            BirthStamp::macos("boot".to_owned(), second)
        );
        let first = super::ProcessLifetime::macos("boot".to_owned(), first);
        let second = super::ProcessLifetime::macos("boot".to_owned(), second);
        assert!(matches!(first, super::LifetimeEvidence::Available(_)));
        assert!(matches!(second, super::LifetimeEvidence::Available(_)));
        assert_ne!(first, second);
    }
}
