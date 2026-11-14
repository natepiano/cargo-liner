//! Pane names cannot replace a proven thread or survive process reuse.
#![expect(clippy::expect_used, reason = "tests use isolated API fixtures")]

use std::os::unix::net::UnixListener;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;

use mpsc::Sender;
use ratatui::buffer::Cell;
use tempfile::TempDir;
use thread::JoinHandle;

use super::*;
use crate::census::CensusFacts;
use crate::census::CensusUpdate;
use crate::census::HierarchyNotice;
use crate::census::HierarchySnapshot;
use crate::census::Observed;
use crate::census::WorkerFacts;
use crate::census::account::ConfigHome;
use crate::census::accounts::AccountsTable;
use crate::census::codex::CodexThread;
use crate::census::codex_names::SessionName;
use crate::census::scan::ScanStage;
use crate::config::Config;

fn process(pid: u32, parent: u32, started: u64, name: &str) -> ProcessEntry {
    ProcessEntry {
        pid,
        parent: Some(parent),
        started,
        name: name.into(),
        arguments: vec![name.into()],
        directory: None,
        claude_pid: None,
        config_home: ConfigHome::Unread,
    }
}

fn fixture() -> (
    Vec<ProcessEntry>,
    HashMap<u32, CodexSessionLink>,
    HashSet<PathBuf>,
) {
    (
        vec![process(1, 0, 10, "sh"), process(2, 1, 20, "codex")],
        HashMap::from([(2, CodexSessionLink::NoMatch)]),
        HashSet::from([PathBuf::from("fixture.sock")]),
    )
}

fn snapshot() -> Value {
    json!({"snapshot": {
        "panes": [{"pane_id":"w18:p4", "tab_id":"w18:t4", "terminal_id":"terminal-a", "revision":11,
            "terminal_title_stripped":"wrong title | user"}],
        "tabs": [{"tab_id":"w18:t4", "label":"morty"}]
    }})
}

fn answer(_: &Path, method: &str, _: &Value) -> Result<Value, ()> {
    match method {
        "session.snapshot" => Ok(snapshot()),
        "pane.process_info" => Ok(json!({"process_info":{"pane_id":"w18:p4", "shell_pid":1,
            "foreground_processes":[{"pid":2}]}})),
        _ => Err(()),
    }
}

fn name(link: &CodexSessionLink) -> String {
    match link {
        CodexSessionLink::Named(thread) => thread.label().expect("thread name"),
        CodexSessionLink::HerdrName(name) => name.clone(),
        CodexSessionLink::NoMatch => "pid 2".into(),
    }
}

#[test]
fn a_proven_thread_name_precedes_the_herdr_name() {
    let (processes, mut links, sockets) = fixture();
    links.insert(
        2,
        CodexSessionLink::Named(CodexThread {
            account_home: Path::new("account").into(),
            cwd:          PathBuf::new(),
            created_ms:   0,
            name:         SessionName::Named("proven thread".into()),
            first_prompt: String::new(),
            rollout:      PathBuf::new(),
        }),
    );
    let mut names = HerdrReader::default();
    names.refresh(&processes, &mut links);
    assert!(
        names.memberships.is_empty(),
        "an absent socket has no pane membership"
    );
    names.refresh_from(&processes, &mut links, &sockets, answer);
    assert_eq!(name(&links[&2]), "proven thread");
}

#[test]
fn a_herdr_name_precedes_the_pid_and_a_label_precedes_the_title() {
    let (processes, mut links, sockets) = fixture();
    let mut names = HerdrReader::default();
    names.refresh_from(&processes, &mut links, &sockets, answer);
    assert_eq!(name(&links[&2]), "morty");
    links.insert(2, CodexSessionLink::NoMatch);
    names.refresh_from(&processes, &mut links, &sockets, |path, method, params| {
        let mut value = answer(path, method, params)?;
        if method == "session.snapshot" {
            value["snapshot"]["panes"][0]["label"] = json!("pane name | user");
        }
        Ok(value)
    });
    assert_eq!(name(&links[&2]), "pane name | user");
}

