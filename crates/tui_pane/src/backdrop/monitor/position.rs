use std::time::Instant;

use crossbeam_channel::Receiver;
use crossbeam_channel::Sender;

use crate::backdrop::constants::POSITION_IDLE_INTERVAL;
use crate::backdrop::constants::POSITION_SETTLE_DURATION;
use crate::backdrop::desktop;
use crate::backdrop::desktop::Frame;

/// The last position outcome and whether movement still needs per-request reads.
#[derive(Clone, Copy, Debug, PartialEq)]
enum WindowPosition {
    /// The window server did not return a frame; retry at the idle interval.
    Unavailable,
    /// No movement has been observed since position reads settled.
    Settled(Frame),
    /// Movement was observed; keep reading until this frame remains unchanged long enough.
    Moving {
        /// The most recently read frame.
        frame:           Frame,
        /// When this frame first replaced the preceding one.
        unchanged_since: Instant,
    },
}

impl WindowPosition {
    /// Record an answered read, retaining the unchanged duration across equal frames.
    fn observe(self, frame: Frame, read_at: Instant) -> Self {
        match self {
            Self::Settled(previous)
            | Self::Moving {
                frame: previous, ..
            } if previous != frame => Self::Moving {
                frame,
                unchanged_since: read_at,
            },
            Self::Moving {
                unchanged_since, ..
            } if read_at.duration_since(unchanged_since) < POSITION_SETTLE_DURATION => {
                Self::Moving {
                    frame,
                    unchanged_since,
                }
            },
            Self::Unavailable | Self::Settled(_) | Self::Moving { .. } => Self::Settled(frame),
        }
    }
}

/// The watched window and completed read that govern the position worker's next lookup.
#[derive(Debug)]
enum PositionReadState {
    /// No window has been requested yet.
    Unwatched,
    /// The outcome held for this window, with its last completed read time.
    Holding {
        /// The window described by this outcome.
        window:   u32,
        /// The frame outcome and its movement state.
        position: WindowPosition,
        /// When the last window-server read completed, excluding time spent sending the answer.
        read_at:  Instant,
    },
}

impl PositionReadState {
    /// Answer from the held position unless movement, a new window, or the idle interval needs a
    /// read.
    fn read(
        &mut self,
        window: u32,
        clock: &mut impl FnMut() -> Instant,
        read_frame: &mut impl FnMut(u32) -> Option<Frame>,
    ) -> WindowPosition {
        let now = clock();
        let previous = match *self {
            Self::Holding {
                window: held,
                position,
                read_at,
            } if held == window => {
                if matches!(
                    position,
                    WindowPosition::Unavailable | WindowPosition::Settled(_)
                ) && now.duration_since(read_at) < POSITION_IDLE_INTERVAL
                {
                    return position;
                }
                position
            },
            Self::Unwatched | Self::Holding { .. } => WindowPosition::Unavailable,
        };
        let frame = read_frame(window);
        let read_at = clock();
        let position = frame.map_or(WindowPosition::Unavailable, |frame| {
            previous.observe(frame, read_at)
        });
        *self = Self::Holding {
            window,
            position,
            read_at,
        };
        position
    }
}

/// Answer each request, pacing settled and unanswered windows while following movement per frame.
/// The render thread still receives one answer per request, including held outcomes.
pub(super) fn position_loop(watches: &Receiver<u32>, frames: &Sender<Option<Frame>>) {
    position_loop_with(watches, frames, Instant::now, desktop::window_frame);
}

