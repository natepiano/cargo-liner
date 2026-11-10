//! Draw scheduled frames and recover terminal rows in bounded batches.

use std::io;
use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::sync::mpsc::RecvTimeoutError;
use std::thread;
use std::time::Duration;
use std::time::Instant;

use crossterm::cursor::RestorePosition;
use crossterm::cursor::SavePosition;
use crossterm::event;
use crossterm::event::Event;
use crossterm::event::KeyEventKind;
use crossterm::event::MouseButton;
use crossterm::event::MouseEvent;
use crossterm::event::MouseEventKind;
use ratatui::Frame;
use ratatui::Terminal;
use ratatui::backend::Backend;
use ratatui::backend::CrosstermBackend;
use ratatui::buffer::Buffer;
use ratatui::buffer::Cell;
use ratatui::layout::Position;
use ratatui::layout::Rect;

use super::app::FramePhase;
use super::app::FrameProbe;
use super::app::PollWork;
use super::app::Repaint;
use super::app::TerminalApp;
use super::constants::RECOVERY_BATCH_MILLIS;
use super::constants::REPAINT_SENTINEL;
use super::deadline::VisualDeadline;
use super::keys;
use crate::FRAME_POLL_MILLIS;
use crate::ScrollDirection;
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
    let mut recovery = Rect::default();
    let mut last_completed = Buffer::empty(Rect::default());
    let mut resize_recovery = Resized::No;
    let mut row_recovery = RowRecovery::new(Instant::now());
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
        if recovery.is_empty() && !app.holds_full_repaint() {
            recovery = row_recovery.batch(Instant::now(), terminal.current_buffer_mut().area);
        }
        let repaint = if dirty {
            Repaint::Needed
        } else {
            Repaint::NotNeeded
        };
        let repaired = prepare_frame::<_, A::Probe>(
            terminal,
            &mut last_completed,
            repaint,
            recovery,
            resize_recovery,
            |frame| {
                let keymap = app.keymap();
                app.draw(frame, &keymap);
            },
        )
        .inspect_err(|_| A::Probe::frame_started(previous.elapsed()))?;
        write_recovery::<A::Probe>(terminal, &repaired)
            .inspect_err(|_| A::Probe::frame_started(previous.elapsed()))?;
        recovery = Rect::default();
        resize_recovery = Resized::No;
        dirty = false;
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
                    resize_recovery = Resized::Yes;
                    recovery = terminal.current_buffer_mut().area;
                    force_repaint(terminal);
                }
                dirty = true;
            },
            Err(RecvTimeoutError::Timeout) => (),
            // The reader hit an unrecoverable crossterm error. A TUI that
            // cannot read input is dead, and without this the loop would
            // spin on a disconnected channel.
            Err(RecvTimeoutError::Disconnected) => break,
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
    }
    A::Probe::frame_started(previous.elapsed());
    Ok(())
}