#[test]
fn two_codex_windows_in_one_pane_keep_their_pids() {
    let (mut processes, mut links, sockets) = fixture();
    processes.push(process(3, 1, 30, "codex"));
    links.insert(3, CodexSessionLink::NoMatch);
    HerdrReader::default().refresh_from(&processes, &mut links, &sockets, answer);
    assert!(matches!(links[&2], CodexSessionLink::NoMatch));
    assert!(matches!(links[&3], CodexSessionLink::NoMatch));
}

#[test]
fn an_absent_or_failed_herdr_api_keeps_the_pid() {
    let (processes, mut links, sockets) = fixture();
    let mut names = HerdrReader::default();
    names.refresh_from(&processes, &mut links, &sockets, |_, _, _| Err(()));
    assert_eq!(name(&links[&2]), "pid 2");
    names.refresh_from(&processes, &mut links, &HashSet::new(), answer);
    assert_eq!(name(&links[&2]), "pid 2");
}

#[test]
fn a_reused_pid_cannot_keep_cached_pane_membership() {
    let (mut processes, mut links, sockets) = fixture();
    let mut names = HerdrReader::default();
    names.refresh_from(&processes, &mut links, &sockets, answer);
    assert_eq!(name(&links[&2]), "morty");
    processes[1].started = 40;
    processes[1].parent = Some(99);
    links.insert(2, CodexSessionLink::NoMatch);
    names.refresh_from(&processes, &mut links, &sockets, |_, method, _| {
        if method == "session.snapshot" {
            Ok(snapshot())
        } else {
            Err(())
        }
    });
    assert_eq!(name(&links[&2]), "pid 2");
}

#[test]
fn pane_revisions_control_process_info_reads() {
    let (processes, mut links, sockets) = fixture();
    let mut names = HerdrReader::default();
    let mut process_reads = 0;
    for revision in [11, 11, 12] {
        links.insert(2, CodexSessionLink::NoMatch);
        names.refresh_from(&processes, &mut links, &sockets, |path, method, params| {
            if method == "pane.process_info" {
                process_reads += 1;
            }
            let mut value = answer(path, method, params)?;
            if method == "session.snapshot" {
                value["snapshot"]["panes"][0]["revision"] = json!(revision);
            }
            Ok(value)
        });
        assert_eq!(name(&links[&2]), "morty");
    }
    assert_eq!(process_reads, 2);
}

#[test]
fn the_socket_api_reads_the_real_response_envelope() {
    let directory = tempfile::tempdir().expect("socket directory");
    let path = directory.path().join("herdr.sock");
    let listener = UnixListener::bind(&path).expect("listener");
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().expect("client");
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).expect("request");
        let request: Value = serde_json::from_str(&line).expect("JSON request");
        assert_eq!(request["method"], "session.snapshot");
        reader.get_mut().write_all(b"{\"id\":\"cargo-handler\",\"result\":{\"snapshot\":{\"panes\":[],\"tabs\":[]}}}\n").expect("response");
    });
    let value = request(
        &path,
        "session.snapshot",
        &json!({}),
        HerdrReadTimeout::default(),
    )
    .expect("socket response");
    assert_eq!(value["snapshot"]["panes"], json!([]));
    server.join().expect("server");
}

#[test]
fn a_scan_uses_cached_names_without_waiting_for_the_api() {
    let (processes, mut links, _) = fixture();
    let (send, work) = mpsc::sync_channel(1);
    let (answers, receive) = mpsc::channel();
    let inputs = Arc::new(Mutex::new(None::<NameInputs>));
    let mut names = HerdrNames {
        worker: NameWorker::Running {
            send,
            receive,
            inputs: Arc::clone(&inputs),
        },
        ..HerdrNames::default()
    };
    names.refresh(&processes, &mut links);
    assert_eq!(name(&links[&2]), "pid 2", "the API has not answered");
    assert!(matches!(
        work.recv().expect("new process request"),
        NameWork::Processes
    ));
    let (snapshot_processes, snapshot_generation) = inputs
        .lock()
        .expect("shared inputs")
        .as_ref()
        .map(|snapshot| (snapshot.processes.clone(), snapshot.generation))
        .expect("process snapshot");
    assert_eq!(
        snapshot_processes, processes,
        "the signal supplies every process"
    );
    assert_eq!(snapshot_generation, names.generation);
    let generation = names.generation;
    answers
        .send(NameAnswer {
            panes: HashMap::new(),
            work: HashMap::new(),
            generation,
            names: vec![(
                PaneProcess {
                    pid:     2,
                    started: 20,
                },
                "morty".into(),
            )],
        })
        .expect("completed names");
    names.refresh(&processes, &mut links);
    assert_eq!(name(&links[&2]), "morty");
    assert!(matches!(
        work.recv().expect("event check"),
        NameWork::CheckPaneEvents
    ));
    links.insert(2, CodexSessionLink::NoMatch);
    names.refresh(&processes, &mut links);
    assert_eq!(name(&links[&2]), "morty", "no new API answer is needed");
    assert!(matches!(
        work.recv().expect("event check"),
        NameWork::CheckPaneEvents
    ));
}

