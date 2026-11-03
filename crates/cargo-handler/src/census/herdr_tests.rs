//! Pane names cannot replace a proven thread or survive process reuse.
#![expect(clippy::expect_used, reason = "tests use isolated API fixtures")]

use std::os::unix::net::UnixListener;

use super::*;
use crate::census::account::ConfigHome;
use crate::census::codex::CodexThread;
use crate::census::codex_names::SessionName;

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
        "proven links need no herdr read"
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
    assert_eq!(name(&links[&2]), "pane name");
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
    let value = request(&path, "session.snapshot", &json!({})).expect("socket response");
    assert_eq!(value["snapshot"]["panes"], json!([]));
    server.join().expect("server");
}

#[test]
fn a_scan_uses_cached_names_without_waiting_for_the_api() {
    let (processes, mut links, _) = fixture();
    let (send, work) = mpsc::sync_channel(1);
    let (answers, receive) = mpsc::channel();
    let mut names = HerdrNames {
        worker: NameWorker::Running { send, receive },
        ..HerdrNames::default()
    };
    names.refresh(&processes, &mut links);
    assert_eq!(name(&links[&2]), "pid 2", "the API has not answered");
    assert!(matches!(
        work.recv().expect("new process request"),
        NameWork::Processes { .. }
    ));
    let generation = names.generation;
    answers
        .send(NameAnswer {
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
    let mut names = HerdrNames {
        worker: NameWorker::Running { send, receive },
        ..HerdrNames::default()
    };
    names.refresh(&processes, &mut links);
    assert!(matches!(
        work.recv().expect("initial request"),
        NameWork::Processes { .. }
    ));
    let generation = names.generation;
    answers
        .send(NameAnswer {
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
    names.finish_initial(&mut links);
    assert_eq!(name(&links[&2]), "morty");
    names.finish_initial(&mut links);
    assert_eq!(name(&links[&2]), "morty", "a second read does not wait");
}
