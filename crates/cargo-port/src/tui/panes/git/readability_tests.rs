use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use anyhow::Context;
use ratatui::layout::Position;
use ratatui::layout::Rect;
use tui_pane::ColorMode;
use tui_pane::Hittable;
use tui_pane::PaneFocusState;

use super::sync_column_tests;
use crate::constants::SECONDS_PER_MINUTE;
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
use crate::tui::panes::pane_data::GithubStars;
use crate::tui::panes::pane_data::PullRequestSection;
use crate::tui::panes::pane_data::PullRequestSectionState;
use crate::tui::panes::pane_data::QuotaObservation;
use crate::tui::panes::pane_data::RemoteRow;
use crate::tui::panes::pane_data::RemoteRows;
use crate::tui::panes::pane_data::WorktreeInfo;
use crate::tui::render;
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
        worktrees: [
            "verify-run-long",
            "sdlc-merge-long",
            "↑123456release",
            "↓123456release",
        ]
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
        for (offset, branch) in [
            "verify-run-long",
            "sdlc-merge-long",
            "↑123456release",
            "↓123456release",
        ]
        .into_iter()
        .enumerate()
        {
            sync_column_tests::assert_fitted_column(
                &buffer,
                worktrees.1 + 1,
                worktrees.1 + 2 + u16::try_from(offset)?,
                0,
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
        vs_local: GitProbe::Probed(Comparison::Counted(AheadBehind {
            ahead:  1,
            behind: 17,
        })),
        rate_limit_core: QuotaObservation::Observed(RateLimitQuota {
            used:      4770,
            limit:     5000,
            remaining: 230,
            reset_at:  None,
        }),
        rate_limit_graphql: QuotaObservation::Observed(RateLimitQuota {
            used:      42,
            limit:     5000,
            remaining: 4958,
            reset_at:  None,
        }),
        ..GitData::default()
    });
    for mode in [ColorMode::Colored, ColorMode::NoColor] {
        app.framework.color_mode = mode;
        for (window, width, stacked) in [(80, 18, true), (95, 23, true), (95, 45, false)] {
            let buffer = test_support::draw_pane_in_window(&mut app, PaneId::Git, window, width)?;
            let rows = test_support::rows(&buffer);
            for (label, value) in [("Core", "230/5000"), ("GraphQL", "4958/5000")] {
                let (_, y) = test_support::position(&buffer, label)?;
                let value_y = y + u16::from(stacked);
                let row = &rows[usize::from(value_y)];
                assert!(row.contains(value), "{window}, {width}, {label}: {rows:?}");
                assert!(!row.contains('…'), "{row}");
                if stacked {
                    assert!(!rows[usize::from(y)].contains(value), "{rows:?}");
                }
            }
        }
    }
    Ok(())
}

fn countdown_git_data(reset_at: u64) -> GitData {
    let quota = |remaining| {
        QuotaObservation::Observed(RateLimitQuota {
            used: 5000 - remaining,
            limit: 5000,
            remaining,
            reset_at: Some(reset_at),
        })
    };
    GitData {
        description: DescriptionState::KnownEmpty,
        github_status: AvailabilityStatus::Reachable,
        rate_limit_core: quota(230),
        rate_limit_graphql: quota(4959),
        worktrees: vec![WorktreeInfo {
            name:         "checkout".into(),
            path:         "/tmp/checkout".into(),
            branch:       Some("main".into()),
            tracked:      None,
            ahead_behind: Comparison::NotCompared,
        }],
        ..GitData::default()
    }
}

