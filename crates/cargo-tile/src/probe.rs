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
use std::io::Write as _;
use std::sync::OnceLock;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;

/// The phases of one frame, each timed on its own.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Phase {
    /// [`crate::attract::Attract::advance`], which carries the backdrop
    /// monitor's own per-frame work inside it.
    Advance,
    /// Reading the newest capture at where the window stands.
    Refresh,
    /// Drawing the panes, with or without their contents.
    Panes,
    /// Drawing the band over them.
    Band,
    /// Everything `terminal.draw` does, the flush to the tty included.
    Draw,
}

impl Phase {
    /// Where this phase's nanoseconds are kept.
    fn slot(self) -> &'static AtomicU64 {
        static SLOTS: [AtomicU64; 5] = [
            AtomicU64::new(0),
            AtomicU64::new(0),
            AtomicU64::new(0),
            AtomicU64::new(0),
            AtomicU64::new(0),
        ];
        &SLOTS[self as usize]
    }

    /// What this phase is called in the log.
    const fn name(self) -> &'static str {
        match self {
            Self::Advance => "advance",
            Self::Refresh => "refresh",
            Self::Panes => "panes",
            Self::Band => "band",
            Self::Draw => "draw",
        }
    }
}

/// Every phase, for walking them in a fixed order.
const PHASES: [Phase; 5] = [
    Phase::Draw,
    Phase::Advance,
    Phase::Refresh,
    Phase::Panes,
    Phase::Band,
];

/// The path to append to, or [`None`] where the probe is off.
fn target() -> Option<&'static str> {
    static TARGET: OnceLock<Option<String>> = OnceLock::new();
    TARGET
        .get_or_init(|| std::env::var("CARGO_TILE_FRAME_LOG").ok())
        .as_deref()
}

/// Whether anything here does any work at all.
pub(crate) fn on() -> bool { target().is_some() }

/// Time `body` and record it as `phase`.
pub(crate) fn timed<T>(phase: Phase, body: impl FnOnce() -> T) -> T {
    if !on() {
        return body();
    }
    let at = Instant::now();
    let answer = body();
    phase.slot().store(
        u64::try_from(at.elapsed().as_nanos()).unwrap_or(u64::MAX),
        Ordering::Relaxed,
    );
    answer
}

/// How many frames go into one summary line.
const SUMMARY_FRAMES: u32 = 120;

/// What has been seen since the last summary line.
static COUNT: AtomicU64 = AtomicU64::new(0);
/// The longest gap since the last summary line, in nanoseconds.
static WORST: AtomicU64 = AtomicU64::new(0);
/// Gaps over the threshold since the last summary line.
static SLOW: AtomicU64 = AtomicU64::new(0);

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
                crate::constants::PROBE_THRESHOLD.as_millis(),
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
pub(crate) fn frame(gap: Duration, threshold: Duration) {
    if !on() {
        return;
    }
    let phases: Vec<(Phase, u64)> = PHASES
        .into_iter()
        .map(|phase| (phase, phase.slot().swap(0, Ordering::Relaxed)))
        .collect();
    let nanos = u64::try_from(gap.as_nanos()).unwrap_or(u64::MAX);
    WORST.fetch_max(nanos, Ordering::Relaxed);
    if gap >= threshold {
        SLOW.fetch_add(1, Ordering::Relaxed);
        append(&describe("gap", gap, &phases));
    }
    if COUNT.fetch_add(1, Ordering::Relaxed) + 1 >= u64::from(SUMMARY_FRAMES) {
        COUNT.store(0, Ordering::Relaxed);
        let worst = Duration::from_nanos(WORST.swap(0, Ordering::Relaxed));
        let slow = SLOW.swap(0, Ordering::Relaxed);
        append(&format!(
            "-- {SUMMARY_FRAMES} frames: worst gap {:.1}ms, {slow} over threshold",
            worst.as_secs_f64() * 1000.0,
        ));
    }
}

/// One frame as a line of the log.
fn describe(label: &str, gap: Duration, phases: &[(Phase, u64)]) -> String {
    let mut line = format!("{label} {:>7.1}ms", gap.as_secs_f64() * 1000.0);
    let mut accounted = 0.0;
    for &(phase, nanos) in phases {
        #[expect(
            clippy::cast_precision_loss,
            reason = "nanoseconds of one frame, printed to a tenth of a \
                      millisecond, are far inside what an f64 carries exactly"
        )]
        let millis = nanos as f64 / 1_000_000.0;
        if matches!(phase, Phase::Draw) {
            accounted = millis;
        }
        let _ = write!(line, "  {}={millis:.1}", phase.name());
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
