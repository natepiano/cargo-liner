//! Reading this machine: the session records Claude Code writes under
//! `~/.claude/sessions`, the process table, the threads interactive
//! Codex sessions started, and the transcripts that name what each
//! agent is running and which agent opened a session tmux holds, handed
//! to [`classify`] and [`tree`]; and the windows `KWin` lists with the
//! panes and clients of tmux, handed to [`desktop`].

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Instant;
use std::time::SystemTime;

use crate::startup_log;

#[cfg(test)]
#[path = "scan/staged_tests.rs"]
mod staged_tests;

use std::ffi::OsStr;
#[cfg(target_os = "linux")]
use std::io::ErrorKind;
#[cfg(target_os = "linux")]
use std::os::unix::ffi::OsStringExt;
#[cfg(test)]
use std::time::Duration;

#[cfg(target_os = "macos")]
use libproc::proc_pid::PIDInfo;
#[cfg(target_os = "macos")]
use libproc::proc_pid::PidInfoFlavor;
use sysinfo::Pid;
use sysinfo::Process;
use sysinfo::ProcessRefreshKind;
use sysinfo::ProcessesToUpdate;
use sysinfo::System;
use sysinfo::UpdateKind;

use super::Agent;
use super::AgentActivity;
use super::AgentRow;
use super::AstraSessionMembership;
use super::ChildKind;
use super::ChildRow;
use super::CodexThreadId;
use super::Observed;
use super::PassFact;
use super::ProvenCodexThread;
use super::ServiceTier;
use super::SessionIdentity;
use super::SourceHealth;
use super::SourceRead;
use super::WorkIdentity;
use super::account;
use super::account::AccountLabels;
use super::account::ConfigHome;
use super::activity;
use super::activity::ActivityRead;
use super::activity::DetachedRunState;
use super::activity::LiveWorkerRun;
use super::activity::RunDirector;
use super::activity::RunDirectory;
use super::activity::RunOwnership;
use super::activity::RunRecord;
use super::activity::RunRecords;
use super::activity::RunSeat;
use super::activity::RunWorkerIdentity;
use super::activity::TranscriptRead;
use super::activity::VerificationProcesses;
use super::activity::WorkerRun;
use super::branch;
use super::classify;
use super::classify::CodexSessionLink;
use super::classify::HeldSession;
use super::classify::ProcessEntry;
use super::classify::SessionRecord;
use super::codex;
use super::codex::CodexLogs;
use super::codex::CodexThread;
use super::codex::OpenThreadRows;
use super::codex::ResumeRequest;
use super::codex_names::SessionName;
use super::codex_names::SessionNames;
use super::compaction_markers::CompactionFamily;
use super::compaction_markers::CompactionMarkerStatus;
use super::compaction_markers::CompactionMarkers;
use super::compaction_markers::CompactionTranscript;
use super::desktop;
use super::desktop::TmuxLayout;
use super::desktop::WindowEntry;
use super::herdr::HerdrNames;
use super::hierarchy::AstraWorkerMembership;
use super::hierarchy::FileRevision;
use super::hierarchy::StaffFileReader;
use super::hierarchy::StaffMemberLookup;
use super::hierarchy::StaffRoster;
use super::hierarchy::StaffRunPointer;
use super::hierarchy::StaffSnapshot;
use super::hierarchy::WorkerReference;
use super::model::ConversationModel;
use super::tier;
use super::tier::PacerHistory;
use super::tier::Rollouts;
use super::tier::ThreadRollout;
use super::transcript;
use super::transcript::BashCall;
use super::transcript::Subagent;
use super::transcript::SubagentReads;
use super::transcript::TranscriptModels;
use super::tree;
use super::tree::RunThread;
use super::tree::ShellCall;
use super::tree::ThreadEntry;
use super::tree::TreeSources;
use super::unix_now;
#[cfg(test)]
use super::worker_dispatch;
use super::worker_dispatch::ManagedSession;
use super::worker_dispatch::NotWorkerDispatch;
use super::worker_dispatch::ParsedDispatch;
use super::worker_dispatch::PassState;
use super::worker_dispatch::ResolvedWorkerDispatch;
use super::worker_dispatch::RunSelection;
use super::worker_dispatch::SessionRoots;
use super::worker_dispatch::WorkerDispatch;
use super::worker_dispatch::WorkerIdentity;
use super::worker_dispatch::WorkerNames;
use super::worker_dispatch::WorkerPass;
use super::worker_dispatch::WorkerRoleSource;
use super::worker_dispatch::WorkerTask;
use crate::config::CodexStartWindow;
use crate::config::HerdrProbeWait;
use crate::config::HerdrReadTimeout;
use crate::constants::ACTIVITY_PROCESS_NAMES;
use crate::constants::CALL_SEARCH_SETTLE;
use crate::constants::CLAUDE_AGENT;
use crate::constants::CLAUDE_DIRNAME;
use crate::constants::CLAUDE_PID_VARIABLE;
use crate::constants::CLAUDE_SESSIONS_DIRNAME;
use crate::constants::CODEX_AGENT;
use crate::constants::CODEX_MESH_SCRIPT;
use crate::constants::CODEX_PACER_STATE;
use crate::constants::CODEX_SESSIONS_DIRNAME;
use crate::constants::DESKTOP_QUERY_INTERVAL;
use crate::constants::MESH_SEAT_FLAGS;
use crate::constants::PROC_DIRNAME;
use crate::constants::PROC_FD_DIRNAME;
use crate::constants::PROCESS_RECOVERY_INTERVAL;
use crate::constants::PROGRESS_HISTORY_STATE_FILE;
use crate::constants::PROJECTS_DIRNAME;
use crate::constants::SESSION_RECORD_EXTENSION;
use crate::constants::TMUX_LIST_CLIENTS;
use crate::constants::TMUX_LIST_PANES;
use crate::constants::VERIFY_SCRIPT;
#[cfg(target_os = "linux")]
use crate::constants::WINDOW_LIST_EXPRESSION;
use crate::constants::WORKER_LABEL;

/// Run evidence read once before the tree and activity stages consume it.
struct WorkerRuns {
    sessions:   HashSet<PathBuf>,
    threads:    HashMap<u32, Vec<RunThread>>,
    activities: HashMap<String, WorkerRun>,
    named:      HashMap<u32, Vec<ChildRow>>,
    folded:     HashSet<u32>,
}

/// One scan joins run records and open passes through the staff pointer.
struct CurrentRunObservations<'a> {
    staff:     &'a StaffRoster,
    processes: &'a [ProcessEntry],
    records:   &'a HashMap<PathBuf, Vec<RunRecord>>,
    health:    &'a HashMap<PathBuf, super::SourceHealth>,
    passes:    HashMap<PathBuf, super::SourceRead<Vec<WorkerPass>>>,
}

impl CurrentRunObservations<'_> {
    fn worker(&self, session: &str) -> super::WorkerFacts {
        let current_run = self.staff.worker_run(session);
        let mut facts = super::WorkerFacts {
            listing:          Observed::Known(self.staff.listing(session)),
            current_run:      current_run.clone(),
            pane:             Observed::unknown("no readable pane observation for this worker"),
            runs:             Observed::unknown("worker has no readable current run"),
            activity_started: Observed::unknown("no running detached run or open pass start"),
            pass:             Observed::unknown("worker has no readable current run"),
        };
        let Observed::Known(StaffRunPointer::Current(run)) = current_run else {
            if matches!(current_run, Observed::Known(StaffRunPointer::NoRun)) {
                facts.runs = Observed::Known(Vec::new());
                facts.pass = Observed::Known(PassFact::Closed);
            }
            return facts;
        };
        let records = self.records.get(run.path()).into_iter().flatten();
        let mut runs = Vec::new();
        for record in records {
            if record.ownership(session, self.staff) == RunOwnership::Other {
                continue;
            }
            if let Observed::Known(fact) = record.fact(self.processes) {
                if fact.state == DetachedRunState::Running
                    && let Observed::Known(stamp) = fact.started
                    && !matches!(facts.activity_started, Observed::Known(old) if old >= stamp)
                {
                    facts.activity_started = Observed::Known(stamp);
                }
                runs.push(fact);
            }
        }
        runs.sort_by(|left, right| left.id.cmp(&right.id));
        facts.runs = match self.health.get(run.path()) {
            Some(SourceHealth::Unreadable(reason)) => Observed::Unknown(reason.clone()),
            Some(SourceHealth::Loaded | SourceHealth::Absent) => Observed::Known(runs),
            None => Observed::unknown("current run records were not read"),
        };
        let pass = self.open_pass(run.path(), session);
        facts.pass = match &pass {
            Observed::Known(PassState::Open {
                assignment: Observed::Known(assignment),
                ..
            }) => Observed::Known(PassFact::Open(assignment.clone())),
            Observed::Known(PassState::Open {
                assignment: Observed::Unknown(reason),
                ..
            })
            | Observed::Unknown(reason) => Observed::Unknown(reason.clone()),
            Observed::Known(PassState::Closed) => Observed::Known(PassFact::Closed),
        };
        if matches!(facts.activity_started, Observed::Unknown(_)) {
            facts.activity_started = match pass {
                Observed::Known(PassState::Open { started, .. }) => started,
                Observed::Known(PassState::Closed) => Observed::unknown("worker has no open pass"),
                Observed::Unknown(reason) => Observed::Unknown(reason),
            };
        }
        facts
    }

    /// The worker's latest open pass in the current run, or the reason the pass map is unread.
    fn open_pass(&self, directory: &Path, session: &str) -> Observed<PassState> {
        match self.passes.get(directory) {
            Some(SourceRead::Loaded(passes)) => Observed::Known(
                passes
                    .iter()
                    .filter(|pass| {
                        matches!(pass.state, PassState::Open { .. })
                            && WorkerReference::Staff {
                                staff_id: pass.worker.clone(),
                            }
                            .session(self.staff)
                                == Some(session)
                    })
                    .max_by_key(|pass| match pass.state {
                        PassState::Open {
                            started: Observed::Known(stamp),
                            ..
                        } => Some(stamp),
                        PassState::Open { .. } | PassState::Closed => None,
                    })
                    .map_or(PassState::Closed, |pass| pass.state.clone()),
            ),
            Some(SourceRead::Unreadable(reason)) => Observed::Unknown(reason.clone()),
            Some(SourceRead::Absent) | None => Observed::unknown("current run has no pass source"),
        }
    }
}

/// `WorkerProcess` keeps a worker's process identity, or names that no process was found.
#[derive(Clone, Copy)]
enum WorkerProcess {
    Running { pid: u32, started: u64 },
    Absent,
}

/// `RunManager` distinguishes a live unit director from an unlocated run.
#[derive(Clone, Copy)]
enum RunManager {
    Director(u32),
    Unlocated,
}

/// Unchanged pass revisions reuse the same filtered staff roster.
#[derive(Debug, Default)]
enum CurrentStaff {
    #[default]
    Unread,
    Selected {
        source:    Arc<StaffRoster>,
        revisions: Vec<(PathBuf, FileRevision)>,
        roster:    Arc<StaffRoster>,
    },
}

impl CurrentStaff {
    fn select(&mut self, source: Arc<StaffRoster>) -> Arc<StaffRoster> {
        let revisions: Vec<_> = source
            .current_runs()
            .map(|(_, run)| {
                let path = run.path().join(PROGRESS_HISTORY_STATE_FILE);
                let revision = fs::metadata(&path).map_or(FileRevision::Missing, |metadata| {
                    super::hierarchy::FileRevision::of(&metadata)
                });
                (path, revision)
            })
            .collect();
        if let Self::Selected {
            source: previous,
            revisions: old,
            roster,
        } = self
            && Arc::ptr_eq(previous, &source)
            && old == &revisions
        {
            return Arc::clone(roster);
        }
        let selected = source.for_current_runs();
        let roster = if selected.session_ids().count() == source.session_ids().count() {
            Arc::clone(&source)
        } else {
            Arc::new(selected)
        };
        *self = Self::Selected {
            source,
            revisions,
            roster: Arc::clone(&roster),
        };
        roster
    }
}

/// `ResolvedWorkerRun` shares one worker identity across all scan consumers.
struct ResolvedWorkerRun<'a> {
    run:     &'a LiveWorkerRun<'a>,
    worker:  RunWorkerIdentity<'a>,
    manager: RunManager,
}

fn resolved_worker_runs<'a>(
    live: &'a [LiveWorkerRun<'a>],
    sessions: &[ManagedSession],
    directors: &BTreeMap<&str, u32>,
    staff: &'a StaffRoster,
) -> Vec<ResolvedWorkerRun<'a>> {
    let units: HashMap<_, _> = sessions
        .iter()
        .map(|session| (session.directory.as_path(), session.director_pid))
        .collect();
    let identities: HashMap<_, _> = directors.iter().map(|(id, pid)| (*pid, *id)).collect();
    live.iter()
        .map(|run| {
            let manager = match run.record.directory() {
                RunDirectory::Unit(path) => units
                    .get(path)
                    .copied()
                    .map_or(RunManager::Unlocated, RunManager::Director),
                RunDirectory::Unlocated => RunManager::Unlocated,
            };
            let director = match manager {
                RunManager::Director(pid) => identities
                    .get(&pid)
                    .map_or(RunDirector::Unrecorded, |id| RunDirector::Session(id)),
                RunManager::Unlocated => RunDirector::Unrecorded,
            };
            ResolvedWorkerRun {
                run,
                worker: run.worker(staff, director),
                manager,
            }
        })
        .collect()
}

