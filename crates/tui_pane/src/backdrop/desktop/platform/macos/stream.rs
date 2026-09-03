//! A process-global registry of one persistent `ScreenCaptureKit` stream per display.
//!
//! [`capture_display_bgra`] pulls the newest frame for a display, opening or reopening its stream
//! when the windows it excludes, its output size, or its liveness change. The wedge-prone
//! window-server call happens once at open -- bounded by [`drive_until`] so it cannot hang the
//! worker thread -- and every steady-state pull is a lock-free read of an already-delivered frame,
//! so several instances capturing the same display never stall one another. A `ScreenCaptureKit`
//! session is multi-client, so each process captures the shared display on its own.

use std::collections::HashMap;
use std::fs::File;
use std::future::Future;
use std::pin::pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::PoisonError;
use std::task::Context;
use std::task::Poll;
use std::task::Wake;
use std::task::Waker;
use std::thread::Thread;
use std::thread::park_timeout;
use std::time::Duration;
use std::time::Instant;

use screencapturekit::async_api::AsyncSCShareableContent;
use screencapturekit::async_api::AsyncSCStream;
use screencapturekit::cm::CMSampleBufferExt;
use screencapturekit::cv::CVPixelBufferLockFlags;
use screencapturekit::cv::CVPixelBufferLockGuard;
use screencapturekit::shareable_content::SCWindow;
use screencapturekit::stream::configuration::PixelFormat;
use screencapturekit::stream::configuration::SCStreamConfiguration;
use screencapturekit::stream::content_filter::SCContentFilter;
use screencapturekit::stream::output_type::SCStreamOutputType;

use super::BYTES_PER_PIXEL;
use crate::backdrop::desktop::CaptureFailure;

/// The lock file every capturing process of this user serializes stream opens through, joined onto
/// the per-user temporary directory.
const OPEN_LOCK_FILE: &str = "tui_pane-desktop-capture-open.lock";
/// How long an opening stream parks between future polls, bounding how stale its deadline and
/// stream-stopped checks can be.
const OPEN_POLL_INTERVAL: Duration = Duration::from_millis(100);
/// Deadline bounding one stream open -- the shareable-content read plus the start-capture
/// confirmation -- held under the open lock. Below twice the monitor's five-second
/// `CAPTURE_ATTEMPT_DEADLINE`, so a slow first open costs at most one tolerated stall rather than a
/// worker replacement.
const OPEN_DEADLINE: Duration = Duration::from_secs(8);
/// How long a freshly opened stream is polled for its first frame, with no lock held, before the
/// attempt gives up.
const FIRST_FRAME_DEADLINE: Duration = Duration::from_secs(3);
/// How long a first-frame poll parks between frame pulls.
const FRAME_POLL_INTERVAL: Duration = Duration::from_millis(25);
/// How many samples the crate's async buffer holds; newest-wins is fine at the backdrop's roughly
/// once-a-second refresh.
const STREAM_BUFFER_CAPACITY: usize = 2;
/// How many frames `ScreenCaptureKit` queues before dropping the oldest.
const STREAM_QUEUE_DEPTH: u32 = 3;
/// The most frames a second `ScreenCaptureKit` delivers; the backdrop refreshes about once a
/// second.
const STREAM_MAX_FPS: u32 = 2;

/// A running `ScreenCaptureKit` capture session for one display.
///
/// Wraps the crate's [`AsyncSCStream`]: its bounded sample buffer keeps the newest frame, which
/// [`try_frame`] pops without blocking, and its delegate records why the system stopped the
/// stream, which [`ScreenCaptureStream::closed_reason`] reports. Dropping it stops the capture.
struct ScreenCaptureStream {
    /// The crate's async capture stream, driven by hand at open and read without blocking after.
    stream: AsyncSCStream,
}

impl ScreenCaptureStream {
    /// Why the stream stopped, or [`None`] while it is still delivering.
    ///
    /// `ScreenCaptureKit` stops a stream on its own when its display is unplugged, when the Screen
    /// Recording permission is revoked, or when another process's capture start contends with this
    /// one; the registry polls this to reopen the display's stream.
    fn closed_reason(&self) -> Option<String> {
        self.stream.is_closed().then(|| stop_message(&self.stream))
    }
}

impl Drop for ScreenCaptureStream {
    fn drop(&mut self) {
        // stop_capture starts the stop eagerly; the future is dropped unawaited so a slow or wedged
        // stop never blocks this thread.
        drop(self.stream.stop_capture());
    }
}

/// The delegate-recorded stop error, or a fallback when the stream closed without one.
fn stop_message(stream: &AsyncSCStream) -> String {
    stream.take_error().map_or_else(
        || "the system stopped the stream".to_string(),
        |error| error.to_string(),
    )
}

/// How [`drive_until`] ended.
enum DriveOutcome<T> {
    /// The future resolved to this value.
    Resolved(T),
    /// `stopped` reported the stream closed before the future resolved.
    Stopped,
    /// The deadline passed before the future resolved.
    TimedOut,
}

/// Wakes the thread parked in [`drive_until`] when the polled future's completion callback fires.
struct ThreadWaker(Thread);

