//! Dispatch identities come from fixture rosters and fold only inside one tile.

#![expect(
    clippy::expect_used,
    reason = "fixture files and rendered rows must exist"
)]
#![expect(clippy::panic, reason = "tests should panic on unexpected values")]

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use serde_json::Value;
use tempfile::TempDir;

use super::*;
use crate::agent_cell;
use crate::agent_cell::TreeColumn;
use crate::census::AccountColumn;
use crate::census::Agent;
use crate::census::AgentPlacement;
use crate::census::AgentRow;
use crate::census::ChildRow;
use crate::census::CodexThreadId;
use crate::census::HierarchyEdge;
use crate::census::HierarchyEdges;
use crate::census::HierarchyRole;
use crate::census::Machine;
use crate::census::MachineState;
use crate::census::SessionIdentity;
use crate::census::account::ConfigHome;
use crate::census::classify::ProcessEntry;
use crate::census::scan;
use crate::footer_colors::FooterColors;
use crate::sort::SortDirection;
use crate::sort::TreeSort;
use crate::theme::RainbowHue;
use crate::theme::Role;

const JOB: &str = "fix the slow saved-scene handover";
const SEAT: &str = "cargo-liner-handler-enh-impl7";

/// A real roster file in an isolated session directory.
fn roster_fixture() -> TempDir {
    let directory = TempDir::new().expect("temporary session directory");
    fs::write(
        directory.path().join("mesh_roster.json"),
        r#"{
        "cargo-liner-handler-enh-impl7": {"thread_id":"worker-thread", "launcher_pid":701},
        "cargo-liner-handler-enh-test2": {"thread_id":"other-thread", "launcher_pid":702}
    }"#,
    )
    .expect("fixture roster writes");
    directory
}

/// The launcher accepts optional flags before its positional arguments.
fn command(directory: &Path, flags: &str) -> String {
    format!(
        "bash ~/.claude/scripts/delegate/implement.sh {flags} '{}' /fixture/cargo-liner-handler-enh /fixture/prompt.md impl '{JOB}' impl 'implement worker' 0 impl",
        directory.display()
    )
}

#[test]
fn explicit_dispatch_seat_resolves_from_the_fixture_roster() {
    let directory = roster_fixture();
    let parsed = ParsedDispatch::parse(&command(
        directory.path(),
        &format!("--phase 1 --to {SEAT}"),
    ))
    .expect("literal dispatch parses");
    let roster = WorkerRoster::read(&parsed.session_dir);
    let WorkerDispatch::Worker(dispatch) = parsed.resolve(&roster, 999) else {
        panic!("an explicit seat resolves independently of the launcher pid");
    };
    assert_eq!(dispatch.seat, SEAT);
    assert_eq!(dispatch.thread_id, "worker-thread");
    assert_eq!(dispatch.label(), format!("impl7 · {JOB}"));
}

#[test]
fn worker_job_headings_are_clean_in_role_roster_and_folded_names() {
    let base = Path::new("/tmp/claude/unit");
    fs::create_dir_all(base).expect("session root exists");
    let directory = tempfile::Builder::new()
        .prefix("handler-job-heading-")
        .tempdir_in(base)
        .expect("isolated worker session");
    fs::write(
        directory.path().join(ROSTER_FILE),
        serde_json::json!({SEAT:{"thread_id":"worker-thread", "launcher_pid":701}}).to_string(),
    )
    .expect("worker roster writes");
    let mut names = WorkerNames::default();
    let worktree = ThreadWorktree::Recorded("/fixture/cargo-liner-handler-enh".into());
    for (heading, job) in [
        (
            "# Phase 6 repair round 1 — slot impl",
            "Phase 6 repair round 1 — slot impl",
        ),
        ("  ###   Repair the worker  ", "Repair the worker"),
        ("Repair the worker", "Repair the worker"),
        ("###   ", ""),
    ] {
        let prompt = format!("{heading}\nBoard: {}/board.log", directory.path().display());
        let role = WorkerRoleSource::Hierarchy(HierarchyRole::Fix);
        let expected_role = if job.is_empty() {
            "fix".to_owned()
        } else {
            format!("fix · {job}")
        };
        let expected_seat = if job.is_empty() {
            "impl7".to_owned()
        } else {
            format!("impl7 · {job}")
        };
        let task = WorkerTask::from_prompt(&prompt);
        assert_eq!(task.label(role, "worker-thread"), expected_role);
        if job.is_empty() {
            assert_eq!(task, WorkerTask::RoleOnly);
        }
        assert_eq!(
            names.name("other-thread", &prompt, &worktree, role),
            expected_role
        );
        assert_eq!(
            names.name("worker-thread", &prompt, &worktree, role),
            expected_seat
        );

        let literal = command(directory.path(), &format!("--to {SEAT}")).replace(JOB, heading);
        let parsed = ParsedDispatch::parse(&literal).expect("heading job parses");
        let WorkerDispatch::Worker(dispatch) =
            parsed.resolve(&WorkerRoster::read(directory.path()), 701)
        else {
            panic!("the heading dispatch resolves its worker");
        };
        assert_eq!(dispatch.label(), expected_seat);
        let mut shell = child("shell", "heading dispatch shell");
        shell.dispatch = WorkerDispatch::Worker(dispatch);
        let mut thread = child("thread", "original heading worker");
        thread.thread_id = CodexThreadId::Recorded("worker-thread".into());
        thread.pid = None;
        let text = drawn_tiles(vec![agent(10, &[shell, thread])]).join("\n");
        assert!(text.contains(&expected_seat), "{text}");
        assert!(!text.contains('#'), "{text}");
        assert!(!text.contains("heading dispatch shell"), "{text}");
        assert!(!text.contains("original heading worker"), "{text}");
    }
}

#[test]
fn dispatch_without_to_selects_the_seat_by_its_launcher_pid() {
    let directory = roster_fixture();
    let parsed = ParsedDispatch::parse(&command(directory.path(), "--phase 1"))
        .expect("new dispatch parses");
    let roster = WorkerRoster::read(&parsed.session_dir);
    let WorkerDispatch::Worker(dispatch) = parsed.resolve(&roster, 701) else {
        panic!("the matching launcher pid resolves its worker");
    };
    assert_eq!(dispatch.seat, SEAT);
    assert_eq!(dispatch.thread_id, "worker-thread");
    assert_eq!(dispatch.label(), format!("impl7 · {JOB}"));
    assert_eq!(parsed.resolve(&roster, 999), WorkerDispatch::Plain);
}