#[test]
fn pane_events_invalidate_names_without_repeated_snapshot_requests() {
    let (reader, mut writer) = UnixStream::pair().expect("event stream");
    reader.set_nonblocking(true).expect("nonblocking reader");
    let mut events = PaneEvents::Connected {
        stream: reader,
        bytes:  Vec::new(),
    };
    writer
        .write_all(b"{\"id\":\"cargo-handler-events\",\"result\":{}}\n")
        .expect("subscription answer");
    assert!(
        !events.changed(),
        "a subscription answer does not change a pane"
    );
    writer
        .write_all(b"{\"event\":\"tab_renamed\",\"data\":{\"tab_id\":\"w18:t4\"}}\n")
        .expect("pane change");
    assert!(events.changed());
    assert!(!events.changed(), "the same event is consumed once");
}

#[test]
fn the_initial_probe_uses_the_background_answer() {
    let (processes, mut links, _) = fixture();
    let (send, work) = mpsc::sync_channel(1);
    let (answers, receive) = mpsc::channel();
    let inputs = Arc::new(Mutex::new(None::<NameInputs>));
    let mut names = HerdrNames {
        worker: NameWorker::Running {
            send,
            receive,
            inputs: Arc::clone(&inputs),
        },
        ..HerdrNames::default()
    };
    names.refresh(&processes, &mut links);
    assert!(matches!(
        work.recv().expect("initial request"),
        NameWork::Processes
    ));
    let (snapshot_processes, snapshot_generation) = inputs
        .lock()
        .expect("shared inputs")
        .as_ref()
        .map(|snapshot| (snapshot.processes.clone(), snapshot.generation))
        .expect("process snapshot");
    assert_eq!(
        snapshot_processes, processes,
        "the signal supplies every process"
    );
    assert_eq!(snapshot_generation, names.generation);
    let generation = names.generation;
    answers
        .send(NameAnswer {
            panes: HashMap::new(),
            work: HashMap::new(),
            generation,
            names: vec![(
                PaneProcess {
                    pid:     2,
                    started: 20,
                },
                "morty".into(),
            )],
        })
        .expect("initial answer");
    names.finish_initial(
        &mut links,
        HerdrProbeDeadline::from_scan_start(
            Instant::now(),
            HerdrProbeWait::try_from(100).expect("probe wait"),
        ),
    );
    assert_eq!(name(&links[&2]), "morty");
    names.finish_initial(
        &mut links,
        HerdrProbeDeadline::from_scan_start(
            Instant::now(),
            HerdrProbeWait::try_from(100).expect("probe wait"),
        ),
    );
    assert_eq!(name(&links[&2]), "morty", "a second read does not wait");
}

#[test]
fn an_initial_probe_finishes_when_the_worker_never_answers() {
    let mut probe = ProbeAnswers::new();
    let budgets = probe.finish(
        HerdrProbeWait::try_from(100).expect("probe wait"),
        &[Duration::from_millis(101)],
    );
    assert_eq!(budgets, vec![Duration::from_millis(100)]);
    assert_eq!(name(&probe.links[&2]), "pid 2");
    assert!(matches!(probe.names.initial, InitialAnswer::Pending));
    let ProbeAnswers {
        mut names,
        answers,
        work,
        mut links,
    } = ProbeAnswers::new();
    let (finished, result) = mpsc::channel();
    let deadline = HerdrProbeDeadline::from_scan_start(
        Instant::now(),
        HerdrProbeWait::try_from(20).expect("short probe wait"),
    );
    let worker = thread::spawn(move || {
        names.finish_initial(&mut links, deadline);
        finished
            .send((name(&links[&2]), names.initial))
            .expect("production probe result");
    });
    let received = result.recv_timeout(Duration::from_millis(400));
    drop(answers);
    drop(work);
    worker.join().expect("production probe thread");
    let (label, initial) =
        received.expect("the production deadline must end a silent connected receive");
    assert_eq!(label, "pid 2");
    assert!(matches!(initial, InitialAnswer::Pending));
}

