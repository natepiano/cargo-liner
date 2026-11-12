//! Scan-time integration chooses detached gates over the conversation's tool.

#![expect(clippy::expect_used, reason = "isolated scan fixture files must parse")]

use std::error::Error;
use std::io::Write;

use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Position;
use serde_json::Value;
use tempfile::TempDir;
use tui_pane::TerminalApp;

use super::*;
use crate::app::App;
use crate::census::CensusUpdate;
use crate::census::ChildRow;
use crate::census::HierarchyEdges;
use crate::census::HierarchyRole;
use crate::census::HierarchySource;
use crate::census::ProvenCodexThread;
use crate::census::RoleFallback;
use crate::census::RowIdentity;
use crate::census::StaffFileSetting;
use crate::census::account::ConfigHome;
use crate::census::accounts::AccountsReading;
use crate::census::accounts::AccountsTable;
use crate::census::hierarchy::WorkerReference;
use crate::census::placement::DispatchWorker;
use crate::census::tier::ThreadWorktree;
use crate::census::worker_dispatch;
use crate::census::worker_dispatch::DIRECTOR_FILE;

const THREAD: &str = "01a1437d-1234-5678-9abc-0123456789ab";

/// A unit with an idle director and a detached worker verification.
struct DetachedWorkerFixture {
    scanner: LocalScanner,
    session: PathBuf,
    root:    TempDir,
}

impl DetachedWorkerFixture {
    fn new() -> Self {
        let root = tempfile::tempdir().expect("session root");
        let session = root.path().join("unit");
        fs::create_dir_all(session.join("detached_runs")).expect("run directory");
        fs::write(session.join(DIRECTOR_FILE), "director-id\n").expect("director identity");
        worker_dispatch::write_worker_fixture(
            &session,
            &serde_json::json!({
                "fixture-impl": {"current_session":THREAD,"role":"test"}
            }),
        );
        fs::remove_file(session.join("progress_history_state.json"))
            .expect("legacy director fixture");
        let mut scanner = LocalScanner::new(AccountLabels::default());
        scanner.worker_names = worker_dispatch::fixture_names(&session);
        scanner.session_roots = SessionRoots::new([root.path().to_path_buf()]);
        scanner.run_records = activity::RunRecords::for_test();
        scanner.desktop_source = DesktopSource::Windows(Vec::new());
        scanner = scanner.with_staff_file(
            StaffFileSetting::new(HierarchySource::File(
                session.join("staff.json").to_string_lossy().into_owned(),
            )),
            StaffFileReader::default(),
        );
        scanner
            .worker_names
            .set_staff(scanner.staff.snapshot().roster);
        Self {
            scanner,
            session,
            root,
        }
    }

    fn write_run(&self, status: &str) { write_run(&self.session, status); }

    fn supervisor(&self) -> ProcessEntry { supervisor(&self.session) }

    fn attach_activity(&mut self, processes: &[ProcessEntry], rows: &mut [AgentRow]) {
        self.scanner
            .attach_activity_and_names(processes, &HashMap::new(), rows);
    }

    fn rows() -> Vec<AgentRow> {
        serde_json::from_value(serde_json::json!([{
            "agent": "claude", "session_id": "director-id", "name": "director",
            "pid": 10, "started": 10, "status": "idle", "directory": "/fixture",
            "desktop": null, "branch": null, "launched_by": null, "children": []
        }]))
        .expect("director row")
    }

    fn scan(&mut self, processes: &[ProcessEntry]) -> Vec<AgentRow> {
        let staff = self.scanner.staff.snapshot();
        self.scanner.worker_names.set_staff(staff.roster);
        let mut rows = Self::rows();
        self.scanner
            .attach_details(processes, &[], Some(self.root.path()), &mut rows);
        rows
    }
}

