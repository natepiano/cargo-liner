//! Interactive Codex sessions draw their own name, activity and service tier.

#![expect(
    clippy::expect_used,
    reason = "isolated Codex fixtures must be readable"
)]

use std::io::Write;

use classify::CodexLogEvidence;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use rusqlite::Connection;
use tempfile::TempDir;

use super::*;
use crate::agent_cell;
use crate::agent_cell::AgentTile;
use crate::census::AccountColumn;
use crate::census::AgentPlacement;
use crate::census::Machine;
use crate::census::MachineState;
use crate::census::account::Program;
use crate::census::accounts::AccountsTable;
use crate::census::activity::Activity;
use crate::config::AccountsConfig;
use crate::config::Config;
use crate::constants::CODEX_AGENT;
use crate::constants::CODEX_APP_SERVER_ARGUMENT;
use crate::constants::CODEX_MANAGED_DAEMON_ARGUMENT;
use crate::constants::CODEX_RESUME_SUBCOMMAND;
use crate::footer_colors::FooterColors;
use crate::sort::AccountsSort;
use crate::sort::SummarySort;
use crate::summary::SummaryFrame;
use crate::summary::{self};

const START: u64 = 1_792_166_400;
const ID: &str = "01a159cc-91d4-70c3-94a8-9c2a4259cd85";
const AREA: Rect = Rect::new(0, 0, 200, 25);
const COMPLETE: &str = "{\"type\":\"event_msg\",\"payload\":{\"type\":\"task_complete\"}}\n";
const READ: &str = "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"exec_command\",\"arguments\":\"{\\\"cmd\\\":\\\"cat src/lib.rs\\\"}\"}}\n";

struct SessionFixture {
    home:      TempDir,
    processes: Vec<ProcessEntry>,
    threads:   Vec<CodexThread>,
    scanner:   LocalScanner,
}

impl SessionFixture {
    fn new() -> Self {
        let home = tempfile::tempdir().expect("fixture home");
        let codex_home = home.path().join(".codex2");
        fs::create_dir_all(&codex_home).expect("Codex home");
        let rollout = codex_home.join(format!("rollout-2026-10-16T00-00-00-{ID}.jsonl"));
        let head = format!(
            "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{ID}\",\"timestamp\":\"2026-10-16T00:00:00Z\",\"cwd\":\"/fixture\"}}}}\n{{\"type\":\"response_item\",\"payload\":{{\"type\":\"message\",\"role\":\"user\",\"content\":[{{\"type\":\"input_text\",\"text\":\"Inspect source files\"}}]}}}}\n{{\"type\":\"event_msg\",\"payload\":{{\"type\":\"thread_settings_applied\",\"thread_settings\":{{\"service_tier\":\"priority\"}}}}}}\n{COMPLETE}"
        );
        fs::write(&rollout, head).expect("rollout fixture");
        let process = ProcessEntry {
            pid:         10,
            parent:      None,
            name:        "codex".into(),
            arguments:   vec!["codex".into()],
            started:     START,
            directory:   Some("/fixture".into()),
            claude_pid:  None,
            config_home: ConfigHome::Named(codex_home.clone()),
        };
        let thread = CodexThread {
            account_home: codex::CodexAccountHome::from(codex_home.as_path()),
            cwd: "/fixture".into(),
            created_ms: START * 1000,
            name: SessionName::Named("database-title".into()),
            first_prompt: "Old task from the database".into(),
            rollout,
        };
        let mut scanner = LocalScanner::new(AccountLabels::new(&AccountsConfig::default(), None));
        scanner.desktop_source = DesktopSource::Windows(Vec::new());
        Self {
            home,
            processes: vec![process],
            threads: vec![thread],
            scanner,
        }
    }

    fn index(&self) -> PathBuf { self.home.path().join(".codex2/session_index.jsonl") }

    fn scan(&mut self) -> Vec<AgentRow> {
        self.scanner
            .codex_logs
            .refresh(&self.processes, Some(self.home.path()));
        let links = classify::codex_session_links(
            &self.processes,
            &self.threads,
            Some(self.home.path()),
            self.scanner.codex_start_window,
            CodexLogEvidence::Recorded(&self.scanner.codex_logs),
        );
        let threads = links
            .iter()
            .filter_map(|(pid, link)| link.thread().map(|thread| (*pid, thread)))
            .collect();
        let mut rows = classify::agent_rows_with_threads(
            &self.processes,
            &[],
            &threads,
            Some(self.home.path()),
            &self.scanner.account_labels,
        );
        self.scanner.attach_details(
            &self.processes,
            &[],
            &self.threads,
            Some(self.home.path()),
            &mut rows,
        );
        rows
    }
}

fn lines(buffer: &Buffer) -> Vec<String> {
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect()
}

