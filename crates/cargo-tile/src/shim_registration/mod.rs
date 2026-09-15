//! Exercise shim publications and the production reader with an explicit fixture parent.

#[cfg(test)]
mod app_scenarios;
#[cfg(test)]
mod rows_readout;
#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod shared_capture;
#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod wire;

/// The child receives a parent path through this constructor, never application configuration.
#[test]
fn reader_child() -> std::io::Result<()> {
    if std::env::var_os("CARGO_TILE_TEST_READER").is_some() {
        let parent = std::env::current_dir()?.join("capture");
        assert_eq!(
            crate::terminal::run_with_capture_parent(parent),
            std::process::ExitCode::SUCCESS
        );
    }
    Ok(())
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::fs;
    use std::io::Write;
    use std::process::Command;
    use std::rc::Rc;
    use std::time::Duration;

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Position;
    use ratatui::layout::Rect;
    use sysinfo::Pid;
    use sysinfo::ProcessRefreshKind;
    use sysinfo::ProcessesToUpdate;
    use sysinfo::System;
    use tui_pane::GlobalAction;
    use tui_pane::NavAction;
    use tui_pane::Navigation;
    use tui_pane::SettingsLineTarget;
    use tui_pane::SettingsRowIdentity;
    use tui_pane::SettingsRowPayload;

    use crate::app::App;
    use crate::census::Measurement;
    use crate::census::spawn_with_resolver;
    use crate::config::Config;
    use crate::constants::POPUP_CHROME_HEIGHT;
    use crate::interaction;
    use crate::navigation::AppNavigation;
    use crate::progress::capture_roots::CaptureRoots;
    use crate::render;
    use crate::roster::Roster;
    use crate::settings;
    use crate::tiles::TileContent;

    /// Span several reporting windows while retaining every completed observation.
    const CPU_OBSERVATION_SCANS: usize = 16;

    /// Exercise the built binary using actual shim publications and a reconstructed PTY screen.
    const READER_SCENARIO_SCRIPT: &str = include_str!("reader_scenario.py");

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

    /// Consume every production scan so PTY polling cannot miss a brief unavailable row.
    #[test]
    fn cpu_scan_child() -> std::io::Result<()> {
        let Ok(pid) = std::env::var("CARGO_TILE_TEST_CPU_PID") else {
            return Ok(());
        };
        let pid: u32 = pid.parse().expect("fixture cargo pid");
        let mut system = System::new();
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[Pid::from_u32(pid)]),
            false,
            ProcessRefreshKind::nothing().without_tasks().with_cpu(),
        );
        let invocation = system
            .process(Pid::from_u32(pid))
            .ok_or_else(|| std::io::Error::other("fixture invocation is not alive"))?;
        if invocation.accumulated_cpu_time() == 0 {
            return Err(std::io::Error::other(
                "fixture invocation must accumulate its own CPU time before scanning descendants",
            ));
        }
        let parent = std::env::current_dir()?.join("capture");
        let mut output = fs::File::create("cpu-scans")?;
        let config = Config::default();
        let (receiver, worker) =
            spawn_with_resolver(&config, move || CaptureRoots::from_parent(&parent));
        let result = (|| {
            for index in 0..CPU_OBSERVATION_SCANS {
                let scan = receiver
                    .recv_timeout(Duration::from_secs(10))
                    .map_err(std::io::Error::other)?;
                let rows: Vec<_> = scan
                    .groups
                    .iter()
                    .flat_map(|group| std::iter::once(&group.lead).chain(&group.rest))
                    .filter(|row| row.pid == pid)
                    .collect();
                if rows.len() != 1 {
                    return Err(std::io::Error::other(format!(
                        "scan {index} has {} rows for fixture pid {pid}",
                        rows.len()
                    )));
                }
                match &rows[0].cpu {
                    Measurement::Reading(cpu) => writeln!(output, "{index}\t{cpu}")?,
                    Measurement::Unavailable(reason) => writeln!(output, "{index}\t{reason:?}")?,
                }
                output.flush()?;
            }
            Ok(())
        })();
        drop(receiver);
        worker.join().expect("CPU scanner shuts down");
        result
    }

    /// Drive the production terminal loop through a PTY; Python owns every child and terminal fd.
    /// The reader receives the shim's actual records, with no copied Rust implementation.
    fn reader_regression(scenario: &str) { run_reader_script(scenario); }

    /// The parser self-check uses the same script without starting a reader.
    fn run_reader_script(scenario: &str) {
        let directory = tempfile::tempdir().expect("isolate writer and reader processes");
        let mut command = Command::new("python3");
        if scenario == "--terminal-frame-self-check" {
            let inner = Rect::new(0, 0, 80, 10);
            let mut buffer = Buffer::empty(inner);
            render::draw_cell_for_test(
                &mut buffer,
                &Roster::new(),
                &TileContent::Summary,
                inner,
                11,
            );
            let readout: String = (0..inner.width)
                .map(|x| buffer[(x, inner.height - 1)].symbol())
                .collect();
            command.env("CARGO_TILE_TEST_ROWS_READOUT", readout);
        }
        let output = command
            .args(["-c", READER_SCENARIO_SCRIPT])
            .arg(directory.path())
            .arg(std::env::current_exe().expect("integration reader executable"))
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/src/cargo-capture-shim.sh"
            ))
            .arg(scenario)
            .arg(CPU_OBSERVATION_SCANS.to_string())
            .output()
            .expect("run isolated production reader regression");
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
        let rows = settings::rows(&app).rows;
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
        AppNavigation::dispatcher()(NavAction::Down, focused, &mut app);
        assert!(app.framework.settings_pane.viewport().scroll_offset() > offset);
        assert_eq!(
            app.framework.settings_pane.viewport().pos(),
            hidden_when_idle + 1
        );
        interaction::handle_click(&mut app, click);

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
    fn reader_excludes_a_live_command_without_sweeping_its_capture() {
        reader_regression("excluded");
    }

    /// Production parsing and kernel verification accept the actual writer's bytes.
    #[test]
    fn reader_accepts_live_writer_under_different_locale_and_timezone() {
        reader_regression("locale");
    }
}
