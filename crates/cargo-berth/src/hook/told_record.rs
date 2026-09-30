//! The notice lines each hook reader has already been told.
//!
//! A `PostToolUse` notice restates every standing alert after every Bash call, so without a
//! record the same lines reach the same reader after each one. A reader is one context the
//! harness publishes hook output into: a session's main agent, or one of its subagents. A
//! subagent's payload carries its parent's `session_id` beside its own `agent_id`, and the
//! subagent keeps a context of its own, so it is told separately from the main agent.
//!
//! Each reader's record lists the lines already stated to it. The record is disposable: a
//! missing, unreadable or foreign record means nothing has been told yet, and a failed write
//! only means a line may be stated again. `SessionStart` forgets the main agent's record,
//! because the context that record describes is gone, and removes every record left unwritten
//! for a week.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;
use std::time::SystemTime;

use serde::Deserialize;
use serde::Serialize;
use uuid::Uuid;

use super::process_binding::HarnessSessionIdentityAvailability;
use crate::ledger::WorktreeContext;
use crate::session;
use crate::session::HarnessSessionId;

/// The prefix every told-record file carries, a temporary one included.
const TOLD_RECORD_FILE_PREFIX: &str = "hook-told-";
/// The suffix every published told-record file carries.
const TOLD_RECORD_FILE_SUFFIX: &str = ".json";
/// How long a record may go unwritten before `SessionStart` removes it.
const TOLD_RECORD_RETENTION: Duration = Duration::from_hours(7 * 24);
/// The FNV-1a 64-bit offset basis the record file name starts from.
const FNV_OFFSET_BASIS: u64 = 14_695_981_039_346_656_037;
/// The FNV-1a 64-bit prime.
const FNV_PRIME: u64 = 1_099_511_628_211;

/// Which agent of one harness session reads a hook response.
enum HookReaderAgent {
    /// The session's main agent, whose payloads carry no `agent_id`.
    Main,
    /// One subagent, named by its payload's `agent_id`.
    Subagent { agent_id: String },
}

/// One context hook responses are published into.
pub(super) struct HookReader {
    harness_session_id: HarnessSessionId,
    agent:              HookReaderAgent,
}

/// What remains of one response's detail once the lines its reader was told are dropped.
pub(super) enum UntoldDetail {
    /// Every line was stated to this reader before.
    NothingNew,
    /// These lines are new to this reader, in the order the response stated them.
    Untold(String),
}

/// The file form of one reader's record.
///
/// The file name is a hash of the reader, so the reader is stored beside its lines: a record
/// whose stored reader differs belongs to another reader and counts as nothing told.
#[derive(Deserialize, Serialize)]
struct ToldRecordFile {
    harness_session_id: String,
    agent_id:           Option<String>,
    lines:              BTreeSet<String>,
}

impl HookReader {
    /// The reader one payload names.
    ///
    /// An `agent_id` is held to the bound a session id is held to. An absent or unusable one
    /// names the main agent, the reader every payload without a usable subagent belongs to.
    pub(super) fn for_payload(
        harness_session_id: HarnessSessionId,
        agent_id: Option<&str>,
    ) -> Self {
        let agent = agent_id
            .filter(|agent_id| session::validate_harness_identifier(agent_id).is_ok())
            .map_or(HookReaderAgent::Main, |agent_id| {
                HookReaderAgent::Subagent {
                    agent_id: agent_id.to_owned(),
                }
            });
        Self {
            harness_session_id,
            agent,
        }
    }

    /// The harness session this reader belongs to.
    pub(super) const fn harness_session_id(&self) -> &HarnessSessionId { &self.harness_session_id }

    /// Drop the lines of `detail` this reader was already told, and record the rest as told.
    ///
    /// A line is the unit because a notice states each standing alert on a line of its own, and
    /// a line whose facts change — a new incident, new commits, new paths — is new text. The
    /// record lives in the ledger directory of the repository this process has entered; with
    /// no such repository there is nothing to record against, and `detail` is stated whole.
    pub(super) fn tell_once(&self, detail: &str) -> UntoldDetail {
        current_ledger_directory().map_or_else(
            || UntoldDetail::Untold(detail.to_owned()),
            |ledger_directory| self.tell_once_in(&ledger_directory, detail),
        )
    }