#[derive(Clone, Copy)]
enum RootWork {
    Present,
    Absent,
}

fn drawn(rows: Vec<AgentRow>, work: RootWork) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut rows = rows;
    if matches!(work, RootWork::Present) {
        rows[0].children.push(
            serde_json::from_value(serde_json::json!({
                "depth":0,"kind":"shell","pid":21,"name":"cargo check","started":START
            }))
            .expect("work row"),
        );
    }
    let state = MachineState::Answered(rows, crate::census::HierarchyEdges::default().into());
    let machines = [Machine {
        name:  "fixture",
        state: &state,
    }];
    let placement = AgentPlacement::from_machines(&machines);
    let cells = agent_cell::cell_order_placed(&placement);
    let mut summary = Buffer::empty(AREA);
    summary::draw(
        &mut summary,
        AREA,
        &SummaryFrame {
            accounts:       &AccountsTable::NotConfigured,
            accounts_sort:  AccountsSort::SourceOrder,
            machines:       &machines,
            placement:      &placement,
            cells:          &cells,
            now:            START + 100,
            account_column: AccountColumn::Shown,
            footer_colors:  &FooterColors::default(),
            sort:           SummarySort::default(),
        },
    );
    let mut tree = Buffer::empty(AREA);
    agent_cell::draw(
        &mut tree,
        AREA,
        &cells[0],
        &cells,
        START + 100,
        AccountColumn::Shown,
        &FooterColors::default(),
    );
    let tiles = agent_cell::group_tiles(&placement, &cells, &HashMap::new(), START + 100);
    let mut pool = Buffer::empty(AREA);
    for tile in &tiles {
        if let AgentTile::Unassigned(entry) = tile {
            agent_cell::draw_unassigned(
                &mut pool,
                AREA,
                entry,
                START + 100,
                AccountColumn::Shown,
                &FooterColors::default(),
            );
        }
    }
    (lines(&summary), lines(&tree), lines(&pool))
}

#[test]
fn codex_session_name_uses_its_home_index_and_newest_complete_record() {
    let mut fixture = SessionFixture::new();
    fs::write(fixture.index(),format!("{{\"id\":\"{ID}\",\"thread_name\":\"old-name\",\"updated_at\":\"2026-10-20T10:00:00Z\"}}\n{{\"id\":\"{ID}\",\"thread_name\":\"designer\",\"updated_at\":\"2026-10-20T10:01:00Z\"}}\n")).expect("name index");
    let other = fixture.home.path().join(".codex");
    fs::create_dir_all(&other).expect("other account home");
    fs::write(
        other.join("session_index.jsonl"),
        format!("{{\"id\":\"{ID}\",\"thread_name\":\"wrong-account\"}}\n"),
    )
    .expect("other account index");
    let rows = fixture.scan();
    let (summary, _, _) = drawn(rows.clone(), RootWork::Present);
    let (_, _, pool) = drawn(rows, RootWork::Absent);
    for surface in [&summary, &pool] {
        assert!(
            surface
                .iter()
                .any(|line| line.split_whitespace().any(|word| word == "designer")),
            "{surface:?}"
        );
        assert!(
            !surface
                .iter()
                .any(|line| line.contains("old-name") || line.contains("wrong-account")),
            "{surface:?}"
        );
    }
    fixture.scan();
    let codex_home = fixture.index().parent().expect("index home").to_path_buf();
    assert_eq!(
        fixture.scanner.session_names.bytes_read(&codex_home),
        0,
        "unchanged indexes fetch no bytes"
    );
    let append = format!("{{\"id\":\"{ID}\",\"thread_name\":\"new-name\"}}");
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(fixture.index())
        .expect("index append");
    file.write_all(append.as_bytes())
        .expect("partial index record");
    assert_eq!(
        fixture.scan()[0].name,
        "designer",
        "incomplete records wait for a newline"
    );
    assert_eq!(
        fixture.scanner.session_names.bytes_read(&codex_home),
        u64::try_from(append.len()).expect("short append")
    );
    file.write_all(b"\n").expect("finish record");
    assert_eq!(
        fixture.scan()[0].name,
        "new-name",
        "existing reader consumes the new name"
    );
    assert_eq!(
        fixture.scanner.session_names.bytes_read(&codex_home),
        1,
        "only the appended newline is fetched"
    );
}

#[test]
fn codex_session_without_a_name_record_draws_its_pid() {
    let mut fixture = SessionFixture::new();
    fixture.threads[0].name = SessionName::Unnamed;
    let (_, _, pool) = drawn(fixture.scan(), RootWork::Absent);
    assert!(pool.iter().any(|line| line.contains("pid 10")), "{pool:?}");
}

