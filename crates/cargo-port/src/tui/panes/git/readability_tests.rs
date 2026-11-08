use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use anyhow::Context;
use ratatui::layout::Position;
use tui_pane::ColorMode;
use tui_pane::Hittable;
use tui_pane::PaneFocusState;

use super::sync_column_tests;
use crate::http::RateLimitQuota;
use crate::project::AheadBehind;
use crate::project::CommitDate;
use crate::project::Comparison;
use crate::project::FirstCommit;
use crate::project::GitStatus;
use crate::project::TrackedRef;
use crate::tui::panes;
use crate::tui::panes::DescriptionState;
use crate::tui::panes::DetailField;
use crate::tui::panes::GitCheckout;
use crate::tui::panes::GitData;
use crate::tui::panes::HeadRelation;
use crate::tui::panes::PaneId;
use crate::tui::panes::pane_data::GitProbe;
use crate::tui::panes::pane_data::PullRequestSection;
use crate::tui::panes::pane_data::PullRequestSectionState;
use crate::tui::panes::pane_data::QuotaObservation;
use crate::tui::panes::pane_data::RemoteRow;
use crate::tui::panes::pane_data::RemoteRows;
use crate::tui::panes::pane_data::WorktreeInfo;
use crate::tui::state::AvailabilityStatus;
use crate::tui::test_support;

#[test]
fn pull_request_titles_show_only_known_counts_and_keep_stale_counts() -> anyhow::Result<()> {
    for (window, width) in [(80, 34), (160, 50)] {
        let mut app = test_support::project_app();
        app.framework.color_mode = ColorMode::Colored;
        for state in [
            PullRequestSectionState::Loading,
            PullRequestSectionState::Unavailable,
            PullRequestSectionState::Loaded,
            PullRequestSectionState::Stale,
        ] {
            let known = matches!(
                state,
                PullRequestSectionState::Loaded | PullRequestSectionState::Stale
            );
            app.panes.git.set_content(GitData {
                checkout: GitCheckout::Branch {
                    name:     "main".to_string(),
                    relation: HeadRelation::Default,
                },
                pull_requests: PullRequestSection {
                    state,
                    rows: if known {
                        vec![crate::tui::panes::PullRequestRow {
                            number:      7,
                            title:       "Keep the known count".to_string(),
                            url:         "https://github.com/acme/repo/pull/7".to_string(),
                            state_label: "open",
                            polling:     crate::tui::panes::PullRequestPolling::Idle,
                            branch:      "topic".to_string(),
                            base:        "main".to_string(),
                        }]
                    } else {
                        Vec::new()
                    },
                    ..PullRequestSection::default()
                },
                ..GitData::default()
            });
            let buffer = test_support::draw_pane_in_window(&mut app, PaneId::Git, window, width)?;
            let rows = test_support::rows(&buffer);
            let title = rows
                .iter()
                .find(|row| row.contains("Pull Requests"))
                .context("Pull Requests title")?;
            assert!(!title.contains("(0)"), "{state:?}: {rows:?}");
            assert_eq!(title.contains("(1)"), known, "{state:?}: {rows:?}");
        }
    }
    Ok(())
}

#[test]
fn cut_worktree_and_remote_values_end_with_an_ellipsis() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.framework.color_mode = ColorMode::Colored;
    app.panes.git.set_content(GitData {
        description: DescriptionState::KnownEmpty,
        worktrees: ["verify-run-long", "sdlc-merge-long"]
            .into_iter()
            .map(|branch| WorktreeInfo {
                name:         "proof_crate".to_string(),
                path:         format!("/tmp/{branch}"),
                branch:       Some(branch.to_string()),
                tracked:      None,
                ahead_behind: Comparison::NotCompared,
            })
            .collect(),
        remotes: RemoteRows::Listed(vec![RemoteRow {
            name:            "origin".to_string(),
            icon:            "",
            display_url:     "github.com/nate/proof".to_string(),
            branch:          "master".to_string(),
            tracked_ref:     TrackedRef::Untracked,
            status:          Comparison::NotCompared,
            full_url:        None,
            push_annotation: None,
        }]),
        ..GitData::default()
    });
    for width in [14, 17] {
        let buffer = test_support::draw_pane(&mut app, PaneId::Git, width)?;
        let worktrees = test_support::position(&buffer, "Worktr")?;
        if width == 17 {
            test_support::position(&buffer, "Worktrees")?;
        }
        for (offset, branch) in ["verify-run-long", "sdlc-merge-long"]
            .into_iter()
            .enumerate()
        {
            sync_column_tests::assert_fitted_column(
                &buffer,
                worktrees.1 + 1,
                worktrees.1 + 2 + u16::try_from(offset)?,
                1,
                branch,
            )?;
        }
        let remotes = test_support::position(&buffer, "Remotes")?;
        sync_column_tests::assert_fitted_column(
            &buffer,
            remotes.1 + 1,
            remotes.1 + 2,
            0,
            "origin",
        )?;
    }
    Ok(())
}

