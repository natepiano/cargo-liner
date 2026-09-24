//! A frame-by-frame record of where the render loop's time goes, for
//! finding a stall rather than for shipping.
//!
//! Switched on by setting `CARGO_TILE_FRAME_LOG` to a path to append
//! to; unset, every call here is an atomic load and a branch.
//!
//! What it is for is telling two very different stalls apart. A frame
//! whose phases add up to the gap it left is one the render loop spent
//! doing something slow, and the phase names which. A frame whose gap
//! is long while every phase is short is one the render loop was not
//! running for -- descheduled, or held up in the allocator behind
//! another thread -- and no amount of looking at what it draws will
//! find that.

use std::fmt::Write as _;
use std::fs::OpenOptions;
use std::io;
use std::io::Stdout;
use std::io::Write;
use std::sync::OnceLock;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;

use tui_pane::FramePhase;
use tui_pane::FrameProbe;

use crate::constants::PROBE_THRESHOLD;

/// Where `phase`'s nanoseconds are kept.
fn slot(phase: FramePhase) -> &'static AtomicU64 {
    static SLOTS: [AtomicU64; 5] = [
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
    ];
    let index = match phase {
        FramePhase::Advance => 0,
        FramePhase::Refresh => 1,
        FramePhase::Panes => 2,
        FramePhase::Band => 3,
        FramePhase::Draw => 4,
    };
    &SLOTS[index]
}

/// What `phase` is called in the log.
const fn name(phase: FramePhase) -> &'static str {
    match phase {
        FramePhase::Advance => "advance",
        FramePhase::Refresh => "refresh",
        FramePhase::Panes => "panes",
        FramePhase::Band => "band",
        FramePhase::Draw => "draw",
    }
}

/// Every phase, for walking them in a fixed order.
const PHASES: [FramePhase; 5] = [
    FramePhase::Draw,
    FramePhase::Advance,
    FramePhase::Refresh,
    FramePhase::Panes,
    FramePhase::Band,
];

/// The path to append to, or [`None`] where the probe is off.
fn target() -> Option<&'static str> {
    static TARGET: OnceLock<Option<String>> = OnceLock::new();
    TARGET
        .get_or_init(|| std::env::var("CARGO_TILE_FRAME_LOG").ok())
        .as_deref()
}

/// Whether anything here does any work at all.
fn on() -> bool { target().is_some() }

/// Time `body` and record it as `phase`.
fn timed<T>(phase: FramePhase, body: impl FnOnce() -> T) -> T {
    if !on() {
        return body();
    }
    let at = Instant::now();
    let answer = body();
    slot(phase).store(
        u64::try_from(at.elapsed().as_nanos()).unwrap_or(u64::MAX),
        Ordering::Relaxed,
    );
    answer
}

/// A writer that counts what passes through it on its way to the
/// terminal.
///
/// What the render loop costs and what the emulator is asked to do are
/// two different numbers, and only the first is visible from this
/// process's own timings. A loop that finishes a frame in half a
/// millisecond can still be handing the emulator more escape sequences
/// per second than it can parse and draw, and what is then on the
/// display is behind the app that fed it -- which no phase timed in
/// here can show.
#[derive(Debug)]
pub(crate) struct Counted<W> {
    /// Where the bytes actually go.
    inner: W,
}

impl<W> Counted<W> {
    /// Wrap `inner` so everything written through it is counted.
    pub(crate) const fn new(inner: W) -> Self { Self { inner } }
}

impl<W: Write> Write for Counted<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let written = self.inner.write(buf)?;
        if on() {
            let count = u64::try_from(written).unwrap_or(0);
            WRITTEN.fetch_add(count, Ordering::Relaxed);
            WRITTEN_FRAME.fetch_add(count, Ordering::Relaxed);
        }
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> { self.inner.flush() }
}

/// The frame log as the runner's [`FrameProbe`]: every pass of the
/// loop is a [`frame`], each [`FramePhase`] is [`timed`] into its own
/// column, and the terminal's output is [`Counted`] once the setup has
/// been written.
pub(crate) enum FrameLog {}