/// Run the same request loop with an injected clock and desktop read, exiting on either channel
/// error.
fn position_loop_with(
    watches: &Receiver<u32>,
    frames: &Sender<Option<Frame>>,
    mut clock: impl FnMut() -> Instant,
    mut read_frame: impl FnMut(u32) -> Option<Frame>,
) {
    let mut position_read_state = PositionReadState::Unwatched;
    while let Ok(window) = watches.recv() {
        let frame = match position_read_state.read(window, &mut clock, &mut read_frame) {
            WindowPosition::Unavailable => None,
            WindowPosition::Settled(frame) | WindowPosition::Moving { frame, .. } => Some(frame),
        };
        if frames.send(frame).is_err() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::error::Error;
    use std::time::Instant;

    use super::POSITION_IDLE_INTERVAL;
    use super::POSITION_SETTLE_DURATION;
    use super::PositionReadState;
    use super::WindowPosition;
    use super::position_loop_with;
    use crate::backdrop::desktop::Frame;

    const WATCHED_WINDOW: u32 = 7;
    const OTHER_WINDOW: u32 = 8;
    const REQUESTS_PER_IDLE_INTERVAL: u32 = 25;
    const IDLE_INTERVALS: u32 = 20;
    const STILL_FRAME: Frame = Frame {
        origin: (10.0, 20.0),
        size:   (800.0, 600.0),
    };
    const MOVED_FRAME: Frame = Frame {
        origin: (30.0, 40.0),
        ..STILL_FRAME
    };

    #[test]
    fn a_still_window_reads_only_once_per_idle_interval() {
        let mut position_read_state = PositionReadState::Unwatched;
        let start = Instant::now();
        let step = POSITION_IDLE_INTERVAL / REQUESTS_PER_IDLE_INTERVAL;
        let mut reads = Vec::new();
        for request in 0..REQUESTS_PER_IDLE_INTERVAL * IDLE_INTERVALS {
            let now = start + step * request;
            let position = position_read_state.read(WATCHED_WINDOW, &mut || now, &mut |window| {
                reads.push((window, now));
                Some(STILL_FRAME)
            });
            assert_eq!(position, WindowPosition::Settled(STILL_FRAME));
        }
        assert_eq!(u32::try_from(reads.len()), Ok(IDLE_INTERVALS));
        assert!(
            reads
                .windows(2)
                .all(|pair| pair[1].1.duration_since(pair[0].1) >= POSITION_IDLE_INTERVAL)
        );
    }

    #[test]
    fn movement_reads_every_request_until_the_last_changed_frame_settles() {
        let mut position_read_state = PositionReadState::Unwatched;
        let start = Instant::now();
        let now = Cell::new(start);
        let frame = Cell::new(STILL_FRAME);
        let reads = Cell::new(0);
        let mut read_frame = |_| {
            reads.set(reads.get() + 1);
            Some(frame.get())
        };
        let mut clock = || now.get();
        let step = POSITION_IDLE_INTERVAL / REQUESTS_PER_IDLE_INTERVAL;

        assert_eq!(
            position_read_state.read(WATCHED_WINDOW, &mut clock, &mut read_frame),
            WindowPosition::Settled(STILL_FRAME)
        );
        frame.set(MOVED_FRAME);
        now.set(start + step * (REQUESTS_PER_IDLE_INTERVAL - 1));
        assert_eq!(
            position_read_state.read(WATCHED_WINDOW, &mut clock, &mut read_frame),
            WindowPosition::Settled(STILL_FRAME)
        );
        assert_eq!(reads.get(), 1);
        now.set(start + POSITION_IDLE_INTERVAL);
        assert_eq!(
            position_read_state.read(WATCHED_WINDOW, &mut clock, &mut read_frame),
            WindowPosition::Moving {
                frame:           MOVED_FRAME,
                unchanged_since: now.get(),
            }
        );
        assert_eq!(reads.get(), 2);

        // `WindowPosition::observe` resets `unchanged_since` for each new frame, even after
        // movement has lasted longer than `POSITION_SETTLE_DURATION`.
        let movement_start = now.get();
        while now.get().duration_since(movement_start) <= POSITION_SETTLE_DURATION {
            now.set(now.get() + step);
            frame.set(Frame {
                origin: (frame.get().origin.0 + 1.0, frame.get().origin.1),
                ..frame.get()
            });
            let before = reads.get();
            assert_eq!(
                position_read_state.read(WATCHED_WINDOW, &mut clock, &mut read_frame),
                WindowPosition::Moving {
                    frame:           frame.get(),
                    unchanged_since: now.get(),
                }
            );
            assert_eq!(reads.get(), before + 1);
        }

        let unchanged_since = now.get();
        while now.get() + step < unchanged_since + POSITION_SETTLE_DURATION {
            now.set(now.get() + step);
            let before = reads.get();
            assert_eq!(
                position_read_state.read(WATCHED_WINDOW, &mut clock, &mut read_frame),
                WindowPosition::Moving {
                    frame: frame.get(),
                    unchanged_since
                }
            );
            assert_eq!(reads.get(), before + 1);
        }
        now.set(unchanged_since + POSITION_SETTLE_DURATION);
        let before_settle = reads.get();
        assert_eq!(
            position_read_state.read(WATCHED_WINDOW, &mut clock, &mut read_frame),
            WindowPosition::Settled(frame.get())
        );
        assert_eq!(reads.get(), before_settle + 1);
        now.set(now.get() + step * (REQUESTS_PER_IDLE_INTERVAL - 1));
        assert_eq!(
            position_read_state.read(WATCHED_WINDOW, &mut clock, &mut read_frame),
            WindowPosition::Settled(frame.get())
        );
        assert_eq!(reads.get(), before_settle + 1);
        now.set(now.get() + step);
        assert_eq!(
            position_read_state.read(WATCHED_WINDOW, &mut clock, &mut read_frame),
            WindowPosition::Settled(frame.get())
        );
        assert_eq!(reads.get(), before_settle + 2);
    }

    #[test]
    fn another_window_reads_immediately_and_starts_a_new_held_position() {
        let mut position_read_state = PositionReadState::Unwatched;
        let now = Instant::now();
        let mut reads = Vec::new();
        for window in [WATCHED_WINDOW, OTHER_WINDOW, WATCHED_WINDOW] {
            let position = position_read_state.read(window, &mut || now, &mut |window| {
                reads.push(window);
                Some(STILL_FRAME)
            });
            assert_eq!(position, WindowPosition::Settled(STILL_FRAME));
        }
        assert_eq!(reads, [WATCHED_WINDOW, OTHER_WINDOW, WATCHED_WINDOW]);
    }

    #[test]
    fn unanswered_reads_are_paced_even_after_movement_and_recover_on_the_interval() {
        let mut position_read_state = PositionReadState::Unwatched;
        let start = Instant::now();
        let step = POSITION_IDLE_INTERVAL / REQUESTS_PER_IDLE_INTERVAL;
        let mut reads = Vec::new();
        for request in 0..REQUESTS_PER_IDLE_INTERVAL * IDLE_INTERVALS {
            let now = start + step * request;
            assert_eq!(
                position_read_state.read(WATCHED_WINDOW, &mut || now, &mut |window| {
                    reads.push((window, now));
                    None
                }),
                WindowPosition::Unavailable
            );
        }
        assert_eq!(u32::try_from(reads.len()), Ok(IDLE_INTERVALS));
        assert!(
            reads
                .windows(2)
                .all(|pair| pair[1].1.duration_since(pair[0].1) >= POSITION_IDLE_INTERVAL)
        );
        let recovered = start + POSITION_IDLE_INTERVAL * IDLE_INTERVALS;
        assert_eq!(
            position_read_state.read(WATCHED_WINDOW, &mut || recovered, &mut |_| Some(
                STILL_FRAME
            )),
            WindowPosition::Settled(STILL_FRAME)
        );
        let changed = recovered + POSITION_IDLE_INTERVAL;
        assert_eq!(
            position_read_state.read(WATCHED_WINDOW, &mut || changed, &mut |_| Some(MOVED_FRAME)),
            WindowPosition::Moving {
                frame:           MOVED_FRAME,
                unchanged_since: changed,
            }
        );
        let unavailable = changed + step;
        assert_eq!(
            position_read_state.read(WATCHED_WINDOW, &mut || unavailable, &mut |_| None),
            WindowPosition::Unavailable
        );
        let attempts = Cell::new(0);
        let mut read_frame = |_| {
            attempts.set(attempts.get() + 1);
            Some(STILL_FRAME)
        };
        assert_eq!(
            position_read_state.read(
                WATCHED_WINDOW,
                &mut || unavailable + step * (REQUESTS_PER_IDLE_INTERVAL - 1),
                &mut read_frame
            ),
            WindowPosition::Unavailable
        );
        assert_eq!(attempts.get(), 0);
        assert_eq!(
            position_read_state.read(
                WATCHED_WINDOW,
                &mut || unavailable + POSITION_IDLE_INTERVAL,
                &mut read_frame
            ),
            WindowPosition::Settled(STILL_FRAME)
        );
        assert_eq!(attempts.get(), 1);
    }

    #[test]
    fn the_idle_interval_starts_when_the_desktop_read_finishes() {
        let mut position_read_state = PositionReadState::Unwatched;
        let now = Cell::new(Instant::now());
        let attempts = Cell::new(0);
        let mut read_frame = |_| {
            attempts.set(attempts.get() + 1);
            now.set(now.get() + POSITION_IDLE_INTERVAL * 2);
            Some(STILL_FRAME)
        };
        let mut clock = || now.get();
        assert_eq!(
            position_read_state.read(WATCHED_WINDOW, &mut clock, &mut read_frame),
            WindowPosition::Settled(STILL_FRAME)
        );
        let completed = now.get();
        now.set(completed + POSITION_IDLE_INTERVAL / 2);
        assert_eq!(
            position_read_state.read(WATCHED_WINDOW, &mut clock, &mut read_frame),
            WindowPosition::Settled(STILL_FRAME)
        );
        assert_eq!(attempts.get(), 1);
        now.set(completed + POSITION_IDLE_INTERVAL);
        assert_eq!(
            position_read_state.read(WATCHED_WINDOW, &mut clock, &mut read_frame),
            WindowPosition::Settled(STILL_FRAME)
        );
        assert_eq!(attempts.get(), 2);
    }

    #[test]
    fn the_position_worker_answers_every_request_and_exits_when_requests_disconnect()
    -> Result<(), Box<dyn Error>> {
        let (requests, watches) = crossbeam_channel::unbounded();
        let (frames, answers) = crossbeam_channel::unbounded();
        for _ in 0..REQUESTS_PER_IDLE_INTERVAL {
            requests.send(WATCHED_WINDOW)?;
        }
        drop(requests);
        let now = Instant::now();
        let mut reads = 0;
        position_loop_with(
            &watches,
            &frames,
            || now,
            |_| {
                reads += 1;
                Some(STILL_FRAME)
            },
        );
        drop(frames);
        assert_eq!(reads, 1);
        let answers: Vec<_> = answers.try_iter().collect();
        assert_eq!(answers.len(), usize::try_from(REQUESTS_PER_IDLE_INTERVAL)?);
        assert!(answers.iter().all(|frame| *frame == Some(STILL_FRAME)));
        Ok(())
    }

    #[test]
    fn the_position_worker_exits_when_answers_disconnect() -> Result<(), Box<dyn Error>> {
        let (requests, watches) = crossbeam_channel::unbounded();
        let (frames, answers) = crossbeam_channel::unbounded();
        drop(answers);
        requests.send(WATCHED_WINDOW)?;
        requests.send(OTHER_WINDOW)?;
        let now = Instant::now();
        let mut reads = 0;
        position_loop_with(
            &watches,
            &frames,
            || now,
            |_| {
                reads += 1;
                None
            },
        );
        assert_eq!(reads, 1);
        assert_eq!(watches.try_recv(), Ok(OTHER_WINDOW));
        Ok(())
    }
}
