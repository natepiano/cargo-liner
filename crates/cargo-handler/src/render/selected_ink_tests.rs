//! Selected row inks are measured on the ground actually painted by the grid.

#![expect(clippy::expect_used, reason = "drawn fixture cells must exist")]
#![expect(
    clippy::panic,
    reason = "a missing fixture cell must fail the regression"
)]

use std::path::Path;
use std::sync::Arc;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Position;
use tui_pane::TerminalApp;

use super::*;
use crate::census::AgentRow;
use crate::census::CensusUpdate;
use crate::census::HierarchyEdges;
use crate::theme;

const CLOCK: u64 = 1_000_000;

fn position(buffer: &Buffer, label: &str) -> Position {
    for y in 0..buffer.area.height {
        let text: String = (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol())
            .collect();
        if let Some(index) = text.find(label) {
            return Position::new(
                u16::try_from(text[..index].chars().count()).expect("column"),
                y,
            );
        }
    }
    panic!("missing drawn {label}");
}

fn frame(app: &mut App, now: u64) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(200, 48)).expect("test terminal");
    let keymap = app.keymap();
    terminal
        .draw(|frame| draw_at_for_test(frame, app, &keymap, now))
        .expect("first frame");
    app.tiles.settle_for_test();
    terminal
        .draw(|frame| draw_at_for_test(frame, app, &keymap, now))
        .expect("settled frame");
    terminal.backend().buffer().clone()
}

/// Keep the composed grid allocated while selection changes between frames.
struct SelectedRowRenderer {
    terminal: Terminal<TestBackend>,
}

impl SelectedRowRenderer {
    fn new(app: &mut App, height: u16) -> Self {
        let mut renderer = Self {
            terminal: Terminal::new(TestBackend::new(200, height)).expect("selection terminal"),
        };
        renderer.paint(app, CLOCK);
        app.tiles.settle_for_test();
        renderer.paint(app, CLOCK);
        renderer
    }

    fn paint(&mut self, app: &mut App, now: u64) -> Buffer {
        let keymap = app.keymap();
        self.terminal
            .draw(|frame| draw_at_for_test(frame, app, &keymap, now))
            .expect("selected row frame");
        self.terminal.backend().buffer().clone()
    }
}

fn covered_contrast(ink: Color, ground: Color) -> f64 {
    fn rgb(color: Color) -> [u8; 3] {
        match tui_pane::blend_color(color, color, 0) {
            Color::Rgb(r, g, b) => [r, g, b],
            other => panic!("unresolved drawn color {other:?}"),
        }
    }
    fn luminance(rgb: [u8; 3]) -> f64 {
        let linear = rgb.map(|channel| {
            let value = f64::from(channel) / 255.0;
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        });
        f64::mul_add(
            linear[2],
            0.0722,
            f64::mul_add(linear[1], 0.7152, linear[0] * 0.2126),
        )
    }
    let ink = rgb(ink);
    let ground = rgb(ground);
    let covered = std::array::from_fn(|index| {
        u8::try_from((u32::from(ink[index]) * 208 + u32::from(ground[index]) * 47) / 255)
            .expect("weighted byte")
    });
    let (first, second) = (luminance(covered), luminance(ground));
    (first.max(second) + 0.05) / (first.min(second) + 0.05)
}

fn fixture() -> Vec<AgentRow> {
    serde_json::from_value(serde_json::json!([
        {"agent":"claude","pid":10,"name":"ink-owner","session_id":"owner","started":100,
         "directory":"/fixture","account":{"home":"~/.claude1","label":"claude1"},"children":[
            {"kind":"shell","depth":0,"pid":11,"name":"timed-shell","started":900_000,
             "timer":{"started":999_900,"deadline":CLOCK}},
            {"kind":"shell","depth":0,"pid":12,"name":"untimed-shell","started":900_001}
         ]},
        {"agent":"claude","pid":20,"name":"colored-session","session_id":"colored","started":200,
         "directory":"/fixture","role_fallback":{"worker":"unit"},
         "activity":{"active":"thinking"},"account":{"home":"~/.claude1","label":"claude1"},"children":[]},
        {"agent":"claude","pid":30,"name":"other-ground","session_id":"other","started":300,
         "directory":"/fixture","account":{"home":"~/.claude2","label":"claude2"},"children":[]}
    ])).expect("selected ink fixture")
}

#[test]
fn selected_row_inks_keep_glyph_contrast_for_sessions_roles_accounts_and_both_timer_states() {
    check_theme("Default Dark");
}

#[test]
fn selected_row_inks_keep_glyph_contrast_in_default_light() { check_theme("Default Light"); }

#[test]
fn selected_row_inks_keep_glyph_contrast_in_high_contrast_dark() {
    check_theme("High Contrast Dark");
}

