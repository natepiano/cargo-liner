//! Public toast focus, activation, and tracked-item timing behavior.

use std::error::Error;
use std::time::Duration;
use std::time::Instant;

use crossterm::event::KeyCode;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::Modifier;
use tui_pane::AppContext;
use tui_pane::FocusedPane;
use tui_pane::Framework;
use tui_pane::FrameworkFocusId;
use tui_pane::KeyBind;
use tui_pane::KeyOutcome;
use tui_pane::PaneFocusState;
use tui_pane::Renderable;
use tui_pane::ToastActivation;
use tui_pane::ToastCommand;
use tui_pane::ToastCountdownState;
use tui_pane::ToastDuration;
use tui_pane::ToastFocus;
use tui_pane::ToastLingerState;
use tui_pane::ToastStyle;
use tui_pane::Toasts;
use tui_pane::ToastsRenderCtx;
use tui_pane::TrackedItem;
use tui_pane::TrackedItemElapsed;
use tui_pane::TrackedItemLifecycle;
use tui_pane::TrackedItemTiming;

struct ToastApp {
    framework: Framework<Self>,
}

impl AppContext for ToastApp {
    type AppPaneId = ();
    type ToastAction = u32;

    fn framework(&self) -> &Framework<Self> { &self.framework }

    fn framework_mut(&mut self) -> &mut Framework<Self> { &mut self.framework }
}

fn draw(
    toasts: &mut Toasts<ToastApp>,
    now: Instant,
    pane_focus_state: PaneFocusState,
) -> Result<Buffer, Box<dyn Error>> {
    let mut terminal = Terminal::new(TestBackend::new(60, 20))?;
    terminal.draw(|frame| {
        let _ = toasts.render(
            frame,
            frame.area(),
            &ToastsRenderCtx {
                now,
                pane_focus_state,
            },
        );
    })?;
    Ok(terminal.backend().buffer().clone())
}

fn row_text(buffer: &Buffer, row: u16) -> String {
    (buffer.area.left()..buffer.area.right())
        .map(|column| buffer[(column, row)].symbol())
        .collect()
}

#[test]
fn an_unfocused_toast_pane_keeps_every_card_unhighlighted() -> Result<(), Box<dyn Error>> {
    let mut toasts = Toasts::<ToastApp>::new();
    let _ = toasts.push("first", "body");
    let _ = toasts.push("second", "body");
    toasts.reset_to_last();
    assert_eq!(
        toasts.focused_id(PaneFocusState::Inactive),
        ToastFocus::PaneUnfocused
    );
    let now = Instant::now() + Duration::from_secs(1);

    for pane_focus in [PaneFocusState::Inactive, PaneFocusState::Remembered] {
        assert_eq!(toasts.focused_id(pane_focus), ToastFocus::PaneUnfocused);
        assert_eq!(
            toasts.focused_toast_id(pane_focus),
            ToastFocus::PaneUnfocused
        );
        let buffer = draw(&mut toasts, now, pane_focus)?;
        assert_eq!(toasts.hits().len(), 2);
        for toast_hitbox in toasts.hits() {
            assert!(
                !buffer[(toast_hitbox.card_rect.x + 2, toast_hitbox.card_rect.y)]
                    .modifier
                    .contains(Modifier::BOLD)
            );
        }
    }
    Ok(())
}

#[test]
fn a_focused_toast_pane_without_a_toast_has_no_selection() -> Result<(), Box<dyn Error>> {
    let mut toasts = Toasts::<ToastApp>::new();
    assert_eq!(
        toasts.focused_id(PaneFocusState::Active),
        ToastFocus::PaneFocusedWithoutToast
    );
    assert_eq!(
        toasts.focused_toast_id(PaneFocusState::Active),
        ToastFocus::PaneFocusedWithoutToast
    );
    assert_eq!(
        toasts.focused_id(PaneFocusState::Active),
        ToastFocus::PaneFocusedWithoutToast
    );
    for pane_focus in [PaneFocusState::Inactive, PaneFocusState::Remembered] {
        assert_eq!(toasts.focused_id(pane_focus), ToastFocus::PaneUnfocused);
        assert_eq!(
            toasts.focused_toast_id(pane_focus),
            ToastFocus::PaneUnfocused
        );
    }
    let buffer = draw(&mut toasts, Instant::now(), PaneFocusState::Active)?;
    assert_eq!(toasts.hits().len(), 0);
    assert!(buffer.content.iter().all(|cell| cell.symbol() == " "));
    assert_eq!(
        toasts.handle_key_command(&KeyBind::from(KeyCode::Enter), PaneFocusState::Active),
        (KeyOutcome::Unhandled, ToastCommand::None)
    );
    Ok(())
}

