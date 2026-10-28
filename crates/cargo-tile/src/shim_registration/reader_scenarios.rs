//! Reader scenarios that run the built binary in a PTY against actual shim publications.

#![allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]

use std::fs;
use std::fs::File;
use std::io::BufRead;
use std::io::BufReader;
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::Child;
use std::process::Command;
use std::process::ExitStatus;
use std::process::Stdio;
use std::rc::Rc;
use std::time::Duration;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Position;
use sysinfo::Pid;
use tempfile::TempDir;
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
use crate::census::cpu_observation;
use crate::constants::POPUP_CHROME_HEIGHT;
use crate::constants::READER_TIMESTAMPS_ENV;
use crate::hook;
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
        let root = std::env::current_dir()?;
        if scenario == "cpu-cache-server" {
            cpu_observation::start()?;
            let mut ready = [0];
            File::open(root.join("reader-start"))?.read_exact(&mut ready)?;
            assert_eq!(ready, [b'R']);
        }
        let parent = root.join("capture");
        let grid_motion = if matches!(scenario.to_str(), Some("child-source-switch" | "locale")) {
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

/// A supplied viewport draws into `TestBackend` without writing into the reader's PTY.
#[test]
fn reader_draws_a_supplied_frame_without_terminal_diagnostics() {
    if std::env::var("CARGO_TILE_TEST_READER").is_ok_and(|scenario| scenario == "supplied-frame") {
        let mut app = App::new_for_test().expect("build supplied reader app");
        let keymap = Rc::clone(&app.keymap);
        let mut terminal =
            Terminal::new(TestBackend::new(80, 12)).expect("create supplied reader terminal");
        terminal
            .draw(|frame| render::draw(frame, &mut app, &keymap))
            .expect("draw supplied reader frame");
        return;
    }
    let output = Command::new(std::env::current_exe().expect("reader test executable"))
        .args([
            "--exact",
            "shim_registration::reader_scenarios::reader_draws_a_supplied_frame_without_terminal_diagnostics",
            "--nocapture",
        ])
        .env("CARGO_TILE_TEST_READER", "supplied-frame")
        .output()
        .expect("run supplied reader frame");
    assert!(
        output.status.success(),
        "{}",
        reader_diagnostics(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "reader draw writes outside its backend: {}",
        reader_diagnostics(&output.stderr)
    );
}

/// A compiler outside cargo's ancestry charges only its requesting target directory.
#[test]
fn reader_attributes_compiler_cache_server_cpu_to_the_requesting_invocation() {
    reader_regression("cpu-cache-server");
}

/// Native changes inside a smoothing window use the held table reading.
#[test]
fn reader_cpu_oracle_applies_smoothing_and_publication_to_changing_load() {
    run_reader_script("--cpu-changing-load-self-check");
}

/// Buffered terminal reads preserve the scan and time of each published frame.
#[test]
fn reader_cpu_oracle_keeps_publication_times_when_frames_are_drained_late() {
    run_reader_script("--cpu-delayed-drain-self-check");
}

/// One compiler lifetime contributes once across all accounting groups.
#[test]
fn reader_cpu_oracle_rejects_a_compiler_counter_charged_twice() {
    run_reader_script("--cpu-duplicate-self-check");
}

/// Pending metadata keeps a native counter owed to its requesting invocation.
#[test]
fn reader_cpu_oracle_requires_pending_compiler_cpu_on_its_first_completed_scan() {
    run_reader_script("--cpu-pending-self-check");
}

/// A native lifetime keeps its CPU baseline through capture recovery, but not reuse.
#[test]
fn reader_cpu_oracle_keeps_the_native_baseline_through_a_capture_join() {
    run_reader_script("--cpu-owner-join-self-check");
}

/// An independent kernel counter rejects different units and another process.
#[test]
fn reader_cpu_oracle_checks_the_compiler_counter_against_the_kernel() {
    run_reader_script("--cpu-kernel-self-check");
}

/// Attribution replies cannot prove that their discovery counters followed SIGSTOP.
#[test]
fn reader_cpu_kernel_check_waits_for_the_stopped_discovery_counter() {
    run_reader_script("--cpu-stopped-snapshot-self-check");
}

/// Exact report deadlines remain exact at 0, 50, 100, and 150 milliseconds.
#[test]
fn reader_cpu_oracle_publishes_at_each_exact_report_deadline() {
    run_reader_script("--cpu-boundary-self-check");
}

/// Samples drawn close together cannot end CPU observation before its report windows pass.
#[test]
fn reader_cpu_observation_waits_for_samples_to_span_reporting_windows() {
    run_reader_script("--cpu-observation-self-check");
}

/// A discovered invocation cannot borrow blocked state from another row or an earlier frame.
#[test]
fn reader_waits_for_blocked_state_on_the_matching_invocation_row() {
    run_reader_script("--blocked-invocation-self-check");
}

/// Drive the production terminal loop through a PTY under the Rust cleanup owner.
/// The reader receives the shim's actual records, with no copied Rust implementation.
fn reader_regression(scenario: &str) { run_reader_script(scenario); }

#[derive(Clone, Copy)]
enum ScenarioDeadline {
    Default,
    After(Duration),
}

/// Own the scenario root for the entire driver lifetime.
struct ScenarioProcesses {
    directory: TempDir,
    cleanup:   Child,
}

impl ScenarioProcesses {
    fn new() -> Self {
        let directory = tempfile::tempdir().expect("isolate writer and reader processes");
        let mut cleanup = Command::new("python3")
            .arg("-S")
            .arg(READER_SCENARIO_SCRIPT)
            .arg("--watch-scenario")
            .arg(directory.path())
            .process_group(0)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("start scenario cleanup owner");
        let mut ready = String::new();
        BufReader::new(cleanup.stdout.take().expect("cleanup readiness pipe"))
            .read_line(&mut ready)
            .expect("wait for scenario cleanup owner");
        assert_eq!(ready, "ready\n");
        Self { directory, cleanup }
    }

    fn path(&self) -> &Path { self.directory.path() }
}

impl Drop for ScenarioProcesses {
    fn drop(&mut self) {
        // EOF also reaches the owner if a timeout kills this Rust test outright.
        drop(self.cleanup.stdin.take());
        let _ = self.cleanup.wait();
    }
}

struct ReaderScenarioResult {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

/// The parser self-check uses the same script without starting a reader.
fn run_reader_script(scenario: &str) {
    let output = reader_script_result(scenario, ScenarioDeadline::Default);
    assert!(
        output.status.success(),
        "{scenario}: {}\n{}",
        reader_diagnostics(&output.stderr),
        reader_diagnostics(&output.stdout)
    );
}

fn reader_script_result(scenario: &str, deadline: ScenarioDeadline) -> ReaderScenarioResult {
    let directory = ScenarioProcesses::new();
    let executable = std::env::current_exe().expect("integration reader executable");
    // Foundation reads the executable directory when it first reads system
    // appearance. Keep that read inside the fixture, away from build artifacts.
    #[cfg(target_os = "macos")]
    let executable = {
        let staged = directory
            .path()
            .join(executable.file_name().expect("reader executable file name"));
        fs::copy(&executable, &staged).expect("stage isolated reader executable");
        staged
    };
    let shim_path = directory.path().join("capture-shim.sh");
    fs::write(&shim_path, hook::shim_source()).expect("write rendered reader scenario shim");
    let stdout_path = directory.path().join("scenario.stdout");
    let stderr_path = directory.path().join("scenario.stderr");
    // The script times its CPU observation against the reader's own windows.
    let cadence = CensusCadence::for_test();
    let mut command = Command::new("python3");
    command
        .process_group(0)
        .arg("-S")
        .arg(READER_SCENARIO_SCRIPT)
        .arg(directory.path())
        .arg(executable)
        .arg(&shim_path)
        .arg(scenario)
        .arg(cpu_observation_window(cadence).as_secs_f64().to_string())
        .arg(cadence.smoothing.as_secs_f64().to_string())
        .stdout(Stdio::from(
            File::create(&stdout_path).expect("create reader scenario stdout file"),
        ))
        .stderr(Stdio::from(
            File::create(&stderr_path).expect("create reader scenario stderr file"),
        ));
    if let ScenarioDeadline::After(duration) = deadline {
        command.arg(duration.as_secs_f64().to_string());
    }
    let status = command
        .status()
        .expect("run isolated production reader regression");
    let stdout = fs::read(stdout_path).expect("read reader scenario stdout file");
    let stderr = fs::read(stderr_path).expect("read reader scenario stderr file");
    if std::env::var_os(READER_TIMESTAMPS_ENV).is_some() {
        print!("{}", String::from_utf8_lossy(&stderr));
    }
    ReaderScenarioResult {
        status,
        stdout,
        stderr,
    }
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

/// A reader that ignores its end request cannot keep the script alive by filling its terminal.
#[test]
fn reader_end_deadline_kills_a_reader_that_ignores_end_and_keeps_writing() {
    run_reader_script("--reader-end-self-check");
}

/// The process-wide deadline identifies the scenario and pending predicate.
#[test]
fn reader_scenario_deadline_names_scenario_and_pending_wait() {
    let scenario = "--scenario-deadline-self-check";
    let output = reader_script_result(scenario, ScenarioDeadline::After(Duration::from_millis(50)));
    let stderr = reader_diagnostics(&output.stderr);
    assert!(!output.status.success(), "{stderr}");
    assert!(
        stderr.contains(
            "--scenario-deadline-self-check exceeded 0.05 seconds while waiting for deadline self-check predicate"
        ),
        "{stderr}"
    );
}

/// Concurrent waits cannot restore the name of a wait that has already finished.
#[test]
fn reader_scenario_deadline_names_a_wait_still_pending_in_another_thread() {
    let scenario = "--concurrent-deadline-self-check";
    let output = reader_script_result(scenario, ScenarioDeadline::After(Duration::from_millis(50)));
    let stderr = reader_diagnostics(&output.stderr);
    assert!(!output.status.success(), "{stderr}");
    assert!(
        stderr.contains(
            "--concurrent-deadline-self-check exceeded 0.05 seconds while waiting for concurrent pending predicate"
        ),
        "{stderr}"
    );
}

#[test]
fn settings_click_after_navigation_selects_the_row_still_drawn() {
    let mut app = App::new_for_test().expect("build isolated settings app");
    app.loaded_config.config.commands.excluded = vec!["wrapped-command ".repeat(24)];
    app.loaded_config.config.commands.hidden = vec!["port".to_owned()];
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
    let hidden_commands = selection("hidden cargo commands");
    let keymap = Rc::clone(&app.keymap);
    keymap.dispatch_framework_global(GlobalAction::OpenSettings, &mut app);
    let mut terminal = Terminal::new(TestBackend::new(80, POPUP_CHROME_HEIGHT + 3))
        .expect("create settings terminal");
    terminal
        .draw(|frame| render::draw(frame, &mut app, &keymap))
        .expect("draw settings layout");
    app.framework.settings_pane.select_row(hidden_commands);
    terminal
        .draw(|frame| render::draw(frame, &mut app, &keymap))
        .expect("draw hidden commands selection");
    let pane = &app.framework.settings_pane;
    let area = pane.viewport().content_area();
    let offset = pane.viewport().scroll_offset();
    assert_eq!(area.height, 3);
    assert_eq!(pane.viewport().pos(), hidden_commands);
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
        hidden_commands + 1
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

/// SIGKILL skips Python finally blocks; the Rust owner must still retire the shim group.
#[test]
fn a_killed_reader_driver_leaves_no_shim_group_or_fixture_root() {
    let directory = ScenarioProcesses::new();
    let root = directory.path().to_owned();
    let shim = root.join("capture-shim.sh");
    fs::write(&shim, hook::shim_source()).expect("write shim for killed driver");
    let cadence = CensusCadence::for_test();
    let mut driver = Command::new("python3")
        .process_group(0)
        .arg("-S")
        .arg(READER_SCENARIO_SCRIPT)
        .arg(&root)
        .arg(std::env::current_exe().expect("reader executable"))
        .arg(shim)
        .arg("--driver-death-self-check")
        .arg(cpu_observation_window(cadence).as_secs_f64().to_string())
        .arg(cadence.smoothing.as_secs_f64().to_string())
        .stdout(Stdio::piped())
        .spawn()
        .expect("start driver with actual capture shim");
    let mut group = String::new();
    BufReader::new(driver.stdout.take().expect("driver readiness pipe"))
        .read_line(&mut group)
        .expect("shim group readiness");
    let group: u32 = group.trim().parse().expect("shim publishes its group id");
    driver.kill().expect("kill driver before Python cleanup");
    let status = driver.wait().expect("reap killed driver");
    assert!(!status.success());
    drop(directory);
    let output = Command::new("ps")
        .args(["-axo", "pgid=,stat="])
        .output()
        .expect("read remaining shim group members");
    assert!(output.status.success());
    let live = String::from_utf8(output.stdout)
        .expect("process group listing")
        .lines()
        .any(|line| {
            let mut fields = line.split_whitespace();
            fields.next().and_then(|value| value.parse::<u32>().ok()) == Some(group)
                && fields.next().is_some_and(|state| !state.starts_with('Z'))
        });
    // Retire only this test's group even on the red run, before the assertion.
    let group_id = rustix::process::Pid::from_raw(i32::try_from(group).expect("owned group id"))
        .expect("nonzero owned group id");
    let _ = rustix::process::kill_process_group(group_id, rustix::process::Signal::KILL);
    assert!(!live, "killed Python driver left live shim group {group}");
    assert!(!root.exists(), "killed driver left its fixture root");
}