#[test]
fn selected_row_inks_keep_glyph_contrast_in_high_contrast_light() {
    check_theme("High Contrast Light");
}

fn check_theme(theme_name: &str) {
    let previous = tui_pane::theme();
    let transparent = tui_pane::transparent_background();
    let tinted = tui_pane::focused_pane_tint_enabled();
    let mut failures = Vec::new();
    let mut measurements = Vec::new();
    let directory = tempfile::tempdir().expect("footer fixture directory");
    let palette = directory.path().join("statusline.conf");
    std::fs::write(
        &palette,
        "role.unit-director=bright-blue\naccount.claude1=brown\naccount.claude2=magenta\n",
    )
    .expect("footer fixture");
    tui_pane::set_transparent_background(false);
    let builtin = theme::builtins()
        .into_iter()
        .find(|builtin| builtin.id.as_str() == theme_name)
        .expect("named built-in theme");
    let name = builtin.id.as_str().to_owned();
    tui_pane::set_active_theme(Arc::new(builtin.theme));
    for tint in [false, true] {
        tui_pane::set_focused_pane_tint(tint);
        let mut app = App::new_for_test().expect("test app");
        app.loaded_config.config.tiles.initial_rows =
            serde_json::from_value(serde_json::json!(8)).expect("single column");
        app.loaded_config.config.tiles.widen_summary = false;
        app.footer_colors = crate::footer_colors::FooterColorFile::fixture(palette.clone());
        app.footer_colors.refresh();
        app.census.apply(CensusUpdate::Local {
            rows:      fixture(),
            hierarchy: serde_json::from_value::<HierarchyEdges>(serde_json::json!([
                {"manager":"owner","member":"colored","role":"unit"}
            ]))
            .expect("management relationship")
            .into(),
            stage:     ScanStage::Completed,
        });
        for (label, now, word) in [
            ("colored-session", CLOCK, "thinking"),
            ("timed-shell", CLOCK - 1, "0:01"),
            ("timed-shell", CLOCK, "running"),
            ("timed-shell", CLOCK + 90, "running"),
            ("untimed-shell", CLOCK, "running"),
        ] {
            let before = frame(&mut app, now);
            app.click(position(&before, label));
            for focused in [true, false] {
                if !focused {
                    let current = frame(&mut app, now);
                    app.click(position(&current, "other-ground"));
                }
                let drawn = frame(&mut app, now);
                let point = position(&drawn, label);
                let expected = if focused {
                    tui_pane::active_focus_color()
                } else {
                    tui_pane::remembered_focus_color()
                };
                assert_eq!(
                    drawn[(point.x, point.y)].bg,
                    expected,
                    "{label} focused={focused}"
                );
                let row: String = (0..drawn.area.width)
                    .map(|x| drawn[(x, point.y)].symbol())
                    .collect();
                assert!(row.contains(word), "{label} at {now}: {row}");
                assert!(row.contains("claude1"), "selected account: {row}");
                if label == "colored-session" {
                    assert!(row.contains("unit"), "selected management role: {row}");
                }
                for x in 0..drawn.area.width {
                    let cell = &drawn[(x, point.y)];
                    if cell.bg == expected && cell.symbol().chars().any(char::is_alphanumeric) {
                        let ratio = covered_contrast(cell.fg, cell.bg);
                        let measurement = format!(
                            "{name} tint={tint} focused={focused} {label} now={now} x={x} {} fg={:?} bg={:?} covered208={ratio:.9}",
                            cell.symbol(),
                            cell.fg,
                            cell.bg
                        );
                        measurements.push(measurement.clone());
                        if ratio < 4.5 {
                            failures.push(measurement);
                        }
                    }
                }
            }
        }
    }
    tui_pane::set_active_theme(previous);
    tui_pane::set_transparent_background(transparent);
    tui_pane::set_focused_pane_tint(tinted);
    save_selected_measurements(
        &format!("{}_coverage", theme_name.replace(' ', "_")),
        &measurements,
    );
    assert!(failures.is_empty(), "selected ink failures: {failures:#?}");
}

fn selection_application(palette: &Path) -> App {
    std::fs::write(
        palette,
        "role.unit-director=bright-blue\naccount.claude1=brown\naccount.claude2=magenta\n",
    )
    .expect("footer fixture");
    let mut app = App::new_for_test().expect("test app");
    app.loaded_config.config.tiles.initial_rows =
        serde_json::from_value(serde_json::json!(8)).expect("single column");
    app.loaded_config.config.tiles.widen_summary = false;
    app.footer_colors = crate::footer_colors::FooterColorFile::fixture(palette.to_owned());
    app.footer_colors.refresh();
    app.census.apply(CensusUpdate::Local {
        rows:      fixture(),
        hierarchy: serde_json::from_value::<HierarchyEdges>(serde_json::json!([
            {"manager":"owner","member":"colored","role":"unit"}
        ]))
        .expect("management relationship")
        .into(),
        stage:     ScanStage::Completed,
    });
    app
}