#[test]
fn codex_interactive_roster_thread_uses_renamed_session_in_summary_and_tile() {
    let mut fixture = SessionFixture::new();
    store_threads(&fixture);
    let codex_home = fixture.index().parent().expect("Codex home").to_path_buf();
    let connection =
        rusqlite::Connection::open(codex_home.join("state_6.sqlite")).expect("fixture database");
    connection
        .execute_batch(
            "ALTER TABLE threads ADD COLUMN source TEXT;
        ALTER TABLE threads ADD COLUMN thread_source TEXT;
        UPDATE threads SET originator = 'roster', source = 'vscode', thread_source = 'user',
        created_at_ms = created_at_ms + 6000;",
        )
        .expect("interactive roster thread");
    fs::write(
        fixture.index(),
        format!("{{\"id\":\"{ID}\",\"thread_name\":\"test-codex\"}}\n"),
    )
    .expect("renamed session index");
    fixture.threads = codex::read_threads(&codex_home, &(START * 1000..=(START + 60) * 1000));
    let rows = fixture.scan();
    assert_eq!(rows[0].name, "test-codex");
    let (summary, _, _) = drawn(rows.clone(), RootWork::Present);
    let (_, _, pool) = drawn(rows, RootWork::Absent);
    for surface in [summary, pool] {
        assert!(
            surface.iter().any(|line| line.contains("test-codex")),
            "{surface:?}"
        );
        assert!(
            !surface.iter().any(|line| line.contains("pid 10")),
            "{surface:?}"
        );
    }
}

#[test]
fn codex_session_fast_tier_draws_on_summary_tree_and_unassigned_accounts() {
    let mut fixture = SessionFixture::new();
    let rows = fixture.scan();
    let (summary, tree, _) = drawn(rows.clone(), RootWork::Present);
    let (_, _, pool) = drawn(rows, RootWork::Absent);
    for surface in [&summary, &tree, &pool] {
        assert!(
            surface.iter().any(|line| line.contains("codex2 fast")),
            "{surface:?}"
        );
    }
}

#[test]
fn codex_session_own_rollout_draws_idle_and_then_reading_after_append() {
    let mut fixture = SessionFixture::new();
    fixture.threads[0].name = SessionName::Unnamed;
    for (append, expected) in [("", "idle"), (READ, "reading"), (COMPLETE, "idle")] {
        if !append.is_empty() {
            fs::OpenOptions::new()
                .append(true)
                .open(&fixture.threads[0].rollout)
                .expect("rollout append")
                .write_all(append.as_bytes())
                .expect("activity record");
        }
        let rows = fixture.scan();
        let (summary, tree, _) = drawn(rows.clone(), RootWork::Present);
        let (_, _, pool) = drawn(rows, RootWork::Absent);
        for surface in [&summary, &tree, &pool] {
            let row = surface
                .iter()
                .find(|line| line.contains("pid 10") || line.starts_with("10 "))
                .expect("Codex session row");
            assert!(
                row.split_whitespace().any(|word| word == expected),
                "{expected}: {row}"
            );
        }
    }
}

#[test]
fn codex_session_app_server_thread_without_dispatch_adds_no_worker_child() {
    let mut fixture = SessionFixture::new();
    fixture.processes.push(ProcessEntry {
        pid:         20,
        parent:      Some(10),
        name:        "codex".into(),
        arguments:   vec!["codex".into(), "app-server".into()],
        started:     START,
        directory:   Some("/fixture".into()),
        claude_pid:  None,
        config_home: fixture.processes[0].config_home.clone(),
    });
    let mut rows = classify::agent_rows(
        &fixture.processes,
        &[],
        &fixture.threads,
        Some(fixture.home.path()),
        &fixture.scanner.account_labels,
    );
    let sources = TreeSources {
        threads: HashMap::from([(
            20,
            vec![ThreadEntry {
                activity:     AgentActivity::Idle,
                thread_id:    CodexThreadId::Recorded(ID.into()),
                name:         "worker".into(),
                started:      START,
                service_tier: ServiceTier::Fast,
                account:      rows[0].account.clone(),
            }],
        )]),
        ..TreeSources::default()
    };
    tree::attach_children(&mut rows, &fixture.processes, &sources);
    assert!(
        rows[0].children.is_empty(),
        "a conversation without a dispatch is not a worker: {:?}",
        rows[0].children
    );
    let (_, _, pool) = drawn(rows, RootWork::Absent);
    assert!(
        !pool
            .iter()
            .any(|line| line.split_whitespace().any(|word| word == "worker")),
        "{pool:?}"
    );
}

