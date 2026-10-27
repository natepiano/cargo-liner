//! Reading this machine: the session records Claude Code writes under
//! `~/.claude/sessions`, the process table, the threads interactive
//! Codex sessions started, and the transcripts that name what each
//! agent is running and which agent opened a session tmux holds, handed
//! to [`classify`] and [`tree`]; and the windows `KWin` lists with the
//! panes and clients of tmux, handed to [`desktop`].

use std::collections::HashMap;
use std::collections::HashSet;
use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;
use std::time::Instant;
use std::time::SystemTime;

use crate::startup_log;

#[cfg(test)]
#[path = "scan/staged_tests.rs"]
mod staged_tests;

use sysinfo::Pid;
use sysinfo::Process;
use sysinfo::ProcessRefreshKind;
use sysinfo::ProcessesToUpdate;
use sysinfo::System;
use sysinfo::UpdateKind;

use super::Agent;
use super::AgentActivity;
use super::AgentRow;
use super::ChildKind;
use super::CodexThreadId;
use super::ServiceTier;
use super::account;
use super::account::AccountLabels;
use super::account::ConfigHome;
use super::activity;
use super::activity::TranscriptRead;
use super::branch;
use super::classify;
use super::classify::HeldSession;
use super::classify::ProcessEntry;
use super::classify::SessionRecord;
use super::codex;
use super::codex::CodexThread;
use super::codex_names::SessionName;
use super::codex_names::SessionNames;
use super::desktop;
use super::desktop::TmuxLayout;
use super::desktop::WindowEntry;
use super::hierarchy::ReviewerOutcome;
use super::hierarchy::ReviewerPools;
use super::hierarchy::ReviewerState;
use super::tier;
use super::tier::PacerHistory;
use super::tier::Rollouts;
use super::transcript;
use super::transcript::BashCall;
use super::transcript::Subagent;
use super::transcript::SubagentReads;
use super::tree;
use super::tree::ShellCall;
use super::tree::ThreadEntry;
use super::tree::TreeSources;
use super::unix_now;
use super::worker_dispatch::ParsedDispatch;
use super::worker_dispatch::WorkerDispatch;
use super::worker_dispatch::WorkerIdentity;
use super::worker_dispatch::WorkerNames;
use super::worker_dispatch::WorkerRoleSource;
use super::worker_dispatch::WorkerTask;
use crate::constants::ACTIVITY_PROCESS_NAMES;
use crate::constants::CALL_SEARCH_SETTLE;
use crate::constants::CLAUDE_AGENT;
use crate::constants::CLAUDE_DIRNAME;
use crate::constants::CLAUDE_PID_VARIABLE;
use crate::constants::CLAUDE_SESSIONS_DIRNAME;
use crate::constants::CODEX_AGENT;
use crate::constants::CODEX_DIRNAME;
use crate::constants::CODEX_MESH_SCRIPT;
use crate::constants::CODEX_PACER_STATE;
use crate::constants::CODEX_SESSIONS_DIRNAME;
use crate::constants::DESKTOP_QUERY_INTERVAL;
use crate::constants::MESH_SEAT_FLAGS;
use crate::constants::PROC_DIRNAME;
use crate::constants::PROC_FD_DIRNAME;
use crate::constants::PROJECTS_DIRNAME;
use crate::constants::SESSION_RECORD_EXTENSION;
use crate::constants::TMUX_LIST_CLIENTS;
use crate::constants::TMUX_LIST_PANES;
use crate::constants::VERIFY_SCRIPT;
#[cfg(target_os = "linux")]
use crate::constants::WINDOW_LIST_EXPRESSION;
use crate::constants::WORKER_LABEL;

/// Shell argv flag whose next argument contains the command text.
const SHELL_COMMAND_FLAG: &str = "-c";

/// A process as a cache knows it: its pid and start, so a pid handed
/// out again is a new key.
type ProcessKey = (u32, u64);

