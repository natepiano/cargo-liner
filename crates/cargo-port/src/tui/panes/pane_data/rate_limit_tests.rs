use std::time::Duration;

use anyhow::Context;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use tui_pane::ColorMode;
use tui_pane::CopyLabel;
use tui_pane::CopyPayload;
use tui_pane::CopySelectionResult;
use tui_pane::PaneBorders;
use tui_pane::ResolvedPane;
use tui_pane::ResolvedPaneLayout;

use super::GitRow;
use super::WorktreeInfo;
use super::copy_payload_for_git;
use super::git_row_at;
use crate::http::RateLimitQuota;
use crate::project::Comparison;
use crate::tui::app_render_state::OverlayRenderInputs;
use crate::tui::panes::GitData;
use crate::tui::panes::PaneId;
use crate::tui::panes::SyncedDescriptionHeight;
use crate::tui::state::AvailabilityStatus;
use crate::tui::test_support;

#[test]
fn narrow_quota_values_stay_beside_their_labels() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.framework.color_mode = ColorMode::Colored;
    let quota = |remaining| RateLimitQuota {
        used: 5000 - remaining,
        remaining,
        limit: 5000,
        reset_at: Some(u64::MAX),
    };
    app.panes.git.set_content(GitData {
        github_status: AvailabilityStatus::Reachable,
        rate_limit_core: Some(quota(4848)),
        rate_limit_graphql: Some(quota(4902)),
        ..GitData::default()
    });
    app.ensure_fit_widths_cached();
    app.ensure_disk_cache();
    let lookup = app.ci.status_lookup();
    let mut terminal = Terminal::new(TestBackend::new(80, 30))?;
    let frame = terminal.draw(|frame| {
        let mut split = app.split_for_render(
            None,
            Duration::ZERO,
            &lookup,
            OverlayRenderInputs::none(),
            SyncedDescriptionHeight::default(),
            app.empty_package_description(),
        );
        let layout = ResolvedPaneLayout::new(vec![ResolvedPane {
            pane: PaneId::Git,
            area: Rect::new(63, 0, 17, 15),
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
    let rows: Vec<String> = (1..14)
        .map(|y| (64..79).map(|x| frame.buffer[(x, y)].symbol()).collect())
        .collect();
    for expected in [" Core     4848 ", " GraphQL  4902 "] {
        assert!(rows.iter().any(|row| row == expected), "{rows:?}");
    }
    assert!(
        rows.iter()
            .all(|row| !matches!(row.trim(), "4848" | "4902" | "4848/5000" | "4902/5000")),
        "quota values need their labels on the same row: {rows:?}"
    );
    Ok(())
}

#[test]
fn expired_quota_keeps_the_counts_without_a_reset_countdown() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.framework.color_mode = ColorMode::Colored;
    app.panes.git.set_content(GitData {
        github_status: AvailabilityStatus::Reachable,
        rate_limit_core: Some(RateLimitQuota {
            used:      371,
            remaining: 4629,
            limit:     5000,
            reset_at:  Some(1),
        }),
        ..GitData::default()
    });
    let buffer = test_support::draw_pane(&mut app, PaneId::Git, 80)?;
    let rows = test_support::rows(&buffer);
    let core = rows
        .iter()
        .find(|row| row.contains("Core"))
        .context("Core row")?;
    assert!(core.contains("4629/5000"), "{core}");
    assert!(!core.contains("resets"), "{core}");
    Ok(())
}

#[test]
fn quota_keeps_its_limit_in_a_ninety_five_column_screen() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.framework.color_mode = ColorMode::Colored;
    app.panes.git.set_content(GitData {
        github_status: AvailabilityStatus::Reachable,
        rate_limit_core: Some(RateLimitQuota {
            used:      261,
            remaining: 4739,
            limit:     5000,
            reset_at:  Some(u64::MAX),
        }),
        ..GitData::default()
    });
    let buffer = test_support::draw_ui(&mut app, 95)?;
    let rows = test_support::rows(&buffer);
    let core = rows
        .iter()
        .find(|row| row.contains("Core"))
        .context("Core row")?;
    assert!(core.contains("4739/5000"), "{rows:?}");
    Ok(())
}

#[test]
fn unreachable_quota_rows_draw_only_cached_values() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.framework.color_mode = ColorMode::Colored;
    for cached in [
        None,
        Some(RateLimitQuota {
            used:      261,
            remaining: 4739,
            limit:     5000,
            reset_at:  Some(1),
        }),
    ] {
        let data = GitData {
            github_status: AvailabilityStatus::Unreachable,
            rate_limit_core: cached,
            rate_limit_graphql: cached,
            worktrees: vec![WorktreeInfo {
                name:         "checkout".to_string(),
                path:         "/tmp/checkout".to_string(),
                branch:       Some("topic".to_string()),
                tracked:      None,
                ahead_behind: Comparison::NotCompared,
            }],
            ..GitData::default()
        };
        app.panes.git.set_content(data.clone());
        let rows = test_support::rows(&test_support::draw_pane(&mut app, PaneId::Git, 80)?);
        let text = rows.join("\n");
        assert_eq!(text.matches("GitHub unreachable").count(), 1, "{text}");
        for label in ["Core", "GraphQL"] {
            match cached {
                Some(_) => {
                    let row = rows
                        .iter()
                        .find(|row| row.contains(label))
                        .context("cached quota row")?;
                    assert!(row.contains("4739/5000"), "{row}");
                },
                None => assert!(!text.contains(label), "{text}"),
            }
        }
        assert!(text.contains("checkout"), "{text}");
        let worktree_position = if cached.is_some() { 3 } else { 1 };
        let Some(GitRow::Worktree(worktree)) = git_row_at(&data, worktree_position) else {
            anyhow::bail!("the drawn worktree needs its addressable row");
        };
        assert_eq!(worktree.name, "checkout");
        assert_eq!(
            copy_payload_for_git(&data, worktree_position),
            CopySelectionResult::Payload(CopyPayload::new("/tmp/checkout", CopyLabel::Path)),
        );
    }
    Ok(())
}