#[test]
fn git_reset_countdown_has_readable_contrast() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    let reset_at = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() + 3599;
    app.panes.git.set_content(countdown_git_data(reset_at));
    let theme = test_support::shipped_dark_theme()?;
    let _theme = crate::tui::test_support::ThemeOverride::install(theme);
    for mode in [ColorMode::Colored, ColorMode::NoColor] {
        app.framework.color_mode = mode;
        app.panes.git.set_content(countdown_git_data(reset_at));
        for (window, width, stacked) in [(80, 80, false), (95, 29, true)] {
            let buffer = test_support::draw_pane_in_window(&mut app, PaneId::Git, window, width)?;
            let failures = test_support::contrast_failures(&buffer, &["resets"], "Git countdown");
            assert!(failures.is_empty(), "{}", failures.join("\n"));
            let label = test_support::position(&buffer, "Core")?;
            let reset = test_support::position(&buffer, "resets")?;
            assert_eq!(reset.1, label.1 + u16::from(stacked));
            if mode == ColorMode::Colored {
                for x in reset.0..width - 2 {
                    if !buffer[(x, reset.1)].symbol().trim().is_empty() {
                        assert_eq!(buffer[(x, reset.1)].fg, tui_pane::secondary_text_color());
                    }
                }
            }
        }
        for (width, stacked, unit, samples) in [
            (24, true, "m", [2760, 2772]),
            (33, false, "m", [2760, 2772]),
            (80, false, "s", [2772, 2773]),
            (32, true, "s", [9, 45]),
            (23, false, "", [2760, 2772]),
        ] {
            let mut shapes = Vec::new();
            for seconds in samples {
                let before = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
                let deadline = before + seconds;
                app.panes.git.set_content(countdown_git_data(deadline));
                let buffer = test_support::draw_pane_in_window(&mut app, PaneId::Git, 95, width)?;
                let after = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
                let label = test_support::position(&buffer, "GraphQL")?;
                let value = test_support::position(&buffer, "4959/5000")?;
                assert_eq!(value.1, label.1 + u16::from(stacked));
                let rows = test_support::rows(&buffer);
                let row = &rows[usize::from(value.1)];
                if unit.is_empty() {
                    assert!(!row.contains("resets"), "{row}");
                } else {
                    let duration = row.split_once("resets ").context("GraphQL reset")?.1;
                    let duration = duration.trim().trim_end_matches('│').trim();
                    assert!(duration.ends_with(unit), "{width}: {row}");
                    if unit == "m" {
                        let minutes = duration.trim_end_matches('m').parse::<u64>()?;
                        assert!(
                            (deadline.saturating_sub(after) / SECONDS_PER_MINUTE
                                ..=seconds / SECONDS_PER_MINUTE)
                                .contains(&minutes)
                        );
                    }
                    if width == 80 {
                        assert!(duration.contains('m'), "{row}");
                    }
                }
                let divider = test_support::position(&buffer, "Worktrees")?;
                shapes.push((label.1, value.1, divider.1));
            }
            assert_eq!(shapes[0], shapes[1], "{width}: countdown shape");
        }
    }
    Ok(())
}

#[test]
fn selected_git_countdown_keeps_selection_ink_on_both_fills() -> anyhow::Result<()> {
    let _theme = test_support::ThemeOverride::install(test_support::shipped_dark_theme()?);
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
        let failures = test_support::contrast_failures(&buffer, &["resets"], "selected countdown");
        assert!(failures.is_empty(), "{}", failures.join("\n"));
        let label = test_support::position(&buffer, "Core")?;
        let reset = test_support::position(&buffer, "resets")?;
        assert_eq!(buffer[label].fg, buffer[reset].fg);
        assert_eq!(buffer[label].bg, buffer[reset].bg);
    }
    Ok(())
}