/// Draw a requested app frame or recover rows from its last completed buffer.
fn prepare_frame<B: Backend, P: FrameProbe>(
    terminal: &mut Terminal<B>,
    last_completed: &mut Buffer,
    repaint: Repaint,
    recovery: Rect,
    resize_recovery: Resized,
    draw: impl FnOnce(&mut Frame<'_>),
) -> Result<RecoveryBatch, B::Error> {
    if repaint == Repaint::Needed {
        let completed = P::timed(FramePhase::Draw, || {
            terminal.draw(|frame| P::timed_callback(|| draw(frame)))
        })?;
        P::completed_frame(completed.buffer, recovery);
        P::timed(FramePhase::Draw, || {
            last_completed.clone_from(completed.buffer);
        });
    }
    if recovery.is_empty() || resize_recovery == Resized::Yes {
        return Ok(RecoveryBatch::default());
    }
    Ok(P::timed(FramePhase::Draw, || {
        RecoveryBatch::read(last_completed, recovery)
    }))
}

/// Write recovery rows without changing the app's cursor position.
fn write_recovery<P: FrameProbe>(
    terminal: &mut Terminal<ProbeBackend<P>>,
    repaired: &RecoveryBatch,
) -> io::Result<()> {
    if repaired.0.is_empty() {
        return Ok(());
    }
    P::timed(FramePhase::Draw, || {
        crossterm::queue!(terminal.backend_mut(), SavePosition)?;
        repaired.draw(terminal.backend_mut())?;
        crossterm::queue!(terminal.backend_mut(), RestorePosition)?;
        io::Write::flush(terminal.backend_mut())
    })
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
/// Left presses and wheel steps reach the app. The position is recorded from every
/// event regardless, which is what lets the framework answer where the
/// pointer was without an event of its own to ask.
fn handle_mouse<A: TerminalApp>(app: &mut A, mouse: MouseEvent) {
    crate::record_mouse_pos(mouse.column, mouse.row);
    let position = Position::new(mouse.column, mouse.row);
    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => app.click(position),
        MouseEventKind::ScrollUp => app.scroll(position, ScrollDirection::Up),
        MouseEventKind::ScrollDown => app.scroll(position, ScrollDirection::Down),
        _ => {},
    }
}

/// Recover half the terminal rows at each fixed deadline.
struct RowRecovery {
    deadline: Instant,
    next_row: u16,
}

impl RowRecovery {
    fn new(now: Instant) -> Self {
        Self {
            deadline: now + Duration::from_millis(RECOVERY_BATCH_MILLIS),
            next_row: 0,
        }
    }

    fn batch(&mut self, now: Instant, area: Rect) -> Rect {
        if now < self.deadline || area.is_empty() {
            return Rect::default();
        }
        let period = Duration::from_millis(RECOVERY_BATCH_MILLIS);
        while self.deadline <= now {
            self.deadline += period;
        }
        let start = self.next_row.min(area.height);
        let height = area
            .height
            .div_ceil(2)
            .min(area.height.saturating_sub(start));
        self.next_row = if start.saturating_add(height) >= area.height {
            0
        } else {
            start + height
        };
        Rect::new(area.x, area.y.saturating_add(start), area.width, height)
    }
}

/// Resize recovery writes the whole screen without first clearing it.
fn force_repaint<B: Backend>(terminal: &mut Terminal<B>) {
    let area = terminal.current_buffer_mut().area;
    invalidate_rows(terminal, area);
}

/// Prepare only the selected rows, using the terminal library's wide-cell rules.
#[derive(Default)]
struct RecoveryBatch(Vec<(u16, u16, Cell)>);

impl RecoveryBatch {
    fn read(buffer: &Buffer, rows: Rect) -> Self {
        let area = rows.intersection(buffer.area);
        let mut wanted = Buffer::empty(area);
        for y in area.top()..area.bottom() {
            let from = buffer.index_of(area.x, y);
            let to = wanted.index_of(area.x, y);
            let width = usize::from(area.width);
            wanted.content[to..to + width].clone_from_slice(&buffer.content[from..from + width]);
        }
        let mut comparison = Buffer::empty(area);
        for cell in &mut comparison.content {
            cell.modifier.insert(REPAINT_SENTINEL);
        }
        Self(
            comparison
                .diff(&wanted)
                .into_iter()
                .map(|(x, y, cell)| (x, y, cell.clone()))
                .collect(),
        )
    }

    fn draw<B: Backend>(&self, backend: &mut B) -> Result<(), B::Error> {
        backend.draw(self.0.iter().map(|(x, y, cell)| (*x, *y, cell)))
    }
}

/// Mark the full comparison buffer for resize recovery.
fn invalidate_rows<B: Backend>(terminal: &mut Terminal<B>, rows: Rect) {
    let buffer = terminal.current_buffer_mut();
    let area = rows.intersection(buffer.area);
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            buffer[(x, y)].modifier.insert(REPAINT_SENTINEL);
        }
    }
    terminal.swap_buffers();
}

