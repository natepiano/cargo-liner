use anyhow::Context;
use crossterm::cursor::MoveToColumn;
use ratatui::Terminal;
use ratatui::backend::Backend;
use ratatui::backend::CrosstermBackend;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use tui_pane::Renderable;
use unicode_width::UnicodeWidthStr;

use super::GitPane;
use crate::constants::IN_SYNC;
use crate::http::RateLimitQuota;
use crate::tui::panes::GitCheckout;
use crate::tui::panes::GitData;
use crate::tui::panes::HeadRelation;
use crate::tui::panes::OutputPresentation;
use crate::tui::panes::SyncedDescriptionHeight;
use crate::tui::panes::constants::FIT_TEXT_ELLIPSIS;
use crate::tui::panes::lang;
use crate::tui::render;
use crate::tui::render_context::PaneRenderCtx;
use crate::tui::test_support;

const TERMINAL_WIDTH: u16 = 80;
const TERMINAL_HEIGHT: u16 = 30;
const BRANCH: &str = "fleet";
const DESCRIPTION: &str = "a fleet of cargo tools";

#[test]
fn git_values_at_80_columns_keep_letters_or_shorten_at_the_end() -> anyhow::Result<()> {
    for pane_width in [40, 20] {
        let data = GitData {
            checkout: GitCheckout::Branch {
                name:     BRANCH.to_string(),
                relation: HeadRelation::AwaitingRepoProbe,
            },
            ..GitData::default()
        };
        let rows = render_git_value(data, pane_width)?;
        let branch_row = rows
            .iter()
            .find(|row| row.contains("Branch"))
            .context("the Git branch row must render")?;
        let value = branch_row
            .split_once("Branch")
            .context("the branch label must precede the value")?
            .1
            .trim();
        let whole = value.starts_with(BRANCH);
        let shortened = value
            .strip_suffix(FIT_TEXT_ELLIPSIS)
            .or_else(|| value.strip_suffix('…'))
            .is_some_and(|prefix| !prefix.is_empty() && BRANCH.starts_with(prefix));
        assert!(
            whole || shortened,
            "the branch must remain whole or end with an ellipsis at width {pane_width}: {branch_row}"
        );
    }
    Ok(())
}

#[test]
fn git_description_at_80_columns_keeps_the_middle_of_fleet() -> anyhow::Result<()> {
    for pane_width in [9, 10, 20] {
        let data = GitData {
            description: Some(DESCRIPTION.to_string()),
            ..GitData::default()
        };
        let rows = render_git_value(data, pane_width)?;
        assert!(
            rows.iter().any(|row| row.contains(BRANCH)),
            "the description column can hold fleet at width {pane_width}: {rows:?}"
        );
    }
    Ok(())
}

#[test]
fn git_inner_width_nine_shows_branch_label_without_value() -> anyhow::Result<()> {
    assert_branch_label_only(11)
}

#[test]
fn git_inner_width_nine_shows_rate_labels_without_values() -> anyhow::Result<()> {
    assert_rate_labels_only(11, &format!("Rate {FIT_TEXT_ELLIPSIS}"))
}

#[test]
fn git_inner_width_ten_uses_nine_cells_for_rate_labels() -> anyhow::Result<()> {
    assert_rate_labels_only(12, &format!("Rate l{FIT_TEXT_ELLIPSIS}"))
}

fn assert_rate_labels_only(outer_width: u16, expected: &str) -> anyhow::Result<()> {
    let quota = RateLimitQuota {
        limit:     5000,
        used:      42,
        remaining: 4958,
        reset_at:  None,
    };
    for quota in [None, Some(quota)] {
        let data = GitData {
            rate_limit_core: quota,
            rate_limit_graphql: quota,
            ..GitData::default()
        };
        let rows = render_git_value(data, outer_width)?;
        let rate_rows: Vec<_> = rows.iter().filter(|row| row.contains("Rat")).collect();
        assert_eq!(
            rate_rows.len(),
            2,
            "both rate-limit labels must render: {rows:?}"
        );
        for row in rate_rows {
            assert_eq!(row.trim(), expected);
        }
    }
    Ok(())
}

#[test]
fn git_inner_width_ten_keeps_label_only_mode() -> anyhow::Result<()> {
    assert_branch_label_only(12)
}

#[test]
fn git_inner_width_eleven_restores_end_shortened_value() -> anyhow::Result<()> {
    let rows = render_git_value(branch_data(), 13)?;
    let row = rows
        .iter()
        .find(|row| row.contains("B..."))
        .context("the fitted branch label must render")?;
    assert_eq!(
        row.trim(),
        format!("B{FIT_TEXT_ELLIPSIS} f{FIT_TEXT_ELLIPSIS}")
    );
    Ok(())
}

