//! Regression fixtures for `ActivityRead` and `TranscriptRead` conversation event
//! boundaries and for detached-run selection by `run_activity` and `verify_activity`.

use std::error::Error;
use std::io::Write;
#[cfg(target_os = "linux")]
use std::sync::atomic::AtomicUsize;
#[cfg(target_os = "linux")]
use std::sync::atomic::Ordering;
#[cfg(target_os = "linux")]
use std::time::Duration;

#[cfg(target_os = "linux")]
use notify::EventKind;
#[cfg(target_os = "linux")]
use notify::RecursiveMode;
#[cfg(target_os = "linux")]
use notify::event::ModifyKind;

use super::*;
use crate::census::AgentRow;
use crate::census::MachineState;
use crate::census::account::ConfigHome;
use crate::render;

type TestResult = Result<(), Box<dyn Error>>;

fn captured_compaction_marker(family: &str) -> Result<Value, Box<dyn Error>> {
    let captured: Value = serde_json::from_str(include_str!("fixtures/compaction_signals.json"))?;
    Ok(captured[family].clone())
}

fn write_compaction_database(
    account: &Path,
    id: &str,
    home: &Path,
    transcript: &Path,
) -> TestResult {
    let database = rusqlite::Connection::open(account.join("state_5.sqlite"))?;
    database.pragma_update(None, "synchronous", "OFF")?;
    database.execute(
        "CREATE TABLE threads (id TEXT PRIMARY KEY, cwd TEXT NOT NULL,
        title TEXT NOT NULL, originator TEXT, created_at_ms INTEGER, name TEXT,
        first_user_message TEXT NOT NULL DEFAULT '', rollout_path TEXT NOT NULL)",
        [],
    )?;
    database.execute(
        "INSERT INTO threads VALUES (?1, ?2, 'fixture', 'codex-tui',
        1794142990000, 'fixture-codex', 'Continue.', ?3)",
        rusqlite::params![id, home.to_string_lossy(), transcript.to_string_lossy()],
    )?;
    let logs = rusqlite::Connection::open(account.join("logs_2.sqlite"))?;
    logs.pragma_update(None, "synchronous", "OFF")?;
    logs.execute(
        "CREATE TABLE logs (id INTEGER PRIMARY KEY, ts INTEGER,
        process_uuid TEXT, thread_id TEXT, feedback_log_body TEXT)",
        [],
    )?;
    logs.execute("CREATE INDEX idx_logs_ts ON logs(ts)", [])?;
    logs.execute(
        "INSERT INTO logs VALUES (1, 1793000001, 'pid:200:fixture', ?1, ?2)",
        rusqlite::params![
            id,
            format!(
                "app_server.client_name=\"codex-tui\" app_server.connection_id=1 \
            rpc.method=\"thread/resume\" rpc.request_id=1 rpc.transport=\"in-process\" \
            shell_snapshot{{thread_id={id}}}: Shell snapshot successfully created:"
            )
        ],
    )?;
    Ok(())
}

fn write_compaction_transcript(
    family: &str,
    id: &str,
    account: &Path,
    home: &Path,
) -> Result<PathBuf, Box<dyn Error>> {
    let transcript = if family == "claude" {
        account.join("projects/fixture").join(format!("{id}.jsonl"))
    } else {
        account
            .join("sessions/2026/11/08")
            .join(format!("rollout-2026-11-08T13-03-10-{id}.jsonl"))
    };
    fs::create_dir_all(transcript.parent().ok_or("transcript directory")?)?;
    fs::write(
        &transcript,
        if family == "codex" {
            format!(
                "{}\n{}\n",
                serde_json::json!({"type":"session_meta","payload":{
            "id":id,"cwd":home,"timestamp":"2026-11-08T13:03:10Z"}}),
                serde_json::json!({"type":"event_msg","timestamp":"2026-11-08T20:26:29Z",
                "payload":{"type":"task_started"}})
            )
        } else {
            format!(
                "{}\n",
                serde_json::json!({"type":"user","timestamp":"2026-11-08T21:15:52Z",
            "message":{"content":"Continue."}})
            )
        },
    )?;
    Ok(transcript)
}

fn assert_compacting_frame(rows: Vec<AgentRow>) {
    let buffer = render::astra_frame_for_test(rows, crate::census::HierarchyEdges::default());
    let text: String = buffer
        .content
        .iter()
        .map(ratatui::buffer::Cell::symbol)
        .collect();
    assert!(text.split_whitespace().any(|word| word == "compacting"));
}

fn check_deleted_marker(
    scanner: &mut crate::census::scan::LocalScanner,
    processes: &[ProcessEntry],
    sessions: &[crate::census::classify::SessionRecord],
    home: &Path,
    marker: &Path,
    payload: &Value,
) -> TestResult {
    fs::remove_file(marker)?;
    let rows = scanner.scan_details_for_test(processes, sessions, home);
    assert_eq!(
        rows.iter()
            .find(|row| row.pid == 200)
            .ok_or("session after marker deletion")?
            .activity
            .text(),
        "thinking",
        "{} marker deletion clears compaction",
        payload["family"]
    );
    fs::write(marker, serde_json::to_vec(payload)?)?;
    let rows = scanner.scan_details_for_test(processes, sessions, home);
    assert_eq!(
        rows.iter()
            .find(|row| row.pid == 200)
            .ok_or("session after marker recreation")?
            .activity
            .text(),
        "compacting"
    );
    Ok(())
}

