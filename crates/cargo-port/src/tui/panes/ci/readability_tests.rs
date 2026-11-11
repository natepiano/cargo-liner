use anyhow::Context;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use tui_pane::ColorMode;
use tui_pane::PaneBorders;
use unicode_width::UnicodeWidthStr;

use crate::ci;
use crate::ci::CiJob;
use crate::ci::CiRun;
use crate::ci::CiStatus;
use crate::tui::panes::CiData;
use crate::tui::panes::CiEmptyState;
use crate::tui::panes::PaneId;
use crate::tui::render;
use crate::tui::test_support;

#[test]
fn narrow_ci_draws_only_complete_columns() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    let run: CiRun = serde_json::from_value(serde_json::json!({
        "run_id": 1, "created_at": "2026-11-05T12:00:00Z", "branch": "fleet",
        "url": "", "ci_status": "passed", "jobs": [],
        "wall_clock_secs": 60, "commit_title": "Repair the display"
    }))?;
    app.ci.set_content(CiData {
        runs:           vec![run],
        mode_label:     Some("branch".to_string()),
        current_branch: Some("fleet".to_string()),
        empty_state:    CiEmptyState::NoRuns,
    });
    let buffer = test_support::draw_pane(&mut app, PaneId::CiRuns, 24)?;
    let (_, y) = test_support::position(&buffer, "Commit")?;
    let header = &test_support::rows(&buffer)[usize::from(y)];
    assert!(!header.contains("Branch"), "{header}");
    assert!(
        !header.contains("Time"),
        "a partial Timestamp column must be hidden: {header}"
    );
    Ok(())
}

#[test]
fn ci_placeholder_wraps_the_full_message_below_a_whole_title() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.framework.color_mode = ColorMode::Colored;
    app.ci.set_content(CiData {
        empty_state:    CiEmptyState::NoWorkflowConfigured,
        runs:           Vec::new(),
        mode_label:     None,
        current_branch: None,
    });
    let buffer = test_support::draw_pane(&mut app, PaneId::CiRuns, 18)?;
    let rows = test_support::rows(&buffer);
    assert!(rows[0].contains(" CI runs "), "{rows:?}");
    let message = rows[1..rows.len() - 1]
        .iter()
        .map(|row| row.trim_matches(['│', '┃', ' ']))
        .filter(|row| !row.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    assert_eq!(message, "No CI workflow configured");
    Ok(())
}

#[test]
fn ci_hides_branch_when_the_whole_value_cannot_fit() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    let run: CiRun = serde_json::from_value(serde_json::json!({
        "run_id": 1, "created_at": "2026-11-05T12:00:00Z",
        "branch": "cargo-liner-port", "url": "", "ci_status": "passed", "jobs": [],
        "wall_clock_secs": 60, "commit_title": "Repair the display"
    }))?;
    app.ci.set_content(CiData {
        runs:           vec![run],
        mode_label:     None,
        current_branch: None,
        empty_state:    CiEmptyState::NoRuns,
    });
    let buffer = test_support::draw_pane(&mut app, PaneId::CiRuns, 24)?;
    let text = test_support::rows(&buffer).join("\n");
    assert!(!text.contains("Branch"), "{text}");
    assert!(!text.contains("cargo"), "{text}");
    test_support::position(&buffer, "Commit")?;
    Ok(())
}

#[test]
fn branch_ci_gives_narrow_cells_to_commit_and_status() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.framework.color_mode = ColorMode::Colored;
    app.ci.set_content(CiData {
        runs:           vec![display_run("fleet")?],
        mode_label:     Some("branch".to_string()),
        current_branch: Some("fleet".to_string()),
        empty_state:    CiEmptyState::NoRuns,
    });
    let buffer = test_support::draw_pane(&mut app, PaneId::CiRuns, 24)?;
    let text = test_support::rows(&buffer).join("\n");
    assert!(!text.contains("Branch"), "{text}");
    test_support::position(&buffer, "Commit")?;
    test_support::position(&buffer, "Total")?;
    test_support::position(&buffer, "🟢")?;
    for mode in [ColorMode::Colored, ColorMode::NoColor] {
        app.framework.color_mode = mode;
        let mut data = app.ci.content().context("CI runs")?.clone();
        data.mode_label = None;
        data.current_branch = None;
        data.runs[0].wall_clock_secs = Some(297);
        data.runs[0].ci_status = CiStatus::Failed;
        app.ci.set_content(data);
        let buffer = test_support::draw_ui(&mut app, 80)?;
        let area = app.panes.tiled_layout.area(PaneId::CiRuns);
        test_support::find_text_cell(&buffer, area, "Commit").context("whole Commit header")?;
        assert_total_header_edge(&buffer, area)?;
    }
    Ok(())
}