impl Wake for ThreadWaker {
    fn wake(self: Arc<Self>) { self.0.unpark(); }
}

/// Polls `future` on the calling thread until it resolves, `stopped` returns true, or `deadline`
/// passes.
///
/// Parks between polls for at most [`OPEN_POLL_INTERVAL`] so `stopped` and the deadline are
/// rechecked even when no completion callback ever fires -- the bound a blocking
/// `SCStream::start_capture` lacks, and the reason this drives the async surface by hand.
fn drive_until<F: Future>(
    future: F,
    deadline: Instant,
    mut stopped: impl FnMut() -> bool,
) -> DriveOutcome<F::Output> {
    let waker = Waker::from(Arc::new(ThreadWaker(std::thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut future = pin!(future);
    loop {
        if let Poll::Ready(output) = future.as_mut().poll(&mut context) {
            return DriveOutcome::Resolved(output);
        }
        if stopped() {
            return DriveOutcome::Stopped;
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return DriveOutcome::TimedOut;
        }
        park_timeout(remaining.min(OPEN_POLL_INTERVAL));
    }
}

/// Takes the exclusive advisory lock on [`OPEN_LOCK_FILE`], blocking until any other in-flight open
/// (in this process or another instance) releases it. The caller holds the returned [`File`] for
/// the duration of its stream setup. Returns [`None`] when the lock file cannot be created or
/// locked -- the open then proceeds unserialized rather than failing.
fn acquire_open_lock() -> Option<File> {
    let lock = File::create(std::env::temp_dir().join(OPEN_LOCK_FILE)).ok()?;
    lock.lock().ok()?;
    Some(lock)
}

/// Opens the persistent capture stream for `display_id`, excluding the windows named by
/// `excluded_ids` and delivering frames sized to `output_size` (display points), then starts it.
///
/// Runs synchronously on the caller's disposable worker thread; [`drive_until`] bounds every await
/// under [`OPEN_DEADLINE`], so an open the system abandons mid-start reports an error instead of
/// wedging the thread the way a blocking `SCStream::start_capture` would.
///
/// # Errors
/// Returns `Err(())` when the shareable content cannot be read, the display is missing from it
/// (also the case when Screen Recording permission is denied), the stream cannot be started, the
/// system stops the stream mid-start, or the deadline passes.
fn open_stream(
    display_id: u32,
    excluded_ids: &[u32],
    output_size: (u32, u32),
) -> Result<ScreenCaptureStream, ()> {
    // Held until this returns, so stream opens across every capturing process (and this process's
    // other displays) run one at a time -- simultaneous opens are what wedge a registration into a
    // stream that confirms its start but never delivers a frame. Taken before the deadline starts,
    // so time spent waiting for another open is not charged to this one.
    let _lock = acquire_open_lock();
    let deadline = Instant::now() + OPEN_DEADLINE;

    let content = match drive_until(AsyncSCShareableContent::get(), deadline, || false) {
        DriveOutcome::Resolved(Ok(content)) => content,
        DriveOutcome::Resolved(Err(_)) | DriveOutcome::Stopped | DriveOutcome::TimedOut => {
            return Err(());
        },
    };
    let display = content
        .displays()
        .into_iter()
        .find(|display| display.display_id() == display_id)
        .ok_or(())?;
    let excluded_windows = content
        .windows()
        .into_iter()
        .filter(|window| excluded_ids.contains(&window.window_id()))
        .collect::<Vec<SCWindow>>();
    let excluded_refs = excluded_windows.iter().collect::<Vec<&SCWindow>>();

    let filter = SCContentFilter::create()
        .with_display(&display)
        .with_excluding_windows(&excluded_refs)
        .build();
    let configuration = SCStreamConfiguration::new()
        .with_width(output_size.0)
        .with_height(output_size.1)
        .with_pixel_format(PixelFormat::BGRA)
        .with_queue_depth(STREAM_QUEUE_DEPTH)
        .with_fps(STREAM_MAX_FPS)
        .with_shows_cursor(false);

    let stream = AsyncSCStream::new(
        &filter,
        &configuration,
        STREAM_BUFFER_CAPACITY,
        SCStreamOutputType::Screen,
    );
    if stream.is_closed() {
        return Err(());
    }
    match drive_until(stream.start_capture(), deadline, || stream.is_closed()) {
        DriveOutcome::Resolved(Ok(())) => Ok(ScreenCaptureStream { stream }),
        DriveOutcome::Resolved(Err(_)) | DriveOutcome::Stopped | DriveOutcome::TimedOut => Err(()),
    }
}

/// Pulls the newest delivered frame from `stream`, or [`None`] when none has arrived since the last
/// pull. Returns tightly-packed BGRA bytes (stride `width * BYTES_PER_PIXEL`) with the frame's
/// width and height in pixels.
fn try_frame(stream: &ScreenCaptureStream) -> Option<(Vec<u8>, usize, usize)> {
    let sample = stream.stream.try_next()?;
    let pixel_buffer = sample.image_buffer()?;
    let pixels = pixel_buffer.lock(CVPixelBufferLockFlags::READ_ONLY).ok()?;
    Some(tightly_packed_frame(&pixels))
}

/// Copies the locked pixel buffer into owned bytes, dropping any per-row padding `CoreVideo` adds
/// beyond `width * BYTES_PER_PIXEL`.
fn tightly_packed_frame(pixels: &CVPixelBufferLockGuard) -> (Vec<u8>, usize, usize) {
    let width = pixels.width();
    let height = pixels.height();
    let row_bytes = width * BYTES_PER_PIXEL;

    let mut data = Vec::with_capacity(row_bytes * height);
    for row in pixels.as_slice().chunks_exact(pixels.bytes_per_row()) {
        data.extend_from_slice(&row[..row_bytes]);
    }

    (data, width, height)
}

/// One display's persistent capture stream and the last good frame read from it.
struct DisplayStream {
    /// The running capture session for this display.
    stream:     ScreenCaptureStream,
    /// The window ids excluded when the stream was opened, sorted and deduplicated; a change
    /// reopens the stream.
    excluded:   Vec<u32>,
    /// The configured output size in display points; a change reopens the stream.
    dimensions: (u32, u32),
    /// The last good tightly-packed BGRA frame with its width and height, kept so a static desktop
    /// still reports success.
    last:       Option<(Vec<u8>, usize, usize)>,
}

/// One persistent capture stream per display, keyed by `CGDirectDisplayID`.
static STREAMS: OnceLock<Mutex<HashMap<u32, DisplayStream>>> = OnceLock::new();

/// Captures the desktop on `display_id` as tightly-packed BGRA, opening or reopening the display's
/// persistent stream as needed and returning the newest frame.
///
/// `display_id` is the `CGDirectDisplayID`, which equals `SCDisplay::display_id()`. `output_size`
/// is the display's bounds in points (see the point-size contract in the parent module).
/// `excluded_window_ids` names every window the terminal emulator owns, which the stream's content
/// filter leaves out. `access_granted` is [`screen_capture_access_is_granted`], used only to
/// classify an open failure as a permission problem rather than a capture one.
///
/// Returns the BGRA bytes with their width, height, and row stride (`width * BYTES_PER_PIXEL`), or
/// the capture stage that failed.
///
/// The registry mutex is held for the whole call: the monitor drives one attempt at a time on a
/// single worker thread, so there is no contention to optimize, and holding it keeps the
/// per-display entry consistent.
///
/// [`screen_capture_access_is_granted`]: super::screen_capture_access_is_granted
pub(super) fn capture_display_bgra(
    display_id: u32,
    output_size: (u32, u32),
    excluded_window_ids: &[u32],
    access_granted: bool,
) -> Result<(Vec<u8>, usize, usize, usize), CaptureFailure> {
    let mut excluded = excluded_window_ids.to_vec();
    excluded.sort_unstable();
    excluded.dedup();

    let streams = STREAMS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut map = streams.lock().unwrap_or_else(PoisonError::into_inner);

    // A stream is reopened when there is none yet, when the windows it excludes or its output size
    // no longer match what is asked for, or when the system has stopped it.
    let reopen = map.get(&display_id).is_none_or(|entry| {
        entry.excluded != excluded
            || entry.dimensions != output_size
            || entry.stream.closed_reason().is_some()
    });
    if reopen {
        map.remove(&display_id);
        let stream = open_stream(display_id, &excluded, output_size).map_err(|()| {
            if access_granted {
                CaptureFailure::DisplayCaptureFailed
            } else {
                CaptureFailure::ScreenRecordingAccessNotGranted
            }
        })?;
        map.insert(
            display_id,
            DisplayStream {
                stream,
                excluded,
                dimensions: output_size,
                last: None,
            },
        );
    }

    // Present by construction -- just inserted, or the pre-existing entry that did not reopen. The
    // fallback keeps this lint-clean without an `expect`, and never triggers.
    let Some(entry) = map.get_mut(&display_id) else {
        return Err(CaptureFailure::DisplayCaptureFailed);
    };

    // A freshly opened stream has delivered nothing yet, so its first pull is polled up to the
    // first-frame deadline. `ScreenCaptureKit` only delivers a frame when the content changes (or
    // at the capped rate), so a later pull over a static desktop returns nothing and the last good
    // frame stands in -- keeping every attempt a success while a genuine change still refreshes it.
    let fresh = entry.last.is_none();
    let mut frame = try_frame(&entry.stream);
    if frame.is_none() && fresh {
        let deadline = Instant::now() + FIRST_FRAME_DEADLINE;
        while frame.is_none() && Instant::now() < deadline {
            park_timeout(FRAME_POLL_INTERVAL);
            frame = try_frame(&entry.stream);
        }
    }
    if let Some(new_frame) = frame {
        entry.last = Some(new_frame);
    }

    // The frame is cloned out of the guarded map into owned bytes, then the guard is dropped before
    // returning -- the registry mutex is held no longer than the copy needs it.
    let captured = entry
        .last
        .as_ref()
        .map(|(bytes, width, height)| (bytes.clone(), *width, *height, *width * BYTES_PER_PIXEL));
    drop(map);
    captured.ok_or(CaptureFailure::DisplayCaptureFailed)
}