fn run_activities(sources: &[ResolvedWorkerRun<'_>]) -> HashMap<String, WorkerRun> {
    let mut latest: HashMap<&str, &LiveWorkerRun<'_>> = HashMap::new();
    for source in sources {
        if let RunWorkerIdentity::Session(id) = source.worker {
            let current = latest.entry(id).or_insert(source.run);
            if source.run.newer_than(current) {
                *current = source.run;
            }
        }
    }
    latest
        .into_iter()
        .map(|(id, run)| (id.to_owned(), run.activity()))
        .collect()
}

fn run_directors(
    live: &[LiveWorkerRun<'_>],
    staff: &StaffRoster,
    directors: &BTreeMap<&str, u32>,
) -> HashMap<PathBuf, HashSet<u32>> {
    let mut owners: HashMap<PathBuf, HashSet<u32>> = HashMap::new();
    for run in live {
        if let (RunDirectory::Unit(directory), RunDirector::Session(id)) =
            (run.record.directory(), run.record.director(staff))
            && let Some(pid) = directors.get(id)
        {
            owners.entry(directory.to_owned()).or_default().insert(*pid);
        }
    }
    owners
}

/// Shell argv flag whose next argument contains the command text.
const SHELL_COMMAND_FLAG: &str = "-c";

/// A process as a cache knows it: its pid and start, so a pid handed
/// out again is a new key.
type ProcessKey = (u32, u64);

/// Scans this machine for its agents and what each is running.
#[derive(Debug)]
pub(super) struct LocalScanner {
    processes:           ProcessCensus,
    run_records:         RunRecords,
    /// Whether a completed answer has replaced the startup classification.
    publication:         LocalPublication,
    /// This machine's home directory: where the session records live,
    /// and what a row's directory is written against.
    home:                Option<PathBuf>,
    /// Account labels resolved once from this machine's startup configuration.
    account_labels:      AccountLabels,
    /// The call found for each shell wrapper, or none found once the
    /// wrapper was [`CALL_SEARCH_SETTLE`] old.
    shell_calls:         HashMap<ProcessKey, Option<ShellCall>>,
    /// The launcher found for each session tmux holds, by the launcher's
    /// pid and start, or none found once the session was
    /// [`CALL_SEARCH_SETTLE`] old.
    launchers:           HashMap<ProcessKey, Option<ProcessKey>>,
    /// Where each session's transcript was found, by session id.
    transcript_paths:    HashMap<String, PathBuf>,
    /// When the session each transcript holds began, by the
    /// transcript's path, or none where it holds no stamped line yet.
    session_starts:      HashMap<PathBuf, Option<u64>>,
    /// Each Codex rollout read for its tier, as far as it was read.
    rollouts:            Rollouts,
    session_names:       SessionNames,
    open_thread_rows:    OpenThreadRows,
    /// Each Codex account log advances from its retained row id.
    codex_logs:          CodexLogs,
    herdr:               HerdrNames,
    codex_links:         HashMap<u32, CodexSessionLink>,
    staff:               super::hierarchy::StaffDiscovery,
    codex_start_window:  CodexStartWindow,
    herdr_probe_wait:    HerdrProbeWait,
    transcript_activity: HashMap<PathBuf, TranscriptRead>,
    compactions:         CompactionMarkers,
    transcript_models:   TranscriptModels,
    subagent_activity:   HashMap<PathBuf, TranscriptRead>,
    subagent_reads:      SubagentReads,
    worker_names:        WorkerNames,
    worker_commands:     WorkerCommands,
    session_roots:       SessionRoots,
    worker_facts:        BTreeMap<String, super::WorkerFacts>,
    activity_starts:     HashMap<String, ActivityRead>,
    current_staff:       CurrentStaff,
    /// Where desktop information comes from for this scanner.
    desktop_source:      DesktopSource,
    /// What `KWin` and tmux last answered, kept for
    /// [`DESKTOP_QUERY_INTERVAL`]; none before the first scan with rows.
    desktops:            Option<DesktopRead>,
}

/// Live desktop queries or windows supplied by an isolated scanner fixture.
#[derive(Debug)]
enum DesktopSource {
    System,
    #[cfg(test)]
    Windows(Vec<WindowEntry>),
}

/// What `KWin` and tmux answered at one moment.
#[derive(Debug)]
struct DesktopRead {
    /// When they were asked.
    read_at: Instant,
    /// Every window `KWin` listed; none where it did not answer, as off
    /// Linux or outside KDE.
    windows: Option<Vec<WindowEntry>>,
    /// The default tmux server's panes and clients; none where tmux held
    /// no row, `KWin` did not answer, or tmux did not.
    tmux:    Option<TmuxLayout>,
}

impl DesktopRead {
    /// Ask `KWin` for its windows now and, when it answers and tmux holds
    /// one of `rows`, tmux for its panes and clients.
    fn now(processes: &[ProcessEntry], rows: &[AgentRow]) -> Self {
        let windows = read_windows();
        let tmux = windows
            .as_ref()
            .and_then(|_| desktop::tmux_program(processes, rows))
            .and_then(|program| read_tmux(&program));
        Self {
            read_at: Instant::now(),
            windows,
            tmux,
        }
    }

    /// Whether this answer is old enough to be asked for again.
    fn is_stale(&self) -> bool { self.read_at.elapsed() >= DESKTOP_QUERY_INTERVAL }
}

/// Whether the first completed local answer has been published.
#[derive(Clone, Copy, Debug)]
enum LocalPublication {
    FirstAnswerPending,
    CompletedAnswerPublished,
}

/// Which work is complete when a local scan publishes its rows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ScanStage {
    /// Ready for immediate display, before detail reads or hierarchy discovery.
    Classified,
    /// All local row detail sources have been read.
    Completed,
}

impl LocalScanner {
    #[cfg(test)]
    pub(super) fn scan_details_for_test(
        &mut self,
        processes: &[ProcessEntry],
        sessions: &[SessionRecord],
        home: &Path,
    ) -> Vec<AgentRow> {
        self.desktop_source = DesktopSource::Windows(Vec::new());
        self.compactions
            .set_directory(home.join(".local/state/compaction"));
        self.codex_logs.refresh(processes, Some(home));
        let mut answer = Vec::new();
        self.scan_stages(
            processes,
            sessions,
            Some(home),
            |rows| answer = rows,
            |scanner, rows| scanner.attach_details(processes, sessions, Some(home), rows),
        );
        answer
    }

    /// Adapt existing row-only fixtures to the staged scan implementation.
    #[cfg(test)]
    pub(super) fn scan_stages(
        &mut self,
        processes: &[ProcessEntry],
        sessions: &[SessionRecord],
        home: Option<&Path>,
        mut publish: impl FnMut(Vec<AgentRow>),
        read_details: impl FnOnce(&mut Self, &mut Vec<AgentRow>),
    ) {
        self.scan_stages_with(
            processes,
            sessions,
            home,
            |_, rows| publish(rows),
            read_details,
        );
    }

    #[cfg(test)]
    fn scan_stages_with(
        &mut self,
        processes: &[ProcessEntry],
        sessions: &[SessionRecord],
        home: Option<&Path>,
        mut publish: impl FnMut(ScanStage, Vec<AgentRow>),
        read_details: impl FnOnce(&mut Self, &mut Vec<AgentRow>),
    ) {
        self.scan_with_staff(
            processes,
            sessions,
            home,
            |stage, rows, _| publish(stage, rows),
            read_details,
        );
    }

    /// Classify synthetic or live inputs, publishing early only on startup.
    fn scan_with_staff(
        &mut self,
        processes: &[ProcessEntry],
        sessions: &[SessionRecord],
        home: Option<&Path>,
        mut publish: impl FnMut(ScanStage, Vec<AgentRow>, &StaffSnapshot),
        read_details: impl FnOnce(&mut Self, &mut Vec<AgentRow>),
    ) -> StaffSnapshot {
        self.resolve_codex_sessions(processes, home);
        let mut staff = startup_log::startup_stage("staff", || self.staff.snapshot());
        staff.roster = self.current_staff.select(staff.roster);
        self.worker_names.set_staff(Arc::clone(&staff.roster));
        let mut rows = startup_log::startup_stage("classify_agents", || {
            let threads = classify::linked_threads(&self.codex_links);
            classify::agent_rows_with_threads(
                processes,
                sessions,
                &threads,
                home,
                &self.account_labels,
            )
        });
        include_astra_processes(
            &mut rows,
            processes,
            home,
            &staff.roster,
            &self.account_labels,
        );
        attach_pane_codex_identities(&mut rows, processes, home, &staff.roster);
        attach_astra_memberships(&mut rows, &staff.roster, processes, home, &self.herdr);
        if matches!(self.publication, LocalPublication::FirstAnswerPending) {
            self.attach_codex_names(&mut rows);
            classify::attach_staff_names(&mut rows, &staff.roster);
            publish(ScanStage::Classified, rows.clone(), &staff);
        }
        startup_log::startup_stage("details", || read_details(self, &mut rows));
        self.attach_codex_names(&mut rows);
        classify::attach_staff_names(&mut rows, &staff.roster);
        apply_astra_work(&mut rows, &self.herdr);
        self.complete_worker_facts(&rows, processes, &staff.roster);
        staff.facts.workers = Observed::Known(self.worker_facts.clone());
        publish(ScanStage::Completed, rows, &staff);
        self.publication = LocalPublication::CompletedAnswerPublished;
        staff
    }

    /// Read current thread rows and resolve one map for this scan.
    fn resolve_codex_sessions(&mut self, processes: &[ProcessEntry], home: Option<&Path>) {
        let records = startup_log::startup_stage("codex_thread_ids", || {
            self.codex_logs
                .thread_rows(processes, home, self.codex_start_window)
        });
        self.codex_links =
            classify::codex_session_links(processes, home, &self.codex_logs, &records);
        startup_log::startup_stage("herdr_names", || {
            self.herdr.refresh(processes, &mut self.codex_links);
        });
    }

    /// A scanner of the user running this process.
    pub(super) fn new(account_labels: AccountLabels) -> Self {
        Self {
            publication: LocalPublication::FirstAnswerPending,
            home: dirs::home_dir(),
            account_labels,
            shell_calls: HashMap::new(),
            launchers: HashMap::new(),
            transcript_paths: HashMap::new(),
            session_starts: HashMap::new(),
            rollouts: Rollouts::default(),
            session_names: super::codex_names::SessionNames::default(),
            open_thread_rows: OpenThreadRows::default(),
            codex_logs: codex::CodexLogs::default(),
            #[cfg(not(test))]
            herdr: HerdrNames::default(),
            #[cfg(test)]
            herdr: HerdrNames::isolated(),
            codex_links: HashMap::new(),
            staff: super::hierarchy::StaffDiscovery::Disabled,
            codex_start_window: crate::config::CodexStartWindow::default(),
            herdr_probe_wait: HerdrProbeWait::default(),
            transcript_activity: HashMap::new(),
            compactions: CompactionMarkers::for_current_user(),
            transcript_models: super::transcript::TranscriptModels::default(),
            subagent_activity: HashMap::new(),
            subagent_reads: SubagentReads::default(),
            worker_names: WorkerNames::default(),
            worker_commands: WorkerCommands::default(),
            session_roots: SessionRoots::production(),
            worker_facts: BTreeMap::new(),
            activity_starts: HashMap::new(),
            current_staff: CurrentStaff::default(),
            processes: ProcessCensus::default(),
            run_records: activity::RunRecords::default(),
            desktop_source: DesktopSource::System,
            desktops: None,
        }
    }

    /// Use the same startup setting as the host or probe configuration.
    pub(super) const fn with_codex_start_window(mut self, window: CodexStartWindow) -> Self {
        self.codex_start_window = window;
        self
    }

    /// Use the configured wait only when the probe requests a completed scan.
    pub(super) const fn with_herdr_probe_wait(mut self, wait: HerdrProbeWait) -> Self {
        self.herdr_probe_wait = wait;
        self
    }

    /// Use the configured socket timeout for this scanner's herdr worker.
    pub(super) fn with_herdr_read_timeout(mut self, timeout: HerdrReadTimeout) -> Self {
        self.herdr = self.herdr.with_read_timeout(timeout);
        self
    }

    #[cfg(test)]
    pub(super) const fn herdr_socket_timeout(&self) -> Duration { self.herdr.socket_timeout() }

    /// Share the staff reader with hierarchy publication.
    pub(super) fn with_staff_file(
        mut self,
        setting: super::StaffFileSetting,
        reader: StaffFileReader,
    ) -> Self {
        self.staff = super::hierarchy::StaffDiscovery::File { setting, reader };
        self
    }

    /// Every agent running now, oldest first, with what it is running.
    /// The wire probe consumes only the completed answer.
    ///
    /// Each scan refreshes the retained process table. sysinfo replaces a reused
    /// pid when its start time changes, including its command and directory.
    pub(super) fn scan(&mut self) -> (Vec<AgentRow>, StaffSnapshot) {
        let deadline = super::herdr::HerdrProbeDeadline::from_scan_start(
            Instant::now(),
            self.herdr_probe_wait,
        );
        let mut rows = Vec::new();
        let mut snapshot = self.scan_staged(|_, answer, _| rows = answer);
        self.herdr.finish_initial(&mut self.codex_links, deadline);
        apply_astra_work(&mut rows, &self.herdr);
        self.attach_codex_names(&mut rows);
        classify::attach_staff_names(&mut rows, &snapshot.roster);
        self.complete_worker_facts(&rows, &[], &snapshot.roster);
        snapshot.facts.workers = Observed::Known(self.worker_facts.clone());
        (rows, snapshot)
    }

    /// Publish classification early on startup; later scans publish completed rows only.
    pub(super) fn scan_staged(
        &mut self,
        publish: impl FnMut(ScanStage, Vec<AgentRow>, &StaffSnapshot),
    ) -> StaffSnapshot {
        let home = self.home.clone();
        let sessions = startup_log::startup_stage("sessions", || {
            home.as_deref()
                .map(|home| read_sessions(&home.join(CLAUDE_DIRNAME).join(CLAUDE_SESSIONS_DIRNAME)))
                .unwrap_or_default()
        });
        let processes =
            startup_log::startup_stage("process_table", || self.processes.read(&sessions));
        self.codex_logs.refresh(&processes, home.as_deref());
        self.scan_with_staff(
            &processes,
            &sessions,
            home.as_deref(),
            publish,
            |scanner, rows| {
                scanner.attach_details(&processes, &sessions, home.as_deref(), rows);
            },
        )
    }

    /// Read the detail sources after the classified rows have been published.
    fn attach_details(
        &mut self,
        processes: &[ProcessEntry],
        sessions: &[SessionRecord],
        home: Option<&Path>,
        rows: &mut [AgentRow],
    ) {
        // The pacer's state is read only while some Codex runs.
        let pacer = home
            .filter(|_| processes.iter().any(ProcessEntry::is_codex))
            .map(|home| PacerHistory::read(&home.join(CODEX_PACER_STATE)))
            .unwrap_or_default();
        let service_tiers = startup_log::startup_stage("codex_service_tiers", || {
            self.attach_codex_tiers(processes, &pacer, rows)
        });
        let transcripts = startup_log::startup_stage("transcript_discovery", || {
            home.map(|home| home.join(CLAUDE_DIRNAME).join(PROJECTS_DIRNAME))
                .map(|projects| self.find_transcripts(&projects, processes, sessions))
                .unwrap_or_default()
        });
        let now = unix_now();
        startup_log::startup_stage("transcript_launchers", || {
            self.find_launchers(processes, sessions, &transcripts, rows, now);
        });
        startup_log::startup_stage("transcript_session_starts", || {
            self.attach_session_starts(&transcripts, rows);
        });
        let subagents = self.read_active_subagents(&transcripts);
        let shell_calls = startup_log::startup_stage("transcript_shell_calls", || {
            self.find_shell_calls(processes, &transcripts, &subagents, now)
        });
        let codex = startup_log::startup_stage("codex_app_server_threads", || {
            codex_threads(
                processes,
                rows,
                &mut self.rollouts,
                &mut self.open_thread_rows,
                &pacer,
                home,
                &self.account_labels,
            )
        });
        self.attach_astra_threads(rows, &codex.accounts, &pacer);
        self.attach_codex_sessions(processes, home, rows);
        let models = self.read_process_models(processes, &transcripts, rows, home, codex.panes);
        let threads = codex.app_servers;
        self.rollouts.forget_unread();
        let dispatches = startup_log::startup_stage("worker_dispatches", || {
            worker_dispatches(processes, &mut self.worker_names, &mut self.worker_commands)
        });
        let thread_dispatches = threads
            .values()
            .flatten()
            .filter_map(|thread| {
                let CodexThreadId::Recorded(id) = &thread.thread_id else {
                    return None;
                };
                let ThreadRollout::Retained(read) = self.rollouts.thread(id) else {
                    return None;
                };
                match self.worker_names.resolve(
                    id,
                    read.first_prompt.text(),
                    &read.worktree,
                    &self.session_roots,
                ) {
                    WorkerDispatch::Worker(worker) => Some((id.clone(), worker)),
                    WorkerDispatch::Plain => None,
                }
            })
            .collect();
        let thread_ids = threads
            .values()
            .flatten()
            .filter_map(|thread| match &thread.thread_id {
                CodexThreadId::Recorded(id) => Some(id.as_str()),
                CodexThreadId::Unrecorded => None,
            });
        let worker_runs = self.read_worker_runs(processes, rows, thread_ids);
        let sources = TreeSources {
            run_threads: worker_runs.threads,
            dispatches,
            thread_dispatches,
            sessions,
            subagents,
            shell_calls,
            threads,
            service_tiers,
        };
        let run_made = startup_log::startup_stage("attach_children", || {
            tree::attach_children(rows, processes, &sources)
        });
        self.find_compaction_transcripts(processes, rows, home);
        self.attach_row_models(rows, &models, &sources.subagents, &transcripts);
        startup_log::startup_stage("transcript_activity_and_worker_names", || {
            self.attach_activity_with_runs(
                processes,
                &transcripts,
                rows,
                &worker_runs.activities,
                &run_made,
            );
            tree::fold_run_processes(rows, &worker_runs.folded);
            tree::append_named_run_workers(rows, &worker_runs.named);
            self.retain_session_caches(&transcripts, worker_runs.sessions);
        });
        startup_log::startup_stage("attach_desktops", || self.attach_desktops(processes, rows));
        startup_log::startup_stage("attach_branches", || branch::attach_branches(rows, home));
    }

    /// Find native completion files only for marked Codex sessions without retained rollouts.
    fn find_compaction_transcripts(
        &mut self,
        processes: &[ProcessEntry],
        rows: &[AgentRow],
        home: Option<&Path>,
    ) {
        let ids: Vec<_> = rows
            .iter()
            .flat_map(|row| {
                row.session_id
                    .ids()
                    .filter(|_| row.agent != Agent::Claude)
                    .chain(
                        row.children
                            .iter()
                            .filter_map(|child| match &child.thread_id {
                                CodexThreadId::Recorded(id) => Some(id.as_str()),
                                CodexThreadId::Unrecorded => None,
                            }),
                    )
            })
            .filter(|id| {
                matches!(self.rollouts.thread(id), ThreadRollout::NotRetained)
                    && matches!(
                        self.compactions.marker_status(id, CompactionFamily::Codex),
                        CompactionMarkerStatus::Open { .. }
                    )
            })
            .map(str::to_owned)
            .collect();
        self.rollouts
            .find_compaction_transcripts(processes, home, &ids);
    }

    /// `read_process_models` reads each process's own conversation without inheriting host values.
    fn read_process_models(
        &mut self,
        processes: &[ProcessEntry],
        transcripts: &HashMap<u32, PathBuf>,
        rows: &[AgentRow],
        home: Option<&Path>,
        mut paths: HashMap<u32, PathBuf>,
    ) -> HashMap<u32, ConversationModel> {
        let mut models: HashMap<_, _> = transcripts
            .iter()
            .map(|(pid, path)| (*pid, self.transcript_models.read(path)))
            .collect();
        for process in processes
            .iter()
            .filter(|process| process.is_codex() && !process.is_app_server())
        {
            if let Some(thread) = self
                .codex_links
                .get(&process.pid)
                .and_then(CodexSessionLink::thread)
            {
                paths.insert(process.pid, thread.rollout.clone());
            }
            if paths.contains_key(&process.pid) {
                continue;
            }
            let proven = rows
                .iter()
                .find(|row| row.pid == process.pid)
                .filter(|row| {
                    matches!(
                        row.session_id,
                        SessionIdentity::Codex(_) | SessionIdentity::Astra(_)
                    )
                });
            if let Some(row) = proven {
                for (id, path) in open_rollouts(process, home) {
                    if row.session_id.ids().any(|proven| proven == id) {
                        paths.insert(process.pid, path);
                        break;
                    }
                }
            }
        }
        for (pid, path) in paths {
            models.insert(pid, self.rollouts.read_model(&path));
        }
        models
    }

    /// `attach_row_models` maps only a row's own process, transcript or proven rollout.
    fn attach_row_models(
        &mut self,
        rows: &mut [AgentRow],
        processes: &HashMap<u32, ConversationModel>,
        subagents: &HashMap<u32, Vec<Subagent>>,
        transcripts: &HashMap<u32, PathBuf>,
    ) {
        let mut paths: HashSet<_> = transcripts.values().cloned().collect();
        let mut agents = HashMap::new();
        for (owner, children) in subagents {
            for child in children {
                paths.insert(child.transcript.clone());
                agents.insert(
                    (*owner, child.id.as_str()),
                    self.transcript_models.read(&child.transcript),
                );
            }
        }
        for row in rows {
            let own = processes.get(&row.pid).cloned().unwrap_or_default();
            row.model = own.model;
            row.effort = own.effort;
            self.attach_child_models(&mut row.children, row.pid, processes, &agents);
        }
        self.transcript_models.retain(&paths);
    }

    /// `attach_child_models` follows flattened child subtrees with their owning Claude pid.
    fn attach_child_models(
        &self,
        mut children: &mut [ChildRow],
        owner: u32,
        processes: &HashMap<u32, ConversationModel>,
        subagents: &HashMap<(u32, &str), ConversationModel>,
    ) {
        while let Some((child, following)) = children.split_first_mut() {
            let end = following
                .iter()
                .position(|next| next.depth <= child.depth)
                .unwrap_or(following.len());
            let (descendants, siblings) = following.split_at_mut(end);
            let own = match (&child.kind, &child.thread_id, &child.work_id) {
                (ChildKind::Shell(_), _, _) => ConversationModel::default(),
                (_, CodexThreadId::Recorded(id), _) => match self.rollouts.thread(id) {
                    ThreadRollout::Retained(read) => read.inference.clone(),
                    ThreadRollout::NotRetained => ConversationModel::default(),
                },
                (ChildKind::Subagent(_), _, WorkIdentity::Recorded(id)) => subagents
                    .get(&(owner, id.as_str()))
                    .cloned()
                    .unwrap_or_default(),
                _ => child
                    .pid
                    .and_then(|pid| processes.get(&pid))
                    .cloned()
                    .unwrap_or_default(),
            };
            child.model = own.model;
            child.effort = own.effort;
            let descendant_owner = match &child.kind {
                ChildKind::Session(Agent::Claude)
                | ChildKind::UnderShell(Agent::Claude)
                | ChildKind::Direct(Agent::Claude)
                | ChildKind::Detached(Agent::Claude) => child.pid.unwrap_or(owner),
                _ => owner,
            };
            self.attach_child_models(descendants, descendant_owner, processes, subagents);
            children = siblings;
        }
    }

    /// Read the active subagents and their transcript activity.
    fn read_active_subagents(
        &mut self,
        transcripts: &HashMap<u32, PathBuf>,
    ) -> HashMap<u32, Vec<Subagent>> {
        let mut subagents = startup_log::startup_stage("transcript_subagents", || {
            transcripts
                .iter()
                .map(|(pid, path)| {
                    (
                        *pid,
                        transcript::running_subagents(
                            path,
                            SystemTime::now(),
                            &mut self.subagent_reads,
                        ),
                    )
                })
                .collect()
        });
        self.attach_subagent_activity(&mut subagents);
        subagents
    }

    /// Resolve the interactive session itself, using the same rollout reader as workers.
    fn attach_codex_sessions(
        &mut self,
        processes: &[ProcessEntry],
        home: Option<&Path>,
        rows: &mut [AgentRow],
    ) {
        let homes: HashSet<_> = processes
            .iter()
            .filter(|process| process.is_codex())
            .filter_map(|process| codex::session_home(process, home))
            .collect();
        for codex_home in &homes {
            self.session_names.read(codex_home);
        }
        for row in rows.iter_mut().filter(|row| row.agent == Agent::Codex) {
            let Some(process) = processes.iter().find(|process| process.pid == row.pid) else {
                continue;
            };
            if process.is_app_server() {
                continue;
            }
            row.activity = AgentActivity::Idle;
            let Some(thread) = self
                .codex_links
                .get(&row.pid)
                .and_then(classify::CodexSessionLink::thread)
            else {
                continue;
            };
            let read = self.rollouts.read(&thread.rollout);
            row.activity = read.activity();
        }
    }

    /// This stage owns the fallback name in each interactive census row.
    fn attach_codex_names(&self, rows: &mut [AgentRow]) {
        for row in rows.iter_mut().filter(|row| row.agent == Agent::Codex) {
            match self.codex_links.get(&row.pid) {
                Some(CodexSessionLink::Named(thread)) => {
                    row.name = match self.session_names.name(thread) {
                        SessionName::Named(name) => name,
                        SessionName::Unnamed => format!("pid {}", row.pid),
                    };
                },
                Some(CodexSessionLink::HerdrName(name)) => row.name.clone_from(name),
                Some(CodexSessionLink::NoMatch) => row.name = format!("pid {}", row.pid),
                None => {},
            }
        }
    }

    /// Advance each live subagent's own Claude transcript and forget finished ones.
    fn attach_subagent_activity(&mut self, subagents: &mut HashMap<u32, Vec<Subagent>>) {
        let mut seen = HashSet::new();
        for agent in subagents.values_mut().flatten() {
            seen.insert(agent.transcript.clone());
            let read = self
                .subagent_activity
                .entry(agent.transcript.clone())
                .or_insert_with(|| TranscriptRead::new(AgentActivity::Idle));
            read.advance(&agent.transcript, AgentActivity::Idle);
            agent.activity = read.activity();
        }
        self.subagent_activity.retain(|path, _| seen.contains(path));
    }

    /// Test fixtures can supply already attached child rows.
    #[cfg(test)]
    fn attach_activity_and_names(
        &mut self,
        processes: &[ProcessEntry],
        transcripts: &HashMap<u32, PathBuf>,
        rows: &mut [AgentRow],
    ) {
        let staff = self.staff.snapshot();
        self.worker_names.set_staff(staff.roster);
        let runs = self.read_worker_runs(processes, rows, std::iter::empty());
        self.attach_activity_with_runs(
            processes,
            transcripts,
            rows,
            &runs.activities,
            &HashSet::new(),
        );
        self.retain_session_caches(transcripts, runs.sessions);
    }

    /// Resolve conversation tools after every run source has supplied its answer.
    fn attach_activity_with_runs(
        &mut self,
        processes: &[ProcessEntry],
        transcripts: &HashMap<u32, PathBuf>,
        rows: &mut [AgentRow],
        runs: &HashMap<String, WorkerRun>,
        run_made: &HashSet<String>,
    ) {
        self.compactions.begin_scan();
        let mut seen_transcripts = HashSet::new();
        let table = classify::pid_table(processes);
        let mut verification_activity = VerificationProcesses::new(processes);
        let verifications: Vec<_> = processes
            .iter()
            .filter(|process| is_verification_process(process))
            .collect();
        for row in rows.iter_mut() {
            if let Some(path) = transcripts.get(&row.pid) {
                seen_transcripts.insert(path.clone());
                let read = self
                    .transcript_activity
                    .entry(path.clone())
                    .or_insert_with(|| activity::TranscriptRead::new(row.activity));
                read.advance(path, row.activity);
                startup_log::startup_event(&format!("transcript_bytes_read {}", read.bytes_read()));
                // `row.activity` comes from the Claude session record status and says
                // whether the turn is open, also while the Stop hooks run after the last
                // answer. In an open turn, `TranscriptRead::activity` names the work.
                match (row.activity, read.activity()) {
                    (AgentActivity::Idle, _)
                    | (_, AgentActivity::Unreported)
                    | (AgentActivity::Active(_), AgentActivity::Idle) => {},
                    (_, transcript) => row.activity = transcript,
                }
            }
            if let Some(verify) = verifications
                .iter()
                .find(|process| process_is_below(process.pid, row.pid, &table))
            {
                row.activity = verification_activity.progress(verify.pid).activity();
            }
            let live_run = row
                .session_id
                .ids()
                .filter_map(|id| runs.get(id))
                .find_map(|run| match run {
                    WorkerRun::Running { activity, .. } => Some(*activity),
                    WorkerRun::Inactive => None,
                });
            if let Some(activity) = live_run {
                row.activity = activity;
                if let AstraSessionMembership::Member(member) = row.session_id.astra_membership() {
                    let member = member.clone().with_run_activity(activity);
                    row.session_id = std::mem::take(&mut row.session_id).with_astra(member);
                }
            }
            self.apply_row_compaction(row, transcripts);
            for child in &mut row.children {
                let CodexThreadId::Recorded(id) = &child.thread_id else {
                    continue;
                };
                let run = runs.get(id).unwrap_or(&WorkerRun::Inactive);
                let read = self.rollouts.thread(id);
                if let ThreadRollout::Retained(read) = read {
                    let prompt = read.first_prompt.text();
                    let resolved =
                        self.worker_names
                            .resolve(id, prompt, &read.worktree, &self.session_roots);
                    if !run_made.contains(id) {
                        child.dispatch = WorkerDispatch::Plain;
                    }
                    if !resolved.is_plain() {
                        child.dispatch = resolved;
                    }
                    child.worker_task = self.worker_names.task(id, prompt, &self.session_roots);
                    child.name = match &child.dispatch {
                        WorkerDispatch::Worker(worker) => worker.label(),
                        WorkerDispatch::Plain => child
                            .worker_task
                            .label(WorkerRoleSource::Unroled, &child.name),
                    };
                    child.activity = match run {
                        WorkerRun::Inactive => read.activity(),
                        WorkerRun::Running { activity, .. } => *activity,
                    };
                } else {
                    child.worker_task = WorkerTask::RoleOnly;
                    if let WorkerRun::Running { activity, .. } = run {
                        child.activity = *activity;
                    }
                }
                child.activity = self.compactions.activity(
                    id,
                    CompactionFamily::Codex,
                    self.rollouts.transcript(id),
                    child.activity,
                );
            }
        }
        self.compactions.finish_scan();
        self.transcript_activity
            .retain(|path, _| seen_transcripts.contains(path));
    }

    /// `apply_row_compaction` resolves the marker after transcript and run activity.
    fn apply_row_compaction(&mut self, row: &mut AgentRow, transcripts: &HashMap<u32, PathBuf>) {
        if let Some(id) = row.session_id.ids().next() {
            let family = if row.agent == Agent::Claude {
                CompactionFamily::Claude
            } else {
                CompactionFamily::Codex
            };
            let transcript = match family {
                CompactionFamily::Claude => transcripts
                    .get(&row.pid)
                    .map_or(CompactionTranscript::Unavailable, |path| {
                        CompactionTranscript::File(path)
                    }),
                CompactionFamily::Codex => self.rollouts.transcript(id),
            };
            row.activity = self
                .compactions
                .activity(id, family, transcript, row.activity);
        }
    }

    /// Read every required run directory before selecting the latest run for each thread.
    fn record_worker_facts(
        &mut self,
        staff: &StaffRoster,
        processes: &[ProcessEntry],
        records: &HashMap<PathBuf, Vec<RunRecord>>,
        health: &HashMap<PathBuf, super::SourceHealth>,
    ) {
        let passes = staff
            .current_runs()
            .map(|(_, run)| (run.path().to_owned(), self.session_roots.passes(run.path())))
            .collect();
        let observations = CurrentRunObservations {
            staff,
            processes,
            records,
            health,
            passes,
        };
        self.worker_facts = staff
            .session_ids()
            .map(|session| (session.to_owned(), observations.worker(session)))
            .collect();
    }

    /// Use the displayed activity while retaining its source timestamp.
    fn complete_worker_facts(
        &mut self,
        rows: &[AgentRow],
        processes: &[ProcessEntry],
        staff: &StaffRoster,
    ) {
        for row in rows {
            for id in row.session_id.ids() {
                let process = WorkerProcess::Running {
                    pid:     row.pid,
                    started: row.process_started,
                };
                self.complete_worker_fact(id, row.activity, process, staff);
            }
            for child in &row.children {
                if let CodexThreadId::Recorded(id) = &child.thread_id {
                    let process = processes
                        .iter()
                        .find(|process| Some(process.pid) == child.pid)
                        .map_or(WorkerProcess::Absent, |process| WorkerProcess::Running {
                            pid:     process.pid,
                            started: process.started,
                        });
                    self.complete_worker_fact(id, child.activity, process, staff);
                }
            }
        }
        self.activity_starts
            .retain(|session, _| self.worker_facts.contains_key(session));
    }

    fn complete_worker_fact(
        &mut self,
        id: &str,
        activity: AgentActivity,
        process: WorkerProcess,
        staff: &StaffRoster,
    ) {
        let facts = self
            .worker_facts
            .entry(id.to_owned())
            .or_insert_with(|| super::WorkerFacts {
                listing:          Observed::Known(staff.listing(id)),
                current_run:      staff.worker_run(id),
                pane:             super::Observed::unknown("worker has no pane observation"),
                runs:             super::Observed::unknown("worker has no current run records"),
                activity_started: super::Observed::unknown(
                    "worker has no current run or open pass",
                ),
                pass:             super::Observed::unknown("worker has no current run records"),
            });
        if let WorkerProcess::Running { pid, started } = process {
            facts.pane = self.herdr.pane(pid, started);
        }
        let read = self
            .activity_starts
            .entry(id.to_owned())
            .or_insert_with(|| activity::ActivityRead::from_activity(activity));
        read.observe(activity, &facts.activity_started);
        facts.activity_started = read.started();
    }

    fn read_worker_runs<'t>(
        &mut self,
        processes: &[ProcessEntry],
        rows: &'t [AgentRow],
        thread_ids: impl Iterator<Item = &'t str>,
    ) -> WorkerRuns {
        let live_processes = processes.iter().map(|process| process.pid).collect();
        let directors: BTreeMap<_, _> = rows
            .iter()
            .filter_map(|row| row.session_id.claude_id().map(|id| (id, row.pid)))
            .chain(
                rows.iter()
                    .filter(|row| row.agent != Agent::Claude)
                    .flat_map(|row| row.session_id.ids().map(move |id| (id, row.pid))),
            )
            .collect();
        let staff = self.worker_names.staff_snapshot();
        let mut records = HashMap::new();
        let mut active_units = self.session_roots.live_directories(processes);
        active_units.extend(staff.current_runs().map(|(_, run)| run.path().to_owned()));
        let mut health = HashMap::new();
        for directory in &active_units {
            let read = self.run_records.current(directory);
            health.insert(directory.clone(), read.health);
            records.insert(directory.clone(), read.records);
        }
        self.read_rollout_runs(thread_ids, rows, &live_processes, &mut records);
        let live = VerificationProcesses::new(processes).live_runs(records.values().flatten());
        let owners = run_directors(&live, &staff, &directors);
        let sessions =
            self.session_roots
                .managed_sessions(&active_units, &directors, &staff, |directory| {
                    if let Some(pids) = owners.get(directory)
                        && pids.len() == 1
                    {
                        return pids.iter().next().copied();
                    }
                    None
                });
        drop(live);
        records.retain(|directory, _| {
            self.session_roots.selection(directory, &staff) == RunSelection::Current
        });
        self.record_worker_facts(&staff, processes, &records, &health);
        let live = VerificationProcesses::new(processes).live_runs(records.values().flatten());
        let resolved = resolved_worker_runs(&live, &sessions, &directors, &staff);
        let activities = run_activities(&resolved);
        let mut answer = WorkerRuns {
            sessions: active_units,
            threads: HashMap::new(),
            activities,
            named: HashMap::new(),
            folded: HashSet::new(),
        };
        Self::attach_record_runs(&resolved, &mut answer, &staff);
        answer.sessions.extend(records.into_keys());
        answer
    }

    /// `RunRecords` includes directories found through retained worker rollouts.
    fn read_rollout_runs<'t>(
        &mut self,
        thread_ids: impl Iterator<Item = &'t str>,
        rows: &'t [AgentRow],
        live_processes: &HashSet<u32>,
        records: &mut HashMap<PathBuf, Vec<RunRecord>>,
    ) {
        let ids: HashSet<_> = thread_ids
            .chain(
                rows.iter()
                    .flat_map(|row| &row.children)
                    .filter_map(|child| match &child.thread_id {
                        CodexThreadId::Recorded(id) => Some(id.as_str()),
                        CodexThreadId::Unrecorded => None,
                    }),
            )
            .collect();
        for id in ids {
            if let ThreadRollout::Retained(read) = self.rollouts.thread(id) {
                for directory in
                    self.worker_names
                        .directories(id, read.first_prompt.text(), &self.session_roots)
                {
                    self.read_run_session(&directory, live_processes, records);
                }
            }
        }
    }

    /// `WorkerRuns` adds rows for `RunRecord` workers that have no visible process.
    fn attach_record_runs(
        sources: &[ResolvedWorkerRun<'_>],
        answer: &mut WorkerRuns,
        staff: &StaffRoster,
    ) {
        let mut present: HashSet<_> = answer
            .threads
            .iter()
            .flat_map(|(pid, threads)| threads.iter().map(move |thread| (*pid, thread.id.clone())))
            .collect();
        let mut named: HashMap<(u32, &str), &LiveWorkerRun<'_>> = HashMap::new();
        for source in sources {
            let RunManager::Director(pid) = source.manager else {
                continue;
            };
            match source.worker {
                RunWorkerIdentity::Session(id) => {
                    let seat = match source.run.record.seat() {
                        RunSeat::Named(seat) => seat,
                        RunSeat::Unrecorded => match staff.member(id) {
                            StaffMemberLookup::Managed { member, .. } => member.name,
                            StaffMemberLookup::Root(_) | StaffMemberLookup::Unavailable => continue,
                        },
                    };
                    if let Some(WorkerRun::Running {
                        activity,
                        started,
                        worktree,
                    }) = answer.activities.get(id)
                        && present.insert((pid, id.to_owned()))
                        && let Ok(dispatch) = ResolvedWorkerDispatch::for_run(
                            seat,
                            WorkerIdentity::CodexThread(id.to_owned()),
                            staff,
                            worktree,
                        )
                    {
                        answer.threads.entry(pid).or_default().push(RunThread {
                            id: id.to_owned(),
                            dispatch,
                            activity: *activity,
                            started: *started,
                        });
                    }
                },
                RunWorkerIdentity::NamedSeat(seat) => {
                    let current = named.entry((pid, seat)).or_insert(source.run);
                    if source.run.newer_than(current) {
                        *current = source.run;
                    }
                },
                RunWorkerIdentity::Unrecorded => continue,
            }
            answer.folded.extend(&source.run.processes);
        }
        for ((pid, seat), run) in named {
            answer.named.entry(pid).or_default().push(ChildRow {
                model:        super::model::ModelName::Unrecorded,
                effort:       super::model::ReasoningEffort::Unrecorded,
                work_id:      WorkIdentity::Legacy,
                worker_task:  WorkerTask::RoleOnly,
                activity:     run.activity,
                dispatch:     WorkerDispatch::Plain,
                thread_id:    CodexThreadId::Unrecorded,
                depth:        0,
                kind:         ChildKind::Thread,
                service_tier: ServiceTier::Unrecorded,
                account:      super::Account::Unrecorded,
                pid:          Some(run.pid),
                name:         seat.to_owned(),
                started:      run.started,
            });
        }
    }

    /// Share one cached run reader between supervisors and rollouts.
    fn read_run_session(
        &mut self,
        directory: &Path,
        live_processes: &HashSet<u32>,
        runs: &mut HashMap<PathBuf, Vec<RunRecord>>,
    ) {
        runs.entry(directory.to_owned()).or_insert_with(|| {
            self.run_records
                .read_for_processes(directory, live_processes)
        });
    }

    /// Release session watches and records together after the census uses them.
    fn retain_session_caches(
        &mut self,
        transcripts: &HashMap<u32, PathBuf>,
        mut sessions: HashSet<PathBuf>,
    ) {
        sessions.extend(
            self.worker_commands
                .processes
                .values()
                .flat_map(|command| [&command.direct, &command.wrapped])
                .filter_map(|launcher| match launcher {
                    WorkerLauncher::Dispatch(dispatch) => {
                        dispatch.session_directory().ok().map(Path::to_path_buf)
                    },
                    WorkerLauncher::OtherCommand => None,
                }),
        );
        self.run_records.retain_sessions(&sessions);
        self.worker_names.retain_sessions(&sessions);
        self.subagent_reads
            .retain_sessions(&transcripts.values().cloned().collect());
    }

    /// `attach_codex_tiers` attaches each process's requested tier to its row.
    fn attach_codex_tiers(
        &mut self,
        processes: &[ProcessEntry],
        pacer: &PacerHistory,
        rows: &mut [AgentRow],
    ) -> HashMap<u32, ServiceTier> {
        let tiers = self.codex_tiers(processes, pacer);
        for row in rows {
            if let Some(tier) = tiers.get(&row.pid) {
                row.service_tier = *tier;
            }
        }
        tiers
    }

    /// The tier each Codex among `processes` that is not an app server
    /// asked for, by pid, from the rollout of the thread it started with
    /// among `codex_threads`, its command line, then `pacer`.
    fn codex_tiers(
        &mut self,
        processes: &[ProcessEntry],
        pacer: &PacerHistory,
    ) -> HashMap<u32, ServiceTier> {
        processes
            .iter()
            .filter(|process| process.is_codex() && !process.is_app_server())
            .map(|process| {
                let thread = self
                    .codex_links
                    .get(&process.pid)
                    .and_then(classify::CodexSessionLink::thread);
                let rollout = thread.map(|thread| self.rollouts.read(&thread.rollout));
                let started = thread.map_or(process.started, |thread| thread.created_ms / 1_000);
                let tier = tier::requested(rollout, &process.arguments, pacer, started);
                (process.pid, tier)
            })
            .collect()
    }

    /// Set the desktop of each of `rows`. `KWin` and tmux are asked
    /// again only once their last answer is [`DESKTOP_QUERY_INTERVAL`]
    /// old, and not at all while there are no rows.
    fn attach_desktops(&mut self, processes: &[ProcessEntry], rows: &mut [AgentRow]) {
        if rows.is_empty() {
            return;
        }
        match &self.desktop_source {
            DesktopSource::System => {
                if self.desktops.as_ref().is_none_or(DesktopRead::is_stale) {
                    self.desktops = Some(DesktopRead::now(processes, rows));
                }
                if let Some(DesktopRead {
                    windows: Some(windows),
                    tmux,
                    ..
                }) = &self.desktops
                {
                    desktop::attach_desktops(rows, processes, windows, tmux.as_ref());
                }
            },
            #[cfg(test)]
            DesktopSource::Windows(windows) => {
                desktop::attach_desktops(rows, processes, windows, None);
            },
        }
    }

    /// Set `launched_by` on each of `rows` a tmux server holds, from the
    /// transcripts of the other Claude Code rows. Each row still carries
    /// its process's start, which is when the call that opened it was
    /// written.
    fn find_launchers(
        &mut self,
        processes: &[ProcessEntry],
        sessions: &[SessionRecord],
        transcripts: &HashMap<u32, PathBuf>,
        rows: &mut [AgentRow],
        now: u64,
    ) {
        let listed: HashSet<ProcessKey> = rows
            .iter()
            .map(|row| (row.pid, row.process_started))
            .collect();
        let mut seen = HashSet::new();
        let mut found = Vec::new();
        for (index, row) in rows.iter().enumerate() {
            if !classify::held_by_tmux(processes, row.pid) {
                continue;
            }
            let key = (row.pid, row.process_started);
            seen.insert(key);
            let launcher = if let Some(cached) = self.launchers.get(&key) {
                *cached
            } else {
                let launcher = search_launcher(processes, sessions, transcripts, rows, row);
                if launcher.is_some() || settled(row.process_started, now) {
                    self.launchers.insert(key, launcher);
                }
                launcher
            };
            // A launcher no longer listed leaves the session top level, as
            // a scan starting afresh would.
            if let Some(launcher) = launcher.filter(|launcher| listed.contains(launcher)) {
                found.push((index, launcher.0));
            }
        }
        self.launchers.retain(|key, _| seen.contains(key));
        for (index, launcher) in found {
            rows[index].launched_by = Some(launcher);
        }
    }

    /// The transcript of each live Claude Code session among `sessions`,
    /// by its pid, under Claude Code's `projects` directory. One found
    /// away from the session's directory, as a session resumed or moved
    /// since has, is kept; one not found yet is where the session's
    /// directory says it will be written.
    fn find_transcripts(
        &mut self,
        projects: &Path,
        processes: &[ProcessEntry],
        sessions: &[SessionRecord],
    ) -> HashMap<u32, PathBuf> {
        let table = classify::pid_table(processes);
        let live: Vec<&SessionRecord> = sessions
            .iter()
            .filter(|session| {
                table
                    .get(&session.pid)
                    .is_some_and(|process| process.is_claude())
            })
            .collect();
        let ids: HashSet<&str> = live
            .iter()
            .map(|session| session.session_id.as_str())
            .collect();
        self.transcript_paths
            .retain(|id, _| ids.contains(id.as_str()));
        let mut found = HashMap::new();
        for session in live {
            let cwd = session.cwd.as_deref();
            let path = if let Some(kept) = self.transcript_paths.get(&session.session_id) {
                Some(kept.clone())
            } else if let Some(path) =
                transcript::find_transcript(projects, cwd, &session.session_id)
            {
                self.transcript_paths
                    .insert(session.session_id.clone(), path.clone());
                Some(path)
            } else {
                cwd.map(|cwd| transcript::transcript_path(projects, cwd, &session.session_id))
            };
            if let Some(path) = path {
                found.insert(session.pid, path);
            }
        }
        found
    }

    /// Date each Claude Code row among `rows` from the first line of its
    /// transcript in `transcripts` where that is earlier than its
    /// process, then put the rows back in order, oldest first. A
    /// transcript's first line never changes, so each is read once.
    fn attach_session_starts(
        &mut self,
        transcripts: &HashMap<u32, PathBuf>,
        rows: &mut [AgentRow],
    ) {
        let current: HashSet<&PathBuf> = transcripts.values().collect();
        self.session_starts.retain(|path, _| current.contains(path));
        for row in rows.iter_mut() {
            let Some(path) = transcripts.get(&row.pid) else {
                continue;
            };
            let began = *self
                .session_starts
                .entry(path.clone())
                .or_insert_with(|| transcript::began(path));
            if let Some(began) = began {
                row.started = row.started.min(began);
            }
        }
        rows.sort_by_key(|row| (row.started, row.pid));
    }

    /// The call that started each shell wrapper under a Claude Code
    /// process with a transcript, by the wrapper's pid.
    fn find_shell_calls(
        &mut self,
        processes: &[ProcessEntry],
        transcripts: &HashMap<u32, PathBuf>,
        subagents: &HashMap<u32, Vec<Subagent>>,
        now: u64,
    ) -> HashMap<u32, ShellCall> {
        let mut seen = HashSet::new();
        let mut found = HashMap::new();
        for process in processes {
            let Some(main) = process.parent.and_then(|parent| transcripts.get(&parent)) else {
                continue;
            };
            let Some(command) = tree::shell_command(process) else {
                continue;
            };
            let key = (process.pid, process.started);
            seen.insert(key);
            let call = if let Some(cached) = self.shell_calls.get(&key) {
                cached.clone()
            } else {
                let theirs = process
                    .parent
                    .and_then(|parent| subagents.get(&parent))
                    .map_or(&[][..], Vec::as_slice);
                let call = search_shell_call(process, &command, main, theirs);
                if call.is_some() || settled(process.started, now) {
                    self.shell_calls.insert(key, call.clone());
                }
                call
            };
            if let Some(call) = call {
                found.insert(process.pid, call);
            }
        }
        self.shell_calls.retain(|key, _| seen.contains(key));
        found
    }
}

