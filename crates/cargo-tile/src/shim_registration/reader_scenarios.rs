//! Reader scenarios that run the built binary in a PTY against actual shim publications.

#![allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]

use std::process::Command;
use std::rc::Rc;
use std::time::Duration;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Position;
use sysinfo::Pid;
use tui_pane::GlobalAction;
use tui_pane::NavAction;
use tui_pane::Navigation;
use tui_pane::SettingsLineTarget;
use tui_pane::SettingsNavigation;
use tui_pane::SettingsRowIdentity;
use tui_pane::SettingsRowPayload;
use tui_pane::TerminalApp;

use crate::app::App;
use crate::census::CensusCadence;
use crate::census::CensusScope;
use crate::constants::POPUP_CHROME_HEIGHT;
use crate::constants::READER_TIMESTAMPS_ENV;
use crate::render;
use crate::settings;
use crate::terminal::GridMotion;

/// Exercise the built binary using actual shim publications and a reconstructed PTY screen.
///
/// Run from its path rather than through `python3 -c`: the script is the
/// parent of every writer the reader draws, so its command line is a step
/// in each cell's chain, and a long tree would put the script's own
/// source -- every string its screen predicates look for included -- on
/// the screen under test.
const READER_SCENARIO_SCRIPT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/shim_registration/reader_scenario.py"
);

/// The child receives a parent path through this constructor, never application configuration.
#[test]
fn reader_child() -> std::io::Result<()> {
    if let Some(scenario) = std::env::var_os("CARGO_TILE_TEST_READER") {
        let parent = std::env::current_dir()?.join("capture");
        let grid_motion = if scenario == "child-source-switch" {
            GridMotion::Immediate
        } else {
            GridMotion::Animated
        };
        assert_eq!(
            crate::terminal::run_with_capture_parent(
                parent,
                CensusCadence::for_test(),
                scenario_scope(),
                grid_motion,
            ),
            std::process::ExitCode::SUCCESS
        );
    }
    Ok(())
}

/// The scenario script starts the reader and every writer it watches, so
/// the census counts only cargo processes running under that script.
fn scenario_scope() -> CensusScope {
    CensusScope::descendants_of(Pid::from_u32(std::os::unix::process::parent_id()))
}

/// How long the CPU scenario reads the table: readings climb for one
/// smoothing window, then the sustained set spans three report windows.
fn cpu_observation_window(cadence: CensusCadence) -> Duration {
    cadence.smoothing + cadence.report * 3
}

/// PTY read boundaries cannot expose a partly redrawn invocation twice.
#[test]
fn reader_snapshots_publish_only_completed_terminal_frames() {
    run_reader_script("--terminal-frame-self-check");
}

/// A compiler outside cargo's ancestry charges only its requesting target directory.
#[test]
fn reader_attributes_compiler_cache_server_cpu_to_the_requesting_invocation() {
    reader_regression("cpu-cache-server");
}

/// Drive the production terminal loop through a PTY; Python owns every child and terminal fd.
/// The reader receives the shim's actual records, with no copied Rust implementation.
fn reader_regression(scenario: &str) { run_reader_script(scenario); }

/// The parser self-check uses the same script without starting a reader.
fn run_reader_script(scenario: &str) {
    let directory = tempfile::tempdir().expect("isolate writer and reader processes");
    // The script times its CPU observation against the reader's own windows.
    let cadence = CensusCadence::for_test();
    let output = Command::new("python3")
        .arg(READER_SCENARIO_SCRIPT)
        .arg(directory.path())
        .arg(std::env::current_exe().expect("integration reader executable"))
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/cargo-capture-shim.sh"
        ))
        .arg(scenario)
        .arg(cpu_observation_window(cadence).as_secs_f64().to_string())
        .arg(cadence.smoothing.as_secs_f64().to_string())
        .output()
        .expect("run isolated production reader regression");
    if std::env::var_os(READER_TIMESTAMPS_ENV).is_some() {
        print!("{}", String::from_utf8_lossy(&output.stderr));
    }
    assert!(
        output.status.success(),
        "{scenario}: {}\n{}",
        reader_diagnostics(&output.stderr),
        reader_diagnostics(&output.stdout)
    );
}