fn ground_contrast(first: Color, second: Color) -> f64 {
    fn luminance(color: Color) -> f64 {
        let Color::Rgb(red, green, blue) = tui_pane::blend_color(color, color, 0) else {
            panic!("unresolved ground {color:?}");
        };
        let linear = [red, green, blue].map(|channel| {
            let value = f64::from(channel) / 255.0;
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        });
        0.0722f64.mul_add(linear[2], 0.7152f64.mul_add(linear[1], 0.2126 * linear[0]))
    }
    let first = luminance(first);
    let second = luminance(second);
    (first.max(second) + 0.05) / (first.min(second) + 0.05)
}

#[test]
fn default_dark_selected_grounds_stay_distinct_from_the_tile_and_each_other() {
    let previous = tui_pane::theme();
    let transparent = tui_pane::transparent_background();
    let tinted = tui_pane::focused_pane_tint_enabled();
    let dark = theme::builtins()
        .into_iter()
        .find(|builtin| builtin.id.as_str() == "Default Dark")
        .expect("default dark theme");
    tui_pane::set_active_theme(Arc::new(dark.theme));
    tui_pane::set_transparent_background(false);
    let directory = tempfile::tempdir().expect("footer fixture directory");
    let mut failures = Vec::new();
    for tint in [false, true] {
        tui_pane::set_focused_pane_tint(tint);
        let mut app = selection_application(&directory.path().join("statusline.conf"));
        let before = frame(&mut app, CLOCK);
        app.click(position(&before, "timed-shell"));
        let active = frame(&mut app, CLOCK);
        let selected = position(&active, "timed-shell");
        let plain = position(&active, "untimed-shell");
        let active_ground = active[(selected.x, selected.y)].bg;
        let active_tile = active[(plain.x, plain.y)].bg;
        app.click(position(&active, "other-ground"));
        let remembered = frame(&mut app, CLOCK);
        let selected = position(&remembered, "timed-shell");
        let plain = position(&remembered, "untimed-shell");
        let remembered_ground = remembered[(selected.x, selected.y)].bg;
        let remembered_tile = remembered[(plain.x, plain.y)].bg;
        for (state, selection, tile, floor) in [
            ("focused", active_ground, active_tile, 1.8),
            ("remembered", remembered_ground, remembered_tile, 1.4),
        ] {
            let ratio = ground_contrast(selection, tile);
            if ratio < floor {
                failures.push(format!(
                    "tint={tint} {state} selection={selection:?} tile={tile:?} ratio={ratio} floor={floor}"
                ));
            }
        }
        let ratio = ground_contrast(active_ground, remembered_ground);
        if ratio < 1.25 {
            failures.push(format!("tint={tint} focused/remembered ratio={ratio}"));
        }
    }
    tui_pane::set_active_theme(previous);
    tui_pane::set_transparent_background(transparent);
    tui_pane::set_focused_pane_tint(tinted);
    assert!(
        failures.is_empty(),
        "selection ground failures: {failures:#?}"
    );
}

#[test]
fn selected_rows_keep_the_unselected_hue_in_default_dark() {
    check_selected_hues("Default Dark", false);
}

#[test]
fn selected_rows_keep_the_unselected_hue_in_default_light() {
    check_selected_hues("Default Light", false);
}

#[test]
fn selected_rows_keep_the_unselected_hue_in_high_contrast_dark() {
    check_selected_hues("High Contrast Dark", false);
}

#[test]
fn selected_rows_keep_the_unselected_hue_in_high_contrast_light() {
    check_selected_hues("High Contrast Light", false);
}

