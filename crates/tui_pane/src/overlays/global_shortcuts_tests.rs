//! Rendered global-shortcut paging regressions.
#![expect(
    clippy::expect_used,
    reason = "render fixtures fail immediately on setup errors"
)]
use crossterm::event::KeyCode;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::Color;

use crate::Action;
use crate::AppContext;
use crate::Appearance;
use crate::Bindings;
use crate::Framework;
use crate::GlobalShortcutsPane;
use crate::Globals;
use crate::Keymap;
use crate::NoToastAction;
use crate::PaneFocusState;

struct ShortcutApp {
    framework: Framework<Self>,
}
impl AppContext for ShortcutApp {
    type AppPaneId = ();
    type ToastAction = NoToastAction;
    fn framework(&self) -> &Framework<Self> { &self.framework }
    fn framework_mut(&mut self) -> &mut Framework<Self> { &mut self.framework }
}
crate::action_enum! {
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub enum ShortcutAction {
        Row01 => ("row_01", "row01", "Inspect shortcut 01");
        Row02 => ("row_02", "row02", "Inspect shortcut 02");
        Row03 => ("row_03", "row03", "Inspect shortcut 03");
        Row04 => ("row_04", "row04", "Inspect shortcut 04");
        Row05 => ("row_05", "row05", "Inspect shortcut 05");
        Row06 => ("row_06", "row06", "Inspect shortcut 06");
        Row07 => ("row_07", "row07", "Inspect shortcut 07");
        Row08 => ("row_08", "row08", "Inspect shortcut 08");
        Row09 => ("row_09", "row09", "Inspect shortcut 09");
        Row10 => ("row_10", "row10", "Inspect shortcut 10");
        Row11 => ("row_11", "row11", "Inspect shortcut 11");
        Row12 => ("row_12", "row12", "Inspect shortcut 12");
        Row13 => ("row_13", "row13", "Inspect shortcut 13");
        Row14 => ("row_14", "row14", "Inspect shortcut 14");
        Row15 => ("row_15", "row15", "Inspect shortcut 15");
        Row16 => ("row_16", "row16", "Inspect shortcut 16");
        Row17 => ("row_17", "row17", "Inspect shortcut 17");
        Row18 => ("row_18", "row18", "Inspect shortcut 18");
        Row19 => ("row_19", "row19", "Inspect shortcut 19");
        Row20 => ("row_20", "row20", "Inspect shortcut 20");
    }
}
struct ShortcutGlobals;
impl Globals<ShortcutApp> for ShortcutGlobals {
    type Actions = ShortcutAction;
    fn render_order() -> &'static [ShortcutAction] { ShortcutAction::ALL }
    fn defaults() -> Bindings<ShortcutAction> {
        crate::bindings! { 'z' => ShortcutAction::Row01 }
    }
    fn dispatcher() -> fn(ShortcutAction, &mut ShortcutApp) { |_, _| {} }
}
fn keymap() -> Keymap<ShortcutApp> {
    Keymap::builder()
        .register_globals::<ShortcutGlobals>()
        .expect("shortcut scope")
        .build()
        .expect("shortcut keymap")
}
fn draw(pane: &mut GlobalShortcutsPane, keymap: &Keymap<ShortcutApp>, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(110, height)).expect("overlay terminal");
    terminal
        .draw(|frame| pane.render(frame, frame.area(), keymap))
        .expect("overlay frame");
    terminal.backend().buffer().clone()
}
fn screen(buffer: &Buffer) -> String {
    buffer
        .content
        .iter()
        .map(ratatui::buffer::Cell::symbol)
        .collect()
}
#[test]
fn tall_global_shortcuts_draw_every_action_without_a_page_marker() {
    let keymap = keymap();
    let rows = keymap.global_shortcut_rows();
    let buffer = draw(&mut GlobalShortcutsPane::new(), &keymap, 64);
    let text = screen(&buffer);
    for row in rows {
        assert!(
            text.contains(row.description),
            "missing shortcut {}",
            row.description
        );
    }
    assert!(!text.contains(" of "), "all rows fit: {text}");
}
#[test]
fn tall_global_shortcuts_keep_blank_frame_padding_below_the_last_action() {
    let keymap = keymap();
    let rows = keymap.global_shortcut_rows();
    let mut pane = GlobalShortcutsPane::new();
    let buffer = draw(&mut pane, &keymap, 64);
    let inner = pane.viewport().content_area();
    let padding_y = inner.bottom() - 1;
    let last_action_y = padding_y - 1;
    let action_text: String = (inner.x..inner.right())
        .map(|x| buffer[(x, last_action_y)].symbol())
        .collect();

    assert!(
        action_text.contains(rows.last().expect("last shortcut").description),
        "last action must sit immediately above frame padding: {action_text}"
    );
    for x in inner.x..inner.right() {
        assert_eq!(buffer[(x, padding_y)].symbol(), " ", "padding at {x}");
    }
    assert_eq!(buffer[(inner.x, inner.bottom())].symbol(), "─");
}
#[test]
fn exact_fit_global_shortcuts_draw_every_action_without_paging_for_trailing_padding() {
    let keymap = keymap();
    let rows = keymap.global_shortcut_rows();
    let section_count = rows
        .chunk_by(|first, second| first.section == second.section)
        .count();
    // Keep the leading blank, each section heading and each action. The final
    // padding blank must not force another page. Margins and borders use four rows.
    let required_rows = 1 + section_count + rows.len();
    let height = u16::try_from(required_rows + 4).expect("fixture terminal height");
    let mut pane = GlobalShortcutsPane::new();
    let text = screen(&draw(&mut pane, &keymap, height));
    for row in &rows {
        assert!(
            text.contains(row.description),
            "exact-fit height {height} misses shortcut {}: {text}",
            row.description
        );
    }
    assert!(
        !text.contains(" of "),
        "every action fits at height {height}; trailing padding must not page: {text}"
    );
    pane.handle_navigation_key(KeyCode::End);
    let last = screen(&draw(&mut pane, &keymap, height));
    assert!(last.contains(rows.last().expect("last shortcut").description));
    assert!(
        !last.contains(" of "),
        "exact-fit End must not page: {last}"
    );
}
#[test]
fn short_global_shortcuts_fill_the_available_page_and_reach_the_last_action() {
    let keymap = keymap();
    let rows = keymap.global_shortcut_rows();
    let height = 30;
    // Two terminal margin rows and two popup border rows leave 26 content rows.
    let usable = usize::from(height - 4);
    let mut logical = vec![String::new()];
    let mut section = "";
    for row in &rows {
        if section != row.section {
            section = row.section;
            logical.push(format!("{section}:"));
        }
        logical.push(row.description.to_owned());
    }
    let pages = logical.len().div_ceil(usable);
    let mut pane = GlobalShortcutsPane::new();
    let first = screen(&draw(&mut pane, &keymap, height));
    assert!(
        first.contains(&format!("1 of {pages}")),
        "first page marker: {first}"
    );
    for line in logical.iter().take(usable).filter(|line| !line.is_empty()) {
        assert!(
            first.contains(line),
            "available first-page row missing: {line}: {first}"
        );
    }
    assert!(
        !first.contains(&logical[usable]),
        "next-page row must not overlap the border"
    );
    pane.handle_navigation_key(KeyCode::End);
    let last = screen(&draw(&mut pane, &keymap, height));
    assert!(
        last.contains(rows.last().expect("last shortcut").description),
        "last action unreachable"
    );
    assert!(
        last.contains(&format!("{pages} of {pages}")),
        "last page marker: {last}"
    );
}

