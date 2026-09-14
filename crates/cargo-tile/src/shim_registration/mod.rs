//! Exercise shim publications and the production reader with an explicit fixture parent.

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

/// Read actual process arguments and reject unsupported options before any installation.
#[test]
fn cli_rejects_unknown_process_arguments() -> std::io::Result<()> {
    if std::env::var_os("CARGO_TILE_TEST_ARGUMENTS").is_some() {
        // Libtest accepts --exact to enter this child, while cargo-tile must reject it.
        let result = crate::cli::Cli::parse_arguments().run();
        assert_eq!(result, std::process::ExitCode::FAILURE);
        return Ok(());
    }
    let output = std::process::Command::new(std::env::current_exe()?)
        .args([
            "--exact",
            "shim_registration::cli_rejects_unknown_process_arguments",
            "--nocapture",
        ])
        .env("CARGO_TILE_TEST_ARGUMENTS", "1")
        .output()?;
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("unexpected argument '--exact'"), "{error}");
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
    use ratatui::layout::Position;
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
    use crate::census::InvocationId;
    use crate::census::Measurement;
    use crate::census::spawn_with_resolver;
    use crate::config::Config;
    use crate::constants::CAPTURE_REGISTRATION_BYTES;
    use crate::constants::POPUP_CHROME_HEIGHT;
    use crate::constants::PROCESS_POLL_MILLIS;
    use crate::constants::SHIM_MARKER_SEARCH_BYTES;
    use crate::constants::SUPPORTED_REGISTRATION_VERSION;
    use crate::interaction;
    use crate::navigation::AppNavigation;
    use crate::progress::capture_roots::CaptureRoots;
    use crate::registration::ParseError;
    use crate::registration::Registration;
    use crate::render;
    use crate::settings;

    /// Span several reporting windows while retaining every completed observation.
    const CPU_OBSERVATION_SCANS: usize = 16;

    /// Exercise the built binary using actual shim publications and a reconstructed PTY screen.
    const READER_SCENARIO_SCRIPT: &str = include_str!("reader_scenario.py");

    /// Reaped children remain measurable while fresh descendants appear between scans.
    #[test]
    fn reader_reports_cpu_on_every_scan_while_descendants_turn_over() {
        reader_regression("cpu-turnover");
    }

    /// PTY read boundaries cannot expose a partly redrawn invocation twice.
    #[test]
    fn reader_snapshots_publish_only_completed_terminal_frames() {
        reader_regression("terminal-frame-completion");
    }

    /// A compiler outside cargo's ancestry charges only its requesting target directory.
    #[test]
    fn reader_attributes_compiler_cache_server_cpu_to_the_requesting_invocation() {
        reader_regression("cpu-cache-server");
    }

    /// Two live wrapper clients sharing a target cannot establish one external compile owner.
    #[test]
    fn reader_refuses_compiler_cache_cpu_when_invocations_share_the_target_directory() {
        reader_regression("cpu-cache-ambiguous");
    }

    /// A hidden requester still prevents another command from claiming its compiler.
    #[test]
    fn reader_refuses_compiler_cache_cpu_when_an_excluded_invocation_shares_the_target() {
        reader_regression("cpu-cache-excluded");
    }

    /// Recovered registration proof preserves the live compiler's rate and prior credit.
    #[test]
    fn reader_keeps_compiler_cache_cpu_when_registration_identity_recovers() {
        reader_regression("cpu-cache-identity-recovery");
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
        let mut identities = fs::File::create("cpu-identities")?;
        let mut config = Config::default();
        if let Ok(excluded) = std::env::var("CARGO_TILE_TEST_CPU_EXCLUDED") {
            config.commands.excluded.push(excluded);
        }
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
                let identity = match &rows[0].invocation_id {
                    InvocationId::Captured(_) => "captured",
                    InvocationId::Process(_) => "process",
                };
                writeln!(identities, "{identity}")?;
                identities.flush()?;
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
    fn reader_regression(scenario: &str) {
        let directory = tempfile::tempdir().expect("isolate writer and reader processes");
        let output = Command::new("python3")
            .args(["-c", READER_SCENARIO_SCRIPT])
            .arg(directory.path())
            .arg(std::env::current_exe().expect("integration reader executable"))
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/src/cargo-capture-shim.sh"
            ))
            .arg(scenario)
            .arg(CAPTURE_REGISTRATION_BYTES.to_string())
            .arg(SHIM_MARKER_SEARCH_BYTES.to_string())
            .arg(PROCESS_POLL_MILLIS.to_string())
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

    /// Two HOME values cannot split one physical directory into separate headings.
    #[test]
    fn reader_groups_one_directory_across_different_writer_homes() {
        reader_regression("grouping");
    }

    /// An unavailable parent sample stays inside the cache server's readiness deadline.
    #[test]
    fn cache_server_readiness_retries_unavailable_parent_observations() {
        reader_regression("cache-readiness-delayed");
    }

    /// A missing cache server fails at the deadline with its retained diagnostics.
    #[test]
    fn cache_server_readiness_rejects_permanent_parent_absence() {
        reader_regression("cache-readiness-never");
    }

    /// Navigation draws every account in a short popup and reaches the settings below them.
    #[test]
    fn reader_scrolls_settings_accounts_into_view_with_keyboard_navigation() {
        reader_regression("settings-scroll");
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

    /// Select the fixture's command pane even when an unrelated pane precedes it.
    #[test]
    fn reader_grouping_finds_fixture_markers_after_an_unrelated_pane() {
        reader_regression("grouping-earlier-pane");
    }

    /// Markers outside a pane or split across panes remain pending until one pane contains all.
    #[test]
    fn reader_pane_readiness_waits_for_all_fixture_markers_in_one_pane() {
        reader_regression("pane-readiness-delayed");
    }

    /// Missing and duplicate fixture panes expire with their final count and terminal screen.
    #[test]
    fn reader_pane_readiness_reports_the_final_screen_on_timeout() {
        reader_regression("pane-readiness-never");
    }

    /// A verified live registration supplies fields absent from the process census.
    #[test]
    fn reader_displays_a_registration_without_a_cargo_process_row() {
        reader_regression("fallback");
    }

    /// The registration's home cannot abbreviate a different scanner's directory.
    #[test]
    fn reader_keeps_a_registration_from_another_home_absolute() {
        reader_regression("fallback-other-home");
    }

    /// A log read failure cannot erase the separately verified registration's row.
    #[test]
    fn reader_displays_a_verified_registration_with_an_unreadable_log() {
        reader_regression("fallback-unreadable-log");
    }

    /// Missing kernel proof permits retention, never a registration-sourced row.
    #[test]
    fn reader_does_not_source_a_row_from_an_unknown_registration() {
        reader_regression("fallback-unknown");
    }

    /// A future layout is diagnosed before its fields can supply identity or cleanup evidence.
    #[test]
    fn reader_retains_a_newer_registration_and_log_while_the_writer_is_alive() {
        reader_regression("version-newer-live");
    }

    /// Repeated completed sweeps preserve unsupported records even after the writer exits.
    #[test]
    fn reader_retains_a_newer_registration_and_log_after_the_writer_exits() {
        reader_regression("version-newer-ended");
    }

    /// The read cap must preserve the version diagnostic and both artifacts during cleanup.
    #[test]
    fn reader_diagnoses_and_retains_an_oversized_newer_registration_after_cleanup() {
        reader_regression("version-newer-oversized");
    }

    /// Header dispatch also precedes the payload cap for callers supplying bytes directly.
    #[test]
    fn oversized_newer_registration_reports_its_version_before_its_size() {
        let encountered = SUPPORTED_REGISTRATION_VERSION + 1;
        let mut bytes = format!("cargo-tile-v{encountered}\0").into_bytes();
        bytes.resize(
            usize::try_from(CAPTURE_REGISTRATION_BYTES).expect("registration cap fits usize") + 1,
            b'x',
        );
        assert_eq!(
            Registration::parse(&bytes),
            Err(ParseError::UnsupportedVersion { encountered })
        );
    }

    /// Supported framing errors remain distinct from a request to upgrade the reader.
    #[test]
    fn reader_reports_a_malformed_supported_registration_separately() {
        reader_regression("version-malformed");
    }

    /// Both framing generations retain their live progress and registration-only row source.
    #[test]
    fn reader_reads_live_v2_and_v3_publications_together() { reader_regression("version-mixed"); }

    /// Auto-install keeps the newer shim and retains recovery guidance after the toast expires.
    #[test]
    fn older_reader_startup_keeps_the_newer_shim_and_reports_it_in_settings() {
        reader_regression("startup-newer");
    }

    /// Auto-install refuses a partial version line before touching either installed file.
    #[test]
    fn reader_startup_refuses_a_truncated_version_without_changing_installed_files() {
        reader_regression("startup-truncated");
    }

    /// A separate failed installation cannot hide or mislabel a retained newer shim.
    #[test]
    fn older_reader_startup_distinguishes_a_kept_newer_shim_from_an_install_failure() {
        reader_regression("startup-newer-failure");
    }

    /// Exclusion applies before either registration or process row construction.
    #[test]
    fn reader_excludes_both_sources_of_the_same_invocation() {
        reader_regression("fallback-excluded");
    }

    /// Exec preserves the invocation's single row and heading in both directions.
    #[test]
    fn reader_keeps_one_row_when_registration_and_process_sources_switch() {
        reader_regression("fallback-source-switch");
    }

    /// Nested invocations keep their own rows and their parent's tile through both source changes.
    #[test]
    fn reader_keeps_process_children_with_a_parent_that_changes_row_source() {
        reader_regression("fallback-nested-source-switch");
    }

    /// The shim's quiet rewrite still produces one directly registered JSON invocation.
    #[test]
    fn reader_keeps_one_direct_row_for_quiet_json() { reader_regression("quiet-json-long"); }

    /// Short quiet options are removed only before the argument passthrough boundary.
    #[test]
    fn reader_keeps_one_direct_row_for_short_quiet_json() { reader_regression("quiet-json-short"); }

    /// Separate message-format arguments authorize the same exact quiet rewrite.
    #[test]
    fn reader_keeps_one_direct_row_for_separate_json_format() {
        reader_regression("quiet-json-separate");
    }

    /// Quiet removal without a JSON registration cannot acquire direct ownership.
    #[test]
    fn reader_rejects_quiet_removal_from_non_json_registrations() {
        for scenario in [
            "rejected-rewrite-non-json-long",
            "rejected-rewrite-non-json-short",
        ] {
            reader_regression(scenario);
        }
    }

    /// Quiet arguments after -- belong to the invoked program and must match exactly.
    #[test]
    fn reader_rejects_quiet_removal_after_the_passthrough_separator() {
        for scenario in ["rejected-rewrite-post-long", "rejected-rewrite-post-short"] {
            reader_regression(scenario);
        }
    }

    /// JSON quiet normalization never authorizes another argument to change.
    #[test]
    fn reader_rejects_unrelated_argument_changes_during_quiet_removal() {
        reader_regression("rejected-rewrite-unrelated");
    }

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

    /// A valid live registration with no process row cannot enter through another uid's name.
    #[test]
    fn reader_never_sources_a_row_from_a_foreign_owned_account_directory() {
        reader_regression("fallback-foreign-owned");
    }

    /// The summary keeps real process ownership when a capture directory claims another uid.
    #[test]
    fn reader_summary_ignores_foreign_owned_account_attribution() {
        reader_regression("summary-root-headings");
    }

    /// The summary cannot hide any of a registration's three unavailable measurements.
    #[test]
    fn reader_shows_unavailable_registration_measurements_in_the_summary() {
        reader_regression("fallback-summary");
    }

    /// Account qualification does not combine independent checkout directories.
    #[test]
    fn reader_keeps_two_directories_under_one_account_and_root_separate() {
        reader_regression("two-directories");
    }

    /// A forged account directory never adds a second view of the process invocation.
    #[test]
    fn reader_keeps_one_process_row_when_a_foreign_owned_directory_copies_its_proof() {
        reader_regression("root-duplicate");
    }

    /// Selection also prevents duplication without any eligible process row.
    #[test]
    fn reader_keeps_one_registration_row_when_a_foreign_owned_directory_copies_its_proof() {
        reader_regression("fallback-root-duplicate");
    }

    /// A foreign-owned directory cannot supply confirmation for an unconfirmed account capture.
    #[test]
    fn reader_does_not_borrow_confirmation_from_a_foreign_owned_directory() {
        reader_regression("fallback-selected-unknown");
    }

    /// Competing generations remain explained without a row and recover on the next scan.
    #[test]
    fn reader_reports_and_recovers_ambiguity_without_a_process_row() {
        reader_regression("fallback-ambiguous");
    }

    /// One enclosing capture supplies progress without replacing nested commands or pids.
    #[test]
    fn reader_keeps_nested_invocations_distinct_with_one_enclosing_capture() {
        reader_regression("nested");
    }

    /// An exec replaces cargo with an application without making its cargo children direct owners.
    #[test]
    fn reader_keeps_application_spawned_cargo_invocations_distinct() {
        reader_regression("exec-nested");
    }

    /// Excluding the captured run cannot hide cargo children launched by its application.
    #[test]
    fn reader_keeps_application_spawned_cargo_when_run_is_excluded() {
        reader_regression("exec-excluded");
    }

    /// Same-birth publications cannot select an old generation by its filename order.
    #[test]
    fn reader_rejects_competing_generations_until_one_publication_remains() {
        reader_regression("ambiguous-generation");
    }

    /// Missing birth evidence cannot prove a competing publication belongs to another lifetime.
    #[test]
    fn reader_rejects_an_unverifiable_competing_generation() {
        reader_regression("unverifiable-generation");
    }

    /// Excluded writers remain live for cleanup and nested capture membership.
    #[test]
    fn reader_excludes_a_live_command_without_sweeping_its_capture() {
        reader_regression("excluded");
    }

    /// Another live pid cannot adopt the identity copied from a published record.
    #[test]
    fn reader_rejects_another_live_processes_registration_identity() {
        reader_regression("forged");
    }

    /// A fresh kernel absence permits staging cleanup while unknown and live records stay.
    #[test]
    fn reader_removes_ended_staging_and_preserves_unknown_and_live_staging() {
        reader_regression("staging");
    }

    /// Production parsing and kernel verification accept the actual writer's bytes.
    #[test]
    fn reader_accepts_live_writer_under_different_locale_and_timezone() {
        reader_regression("locale");
    }
}
