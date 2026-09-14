//! Constants for the shared reservation ledger.

use std::time::Duration;

// file names
pub(super) const COORDINATION_RUN_MARKER_FILE_NAME: &str = "cargo-berth-run-id";
pub(super) const COORDINATION_RUN_MARKER_RETIREMENT_SUFFIX: &str = "retiring";
pub(super) const JOURNAL_FILE_NAME: &str = "journal.ndjson";
pub(super) const LOCK_FILE_NAME: &str = "mutation.lock";
pub(super) const PROJECTION_FILE_NAME: &str = "reservations.json";
pub(super) const PROJECTION_TEMPORARY_FILE_NAME: &str = "reservations.json.tmp";
pub(super) const REPO_INSTANCE_ID_FILE_NAME: &str = "repo-instance-id";
pub(super) const WORKTREE_ID_FILE_NAME: &str = "cargo-berth-worktree-id";

// git reference validation
pub(super) const DELETE_CONTROL_BYTE: u8 = 0x7f;

// journal limits
pub(super) const MAXIMUM_JOURNAL_RECORD_BYTES: usize = 16 * 1_024;
/// Maximum JSON-encoded string-content bytes retained for one actor identity input.
pub(super) const MAXIMUM_RECORDED_IDENTITY_INPUT_VALUE_BYTES: usize = 256;

// ledger layout
pub(super) const LEDGER_DIRECTORY_NAME: &str = "cargo-berth";

// lock acquisition
pub(crate) const MUTATING_VERB_CONTENTION_TOLERANCE: Duration = Duration::from_secs(10);
pub(super) const MUTATION_LOCK_INITIAL_RETRY_INTERVAL: Duration = Duration::from_millis(50);
pub(super) const MUTATION_LOCK_MAXIMUM_RETRY_INTERVAL: Duration = Duration::from_secs(1);
/// Test-only signal path that makes a waiting `MutationLock::acquire` observable.
pub(super) const MUTATION_LOCK_READY_PATH_ENVIRONMENT: &str =
    "CARGO_BERTH_TEST_MUTATION_LOCK_READY_PATH";

// process context
pub(super) const COORDINATION_RUN_ENVIRONMENT: &str = "CARGO_BERTH_RUN";
pub(super) const GIT_COMMON_DIRECTORY_ENVIRONMENT: &str = "GIT_COMMON_DIR";
pub(super) const GIT_DIRECTORY_ENVIRONMENT: &str = "GIT_DIR";
pub(crate) const HARNESS_SESSION_ENVIRONMENT: &str = "CARGO_BERTH_SESSION_ID";

// test deadlines
/// Test-only milliseconds that can shorten the total gate deadline in debug builds.
pub(crate) const GATE_DEADLINE_ENVIRONMENT: &str = "CARGO_BERTH_TEST_GATE_DEADLINE_MS";
/// Test-only milliseconds that can shorten mutation lock contention in debug builds.
pub(super) const MUTATION_LOCK_CONTENTION_TOLERANCE_ENVIRONMENT: &str =
    "CARGO_BERTH_TEST_LOCK_CONTENTION_TOLERANCE_MS";

// wire format
/// The schema version written by new projection caches.
pub(super) const CURRENT_PROJECTION_SCHEMA_VERSION: u32 = 3;
/// The schema version written by new append-only journal records.
pub(super) const CURRENT_SCHEMA_VERSION: u32 = 2;

/// Shorten a supplied duration using unsigned milliseconds from a debug-only test hook.
pub(crate) fn shortened_by_environment(variable: &str, supplied: Duration) -> Duration {
    #[cfg(debug_assertions)]
    {
        std::env::var(variable)
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .map_or(supplied, |milliseconds| {
                supplied.min(Duration::from_millis(milliseconds))
            })
    }
    #[cfg(not(debug_assertions))]
    {
        let _ = variable;
        supplied
    }
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::process::Command;
    use std::time::Duration;

    use super::MUTATING_VERB_CONTENTION_TOLERANCE;
    use super::MUTATION_LOCK_CONTENTION_TOLERANCE_ENVIRONMENT;
    use super::shortened_by_environment;

    const EXPECTED_MILLISECONDS: &str = "CARGO_BERTH_TEST_EXPECTED_DEADLINE_MS";
    const SHORTENED: Duration = Duration::from_millis(25);

    #[cfg(debug_assertions)]
    #[test]
    fn shortened_by_environment_only_shortens() -> Result<(), Box<dyn std::error::Error>> {
        assert_shortened_by_environment_cases(
            "ledger::constants::tests::shortened_by_environment_only_shortens",
        )
    }

    #[cfg(not(debug_assertions))]
    #[test]
    fn shortened_by_environment_ignores_override_in_release()
    -> Result<(), Box<dyn std::error::Error>> {
        assert_shortened_by_environment_cases(
            "ledger::constants::tests::shortened_by_environment_ignores_override_in_release",
        )
    }

    fn assert_shortened_by_environment_cases(
        test_name: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Re-execute this unit test so environment cases cannot race other tests.
        let supplied = MUTATING_VERB_CONTENTION_TOLERANCE;
        if let Ok(expected) = env::var(EXPECTED_MILLISECONDS) {
            assert_eq!(
                shortened_by_environment(MUTATION_LOCK_CONTENTION_TOLERANCE_ENVIRONMENT, supplied),
                Duration::from_millis(expected.parse()?)
            );
            assert_eq!(
                shortened_by_environment(
                    MUTATION_LOCK_CONTENTION_TOLERANCE_ENVIRONMENT,
                    Duration::ZERO,
                ),
                Duration::ZERO,
            );
            return Ok(());
        }
        for (value, expected) in [
            (None, supplied),
            (Some("invalid".to_owned()), supplied),
            (Some("-1".to_owned()), supplied),
            (Some("18446744073709551616".to_owned()), supplied),
            (Some((supplied.as_millis() + 1).to_string()), supplied),
            (Some(SHORTENED.as_millis().to_string()), SHORTENED),
            (Some("0".to_owned()), Duration::ZERO),
        ] {
            let mut command = Command::new(env::current_exe()?);
            command.args(["--exact", test_name, "--nocapture"]);
            command.env_remove(MUTATION_LOCK_CONTENTION_TOLERANCE_ENVIRONMENT);
            if let Some(value) = value {
                command.env(MUTATION_LOCK_CONTENTION_TOLERANCE_ENVIRONMENT, value);
            }
            let expected = if cfg!(debug_assertions) {
                expected
            } else {
                supplied
            };
            let output = command
                .env(EXPECTED_MILLISECONDS, expected.as_millis().to_string())
                .output()?;
            assert!(output.status.success(), "{output:?}");
        }
        Ok(())
    }
}