#[test]
fn detached_worker_run_keeps_its_row_and_step_without_a_launcher() -> Result<(), &'static str> {
    let mut fixture = DetachedWorkerFixture::new();
    fixture.write_run("running");
    let mut processes = vec![claude(), fixture.supervisor()];
    let waiting = fixture.scan(&processes);
    assert_eq!(waiting[0].activity, AgentActivity::Idle);
    assert_eq!(waiting[0].children.len(), 1);
    let worker = &waiting[0].children[0];
    assert_eq!(worker.name, "fixture-impl");
    assert_eq!(worker.activity.text(), "waiting");
    assert_eq!(worker.thread_id, CodexThreadId::Recorded(THREAD.to_owned()));
    let WorkerDispatch::Worker(dispatch) = &worker.dispatch else {
        return Err("a live run must retain its roster identity");
    };
    assert_eq!(
        dispatch.fallback(),
        RoleFallback::Worker(HierarchyRole::Test)
    );
    processes.push(process(101, 100, &["cargo", "nextest", "run"]));
    let running = fixture.scan(&processes);
    assert_eq!(running[0].activity, AgentActivity::Idle);
    assert_eq!(running[0].children.len(), 1);
    assert_eq!(running[0].children[0].activity.text(), "nextest");
    fixture.write_run("finished");
    fixture
        .scanner
        .run_records
        .record_changed(&fixture.session, "gate.json".into());
    assert_eq!(fixture.scan(&processes)[0].children, []);
    Ok(())
}

#[test]
fn detached_worker_run_needs_a_live_process_and_its_own_director() {
    let mut fixture = DetachedWorkerFixture::new();
    fixture.write_run("running");
    assert_eq!(fixture.scan(&[])[0].children, []);
    fs::write(fixture.session.join(DIRECTOR_FILE), "another-director").expect("other director");
    let processes = [fixture.supervisor()];
    assert_eq!(fixture.scan(&processes)[0].children, []);
}

#[test]
fn detached_worker_run_reuses_an_existing_thread_row() {
    let mut fixture = DetachedWorkerFixture::new();
    fixture.write_run("running");
    let processes = [claude(), fixture.supervisor()];
    let mut rows = DetachedWorkerFixture::rows();
    let worker: ChildRow = serde_json::from_value(serde_json::json!({
        "kind": "thread", "thread_id": THREAD, "depth": 0,
        "name": "open thread", "pid": null, "started": 20, "timer": null
    }))
    .expect("launcher thread row");
    rows[0].children.push(worker);
    fixture.attach_activity(&processes, &mut rows);
    assert_eq!(rows[0].children.len(), 1);
    assert_eq!(rows[0].children[0].started, 20);
    assert_eq!(rows[0].children[0].activity.text(), "waiting");
    assert_eq!(rows[0].activity, AgentActivity::Idle);
}

/// Current producers record stable staff IDs without a thread copy.
fn write_staff_run(fixture: &DetachedWorkerFixture, status: &str) {
    fs::write(
        fixture.session.join("detached_runs/gate.json"),
        serde_json::json!({
            "command": ["bash", "verify.sh", "check", "cargo-handler"],
            "delivery": "", "director_dir": fixture.session,
            "done_message_file": "", "exit_code": null, "finished_at": null,
            "label": "worker gate", "log": fixture.session.join("gate.log"),
            "pid": 100, "run_id": "gate", "session_dir": fixture.session,
            "staff_id": "stable-worker", "started_at": 100.0, "status": status,
            "tree_key": "fixture", "worker": "fixture-test"
        })
        .to_string(),
    )
    .expect("current run record");
}

fn write_staff_membership(fixture: &DetachedWorkerFixture, thread: &str) {
    fs::write(
        fixture.root.path().join("staff.json"),
        serde_json::json!({"schema": 2, "rows": [
            {"id": "boss", "kind": "showrunner", "name": "boss",
             "parent": null, "current_session": "boss-session"},
            {"id": "stable-unit", "kind": "unit", "name": "fixture",
             "parent": "boss", "current_session": "director-id"},
            {"id": "stable-worker", "kind": "worker", "name": "fixture-test",
             "parent": "stable-unit", "current_session": thread, "role":"test"}
        ]})
        .to_string(),
    )
    .expect("staff membership");
}

fn current_staff_fixture(thread: &str) -> DetachedWorkerFixture {
    let mut fixture = DetachedWorkerFixture::new();
    write_staff_membership(&fixture, thread);
    let setting = StaffFileSetting::new(HierarchySource::File(
        fixture
            .root
            .path()
            .join("staff.json")
            .to_string_lossy()
            .into_owned(),
    ));
    fixture.scanner = fixture
        .scanner
        .with_staff_file(setting, StaffFileReader::default());
    fixture
        .scanner
        .worker_names
        .set_staff(fixture.scanner.staff.snapshot().roster);
    write_staff_run(&fixture, "running");
    fixture
}