/// Cycle-safe process ancestry for live launches and descendant gates.
fn process_is_below(pid: u32, root: u32, table: &HashMap<u32, &ProcessEntry>) -> bool {
    let mut current = Some(pid);
    let mut seen = HashSet::new();
    while let Some(pid) = current {
        if pid == root {
            return true;
        }
        if !seen.insert(pid) {
            break;
        }
        current = table.get(&pid).and_then(|process| process.parent);
    }
    false
}

/// Resolve fixture launchers against one shared staff snapshot.
/// A launcher's actual pid is used even when a shell wrapper owns its row.
#[cfg(test)]
pub(super) fn find_worker_dispatches(processes: &[ProcessEntry]) -> HashMap<u32, WorkerDispatch> {
    let mut names = WorkerNames::default();
    let mut commands = WorkerCommands::default();
    let staff = commands
        .read(processes)
        .values()
        .flat_map(|command| [&command.direct, &command.wrapped])
        .filter_map(|launcher| match launcher {
            WorkerLauncher::Dispatch(dispatch) => dispatch.session_directory().ok(),
            WorkerLauncher::OtherCommand => None,
        })
        .map(|directory| worker_dispatch::fixture_names(directory).staff_snapshot())
        .find(|staff| !staff.edges.0.is_empty());
    if let Some(staff) = staff {
        names.set_staff(staff);
    }
    worker_dispatches(processes, &mut names, &mut commands)
}

