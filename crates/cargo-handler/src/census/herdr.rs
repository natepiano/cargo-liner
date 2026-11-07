//! Herdr pane names apply only when no thread link exists.

use std::collections::HashMap;
use std::collections::HashSet;
use std::io::BufRead;
use std::io::BufReader;
use std::io::ErrorKind;
use std::io::Read;
use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::mpsc::Receiver;
use std::sync::mpsc::RecvTimeoutError;
use std::sync::mpsc::SyncSender;
use std::sync::mpsc::{self};
use std::thread;
use std::time::Duration;
use std::time::Instant;

use serde::Deserialize;
use serde_json::Value;
use serde_json::json;
#[cfg(not(target_os = "linux"))]
use sysinfo::Pid;
#[cfg(not(target_os = "linux"))]
use sysinfo::ProcessRefreshKind;
#[cfg(not(target_os = "linux"))]
use sysinfo::ProcessesToUpdate;
#[cfg(not(target_os = "linux"))]
use sysinfo::System;
#[cfg(not(target_os = "linux"))]
use sysinfo::UpdateKind;

use super::classify;
use super::classify::CodexSessionLink;
use super::classify::ProcessEntry;
use crate::config::HerdrProbeWait;
use crate::config::HerdrReadTimeout;
use crate::constants::HERDR_ANSWER_LIMIT;
use crate::constants::HERDR_SOCKET_VARIABLE;
use crate::constants::HERDR_SUBSCRIPTION_RETRY;

/// The probe shares one name deadline with the scan that requested the names.
#[derive(Clone, Copy, Debug)]
pub(super) struct HerdrProbeDeadline {
    scan_started: Instant,
    wait:         HerdrProbeWait,
}

impl HerdrProbeDeadline {
    pub(super) const fn from_scan_start(scan_started: Instant, wait: HerdrProbeWait) -> Self {
        Self { scan_started, wait }
    }

    fn remaining(self, now: Instant) -> Duration {
        (self.scan_started + self.wait.duration()).saturating_duration_since(now)
    }
}

/// Environment discovery is specific to one process lifetime.
#[derive(Clone, Debug)]
enum HerdrAddress {
    Socket(PathBuf),
    Absent,
}

/// A cached process identity cannot survive pid reuse.
#[derive(Clone, Debug, Eq, PartialEq)]
struct PaneProcess {
    pid:     u32,
    started: u64,
}

impl PaneProcess {
    fn current(pid: u32, processes: &[ProcessEntry]) -> Option<Self> {
        processes.iter().find(|p| p.pid == pid).map(|p| Self {
            pid,
            started: p.started,
        })
    }

    fn is_current(&self, processes: &[ProcessEntry]) -> bool {
        processes
            .iter()
            .any(|p| p.pid == self.pid && p.started == self.started)
    }
}

/// Only pane membership survives a scan, under its terminal identity and revision.
#[derive(Debug)]
struct PaneMembership {
    revision:    u64,
    terminal_id: String,
    shells:      Vec<PaneProcess>,
    foreground:  Vec<PaneProcess>,
}

/// Pane membership and environment addresses have separate lifetime keys.
#[derive(Debug, Default)]
struct HerdrReader {
    read_timeout: HerdrReadTimeout,
    addresses:    HashMap<(u32, u64), HerdrAddress>,
    memberships:  HashMap<(PathBuf, String), PaneMembership>,
    events:       HashMap<PathBuf, PaneEvents>,
}

/// The scan uses completed answers. Only the worker reads the API.
#[derive(Debug, Default)]
pub(super) struct HerdrNames {
    read_timeout:            HerdrReadTimeout,
    #[cfg(test)]
    pub(super) probe_starts: Vec<Instant>,
    worker:                  NameWorker,
    clients:                 Vec<NameClient>,
    generation:              u64,
    answers:                 HashMap<(u32, u64), String>,
    initial:                 InitialAnswer,
}