fn drawn_rows(buffer: &Buffer) -> Vec<String> {
    buffer
        .content
        .chunks(usize::from(buffer.area.width))
        .map(|cells| cells.iter().map(ratatui::buffer::Cell::symbol).collect())
        .collect()
}

fn assert_drawn_worker_details(rows: Vec<AgentRow>, expected: &str) -> Result<(), Box<dyn Error>> {
    let mut app = App::new_for_test()?;
    app.census.apply(CensusUpdate::Local {
        rows,
        hierarchy: HierarchyEdges::default().into(),
        stage: ScanStage::Completed,
    });
    app.accounts.table = AccountsTable::Ready(
        AccountsReading::parse(&serde_json::to_vec(
            &serde_json::json!({"accounts":[{"label":"fixture","roles":[],"use_credits":false,
            "sessions":[{"session":THREAD,"state":"idle"}]}],"problems":[]}),
        )?)
        .map_err(|error| error.to_string())?,
    );
    let mut terminal = Terminal::new(TestBackend::new(180, 45))?;
    let keymap = app.keymap();
    terminal.draw(|frame| app.draw(frame, &keymap))?;
    app.tiles.settle_for_test();
    terminal.draw(|frame| app.draw(frame, &keymap))?;
    let before = drawn_rows(terminal.backend().buffer());
    let (y, line) = before
        .iter()
        .enumerate()
        .find(|(_, line)| line.contains("fixture-test"))
        .ok_or("the detached run must draw its worker row")?;
    let x = line
        .find("fixture-test")
        .ok_or("the worker name must have a column")?;
    let x = u16::try_from(line[..x].chars().count())?;
    let status_x = before[..y]
        .iter()
        .rev()
        .find_map(|header| header.find("status").map(|x| header[..x].chars().count()))
        .ok_or("the worker row must have a status column")?;
    let status: String = line.chars().skip(status_x).collect();
    assert_eq!(status.split_whitespace().next(), Some("idle"));
    assert!(
        !line.contains(expected),
        "the detailed step belongs in the details view"
    );
    app.click(Position::new(x, u16::try_from(y)?));
    tui_pane::dispatch_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE),
    );
    terminal.draw(|frame| app.draw(frame, &keymap))?;
    let details = drawn_rows(terminal.backend().buffer());
    for property in [format!("state: {expected}"), "status: idle".to_owned()] {
        assert!(
            details
                .iter()
                .any(|line| line.split('│').any(|cell| cell.trim() == property)),
            "missing {property}: {details:?}"
        );
    }
    Ok(())
}

