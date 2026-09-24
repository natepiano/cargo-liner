//! The paced loop: draw when something changed, wait for the next
//! frame or event, fold the app's work in, and repaint the whole screen
//! on a cadence.

use std::io;
use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::sync::mpsc::RecvTimeoutError;
use std::thread;
use std::time::Duration;
use std::time::Instant;

use crossterm::event;
use crossterm::event::Event;
use crossterm::event::KeyEventKind;
use crossterm::event::MouseButton;
use crossterm::event::MouseEvent;
use crossterm::event::MouseEventKind;
use ratatui::Terminal;
use ratatui::backend::Backend;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Position;
use ratatui::layout::Rect;

use super::app::FrameProbe;
use super::app::PollWork;
use super::app::Repaint;
use super::app::TerminalApp;
use super::constants::FULL_REPAINT_SECONDS;
use super::constants::REPAINT_SENTINEL;
use super::deadline::VisualDeadline;
use super::keys;
use crate::FRAME_POLL_MILLIS;
use crate::ToastVisualDeadline;

/// The terminal backend, writing through the app's
/// [`FrameProbe::Output`].
pub(super) type ProbeBackend<P> = CrosstermBackend<<P as FrameProbe>::Output>;

/// Draw until [`GlobalAction::Quit`](crate::GlobalAction::Quit) or
/// [`GlobalAction::Restart`](crate::GlobalAction::Restart) sets the
/// matching lifecycle flag.
///
/// Drawing is demand-driven: a frame is painted only when an event
/// arrived or the app's work said something changed. With nothing
/// arriving and nobody typing there is nothing to repaint, so an idle
/// app costs essentially nothing.
pub(super) fn event_loop<A: TerminalApp, W: PollWork<A>>(
    terminal: &mut Terminal<ProbeBackend<A::Probe>>,
    app: &mut A,
    work: &mut W,
) -> io::Result<()> {
    let input = spawn_input_thread();
    let mut dirty = true;
    let mut repainted = Instant::now();
    let mut previous = Instant::now();
    let period = Duration::from_millis(FRAME_POLL_MILLIS);
    let mut deadline = Instant::now() + period;
    while !app.framework().quit_requested() && !app.framework().restart_requested() {
        let started = Instant::now();
        A::Probe::frame_started(started.duration_since(previous));
        previous = started;
        // Before the frame rather than inside it: whatever the app does
        // here can cost far longer than a frame, and `terminal.draw` is
        // no place to spend it.
        app.before_draw();
        if dirty {
            // Re-borrowed every frame: rebinding a key in the keymap
            // overlay swaps the whole map out from under the loop.
            let keymap = app.keymap();
            A::Probe::time_draw(|| terminal.draw(|frame| app.draw(frame, &keymap)))?;
            dirty = false;
        }
        // When the next frame is due, carried forward from when the
        // last one was due rather than worked out afresh from the top
        // of this one.
        //
        // The wait below is a condvar timeout, and the system grants it
        // late by a varying couple of milliseconds. Measured from the
        // top of the current frame, that lateness becomes the next
        // frame's starting point and is never given back: the loop asks
        // for its interval, is woken well past it, and settles there --
        // except on the frames where the wake happens to be prompt,
        // which arrive on time. A period alternating between the two is
        // what the eye reads as stop motion. Against a fixed deadline
        // the same lateness merely shortens the following wait.
        deadline += period;
        let now = Instant::now();
        // Far enough behind that catching up would mean a run of frames
        // with no wait at all between them -- a long draw, or a write
        // the emulator held on to. Start the schedule again from here.
        if deadline < now {
            deadline = now + period;
        }
        let toast_visual_deadline = app.framework().toasts.next_visual_change_deadline(now);
        let visual_deadline =
            VisualDeadline::from(toast_visual_deadline).earlier(app.visual_deadline(now, period));
        let remaining = visual_deadline.limit_wait(now, deadline.saturating_duration_since(now));
        match input.recv_timeout(remaining) {
            Ok(event) => {
                let mut resized = apply_event(app, &event);
                // Drain the rest of the burst before drawing again: an
                // iTerm2 resize drag delivers many events, and one repaint
                // at the settled size beats a repaint per intermediate width.
                while let Ok(event) = input.try_recv() {
                    if apply_event(app, &event) == Resized::Yes {
                        resized = Resized::Yes;
                    }
                }
                if resized == Resized::Yes {
                    app.resize_settled();
                    force_repaint(terminal);
                }
                dirty = true;
            },
            Err(RecvTimeoutError::Timeout) => (),
            // The reader hit an unrecoverable crossterm error. A TUI that
            // cannot read input is dead, and without this the loop would
            // spin on a disconnected channel.
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
        }
        let now = Instant::now();
        app.framework_mut().toasts.prune(now);
        if matches!(
            toast_visual_deadline,
            ToastVisualDeadline::At(deadline) if now >= deadline
        ) {
            dirty = true;
        }
        if work.poll(app, now) == Repaint::Needed {
            dirty = true;
        }
        // Whatever else has written to this terminal is written over
        // here, because a difference-based draw would leave it standing.
        //
        // Never while the app holds it off. Marking every cell for
        // redraw puts a write of the whole grid into one frame of an
        // animation, which the terminal shows as a tear rather than as
        // a repaint -- and there is nothing to write over anyway when
        // the animation already paints every cell it covers. The moment
        // the hold lifts, the wait is long overdue and one fires.
        if !app.holds_full_repaint()
            && repainted.elapsed() >= Duration::from_secs(FULL_REPAINT_SECONDS)
        {
            force_repaint(terminal);
            repainted = Instant::now();
            dirty = true;
        }
    }
    Ok(())
}

