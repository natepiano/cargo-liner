use tui_pane::PaneFocusState;

use crate::http::RateLimitQuota;
use crate::tui::panes;
use crate::tui::panes::GitCheckout;
use crate::tui::panes::GitData;
use crate::tui::panes::HeadRelation;
use crate::tui::panes::PaneId;
use crate::tui::state::AvailabilityStatus;
use crate::tui::test_support;

#[test]
fn git_selected_field_fills_the_whole_row() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.panes.git.set_content(GitData {
        checkout: GitCheckout::Branch {
            name:     "fleet".to_string(),
            relation: HeadRelation::AwaitingRepoProbe,
        },
        ..crate::tui::panes::GitData::default()
    });
    app.panes.git.focus.pane_focus_state = PaneFocusState::Active;
    test_support::draw_pane(&mut app, PaneId::Git, 40)?;
    app.panes.git.viewport.down();
    let buffer = test_support::draw_pane(&mut app, PaneId::Git, 40)?;
    let branch = test_support::position(&buffer, "Branch")?;
    assert_eq!(
        buffer[(buffer.area.right() - 2, branch.1)].bg,
        buffer[branch].bg
    );
    Ok(())
}

#[test]
fn git_rate_rows_keep_the_remaining_count_when_the_quota_does_not_fit() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.panes.git.set_content(GitData {
        rate_limit_core: Some(RateLimitQuota {
            used:      4770,
            limit:     5000,
            remaining: 230,
            reset_at:  None,
        }),
        ..crate::tui::panes::GitData::default()
    });
    let buffer = test_support::draw_pane(&mut app, PaneId::Git, 18)?;
    let (_, y) = test_support::position(&buffer, "Core")?;
    let row = &test_support::rows(&buffer)[usize::from(y)];
    assert!(row.contains("230"), "{row}");
    assert!(!row.contains('/'), "{row}");
    Ok(())
}

#[test]
fn git_reset_countdown_has_readable_contrast() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.panes.git.set_content(GitData {
        github_status: AvailabilityStatus::Reachable,
        rate_limit_core: Some(RateLimitQuota {
            used:      4770,
            limit:     5000,
            remaining: 230,
            reset_at:  Some(0),
        }),
        ..crate::tui::panes::GitData::default()
    });
    let theme = test_support::shipped_dark_theme()?;
    let previous = tui_pane::theme();
    tui_pane::set_active_theme(std::sync::Arc::new(theme));
    let buffer = test_support::draw_pane(&mut app, PaneId::Git, 80)?;
    tui_pane::set_active_theme(previous);
    let failures = test_support::contrast_failures(&buffer, &["resets"], "Git countdown");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    Ok(())
}

#[test]
fn selected_git_countdown_keeps_selection_ink_on_both_fills() -> anyhow::Result<()> {
    let previous = tui_pane::theme();
    tui_pane::set_active_theme(std::sync::Arc::new(test_support::shipped_dark_theme()?));
    let result = (|| -> anyhow::Result<()> {
        for focus in [PaneFocusState::Active, PaneFocusState::Remembered] {
            let mut app = test_support::project_app();
            app.panes.git.set_content(GitData {
                github_status: AvailabilityStatus::Reachable,
                rate_limit_core: Some(RateLimitQuota {
                    used:      30,
                    remaining: 4970,
                    limit:     5000,
                    reset_at:  Some(0),
                }),
                ..GitData::default()
            });
            app.panes.git.focus.pane_focus_state = focus;
            test_support::draw_pane(&mut app, PaneId::Git, 80)?;
            let fields = panes::git_fields_from_data(
                app.panes
                    .git
                    .content()
                    .ok_or_else(|| anyhow::anyhow!("Git content"))?,
            );
            let index = fields
                .iter()
                .position(|field| field.label() == "Core")
                .ok_or_else(|| anyhow::anyhow!("Core field"))?;
            app.panes.git.viewport.set_pos(index);
            let buffer = test_support::draw_pane(&mut app, PaneId::Git, 80)?;
            let failures =
                test_support::contrast_failures(&buffer, &["resets"], "selected countdown");
            assert!(failures.is_empty(), "{}", failures.join("\n"));
            let label = test_support::position(&buffer, "Core")?;
            let reset = test_support::position(&buffer, "resets")?;
            assert_eq!(buffer[label].fg, buffer[reset].fg);
            assert_eq!(buffer[label].bg, buffer[reset].bg);
        }
        Ok(())
    })();
    tui_pane::set_active_theme(previous);
    result
}
