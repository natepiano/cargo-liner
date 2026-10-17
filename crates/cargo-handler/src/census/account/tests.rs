//! Account discovery and schema-3 compatibility.

#![expect(
    clippy::expect_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]

use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::path::Path;
use std::path::PathBuf;

use super::Account;
use super::AccountColumn;
use super::AccountHome;
use super::AccountLabels;
use super::ConfigHome;
use super::Program;
use super::account_of;
use super::config_home;
use crate::census::Agent;
use crate::census::AgentActivity;
use crate::census::AgentRow;
use crate::census::ChildKind;
use crate::census::ChildRow;
use crate::census::CodexThreadId;
use crate::census::ServiceTier;
use crate::census::SessionIdentity;
use crate::census::classify::ProcessEntry;
use crate::census::probe;
use crate::census::worker_dispatch::WorkerDispatch;
use crate::census::worker_dispatch::WorkerTask;
use crate::config::AccountsConfig;

/// The machine home used by account fixtures, independent of the test runner.
const MACHINE_HOME: &str = "/home/u";

/// Labels as they would be read from the optional accounts table.
fn labels(pairs: &[(&str, &str)]) -> AccountLabels {
    AccountLabels::new(
        &AccountsConfig(
            pairs
                .iter()
                .map(|(home, label)| ((*home).to_string(), (*label).to_string()))
                .collect(),
        ),
        Some(Path::new(MACHINE_HOME)),
    )
}

/// A process whose environment has already been read or marked unread.
fn process(program: &str, config_home: ConfigHome) -> ProcessEntry {
    ProcessEntry {
        pid: 42,
        parent: Some(1),
        name: program.to_string(),
        arguments: vec![program.to_string()],
        started: 100,
        directory: None,
        claude_pid: None,
        config_home,
    }
}

/// Require a discovered home rather than silently accepting an unread account.
fn recorded(account: &Account) -> &AccountHome {
    match account {
        Account::Home(home) => home,
        Account::Unrecorded => panic!("expected a recorded account home"),
    }
}

/// Resolve an account through the same constructor used for scanned rows.
fn account(home: &str) -> Account {
    Account::Home(AccountLabels::default().resolve(Path::new(home), Some(Path::new(MACHINE_HOME))))
}

#[test]
fn config_home_finds_the_claude_variable_among_other_environment_entries() {
    let environment = [
        OsString::from("HOME=/home/u"),
        OsString::from("CLAUDE_CONFIG_DIR=/home/u/.claude1"),
        OsString::from("CODEX_HOME=/home/u/.codex2"),
        OsString::from("TERM=xterm-256color"),
    ];

    assert_eq!(
        config_home(&environment, Program::Claude),
        ConfigHome::Named(PathBuf::from("/home/u/.claude1")),
    );
}

#[test]
fn config_home_uses_the_default_for_unset_empty_or_inexact_claude_variables() {
    for entries in [
        Vec::<OsString>::new(),
        vec![OsString::from("HOME=/home/u")],
        vec![OsString::from("CLAUDE_CONFIG_DIR=")],
        vec![OsString::from("CLAUDE_CONFIG_DIRS=x")],
        vec![OsString::from("CODEX_HOME=/home/u/.codex2")],
    ] {
        assert_eq!(
            config_home(&entries, Program::Claude),
            ConfigHome::Default(Program::Claude),
            "{entries:?}"
        );
    }
}

#[test]
fn config_home_reads_codex_home_instead_of_the_claude_variable_for_codex() {
    let environment = [
        OsString::from("CLAUDE_CONFIG_DIR=/home/u/.claude1"),
        OsString::from("CODEX_HOME=/home/u/.codex2"),
    ];
    assert_eq!(
        config_home(&environment, Program::Codex),
        ConfigHome::Named(PathBuf::from("/home/u/.codex2")),
    );

    for entries in [
        vec![OsString::from("CLAUDE_CONFIG_DIR=/home/u/.claude1")],
        vec![OsString::from("CODEX_HOME=")],
        vec![OsString::from("CODEX_HOMES=/home/u/.codex2")],
    ] {
        assert_eq!(
            config_home(&entries, Program::Codex),
            ConfigHome::Default(Program::Codex),
            "{entries:?}"
        );
    }
}

#[test]
fn config_home_keeps_a_non_utf8_codex_home_named() {
    let environment = [OsString::from_vec(b"CODEX_HOME=/tmp/account-\xff".to_vec())];
    let home = config_home(&environment, Program::Codex);
    assert_eq!(
        home,
        ConfigHome::Named(PathBuf::from(OsString::from_vec(
            b"/tmp/account-\xff".to_vec()
        ))),
    );

    let account = account_of(
        &process("codex", home),
        Some(Path::new(MACHINE_HOME)),
        &AccountLabels::default(),
    );
    assert_ne!(recorded(&account).home, "~/.codex");
}

