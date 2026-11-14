//! A shell row's command draws each path argument as its file name.

use std::ops::Range;

use super::*;
use crate::census::ShellTimerObservation;
use crate::shell_command;

#[test]
fn a_path_argument_draws_as_its_file_name() {
    assert_eq!(
        shell_command::short_command(
            "python3 /tmp/claude/session/abc/scratchpad/watchgates.py 2>&1"
        ),
        "python3 watchgates.py 2>&1"
    );
    assert_eq!(
        shell_command::short_command(
            "cargo nextest run -p hana_video --no-fail-fast -- permission_queue"
        ),
        "cargo nextest run -p hana_video --no-fail-fast -- permission_queue"
    );
}

#[test]
fn quoted_relative_and_redirected_paths_cut_and_other_arguments_stay() {
    let cases = [
        (
            r#"bash "/tmp/my dir/run tests.sh" --fast"#,
            r#"bash "run tests.sh" --fast"#,
        ),
        ("bash './scripts/go.sh'", "bash 'go.sh'"),
        (
            "python3 scripts/check.py ../data/",
            "python3 check.py data/",
        ),
        (
            "cmd >/tmp/out.log 2>>/var/log/err.log <~/input.txt",
            "cmd >out.log 2>>err.log <input.txt",
        ),
        (
            "tool --config=/etc/tool/a.toml -v",
            "tool --config=a.toml -v",
        ),
        (
            "sed 's/a/b/' file  |  grep x",
            "sed 's/a/b/' file  |  grep x",
        ),
        (
            "curl https://example.com/a/b.json",
            "curl https://example.com/a/b.json",
        ),
        ("echo \"hello world\" /", "echo \"hello world\" /"),
    ];
    for (command, short) in cases {
        assert_eq!(
            shell_command::short_command(command),
            short,
            "for {command}"
        );
    }
}

const LONG_SHELL: &str = "python3 /tmp/claude/session/abc/scratchpad/a-long-watch-script-with-a-very-long-name.py --interval 30 --until-ready 2>&1";

/// An owner with a worker and one shell command; the worker row sets the
/// session width, and the short command keeps the type column in both tiles.
fn owner_with_shell(command: &str) -> AgentRow {
    let mut owner = agent(10, "owner-session", HOUR, None);
    owner.directory = "/fixture".into();
    owner
        .children
        .push(child(0, ChildKind::Thread, None, "worker-session", MINUTE));
    owner.children.push(child(
        0,
        ChildKind::Shell(ShellTimerObservation::Untimed),
        Some(11),
        command,
        MINUTE,
    ));
    owner
}

fn session_span(cell: &AgentEntry<'_>, width: u16) -> Range<u16> {
    header_spans(cell, area(width, 12), NOW, AccountColumn::Shown)
        .into_iter()
        .find(|span| span.column == TreeColumn::Session)
        .expect("the session column")
        .columns
}

#[test]
fn a_long_shell_row_takes_the_agent_width_and_ends_in_a_mark_at_the_column_edge() {
    let with_shell = owner_with_shell(LONG_SHELL);
    let short_shell = owner_with_shell("sleep 5");
    let cell = entry(&with_shell, None);
    let plain = entry(&short_shell, None);
    let width = 70;
    let span = session_span(&cell, width);
    assert_eq!(
        span,
        session_span(&plain, width),
        "the shell row adds no width"
    );
    let buffer = drawn_into(&cell, &[], width, 12);
    let (_, y) = find(&buffer, "python3").expect("the shell row is drawn");
    let (role, _) = find(&buffer, "role").expect("the role header");
    // The column gap separates the session column's last cell from the role column.
    assert_eq!(
        buffer[(role - TABLE_COLUMN_SPACING - 1, y)].symbol(),
        TRUNCATION_MARK.to_string(),
        "{:?}",
        lines(&buffer)
    );
    assert!(
        lines(&buffer)[usize::from(y)].contains("python3 a-long-watch"),
        "the path argument draws as its file name: {:?}",
        lines(&buffer)
    );
}

#[test]
fn a_long_shell_row_leaves_its_neighbor_tile_widths_unchanged() {
    let with_shell = owner_with_shell(LONG_SHELL);
    let short_shell = owner_with_shell("sleep 5");
    let neighbor = agent(20, "neighbor-session", HOUR, None);
    let shared = [entry(&with_shell, None), entry(&neighbor, None)];
    let baseline = [entry(&short_shell, None), entry(&neighbor, None)];
    let width = 160;
    let drawn = drawn_into(&shared[1], &shared, width, 12);
    let expected = drawn_into(&baseline[1], &baseline, width, 12);
    assert_eq!(lines(&drawn), lines(&expected));
}
