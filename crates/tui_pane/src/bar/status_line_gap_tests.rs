use std::error::Error;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::text::Span;

use super::StatusLineItems;
use super::render_sections;
use crate::ColorMode;
use crate::ShortcutState;
use crate::bar::BarPalette;
use crate::bar::support;

#[test]
fn notes_keep_at_least_three_blank_cells_without_moving_left_hints() -> Result<(), Box<dyn Error>> {
    for mode in [ColorMode::Colored, ColorMode::NoColor] {
        for width in [80, 95] {
            for uptime in [" Uptime: 9s ", " Uptime: 10s "] {
                for note in ["Disk cache: 64 GiB", "Disk cache: 1024 GiB"] {
                    let mut terminal = Terminal::new(TestBackend::new(width, 1))?;
                    let palette = BarPalette::default();
                    let mut navigation = Vec::new();
                    for (key, label) in [("tab", "pane"), ("up/down", "nav"), ("space", "select")] {
                        support::push_hint(
                            &mut navigation,
                            key,
                            label,
                            ShortcutState::Enabled,
                            &palette,
                        );
                    }
                    let mut center = Vec::new();
                    support::push_hint(
                        &mut center,
                        "enter",
                        "activate",
                        ShortcutState::Enabled,
                        &palette,
                    );
                    terminal.draw(|frame| {
                        render_sections(
                            frame,
                            Rect::new(0, 0, width, 1),
                            &palette,
                            StatusLineItems {
                                uptime: vec![Span::raw(uptime)],
                                navigation,
                                center,
                                notes: vec![vec![Span::raw(format!(" {note} "))]],
                                globals: Vec::new(),
                                with_hints: Vec::new(),
                            },
                        );
                        mode.clear_colors(frame.buffer_mut());
                    })?;
                    let row: String = (0..width)
                        .map(|x| terminal.backend().buffer()[(x, 0)].symbol())
                        .collect();
                    let note_start = row.find(note).ok_or("note missing")?;
                    let hints = &row[..note_start];
                    assert!(hints.ends_with("   "), "{mode:?}, {width}: {row:?}");
                    assert!(row.starts_with(&format!("{uptime} tab pane")), "{row:?}");
                    assert!(hints.contains("tab pane"), "{row:?}");
                    if width == 80 {
                        assert!(!hints.contains("enter activate"), "{row:?}");
                        assert!(hints.contains("space select"), "{row:?}");
                    }
                }
            }
        }
    }
    Ok(())
}

#[test]
fn centered_hints_shift_only_when_the_note_needs_the_three_cell_gap() -> Result<(), Box<dyn Error>>
{
    for mode in [ColorMode::Colored, ColorMode::NoColor] {
        for width in [80, 95] {
            for uptime in [" Uptime: 9s ", " Uptime: 10s "] {
                for extra in [0, 2] {
                    let note = format!(
                        "Disk cache: {}",
                        "8".repeat(usize::from(width / 2) - 12 + extra)
                    );
                    let mut terminal = Terminal::new(TestBackend::new(width, 1))?;
                    let palette = BarPalette::default();
                    let mut center = Vec::new();
                    support::push_hint(
                        &mut center,
                        "enter",
                        "activate",
                        ShortcutState::Enabled,
                        &palette,
                    );
                    terminal.draw(|frame| {
                        render_sections(
                            frame,
                            Rect::new(0, 0, width, 1),
                            &palette,
                            StatusLineItems {
                                uptime: vec![Span::raw(uptime)],
                                navigation: Vec::new(),
                                center,
                                notes: vec![vec![Span::raw(format!(" {note} "))]],
                                globals: Vec::new(),
                                with_hints: Vec::new(),
                            },
                        );
                        mode.clear_colors(frame.buffer_mut());
                    })?;
                    let row: String = (0..width)
                        .map(|x| terminal.backend().buffer()[(x, 0)].symbol())
                        .collect();
                    assert!(row.starts_with(uptime), "{row:?}");
                    let note_start = row.find(&note).ok_or("note missing")?;
                    let hint_end = row.find("enter activate").ok_or("center hint missing")?
                        + "enter activate".len();
                    assert_eq!(
                        &row[hint_end..note_start],
                        "   ",
                        "{width}, {mode:?}: {row:?}"
                    );
                }
            }
        }
    }
    Ok(())
}

#[test]
fn disk_cache_notes_drop_only_the_rightmost_hints_at_two_digit_uptimes()
-> Result<(), Box<dyn Error>> {
    for mode in [ColorMode::Colored, ColorMode::NoColor] {
        for width in [80, 95] {
            for seconds in [13, 59] {
                let mut terminal = Terminal::new(TestBackend::new(width, 1))?;
                let palette = BarPalette::default();
                let mut navigation = Vec::new();
                for (key, label) in [("↑/↓", "nav"), ("=/-", "all"), ("tab", "pane")] {
                    support::push_hint(
                        &mut navigation,
                        key,
                        label,
                        ShortcutState::Enabled,
                        &palette,
                    );
                }
                let mut globals = Vec::new();
                support::push_hint(
                    &mut globals,
                    "?",
                    "shortcuts",
                    ShortcutState::Enabled,
                    &palette,
                );
                let note = "Disk cache: 64 GiB - hit rate 92.34 %";
                let uptime = format!(" Uptime: {seconds}s ");
                terminal.draw(|frame| {
                    render_sections(
                        frame,
                        Rect::new(0, 0, width, 1),
                        &palette,
                        StatusLineItems {
                            uptime: vec![Span::raw(uptime.clone())],
                            navigation,
                            center: Vec::new(),
                            notes: vec![vec![Span::raw(format!(" {note} "))]],
                            globals,
                            with_hints: Vec::new(),
                        },
                    );
                    mode.clear_colors(frame.buffer_mut());
                })?;
                let row: String = (0..width)
                    .map(|x| terminal.backend().buffer()[(x, 0)].symbol())
                    .collect();
                assert!(
                    row.starts_with(&format!("{uptime} ↑/↓ nav")),
                    "{width}, {mode:?}: {row:?}"
                );
                assert!(!row.contains("tab pane"), "{row:?}");
                if width == 95 {
                    assert!(row.contains("↑/↓ nav   =/- all"), "{row:?}");
                } else {
                    assert!(!row.contains("=/- all"), "{row:?}");
                }
                let note_start = row.find(note).ok_or("disk note missing")?;
                assert!(row[..note_start].ends_with("   "), "{row:?}");
                assert!(row.contains("? shortcuts"), "{row:?}");
            }
        }
    }
    Ok(())
}