#[test]
fn resolve_abbreviates_the_machine_home_and_removes_one_leading_dot() {
    let home = AccountLabels::default()
        .resolve(Path::new("/home/u/.claude1"), Some(Path::new(MACHINE_HOME)));

    assert_eq!(home.home, "~/.claude1");
    assert_eq!(home.label, "claude1");

    let dotted = AccountLabels::default()
        .resolve(Path::new("/home/u/..claude"), Some(Path::new(MACHINE_HOME)));
    assert_eq!(dotted.label, ".claude");
}

#[test]
fn account_of_defaults_to_the_home_for_its_program() {
    for (name, program, expected_home, expected_label) in [
        ("claude", Program::Claude, "~/.claude", "claude"),
        ("codex", Program::Codex, "~/.codex", "codex"),
    ] {
        let account = account_of(
            &process(name, ConfigHome::Default(program)),
            Some(Path::new(MACHINE_HOME)),
            &AccountLabels::default(),
        );

        assert_eq!(recorded(&account).home, expected_home, "{name}");
        assert_eq!(recorded(&account).label, expected_label, "{name}");
    }
}

#[test]
fn account_of_reads_the_default_home_from_the_program_it_carries() {
    let account = account_of(
        &process("bash", ConfigHome::Default(Program::Codex)),
        Some(Path::new(MACHINE_HOME)),
        &AccountLabels::default(),
    );

    assert_eq!(recorded(&account).home, "~/.codex");
    assert_eq!(recorded(&account).label, "codex");
}

#[test]
fn resolve_uses_the_label_configured_for_an_abbreviated_home() {
    let home = labels(&[("~/.claude", "claude2")])
        .resolve(Path::new("/home/u/.claude"), Some(Path::new(MACHINE_HOME)));

    assert_eq!(home.home, "~/.claude");
    assert_eq!(home.label, "claude2");
}

#[test]
fn resolve_matches_an_absolute_config_key_to_the_abbreviated_home() {
    let home = labels(&[("/home/u/.codex", "codex2")])
        .resolve(Path::new("/home/u/.codex"), Some(Path::new(MACHINE_HOME)));

    assert_eq!(home.home, "~/.codex");
    assert_eq!(home.label, "codex2");
}

#[test]
fn resolve_keeps_a_home_outside_the_machine_home_absolute() {
    let home = AccountLabels::default().resolve(
        Path::new("/home/other/.codex7"),
        Some(Path::new(MACHINE_HOME)),
    );

    assert_eq!(home.home, "/home/other/.codex7");
    assert_eq!(home.label, "codex7");
}

#[test]
fn account_of_an_unread_environment_stays_unrecorded() {
    let account = account_of(
        &process("claude", ConfigHome::Unread),
        Some(Path::new(MACHINE_HOME)),
        &labels(&[("~/.claude", "claude2")]),
    );

    assert_eq!(account, Account::Unrecorded);
}

#[test]
fn account_of_a_named_environment_resolves_its_configured_label() {
    let account = account_of(
        &process("codex", ConfigHome::Named(PathBuf::from("/home/u/.codex3"))),
        Some(Path::new(MACHINE_HOME)),
        &labels(&[("~/.codex3", "work")]),
    );

    assert_eq!(recorded(&account).home, "~/.codex3");
    assert_eq!(recorded(&account).label, "work");
}

#[test]
fn account_column_is_hidden_for_one_claude_label() {
    assert_eq!(
        AccountColumn::of([(Agent::Claude, &account("/home/u/.claude1"))]),
        AccountColumn::Hidden,
    );
}

#[test]
fn account_column_is_shown_for_two_claude_labels() {
    assert_eq!(
        AccountColumn::of([
            (Agent::Claude, &account("/home/u/.claude1")),
            (Agent::Claude, &account("/home/u/.claude2")),
        ]),
        AccountColumn::Shown,
    );
}

#[test]
fn account_column_compares_labels_only_within_the_same_program() {
    assert_eq!(
        AccountColumn::of([
            (Agent::Claude, &account("/home/u/.claude2")),
            (Agent::Codex, &account("/home/u/.codex1")),
        ]),
        AccountColumn::Hidden,
    );
}

#[test]
fn account_column_counts_codex_and_thread_hosts_as_the_same_program() {
    assert_eq!(
        AccountColumn::of([
            (Agent::Codex, &account("/home/u/.codex1")),
            (Agent::CodexThreadHost, &account("/home/u/.codex2")),
        ]),
        AccountColumn::Shown,
    );
}