#[test]
fn all_ci_keeps_the_branch_for_runs_from_different_branches() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.ci.set_content(CiData {
        runs:           vec![display_run("fleet")?, display_run("main")?],
        mode_label:     Some("all".to_string()),
        current_branch: Some("fleet".to_string()),
        empty_state:    CiEmptyState::NoRuns,
    });
    let buffer = test_support::draw_pane(&mut app, PaneId::CiRuns, 40)?;
    for text in ["Branch", "fleet", "main"] {
        test_support::position(&buffer, text)?;
    }
    Ok(())
}

fn display_run(branch: &str) -> anyhow::Result<CiRun> {
    Ok(serde_json::from_value(serde_json::json!({
        "run_id": 1, "created_at": "2026-11-05T12:00:00Z", "branch": branch,
        "url": "", "ci_status": "passed", "jobs": [],
        "wall_clock_secs": 60, "commit_title": "Repair the display"
    }))?)
}

#[test]
fn all_ci_keeps_a_truncated_branch_when_names_are_long() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.ci.set_content(CiData {
        runs:           vec![
            display_run("feature/a-long-branch-name")?,
            display_run("main")?,
        ],
        mode_label:     Some("all".to_string()),
        current_branch: None,
        empty_state:    CiEmptyState::NoRuns,
    });
    let buffer = test_support::draw_pane(&mut app, PaneId::CiRuns, 40)?;
    test_support::position(&buffer, "Branch")?;
    test_support::position(&buffer, "main")?;
    let rows = test_support::rows(&buffer);
    assert!(
        rows.iter()
            .any(|row| row.contains("featur") && row.contains('…'))
    );
    Ok(())
}

#[test]
fn branch_ci_keeps_the_whole_duration() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    let mut run = display_run("fleet")?;
    run.wall_clock_secs = Some(256);
    run.ci_status = CiStatus::Failed;
    app.ci.set_content(CiData {
        runs:           vec![run],
        mode_label:     Some("branch".to_string()),
        current_branch: Some("fleet".to_string()),
        empty_state:    CiEmptyState::NoRuns,
    });
    let buffer = test_support::draw_pane(&mut app, PaneId::CiRuns, 40)?;
    test_support::position(&buffer, "4m 16s")?;
    test_support::position(&buffer, "Total")?;
    test_support::position(&buffer, "🔴")?;
    Ok(())
}

#[test]
fn all_ci_keeps_the_whole_status_when_branch_and_status_do_not_fit() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    let mut run = display_run("feature/a-long-branch-name")?;
    run.wall_clock_secs = Some(256);
    run.ci_status = CiStatus::Failed;
    app.ci.set_content(CiData {
        runs:           vec![run],
        mode_label:     Some("all".to_string()),
        current_branch: None,
        empty_state:    CiEmptyState::NoRuns,
    });
    let buffer = test_support::draw_pane(&mut app, PaneId::CiRuns, 18)?;
    let rows = test_support::rows(&buffer);
    assert!(!rows.join("\n").contains("Branch"), "{rows:?}");
    test_support::position(&buffer, "Commit")?;
    test_support::position(&buffer, "Total")?;
    let (_, y) = test_support::position(&buffer, "4m 16s")?;
    assert!(rows[usize::from(y)].contains("4m 16s 🔴"), "{rows:?}");
    Ok(())
}

fn assert_total_header_edge(buffer: &Buffer, area: Rect) -> anyhow::Result<()> {
    let (header_x, _) =
        test_support::find_text_cell(buffer, area, "Total").context("Total header")?;
    let value = "4m 57s 🔴";
    let (value_x, _) = test_support::find_text_cell(buffer, area, value).with_context(|| {
        format!(
            "whole duration and status: {:?}",
            test_support::rows(buffer)
        )
    })?;
    assert_eq!(
        usize::from(header_x) + "Total".width(),
        usize::from(value_x) + value.width()
    );
    Ok(())
}

#[test]
fn ci_total_header_aligns_with_duration_and_status_at_eighty_and_wide() -> anyhow::Result<()> {
    for width in [80, 95, 160] {
        for mode in ["branch", "all"] {
            let mut app = test_support::project_app();
            app.framework.color_mode = ColorMode::Colored;
            let mut run = display_run("fleet")?;
            run.wall_clock_secs = Some(297);
            run.ci_status = CiStatus::Failed;
            run.commit_title = Some("phase cargo tools ".repeat(12));
            app.ci.set_content(CiData {
                runs:           vec![run],
                mode_label:     Some(mode.to_string()),
                current_branch: Some("fleet".to_string()),
                empty_state:    CiEmptyState::NoRuns,
            });
            let buffer = test_support::draw_ui(&mut app, width)?;
            assert_total_header_edge(&buffer, app.panes.tiled_layout.area(PaneId::CiRuns))?;
            let area = app.panes.tiled_layout.area(PaneId::CiRuns);
            test_support::assert_ellipses_follow_text(&buffer, area);
        }
    }
    Ok(())
}