#[test]
fn codex_session_removed_or_unreadable_name_index_clears_the_previous_name() {
    let mut fixture = SessionFixture::new();
    fixture.threads[0].name = SessionName::Unnamed;
    fs::write(
        fixture.index(),
        format!("{{\"id\":\"{ID}\",\"thread_name\":\"designer\"}}\n"),
    )
    .expect("name index");
    assert_eq!(fixture.scan()[0].name, "designer");
    fs::remove_file(fixture.index()).expect("remove index");
    assert_eq!(
        fixture.scan()[0].name,
        "pid 10",
        "a missing index clears cached names"
    );
    fs::create_dir(fixture.index()).expect("unreadable index path");
    assert_eq!(
        fixture.scan()[0].name,
        "pid 10",
        "a directory cannot supply names"
    );
    let (_, _, pool) = drawn(fixture.scan(), RootWork::Absent);
    assert!(pool.iter().any(|line| line.contains("pid 10")), "{pool:?}");
}

#[test]
fn default_and_named_codex_homes_keep_their_own_threads_with_shared_sessions() {
    let mut fixture = SessionFixture::new();
    let default_home = fixture.home.path().join(".codex");
    let sessions = default_home.join("sessions");
    fs::create_dir_all(&sessions).expect("shared sessions");
    let named_home = fixture.home.path().join(".codex2");
    std::os::unix::fs::symlink(&sessions, named_home.join("sessions")).expect("sessions link");
    let contents = fs::read(&fixture.threads[0].rollout).expect("rollout contents");
    fixture.threads[0].rollout = default_home.join(format!("sessions/rollout-{ID}.jsonl"));
    fs::write(&fixture.threads[0].rollout, &contents).expect("default rollout");
    fixture.processes[0].config_home = ConfigHome::Default(Program::Codex);
    fixture.threads[0].account_home = codex::CodexAccountHome::from(default_home.as_path());
    let other_id = "01a159cc-91d4-70c3-94a8-9c2a4259cd86";
    let mut other = fixture.threads[0].clone();
    other.rollout = named_home.join(format!("sessions/rollout-{other_id}.jsonl"));
    other.account_home = codex::CodexAccountHome::from(named_home.as_path());
    fs::write(&other.rollout, contents).expect("named rollout");
    fixture.threads.push(other);
    let mut process = fixture.processes[0].clone();
    process.pid = 20;
    process.config_home = ConfigHome::Named(named_home);
    fixture.processes.push(process);
    fs::write(
        default_home.join("session_index.jsonl"),
        format!("{{\"id\":\"{ID}\",\"thread_name\":\"default session\"}}\n"),
    )
    .expect("default index");
    fs::write(
        fixture.index(),
        format!("{{\"id\":\"{other_id}\",\"thread_name\":\"named session\"}}\n"),
    )
    .expect("named index");
    let rows = fixture.scan();
    assert_eq!(
        rows.iter()
            .map(|row| (row.pid, row.name.as_str()))
            .collect::<Vec<_>>(),
        [(10, "default session"), (20, "named session")]
    );
}

#[test]
fn codex_resume_last_uses_the_newest_older_thread_in_its_home_and_directory() {
    let mut fixture = SessionFixture::new();
    fixture.processes[0].arguments = vec!["codex".into(), "resume".into(), "--last".into()];
    fixture.threads[0].created_ms = (START - 100) * 1000;
    let mut older = fixture.threads[0].clone();
    older.created_ms -= 1000;
    older.rollout = fixture
        .home
        .path()
        .join(".codex2/rollout-01a159cc-91d4-70c3-94a8-9c2a4259cd86.jsonl");
    fs::write(&older.rollout, COMPLETE).expect("older rollout");
    let mut other_cwd = fixture.threads[0].clone();
    other_cwd.cwd = "/elsewhere".into();
    other_cwd.created_ms += 1000;
    other_cwd.rollout = fixture
        .home
        .path()
        .join(".codex2/rollout-01a159cc-91d4-70c3-94a8-9c2a4259cd87.jsonl");
    fs::write(&other_cwd.rollout, COMPLETE).expect("other directory rollout");
    let mut other_home = fixture.threads[0].clone();
    other_home.rollout = fixture
        .home
        .path()
        .join(format!(".codex/rollout-{ID}.jsonl"));
    other_home.created_ms += 2000;
    other_home.account_home =
        codex::CodexAccountHome::from(fixture.home.path().join(".codex").as_path());
    fixture.threads.extend([older, other_cwd, other_home]);
    fs::write(
        fixture.index(),
        format!("{{\"id\":\"{ID}\",\"thread_name\":\"resumed designer\"}}\n"),
    )
    .expect("session index");
    fs::OpenOptions::new()
        .append(true)
        .open(&fixture.threads[0].rollout)
        .expect("rollout append")
        .write_all(READ.as_bytes())
        .expect("read activity");
    store_threads(&fixture);
    let resumed =
        codex::read_resumed_threads(&fixture.processes[0], &fixture.home.path().join(".codex2"));
    assert_eq!(resumed.len(), 1);
    assert_eq!(resumed[0].rollout, fixture.threads[0].rollout);
    let rows = fixture.scan();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name, "resumed designer");
    assert_eq!(rows[0].activity, AgentActivity::Active(Activity::Reading));
}