/// Resolve fixture dispatches through the same staff snapshot as a census scan.
#[cfg(test)]
pub(super) fn find_staff_dispatches(
    processes: &[ProcessEntry],
    staff: Arc<StaffRoster>,
) -> HashMap<u32, WorkerDispatch> {
    let mut names = WorkerNames::default();
    names.set_staff(staff);
    worker_dispatches(processes, &mut names, &mut WorkerCommands::default())
}

/// Snapshot decoding belongs to shell calls; seat identity can also come from raw argv.
fn seat_sender_command(process: &ProcessEntry) -> Option<String> {
    tree::shell_command(process).or_else(|| match process.arguments.as_slice() {
        [_, flag, command, ..] if flag == SHELL_COMMAND_FLAG => Some(command.clone()),
        _ => None,
    })
}

/// A process command can contain a literal launcher or other shell work.
#[derive(Debug)]
enum WorkerLauncher {
    OtherCommand,
    Dispatch(ParsedDispatch),
}

impl WorkerLauncher {
    fn from_result(answer: Result<ParsedDispatch, NotWorkerDispatch>) -> Self {
        answer.map_or(Self::OtherCommand, Self::Dispatch)
    }
}

/// The command inputs and their two supported launcher forms belong to one process.
#[derive(Debug)]
struct WorkerCommand {
    started:   u64,
    arguments: Vec<String>,
    directory: ProcessDirectory,
    direct:    WorkerLauncher,
    wrapped:   WorkerLauncher,
}

#[derive(Debug, Default)]
struct WorkerCommands {
    processes:     HashMap<u32, WorkerCommand>,
    #[cfg(test)]
    command_reads: usize,
}

impl WorkerCommands {
    fn read(&mut self, processes: &[ProcessEntry]) -> &HashMap<u32, WorkerCommand> {
        let live: HashSet<_> = processes.iter().map(|process| process.pid).collect();
        self.processes.retain(|pid, _| live.contains(pid));
        for process in processes {
            if self.processes.get(&process.pid).is_some_and(|command| {
                command.started == process.started
                    && command.arguments == process.arguments
                    && match (&command.directory, &process.directory) {
                        (ProcessDirectory::Known(previous), Some(current)) => previous == current,
                        (ProcessDirectory::Unreadable, None) => true,
                        _ => false,
                    }
            }) {
                continue;
            }
            #[cfg(test)]
            {
                self.command_reads += 1;
            }
            let wrapped =
                seat_sender_command(process).map_or(WorkerLauncher::OtherCommand, |command| {
                    WorkerLauncher::from_result(process.directory.as_ref().map_or_else(
                        || ParsedDispatch::parse(&command),
                        |directory| ParsedDispatch::parse_at(&command, directory),
                    ))
                });
            self.processes.insert(
                process.pid,
                WorkerCommand {
                    started: process.started,
                    arguments: process.arguments.clone(),
                    directory: process
                        .directory
                        .clone()
                        .map_or(ProcessDirectory::Unreadable, ProcessDirectory::Known),
                    direct: WorkerLauncher::from_result(ParsedDispatch::from_arguments(
                        &process.arguments,
                    )),
                    wrapped,
                },
            );
        }
        &self.processes
    }
}

/// Share the staff snapshot with launcher-free naming.
fn worker_dispatches(
    processes: &[ProcessEntry],
    names: &mut WorkerNames,
    commands: &mut WorkerCommands,
) -> HashMap<u32, WorkerDispatch> {
    let table = classify::pid_table(processes);
    let commands = commands.read(processes);
    let mut found = HashMap::new();
    for process in processes {
        let Some(WorkerCommand {
            direct: WorkerLauncher::Dispatch(dispatch),
            ..
        }) = commands.get(&process.pid)
        else {
            continue;
        };
        let selected = processes
            .iter()
            .filter(|candidate| {
                candidate.arguments.iter().any(|arg| {
                    Path::new(arg)
                        .file_name()
                        .is_some_and(|name| name == CODEX_MESH_SCRIPT)
                }) && process_is_below(candidate.pid, process.pid, &table)
            })
            .find_map(|candidate| {
                candidate
                    .arguments
                    .windows(2)
                    .find(|pair| MESH_SEAT_FLAGS.contains(&pair[0].as_str()))
                    .map(|pair| (pair[1].clone(), candidate.pid))
            });
        let launcher = dispatch;
        let mut dispatch = dispatch.clone();
        if let Some((seat, child_pid)) = selected {
            dispatch.select_seat(seat);
            if let Some(WorkerCommand {
                direct: WorkerLauncher::Dispatch(child),
                ..
            }) = commands.get(&child_pid)
            {
                dispatch.select_session_directory(child);
            }
        }
        let worker = names.resolve_dispatch(&dispatch, processes, process.pid);
        found.insert(process.pid, worker.clone());
        let mut parent = process;
        let mut seen = HashSet::new();
        while seen.insert(parent.pid) {
            if commands.get(&parent.pid).is_some_and(|command| {
                matches!(&command.wrapped, WorkerLauncher::Dispatch(parsed) if parsed == launcher)
            }) {
                found.insert(parent.pid, worker.clone());
            }
            let Some(ancestor) = parent.parent.and_then(|pid| table.get(&pid)) else {
                break;
            };
            if ancestor.is_agent() {
                break;
            }
            parent = ancestor;
        }
    }
    for (&pid, command) in commands {
        let WorkerLauncher::Dispatch(dispatch) = &command.wrapped else {
            continue;
        };
        if found.contains_key(&pid) {
            continue;
        }
        found.insert(pid, names.resolve_dispatch(dispatch, processes, pid));
    }
    found
}

/// The launcher of `held`, a row tmux holds, by its pid and start: the
/// other Claude Code row among `rows` whose transcript holds the call
/// that opened it. The launcher's own process may have started later,
/// as one restarted since does.
fn search_launcher(
    processes: &[ProcessEntry],
    sessions: &[SessionRecord],
    transcripts: &HashMap<u32, PathBuf>,
    rows: &[AgentRow],
    held: &AgentRow,
) -> Option<ProcessKey> {
    let record = sessions.iter().find(|session| session.pid == held.pid);
    let directory = record.and_then(|session| session.cwd.clone()).or_else(|| {
        processes
            .iter()
            .find(|process| process.pid == held.pid)?
            .directory
            .clone()
    });
    let session = HeldSession {
        name:       &held.name,
        given_name: held.agent == Agent::Claude
            && record.is_some_and(|session| {
                session.name.as_deref().is_some_and(|name| !name.is_empty())
            }),
        directory:  directory.as_deref(),
        started:    held.process_started,
    };
    let span = session.call_span();
    let calls: Vec<(u32, BashCall)> = rows
        .iter()
        .filter(|other| other.agent == Agent::Claude && other.pid != held.pid)
        .filter_map(|other| Some((other.pid, transcripts.get(&other.pid)?)))
        .flat_map(|(pid, path)| {
            transcript::bash_calls(path, &span)
                .into_iter()
                .map(move |call| (pid, call))
        })
        .collect();
    let launcher = classify::pick_launcher(&session, &calls)?;
    rows.iter()
        .find(|other| other.pid == launcher)
        .map(|other| (other.pid, other.process_started))
}

/// The call that started the shell wrapper `process`, running `command`,
/// among the calls in `main`, its session's transcript, and in the
/// transcripts of `subagents`, the session's running subagents.
fn search_shell_call(
    process: &ProcessEntry,
    command: &str,
    main: &Path,
    subagents: &[Subagent],
) -> Option<ShellCall> {
    let span = classify::call_span(process.started);
    let theirs: Vec<(String, Vec<BashCall>)> = subagents
        .iter()
        .map(|agent| {
            (
                agent.id.clone(),
                transcript::bash_calls(&agent.transcript, &span),
            )
        })
        .collect();
    let own = transcript::bash_calls(main, &span);
    tree::match_shell_call(command, process.started, &own, &theirs)
}

/// Whether a process started at `started` is old enough, at `now`, that
/// a search that found nothing for it will find nothing later.
const fn settled(started: u64, now: u64) -> bool {
    now.saturating_sub(started) >= CALL_SEARCH_SETTLE.as_secs()
}