#[test]
fn staff_run_keeps_one_waiting_worker_without_a_launcher_or_open_thread()
-> Result<(), Box<dyn Error>> {
    for use_staff_id in [true, false] {
        let mut fixture = current_staff_fixture(THREAD);
        if !use_staff_id {
            let path = fixture.session.join("detached_runs/gate.json");
            let mut record: Value = serde_json::from_slice(&fs::read(&path)?)?;
            record
                .as_object_mut()
                .expect("run record object")
                .remove("staff_id");
            fs::write(path, serde_json::to_vec(&record)?)?;
        }
        assert_eq!(fixture.scan(&[claude()])[0].children, []);
        let processes = [claude(), fixture.supervisor()];
        let waiting = fixture.scan(&processes);
        assert_eq!(waiting[0].activity, AgentActivity::Idle);
        assert_eq!(waiting[0].children.len(), 1);
        let worker = &waiting[0].children[0];
        assert_eq!(worker.thread_id, CodexThreadId::Recorded(THREAD.to_owned()));
        assert_eq!(worker.activity.text(), "waiting");
        let WorkerDispatch::Worker(dispatch) = &worker.dispatch else {
            return Err("the staff roster must supply the worker role".into());
        };
        assert_eq!(
            dispatch.fallback(),
            RoleFallback::Worker(HierarchyRole::Test)
        );
        assert_drawn_worker_details(waiting.clone(), "waiting")?;
        let checking = [
            claude(),
            fixture.supervisor(),
            process(101, 100, &["cargo", "check"]),
        ];
        let active = fixture.scan(&checking);
        assert_eq!(active[0].children.len(), 1);
        assert_eq!(active[0].children[0].activity.text(), "check");
        assert_eq!(active[0].activity, AgentActivity::Idle);
        assert_drawn_worker_details(active, "check")?;
        for (command, cargo_step, expected) in
            [("lint", "clippy", "clippy"), ("test", "nextest", "nextest")]
        {
            let path = fixture.session.join("detached_runs/gate.json");
            let mut record: Value = serde_json::from_slice(&fs::read(&path)?)?;
            record["command"] = serde_json::json!(["bash", "verify.sh", command, "cargo-handler"]);
            fs::write(path, serde_json::to_vec(&record)?)?;
            fixture
                .scanner
                .run_records
                .record_changed(&fixture.session, "gate.json".into());
            let active = fixture.scan(&[
                claude(),
                fixture.supervisor(),
                process(101, 100, &["cargo", cargo_step]),
            ]);
            assert_eq!(active[0].children.len(), 1);
            assert_eq!(active[0].children[0].activity.text(), expected);
            assert_drawn_worker_details(active, expected)?;
        }
        write_staff_run(&fixture, "finished");
        fixture
            .scanner
            .run_records
            .record_changed(&fixture.session, "gate.json".into());
        assert_eq!(fixture.scan(&processes)[0].children, []);
    }
    Ok(())
}

#[test]
fn supervisor_keeps_the_staff_snapshot_without_a_live_director() {
    let mut fixture = DetachedWorkerFixture::new();
    fixture.write_run("running");
    let processes = [fixture.supervisor()];
    let staff = fixture.scanner.worker_names.staff_snapshot();
    let mut rows = Vec::new();
    fixture
        .scanner
        .attach_details(&processes, &[], Some(fixture.root.path()), &mut rows);
    assert_eq!(rows, []);
    assert!(Arc::ptr_eq(
        &staff,
        &fixture.scanner.worker_names.staff_snapshot()
    ));
    fixture
        .scanner
        .attach_details(&processes, &[], Some(fixture.root.path()), &mut rows);
    assert_eq!(rows, []);
    assert!(Arc::ptr_eq(
        &staff,
        &fixture.scanner.worker_names.staff_snapshot()
    ));
}

#[test]
fn staff_handoff_moves_run_activity_without_changing_run_records() {
    let mut fixture = current_staff_fixture(THREAD);
    let processes = [claude(), fixture.supervisor()];
    let staff = fixture.scanner.worker_names.staff_snapshot();
    assert_eq!(fixture.scan(&processes)[0].children.len(), 1);
    assert!(Arc::ptr_eq(
        &staff,
        &fixture.scanner.worker_names.staff_snapshot()
    ));
    let run = fs::read(fixture.session.join("detached_runs/gate.json")).expect("run bytes");
    write_staff_membership(&fixture, "replacement-thread");
    let changed = fixture.scan(&processes);
    assert_eq!(changed[0].children.len(), 1);
    assert_eq!(
        changed[0].children[0].thread_id,
        CodexThreadId::Recorded("replacement-thread".into())
    );
    assert_eq!(changed[0].children[0].activity.text(), "waiting");
    assert_eq!(changed[0].activity, AgentActivity::Idle);
    assert!(!Arc::ptr_eq(
        &staff,
        &fixture.scanner.worker_names.staff_snapshot()
    ));
    assert_eq!(
        fs::read(fixture.session.join("detached_runs/gate.json")).expect("run bytes"),
        run
    );
    assert_eq!(
        WorkerReference::Staff {
            staff_id: "stable-worker".into(),
        }
        .session(&fixture.scanner.worker_names.staff_snapshot()),
        Some("replacement-thread")
    );
}