#[test]
fn status_only_ci_total_header_aligns_in_an_eighty_column_frame() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.framework.color_mode = ColorMode::Colored;
    let mut run = display_run("fleet")?;
    run.wall_clock_secs = Some(297);
    run.ci_status = CiStatus::Failed;
    app.ci.set_content(CiData {
        runs:           vec![run],
        mode_label:     Some("all".to_string()),
        current_branch: None,
        empty_state:    CiEmptyState::NoRuns,
    });
    let area = Rect::new(68, 0, 12, 10);
    let lookup = app.ci.status_lookup();
    let mut terminal = Terminal::new(TestBackend::new(80, 30))?;
    terminal.draw(|frame| {
        let mut split = app.split_for_render(
            None,
            std::time::Duration::ZERO,
            &lookup,
            crate::tui::app_render_state::OverlayRenderInputs::none(),
            crate::tui::panes::SyncedDescriptionHeight::default(),
            app.empty_package_description(),
        );
        let layout = tui_pane::ResolvedPaneLayout::new(vec![tui_pane::ResolvedPane {
            pane: PaneId::CiRuns,
            area,
        }]);
        tui_pane::render_panes(
            frame,
            &mut split.registry,
            &layout,
            &split.pane_render_ctx,
            tui_pane::default_pane_chrome(ColorMode::Colored),
            PaneBorders::Separate,
        );
    })?;
    let buffer = terminal.backend().buffer();
    assert!(test_support::find_text_cell(buffer, area, "Commit").is_none());
    assert!(test_support::find_text_cell(buffer, area, "Branch").is_none());
    assert_total_header_edge(buffer, area)?;
    Ok(())
}

#[test]
fn phase4e_ci_total_does_not_understate_a_job_duration() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.framework.color_mode = ColorMode::Colored;
    let mut run = display_run("fleet")?;
    run.wall_clock_secs = Some(256);
    run.jobs = (0..8)
        .map(|index| CiJob {
            name:          format!("test-{index}"),
            ci_status:     CiStatus::Passed,
            duration:      "1m 45s".to_string(),
            duration_secs: Some(105),
        })
        .collect();
    app.ci.set_content(CiData {
        runs:           vec![run],
        mode_label:     Some("all".to_string()),
        current_branch: None,
        empty_state:    CiEmptyState::NoRuns,
    });
    let buffer = test_support::draw_pane(&mut app, PaneId::CiRuns, 80)?;
    let rows = test_support::rows(&buffer);
    if let Some((x, y)) = test_support::find_text_cell(&buffer, buffer.area, "4m 16s") {
        let header: String = (x.saturating_sub(2)..buffer.area.right() - 1)
            .map(|column| buffer[(column, y - 1)].symbol())
            .collect();
        assert!(
            header.contains("Elapsed") || header.contains("Wall"),
            "the elapsed duration needs an accurate label: {rows:?}"
        );
    } else {
        test_support::position(&buffer, "Total")?;
        test_support::position(&buffer, "14m  0s")?;
    }
    Ok(())
}

fn job(name: &str, seconds: u64) -> CiJob {
    CiJob {
        name:          name.to_string(),
        ci_status:     CiStatus::Passed,
        duration:      ci::format_secs(seconds),
        duration_secs: Some(seconds),
    }
}

fn ci_area_rows(buffer: &Buffer, area: Rect) -> Vec<String> {
    (area.y..area.bottom())
        .map(|y| {
            (area.x..area.right())
                .map(|x| buffer[(x, y)].symbol())
                .collect()
        })
        .collect()
}

#[test]
fn wide_ci_job_headers_take_cells_from_the_commit_column() -> anyhow::Result<()> {
    for (windows_job, mixed_jobs) in [
        ("Windows Build", false),
        ("Windows Build Tests", false),
        ("Windows Build", true),
        ("Windows Build Tests", true),
    ] {
        let mut app = test_support::project_app();
        app.framework.color_mode = ColorMode::Colored;
        let mut run = display_run("main")?;
        run.commit_title = Some("phase 32d — move the tile cache into its own crate".to_string());
        run.jobs = vec![job(windows_job, 1489), job("Linux Build", 905)];
        if mixed_jobs {
            run.jobs.extend([
                job("Taplo Check", 7),
                job("Test Suite", 12),
                job("Other", 3),
            ]);
        }
        app.ci.set_content(CiData {
            runs:           vec![run],
            mode_label:     Some("all".to_string()),
            current_branch: None,
            empty_state:    CiEmptyState::NoRuns,
        });
        let (buffer, area) = if mixed_jobs {
            let buffer = test_support::draw_pane_in_window(&mut app, PaneId::CiRuns, 160, 107)?;
            let area = buffer.area;
            (buffer, area)
        } else {
            let buffer = test_support::draw_ui(&mut app, 160)?;
            let area = app.panes.tiled_layout.area(PaneId::CiRuns);
            (buffer, area)
        };
        let rows = ci_area_rows(&buffer, area);
        let header = rows
            .iter()
            .find(|row| row.contains(" Commit"))
            .context("CI header row")?;
        assert!(header.contains(windows_job), "{rows:?}");
        assert!(header.contains("Linux Build"), "{rows:?}");
        assert!(!header.contains('…'), "{rows:?}");
    }
    Ok(())
}