fn check_selected_hues(theme_name: &str, tint: bool) {
    let previous = tui_pane::theme();
    let transparent = tui_pane::transparent_background();
    let tinted = tui_pane::focused_pane_tint_enabled();
    let builtin = theme::builtins()
        .into_iter()
        .find(|builtin| builtin.id.as_str() == theme_name)
        .expect("built-in theme");
    tui_pane::set_active_theme(Arc::new(builtin.theme));
    tui_pane::set_transparent_background(false);
    let directory = tempfile::tempdir().expect("footer fixture directory");
    let mut failures = Vec::new();
    let mut checked_colors = Vec::new();
    tui_pane::set_focused_pane_tint(tint);
    let mut app = selection_application(&directory.path().join("statusline.conf"));
    let mut renderer = SelectedRowRenderer::new(&mut app, 48);
    for (label, now) in [
        ("colored-session", CLOCK),
        ("timed-shell", CLOCK - 1),
        ("timed-shell", CLOCK),
        ("timed-shell", CLOCK + 90),
        ("untimed-shell", CLOCK),
    ] {
        let alternative = if label == "untimed-shell" {
            "timed-shell"
        } else {
            "untimed-shell"
        };
        for focused in [true, false] {
            let before = renderer.paint(&mut app, now);
            app.click(position(&before, alternative));
            if !focused {
                let current = renderer.paint(&mut app, now);
                app.click(position(&current, "other-ground"));
            }
            let unselected = renderer.paint(&mut app, now);
            app.click(position(&unselected, label));
            if !focused {
                let current = renderer.paint(&mut app, now);
                app.click(position(&current, "other-ground"));
            }
            let selected = renderer.paint(&mut app, now);
            let old = position(&unselected, label);
            let new = position(&selected, label);
            let selected_ground = selected[(new.x, new.y)].bg;
            assert_ne!(
                selected_ground,
                unselected[(old.x, old.y)].bg,
                "row selection changes its ground"
            );
            for x in 0..selected.area.width {
                let cell = &selected[(x, new.y)];
                if cell.bg == selected_ground && cell.symbol().chars().any(char::is_alphanumeric) {
                    let original = &unselected[(x, old.y)];
                    let colors = (original.fg, cell.fg, cell.bg);
                    let valid_hue = if let Some((_, valid)) = checked_colors
                        .iter()
                        .find(|(measured, _)| *measured == colors)
                    {
                        *valid
                    } else {
                        let valid = same_oklch_hue_away_from_ground(original.fg, cell.fg, cell.bg);
                        checked_colors.push((colors, valid));
                        valid
                    };
                    if cell.symbol() != original.symbol()
                        || !valid_hue
                        || cell.modifier.intersects(
                            ratatui::style::Modifier::UNDERLINED
                                | ratatui::style::Modifier::REVERSED,
                        )
                    {
                        failures.push(format!("{theme_name} tint={tint} focused={focused} {label} now={now} x={x}: before={original:?} selected={cell:?}"));
                    }
                }
            }
        }
    }
    tui_pane::set_active_theme(previous);
    tui_pane::set_transparent_background(transparent);
    tui_pane::set_focused_pane_tint(tinted);
    assert!(
        failures.is_empty(),
        "selection changed hue, text or decoration: {failures:#?}"
    );
}

#[test]
fn selected_rows_keep_the_unselected_hue_in_tinted_default_dark() {
    check_selected_hues("Default Dark", true);
}

#[test]
fn selected_rows_keep_the_unselected_hue_in_tinted_default_light() {
    check_selected_hues("Default Light", true);
}

#[test]
fn selected_rows_keep_the_unselected_hue_in_tinted_high_contrast_dark() {
    check_selected_hues("High Contrast Dark", true);
}

#[test]
fn selected_rows_keep_the_unselected_hue_in_tinted_high_contrast_light() {
    check_selected_hues("High Contrast Light", true);
}

// Frozen from the committed main theme and ink solver before selected-row changes.
// Order: session, Busy, Shell, unit role, claude1 account, plain text.
#[test]
fn unselected_rows_keep_main_inks_in_default_dark() {
    check_main_inks(
        "Default Dark",
        [
            [
                Color::Rgb(218, 251, 142),
                Color::Rgb(100, 220, 100),
                Color::Rgb(140, 230, 240),
                Color::Rgb(129, 129, 255),
                Color::Rgb(176, 136, 96),
                Color::Rgb(255, 255, 255),
            ],
            [
                Color::Rgb(218, 251, 142),
                Color::Rgb(100, 220, 100),
                Color::Rgb(140, 230, 240),
                Color::Rgb(165, 165, 255),
                Color::Rgb(197, 169, 140),
                Color::Rgb(255, 255, 255),
            ],
        ],
    );
}

#[test]
fn unselected_rows_keep_main_inks_in_default_light() {
    check_main_inks(
        "Default Light",
        [
            [
                Color::Rgb(180, 20, 110),
                Color::Rgb(0, 109, 0),
                Color::Rgb(0, 104, 87),
                Color::Rgb(62, 62, 255),
                Color::Rgb(111, 83, 56),
                Color::Rgb(0, 0, 0),
            ],
            [
                Color::Rgb(159, 18, 97),
                Color::Rgb(0, 90, 0),
                Color::Rgb(0, 86, 72),
                Color::Rgb(37, 37, 255),
                Color::Rgb(91, 69, 46),
                Color::Rgb(0, 0, 0),
            ],
        ],
    );
}