#[test]
fn a_focused_toast_pane_highlights_only_its_selected_toast() -> Result<(), Box<dyn Error>> {
    let mut toasts = Toasts::<ToastApp>::new();
    let _ = toasts.push("first", "body");
    let second_toast_id = toasts.push("second", "body");
    toasts.reset_to_last();
    assert_eq!(
        toasts.focused_id(PaneFocusState::Active),
        ToastFocus::PaneFocusedOn(second_toast_id)
    );
    assert_eq!(
        toasts.focused_toast_id(PaneFocusState::Active),
        ToastFocus::PaneFocusedOn(second_toast_id)
    );
    assert_eq!(
        toasts.focused_id(PaneFocusState::Active),
        ToastFocus::PaneFocusedOn(second_toast_id)
    );
    let now = Instant::now() + Duration::from_secs(1);
    let buffer = draw(&mut toasts, now, PaneFocusState::Active)?;
    assert_eq!(toasts.hits().len(), 2);
    for toast_hitbox in toasts.hits() {
        assert_eq!(
            buffer[(toast_hitbox.card_rect.x + 2, toast_hitbox.card_rect.y)]
                .modifier
                .contains(Modifier::BOLD),
            toast_hitbox.id == second_toast_id
        );
    }
    Ok(())
}

#[test]
fn leaving_the_toast_pane_suspends_selection_commands_until_focus_returns() {
    let mut toast_app = ToastApp {
        framework: Framework::new(FocusedPane::Framework(FrameworkFocusId::Toasts)),
    };
    let first_toast_id = toast_app.framework.toasts.push("first", "body");
    let selected_toast_id = toast_app.framework.toasts.push_persistent(
        "selected",
        "body",
        ToastStyle::Normal,
        ToastActivation::Opens(7),
        1,
    );
    toast_app.framework.toasts.reset_to_last();
    let enter_key_bind = KeyBind::from(KeyCode::Enter);
    let pane = toast_app.framework().toast_pane_focus_state();
    assert_eq!(pane, PaneFocusState::Active);
    assert_eq!(
        toast_app.framework.toasts.focused_id(pane),
        ToastFocus::PaneFocusedOn(selected_toast_id)
    );
    assert_eq!(
        toast_app
            .framework
            .toasts
            .handle_key_command(&enter_key_bind, pane),
        (KeyOutcome::Consumed, ToastCommand::Activate(7))
    );

    toast_app.set_focus(FocusedPane::App(()));
    let pane = toast_app.framework().toast_pane_focus_state();
    assert_eq!(pane, PaneFocusState::Inactive);
    assert_eq!(toast_app.framework().focused(), &FocusedPane::App(()));
    assert_eq!(
        toast_app.framework.toasts.focused_id(pane),
        ToastFocus::PaneUnfocused
    );
    assert_eq!(
        toast_app.framework.toasts.focused_toast_id(pane),
        ToastFocus::PaneUnfocused
    );
    assert!(!toast_app.framework.toasts.dismiss_focused(pane));
    assert_eq!(
        toast_app
            .framework
            .toasts
            .handle_key_command(&enter_key_bind, pane),
        (KeyOutcome::Unhandled, ToastCommand::None)
    );
    assert_eq!(toast_app.framework.toasts.active().len(), 2);

    toast_app.set_focus(FocusedPane::Framework(FrameworkFocusId::Toasts));
    let pane = toast_app.framework().toast_pane_focus_state();
    assert_eq!(pane, PaneFocusState::Active);
    assert_eq!(
        toast_app.framework.toasts.focused_id(pane),
        ToastFocus::PaneFocusedOn(selected_toast_id)
    );
    assert_eq!(
        toast_app.framework.toasts.focused_toast_id(pane),
        ToastFocus::PaneFocusedOn(selected_toast_id)
    );
    assert_eq!(
        toast_app
            .framework
            .toasts
            .handle_key_command(&enter_key_bind, pane),
        (KeyOutcome::Consumed, ToastCommand::Activate(7))
    );
    assert!(toast_app.framework.toasts.dismiss_focused(pane));
    toast_app
        .framework
        .toasts
        .prune(Instant::now() + Duration::from_secs(3));
    assert_eq!(toast_app.framework.toasts.active().len(), 1);
    assert_eq!(toast_app.framework.toasts.active()[0].id(), first_toast_id);
}