fn check_captured_compaction(family: &str) -> TestResult {
    use crate::census::account::AccountLabels;
    use crate::census::classify::SessionRecord;
    use crate::census::scan::LocalScanner;

    let directory = tempfile::tempdir()?;
    let captured = captured_compaction_marker(family)?;
    let id = captured["marker"]["session_id"]
        .as_str()
        .ok_or("captured session")?;
    let account = directory.path().join(format!(".{family}"));
    let transcript = write_compaction_transcript(family, id, &account, directory.path())?;
    if family == "codex" {
        write_compaction_database(&account, id, directory.path(), &transcript)?;
    }
    let processes = vec![ProcessEntry {
        pid:         200,
        parent:      None,
        name:        family.to_owned(),
        arguments:   if family == "codex" {
            vec!["codex".into(), "resume".into(), id.into()]
        } else {
            vec!["claude".into()]
        },
        started:     1_793_000_000,
        directory:   Some(directory.path().to_path_buf()),
        config_home: ConfigHome::Named(account),
        claude_pid:  None,
    }];
    let sessions: Vec<SessionRecord> = if family == "claude" {
        serde_json::from_value(serde_json::json!([{"pid":200,"sessionId":id,
            "name":"fixture-claude","status":"busy","cwd":directory.path()}]))?
    } else {
        Vec::new()
    };
    let mut scanner = LocalScanner::new(AccountLabels::default());
    let rows = scanner.scan_details_for_test(&processes, &sessions, directory.path());
    let row = rows
        .iter()
        .find(|row| row.pid == 200)
        .ok_or("ordinary session row")?;
    assert_eq!(
        row.activity.text(),
        "thinking",
        "an ordinary turn stays thinking"
    );

    let markers = directory.path().join(".local/state/compaction");
    fs::create_dir_all(&markers)?;
    let marker = markers.join(format!("{id}.json"));
    for invalid in [
        serde_json::json!({"session_id":id,"family":"other","started_at":"2026-11-08T20:26:30Z"}),
        serde_json::json!({"session_id":"another-session","family":family,"started_at":"2026-11-08T20:26:30Z"}),
        serde_json::json!({"session_id":id,"family":family,"started_at":"2026-11-08T20:26:30"}),
        serde_json::json!({"session_id":id,"family":family}),
    ] {
        fs::write(&marker, serde_json::to_vec(&invalid)?)?;
        let rows = scanner.scan_details_for_test(&processes, &sessions, directory.path());
        assert_eq!(
            rows.iter()
                .find(|row| row.pid == 200)
                .ok_or("ordinary row")?
                .activity
                .text(),
            "thinking"
        );
    }
    fs::write(&marker, serde_json::to_vec(&captured["marker"])?)?;
    let rows = scanner.scan_details_for_test(&processes, &sessions, directory.path());
    let row = rows
        .iter()
        .find(|row| row.pid == 200)
        .ok_or("compacting session row")?;
    assert_eq!(row.activity.text(), "compacting", "{family} start marker");
    assert_eq!(row.activity.color_role(), Role::Busy);
    assert_compacting_frame(rows);
    check_deleted_marker(
        &mut scanner,
        &processes,
        &sessions,
        directory.path(),
        &marker,
        &captured["marker"],
    )?;

    let mut file = fs::OpenOptions::new().append(true).open(&transcript)?;
    writeln!(file, "{}", captured["completed"])?;
    let rows = scanner.scan_details_for_test(&processes, &sessions, directory.path());
    let row = rows
        .iter()
        .find(|row| row.pid == 200)
        .ok_or("completed session row")?;
    assert_eq!(
        row.activity.text(),
        "thinking",
        "{family} completion expires the marker"
    );
    Ok(())
}

#[test]
fn compacting_claude_marker_tracks_the_captured_boundary() -> TestResult {
    check_captured_compaction("claude")
}

#[test]
fn compacting_codex_marker_tracks_the_captured_completion() -> TestResult {
    check_captured_compaction("codex")
}

#[derive(Clone, Copy)]
enum CompactionFixture {
    Absent,
    Open,
    Completed,
}

/// The captured pane either exists or has only its detached supervisor left.
#[derive(Clone, Copy)]
enum PaneProcessPresence {
    Visible,
    Absent,
}

#[derive(serde::Deserialize)]
struct PaneRunProcessRecord {
    pid:       u32,
    parent:    u32,
    name:      String,
    arguments: Vec<String>,
    started:   u64,
    directory: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug)]
enum CapturedImplRun {
    Live,
    Finished,
}

