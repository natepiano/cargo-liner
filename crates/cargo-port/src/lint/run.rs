use serde::Deserialize;
use serde::Serialize;

/// Origin of a live lint run as reported by the runtime.
///
/// This is intentionally separate from `LintStatus`: cached/project status
/// only needs `Running` / terminal state, while toast routing must distinguish
/// startup catch-up work from later file-triggered work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LintRunOrigin {
    CatchUp,
    Normal,
}

impl LintRunOrigin {
    pub const fn merged_with(self, other: Self) -> Self {
        match (self, other) {
            (Self::Normal, _) | (_, Self::Normal) => Self::Normal,
            (Self::CatchUp, Self::CatchUp) => Self::CatchUp,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LintRunStatus {
    Running,
    Passed,
    Failed,
    /// The project's environment could not be prepared, so none of its lint
    /// commands ran. Distinct from `Failed`: nothing was found wrong with the
    /// code, because nothing examined it. Only `direnv exec` projects can
    /// reach this — see `crate::lint::runtime::command::env_probe`.
    EnvUnavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LintCommandStatus {
    Pending,
    Passed,
    Failed,
    /// The run ended before this command started, so it has no result and no
    /// log. Written when the environment probe fails.
    Skipped,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(from = "Option<u64>", into = "Option<u64>")]
pub enum LintCommandDuration {
    #[default]
    NotRecorded,
    MeasuredMilliseconds(u64),
}

impl From<Option<u64>> for LintCommandDuration {
    fn from(value: Option<u64>) -> Self {
        value.map_or(Self::NotRecorded, Self::MeasuredMilliseconds)
    }
}

impl From<LintCommandDuration> for Option<u64> {
    fn from(value: LintCommandDuration) -> Self {
        match value {
            LintCommandDuration::MeasuredMilliseconds(value) => Some(value),
            LintCommandDuration::NotRecorded => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(from = "Option<i32>", into = "Option<i32>")]
pub enum LintCommandExitCode {
    #[default]
    NotRecorded,
    Recorded(i32),
}

impl From<Option<i32>> for LintCommandExitCode {
    fn from(value: Option<i32>) -> Self { value.map_or(Self::NotRecorded, Self::Recorded) }
}

impl From<LintCommandExitCode> for Option<i32> {
    fn from(value: LintCommandExitCode) -> Self {
        match value {
            LintCommandExitCode::Recorded(value) => Some(value),
            LintCommandExitCode::NotRecorded => None,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(from = "Option<String>", into = "Option<String>")]
pub enum LintRunFinishTime {
    #[default]
    NotRecorded,
    Recorded(String),
}

impl From<Option<String>> for LintRunFinishTime {
    fn from(value: Option<String>) -> Self { value.map_or(Self::NotRecorded, Self::Recorded) }
}

impl From<LintRunFinishTime> for Option<String> {
    fn from(value: LintRunFinishTime) -> Self {
        match value {
            LintRunFinishTime::Recorded(value) => Some(value),
            LintRunFinishTime::NotRecorded => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(from = "Option<u64>", into = "Option<u64>")]
pub enum LintRunDuration {
    #[default]
    NotRecorded,
    MeasuredMilliseconds(u64),
}

impl From<Option<u64>> for LintRunDuration {
    fn from(value: Option<u64>) -> Self {
        value.map_or(Self::NotRecorded, Self::MeasuredMilliseconds)
    }
}

impl From<LintRunDuration> for Option<u64> {
    fn from(value: LintRunDuration) -> Self {
        match value {
            LintRunDuration::MeasuredMilliseconds(value) => Some(value),
            LintRunDuration::NotRecorded => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LintCommand {
    pub name:        String,
    pub command:     String,
    pub status:      LintCommandStatus,
    #[serde(default)]
    pub duration_ms: LintCommandDuration,
    #[serde(default)]
    pub exit_code:   LintCommandExitCode,
    pub log_file:    String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LintRun {
    pub run_id:        String,
    pub started_at:    String,
    #[serde(default)]
    pub finished_at:   LintRunFinishTime,
    #[serde(default)]
    pub duration_ms:   LintRunDuration,
    pub status:        LintRunStatus,
    pub commands:      Vec<LintCommand>,
    /// Total bytes of this run's archived command logs, summed once when the
    /// run is archived (see [`crate::lint::history::archive_run_output`]) and
    /// persisted on the history line. Reading it back is O(1) — the UI no
    /// longer walks every run's directory at startup to size the list.
    #[serde(default)]
    pub archive_bytes: u64,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn named_fields_preserve_legacy_json_values() -> serde_json::Result<()> {
        let legacy = json!({
            "run_id": "legacy",
            "started_at": "2026-04-24T12:00:00Z",
            "finished_at": "legacy malformed timestamp",
            "duration_ms": u64::MAX,
            "status": "passed",
            "commands": [{
                "name": "check",
                "command": "cargo check",
                "status": "passed",
                "duration_ms": 0,
                "exit_code": i32::MIN,
                "log_file": "check.log"
            }],
            "archive_bytes": 0
        });
        let run: LintRun = serde_json::from_value(legacy.clone())?;
        assert_eq!(
            run.finished_at,
            LintRunFinishTime::Recorded("legacy malformed timestamp".to_string())
        );
        assert_eq!(
            run.duration_ms,
            LintRunDuration::MeasuredMilliseconds(u64::MAX)
        );
        assert_eq!(
            run.commands[0].duration_ms,
            LintCommandDuration::MeasuredMilliseconds(0)
        );
        assert_eq!(
            run.commands[0].exit_code,
            LintCommandExitCode::Recorded(i32::MIN)
        );
        assert_eq!(serde_json::to_value(run)?, legacy);
        Ok(())
    }

    #[test]
    fn missing_and_null_fields_remain_unrecorded() -> serde_json::Result<()> {
        let missing = json!({
            "run_id": "legacy",
            "started_at": "2026-04-24T12:00:00Z",
            "status": "passed",
            "commands": [{
                "name": "check",
                "command": "cargo check",
                "status": "pending",
                "log_file": "check.log"
            }]
        });
        let mut null_fields = missing.clone();
        null_fields["finished_at"] = json!(null);
        null_fields["duration_ms"] = json!(null);
        null_fields["commands"][0]["duration_ms"] = json!(null);
        null_fields["commands"][0]["exit_code"] = json!(null);
        let absent: LintRun = serde_json::from_value(missing)?;
        let explicit: LintRun = serde_json::from_value(null_fields)?;
        assert_eq!(absent, explicit);
        assert_eq!(absent.finished_at, LintRunFinishTime::NotRecorded);
        assert_eq!(absent.duration_ms, LintRunDuration::NotRecorded);
        assert_eq!(
            absent.commands[0].duration_ms,
            LintCommandDuration::NotRecorded
        );
        assert_eq!(
            absent.commands[0].exit_code,
            LintCommandExitCode::NotRecorded
        );
        let serialized = serde_json::to_value(absent)?;
        assert!(serialized["finished_at"].is_null());
        assert!(serialized["duration_ms"].is_null());
        assert!(serialized["commands"][0]["duration_ms"].is_null());
        assert!(serialized["commands"][0]["exit_code"].is_null());
        Ok(())
    }
}
