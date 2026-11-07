use std::error::Error;
use std::time::Duration;
use std::time::Instant;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Margin;
use ratatui::layout::Rect;
use ratatui::style::Color;

use super::ToastsRenderCtx;
use super::test_support::TestApp;
use crate::ColorMode;
use crate::PaneFocusState;
use crate::Renderable;
use crate::ToastActivation;
use crate::ToastId;
use crate::ToastStyle;
use crate::Toasts;
use crate::toasts::toast::ToastLifetime;
use crate::toasts::toast::ToastPhase;

const SENTENCE: &str = "Crate metadata is unavailable until crates.io recovers.";

fn draw(
    toasts: &mut Toasts<TestApp>,
    now: Instant,
    mode: ColorMode,
    area: Rect,
    panes: &[Rect],
) -> Result<Buffer, Box<dyn Error>> {
    let mut terminal = Terminal::new(TestBackend::new(80, 30))?;
    terminal.draw(|frame| {
        for pane in panes {
            frame.render_widget(
                crate::default_pane_chrome(mode).block("CI runs".to_owned(), false),
                *pane,
            );
        }
        Renderable::render(
            toasts,
            frame,
            area,
            &ToastsRenderCtx {
                now,
                pane_focus_state: PaneFocusState::Inactive,
            },
        );
        mode.clear_colors(frame.buffer_mut());
    })?;
    Ok(terminal.backend().buffer().clone())
}

fn card(toasts: &Toasts<TestApp>, id: ToastId) -> Result<Rect, Box<dyn Error>> {
    Ok(toasts
        .hits()
        .iter()
        .find(|hit| hit.id == id)
        .ok_or("toast card")?
        .card_rect)
}

fn row(buffer: &Buffer, card: Rect, y: u16) -> String {
    (card.left() + 1..card.right() - 1)
        .map(|x| buffer[(x, y)].symbol())
        .collect()
}

#[test]
fn timed_and_persistent_sentence_toasts_shrink_during_exit() -> Result<(), Box<dyn Error>> {
    for mode in [ColorMode::Colored, ColorMode::NoColor] {
        for timed in [true, false] {
            let mut toasts = Toasts::<TestApp>::new();
            let id = if timed {
                toasts.push_timed("Metadata", SENTENCE, Duration::from_secs(2), 1)
            } else {
                toasts.push_persistent(
                    "Metadata",
                    SENTENCE,
                    ToastStyle::Warning,
                    ToastActivation::Informational,
                    1,
                )
            };
            let area = Rect::new(47, 0, 32, 30);
            let settled = toasts.entries[0].created_at + Duration::from_secs(1);
            draw(&mut toasts, settled, mode, area, &[])?;
            let full_height = card(&toasts, id)?.height;
            assert!(full_height > 3);
            let exit_at = if timed {
                let ToastLifetime::Timed { timeout_at } = toasts.entries[0].lifetime else {
                    return Err("timed lifetime".into());
                };
                toasts.prune(timeout_at);
                timeout_at
            } else {
                assert!(toasts.dismiss(id));
                let ToastPhase::Exiting { started_at } = toasts.entries[0].phase else {
                    return Err("exit phase".into());
                };
                started_at
            };
            let step = toasts.settings().animation.exit_duration.get();
            for hidden in 0..full_height {
                let now = exit_at + step * u32::from(hidden);
                toasts.prune(now);
                let buffer = draw(&mut toasts, now, mode, area, &[])?;
                let card = card(&toasts, id)?;
                assert_eq!(
                    card.height,
                    full_height.saturating_sub(hidden).max(3),
                    "{mode:?}: timed={timed}, hidden={hidden}"
                );
                assert!(row(&buffer, card, card.top()).contains("Metadata"));
                assert!(
                    buffer[(card.left(), card.bottom() - 1)]
                        .symbol()
                        .contains('└')
                );
            }
            let removed_at = exit_at + step * u32::from(full_height);
            toasts.prune(removed_at);
            let buffer = draw(&mut toasts, removed_at, mode, area, &[])?;
            assert_eq!(toasts.hits(), []);
            assert!(buffer.content.iter().all(|cell| cell.symbol() == " "));
        }
    }
    Ok(())
}