#[test]
fn a_staff_run_keeps_one_worker_under_a_codex_director() {
    let mut fixture = current_staff_fixture(THREAD);
    let mut rows = DetachedWorkerFixture::rows();
    rows[0].agent = Agent::Codex;
    rows[0].session_id =
        SessionIdentity::Codex(ProvenCodexThread::OpenStaffRollout("director-id".into()));
    let processes = [process(10, 1, &["codex"]), fixture.supervisor()];
    fixture
        .scanner
        .attach_details(&processes, &[], Some(fixture.root.path()), &mut rows);
    assert_eq!(rows[0].children.len(), 1);
    assert_eq!(
        rows[0].children[0].thread_id,
        CodexThreadId::Recorded(THREAD.into())
    );
    assert_eq!(rows[0].children[0].activity.text(), "waiting");
    assert_eq!(rows[0].activity, AgentActivity::Idle);
    write_staff_run(&fixture, "finished");
    fixture
        .scanner
        .run_records
        .record_changed(&fixture.session, "gate.json".into());
    fixture
        .scanner
        .attach_details(&processes, &[], Some(fixture.root.path()), &mut rows);
    assert_eq!(rows[0].children, []);
}

#[test]
fn staff_membership_keeps_a_detached_worker_under_a_restarted_director() {
    let mut fixture = current_staff_fixture("replacement-thread");
    fs::write(fixture.session.join(DIRECTOR_FILE), "retired-director").expect("stale director");
    let processes = [claude(), fixture.supervisor()];
    let waiting = fixture.scan(&processes);
    assert_eq!(waiting[0].session_id.claude_id(), Some("director-id"));
    assert_eq!(waiting[0].children.len(), 1);
    assert_eq!(
        waiting[0].children[0].thread_id,
        CodexThreadId::Recorded("replacement-thread".into())
    );
    assert_eq!(waiting[0].children[0].activity.text(), "waiting");
    assert_eq!(waiting[0].activity, AgentActivity::Idle);
}

#[test]
fn staff_run_updates_the_existing_current_thread_without_a_second_row() {
    let mut fixture = current_staff_fixture("replacement-thread");
    let processes = [claude(), fixture.supervisor()];
    let mut rows = DetachedWorkerFixture::rows();
    let worker: ChildRow = serde_json::from_value(serde_json::json!({
        "kind": "thread", "thread_id": "replacement-thread", "depth": 0,
        "name": "open thread", "pid": null, "started": 20, "timer": null
    }))
    .expect("current thread row");
    rows[0].children.push(worker);
    fixture.attach_activity(&processes, &mut rows);
    assert_eq!(rows[0].children.len(), 1);
    assert_eq!(
        rows[0].children[0].thread_id,
        CodexThreadId::Recorded("replacement-thread".into())
    );
    assert_eq!(rows[0].children[0].started, 20);
    assert_eq!(rows[0].children[0].activity.text(), "waiting");
    assert_eq!(rows[0].activity, AgentActivity::Idle);
}

#[test]
fn detached_worker_run_overrides_an_idle_rollout_without_session_paths() {
    let mut fixture = DetachedWorkerFixture::new();
    fixture.write_run("running");
    let rollout = fixture
        .root
        .path()
        .join(format!("rollout-test-{THREAD}.jsonl"));
    fs::write(&rollout, format!("{}\n", user_message("Write fixtures"))).expect("idle rollout");
    fixture.scanner.rollouts.read(&rollout);
    let processes = [fixture.supervisor()];
    let rows = fixture.scan(&processes);
    assert_eq!(rows[0].children.len(), 1);
    assert_eq!(rows[0].children[0].activity.text(), "waiting");
    assert_eq!(rows[0].children[0].name, "fixture-impl");
    assert_eq!(rows[0].activity, AgentActivity::Idle);
}

#[test]
fn detached_worker_launcher_and_run_share_one_visible_thread() {
    let mut fixture = DetachedWorkerFixture::new();
    fixture.write_run("running");
    let command = format!(
        "bash /home/nate/.claude/scripts/direct/implement.sh --to fixture-impl '{}' /work/fixture /prompt.md impl 'Write fixtures' impl 'implement worker' 0 impl",
        fixture.session.display()
    );
    let processes = [claude(), shell_wrapper(20, &command), fixture.supervisor()];
    let rows = fixture.scan(&processes);
    let children: Vec<_> = rows[0]
        .children
        .iter()
        .map(|child| {
            (
                RowIdentity {
                    machine: "local",
                    pid:     10,
                },
                child,
            )
        })
        .collect();
    let folded: Vec<_> = children
        .iter()
        .filter_map(|(tile, shell)| {
            children.iter().find_map(|(worker_tile, thread)| {
                crate::census::placement::DispatchPairing::join(
                    *tile,
                    shell,
                    *worker_tile,
                    DispatchWorker::Thread(thread),
                )
                .ok()
                .map(|_| *thread)
            })
        })
        .collect();
    assert_eq!(folded.len(), 1, "the launcher folds into its run thread");
    assert_eq!(
        children.len() - folded.len(),
        1,
        "only one worker row is visible"
    );
    assert_eq!(folded[0].activity.text(), "waiting");
    assert_eq!(rows[0].activity, AgentActivity::Idle);
}