/// The service thread stops before its socket directory is removed.
struct ReturningHerdrService {
    reader:    HerdrReader,
    stop:      Arc<AtomicBool>,
    worker:    ServiceThread,
    directory: TempDir,
}

enum ServiceThread {
    Running(JoinHandle<()>),
    Joined,
}

impl ReturningHerdrService {
    fn new() -> Self {
        let directory = tempfile::tempdir().expect("socket directory");
        let path = directory.path().join("herdr.sock");
        let mut reader = HerdrReader::default();
        reader
            .addresses
            .insert((2, 20), HerdrAddress::Socket(path.clone()));
        reader.events.insert(
            path.clone(),
            PaneEvents::subscribe(&path, HerdrReadTimeout::default()),
        );
        let listener = UnixListener::bind(&path).expect("returned service");
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stopping = Arc::clone(&stop);
        let worker = thread::spawn(move || {
            for request_index in 0..2 {
                let (stream, _) = listener.accept().expect("service connection");
                if stopping.load(Ordering::Acquire) {
                    return;
                }
                stream
                    .set_read_timeout(Some(Duration::from_millis(100)))
                    .expect("bounded service read");
                stream
                    .set_write_timeout(Some(Duration::from_millis(100)))
                    .expect("bounded service write");
                let mut reader = BufReader::new(stream);
                let mut request = String::new();
                if reader.read_line(&mut request).is_err() {
                    return;
                }
                if request_index == 1 {
                    reader.get_mut().write_all(
                        b"{\"id\":\"cargo-handler\",\"result\":{\"snapshot\":{\"panes\":[],\"tabs\":[]}}}\n"
                    ).expect("snapshot answer");
                }
            }
        });
        Self {
            reader,
            stop,
            worker: ServiceThread::Running(worker),
            directory,
        }
    }

    fn path(&self) -> PathBuf { self.directory.path().join("herdr.sock") }
}

impl Drop for ReturningHerdrService {
    fn drop(&mut self) {
        self.reader.events.clear();
        self.stop.store(true, Ordering::Release);
        let _ = UnixStream::connect(self.path());
        if let ServiceThread::Running(worker) =
            std::mem::replace(&mut self.worker, ServiceThread::Joined)
        {
            let result = worker.join();
            if !thread::panicking() {
                result.expect("returned service thread");
            }
        }
    }
}

#[test]
fn a_failed_subscription_retries_when_the_service_returns() {
    let mut service = ReturningHerdrService::new();
    let path = service.path();
    let (processes, mut links, _) = fixture();
    // The retry deadline passes before the next worker refresh.
    service.reader.events.insert(
        path.clone(),
        PaneEvents::Unavailable {
            retry_at: Instant::now(),
        },
    );
    links.insert(2, CodexSessionLink::NoMatch);
    service.reader.refresh(&processes, &mut links);
    assert!(matches!(
        service.reader.events.get(&path),
        Some(PaneEvents::Connected { .. })
    ));
}

#[test]
fn a_failed_event_connection_discards_its_old_names() {
    let (processes, mut links, sockets) = fixture();
    let (stream, peer) = UnixStream::pair().expect("event connection");
    stream.set_nonblocking(true).expect("nonblocking reader");
    let mut reader = HerdrReader::default();
    reader.events.insert(
        PathBuf::from("fixture.sock"),
        PaneEvents::Connected {
            stream,
            bytes: Vec::new(),
        },
    );
    drop(peer);
    assert!(reader.events.values_mut().any(PaneEvents::changed));
    reader.refresh_from(&processes, &mut links, &sockets, answer);
    assert_eq!(
        name(&links[&2]),
        "pid 2",
        "a failed subscription cannot preserve a name"
    );
}

