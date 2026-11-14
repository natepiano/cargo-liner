//! `census --json` reports the tile values and names each unread fact.

#![expect(
    clippy::expect_used,
    reason = "fixture JSON, files and terminal drawing must succeed"
)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use serde_json::Value;
use tempfile::TempDir;
use tui_pane::ConfigError;
use tui_pane::TerminalApp;

use super::*;
use crate::app::App;
use crate::census::CensusFacts;
use crate::census::HierarchyEdges;
use crate::census::PassAssignment;
use crate::census::UnknownReason;
use crate::census::accounts::AccountsReading;
use crate::census::hierarchy::HierarchyNotice;
use crate::census::hierarchy::RunId;
use crate::census::remote;
use crate::census::remote::ProbeOutcome;

const README: &str = include_str!("../../README.md");
const EXAMPLE_MARKER: &str = "<!-- census-json-example";
const ACCOUNTS: &str = r#"{"accounts":[{"label":"claude1","roles":[],"use_credits":false,
    "sessions":[{"session":"boss","state":"idle"},{"session":"director","state":"busy"},
        {"session":"alpha","state":"busy"}]}],
    "problems":[]}"#;
/// Each plain activity word of the definition shared with the accounts board.
const ACTIVITY_WORDS: [&str; 11] = [
    "thinking",
    "compacting",
    "writing",
    "reading",
    "git",
    "delegating",
    "web",
    "running",
    "asking",
    "waiting",
    "starting",
];

/// A remote that never answers.
struct Unreachable;

impl RemoteRunner for Unreachable {
    fn probe(&self, _: &str) -> ProbeOutcome { ProbeOutcome::TimedOut }
}

fn rows() -> Vec<AgentRow> { rows_with("waiting") }

/// A showrunner, its unit director, two current-run workers and one stale worker row.
/// The alpha worker does `alpha_activity`.
fn rows_with(alpha_activity: &str) -> Vec<AgentRow> {
    serde_json::from_value(serde_json::json!([
        {"agent":"claude","pid":10,"session_id":"boss","name":"boss-name","started":1,
         "directory":"/fixture","activity":{"active":"thinking"},"children":[]},
        {"agent":"claude","pid":20,"session_id":"director","name":"director-name","started":1,
         "directory":"/fixture","activity":{"active":"waiting"},"children":[]},
        {"agent":"claude","pid":30,"session_id":"alpha","name":"alpha-name","started":1,
         "directory":"/fixture","activity":{"active":alpha_activity},"children":[]},
        {"agent":"claude","pid":40,"session_id":"beta","name":"beta-name","started":1,
         "directory":"/fixture","children":[]},
        {"agent":"claude","pid":50,"session_id":"stale","name":"stale-name","started":1,
         "directory":"/fixture","children":[]}
    ]))
    .expect("fixture rows")
}

fn edges() -> HierarchyEdges {
    serde_json::from_value(serde_json::json!([
        {"manager":"boss","member":"director","role":"unit"},
        {"manager":"director","member":"alpha","role":"worker"},
        {"manager":"director","member":"beta","role":"worker"},
        {"manager":"director","member":"stale","role":"worker"}
    ]))
    .expect("fixture edges")
}

fn run(label: &str, state: DetachedRunState, started: Observed<u64>) -> DetachedRunFact {
    DetachedRunFact {
        id: label.to_owned(),
        label: label.to_owned(),
        state,
        started,
    }
}

fn worker(id: &str, current_run: StaffRunPointer) -> WorkerFacts {
    WorkerFacts {
        listing:          Observed::Known(StaffListing::Worker {
            id:     id.to_owned(),
            parent: "unit-id".to_owned(),
        }),
        current_run:      Observed::Known(current_run),
        pane:             Observed::Known(PaneState::Open),
        runs:             Observed::Known(Vec::new()),
        activity_started: Observed::unknown("worker has no open pass"),
        pass:             Observed::Known(PassFact::Closed),
    }
}

fn current_run() -> StaffRunPointer {
    StaffRunPointer::Current(RunId::try_from(PathBuf::from("/runs/unit")).expect("absolute run"))
}

/// The facts a scan records for the fixture: the stale worker is outside the current run,
/// and the retired director and the gone worker have no tile line.
fn facts() -> CensusFacts { facts_with(|_| {}) }