#[cfg(test)]
mod tests {
    use ratatui::Frame;
    use ratatui::Terminal;
    use ratatui::backend::Backend;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
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
    #[test]
    fn periodic_recovery_repairs_idle_damage_within_two_seconds() {
        for height in [3_u16, 4, 80] {
            for repaint in [super::Repaint::Needed, super::Repaint::NotNeeded] {
                for first_batch_millis in [1000, 1001] {
                    check_idle_recovery(height, repaint, first_batch_millis);
                }
            }
        }
    }

    fn check_idle_recovery(height: u16, repaint: super::Repaint, first_batch_millis: u64) {
        let mut terminal = Terminal::new(TestBackend::new(6, height)).expect("test terminal");
        let draw_calls = std::cell::Cell::new(0);
        let draw = |frame: &mut Frame<'_>| {
            draw_calls.set(draw_calls.get() + 1);
            let area = frame.area();
            for y in area.top()..area.bottom() {
                frame.buffer_mut().set_string(
                    area.x,
                    y,
                    "界 tile",
                    ratatui::style::Style::default(),
                );
            }
        };
        let mut last_completed = ratatui::buffer::Buffer::empty(ratatui::layout::Rect::default());
        super::prepare_frame::<_, super::super::app::NoProbe>(
            &mut terminal,
            &mut last_completed,
            super::Repaint::Needed,
            ratatui::layout::Rect::default(),
            super::Resized::No,
            draw,
        )
        .expect("the initial screen draws");
        let expected = terminal.backend().buffer().clone();
        stray_everywhere(&mut terminal);
        let start = std::time::Instant::now();
        let mut recovery = super::RowRecovery::new(start);
        for (index, (millis, repaired_rows)) in [
            (0, 0),
            (999, 0),
            (first_batch_millis, height.div_ceil(2)),
            (1999, height.div_ceil(2)),
            (2000, height),
            (3000, height),
        ]
        .into_iter()
        .enumerate()
        {
            let rows = recovery.batch(
                start + std::time::Duration::from_millis(millis),
                expected.area,
            );
            let batch = super::prepare_frame::<_, super::super::app::NoProbe>(
                &mut terminal,
                &mut last_completed,
                repaint,
                rows,
                super::Resized::No,
                draw,
            )
            .expect("a scheduled idle frame draws");
            batch
                .draw(terminal.backend_mut())
                .expect("the recovery rows draw");
            check_repaired_rows(&terminal, &expected, &batch, millis, repaired_rows);
            assert_eq!(
                draw_calls.get(),
                if repaint == super::Repaint::Needed {
                    index + 2
                } else {
                    1
                },
                "recovery adds no app frame at {millis} ms"
            );
        }
    }

    fn check_repaired_rows(
        terminal: &Terminal<TestBackend>,
        expected: &Buffer,
        batch: &super::RecoveryBatch,
        millis: u64,
        repaired_rows: u16,
    ) {
        let height = expected.area.height;
        for y in 0..height {
            for x in 0..6 {
                if y < repaired_rows {
                    let cover = (0..x).rev().find(|&lead| {
                        unicode_width::UnicodeWidthStr::width(expected[(lead, y)].symbol())
                            > usize::from(x - lead)
                    });
                    if let Some(lead) = cover {
                        assert!(
                            unicode_width::UnicodeWidthStr::width(
                                terminal.backend().buffer()[(lead, y)].symbol()
                            ) > usize::from(x - lead),
                            "the wide glyph covers column {x} at {millis} ms"
                        );
                        assert!(
                            batch
                                .0
                                .iter()
                                .all(|(written_x, written_y, _)| *written_x != x
                                    || *written_y != y),
                            "the batch writes no other symbol inside the wide glyph"
                        );
                    } else {
                        assert_eq!(
                            terminal.backend().buffer()[(x, y)],
                            expected[(x, y)],
                            "row {y} at {millis} ms"
                        );
                    }
                } else {
                    assert_eq!(
                        terminal.backend().buffer()[(x, y)].symbol(),
                        "Z",
                        "row {y} waits for its scheduled batch"
                    );
                }
            }
        }
    }
}