#[test]
fn account_column_ignores_unrecorded_accounts() {
    assert_eq!(
        AccountColumn::of([
            (Agent::Claude, &Account::Unrecorded),
            (Agent::Claude, &Account::Unrecorded),
            (Agent::Codex, &Account::Unrecorded),
        ]),
        AccountColumn::Hidden,
    );
    assert_eq!(
        AccountColumn::of([
            (Agent::Claude, &Account::Unrecorded),
            (Agent::Claude, &account("/home/u/.claude1")),
        ]),
        AccountColumn::Hidden,
    );
}

#[test]
fn account_column_counts_the_same_label_on_two_machines_once() {
    let configured = labels(&[("~/.claude", "work"), ("~/.claude7", "work")]);
    let first =
        Account::Home(configured.resolve(Path::new("/home/u/.claude"), Some(Path::new("/home/u"))));
    let second = Account::Home(configured.resolve(
        Path::new("/Users/remote/.claude7"),
        Some(Path::new("/Users/remote")),
    ));

    assert_eq!(
        AccountColumn::of([(Agent::Claude, &first), (Agent::Claude, &second)]),
        AccountColumn::Hidden,
    );
}

/// An agent fixture serialized by the public row type, not a duplicate wire struct.
fn agent_row(account: Account) -> AgentRow {
    AgentRow {
        agent: Agent::Codex,
        session_id: SessionIdentity::Unrecorded,
        service_tier: ServiceTier::Standard,
        account,
        name: "account-wire".to_string(),
        activity: AgentActivity::Unreported,
        started: 100,
        process_started: 101,
        pid: 42,
        desktop: None,
        directory: "~/rust/project".to_string(),
        branch: None,
        launched_by: None,
        children: Vec::new(),
    }
}

/// Decode a real schema-3 line as a remote census would.
fn probe_round_trip(row: &AgentRow) -> Vec<AgentRow> {
    let report = serde_json::json!({"schema": 3, "machine": "test", "rows": [row]});
    let mut line = serde_json::to_vec(&report).expect("report serializes");
    line.push(b'\n');
    probe::parse_report(&line)
        .expect("schema-3 report parses")
        .0
}

#[test]
fn a_home_account_round_trips_on_an_agent_in_a_schema_three_probe() {
    let row = agent_row(account("/home/u/.codex1"));
    let wire = serde_json::to_value(&row).expect("agent serializes");

    assert_eq!(
        wire.get("account"),
        Some(&serde_json::json!({"home": "~/.codex1", "label": "codex1"})),
    );
    assert_eq!(probe_round_trip(&row), vec![row]);
}

#[test]
fn a_schema_three_probe_without_accounts_keeps_them_unrecorded() {
    let line = br#"{"schema":3,"machine":"old","rows":[{"agent":"claude","name":"old-agent","status":"idle","started":100,"pid":42,"directory":"~/rust/project","children":[]}]}
"#;
    let (rows, _) = probe::parse_report(line).expect("pre-account schema-3 probe parses");
    let row = rows.first().expect("old report keeps its agent");

    assert_eq!(rows.len(), 1);
    assert_eq!(row.name, "old-agent");
    assert_eq!(row.account, Account::Unrecorded);
}

/// A child with enough identity to check that account transport preserves its row.
fn child_row(kind: ChildKind, account: Account) -> ChildRow {
    ChildRow {
        worker_task: WorkerTask::Unrecorded,
        thread_id: CodexThreadId::Unrecorded,
        dispatch: WorkerDispatch::Plain,
        activity: AgentActivity::Idle,
        depth: 0,
        kind,
        service_tier: ServiceTier::Unrecorded,
        account,
        pid: None,
        name: "child-account".to_string(),
        started: 102,
        timer: None,
    }
}

#[test]
fn an_unrecorded_child_omits_its_account_on_the_wire() {
    let child = child_row(ChildKind::Shell, Account::Unrecorded);
    let wire = serde_json::to_value(&child).expect("child serializes");

    assert!(
        !wire
            .as_object()
            .expect("child is an object")
            .contains_key("account")
    );
    let decoded: ChildRow = serde_json::from_value(wire).expect("child without account parses");
    assert_eq!(decoded, child);
}

#[test]
fn a_thread_child_transports_its_home_through_the_probe() {
    let mut row = agent_row(account("/home/u/.codex2"));
    let mut thread = child_row(ChildKind::Thread, account("/home/u/.codex2"));
    thread.thread_id = CodexThreadId::Recorded("thread-123".to_string());
    thread.service_tier = ServiceTier::Fast;
    let wire = serde_json::to_value(&thread).expect("thread serializes");
    assert_eq!(
        wire.get("account"),
        Some(&serde_json::json!({"home": "~/.codex2", "label": "codex2"})),
    );
    row.children.push(thread);

    assert_eq!(probe_round_trip(&row), vec![row]);
}