impl FrameProbe for FrameLog {
    type Output = Counted<Stdout>;

    fn output(stdout: Stdout) -> Self::Output { Counted::new(stdout) }

    fn frame_started(gap: Duration) { frame(gap, PROBE_THRESHOLD); }

    fn timed<T>(phase: FramePhase, body: impl FnOnce() -> T) -> T { timed(phase, body) }

    fn note(line: &str) { note(line); }

    fn trace() { trace(); }
}

/// How many frames go into one summary line.
const SUMMARY_FRAMES: u32 = 120;

/// What has been seen since the last summary line.
static COUNT: AtomicU64 = AtomicU64::new(0);
/// Nanoseconds the frames since the last summary line covered.
static ELAPSED: AtomicU64 = AtomicU64::new(0);
/// Bytes handed to the terminal since the last summary line.
static WRITTEN: AtomicU64 = AtomicU64::new(0);
/// Bytes handed to the terminal since the last frame line.
static WRITTEN_FRAME: AtomicU64 = AtomicU64::new(0);
/// The longest gap since the last summary line, in nanoseconds.
static WORST: AtomicU64 = AtomicU64::new(0);
/// Gaps over the threshold since the last summary line.
static SLOW: AtomicU64 = AtomicU64::new(0);

/// How many frames are written out in full once tracing has started.
///
/// Four seconds at the poll interval, which is long enough to show a
/// cadence rather than a moment of one.
const TRACE_FRAMES: u64 = 500;

/// Whether every frame is being written out in full.
static TRACING: AtomicBool = AtomicBool::new(false);
/// How many frames have been written out since tracing started.
static TRACED: AtomicU64 = AtomicU64::new(0);

/// Start writing every frame out in full for the next
/// [`TRACE_FRAMES`] frames, rather than only the slow ones.
///
/// A summary hides the two things a stuttering animation is most
/// likely to be made of: frames that arrive early, which no worst-case
/// gap can show, and frames that drew nothing at all.
fn trace() {
    if !on() {
        return;
    }
    TRACING.store(true, Ordering::Relaxed);
}

/// Whether the log has been opened yet, so the first write truncates
/// what a previous run left and the rest append.
static STARTED: OnceLock<()> = OnceLock::new();

/// Append `line` to the log, opening it if this is the first write.
fn append(line: &str) {
    let Some(path) = target() else {
        return;
    };
    let first = STARTED.set(()).is_ok();
    let opened = if first {
        OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)
    } else {
        OpenOptions::new().append(true).create(true).open(path)
    };
    if let Ok(mut file) = opened {
        if first {
            let _ = writeln!(
                file,
                "probe on -- one line per {SUMMARY_FRAMES} frames, plus every \
                 frame over {}ms",
                PROBE_THRESHOLD.as_millis(),
            );
        }
        let _ = writeln!(file, "{line}");
    }
}

