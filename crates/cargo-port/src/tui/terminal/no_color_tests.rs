use std::process::Command;

use anyhow::Context;
use ratatui::Terminal;
use ratatui::backend::Backend;
use ratatui::backend::CrosstermBackend;
use ratatui::backend::TestBackend;
use tui_pane::ColorMode;
use tui_pane::PaneFocusState;

use crate::project::RootItem;
use crate::project::Visibility;
use crate::sccache::StatsResult;
use crate::tui::panes::GitData;
use crate::tui::panes::PaneId;
use crate::tui::render;
use crate::tui::test_support;

const CHILD_ENV: &str = "CARGO_PORT_NO_COLOR_TEST_CHILD";
const PROJECT_NAME: &str = "deleted-no-color";
const TERMINAL_WIDTH: u16 = 160;
const TERMINAL_HEIGHT: u16 = 40;

#[test]
fn deleted_row_under_no_color_keeps_strikethrough_in_output_bytes() -> anyhow::Result<()> {
    if std::env::var_os(CHILD_ENV).is_none() {
        let module = module_path!()
            .split_once("::")
            .context("test module path")?
            .1;
        let test =
            format!("{module}::deleted_row_under_no_color_keeps_strikethrough_in_output_bytes");
        let output = Command::new(std::env::current_exe()?)
            .args(["--exact", &test, "--nocapture"])
            .env(CHILD_ENV, "1")
            .env("NO_COLOR", "1")
            .output()?;
        assert!(
            output.status.success(),
            "NO_COLOR child failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return Ok(());
    }
    let scratch = tempfile::tempdir()?;
    let path = scratch.path().join(PROJECT_NAME);
    let project = test_support::make_package(PROJECT_NAME, &path);
    let mut app = test_support::make_app_with_color_mode(&[project], ColorMode::from_environment());
    let project = app
        .project_list
        .at_path_mut(&path)
        .expect_found("the fixture project");
    project.visibility = Visibility::Deleted;
    app.ensure_visible_rows_cached();
    app.ensure_detail_cached();
    app.ensure_fit_widths_cached();
    app.ensure_disk_cache();
    let mut terminal = Terminal::new(TestBackend::new(TERMINAL_WIDTH, TERMINAL_HEIGHT))?;
    terminal.draw(|frame| render::ui(frame, &mut app))?;
    let buffer = terminal.backend().buffer();
    let body = app.panes.project_list.body_rect;
    let (_, row) = test_support::find_text_cell(buffer, body, PROJECT_NAME)
        .context("the Deleted project row must render")?;
    let mut bytes = Vec::new();
    CrosstermBackend::new(&mut bytes)
        .draw((body.x..body.right()).map(|x| (x, row, &buffer[(x, row)])))?;
    let glyphs = glyphs_with_strikethrough(std::str::from_utf8(&bytes)?)?;
    let text = glyphs.iter().map(|(ch, _)| *ch).collect::<String>();
    let start = text
        .find(PROJECT_NAME)
        .context("the backend must write the project name")?;
    let start = text[..start].chars().count();
    assert!(
        glyphs[start..start + PROJECT_NAME.len()]
            .iter()
            .all(|(_, crossed_out)| *crossed_out),
        "NO_COLOR must retain SGR 9 on every project-name glyph: {bytes:?}"
    );
    Ok(())
}

fn glyphs_with_strikethrough(output: &str) -> anyhow::Result<Vec<(char, bool)>> {
    let mut glyphs = Vec::new();
    let mut crossed_out = false;
    let mut chars = output.chars();
    while let Some(ch) = chars.next() {
        if ch != '\u{1b}' {
            glyphs.push((ch, crossed_out));
            continue;
        }
        assert_eq!(
            chars.next(),
            Some('['),
            "the backend must emit CSI commands"
        );
        let mut parameters = String::new();
        let command = chars
            .by_ref()
            .find(|ch| {
                if ('@'..='~').contains(ch) {
                    true
                } else {
                    parameters.push(*ch);
                    false
                }
            })
            .context("a CSI command must have a final byte")?;
        if command == 'm' {
            for parameter in parameters.split(';') {
                match parameter.parse::<u16>().unwrap_or(0) {
                    0 | 29 => crossed_out = false,
                    9 => crossed_out = true,
                    _ => {},
                }
            }
        }
    }
    Ok(glyphs)
}

#[test]
fn selected_row_and_focused_border_remain_distinct_without_color() -> anyhow::Result<()> {
    const TEST: &str = "selected_row_and_focused_border_remain_distinct_without_color";
    if std::env::var_os(CHILD_ENV).is_none() {
        let module = module_path!()
            .split_once("::")
            .context("test module path")?
            .1;
        let output = Command::new(std::env::current_exe()?)
            .args(["--exact", &format!("{module}::{TEST}")])
            .env(CHILD_ENV, "1")
            .env("NO_COLOR", "1")
            .output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        return Ok(());
    }
    let path = std::path::Path::new("/tmp/test/selected-project");
    let RootItem::Rust(primary) = test_support::make_package("selected-project", path) else {
        anyhow::bail!("the primary fixture must be a Rust project");
    };
    let RootItem::Rust(linked) = test_support::make_package(
        "linked-project",
        std::path::Path::new("/tmp/test/linked-project"),
    ) else {
        anyhow::bail!("the linked fixture must be a Rust project");
    };
    let mut app = test_support::make_app_with_color_mode(
        &[
            RootItem::Worktrees(crate::project::WorktreeGroup::new(primary, vec![linked])),
            test_support::make_package(
                "other-project",
                std::path::Path::new("/tmp/test/other-project"),
            ),
        ],
        ColorMode::from_environment(),
    );
    app.project_list.select_project_in_tree(path, true);
    app.project_list.collapse_all(true);
    app.ensure_visible_rows_cached();
    app.ensure_fit_widths_cached();
    app.ensure_disk_cache();
    let mut terminal = Terminal::new(TestBackend::new(TERMINAL_WIDTH, TERMINAL_HEIGHT))?;
    terminal.draw(|frame| render::ui(frame, &mut app))?;
    let buffer = terminal.backend().buffer();
    let selected =
        test_support::find_text_cell(buffer, app.panes.project_list.body_rect, "selected-project")
            .context("selected row")?;
    let other =
        test_support::find_text_cell(buffer, app.panes.project_list.body_rect, "other-project")
            .context("other row")?;
    assert!(
        buffer[selected]
            .modifier
            .contains(ratatui::style::Modifier::REVERSED)
    );
    assert!(
        !buffer[other]
            .modifier
            .contains(ratatui::style::Modifier::REVERSED)
    );
    let marker = test_support::find_text_cell(buffer, app.panes.project_list.body_rect, "▶")
        .context("the selection marker")?;
    assert_eq!(marker.1, selected.1);
    let pane = app.panes.tiled_layout.area(PaneId::ProjectList);
    assert!(
        buffer[(pane.x, pane.y + 1)]
            .modifier
            .contains(ratatui::style::Modifier::BOLD)
    );
    Ok(())
}

#[test]
fn no_color_focus_uses_heavy_borders_and_one_active_selection() -> anyhow::Result<()> {
    const TEST: &str = "no_color_focus_uses_heavy_borders_and_one_active_selection";
    if std::env::var_os(CHILD_ENV).is_none() {
        let module = module_path!()
            .split_once("::")
            .context("test module path")?
            .1;
        let output = Command::new(std::env::current_exe()?)
            .args(["--exact", &format!("{module}::{TEST}")])
            .env(CHILD_ENV, "1")
            .env("NO_COLOR", "1")
            .output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        return Ok(());
    }
    let path = std::path::Path::new("/tmp/test/selected-project");
    let mut app = test_support::make_app_with_color_mode(
        &[test_support::make_package("selected-project", path)],
        ColorMode::from_environment(),
    );
    app.ensure_visible_rows_cached();
    app.ensure_detail_cached();
    app.ensure_fit_widths_cached();
    app.ensure_disk_cache();
    let mut terminal = Terminal::new(TestBackend::new(TERMINAL_WIDTH, TERMINAL_HEIGHT))?;
    terminal.draw(|frame| render::ui(frame, &mut app))?;
    let pane = app.panes.tiled_layout.area(PaneId::ProjectList);
    assert_eq!(
        terminal.backend().buffer()[(pane.x, pane.y + 1)].symbol(),
        "┃"
    );
    app.set_focus_to_pane(PaneId::Cpu);
    terminal.draw(|frame| render::ui(frame, &mut app))?;
    let body = app.panes.project_list.body_rect;
    let selected =
        test_support::find_text_cell(terminal.backend().buffer(), body, "selected-project")
            .context("the project row")?;
    assert!(
        terminal.backend().buffer()[selected]
            .modifier
            .contains(ratatui::style::Modifier::BOLD)
    );
    assert!(
        !terminal.backend().buffer()[selected]
            .modifier
            .contains(ratatui::style::Modifier::REVERSED)
    );
    app.set_focus_to_pane(PaneId::ProjectList);
    app.overlays.open_sccache();
    let request = app.overlays.sccache_pane.start_loading();
    app.overlays.sccache_pane.apply_result(
        request,
        StatsResult::Ready(vec!["Cache hits  7".to_string()]),
    );
    terminal.draw(|frame| render::ui(frame, &mut app))?;
    assert!(
        !terminal.backend().buffer()[selected]
            .modifier
            .contains(ratatui::style::Modifier::REVERSED)
    );
    Ok(())
}

#[test]
fn selected_description_reverses_every_inner_cell_without_color() -> anyhow::Result<()> {
    const TEST: &str = "selected_description_reverses_every_inner_cell_without_color";
    if std::env::var_os(CHILD_ENV).is_none() {
        let module = module_path!()
            .split_once("::")
            .context("test module path")?
            .1;
        let output = Command::new(std::env::current_exe()?)
            .args(["--exact", &format!("{module}::{TEST}")])
            .env(CHILD_ENV, "1")
            .env("NO_COLOR", "1")
            .output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        return Ok(());
    }
    for pane in [PaneId::Package, PaneId::Git] {
        let path = std::path::Path::new(test_support::PROJECT_PATH);
        let mut app = test_support::make_app_with_color_mode(
            &[test_support::make_package(test_support::PROJECT, path)],
            ColorMode::from_environment(),
        );
        app.scan
            .manifest_metadata
            .record_success(&crate::project::AbsolutePath::from(path));
        app.ensure_visible_rows_cached();
        app.ensure_detail_cached();
        if pane == PaneId::Package {
            app.panes.package.focus.pane_focus_state = PaneFocusState::Active;
        } else {
            app.panes.git.set_content(GitData {
                description: Some(
                    "A fleet of tools with a description that spans several rows".to_string(),
                ),
                ..crate::tui::panes::GitData::default()
            });
            app.panes.git.focus.pane_focus_state = PaneFocusState::Active;
        }
        let buffer = test_support::draw_pane(&mut app, pane, 24)?;
        let word = if pane == PaneId::Package {
            "No description"
        } else {
            "A fleet"
        };
        let (_, y) = test_support::position(&buffer, word)?;
        for x in 1..buffer.area.right() - 1 {
            assert!(
                buffer[(x, y)]
                    .modifier
                    .contains(ratatui::style::Modifier::REVERSED),
                "{pane:?} cell {x},{y}"
            );
        }
    }
    Ok(())
}

#[test]
fn sccache_popup_has_a_focused_border_without_color() -> anyhow::Result<()> {
    const TEST: &str = "sccache_popup_has_a_focused_border_without_color";
    if std::env::var_os(CHILD_ENV).is_none() {
        let module = module_path!()
            .split_once("::")
            .context("the test module path")?
            .1;
        let output = Command::new(std::env::current_exe()?)
            .args(["--exact", &format!("{module}::{TEST}")])
            .env(CHILD_ENV, "1")
            .env("NO_COLOR", "1")
            .output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        return Ok(());
    }
    let mut app = test_support::stats_app(vec!["Cache hits  7".to_string()]);
    app.framework.color_mode = ColorMode::NoColor;
    let buffer = test_support::draw_stats(&mut app, TERMINAL_HEIGHT)?;
    let inner = app.overlays.sccache_pane.viewport.content_area();
    assert_eq!(buffer[(inner.x - 1, inner.y)].symbol(), "┃");
    assert!(
        buffer[(inner.x - 1, inner.y)]
            .modifier
            .contains(ratatui::style::Modifier::BOLD)
    );
    Ok(())
}
