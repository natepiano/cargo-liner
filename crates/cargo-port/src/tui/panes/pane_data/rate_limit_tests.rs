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
use super::QuotaObservation;
use super::WorktreeInfo;
use super::copy_payload_for_git;
use super::git_row_at;
use crate::http::GitHubRateLimit;
use crate::http::RateLimitQuota;
use crate::project::Comparison;
use crate::project::Package;
use crate::project::RootItem;
use crate::project::RustProject;
use crate::tui::app_render_state::OverlayRenderInputs;
use crate::tui::panes::DescriptionState;
use crate::tui::panes::GitData;
use crate::tui::panes::PaneId;
use crate::tui::panes::SyncedDescriptionHeight;
use crate::tui::state::AvailabilityStatus;
use crate::tui::test_support;

#[test]
fn non_git_pane_keeps_its_title_when_quotas_arrive() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let item = RootItem::Rust(RustProject::Package(Package {
        path: root.path().into(),
        name: Some("without-git".to_string()),
        ..Package::default()
    }));
    for width in [80, 95] {
        let mut app = test_support::make_app(std::slice::from_ref(&item));
        app.framework.color_mode = ColorMode::Colored;
        app.panes
            .git
            .set_content(super::build_pane_data(&app, &item).git);
        let before = test_support::rows(&test_support::draw_pane(&mut app, PaneId::Git, width)?);
        assert!(before[0].contains(" Git "), "{before:?}");
        assert!(
            before[1..before.len() - 1]
                .iter()
                .all(|row| row.trim_matches('│').trim().is_empty()),
            "{before:?}"
        );
        let quota = RateLimitQuota {
            limit:     5000,
            used:      261,
            remaining: 4739,
            reset_at:  None,
        };
        app.net
            .http_client()
            .store_rate_limit_for_test(GitHubRateLimit {
                core:    Some(quota),
                graphql: Some(quota),
            });
        app.panes
            .git
            .set_content(super::build_pane_data(&app, &item).git);
        let after = test_support::rows(&test_support::draw_pane(&mut app, PaneId::Git, width)?);
        assert_eq!(before[0], after[0], "{after:?}");
        for label in ["Core", "GraphQL"] {
            let row = after
                .iter()
                .find(|row| row.contains(label))
                .context("observed quota row")?;
            assert!(row.contains("4739/5000"), "{after:?}");
            assert!(!before.iter().any(|row| row.contains(label)), "{before:?}");
        }
    }
    Ok(())
}

#[test]
fn quota_rows_wait_for_each_bucket_and_keep_data_after_failure() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    std::fs::write(
        root.path().join("Cargo.toml"),
        "[package]\nname = \"quota-display\"\nversion = \"0.1.0\"\n",
    )?;
    let item = RootItem::Rust(RustProject::Package(Package {
        path: root.path().into(),
        name: Some("quota-display".to_string()),
        ..crate::project::Package::default()
    }));
    let quota = |remaining| RateLimitQuota {
        used: 5000 - remaining,
        remaining,
        limit: 5000,
        reset_at: Some(1),
    };
    for width in [80, 95] {
        let mut app = test_support::make_app(std::slice::from_ref(&item));
        app.framework.color_mode = ColorMode::Colored;
        for step in 0..4 {
            match step {
                0 => {},
                1 => app
                    .net
                    .http_client()
                    .store_rate_limit_for_test(GitHubRateLimit {
                        core:    Some(quota(4739)),
                        graphql: None,
                    }),
                2 => app
                    .net
                    .http_client()
                    .store_rate_limit_for_test(GitHubRateLimit {
                        core:    Some(quota(4739)),
                        graphql: Some(quota(4821)),
                    }),
                _ => {
                    app.net.github.availability.mark_unreachable();
                },
            }
            let data = super::build_pane_data(&app, &item);
            app.panes.git.set_content(data.git);
            let rows = test_support::rows(&test_support::draw_pane(&mut app, PaneId::Git, width)?);
            let expected = match step {
                0 => [QuotaObservation::Unobserved; 2],
                1 => [
                    QuotaObservation::Observed(quota(4739)),
                    QuotaObservation::Unobserved,
                ],
                _ => [
                    QuotaObservation::Observed(quota(4739)),
                    QuotaObservation::Observed(quota(4821)),
                ],
            };
            for ((label, value), observation) in [("Core", "4739/5000"), ("GraphQL", "4821/5000")]
                .into_iter()
                .zip(expected)
            {
                let row = rows.iter().find(|row| row.contains(label));
                match observation {
                    QuotaObservation::Observed(_) => {
                        let row = row.context("observed quota row")?;
                        assert!(row.contains(value), "{rows:?}");
                    },
                    QuotaObservation::Unobserved => assert!(row.is_none(), "{rows:?}"),
                }
            }
            if step == 3 {
                assert!(
                    rows.iter().any(|row| row.contains("GitHub unreachable")),
                    "{rows:?}"
                );
            }
        }
    }
    Ok(())
}

#[test]
fn narrow_quota_values_keep_whole_labels_above_whole_counts() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.framework.color_mode = ColorMode::Colored;
    let quota = |remaining| RateLimitQuota {
        used: 5000 - remaining,
        remaining,
        limit: 5000,
        reset_at: Some(u64::MAX),
    };
    app.panes.git.set_content(GitData {
        description: DescriptionState::KnownEmpty,
        github_status: AvailabilityStatus::Reachable,
        rate_limit_core: QuotaObservation::Observed(quota(4848)),
        rate_limit_graphql: QuotaObservation::Observed(quota(4902)),
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
    for (label, value) in [("Core", "4848/5000"), ("GraphQL", "4902/5000")] {
        assert!(
            rows.windows(2)
                .any(|pair| pair[0].trim() == label && pair[1].trim() == value),
            "{rows:?}"
        );
    }
    assert!(
        rows.iter().all(|row| !row.contains('…')),
        "quota values must stay whole: {rows:?}"
    );
    Ok(())
}

#[test]
fn expired_quota_keeps_the_counts_without_a_reset_countdown() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.framework.color_mode = ColorMode::Colored;
    app.panes.git.set_content(GitData {
        description: DescriptionState::KnownEmpty,
        github_status: AvailabilityStatus::Reachable,
        rate_limit_core: QuotaObservation::Observed(RateLimitQuota {
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
        description: DescriptionState::KnownEmpty,
        github_status: AvailabilityStatus::Reachable,
        rate_limit_core: QuotaObservation::Observed(RateLimitQuota {
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
        QuotaObservation::Unobserved,
        QuotaObservation::Observed(RateLimitQuota {
            used:      261,
            remaining: 4739,
            limit:     5000,
            reset_at:  Some(1),
        }),
    ] {
        let data = GitData {
            description: DescriptionState::KnownEmpty,
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
                QuotaObservation::Observed(_) => {
                    let row = rows
                        .iter()
                        .find(|row| row.contains(label))
                        .context("cached quota row")?;
                    assert!(row.contains("4739/5000"), "{row}");
                },
                QuotaObservation::Unobserved => assert!(!text.contains(label), "{text}"),
            }
        }
        assert!(text.contains("checkout"), "{text}");
        let worktree_position = if matches!(cached, QuotaObservation::Observed(_)) {
            3
        } else {
            1
        };
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