#[derive(Debug, Eq, PartialEq)]
struct NameClient {
    process: PaneProcess,
    parent:  Option<u32>,
    need:    NameNeed,
}

#[derive(Debug, Eq, PartialEq)]
enum NameNeed {
    ProvenThread,
    PaneName,
}

#[derive(Debug)]
struct NameAnswer {
    generation: u64,
    names:      Vec<(PaneProcess, String)>,
}

#[derive(Debug, Default)]
enum InitialAnswer {
    #[default]
    NotRequested,
    Pending,
    Ready,
}

#[derive(Debug, Default)]
enum NameWorker {
    #[default]
    Dormant,
    Running {
        send:    SyncSender<NameWork>,
        receive: Receiver<NameAnswer>,
        inputs:  Arc<Mutex<Option<NameInputs>>>,
    },
    Unavailable,
}

#[derive(Debug)]
struct NameInputs {
    generation: u64,
    processes:  Vec<ProcessEntry>,
    links:      HashMap<u32, CodexSessionLink>,
}

#[derive(Debug)]
enum NameWork {
    Processes,
    CheckPaneEvents,
}

impl HerdrNames {
    /// Tests use fake answers and never read the user's herdr service.
    #[cfg(test)]
    pub(super) fn isolated() -> Self {
        Self {
            worker: NameWorker::Unavailable,
            ..Self::default()
        }
    }

    /// Keep an unanswered channel connected while a staged scan runs.
    #[cfg(test)]
    pub(super) fn with_pending_answer<T>(test: impl FnOnce(Self) -> T) -> T {
        let (send, work) = mpsc::sync_channel(1);
        let (answers, receive) = mpsc::channel();
        let names = Self {
            worker: NameWorker::Running {
                send,
                receive,
                inputs: Arc::default(),
            },
            initial: InitialAnswer::Pending,
            ..Self::default()
        };
        let result = test(names);
        drop(answers);
        drop(work);
        result
    }

    pub(super) fn refresh(
        &mut self,
        processes: &[ProcessEntry],
        links: &mut HashMap<u32, CodexSessionLink>,
    ) {
        self.collect_answers();
        let clients: Vec<_> = processes
            .iter()
            .filter(|p| p.is_codex() && !p.is_app_server())
            .map(|p| NameClient {
                process: PaneProcess {
                    pid:     p.pid,
                    started: p.started,
                },
                parent:  p.parent,
                need:    if matches!(links.get(&p.pid), Some(CodexSessionLink::NoMatch)) {
                    NameNeed::PaneName
                } else {
                    NameNeed::ProvenThread
                },
            })
            .collect();
        self.answers.retain(|&(pid, started), _| {
            clients
                .iter()
                .any(|c| c.process.pid == pid && c.process.started == started)
        });
        if clients.iter().any(|c| c.need == NameNeed::PaneName)
            && matches!(self.worker, NameWorker::Dormant)
        {
            self.worker = NameWorker::start(self.read_timeout);
        }
        if let NameWorker::Running { send, inputs, .. } = &self.worker {
            if clients != self.clients {
                if let Ok(mut latest) = inputs.try_lock() {
                    self.generation += 1;
                    *latest = Some(NameInputs {
                        generation: self.generation,
                        processes:  processes.to_vec(),
                        links:      links.clone(),
                    });
                    self.answers.clear();
                    self.clients = clients;
                    self.initial = InitialAnswer::Pending;
                    // Any queued signal makes the worker take the latest snapshot.
                    let _ = send.try_send(NameWork::Processes);
                }
            } else if clients.iter().any(|c| c.need == NameNeed::PaneName) {
                let _ = send.try_send(NameWork::CheckPaneEvents);
            }
        }
        self.apply(links);
    }