#[test]
fn an_unknown_start_stays_unknown_after_completion() -> Result<(), Box<dyn Error>> {
    let mut tracked_item = TrackedItem::new("unknown start", "unknown");
    tracked_item.timing = TrackedItemTiming::StartUnknown;
    let mut toasts = Toasts::<ToastApp>::new();
    let toast_task_id = toasts.start_task("task", "");
    assert!(toasts.set_tracked_items(toast_task_id, std::slice::from_ref(&tracked_item)));
    let now = Instant::now() + Duration::from_secs(1);
    let toast_views = toasts.active_views(now);
    assert_eq!(toast_views.len(), 1);
    assert_eq!(toast_views[0].tracked_items().len(), 1);
    assert_eq!(
        toast_views[0].tracked_items()[0].elapsed,
        TrackedItemElapsed::StartUnknown
    );
    assert_eq!(
        toast_views[0].tracked_items()[0].lifecycle,
        TrackedItemLifecycle::Running
    );
    let buffer = draw(&mut toasts, now, PaneFocusState::Inactive)?;
    assert_eq!(toasts.hits().len(), 1);
    let card = toasts.hits()[0].card_rect;
    assert_eq!(
        row_text(&buffer, card.y + 1).trim_matches([' ', '│']),
        "unknown start"
    );

    tracked_item.mark_completed(now);
    assert_eq!(
        tracked_item.timing,
        TrackedItemTiming::CompletedWithoutStart { completed_at: now }
    );
    assert!(toasts.set_tracked_items(toast_task_id, &[tracked_item]));
    let toast_views = toasts.active_views(now);
    assert_eq!(toast_views.len(), 1);
    assert_eq!(
        toast_views[0].tracked_items()[0].elapsed,
        TrackedItemElapsed::StartUnknown
    );
    assert_eq!(
        toast_views[0].tracked_items()[0].lifecycle,
        TrackedItemLifecycle::CompletedFading { progress: 0.0 }
    );
    let buffer = draw(&mut toasts, now, PaneFocusState::Inactive)?;
    assert_eq!(toasts.hits().len(), 1);
    let card = toasts.hits()[0].card_rect;
    assert_eq!(
        row_text(&buffer, card.y + 1).trim_matches([' ', '│']),
        "unknown start"
    );
    Ok(())
}

