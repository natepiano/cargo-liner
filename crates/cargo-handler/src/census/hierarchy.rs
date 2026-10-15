//! Bounded hierarchy commands and their per-session-set cache, run off the UI thread.

use std::collections::BTreeSet;
use std::error::Error;
use std::fmt::Display;
use std::fmt::Formatter;
use std::io::Read;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::process::Child;
use std::process::Command;
use std::process::Stdio;
use std::sync::mpsc;
use std::sync::mpsc::RecvTimeoutError;
use std::thread;
use std::time::Duration;
use std::time::Instant;

use serde::Deserialize;

use super::AgentRow;
use super::HierarchyEdge;
use super::HierarchyEdges;
use super::HierarchyRole;
use super::HierarchySource;
use super::SessionIdentity;
use crate::constants::HIERARCHY_OUTPUT_LIMIT;
use crate::constants::HIERARCHY_REFRESH_INTERVAL;
use crate::constants::HIERARCHY_SCHEMA;
use crate::constants::HIERARCHY_TIMEOUT;
#[cfg(unix)]
use crate::constants::KILL_GROUP_OPTIONS;
#[cfg(unix)]
use crate::constants::KILL_PROGRAM;
use crate::constants::PROBE_EXIT_CHECK;
use crate::constants::SHELL_PROGRAM;
use crate::constants::SHELL_SCRIPT_FLAG;
use crate::startup_log;

/// Why hierarchy discovery could not produce a usable answer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HierarchyError {
    /// The configured shell could not start.
    CouldNotStart,
    /// The command or its output exceeded the deadline.
    TimedOut,
    /// The command exited with a failure status.
    ExitedUnsuccessfully,
    /// Stdout could not be read or was not a hierarchy report.
    OutputUnreadable,
    /// Stdout exceeded the permitted answer size.
    OutputTooLarge,
    /// The command's exit status could not be inspected.
    ExitUnreadable,
    /// The answer uses another hierarchy contract version.
    UnknownSchema(u32),
    /// An edge is missing a required field.
    MalformedEdge,
    /// An edge names an empty manager or member session.
    EmptySessionId,
}

impl Display for HierarchyError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CouldNotStart => formatter.write_str("command could not start"),
            Self::TimedOut => formatter.write_str("command timed out"),
            Self::ExitedUnsuccessfully => formatter.write_str("command failed"),
            Self::OutputUnreadable => formatter.write_str("unreadable hierarchy output"),
            Self::OutputTooLarge => formatter.write_str("hierarchy output too large"),
            Self::ExitUnreadable => formatter.write_str("command exit status unreadable"),
            Self::UnknownSchema(schema) => write!(formatter, "unknown hierarchy schema {schema}"),
            Self::MalformedEdge => formatter.write_str("malformed hierarchy edge"),
            Self::EmptySessionId => formatter.write_str("empty hierarchy session id"),
        }
    }
}

impl Error for HierarchyError {}

/// A source of hierarchy edges, replaceable by fixtures in scheduler tests.
pub(crate) trait HierarchyRunner: Send + Sync {
    /// Run `command`, allowing at most `timeout` for output and exit.
    fn run(&self, command: &str, timeout: Duration) -> Result<HierarchyEdges, HierarchyError>;
}

/// Runs the configured shell command on this machine.
pub(crate) struct CommandRunner;

impl HierarchyRunner for CommandRunner {
    fn run(&self, command: &str, timeout: Duration) -> Result<HierarchyEdges, HierarchyError> {
        let deadline = Instant::now() + timeout;
        let mut process = Command::new(SHELL_PROGRAM);
        process
            .arg(SHELL_SCRIPT_FLAG)
            .arg(command)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .stdout(Stdio::piped());
        // A shell can start children; end the whole group when its deadline expires.
        #[cfg(unix)]
        {
            process.process_group(0);
        }
        let mut child = process.spawn().map_err(|_| HierarchyError::CouldNotStart)?;
        let Some(stdout) = child.stdout.take() else {
            end(&mut child);
            return Err(HierarchyError::OutputUnreadable);
        };
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let _ = sender.send(read_output(stdout));
        });
        let output = match receiver.recv_timeout(deadline.saturating_duration_since(Instant::now()))
        {
            Ok(Ok(output)) => output,
            Ok(Err(error)) => {
                end(&mut child);
                return Err(error);
            },
            Err(error) => {
                end(&mut child);
                return Err(match error {
                    RecvTimeoutError::Timeout => HierarchyError::TimedOut,
                    RecvTimeoutError::Disconnected => HierarchyError::OutputUnreadable,
                });
            },
        };
        loop {
            match child.try_wait() {
                Ok(Some(status)) if status.success() => return parse(&output),
                Ok(Some(_)) => return Err(HierarchyError::ExitedUnsuccessfully),
                Ok(None) if Instant::now() < deadline => thread::sleep(
                    PROBE_EXIT_CHECK.min(deadline.saturating_duration_since(Instant::now())),
                ),
                Ok(None) => {
                    end(&mut child);
                    return Err(HierarchyError::TimedOut);
                },
                Err(_) => {
                    end(&mut child);
                    return Err(HierarchyError::ExitUnreadable);
                },
            }
        }
    }
}