#[test]
fn git_selected_field_fills_the_whole_row() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.panes.git.set_content(GitData {
        description: DescriptionState::KnownEmpty,
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
        description: DescriptionState::KnownEmpty,
        rate_limit_core: QuotaObservation::Observed(RateLimitQuota {
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
        description: DescriptionState::KnownEmpty,
        github_status: AvailabilityStatus::Reachable,
        rate_limit_core: QuotaObservation::Observed(RateLimitQuota {
            used:      4770,
            limit:     5000,
            remaining: 230,
            reset_at:  Some(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() + 3600),
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
                description: DescriptionState::KnownEmpty,
                github_status: AvailabilityStatus::Reachable,
                rate_limit_core: QuotaObservation::Observed(RateLimitQuota {
                    used:      30,
                    remaining: 4970,
                    limit:     5000,
                    reset_at:  Some(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() + 3600),
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

#[test]
fn narrow_git_fields_keep_labels_values_and_dates() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.framework.color_mode = ColorMode::Colored;
    app.panes.git.set_content(GitData {
        description: DescriptionState::KnownEmpty,
        checkout: GitCheckout::Branch {
            name:     "master".to_string(),
            relation: HeadRelation::Default,
        },
        status: Some(GitStatus::Clean),
        inception: FirstCommit::Probed(CommitDate::At("2026-11-06 08:21".to_string())),
        last_commit: GitProbe::Probed(CommitDate::At("2026-11-06 15:13".to_string())),
        rate_limit_graphql: QuotaObservation::Observed(RateLimitQuota {
            used:      374,
            remaining: 4626,
            limit:     5000,
            reset_at:  None,
        }),
        ..GitData::default()
    });
    let buffer = test_support::draw_pane(&mut app, PaneId::Git, 17)?;
    for text in [
        "Status",
        "✨ clean",
        "Branch",
        "master",
        "GraphQL",
        "4626",
        "Incept",
        "Latest",
        "2026-11-06",
    ] {
        test_support::position(&buffer, text)?;
    }
    let status = test_support::position(&buffer, "Status")?;
    let clean = test_support::position(&buffer, "✨ clean")?;
    assert_eq!(clean.1, status.1 + 1);
    Ok(())
}

#[test]
fn wrapped_git_values_share_selection_and_row_hits() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.framework.color_mode = ColorMode::Colored;
    app.panes.git.set_content(GitData {
        description: DescriptionState::KnownEmpty,
        status: Some(GitStatus::Clean),
        ..GitData::default()
    });
    app.panes.git.focus.pane_focus_state = PaneFocusState::Active;
    let fields = panes::git_fields_from_data(app.panes.git.content().context("Git content")?);
    let index = fields
        .iter()
        .position(|field| field.label() == "Status")
        .context("Status field")?;
    app.panes.git.viewport.set_pos(index);
    let buffer = test_support::draw_pane(&mut app, PaneId::Git, 17)?;
    let label = test_support::position(&buffer, "Status")?;
    let value = test_support::position(&buffer, "✨ clean")?;
    assert_eq!(value.1, label.1 + 1);
    assert_eq!(buffer[label].bg, buffer[value].bg);
    assert_eq!(buffer[label].fg, buffer[value].fg);
    for (x, y) in [label, value] {
        assert!(matches!(app.panes.git.hit_test_at(Position::new(x, y)),
            Some(crate::tui::hit_test::HoverTarget::PaneRow { pane: PaneId::Git, row }) if row == index));
    }
    Ok(())
}

#[test]
fn narrow_worktree_table_keeps_its_title_and_branch_with_a_remote() -> anyhow::Result<()> {
    for with_remote in [false, true] {
        let mut app = test_support::project_app();
        app.framework.color_mode = ColorMode::Colored;
        let remote = RemoteRow {
            name:            "origin".to_string(),
            icon:            "",
            display_url:     "github.com/nate/proof".to_string(),
            branch:          "master".to_string(),
            tracked_ref:     TrackedRef::Untracked,
            status:          Comparison::NotCompared,
            full_url:        None,
            push_annotation: None,
        };
        let data = GitData {
            description: DescriptionState::KnownEmpty,
            worktrees: ["master", "verify-long", "sdlc-merge"]
                .into_iter()
                .map(|branch| WorktreeInfo {
                    name:         "proof_crate".to_string(),
                    path:         "/tmp/proof".to_string(),
                    branch:       Some(branch.to_string()),
                    tracked:      None,
                    ahead_behind: Comparison::NotCompared,
                })
                .collect(),
            remotes: RemoteRows::Listed(if with_remote {
                vec![remote.clone()]
            } else {
                vec![]
            }),
            ..GitData::default()
        };
        app.panes.git.set_content(data);
        let buffer = test_support::draw_pane(&mut app, PaneId::Git, 17)?;
        test_support::position(&buffer, "├ Worktrees (3) ┤")?;
        let title = test_support::position(&buffer, "Worktrees")?;
        for (offset, branch) in ["master", "verify-long", "sdlc-merge"]
            .into_iter()
            .enumerate()
        {
            sync_column_tests::assert_worktree_columns(
                &buffer,
                buffer.area,
                title.1 + 1,
                title.1 + 2 + u16::try_from(offset)?,
                ["proof_crate", branch, "──", "──"],
            )?;
        }
        for y in 1..buffer.area.bottom() - 1 {
            assert!(matches!(buffer[(16, y)].symbol(), "│" | "┤"));
        }
        if with_remote {
            let remotes = test_support::position(&buffer, "Remotes")?;
            sync_column_tests::assert_shared_remote_columns(
                &buffer,
                buffer.area,
                remotes.1 + 1,
                title.1 + 1,
            )?;
        }
    }
    Ok(())
}

#[test]
fn wrapped_worktree_error_continuation_selects_its_field() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.framework.color_mode = ColorMode::Colored;
    app.panes.git.set_content(GitData {
        description: DescriptionState::KnownEmpty,
        checkout: GitCheckout::Broken,
        status: Some(GitStatus::Clean),
        ..GitData::default()
    });
    app.panes.git.focus.pane_focus_state = PaneFocusState::Active;
    let fields = panes::git_fields_from_data(app.panes.git.content().context("Git content")?);
    let index = fields
        .iter()
        .position(|field| *field == DetailField::WorktreeError)
        .context("Worktree error field")?;
    let buffer = test_support::draw_pane(&mut app, PaneId::Git, 40)?;
    let label = test_support::position(&buffer, "Error")?;
    let continuation = test_support::position(&buffer, "missing")?;
    assert_eq!(continuation.1, label.1 + 1);
    let position = Position::new(continuation.0, continuation.1);
    assert!(matches!(app.panes.git.hit_test_at(position),
        Some(crate::tui::hit_test::HoverTarget::PaneRow { pane: PaneId::Git, row }) if row == index));
    app.panes.git.viewport.set_pos(index + 1);
    assert!(crate::tui::interaction::handle_click(
        &mut app,
        position,
        crate::tui::interaction::ClickMode::Dispatch,
    ));
    assert_eq!(app.panes.git.viewport.pos(), index);
    let selected = test_support::draw_pane(&mut app, PaneId::Git, 40)?;
    assert_eq!(selected[label].bg, selected[continuation].bg);
    assert_eq!(selected[label].fg, selected[continuation].fg);
    Ok(())
}

#[test]
fn worktree_branches_keep_an_ellipsis_at_medium_widths() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.framework.color_mode = ColorMode::Colored;
    app.panes.git.set_content(GitData {
        description: DescriptionState::KnownEmpty,
        worktrees: ["verify-run-place", "sdlc-merge-gate"]
            .into_iter()
            .map(|branch| WorktreeInfo {
                name:         "proof_crate".to_string(),
                path:         format!("/tmp/{branch}"),
                branch:       Some(branch.to_string()),
                tracked:      None,
                ahead_behind: Comparison::NotCompared,
            })
            .collect(),
        ..GitData::default()
    });
    for width in [22, 24] {
        let buffer = test_support::draw_pane(&mut app, PaneId::Git, width)?;
        let rows = test_support::rows(&buffer);
        let title = test_support::position(&buffer, "Worktrees")?;
        for (offset, branch) in ["verify-run-place", "sdlc-merge-gate"]
            .into_iter()
            .enumerate()
        {
            let y = title.1 + 2 + u16::try_from(offset)?;
            sync_column_tests::assert_worktree_columns(
                &buffer,
                buffer.area,
                title.1 + 1,
                y,
                ["proof_crate", branch, "──", "──"],
            )?;
            let row = &rows[usize::from(y)];
            let values: Vec<_> = row.trim_matches('│').split_whitespace().collect();
            assert!(values[1].ends_with('…'), "{row}");
            assert_eq!(&values[2..], &["──", "──"], "{row}");
            assert_eq!(buffer[(width - 2, y)].symbol(), " ");
        }
    }
    Ok(())
}

#[test]
fn remote_track_cuts_before_the_right_margin() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.framework.color_mode = ColorMode::Colored;
    app.panes.git.set_content(GitData {
        description: DescriptionState::KnownEmpty,
        remotes: RemoteRows::Listed(vec![RemoteRow {
            name:            "origin".to_string(),
            icon:            "👯",
            display_url:     "github.com/nate/cargo-liner".to_string(),
            branch:          "cargo-liner-port".to_string(),
            tracked_ref:     TrackedRef::Named("origin/main".to_string()),
            status:          Comparison::NotCompared,
            full_url:        None,
            push_annotation: None,
        }]),
        ..GitData::default()
    });
    let buffer = test_support::draw_pane(&mut app, PaneId::Git, 47)?;
    let rows = test_support::rows(&buffer);
    let row = rows
        .iter()
        .find(|row| row.contains("cargo-liner-port"))
        .context("remote row")?;
    assert!(row.contains("origin/main"), "{rows:?}");
    for (y, row) in rows
        .iter()
        .enumerate()
        .filter(|(_, row)| row.contains("Branch") || row.contains("cargo-liner-port"))
    {
        assert_eq!(buffer[(45, u16::try_from(y)?)].symbol(), " ", "{row}");
    }
    Ok(())
}

#[test]
fn narrow_worktree_divider_keeps_its_count() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.framework.color_mode = ColorMode::Colored;
    app.panes.git.set_content(GitData {
        description: DescriptionState::KnownEmpty,
        worktrees: (0..3)
            .map(|i| WorktreeInfo {
                name:         format!("tree-{i}"),
                path:         format!("/tmp/tree-{i}"),
                branch:       Some("master".to_string()),
                tracked:      None,
                ahead_behind: Comparison::NotCompared,
            })
            .collect(),
        ..GitData::default()
    });
    for width in [16, 17] {
        let buffer = test_support::draw_pane(&mut app, PaneId::Git, width)?;
        let rows = test_support::rows(&buffer);
        let title = rows
            .iter()
            .find(|row| row.contains("(3)"))
            .context("whole count")?;
        assert!(title.contains('├') && title.contains('┤'), "{title}");
    }
    Ok(())
}

#[test]
fn narrow_git_fields_keep_ahead_behind_whole_and_loading_has_an_ellipsis() -> anyhow::Result<()> {
    for (window, pane_width) in [(80, 17), (95, 24)] {
        let mut app = test_support::project_app();
        app.framework.color_mode = ColorMode::Colored;
        app.panes.git.set_content(GitData {
            description: DescriptionState::KnownEmpty,
            vs_local: GitProbe::Probed(Comparison::Counted(AheadBehind {
                ahead:  123,
                behind: 456,
            })),
            pull_requests: PullRequestSection {
                state: PullRequestSectionState::Loading,
                ..Default::default()
            },
            ..GitData::default()
        });
        let buffer = test_support::draw_pane_in_window(&mut app, PaneId::Git, window, pane_width)?;
        test_support::position(&buffer, "Ahead/Behind")?;
        let loading = test_support::rows(&buffer)
            .into_iter()
            .find(|row| row.contains("loading pull"))
            .context("Pull request loading row")?;
        let body: String = loading.chars().take(usize::from(pane_width - 1)).collect();
        assert!(body.trim_end().ends_with('…'), "{loading}");
        let y = test_support::position(&buffer, "loading pull")?.1;
        assert_eq!(buffer[(pane_width - 2, y)].symbol(), " ");
    }
    Ok(())
}