#[test]
fn narrow_git_fields_keep_labels_values_and_dates() -> anyhow::Result<()> {
    for (pane_width, comparison) in [
        (17, Comparison::NotCompared),
        (16, Comparison::NotCompared),
        (
            16,
            Comparison::Counted(AheadBehind {
                ahead:  3,
                behind: 2,
            }),
        ),
    ] {
        let mut app = test_support::project_app();
        app.framework.color_mode = ColorMode::Colored;
        app.panes.git.set_content(GitData {
            description: DescriptionState::KnownEmpty,
            checkout: GitCheckout::Branch {
                name:     "master".to_string(),
                relation: HeadRelation::Default,
            },
            status: Some(GitStatus::Clean),
            vs_local: GitProbe::Probed(comparison),
            stars: if pane_width == 16 {
                GithubStars::Measured(1)
            } else {
                GithubStars::NotGithub
            },
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
        let buffer = test_support::draw_pane(&mut app, PaneId::Git, pane_width)?;
        for text in [
            "Status",
            "✨ clean",
            "Branch",
            "master",
            "Incept",
            "Latest",
            "2026-11-06",
        ] {
            test_support::position(&buffer, text)?;
        }
        if pane_width == 17 {
            test_support::position(&buffer, "GraphQL")?;
            test_support::position(&buffer, "4626")?;
        }
        let status = test_support::position(&buffer, "Status")?;
        let clean = test_support::position(&buffer, "✨ clean")?;
        assert_eq!(clean.1, status.1 + 1);
        if pane_width == 16 {
            for label in ["Branch", "Stars"]
                .into_iter()
                .chain(matches!(comparison, Comparison::Counted(_)).then_some("Ahead/Behind"))
            {
                let (x, y) = test_support::position(&buffer, label)?;
                assert_eq!(x, 2, "{label}: {:?}", test_support::rows(&buffer));
                assert_eq!(buffer[(1, y)].symbol(), " ");
                assert_eq!(buffer[(pane_width - 2, y)].symbol(), " ");
            }
            let stars = test_support::position(&buffer, "Stars")?;
            let value = test_support::position(&buffer, "⭐ 1")?;
            assert_eq!(value.0, 2);
            assert_eq!(value.1, stars.1 + 1, "Stars stacks at inner width 14");
        }
    }
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
            assert_eq!(&values[2..], &["──"], "{row}");
            assert_eq!(buffer[(width - 2, y)].symbol(), " ");
        }
    }
    Ok(())
}