/// Kill the command's descendants as well as the shell, then reap the shell.
fn end(child: &mut Child) {
    #[cfg(unix)]
    {
        // The process group was created above. `kill` accepts a negative group id.
        let _ = Command::new(KILL_PROGRAM)
            .args(KILL_GROUP_OPTIONS)
            .arg(format!("-{}", child.id()))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// Read at most one byte beyond the limit, so oversized output fails without waiting for EOF.
fn read_output(stdout: impl Read) -> Result<Vec<u8>, HierarchyError> {
    let mut bytes = Vec::new();
    stdout
        .take((HIERARCHY_OUTPUT_LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| HierarchyError::OutputUnreadable)?;
    if bytes.len() > HIERARCHY_OUTPUT_LIMIT {
        return Err(HierarchyError::OutputTooLarge);
    }
    Ok(bytes)
}

/// The command output before wire fields become validated domain edges.
#[derive(Deserialize)]
struct HierarchyReport {
    /// Version of the command contract, independent of the probe schema.
    schema: u32,
    /// Each edge is decoded once, including roles a newer command may introduce.
    edges:  Vec<HierarchyWireEdge>,
}

/// Optional wire fields become a named failure at the domain boundary.
#[derive(Deserialize)]
struct HierarchyWireEdge {
    /// Managing Claude session, required for known roles.
    manager: Option<String>,
    /// Local managed Claude session, required for known roles.
    member:  Option<String>,
    /// An absent role fails the answer; a future role discards only its edge.
    role:    Option<HierarchyWireRole>,
}

/// Unknown roles discard only their own edge.
#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum HierarchyWireRole {
    Showrunner,
    Unit,
    Impl,
    Test,
    Review,
    Fix,
    #[serde(other)]
    Unknown,
}

/// Decode a command answer, rejecting a foreign schema or malformed JSON.
fn parse(output: &[u8]) -> Result<HierarchyEdges, HierarchyError> {
    let report: HierarchyReport =
        serde_json::from_slice(output).map_err(|_| HierarchyError::OutputUnreadable)?;
    if report.schema != HIERARCHY_SCHEMA {
        return Err(HierarchyError::UnknownSchema(report.schema));
    }
    report
        .edges
        .into_iter()
        .filter(|edge| !matches!(edge.role, Some(HierarchyWireRole::Unknown)))
        .map(|edge| {
            let manager = edge.manager.ok_or(HierarchyError::MalformedEdge)?;
            let member = edge.member.ok_or(HierarchyError::MalformedEdge)?;
            let role = match edge.role {
                Some(HierarchyWireRole::Showrunner) => HierarchyRole::Showrunner,
                Some(HierarchyWireRole::Unit) => HierarchyRole::Unit,
                Some(HierarchyWireRole::Impl) => HierarchyRole::Impl,
                Some(HierarchyWireRole::Test) => HierarchyRole::Test,
                Some(HierarchyWireRole::Review) => HierarchyRole::Review,
                Some(HierarchyWireRole::Fix) => HierarchyRole::Fix,
                None | Some(HierarchyWireRole::Unknown) => {
                    return Err(HierarchyError::MalformedEdge);
                },
            };
            if manager.is_empty() || member.is_empty() {
                return Err(HierarchyError::EmptySessionId);
            }
            Ok(HierarchyEdge {
                manager,
                member,
                role,
            })
        })
        .collect::<Result<Vec<_>, _>>()
        .map(HierarchyEdges)
}

/// Whether discovery answered or must preserve the last successful links.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum HierarchyAnswer {
    /// A successful command, or discovery explicitly turned off.
    Answered(HierarchyEdges),
    /// Discovery failed; absence of edges is not an authoritative answer.
    Failed,
}

/// Run one configured discovery, logging a single line for each failed command.
pub(super) fn read(source: &HierarchySource, runner: &dyn HierarchyRunner) -> HierarchyAnswer {
    let HierarchySource::Command(command) = source else {
        return HierarchyAnswer::Answered(HierarchyEdges::default());
    };
    match runner.run(command, HIERARCHY_TIMEOUT) {
        Ok(edges) => HierarchyAnswer::Answered(edges),
        Err(reason) => {
            startup_log::startup_event(&format!("hierarchy_failed {reason}"));
            HierarchyAnswer::Failed
        },
    }
}

/// The last success for one source, retained across unsuccessful attempts.
#[derive(Default)]
pub(super) enum LastSuccessfulHierarchy {
    /// This source has never answered successfully.
    #[default]
    Unanswered,
    /// The last authoritative links and the time they were obtained.
    Answered {
        /// When discovery successfully answered.
        refreshed: Instant,
        /// Links from that successful answer.
        edges:     HierarchyEdges,
    },
}

impl LastSuccessfulHierarchy {
    /// Links usable while the next attempt is running or has failed.
    pub(super) fn edges(&self) -> HierarchyEdges {
        match self {
            Self::Unanswered => HierarchyEdges::default(),
            Self::Answered { edges, .. } => edges.clone(),
        }
    }
}

/// Whether the next scheduler tick must retry discovery.
#[derive(Default)]
enum HierarchyRetry {
    /// A successful answer may be reused until its normal expiration.
    #[default]
    OnExpiration,
    /// The latest attempt failed; retry on the next tick.
    NextTick,
}

/// Edges reused until their source, local sessions, or freshness changes.
#[derive(Default)]
pub(super) struct HierarchyCache {
    /// Source to which the retained success belongs.
    source:   HierarchySource,
    /// Local Claude session ids, independent of row order and process ids.
    sessions: BTreeSet<String>,
    /// Last successful answer from this source, never a failed empty answer.
    success:  LastSuccessfulHierarchy,
    /// Failure invalidates the ordinary refresh interval.
    retry:    HierarchyRetry,
}

impl HierarchyCache {
    /// Cached links, cleared immediately when the setting changes or turns off.
    pub(super) fn cached(&mut self, source: HierarchySource) -> &LastSuccessfulHierarchy {
        if self.source != source || matches!(source, HierarchySource::Off) {
            self.source = source;
            self.success = LastSuccessfulHierarchy::Unanswered;
            self.retry = HierarchyRetry::OnExpiration;
        }
        &self.success
    }

    /// Refresh expired links; unsuccessful attempts retain success and retry next tick.
    pub(super) fn edges(
        &mut self,
        source: HierarchySource,
        rows: &[AgentRow],
        runner: &dyn HierarchyRunner,
        now: Instant,
    ) -> HierarchyEdges {
        self.cached(source);
        if matches!(self.source, HierarchySource::Off) {
            return HierarchyEdges::default();
        }
        let sessions: BTreeSet<_> = rows
            .iter()
            .filter_map(|row| match &row.session_id {
                SessionIdentity::Claude(id) => Some(id.clone()),
                SessionIdentity::Unrecorded => None,
            })
            .collect();
        if let LastSuccessfulHierarchy::Answered { refreshed, edges } = &self.success
            && matches!(self.retry, HierarchyRetry::OnExpiration)
            && self.sessions == sessions
            && now.saturating_duration_since(*refreshed) < HIERARCHY_REFRESH_INTERVAL
        {
            return edges.clone();
        }
        self.sessions = sessions;
        match read(&self.source, runner) {
            HierarchyAnswer::Answered(edges) => {
                self.success = LastSuccessfulHierarchy::Answered {
                    refreshed: now,
                    edges,
                };
                self.retry = HierarchyRetry::OnExpiration;
            },
            HierarchyAnswer::Failed => self.retry = HierarchyRetry::NextTick,
        }
        self.success.edges()
    }
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use super::*;

    /// A valid edge plus a role introduced by a future command implementation.
    #[test]
    fn future_roles_do_not_hide_known_edges_and_foreign_schemas_fail() {
        let output = br#"{"schema":1,"edges":[{"manager":"boss","member":"worker","role":"impl"},{"manager":"boss","member":"future","role":"future"}]}"#;
        assert_eq!(
            parse(output),
            Ok(HierarchyEdges(vec![HierarchyEdge {
                manager: "boss".to_string(),
                member:  "worker".to_string(),
                role:    HierarchyRole::Impl,
            }]))
        );
        assert!(parse(br#"{"schema":2,"edges":[]}"#).is_err());
        assert!(parse(br#"{"schema":1,"edges":[{"role":"impl"}]}"#).is_err());
    }

    /// Keep error categories distinct for failed commands and malformed contracts.
    #[test]
    fn command_and_contract_failures_keep_their_named_reasons() {
        for (output, reason) in [
            (b"not json".as_slice(), HierarchyError::OutputUnreadable),
            (
                br#"{"schema":2,"edges":[]}"#,
                HierarchyError::UnknownSchema(2),
            ),
            (
                br#"{"schema":1,"edges":[{"role":"impl"}]}"#,
                HierarchyError::MalformedEdge,
            ),
            (
                br#"{"schema":1,"edges":[{"manager":"boss","member":"worker"}]}"#,
                HierarchyError::MalformedEdge,
            ),
            (
                br#"{"schema":1,"edges":[{"manager":"","member":"worker","role":"impl"}]}"#,
                HierarchyError::EmptySessionId,
            ),
            (
                br#"{"schema":1,"edges":[{"manager":"boss","member":"","role":"impl"}]}"#,
                HierarchyError::EmptySessionId,
            ),
        ] {
            assert_eq!(parse(output), Err(reason));
        }
        assert_eq!(
            CommandRunner.run("exit 7", Duration::from_millis(150)),
            Err(HierarchyError::ExitedUnsuccessfully)
        );
        assert_eq!(
            CommandRunner.run("exec sleep 10", Duration::from_millis(100)),
            Err(HierarchyError::TimedOut)
        );
    }

    /// An endless producer fails at the output limit and its process is reaped promptly.
    #[test]
    fn oversized_output_fails_before_the_deadline_and_reaps_the_command() {
        let temp = tempfile::tempdir().expect("a fixture directory should open");
        let pid_file = temp.path().join("pid");
        let command = format!("echo $$ > '{}'; exec yes", pid_file.display());
        let started = Instant::now();
        assert_eq!(
            CommandRunner.run(&command, Duration::from_millis(500)),
            Err(HierarchyError::OutputTooLarge)
        );
        assert!(started.elapsed() < Duration::from_millis(800));
        let pid = std::fs::read_to_string(pid_file).expect("the command should record its pid");
        let status = Command::new("kill")
            .args(["-0", pid.trim()])
            .stderr(Stdio::null())
            .status()
            .expect("kill should start");
        assert!(
            !status.success(),
            "the overflowing command must be killed and reaped"
        );
    }

    /// A full-size valid answer is accepted; even one byte more is rejected.
    #[test]
    fn output_limit_accepts_a_complete_answer_at_the_boundary() {
        let mut output =
            br#"{"schema":1,"edges":[{"manager":"boss","member":"worker","role":"review"}]}"#
                .to_vec();
        output.resize(HIERARCHY_OUTPUT_LIMIT, b' ');
        let bytes = read_output(output.as_slice()).expect("an answer at the limit should be read");
        assert_eq!(
            parse(&bytes),
            Ok(HierarchyEdges(vec![HierarchyEdge {
                manager: "boss".to_string(),
                member:  "worker".to_string(),
                role:    HierarchyRole::Review,
            }]))
        );
        output.push(b' ');
        assert_eq!(
            read_output(output.as_slice()),
            Err(HierarchyError::OutputTooLarge)
        );
    }

    /// Records refreshes; one setting deliberately fails after a previous success.
    struct RecordingRunner(Mutex<Vec<String>>);

    impl HierarchyRunner for RecordingRunner {
        fn run(&self, command: &str, _: Duration) -> Result<HierarchyEdges, HierarchyError> {
            self.0
                .lock()
                .expect("calls should lock")
                .push(command.to_string());
            if command == "fail" {
                return Err(HierarchyError::ExitedUnsuccessfully);
            }
            Ok(HierarchyEdges(vec![HierarchyEdge {
                manager: command.to_string(),
                member:  "worker".to_string(),
                role:    HierarchyRole::Impl,
            }]))
        }
    }

    /// Failures preserve links for an unchanged source and retry on the very next tick.
    #[test]
    fn hierarchy_failures_keep_success_and_retry_next_tick_until_disabled() {
        struct ScriptedRunner(Mutex<VecDeque<Result<HierarchyEdges, HierarchyError>>>);
        impl HierarchyRunner for ScriptedRunner {
            fn run(&self, _: &str, _: Duration) -> Result<HierarchyEdges, HierarchyError> {
                self.0
                    .lock()
                    .expect("answer queue")
                    .pop_front()
                    .expect("unexpected extra discovery")
            }
        }
        let first = HierarchyEdges(vec![HierarchyEdge {
            manager: "boss".to_string(),
            member:  "worker".to_string(),
            role:    HierarchyRole::Unit,
        }]);
        let replacement = HierarchyEdges(vec![HierarchyEdge {
            manager: "new-boss".to_string(),
            ..first.0[0].clone()
        }]);
        let runner = ScriptedRunner(Mutex::new(std::collections::VecDeque::from([
            Ok(first.clone()),
            Err(HierarchyError::TimedOut),
            Err(HierarchyError::ExitedUnsuccessfully),
            Ok(replacement.clone()),
        ])));
        let source = HierarchySource::Command("unchanged".to_string());
        let mut cache = HierarchyCache::default();
        let start = Instant::now();
        assert_eq!(cache.edges(source.clone(), &[], &runner, start), first);
        let expired = start + HIERARCHY_REFRESH_INTERVAL;
        assert_eq!(cache.edges(source.clone(), &[], &runner, expired), first);
        assert_eq!(
            cache.edges(
                source.clone(),
                &[],
                &runner,
                expired + Duration::from_millis(1)
            ),
            first
        );
        assert_eq!(
            cache.edges(
                source.clone(),
                &[],
                &runner,
                expired + Duration::from_millis(2)
            ),
            replacement
        );
        assert_eq!(
            cache.edges(source, &[], &runner, expired + Duration::from_millis(3)),
            replacement
        );
        assert!(runner.0.lock().expect("answer queue").is_empty());
        assert_eq!(
            cache.edges(HierarchySource::Off, &[], &runner, expired),
            HierarchyEdges::default()
        );
    }

    /// Time, identities and configuration invalidate cached success and failure alike.
    #[test]
    fn hierarchy_cache_expires_and_configuration_changes_clear_previous_success() {
        let runner = RecordingRunner(Mutex::new(Vec::new()));
        let mut cache = HierarchyCache::default();
        let now = Instant::now();
        let source = HierarchySource::Command("good".to_string());
        let initial = cache.edges(source.clone(), &[], &runner, now);
        assert_eq!(
            cache.edges(source.clone(), &[], &runner, now + Duration::from_secs(29)),
            initial
        );
        assert_eq!(runner.0.lock().expect("calls should lock").len(), 1);
        assert_eq!(
            cache.edges(source, &[], &runner, now + HIERARCHY_REFRESH_INTERVAL),
            initial
        );
        assert_eq!(runner.0.lock().expect("calls should lock").len(), 2);
        assert_eq!(
            cache.edges(
                HierarchySource::Command("fail".to_string()),
                &[],
                &runner,
                now
            ),
            HierarchyEdges::default()
        );
        assert_eq!(
            cache.edges(
                HierarchySource::Command("fail".to_string()),
                &[],
                &runner,
                now
            ),
            HierarchyEdges::default()
        );
        assert_eq!(runner.0.lock().expect("calls should lock").len(), 4);
        assert_eq!(
            cache.edges(HierarchySource::Off, &[], &runner, now),
            HierarchyEdges::default()
        );
        assert_eq!(runner.0.lock().expect("calls should lock").len(), 4);
        assert_eq!(
            cache.edges(
                HierarchySource::Command("good".to_string()),
                &[],
                &runner,
                now
            ),
            initial
        );
        assert_eq!(runner.0.lock().expect("calls should lock").len(), 5);
    }
}