#[test]
fn unselected_rows_keep_main_inks_in_high_contrast_dark() {
    check_main_inks(
        "High Contrast Dark",
        [
            [
                Color::Rgb(205, 246, 140),
                Color::Rgb(0, 255, 0),
                Color::Rgb(0, 255, 255),
                Color::Rgb(129, 129, 255),
                Color::Rgb(176, 136, 96),
                Color::Rgb(255, 255, 255),
            ],
            [
                Color::Rgb(205, 246, 140),
                Color::Rgb(0, 255, 0),
                Color::Rgb(0, 255, 255),
                Color::Rgb(165, 165, 255),
                Color::Rgb(197, 169, 140),
                Color::Rgb(255, 255, 255),
            ],
        ],
    );
}

#[test]
fn unselected_rows_keep_main_inks_in_high_contrast_light() {
    check_main_inks(
        "High Contrast Light",
        [
            [
                Color::Rgb(203, 4, 119),
                Color::Rgb(0, 100, 0),
                Color::Rgb(0, 80, 65),
                Color::Rgb(62, 62, 255),
                Color::Rgb(111, 83, 56),
                Color::Rgb(0, 0, 0),
            ],
            [
                Color::Rgb(168, 4, 99),
                Color::Rgb(0, 90, 0),
                Color::Rgb(0, 80, 65),
                Color::Rgb(37, 37, 255),
                Color::Rgb(91, 69, 46),
                Color::Rgb(0, 0, 0),
            ],
        ],
    );
}

fn check_main_inks(theme_name: &str, expected: [[Color; 6]; 2]) {
    let previous = tui_pane::theme();
    let tinted = tui_pane::focused_pane_tint_enabled();
    let transparent = tui_pane::transparent_background();
    let builtin = theme::builtins()
        .into_iter()
        .find(|builtin| builtin.id.as_str() == theme_name)
        .expect("main theme");
    tui_pane::set_active_theme(Arc::new(builtin.theme));
    tui_pane::set_transparent_background(false);
    let directory = tempfile::tempdir().expect("main fixture");
    let mut failures = Vec::new();
    for (tint, colors) in [false, true].into_iter().zip(expected) {
        tui_pane::set_focused_pane_tint(tint);
        let mut app = selection_application(&directory.path().join("footer.conf"));
        for focused in [true, false] {
            let before = frame(&mut app, CLOCK - 1);
            app.click(position(&before, "untimed-shell"));
            if !focused {
                let current = frame(&mut app, CLOCK - 1);
                app.click(position(&current, "other-ground"));
            }
            for (now, label, words) in [
                (
                    CLOCK - 1,
                    "colored-session",
                    vec![
                        ("colored-session", 0),
                        ("thinking", 1),
                        ("unit", 3),
                        ("claude1", 4),
                    ],
                ),
                (
                    CLOCK - 1,
                    "timed-shell",
                    vec![("0:01", 2), ("shell", 5), ("claude1", 4)],
                ),
                (
                    CLOCK,
                    "timed-shell",
                    vec![("running", 1), ("shell", 5), ("claude1", 4)],
                ),
                (
                    CLOCK + 90,
                    "timed-shell",
                    vec![("running", 1), ("shell", 5), ("claude1", 4)],
                ),
            ] {
                let buffer = frame(&mut app, now);
                let point = position(&buffer, label);
                assert_ne!(
                    buffer[(point.x, point.y)].bg,
                    if focused {
                        tui_pane::active_focus_color()
                    } else {
                        tui_pane::remembered_focus_color()
                    },
                    "baseline row is unselected"
                );
                for (word, index) in words {
                    let actual = row_ink(&buffer, label, word);
                    if tui_pane::blend_color(actual, actual, 0) != colors[index] {
                        failures.push(format!("{theme_name} tint={tint} focused={focused} {label} {word}: {actual:?} != {:?}", colors[index]));
                    }
                }
            }
        }
    }
    tui_pane::set_active_theme(previous);
    tui_pane::set_focused_pane_tint(tinted);
    tui_pane::set_transparent_background(transparent);
    assert!(failures.is_empty(), "main ink differences: {failures:#?}");
}

/// Read a word from one actual tree row, without matching a neighboring tile.
fn row_ink(buffer: &Buffer, label: &str, word: &str) -> Color {
    let point = position(buffer, label);
    let text: String = (0..buffer.area.width)
        .map(|x| buffer[(x, point.y)].symbol())
        .collect();
    let index = text
        .match_indices(word)
        .find_map(|(offset, _)| {
            let before = text[..offset].chars().next_back();
            let after = text[offset + word.len()..].chars().next();
            (before.is_none_or(char::is_whitespace) && after.is_none_or(char::is_whitespace))
                .then_some(offset)
        })
        .expect("drawn whole color word");
    let x = u16::try_from(text[..index].chars().count()).expect("column");
    buffer[(x, point.y)].fg
}

