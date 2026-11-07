use std::time::Duration;
use std::time::Instant;

use ratatui::Terminal;
use ratatui::backend::TestBackend;

use super::ToastsRenderCtx;
use crate::AppContext;
use crate::ColorMode;
use crate::Framework;
use crate::PaneFocusState;
use crate::Renderable;
use crate::Toasts;
use crate::TrackedItem;

const ROWS: [&str; 8] = [
    "Cargo metadata",
    "crates.io",
    "Disk usage",
    "GitHub repos",
    "Languages",
    "Lint history",
    "Local git repos",
    "Test counts",
];

struct ToastTestApp {
    framework: Framework<Self>,
}

impl AppContext for ToastTestApp {
    type AppPaneId = ();
    type ToastAction = ();

    fn framework(&self) -> &Framework<Self> { &self.framework }

    fn framework_mut(&mut self) -> &mut Framework<Self> { &mut self.framework }
}

fn assert_hidden_rows_use_bottom_page_mark(toasts: &mut Toasts<ToastTestApp>, mode: ColorMode) {
    let mut terminal = Terminal::new(TestBackend::new(62, 8)).expect("the terminal initializes");
    terminal
        .draw(|frame| {
            Renderable::render(
                toasts,
                frame,
                frame.area(),
                &ToastsRenderCtx {
                    now:              Instant::now() + Duration::from_secs(5),
                    pane_focus_state: PaneFocusState::Inactive,
                },
            );
            mode.clear_colors(frame.buffer_mut());
        })
        .expect("the toast draws");
    let buffer = terminal.backend().buffer();
    let card = toasts.hits()[0].card_rect;
    let bottom: String = (card.x..card.right())
        .map(|x| buffer[(x, card.bottom() - 1)].symbol())
        .collect();
    assert!(bottom.contains('▼'), "{mode:?}: {bottom}");
    let mut visible = 0;
    for y in card.y + 1..card.bottom() - 1 {
        let row: String = (card.x + 1..card.right() - 1)
            .map(|x| buffer[(x, y)].symbol())
            .collect();
        assert!(!row.contains('…'), "{mode:?}: {row}");
        if row.trim().starts_with("(+") {
            assert_eq!(row.trim(), format!("(+{} more)", ROWS.len() - visible));
            assert_eq!(y, card.bottom() - 2);
        } else {
            assert!(row.contains(ROWS[visible]), "{mode:?}: {row}");
            visible += 1;
        }
    }
    assert!(visible > 0 && visible < ROWS.len());
    assert_eq!(buffer[(card.x, card.bottom() - 1)].symbol(), "└");
    assert_eq!(buffer[(card.right() - 1, card.bottom() - 1)].symbol(), "┘");
}

#[test]
fn plain_hidden_rows_draw_a_bottom_page_mark_in_both_color_modes() {
    for mode in [ColorMode::Colored, ColorMode::NoColor] {
        let mut toasts = Toasts::<ToastTestApp>::new();
        toasts.push("Startup", ROWS.join("\n"));
        assert_hidden_rows_use_bottom_page_mark(&mut toasts, mode);
    }
}

#[test]
fn tracked_hidden_rows_draw_a_bottom_page_mark_in_both_color_modes() {
    for mode in [ColorMode::Colored, ColorMode::NoColor] {
        let mut toasts = Toasts::<ToastTestApp>::new();
        let task = toasts.start_task("Startup", "");
        let items = ROWS.map(|label| TrackedItem::new(label, label));
        assert!(toasts.set_tracked_items(task, &items));
        assert_hidden_rows_use_bottom_page_mark(&mut toasts, mode);
    }
}