/// Scans this machine for its agents and what each is running.
#[derive(Debug)]
pub(super) struct LocalScanner {
    reviewer_pools:      ReviewerPools,
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
    transcript_activity: HashMap<PathBuf, TranscriptRead>,
    subagent_activity:   HashMap<PathBuf, TranscriptRead>,
    subagent_reads:      SubagentReads,
    worker_names:        WorkerNames,
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
    /// Adapt existing row-only fixtures to the staged scan implementation.
    #[cfg(test)]
    fn scan_stages(
        &mut self,
        processes: &[ProcessEntry],
        sessions: &[SessionRecord],
        codex_threads: &[CodexThread],
        home: Option<&Path>,
        mut publish: impl FnMut(Vec<AgentRow>),
        read_details: impl FnOnce(&mut Self, &mut Vec<AgentRow>),
    ) {
        self.scan_stages_with(
            processes,
            sessions,
            codex_threads,
            home,
            |_, rows| publish(rows),
            read_details,
        );
    }

    /// Classify synthetic or live inputs, publishing early only on startup.
    fn scan_stages_with(
        &mut self,
        processes: &[ProcessEntry],
        sessions: &[SessionRecord],
        codex_threads: &[CodexThread],
        home: Option<&Path>,
        mut publish: impl FnMut(ScanStage, Vec<AgentRow>),
        read_details: impl FnOnce(&mut Self, &mut Vec<AgentRow>),
    ) {
        let mut rows = startup_log::startup_stage("classify_agents", || {
            classify::agent_rows(
                processes,
                sessions,
                codex_threads,
                home,
                &self.account_labels,
            )
        });
        if matches!(self.publication, LocalPublication::FirstAnswerPending) {
            publish(ScanStage::Classified, rows.clone());
        }
        startup_log::startup_stage("details", || read_details(self, &mut rows));
        publish(ScanStage::Completed, rows);
        self.publication = LocalPublication::CompletedAnswerPublished;
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
            transcript_activity: HashMap::new(),
            subagent_activity: HashMap::new(),
            subagent_reads: SubagentReads::default(),
            worker_names: WorkerNames::default(),
            reviewer_pools: ReviewerPools::default(),
            desktop_source: DesktopSource::System,
            desktops: None,
        }
    }

    /// Every agent running now, oldest first, with what it is running.
    /// The wire probe consumes only the completed answer.
    ///
    /// Each scan reads a fresh process table, so a pid handed out again
    /// since the last scan cannot keep the command line or directory of
    /// the process that had it before.
    pub(super) fn scan(&mut self) -> Vec<AgentRow> {
        let mut rows = Vec::new();
        self.scan_staged(|_, answer| rows = answer);
        rows
    }

    /// Publish classification early on startup; later scans publish completed rows only.
    pub(super) fn scan_staged(&mut self, publish: impl FnMut(ScanStage, Vec<AgentRow>)) {
        let home = self.home.clone();
        let sessions = startup_log::startup_stage("sessions", || {
            home.as_deref()
                .map(|home| read_sessions(&home.join(CLAUDE_DIRNAME).join(CLAUDE_SESSIONS_DIRNAME)))
                .unwrap_or_default()
        });
        let processes = startup_log::startup_stage("process_table", || process_table(&sessions));
        // Codex's database is opened only while an interactive Codex runs,
        // and read only for the span its threads could have been created in.
        let codex_threads = startup_log::startup_stage("codex_sqlite", || {
            let mut threads = Vec::new();
            let homes: HashSet<_> = processes
                .iter()
                .filter(|process| process.is_codex())
                .filter_map(|process| codex::session_home(process, home.as_deref()))
                .collect();
            for codex_home in homes {
                if let Some(window) = classify::codex_thread_window(&processes) {
                    threads.extend(codex::read_threads(&codex_home, &window));
                }
                let resumed = processes
                    .iter()
                    .filter(|process| {
                        codex::session_home(process, home.as_deref()).as_ref() == Some(&codex_home)
                    })
                    .flat_map(|process| codex::read_resumed_threads(process, &codex_home));
                threads.extend(resumed);
            }
            threads
        });
        self.scan_stages_with(
            &processes,
            &sessions,
            &codex_threads,
            home.as_deref(),
            publish,
            |scanner, rows| {
                scanner.attach_details(
                    &processes,
                    &sessions,
                    &codex_threads,
                    home.as_deref(),
                    rows,
                );
            },
        );
    }

    /// Read the detail sources after the classified rows have been published.
    fn attach_details(
        &mut self,
        processes: &[ProcessEntry],
        sessions: &[SessionRecord],
        codex_threads: &[CodexThread],
        home: Option<&Path>,
        rows: &mut [AgentRow],
    ) {
        // The pacer's state is read only while some Codex runs.
        let pacer = home
            .filter(|_| processes.iter().any(ProcessEntry::is_codex))
            .map(|home| PacerHistory::read(&home.join(CODEX_PACER_STATE)))
            .unwrap_or_default();
        let service_tiers = startup_log::startup_stage("codex_service_tiers", || {
            self.codex_tiers(processes, codex_threads, &pacer, home)
        });
        for row in rows.iter_mut() {
            if let Some(tier) = service_tiers.get(&row.pid) {
                row.service_tier = *tier;
            }
        }
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
        let mut subagents: HashMap<u32, Vec<Subagent>> =
            startup_log::startup_stage("transcript_subagents", || {
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
        let shell_calls = startup_log::startup_stage("transcript_shell_calls", || {
            self.find_shell_calls(processes, &transcripts, &subagents, now)
        });
        let threads = startup_log::startup_stage("codex_app_server_threads", || {
            home.map(|home| {
                app_server_threads(
                    processes,
                    &home.join(CODEX_DIRNAME),
                    &mut self.rollouts,
                    &pacer,
                    Some(home),
                    &self.account_labels,
                )
            })
            .unwrap_or_default()
        });
        self.attach_codex_sessions(processes, codex_threads, home, rows);
        self.rollouts.forget_unread();
        let dispatches = startup_log::startup_stage("worker_dispatches", || {
            worker_dispatches(processes, &mut self.worker_names)
        });
        let thread_dispatches = threads
            .values()
            .flatten()
            .filter_map(|thread| {
                let CodexThreadId::Recorded(id) = &thread.thread_id else {
                    return None;
                };
                let read = self.rollouts.thread(id)?;
                match self
                    .worker_names
                    .resolve(id, read.first_prompt.text(), &read.worktree)
                {
                    WorkerDispatch::Worker(worker) => Some((id.clone(), worker)),
                    WorkerDispatch::Plain => None,
                }
            })
            .collect();
        let sources = TreeSources {
            dispatches,
            thread_dispatches,
            sessions,
            subagents,
            shell_calls,
            threads,
            service_tiers,
        };
        startup_log::startup_stage("attach_children", || {
            tree::attach_children(rows, processes, &sources);
        });
        startup_log::startup_stage("transcript_activity_and_worker_names", || {
            self.attach_activity_and_names(processes, &transcripts, rows);
        });
        startup_log::startup_stage("attach_desktops", || self.attach_desktops(processes, rows));
        startup_log::startup_stage("attach_branches", || branch::attach_branches(rows, home));
    }

    /// Resolve the interactive session itself, using the same rollout reader as workers.
    fn attach_codex_sessions(
        &mut self,
        processes: &[ProcessEntry],
        threads: &[CodexThread],
        home: Option<&Path>,
        rows: &mut [AgentRow],
    ) {
        let startup = classify::codex_startup_threads(processes, threads, home);
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
            row.name = format!("pid {}", row.pid);
            let Some(thread) = startup.get(&row.pid) else {
                continue;
            };
            let read = self.rollouts.read(&thread.rollout);
            row.activity = read.activity();
            let Some(id) = tree::thread_id(&thread.rollout) else {
                continue;
            };
            if let Some(codex_home) = codex::session_home(process, home)
                && let SessionName::Named(name) = self.session_names.name(&codex_home, &id)
            {
                row.name = name;
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

    /// Resolve conversation tools and detached runs once, after child attachment.
    fn attach_activity_and_names(
        &mut self,
        processes: &[ProcessEntry],
        transcripts: &HashMap<u32, PathBuf>,
        rows: &mut [AgentRow],
    ) {
        let mut runs = HashMap::new();
        ReviewerPools::root().update(&mut self.reviewer_pools);
        let reviewer_outcomes = self.reviewer_pools.outcomes();
        ReviewerPools::apply_outcomes(rows, &reviewer_outcomes);
        let mut seen_transcripts = HashSet::new();
        let table = classify::pid_table(processes);
        for row in rows.iter_mut() {
            if let Some(path) = transcripts.get(&row.pid) {
                seen_transcripts.insert(path.clone());
                let read = self
                    .transcript_activity
                    .entry(path.clone())
                    .or_insert_with(|| activity::TranscriptRead::new(row.activity));
                read.advance(path, row.activity);
                startup_log::startup_event(&format!("transcript_bytes_read {}", read.bytes_read()));
                // The session record is authoritative for a closed Claude turn.
                if row.activity != AgentActivity::Idle
                    && read.activity() != AgentActivity::Unreported
                {
                    row.activity = read.activity();
                }
            }
            if let Some(verify) = processes.iter().find(|process| {
                process.arguments.iter().any(|arg| {
                    Path::new(arg)
                        .file_name()
                        .is_some_and(|name| name == VERIFY_SCRIPT)
                }) && process_is_below(process.pid, row.pid, &table)
            }) {
                row.activity = activity::verify_activity(processes, verify.pid).activity();
            }
            for child in &mut row.children {
                let CodexThreadId::Recorded(id) = &child.thread_id else {
                    continue;
                };
                let Some(read) = self.rollouts.thread(id) else {
                    child.worker_task = WorkerTask::RoleOnly;
                    continue;
                };
                let prompt = read.first_prompt.text();
                let dispatch = self.worker_names.resolve(id, prompt, &read.worktree);
                child.worker_task = WorkerTask::from_prompt(prompt);
                child.name = match &dispatch {
                    WorkerDispatch::Worker(worker) => worker.label(),
                    WorkerDispatch::Plain => child
                        .worker_task
                        .label(WorkerRoleSource::Unroled, &child.name),
                };
                child.dispatch = dispatch;
                if let Some(ReviewerOutcome {
                    state: ReviewerState::Busy { name, started, .. },
                    ..
                }) = reviewer_outcomes
                    .iter()
                    .find(|outcome| outcome.thread_id == *id)
                {
                    child.worker_task = read.reviewer_task(name, *started);
                    child.dispatch = WorkerDispatch::Plain;
                }

                let records: Vec<_> = WorkerNames::session_dirs(prompt)
                    .into_iter()
                    .flat_map(|directory| {
                        runs.entry(directory.clone())
                            .or_insert_with(|| activity::read_runs(&directory))
                            .clone()
                    })
                    .collect();
                let gate = activity::run_activity(&records, processes, id);
                child.activity = if gate == AgentActivity::Unreported {
                    read.activity()
                } else {
                    gate
                };
            }
        }
        fold_claude_dispatches(rows);
        self.transcript_activity
            .retain(|path, _| seen_transcripts.contains(path));
    }

    /// The tier each Codex among `processes` that is not an app server
    /// asked for, by pid, from the rollout of the thread it started with
    /// among `codex_threads`, its command line, then `pacer`.
    fn codex_tiers(
        &mut self,
        processes: &[ProcessEntry],
        codex_threads: &[CodexThread],
        pacer: &PacerHistory,
        home: Option<&Path>,
    ) -> HashMap<u32, ServiceTier> {
        let startup = classify::codex_startup_threads(processes, codex_threads, home);
        processes
            .iter()
            .filter(|process| process.is_codex() && !process.is_app_server())
            .map(|process| {
                let thread = startup.get(&process.pid);
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

/// Resolve literal launchers against one roster read per directory per scan.
/// A launcher's actual pid is used even when a shell wrapper owns its row.
#[cfg(test)]
pub(super) fn find_worker_dispatches(processes: &[ProcessEntry]) -> HashMap<u32, WorkerDispatch> {
    worker_dispatches(processes, &mut WorkerNames::default())
}

/// Snapshot decoding belongs to shell calls; seat identity can also come from raw argv.
fn seat_sender_command(process: &ProcessEntry) -> Option<String> {
    tree::shell_command(process).or_else(|| match process.arguments.as_slice() {
        [_, flag, command, ..] if flag == SHELL_COMMAND_FLAG => Some(command.clone()),
        _ => None,
    })
}

/// Share the mtime roster cache with launcher-free naming.
fn worker_dispatches(
    processes: &[ProcessEntry],
    names: &mut WorkerNames,
) -> HashMap<u32, WorkerDispatch> {
    let table = classify::pid_table(processes);
    let wrappers: HashMap<_, _> = processes
        .iter()
        .filter_map(|process| {
            let command = seat_sender_command(process)?;
            let dispatch = process.directory.as_ref().map_or_else(
                || ParsedDispatch::parse(&command),
                |directory| ParsedDispatch::parse_at(&command, directory),
            );
            Some((process.pid, dispatch.ok()?))
        })
        .collect();
    let mut found = HashMap::new();
    for process in processes {
        let Ok(dispatch) = ParsedDispatch::from_arguments(&process.arguments) else {
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
                    .map(|pair| pair[1].clone())
            });
        let launcher = dispatch.clone();
        let mut dispatch = dispatch;
        if let Some(seat) = selected {
            dispatch.select_seat(seat);
        }
        let worker = match names.resolve_claude(&dispatch, processes, process.pid) {
            WorkerDispatch::Plain => {
                dispatch.resolve(names.roster(&dispatch.session_dir), process.pid)
            },
            worker @ WorkerDispatch::Worker(_) => worker,
        };
        found.insert(process.pid, worker.clone());
        let mut parent = process;
        let mut seen = HashSet::new();
        while seen.insert(parent.pid) {
            if seat_sender_command(parent).is_some_and(|command| {
                parent.directory.as_ref().map_or_else(
                    || launcher.is_final_command_of(&command),
                    |directory| {
                        ParsedDispatch::parse_at(&command, directory)
                            .is_ok_and(|parsed| parsed == launcher)
                    },
                )
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
    for (pid, dispatch) in wrappers {
        if found.contains_key(&pid) {
            continue;
        }
        found.insert(
            pid,
            match names.resolve_claude(&dispatch, processes, pid) {
                WorkerDispatch::Plain => dispatch.resolve(names.roster(&dispatch.session_dir), pid),
                worker @ WorkerDispatch::Worker(_) => worker,
            },
        );
    }
    found
}

/// Move a live Claude seat's launcher work into that session's own row.
fn fold_claude_dispatches(rows: &mut [AgentRow]) {
    let matches: Vec<_> = rows.iter().flat_map(|row| row.children.iter()).filter_map(|child| {
        if !matches!(child.kind, ChildKind::Shell(_)) { return None; }
        let WorkerDispatch::Worker(dispatch) = &child.dispatch else { return None; };
        let WorkerIdentity::ClaudeSession(id) = &dispatch.identity else { return None; };
        rows.iter().position(|row| matches!(&row.session_id, super::SessionIdentity::Claude(session) if session == id))
            .map(|target| (target, child.pid, dispatch.clone()))
    }).collect();
    for (target, pid, dispatch) in matches {
        rows[target].name = dispatch.label();
        rows[target].role_fallback = dispatch.fallback();
        let mut work = Vec::new();
        for row in rows.iter_mut() {
            if let Some(index) = row
                .children
                .iter()
                .position(|child| matches!(child.kind, ChildKind::Shell(_)) && child.pid == pid)
            {
                let depth = row.children[index].depth;
                let end = row.children[index + 1..]
                    .iter()
                    .position(|child| child.depth <= depth)
                    .map_or(row.children.len(), |offset| index + 1 + offset);
                work.extend(row.children.drain(index + 1..end).map(|mut child| {
                    child.depth = child.depth.saturating_sub(depth + 1);
                    child
                }));
                row.children.remove(index);
            }
        }
        rows[target].children.extend(work);
    }
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

/// The threads each Codex app server in `processes` holds open, by its
/// pid, named from the thread database in `codex_dir`, each with the
/// tier its rollout in `rollouts` or `pacer` names. Only Linux lists a
/// process's open files, so elsewhere there are none.
fn app_server_threads(
    processes: &[ProcessEntry],
    codex_dir: &Path,
    rollouts: &mut Rollouts,
    pacer: &PacerHistory,
    home: Option<&Path>,
    account_labels: &AccountLabels,
) -> HashMap<u32, Vec<ThreadEntry>> {
    if !cfg!(target_os = "linux") {
        return HashMap::new();
    }
    let conversations = codex_dir.join(CODEX_SESSIONS_DIRNAME);
    let held: Vec<(&ProcessEntry, Vec<(String, PathBuf)>)> = processes
        .iter()
        .filter(|process| process.is_codex() && process.is_app_server())
        .map(|process| {
            let descriptors = Path::new(PROC_DIRNAME)
                .join(process.pid.to_string())
                .join(PROC_FD_DIRNAME);
            let mut ids: Vec<(String, PathBuf)> = fs::read_dir(descriptors)
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
                .filter_map(|entry| fs::read_link(entry.path()).ok())
                .filter(|target| target.starts_with(&conversations))
                .filter_map(|target| Some((tree::thread_id(&target)?, target)))
                .collect();
            ids.sort();
            ids.dedup_by(|left, right| left.0 == right.0);
            (process, ids)
        })
        .filter(|(_, ids)| !ids.is_empty())
        .collect();
    let every_id: Vec<String> = held
        .iter()
        .flat_map(|(_, ids)| ids.iter().map(|(id, _)| id.clone()))
        .collect();
    let named = codex::read_threads_by_id(codex_dir, &every_id);
    held.into_iter()
        .map(|(process, ids)| {
            let threads = ids
                .into_iter()
                .map(|(id, rollout)| {
                    let (name, started) = named.get(&id).map_or_else(
                        || (WORKER_LABEL.to_owned(), process.started),
                        |thread| {
                            (
                                thread.label().unwrap_or_else(|| WORKER_LABEL.to_owned()),
                                thread.created_ms / 1_000,
                            )
                        },
                    );
                    let read = rollouts.read(&rollout);
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
        .collect()
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

/// The process table the classification and the tree read.
///
/// Three passes. The first reads every process's name, parent and
/// start time, which is all the ancestry walk needs. The second reads
/// the command line and directory, and only for the processes
/// [`detailed_pids`] names. The third reads every Codex and Claude Code
/// environment for its configuration home, including version-named Claude
/// binaries identified by session records. Codex also supplies `CLAUDE_PID`.
fn process_table(sessions: &[SessionRecord]) -> Vec<ProcessEntry> {
    let mut system = System::new();
    startup_log::startup_stage("proc_ancestry", || {
        system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing().without_tasks(),
        )
    });
    let detailed =
        startup_log::startup_stage("proc_detail_selection", || detailed_pids(&system, sessions));
    startup_log::startup_stage("proc_commands_directories", || {
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&detailed),
            false,
            ProcessRefreshKind::nothing()
                .without_tasks()
                .with_cmd(UpdateKind::OnlyIfNotSet)
                .with_cwd(UpdateKind::OnlyIfNotSet),
        )
    });
    let session_pids: HashSet<u32> = sessions.iter().map(|session| session.pid).collect();
    let agents: Vec<Pid> = system
        .processes()
        .values()
        .filter(|process| {
            process.name() == CODEX_AGENT
                || process.name() == CLAUDE_AGENT
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
                .with_environ(UpdateKind::OnlyIfNotSet),
        )
    });
    system
        .processes()
        .values()
        .map(|process| {
            let mut entry = ProcessEntry {
                pid:         process.pid().as_u32(),
                parent:      process.parent().map(Pid::as_u32),
                name:        process.name().to_string_lossy().into_owned(),
                arguments:   process
                    .cmd()
                    .iter()
                    .map(|argument| argument.to_string_lossy().into_owned())
                    .collect(),
                started:     process.start_time(),
                directory:   process.cwd().map(Path::to_path_buf),
                config_home: ConfigHome::Unread,
                claude_pid:  if process.name() == CODEX_AGENT {
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
        })
        .collect()
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
fn detailed_pids(system: &System, sessions: &[SessionRecord]) -> Vec<Pid> {
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
                .filter(|process| process.name() == CLAUDE_AGENT)
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
                .filter(|process| process.name() == CODEX_AGENT)
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
                    .any(|name| process.name() == *name)
            })
            .map(Process::pid),
    );
    detailed.into_iter().collect()
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use chrono::DateTime;
    use serde_json::json;
    use tempfile::TempDir;

    use super::*;
    use crate::census::Account;
    use crate::census::RoleFallback;
    use crate::census::SessionIdentity;
    use crate::census::hierarchy::ReviewerPools;

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
            publication:         LocalPublication::FirstAnswerPending,
            home:                None,
            account_labels:      AccountLabels::default(),
            shell_calls:         HashMap::new(),
            launchers:           HashMap::new(),
            transcript_paths:    HashMap::new(),
            session_starts:      HashMap::new(),
            rollouts:            Rollouts::default(),
            session_names:       super::super::codex_names::SessionNames::default(),
            transcript_activity: HashMap::new(),
            subagent_activity:   HashMap::new(),
            subagent_reads:      SubagentReads::default(),
            worker_names:        WorkerNames::default(),
            reviewer_pools:      ReviewerPools::default(),
            desktop_source:      DesktopSource::Windows(Vec::new()),
            desktops:            None,
        }
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
                parent:      None,
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
    use crate::census::HierarchyCommand;
    use crate::census::HierarchyEdge;
    use crate::census::HierarchyEdges;
    use crate::census::HierarchyNotice;
    use crate::census::HierarchyRole;
    use crate::census::HierarchySnapshot;
    use crate::census::HierarchySource;
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
                reviewer: crate::census::hierarchy::ReviewerState::NotPool,
                manager:  "boss".to_string(),
                member:   "worker".to_string(),
                role:     HierarchyRole::Impl,
            }]))
        }
    }

    #[test]
    fn classified_rows_include_hierarchy_before_detail_reads() {
        let runner = Arc::new(CountingRunner(AtomicUsize::new(0)));
        let observed = Arc::clone(&runner);
        let updates = schedule::spawn_with_test_source(
            HierarchyCommand::new(HierarchySource::Command("fixture".to_string())),
            runner,
            move |answers| {
                LocalScanner::new(AccountLabels::default()).scan_stages_with(
                    &[],
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
            reviewer: crate::census::hierarchy::ReviewerState::NotPool,
            manager:  "boss".to_string(),
            member:   "worker".to_string(),
            role:     HierarchyRole::Impl,
        }]);
        assert_eq!(
            updates
                .recv_timeout(Duration::from_millis(500))
                .expect("classified roles should arrive"),
            CensusUpdate::Local {
                rows:      Vec::new(),
                hierarchy: HierarchySnapshot {
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
#[path = "subagent_activity_tests.rs"]
mod subagent_activity_tests;

#[cfg(test)]
#[path = "codex_session_tests.rs"]
mod codex_session_tests;