/// The fixture facts, with one change to the alpha worker.
fn facts_with(edit: impl FnOnce(&mut WorkerFacts)) -> CensusFacts {
    let mut alpha = worker("alpha-id", current_run());
    alpha.runs = Observed::Known(vec![
        run(
            "verify test",
            DetachedRunState::Running,
            Observed::Known(300),
        ),
        run(
            "verify lint",
            DetachedRunState::FinishedQueued,
            Observed::Known(200),
        ),
        run(
            "verify doc",
            DetachedRunState::Missing,
            Observed::unknown("run has no start time"),
        ),
    ]);
    alpha.activity_started = Observed::Known(300);
    alpha.pass = Observed::Known(PassFact::Open(PassAssignment {
        role:  PassRole::Impl,
        phase: "8".to_owned(),
    }));
    edit(&mut alpha);
    let mut beta = worker("beta-id", StaffRunPointer::NoRun);
    beta.pane = Observed::Known(PaneState::Stopped);
    let director = WorkerFacts {
        listing:          Observed::Known(StaffListing::Director {
            id: "unit-id".to_owned(),
        }),
        current_run:      Observed::Known(current_run()),
        pane:             Observed::unknown("no readable pane observation for this worker"),
        runs:             Observed::Known(Vec::new()),
        activity_started: Observed::unknown("no running detached run or open pass start"),
        pass:             Observed::Known(PassFact::Closed),
    };
    let boss = WorkerFacts {
        listing: Observed::Known(StaffListing::Other),
        ..director.clone()
    };
    let retired = WorkerFacts {
        listing: Observed::Known(StaffListing::Director {
            id: "retired-id".to_owned(),
        }),
        ..director.clone()
    };
    let gone = worker("gone-id", current_run());
    CensusFacts {
        staff:    Observed::Known(SourceHealth::Loaded),
        accounts: Observed::Known(SourceHealth::Loaded),
        workers:  Observed::Known(BTreeMap::from([
            ("boss".to_owned(), boss),
            ("director".to_owned(), director),
            ("alpha".to_owned(), alpha),
            ("beta".to_owned(), beta),
            ("retired".to_owned(), retired),
            ("gone".to_owned(), gone),
        ])),
    }
}

fn snapshot(facts: CensusFacts) -> HierarchySnapshot {
    HierarchySnapshot {
        edges:  edges(),
        notice: HierarchyNotice::Clear,
        facts:  Box::new(facts),
    }
}

/// A config whose accounts command prints `ACCOUNTS`, or reads no accounts.
fn config(directory: &TempDir, accounts: bool) -> Config {
    if accounts {
        config_with_accounts(directory, ACCOUNTS)
    } else {
        let mut config = Config::default();
        config.census.accounts_command = String::new();
        config
    }
}

/// A config whose accounts command prints `accounts`.
fn config_with_accounts(directory: &TempDir, accounts: &str) -> Config {
    let mut config = Config::default();
    let path = directory.path().join("accounts.json");
    std::fs::write(&path, accounts).expect("accounts fixture");
    config.census.accounts_command = format!("cat '{}'", path.display());
    config
}

fn print_census(loaded: &LoadedConfig, facts: CensusFacts) -> (ExitCode, Value, String) {
    print_census_with(loaded, rows(), facts)
}

fn print_census_with(
    loaded: &LoadedConfig,
    rows: Vec<AgentRow>,
    facts: CensusFacts,
) -> (ExitCode, Value, String) {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let status = print_to(
        loaded,
        &Unreachable,
        |_| (rows, snapshot(facts)),
        &mut stdout,
        &mut stderr,
    );
    let json = if stdout.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&stdout).expect("census JSON")
    };
    (
        status,
        json,
        String::from_utf8(stderr).expect("diagnostic text"),
    )
}

fn report(config: Config, facts: CensusFacts) -> Value { report_with(config, rows(), facts) }

fn report_with(config: Config, rows: Vec<AgentRow>, facts: CensusFacts) -> Value {
    let (status, json, stderr) = print_census_with(
        &LoadedConfig {
            config,
            error: None,
        },
        rows,
        facts,
    );
    assert_eq!(status, ExitCode::SUCCESS, "{stderr}");
    json
}

