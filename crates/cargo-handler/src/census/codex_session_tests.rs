//! Interactive Codex sessions draw their own name, activity and service tier.

#![expect(
    clippy::expect_used,
    reason = "isolated Codex fixtures must be readable"
)]

use std::io::Write;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
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
            config_home: ConfigHome::Named(codex_home),
        };
        let thread = CodexThread {
            cwd: "/fixture".into(),
            created_ms: START * 1000,
            name: Some("database-title".into()),
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
        let mut rows = classify::agent_rows(
            &self.processes,
            &[],
            &self.threads,
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
    let (_, _, pool) = drawn(fixture.scan(), RootWork::Absent);
    assert!(pool.iter().any(|line| line.contains("pid 10")), "{pool:?}");
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
    let other_id = "01a159cc-91d4-70c3-94a8-9c2a4259cd86";
    let mut other = fixture.threads[0].clone();
    other.rollout = named_home.join(format!("sessions/rollout-{other_id}.jsonl"));
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
    connection.execute_batch("CREATE TABLE threads (id TEXT PRIMARY KEY, cwd TEXT NOT NULL, created_at_ms INTEGER, name TEXT, first_user_message TEXT, rollout_path TEXT)").expect("threads table");
    for thread in fixture
        .threads
        .iter()
        .filter(|thread| thread.rollout.starts_with(&codex_home))
    {
        connection
            .execute(
                "INSERT INTO threads VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    tree::thread_id(&thread.rollout).expect("fixture thread id"),
                    thread.cwd.to_string_lossy(),
                    i64::try_from(thread.created_ms).expect("fixture time fits"),
                    thread.name,
                    thread.first_prompt,
                    thread.rollout.to_string_lossy(),
                ],
            )
            .expect("stored thread");
    }
}
