use anyhow::Context;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use tui_pane::ColorMode;
use tui_pane::PaneBorders;
use unicode_width::UnicodeWidthStr;

use crate::ci::CiJob;
use crate::ci::CiRun;
use crate::ci::CiStatus;
use crate::tui::panes::CiData;
use crate::tui::panes::CiEmptyState;
use crate::tui::panes::PaneId;
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
fn ci_placeholder_marks_its_cut_text() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.ci.set_content(CiData {
        empty_state:    CiEmptyState::NoWorkflowConfigured,
        runs:           Vec::new(),
        mode_label:     None,
        current_branch: None,
    });
    let buffer = test_support::draw_pane(&mut app, PaneId::CiRuns, 18)?;
    let text = test_support::rows(&buffer).join("\n");
    assert!(
        text.contains("No CI workflow configured") || text.contains("No CI workflow…"),
        "{text}"
    );
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
    for width in [80, 160] {
        for mode in ["branch", "all"] {
            let mut app = test_support::project_app();
            app.framework.color_mode = ColorMode::Colored;
            let mut run = display_run("fleet")?;
            run.wall_clock_secs = Some(297);
            run.ci_status = CiStatus::Failed;
            app.ci.set_content(CiData {
                runs:           vec![run],
                mode_label:     Some(mode.to_string()),
                current_branch: Some("fleet".to_string()),
                empty_state:    CiEmptyState::NoRuns,
            });
            let buffer = test_support::draw_ui(&mut app, width)?;
            assert_total_header_edge(&buffer, app.panes.tiled_layout.area(PaneId::CiRuns))?;
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
