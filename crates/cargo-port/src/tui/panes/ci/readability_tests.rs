use crate::ci::CiRun;
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
        mode_label:     None,
        current_branch: None,
        empty_state:    CiEmptyState::NoRuns,
    });
    let buffer = test_support::draw_pane(&mut app, PaneId::CiRuns, 24)?;
    let (_, y) = test_support::position(&buffer, "Commit")?;
    let header = &test_support::rows(&buffer)[usize::from(y)];
    assert!(header.contains("Branch"), "{header}");
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