    fn collect_answers(&mut self) {
        if let NameWorker::Running { receive, .. } = &self.worker {
            for answers in receive.try_iter() {
                if answers.generation == self.generation {
                    self.initial = InitialAnswer::Ready;
                    self.answers = answers
                        .names
                        .into_iter()
                        .map(|(p, name)| ((p.pid, p.started), name))
                        .collect();
                }
            }
        }
    }

    fn apply(&self, links: &mut HashMap<u32, CodexSessionLink>) {
        for client in &self.clients {
            let process = &client.process;
            if let Some(name) = self.answers.get(&(process.pid, process.started))
                && let Some(link @ CodexSessionLink::NoMatch) = links.get_mut(&process.pid)
            {
                *link = CodexSessionLink::HerdrName(name.clone());
            }
        }
    }

    /// Set the socket timeout before the worker starts.
    pub(super) const fn with_read_timeout(mut self, timeout: HerdrReadTimeout) -> Self {
        self.read_timeout = timeout;
        self
    }

    #[cfg(test)]
    pub(super) const fn socket_timeout(&self) -> Duration { self.read_timeout.duration() }

    /// A probe waits for its current generation within the scan deadline.
    pub(super) fn finish_initial(
        &mut self,
        links: &mut HashMap<u32, CodexSessionLink>,
        deadline: HerdrProbeDeadline,
    ) {
        #[cfg(test)]
        self.probe_starts.push(deadline.scan_started);
        self.finish_initial_with(links, deadline, Instant::now, Receiver::recv_timeout);
    }

    /// The channel and clock boundary lets tests advance time without sleeping.
    fn finish_initial_with(
        &mut self,
        links: &mut HashMap<u32, CodexSessionLink>,
        deadline: HerdrProbeDeadline,
        mut now: impl FnMut() -> Instant,
        mut receive: impl FnMut(&Receiver<NameAnswer>, Duration) -> Result<NameAnswer, RecvTimeoutError>,
    ) {
        while matches!(self.initial, InitialAnswer::Pending) {
            let NameWorker::Running {
                receive: channel, ..
            } = &self.worker
            else {
                break;
            };
            let Ok(answer) = receive(channel, deadline.remaining(now())) else {
                break;
            };
            if answer.generation == self.generation {
                self.initial = InitialAnswer::Ready;
                self.answers = answer
                    .names
                    .into_iter()
                    .map(|(process, name)| ((process.pid, process.started), name))
                    .collect();
            }
        }
        self.apply(links);
    }
}

impl NameWorker {
    fn start(read_timeout: HerdrReadTimeout) -> Self {
        let (send, work) = mpsc::sync_channel(1);
        let (answers, receive) = mpsc::channel();
        let inputs = Arc::new(Mutex::new(None::<NameInputs>));
        let latest = Arc::clone(&inputs);
        let launched = thread::Builder::new()
            .name("herdr-names".into())
            .spawn(move || {
                let mut reader = HerdrReader {
                    read_timeout,
                    ..HerdrReader::default()
                };
                let mut processes = Vec::new();
                let mut links = HashMap::new();
                let mut generation = 0;
                while let Ok(_) | Err(RecvTimeoutError::Timeout) =
                    work.recv_timeout(HERDR_SUBSCRIPTION_RETRY)
                {
                    let current = latest.lock().ok().and_then(|mut inputs| inputs.take());
                    if let Some(current) = current {
                        generation = current.generation;
                        processes = current.processes;
                        links = current.links;
                    } else if !reader.check_events() {
                        continue;
                    }
                    for link in links.values_mut() {
                        if matches!(link, CodexSessionLink::HerdrName(_)) {
                            *link = CodexSessionLink::NoMatch;
                        }
                    }
                    reader.refresh(&processes, &mut links);
                    let names = links
                        .iter()
                        .filter_map(|(&pid, link)| {
                            let CodexSessionLink::HerdrName(name) = link else {
                                return None;
                            };
                            PaneProcess::current(pid, &processes).map(|p| (p, name.clone()))
                        })
                        .collect();
                    if answers.send(NameAnswer { generation, names }).is_err() {
                        break;
                    }
                }
            });
        if launched.is_ok() {
            Self::Running {
                send,
                receive,
                inputs,
            }
        } else {
            Self::Unavailable
        }
    }
}