#[test]
fn ordinary_or_ambiguous_commands_do_not_become_worker_dispatches() {
    for command in [
        "bash build.sh --to cargo-liner-handler-enh-impl7",
        "bash ~/.claude/scripts/delegate/implement.sh --to missing",
        "bash ~/.claude/scripts/delegate/implement.sh /session /work /prompt impl 'job' && echo done",
    ] {
        assert!(ParsedDispatch::parse(command).is_err(), "{command}");
    }
}

#[test]
fn command_continuations_keep_the_shell_row_beside_its_worker() {
    let directory = roster_fixture();
    for assignments in ["", "PLAN_DELEGATE_RESOLVES_ROUND=1 "] {
        let literal = format!(
            "{assignments}{}",
            command(directory.path(), &format!("--phase 1 --to {SEAT}"))
        );
        for continuation in [
            "\nsleep 600",
            "; sleep 600",
            " && sleep 600",
            " || sleep 600",
            " | sleep 600",
            " & sleep 600",
            " mesh-prefix unexpected-argument",
        ] {
            let compound = format!("{literal}{continuation}");
            assert!(ParsedDispatch::parse(&compound).is_err(), "{compound}");
            let quoted = compound.replace('\'', "'\\''");
            let wrapper = process(
                600,
                10,
                "zsh",
                vec![
                    "zsh".to_string(),
                    "-c".to_string(),
                    format!(
                        "source /fixture/.claude/shell-snapshots/snapshot-zsh-fixture.sh && eval '{quoted}'"
                    ),
                ],
            );
            let mut arguments = launcher_arguments(directory.path());
            arguments.splice(2..2, ["--to".to_string(), SEAT.to_string()]);
            let launcher = process(701, 600, "bash", arguments);
            let after_launcher_exit = scan::find_worker_dispatches(std::slice::from_ref(&wrapper));
            assert!(!after_launcher_exit.contains_key(&600), "{compound}");
            let dispatches = scan::find_worker_dispatches(&[wrapper, launcher]);
            assert!(!dispatches.contains_key(&600), "{compound}");
            let mut shell = child("shell", "keep the continuing shell");
            shell.pid = Some(600);
            shell.dispatch = dispatches
                .get(&600)
                .cloned()
                .unwrap_or(WorkerDispatch::Plain);
            let mut thread = child("thread", "original worker title");
            thread.thread_id = CodexThreadId::Recorded("worker-thread".to_string());
            thread.pid = None;
            let text = drawn_tiles(vec![agent(10, &[shell, thread])]).join("\n");
            assert!(text.contains("keep the continuing shell"), "{compound}");
            assert!(text.contains("original worker title"), "{compound}");
            assert!(!text.contains(&format!("impl7 · {JOB}")), "{compound}");
        }
    }
}

#[test]
fn leading_literal_assignments_fold_the_shell_into_its_worker() {
    let directory = roster_fixture();
    let literal = command(directory.path(), &format!("--phase 1 --to {SEAT}"));
    for assignments in [
        "PLAN_DELEGATE_RESOLVES_ROUND=1",
        "PLAN_DELEGATE_RESOLVES_ROUND=0 OTHER_VALUE=literal",
        "PLAN_DELEGATE_RESOLVES_ROUND=1 OTHER_VALUE='literal with spaces'",
        "_VALUE=\"literal with spaces=and equals\" EMPTY=",
    ] {
        let assigned = format!("{assignments} {literal}");
        assert_eq!(
            ParsedDispatch::parse(&assigned),
            ParsedDispatch::parse(&literal),
            "{assigned}"
        );
        let quoted = assigned.replace('\'', "'\\''");
        let wrapper = process(
            600,
            10,
            "zsh",
            vec![
                "zsh".to_string(),
                "-c".to_string(),
                format!(
                    "source /fixture/.claude/shell-snapshots/snapshot-zsh-fixture.sh && eval '{quoted}'"
                ),
            ],
        );
        let mut arguments = launcher_arguments(directory.path());
        arguments.splice(2..2, ["--to".to_string(), SEAT.to_string()]);
        let launcher = process(701, 600, "bash", arguments);
        let dispatches = scan::find_worker_dispatches(&[wrapper.clone(), launcher]);
        // An explicit seat still resolves while the shell is the only process observed.
        assert_eq!(
            scan::find_worker_dispatches(&[wrapper]).get(&600),
            dispatches.get(&600),
            "{assigned}"
        );
        let mut shell = child("shell", "assignment dispatch shell");
        shell.pid = Some(600);
        shell.dispatch = dispatches
            .get(&600)
            .expect("literal assignments match actual launcher argv")
            .clone();
        let mut thread = child("thread", "original worker title");
        thread.thread_id = CodexThreadId::Recorded("worker-thread".to_string());
        thread.pid = None;
        let text = drawn_tiles(vec![agent(10, &[shell, thread])]).join("\n");
        assert!(text.contains(&format!("impl7 · {JOB}")), "{assigned}");
        assert!(!text.contains("assignment dispatch shell"), "{assigned}");
        assert!(!text.contains("original worker title"), "{assigned}");
    }
}