#[test]
fn only_terminal_titles_lose_the_host_suffix() {
    let tabs = vec![Tab {
        tab_id: "tab".into(),
        label:  "tab name | user".into(),
    }];
    assert_eq!(
        pane_name(Some("pane name | user"), Some("title | user"), "tab", &tabs),
        Some("pane name | user".into())
    );
    assert_eq!(
        pane_name(None, Some("title | user"), "tab", &tabs),
        Some("tab name | user".into())
    );
    assert_eq!(
        pane_name(None, Some("title | user"), "absent", &tabs),
        Some("title".into())
    );
}

#[test]
fn a_full_queue_keeps_the_latest_snapshot_without_repeated_copies() {
    let (mut processes, mut links, _) = fixture();
    let (send, work) = mpsc::sync_channel(1);
    let (answers, receive) = mpsc::channel();
    let inputs = Arc::new(Mutex::new(None::<NameInputs>));
    let mut names = HerdrNames {
        worker: NameWorker::Running {
            send,
            receive,
            inputs: Arc::clone(&inputs),
        },
        ..HerdrNames::default()
    };
    names.refresh(&processes, &mut links);
    processes.push(process(3, 1, 30, "codex"));
    links.insert(3, CodexSessionLink::NoMatch);
    names.refresh(&processes, &mut links);
    let pointer = inputs
        .lock()
        .expect("latest snapshot")
        .as_ref()
        .expect("pending inputs")
        .processes
        .as_ptr();
    names.refresh(&processes, &mut links);
    let (process_count, generation, latest_pointer) = inputs
        .lock()
        .expect("latest snapshot")
        .as_ref()
        .map(|latest| {
            (
                latest.processes.len(),
                latest.generation,
                latest.processes.as_ptr(),
            )
        })
        .expect("pending inputs");
    assert_eq!(process_count, 3, "a full queue retains the new process");
    assert_eq!(generation, names.generation);
    assert_eq!(
        latest_pointer, pointer,
        "unchanged clients do not copy the snapshot"
    );
    assert!(
        matches!(work.try_recv(), Ok(NameWork::Processes)),
        "one small signal covers all updates"
    );
    drop(answers);
}

/// A real answer channel with a clock that advances at each controlled read.
struct ProbeAnswers {
    names:   HerdrNames,
    answers: Sender<NameAnswer>,
    work:    Receiver<NameWork>,
    links:   HashMap<u32, CodexSessionLink>,
}

impl ProbeAnswers {
    fn new() -> Self {
        let (processes, mut links, _) = fixture();
        let (send, work) = mpsc::sync_channel(1);
        let (answers, receive) = mpsc::channel();
        let mut names = HerdrNames {
            worker: NameWorker::Running {
                send,
                receive,
                inputs: Arc::default(),
            },
            ..HerdrNames::default()
        };
        names.refresh(&processes, &mut links);
        work.recv().expect("initial request");
        Self {
            names,
            answers,
            work,
            links,
        }
    }

    fn answer(&self, generation: u64, label: &str) {
        self.answers
            .send(NameAnswer {
                panes: HashMap::new(),
                work: HashMap::new(),
                generation,
                names: vec![(
                    PaneProcess {
                        pid:     2,
                        started: 20,
                    },
                    label.to_owned(),
                )],
            })
            .expect("controlled answer");
    }

    fn finish(&mut self, wait: HerdrProbeWait, delays: &[Duration]) -> Vec<Duration> {
        self.finish_after_scan(wait, Duration::ZERO, delays)
    }

    fn finish_after_scan(
        &mut self,
        wait: HerdrProbeWait,
        scan_time: Duration,
        delays: &[Duration],
    ) -> Vec<Duration> {
        let base = Instant::now();
        let elapsed = std::cell::Cell::new(scan_time);
        let mut delays = delays.iter();
        let mut budgets = Vec::new();
        self.names.finish_initial_with(
            &mut self.links,
            HerdrProbeDeadline::from_scan_start(base, wait),
            || base + elapsed.get(),
            |channel, remaining| {
                budgets.push(remaining);
                let delay = delays.next().copied().expect("one delay per channel read");
                elapsed.set(elapsed.get() + delay.min(remaining));
                if delay > remaining {
                    Err(RecvTimeoutError::Timeout)
                } else {
                    channel.recv_timeout(Duration::ZERO)
                }
            },
        );
        budgets
    }
}