/// The event stream changes the cache only when a pane or its label changes.
#[derive(Debug)]
enum PaneEvents {
    Connected { stream: UnixStream, bytes: Vec<u8> },
    Unavailable { retry_at: Instant },
}

impl PaneEvents {
    fn subscribe(path: &Path, timeout: HerdrReadTimeout) -> Self {
        let open = || -> Result<UnixStream, ()> {
            let mut stream = UnixStream::connect(path).map_err(|_| ())?;
            stream
                .set_write_timeout(Some(timeout.duration()))
                .map_err(|_| ())?;
            let subscriptions: Vec<_> = [
                "pane.created",
                "pane.closed",
                "pane.updated",
                "pane.moved",
                "pane.exited",
                "tab.renamed",
                "tab.moved",
            ]
            .into_iter()
            .map(|kind| json!({"type":kind}))
            .collect();
            let mut bytes = serde_json::to_vec(&json!({"id":"cargo-handler-events",
                "method":"events.subscribe", "params":{"subscriptions":subscriptions}}))
            .map_err(|_| ())?;
            bytes.push(b'\n');
            stream.write_all(&bytes).map_err(|_| ())?;
            stream.set_nonblocking(true).map_err(|_| ())?;
            Ok(stream)
        };
        open().map_or_else(
            |()| Self::unavailable(),
            |stream| Self::Connected {
                stream,
                bytes: Vec::new(),
            },
        )
    }

    fn unavailable() -> Self {
        Self::Unavailable {
            retry_at: Instant::now() + HERDR_SUBSCRIPTION_RETRY,
        }
    }

    fn retry(&mut self, path: &Path, timeout: HerdrReadTimeout) -> bool {
        if matches!(self, Self::Unavailable { retry_at } if Instant::now() >= *retry_at) {
            *self = Self::subscribe(path, timeout);
            return matches!(self, Self::Connected { .. });
        }
        false
    }

    fn changed(&mut self) -> bool {
        let Self::Connected { stream, bytes } = self else {
            return false;
        };
        let mut buffer = [0_u8; 4096];
        loop {
            match stream.read(&mut buffer) {
                Ok(0) => {
                    *self = Self::unavailable();
                    return true;
                },
                Ok(count) => bytes.extend_from_slice(&buffer[..count]),
                Err(error) if error.kind() == ErrorKind::WouldBlock => break,
                Err(_) => {
                    *self = Self::unavailable();
                    return true;
                },
            }
            if bytes.len() as u64 >= HERDR_ANSWER_LIMIT {
                *self = Self::unavailable();
                return true;
            }
        }
        let mut changed = false;
        while let Some(end) = bytes.iter().position(|b| *b == b'\n') {
            let line: Vec<_> = bytes.drain(..=end).collect();
            if let Ok(value) = serde_json::from_slice::<Value>(&line) {
                changed |= value.get("event").is_some();
            }
        }
        changed
    }
}

#[derive(Deserialize)]
struct Snapshot {
    panes: Vec<Pane>,
    tabs:  Vec<Tab>,
}

#[derive(Deserialize)]
struct Pane {
    #[serde(rename = "pane_id")]
    id:                      String,
    tab_id:                  String,
    terminal_id:             String,
    revision:                u64,
    label:                   Option<String>,
    terminal_title_stripped: Option<String>,
}

#[derive(Deserialize)]
struct Tab {
    tab_id: String,
    label:  String,
}

#[derive(Deserialize)]
struct ProcessInfo {
    pane_id:              String,
    shell_pid:            Option<u32>,
    #[serde(default)]
    foreground_processes: Vec<ForegroundProcess>,
}

