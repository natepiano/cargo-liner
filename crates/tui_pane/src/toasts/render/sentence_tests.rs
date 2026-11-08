use std::error::Error;
use std::time::Duration;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;

use super::ToastsRenderCtx;
use super::test_support::TestApp;
use crate::ColorMode;
use crate::PaneFocusState;
use crate::Renderable;
use crate::Toasts;
use crate::toasts::ToastActivation;
use crate::toasts::ToastStyle;

#[test]
fn every_toast_style_keeps_a_blank_cell_before_the_close_button() -> Result<(), Box<dyn Error>> {
    for mode in [ColorMode::Colored, ColorMode::NoColor] {
        for style in [
            ToastStyle::Normal,
            ToastStyle::Success,
            ToastStyle::Warning,
            ToastStyle::Error,
        ] {
            let mut toasts = Toasts::<TestApp>::new();
            toasts.push_persistent(
                "crates.io unreachable with a long title",
                "Metadata is unavailable.",
                style,
                ToastActivation::Informational,
                1,
            );
            let now = toasts.entries[0].created_at + Duration::from_secs(1);
            let mut terminal = Terminal::new(TestBackend::new(80, 30))?;
            terminal.draw(|frame| {
                Renderable::render(
                    &mut toasts,
                    frame,
                    Rect::new(46, 0, 34, 30),
                    &ToastsRenderCtx {
                        now,
                        pane_focus_state: PaneFocusState::Inactive,
                    },
                );
                mode.clear_colors(frame.buffer_mut());
            })?;
            let hit = toasts.hits().first().ok_or("toast card")?;
            let buffer = terminal.backend().buffer();
            assert_eq!(
                buffer[(hit.close_rect.x - 1, hit.close_rect.y)].symbol(),
                " ",
                "{mode:?}: {style:?}"
            );
            let close: String = (hit.close_rect.x..hit.close_rect.x + 3)
                .map(|x| buffer[(x, hit.close_rect.y)].symbol())
                .collect();
            assert_eq!(close, "[x]");
            let title: String = (hit.card_rect.left() + 1..hit.close_rect.x - 1)
                .map(|x| buffer[(x, hit.close_rect.y)].symbol())
                .collect();
            assert!(title.contains("crates.io"), "{title}");
        }
    }
    Ok(())
}

#[test]
fn an_opening_error_toast_keeps_its_first_sentence_on_one_page() -> Result<(), Box<dyn Error>> {
    let sentence = "Crate metadata is unavailable until crates.io recovers.";
    for mode in [ColorMode::Colored, ColorMode::NoColor] {
        let mut toasts = Toasts::<TestApp>::new();
        toasts.push_persistent(
            "crates.io unreachable",
            sentence,
            ToastStyle::Error,
            ToastActivation::Informational,
            1,
        );
        let created = toasts.entries[0].created_at;
        for elapsed in [
            Duration::ZERO,
            Duration::from_millis(40),
            Duration::from_secs(1),
        ] {
            let pane = Rect::new(46, 20, 34, 9);
            let mut terminal = Terminal::new(TestBackend::new(80, 30))?;
            terminal.draw(|frame| {
                frame.render_widget(
                    crate::default_pane_chrome(mode).block("CI runs".to_owned(), false),
                    pane,
                );
                Renderable::render(
                    &mut toasts,
                    frame,
                    pane.inner(ratatui::layout::Margin::new(1, 1)),
                    &ToastsRenderCtx {
                        now:              created + elapsed,
                        pane_focus_state: PaneFocusState::Inactive,
                    },
                );
                mode.clear_colors(frame.buffer_mut());
            })?;
            let card = toasts.hits().first().ok_or("error card")?.card_rect;
            let buffer = terminal.backend().buffer();
            let body: Vec<_> = (card.top() + 1..card.bottom() - 1)
                .map(|y| {
                    (card.left() + 1..card.right() - 1)
                        .map(|x| buffer[(x, y)].symbol())
                        .collect::<String>()
                        .trim()
                        .to_owned()
                })
                .collect();
            assert_eq!(body.join(" "), sentence, "{mode:?}: {elapsed:?}");
            let bottom: String = (card.left()..card.right())
                .map(|x| buffer[(x, card.bottom() - 1)].symbol())
                .collect();
            assert!(!bottom.contains('▼'), "{bottom}");
            assert!(card.top() > pane.top());
            assert!(card.bottom() < pane.bottom());
            for x in card.left()..card.right() {
                assert_eq!(buffer[(x, card.bottom())].symbol(), " ");
            }
        }
    }
    Ok(())
}
