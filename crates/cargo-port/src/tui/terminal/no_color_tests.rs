use std::process::Command;

use anyhow::Context;
use ratatui::Terminal;
use ratatui::backend::Backend;
use ratatui::backend::CrosstermBackend;
use ratatui::backend::TestBackend;

use crate::project::Visibility;
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
    let mut app = test_support::make_app(&[project]);
    let project = app
        .project_list
        .at_path_mut(&path)
        .expect_found("the fixture project");
    project.visibility = Visibility::Deleted;
    app.ensure_visible_rows_cached();
    app.ensure_detail_cached();
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