#[derive(Clone, Copy)]
enum CapturedPane {
    Codex,
    Astra(&'static str),
}

fn write_captured_tile_runs(
    captured: &Value,
    directory: &Path,
    implementation: CapturedImplRun,
) -> Result<PathBuf, Box<dyn Error>> {
    let runs = directory.join("detached_runs");
    fs::create_dir(&runs)?;
    for mut run in serde_json::from_value::<Vec<Value>>(captured["runs"].clone())? {
        run["status"] = serde_json::json!(if run["worker"] == "impl"
            && matches!(implementation, CapturedImplRun::Finished)
        {
            "finished"
        } else {
            "running"
        });
        run["session_dir"] = serde_json::json!(directory);
        run["director_dir"] = serde_json::json!(directory);
        let id = run["run_id"].as_str().ok_or("captured run id")?;
        fs::write(runs.join(format!("{id}.json")), serde_json::to_vec(&run)?)?;
    }
    Ok(runs)
}

fn pane_run_processes(
    captured: &Value,
    pane: PaneProcessPresence,
    directory: &Path,
    runs: &Path,
) -> Result<Vec<ProcessEntry>, Box<dyn Error>> {
    let pane_pid: u32 = serde_json::from_value(captured["pane_pid"].clone())?;
    let mut processes = Vec::new();
    for mut process in
        serde_json::from_value::<Vec<PaneRunProcessRecord>>(captured["processes"].clone())?
    {
        if process.pid == pane_pid {
            if matches!(pane, PaneProcessPresence::Absent) {
                continue;
            }
            process.arguments = vec![
                "codex".into(),
                "resume".into(),
                captured["pane_session"]
                    .as_str()
                    .ok_or("captured pane session")?
                    .into(),
            ];
        }
        if let Some(argument) = process.arguments.last_mut()
            && let Some(name) = Path::new(argument).file_name()
            && name.to_string_lossy().ends_with(".json")
        {
            *argument = runs.join(name).to_string_lossy().into_owned();
        }
        processes.push(ProcessEntry {
            pid:         process.pid,
            parent:      Some(process.parent),
            name:        process.name,
            arguments:   process.arguments,
            started:     process.started,
            directory:   process.directory,
            claude_pid:  None,
            config_home: ConfigHome::Named(directory.join(".codex")),
        });
    }
    Ok(processes)
}

fn captured_tile_run_rows(
    pane: PaneProcessPresence,
    implementation: CapturedImplRun,
    kind: CapturedPane,
) -> Result<Vec<AgentRow>, Box<dyn Error>> {
    captured_tile_run_rows_with_compaction(pane, implementation, kind, CompactionFixture::Absent)
}

fn write_captured_compaction(
    captured: &Value,
    directory: &Path,
    compaction: CompactionFixture,
) -> TestResult {
    if matches!(compaction, CompactionFixture::Absent) {
        return Ok(());
    }
    let signal = captured_compaction_marker("codex")?;
    let mut marker = signal["marker"].clone();
    marker["session_id"] = captured["pane_session"].clone();
    let markers = directory.join(".local/state/compaction");
    fs::create_dir_all(&markers)?;
    let id = marker["session_id"].as_str().ok_or("pane session")?;
    fs::write(
        markers.join(format!("{id}.json")),
        serde_json::to_vec(&marker)?,
    )?;
    if matches!(compaction, CompactionFixture::Completed) {
        let account = directory.join(".codex");
        let transcript = write_compaction_transcript("codex", id, &account, directory)?;
        write_compaction_database(&account, id, directory, &transcript)?;
        writeln!(
            fs::OpenOptions::new().append(true).open(&transcript)?,
            "{}",
            signal["completed"]
        )?;
    }
    Ok(())
}

fn captured_tile_run_rows_with_compaction(
    pane: PaneProcessPresence,
    implementation: CapturedImplRun,
    kind: CapturedPane,
    compaction: CompactionFixture,
) -> Result<Vec<AgentRow>, Box<dyn Error>> {
    use crate::census::HierarchySource;
    use crate::census::StaffFileSetting;
    use crate::census::account::AccountLabels;
    use crate::census::classify::SessionRecord;
    use crate::census::hierarchy::StaffFileReader;
    use crate::census::scan::LocalScanner;

    let mut captured: Value =
        serde_json::from_str(include_str!("fixtures/pane_detached_tile.json"))?;
    let root = Path::new(crate::constants::SESSION_ROOTS[0]);
    fs::create_dir_all(root)?;
    let directory = tempfile::Builder::new()
        .prefix("pane-run-")
        .tempdir_in(root)?;
    let runs = write_captured_tile_runs(&captured, directory.path(), implementation)?;
    write_captured_compaction(&captured, directory.path(), compaction)?;
    let mut passes = serde_json::Map::new();
    let started = captured["runs"][0]["started_at"].clone();
    for member in captured["staff"]["rows"]
        .as_array_mut()
        .ok_or("captured staff rows")?
    {
        if member["kind"] == "unit" {
            member["run"] = serde_json::json!(directory.path());
        } else if member["kind"] == "worker" {
            let id = member["id"].as_str().ok_or("captured staff worker")?;
            passes.insert(
                id.to_owned(),
                serde_json::json!({"worker_id":id,"status":"active","started_at":started}),
            );
        }
    }
    captured["unit_owner"]["pass"] = Value::Object(passes);
    fs::write(
        directory.path().join("progress_history_state.json"),
        serde_json::to_vec(&captured["unit_owner"])?,
    )?;
    if matches!(kind, CapturedPane::Astra(_)) {
        captured["staff"]["rows"][0]["role"] = serde_json::json!("astra-designer");
        captured["staff"]["rows"][0]["home"] = serde_json::json!(directory.path().join(".codex"));
    }
    let staff_path = directory.path().join("staff.json");
    fs::write(&staff_path, serde_json::to_vec(&captured["staff"])?)?;
    let processes = pane_run_processes(&captured, pane, directory.path(), &runs)?;
    let sessions: Vec<SessionRecord> = serde_json::from_value(serde_json::json!([{
        "pid":captured["director_pid"], "sessionId":captured["director_session"], "name":"tile"
    }]))?;
    let mut scanner = LocalScanner::new(AccountLabels::default()).with_staff_file(
        StaffFileSetting::new(HierarchySource::File(
            staff_path.to_string_lossy().into_owned(),
        )),
        StaffFileReader::default(),
    );
    scanner.use_fixture_run_events();
    if let CapturedPane::Astra(status) = kind {
        let snapshot = serde_json::json!({"snapshot":{"tabs":[],"panes":[{
            "pane_id":captured["pane_pid"].to_string(), "tab_id":"workers", "terminal_id":"captured-pane", "revision":1,"agent_status":status
        }]}});
        scanner.set_herdr_for_test(crate::census::herdr::HerdrNames::from_snapshot_for_test(
            &processes, &snapshot,
        ));
    }
    Ok(scanner.scan_details_for_test(&processes, &sessions, directory.path()))
}

#[test]
fn captured_tile_impl_pane_joins_its_detached_package_run() -> TestResult {
    let rows = captured_tile_run_rows(
        PaneProcessPresence::Visible,
        CapturedImplRun::Live,
        CapturedPane::Codex,
    )?;
    let worker = rows
        .iter()
        .find(|row| {
            row.session_id
                .ids()
                .any(|id| id == "01a1b9c6-4b1b-79c0-ad78-920ac1efe28a")
        })
        .ok_or("the captured impl pane has its own row")?;
    assert_eq!(
        worker.activity.text(),
        "waiting",
        "the captured package wrapper belongs to impl"
    );
    Ok(())
}

#[test]
fn captured_tile_test_worker_keeps_a_row_with_only_its_live_run() -> TestResult {
    for implementation in [CapturedImplRun::Live, CapturedImplRun::Finished] {
        let rows = captured_tile_run_rows(
            PaneProcessPresence::Absent,
            implementation,
            CapturedPane::Codex,
        )?;
        let worker = rows.iter().flat_map(|row| &row.children)
        .find(|child| child.name == "test" || matches!(&child.dispatch,
            crate::census::worker_dispatch::WorkerDispatch::Worker(worker) if worker.seat() == "test"))
        .ok_or("the captured test worker with a live run must retain its row")?;
        assert_eq!(worker.activity.text(), "waiting");
    }
    Ok(())
}

#[test]
fn captured_tile_astra_pane_keeps_its_live_run_status() -> TestResult {
    use crate::census::HierarchyEdge;
    use crate::census::HierarchyEdges;
    use crate::census::HierarchyRole;
    for (status, without_run) in [
        ("idle", "astra designer"),
        ("unknown", "astra designer"),
        ("working", "astra designer · thinking"),
    ] {
        for implementation in [CapturedImplRun::Live, CapturedImplRun::Finished] {
            let rows = captured_tile_run_rows(
                PaneProcessPresence::Visible,
                implementation,
                CapturedPane::Astra(status),
            )?;
            let edges = HierarchyEdges(vec![HierarchyEdge {
                manager: "da82452f-3640-4dec-b6c5-0c783d77e79f".into(),
                member:  "01a1b9c6-4b1b-79c0-ad78-920ac1efe28a".into(),
                role:    HierarchyRole::AstraDesigner,
            }]);
            let buffer = render::astra_frame_for_test(rows.clone(), edges.clone());
            let line = buffer
                .content
                .chunks(220)
                .map(|cells| {
                    cells
                        .iter()
                        .map(ratatui::buffer::Cell::symbol)
                        .collect::<String>()
                })
                .find(|line| line.contains("impl") && line.contains("astra designer"))
                .ok_or("the Astra pane worker must draw its status")?;
            let expected = match implementation {
                CapturedImplRun::Live => "astra designer · waiting",
                CapturedImplRun::Finished => without_run,
            };
            assert!(
                line.contains("worker"),
                "{status}, {implementation:?}: {line}"
            );
            let name = &rows
                .iter()
                .find(|row| {
                    row.session_id
                        .ids()
                        .any(|id| id == "01a1b9c6-4b1b-79c0-ad78-920ac1efe28a")
                })
                .ok_or("Astra worker")?
                .name;
            let state = MachineState::Answered(rows.clone(), edges.into());
            assert_eq!(
                crate::agent_cell::detail_property_for_test(
                    &[crate::census::Machine {
                        name:  "fixture",
                        state: &state,
                    }],
                    name,
                    "activity",
                    crate::census::unix_now()
                ),
                expected
            );
        }
    }
    Ok(())
}

#[test]
fn compacting_captured_run_and_astra_rows_keep_the_run_activity() -> TestResult {
    for kind in [
        CapturedPane::Codex,
        CapturedPane::Astra("idle"),
        CapturedPane::Astra("working"),
    ] {
        for implementation in [CapturedImplRun::Live, CapturedImplRun::Finished] {
            let rows = captured_tile_run_rows_with_compaction(
                PaneProcessPresence::Visible,
                implementation,
                kind,
                CompactionFixture::Open,
            )?;
            let worker = rows
                .iter()
                .find(|row| {
                    row.session_id
                        .ids()
                        .any(|id| id == "01a1b9c6-4b1b-79c0-ad78-920ac1efe28a")
                })
                .ok_or("compacting worker")?;
            assert_eq!(worker.activity.text(), "compacting");
            if matches!(kind, CapturedPane::Astra(_))
                && matches!(implementation, CapturedImplRun::Live)
            {
                let wire = serde_json::to_value(worker)?;
                assert_eq!(wire["astra_member"]["run"]["live"], "waiting");
            }
            if matches!(kind, CapturedPane::Astra(_)) {
                let edges = crate::census::HierarchyEdges(vec![crate::census::HierarchyEdge {
                    manager: "da82452f-3640-4dec-b6c5-0c783d77e79f".into(),
                    member:  "01a1b9c6-4b1b-79c0-ad78-920ac1efe28a".into(),
                    role:    crate::census::HierarchyRole::AstraDesigner,
                }]);
                let buffer = render::astra_frame_for_test(rows, edges);
                let text: String = buffer
                    .content
                    .iter()
                    .map(ratatui::buffer::Cell::symbol)
                    .collect();
                assert!(text.split_whitespace().any(|word| word == "compacting"));
            }
        }
    }
    for (compaction, expected) in [
        (CompactionFixture::Open, "compacting"),
        (CompactionFixture::Completed, "waiting"),
    ] {
        let rows = captured_tile_run_rows_with_compaction(
            PaneProcessPresence::Absent,
            CapturedImplRun::Live,
            CapturedPane::Codex,
            compaction,
        )?;
        let worker = rows
            .iter()
            .flat_map(|row| &row.children)
            .find(|child| {
                matches!(&child.thread_id, crate::census::CodexThreadId::Recorded(id)
                if id == "01a1b9c6-4b1b-79c0-ad78-920ac1efe28a")
            })
            .ok_or("run-only compacting worker")?;
        assert_eq!(
            worker.activity.text(),
            expected,
            "run-only worker after native completion"
        );
    }
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn idle_run_records_os_watch_does_not_deliver_read_access_events() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("record.json");
    fs::write(&path, "initial")?;
    let reads = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&reads);
    let (sent, received) = std::sync::mpsc::channel();
    let mut watcher = RecommendedWatcher::new(
        move |answer: notify::Result<Event>| {
            if let Ok(event) = answer {
                match event.kind {
                    EventKind::Access(_) => {
                        observed.fetch_add(1, Ordering::Relaxed);
                    },
                    EventKind::Modify(ModifyKind::Data(_)) => {
                        let _ = sent.send(());
                    },
                    _ => {},
                }
            }
        },
        RunDirectoryWatch::config(),
    )?;
    watcher.watch(&path, RecursiveMode::NonRecursive)?;
    for _ in 0..32 {
        fs::read(&path)?;
    }
    // The write event follows all read events on this watch; it is the completion signal.
    fs::write(&path, "changed")?;
    received.recv_timeout(Duration::from_millis(750))?;
    assert_eq!(
        reads.load(Ordering::Relaxed),
        0,
        "unchanged record reads must not wake the watch"
    );
    Ok(())
}