#[test]
fn job_arguments_with_equals_fold_with_or_without_leading_assignments() {
    let directory = roster_fixture();
    let literal = command(directory.path(), &format!("--phase 1 --to {SEAT}"));
    for assignments in ["", "PLAN_DELEGATE_RESOLVES_ROUND=1 "] {
        for (job_word, job) in [("'cap=3 retries'", "cap=3 retries"), ("x=y", "x=y")] {
            let literal = format!(
                "{assignments}{}",
                literal.replace(&format!("'{JOB}'"), job_word)
            );
            let quoted = literal.replace('\'', "'\\''");
            let wrapper = process(
                600,
                10,
                "zsh",
                vec![
                    "zsh".to_string(),
                    "-c".to_string(),
                    format!(
                        "source /fixture/.claude/shell-snapshots/snapshot-zsh-fixture.sh && eval '{quoted}'"
                    ),
                ],
            );
            let mut arguments = launcher_arguments(directory.path());
            arguments[8] = job.to_string();
            arguments.splice(2..2, ["--to".to_string(), SEAT.to_string()]);
            assert_eq!(
                ParsedDispatch::parse(&literal),
                ParsedDispatch::from_arguments(&arguments),
                "literal and process argv must identify the same dispatch: {literal}"
            );
            let launcher = process(701, 600, "bash", arguments);
            let dispatches = scan::find_worker_dispatches(&[wrapper.clone(), launcher]);
            assert_eq!(
                scan::find_worker_dispatches(&[wrapper]).get(&600),
                dispatches.get(&600),
                "an explicit seat resolves without a launcher process: {literal}"
            );
            let mut shell = child("shell", "equals job dispatch shell");
            shell.pid = Some(600);
            shell.dispatch = dispatches
                .get(&600)
                .expect("equals in a job argument matches launcher argv")
                .clone();
            let mut thread = child("thread", "original equals worker title");
            thread.thread_id = CodexThreadId::Recorded("worker-thread".to_string());
            thread.pid = None;
            let text = drawn_tiles(vec![agent(10, &[shell, thread])]).join("\n");
            assert!(text.contains(&format!("impl7 · {job}")), "{literal}");
            assert!(!text.contains("equals job dispatch shell"), "{literal}");
            assert!(!text.contains("original equals worker title"), "{literal}");
        }
    }
}

#[test]
fn ansi_c_quoted_live_repair_jobs_fold_with_or_without_assignments() {
    let directory = roster_fixture();
    let job = "Phase 69 repair round 3\nScreen Capture status card fits its text";
    for assignments in ["", "PLAN_DELEGATE_RESOLVES_ROUND=1 "] {
        let literal = format!(
            "{assignments}bash ~/.claude/scripts/delegate/implement.sh --phase 69 --to {SEAT} '{}' /fixture/cargo-liner-handler-enh /fixture/prompt.md fix $'Phase 69 repair round 3\\nScreen Capture status card fits its text' fix \"repair capture status\" 3 test mat",
            directory.path().display()
        );
        let mut arguments = launcher_arguments(directory.path());
        arguments[3] = "69".to_string();
        arguments[7] = "fix".to_string();
        arguments[8] = job.to_string();
        arguments[9] = "fix".to_string();
        arguments[10] = "repair capture status".to_string();
        arguments[11] = "3".to_string();
        arguments[12] = "test".to_string();
        arguments.push("mat".to_string());
        arguments.splice(2..2, ["--to".to_string(), SEAT.to_string()]);
        assert_eq!(
            ParsedDispatch::parse(&literal),
            ParsedDispatch::from_arguments(&arguments),
            "ANSI-C decoding must match the launcher's real newline: {literal}"
        );
        let quoted = literal.replace('\'', "'\\''");
        let wrapper = process(
            600,
            10,
            "zsh",
            vec![
                "zsh".to_string(),
                "-c".to_string(),
                format!(
                    "source /fixture/.claude/shell-snapshots/snapshot-zsh-fixture.sh && eval '{quoted}'"
                ),
            ],
        );
        let launcher = process(701, 600, "bash", arguments);
        let dispatches = scan::find_worker_dispatches(&[wrapper.clone(), launcher]);
        let dispatch = dispatches
            .get(&600)
            .expect("ANSI-C job resolves its worker");
        let WorkerDispatch::Worker(worker) = dispatch else {
            panic!("the ANSI-C launch has a recorded seat");
        };
        assert_eq!(
            worker.label(),
            "impl7 · Phase 69 repair round 3 Screen Capture status card fits its text",
            "the folded label keeps a multi-line job on one line"
        );
        assert_eq!(
            scan::find_worker_dispatches(&[wrapper]).get(&600),
            Some(dispatch),
            "the literal launch also resolves without its launcher process"
        );
        let mut shell = child("shell", "ANSI-C dispatch shell");
        shell.pid = Some(600);
        shell.dispatch = dispatch.clone();
        let mut thread = child("thread", "original ANSI-C worker title");
        thread.thread_id = CodexThreadId::Recorded("worker-thread".to_string());
        thread.pid = None;
        let text = drawn_tiles(vec![agent(10, &[shell, thread])]).join("\n");
        assert!(text.contains("impl7 · Phase 69 repair round 3"), "{text}");
        assert!(!text.contains("ANSI-C dispatch shell"), "{text}");
        assert!(!text.contains("original ANSI-C worker title"), "{text}");
    }
}

#[test]
fn ansi_c_escapes_decode_to_the_literal_launcher_argument() {
    let directory = roster_fixture();
    let literal = command(directory.path(), &format!("--to {SEAT}"));
    for (escaped, decoded) in [
        (
            r#"\n\t\r\\\'\"\a\b\e\f\v\?"#,
            "\n\t\r\\'\"\x07\x08\x1b\x0c\x0b?",
        ),
        (r"\E\cA\c?\c1\c\\\c", "\x1b\x01\x7f\x11\x1c\\c"),
        (r"\1\12\101\1017\501", "\x01\nAA7A"),
        (r"\x4\X\x41f", "\x04\\XAf"),
        (r"\u41\u03bb\U0001f680", "Aλ🚀"),
        (r"\303\251 \xc3\xa9", "é é"),
        (r"\u4Z", "\x04Z"),
        (r"\u\U\x\q", r"\u\U\x\q"),
        (r"before\0ignored", "before"),
        (r"before\x00ignored", "before"),
        (
            "$HOME `date` $(sleep 600) ; && || | &",
            "$HOME `date` $(sleep 600) ; && || | &",
        ),
    ] {
        let command = literal.replace(&format!("'{JOB}'"), &format!("$'{escaped}'"));
        let parsed = ParsedDispatch::parse(&command).expect("literal ANSI-C escape parses");
        assert_eq!(parsed.job, decoded, "{escaped}");
        let mut arguments = launcher_arguments(directory.path());
        arguments[8] = decoded.to_string();
        arguments.splice(2..2, ["--to".to_string(), SEAT.to_string()]);
        assert_eq!(
            Ok(parsed),
            ParsedDispatch::from_arguments(&arguments),
            "{escaped}"
        );
    }
    let concatenated = literal.replace(&format!("'{JOB}'"), r"before$'\0ignored'after$'\n'last");
    assert_eq!(
        ParsedDispatch::parse(&concatenated)
            .expect("quoted segments concatenate")
            .job,
        "beforeafter\nlast"
    );
    let assigned = format!("VALUE=$'line\\nvalue' {literal}");
    assert_eq!(
        ParsedDispatch::parse(&assigned),
        ParsedDispatch::parse(&literal)
    );
}