#[test]
fn the_probe_keeps_an_answer_that_arrives_after_one_hundred_milliseconds() {
    let mut probe = ProbeAnswers::new();
    probe.answer(probe.names.generation, "morty");
    probe.finish(HerdrProbeWait::default(), &[Duration::from_millis(150)]);
    assert_eq!(name(&probe.links[&2]), "morty");
}

#[test]
fn the_probe_ignores_old_answers_and_keeps_one_deadline() {
    let mut probe = ProbeAnswers::new();
    probe.answer(probe.names.generation - 1, "old pane");
    probe.answer(probe.names.generation, "morty");
    let budgets = probe.finish(
        HerdrProbeWait::try_from(400).expect("wait"),
        &[Duration::from_millis(150), Duration::from_millis(150)],
    );
    assert_eq!(name(&probe.links[&2]), "morty");
    assert_eq!(
        budgets,
        vec![Duration::from_millis(400), Duration::from_millis(250)]
    );
}

#[test]
fn a_configured_probe_wait_bounds_late_answers() {
    for (milliseconds, expected) in [(120, "pid 2"), (200, "morty")] {
        let config: Config =
            toml::from_str(&format!("[census]\nherdr_probe_wait = {milliseconds}\n"))
                .expect("probe config");
        let mut probe = ProbeAnswers::new();
        probe.answer(probe.names.generation, "morty");
        probe.finish(
            config.census.herdr_probe_wait,
            &[Duration::from_millis(150)],
        );
        assert_eq!(name(&probe.links[&2]), expected);
    }
}

#[test]
fn an_old_answer_cannot_extend_the_probe_deadline() {
    let mut probe = ProbeAnswers::new();
    probe.answer(probe.names.generation - 1, "old pane");
    probe.answer(probe.names.generation, "morty");
    probe.finish(
        HerdrProbeWait::try_from(200).expect("wait"),
        &[Duration::from_millis(150), Duration::from_millis(60)],
    );
    assert_eq!(name(&probe.links[&2]), "pid 2");
    assert!(matches!(probe.names.initial, InitialAnswer::Pending));
}

#[test]
fn an_absent_herdr_answer_ends_the_probe_wait_immediately() {
    let mut probe = ProbeAnswers::new();
    probe
        .answers
        .send(NameAnswer {
            panes:      HashMap::new(),
            work:       HashMap::new(),
            generation: probe.names.generation,
            names:      Vec::new(),
        })
        .expect("service absence answer");
    let budgets = probe.finish(HerdrProbeWait::default(), &[Duration::ZERO]);
    assert_eq!(name(&probe.links[&2]), "pid 2");
    assert!(matches!(probe.names.initial, InitialAnswer::Ready));
    assert_eq!(budgets.len(), 1, "an empty answer completes the generation");
    let budgets = probe.finish(HerdrProbeWait::default(), &[]);
    assert!(budgets.is_empty(), "a completed probe must not read again");
}

#[test]
fn a_disconnected_herdr_worker_ends_the_probe_wait_immediately() {
    let ProbeAnswers {
        mut names,
        answers,
        work,
        mut links,
    } = ProbeAnswers::new();
    drop(answers);
    names.finish_initial_with(
        &mut links,
        HerdrProbeDeadline::from_scan_start(Instant::now(), HerdrProbeWait::default()),
        Instant::now,
        |channel, remaining| {
            assert!(remaining > Duration::ZERO);
            assert!(matches!(
                channel.try_recv(),
                Err(mpsc::TryRecvError::Disconnected)
            ));
            Err(RecvTimeoutError::Disconnected)
        },
    );
    assert_eq!(name(&links[&2]), "pid 2");
    drop(work);
}

#[test]
fn a_new_generation_waits_even_after_an_earlier_generation_is_ready() {
    let mut probe = ProbeAnswers::new();
    probe.answer(probe.names.generation, "old pane");
    probe.finish(HerdrProbeWait::default(), &[Duration::ZERO]);
    let (mut processes, mut links, _) = fixture();
    processes.push(process(3, 1, 30, "codex"));
    links.insert(3, CodexSessionLink::NoMatch);
    probe.names.refresh(&processes, &mut links);
    probe.links = links;
    probe.work.recv().expect("new process request");
    probe.answer(probe.names.generation, "morty");
    probe.finish(HerdrProbeWait::default(), &[Duration::ZERO]);
    assert_eq!(name(&probe.links[&2]), "morty");
}