/// Drawn ink measured in Oklab for independent hue and chroma checks.
/// Conversion matrices: <https://bottosson.github.io/posts/oklab>/ .
struct OklabInk {
    lightness:   f64,
    red_green:   f64,
    yellow_blue: f64,
}

impl OklabInk {
    fn from_drawn(color: Color) -> Self {
        let [red, green, blue] = drawn_rgb(color).map(|channel| {
            let channel = channel / 255.0;
            if channel <= 0.040_45 {
                channel / 12.92
            } else {
                ((channel + 0.055) / 1.055).powf(2.4)
            }
        });
        let long = 0.051_445_992_9f64
            .mul_add(
                blue,
                0.536_332_536_3f64.mul_add(green, 0.412_221_470_8 * red),
            )
            .cbrt();
        let medium = 0.107_396_956_6f64
            .mul_add(
                blue,
                0.680_699_545_1f64.mul_add(green, 0.211_903_498_2 * red),
            )
            .cbrt();
        let short = 0.629_978_700_5f64
            .mul_add(
                blue,
                0.281_718_837_6f64.mul_add(green, 0.088_302_461_9 * red),
            )
            .cbrt();
        Self {
            lightness:   0.004_072_046_8f64.mul_add(
                -short,
                0.793_617_785_0f64.mul_add(medium, 0.210_454_255_3 * long),
            ),
            red_green:   0.450_593_709_9f64.mul_add(
                short,
                2.428_592_205_0f64.mul_add(-medium, 1.977_998_495_1 * long),
            ),
            yellow_blue: 0.808_675_766_0f64.mul_add(
                -short,
                0.782_771_766_2f64.mul_add(medium, 0.025_904_037_1 * long),
            ),
        }
    }

    /// Bound display quantization by the neighboring one-byte RGB cube.
    fn rounding_tolerance(&self, color: Color) -> f64 {
        let Color::Rgb(red, green, blue) = tui_pane::blend_color(color, color, 0) else {
            panic!("concrete drawn ink");
        };
        let mut tolerance = 0.0_f64;
        for red in [red.saturating_sub(1), red.saturating_add(1)] {
            for green in [green.saturating_sub(1), green.saturating_add(1)] {
                for blue in [blue.saturating_sub(1), blue.saturating_add(1)] {
                    let adjacent = Self::from_drawn(Color::Rgb(red, green, blue));
                    let distance = (adjacent.lightness - self.lightness)
                        .hypot(adjacent.red_green - self.red_green)
                        .hypot(adjacent.yellow_blue - self.yellow_blue);
                    tolerance = tolerance.max(distance);
                }
            }
        }
        tolerance
    }
}

fn drawn_rgb(color: Color) -> [f64; 3] {
    let Color::Rgb(red, green, blue) = tui_pane::blend_color(color, color, 0) else {
        panic!("concrete drawn ink");
    };
    [red, green, blue].map(f64::from)
}

/// Keep the measured hue ray; gamut mapping may reduce chroma, never increase it.
/// Display quantization sets the tolerance, independently of the production search.
fn same_oklch_hue_away_from_ground(original: Color, selected: Color, ground: Color) -> bool {
    if tui_pane::blend_color(original, original, 0) == tui_pane::blend_color(selected, selected, 0)
    {
        return true;
    }
    let source = OklabInk::from_drawn(original);
    let result = OklabInk::from_drawn(selected);
    let tolerance = result.rounding_tolerance(selected);
    let source_chroma = source.red_green.hypot(source.yellow_blue);
    let result_chroma = result.red_green.hypot(result.yellow_blue);
    let light_ground =
        ground_contrast(ground, Color::Black) > ground_contrast(ground, Color::White);
    let away_from_ground = if light_ground {
        result.lightness <= source.lightness + tolerance
    } else {
        result.lightness + tolerance >= source.lightness
    };
    if source_chroma <= f64::EPSILON {
        return away_from_ground && result_chroma <= tolerance;
    }
    let along_hue = source
        .yellow_blue
        .mul_add(result.yellow_blue, source.red_green * result.red_green)
        / source_chroma;
    let across_hue = source
        .yellow_blue
        .mul_add(-result.red_green, source.red_green * result.yellow_blue)
        / source_chroma;
    away_from_ground
        && along_hue >= -tolerance
        && across_hue.abs() <= tolerance
        && result_chroma <= source_chroma + tolerance
}

fn save_selected_measurements(name: &str, measurements: &[String]) {
    let directory = std::env::temp_dir().join("cargo-handler-selected-ink-measurements");
    std::fs::create_dir_all(&directory).expect("measurement artifact directory");
    let path = directory.join(format!("{}_{name}.txt", std::process::id()));
    std::fs::write(&path, measurements.join("\n")).expect("selected ink measurements");
    eprintln!("selected ink measurements: {}", path.display());
}