#[test]
fn idle_verification_trees_visit_only_descendants_once_per_scan() {
    let processes = [
        process(200, 1, &["unrelated"]),
        process(101, 100, &["cargo", "clippy"]),
        process(100, 1, &["bash", "/fixture/verify.sh"]),
        process(201, 200, &["cargo", "test"]),
    ];
    let mut activity = VerificationProcesses::new(&processes);
    assert_eq!(activity.progress(100).text(), "clippy");
    assert_eq!(activity.progress(100).text(), "clippy");
    assert_eq!(
        activity.process_visits, 2,
        "only the two verification descendants are visited once"
    );
}

fn process(pid: u32, parent: u32, arguments: &[&str]) -> ProcessEntry {
    ProcessEntry {
        pid,
        parent: Some(parent),
        name: arguments[0].into(),
        arguments: arguments.iter().map(|word| (*word).to_owned()).collect(),
        started: 100,
        directory: None,
        claude_pid: None,
        config_home: ConfigHome::Unread,
    }
}

fn record(pid: u32, started_at: f64, command: &[&str]) -> Result<RunRecord, serde_json::Error> {
    serde_json::from_value::<RunRecord>(serde_json::json!({
        "pid":pid,"started_at":started_at,"thread_id":"worker","status":"running","command":command
    }))
    .map(|record| {
        record.at_file(&PathBuf::from(format!(
            "/fixture/session/detached_runs/run-{pid}.json"
        )))
    })
}