/// `open_rollouts` reads the session files held by one live Codex process.
fn open_rollouts(process: &ProcessEntry, home: Option<&Path>) -> Vec<(String, PathBuf)> {
    if !cfg!(target_os = "linux") {
        return Vec::new();
    }
    let Some(codex_home) = codex::session_home(process, home) else {
        return Vec::new();
    };
    let Ok(conversations) = fs::canonicalize(codex_home.join(CODEX_SESSIONS_DIRNAME)) else {
        return Vec::new();
    };
    let descriptors = Path::new(PROC_DIRNAME)
        .join(process.pid.to_string())
        .join(PROC_FD_DIRNAME);
    let mut ids: Vec<_> = fs::read_dir(descriptors)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| fs::read_link(entry.path()).ok())
        .filter(|target| target.starts_with(&conversations))
        .filter_map(|target| Some((tree::thread_id(&target)?, target)))
        .collect();
    ids.sort();
    ids.dedup_by(|left, right| left.0 == right.0);
    ids
}

/// `attach_pane_codex_identities` accepts one held staff rollout for each unproven pane.
fn attach_pane_codex_identities(
    rows: &mut [AgentRow],
    processes: &[ProcessEntry],
    home: Option<&Path>,
    staff: &StaffRoster,
) {
    for row in rows {
        if row.agent != Agent::Codex || row.session_id != SessionIdentity::Unrecorded {
            continue;
        }
        let Some(process) = processes.iter().find(|process| process.pid == row.pid) else {
            continue;
        };
        if process.is_app_server() {
            continue;
        }
        let held: Vec<_> = open_rollouts(process, home)
            .into_iter()
            .filter(|(id, _)| {
                matches!(
                    staff.session_membership(id),
                    super::hierarchy::StaffSessionMembership::Listed
                )
            })
            .collect();
        if let [(id, _)] = held.as_slice() {
            row.session_id =
                SessionIdentity::Codex(ProvenCodexThread::OpenStaffRollout(id.clone()));
        }
    }
}

/// `CodexThreads` holds one scan's app-server children and pane rollout paths.
struct CodexThreads {
    app_servers: HashMap<u32, Vec<ThreadEntry>>,
    panes:       HashMap<u32, PathBuf>,
    accounts:    HashMap<PathBuf, HashMap<String, CodexThread>>,
}

/// `codex_threads` gathers both requester sets before reading their account answers.
fn codex_threads(
    processes: &[ProcessEntry],
    rows: &[AgentRow],
    rollouts: &mut Rollouts,
    thread_rows: &mut OpenThreadRows,
    pacer: &PacerHistory,
    home: Option<&Path>,
    account_labels: &AccountLabels,
) -> CodexThreads {
    let mut by_home: HashMap<PathBuf, Vec<String>> = HashMap::new();
    let held: Vec<_> = processes
        .iter()
        .filter(|process| process.is_codex() && process.is_app_server())
        .filter_map(|process| {
            let codex_home = codex::session_home(process, home)?;
            let ids = open_rollouts(process, home);
            by_home
                .entry(codex_home.clone())
                .or_default()
                .extend(ids.iter().map(|(id, _)| id.clone()));
            Some((process, codex_home, ids))
        })
        .filter(|(_, _, ids)| !ids.is_empty())
        .collect();
    let live = thread_rows.read_scan(processes, rows, home, &by_home);
    let app_servers = held
        .into_iter()
        .map(|(process, codex_home, ids)| {
            let threads = ids
                .into_iter()
                .map(|(id, rollout)| {
                    let (name, started) = live
                        .accounts
                        .get(&codex_home)
                        .and_then(|rows| rows.get(&id))
                        .map_or_else(
                            || (WORKER_LABEL.to_owned(), process.started),
                            |thread| {
                                (
                                    thread.label().unwrap_or_else(|| WORKER_LABEL.to_owned()),
                                    thread.created_ms / 1_000,
                                )
                            },
                        );
                    let read = rollouts.read(&rollout);
                    rollouts.read_model(&rollout);
                    let service_tier = tier::requested(Some(read.clone()), &[], pacer, started);
                    ThreadEntry {
                        activity: read.activity(),
                        thread_id: CodexThreadId::Recorded(id),
                        name,
                        started,
                        service_tier,
                        account: account::account_of(process, home, account_labels),
                    }
                })
                .collect();
            (process.pid, threads)
        })
        .collect();
    CodexThreads {
        app_servers,
        panes: live.panes,
        accounts: live.accounts,
    }
}

/// `app_server_threads` preserves the existing account-isolation test interface.
#[cfg(all(test, target_os = "linux"))]
fn app_server_threads(
    processes: &[ProcessEntry],
    rollouts: &mut Rollouts,
    thread_rows: &mut OpenThreadRows,
    pacer: &PacerHistory,
    home: Option<&Path>,
    account_labels: &AccountLabels,
) -> HashMap<u32, Vec<ThreadEntry>> {
    codex_threads(
        processes,
        &[],
        rollouts,
        thread_rows,
        pacer,
        home,
        account_labels,
    )
    .app_servers
}

/// Every session record in `directory` that parses. The directory holds
/// other files beside the records, and a record being written as it is
/// read fails to parse; both are skipped.
fn read_sessions(directory: &Path) -> Vec<SessionRecord> {
    let Ok(entries) = fs::read_dir(directory) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == SESSION_RECORD_EXTENSION)
        })
        .filter_map(|path| fs::read_to_string(path).ok())
        .filter_map(|text| serde_json::from_str(&text).ok())
        .collect()
}

#[cfg(test)]
thread_local! {
    static VERIFICATION_ARGUMENT_CHECKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn is_verification_process(process: &ProcessEntry) -> bool {
    #[cfg(test)]
    VERIFICATION_ARGUMENT_CHECKS.with(|checks| checks.set(checks.get() + 1));
    process.arguments.iter().any(|arg| {
        Path::new(arg)
            .file_name()
            .is_some_and(|name| name == VERIFY_SCRIPT)
    })
}

/// Process rows retained by one local scanner across census ticks.
#[derive(Debug, Default)]
struct ProcessCensus {
    system:           System,
    snapshot:         ProcessSnapshot,
    #[cfg(test)]
    discoveries:      usize,
    #[cfg(test)]
    detail_refreshes: usize,
    #[cfg(test)]
    read_starts:      Vec<Instant>,
    #[cfg(test)]
    discovery:        TestProcessDiscovery,
}

#[cfg(test)]
#[derive(Debug, Default)]
enum TestProcessDiscovery {
    #[default]
    FullTable,
    FixtureProcesses {
        pids: Vec<Pid>,
    },
}

#[derive(Debug, Default)]
enum ProcessSnapshot {
    #[default]
    Unread,
    Current {
        rows:        Arc<[ProcessEntry]>,
        revisions:   HashMap<Pid, ProcessRevision>,
        descendants: HashSet<Pid>,
        sessions:    HashSet<(u32, String)>,
        recovery:    Instant,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ProcessBirth {
    #[cfg(target_os = "linux")]
    KernelTicks(u64),
    #[cfg(target_os = "macos")]
    UnixTime {
        seconds:      u64,
        microseconds: u64,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ProcessDirectory {
    Known(PathBuf),
    Unreadable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ExecObservation {
    #[cfg(target_os = "linux")]
    MemoryLayout {
        code_start:  u64,
        code_end:    u64,
        stack_start: u64,
    },
    #[cfg(target_os = "macos")]
    NameOnly,
}

/// Only identity, image and directory changes require command or environment reads.
#[derive(Clone, Debug, Eq, PartialEq)]
enum ProcessRevision {
    Observed {
        birth:     ProcessBirth,
        image:     ExecObservation,
        name:      OsString,
        parent:    u32,
        directory: ProcessDirectory,
    },
    Exited,
    Unreadable,
}

impl ProcessCensus {
    #[cfg(test)]
    fn for_test() -> Self {
        Self {
            discovery: TestProcessDiscovery::FixtureProcesses { pids: Vec::new() },
            ..Self::default()
        }
    }

    fn read(&mut self, sessions: &[SessionRecord]) -> Arc<[ProcessEntry]> {
        #[cfg(test)]
        self.read_starts.push(Instant::now());
        self.read_at(sessions, Instant::now())
    }

    fn read_at(&mut self, sessions: &[SessionRecord], now: Instant) -> Arc<[ProcessEntry]> {
        let session_keys: HashSet<_> = sessions
            .iter()
            .map(|session| (session.pid, session.session_id.clone()))
            .collect();
        let recover = match &self.snapshot {
            ProcessSnapshot::Unread => true,
            ProcessSnapshot::Current { recovery, .. } => now >= *recovery,
        };
        if recover {
            let rows = self.discover(sessions);
            let (tracked, descendants) = process_interests(&rows, sessions);
            self.snapshot = ProcessSnapshot::Current {
                rows: Arc::clone(&rows),
                revisions: tracked
                    .into_iter()
                    .map(|pid| (pid, process_revision(pid)))
                    .collect(),
                descendants,
                sessions: session_keys,
                recovery: now + PROCESS_RECOVERY_INTERVAL,
            };
            return rows;
        }
        let ProcessSnapshot::Current {
            rows,
            revisions,
            descendants,
            sessions: previous_sessions,
            ..
        } = &mut self.snapshot
        else {
            return Arc::from([]);
        };
        let new_sessions: HashSet<_> = session_keys
            .difference(previous_sessions)
            .map(|(pid, _)| Pid::from_u32(*pid))
            .collect();
        let sessions_changed = *previous_sessions != session_keys;
        *previous_sessions = session_keys;
        let mut candidates: HashSet<_> = revisions.keys().copied().collect();
        candidates.extend(sessions.iter().map(|session| Pid::from_u32(session.pid)));
        for &pid in descendants.iter() {
            candidates.extend(process_children(pid));
        }
        let mut changed = Vec::new();
        let mut removed = HashSet::new();
        let mut names = HashMap::new();
        for pid in candidates {
            let revision = process_revision(pid);
            match &revision {
                ProcessRevision::Exited => {
                    removed.insert(pid);
                    revisions.remove(&pid);
                },
                ProcessRevision::Unreadable => {},
                ProcessRevision::Observed { name, .. } => {
                    if new_sessions.contains(&pid) || revisions.get(&pid) != Some(&revision) {
                        changed.push(pid);
                        names.insert(pid, name.clone());
                        revisions.insert(pid, revision);
                    }
                },
            }
        }
        if changed.is_empty() && removed.is_empty() {
            if sessions_changed {
                retain_process_interests(rows, sessions, revisions, descendants);
            }
            return Arc::clone(rows);
        }
        #[cfg(test)]
        {
            self.detail_refreshes += changed.len();
        }
        let mut updated = refresh_changed_processes(&mut self.system, &changed, &removed, names);
        let mut next: Vec<_> = rows
            .iter()
            .filter(|row| !removed.contains(&Pid::from_u32(row.pid)))
            .map(|row| updated.remove(&row.pid).unwrap_or_else(|| row.clone()))
            .collect();
        next.extend(updated.into_values());
        retain_process_interests(&next, sessions, revisions, descendants);
        *rows = next.into();
        Arc::clone(rows)
    }

    /// Recovery discovers roots missed by the per-process probes.
    fn discover(&mut self, sessions: &[SessionRecord]) -> Arc<[ProcessEntry]> {
        #[cfg(test)]
        {
            self.discoveries += 1;
        }
        #[cfg(not(test))]
        {
            process_table(&mut self.system, sessions).into()
        }
        #[cfg(test)]
        match &self.discovery {
            TestProcessDiscovery::FullTable => process_table(&mut self.system, sessions).into(),
            TestProcessDiscovery::FixtureProcesses { pids: discovered } => {
                let mut pids = vec![Pid::from_u32(std::process::id())];
                pids.extend(discovered);
                pids.extend(sessions.iter().map(|session| Pid::from_u32(session.pid)));
                pids.sort_unstable();
                pids.dedup();
                self.system.refresh_processes_specifics(
                    ProcessesToUpdate::Some(&pids),
                    true,
                    ProcessRefreshKind::nothing()
                        .without_tasks()
                        .with_cmd(UpdateKind::Always)
                        .with_cwd(UpdateKind::Always)
                        .with_environ(UpdateKind::Always),
                );
                let names = CurrentProcessNames::read(&self.system);
                self.system
                    .processes()
                    .values()
                    .map(|process| process_entry(process, &names))
                    .collect::<Vec<_>>()
                    .into()
            },
        }
    }
}

/// Keep probes for current session trees and their ancestors.
fn retain_process_interests(
    rows: &[ProcessEntry],
    sessions: &[SessionRecord],
    revisions: &mut HashMap<Pid, ProcessRevision>,
    descendants: &mut HashSet<Pid>,
) {
    let (tracked, next_descendants) = process_interests(rows, sessions);
    revisions.retain(|pid, _| tracked.contains(pid));
    for pid in tracked {
        revisions
            .entry(pid)
            .or_insert_with(|| process_revision(pid));
    }
    *descendants = next_descendants;
}

/// Read detailed fields only for changed identities and remove exited identities.
fn refresh_changed_processes(
    system: &mut System,
    changed: &[Pid],
    removed: &HashSet<Pid>,
    names: HashMap<Pid, OsString>,
) -> HashMap<u32, ProcessEntry> {
    let refresh: Vec<_> = changed.iter().chain(removed.iter()).copied().collect();
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&refresh),
        true,
        ProcessRefreshKind::nothing()
            .without_tasks()
            .with_cmd(UpdateKind::Always)
            .with_cwd(UpdateKind::Always)
            .with_environ(UpdateKind::Always),
    );
    let names = CurrentProcessNames(names);
    changed
        .iter()
        .filter_map(|pid| {
            system
                .process(*pid)
                .map(|process| (pid.as_u32(), process_entry(process, &names)))
        })
        .collect()
}

/// Track displayed roots, their descendants and ancestors, excluding unrelated jobs.
fn process_interests(
    rows: &[ProcessEntry],
    sessions: &[SessionRecord],
) -> (HashSet<Pid>, HashSet<Pid>) {
    let mut descendants: HashSet<_> = rows
        .iter()
        .filter(|row| {
            row.is_codex()
                || row.name == CLAUDE_AGENT
                || sessions.iter().any(|session| session.pid == row.pid)
        })
        .map(|row| Pid::from_u32(row.pid))
        .collect();
    loop {
        let before = descendants.len();
        for row in rows {
            if row
                .parent
                .is_some_and(|parent| descendants.contains(&Pid::from_u32(parent)))
            {
                descendants.insert(Pid::from_u32(row.pid));
            }
        }
        if descendants.len() == before {
            break;
        }
    }
    let mut tracked = descendants.clone();
    let parents: HashMap<_, _> = rows
        .iter()
        .map(|row| (Pid::from_u32(row.pid), row.parent))
        .collect();
    for &pid in &descendants {
        let mut current = pid;
        while let Some(Some(parent)) = parents.get(&current) {
            current = Pid::from_u32(*parent);
            if !tracked.insert(current) {
                break;
            }
        }
    }
    (tracked, descendants)
}

/// Linux exposes child births without reading the full process table.
#[cfg(target_os = "linux")]
fn process_children(pid: Pid) -> Vec<Pid> {
    fs::read_to_string(format!("/proc/{pid}/task/{pid}/children"))
        .unwrap_or_default()
        .split_whitespace()
        .filter_map(|pid| pid.parse().ok().map(Pid::from_u32))
        .collect()
}

/// New macOS descendants are recovered with the bounded full discovery scan.
#[cfg(not(target_os = "linux"))]
const fn process_children(_: Pid) -> Vec<Pid> { Vec::new() }

#[cfg(target_os = "linux")]
fn process_revision(pid: Pid) -> ProcessRevision {
    let bytes = match fs::read(format!("/proc/{pid}/stat")) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => return ProcessRevision::Exited,
        Err(_) => return ProcessRevision::Unreadable,
    };
    let Some(open) = bytes.iter().position(|byte| *byte == b'(') else {
        return ProcessRevision::Unreadable;
    };
    let Some(close) = bytes.iter().rposition(|byte| *byte == b')') else {
        return ProcessRevision::Unreadable;
    };
    let fields: Vec<_> = bytes[close + 1..]
        .split(u8::is_ascii_whitespace)
        .filter(|field| !field.is_empty())
        .collect();
    let number = |index: usize| {
        fields
            .get(index)
            .and_then(|field| std::str::from_utf8(field).ok())?
            .parse::<u64>()
            .ok()
    };
    let (Some(started), Some(code_start), Some(code_end), Some(stack_start), Some(parent)) =
        (number(19), number(23), number(24), number(25), number(1))
    else {
        return ProcessRevision::Unreadable;
    };
    ProcessRevision::Observed {
        birth:     ProcessBirth::KernelTicks(started),
        image:     ExecObservation::MemoryLayout {
            code_start,
            code_end,
            stack_start,
        },
        name:      OsString::from_vec(bytes[open + 1..close].to_vec()),
        parent:    u32::try_from(parent).unwrap_or_default(),
        directory: fs::read_link(format!("/proc/{pid}/cwd"))
            .map_or(ProcessDirectory::Unreadable, ProcessDirectory::Known),
    }
}

/// libproc's typed reader fills the native directory record behind its safe API.
#[cfg(target_os = "macos")]
#[repr(transparent)]
struct MacProcessDirectories(libc::proc_vnodepathinfo);

#[cfg(target_os = "macos")]
impl PIDInfo for MacProcessDirectories {
    fn flavor() -> PidInfoFlavor { PidInfoFlavor::VNodePathInfo }
}

#[cfg(target_os = "macos")]
fn process_revision(pid: Pid) -> ProcessRevision {
    let Ok(native_pid) = i32::try_from(pid.as_u32()) else {
        return ProcessRevision::Unreadable;
    };
    let Ok(identity) = libproc::proc_pid::pidinfo::<libproc::bsd_info::BSDInfo>(native_pid, 0)
    else {
        return if std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
            ProcessRevision::Exited
        } else {
            ProcessRevision::Unreadable
        };
    };
    // Preserve the extended sixteenth character used by ps ucomm.
    let short_name: Vec<_> = identity
        .pbi_comm
        .iter()
        .take_while(|byte| **byte != 0)
        .copied()
        .collect();
    let extended_name: Vec<_> = identity
        .pbi_name
        .iter()
        .take_while(|byte| **byte != 0)
        .copied()
        .collect();
    let name = if short_name.len() == 15 && extended_name.starts_with(&short_name) {
        &extended_name[..extended_name.len().min(16)]
    } else {
        &short_name
    }
    .iter()
    .map(|byte| byte.to_ne_bytes()[0])
    .collect::<Vec<_>>();
    let directory = libproc::proc_pid::pidinfo::<MacProcessDirectories>(native_pid, 0).map_or(
        ProcessDirectory::Unreadable,
        |paths| {
            let bytes = paths
                .0
                .pvi_cdir
                .vip_path
                .iter()
                .flatten()
                .take_while(|byte| **byte != 0)
                .map(|byte| byte.to_ne_bytes()[0])
                .collect::<Vec<_>>();
            ProcessDirectory::Known(PathBuf::from(String::from_utf8_lossy(&bytes).into_owned()))
        },
    );
    ProcessRevision::Observed {
        birth: ProcessBirth::UnixTime {
            seconds:      identity.pbi_start_tvsec,
            microseconds: identity.pbi_start_tvusec,
        },
        image: ExecObservation::NameOnly,
        name: OsString::from(String::from_utf8_lossy(&name).into_owned()),
        parent: identity.pbi_ppid,
        directory,
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn process_revision(_: Pid) -> ProcessRevision { ProcessRevision::Unreadable }

/// The process table the classification and the tree read.
///
/// Three passes. The first reads every process's name, parent and
/// start time, which is all the ancestry walk needs. The second reads
/// the command line and directory, and only for the processes
/// [`detailed_pids`] names. The third reads every Codex and Claude Code
/// environment for its configuration home, including version-named Claude
/// binaries identified by session records. Codex also supplies `CLAUDE_PID`.
fn process_table(system: &mut System, sessions: &[SessionRecord]) -> Vec<ProcessEntry> {
    let names = startup_log::startup_stage("proc_ancestry", || refresh_process_identities(system));
    let detailed = startup_log::startup_stage("proc_detail_selection", || {
        detailed_pids(system, sessions, &names)
    });
    startup_log::startup_stage("proc_commands_directories", || {
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&detailed),
            false,
            ProcessRefreshKind::nothing()
                .without_tasks()
                .with_cmd(UpdateKind::Always)
                .with_cwd(UpdateKind::Always),
        )
    });
    let session_pids: HashSet<u32> = sessions.iter().map(|session| session.pid).collect();
    let agents: Vec<Pid> = system
        .processes()
        .values()
        .filter(|process| {
            names.of(process) == CODEX_AGENT
                || names.of(process) == CLAUDE_AGENT
                || session_pids.contains(&process.pid().as_u32())
        })
        .map(Process::pid)
        .collect();
    startup_log::startup_stage("proc_agent_environments", || {
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&agents),
            false,
            ProcessRefreshKind::nothing()
                .without_tasks()
                .with_environ(UpdateKind::Always),
        )
    });
    system
        .processes()
        .values()
        .map(|process| process_entry(process, &names))
        .collect()
}