fn session<'a>(json: &'a Value, id: &str) -> &'a Value {
    json["sessions"]
        .as_array()
        .expect("sessions")
        .iter()
        .find(|entry| entry["session"] == id)
        .expect("listed session")
}

/// The tile line that names `name`, or an empty line.
fn line_of<'a>(painted: &'a [String], name: &str) -> &'a str {
    painted
        .iter()
        .find(|line| line.contains(name))
        .map_or("", String::as_str)
}

fn entry<'a>(json: &'a Value, list: &str, staff_id: &str) -> &'a Value {
    json[list]
        .as_array()
        .expect("list")
        .iter()
        .find(|entry| entry["staff_id"] == staff_id)
        .expect("listed staff id")
}

fn painted(directory: &TempDir) -> Vec<String> { painted_with(directory, rows()) }

fn painted_with(directory: &TempDir, rows: Vec<AgentRow>) -> Vec<String> {
    let mut app = App::new_for_test().expect("test app");
    app.census.apply(CensusUpdate::Local {
        rows,
        hierarchy: snapshot(facts()),
        stage: ScanStage::Completed,
    });
    let bytes = std::fs::read(directory.path().join("accounts.json")).expect("accounts fixture");
    app.accounts.table = AccountsTable::Ready(AccountsReading::parse(&bytes).expect("accounts"));
    let mut terminal = Terminal::new(TestBackend::new(200, 60)).expect("terminal");
    let keymap = app.keymap();
    terminal
        .draw(|frame| app.draw(frame, &keymap))
        .expect("first draw");
    // The tile draws worker lines only under an expanded director.
    tui_pane::dispatch_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('='), KeyModifiers::NONE),
    );
    app.tiles.settle_for_test();
    terminal
        .draw(|frame| app.draw(frame, &keymap))
        .expect("settled draw");
    lines(terminal.backend().buffer())
}

fn lines(buffer: &Buffer) -> Vec<String> {
    (buffer.area.y..buffer.area.bottom())
        .map(|y| {
            (buffer.area.x..buffer.area.right())
                .map(|x| buffer[(x, y)].symbol())
                .collect()
        })
        .collect()
}

#[test]
fn census_json_lists_directors_and_workers_with_the_tile_status_and_activity() {
    let directory = tempfile::tempdir().expect("fixture directory");
    let json = report(config(&directory, true), facts());
    let director = entry(&json, "directors", "unit-id");
    assert_eq!(director["name"], "director-name");
    assert_eq!(director["machine"], local_machine_name());
    assert_eq!(director["current_run_id"], "/runs/unit");
    assert_eq!(director["status"], "busy");
    assert_eq!(
        director["unknown"]["reason"],
        "no running detached run or open pass start"
    );
    let alpha = entry(&json, "workers", "alpha-id");
    assert_eq!(alpha["parent_staff_id"], "unit-id");
    assert_eq!(alpha["status"], "busy");
    assert_eq!(alpha["activity_started_at"], 300);
    assert_eq!(
        alpha["pass"],
        serde_json::json!({"role": "impl", "phase": "8"})
    );
    let painted = painted(&directory);
    for (list, staff_id) in [
        ("directors", "unit-id"),
        ("workers", "alpha-id"),
        ("workers", "beta-id"),
    ] {
        let reported = entry(&json, list, staff_id);
        let name = reported["name"].as_str().expect("name");
        let line = painted
            .iter()
            .find(|line| line.contains(name))
            .map_or("", String::as_str);
        assert!(line.contains(name), "{name} is drawn: {painted:#?}");
        for field in ["status", "activity"] {
            if let Some(text) = reported[field].as_str() {
                assert!(line.contains(text), "{name} {field} {text}: {line}");
            }
        }
    }
}

