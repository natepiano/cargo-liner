//! Account source absence and command failures remain distinct.

use super::*;
use crate::census::SourceHealth;
use crate::census::UnknownReason;

#[test]
fn accounts_source_retains_absence_loaded_rows_and_command_failure_reason() {
    for (command, expected) in [
        ("", SourceHealth::Absent),
        (
            "printf '{\"accounts\":[],\"problems\":[]}'",
            SourceHealth::Loaded,
        ),
        (
            "exit 7",
            SourceHealth::Unreadable(UnknownReason("accounts: command failed".into())),
        ),
    ] {
        assert_eq!(
            AccountsTable::read_once(command).health(),
            Observed::Known(expected),
            "{command}"
        );
    }
}