fn write_run(session: &Path, status: &str) {
    fs::write(
        session.join("detached_runs/gate.json"),
        serde_json::json!({
            "thread_id": THREAD, "pid": 100, "started_at": 100.0,
            "command": ["bash", VERIFY_SCRIPT, "test", "cargo-handler"], "status": status
        })
        .to_string(),
    )
    .expect("run record");
}

fn supervisor(session: &Path) -> ProcessEntry {
    let record = session.join("detached_runs/gate.json");
    process(
        100,
        1,
        &[
            "python3",
            "/fixture/.claude/scripts/agents/detached_runs.py",
            "supervise",
            record.to_str().expect("record path"),
        ],
    )
}

fn claude() -> ProcessEntry { process(10, 1, &["claude"]) }

fn shell_wrapper(pid: u32, command: &str) -> ProcessEntry {
    let escaped = command.replace('\'', "'\\''");
    let script = format!(
        "source /fixture/.claude/shell-snapshots/snapshot-zsh-1.sh && eval '{escaped}' < /dev/null"
    );
    process(pid, 10, &["zsh", "-c", &script])
}

fn user_message(text: &str) -> Value {
    serde_json::json!({"type":"response_item", "payload":{"type":"message", "role":"user", "content":[{"type":"input_text", "text":text}]}})
}

pub(super) fn process(pid: u32, parent: u32, args: &[&str]) -> ProcessEntry {
    ProcessEntry {
        pid,
        parent: Some(parent),
        name: args[0].into(),
        arguments: args.iter().map(|arg| (*arg).to_owned()).collect(),
        started: 100,
        directory: None,
        claude_pid: None,
        config_home: ConfigHome::Unread,
    }
}

#[test]
fn activity_scan_detached_gate_overrides_tool_then_finished_run_reveals_the_tool() {
    let root = tempfile::tempdir().expect("isolated session root");
    let directory = tempfile::Builder::new()
        .prefix("handler-activity-")
        .tempdir_in(root.path())
        .expect("isolated session");
    let run_dir = directory.path().join("detached_runs");
    fs::create_dir(&run_dir).expect("run directory");
    write_run(directory.path(), "running");
    let rollout = directory.path().join(format!("rollout-{THREAD}.jsonl"));
    let lines = [
        serde_json::json!({"type":"session_meta","payload":{"cwd":"/work/fixture"}}),
        serde_json::json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":format!("Write fixtures\nSummary: {}/phases/4/summary.txt",directory.path().display())}]}}),
        serde_json::json!({"type":"event_msg","payload":{"type":"task_started"}}),
        serde_json::json!({"type":"response_item","payload":{"type":"function_call","name":"apply_patch","arguments":"{}"}}),
    ];
    fs::write(
        &rollout,
        format!(
            "{}\n",
            lines
                .iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join("\n")
        ),
    )
    .expect("rollout writes");
    let mut scanner = LocalScanner::new(AccountLabels::default());
    scanner.session_roots = SessionRoots::new([root.path().to_path_buf()]);
    scanner.rollouts.read(&rollout);
    let mut rows: Vec<AgentRow> = serde_json::from_value(serde_json::json!([{
        "agent":"claude","name":"root","pid":10,"started":10,"status":"busy","desktop":null,"directory":"/work/fixture","branch":null,"launched_by":null,
        "children":[{"kind":"thread","depth":0,"pid":null,"thread_id":THREAD,"name":THREAD,"started":20,"timer":null}]
    }])).expect("scan rows");
    let processes = [
        supervisor(directory.path()),
        process(101, 100, &["cargo", "nextest", "run"]),
    ];
    scanner.attach_activity_and_names(&processes, &HashMap::new(), &mut rows);
    assert_eq!(rows[0].children[0].activity.text(), "nextest");
    write_run(directory.path(), "finished");
    scanner
        .run_records
        .record_changed(directory.path(), "gate.json".into());
    scanner.attach_activity_and_names(&processes, &HashMap::new(), &mut rows);
    assert_eq!(rows[0].children[0].activity.text(), "writing");
    assert_eq!(rows[0].children[0].name, "worker · Write fixtures");
}