#[test]
fn unrelated_detached_commands_never_override_a_tool_or_an_older_gate() -> TestResult {
    let processes = [
        process(
            100,
            1,
            &[
                "python3",
                "detached_runs.py",
                "supervise",
                "/fixture/session/detached_runs/run-100.json",
            ],
        ),
        process(101, 100, &["cargo", "clippy"]),
        process(200, 1, &["python3", "send.py"]),
    ];
    let unrelated = record(200, 200.0, &["python3", "send.py", "--text", "test"])?;
    assert_eq!(
        run_activity(std::slice::from_ref(&unrelated), &processes, "worker"),
        AgentActivity::Unreported
    );
    let mentioned = record(200, 200.0, &["echo", "/fixture/verify.sh"])?;
    assert_eq!(
        run_activity(&[mentioned], &processes, "worker"),
        AgentActivity::Unreported
    );
    assert_eq!(
        run_activity(
            &[
                record(100, 100.0, &["bash", "/fixture/verify.sh", "lint"])?,
                unrelated
            ],
            &processes,
            "worker"
        )
        .text(),
        "clippy"
    );
    Ok(())
}

#[test]
fn shell_reads_keep_their_word_with_pipes_expansions_and_redirections() {
    for command in [
        "rg needle src | head -20",
        "cat \"$file\"",
        "sed -n 1,5p f > out",
        "find src 2>/dev/null",
        "grep \"$(cat pattern)\" input",
    ] {
        assert_eq!(
            tool_activity("exec_command", &serde_json::json!({"cmd":command})).text(),
            "reading",
            "{command}"
        );
    }
    assert_eq!(
        tool_activity(
            "Bash",
            &serde_json::json!({"command":"echo 'git commit' | cat"})
        )
        .text(),
        "running"
    );
}

#[test]
fn claude_text_block_user_turn_reopens_after_completion_but_tool_results_do_not() {
    let mut read = ActivityRead::default();
    read.take_claude(
        &serde_json::json!({"type":"assistant","message":{"content":[],"stop_reason":"end_turn"}}),
    );
    read.take_claude(&serde_json::json!({"type":"user","message":{"content":[{"type":"tool_result","content":"done"}]}}));
    assert_eq!(read.activity(), AgentActivity::Idle);
    read.take_claude(&serde_json::json!({"type":"user","message":{"content":[{"type":"text","text":"Next task"}]}}));
    assert_eq!(read.activity().text(), "thinking");
}

#[test]
fn transcript_cursor_reuses_unchanged_input_and_consumes_only_complete_appends() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("transcript.jsonl");
    let message = |tool| {
        serde_json::json!({"type":"assistant","message":{"content":[{"type":"tool_use","name":tool,"input":{}}]}}).to_string()
    };
    std::fs::write(&path, format!("{}\n", message("Read")))?;
    let mut read = TranscriptRead::new(AgentActivity::Unreported);
    assert_eq!(read.advance(&path, AgentActivity::Unreported), 1);
    assert_eq!(read.activity().text(), "reading");
    assert_eq!(read.advance(&path, AgentActivity::Unreported), 0);
    let mut file = std::fs::OpenOptions::new().append(true).open(&path)?;
    write!(file, "{}", message("Edit"))?;
    assert_eq!(read.advance(&path, AgentActivity::Unreported), 0);
    assert_eq!(read.activity().text(), "reading");
    writeln!(file)?;
    assert_eq!(read.advance(&path, AgentActivity::Unreported), 1);
    assert_eq!(read.activity().text(), "writing");
    std::fs::write(
        &path,
        "{\"type\":\"user\",\"message\":{\"content\":\"restart\"}}\n",
    )?;
    assert_eq!(read.advance(&path, AgentActivity::Unreported), 1);
    assert_eq!(read.activity().text(), "thinking");
    Ok(())
}

#[test]
fn incomplete_run_identity_and_start_time_never_enter_live_gate_selection() -> TestResult {
    let processes = [
        process(100, 1, &["bash", "verify.sh"]),
        process(101, 100, &["cargo", "check"]),
    ];
    let complete = serde_json::json!({"pid":100,"started_at":100.0,"thread_id":"worker","status":"running","command":["bash","verify.sh","check"]});
    for missing in ["thread_id", "started_at"] {
        let mut payload = complete.clone();
        if let Some(fields) = payload.as_object_mut() {
            fields.remove(missing);
        }
        let record = serde_json::from_value(payload)?;
        assert_eq!(
            run_activity(&[record], &processes, "worker"),
            AgentActivity::Unreported
        );
    }
    Ok(())
}

#[test]
fn remote_step_unknown_uses_the_verify_request_without_changing_local_waits() -> TestResult {
    let records = [record(
        100,
        100.0,
        &["bash", "/fixture/verify.sh", "test", "cargo-handler"],
    )?];
    let processes = [
        process(
            100,
            1,
            &[
                "python3",
                "detached_runs.py",
                "supervise",
                "/fixture/session/detached_runs/run-100.json",
            ],
        ),
        process(101, 100, &["ssh", "mac", "remote-dispatch"]),
    ];
    assert_eq!(
        run_activity(&records, &processes, "worker").text(),
        "nextest (mac)"
    );
    assert_eq!(
        run_activity(&records, &processes[..1], "worker").text(),
        "waiting"
    );
    Ok(())
}