/// Read events on their own thread and forward them to the render loop.
///
/// [`event::read`] blocks whenever the bytes crossterm has buffered do not
/// yet form a whole event — a partial escape sequence parks it until the
/// rest arrives. On the render thread that stalls drawing *and* the
/// per-frame terminal size query, which is how a resize ends up invisible.
/// Here it stalls nothing but itself.
fn spawn_input_thread() -> Receiver<Event> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        while let Ok(event) = event::read() {
            if sender.send(event).is_err() {
                return;
            }
        }
    });
    receiver
}

/// Whether a drained event changed the terminal size.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Resized {
    /// The terminal changed size.
    Yes,
    /// It did not.
    No,
}

/// Apply one event from the input thread.
fn apply_event<A: TerminalApp>(app: &mut A, event: &Event) -> Resized {
    match *event {
        Event::Key(key) if key.kind == KeyEventKind::Press => {
            keys::dispatch_key(app, key);
            Resized::No
        },
        Event::Mouse(mouse) => {
            handle_mouse(app, mouse);
            Resized::No
        },
        Event::Resize(width, height) => {
            app.resized(Rect::new(0, 0, width, height));
            Resized::Yes
        },
        _ => Resized::No,
    }
}

/// Apply one mouse event.
///
/// Only a left press does anything. The position is recorded from every
/// event regardless, which is what lets the framework answer where the
/// pointer was without an event of its own to ask.
fn handle_mouse<A: TerminalApp>(app: &mut A, mouse: MouseEvent) {
    crate::record_mouse_pos(mouse.column, mouse.row);
    if mouse.kind != MouseEventKind::Down(MouseButton::Left) {
        return;
    }
    app.click(Position::new(mouse.column, mouse.row));
}

/// Reset the buffer the next frame is compared against, so that draw
/// writes every cell.
///
/// Neither [`Terminal::clear`] nor [`Terminal::resize`]: both blank the
/// screen before the repaint lands, and at this cadence that blank is a
/// visible blink every couple of seconds. Blanking is also the half of
/// the job that is not needed -- what forces the redraw is the *other*
/// thing those two do, resetting the buffer the next frame is compared
/// against.
///
/// That reset is reachable on its own. Marking every cell of the buffer
/// the next frame will be compared against with [`REPAINT_SENTINEL`]
/// makes all of them differ, so the next draw writes all of them, in
/// one pass, over what is already there. Same repaint, no blank in
/// front of it.
///
/// [`Terminal::swap_buffers`] is what moves the filled buffer into the
/// comparison slot; the frame renders into the other one.
fn force_repaint<B: Backend>(terminal: &mut Terminal<B>) {
    let buffer = terminal.current_buffer_mut();
    let area = buffer.area;
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            buffer[(x, y)].modifier.insert(REPAINT_SENTINEL);
        }
    }
    terminal.swap_buffers();
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::Backend;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Cell;

    use super::force_repaint;

    /// Every cell of the screen, as something other than the app wrote
    /// it, left where a difference-based draw would never look.
    fn stray_everywhere(terminal: &mut Terminal<TestBackend>) {
        let stray = Cell::new("Z");
        let area = terminal.backend().buffer().area;
        let cells: Vec<(u16, u16)> = (area.top()..area.bottom())
            .flat_map(|y| (area.left()..area.right()).map(move |x| (x, y)))
            .collect();
        terminal
            .backend_mut()
            .draw(cells.iter().map(|&(x, y)| (x, y, &stray)))
            .expect("stray output lands");
    }

    fn symbols(terminal: &Terminal<TestBackend>) -> Vec<String> {
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol().to_owned())
            .collect()
    }

    #[test]
    fn force_repaint_writes_every_cell_on_the_next_draw() {
        let mut terminal = Terminal::new(TestBackend::new(6, 3)).expect("test terminal");
        terminal.draw(|_frame| {}).expect("an empty frame draws");
        stray_everywhere(&mut terminal);
        terminal
            .draw(|_frame| {})
            .expect("a difference-based frame draws");
        assert!(
            symbols(&terminal).iter().all(|symbol| symbol == "Z"),
            "a difference-based draw leaves stray output standing"
        );

        force_repaint(&mut terminal);
        terminal
            .draw(|_frame| {})
            .expect("the repainted frame draws");
        assert!(
            symbols(&terminal).iter().all(|symbol| symbol == " "),
            "after force_repaint the next draw writes over every cell"
        );
    }
}
