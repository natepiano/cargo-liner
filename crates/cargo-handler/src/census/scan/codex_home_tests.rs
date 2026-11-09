//! A database owns its account even when accounts share their rollout directory.

#![expect(
    clippy::expect_used,
    reason = "isolated account fixtures must be readable"
)]

#[cfg(target_os = "linux")]
use std::io::Write;

#[cfg(target_os = "linux")]
use super::activity_scan_tests;
use super::*;
use crate::census::account::ConfigHome;
#[cfg(target_os = "linux")]
use crate::census::worker_dispatch;

const THREAD: &str = "01a1437d-1234-5678-9abc-0123456789ab";

fn database(home: &Path, rollout: &Path, name: &str) {
    fs::create_dir_all(home).expect("account home");
    let connection =
        rusqlite::Connection::open(home.join("state_6.sqlite")).expect("account database");
    connection
        .execute_batch(
            "PRAGMA synchronous = OFF;
        CREATE TABLE threads (id TEXT PRIMARY KEY, cwd TEXT, created_at_ms INTEGER,
        name TEXT, first_user_message TEXT, rollout_path TEXT, originator TEXT DEFAULT 'codex-tui');",
        )
        .expect("threads table");
    connection
        .execute(
            "INSERT INTO threads (id, cwd, created_at_ms, name, first_user_message, rollout_path) VALUES (?1, '/fixture', 1000, ?2, '', ?3)",
            rusqlite::params![THREAD, name, rollout.to_string_lossy()],
        )
        .expect("named thread");
}

fn scan(process: &ProcessEntry, home: &Path) -> Vec<AgentRow> {
    let mut scanner = LocalScanner::new(AccountLabels::default());
    let processes = std::slice::from_ref(process);
    scanner.codex_logs.refresh(processes, Some(home));
    let mut rows = Vec::new();
    scanner.scan_stages_with(
        processes,
        &[],
        Some(home),
        |_, answer| rows = answer,
        |scanner, rows| scanner.attach_codex_sessions(processes, Some(home), rows),
    );
    rows
}

#[test]
fn resumed_codex_with_shared_rollouts_keeps_its_database_name_and_account() {
    let root = tempfile::tempdir().expect("isolated homes");
    let first = root.path().join(".codex");
    let second = root.path().join(".codex2");
    let sessions = first.join("sessions");
    fs::create_dir_all(&sessions).expect("shared rollouts");
    let rollout = sessions.join(format!("rollout-{THREAD}.jsonl"));
    fs::write(&rollout, "").expect("rollout");
    database(&first, &rollout, "other account");
    database(&second, &rollout, "morty");
    std::os::unix::fs::symlink(&sessions, second.join("sessions")).expect("shared sessions link");
    fs::write(
        first.join("session_index.jsonl"),
        format!("{{\"id\":\"{THREAD}\",\"thread_name\":\"wrong index\"}}\n"),
    )
    .expect("other account index");
    let process = ProcessEntry {
        pid:         10,
        parent:      None,
        name:        "codex".into(),
        arguments:   vec!["codex".into(), "resume".into(), THREAD.into()],
        started:     100,
        directory:   Some("/fixture".into()),
        claude_pid:  None,
        config_home: ConfigHome::Named(second.clone()),
    };
    let foreign = codex::read_resumed_threads(&process, &first);
    assert_eq!(
        foreign.len(),
        1,
        "the other database contains the same thread id"
    );
    assert!(
        classify::linked_threads(&classify::codex_session_links(
            std::slice::from_ref(&process),
            Some(root.path()),
            &codex::CodexLogs::default(),
            &codex::ThreadRows::default(),
        ))
        .is_empty(),
        "a shared rollout cannot give another account ownership"
    );
    assert_eq!(scan(&process, root.path())[0].name, "pid 10");
    let mut threads = foreign;
    threads.extend(codex::read_resumed_threads(&process, &second));
    assert_eq!(threads.len(), 2, "both account databases were read");
    let logs = rusqlite::Connection::open(second.join("logs_2.sqlite")).expect("own log database");
    logs.execute_batch("PRAGMA synchronous = OFF; CREATE TABLE logs (id INTEGER PRIMARY KEY, ts INTEGER, process_uuid TEXT, thread_id TEXT, feedback_log_body TEXT); CREATE INDEX idx_logs_ts ON logs(ts DESC);").expect("own log table");
    logs.execute("INSERT INTO logs VALUES (1, 100, 'pid:10:fixture', ?1, ?2)", rusqlite::params![THREAD,
        "app_server.request{rpc.method=\"thread/resume\" rpc.request_id=1 app_server.connection_id=1 app_server.client_name=\"codex-tui\"}: composing running thread resume response request_id=ConnectionRequestId { connection_id: ConnectionId(1), request_id: Integer(1) }"]).expect("own process selection");
    let rows = scan(&process, root.path());
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].name, "morty",
        "a missing index retains this database's name"
    );
    assert_eq!(
        rows[0].account,
        crate::census::account::Account::Home(
            AccountLabels::default().resolve(&second, Some(root.path()))
        )
    );
    fs::write(
        second.join("session_index.jsonl"),
        format!("{{\"id\":\"{THREAD}\",\"thread_name\":\"renamed morty\"}}\n"),
    )
    .expect("own index");
    assert_eq!(scan(&process, root.path())[0].name, "renamed morty");
}