/// Keep failed assertions readable without hundreds of empty terminal cells.
fn reader_diagnostics(output: &[u8]) -> String {
    String::from_utf8_lossy(output)
        .lines()
        .filter(|line| {
            !line
                .chars()
                .all(|character| character.is_whitespace() || "│┌┐└┘├┤─".contains(character))
        })
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect::<Vec<_>>()
        .join("\n")
}

/// One key queued beside a resize must open settings after the reader resumes.
#[test]
fn reader_opens_settings_when_resize_and_key_are_pending_together() {
    reader_regression("settings-scroll-burst");
}

#[test]
fn settings_click_after_navigation_selects_the_row_still_drawn() {
    let mut app = App::new_for_test().expect("build isolated settings app");
    app.loaded_config.config.commands.excluded = vec!["wrapped-command ".repeat(24)];
    app.loaded_config.config.commands.hidden_when_idle = vec!["port".to_owned()];
    let rows = settings::rows(&app).rows().to_vec();
    let selection = |label| {
        rows.iter()
            .find_map(|row| match row.identity {
                SettingsRowIdentity::Selectable(payload) if row.label == label => {
                    Some(payload.get())
                },
                _ => None,
            })
            .expect("find selectable settings row")
    };
    let excluded = selection("excluded");
    let hidden_when_idle = selection("hidden when idle");
    let keymap = Rc::clone(&app.keymap);
    keymap.dispatch_framework_global(GlobalAction::OpenSettings, &mut app);
    let mut terminal = Terminal::new(TestBackend::new(80, POPUP_CHROME_HEIGHT + 3))
        .expect("create settings terminal");
    terminal
        .draw(|frame| render::draw(frame, &mut app, &keymap))
        .expect("draw settings layout");
    app.framework.settings_pane.select_row(hidden_when_idle);
    terminal
        .draw(|frame| render::draw(frame, &mut app, &keymap))
        .expect("draw hidden-when-idle selection");
    let pane = &app.framework.settings_pane;
    let area = pane.viewport().content_area();
    let offset = pane.viewport().scroll_offset();
    assert_eq!(area.height, 3);
    assert_eq!(pane.viewport().pos(), hidden_when_idle);
    assert_eq!(
        pane.line_target(offset + 1),
        SettingsLineTarget::Row(SettingsRowPayload::new(excluded))
    );
    let click = Position::new(area.x + 1, area.y + 1);
    let painted = terminal.backend().buffer().clone();
    let clicked_line = (area.x..area.right())
        .map(|x| painted[(x, click.y)].symbol())
        .collect::<String>();
    assert!(clicked_line.contains("wrapped-command"), "{clicked_line}");
    let focused = *app.framework.focused();

    // The terminal drains navigation and mouse input before repainting.
    SettingsNavigation::<App>::dispatcher()(NavAction::Down, focused, &mut app);
    assert!(app.framework.settings_pane.viewport().scroll_offset() > offset);
    assert_eq!(
        app.framework.settings_pane.viewport().pos(),
        hidden_when_idle + 1
    );
    app.click(click);

    assert_eq!(terminal.backend().buffer(), &painted);
    assert_eq!(app.framework.settings_pane.viewport().pos(), excluded);
}

/// The shim's quiet rewrite still produces one directly registered JSON invocation.
#[test]
fn reader_keeps_one_direct_row_for_quiet_json() { reader_regression("quiet-json-long"); }

/// A readable parent retains its family when its only child changes row source.
#[test]
fn reader_keeps_parent_family_across_its_only_childs_source_switch() {
    reader_regression("child-source-switch");
}

/// A foreign-owned uid directory cannot relabel a live cargo process and is explained in
/// settings.
#[test]
fn reader_ignores_foreign_owned_account_and_reports_its_owner_in_settings() {
    reader_regression("root-headings");
}

/// Excluded writers remain live for cleanup and nested capture membership.
#[test]
fn reader_excludes_a_live_command_without_sweeping_its_capture() { reader_regression("excluded"); }

/// Production parsing and kernel verification accept the actual writer's bytes.
#[test]
fn reader_accepts_live_writer_under_different_locale_and_timezone() { reader_regression("locale"); }
