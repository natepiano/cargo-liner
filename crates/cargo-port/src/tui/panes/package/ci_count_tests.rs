use anyhow::Context;
use tui_pane::ColorMode;
use tui_pane::PaneFocusState;

use crate::ci::CiStatus;
use crate::project::AbsolutePath;
use crate::project::CiPagination;
use crate::project::GithubRunCount;
use crate::project::ProjectCiInfo;
use crate::project::RepoInfo;
use crate::project::WorkflowPresence;
use crate::tui::panes;
use crate::tui::panes::DetailField;
use crate::tui::panes::PackageData;
use crate::tui::panes::PackageRow;
use crate::tui::panes::PaneId;
use crate::tui::state::CiDisplay;
use crate::tui::test_support;

#[test]
fn ci_counts_keep_labels_and_distinct_github_states_at_narrow_window_widths() -> anyhow::Result<()>
{
    for width in [80, 95] {
        for (count, text) in [
            (GithubRunCount::Loading, "gh …"),
            (GithubRunCount::Known(0), "gh 0"),
            (GithubRunCount::Known(2104), "gh 2104"),
            (GithubRunCount::Unavailable, "gh ?"),
        ] {
            let mut app = test_support::project_app();
            app.framework.color_mode = ColorMode::Colored;
            app.panes.package.set_content(PackageData {
                ci_display: CiDisplay::Runs {
                    ci_status:    Some(CiStatus::Failed),
                    local:        9,
                    github_total: count,
                },
                ..PackageData::default()
            });
            let buffer = test_support::draw_ui(&mut app, width)?;
            let area = app.panes.tiled_layout.area(PaneId::Package);
            let local = test_support::find_text_cell(&buffer, area, "local 9")
                .with_context(|| format!("{width}: {:?}", test_support::rows(&buffer)))?;
            let icon = test_support::find_text_cell(&buffer, area, "🔴").context("CI status")?;
            let github = test_support::find_text_cell(&buffer, area, text)
                .with_context(|| format!("{width}: {:?}", test_support::rows(&buffer)))?;
            assert_eq!(icon, (local.0 - 3, local.1));
            assert_eq!(github, (local.0, local.1 + 1));
        }
    }
    Ok(())
}

#[test]
fn ci_keeps_the_local_label_when_the_column_has_no_next_row() -> anyhow::Result<()> {
    for width in [80, 95] {
        let mut app = test_support::project_app();
        app.framework.color_mode = ColorMode::Colored;
        app.panes.package.set_content(PackageData {
            ci_display: CiDisplay::Runs {
                ci_status:    Some(CiStatus::Failed),
                local:        9,
                github_total: GithubRunCount::Known(2104),
            },
            ..PackageData::default()
        });
        test_support::draw_ui(&mut app, width)?;
        let pane_width = app.panes.tiled_layout.area(PaneId::Package).width;
        let rows =
            panes::package_rows_from_data(app.panes.package.content().context("Package data")?);
        let ci = rows
            .iter()
            .position(|row| *row == PackageRow::Field(DetailField::Ci))
            .context("CI field")?;
        app.panes.package.focus.pane_focus_state = PaneFocusState::Active;
        let mut last_row = None;
        for height in 4..=test_support::HEIGHT {
            app.panes.package.viewport.set_pos(ci);
            let buffer = test_support::draw_pane_in_window_at_size(
                &mut app,
                PaneId::Package,
                width,
                pane_width,
                height,
            )?;
            if let Some(local) = test_support::find_text_cell(&buffer, buffer.area, "local 9")
                && local.1 == height - 2
            {
                last_row = Some(buffer);
                break;
            }
        }
        let buffer = last_row.context("CI at the bottom of the column")?;
        let local = test_support::position(&buffer, "local 9")?;
        assert_eq!(
            test_support::position(&buffer, "🔴")?,
            (local.0 - 3, local.1)
        );
        assert!(!test_support::rows(&buffer).join("\n").contains("gh"));
    }
    Ok(())
}

#[test]
fn known_zero_ci_draws_the_empty_message_while_loading_keeps_both_counts() -> anyhow::Result<()> {
    let mut failures = Vec::new();
    for width in [80, 95] {
        for count in [GithubRunCount::Known(0), GithubRunCount::Loading] {
            let mut app = test_support::project_app();
            app.framework.color_mode = ColorMode::Colored;
            let repo = RepoInfo {
                workflows: WorkflowPresence::Present,
                ..RepoInfo::default()
            };
            let info = ProjectCiInfo {
                runs:          Vec::new(),
                github_total:  count,
                ci_pagination: CiPagination::HasMore,
            };
            let display = app.ci.package_display(
                &AbsolutePath::from(test_support::PROJECT_PATH),
                Some(&repo),
                None,
                Some(&info),
                None,
                false,
            );
            app.panes.package.set_content(PackageData {
                ci_display: display,
                ..PackageData::default()
            });
            let buffer = test_support::draw_ui(&mut app, width)?;
            let area = app.panes.tiled_layout.area(PaneId::Package);
            if count == GithubRunCount::Known(0) {
                if test_support::find_text_cell(&buffer, area, "No CI runs").is_none() {
                    failures.push(format!("{width}: {:?}", test_support::rows(&buffer)));
                }
                if test_support::find_text_cell(&buffer, area, "gh 0").is_some() {
                    failures.push(format!("{width}: Known zero still draws gh 0"));
                }
            } else {
                let local = test_support::find_text_cell(&buffer, area, "local 0")
                    .context("Loading local count")?;
                let github = test_support::find_text_cell(&buffer, area, "gh …")
                    .context("Loading GitHub count")?;
                assert_eq!(github, (local.0, local.1 + 1));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    Ok(())
}

#[test]
fn github_count_moves_left_to_keep_the_margin_at_the_exact_boundary() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.framework.color_mode = ColorMode::Colored;
    app.panes.package.set_content(PackageData {
        ci_display: CiDisplay::Runs {
            ci_status:    Some(CiStatus::Failed),
            local:        9,
            github_total: GithubRunCount::Known(10_000_000),
        },
        ..PackageData::default()
    });
    let buffer = test_support::draw_pane_in_window(&mut app, PaneId::Package, 80, 17)?;
    let local = test_support::position(&buffer, "local 9")?;
    let github = test_support::position(&buffer, "gh 10000000")?;
    assert_eq!(github.1, local.1 + 1);
    assert!(github.0 < local.0);
    assert_eq!(buffer[(15, github.1)].symbol(), " ");
    Ok(())
}