/// Store the same candidates the production scan queries from this account's database.
fn store_threads(fixture: &SessionFixture) {
    let codex_home = fixture.home.path().join(".codex2");
    let connection =
        rusqlite::Connection::open(codex_home.join("state_6.sqlite")).expect("Codex database");
    connection.execute_batch("CREATE TABLE threads (id TEXT PRIMARY KEY, cwd TEXT NOT NULL, created_at_ms INTEGER, name TEXT, first_user_message TEXT, rollout_path TEXT, originator TEXT DEFAULT 'codex-tui')").expect("threads table");
    for thread in fixture
        .threads
        .iter()
        .filter(|thread| thread.account_home.as_ref() == codex_home)
    {
        connection
            .execute(
                "INSERT INTO threads (id, cwd, created_at_ms, name, first_user_message, rollout_path) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    tree::thread_id(&thread.rollout).expect("fixture thread id"),
                    thread.cwd.to_string_lossy(),
                    i64::try_from(thread.created_ms).expect("fixture time fits"),
                    match &thread.name {
                        SessionName::Named(name) => Some(name.as_str()),
                        SessionName::Unnamed => None,
                    },
                    thread.first_prompt,
                    thread.rollout.to_string_lossy(),
                ],
            )
            .expect("stored thread");
    }
}

#[test]
fn codex_resume_picker_uses_its_child_log_and_tracks_a_thread_switch() {
    let mut fixture = SessionFixture::new();
    fixture.processes[0]
        .arguments
        .push(CODEX_RESUME_SUBCOMMAND.into());
    fixture.threads[0].created_ms = (START - 86_400) * 1000;
    fixture.threads[0].name = SessionName::Named("morty".into());
    let mut child = fixture.processes[0].clone();
    child.pid = 20;
    child.parent = Some(10);
    child.arguments = vec![
        CODEX_AGENT.into(),
        CODEX_APP_SERVER_ARGUMENT.into(),
        CODEX_MANAGED_DAEMON_ARGUMENT.into(),
    ];
    fixture.processes.push(child);
    store_threads(&fixture);
    let logs = log_database(&fixture);
    logs.execute(
        "INSERT INTO logs VALUES (1, ?1, 'pid:20:daemon', ?2)",
        rusqlite::params![i64::try_from(START).expect("fixture time"), ID],
    )
    .expect("thread log");
    let rows = fixture.scan();
    assert_eq!(rows[0].name, "morty", "the picker resumes an older thread");
    let (summary, _, _) = drawn(rows.clone(), RootWork::Present);
    let (_, _, tile) = drawn(rows, RootWork::Absent);
    for surface in [summary, tile] {
        assert!(
            surface.iter().any(|line| line.contains("morty")),
            "{surface:?}"
        );
    }
    assert!(fixture.scanner.codex_logs.rows_read() > 0);
    assert_eq!(fixture.scan()[0].name, "morty");
    assert_eq!(
        fixture.scanner.codex_logs.rows_read(),
        0,
        "unchanged logs return no old rows"
    );
    let next = "01a159cc-91d4-70c3-94a8-9c2a4259cd86";
    let database = rusqlite::Connection::open(fixture.home.path().join(".codex2/state_6.sqlite"))
        .expect("thread database");
    database.execute("INSERT INTO threads SELECT ?1, cwd, created_at_ms, 'summer', first_user_message, replace(rollout_path, ?2, ?1), originator FROM threads WHERE id = ?2",
        rusqlite::params![next, ID]).expect("second thread");
    logs.execute(
        "INSERT INTO logs VALUES (2, ?1, 'pid:20:daemon', ?2)",
        rusqlite::params![i64::try_from(START + 1).expect("fixture time"), next],
    )
    .expect("thread switch");
    assert_eq!(
        fixture.scan()[0].name,
        "summer",
        "the next scan follows /resume"
    );
    assert_eq!(
        fixture.scanner.codex_logs.rows_read(),
        1,
        "only the appended row is read"
    );
}

