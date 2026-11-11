use anyhow::Context;
use ratatui::Terminal;
use ratatui::backend::TestBackend;

use super::render_sccache_popup;
use crate::sccache::StatsResult;
use crate::tui::test_support;

const TERMINAL_WIDTH: u16 = 160;
const TERMINAL_HEIGHT: u16 = 32;
const CACHE_LOCATION: &str = "~/.cache/sccache/a-long-cache-location-for-padding";

#[test]
fn sccache_popup_widest_row_has_equal_padding_at_160_columns() -> anyhow::Result<()> {
    let mut app = test_support::make_app(&[]);
    let request = app.overlays.sccache_pane.start_loading();
    app.overlays.sccache_pane.apply_result(
        request,
        StatsResult::Ready(vec![format!("Cache location  {CACHE_LOCATION}")]),
    );
    let mut terminal = Terminal::new(TestBackend::new(TERMINAL_WIDTH, TERMINAL_HEIGHT))?;
    terminal.draw(|frame| render_sccache_popup(frame, &mut app))?;
    let buffer = terminal.backend().buffer();
    let row = (0..TERMINAL_HEIGHT)
        .map(|y| {
            (0..TERMINAL_WIDTH)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .find(|line| line.contains("Cache location"))
        .context("the cache location row must render")?;
    let left_border = row.find('┃').context("left popup border")?;
    let right_border = row.rfind('┃').context("right popup border")?;
    let body = &row[left_border + '┃'.len_utf8()..right_border];
    assert!(
        body.contains(CACHE_LOCATION),
        "the widest value must fit: {row}"
    );
    let left = body.chars().take_while(|ch| *ch == ' ').count();
    let right = body.chars().rev().take_while(|ch| *ch == ' ').count();
    assert!(left > 0, "the body needs padding: {row}");
    assert_eq!(left, right, "the widest row needs equal padding: {row}");
    Ok(())
}