#[test]
fn activity_scan_claude_transcript_updates_and_its_descendant_gate_takes_precedence() {
    let directory = tempfile::tempdir().expect("transcript fixture");
    let transcript = directory.path().join("claude.jsonl");
    let write_tool = |tool: &str| {
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&transcript)
            .expect("transcript opens");
        writeln!(file,"{}",serde_json::json!({
        "type":"assistant","message":{"content":[{"type":"tool_use","name":tool,"input":{"file_path":"/fixture/input.rs"}}],"stop_reason":"tool_use"}
    })).expect("transcript writes");
    };
    write_tool("Read");
    let mut rows: Vec<AgentRow> = serde_json::from_value(serde_json::json!([{
        "agent":"claude","name":"root","pid":10,"started":10,"status":"busy","desktop":null,"directory":"/work/fixture","branch":null,"launched_by":null,"children":[]
    }])).expect("Claude row");
    let transcripts = HashMap::from([(10, transcript.clone())]);
    let mut scanner = LocalScanner::new(AccountLabels::default());
    scanner.attach_activity_and_names(&[], &transcripts, &mut rows);
    assert_eq!(rows[0].activity.text(), "reading");
    write_tool("Edit");
    scanner.attach_activity_and_names(&[], &transcripts, &mut rows);
    assert_eq!(
        rows[0].activity.text(),
        "writing",
        "a subsequent scan reads the changed transcript"
    );
    let processes = [
        claude(),
        process(
            100,
            10,
            &["bash", "/fixture/verify.sh", "lint", "cargo-handler"],
        ),
        process(101, 100, &["cargo", "clippy"]),
    ];
    scanner.attach_activity_and_names(&processes, &transcripts, &mut rows);
    assert_eq!(rows[0].activity.text(), "clippy");
    scanner.attach_activity_and_names(&processes[..1], &transcripts, &mut rows);
    assert_eq!(
        rows[0].activity.text(),
        "writing",
        "a completed gate exposes the last tool"
    );
}

#[test]
fn a_run_does_not_move_an_open_thread_away_from_its_app_server() {
    let mut fixture = DetachedWorkerFixture::new();
    fixture.write_run("running");
    let mut rows = DetachedWorkerFixture::rows();
    let mut other = DetachedWorkerFixture::rows().remove(0);
    other.pid = 20;
    other.session_id = SessionIdentity::Claude("other-director".into());
    rows.push(other);
    let processes = [
        claude(),
        process(20, 1, &["claude"]),
        process(30, 20, &["codex", "app-server"]),
        fixture.supervisor(),
    ];
    let runs = fixture
        .scanner
        .read_worker_runs(&processes, &rows, std::iter::empty());
    let sources = TreeSources {
        run_threads: runs.threads,
        threads: HashMap::from([(
            30,
            vec![ThreadEntry {
                activity:     AgentActivity::Idle,
                thread_id:    CodexThreadId::Recorded(THREAD.into()),
                name:         "open thread".into(),
                started:      20,
                service_tier: ServiceTier::Unrecorded,
                account:      crate::census::Account::Unrecorded,
            }],
        )]),
        ..TreeSources::default()
    };
    let added = tree::attach_children(&mut rows, &processes, &sources);
    assert!(added.is_empty(), "the existing thread keeps its place");
    fixture.scanner.attach_activity_with_runs(
        &processes,
        &HashMap::new(),
        &mut rows,
        &runs.activities,
        &added,
    );
    assert!(!rows[0].children.iter().any(|child| child.is_thread(THREAD)));
    let worker = rows[1]
        .children
        .iter()
        .find(|child| child.is_thread(THREAD))
        .expect("server thread");
    assert_eq!(worker.started, 20);
    assert_eq!(worker.activity.text(), "waiting");
}

