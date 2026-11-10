//! Record the render stages of every drawn frame to locate slow draws.
//!
//! Set `CARGO_TILE_FRAME_LOG` to the output path to enable the log.
//! When the variable is absent, each call uses an atomic load and a branch.
//!
//! What it is for is telling two very different stalls apart. A frame
//! whose phases add up to the gap it left is one the render loop spent
//! doing something slow, and the phase names which. A frame whose gap
//! is long while every phase is short is one the render loop was not
//! running for -- descheduled, or held up in the allocator behind
//! another thread -- and no amount of looking at what it draws will
//! find that.

use std::cell::Cell;
use std::cell::RefCell;
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

use rustix::time::ClockId;
use tui_pane::FramePhase;
use tui_pane::FrameProbe;

#[cfg(feature = "attract")]
use crate::constants::ATTRACT_BACKDROP_UNAVAILABLE_NOTICE;
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
static ENABLED: AtomicBool = AtomicBool::new(false);

fn on() -> bool { ENABLED.load(Ordering::Relaxed) }

/// Cache work for the pending draw.
#[derive(Default)]
pub(crate) struct PaintWork {
    pub(crate) hits:        u64,
    pub(crate) missing:     u64,
    pub(crate) inputs:      u64,
    pub(crate) destination: u64,
    pub(crate) source:      u64,
    pub(crate) scanned:     usize,
    pub(crate) copied:      usize,
    pub(crate) revision:    u64,
    pub(crate) hidden:      u64,
    pub(crate) view:        u64,
    pub(crate) height:      u64,
    pub(crate) theme:       u64,
    pub(crate) unprepared:  u64,
}

thread_local! {
    static PAINT_WORK: RefCell<PaintWork> = RefCell::new(PaintWork::default());
}

/// Record cache work when the frame log is enabled.
pub(crate) fn paint_work(record: impl FnOnce(&mut PaintWork)) {
    if on() {
        PAINT_WORK.with_borrow_mut(record);
    }
}

/// Read the CPU clock for the current thread.
fn thread_cpu_ns() -> u64 {
    let time = rustix::time::clock_gettime(ClockId::ThreadCPUTime);
    u64::try_from(time.tv_sec)
        .unwrap_or(0)
        .saturating_mul(1_000_000_000)
        .saturating_add(u64::try_from(time.tv_nsec).unwrap_or(0))
}

/// CPU time within the pending terminal draw.
#[derive(Default)]
struct DrawCpuTime {
    total:    u64,
    callback: u64,
    write:    u64,
    flush:    u64,
}

thread_local! {
    static DRAW_CPU: RefCell<DrawCpuTime> = RefCell::new(DrawCpuTime::default());
}

/// The stored scheduler runtime and run-queue time.
#[derive(Clone, Copy)]
enum RenderThreadTime {
    Unavailable,
    Measured { cpu_ns: u64, runq_ns: u64 },
}

impl RenderThreadTime {
    fn read() -> Self {
        let Ok(contents) = std::fs::read_to_string("/proc/thread-self/schedstat") else {
            return Self::Unavailable;
        };
        let mut fields = contents.split_whitespace();
        match (
            fields.next().and_then(|value| value.parse().ok()),
            fields.next().and_then(|value| value.parse().ok()),
        ) {
            (Some(cpu_ns), Some(runq_ns)) => Self::Measured { cpu_ns, runq_ns },
            _ => Self::Unavailable,
        }
    }

    const fn since(self, earlier: Self) -> Self {
        match (self, earlier) {
            (
                Self::Measured { cpu_ns, runq_ns },
                Self::Measured {
                    cpu_ns: before_cpu,
                    runq_ns: before_runq,
                },
            ) => Self::Measured {
                cpu_ns:  cpu_ns.saturating_sub(before_cpu),
                runq_ns: runq_ns.saturating_sub(before_runq),
            },
            _ => Self::Unavailable,
        }
    }
}

thread_local! {
    static DRAW_THREAD_TIME: Cell<RenderThreadTime> = const { Cell::new(RenderThreadTime::Unavailable) };
}

/// Time `body` and record it as `phase`.
fn timed<T>(phase: FramePhase, body: impl FnOnce() -> T) -> T {
    if !on() {
        return body();
    }
    let drawing = matches!(phase, FramePhase::Draw);
    let before = if drawing {
        DRAW_CPU.with_borrow_mut(|time| *time = DrawCpuTime::default());
        RenderThreadTime::read()
    } else {
        RenderThreadTime::Unavailable
    };
    let at = Instant::now();
    let cpu_before = if drawing { thread_cpu_ns() } else { 0 };
    let answer = body();
    let cpu_after = if drawing { thread_cpu_ns() } else { 0 };
    let elapsed = at.elapsed();
    if drawing {
        DRAW_CPU.with_borrow_mut(|time| time.total = cpu_after.saturating_sub(cpu_before));
        DRAW_THREAD_TIME.set(RenderThreadTime::read().since(before));
    }
    slot(phase).store(
        u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX),
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
        if !on() {
            return self.inner.write(buf);
        }
        let before = thread_cpu_ns();
        let result = self.inner.write(buf);
        let elapsed = thread_cpu_ns().saturating_sub(before);
        DRAW_CPU.with_borrow_mut(|time| time.write += elapsed);
        if let Ok(written) = result {
            let count = u64::try_from(written).unwrap_or(0);
            WRITTEN.fetch_add(count, Ordering::Relaxed);
            WRITTEN_FRAME.fetch_add(count, Ordering::Relaxed);
        }
        result
    }

    fn flush(&mut self) -> io::Result<()> {
        if !on() {
            return self.inner.flush();
        }
        let before = thread_cpu_ns();
        let result = self.inner.flush();
        let elapsed = thread_cpu_ns().saturating_sub(before);
        DRAW_CPU.with_borrow_mut(|time| time.flush += elapsed);
        result
    }
}