#[test]
fn a_completed_item_without_linger_keeps_its_elapsed_row() -> Result<(), Box<dyn Error>> {
    let started_at = Instant::now() + Duration::from_secs(1);
    let now = started_at + Duration::from_secs(2);
    let mut completed_tracked_item = TrackedItem::new("item", "completed");
    completed_tracked_item.timing = TrackedItemTiming::Completed {
        started_at,
        completed_at: now,
    };
    let mut running_tracked_item = TrackedItem::new("item", "running");
    running_tracked_item.timing = TrackedItemTiming::Running { started_at };
    let mut toasts = Toasts::<ToastApp>::new();
    toasts.settings_mut().finished_task_visible = ToastDuration::try_from_secs("test", 0.0)?;
    let toast_task_id = toasts.start_task("task", "");
    assert!(toasts.set_tracked_items(
        toast_task_id,
        &[completed_tracked_item, running_tracked_item]
    ));
    let toast_views = toasts.active_views(now);
    assert_eq!(toast_views.len(), 1);
    assert_eq!(toast_views[0].tracked_items().len(), 2);
    assert_eq!(
        toast_views[0].tracked_items()[0].elapsed,
        TrackedItemElapsed::Measured(Duration::from_secs(2))
    );
    assert_eq!(
        toast_views[0].tracked_items()[0].lifecycle,
        TrackedItemLifecycle::CompletedWithoutFade
    );
    assert_eq!(
        toast_views[0].tracked_items()[1].lifecycle,
        TrackedItemLifecycle::Running
    );
    let buffer = draw(&mut toasts, now, PaneFocusState::Inactive)?;
    assert_eq!(toasts.hits().len(), 1);
    let card = toasts.hits()[0].card_rect;
    let completed_row = row_text(&buffer, card.y + 1);
    assert_eq!(completed_row, row_text(&buffer, card.y + 2));
    assert_eq!(
        completed_row.trim_matches([' ', '│']),
        "item                                        ⣆⠀ 2000ms"
    );
    for column in card.left()..card.right() {
        assert_eq!(buffer[(column, card.y + 1)], buffer[(column, card.y + 2)]);
    }
    Ok(())
}

#[test]
fn informational_and_opening_toasts_have_distinct_commands_and_row_costs()
-> Result<(), Box<dyn Error>> {
    let mut toasts = Toasts::<ToastApp>::new();
    let informational_toast_id = toasts.push_persistent(
        "information",
        "body",
        ToastStyle::Normal,
        ToastActivation::Informational,
        1,
    );
    let opening_toast_id = toasts.push_persistent_styled(
        "opening",
        "body",
        ToastStyle::Normal,
        ToastActivation::Opens(7),
        1,
    );
    assert_eq!(toasts.active()[0].action(), ToastActivation::Informational);
    assert_eq!(toasts.active()[1].action(), ToastActivation::Opens(&7));
    let now = Instant::now() + Duration::from_secs(1);
    let toast_views = toasts.active_views(now);
    assert_eq!(toast_views.len(), 2);
    assert!(!toast_views[0].has_action());
    assert!(toast_views[1].has_action());
    assert_eq!(
        toast_views[1].desired_height(),
        toast_views[0].desired_height() + 1
    );
    toasts.reset_to_first();
    assert_eq!(
        toasts.focused_id(PaneFocusState::Active),
        ToastFocus::PaneFocusedOn(informational_toast_id)
    );
    let enter_key_bind = KeyBind::from(KeyCode::Enter);
    assert_eq!(
        toasts.handle_key_command(&enter_key_bind, PaneFocusState::Active),
        (KeyOutcome::Unhandled, ToastCommand::None)
    );
    toasts.reset_to_last();
    assert_eq!(
        toasts.focused_id(PaneFocusState::Active),
        ToastFocus::PaneFocusedOn(opening_toast_id)
    );
    assert_eq!(
        toasts.handle_key_command(&enter_key_bind, PaneFocusState::Active),
        (KeyOutcome::Consumed, ToastCommand::Activate(7))
    );
    let buffer = draw(&mut toasts, now, PaneFocusState::Active)?;
    assert_eq!(toasts.hits().len(), 2);
    let toast_hitboxes = toasts.hits();
    let informational_toast_hitbox = toast_hitboxes
        .iter()
        .find(|toast_hitbox| toast_hitbox.id == informational_toast_id)
        .ok_or("informational card should render")?;
    let opening_toast_hitbox = toast_hitboxes
        .iter()
        .find(|toast_hitbox| toast_hitbox.id == opening_toast_id)
        .ok_or("opening card should render")?;
    assert_eq!(
        opening_toast_hitbox.card_rect.height,
        informational_toast_hitbox.card_rect.height + 1
    );
    assert!(row_text(&buffer, opening_toast_hitbox.card_rect.bottom() - 2).contains("Enter"));
    assert!(
        !row_text(&buffer, informational_toast_hitbox.card_rect.bottom() - 2).contains("Enter")
    );
    Ok(())
}