#[derive(Deserialize)]
struct ForegroundProcess {
    pid: u32,
}

impl HerdrReader {
    fn check_events(&mut self) -> bool {
        let mut changed = false;
        for (path, events) in &mut self.events {
            changed |= events.changed();
            changed |= events.retry(path, self.read_timeout);
        }
        changed
    }

    /// Do not read herdr while every interactive process has a thread link.
    pub(super) fn refresh(
        &mut self,
        processes: &[ProcessEntry],
        links: &mut HashMap<u32, CodexSessionLink>,
    ) {
        if !links
            .values()
            .any(|link| matches!(link, CodexSessionLink::NoMatch))
        {
            self.addresses.clear();
            self.memberships.clear();
            self.events.clear();
            return;
        }
        let clients = classify::interactive_codex(&classify::pid_table(processes), processes);
        self.addresses.retain(|&(pid, started), _| {
            clients.iter().any(|p| p.pid == pid && p.started == started)
        });
        let mut sockets = HashSet::new();
        for client in clients
            .into_iter()
            .filter(|p| matches!(links.get(&p.pid), Some(CodexSessionLink::NoMatch)))
        {
            let address = self
                .addresses
                .entry((client.pid, client.started))
                .or_insert_with(|| process_address(client.pid));
            if let HerdrAddress::Socket(path) = address {
                sockets.insert(path.clone());
            }
        }
        self.events.retain(|path, _| sockets.contains(path));
        for path in &sockets {
            self.events
                .entry(path.clone())
                .or_insert_with(|| PaneEvents::subscribe(path, self.read_timeout))
                .retry(path, self.read_timeout);
        }
        let timeout = self.read_timeout;
        self.refresh_from(processes, links, &sockets, |path, method, params| {
            request(path, method, params, timeout)
        });
    }

    fn refresh_from(
        &mut self,
        processes: &[ProcessEntry],
        links: &mut HashMap<u32, CodexSessionLink>,
        sockets: &HashSet<PathBuf>,
        mut read: impl FnMut(&Path, &str, &Value) -> Result<Value, ()>,
    ) {
        let clients = classify::interactive_codex(&classify::pid_table(processes), processes);
        let table = classify::pid_table(processes);
        let mut names: HashMap<u32, Vec<String>> = HashMap::new();
        let mut present = HashSet::new();
        for socket in sockets {
            if matches!(
                self.events.get(socket),
                Some(PaneEvents::Unavailable { .. })
            ) {
                continue;
            }
            let Ok(answer) = read(socket, "session.snapshot", &json!({})) else {
                continue;
            };
            let Ok(snapshot) = serde_json::from_value::<Snapshot>(answer["snapshot"].clone())
            else {
                continue;
            };
            for pane in snapshot.panes {
                let key = (socket.clone(), pane.id.clone());
                let cached = self.memberships.get(&key).is_some_and(|m| {
                    m.revision == pane.revision
                        && m.terminal_id == pane.terminal_id
                        && m.shells
                            .iter()
                            .chain(&m.foreground)
                            .all(|p| p.is_current(processes))
                });
                if !cached {
                    self.memberships.remove(&key);
                    if let Ok(answer) =
                        read(socket, "pane.process_info", &json!({"pane_id": pane.id}))
                        && let Ok(info) =
                            serde_json::from_value::<ProcessInfo>(answer["process_info"].clone())
                        && info.pane_id == pane.id
                    {
                        self.memberships.insert(
                            key.clone(),
                            PaneMembership {
                                revision:    pane.revision,
                                terminal_id: pane.terminal_id,
                                shells:      info
                                    .shell_pid
                                    .and_then(|pid| PaneProcess::current(pid, processes))
                                    .into_iter()
                                    .collect(),
                                foreground:  info
                                    .foreground_processes
                                    .iter()
                                    .filter_map(|p| PaneProcess::current(p.pid, processes))
                                    .collect(),
                            },
                        );
                    }
                }
                present.insert(key.clone());
                let Some(membership) = self.memberships.get(&key) else {
                    continue;
                };
                let members: Vec<_> = clients
                    .iter()
                    .filter(|p| {
                        membership.foreground.iter().any(|f| f.pid == p.pid)
                            || classify::ancestors(&table, p)
                                .any(|a| membership.shells.iter().any(|s| s.pid == a.pid))
                    })
                    .collect();
                if let [client] = members.as_slice()
                    && let Some(name) = pane_name(
                        pane.label.as_deref(),
                        pane.terminal_title_stripped.as_deref(),
                        &pane.tab_id,
                        &snapshot.tabs,
                    )
                {
                    names.entry(client.pid).or_default().push(name);
                }
            }
        }
        self.memberships.retain(|key, _| present.contains(key));
        for (pid, names) in names {
            if let [name] = names.as_slice()
                && let Some(link @ CodexSessionLink::NoMatch) = links.get_mut(&pid)
            {
                *link = CodexSessionLink::HerdrName(name.clone());
            }
        }
    }
}