/// Each family comes from the footer fixture or the active production hue table.
#[derive(Clone, Copy)]
enum FullPaletteRows {
    Roles,
    Accounts,
    Sessions,
}

fn palette_rows(family: FullPaletteRows) -> (Vec<AgentRow>, Vec<(String, String)>) {
    let footer = include_str!("../footer_colors/fixtures/statusline.conf");
    let assignments: Vec<_> = footer
        .lines()
        .filter_map(|line| {
            let (key, _) = line.split_once('=')?;
            Some(key.trim())
        })
        .collect();
    let labels: Vec<String> = match family {
        FullPaletteRows::Roles => assignments
            .iter()
            .filter_map(|key| key.strip_prefix("role.").map(str::to_owned))
            .collect(),
        FullPaletteRows::Accounts => assignments
            .iter()
            .filter_map(|key| key.strip_prefix("account.").map(str::to_owned))
            .collect(),
        FullPaletteRows::Sessions => tui_pane::theme()
            .roles
            .keys()
            .filter(|key| key.starts_with("session_"))
            .cloned()
            .collect(),
    };
    let mut targets = Vec::new();
    let mut rows = Vec::new();
    for (index, word) in labels.iter().enumerate() {
        let pid = 100 + u32::try_from(index).expect("palette row");
        let label = format!("palette-row-{index:02}");
        match family {
            FullPaletteRows::Roles => {
                targets.push((label.clone(), word.clone()));
            },
            FullPaletteRows::Accounts | FullPaletteRows::Sessions => {
                let mut value = serde_json::json!({
                    "agent":"claude", "pid":pid, "session_id":format!("palette-{pid}"),
                    "name":label, "started":pid, "directory":"/fixture", "status":"idle", "children":[]
                });
                if matches!(family, FullPaletteRows::Accounts) {
                    value["account"] =
                        serde_json::json!({"home":format!("~/.{word}"),"label":word});
                    targets.push((label, word.clone()));
                } else {
                    value["children"] = serde_json::json!([{
                        "kind":"shell", "depth":0, "pid":pid+1000, "name":format!("hue-work-{index}"), "started":pid
                    }]);
                    targets.push((label.clone(), label));
                }
                rows.push(serde_json::from_value(value).expect("palette session"));
            },
        }
    }
    let children: Vec<_> = if matches!(family, FullPaletteRows::Roles) {
        labels
            .iter()
            .zip(&targets)
            .enumerate()
            .map(|(index, (role, (label, _)))| {
                serde_json::json!({
                    "kind":"subagent","name":label,"depth":0,"started":100+index,
                    "subagent_role":crate::census::transcript::SubagentRole::from_record(role,"")
                })
            })
            .collect()
    } else {
        vec![
            serde_json::json!({"kind":"shell","depth":0,"pid":11,"name":"palette-focus-away","started":100}),
        ]
    };
    rows.push(
        serde_json::from_value(serde_json::json!({
            "agent":"claude","pid":10,"session_id":"palette-owner","name":"palette-owner",
            "started":100,"directory":"/fixture","children":children
        }))
        .expect("palette owner"),
    );
    if matches!(family, FullPaletteRows::Roles) {
        rows.push(
            serde_json::from_value(serde_json::json!({
                "agent":"claude","pid":20,"session_id":"palette-away","name":"palette-away",
                "started":100,"directory":"/fixture","children":[
                    {"kind":"shell","depth":0,"pid":21,"name":"palette-focus-away","started":100}
                ]
            }))
            .expect("second focus surface"),
        );
    }
    (rows, targets)
}

