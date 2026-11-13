use std::error::Error;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::text::Span;

use super::HintFit;
use super::StatusLineItems;
use super::render_sections;
use super::status_line_note_spans;
use crate::ColorMode;
use crate::ScanIndicator;
use crate::ShortcutState;
use crate::StatusLine;
use crate::StatusLineNote;
use crate::bar::BarPalette;
use crate::bar::support;

#[test]
fn a_version_note_keeps_whole_hint_groups_with_the_default_policy() -> Result<(), Box<dyn Error>> {
    let notes = [StatusLineNote {
        label: "cargo-tile ".into(),
        value: "<version>".into(),
    }];
    let status = StatusLine::<crate::GlobalAction>::new(0, ScanIndicator::Hidden, &notes, &[]);
    for mode in [ColorMode::Colored, ColorMode::NoColor] {
        for centered in [false, true] {
            let mut terminal = Terminal::new(TestBackend::new(60, 1))?;
            let palette = BarPalette::default();
            let mut hints = Vec::new();
            for (key, label) in [("↑/↓", "nav"), ("=/-", "all"), ("tab", "pane")] {
                support::push_hint(&mut hints, key, label, ShortcutState::Enabled, &palette);
            }
            let mut globals = Vec::new();
            support::push_hint(
                &mut globals,
                "?",
                "shortcuts",
                ShortcutState::Enabled,
                &palette,
            );
            let (navigation, center) = if centered {
                (Vec::new(), hints)
            } else {
                (hints, Vec::new())
            };
            terminal.draw(|frame| {
                render_sections(
                    frame,
                    Rect::new(0, 0, 60, 1),
                    &palette,
                    StatusLineItems {
                        uptime: vec![Span::raw(" Uptime: 0s ")],
                        disk_reads: Vec::new(),
                        navigation,
                        center,
                        notes: vec![status_line_note_spans(status.notes, &palette)],
                        globals,
                        with_hints: Vec::new(),
                        hint_fit: status.hint_fit,
                    },
                );
                mode.clear_colors(frame.buffer_mut());
            })?;
            let row: String = (0..60)
                .map(|x| terminal.backend().buffer()[(x, 0)].symbol())
                .collect();
            assert!(row.starts_with(" Uptime: 0s "), "{row:?}");
            assert!(row.contains("cargo-tile <version>"), "{row:?}");
            assert!(row.contains("? shortcuts"), "{row:?}");
            for hint in ["↑/↓ nav", "=/- all", "tab pane"] {
                assert!(
                    !row.contains(hint),
                    "{mode:?}, centered={centered}: {row:?}"
                );
            }
        }
    }
    Ok(())
}

#[test]
fn rows_without_notes_draw_navigation_and_center_hints_only_as_whole_groups()
-> Result<(), Box<dyn Error>> {
    for mode in [ColorMode::Colored, ColorMode::NoColor] {
        for width in [40, 80] {
            for centered in [false, true] {
                let mut terminal = Terminal::new(TestBackend::new(width, 1))?;
                let palette = BarPalette::default();
                let mut hints = Vec::new();
                for (key, label) in [("↑/↓", "nav"), ("=/-", "all"), ("tab", "pane")] {
                    support::push_hint(&mut hints, key, label, ShortcutState::Enabled, &palette);
                }
                let mut globals = Vec::new();
                support::push_hint(
                    &mut globals,
                    "?",
                    "shortcuts",
                    ShortcutState::Enabled,
                    &palette,
                );
                let (navigation, center) = if centered {
                    (Vec::new(), hints)
                } else {
                    (hints, Vec::new())
                };
                terminal.draw(|frame| {
                    render_sections(
                        frame,
                        Rect::new(0, 0, width, 1),
                        &palette,
                        StatusLineItems {
                            uptime: vec![Span::raw(" Uptime: 13s ")],
                            disk_reads: Vec::new(),
                            navigation,
                            center,
                            notes: Vec::new(),
                            globals,
                            with_hints: Vec::new(),
                            hint_fit: HintFit::WholeGroups,
                        },
                    );
                    mode.clear_colors(frame.buffer_mut());
                })?;
                let row: String = (0..width)
                    .map(|x| terminal.backend().buffer()[(x, 0)].symbol())
                    .collect();
                assert!(row.starts_with(" Uptime: 13s "), "{row:?}");
                assert!(row.contains("? shortcuts"), "{row:?}");
                if width == 40 {
                    for hint in ["↑/↓ nav", "=/- all", "tab pane"] {
                        assert!(
                            !row.contains(hint),
                            "{mode:?}, centered={centered}: {row:?}"
                        );
                    }
                } else {
                    assert!(row.contains("↑/↓ nav   =/- all   tab pane"), "{row:?}");
                }
            }
        }
    }
    Ok(())
}

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
                                disk_reads: Vec::new(),
                                navigation,
                                center,
                                notes: vec![vec![Span::raw(format!(" {note} "))]],
                                globals: Vec::new(),
                                with_hints: Vec::new(),
                                hint_fit: HintFit::IndividualHints,
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
                                disk_reads: Vec::new(),
                                navigation: Vec::new(),
                                center,
                                notes: vec![vec![Span::raw(format!(" {note} "))]],
                                globals: Vec::new(),
                                with_hints: Vec::new(),
                                hint_fit: HintFit::IndividualHints,
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
                            disk_reads: Vec::new(),
                            navigation,
                            center: Vec::new(),
                            notes: vec![vec![Span::raw(format!(" {note} "))]],
                            globals,
                            with_hints: Vec::new(),
                            hint_fit: HintFit::IndividualHints,
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