#[test]
fn an_error_title_grows_before_it_uses_an_ellipsis() -> Result<(), Box<dyn Error>> {
    for mode in [ColorMode::Colored, ColorMode::NoColor] {
        for title in [
            "crates.io unreachable",
            "crates.io unreachable beyond this pane width",
        ] {
            let mut toasts = Toasts::<TestApp>::new();
            let id = toasts.push_persistent(
                title,
                SENTENCE,
                ToastStyle::Warning,
                ToastActivation::Informational,
                1,
            );
            let now = toasts.entries[0].created_at + Duration::from_secs(1);
            let area = Rect::new(47, 23, 32, 5);
            let buffer = draw(&mut toasts, now, mode, area, &[])?;
            let hit = toasts
                .hits()
                .iter()
                .find(|hit| hit.id == id)
                .ok_or("title hitbox")?;
            let title_row: String = (hit.card_rect.left()..hit.card_rect.right())
                .map(|x| buffer[(x, hit.card_rect.top())].symbol())
                .collect();
            if title == "crates.io unreachable" {
                assert!(
                    title_row.contains("! crates.io unreachable [x]"),
                    "{mode:?}: {title_row}"
                );
            } else {
                assert!(title_row.contains("… [x]"), "{mode:?}: {title_row}");
            }
            assert_eq!(
                buffer[(hit.close_rect.x - 1, hit.close_rect.y)].symbol(),
                " "
            );
            assert!(area.contains((hit.card_rect.left(), hit.card_rect.top()).into()));
            assert!(hit.card_rect.right() <= area.right());
        }
    }
    Ok(())
}

#[test]
fn a_live_corner_error_keeps_its_sentence_with_startup_present() -> Result<(), Box<dyn Error>> {
    let panes = [
        Rect::new(0, 0, 46, 22),
        Rect::new(46, 0, 17, 10),
        Rect::new(63, 0, 17, 10),
        Rect::new(46, 10, 34, 12),
        Rect::new(0, 22, 46, 7),
        Rect::new(46, 22, 34, 7),
    ];
    for mode in [ColorMode::Colored, ColorMode::NoColor] {
        let borders = draw(
            &mut Toasts::<TestApp>::new(),
            Instant::now(),
            mode,
            panes[5].inner(Margin::new(1, 1)),
            &panes,
        )?;
        let mut toasts = Toasts::<TestApp>::new();
        let startup: Vec<_> = [
            "Cargo metadata",
            "crates.io",
            "Disk usage",
            "GitHub repos",
            "Languages",
            "Lint history",
            "Local git repos",
            "Test counts",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        toasts.push_colored_persistent(
            "Startup",
            startup.clone(),
            vec![Color::Green; startup.len()],
        );
        let id = toasts.push_persistent(
            "crates.io unreachable",
            SENTENCE,
            ToastStyle::Warning,
            ToastActivation::Informational,
            1,
        );
        let created = toasts.entries[1].created_at;
        for elapsed in [
            Duration::ZERO,
            Duration::from_millis(40),
            Duration::from_secs(1),
        ] {
            let buffer = draw(
                &mut toasts,
                created + elapsed,
                mode,
                panes[5].inner(Margin::new(1, 1)),
                &panes,
            )?;
            let card = card(&toasts, id)?;
            let body: Vec<_> = (card.top() + 1..card.bottom() - 1)
                .map(|y| row(&buffer, card, y).trim().to_owned())
                .collect();
            assert_eq!(body.join(" "), SENTENCE, "{mode:?}: {elapsed:?}");
            assert!(!row(&buffer, card, card.bottom() - 1).contains('▼'));
            assert!(card.top() > panes[5].top());
            assert!(card.bottom() < panes[5].bottom());
            for x in panes[5].left()..panes[5].right() {
                for y in [panes[5].top(), panes[5].bottom() - 1] {
                    assert_eq!(buffer[(x, y)].symbol(), borders[(x, y)].symbol());
                }
            }
            for y in panes[5].top()..panes[5].bottom() {
                for x in [panes[5].left(), panes[5].right() - 1] {
                    assert_eq!(buffer[(x, y)].symbol(), borders[(x, y)].symbol());
                }
            }
        }
    }
    Ok(())
}