fn check_full_default_dark_palette(family: FullPaletteRows, tint: bool, active: bool) {
    let previous = tui_pane::theme();
    let transparent = tui_pane::transparent_background();
    let tinted = tui_pane::focused_pane_tint_enabled();
    let dark = theme::builtins()
        .into_iter()
        .find(|variant| variant.id.as_str() == "Default Dark")
        .expect("default dark");
    tui_pane::set_active_theme(Arc::new(dark.theme));
    tui_pane::set_transparent_background(false);
    tui_pane::set_focused_pane_tint(tint);
    let directory = tempfile::tempdir().expect("full palette fixture");
    let path = directory.path().join("footer.conf");
    std::fs::write(
        &path,
        include_str!("../footer_colors/fixtures/statusline.conf"),
    )
    .expect("committed palette");
    let mut app = App::new_for_test().expect("palette application");
    app.loaded_config.config.tiles.initial_rows =
        serde_json::from_value(serde_json::json!(8)).expect("single column");
    app.loaded_config.config.tiles.widen_summary = false;
    app.footer_colors = crate::footer_colors::FooterColorFile::fixture(path);
    app.footer_colors.refresh();
    let (rows, targets) = palette_rows(family);
    app.census.apply(CensusUpdate::Local {
        rows,
        hierarchy: HierarchyEdges::default().into(),
        stage: ScanStage::Completed,
    });
    let mut renderer = SelectedRowRenderer::new(&mut app, 80);
    let mut measurements = Vec::new();
    let mut failures = Vec::new();
    for (index, (label, word)) in targets.iter().enumerate() {
        let current = renderer.paint(&mut app, CLOCK);
        let alternate = &targets[(index + 1) % targets.len()].0;
        app.click(position(&current, alternate));
        let original = renderer.paint(&mut app, CLOCK);
        let original_ink = row_ink(&original, label, word);
        app.click(position(&original, label));
        let mut selected = renderer.paint(&mut app, CLOCK);
        if !active {
            app.click(position(&selected, "palette-focus-away"));
            selected = renderer.paint(&mut app, CLOCK);
        }
        let point = position(&selected, label);
        let ground = selected[(point.x, point.y)].bg;
        let expected_ground = if active {
            tui_pane::active_focus_color()
        } else {
            tui_pane::remembered_focus_color()
        };
        assert_eq!(ground, expected_ground, "selected palette row {label}");
        let ink = row_ink(&selected, label, word);
        let ratio = covered_contrast(ink, ground);
        let hue = same_oklch_hue_away_from_ground(original_ink, ink, ground);
        let measurement = format!(
            "{word} tint={tint} active={active} original={original_ink:?} selected={ink:?} ground={ground:?} coverage208={ratio:.9} hue={hue}"
        );
        measurements.push(measurement.clone());
        if ratio < 4.5 || !hue {
            failures.push(measurement);
        }
    }
    tui_pane::set_active_theme(previous);
    tui_pane::set_transparent_background(transparent);
    tui_pane::set_focused_pane_tint(tinted);
    let name = match family {
        FullPaletteRows::Roles => "roles",
        FullPaletteRows::Accounts => "accounts",
        FullPaletteRows::Sessions => "sessions",
    };
    save_selected_measurements(
        &format!("full_{name}_tint{tint}_active{active}"),
        &measurements,
    );
    assert!(
        failures.is_empty(),
        "full selected palette failures: {failures:#?}"
    );
}

#[test]
fn every_default_dark_roles_ink_keeps_coverage_and_hue_on_active_selection() {
    check_full_default_dark_palette(FullPaletteRows::Roles, false, true);
}

#[test]
fn every_default_dark_roles_ink_keeps_coverage_and_hue_on_remembered_selection() {
    check_full_default_dark_palette(FullPaletteRows::Roles, false, false);
}

#[test]
fn every_default_dark_roles_ink_keeps_coverage_and_hue_on_tinted_active_selection() {
    check_full_default_dark_palette(FullPaletteRows::Roles, true, true);
}

#[test]
fn every_default_dark_roles_ink_keeps_coverage_and_hue_on_tinted_remembered_selection() {
    check_full_default_dark_palette(FullPaletteRows::Roles, true, false);
}

#[test]
fn every_default_dark_accounts_ink_keeps_coverage_and_hue_on_active_selection() {
    check_full_default_dark_palette(FullPaletteRows::Accounts, false, true);
}

#[test]
fn every_default_dark_accounts_ink_keeps_coverage_and_hue_on_remembered_selection() {
    check_full_default_dark_palette(FullPaletteRows::Accounts, false, false);
}

#[test]
fn every_default_dark_accounts_ink_keeps_coverage_and_hue_on_tinted_active_selection() {
    check_full_default_dark_palette(FullPaletteRows::Accounts, true, true);
}

#[test]
fn every_default_dark_accounts_ink_keeps_coverage_and_hue_on_tinted_remembered_selection() {
    check_full_default_dark_palette(FullPaletteRows::Accounts, true, false);
}

#[test]
fn every_default_dark_sessions_ink_keeps_coverage_and_hue_on_active_selection() {
    check_full_default_dark_palette(FullPaletteRows::Sessions, false, true);
}

#[test]
fn every_default_dark_sessions_ink_keeps_coverage_and_hue_on_remembered_selection() {
    check_full_default_dark_palette(FullPaletteRows::Sessions, false, false);
}

#[test]
fn every_default_dark_sessions_ink_keeps_coverage_and_hue_on_tinted_active_selection() {
    check_full_default_dark_palette(FullPaletteRows::Sessions, true, true);
}

#[test]
fn every_default_dark_sessions_ink_keeps_coverage_and_hue_on_tinted_remembered_selection() {
    check_full_default_dark_palette(FullPaletteRows::Sessions, true, false);
}