fn pane_name(
    label: Option<&str>,
    title: Option<&str>,
    tab_id: &str,
    tabs: &[Tab],
) -> Option<String> {
    label
        .filter(|name| !name.trim().is_empty())
        .or_else(|| {
            tabs.iter()
                .find(|tab| tab.tab_id == tab_id)
                .map(|tab| tab.label.as_str())
                .filter(|name| !name.trim().is_empty())
        })
        .or_else(|| title.map(|name| name.split_once(" | ").map_or(name, |(name, _)| name)))
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
}

/// The socket path comes from the target process, without a default path.
fn process_address(pid: u32) -> HerdrAddress {
    #[cfg(target_os = "linux")]
    let environment: Vec<String> = std::fs::read(format!("/proc/{pid}/environ"))
        .unwrap_or_default()
        .split(|byte| *byte == 0)
        .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
        .collect();
    #[cfg(not(target_os = "linux"))]
    let environment: Vec<String> = {
        let mut system = System::new();
        let pid = Pid::from_u32(pid);
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[pid]),
            true,
            ProcessRefreshKind::nothing().with_environ(UpdateKind::Always),
        );
        system
            .process(pid)
            .map(|p| {
                p.environ()
                    .iter()
                    .map(|v| v.to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default()
    };
    environment
        .iter()
        .find_map(|entry| {
            entry
                .split_once('=')
                .filter(|(key, value)| *key == HERDR_SOCKET_VARIABLE && !value.is_empty())
                .map(|(_, value)| PathBuf::from(value))
        })
        .map_or(HerdrAddress::Absent, HerdrAddress::Socket)
}

/// Send only read requests. A missing or failed API silently leaves the pid.
fn request(
    socket: &Path,
    method: &str,
    params: &Value,
    timeout: HerdrReadTimeout,
) -> Result<Value, ()> {
    let mut stream = UnixStream::connect(socket).map_err(|_| ())?;
    stream
        .set_read_timeout(Some(timeout.duration()))
        .map_err(|_| ())?;
    stream
        .set_write_timeout(Some(timeout.duration()))
        .map_err(|_| ())?;
    let mut bytes =
        serde_json::to_vec(&json!({"id":"cargo-handler", "method":method, "params":params}))
            .map_err(|_| ())?;
    bytes.push(b'\n');
    stream.write_all(&bytes).map_err(|_| ())?;
    let mut answer = String::new();
    BufReader::new(stream.take(HERDR_ANSWER_LIMIT))
        .read_line(&mut answer)
        .map_err(|_| ())?;
    let value: Value = serde_json::from_str(&answer).map_err(|_| ())?;
    if value.get("error").is_some() || value["id"] != "cargo-handler" {
        return Err(());
    }
    value.get("result").cloned().ok_or(())
}

#[cfg(test)]
#[path = "herdr_tests.rs"]
mod tests;
