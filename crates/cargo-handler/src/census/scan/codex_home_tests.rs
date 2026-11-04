//! A database owns its account even when accounts share their rollout directory.

#![expect(
    clippy::expect_used,
    reason = "isolated account fixtures must be readable"
)]

use super::*;
use crate::census::account::ConfigHome;

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