#[test]
fn a_cut_commit_drops_the_separator_before_the_ellipsis() -> anyhow::Result<()> {
    let cut = render::truncate_with_ellipsis("phase 32d — move the tile cache", 13, "…");
    assert_eq!(cut, "phase 32d…");
    for (title, forbidden) in [
        ("plan(remove-running-section): split the phase", "-…"),
        ("plan(remove_running_section): split the phase", "_…"),
        ("plan(remove/running/section): split the phase", "/…"),
        ("plan(remove|running|section): split the phase", "|…"),
    ] {
        let mut app = test_support::project_app();
        let mut run = display_run("main")?;
        run.commit_title = Some(title.to_string());
        run.wall_clock_secs = Some(1505);
        run.jobs.clear();
        app.ci.set_content(CiData {
            runs:           vec![run],
            mode_label:     Some("all".to_string()),
            current_branch: None,
            empty_state:    CiEmptyState::NoRuns,
        });
        let buffer = test_support::draw_pane_in_window(&mut app, PaneId::CiRuns, 80, 35)?;
        let rows = test_support::rows(&buffer);
        let row = rows
            .iter()
            .find(|row| row.contains("plan("))
            .context("Commit row")?;
        assert!(!row.contains(forbidden), "{row}");
        assert!(row.contains("plan(remove…"), "{row}");
    }
    Ok(())
}

#[test]
fn narrow_ci_status_rows_keep_one_cell_after_the_left_border() -> anyhow::Result<()> {
    for mode in [ColorMode::Colored, ColorMode::NoColor] {
        let mut app = test_support::project_app();
        app.framework.color_mode = mode;
        let mut run = display_run("main")?;
        run.wall_clock_secs = Some(1489);
        run.ci_status = CiStatus::Failed;
        app.ci.set_content(CiData {
            runs:           vec![run],
            mode_label:     Some("all".to_string()),
            current_branch: None,
            empty_state:    CiEmptyState::NoRuns,
        });
        let buffer = test_support::draw_ui(&mut app, 60)?;
        let area = app.panes.tiled_layout.area(PaneId::CiRuns);
        let rows = ci_area_rows(&buffer, area);
        for row in &rows[1..rows.len() - 1] {
            let inside: Vec<char> = row.chars().skip(1).collect();
            if inside.iter().any(|cell| !cell.is_whitespace()) {
                assert_eq!(inside.first(), Some(&' '), "{mode:?}: {rows:?}");
            }
        }
        test_support::find_text_cell(&buffer, area, "24m 49s").context("whole duration")?;
    }
    Ok(())
}

#[test]
fn ci_titles_drop_suffixes_before_the_count() -> anyhow::Result<()> {
    for (window, pane_width) in [(60, 15), (80, 34), (160, 80)] {
        for mode in [ColorMode::Colored, ColorMode::NoColor] {
            let mut app = test_support::project_app();
            app.framework.color_mode = mode;
            app.ci.set_content(CiData {
                runs:           vec![display_run("main")?; 48],
                mode_label:     Some("branch".to_string()),
                current_branch: Some("main".to_string()),
                empty_state:    CiEmptyState::NoRuns,
            });
            for focused in [false, true] {
                app.set_focus_to_pane(if focused {
                    PaneId::CiRuns
                } else {
                    PaneId::ProjectList
                });
                app.ci.focus.pane_focus_state = app.pane_focus_state(PaneId::CiRuns);
                let buffer = test_support::draw_pane_in_window(
                    &mut app,
                    PaneId::CiRuns,
                    window,
                    pane_width,
                )?;
                let title = &test_support::rows(&buffer)[0];
                let expected = if window != 60 && focused {
                    "CI runs (1 of 48)"
                } else {
                    "CI runs (48)"
                };
                assert!(title.contains(expected), "{window}: {title}");
                assert!(!title.contains('…'), "{title}");
                if window == 60 {
                    assert!(!title.contains("branch"), "{title}");
                    assert_eq!(test_support::position(&buffer, "CI runs (48)")?, (1, 0));
                } else {
                    assert!(title.contains("branch: main"), "{title}");
                }
            }
        }
    }
    Ok(())
}