fn branch_data() -> GitData {
    GitData {
        checkout: GitCheckout::Branch {
            name:     BRANCH.to_string(),
            relation: HeadRelation::AwaitingRepoProbe,
        },
        ..GitData::default()
    }
}

fn assert_branch_label_only(outer_width: u16) -> anyhow::Result<()> {
    let rows = render_git_value(branch_data(), outer_width)?;
    let row = rows
        .iter()
        .find(|row| row.contains("Branch"))
        .context("the whole branch label must render")?;
    assert_eq!(
        row.trim(),
        "Branch",
        "the narrow row must contain only its label"
    );
    assert!(
        !rows.iter().any(|row| row.contains(BRANCH)),
        "the narrow pane must omit the branch value: {rows:?}"
    );
    Ok(())
}

fn render_git_value(data: GitData, pane_width: u16) -> anyhow::Result<Vec<String>> {
    let app = test_support::make_app(&[]);
    let lookup = app.ci.status_lookup();
    let ctx = PaneRenderCtx {
        animation_elapsed:         std::time::Duration::ZERO,
        config:                    &app.config,
        project_list:              &app.project_list,
        selected_project_path:     None,
        scan:                      &app.scan,
        ci_status_lookup:          &lookup,
        settings_render_inputs:    None,
        synced_description_height: SyncedDescriptionHeight::default(),
        output_presentation:       OutputPresentation::Hidden,
    };
    let mut pane = GitPane::new();
    pane.set_content(data);
    let mut terminal = Terminal::new(TestBackend::new(TERMINAL_WIDTH, TERMINAL_HEIGHT))?;
    let area = Rect::new(TERMINAL_WIDTH - pane_width, 0, pane_width, TERMINAL_HEIGHT);
    terminal.draw(|frame| {
        let _ = pane.render(frame, area, &ctx);
    })?;
    let buffer = terminal.backend().buffer();
    Ok((area.y..area.bottom())
        .map(|y| {
            (area.x..area.right())
                .map(|x| buffer[(x, y)].symbol())
                .collect()
        })
        .collect())
}

#[test]
fn shortened_text_keeps_the_ellipsis_within_the_available_columns() {
    for (width, expected) in [(0, ""), (1, "."), (2, ".."), (3, "...")] {
        assert_eq!(
            crate::tui::render::truncate_with_ellipsis("fleet", width, FIT_TEXT_ELLIPSIS),
            expected
        );
    }
}

#[test]
fn partial_redraw_anchors_the_cursor_after_variation_selector_glyphs() -> anyhow::Result<()> {
    let mut app = test_support::make_app(&[]);
    let mut terminal = Terminal::new(TestBackend::new(100, 30))?;
    for glyph in [
        IN_SYNC,
        lang::language_icon("toml"),
        lang::language_icon("makefile"),
    ] {
        app.panes.git.set_content(GitData {
            description: Some("ab fleet 18.7".to_string()),
            ..GitData::default()
        });
        terminal.draw(|frame| render::ui(frame, &mut app))?;
        let previous = terminal.backend().buffer().clone();
        app.panes.git.set_content(GitData {
            description: Some(format!("{glyph} fleet 18.7")),
            ..GitData::default()
        });
        terminal.draw(|frame| render::ui(frame, &mut app))?;
        let buffer = terminal.backend().buffer();
        let (x, y) = buffer
            .content
            .iter()
            .enumerate()
            .find(|(_, cell)| cell.symbol().contains(glyph))
            .map(|(index, _)| buffer.pos_of(index))
            .context("the Git description must contain the original glyph")?;
        let next_column = x + u16::try_from(glyph.width())?;
        assert_eq!(previous[(x + 1, y)].symbol(), "b");
        let mut bytes = Vec::new();
        CrosstermBackend::new(&mut bytes).draw(previous.diff(buffer).into_iter())?;
        let output = std::str::from_utf8(&bytes)?;
        assert!(
            output.contains(&format!("{glyph}{}", MoveToColumn(next_column))),
            "the backend must anchor the cursor after {glyph}: {output:?}"
        );
        assert!(
            output.contains(&format!(
                "{}{}{glyph}{}",
                " ".repeat(glyph.width()),
                MoveToColumn(x),
                MoveToColumn(next_column)
            )),
            "the backend must clear every glyph column before drawing {glyph}: {output:?}"
        );
    }
    Ok(())
}