#[test]
fn codex_log_without_a_process_record_keeps_the_configured_start_window() {
    let mut fixture = SessionFixture::new();
    store_threads(&fixture);
    let logs = log_database(&fixture);
    logs.execute(
        "INSERT INTO logs VALUES (1, ?1, 'pid:10:old-process', ?2)",
        rusqlite::params![i64::try_from(START - 10).expect("fixture time"), ID],
    )
    .expect("old log");
    fixture.threads[0].created_ms += 5000;
    assert_eq!(fixture.scan()[0].name, "database-title");
    assert_eq!(
        fixture.scanner.codex_logs.rows_read(),
        0,
        "the time index excludes old rows"
    );
    assert_eq!(fixture.scan()[0].name, "database-title");
    assert_eq!(
        fixture.scanner.codex_logs.rows_read(),
        0,
        "an empty initial range still advances the cursor"
    );
    let config: Config =
        toml::from_str("[census]\ncodex_thread_start_window = 2").expect("short startup window");
    fixture.scanner.codex_start_window = config.census.codex_thread_start_window;
    assert_eq!(
        fixture.scan()[0].name,
        "pid 10",
        "the configured window controls fallback"
    );
}

#[test]
fn codex_shared_daemon_never_assigns_its_thread_to_two_tuis() {
    let mut fixture = SessionFixture::new();
    fixture.threads[0].created_ms = (START - 86_400) * 1000;
    fixture.processes[0]
        .arguments
        .push(CODEX_RESUME_SUBCOMMAND.into());
    store_threads(&fixture);
    let mut child = fixture.processes[0].clone();
    child.pid = 20;
    child.parent = Some(10);
    child.arguments = vec![
        CODEX_AGENT.into(),
        CODEX_APP_SERVER_ARGUMENT.into(),
        CODEX_MANAGED_DAEMON_ARGUMENT.into(),
    ];
    let mut second = fixture.processes[0].clone();
    second.pid = 30;
    fixture.processes.extend([child, second]);
    let logs = log_database(&fixture);
    logs.execute(
        "INSERT INTO logs VALUES (1, ?1, 'pid:20:daemon', ?2)",
        rusqlite::params![i64::try_from(START).expect("fixture time"), ID],
    )
    .expect("shared log");
    assert_eq!(
        fixture
            .scan()
            .iter()
            .map(|row| row.name.as_str())
            .collect::<Vec<_>>(),
        ["pid 10", "app-server", "pid 30"]
    );
    logs.execute(
        "INSERT INTO logs VALUES (2, ?1, 'pid:30:tui', ?2)",
        rusqlite::params![i64::try_from(START).expect("fixture time"), ID],
    )
    .expect("direct process log");
    let rows = fixture.scan();
    assert_eq!(rows[0].name, "pid 10");
    assert_eq!(
        rows[2].name, "database-title",
        "a direct process link remains exact"
    );
}

/// Create a disposable log database with the production time index.
fn log_database(fixture: &SessionFixture) -> Connection {
    let connection = rusqlite::Connection::open(fixture.home.path().join(".codex2/logs_2.sqlite"))
        .expect("log database");
    connection
        .execute_batch(
            "PRAGMA synchronous = OFF;
        CREATE TABLE logs (id INTEGER PRIMARY KEY, ts INTEGER, process_uuid TEXT, thread_id TEXT);
        CREATE INDEX idx_logs_ts ON logs(ts DESC);",
        )
        .expect("log table");
    connection
}

fn log_thread(logs: &Connection, id: i64, pid: u32, thread: &str) {
    logs.execute(
        "INSERT INTO logs VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![
            id,
            i64::try_from(START).expect("fixture time"),
            format!("pid:{pid}:fixture"),
            thread
        ],
    )
    .expect("thread log");
}

fn managed_child(fixture: &mut SessionFixture) {
    let mut child = fixture.processes[0].clone();
    child.pid = 20;
    child.parent = Some(10);
    child.arguments = vec![
        CODEX_AGENT.into(),
        CODEX_APP_SERVER_ARGUMENT.into(),
        CODEX_MANAGED_DAEMON_ARGUMENT.into(),
    ];
    fixture.processes.push(child);
}

#[test]
fn resume_picker_with_a_managed_child_uses_the_child_thread_record() {
    let mut fixture = SessionFixture::new();
    fixture.processes[0]
        .arguments
        .push(CODEX_RESUME_SUBCOMMAND.into());
    fixture.threads[0].created_ms = (START - 86_400) * 1000;
    fixture.threads[0].name = SessionName::Named("morty".into());
    managed_child(&mut fixture);
    store_threads(&fixture);
    let logs = log_database(&fixture);
    log_thread(&logs, 1, 20, ID);
    let rows = fixture.scan();
    assert_eq!(rows[0].name, "morty");
    let (summary, _, _) = drawn(rows.clone(), RootWork::Present);
    let (_, _, tile) = drawn(rows, RootWork::Absent);
    for surface in [summary, tile] {
        assert!(
            surface.iter().any(|line| line.contains("morty")),
            "{surface:?}"
        );
    }
}