    fn tell_once_in(&self, ledger_directory: &Path, detail: &str) -> UntoldDetail {
        let stated_lines = detail
            .split('\n')
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>();
        if stated_lines.is_empty() {
            return UntoldDetail::Untold(detail.to_owned());
        }
        let record_path = self.record_path(ledger_directory);
        let mut record = self.read_record(&record_path);
        let untold_lines = stated_lines
            .into_iter()
            .filter(|line| !record.lines.contains(*line))
            .collect::<Vec<_>>();
        if untold_lines.is_empty() {
            return UntoldDetail::NothingNew;
        }
        record
            .lines
            .extend(untold_lines.iter().map(|line| (*line).to_owned()));
        publish_record(&record_path, &record);
        UntoldDetail::Untold(untold_lines.join("\n"))
    }

    fn record_path(&self, ledger_directory: &Path) -> PathBuf {
        let mut identity = self.harness_session_id.as_str().as_bytes().to_vec();
        identity.push(0);
        if let HookReaderAgent::Subagent { agent_id } = &self.agent {
            identity.extend_from_slice(agent_id.as_bytes());
        }
        let fingerprint = identity.iter().fold(FNV_OFFSET_BASIS, |fingerprint, byte| {
            (fingerprint ^ u64::from(*byte)).wrapping_mul(FNV_PRIME)
        });
        ledger_directory.join(format!(
            "{TOLD_RECORD_FILE_PREFIX}{fingerprint:016x}{TOLD_RECORD_FILE_SUFFIX}"
        ))
    }

    /// Read this reader's record, taking a missing, unreadable or foreign one as empty.
    fn read_record(&self, record_path: &Path) -> ToldRecordFile {
        let empty = ToldRecordFile {
            harness_session_id: self.harness_session_id.as_str().to_owned(),
            agent_id:           match &self.agent {
                HookReaderAgent::Main => None,
                HookReaderAgent::Subagent { agent_id } => Some(agent_id.clone()),
            },
            lines:              BTreeSet::new(),
        };
        fs::read(record_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<ToldRecordFile>(&bytes).ok())
            .filter(|stored| {
                stored.harness_session_id == empty.harness_session_id
                    && stored.agent_id == empty.agent_id
            })
            .unwrap_or(empty)
    }
}

/// Forget what an opening session's main agent was told, and remove week-old records.
///
/// `SessionStart` fires for a main agent alone, on startup, resume, clear, compaction and fork;
/// in each case the context the main agent's record describes is not the one it now reads, so
/// the record goes. Subagent records are never reset this way, since no event marks a
/// subagent's context as new; the retention sweep removes them once they go unwritten.
pub(super) fn forget_for_opening_session(availability: &HarnessSessionIdentityAvailability) {
    let Some(ledger_directory) = current_ledger_directory() else {
        return;
    };
    if let HarnessSessionIdentityAvailability::Available(harness_session_id) = availability {
        let main_reader = HookReader {
            harness_session_id: harness_session_id.clone(),
            agent:              HookReaderAgent::Main,
        };
        std::mem::drop(fs::remove_file(main_reader.record_path(&ledger_directory)));
    }
    remove_stale_records(&ledger_directory, SystemTime::now());
}

/// The ledger directory of the repository this process has entered, when there is one.
fn current_ledger_directory() -> Option<PathBuf> {
    std::env::current_dir()
        .ok()
        .and_then(|directory| WorktreeContext::discover(&directory).ok())
        .map(|worktree_context| worktree_context.ledger_directory())
}

/// Replace one record through a temporary file, so a reader never sees half of one.
fn publish_record(record_path: &Path, record: &ToldRecordFile) {
    let (Ok(bytes), Some(file_name)) = (
        serde_json::to_vec(record),
        record_path.file_name().and_then(|name| name.to_str()),
    ) else {
        return;
    };
    let temporary_path = record_path.with_file_name(format!("{file_name}.{}.tmp", Uuid::now_v7()));
    if fs::write(&temporary_path, bytes)
        .and_then(|()| fs::rename(&temporary_path, record_path))
        .is_err()
    {
        std::mem::drop(fs::remove_file(&temporary_path));
    }
}