/// The frame log as the runner's [`FrameProbe`]: every pass of the
/// loop is a [`frame`], each [`FramePhase`] is [`timed`] into its own
/// column, and the terminal's output is [`Counted`] once the setup has
/// been written.
pub(crate) enum FrameLog {}

impl FrameProbe for FrameLog {
    type Output = Counted<Stdout>;

    #[cfg(feature = "attract")]
    const BACKDROP_UNAVAILABLE_NOTICE: &'static str = ATTRACT_BACKDROP_UNAVAILABLE_NOTICE;

    fn output(stdout: Stdout) -> Self::Output {
        ENABLED.store(target().is_some(), Ordering::Relaxed);
        Counted::new(stdout)
    }

    fn frame_started(gap: Duration) { frame(gap, PROBE_THRESHOLD); }

    fn timed<T>(phase: FramePhase, body: impl FnOnce() -> T) -> T { timed(phase, body) }

    fn timed_callback<T>(body: impl FnOnce() -> T) -> T {
        if !on() {
            return body();
        }
        let before = thread_cpu_ns();
        let answer = body();
        let elapsed = thread_cpu_ns().saturating_sub(before);
        DRAW_CPU.with_borrow_mut(|time| time.callback += elapsed);
        answer
    }

    fn note(line: &str) { note(line); }

    fn trace() { trace(); }
}

/// How many frames go into one summary line.
const SUMMARY_FRAMES: u32 = 120;

/// What has been seen since the last summary line.
static COUNT: AtomicU64 = AtomicU64::new(0);
/// The sequence number shared by each draw's measurements.
static FRAME_NUMBER: AtomicU64 = AtomicU64::new(0);
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
                "probe on -- every drawn frame, one summary per {SUMMARY_FRAMES} frames, \
                 plus every gap over {}ms",
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
    let draw_nanos = slot(FramePhase::Draw).load(Ordering::Relaxed);
    let thread_time = DRAW_THREAD_TIME.replace(RenderThreadTime::Unavailable);
    let phases: Vec<(FramePhase, u64)> = PHASES
        .into_iter()
        .map(|phase| (phase, slot(phase).swap(0, Ordering::Relaxed)))
        .collect();
    let nanos = u64::try_from(gap.as_nanos()).unwrap_or(u64::MAX);
    WORST.fetch_max(nanos, Ordering::Relaxed);
    ELAPSED.fetch_add(nanos, Ordering::Relaxed);
    if gap >= threshold {
        SLOW.fetch_add(1, Ordering::Relaxed);
    }
    if draw_nanos > 0 || gap >= threshold {
        let label = if gap >= threshold { "gap" } else { "frame" };
        let mut line = format!("{} draw_ns={draw_nanos}", describe(label, gap, &phases));
        let cpu = DRAW_CPU.take();
        let _ = write!(line, " cpu_ns={}", cpu.total);
        if let RenderThreadTime::Measured { cpu_ns, runq_ns } = thread_time {
            let _ = write!(line, " runq_ns={runq_ns} sched_cpu_ns={cpu_ns}");
        }
        let terminal = cpu
            .total
            .saturating_sub(cpu.callback + cpu.write + cpu.flush);
        let _ = write!(
            line,
            " callback_cpu_ns={} terminal_cpu_ns={terminal} write_cpu_ns={} flush_cpu_ns={}",
            cpu.callback, cpu.write, cpu.flush
        );
        let work = PAINT_WORK.take();
        let frame_number = FRAME_NUMBER.fetch_add(1, Ordering::Relaxed) + 1;
        let _ = write!(
            line,
            " frame_id={frame_number} cache_hits={} cache_missing={} cache_inputs={} cache_destination={} cache_source={} cells_scanned={} cells_copied={} invalid_revision={} invalid_hidden={} invalid_view={} invalid_height={} invalid_theme={} unprepared={}",
            work.hits,
            work.missing,
            work.inputs,
            work.destination,
            work.source,
            work.scanned,
            work.copied,
            work.revision,
            work.hidden,
            work.view,
            work.height,
            work.theme,
            work.unprepared
        );
        append(&line);
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