fn second_thread(fixture: &SessionFixture) -> String {
    let next = "01a159cc-91d4-70c3-94a8-9c2a4259cd86";
    let database = rusqlite::Connection::open(fixture.home.path().join(".codex2/state_6.sqlite"))
        .expect("thread database");
    database.execute("INSERT INTO threads SELECT ?1, cwd, created_at_ms, 'summer', first_user_message, replace(rollout_path, ?2, ?1), originator FROM threads WHERE id = ?2",
        rusqlite::params![next, ID]).expect("second thread");
    next.into()
}

#[test]
fn shared_daemon_preserves_start_window_and_argv_names() {
    let mut fixture = SessionFixture::new();
    store_threads(&fixture);
    managed_child(&mut fixture);
    let mut second = fixture.processes[0].clone();
    second.pid = 30;
    second
        .arguments
        .extend([CODEX_RESUME_SUBCOMMAND.into(), ID.into()]);
    second.directory = Some("/other".into());
    fixture.processes.push(second);
    let logs = log_database(&fixture);
    log_thread(&logs, 1, 20, ID);
    let rows = fixture.scan();
    assert_eq!(
        rows.iter()
            .find(|row| row.pid == 10)
            .expect("first TUI")
            .name,
        "database-title"
    );
    assert_eq!(
        rows.iter()
            .find(|row| row.pid == 30)
            .expect("second TUI")
            .name,
        "database-title"
    );
}

#[test]
fn shared_daemon_record_stays_ambiguous_after_a_client_exits() {
    let mut fixture = SessionFixture::new();
    fixture.threads[0].created_ms = (START - 86_400) * 1000;
    store_threads(&fixture);
    managed_child(&mut fixture);
    let mut second = fixture.processes[0].clone();
    second.pid = 30;
    fixture.processes.push(second);
    let logs = log_database(&fixture);
    log_thread(&logs, 1, 20, ID);
    assert_eq!(fixture.scan()[0].name, "pid 10");
    fixture.processes.retain(|process| process.pid != 30);
    assert_eq!(
        fixture.scan()[0].name,
        "pid 10",
        "a departed client does not make its shared record exact"
    );
    log_thread(&logs, 2, 10, ID);
    assert_eq!(
        fixture.scan()[0].name,
        "database-title",
        "a new direct record is exact"
    );
}

#[test]
fn explicit_resume_follows_a_newer_log_thread() {
    let mut fixture = SessionFixture::new();
    fixture.processes[0]
        .arguments
        .extend([CODEX_RESUME_SUBCOMMAND.into(), ID.into()]);
    store_threads(&fixture);
    let next = second_thread(&fixture);
    let logs = log_database(&fixture);
    assert_eq!(fixture.scan()[0].name, "database-title");
    log_thread(&logs, 1, 10, &next);
    assert_eq!(
        fixture.scan()[0].name,
        "summer",
        "a log switch supersedes the argv id"
    );
}

#[test]
fn newest_log_wins_across_the_tui_and_its_child() {
    let mut fixture = SessionFixture::new();
    store_threads(&fixture);
    managed_child(&mut fixture);
    let next = second_thread(&fixture);
    let logs = log_database(&fixture);
    log_thread(&logs, 1, 10, ID);
    log_thread(&logs, 2, 20, &next);
    assert_eq!(
        fixture.scan()[0].name,
        "summer",
        "the child has the newest log id"
    );
    log_thread(&logs, 3, 10, ID);
    assert_eq!(
        fixture.scan()[0].name,
        "database-title",
        "the TUI now has the newest log id"
    );
}

#[test]
fn delayed_thread_row_resolves_without_reading_old_logs() {
    let mut fixture = SessionFixture::new();
    fixture.threads[0].created_ms = (START - 86_400) * 1000;
    store_threads(&fixture);
    let database = rusqlite::Connection::open(fixture.home.path().join(".codex2/state_6.sqlite"))
        .expect("thread database");
    database
        .execute("DELETE FROM threads", [])
        .expect("delayed thread");
    let logs = log_database(&fixture);
    log_thread(&logs, 1, 10, ID);
    assert_eq!(fixture.scan()[0].name, "pid 10");
    database
        .execute(
            "INSERT INTO threads VALUES (?1, '/fixture', 0, 'late name', '', ?2, 'codex-tui')",
            rusqlite::params![ID, fixture.threads[0].rollout.to_string_lossy()],
        )
        .expect("thread commit");
    assert_eq!(fixture.scan()[0].name, "late name");
    assert_eq!(fixture.scanner.codex_logs.rows_read(), 0);
    database
        .execute("UPDATE threads SET originator = 'exec'", [])
        .expect("non-user thread");
    assert_eq!(
        fixture.scan()[0].name,
        "pid 10",
        "a confirmed non-user thread supplies no link"
    );
}