#[cfg(target_os = "linux")]
#[test]
fn app_server_open_thread_uses_its_second_codex_home() {
    let root = tempfile::tempdir().expect("isolated homes");
    let session = root.path().join("unit");
    fs::create_dir_all(&session).expect("unit directory");
    worker_dispatch::write_worker_fixture(
        &session,
        &serde_json::json!({
            "fixture-impl":{"current_session":THREAD,"role":"test"}
        }),
    );
    let mut scanner = LocalScanner::new(AccountLabels::default());
    scanner.worker_names = worker_dispatch::fixture_names(&session);
    scanner.desktop_source = DesktopSource::Windows(Vec::new());
    scanner.session_roots = SessionRoots::new([root.path().to_owned()]);
    let second_home = root.path().join(".codex2");
    fs::create_dir_all(second_home.join(CODEX_SESSIONS_DIRNAME)).expect("second home sessions");
    let rollout = second_home
        .join(CODEX_SESSIONS_DIRNAME)
        .join(format!("rollout-test-{THREAD}.jsonl"));
    let prompt = format!(
        "Write fixtures\nSummary: {}/phases/4/summary.txt",
        session.display()
    );
    let mut held = fs::File::create(&rollout).expect("owned rollout descriptor");
    writeln!(
        held,
        "{}",
        serde_json::json!({"type":"response_item", "payload":{
            "type":"message", "role":"user", "content":[{"type":"input_text", "text":prompt}]
        }})
    )
    .expect("thread prompt");
    let mut server = activity_scan_tests::process(std::process::id(), 10, &["codex", "app-server"]);
    server.config_home = ConfigHome::Named(second_home);
    let processes = [activity_scan_tests::process(10, 1, &["claude"]), server];
    let mut rows: Vec<AgentRow> = serde_json::from_value(serde_json::json!([{
        "agent":"claude", "session_id":"director-id", "name":"director", "pid":10, "started":10,
        "status":"idle", "desktop":null, "directory":"/fixture", "branch":null, "launched_by":null, "children":[]
    }])).expect("director row");
    scanner.attach_details(&processes, &[], Some(root.path()), &mut rows);
    assert_eq!(
        rows[0]
            .children
            .iter()
            .filter(|child| child.thread_id == CodexThreadId::Recorded(THREAD.to_owned()))
            .count(),
        1
    );
    assert_eq!(rows[0].activity, AgentActivity::Idle);
    drop(held);
}

#[cfg(target_os = "linux")]
#[test]
fn an_open_thread_in_symlinked_sessions_keeps_its_account_database_name() {
    let root = tempfile::tempdir().expect("isolated homes");
    let first = root.path().join(".codex");
    let second = root.path().join(".codex2");
    let sessions = first.join(CODEX_SESSIONS_DIRNAME);
    fs::create_dir_all(&sessions).expect("shared sessions");
    let rollout = sessions.join(format!("rollout-test-{THREAD}.jsonl"));
    fs::write(&rollout, "").expect("rollout");
    database(&first, &rollout, "wrong account");
    database(&second, &rollout, "second account thread");
    std::os::unix::fs::symlink(&sessions, second.join(CODEX_SESSIONS_DIRNAME))
        .expect("sessions link");
    let held = fs::File::open(&rollout).expect("owned open rollout");
    let mut server = activity_scan_tests::process(std::process::id(), 1, &["codex", "app-server"]);
    server.config_home = ConfigHome::Named(second.clone());
    let mut rollouts = Rollouts::default();
    let mut names = codex::OpenThreadRows::default();
    let rows = app_server_threads(
        &[server],
        &mut rollouts,
        &mut names,
        &PacerHistory::default(),
        Some(root.path()),
        &AccountLabels::default(),
    );
    let thread = &rows[&std::process::id()][0];
    assert_eq!(thread.name, "second account thread");
    assert_eq!(
        thread.account,
        crate::census::Account::Home(AccountLabels::default().resolve(&second, Some(root.path())))
    );
    drop(held);
}

#[test]
fn open_thread_names_reuse_the_database_until_its_wal_changes() {
    let root = tempfile::tempdir().expect("isolated database");
    let rollout = root.path().join(format!("rollout-test-{THREAD}.jsonl"));
    database(root.path(), &rollout, "first name");
    let connection =
        rusqlite::Connection::open(root.path().join("state_6.sqlite")).expect("database writer");
    connection
        .execute_batch("PRAGMA journal_mode = WAL; PRAGMA wal_autocheckpoint = 0;")
        .expect("WAL mode");
    let mut names = codex::OpenThreadRows::default();
    let ids = [THREAD.to_owned()];
    assert_eq!(
        names.read(root.path(), &ids)[THREAD].label().as_deref(),
        Some("first name")
    );
    assert_eq!(names.database_reads, 1);
    assert_eq!(
        names.read(root.path(), &ids)[THREAD].label().as_deref(),
        Some("first name")
    );
    assert_eq!(
        names.database_reads, 1,
        "an unchanged database is read once"
    );
    connection
        .execute(
            "UPDATE threads SET name = 'new name' WHERE id = ?1",
            [THREAD],
        )
        .expect("rename in WAL");
    assert_eq!(
        names.read(root.path(), &ids)[THREAD].label().as_deref(),
        Some("new name")
    );
    assert_eq!(names.database_reads, 2);
}