fn process_entry(process: &Process, names: &CurrentProcessNames) -> ProcessEntry {
    let mut entry = ProcessEntry {
        pid:         process.pid().as_u32(),
        parent:      process.parent().map(Pid::as_u32),
        name:        names.of(process).to_string_lossy().into_owned(),
        arguments:   process
            .cmd()
            .iter()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect(),
        started:     process.start_time(),
        directory:   process.cwd().map(Path::to_path_buf),
        config_home: ConfigHome::Unread,
        claude_pid:  if names.of(process) == CODEX_AGENT {
            claude_pid(process.environ())
        } else {
            None
        },
    };
    // sysinfo leaves the environment empty when it could not read it.
    if !process.environ().is_empty() {
        entry.config_home = account::config_home_of(&entry, process.environ());
    }
    entry
}

/// Names observed now, rather than sysinfo's names retained from process birth.
/// A failed OS read falls back to the retained name for that process.
struct CurrentProcessNames(HashMap<Pid, OsString>);

impl CurrentProcessNames {
    fn of<'a>(&'a self, process: &'a Process) -> &'a OsStr {
        self.0
            .get(&process.pid())
            .map_or_else(|| process.name(), OsString::as_os_str)
    }

    #[cfg(target_os = "linux")]
    fn read(system: &System) -> Self {
        Self(
            system
                .processes()
                .keys()
                .filter_map(|pid| {
                    let mut name = fs::read(format!("/proc/{pid}/comm")).ok()?;
                    if name.last() == Some(&b'\n') {
                        name.pop();
                    }
                    Some((*pid, OsString::from_vec(name)))
                })
                .collect(),
        )
    }

    #[cfg(target_os = "macos")]
    fn read(_system: &System) -> Self {
        let names = Command::new("/bin/ps")
            .args(["-axo", "pid=,ucomm="])
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
            .unwrap_or_default();
        Self(
            names
                .lines()
                .filter_map(|line| {
                    let (pid, name) = line.trim().split_once(char::is_whitespace)?;
                    Some((
                        Pid::from_u32(pid.parse().ok()?),
                        OsString::from(name.trim()),
                    ))
                })
                .collect(),
        )
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    fn read(_system: &System) -> Self { Self(HashMap::new()) }
}

/// Refresh only the ancestry fields consumed by the census. Executable paths
/// are not census inputs. Read current names separately so exec cannot leave a
/// cached launcher name, and never replace the retained System to update a name.
fn refresh_process_identities(system: &mut System) -> CurrentProcessNames {
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().without_tasks(),
    );
    CurrentProcessNames::read(system)
}

/// The pid `CLAUDE_PID` names in `environment`, one `NAME=value` per
/// entry; none when it is not set or is not a pid.
fn claude_pid(environment: &[OsString]) -> Option<u32> {
    environment.iter().find_map(|variable| {
        variable
            .to_str()?
            .strip_prefix(CLAUDE_PID_VARIABLE)?
            .strip_prefix('=')?
            .parse()
            .ok()
    })
}

/// Every window `KWin` lists, from a script it runs; none where it does
/// not answer.
#[cfg(target_os = "linux")]
fn read_windows() -> Option<Vec<WindowEntry>> {
    startup_log::startup_stage("kwin_evaluate", || {
        desktop::parse_windows(&tui_pane::kwin_evaluate(WINDOW_LIST_EXPRESSION)?)
    })
}

/// No windows off Linux, where there is no `KWin` to ask.
#[cfg(not(target_os = "linux"))]
const fn read_windows() -> Option<Vec<WindowEntry>> { None }

/// The default tmux server's panes and clients, asked of `program`;
/// none where either list fails.
fn read_tmux(program: &str) -> Option<TmuxLayout> {
    let panes =
        startup_log::startup_stage("tmux_list_panes", || tmux_output(program, &TMUX_LIST_PANES))?;
    let clients = startup_log::startup_stage("tmux_list_clients", || {
        tmux_output(program, &TMUX_LIST_CLIENTS)
    })?;
    Some(TmuxLayout::parse(&panes, &clients))
}