#[test]
fn scan_time_reduces_the_channel_wait() {
    let mut probe = ProbeAnswers::new();
    probe.answer(probe.names.generation, "morty");
    let budgets = probe.finish_after_scan(
        HerdrProbeWait::try_from(400).expect("wait"),
        Duration::from_millis(150),
        &[Duration::from_millis(200)],
    );
    assert_eq!(name(&probe.links[&2]), "morty");
    assert_eq!(budgets, vec![Duration::from_millis(250)]);
}

#[test]
fn a_scan_past_the_deadline_does_not_wait_for_late_names() {
    let mut probe = ProbeAnswers::new();
    probe.answer(probe.names.generation, "morty");
    let budgets = probe.finish_after_scan(
        HerdrProbeWait::try_from(100).expect("wait"),
        Duration::from_millis(150),
        &[Duration::from_millis(1)],
    );
    assert_eq!(name(&probe.links[&2]), "pid 2");
    assert_eq!(budgets, vec![Duration::ZERO]);
}

#[test]
fn a_scan_past_the_deadline_keeps_names_that_already_arrived() {
    let mut probe = ProbeAnswers::new();
    probe.answer(probe.names.generation, "morty");
    probe.finish_after_scan(
        HerdrProbeWait::try_from(100).expect("wait"),
        Duration::from_millis(150),
        &[Duration::ZERO],
    );
    assert_eq!(name(&probe.links[&2]), "morty");
}

#[test]
fn raw_pane_states_apply_to_proven_and_nested_codex_processes() {
    let (mut processes, mut links, sockets) = fixture();
    processes[0].name = "claude".into();
    processes[0].arguments = vec!["claude".into()];
    links.insert(
        2,
        CodexSessionLink::Named(CodexThread {
            account_home: Path::new("/fixture/account").into(),
            cwd:          PathBuf::new(),
            created_ms:   0,
            name:         SessionName::Named("proven thread".into()),
            first_prompt: String::new(),
            rollout:      PathBuf::new(),
        }),
    );
    let mut reader = HerdrReader::default();
    let mut status_paint = Vec::new();
    for (raw, expected, observed) in [
        (
            json!("working"),
            PaneWork::Busy,
            Observed::Known(PaneState::Open),
        ),
        (
            json!("idle"),
            PaneWork::Idle,
            Observed::Known(PaneState::Open),
        ),
        (
            json!("done"),
            PaneWork::Idle,
            Observed::Known(PaneState::Open),
        ),
        (
            json!("stopped"),
            PaneWork::Busy,
            Observed::Known(PaneState::Stopped),
        ),
        (
            json!("exited"),
            PaneWork::Busy,
            Observed::Known(PaneState::Closed),
        ),
        (
            json!("dead"),
            PaneWork::Busy,
            Observed::Known(PaneState::Closed),
        ),
        (
            json!("future-state"),
            PaneWork::Unknown,
            crate::census::Observed::unknown("unrecognized pane state: future-state"),
        ),
        (
            Value::Null,
            PaneWork::Unknown,
            crate::census::Observed::unknown("pane state is not reported"),
        ),
        (
            json!({"future":"shape"}),
            PaneWork::Unknown,
            crate::census::Observed::unknown("pane state is not reported"),
        ),
    ] {
        let mut snapshots = 0;
        reader.refresh_from(&processes, &mut links, &sockets, |path, method, params| {
            let mut value = answer(path, method, params)?;
            if method == "session.snapshot" {
                snapshots += 1;
                value["snapshot"]["panes"][0]["agent_status"] = raw.clone();
            }
            Ok(value)
        });
        assert_eq!(snapshots, 1, "names and state share one snapshot");
        assert_eq!(reader.work.get(&(2, 20)), Some(&expected));
        assert_eq!(reader.panes.get(&(2, 20)), Some(&observed));
        let painted = drawn_accounts_status(observed).expect("drawn busy status");
        if status_paint.is_empty() {
            status_paint = painted;
        } else {
            assert_eq!(
                painted, status_paint,
                "pane lifecycle cannot change status text or ink"
            );
        }
        assert_eq!(name(&links[&2]), "proven thread");
    }
    reader.refresh_from(&processes, &mut links, &sockets, |_, _, _| Err(()));
    assert!(
        reader.work.is_empty(),
        "a failed read cannot keep an old work state"
    );
}