#[test]
fn verification_words_have_the_same_meaning_in_cargo_and_ssh_commands() {
    for word in [
        "fmt", "clippy", "nextest", "build", "check", "doc", "mend", "taplo",
    ] {
        let local = [
            process(100, 1, &["bash", "verify.sh"]),
            process(101, 100, &["cargo", word]),
        ];
        let remote_command = format!("cargo {word}");
        let remote = [
            process(100, 1, &["bash", "verify.sh"]),
            process(101, 100, &["ssh", "mac", &remote_command]),
        ];
        assert_eq!(verify_activity(&local, 100).text(), word);
        assert_eq!(
            verify_activity(&remote, 100).text(),
            format!("{word} (mac)")
        );
    }
}

#[test]
fn remote_verification_finds_a_step_after_shell_setup_without_matching_substrings() {
    let root = process(100, 1, &["bash", "verify.sh"]);
    let processes = [
        root.clone(),
        process(
            101,
            100,
            &["ssh", "mac", "cd /work && cargo nextest run -p fixture"],
        ),
    ];
    assert_eq!(verify_activity(&processes, 100).text(), "nextest (mac)");
    let unrelated = [
        root,
        process(101, 100, &["ssh", "mac", "cat /work/nextest-results.log"]),
    ];
    assert_eq!(verify_activity(&unrelated, 100).text(), "waiting");
}

fn run_file(directory: &Path, name: &str, pid: u32) -> TestResult {
    fs::create_dir_all(directory.join("detached_runs"))?;
    fs::write(
        directory.join("detached_runs").join(name),
        serde_json::to_vec(&serde_json::json!({
            "pid":pid,"started_at":1,"thread_id":"worker","status":"running",
            "command":["bash","verify.sh","test"]
        }))?,
    )?;
    Ok(())
}

#[test]
fn idle_run_records_parse_unchanged_files_only_once() -> TestResult {
    let directory = tempfile::tempdir()?;
    run_file(directory.path(), "one.json", 1)?;
    let mut records = RunRecords::for_test();
    let mut parses = 0;
    for _ in 0..2 {
        let answer = records.read_with(directory.path(), |bytes| {
            parses += 1;
            serde_json::from_slice(bytes).ok()
        });
        assert_eq!(answer.len(), 1);
    }
    assert_eq!(parses, 1, "unchanged records must not be parsed again");
    Ok(())
}

#[test]
fn idle_run_records_parse_changed_and_new_files_only() -> TestResult {
    let directory = tempfile::tempdir()?;
    run_file(directory.path(), "one.json", 1)?;
    let mut records = RunRecords::for_test();
    records.read(directory.path());
    run_file(directory.path(), "one.json", 12345)?;
    run_file(directory.path(), "two.json", 2)?;
    let mut parses = 0;
    let answer = records.read_with(directory.path(), |bytes| {
        parses += 1;
        serde_json::from_slice(bytes).ok()
    });
    assert_eq!(parses, 2);
    assert_eq!(answer.len(), 2);
    let mut unchanged = 0;
    records.read_with(directory.path(), |bytes| {
        unchanged += 1;
        serde_json::from_slice(bytes).ok()
    });
    assert_eq!(
        unchanged, 0,
        "changed records are retained after their first parse"
    );
    Ok(())
}

#[test]
fn idle_run_records_remove_deleted_files_from_the_cache() -> TestResult {
    let directory = tempfile::tempdir()?;
    run_file(directory.path(), "one.json", 1)?;
    let mut records = RunRecords::for_test();
    assert_eq!(records.read(directory.path()).len(), 1);
    fs::remove_file(directory.path().join("detached_runs/one.json"))?;
    assert!(
        records.read(directory.path()).is_empty(),
        "removed records leave the cache"
    );
    Ok(())
}

#[test]
fn idle_run_records_keep_sessions_separate_and_retry_revised_partial_json() -> TestResult {
    let first = tempfile::tempdir()?;
    let second = tempfile::tempdir()?;
    run_file(first.path(), "one.json", 1)?;
    run_file(second.path(), "two.json", 2)?;
    let mut records = RunRecords::for_test();
    assert_eq!(records.read(first.path()).len(), 1);
    assert_eq!(records.read(second.path()).len(), 1);
    let partial = first.path().join("detached_runs/partial.json");
    fs::write(&partial, "{")?;
    let mut parses = 0;
    for _ in 0..2 {
        assert_eq!(
            records
                .read_with(first.path(), |bytes| {
                    parses += 1;
                    serde_json::from_slice(bytes).ok()
                })
                .len(),
            1
        );
    }
    assert_eq!(parses, 1, "one parse per partial revision");
    run_file(first.path(), "partial.json", 3)?;
    assert_eq!(records.read(first.path()).len(), 2);
    records.retain_sessions(&HashSet::from([second.path().to_owned()]));
    assert_eq!(records.sessions.len(), 1);
    Ok(())
}

#[test]
fn idle_run_records_detect_same_size_rewrites_and_replacements() -> TestResult {
    let directory = tempfile::tempdir()?;
    run_file(directory.path(), "one.json", 1)?;
    let path = directory.path().join("detached_runs/one.json");
    let modified = fs::metadata(&path)?.modified()?;
    let mut records = RunRecords::for_test();
    records.read(directory.path());
    for (pid, replace) in [(2, false), (3, true)] {
        if replace {
            run_file(directory.path(), "replacement.json", pid)?;
            fs::rename(
                directory.path().join("detached_runs/replacement.json"),
                &path,
            )?;
        } else {
            run_file(directory.path(), "one.json", pid)?;
        }
        fs::File::options()
            .write(true)
            .open(&path)?
            .set_times(fs::FileTimes::new().set_modified(modified))?;
        let answer = records.read(directory.path());
        assert!(
            matches!(&answer[..], [RunRecord::Running(run)] if run.pid == pid),
            "equal-size records with restored mtime must replace the old answer"
        );
    }
    Ok(())
}