#[test]
fn malformed_ansi_c_quotes_and_expansions_remain_ordinary_shells() {
    let directory = roster_fixture();
    let literal = command(directory.path(), &format!("--to {SEAT}"));
    for job_word in [
        r"$'unterminated",
        "$HOME",
        "${HOME}",
        "$(sleep 600)",
        r#"$"translated""#,
        r"$'\uD800'",
        r"$'\xFF'",
    ] {
        let command = literal.replace(&format!("'{JOB}'"), job_word);
        assert!(ParsedDispatch::parse(&command).is_err(), "{command}");
    }
    assert!(command_words("$'unterminated").is_err());
    assert!(command_words("$'trailing\\").is_err());
    assert!(ParsedDispatch::parse(&format!("$'A'=1 {literal}")).is_err());
    let ansi_c = literal.replace(&format!("'{JOB}'"), r"$'literal\njob'");
    for continuation in [
        "\nsleep 600",
        "; sleep 600",
        " && sleep 600",
        " || sleep 600",
        " | sleep 600",
        " & sleep 600",
        " mat extra",
    ] {
        let command = format!("{ansi_c}{continuation}");
        assert!(ParsedDispatch::parse(&command).is_err(), "{command}");
    }
}

#[test]
fn invalid_or_misplaced_assignments_do_not_parse_as_dispatches() {
    let directory = roster_fixture();
    let literal = command(directory.path(), &format!("--to {SEAT}"));
    for assignments in [
        "1X=1",
        "BAD-NAME=1",
        "=1",
        "É_VALUE=1",
        "'NAME'=1",
        "\"A\"=1",
        "'NAME=1'",
        "NAME\\=1",
        "NAME=$VALUE",
        "NAME=\"$VALUE\"",
        "NAME=`value`",
    ] {
        let assigned = format!("{assignments} {literal}");
        assert!(ParsedDispatch::parse(&assigned).is_err(), "{assigned}");
    }
    let misplaced = literal.replacen("bash ", "bash NAME=1 ", 1);
    assert!(ParsedDispatch::parse(&misplaced).is_err());
    assert!(
        ParsedDispatch::parse(&format!("{literal} NAME=1")).is_ok(),
        "the optional mesh prefix is an ordinary argument after the command word"
    );
    let mut arguments = launcher_arguments(directory.path());
    arguments.insert(0, "PLAN_DELEGATE_RESOLVES_ROUND=1".to_string());
    assert!(
        ParsedDispatch::from_arguments(&arguments).is_err(),
        "process argv never contains leading environment assignments"
    );
}

#[test]
fn launcher_arguments_end_at_the_team_role_or_optional_mesh_prefix() {
    let directory = roster_fixture();
    let mut arguments = launcher_arguments(directory.path());
    assert!(ParsedDispatch::from_arguments(&arguments).is_ok());
    arguments.push("mesh-prefix".to_string());
    assert!(ParsedDispatch::from_arguments(&arguments).is_ok());
    arguments.push("unexpected-argument".to_string());
    assert!(ParsedDispatch::from_arguments(&arguments).is_err());
    arguments.truncate(arguments.len() - 3);
    assert!(ParsedDispatch::from_arguments(&arguments).is_err());

    let literal = command(directory.path(), &format!("--to {SEAT}"));
    assert!(ParsedDispatch::parse(&format!("{literal} mesh-prefix")).is_ok());
    assert!(ParsedDispatch::parse(&format!("{literal} mesh-prefix extra")).is_err());
}

#[test]
fn quoted_job_newlines_and_escaped_line_breaks_remain_one_invocation() {
    let directory = roster_fixture();
    let literal = command(directory.path(), &format!("--to {SEAT}"));
    let multiline_job = literal.replace(JOB, "fix the handover\nand retain its state");
    let parsed = ParsedDispatch::parse(&multiline_job).expect("quoted job remains one argument");
    assert_eq!(parsed.job, "fix the handover\nand retain its state");
    let escaped_break = literal.replace(" /fixture/prompt.md ", " \\\n/fixture/prompt.md ");
    assert_eq!(
        ParsedDispatch::parse(&escaped_break),
        ParsedDispatch::parse(&literal)
    );
}

/// Probe-compatible children avoid depending on unrelated scan fixture helpers.
fn child(kind: &str, name: &str) -> ChildRow {
    serde_json::from_value(serde_json::json!({
        "kind":kind, "depth":0, "pid":701, "name":name,
        "started":900, "timer":null
    }))
    .expect("child fixture parses")
}

fn agent(pid: u32, children: &[ChildRow]) -> AgentRow {
    serde_json::from_value(serde_json::json!({
        "agent":"claude", "name":format!("root-{pid}"), "status":"busy",
        "started":800, "process_started":800, "pid":pid,
        "desktop":null, "directory":"/fixture", "branch":null,
        "launched_by":null, "children":children
    }))
    .expect("agent fixture parses")
}

/// Draw every normalized tile, so a cross-tile match cannot silently disappear.
fn drawn_tiles(rows: Vec<AgentRow>) -> Vec<String> {
    drawn_tiles_with_edges(rows, HierarchyEdges::default())
}