#[test]
fn persistent_and_timed_toasts_report_their_countdown_policy() {
    let mut toasts = Toasts::<ToastApp>::new();
    let _ = toasts.push("persistent", "body");
    let _ = toasts.push_timed("timed", "body", Duration::from_secs(5), 1);
    let toast_views = toasts.active_views(Instant::now());
    assert_eq!(toast_views.len(), 2);
    assert_eq!(toast_views[0].linger(), ToastLingerState::Live);
    assert_eq!(toast_views[0].countdown(), ToastCountdownState::NoCountdown);
    assert!(matches!(
        toast_views[1].countdown(),
        ToastCountdownState::CountingDown { remaining_secs: 5 }
    ));
}

#[test]
fn a_known_start_measures_running_time_and_freezes_at_completion() {
    let started_at = Instant::now();
    let completed_at = started_at + Duration::from_secs(2);
    let mut tracked_item = TrackedItem::new("known start", "known");
    tracked_item.timing = TrackedItemTiming::Running { started_at };
    let mut toasts = Toasts::<ToastApp>::new();
    let toast_task_id = toasts.start_task("task", "");
    assert!(toasts.set_tracked_items(toast_task_id, std::slice::from_ref(&tracked_item)));
    let toast_views = toasts.active_views(started_at + Duration::from_secs(1));
    assert_eq!(toast_views.len(), 1);
    assert_eq!(toast_views[0].tracked_items().len(), 1);
    assert_eq!(
        toast_views[0].tracked_items()[0].elapsed,
        TrackedItemElapsed::Measured(Duration::from_secs(1))
    );
    assert_eq!(toast_views[0].linger(), ToastLingerState::Live);
    assert_eq!(toast_views[0].countdown(), ToastCountdownState::NoCountdown);

    tracked_item.mark_completed(completed_at);
    assert_eq!(
        tracked_item.timing(),
        TrackedItemTiming::Completed {
            started_at,
            completed_at
        }
    );
    assert!(toasts.set_tracked_items(toast_task_id, &[tracked_item]));
    let now = completed_at + Duration::from_millis(100);
    let toast_views = toasts.active_views(now);
    assert_eq!(toast_views.len(), 1);
    assert_eq!(
        toast_views[0].tracked_items()[0].elapsed,
        TrackedItemElapsed::Measured(Duration::from_secs(2))
    );
    assert!(
        matches!(toast_views[0].linger(), ToastLingerState::Lingering { progress } if progress > 0.0 && progress < 1.0)
    );
    assert!(
        matches!(toast_views[0].countdown(), ToastCountdownState::CountingDown { remaining_secs } if remaining_secs > 0)
    );

    assert!(toasts.restart_tracked_item(toast_task_id, &"known".into(), now));
    let toast_views = toasts.active_views(now + Duration::from_secs(1));
    assert_eq!(toast_views.len(), 1);
    assert_eq!(
        toast_views[0].tracked_items()[0].elapsed,
        TrackedItemElapsed::Measured(Duration::from_secs(1))
    );
    assert_eq!(
        toast_views[0].tracked_items()[0].lifecycle,
        TrackedItemLifecycle::Running
    );
    assert_eq!(toast_views[0].linger(), ToastLingerState::Live);
    assert_eq!(toast_views[0].countdown(), ToastCountdownState::NoCountdown);
}

#[test]
fn explicit_completion_uses_the_latest_timestamp_and_preserves_the_start() {
    let started_at = Instant::now();
    let first_completion = started_at + Duration::from_secs(1);
    let latest_completion = started_at + Duration::from_secs(2);
    for (initial, completed) in [
        (
            TrackedItemTiming::StartUnknown,
            TrackedItemTiming::CompletedWithoutStart {
                completed_at: latest_completion,
            },
        ),
        (
            TrackedItemTiming::Running { started_at },
            TrackedItemTiming::Completed {
                started_at,
                completed_at: latest_completion,
            },
        ),
    ] {
        let mut tracked_item = TrackedItem::new("item", "item");
        tracked_item.timing = initial;
        tracked_item.mark_completed(first_completion);
        tracked_item.mark_completed(latest_completion);
        assert_eq!(tracked_item.timing(), completed);
    }
}