#[test]
fn idle_run_records_retry_failed_io_but_keep_malformed_revisions() -> TestResult {
    let directory = tempfile::tempdir()?;
    run_file(directory.path(), "one.json", 1)?;
    let mut records = RunRecords::for_test();
    let parse = |bytes: &[u8]| serde_json::from_slice(bytes).ok();
    assert!(
        records
            .read_with_io(directory.path(), parse, |_| {
                Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
            })
            .is_empty()
    );
    assert_eq!(
        records.read(directory.path()).len(),
        1,
        "a transient read failure must retry even with unchanged metadata"
    );
    fs::write(directory.path().join("detached_runs/one.json"), "{")?;
    let mut parses = 0;
    for _ in 0..2 {
        records.read_with(directory.path(), |bytes| {
            parses += 1;
            serde_json::from_slice(bytes).ok()
        });
    }
    assert_eq!(
        parses, 1,
        "malformed JSON retries only after its revision changes"
    );
    Ok(())
}

#[test]
fn idle_run_records_copy_only_live_verification_candidates() -> TestResult {
    let directory = tempfile::tempdir()?;
    for pid in 1..=3 {
        run_file(directory.path(), &format!("{pid}.json"), pid)?;
    }
    let mut records = RunRecords::for_test();
    for _ in 0..2 {
        let answer = records.read_for_processes(directory.path(), &HashSet::from([2]));
        assert_eq!(
            answer.len(),
            1,
            "only live runs are copied out for worker matching"
        );
        assert!(matches!(&answer[..], [RunRecord::Running(run)] if run.pid == 2));
    }
    Ok(())
}

#[test]
fn idle_run_watch_recovery_reuses_its_unchanged_directory() -> TestResult {
    let directory = tempfile::tempdir()?;
    run_file(directory.path(), "one.json", 1)?;
    let now = Instant::now();
    let mut watch =
        RunDirectoryWatch::new(directory.path(), now, RunRecords::for_test().install_watch);
    assert!(matches!(
        watch.take_changes(directory.path(), now),
        RunChanges::Reconcile
    ));
    assert!(matches!(
        watch.take_changes(directory.path(), now + RUN_WATCH_RECOVERY),
        RunChanges::Reconcile
    ));
    assert_eq!(
        watch.installations, 1,
        "recovery keeps the healthy OS watch"
    );
    fs::rename(
        directory.path().join("detached_runs"),
        directory.path().join("old-runs"),
    )?;
    run_file(directory.path(), "two.json", 2)?;
    assert!(matches!(
        watch.take_changes(directory.path(), now + RUN_WATCH_RECOVERY * 2),
        RunChanges::Reconcile
    ));
    assert_eq!(
        watch.installations, 2,
        "a replaced directory installs its new watch"
    );
    Ok(())
}

#[test]
fn idle_run_records_do_not_list_an_unchanged_watched_directory() -> TestResult {
    let directory = tempfile::tempdir()?;
    run_file(directory.path(), "one.json", 1)?;
    let mut records = RunRecords::for_test();
    let live = HashSet::from([1]);
    for _ in 0..2 {
        let answer = records.read_for_processes(directory.path(), &live);
        assert!(matches!(&answer[..], [RunRecord::Running(run)] if run.pid == 1));
    }
    assert_eq!(
        records.directory_reads, 1,
        "unchanged watch does not list or stat records"
    );
    Ok(())
}

/// Delivery, loss and unavailable watches use the same cache invalidation path.
#[test]
fn idle_run_records_recover_events_and_directory_replacement() -> TestResult {
    let directory = tempfile::tempdir()?;
    run_file(directory.path(), "one.json", 1)?;
    let mut records = RunRecords::for_test();
    let live = HashSet::from([1, 2, 3]);
    assert_eq!(records.read_for_processes(directory.path(), &live).len(), 1);
    run_file(directory.path(), "two.json", 2)?;
    records.watches[directory.path()].invalidate();
    assert_eq!(records.read_for_processes(directory.path(), &live).len(), 2);
    assert_eq!(records.directory_reads, 2);

    // No delivered event: the thirty-second recovery still reads the new revision.
    run_file(directory.path(), "one.json", 3)?;
    records
        .watches
        .get_mut(directory.path())
        .ok_or("missing watch")?
        .recovery = Instant::now();
    let answer = records.read_for_processes(directory.path(), &live);
    assert!(
        answer
            .iter()
            .any(|record| matches!(record, RunRecord::Running(run) if run.pid == 3))
    );
    assert_eq!(records.directory_reads, 3);

    fs::remove_dir_all(directory.path().join("detached_runs"))?;
    records
        .watches
        .get_mut(directory.path())
        .ok_or("missing watch")?
        .recovery = Instant::now();
    assert!(
        records
            .read_for_processes(directory.path(), &live)
            .is_empty()
    );
    assert!(matches!(
        records.watches[directory.path()].backend,
        RunWatchBackend::Unavailable
    ));
    run_file(directory.path(), "new.json", 2)?;
    records
        .watches
        .get_mut(directory.path())
        .ok_or("missing watch")?
        .recovery = Instant::now();
    let answer = records.read_for_processes(directory.path(), &live);
    assert!(matches!(&answer[..], [RunRecord::Running(run)] if run.pid == 2));
    assert_eq!(records.directory_reads, 5);
    records.retain_sessions(&HashSet::new());
    assert!(records.sessions.is_empty());
    assert!(records.watches.is_empty());
    Ok(())
}

#[test]
fn idle_run_records_recover_an_unavailable_watch_and_discover_a_new_session() -> TestResult {
    let directory = tempfile::tempdir()?;
    let session = directory.path().join("new-session");
    let live = HashSet::from([1]);
    let mut records = RunRecords::for_test();
    assert!(records.read_for_processes(&session, &live).is_empty());
    let now = Instant::now();
    let watch = &records.watches[&session];
    assert!(matches!(watch.backend, RunWatchBackend::Unavailable));
    assert!(watch.recovery <= now + RUN_WATCH_RETRY);
    run_file(&session, "one.json", 1)?;
    records
        .watches
        .get_mut(&session)
        .ok_or("missing watch")?
        .recovery = now;
    let answer = records.read_for_processes(&session, &live);
    assert!(matches!(&answer[..], [RunRecord::Running(run)] if run.pid == 1));
    assert_eq!(records.directory_reads, 2);
    Ok(())
}

