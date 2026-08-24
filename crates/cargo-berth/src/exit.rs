//! The process exit-code contract for `cargo-berth`.
//!
//! | Value | `BerthExit` variant | Meaning |
//! | --- | --- | --- |
//! | `0` | [`BerthExit::Clear`] | The command may proceed. |
//! | `1` | [`BerthExit::BlockedByOverlap`] | A reservation overlap blocks the command. |
//! | `2` | [`BerthExit::BlockedByOrdering`] | An unsatisfied ordering edge blocks the command. |
//! | `3` | [`BerthExit::NeedsUserAuthorization`] | The command needs user authorization. |
//! | `4` | [`BerthExit::LedgerUnreadable`] | The ledger cannot be read. Edit paths fail open; `integrate` fails closed. |
//! | `5` | [`BerthExit::UsageError`] | The command line is invalid. |
//! | `6` | [`BerthExit::BlockedByContention`] | Another mutation holds the ledger lock; retry the command. |

use std::error::Error;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::process::ExitCode;

use serde::Deserialize;
use serde::Serialize;

/// An exit code returned by `cargo-berth`.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[repr(u8)]
#[serde(into = "u8", try_from = "u8")]
pub(crate) enum BerthExit {
    /// The command is clear to proceed.
    Clear                  = 0,
    /// A reservation overlap blocks the command.
    BlockedByOverlap       = 1,
    /// An unsatisfied ordering edge blocks the command.
    BlockedByOrdering      = 2,
    /// The command needs user authorization.
    NeedsUserAuthorization = 3,
    /// The ledger cannot be read.
    LedgerUnreadable       = 4,
    /// The command line is invalid.
    UsageError             = 5,
    /// Another mutation holds the ledger lock through the bounded wait.
    BlockedByContention    = 6,
}

impl BerthExit {
    /// Return this exit status's published numeric value.
    #[must_use]
    const fn code(self) -> u8 { self as u8 }
}

impl From<BerthExit> for u8 {
    fn from(berth_exit: BerthExit) -> Self { berth_exit.code() }
}

impl TryFrom<u8> for BerthExit {
    type Error = InvalidBerthExitCode;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Clear),
            1 => Ok(Self::BlockedByOverlap),
            2 => Ok(Self::BlockedByOrdering),
            3 => Ok(Self::NeedsUserAuthorization),
            4 => Ok(Self::LedgerUnreadable),
            5 => Ok(Self::UsageError),
            6 => Ok(Self::BlockedByContention),
            _ => Err(InvalidBerthExitCode(value)),
        }
    }
}

#[derive(Debug)]
pub(crate) struct InvalidBerthExitCode(u8);

impl Display for InvalidBerthExitCode {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} is not a cargo-berth exit code", self.0)
    }
}

impl Error for InvalidBerthExitCode {}

impl From<BerthExit> for ExitCode {
    fn from(berth_exit: BerthExit) -> Self { Self::from(berth_exit.code()) }
}

#[cfg(test)]
mod tests {
    use super::BerthExit;

    #[test]
    fn every_documented_exit_code_has_its_published_value() {
        assert_eq!(BerthExit::Clear.code(), 0);
        assert_eq!(BerthExit::BlockedByOverlap.code(), 1);
        assert_eq!(BerthExit::BlockedByOrdering.code(), 2);
        assert_eq!(BerthExit::NeedsUserAuthorization.code(), 3);
        assert_eq!(BerthExit::LedgerUnreadable.code(), 4);
        assert_eq!(BerthExit::UsageError.code(), 5);
        assert_eq!(BerthExit::BlockedByContention.code(), 6);
    }

    #[test]
    fn exit_codes_serialize_as_numbers_and_reject_unknown_values() {
        let serialized_exit = serde_json::to_string(&BerthExit::Clear);

        assert!(
            serialized_exit
                .as_ref()
                .is_ok_and(|serialized_exit| serialized_exit == "0")
        );
        assert!(serde_json::from_str::<BerthExit>("6").is_ok());
        assert!(serde_json::from_str::<BerthExit>("7").is_err());
    }
}