fn drawn_tiles_with_edges(rows: Vec<AgentRow>, edges: HierarchyEdges) -> Vec<String> {
    let state = MachineState::Answered(rows, edges);
    let placement = AgentPlacement::from_machines(&[Machine {
        name:  "fixture",
        state: &state,
    }]);
    let cells = agent_cell::cell_order_placed(&placement);
    cells
        .iter()
        .map(|cell| {
            let height = u16::try_from(agent_cell::height(cell, 220, 1_000, AccountColumn::Shown))
                .expect("fixture tile height fits u16");
            let area = Rect::new(0, 0, 220, height);
            let mut buffer = Buffer::empty(area);
            agent_cell::draw(
                &mut buffer,
                area,
                cell,
                &cells,
                1_000,
                AccountColumn::Shown,
                &FooterColors::default(),
            );
            (0..area.height)
                .map(|y| {
                    (0..area.width)
                        .map(|x| buffer[(x, y)].symbol())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .collect()
}

#[test]
fn a_dispatch_folds_into_a_managed_thread_moved_from_another_host_row() {
    let directory = roster_fixture();
    let dispatch = ParsedDispatch::parse(&command(directory.path(), &format!("--to {SEAT}")))
        .expect("dispatch parses")
        .resolve(&WorkerRoster::read(directory.path()), 701);
    let mut shell = child("shell", "managed dispatch shell");
    shell.dispatch = dispatch;
    let mut root = agent(10, &[shell]);
    root.session_id = SessionIdentity::Claude("manager-id".to_string());
    let mut thread = child("thread", "host thread title");
    thread.thread_id = CodexThreadId::Recorded("worker-thread".to_string());
    thread.pid = None;
    let mut host = agent(20, &[thread]);
    host.agent = Agent::CodexThreadHost;
    let edges = HierarchyEdges(vec![HierarchyEdge {
        manager: "manager-id".to_string(),
        member:  "worker-thread".to_string(),
        role:    HierarchyRole::Impl,
    }]);
    let tiles = drawn_tiles_with_edges(vec![root, host], edges);
    assert_eq!(
        tiles.len(),
        1,
        "the host's managed thread joins its manager's tile"
    );
    assert!(tiles[0].contains(&format!("impl7 · {JOB}")), "{}", tiles[0]);
    assert!(!tiles[0].contains("managed dispatch shell"));
    assert!(!tiles[0].contains("host thread title"));
}

/// An actual launcher's argv keeps its quoted job as one argument.
fn launcher_arguments(directory: &Path) -> Vec<String> {
    vec![
        "bash".to_string(),
        "/fixture/.claude/scripts/delegate/implement.sh".to_string(),
        "--phase".to_string(),
        "1".to_string(),
        directory.display().to_string(),
        "/fixture/cargo-liner-handler-enh".to_string(),
        "/fixture/prompt.md".to_string(),
        "impl".to_string(),
        JOB.to_string(),
        "impl".to_string(),
        "implement worker".to_string(),
        "0".to_string(),
        "impl".to_string(),
    ]
}

fn process(pid: u32, parent: u32, name: &str, arguments: Vec<String>) -> ProcessEntry {
    ProcessEntry {
        pid,
        parent: Some(parent),
        name: name.to_string(),
        arguments,
        started: 900,
        directory: None,
        claude_pid: None,
        config_home: ConfigHome::Unread,
    }
}

#[test]
fn the_launcher_pid_resolves_the_owning_shell_wrapper_without_to() {
    let directory = roster_fixture();
    let arguments = launcher_arguments(directory.path());
    let parsed = ParsedDispatch::from_arguments(&arguments).expect("actual launcher argv parses");
    let WorkerDispatch::Worker(worker) = parsed.resolve(&WorkerRoster::read(directory.path()), 701)
    else {
        panic!("the actual launcher owns seat impl7");
    };
    assert_eq!(worker.label(), format!("impl7 · {JOB}"));
    // The wrapper is pid 600, while the roster records its child launcher, 701.
    let literal = format!(
        "bash /fixture/.claude/scripts/delegate/implement.sh --phase 1 '{}' /fixture/cargo-liner-handler-enh /fixture/prompt.md impl '{JOB}' impl 'implement worker' 0 impl",
        directory.path().display()
    );
    let quoted = literal.replace('\'', "'\\''");
    let wrapper_script = format!(
        "source /fixture/.claude/shell-snapshots/snapshot-zsh-fixture.sh && eval '{quoted}'"
    );
    let wrapper = process(
        600,
        10,
        "zsh",
        vec!["zsh".to_string(), "-c".to_string(), wrapper_script],
    );
    let launcher = process(701, 600, "bash", arguments);
    let found = scan::find_worker_dispatches(&[wrapper, launcher]);
    let WorkerDispatch::Worker(dispatch) = found
        .get(&600)
        .expect("the wrapper receives dispatch metadata")
    else {
        panic!("the wrapper must resolve using its launcher's pid");
    };
    assert_eq!(dispatch.seat, SEAT);
    assert_eq!(dispatch.thread_id, "worker-thread");
    assert_eq!(dispatch.label(), format!("impl7 · {JOB}"));
}

#[test]
fn probe_rows_round_trip_dispatch_identity_and_accept_older_rows_without_it() {
    let directory = roster_fixture();
    let mut shell = child("shell", "dispatch shell");
    assert_eq!(
        shell.dispatch,
        WorkerDispatch::Plain,
        "older probes omit dispatch metadata"
    );
    let legacy_wire = serde_json::to_value(&shell).expect("legacy child serializes");
    assert!(
        legacy_wire.get("dispatch").is_none(),
        "ordinary shells keep the old probe shape"
    );
    shell.dispatch = ParsedDispatch::parse(&command(directory.path(), &format!("--to {SEAT}")))
        .expect("dispatch parses")
        .resolve(&WorkerRoster::read(directory.path()), 701);
    let encoded = serde_json::to_string(&agent(10, std::slice::from_ref(&shell)))
        .expect("probe row serializes");
    let decoded: AgentRow = serde_json::from_str(&encoded).expect("probe row reads back");
    assert_eq!(decoded.children[0].dispatch, shell.dispatch);
    let WorkerDispatch::Worker(dispatch) = &decoded.children[0].dispatch else {
        panic!("the remote probe retains a resolved worker identity");
    };
    assert_eq!(dispatch.thread_id, "worker-thread");
    assert_eq!(dispatch.label(), format!("impl7 · {JOB}"));
}

#[test]
fn resolved_dispatch_folds_into_its_thread_in_the_same_tile() {
    let directory = roster_fixture();
    let parsed = ParsedDispatch::parse(&command(directory.path(), &format!("--to {SEAT}")))
        .expect("dispatch parses");
    let mut shell = child("shell", "original dispatch shell");
    shell.dispatch = parsed.resolve(&WorkerRoster::read(directory.path()), 701);
    let mut thread = child("thread", "original thread title");
    thread.thread_id = CodexThreadId::Recorded("worker-thread".to_string());
    thread.pid = None;
    let tiles = drawn_tiles(vec![agent(10, &[shell, thread])]);
    assert_eq!(tiles.len(), 1);
    assert!(tiles[0].contains(&format!("impl7 · {JOB}")), "{}", tiles[0]);
    assert!(!tiles[0].contains("original dispatch shell"));
    assert!(!tiles[0].contains("original thread title"));
}

#[test]
fn unresolved_and_cross_tile_dispatches_keep_their_shell_rows() {
    let directory = roster_fixture();
    let roster = WorkerRoster::read(directory.path());
    let missing = ParsedDispatch::parse(&command(directory.path(), "--to missing-seat"))
        .expect("missing seat command still parses");
    assert_eq!(
        missing.resolve(&roster, 701),
        WorkerDispatch::Plain,
        "an explicit unknown seat must not fall back to its launcher's seat"
    );
    let resolved = ParsedDispatch::parse(&command(directory.path(), &format!("--to {SEAT}")))
        .expect("dispatch parses")
        .resolve(&roster, 701);
    for dispatch in [
        WorkerDispatch::Plain,
        missing.resolve(&roster, 701),
        resolved.clone(),
    ] {
        let mut shell = child("shell", "keep this shell row");
        shell.dispatch = dispatch;
        let text = drawn_tiles(vec![agent(10, &[shell])]).join("\n");
        assert!(text.contains("keep this shell row"), "{text}");
    }
    let mut shell = child("shell", "cross tile shell");
    shell.dispatch = resolved;
    let mut thread = child("thread", "other tile thread");
    thread.thread_id = CodexThreadId::Recorded("worker-thread".to_string());
    thread.pid = None;
    let text = drawn_tiles(vec![agent(10, &[shell]), agent(20, &[thread])]).join("\n");
    assert!(text.contains("cross tile shell"));
    assert!(text.contains("other tile thread"));
    assert!(!text.contains(&format!("impl7 · {JOB}")));
}

#[test]
fn roster_roles_override_the_seat_stem_and_missing_roles_use_only_worker_stems() {
    for (short, recorded, expected) in [
        ("impl4", Some("fix"), DispatchRole::Worker(WorkerRole::Fix)),
        (
            "arrange",
            Some("impl"),
            DispatchRole::Worker(WorkerRole::Impl),
        ),
        (
            "arrange",
            Some("test"),
            DispatchRole::Worker(WorkerRole::Test),
        ),
        (
            "arrange",
            Some("review"),
            DispatchRole::Worker(WorkerRole::Review),
        ),
        (
            "arrange",
            Some("fix"),
            DispatchRole::Worker(WorkerRole::Fix),
        ),
        ("impl4", None, DispatchRole::Worker(WorkerRole::Impl)),
        ("test22", None, DispatchRole::Worker(WorkerRole::Test)),
        ("review3", None, DispatchRole::Worker(WorkerRole::Review)),
        ("fix2", None, DispatchRole::Worker(WorkerRole::Fix)),
        ("impl", None, DispatchRole::Worker(WorkerRole::Impl)),
        ("arrange", None, DispatchRole::Unroled),
        ("impl4-extra", None, DispatchRole::Worker(WorkerRole::Impl)),
        ("mat-test", None, DispatchRole::Worker(WorkerRole::Test)),
        ("mat_test3", None, DispatchRole::Worker(WorkerRole::Test)),
        (
            "prefix_fix2_later-impl4",
            None,
            DispatchRole::Worker(WorkerRole::Fix),
        ),
        ("test_impl", None, DispatchRole::Worker(WorkerRole::Test)),
        (
            "mat-test",
            Some("review"),
            DispatchRole::Worker(WorkerRole::Review),
        ),
        ("myimpl4", None, DispatchRole::Unroled),
    ] {
        let seat = format!("cargo-liner-handler-enh-{short}");
        let roster = WorkerRoster::parse(
            &serde_json::json!({
                seat.clone(): {"thread_id":"worker-thread", "launcher_pid":701, "role":recorded}
            })
            .to_string(),
        );
        let parsed = ParsedDispatch::parse(&command(
            Path::new("/fixture/session"),
            &format!("--to {seat}"),
        ))
        .expect("literal dispatch parses");
        let WorkerDispatch::Worker(worker) = parsed.resolve(&roster, 701) else {
            panic!("a valid thread resolves for {short}");
        };
        assert_eq!(worker.role, expected, "{short} with role {recorded:?}");
        assert_eq!(worker.label(), format!("{short} · {JOB}"));
        let encoded = serde_json::to_string(&WorkerDispatch::Worker(worker.clone()))
            .expect("worker serializes");
        assert_eq!(
            serde_json::from_str::<WorkerDispatch>(&encoded).expect("worker round trip"),
            WorkerDispatch::Worker(worker)
        );
    }
}

#[test]
fn typed_roster_fields_ignore_malformed_neighbours_and_optional_metadata() {
    let roster = WorkerRoster::parse(
        r#"{
        "cargo-liner-handler-enh-impl7":{"thread_id":" worker-thread ","launcher_pid":"bad","role":{ "future":true }},
        "broken": 17,
        "missing-thread": {"launcher_pid":701},
        "blank-thread": {"thread_id":"  "},
        "wrong-thread": {"thread_id":42},
        "cargo-liner-handler-enh-fix2":{"thread_id":"fix-thread","launcher_pid":702,"role":"review","unrelated":{"schema":2}}
    }"#,
    );
    let parsed = ParsedDispatch::parse(&command(
        Path::new("/fixture/session"),
        &format!("--to {SEAT}"),
    ))
    .expect("explicit dispatch");
    let WorkerDispatch::Worker(worker) = parsed.resolve(&roster, 701) else {
        panic!("invalid optional metadata must not discard its valid thread");
    };
    assert_eq!(worker.thread_id, "worker-thread");
    assert_eq!(worker.role, DispatchRole::Worker(WorkerRole::Impl));
    let implicit = ParsedDispatch::parse(&command(Path::new("/fixture/session"), ""))
        .expect("implicit dispatch");
    assert_eq!(implicit.resolve(&roster, 701), WorkerDispatch::Plain);
    let WorkerDispatch::Worker(worker) = implicit.resolve(&roster, 702) else {
        panic!("the valid neighbour still resolves by pid");
    };
    assert_eq!(worker.role, DispatchRole::Worker(WorkerRole::Review));
    assert_eq!(worker.thread_id, "fix-thread");
}

#[test]
fn folded_worker_role_reaches_the_final_tile_through_placement() {
    for (short, role, expected) in [
        ("impl4", None, DispatchRole::Worker(WorkerRole::Impl)),
        ("fix2", Some("test"), DispatchRole::Worker(WorkerRole::Test)),
        ("arrange", None, DispatchRole::Unroled),
    ] {
        let seat = format!("cargo-liner-handler-enh-{short}");
        let roster = WorkerRoster::parse(
            &serde_json::json!({seat.clone(): {"thread_id":"worker-thread","role":role}})
                .to_string(),
        );
        let parsed = ParsedDispatch::parse(&command(
            Path::new("/fixture/session"),
            &format!("--to {seat}"),
        ))
        .expect("dispatch parses");
        let mut shell = child("shell", "dispatch shell");
        shell.dispatch = parsed.resolve(&roster, 701);
        let mut thread = child("thread", "original thread");
        thread.thread_id = CodexThreadId::Recorded("worker-thread".to_string());
        thread.pid = None;
        let state =
            MachineState::Answered(vec![agent(10, &[shell, thread])], HierarchyEdges::default());
        let placement = AgentPlacement::from_machines(&[Machine {
            name:  "fixture",
            state: &state,
        }]);
        assert!(placement.folds_dispatch(&state.rows()[0].children[0]));
        let DispatchLabel::Worker(worker) = placement.dispatch_label(&state.rows()[0].children[1])
        else {
            panic!("the visible thread carries its joined dispatch");
        };
        assert_eq!(worker.role, expected);
        assert_eq!(worker.label(), format!("{short} · {JOB}"));
    }
}

/// Draw a folded seat with either a roster role or its name-stem role.
fn folded_role_buffer(short: &str, recorded: Option<&str>, managed: bool) -> Buffer {
    let seat = format!("cargo-liner-handler-enh-{short}");
    let roster = WorkerRoster::parse(
        &serde_json::json!({seat.clone():{"thread_id":"worker-thread","role":recorded}})
            .to_string(),
    );
    let parsed = ParsedDispatch::parse(&command(
        Path::new("/fixture/session"),
        &format!("--to {seat}"),
    ))
    .expect("dispatch parses");
    let mut shell = child("shell", "folded shell");
    shell.dispatch = parsed.resolve(&roster, 701);
    let mut thread = child("thread", "original worker name");
    thread.pid = None;
    thread.thread_id = CodexThreadId::Recorded("worker-thread".to_string());
    let mut root = agent(10, &[shell, thread]);
    root.session_id = SessionIdentity::Claude("manager-id".into());
    let edges = if managed {
        HierarchyEdges(vec![HierarchyEdge {
            manager: "manager-id".into(),
            member:  "worker-thread".into(),
            role:    HierarchyRole::Test,
        }])
    } else {
        HierarchyEdges::default()
    };
    let state = MachineState::Answered(vec![root], edges);
    let placement = AgentPlacement::from_machines(&[Machine {
        name:  "fixture",
        state: &state,
    }]);
    let cells = agent_cell::cell_order_placed(&placement);
    let palette = FooterColors::parse("role.worker=bright-cyan");
    let rect = Rect::new(0, 0, 220, 12);
    let mut buffer = Buffer::empty(rect);
    agent_cell::draw(
        &mut buffer,
        rect,
        &cells[0],
        &cells,
        1000,
        AccountColumn::Shown,
        &palette,
    );
    buffer
}

#[test]
fn folded_roles_draw_like_managed_workers_and_unroled_seats_show_worker() {
    for (short, recorded, expected_role) in [
        ("impl4", None, "impl"),
        ("fix2", None, "fix"),
        ("impl4", Some("review"), "review"),
        ("arrange", None, "worker"),
        ("mat-test", None, "test"),
    ] {
        for managed in [false, true] {
            let expected_role = if managed { "test" } else { expected_role };
            let buffer = folded_role_buffer(short, recorded, managed);
            let rect = *buffer.area();
            let text: Vec<String> = (0..rect.height)
                .map(|y| (0..rect.width).map(|x| buffer[(x, y)].symbol()).collect())
                .collect();
            let name = format!("{short} · {JOB}");
            let row = text
                .iter()
                .position(|line| line.contains(&name))
                .expect("folded name visible");
            let header = text
                .iter()
                .position(|line| line.contains("session") && line.contains("role"))
                .expect("tree header");
            let role_x = u16::try_from(text[header].find("role").expect("role column"))
                .expect("role column fits");
            let y = u16::try_from(row).expect("row fits");
            let name_start = text[row].find(&name).expect("folded name starts");
            let name_x =
                u16::try_from(text[row][..name_start].chars().count()).expect("name column fits");
            let drawn_role: String = (role_x..role_x + 6)
                .map(|x| buffer[(x, y)].symbol())
                .collect();
            assert_eq!(
                drawn_role.trim(),
                expected_role,
                "{short} managed={managed}"
            );
            assert_eq!(buffer[(role_x, y)].fg, Color::LightCyan);
            let name_color = Role::Rainbow(RainbowHue::Red)
                .style()
                .fg
                .expect("parent hue");
            assert_eq!(
                buffer[(name_x, y)].fg,
                name_color,
                "{short} managed={managed}"
            );
            assert!(
                !text
                    .iter()
                    .any(|line| line.contains("folded shell")
                        || line.contains("original worker name"))
            );
        }
    }
}

#[test]
fn folded_dispatches_sort_as_single_worker_rows() {
    let roster = WorkerRoster::parse(
        r#"{
        "cargo-liner-handler-enh-impl4":{"thread_id":"impl-thread"},
        "cargo-liner-handler-enh-fix2":{"thread_id":"fix-thread"},
        "cargo-liner-handler-enh-arrange":{"thread_id":"arrange-thread"}
    }"#,
    );
    let mut children = Vec::new();
    for (short, id, started) in [
        ("impl4", "impl-thread", 100),
        ("fix2", "fix-thread", 200),
        ("arrange", "arrange-thread", 300),
    ] {
        let seat = format!("cargo-liner-handler-enh-{short}");
        let parsed = ParsedDispatch::parse(&command(
            Path::new("/fixture/session"),
            &format!("--to {seat}"),
        ))
        .expect("dispatch parses");
        let mut shell = child("shell", &format!("hidden-{short}"));
        shell.dispatch = parsed.resolve(&roster, 701);
        let mut thread = child("thread", &format!("original-{short}"));
        thread.pid = None;
        thread.thread_id = CodexThreadId::Recorded(id.to_string());
        thread.started = started;
        children.extend([shell, thread]);
    }
    let state = MachineState::Answered(vec![agent(10, &children)], HierarchyEdges::default());
    let placement = AgentPlacement::from_machines(&[Machine {
        name:  "fixture",
        state: &state,
    }]);
    let defaults = agent_cell::cell_order_placed(&placement);
    for (sort, expected) in [
        (TreeSort::default(), ["impl4", "fix2", "arrange"]),
        (
            TreeSort {
                column:    TreeColumn::Role,
                direction: SortDirection::Ascending,
            },
            ["fix2", "impl4", "arrange"],
        ),
        (
            TreeSort {
                column:    TreeColumn::Age,
                direction: SortDirection::Ascending,
            },
            ["arrange", "fix2", "impl4"],
        ),
    ] {
        let sorts = std::collections::HashMap::from([(defaults[0].id.clone(), sort)]);
        let cells = agent_cell::cell_order_sorted(&placement, &sorts, 1000);
        let area = Rect::new(0, 0, 220, 12);
        let mut buffer = Buffer::empty(area);
        agent_cell::draw(
            &mut buffer,
            area,
            &cells[0],
            &cells,
            1000,
            AccountColumn::Shown,
            &FooterColors::default(),
        );
        let text: Vec<String> = (0..area.height)
            .map(|y| (0..area.width).map(|x| buffer[(x, y)].symbol()).collect())
            .collect();
        let actual: Vec<_> = text
            .iter()
            .filter_map(|line| {
                ["impl4", "fix2", "arrange"]
                    .into_iter()
                    .find(|name| line.contains(&format!("{name} · {JOB}")))
            })
            .collect();
        assert_eq!(actual, expected, "{sort:?}");
        assert!(
            !text
                .iter()
                .any(|line| line.contains("hidden-") || line.contains("original-"))
        );
    }
}

/// Old probe identities infer a role, while explicit roles remain authoritative.
#[test]
fn older_probe_dispatches_infer_roles_only_when_the_role_field_is_absent() {
    for (short, inferred) in [
        ("impl4", DispatchRole::Worker(WorkerRole::Impl)),
        ("fix2", DispatchRole::Worker(WorkerRole::Fix)),
        ("arrange", DispatchRole::Unroled),
        ("mat-test", DispatchRole::Worker(WorkerRole::Test)),
    ] {
        let payload = serde_json::json!({"worker": {
            "seat": format!("fixture-{short}"), "short_seat": short,
            "thread_id": "worker-thread", "job": JOB,
        }});
        let WorkerDispatch::Worker(worker) =
            serde_json::from_value(payload.clone()).expect("old probe identity")
        else {
            panic!("a recorded worker identity remains a worker");
        };
        assert_eq!(worker.role, inferred, "{short}");
        let mut malformed = payload.clone();
        malformed["worker"]["role"] = Value::Null;
        assert!(
            serde_json::from_value::<WorkerDispatch>(malformed).is_err(),
            "explicit null is not an omitted role"
        );
        for explicit in [
            DispatchRole::Unroled,
            DispatchRole::Worker(WorkerRole::Review),
        ] {
            let mut recorded = payload.clone();
            recorded["worker"]["role"] = serde_json::to_value(explicit).expect("role serializes");
            let WorkerDispatch::Worker(worker) =
                serde_json::from_value(recorded).expect("current probe identity")
            else {
                panic!("a recorded worker identity remains a worker");
            };
            assert_eq!(
                worker.role, explicit,
                "an explicit role must not be re-inferred"
            );
        }
    }
}

#[test]
fn pending_dispatch_to_a_different_seat_keeps_its_wrapper_visible() {
    let directory = roster_fixture();
    let first = command(directory.path(), &format!("--to {SEAT}"));
    let second = command(directory.path(), "--to cargo-liner-handler-enh-test2");
    let parsed = ParsedDispatch::parse(&first).expect("first dispatch");
    assert!(!parsed.is_final_command_of(&format!("{first} && {second}")));
    assert!(parsed.is_final_command_of(&format!("export READY=1; {first}")));
    let wrapper = process(
        600,
        10,
        "zsh",
        vec![
            "zsh".into(),
            "-c".into(),
            format!(
                "source /fixture/.claude/shell-snapshots/snapshot-test.sh && eval '{}'",
                format!("{first} && {second}").replace('\'', "'\\''")
            ),
        ],
    );
    let mut argv = launcher_arguments(directory.path());
    argv.splice(2..2, ["--to".to_owned(), SEAT.to_owned()]);
    let launcher = process(701, 600, "bash", argv);
    let dispatches = scan::find_worker_dispatches(&[wrapper, launcher]);
    assert!(!dispatches.contains_key(&600));
    assert!(matches!(
        dispatches.get(&701),
        Some(WorkerDispatch::Worker(_))
    ));
}