/// What `program` prints to standard output run with `arguments`; none
/// where it cannot be run or exits with a failure.
fn tmux_output(program: &str, arguments: &[&str]) -> Option<String> {
    let output = Command::new(program)
        .args(arguments)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The processes whose command line and directory the second pass
/// reads, and every process above one of them, then every `sleep`.
///
/// A row can come from each session record's process and every
/// `codex`. The tree reads the command line of each child of a Claude
/// Code process, which says whether the child is its shell wrapper, so
/// every process named `claude` and each such child are read too. On
/// macOS a Claude Code process is named for its version, so an ancestor
/// is only recognised as Claude Code by its command line. The tree
/// reads a `sleep`'s arguments for the span of a shell timer, and
/// nothing above it for that, so each [`ACTIVITY_PROCESS_NAMES`] is read alone.
fn detailed_pids(
    system: &System,
    sessions: &[SessionRecord],
    names: &CurrentProcessNames,
) -> Vec<Pid> {
    let processes = system.processes();
    let mut children: HashMap<Pid, Vec<Pid>> = HashMap::new();
    for process in processes.values() {
        if let Some(parent) = process.parent() {
            children.entry(parent).or_default().push(process.pid());
        }
    }
    let claude: Vec<Pid> = sessions
        .iter()
        .map(|session| Pid::from_u32(session.pid))
        .chain(
            processes
                .values()
                .filter(|process| names.of(process) == CLAUDE_AGENT)
                .map(Process::pid),
        )
        .collect();
    let starts = claude
        .iter()
        .copied()
        .chain(
            claude
                .iter()
                .filter_map(|pid| children.get(pid))
                .flatten()
                .copied(),
        )
        .chain(
            processes
                .values()
                .filter(|process| names.of(process) == CODEX_AGENT)
                .map(Process::pid),
        );
    let mut detailed = HashSet::new();
    for start in starts {
        let mut next = Some(start);
        while let Some(pid) = next {
            // A pid already taken was reached from another start, and so
            // was everything above it.
            if !detailed.insert(pid) {
                break;
            }
            next = processes.get(&pid).and_then(Process::parent);
        }
    }
    detailed.extend(
        processes
            .values()
            .filter(|process| {
                ACTIVITY_PROCESS_NAMES
                    .iter()
                    .any(|name| names.of(process) == *name)
            })
            .map(Process::pid),
    );
    detailed.into_iter().collect()
}

/// `attach_astra_memberships` adds `AstraMembership` through `SessionIdentity::with_astra`.
fn attach_astra_memberships(
    rows: &mut [AgentRow],
    staff: &StaffRoster,
    processes: &[ProcessEntry],
    home: Option<&Path>,
    herdr: &HerdrNames,
) {
    for row in rows {
        let member = row
            .session_id
            .ids()
            .next()
            .map(|id| staff.astra_membership(id));
        if let Some(AstraWorkerMembership::Member(member)) = member
            && processes
                .iter()
                .find(|process| process.pid == row.pid)
                .and_then(|process| codex::session_home(process, home))
                .is_some_and(|account| account == member.home())
        {
            row.session_id = std::mem::take(&mut row.session_id)
                .with_astra(member.with_work(herdr.work(row.pid, row.process_started)));
        }
    }
}

fn apply_astra_work(rows: &mut [AgentRow], herdr: &HerdrNames) {
    for row in rows {
        if let AstraSessionMembership::Member(member) = row.session_id.astra_membership() {
            let member = member
                .clone()
                .with_work(herdr.work(row.pid, row.process_started));
            row.activity = member.activity(row.activity);
            row.session_id = std::mem::take(&mut row.session_id).with_astra(member);
        }
    }
}

/// A live Astra process remains a session even beneath another agent process.
fn include_astra_processes(
    rows: &mut Vec<AgentRow>,
    processes: &[ProcessEntry],
    home: Option<&Path>,
    staff: &StaffRoster,
    labels: &AccountLabels,
) {
    for process in processes
        .iter()
        .filter(|process| process.is_codex() && !process.is_app_server())
    {
        if rows.iter().any(|row| row.pid == process.pid) {
            continue;
        }
        let ids: Vec<_> = match codex::resume_request(process) {
            ResumeRequest::NamedId(id) => vec![id.to_owned()],
            _ => open_rollouts(process, home)
                .into_iter()
                .map(|(id, _)| id)
                .collect(),
        };
        let matched = ids.iter().any(|id| {
            matches!(
                staff.session_membership(id),
                super::hierarchy::StaffSessionMembership::Listed
            )
        });
        if matched {
            rows.extend(classify::agent_rows_with_threads(
                std::slice::from_ref(process),
                &[],
                &HashMap::new(),
                home,
                labels,
            ));
        }
    }
}

impl LocalScanner {
    /// The first explicit lookup supplies the thread link and its requested fast mark.
    fn attach_astra_threads(
        &mut self,
        rows: &mut [AgentRow],
        accounts: &HashMap<PathBuf, HashMap<String, CodexThread>>,
        pacer: &PacerHistory,
    ) {
        for row in rows {
            let AstraSessionMembership::Member(member) = row.session_id.astra_membership() else {
                continue;
            };
            let Some(thread) = accounts
                .get(member.home())
                .and_then(|threads| threads.get(member.session()))
            else {
                continue;
            };
            self.codex_links
                .insert(row.pid, CodexSessionLink::Named(thread.clone()));
            let read = self.rollouts.read(&thread.rollout);
            let tier = tier::requested(Some(read), &[], pacer, row.started);
            if tier != ServiceTier::Unrecorded {
                row.service_tier = tier;
            }
        }
    }
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    #[cfg(unix)]
    use std::io::BufRead;
    #[cfg(unix)]
    use std::process::Child;
    use std::time::Duration;

    #[cfg(unix)]
    use census_test_support::StdinEchoExecutable;
    use chrono::DateTime;
    use serde_json::json;
    use tempfile::TempDir;

    use super::*;
    use crate::census::Account;
    use crate::census::HierarchySource;
    use crate::census::RoleFallback;
    use crate::census::SessionIdentity;
    use crate::census::StaffRowNames;
    use crate::census::herdr::HerdrNames;
    use crate::census::model::ModelName;
    use crate::census::model::ReasoningEffort;

    const SCAN_REQUEST_IDS: [&str; 3] = [
        "01a18dbb-7e8f-7641-b3dd-bb2071ab9d80",
        "01a18dbb-7e8f-7641-b3dd-bb2071ab9d81",
        "01a18dbb-7e8f-7641-b3dd-bb2071ab9d82",
    ];

    struct LiveThreadScanFixture {
        home: TempDir,
    }

    impl LiveThreadScanFixture {
        fn new() -> Self {
            let home = TempDir::new().expect("scan home");
            let sessions = home.path().join(".codex/sessions");
            fs::create_dir_all(&sessions).expect("rollout directory");
            let database = rusqlite::Connection::open(home.path().join(".codex/state_6.sqlite"))
                .expect("scan database");
            database.execute_batch("CREATE TABLE threads(id TEXT PRIMARY KEY, cwd TEXT,
                created_at_ms INTEGER, name TEXT, first_user_message TEXT, rollout_path TEXT, originator TEXT)")
                .expect("thread schema");
            for id in SCAN_REQUEST_IDS {
                let path = sessions.join(format!("rollout-{id}.jsonl"));
                fs::write(&path, format!("{}\n{}\n{}\n{}\n",
                    json!({"type":"session_meta","payload":{"id":id,"cwd":"/work/fixture","timestamp":"2026-11-07T00:00:00Z"}}),
                    json!({"type":"response_item","payload":{"role":"user","content":[{"text":"work"}]}}),
                    json!({"type":"turn_context","payload":{"model":"gpt-6.1-sol","effort":"high"}}),
                    json!({"type":"event_msg","payload":{"type":"task_started"}})))
                    .expect("rollout records");
                database.execute("INSERT INTO threads VALUES(?1,'/work/fixture',100000,?1,'work',?2,'codex-tui')",
                    rusqlite::params![id,path.to_string_lossy()]).expect("thread row");
            }
            fs::write(
                home.path().join("staff.json"),
                json!({"schema":2,"rows":[]}).to_string(),
            )
            .expect("isolated staff file");
            Self { home }
        }

        fn process(&self, pid: u32, arguments: &[&str]) -> ProcessEntry {
            ProcessEntry {
                pid,
                parent: Some(1),
                name: "codex".into(),
                arguments: arguments.iter().map(|word| (*word).to_owned()).collect(),
                started: 100,
                directory: Some("/work/fixture".into()),
                claude_pid: None,
                config_home: ConfigHome::Named(self.home.path().join(".codex")),
            }
        }

        fn scanner(&self) -> LocalScanner {
            LocalScanner::new(AccountLabels::default()).with_staff_file(
                crate::census::StaffFileSetting::new(HierarchySource::File(
                    self.home
                        .path()
                        .join("staff.json")
                        .to_string_lossy()
                        .into_owned(),
                )),
                StaffFileReader::default(),
            )
        }

        fn revise(&self) {
            rusqlite::Connection::open(self.home.path().join(".codex/state_6.sqlite"))
                .expect("scan database")
                .execute("UPDATE threads SET created_at_ms=created_at_ms+1", [])
                .expect("new database revision");
        }
    }

    #[test]
    fn a_scan_revision_query_excludes_a_pane_that_closed_before_the_scan() {
        let fixture = LiveThreadScanFixture::new();
        let closed = fixture.process(40, &["codex", "resume", SCAN_REQUEST_IDS[0]]);
        let live = fixture.process(41, &["codex", "resume", SCAN_REQUEST_IDS[1]]);
        let mut scanner = fixture.scanner();
        scanner.scan_details_for_test(&[closed, live.clone()], &[], fixture.home.path());
        let reads = scanner.open_thread_rows.database_reads;
        let queries = scanner.open_thread_rows.queried_ids.len();
        fixture.revise();
        let rows = scanner.scan_details_for_test(&[live], &[], fixture.home.path());
        assert_eq!(
            rows.iter()
                .find(|row| row.pid == 41)
                .expect("live pane")
                .model,
            ModelName::Sol
        );
        assert_eq!(scanner.open_thread_rows.database_reads - reads, 1);
        assert_eq!(
            &scanner.open_thread_rows.queried_ids[queries..],
            &[HashSet::from([SCAN_REQUEST_IDS[1].to_owned()])],
            "the first query in the production scan excludes the departed pane"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_cold_scan_queries_pane_and_app_server_requests_together() {
        let fixture = LiveThreadScanFixture::new();
        let held = fs::File::open(
            fixture
                .home
                .path()
                .join(".codex/sessions")
                .join(format!("rollout-{}.jsonl", SCAN_REQUEST_IDS[2])),
        )
        .expect("held app-server rollout");
        let pane = fixture.process(41, &["codex", "resume", SCAN_REQUEST_IDS[1]]);
        let server = fixture.process(std::process::id(), &["codex", "app-server"]);
        let mut scanner = fixture.scanner();
        scanner.scan_details_for_test(&[pane, server], &[], fixture.home.path());
        assert_eq!(
            scanner.open_thread_rows.database_reads, 1,
            "a cold production scan reads one complete account answer"
        );
        assert_eq!(
            scanner.open_thread_rows.queried_ids.as_slice(),
            &[HashSet::from([
                SCAN_REQUEST_IDS[1].to_owned(),
                SCAN_REQUEST_IDS[2].to_owned()
            ])]
        );
        drop(held);
    }

    #[test]
    fn idle_verification_arguments_are_checked_once_per_process() {
        let processes: Vec<_> = (1..=3)
            .map(|pid| entry(pid, 0, "claude", 1, &["claude"]))
            .collect();
        let mut rows: Vec<_> = (1..=3).map(|pid| claude_row(pid, "agent", 1)).collect();
        VERIFICATION_ARGUMENT_CHECKS.with(|checks| checks.set(0));
        scanner().attach_activity_and_names(&processes, &HashMap::new(), &mut rows);
        assert_eq!(
            VERIFICATION_ARGUMENT_CHECKS.with(std::cell::Cell::get),
            processes.len(),
            "argument classification must not repeat for every agent row"
        );
    }

    #[test]
    fn idle_process_table_does_not_read_unused_executable_paths() {
        let pid = Pid::from_u32(std::process::id());
        let mut census = ProcessCensus::for_test();
        census.system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[pid]),
            true,
            ProcessRefreshKind::nothing().without_tasks(),
        );
        assert!(
            census
                .system
                .process(pid)
                .expect("current process")
                .exe()
                .is_none()
        );
        for _ in 0..2 {
            let entries = census.read(&[]);
            assert!(entries.iter().any(|entry| entry.pid == pid.as_u32()));
            let paths = census
                .system
                .processes()
                .values()
                .filter(|process| process.exe().is_some())
                .count();
            assert_eq!(paths, 0, "the census does not consume executable paths");
        }
    }

    #[cfg(unix)]
    fn agent_reply(
        input: &mut impl std::io::Write,
        output: &mut impl std::io::BufRead,
        command: &[u8],
    ) -> std::io::Result<String> {
        input.write_all(command)?;
        input.flush()?;
        let mut reply = String::new();
        output.read_line(&mut reply)?;
        Ok(reply)
    }

    #[cfg(unix)]
    fn same_pid_exec_launcher(binary: &StdinEchoExecutable, directory: &Path) -> Command {
        let launcher = "import os,sys; print('launcher',flush=True); sys.stdin.readline(); os.chdir(sys.argv[2]); print('moved',flush=True); sys.stdin.readline(); print('executing',flush=True); os.execv(sys.argv[1],[sys.argv[1],*sys.argv[3:]])";
        let mut command = Command::new("python3");
        command
            .args(["-u", "-c", launcher])
            .arg(binary.path())
            .arg(directory)
            .args(binary.arguments())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped());
        command
    }

    #[cfg(unix)]
    #[test]
    fn idle_process_table_recognizes_an_agent_after_same_pid_exec()
    -> Result<(), Box<dyn std::error::Error>> {
        struct ChildUntilDrop(Child);
        impl Drop for ChildUntilDrop {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let directory = tempfile::tempdir()?;
        let executable =
            census_test_support::StdinEchoExecutable::copy_to(directory.path(), "codex")?;
        let binary = executable.path();
        let mut child =
            ChildUntilDrop(same_pid_exec_launcher(&executable, directory.path()).spawn()?);
        let mut input = child.0.stdin.take().ok_or("child stdin")?;
        let mut output = std::io::BufReader::new(child.0.stdout.take().ok_or("child stdout")?);
        let mut line = String::new();
        output.read_line(&mut line)?;
        assert_eq!(line.trim(), "launcher");
        let pid = Pid::from_u32(child.0.id());
        let sessions = [SessionRecord {
            pid:        pid.as_u32(),
            session_id: "fixture".into(),
            cwd:        None,
            name:       None,
            status:     None,
            entrypoint: None,
        }];
        let mut census = ProcessCensus::for_test();
        let entries = census.read(&sessions);
        let started = entries
            .iter()
            .find(|row| row.pid == pid.as_u32())
            .ok_or("live launcher")?
            .started;
        line = agent_reply(&mut input, &mut output, b"move\n")?;
        assert_eq!(line.trim(), "moved");
        let entries = census.read(&sessions);
        assert_eq!(
            entries
                .iter()
                .find(|row| row.pid == pid.as_u32())
                .ok_or("moved launcher")?
                .directory
                .as_deref(),
            Some(directory.path().canonicalize()?.as_path())
        );
        assert_eq!(
            census.discoveries, 1,
            "a cwd change patches the existing table"
        );
        line = agent_reply(&mut input, &mut output, b"exec\n")?;
        assert_eq!(line.trim(), "executing");
        line = agent_reply(&mut input, &mut output, b"agent\n")?;
        assert_eq!(
            line.trim(),
            "agent",
            "the copied agent executable is running"
        );
        let entries = census.read(&sessions);
        let agent = entries
            .iter()
            .find(|entry| entry.pid == pid.as_u32())
            .ok_or("live agent")?;
        assert_eq!(agent.started, started, "exec retains PID and start time");
        assert!(
            agent.is_codex(),
            "exec must replace the cached launcher name: {}",
            agent.name
        );
        assert!(
            agent
                .arguments
                .iter()
                .any(|argument| argument == &binary.to_string_lossy())
        );
        assert_eq!(
            census.discoveries, 1,
            "exec does not enumerate the process table"
        );
        assert!(
            census.detail_refreshes >= 2,
            "cwd and exec refresh their changed row"
        );
        child.0.kill()?;
        child.0.wait()?;
        assert!(
            !census
                .read(&sessions)
                .iter()
                .any(|row| row.pid == pid.as_u32()),
            "an exited process leaves the next census"
        );
        assert_eq!(census.discoveries, 1, "exit removes only the departed row");
        Ok(())
    }

    /// Only `.json` files that parse are records: the key files beside
    /// them and a record caught half written are skipped.
    #[test]
    fn only_json_files_that_parse_are_read() {
        let directory = tempfile::tempdir().expect("a temporary directory should open");
        let write = |name: &str, text: &str| {
            fs::write(directory.path().join(name), text).expect("the fixture should write");
        };
        write(
            "428044.json",
            r#"{"pid":428044,"sessionId":"9f1c","cwd":"/home/natepiano/rust/handler","name":"enh/handler","status":"busy","startedAt":1790000000000,"kind":"interactive","procStart":"123","parentSessionId":null}"#,
        );
        write("46560.json", r#"{"pid":46560,"sessionId":"abcd1234ef"}"#);
        write("428044.5f2e.key", "not a record");
        write("99.json", r#"{"pid":99,"sessionId":"#);

        let mut sessions = read_sessions(directory.path());
        sessions.sort_by_key(|session| session.pid);

        assert_eq!(
            sessions,
            [
                SessionRecord {
                    pid:        46_560,
                    session_id: "abcd1234ef".to_string(),
                    cwd:        None,
                    name:       None,
                    status:     None,
                    entrypoint: None,
                },
                SessionRecord {
                    pid:        428_044,
                    session_id: "9f1c".to_string(),
                    cwd:        Some(PathBuf::from("/home/natepiano/rust/handler")),
                    name:       Some("enh/handler".to_string()),
                    status:     Some("busy".to_string()),
                    entrypoint: None,
                },
            ]
        );
    }

    /// A directory that is not there holds no records.
    #[test]
    fn a_missing_directory_holds_no_records() {
        let directory = tempfile::tempdir().expect("a temporary directory should open");

        assert_eq!(
            read_sessions(&directory.path().join("sessions")),
            [] as [SessionRecord; 0]
        );
    }

    /// `CLAUDE_PID` is read from among the other variables; a variable
    /// that only starts the same way, or a value that is not a pid,
    /// names nothing.
    #[test]
    fn claude_pid_is_read_from_the_environment() {
        let environment = |variables: &[&str]| -> Vec<OsString> {
            variables.iter().map(OsString::from).collect()
        };

        assert_eq!(
            claude_pid(&environment(&[
                "HOME=/home/natepiano",
                "CLAUDE_PID=1579022",
                "CLAUDE_CODE_SESSION_ID=e296",
            ])),
            Some(1_579_022)
        );
        assert_eq!(claude_pid(&environment(&["CLAUDE_PIDS=5"])), None);
        assert_eq!(claude_pid(&environment(&["CLAUDE_PID=soon"])), None);
        assert_eq!(claude_pid(&[]), None);
    }

    /// boss of bosses, the launcher in these tests.
    const BOSS: u32 = 1_579_022;
    /// A session boss opened in tmux.
    const TRUNK: u32 = 3_266_367;
    /// A session tmux holds that no transcript opened: it started more
    /// than [`CALL_LOOKBACK`](crate::constants::CALL_LOOKBACK) after
    /// trunk's call.
    const ARRANGE: u32 = 3_337_048;
    /// The unix second trunk and the first shell wrappers started.
    const LAUNCH: u64 = 1_790_000_000;

    /// A scanner with nothing found yet, reading no home directory.
    fn scanner() -> LocalScanner {
        LocalScanner {
            staff:               super::super::hierarchy::StaffDiscovery::Disabled,
            processes:           ProcessCensus::default(),
            run_records:         activity::RunRecords::default(),
            publication:         LocalPublication::FirstAnswerPending,
            home:                None,
            account_labels:      AccountLabels::default(),
            shell_calls:         HashMap::new(),
            launchers:           HashMap::new(),
            transcript_paths:    HashMap::new(),
            session_starts:      HashMap::new(),
            rollouts:            Rollouts::default(),
            session_names:       super::super::codex_names::SessionNames::default(),
            open_thread_rows:    OpenThreadRows::default(),
            codex_logs:          codex::CodexLogs::default(),
            herdr:               HerdrNames::isolated(),
            codex_links:         HashMap::new(),
            codex_start_window:  crate::config::CodexStartWindow::default(),
            herdr_probe_wait:    HerdrProbeWait::default(),
            transcript_activity: HashMap::new(),
            compactions:         CompactionMarkers::default(),
            transcript_models:   super::transcript::TranscriptModels::default(),
            subagent_activity:   HashMap::new(),
            subagent_reads:      SubagentReads::default(),
            worker_names:        WorkerNames::default(),
            worker_commands:     WorkerCommands::default(),
            session_roots:       SessionRoots::new([]),
            worker_facts:        BTreeMap::new(),
            activity_starts:     HashMap::new(),
            current_staff:       CurrentStaff::default(),

            desktop_source: DesktopSource::Windows(Vec::new()),
            desktops:       None,
        }
    }

    #[test]
    fn the_probe_deadline_starts_before_the_process_scan() {
        let mut scanner = scanner();
        scanner.processes = ProcessCensus::for_test();
        scanner.scan();
        assert!(
            scanner.herdr.probe_starts[0] <= scanner.processes.read_starts[0],
            "process discovery must spend the shared probe budget"
        );
    }

    #[test]
    fn scan_staged_publishes_without_waiting_for_herdr() {
        let (finished, result) = std::sync::mpsc::channel();
        let (received, scan) = HerdrNames::with_pending_answer(|herdr| {
            let mut scanner = scanner();
            scanner.processes = ProcessCensus::for_test();
            // Initialize sysinfo before measuring publication with a pending Herdr answer.
            scanner.processes.read(&[]);
            scanner.herdr = herdr;
            let scan = std::thread::spawn(move || {
                let mut stages = Vec::new();
                scanner.scan_staged(|stage, _, _| stages.push(stage));
                finished.send(stages).expect("published stages");
            });
            (result.recv_timeout(Duration::from_millis(250)), scan)
        });
        scan.join().expect("staged scan thread");
        assert_eq!(
            received.expect("herdr must not block publication"),
            vec![ScanStage::Classified, ScanStage::Completed]
        );
    }

    #[test]
    fn the_census_name_stage_keeps_the_herdr_name_in_the_wire_row() {
        let mut scanner = scanner();
        let processes = vec![entry(2, 1, "codex", 20, &["codex"])];
        let mut rows = classify::agent_rows_with_threads(
            &processes,
            &[],
            &HashMap::new(),
            None,
            &AccountLabels::default(),
        );
        scanner
            .codex_links
            .insert(2, CodexSessionLink::HerdrName("morty".into()));
        scanner.attach_codex_names(&mut rows);
        assert_eq!(rows[0].name, "morty");
        scanner.attach_codex_sessions(&processes, None, &mut rows);
        scanner.attach_codex_names(&mut rows);
        let wire = serde_json::to_value(&rows).expect("census wire rows");
        assert_eq!(wire[0]["name"], "morty");
    }

    /// A process with its command line read.
    fn entry(pid: u32, parent: u32, name: &str, started: u64, arguments: &[&str]) -> ProcessEntry {
        ProcessEntry {
            pid,
            parent: Some(parent),
            name: name.to_string(),
            arguments: arguments
                .iter()
                .map(|argument| (*argument).to_string())
                .collect(),
            started,
            directory: None,
            config_home: ConfigHome::Unread,
            claude_pid: None,
        }
    }

    /// Claude Code's shell wrapper under [`BOSS`] running `command`.
    fn wrapper(pid: u32, started: u64, command: &str) -> ProcessEntry {
        let script = format!(
            "source /home/natepiano/.claude/shell-snapshots/snapshot-zsh-1.sh && eval '{command}' \
             < /dev/null"
        );
        entry(pid, BOSS, "zsh", started, &["zsh", "-c", &script])
    }

    /// A Claude Code row.
    fn claude_row(pid: u32, name: &str, started: u64) -> AgentRow {
        AgentRow {
            model: ModelName::Unrecorded,
            effort: ReasoningEffort::Unrecorded,
            staff_names: StaffRowNames::default(),
            agent: Agent::Claude,
            role_fallback: RoleFallback::None,
            session_id: SessionIdentity::Unrecorded,
            service_tier: ServiceTier::Unrecorded,
            account: Account::Unrecorded,
            name: name.to_string(),
            activity: AgentActivity::Unreported,
            started,
            process_started: started,
            pid,
            desktop: None,
            directory: "~".to_string(),
            branch: None,
            launched_by: None,
            children: Vec::new(),
        }
    }

    /// Write a transcript at `path` holding one shell call, written at
    /// `at_ms`.
    fn write_call(path: &Path, at_ms: u64, command: &str, description: &str) {
        let stamp = DateTime::from_timestamp_millis(
            i64::try_from(at_ms).expect("the fixture time should fit"),
        )
        .expect("the fixture time should be representable")
        .to_rfc3339();
        let line = json!({
            "type": "assistant",
            "timestamp": stamp,
            "message": {"content": [{"type": "tool_use", "name": "Bash",
                                     "input": {"command": command, "description": description}}]},
        });
        fs::write(path, format!("{line}\n")).expect("the fixture transcript should write");
    }

    /// A session tmux holds is launched by the agent whose transcript
    /// opened it, which is kept. A miss is looked for again until the
    /// session is [`CALL_SEARCH_SETTLE`] old, then kept too. A launcher
    /// no longer listed leaves its session top level, and a session gone
    /// from the scan is forgotten.
    #[test]
    fn launchers_are_found_in_transcripts_and_kept() {
        let directory = TempDir::new().expect("a temporary directory should open");
        let transcript = directory.path().join("boss.jsonl");
        write_call(
            &transcript,
            (LAUNCH - 1) * 1_000,
            "tmux new-session -d -s tool-based-ui-trunk zsh -ic claude",
            "Open trunk",
        );
        let transcripts = HashMap::from([(BOSS, transcript)]);
        let processes = [
            entry(1, 0, "systemd", 0, &[]),
            entry(45_494, 1, "zsh", LAUNCH - 100, &[]),
            entry(BOSS, 45_494, "claude", LAUNCH - 100, &["claude"]),
            entry(3_261_729, 1, "tmux: server", LAUNCH - 50, &[]),
            entry(3_266_307, 3_261_729, "zsh", LAUNCH, &[]),
            entry(TRUNK, 3_266_307, "claude", LAUNCH, &["claude"]),
            entry(3_336_767, 3_261_729, "zsh", LAUNCH + 700, &[]),
            entry(ARRANGE, 3_336_767, "claude", LAUNCH + 700, &["claude"]),
        ];
        let rows = || {
            vec![
                claude_row(BOSS, "boss of bosses", LAUNCH - 100),
                claude_row(TRUNK, "tool-based-ui-trunk", LAUNCH),
                claude_row(ARRANGE, "tool-based-ui-arrange", LAUNCH + 700),
            ]
        };
        let launched = |rows: &[AgentRow]| -> Vec<Option<u32>> {
            rows.iter().map(|row| row.launched_by).collect()
        };
        let mut scanner = scanner();

        let mut young = rows();
        scanner.find_launchers(&processes, &[], &transcripts, &mut young, LAUNCH + 710);
        assert_eq!(launched(&young), [None, Some(BOSS), None]);
        assert_eq!(
            scanner.launchers,
            HashMap::from([((TRUNK, LAUNCH), Some((BOSS, LAUNCH - 100)))])
        );

        let mut settled = rows();
        scanner.find_launchers(&processes, &[], &transcripts, &mut settled, LAUNCH + 730);
        assert_eq!(launched(&settled), [None, Some(BOSS), None]);
        assert_eq!(scanner.launchers.get(&(ARRANGE, LAUNCH + 700)), Some(&None));

        let mut orphaned = rows().split_off(1);
        scanner.find_launchers(&processes, &[], &transcripts, &mut orphaned, LAUNCH + 730);
        assert_eq!(launched(&orphaned), [None, None]);

        let mut alone = rows();
        alone.truncate(1);
        scanner.find_launchers(&processes, &[], &transcripts, &mut alone, LAUNCH + 730);
        assert!(scanner.launchers.is_empty());
    }

    /// A launcher whose process was restarted after it opened a session,
    /// as one resumed in a new process is, is still that session's
    /// launcher: its transcript holds the call.
    #[test]
    fn a_launcher_restarted_since_keeps_the_sessions_it_opened() {
        let directory = TempDir::new().expect("a temporary directory should open");
        let transcript = directory.path().join("boss.jsonl");
        write_call(
            &transcript,
            (LAUNCH - 1) * 1_000,
            "tmux new-session -d -s tool-based-ui-trunk zsh -ic claude",
            "Resume trunk",
        );
        let transcripts = HashMap::from([(BOSS, transcript)]);
        let processes = [
            entry(1, 0, "systemd", 0, &[]),
            entry(3_261_729, 1, "tmux: server", LAUNCH, &[]),
            entry(3_266_307, 3_261_729, "zsh", LAUNCH, &[]),
            entry(TRUNK, 3_266_307, "claude", LAUNCH, &["claude"]),
            entry(45_494, 1, "zsh", LAUNCH - 100, &[]),
            entry(BOSS, 45_494, "claude", LAUNCH + 200, &["claude"]),
        ];
        let mut rows = vec![
            claude_row(TRUNK, "tool-based-ui-trunk", LAUNCH),
            claude_row(BOSS, "boss of bosses", LAUNCH + 200),
        ];

        scanner().find_launchers(&processes, &[], &transcripts, &mut rows, LAUNCH + 210);

        assert_eq!(rows[0].launched_by, Some(BOSS));
    }

    /// A Claude Code row dates from its transcript's first line where
    /// that is earlier than its process, and from its process where the
    /// transcript begins later or is not there; the rows are put back in
    /// order, and a transcript no longer listed is forgotten.
    #[test]
    fn a_session_dates_from_its_transcript_and_the_rows_reorder() {
        let directory = TempDir::new().expect("a temporary directory should open");
        let resumed = directory.path().join("resumed.jsonl");
        write_call(&resumed, (LAUNCH - 1_000) * 1_000, "ls", "List");
        let fresh = directory.path().join("fresh.jsonl");
        write_call(&fresh, (LAUNCH + 3) * 1_000, "ls", "List");
        let transcripts = HashMap::from([(BOSS, resumed), (TRUNK, fresh.clone())]);
        let mut rows = vec![
            claude_row(ARRANGE, "no transcript", LAUNCH - 500),
            claude_row(TRUNK, "fresh", LAUNCH),
            claude_row(BOSS, "resumed", LAUNCH + 200),
        ];
        let mut scanner = scanner();

        scanner.attach_session_starts(&transcripts, &mut rows);

        let dated: Vec<(&str, u64)> = rows
            .iter()
            .map(|row| (row.name.as_str(), row.started))
            .collect();
        assert_eq!(
            dated,
            [
                ("resumed", LAUNCH - 1_000),
                ("no transcript", LAUNCH - 500),
                ("fresh", LAUNCH),
            ]
        );
        scanner.attach_session_starts(&HashMap::from([(TRUNK, fresh.clone())]), &mut rows);
        assert_eq!(
            scanner.session_starts,
            HashMap::from([(fresh, Some(LAUNCH + 3))])
        );
    }

    /// A shell wrapper's call is found in its session's transcript or a
    /// running subagent's, and kept. A miss is looked for again until
    /// the wrapper is [`CALL_SEARCH_SETTLE`] old, then kept too, and a
    /// wrapper gone from the scan is forgotten.
    #[test]
    fn shell_calls_are_found_in_transcripts_and_kept() {
        let directory = TempDir::new().expect("a temporary directory should open");
        let own = directory.path().join("boss.jsonl");
        write_call(&own, (LAUNCH - 1) * 1_000, "cargo build", "Build it");
        let theirs = directory.path().join("agent-a1.jsonl");
        write_call(
            &theirs,
            LAUNCH * 1_000,
            "cargo nextest run",
            "Run the tests",
        );
        let transcripts = HashMap::from([(BOSS, own)]);
        let subagents = HashMap::from([(
            BOSS,
            vec![Subagent {
                agent_type:  String::new(),
                role:        super::transcript::SubagentRole::Test,
                activity:    AgentActivity::Idle,
                id:          "a1".to_string(),
                parent:      super::transcript::SubagentParent::OwningSession,
                description: "Test it".to_string(),
                started:     LAUNCH - 10,
                transcript:  theirs,
            }],
        )]);
        let boss = entry(BOSS, 1, "claude", LAUNCH - 100, &["claude"]);
        let processes = [
            boss.clone(),
            wrapper(3_900_000, LAUNCH, "cargo build"),
            wrapper(3_900_010, LAUNCH, "cargo nextest run"),
            wrapper(3_900_020, LAUNCH + 10, "ls"),
        ];
        let mut scanner = scanner();

        let found = scanner.find_shell_calls(&processes, &transcripts, &subagents, LAUNCH + 20);

        let call = |description: &str, subagent: Option<&str>| ShellCall {
            description: Some(description.to_string()),
            subagent:    subagent.map(str::to_string),
        };
        assert_eq!(
            found,
            HashMap::from([
                (3_900_000, call("Build it", None)),
                (3_900_010, call("Run the tests", Some("a1"))),
            ])
        );
        assert_eq!(scanner.shell_calls.len(), 2);

        scanner.find_shell_calls(&processes, &transcripts, &subagents, LAUNCH + 40);
        assert_eq!(
            scanner.shell_calls.get(&(3_900_020, LAUNCH + 10)),
            Some(&None)
        );

        scanner.find_shell_calls(&[boss], &transcripts, &subagents, LAUNCH + 40);
        assert!(scanner.shell_calls.is_empty());
    }
}

/// Staged publication must remain fast even with hierarchy discovery enabled.
#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod hierarchy_publication_tests {
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    use super::LocalScanner;
    use super::ScanStage;
    use crate::census::CensusUpdate;
    use crate::census::HierarchyEdge;
    use crate::census::HierarchyEdges;
    use crate::census::HierarchyNotice;
    use crate::census::HierarchyRole;
    use crate::census::HierarchySnapshot;
    use crate::census::HierarchySource;
    use crate::census::StaffFileSetting;
    use crate::census::account::AccountLabels;
    use crate::census::hierarchy::HierarchyError;
    use crate::census::hierarchy::HierarchyRunner;
    use crate::census::schedule;

    /// Counts discovery attempts independently of the scanner's stage callbacks.
    struct CountingRunner(AtomicUsize);

    impl HierarchyRunner for CountingRunner {
        fn run(&self, _: &str, _: Duration) -> Result<HierarchyEdges, HierarchyError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(HierarchyEdges(vec![HierarchyEdge {
                manager: "boss".to_string(),
                member:  "worker".to_string(),
                role:    HierarchyRole::Impl,
            }]))
        }
    }

    #[test]
    fn classified_rows_include_hierarchy_before_detail_reads() {
        let runner = Arc::new(CountingRunner(AtomicUsize::new(0)));
        let observed = Arc::clone(&runner);
        let updates = schedule::spawn_with_test_source(
            StaffFileSetting::new(HierarchySource::File("fixture".to_string())),
            runner,
            move |answers| {
                LocalScanner::new(AccountLabels::default()).scan_stages_with(
                    &[],
                    &[],
                    None,
                    |stage, rows| {
                        answers
                            .publish(stage, rows)
                            .expect("the scan should publish");
                    },
                    |_, _| {
                        assert_eq!(
                            observed.0.load(Ordering::SeqCst),
                            1,
                            "roles must be discovered before slow detail reads"
                        );
                    },
                );
            },
        );
        let edges = HierarchyEdges(vec![HierarchyEdge {
            manager: "boss".to_string(),
            member:  "worker".to_string(),
            role:    HierarchyRole::Impl,
        }]);
        assert_eq!(
            updates
                .recv_timeout(Duration::from_millis(500))
                .expect("classified roles should arrive"),
            CensusUpdate::Local {
                rows:      Vec::new(),
                hierarchy: HierarchySnapshot {
                    facts:  crate::census::CensusFacts::default().into(),
                    edges:  edges.clone(),
                    notice: HierarchyNotice::NoSessionMatched,
                },
                stage:     super::ScanStage::Classified,
            }
        );
        assert_eq!(
            updates
                .recv_timeout(Duration::from_millis(500))
                .expect("completed details should arrive"),
            CensusUpdate::Local {
                rows:      Vec::new(),
                hierarchy: HierarchySnapshot {
                    facts:  crate::census::CensusFacts::default().into(),
                    edges:  edges.clone(),
                    notice: HierarchyNotice::NoSessionMatched,
                },
                stage:     ScanStage::Classified,
            }
        );
        assert_eq!(
            updates
                .recv_timeout(Duration::from_millis(500))
                .expect("completion should retain classified roles"),
            CensusUpdate::Local {
                rows:      Vec::new(),
                hierarchy: HierarchySnapshot {
                    facts: crate::census::CensusFacts::default().into(),
                    edges,
                    notice: HierarchyNotice::NoSessionMatched
                },
                stage:     super::ScanStage::Completed,
            }
        );
    }
}

#[cfg(test)]
#[path = "scan/activity_scan_tests.rs"]
mod activity_scan_tests;

#[cfg(test)]
#[path = "scan/codex_home_tests.rs"]
mod codex_home_tests;

#[cfg(test)]
#[path = "scan/current_run_repair_tests.rs"]
mod current_run_repair_tests;

#[cfg(test)]
#[path = "subagent_activity_tests.rs"]
mod subagent_activity_tests;

#[cfg(test)]
#[path = "codex_session_tests.rs"]
mod codex_session_tests;
#[cfg(test)]
mod process_cache_tests {
    use std::process::Child;
    use std::time::Duration;

    use super::*;

    #[test]
    fn idle_worker_commands_parse_only_changed_process_inputs() {
        let mut commands = WorkerCommands::default();
        let mut names = WorkerNames::default();
        let mut processes = vec![ProcessEntry {
            pid:         42,
            parent:      Some(1),
            name:        "bash".into(),
            started:     100,
            arguments:   vec!["bash".into(), "-c".into(), "echo unchanged".into()],
            directory:   Some(PathBuf::from("/one")),
            claude_pid:  None,
            config_home: ConfigHome::Unread,
        }];
        worker_dispatches(&processes, &mut names, &mut commands);
        worker_dispatches(&processes, &mut names, &mut commands);
        assert_eq!(
            commands.command_reads, 1,
            "unchanged process commands are parsed once"
        );
        processes[0].arguments[2] = "echo changed".into();
        worker_dispatches(&processes, &mut names, &mut commands);
        assert_eq!(
            commands.command_reads, 2,
            "exec arguments replace the parsed command"
        );
        processes[0].directory = Some(PathBuf::from("/two"));
        worker_dispatches(&processes, &mut names, &mut commands);
        assert_eq!(
            commands.command_reads, 3,
            "cwd changes replace relative launcher paths"
        );
        processes[0].started += 1;
        worker_dispatches(&processes, &mut names, &mut commands);
        assert_eq!(
            commands.command_reads, 4,
            "pid reuse starts a new command read"
        );
        worker_dispatches(&[], &mut names, &mut commands);
        assert!(
            commands.processes.is_empty(),
            "exited process commands leave the cache"
        );
    }

    #[test]
    fn idle_process_table_new_sessions_patch_only_their_process_rows() {
        let mut census = ProcessCensus::for_test();
        let now = Instant::now();
        census.read_at(&[], now);
        let session = SessionRecord {
            pid:        std::process::id(),
            session_id: "new-session".into(),
            cwd:        None,
            name:       None,
            status:     None,
            entrypoint: None,
        };
        let rows = census.read_at(&[session], now + Duration::from_secs(2));
        assert!(rows.iter().any(|row| row.pid == std::process::id()));
        assert_eq!(
            census.discoveries, 1,
            "a new session does not enumerate other processes"
        );
        assert_eq!(
            census.detail_refreshes, 1,
            "only the new session needs detailed fields"
        );
    }

    #[test]
    fn idle_process_table_does_not_repeat_unchanged_process_discovery() {
        let mut census = ProcessCensus::for_test();
        let first = census.read(&[]);
        let second = census.read(&[]);
        assert!(!first.is_empty());
        assert!(!second.is_empty());
        assert_eq!(
            census.discoveries, 1,
            "an unchanged tick does not enumerate the process table"
        );
        assert_eq!(
            census.detail_refreshes, 0,
            "unchanged rows need no detailed field refresh"
        );
    }

    #[test]
    fn idle_process_table_replaces_a_reused_pid_and_retries_an_unread_identity()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut census = ProcessCensus::for_test();
        let pid = Pid::from_u32(std::process::id());
        let session = SessionRecord {
            pid:        pid.as_u32(),
            session_id: "reused-pid".into(),
            cwd:        None,
            name:       None,
            status:     None,
            entrypoint: None,
        };
        let sessions = [session];
        let first = census.read(&sessions);
        let expected = first
            .iter()
            .find(|row| row.pid == pid.as_u32())
            .ok_or("current process")?
            .clone();
        if let ProcessSnapshot::Current {
            rows, revisions, ..
        } = &mut census.snapshot
        {
            let revision = revisions.get_mut(&pid).ok_or("tracked identity")?;
            if let ProcessRevision::Observed { birth, .. } = revision {
                match birth {
                    #[cfg(target_os = "linux")]
                    ProcessBirth::KernelTicks(ticks) => *ticks += 1,
                    #[cfg(target_os = "macos")]
                    ProcessBirth::UnixTime { microseconds, .. } => *microseconds += 1,
                }
            }
            let mut stale = rows.to_vec();
            let row = stale
                .iter_mut()
                .find(|row| row.pid == pid.as_u32())
                .ok_or("cached process")?;
            row.name = "departed-agent".into();
            row.arguments = vec!["departed-command".into()];
            *rows = stale.into();
        }
        let fresh = census.read(&sessions);
        let actual = fresh
            .iter()
            .find(|row| row.pid == pid.as_u32())
            .ok_or("reused pid")?;
        assert_eq!(actual.name, expected.name);
        assert_eq!(actual.arguments, expected.arguments);
        assert_eq!(census.discoveries, 1);
        assert_eq!(census.detail_refreshes, 1);
        if let ProcessSnapshot::Current { revisions, .. } = &mut census.snapshot {
            revisions.insert(pid, ProcessRevision::Unreadable);
        }
        census.read(&sessions);
        assert_eq!(census.discoveries, 1);
        assert_eq!(
            census.detail_refreshes, 2,
            "an unread identity retries on the next tick"
        );
        Ok(())
    }

    #[test]
    fn idle_process_table_recovers_lost_events_and_new_sessions_within_thirty_seconds()
    -> Result<(), Box<dyn std::error::Error>> {
        struct RecoveryChild(Child);
        impl Drop for RecoveryChild {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let mut census = ProcessCensus::for_test();
        let now = Instant::now();
        census.read_at(&[], now);
        // This birth has no session or tracked ancestor. Only recovery can discover it.
        let mut child = RecoveryChild(
            Command::new("bash")
                .args(["-c", "exec -a claude bash -c 'echo ready; read -r finish'"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()?,
        );
        let mut signal = String::new();
        std::io::BufRead::read_line(
            &mut std::io::BufReader::new(child.0.stdout.take().ok_or("child stdout")?),
            &mut signal,
        )?;
        assert_eq!(signal.trim(), "ready");
        let child_pid = child.0.id();
        census.discovery = TestProcessDiscovery::FixtureProcesses {
            pids: vec![Pid::from_u32(child_pid)],
        };
        let session = SessionRecord {
            pid:        child_pid,
            session_id: "missed-birth".into(),
            cwd:        None,
            name:       None,
            status:     None,
            entrypoint: None,
        };
        let before = census.read_at(
            &[],
            now + PROCESS_RECOVERY_INTERVAL.saturating_sub(Duration::from_millis(1)),
        );
        assert!(
            !classify::agent_rows_with_threads(
                &before,
                std::slice::from_ref(&session),
                &HashMap::default(),
                None,
                &AccountLabels::default()
            )
            .iter()
            .any(|row| row.pid == child_pid),
            "the missed process is absent before recovery"
        );
        assert_eq!(census.discoveries, 1);
        let recovered = census.read_at(&[], now + PROCESS_RECOVERY_INTERVAL);
        let shown = classify::agent_rows_with_threads(
            &recovered,
            &[session],
            &HashMap::default(),
            None,
            &AccountLabels::default(),
        );
        assert!(
            shown
                .iter()
                .any(|row| row.pid == child_pid && row.agent == Agent::Claude),
            "recovery adds the missed live agent to displayed membership"
        );
        assert_eq!(
            census.discoveries, 2,
            "lost births, exec and inaccessible identities have a bounded recovery scan"
        );
        let session = SessionRecord {
            pid:        std::process::id(),
            session_id: "new-session".into(),
            cwd:        None,
            name:       None,
            status:     None,
            entrypoint: None,
        };
        let rows = census.read_at(&[session], now + PROCESS_RECOVERY_INTERVAL);
        assert_eq!(
            census.discoveries, 2,
            "a new session updates its row without another recovery scan"
        );
        assert!(rows.iter().any(|row| row.pid == std::process::id()));
        Ok(())
    }
}

#[cfg(test)]
impl LocalScanner {
    pub(super) fn set_herdr_for_test(&mut self, herdr: HerdrNames) { self.herdr = herdr; }

    pub(super) fn use_fixture_run_events(&mut self) {
        self.run_records = activity::RunRecords::for_test();
    }

    pub(super) fn scan_astra_for_test(
        &mut self,
        processes: &[ProcessEntry],
        sessions: &[SessionRecord],
        home: &Path,
    ) -> (Vec<AgentRow>, super::HierarchyEdges) {
        self.desktop_source = DesktopSource::Windows(Vec::new());
        let mut rows = Vec::new();
        let snapshot = self.scan_with_staff(
            processes,
            sessions,
            Some(home),
            |_, answer, _| rows = answer,
            |scanner, rows| scanner.attach_details(processes, sessions, Some(home), rows),
        );
        let edges = snapshot.publication().edges;
        (rows, edges)
    }
}

#[cfg(test)]
impl LocalScanner {
    pub(super) const fn astra_database_reads_for_test(&self) -> usize {
        self.open_thread_rows.database_reads
    }
}