#[test]
fn linked_thread_uses_current_database_name_and_first_prompt() {
    let mut fixture = SessionFixture::new();
    store_threads(&fixture);
    let logs = log_database(&fixture);
    log_thread(&logs, 1, 10, ID);
    assert_eq!(fixture.scan()[0].name, "database-title");
    let database = rusqlite::Connection::open(fixture.home.path().join(".codex2/state_6.sqlite"))
        .expect("thread database");
    database
        .execute("UPDATE threads SET name = 'renamed'", [])
        .expect("rename");
    assert_eq!(fixture.scan()[0].name, "renamed");
    database
        .execute(
            "UPDATE threads SET name = NULL, first_user_message = 'new first prompt'",
            [],
        )
        .expect("prompt");
    assert_eq!(fixture.scan()[0].name, "pid 10");
    let links = classify::codex_session_links(
        &fixture.processes,
        &fixture.threads,
        Some(fixture.home.path()),
        fixture.scanner.codex_start_window,
        CodexLogEvidence::Recorded(&fixture.scanner.codex_logs),
    );
    let thread = links
        .get(&10)
        .and_then(classify::CodexSessionLink::thread)
        .expect("linked thread");
    assert_eq!(thread.first_prompt, "new first prompt");
    assert_eq!(fixture.scanner.codex_logs.rows_read(), 0);
}

#[test]
fn log_scans_resolve_only_live_process_threads_and_reuse_unchanged_rows() {
    let mut fixture = SessionFixture::new();
    store_threads(&fixture);
    let logs = log_database(&fixture);
    log_thread(&logs, 1, 99, "unrelated-thread");
    log_thread(&logs, 2, 10, ID);
    assert_eq!(fixture.scan()[0].name, "database-title");
    assert_eq!(fixture.scanner.codex_logs.thread_lookups(), 1);
    assert_eq!(fixture.scan()[0].name, "database-title");
    assert_eq!(
        fixture.scanner.codex_logs.thread_lookups(),
        0,
        "an unchanged thread database needs no row lookup"
    );
    assert_eq!(fixture.scanner.codex_logs.rows_read(), 0);
}

#[test]
fn a_process_missing_from_one_snapshot_keeps_its_new_log_record() {
    let mut fixture = SessionFixture::new();
    fixture.threads[0].created_ms = (START - 86_400) * 1000;
    store_threads(&fixture);
    let logs = log_database(&fixture);
    assert_eq!(fixture.scan()[0].name, "pid 10");
    log_thread(&logs, 1, 30, ID);
    assert_eq!(fixture.scan()[0].name, "pid 10");
    let mut second = fixture.processes[0].clone();
    second.pid = 30;
    fixture.processes.push(second);
    let rows = fixture.scan();
    assert_eq!(
        rows.iter().find(|row| row.pid == 30).expect("new TUI").name,
        "database-title"
    );
    assert_eq!(fixture.scanner.codex_logs.rows_read(), 0);
    log_thread(&logs, 2, 40, ID);
    fixture.scan();
    fixture.scan();
    let mut departed = fixture.processes[0].clone();
    departed.pid = 40;
    fixture.processes.push(departed);
    let rows = fixture.scan();
    assert_eq!(
        rows.iter()
            .find(|row| row.pid == 40)
            .expect("absent pid")
            .name,
        "pid 40"
    );
    fixture.processes[1].started += 1;
    let rows = fixture.scan();
    assert_eq!(
        rows.iter()
            .find(|row| row.pid == 30)
            .expect("reused pid")
            .name,
        "pid 30"
    );
}

#[test]
fn linked_threads_follow_a_new_database_version_and_a_replaced_file() {
    let mut fixture = SessionFixture::new();
    fixture.threads[0].created_ms = (START - 86_400) * 1000;
    store_threads(&fixture);
    let logs = log_database(&fixture);
    log_thread(&logs, 1, 10, ID);
    assert_eq!(fixture.scan()[0].name, "database-title");
    let home = fixture.home.path().join(".codex2");
    fs::copy(home.join("state_6.sqlite"), home.join("state_7.sqlite")).expect("new version");
    let database = Connection::open(home.join("state_7.sqlite")).expect("new database");
    database
        .execute("UPDATE threads SET name = 'new version'", [])
        .expect("new name");
    drop(database);
    assert_eq!(fixture.scan()[0].name, "new version");
    assert_eq!(fixture.scanner.codex_logs.rows_read(), 0);
    fs::copy(home.join("state_6.sqlite"), home.join("replacement.sqlite")).expect("replacement");
    fs::rename(home.join("replacement.sqlite"), home.join("state_7.sqlite")).expect("replace file");
    assert_eq!(fixture.scan()[0].name, "database-title");
    assert_eq!(fixture.scanner.codex_logs.rows_read(), 0);
}
