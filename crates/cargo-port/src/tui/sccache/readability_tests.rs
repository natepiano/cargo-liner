use tui_pane::ColorMode;

use crate::tui::test_support;
use crate::tui::test_support::HEIGHT;

#[test]
fn sccache_draws_one_thick_border_in_both_color_modes() -> anyhow::Result<()> {
    let mut app = test_support::stats_app(vec!["Cache hits  7".to_string()]);
    for mode in [ColorMode::Colored, ColorMode::NoColor] {
        app.framework.color_mode = mode;
        for width in [60, 80, 160] {
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, HEIGHT))?;
            terminal.draw(|frame| {
                super::render_sccache_popup(frame, &mut app);
                mode.clear_colors(frame.buffer_mut());
            })?;
            let buffer = terminal.backend().buffer();
            let (left, top) = test_support::position(buffer, "┏")?;
            let (right, bottom) = test_support::position(buffer, "┛")?;
            assert_eq!(buffer[(right, top)].symbol(), "┓");
            assert_eq!(buffer[(left, bottom)].symbol(), "┗");
            for x in left + 1..right {
                assert_eq!(buffer[(x, bottom)].symbol(), "━");
            }
            for y in top + 1..bottom {
                assert_eq!(buffer[(left, y)].symbol(), "┃");
                assert_eq!(buffer[(right, y)].symbol(), "┃");
            }
            for corner in ["┏", "┓", "┗", "┛"] {
                assert_eq!(
                    buffer
                        .content
                        .iter()
                        .filter(|cell| cell.symbol() == corner)
                        .count(),
                    1,
                    "{mode:?}, width={width}: {corner}"
                );
            }
            test_support::position(buffer, "Sccache stats")?;
            test_support::position(buffer, "Cache hits")?;
            test_support::position(buffer, "Esc close")?;
        }
    }
    Ok(())
}

#[test]
fn sccache_last_page_names_the_second_page() -> anyhow::Result<()> {
    let mut app = test_support::stats_app(
        (0..9)
            .map(|i| format!("Compile request {i}  {i}"))
            .collect(),
    );
    test_support::draw_stats(&mut app, 12)?;
    app.overlays.sccache_pane.viewport.end();
    let buffer = test_support::draw_stats(&mut app, 12)?;
    test_support::position(&buffer, "▲ 2 of 2")?;
    Ok(())
}

#[test]
fn sccache_selection_fills_the_row_to_the_border() -> anyhow::Result<()> {
    let mut app = test_support::stats_app(vec!["Cache hits  7".to_string()]);
    let buffer = test_support::draw_stats(&mut app, HEIGHT)?;
    let label = test_support::position(&buffer, "Cache hits")?;
    let inner = app.overlays.sccache_pane.viewport.content_area();
    assert_eq!(buffer[(inner.right() - 1, label.1)].bg, buffer[label].bg);
    Ok(())
}

#[test]
fn sccache_hints_name_first_and_last() -> anyhow::Result<()> {
    let mut app = test_support::stats_app(Vec::new());
    let buffer = test_support::draw_stats(&mut app, HEIGHT)?;
    test_support::position(&buffer, "home/end first/last")?;
    Ok(())
}

#[test]
fn popup_titles_use_sentence_case() -> anyhow::Result<()> {
    let mut app = test_support::stats_app(Vec::new());
    let buffer = test_support::draw_stats(&mut app, HEIGHT)?;
    test_support::position(&buffer, "Sccache stats")?;
    Ok(())
}

#[test]
fn sccache_reasons_explain_the_uncached_call() -> anyhow::Result<()> {
    let mut app = test_support::stats_app(vec![
        "Non-cacheable reasons:".to_string(),
        "-  1".to_string(),
        "-o  2".to_string(),
        "missing output_dir  3".to_string(),
        "multiple input files  4".to_string(),
    ]);
    let buffer = test_support::draw_stats(&mut app, HEIGHT)?;
    for reason in [
        "Source read from standard input",
        "Output file named directly",
        "No output directory",
        "Multiple input files",
    ] {
        test_support::position(&buffer, reason)?;
    }
    Ok(())
}

#[test]
fn selected_sccache_label_has_readable_contrast() -> anyhow::Result<()> {
    let mut app = test_support::stats_app(vec!["Cache hits  7".to_string()]);
    let theme = test_support::shipped_dark_theme()?;
    let _theme = crate::tui::test_support::ThemeOverride::install(theme);
    let buffer = test_support::draw_stats(&mut app, HEIGHT)?;
    let failures = test_support::contrast_failures(&buffer, &["Cache hits"], "selected sccache");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    Ok(())
}

#[test]
fn sccache_all_reasons_use_plain_sentence_case() -> anyhow::Result<()> {
    let mut app = test_support::stats_app(vec![
        "Non-cacheable reasons:".to_string(),
        "crate-type  1".to_string(),
        "incremental  2".to_string(),
        "missing input  3".to_string(),
        "missing emit  4".to_string(),
        "compiler option not supported  5".to_string(),
    ]);
    let buffer = test_support::draw_stats(&mut app, HEIGHT)?;
    for text in [
        "Unsupported crate type",
        "Incremental compilation",
        "No input file",
        "No output kinds requested",
        "Compiler option not supported",
    ] {
        test_support::position(&buffer, text)?;
    }
    Ok(())
}

#[test]
fn narrow_sccache_footer_keeps_only_whole_hints() -> anyhow::Result<()> {
    let mut app = test_support::stats_app(vec!["Cache hits  7".to_string()]);
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, HEIGHT))?;
    terminal.draw(|frame| super::render_sccache_popup(frame, &mut app))?;
    let buffer = terminal.backend().buffer();
    let (_, y) = test_support::position(buffer, "Esc close")?;
    let row = &test_support::rows(buffer)[usize::from(y)];
    assert!(row.contains("home/end first/last"), "{row}");
    assert_eq!(
        row.trim_matches(|cell: char| cell.is_whitespace() || cell == '┃'),
        "Esc close · y copy · ↑/↓ nav · home/end first/last · pageup/pagedown page"
    );
    Ok(())
}