#[test]
fn cut_git_values_have_no_space_before_the_ellipsis() -> anyhow::Result<()> {
    for (window, width) in [(80, 25), (95, 32), (95, 23)] {
        for mode in [ColorMode::Colored, ColorMode::NoColor] {
            let mut app = test_support::project_app();
            app.framework.color_mode = mode;
            app.panes.git.set_content(GitData {
                description: DescriptionState::KnownEmpty,
                checkout: GitCheckout::Branch {
                    name:     if width == 23 {
                        "master".to_string()
                    } else {
                        "cargo tools phase ".repeat(8)
                    },
                    relation: if width == 23 {
                        HeadRelation::Worktree
                    } else {
                        HeadRelation::Feature
                    },
                },
                worktrees: vec![WorktreeInfo {
                    name:         "cargo tools ".repeat(8),
                    path:         "/tmp/worktree".to_string(),
                    branch:       Some("phase cargo tools ".repeat(8)),
                    tracked:      Some("phase cargo tools ".repeat(8)),
                    ahead_behind: Comparison::NotCompared,
                }],
                ..GitData::default()
            });
            let buffer = test_support::draw_pane_in_window(&mut app, PaneId::Git, window, width)?;
            test_support::assert_ellipses_follow_text(
                &buffer,
                Rect::new(0, 0, width, buffer.area.height),
            );
            if width == 23 {
                test_support::position(&buffer, "master…")?;
            }
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
fn narrow_git_fields_keep_ahead_behind_whole_and_loading_wraps() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    for mode in [ColorMode::Colored, ColorMode::NoColor] {
        app.framework.color_mode = mode;
        for (window, pane_width, ahead, behind, expected) in [
            (80, 17, 123, 456, "↑123 ↓456 di…"),
            (95, 24, 123, 456, "↑123…"),
        ] {
            app.panes.git.set_content(GitData {
                description: DescriptionState::KnownEmpty,
                vs_local: GitProbe::Probed(Comparison::Counted(AheadBehind { ahead, behind })),
                pull_requests: PullRequestSection {
                    state: PullRequestSectionState::Loading,
                    ..Default::default()
                },
                ..GitData::default()
            });
            let buffer =
                test_support::draw_pane_in_window(&mut app, PaneId::Git, window, pane_width)?;
            test_support::position(&buffer, "Ahead/Behind")?;
            test_support::position(&buffer, expected)?;
            let loading = test_support::rows(&buffer)
                .into_iter()
                .find(|row| row.contains("loading pull"))
                .context("Pull request loading row")?;
            assert!(!loading.contains('…'), "{loading}");
            let (_, loading_y) = test_support::position(&buffer, "loading pull")?;
            let (_, requests_y) = test_support::position(&buffer, "requests")?;
            for y in loading_y..=requests_y {
                assert_eq!(buffer[(pane_width - 2, y)].symbol(), " ");
            }
        }
        for (window, pane_width, ahead, behind, expected) in [
            (80, 15, 1234, 5678, "↑1234…"),
            (95, 16, 1234, 5678, "↑1234 ↓5678…"),
            (80, 16, 1, 91, "↑1 ↓91 dive…"),
            (95, 17, 1, 91, "↑1 ↓91 diver…"),
        ] {
            app.panes.git.set_content(GitData {
                description: DescriptionState::KnownEmpty,
                vs_local: GitProbe::Probed(Comparison::Counted(AheadBehind { ahead, behind })),
                ..GitData::default()
            });
            let buffer =
                test_support::draw_pane_in_window(&mut app, PaneId::Git, window, pane_width)?;
            test_support::position(&buffer, "Ahead/Behind")?;
            test_support::position(&buffer, expected)?;
        }
    }
    Ok(())
}

#[test]
fn cut_sync_values_keep_each_visible_count_whole() -> anyhow::Result<()> {
    #[derive(Debug, PartialEq, Eq)]
    enum CountCoverage {
        Unseen,
        Seen,
    }
    let mut app = test_support::project_app();
    app.panes.git.set_content(GitData {
        description: DescriptionState::KnownEmpty,
        remotes: RemoteRows::Listed(vec![RemoteRow {
            name:            "origin".into(),
            icon:            "",
            display_url:     "github.com/acme/project".into(),
            branch:          "main".into(),
            tracked_ref:     TrackedRef::Named("origin/main".into()),
            status:          Comparison::Counted(AheadBehind {
                ahead:  1,
                behind: 17,
            }),
            full_url:        None,
            push_annotation: None,
        }]),
        ..GitData::default()
    });
    let mut saw_cut = CountCoverage::Unseen;
    let mut saw_whole = CountCoverage::Unseen;
    for mode in [ColorMode::Colored, ColorMode::NoColor] {
        app.framework.color_mode = mode;
        for (window, width) in [(80, 24), (80, 30), (95, 35), (95, 60)] {
            let buffer = test_support::draw_pane_in_window(&mut app, PaneId::Git, window, width)?;
            for row in test_support::rows(&buffer) {
                if row.contains('↑') {
                    assert!(row.contains("↑1"), "{width}: {row}");
                    assert!(!row.contains("↓1…"), "{width}: {row}");
                    assert!(!row.contains("↓…"), "{width}: {row}");
                    if row.contains('↓') {
                        assert!(row.contains("↓17"), "{width}: {row}");
                        saw_whole = CountCoverage::Seen;
                    } else {
                        assert!(row.contains('…'), "{width}: {row}");
                        saw_cut = CountCoverage::Seen;
                    }
                }
            }
        }
    }
    assert_eq!(
        (saw_cut, saw_whole),
        (CountCoverage::Seen, CountCoverage::Seen)
    );
    Ok(())
}

#[test]
fn a_section_at_the_top_of_a_page_draws_no_second_rule() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.framework.color_mode = ColorMode::Colored;
    app.panes.git.set_content(GitData {
        description: DescriptionState::Known("a fleet of cargo tools".to_string()),
        remotes: RemoteRows::Listed(vec![RemoteRow {
            name:            "origin".to_string(),
            icon:            "",
            display_url:     "github.com/nate/fleet".to_string(),
            branch:          "main".to_string(),
            tracked_ref:     TrackedRef::Untracked,
            status:          Comparison::NotCompared,
            full_url:        None,
            push_annotation: None,
        }]),
        worktrees: (0..13)
            .map(|i| WorktreeInfo {
                name:         format!("tree-{i}"),
                path:         format!("/tmp/tree-{i}"),
                branch:       Some("main".to_string()),
                tracked:      None,
                ahead_behind: Comparison::NotCompared,
            })
            .collect(),
        ..GitData::default()
    });
    app.set_focus_to_pane(PaneId::Git);
    for pos in 0..20 {
        app.panes.git.viewport.set_pos(pos);
        let buffer = test_support::draw_pane_at_size(&mut app, PaneId::Git, 17, 9)?;
        let rows = test_support::rows(&buffer);
        for pair in rows.windows(2) {
            assert!(
                !(pair[1].starts_with('├')
                    && (pair[0].starts_with('├') || pair[0].starts_with('┌'))),
                "pos {pos}: {rows:?}"
            );
        }
    }
    Ok(())
}

#[test]
fn an_empty_git_pane_names_its_state() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    for mode in [ColorMode::Colored, ColorMode::NoColor] {
        app.framework.color_mode = mode;
        for (checkout, text) in [
            (GitCheckout::AwaitingProbe, "loading…"),
            (GitCheckout::NotRepository, "not a git repo"),
        ] {
            app.panes.git.set_content(GitData {
                checkout,
                description: DescriptionState::KnownEmpty,
                ..GitData::default()
            });
            for width in [17, 18] {
                let buffer = test_support::draw_pane_at_size(&mut app, PaneId::Git, width, 9)?;
                let first = if width == 17 && text == "not a git repo" {
                    "not a git"
                } else {
                    text
                };
                assert_eq!(test_support::position(&buffer, first)?, (2, 1));
                if first != text {
                    assert_eq!(test_support::position(&buffer, "repo")?, (2, 2));
                }
                for y in 1..3 {
                    assert_eq!(buffer[(width - 2, y)].symbol(), " ");
                }
            }
        }
    }
    Ok(())
}