#[test]
fn idle_run_records_patch_one_changed_record_without_listing_its_mates() -> TestResult {
    let directory = tempfile::tempdir()?;
    for pid in 1..=3 {
        run_file(directory.path(), &format!("{pid}.json"), pid)?;
    }
    let mut records = RunRecords::for_test();
    let live = HashSet::from([1, 2, 3, 4]);
    assert_eq!(records.read_for_processes(directory.path(), &live).len(), 3);
    run_file(directory.path(), "2.json", 4)?;
    records.watches[directory.path()].record_changed("2.json".into());
    let rows = records.read_for_processes(directory.path(), &live);
    assert!(
        rows.iter()
            .any(|row| matches!(row, RunRecord::Running(run) if run.pid == 4))
    );
    assert_eq!(
        records.directory_reads, 1,
        "a file event does not list its unchanged mates"
    );
    fs::remove_file(directory.path().join("detached_runs/1.json"))?;
    records.watches[directory.path()].record_changed("1.json".into());
    assert_eq!(records.read_for_processes(directory.path(), &live).len(), 2);
    run_file(directory.path(), "new.json", 1)?;
    records.watches[directory.path()].record_changed("new.json".into());
    assert_eq!(records.read_for_processes(directory.path(), &live).len(), 3);
    assert_eq!(records.directory_reads, 1);
    Ok(())
}

fn assert_live_detached_run_waits(encoded: &str) -> TestResult {
    let mut payload: Value = serde_json::from_str(encoded)?;
    payload["status"] = serde_json::json!("running");
    let directory = tempfile::tempdir()?;
    let runs = directory.path().join("detached_runs");
    fs::create_dir(&runs)?;
    let path = runs.join("run.json");
    fs::write(&path, serde_json::to_vec(&payload)?)?;
    let mut records = RunRecords::for_test();
    let records = records.read(directory.path());
    let pid: u32 = serde_json::from_value(payload["pid"].clone())?;
    let processes = [process(
        pid,
        1,
        &[
            "python3",
            "detached_runs.py",
            "supervise",
            &path.to_string_lossy(),
        ],
    )];
    let mut verification = VerificationProcesses::new(&processes);
    assert!(
        matches!(verification.worker_run(&records), WorkerRun::Running { activity, .. }
            if activity == AgentActivity::Active(Activity::Waiting)),
        "a live detached run must show waiting for command {}",
        payload["command"]
    );
    assert!(matches!(
        VerificationProcesses::new(&[]).worker_run(&records),
        WorkerRun::Inactive
    ));
    payload["status"] = serde_json::json!("finished");
    let finished: RunRecord = serde_json::from_value(payload)?;
    assert!(matches!(
        verification.worker_run(&[finished.at_file(&path)]),
        WorkerRun::Inactive
    ));
    Ok(())
}

#[test]
fn unknown_detached_command_waits_without_a_step_name() -> TestResult {
    assert_live_detached_run_waits(
        r#"{
        "command":["unrecognized-job","test"],"pid":100,
        "thread_id":"worker","started_at":100,"status":"running"
    }"#,
    )
}

#[test]
fn herdr_python_detached_run_waits_until_its_supervisor_ends() -> TestResult {
    assert_live_detached_run_waits(
        r#"{
  "command": [
    "python3",
    "-c",
    "import json, shlex, subprocess, sys\ncommands=json.loads(\"[[\\\"basedpyright\\\",\\\"scripts/direct/assign.py\\\",\\\"scripts/direct/assign_gate.py\\\",\\\"scripts/direct/test_assign_gate.py\\\",\\\"scripts/direct/test_assign_launcher.py\\\",\\\"scripts/direct/test_assign_claude_door.py\\\",\\\"scripts/direct/test_one_launch_path.py\\\"],[\\\"env\\\",\\\"-u\\\",\\\"PYTHONPATH\\\",\\\"-u\\\",\\\"STAFF_OPERATION_FD\\\",\\\"-u\\\",\\\"PYTHONUNBUFFERED\\\",\\\"-u\\\",\\\"REFUSAL_WATCH_INVENTORY\\\",\\\"HERDR_BIN=/nonexistent/herdr\\\",\\\"scripts/lib/py\\\",\\\"-m\\\",\\\"unittest\\\",\\\"scripts.direct.test_assign_gate\\\",\\\"scripts.direct.test_assign_launcher\\\",\\\"scripts.direct.test_assign_claude_door\\\",\\\"scripts.direct.test_one_launch_path\\\"]]\")\nresults=[]\nfor command in commands:\n print('COMMAND: '+shlex.join(command),flush=True)\n result=subprocess.run(command,check=False)\n print('EXIT: '+str(result.returncode),flush=True)\n results.append(result.returncode)\nsys.exit(0 if all(code == 0 for code in results) else 1)\n"
  ],
  "pid": 3410210,
  "staff_id": "01a1a6ee-8b1d-7520-8d5d-d9fb8042e624",
  "started_at": 1793929230.1851988,
  "status": "finished"
}"#,
    )
}

#[test]
fn herdr_shell_detached_run_waits_until_its_supervisor_ends() -> TestResult {
    assert_live_detached_run_waits(
        r#"{
  "command": [
    "bash",
    "-c",
    "env -u PYTHONPATH basedpyright scripts/herdr/side_pane.py scripts/herdr/test_side_pane.py scripts/herdr/herdr.py scripts/herdr/test_herdr.py && env -u PYTHONPATH -u STAFF_OPERATION_FD -u PYTHONUNBUFFERED -u REFUSAL_WATCH_INVENTORY HERDR_BIN=/nonexistent/herdr scripts/lib/py -m unittest scripts.herdr.test_side_pane scripts.herdr.test_herdr"
  ],
  "pid": 3492604,
  "staff_id": "01a1a862-64ea-78a2-a637-2ff184ab8fe8",
  "started_at": 1793929292.3816147,
  "status": "finished"
}"#,
    )
}