#[test]
fn claude_pane_records_only_its_pane_state() {
    let (mut processes, mut links, sockets) = fixture();
    processes[1] = process(2, 1, 20, "claude");
    links.clear();
    let mut reader = HerdrReader::default();
    reader.refresh_from(&processes, &mut links, &sockets, |path, method, params| {
        let mut value = answer(path, method, params)?;
        if method == "session.snapshot" {
            value["snapshot"]["panes"][0]["agent_status"] = json!("working");
        }
        Ok(value)
    });
    assert_eq!(
        reader.work.get(&(2, 20)),
        None,
        "a Claude tile keeps its status source"
    );
    assert_eq!(
        reader.panes.get(&(2, 20)),
        Some(&Observed::Known(PaneState::Open))
    );
    assert!(links.is_empty(), "a Claude pane names no Codex session");
}

fn drawn_accounts_status(pane: Observed<PaneState>) -> Result<Vec<Cell>, &'static str> {
    use tui_pane::TerminalApp;
    let mut app = crate::app::App::new_for_test().expect("isolated app");
    let rows = serde_json::from_value(json!([{
        "agent":"claude","session_id":"pane-worker","name":"pane-worker",
        "started":100,"pid":2,"directory":"/fixture","children":[]
    }]))
    .expect("pane worker row");
    let facts = WorkerFacts {
        pane,
        ..crate::census::WorkerFacts::older_probe()
    };
    app.census.apply(CensusUpdate::Local {
        rows,
        hierarchy: HierarchySnapshot {
            edges:  crate::census::HierarchyEdges::default(),
            notice: HierarchyNotice::Clear,
            facts:  CensusFacts {
                workers: Observed::Known(std::collections::BTreeMap::from([(
                    "pane-worker".into(),
                    facts,
                )])),
                ..crate::census::CensusFacts::default()
            }
            .into(),
        },
        stage: ScanStage::Completed,
    });
    app.accounts.table = AccountsTable::Ready(
        crate::census::accounts::AccountsReading::parse(
            &serde_json::to_vec(&json!({
                "accounts":[{"label":"fixture","roles":[],"use_credits":false,
                    "sessions":[{"session":"pane-worker","state":"busy"}]}],"problems":[]
            }))
            .expect("accounts JSON"),
        )
        .expect("accounts reading"),
    );
    let mut terminal =
        ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 24)).expect("test terminal");
    let keymap = app.keymap();
    terminal
        .draw(|frame| app.draw(frame, &keymap))
        .expect("first frame");
    app.tiles.settle_for_test();
    terminal
        .draw(|frame| app.draw(frame, &keymap))
        .expect("settled frame");
    let buffer = terminal.backend().buffer();
    for y in 0..buffer.area.height {
        let line: String = (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol())
            .collect();
        if line.contains("pane-worker")
            && let Some(offset) = line.find("busy")
        {
            let x = u16::try_from(line[..offset].chars().count()).expect("status column");
            return Ok((x..x + 4).map(|x| buffer[(x, y)].clone()).collect());
        }
    }
    Err("busy status must draw")
}

#[test]
fn a_completed_snapshot_delivers_work_with_the_same_generation() {
    let mut probe = ProbeAnswers::new();
    probe
        .answers
        .send(NameAnswer {
            panes:      HashMap::new(),
            generation: probe.names.generation,
            names:      Vec::new(),
            work:       HashMap::from([((2, 20), PaneWork::Busy)]),
        })
        .expect("completed pane state");
    probe.finish(HerdrProbeWait::default(), &[Duration::ZERO]);
    assert_eq!(probe.names.work(2, 20), PaneWork::Busy);
    assert_eq!(
        probe.names.work(2, 21),
        PaneWork::Unknown,
        "pid reuse cannot retain a work state"
    );
}