fn glyph_rgb(color: Color) -> [u8; 3] {
    assert_ne!(
        color,
        Color::Reset,
        "drawn shortcut must have an explicit color"
    );
    let index = match color {
        Color::Rgb(r, g, b) => return [r, g, b],
        Color::Indexed(index) => index,
        Color::Black | Color::Reset => 0,
        Color::Red => 1,
        Color::Green => 2,
        Color::Yellow => 3,
        Color::Blue => 4,
        Color::Magenta => 5,
        Color::Cyan => 6,
        Color::Gray => 7,
        Color::DarkGray => 8,
        Color::LightRed => 9,
        Color::LightGreen => 10,
        Color::LightYellow => 11,
        Color::LightBlue => 12,
        Color::LightMagenta => 13,
        Color::LightCyan => 14,
        Color::White => 15,
    };
    let ansi = [
        [0, 0, 0],
        [205, 0, 0],
        [0, 205, 0],
        [205, 205, 0],
        [0, 0, 238],
        [205, 0, 205],
        [0, 205, 205],
        [229, 229, 229],
        [127, 127, 127],
        [255, 0, 0],
        [0, 255, 0],
        [255, 255, 0],
        [92, 92, 255],
        [255, 0, 255],
        [0, 255, 255],
        [255, 255, 255],
    ];
    match index {
        0..=15 => ansi[usize::from(index)],
        16..=231 => {
            let levels = [0, 95, 135, 175, 215, 255];
            let value = usize::from(index - 16);
            [levels[value / 36], levels[value / 6 % 6], levels[value % 6]]
        },
        _ => [8 + (index - 232) * 10; 3],
    }
}
fn luminance(channels: [f64; 3]) -> f64 {
    let linear = channels.map(|channel| {
        let value = channel / 255.0;
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
fn covered_contrast(ink: Color, ground: Color) -> f64 {
    let ink = glyph_rgb(ink).map(f64::from);
    let ground = glyph_rgb(ground).map(f64::from);
    let covered =
        std::array::from_fn(|index| f64::mul_add(ground[index], 47.0, ink[index] * 208.0) / 255.0);
    let first = luminance(covered);
    let second = luminance(ground);
    (first.max(second) + 0.05) / (first.min(second) + 0.05)
}

fn check_selected_global_shortcuts(appearance: crate::Appearance) {
    let previous = crate::theme();
    let theme = crate::fallback_theme(appearance);
    let expected_ink = theme.text.default.color;
    crate::set_active_theme(std::sync::Arc::new(theme));
    let keymap = keymap();
    let selected = keymap
        .global_shortcut_rows()
        .iter()
        .position(|row| row.description == "Inspect shortcut 01")
        .expect("fixture action is registered");
    let mut failures = Vec::new();
    for focus in [
        PaneFocusState::Active,
        PaneFocusState::Inactive,
        PaneFocusState::Remembered,
    ] {
        let mut pane = GlobalShortcutsPane::new();
        pane.focus.pane_focus_state = focus;
        pane.viewport_mut().set_pos(selected);
        pane.viewport_mut().set_hovered(Some(selected));
        let buffer = draw(&mut pane, &keymap, 64);
        let inner = pane.viewport().content_area();
        let y = inner.y
            + u16::try_from(pane.line_for_selection(selected).expect("selected line"))
                .expect("selected line fits terminal");
        let mut row = Buffer::empty(ratatui::layout::Rect::new(0, 0, inner.width, 1));
        for x in 0..inner.width {
            row[(x, 0)] = buffer[(inner.x + x, y)].clone();
        }
        let case = format!("fallback {appearance:?} {focus:?}");
        let text = screen(&row);
        for span in ["▸", "Inspect shortcut 01", "z"] {
            if !text.contains(span) {
                failures.push(format!("{case}: missing drawn span {span:?}: {text}"));
            }
        }
        for cell in &row.content {
            if cell.symbol().trim().is_empty() {
                continue;
            }
            let ratio = covered_contrast(cell.fg, cell.bg);
            if ratio < 4.5 {
                failures.push(format!(
                    "{case}: {:?} ink={:?} ground={:?}: {ratio:.9}:1",
                    cell.symbol(),
                    cell.fg,
                    cell.bg
                ));
            }
            if cell.fg != expected_ink {
                failures.push(format!(
                    "{case}: {:?} uses {:?}, expected {expected_ink:?}",
                    cell.symbol(),
                    cell.fg
                ));
            }
        }
    }
    crate::set_active_theme(previous);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn selected_global_shortcuts_are_readable_in_fallback_dark() {
    check_selected_global_shortcuts(Appearance::Dark);
}

#[test]
fn selected_global_shortcuts_are_readable_in_fallback_light() {
    check_selected_global_shortcuts(Appearance::Light);
}