/// Write out one frame's phases, and a summary line every
/// [`SUMMARY_FRAMES`] frames whether or not anything was slow.
///
/// `gap` is from the top of the previous iteration to the top of this
/// one, which is what the eye is actually reading. A summary always
/// arrives, so a log that says nothing is slow is telling the reader
/// that rather than leaving them to wonder whether it ran.
fn frame(gap: Duration, threshold: Duration) {
    if !on() {
        return;
    }
    let phases: Vec<(FramePhase, u64)> = PHASES
        .into_iter()
        .map(|phase| (phase, slot(phase).swap(0, Ordering::Relaxed)))
        .collect();
    let nanos = u64::try_from(gap.as_nanos()).unwrap_or(u64::MAX);
    WORST.fetch_max(nanos, Ordering::Relaxed);
    ELAPSED.fetch_add(nanos, Ordering::Relaxed);
    if gap >= threshold {
        SLOW.fetch_add(1, Ordering::Relaxed);
        append(&describe("gap", gap, &phases));
    } else if TRACING.load(Ordering::Relaxed) {
        if TRACED.fetch_add(1, Ordering::Relaxed) < TRACE_FRAMES {
            append(&describe("frame", gap, &phases));
        } else {
            TRACING.store(false, Ordering::Relaxed);
        }
    }
    if COUNT.fetch_add(1, Ordering::Relaxed) + 1 >= u64::from(SUMMARY_FRAMES) {
        COUNT.store(0, Ordering::Relaxed);
        let worst = Duration::from_nanos(WORST.swap(0, Ordering::Relaxed));
        let slow = SLOW.swap(0, Ordering::Relaxed);
        let over = Duration::from_nanos(ELAPSED.swap(0, Ordering::Relaxed));
        let wrote = WRITTEN.swap(0, Ordering::Relaxed);
        append(&format!(
            "-- {SUMMARY_FRAMES} frames in {:.2}s: worst gap {:.1}ms, {slow} over \
             threshold, wrote {wrote} bytes ({:.0} KiB/s)",
            over.as_secs_f64(),
            worst.as_secs_f64() * 1000.0,
            throughput(wrote, over) / 1024.0,
        ));
    }
}

/// Write `message` to the log as a line of its own, where the probe is
/// switched on. For the things that happen once rather than per frame.
fn note(message: &str) {
    if !on() {
        return;
    }
    append(message);
}

/// `bytes` spread over `over`, in bytes per second, or zero where no
/// time has passed for them to be spread over.
#[expect(
    clippy::cast_precision_loss,
    reason = "a byte count over one summary's worth of frames is far \
              inside what an f64 carries exactly"
)]
fn throughput(bytes: u64, over: Duration) -> f64 {
    let seconds = over.as_secs_f64();
    if seconds <= 0.0 {
        return 0.0;
    }
    bytes as f64 / seconds
}

/// One frame as a line of the log.
fn describe(label: &str, gap: Duration, phases: &[(FramePhase, u64)]) -> String {
    let mut line = format!(
        "{label} {:>7.1}ms  wrote={:<6}",
        gap.as_secs_f64() * 1000.0,
        WRITTEN_FRAME.swap(0, Ordering::Relaxed),
    );
    let mut accounted = 0.0;
    for &(phase, nanos) in phases {
        #[expect(
            clippy::cast_precision_loss,
            reason = "nanoseconds of one frame, printed to a tenth of a \
                      millisecond, are far inside what an f64 carries exactly"
        )]
        let millis = nanos as f64 / 1_000_000.0;
        if phase == FramePhase::Draw {
            accounted = millis;
        }
        let _ = write!(line, "  {}={millis:.1}", name(phase));
    }
    // What the loop cannot account for is what it was not running for,
    // which is the whole reason to print the gap beside the phases.
    let _ = write!(
        line,
        "  unaccounted={:.1}",
        gap.as_secs_f64().mul_add(1000.0, -accounted).max(0.0),
    );
    line
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    use tui_pane::FramePhase;

    use super::PHASES;
    use super::describe;
    use super::slot;

    /// Each phase keeps its own slot and its own column, and the columns
    /// run in the order every frame log has written them.
    #[test]
    fn a_frame_line_names_each_phase_in_a_fixed_column_order() {
        for (phase, millis) in [
            (FramePhase::Draw, 5),
            (FramePhase::Advance, 1),
            (FramePhase::Refresh, 2),
            (FramePhase::Panes, 3),
            (FramePhase::Band, 4),
        ] {
            slot(phase).store(millis * 1_000_000, Ordering::Relaxed);
        }
        let phases: Vec<(FramePhase, u64)> = PHASES
            .into_iter()
            .map(|phase| (phase, slot(phase).swap(0, Ordering::Relaxed)))
            .collect();

        assert_eq!(
            describe("frame", Duration::from_millis(20), &phases),
            "frame    20.0ms  wrote=0       draw=5.0  advance=1.0  refresh=2.0  panes=3.0  band=4.0  \
             unaccounted=15.0",
        );
    }
}