#[test]
fn the_latest_worker_run_does_not_depend_on_child_order() {
    const OTHER: &str = "01a1437d-1234-5678-9abc-0123456789ac";
    let mut fixture = DetachedWorkerFixture::new();
    let session = fixture.root.path().join("other");
    fs::create_dir_all(session.join("detached_runs")).expect("other run directory");
    write_run(&session, "running");
    for (id, prompt) in [
        (THREAD, "Work without a session path".to_owned()),
        (OTHER, format!("Summary: {}/summary.txt", session.display())),
    ] {
        let rollout = fixture.root.path().join(format!("rollout-test-{id}.jsonl"));
        fs::write(&rollout, format!("{}\n", user_message(&prompt))).expect("rollout prompt");
        fixture.scanner.rollouts.read(&rollout);
    }
    let mut rows = DetachedWorkerFixture::rows();
    for id in [THREAD, OTHER] {
        rows[0].children.push(serde_json::from_value(serde_json::json!({"kind":"thread", "thread_id":id, "name":"worker", "started":20, "pid":null, "depth":0, "timer":null})).expect("thread row"));
    }
    let processes = [
        supervisor(&session),
        process(101, 100, &["cargo", "nextest", "run"]),
    ];
    fixture.attach_activity(&processes, &mut rows);
    assert_eq!(rows[0].children[0].activity.text(), "nextest");
    rows[0].children.reverse();
    fixture.attach_activity(&processes, &mut rows);
    assert_eq!(rows[0].children[1].activity.text(), "nextest");
}

#[test]
fn an_open_thread_without_a_prompt_dispatch_does_not_keep_a_launcher_label() {
    let mut fixture = DetachedWorkerFixture::new();
    let rollout = fixture
        .root
        .path()
        .join(format!("rollout-test-{THREAD}.jsonl"));
    fs::write(
        &rollout,
        format!("{}\n", user_message("Work without a dispatch")),
    )
    .expect("thread prompt");
    fixture.scanner.rollouts.read(&rollout);
    let dispatch = ResolvedWorkerDispatch::for_run(
        "fixture-impl",
        WorkerIdentity::CodexThread(THREAD.to_owned()),
        &fixture.scanner.worker_names.staff_snapshot(),
        &ThreadWorktree::Unrecorded,
    )
    .expect("worker dispatch");
    let mut rows = DetachedWorkerFixture::rows();
    let mut child: ChildRow = serde_json::from_value(serde_json::json!({"kind":"thread", "thread_id":THREAD, "name":"worker", "started":20, "pid":null, "depth":0, "timer":null})).expect("thread row");
    child.dispatch = WorkerDispatch::Worker(dispatch);
    rows[0].children.push(child);
    fixture.attach_activity(&[], &mut rows);
    assert!(rows[0].children[0].dispatch.is_plain());
}

#[test]
fn a_run_only_worker_uses_the_same_short_seat_as_its_rollout() {
    let mut fixture = DetachedWorkerFixture::new();
    fixture.write_run("running");
    let path = fixture.session.join("detached_runs/gate.json");
    let mut record: Value =
        serde_json::from_slice(&fs::read(&path).expect("run record")).expect("run JSON");
    record["cwd"] = serde_json::json!("/work/fixture");
    fs::write(&path, record.to_string()).expect("run worktree");
    let processes = [fixture.supervisor()];
    let from_run = fixture.scan(&processes);
    assert_eq!(from_run[0].children[0].name, "impl");
    let rollout = fixture.root.path().join(format!("rollout-{THREAD}.jsonl"));
    fs::write(&rollout, format!("{}\n{}\n", serde_json::json!({"type":"session_meta","payload":{"id":THREAD,"cwd":"/work/fixture"}}), user_message("Write fixtures"))).expect("worker rollout");
    fixture.scanner.rollouts.read(&rollout);
    let from_rollout = fixture.scan(&processes);
    assert_eq!(from_rollout[0].children[0].name, "impl");
    assert_eq!(
        from_rollout[0].children[0].dispatch,
        from_run[0].children[0].dispatch
    );
}