/// Remove every told-record file, temporary ones included, last written before the retention.
fn remove_stale_records(ledger_directory: &Path, now: SystemTime) {
    let Ok(entries) = fs::read_dir(ledger_directory) else {
        return;
    };
    for entry in entries.flatten() {
        let told_record = entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with(TOLD_RECORD_FILE_PREFIX));
        let stale = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age > TOLD_RECORD_RETENTION);
        if told_record && stale {
            std::mem::drop(fs::remove_file(entry.path()));
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        reason = "tests should panic on unexpected values"
    )]

    use std::fs;
    use std::fs::File;
    use std::time::Duration;
    use std::time::SystemTime;

    use tempfile::TempDir;

    use super::HookReader;
    use super::TOLD_RECORD_RETENTION;
    use super::UntoldDetail;
    use super::remove_stale_records;
    use crate::session::HarnessSessionId;

    fn reader(agent_id: Option<&str>) -> HookReader {
        HookReader::for_payload(
            "told-record-session"
                .parse::<HarnessSessionId>()
                .expect("the session id should be valid"),
            agent_id,
        )
    }

    fn untold(reader: &HookReader, ledger_directory: &TempDir, detail: &str) -> Option<String> {
        match reader.tell_once_in(ledger_directory.path(), detail) {
            UntoldDetail::NothingNew => None,
            UntoldDetail::Untold(untold) => Some(untold),
        }
    }

    #[test]
    fn a_line_is_told_once_and_a_new_line_is_told_alone() {
        let ledger_directory = TempDir::new().expect("a scratch ledger directory");
        let main = reader(None);

        assert_eq!(
            untold(&main, &ledger_directory, "standing\nwidened"),
            Some("standing\nwidened".to_owned())
        );
        assert_eq!(untold(&main, &ledger_directory, "standing\nwidened"), None);
        assert_eq!(
            untold(&main, &ledger_directory, "standing\nchanged"),
            Some("changed".to_owned())
        );
    }

    #[test]
    fn each_agent_of_one_session_is_told_separately() {
        let ledger_directory = TempDir::new().expect("a scratch ledger directory");

        for agent_id in [None, Some("first-subagent"), Some("second-subagent")] {
            let agent = reader(agent_id);
            assert_eq!(
                untold(&agent, &ledger_directory, "standing"),
                Some("standing".to_owned()),
                "{agent_id:?} should be told the line once"
            );
            assert_eq!(untold(&agent, &ledger_directory, "standing"), None);
        }
    }

    #[test]
    fn an_unusable_agent_id_reads_as_the_main_agent() {
        let ledger_directory = TempDir::new().expect("a scratch ledger directory");
        let main = reader(None);
        assert_eq!(
            untold(&main, &ledger_directory, "standing"),
            Some("standing".to_owned())
        );

        for unusable in [
            "",
            "agent\u{0007}",
            &"a".repeat(HarnessSessionId::MAXIMUM_CHARACTERS + 1),
        ] {
            assert_eq!(
                untold(&reader(Some(unusable)), &ledger_directory, "standing"),
                None
            );
        }
    }

    #[test]
    fn a_blank_detail_is_stated_as_it_is() {
        let ledger_directory = TempDir::new().expect("a scratch ledger directory");
        let main = reader(None);

        for _ in 0..2 {
            assert_eq!(untold(&main, &ledger_directory, ""), Some(String::new()));
        }
    }

    #[test]
    fn an_unreadable_record_reads_as_nothing_told() {
        let ledger_directory = TempDir::new().expect("a scratch ledger directory");
        let main = reader(None);
        fs::write(main.record_path(ledger_directory.path()), b"not a record")
            .expect("the damaged record should be written");

        assert_eq!(
            untold(&main, &ledger_directory, "standing"),
            Some("standing".to_owned())
        );
        assert_eq!(untold(&main, &ledger_directory, "standing"), None);
    }

    #[test]
    fn only_told_records_past_the_retention_are_removed() {
        let ledger_directory = TempDir::new().expect("a scratch ledger directory");
        let now = SystemTime::now();
        let week_old = now - TOLD_RECORD_RETENTION - Duration::from_secs(60);
        let paths = [
            ("hook-told-stale.json", week_old, false),
            ("hook-told-stale.json.leftover.tmp", week_old, false),
            ("hook-told-fresh.json", now, true),
            ("journal.ndjson", week_old, true),
        ];
        for (name, modified, _) in paths {
            File::create(ledger_directory.path().join(name))
                .and_then(|file| file.set_modified(modified))
                .expect("the scratch file should take its modification time");
        }

        remove_stale_records(ledger_directory.path(), now);

        for (name, _, kept) in paths {
            assert_eq!(
                ledger_directory.path().join(name).exists(),
                kept,
                "{name} should be kept: {kept}"
            );
        }
    }
}