#[test]
fn a_git_date_that_does_not_fit_drops_the_time_then_the_century() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.framework.color_mode = ColorMode::Colored;
    app.panes.git.set_content(GitData {
        description: DescriptionState::KnownEmpty,
        checkout: GitCheckout::Branch {
            name:     "main".to_string(),
            relation: HeadRelation::Default,
        },
        inception: FirstCommit::Probed(CommitDate::At("2026-03-07 09:12".to_string())),
        ..GitData::default()
    });
    for (pane_width, expected) in [(17, "2026-03-07"), (12, "26-03-07")] {
        let buffer = test_support::draw_pane(&mut app, PaneId::Git, pane_width)?;
        let rows = test_support::rows(&buffer);
        let date = rows
            .iter()
            .find(|row| row.contains("03-07"))
            .with_context(|| format!("{pane_width}: {rows:?}"))?;
        assert!(date.contains(expected), "{pane_width}: {rows:?}");
        assert!(
            !date.contains('…') && !date.contains("09:"),
            "{pane_width}: {rows:?}"
        );
    }
    Ok(())
}

#[test]
fn a_long_git_description_keeps_the_short_cap_in_a_taller_pane() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.framework.color_mode = ColorMode::Colored;
    app.panes.git.set_content(GitData {
        description: DescriptionState::Known(
            "a fleet of cargo tools - cargo-port (project TUI), cargo-mend (visibility \
             auditing), tui_pane (ratatui pane framework)"
                .to_string(),
        ),
        status: Some(GitStatus::Clean),
        ..GitData::default()
    });
    for (window, height) in [(80, 13), (80, 15)] {
        let buffer =
            test_support::draw_pane_in_window_at_size(&mut app, PaneId::Git, window, 17, height)?;
        let rows = test_support::rows(&buffer);
        let separator = rows
            .iter()
            .position(|row| row.starts_with('├'))
            .with_context(|| format!("{height}: {rows:?}"))?;
        assert_eq!(separator, 3, "{height}: {rows:?}");
    }
    let mut descriptions = Vec::new();
    for height in [30, 45] {
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, height))?;
        let frame = terminal.draw(|frame| render::ui(frame, &mut app))?;
        let area = app.panes.tiled_layout.area(PaneId::Git);
        let separator = (area.y + 1..area.bottom() - 1)
            .find(|y| matches!(frame.buffer[(area.x, *y)].symbol(), "├" | "┝"))
            .with_context(|| format!("Git divider at 80×{height}"))?;
        assert_eq!(separator - area.y, 3, "80×{height}: {area:?}");
        let rows: Vec<String> = (area.y + 1..separator)
            .map(|y| {
                (area.x + 1..area.right() - 1)
                    .map(|x| frame.buffer[(x, y)].symbol())
                    .collect()
            })
            .collect();
        descriptions.push(rows);
    }
    assert_eq!(descriptions[0], descriptions[1]);
    Ok(())
}