#[test]
fn each_status_value_draws_in_the_status_cell_and_starting_and_waiting_draw_busy() {
    let directory = tempfile::tempdir().expect("fixture directory");
    for (state, word) in [
        ("busy", "busy"),
        ("idle", "idle"),
        ("paused", "paused"),
        ("exited", "exited"),
        ("starting", "busy"),
        ("waiting", "busy"),
    ] {
        let accounts = format!(
            r#"{{"accounts":[{{"label":"claude1","roles":[],"use_credits":false,
            "sessions":[{{"session":"alpha","state":"{state}"}}]}}],"problems":[]}}"#
        );
        let json = report_with(
            config_with_accounts(&directory, &accounts),
            rows_with("thinking"),
            facts(),
        );
        assert_eq!(session(&json, "alpha")["status"], word, "{state}");
        let painted = painted_with(&directory, rows_with("thinking"));
        let line = line_of(&painted, "alpha-name");
        assert!(line.contains(word), "{state}: {line}");
        for retired in ["starting", "waiting"] {
            assert!(!line.contains(retired), "{state} draws {retired}: {line}");
        }
    }
}

#[test]
fn each_activity_word_comes_from_the_census_into_the_tile_and_the_sessions_list() {
    let directory = tempfile::tempdir().expect("fixture directory");
    for word in ACTIVITY_WORDS {
        let json = report_with(config(&directory, true), rows_with(word), facts());
        let alpha = session(&json, "alpha");
        assert_eq!(alpha["status"], "busy");
        let activity = alpha["activity"].as_str().expect("busy activity");
        assert!(activity.ends_with(word), "{word}: {activity}");
        assert_eq!(entry(&json, "workers", "alpha-id")["activity"], activity);
        let painted = painted_with(&directory, rows_with(word));
        let line = line_of(&painted, "alpha-name");
        assert!(line.contains(activity), "{word}: {line}");
    }
}

#[test]
fn sessions_list_each_tile_session_with_a_null_activity_when_not_busy() {
    let directory = tempfile::tempdir().expect("fixture directory");
    let json = report(config(&directory, true), facts());
    assert_eq!(json["schema"], 2);
    let ids: Vec<&str> = json["sessions"]
        .as_array()
        .expect("sessions")
        .iter()
        .map(|entry| entry["session"].as_str().expect("session id"))
        .collect();
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    assert_eq!(ids, sorted, "session id order");
    for id in ["alpha", "beta", "boss", "director"] {
        assert!(ids.contains(&id), "{id} in {ids:?}");
        assert_eq!(session(&json, id)["machine"], local_machine_name());
    }
    let boss = session(&json, "boss");
    assert_eq!(boss["status"], "idle");
    assert_eq!(boss["activity"], Value::Null, "an idle row has no activity");
    let beta = session(&json, "beta");
    assert_eq!(beta["status"], Value::Null);
    assert_eq!(beta["activity"], Value::Null);
    let director = session(&json, "director");
    assert_eq!(director["status"], "busy");
    assert_eq!(
        director["activity"],
        entry(&json, "directors", "unit-id")["activity"]
    );
}

#[test]
fn each_run_state_and_pane_value_is_named_and_unknown_gives_its_reason() {
    let directory = tempfile::tempdir().expect("fixture directory");
    let json = report(config(&directory, true), facts());
    let alpha = entry(&json, "workers", "alpha-id");
    assert_eq!(
        alpha["runs"],
        serde_json::json!([
            {"label":"verify test","started_at":300,"state":"running","unknown":null},
            {"label":"verify lint","started_at":200,"state":"finished_queued","unknown":null},
            {"label":"verify doc","started_at":null,"state":"missing",
             "unknown":{"reason":"run has no start time"}}
        ])
    );
    for (pane, expected) in [
        (Observed::Known(PaneState::Open), "open"),
        (Observed::Known(PaneState::Stopped), "stopped"),
        (Observed::Known(PaneState::Closed), "closed"),
        (Observed::unknown("herdr did not answer"), "unknown"),
    ] {
        let json = report(
            config(&directory, true),
            facts_with(|alpha| alpha.pane = pane),
        );
        let alpha = entry(&json, "workers", "alpha-id");
        assert_eq!(alpha["pane"], expected);
        let reason = &alpha["unknown"];
        if expected == "unknown" {
            assert_eq!(reason["reason"], "herdr did not answer");
        } else {
            assert_eq!(*reason, Value::Null);
        }
    }
}

