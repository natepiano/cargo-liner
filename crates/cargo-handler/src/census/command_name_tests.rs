//! Unnamed command rows display the executable after environment assignments.

#![expect(clippy::expect_used, reason = "process and agent fixtures must parse")]

use super::account::ConfigHome;
use super::classify::ProcessEntry;
use super::tree::TreeSources;
use super::*;

#[test]
fn command_rows_skip_leading_assignments_and_keep_assignment_only_commands() {
    for (command, expected) in [
        ("P=/tmp/x cargo test", "cargo test"),
        ("A='a b' B=2 make", "make"),
        ("P=/tmp/x", "P=/tmp/x"),
    ] {
        assert_eq!(command_row_name(command), expected, "{command}");
    }
}

fn command_row_name(command: &str) -> String {
    let mut rows: Vec<AgentRow> = serde_json::from_value(serde_json::json!([{
        "agent":"claude","name":"director","pid":10,"started":10,"directory":"/fixture","children":[]
    }])).expect("agent fixture");
    let process = |pid, parent, name: &str, args: Vec<String>| ProcessEntry {
        pid,
        parent,
        name: name.into(),
        arguments: args,
        started: 10,
        directory: None,
        claude_pid: None,
        config_home: ConfigHome::Unread,
    };
    let quoted = command.replace('\'', "'\\''");
    let script = format!("source /fixture/shell-snapshots/snapshot-fixture; eval '{quoted}'");
    let processes = [
        process(10, None, "claude", vec!["claude".into()]),
        process(
            20,
            Some(10),
            "bash",
            vec!["bash".into(), "-c".into(), script],
        ),
    ];
    tree::attach_children(&mut rows, &processes, &TreeSources::default());
    assert_eq!(rows[0].children.len(), 1);
    rows.remove(0).children.remove(0).name
}

#[test]
fn command_rows_skip_command_substitution_assignments() {
    assert_eq!(
        command_row_name("P=$(git rev-parse --show-toplevel) cargo test"),
        "cargo test"
    );
}

#[test]
fn command_rows_skip_nested_command_substitution_assignments() {
    for command in [
        "P=$(dirname $(git rev-parse --show-toplevel)) cargo test",
        "P=\"$(printf '%s' \"$(pwd)\")\" cargo test",
    ] {
        assert_eq!(command_row_name(command), "cargo test", "{command}");
    }
}

#[test]
fn command_rows_skip_backtick_substitution_assignments() {
    assert_eq!(
        command_row_name("P=`git rev-parse --show-toplevel` cargo test"),
        "cargo test"
    );
}

#[test]
fn assignment_only_command_substitutions_keep_the_original_command() {
    for command in [
        "P=$(git rev-parse --show-toplevel)",
        "P=$(dirname $(pwd))",
        "P=`git rev-parse --show-toplevel`",
    ] {
        assert_eq!(command_row_name(command), command);
    }
}