#[test]
fn no_accounts_entry_gives_null_status_and_no_run_gives_empty_runs() {
    let directory = tempfile::tempdir().expect("fixture directory");
    let json = report(config(&directory, true), facts());
    let beta = entry(&json, "workers", "beta-id");
    assert_eq!(beta["status"], Value::Null);
    assert_eq!(beta["runs"], serde_json::json!([]));
    assert_eq!(beta["pass"], Value::Null);
    assert_eq!(beta["pane"], "stopped");
    let unread = report(config(&directory, false), facts());
    for (list, staff_id) in [
        ("directors", "unit-id"),
        ("workers", "alpha-id"),
        ("workers", "beta-id"),
    ] {
        assert_eq!(entry(&unread, list, staff_id)["status"], Value::Null);
    }
}

#[test]
fn only_current_run_workers_and_unit_directors_are_listed() {
    let directory = tempfile::tempdir().expect("fixture directory");
    let json = report(config(&directory, true), facts());
    let ids = |list: &str| -> Vec<String> {
        json[list]
            .as_array()
            .expect("list")
            .iter()
            .map(|entry| entry["staff_id"].as_str().expect("staff id").to_owned())
            .collect()
    };
    assert_eq!(ids("directors"), ["unit-id"]);
    assert_eq!(ids("workers"), ["alpha-id", "beta-id"]);
}

#[test]
fn source_failures_exit_zero_with_the_machine_error_and_a_bad_config_fails() {
    let directory = tempfile::tempdir().expect("fixture directory");
    let mut remote = config(&directory, true);
    remote.machines.remote = vec!["far".to_owned()];
    let json = report(remote, facts());
    let unreachable = MachineReport::new("far", &remote::machine_state(ProbeOutcome::TimedOut));
    assert_eq!(
        json["machines"][1],
        serde_json::to_value(unreachable).expect("machine JSON")
    );
    assert_eq!(json["machines"][0]["reachable"], true);

    let mut unreadable = facts();
    unreadable.staff = Observed::Known(SourceHealth::Unreadable(UnknownReason(
        "staff: invalid JSON".to_owned(),
    )));
    let json = report(config(&directory, true), unreadable);
    assert_eq!(
        json["machines"][0],
        serde_json::json!({"name":local_machine_name(),"reachable":true,"error":"staff: invalid JSON"})
    );

    let error = toml::from_str::<Config>("[census]\nherdr_read_timeout = -1\n")
        .expect_err("a negative timeout is refused")
        .to_string();
    let (status, json, stderr) = print_census(
        &LoadedConfig {
            config: Config::default(),
            error:  Some(ConfigError::Parse(error.clone())),
        },
        facts(),
    );
    assert_eq!(status, ExitCode::FAILURE);
    assert_eq!(json, Value::Null);
    assert!(stderr.contains(&error), "{stderr}");
}

#[test]
fn census_json_matches_the_readme_example_keys_and_types() {
    let block = README
        .split_once(EXAMPLE_MARKER)
        .and_then(|(_, rest)| rest.split_once("```json\n"))
        .and_then(|(_, rest)| rest.split_once("```"))
        .map(|(block, _)| block)
        .expect("README census example");
    let example: Value = serde_json::from_str(block).expect("README example JSON");
    let directory = tempfile::tempdir().expect("fixture directory");
    let mut config = config(&directory, true);
    config.machines.remote = vec!["far".to_owned()];
    same_shape(&example, &report(config, facts()), "$");
}

/// Each object has the example's keys, and each value has the example's type or null.
fn same_shape(example: &Value, actual: &Value, at: &str) {
    match (example, actual) {
        (Value::Object(expected), Value::Object(object)) => {
            assert_eq!(
                expected.keys().collect::<Vec<_>>(),
                object.keys().collect::<Vec<_>>(),
                "keys at {at}"
            );
            for (key, value) in expected {
                same_shape(value, &object[key], &format!("{at}.{key}"));
            }
        },
        (Value::Array(expected), Value::Array(items)) => {
            if let Some(first) = expected.first() {
                for (index, item) in items.iter().enumerate() {
                    same_shape(first, item, &format!("{at}[{index}]"));
                }
            }
        },
        (expected, actual) => {
            if !expected.is_null() && !actual.is_null() {
                assert_eq!(kind(expected), kind(actual), "type at {at}");
            }
        },
    }
}

const fn kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}
