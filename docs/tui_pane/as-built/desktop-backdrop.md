# Desktop backdrop capture pipeline — as-built

A reference for the next engineer modifying `tui_pane`'s desktop-backdrop feature.
Covers the capture pipeline under `crates/tui_pane/src/backdrop/` (behind the opt-in
`backdrop` feature that only `cargo-tile` enables), the moving-band renderer that paints
a captured desktop under drifting glyphs, and the failure→notice reporting that completes
the story in cargo-tile's attract screen.

`tui_pane` is the reusable ratatui pane framework shared by `cargo-tile` and `cargo-port`;
all backdrop code sits behind `#[cfg(feature = "backdrop")]`, which only `cargo-tile`
enables.

---

## What it is

cargo-tile's idle attract screen animates the *actual desktop* behind the terminal —
sampled live, reduced to one color per character cell, and painted under drifting text,
resolving pixels, or a traveling band. When that capture cannot happen (no Screen
Recording permission, a wedged window-server call, no matchable window), the screen says
so in a single status line that names the real cause and, where the user can act, tells
them how.

The render loop's repaint pacing — *when* the next frame is scheduled so these animations
are demand-driven rather than polled — is a separate `tui_pane` concern documented in
[toast-visual-deadline.md](./toast-visual-deadline.md).

---

## How it works

The capture path lives under `crates/tui_pane/src/backdrop/`, entirely behind
`#[cfg(feature = "backdrop")]`. Data flows: a **monitor** on the render thread paces and
bounds attempts → a disposable **worker thread** runs one `Desktop::capture` at a time →
capture **selects a terminal window**, finds its display, and pulls a frame from a
**persistent per-display ScreenCaptureKit stream** → the frame is reduced to a cell grid
→ the monitor retains the newest good `Desktop` and a lightweight per-attempt diagnostic.

### The persistent stream registry — `backdrop/desktop/platform/macos/stream.rs`

This is the ground-truth capture mechanism. Each display is captured through **one
persistent, multi-client `ScreenCaptureKit` `SCStream`**, opened once and shared, rather
than a fresh in-process screenshot per refresh. (Per-refresh screenshots wedge
process-to-process on macOS 26 the moment several cargo-tile instances overlap on the
window server; a multi-client SCStream lets each process capture the shared display on
its own.)

- `static STREAMS: OnceLock<Mutex<HashMap<u32, DisplayStream>>>` — the process-global
  registry, keyed by `CGDirectDisplayID`.
- `struct DisplayStream { stream: ScreenCaptureStream, excluded: Vec<u32>, dimensions:
  (u32, u32), last: Option<(Vec<u8>, usize, usize)> }` — one display's running session
  plus the **last good frame** (`last`), kept so a static desktop still reports success.
- `pub(super) fn capture_display_bgra(display_id: u32, output_size: (u32,u32),
  excluded_window_ids: &[u32], access_granted: bool) -> Result<(Vec<u8>, usize, usize,
  usize), CaptureFailure>` — the entry point. Returns tightly-packed BGRA bytes with
  `(width, height, stride = width * BYTES_PER_PIXEL)`, or a `CaptureFailure`. It sorts +
  dedups the excluded ids, then **reopens** the display's stream when there is none, when
  the excluded set or `output_size` changed, or when `ScreenCaptureStream::closed_reason`
  reports the system stopped it; otherwise it pops the newest already-delivered frame.
  Holds the registry mutex for the whole call (the monitor drives one attempt at a time,
  so there is no contention to optimize).
- `struct ScreenCaptureStream { stream: AsyncSCStream }` — wraps the crate's async
  stream; `try_frame` pops the newest sample without blocking; `closed_reason()` reports
  why the system stopped it; `Drop` starts `stop_capture()` and drops the future
  unawaited so a wedged stop never blocks the thread.

The **wedge-prone window-server work happens once, at open**, and is serialized and
bounded:

- `fn open_stream(display_id, excluded_ids, output_size) -> Result<ScreenCaptureStream,
  ()>` reads shareable content, builds an `SCContentFilter` (display minus every excluded
  window), configures BGRA output at `output_size`, creates the stream, and starts
  capture.
- `fn acquire_open_lock() -> Option<File>` takes an **exclusive advisory file lock** on
  `$TMPDIR/tui_pane-desktop-capture-open.lock`. It is held for the whole of `open_stream`,
  so stream opens across every capturing process of this user (and this process's other
  displays) run one at a time — simultaneous opens are what wedge a registration into a
  stream that confirms its start but never delivers a frame. Taken *before* the deadline
  starts, so waiting on another open is not charged to this one; a lock that cannot be
  taken proceeds unserialized rather than failing.
- `fn drive_until<F: Future>(future, deadline, stopped) -> DriveOutcome<F::Output>` drives
  the async surface **by hand** on the worker thread. It parks (`park_timeout`) at most
  `OPEN_POLL_INTERVAL` between polls so the `deadline` and the stream-stopped check are
  rechecked even when no completion callback ever fires — the bound a blocking
  `SCStream::start_capture` lacks. `DriveOutcome::{Resolved(T), Stopped, TimedOut}`.

An open failure maps to a permission problem or a capture problem by the `access_granted`
flag the caller passes: `ScreenRecordingAccessNotGranted` when access is not granted,
else `DisplayCaptureFailed`.

### The macOS backend — `backdrop/desktop/platform/macos/mod.rs`

`pub(in crate::backdrop::desktop) fn capture(metrics, capture_window_target, sequence) ->
CaptureAttemptResult` orchestrates one attempt. Everything except the stream frame-pull
uses **CoreGraphics**, not ScreenCaptureKit, because the drawing threads ask the
per-window questions (where a window stands, its title) far more often and
`SCShareableContent::get` costs ~70ms where `CGWindowList`-by-id costs a few hundred
microseconds.

- `struct Listed` + `Listed::on_screen()` enumerate every on-screen window (front-to-back)
  from `CGWindowListCopyWindowInfo`.
- `struct Display` + `active_displays()` read the active displays from
  `CGGetActiveDisplayList`; `display_under(displays, frame)` finds the display holding a
  window's center (nearest-center fallback so a straddling window never silently snaps to
  the primary).
- Window selection is delegated to the target-independent `candidate` module (below);
  `capture` supplies the size-fallback closure `frontmost_window` (matches this tty's
  `TIOCGWINSZ` text area against each candidate frame, scored per-display via
  `backing_scale`/`mismatch`).
- `fn capture_selected_window(...) -> Result<Desktop, CaptureFailure>` computes the
  display's point bounds, the cell size in points (`metrics.cell_points(backing_scale)`),
  the excluded window ids (every window owned by the chosen window's pid, or just the
  chosen window when the owner is `Unnamed`), calls `stream::capture_display_bgra`, then
  `bgra_rows_to_rgba` → `reduce_capture` → `Desktop`.
- `fn reduce_capture(pixels, image, cell)` / `fn reduce(...)` average each cell's share of
  the image (`SAMPLES_PER_CELL` samples per axis) down to one `Color::Rgb`.
- `screen_capture_access_is_granted()` is a direct `CGPreflightScreenCaptureAccess` — it
  never prompts and cannot distinguish "never asked" from "refused".

The non-macOS backend (`platform/fallback.rs`) returns `CaptureFailure::UnsupportedPlatform`.

### Window selection — `backdrop/desktop/candidate.rs`

Target-independent so tests drive the production selector rather than a copy.

- `enum CaptureWindowTarget { PreferWindow { window_id: u32 }, TerminalWindowHeuristic }`
  — what the monitor asks capture to aim at.
- `fn select_capture_window(windows, capture_window_target, terminal_window_candidates,
  window_id, closest_size_match) -> Result<(&W, CaptureWindowSelectionMethod),
  CaptureFailure>` — honors a pinned window if it still exists, else the closest-size
  match from the classified candidate set.
- `fn terminal_window_candidates(windows, process_is_ancestor,
  window_is_owned_by_terminal_program) -> TerminalWindowCandidates` classifies by, in
  order: **process ancestry**, then the app named by `TERM_PROGRAM`, then the **frontmost
  application's** owner. (An emulator hosting sessions in a server process — iTerm2 under
  `iTermServer` — is nowhere in this app's parent chain, so `TERM_PROGRAM` and frontmost
  are the fallbacks.) The source is carried as
  `TerminalWindowCandidateSource::{ProcessAncestry, TerminalProgramName,
  FrontmostApplication}`.
- `enum TerminalWindowSearchOutcome { NotFound, Found { window_id: u32 } }` — the answer of
  both marker/position window lookups (`desktop::window_titled`, `desktop::window_at`).

### The failure/attempt record — `backdrop/desktop/capture_attempt.rs`

- `enum CaptureFailure` — the **failure-staging enum**, `Copy`, naming the one stage or
  worker-lifecycle event that stopped capture, so it is cheap to send from the worker and
  retain as monitor state. Twelve variants: `UnsupportedPlatform`, `AttemptStalled`,
  `CaptureWorkerReplaced`, `WorkerLaunchFailed`, `WorkerDisconnected`,
  `WorkerReplacementLimitReached`, `ScreenRecordingAccessNotGranted`,
  `TerminalWindowNotFound`, `DisplayNotFound`, `DisplayCaptureFailed`,
  `PixelExtractionFailed`, `ImageReductionFailed`. (`DisplayCaptureFailed` = "the stream
  delivered no frame" — the stream rework folded the former one-shot-"Screenshot" stage
  into this.)
- `struct CaptureAttemptResult { sequence, window_selection, outcome }` — the worker's
  result. `CaptureAttemptOutcome::{Succeeded(Arc<Desktop>), Failed(CaptureFailure)}` is
  internal; on receipt the monitor splits it with `into_diagnostic_and_desktop_result()`
  so a queued diagnostic never retains an `Arc<Desktop>`.
- `struct CompletedCaptureAttemptDiagnostic { sequence, window_selection, outcome:
  Result<(), CaptureFailure> }` — the retained `Copy` record.
- `enum CaptureAttemptWindowSelection { SelectionNotReached, Selected { window_id, method:
  CaptureWindowSelectionMethod } }`, `enum CaptureWindowSelectionMethod { PinnedWindow,
  ClosestSizeMatch { terminal_window_candidate_source } }`.
- `struct CaptureAttemptSequence(u64)` — monotonic per-monitor sequence.
- `enum CaptureAttemptTestCase` (`#[doc(hidden)]`) — synthetic situations that drive the
  production selector from a client crate's tests.

### The monitor — `backdrop/monitor/{mod,window_identification,capture_test_driver}.rs`

`pub struct BackdropMonitor` owns all backdrop state on the render thread. Key fields:
`capture_worker: CaptureWorkerAvailability`, `capture_worker_launcher`,
`worker_replacements`, `capture_worker_responsiveness`, `last_successful_desktop:
LastSuccessfulDesktop`, `status: BackdropStatus`, `latest_window_selection`,
`completed_attempts: VecDeque<CompletedCaptureAttemptDiagnostic>`, `next_sequence`,
`current: Option<Backdrop>`, `capture_request_cadence`, `capture_attempt_progress`,
`placement`, `window_identification_state`, plus the `watches`/`frames` channels to a
separate position worker.

- `pub fn refresh(&mut self, area: Rect)` — one frame's work: drain worker results
  (`receive_capture_attempt_results`), **recover a stalled attempt**
  (`recover_stalled_capture_attempt(Instant::now())`), read the latest window position,
  recompute the drawable `current: Backdrop` from the retained desktop + placement, and
  request a fresh capture when due. A **moving** (being-dragged) window requests nothing —
  a drag changes nothing a capture holds, and re-compositing the display while the window
  server is busy moving a window is the most expensive possible moment.
- `pub fn identify(&mut self, out: &mut impl Write) -> WindowIdentification` — delegates to
  `window_identification_state` (below).
- Worker plumbing: `request_capture_if_worker_available` is the **single** `CaptureRequest`
  construction site; `capture_loop` runs `Desktop::capture` on the worker thread;
  `record_capture_attempt_result` retains the desktop, sets `status`, updates
  `latest_window_selection`, and pushes the bounded diagnostic (dropping the oldest at
  `MAX_RETAINED_CAPTURE_ATTEMPT_DIAGNOSTICS`).
- **Stall/replace bounding**: `recover_stalled_capture_attempt` completes an attempt older
  than `CAPTURE_ATTEMPT_DEADLINE` with a synthesized failed `CaptureAttemptResult`
  (`SelectionNotReached`, `AttemptStalled`). `CaptureWorkerResponsiveness::{Answering,
  SilentSinceDeadline}` distinguishes a first stall (kept — a first capture can
  legitimately take seconds) from a second consecutive one (`CaptureWorkerReplaced` →
  `replace_capture_worker`). Replacement marks the old worker `PermanentlyUnavailable`,
  relaunches via `CaptureWorkerLauncher`, resets cadence to `DueImmediately`; at
  `MAX_CAPTURE_WORKER_REPLACEMENTS` it stops and sets
  `BackdropStatus::Failed(WorkerReplacementLimitReached)`. **The wedged thread is never
  joined — it leaks, deliberately** (there is no timeout-bounded wait available at the SCK
  call site).
- Reporting accessors: `pub const fn status(&self) -> BackdropStatus`
  (`{WaitingForFirstResult, Ready, Failed(CaptureFailure)}` — describes the *newest
  attempt*, not availability), `pub const fn current(&self) -> Option<&Backdrop>`, `pub
  const fn captured_window_id(&self) -> LastSuccessfulCaptureWindowId`, `pub const fn
  latest_capture_attempt_window_selection(&self) -> LatestCaptureAttemptWindowSelection`,
  and `pub fn take_completed_capture_attempt_diagnostics(&mut self) -> impl
  Iterator<Item = CompletedCaptureAttemptDiagnostic>` (pulls from the channel first, so a
  caller need not `refresh` first).

`window_identification.rs` holds the search state machine:
`enum WindowIdentificationState { NotAttempted, PendingBeforeMarker{..},
PendingWithMarker{..}, Identified{window_id}, Fallback }` owns all phase-dependent data
(no bare `Option` fields that could contradict each other) and projects to two views:
`report() -> WindowIdentification` (the public report) and `capture_window_target() ->
CaptureWindowTarget` (what to ask capture for). `pub enum WindowIdentification {
NotAttempted, Pending, Identified{window_id}, Fallback }` — `Fallback` describes window
*selection* (pinning exhausted, using frontmost/size), never a capture failure. Also here:
`pub enum LastSuccessfulCaptureWindowId { WaitingForFirstSuccess, Available{window_id} }`
and `pub enum LatestCaptureAttemptWindowSelection { WaitingForFirstResult,
Completed(CaptureAttemptWindowSelection) }`.

`capture_test_driver.rs` holds `BackdropMonitor::with_capture_test_driver()` and the
`#[doc(hidden)]` `BackdropMonitorCaptureTestDriver` (with
`abandon_capture_attempt_after_deadline`, `disconnect_capture_worker_during_attempt`,
`send_capture_attempt`, `complete_capture_attempt`) that drives the real monitor and real
selector from cargo-tile tests. This surface ships in production builds deliberately —
`tui_pane` is a normal dependency, so a `test-support` feature would unify into the normal
build anyway.

The public `Backdrop` (`backdrop/mod.rs`) is the render-thread view: `{width, height,
colors: Vec<Color>}` with `color_at(column, row) -> Option<Color>` (refuses out-of-range
cells so drawing thins out rather than stopping during a resize).

### Capture failure → attract notice — `cargo-tile/src/attract/backdrop_notice.rs`

This reporting lives in **cargo-tile**, not `tui_pane`: the classifier and notice
rendering (`classify_backdrop_notice`, `BackdropNotice`, `draw_backdrop_notice`) sit in
cargo-tile's attract screen (`cargo-tile/src/attract/`) and consume this crate's
`BackdropStatus`/`CaptureFailure`. It is documented here because it completes the backdrop
story.

The attract screen turns backdrop state into at most one status line.

- `pub(crate) enum BackdropNotice { None, ScreenRecordingAccessInstruction, CaptureStalled,
  CaptureRecoveryStopped, CaptureUnavailable }`.
- `pub(super) const fn classify_backdrop_notice(attract_screen_visibility:
  AttractScreenVisibility, grace_period: BackdropGracePeriod, current_backdrop:
  CurrentBackdrop, backdrop_status: BackdropStatus) -> BackdropNotice` — a total, pure
  mapping over four inputs, so what the reader is told is decided in one place. Priority:
  a `Hidden` attract screen suppresses everything; while `Showing`, a
  `CaptureWorkerReplaced`/`WorkerReplacementLimitReached` status shows
  `CaptureStalled`/`CaptureRecoveryStopped` **even over a retained current backdrop**;
  otherwise a current backdrop or an unelapsed grace period suppresses the notice; once the
  grace period has elapsed with no current backdrop, `ScreenRecordingAccessNotGranted`
  gives the Settings instruction and every other failure/still-waiting/ready-but-unplaced
  state gives the neutral `CaptureUnavailable`.
- `Attract::backdrop_notice(now)` (`attract/mod.rs`) builds the four inputs — visibility
  from `showing()`, `grace_period` from `BackdropWait::{NotWaiting, WaitingSince(Instant)}`
  vs `ATTRACT_BACKDROP_GRACE`, `current_backdrop` from `monitor.current()`, status from
  `monitor.status()` — and calls the classifier every frame.
- `render.rs::draw_backdrop_notice` (called from the attract branch, on the body's last
  row) maps the notice to its string constant (`ATTRACT_NO_BACKDROP_NOTICE`,
  `ATTRACT_BACKDROP_STALLED_NOTICE`, `ATTRACT_BACKDROP_RECOVERY_STOPPED_NOTICE`,
  `ATTRACT_BACKDROP_UNAVAILABLE_NOTICE`).
- Diagnostics (same story, for the probe log not the screen): `struct BackdropDiagnostic`
  is logged **only on a discriminant transition** of its fields via
  `backdrop_diagnostic_record` (`backdrop: report=… capture_status=… captured_window_id=…
  latest_attempt_window_selection=…`); `note_backdrop_attempts` writes one
  `backdrop_attempt:` line per drained `CompletedCaptureAttemptDiagnostic`. Both are inert
  unless `CARGO_TILE_FRAME_LOG` is set (and each process truncates that file on first
  write, so two instances need distinct paths).

### Moving band — `tui_pane/src/backdrop/band.rs`

`TravelingBand::render(area, backdrop, ground, buffer)` paints a desktop-derived
background into **every cell** the backdrop has a sample for, then draws the strip's
glyphs over that field (no cell left at the flat pane background). Geometric coverage and
ink strength are separate questions: `coverage(column, row) -> Option<u8>` is the cell's
sub-cell share of the strip; the private `glyph_strength(column, row, strip_coverage)`
ramps both strip boundaries across `BAND_EDGE_FALLOFF_CELLS` and caps by coverage, so
travel stays smooth while edges fade into the desktop. Background uses
`BAND_BEHIND_FADE = 64` (keeps three-quarters of the sampled desktop color, so
neighbouring cells stay distinct); all colors go through `theme::blend_color`. The
renderer takes no second capture and allocates nothing per frame. `AttractMode::draw`
indexes `AttractMode::ALL` directly, so a mode added to `ALL` is drawable by construction.

---

## Invariants

1. **Point-size capture contract.** The stream is configured with `output_size` in display
   **points**, so ScreenCaptureKit returns a frame that is one pixel per point; the same
   cell measured in points (`metrics.cell_points(backing_scale)`) reduces the image and
   places the window. Do not mix native pixels into the reduce grid — the cell size is
   deliberately kept out of reach of the (fallible) window match so a wrong match cannot
   carry its error into every cell.
2. **Last-good-frame caching.** `DisplayStream.last` keeps the newest good frame;
   ScreenCaptureKit only delivers on change (or at the capped rate), so a static desktop
   returns no new frame and the cached one stands in — **every attempt over a static
   desktop still reports `Ready`**. A capture failure never removes a desktop already on
   screen; a later success clears a stored failure.
3. **`BackdropStatus` describes the newest attempt, not availability.** A monitor whose
   status is `Failed` still renders its retained `current` backdrop, so a notice keyed on
   status alone is wrong while content is on screen — hence the `CurrentBackdrop` input to
   `classify_backdrop_notice`.
4. **Terminal-owned windows are excluded from the capture.** The content filter leaves out
   every window owned by the chosen window's pid (or just the chosen window when the owner
   is `Unnamed`), so what comes back is the desktop the terminal is drawn over, not the
   terminal.
5. **Cross-process open lock.** Every stream open — in this process or any other capturing
   instance of the user — serializes through the `$TMPDIR` advisory lock; the once-only
   window-server work is what wedges when opens overlap, and the lock is the thing that
   keeps them from overlapping. A lock that cannot be acquired proceeds unserialized rather
   than failing.
6. **The window-server call runs once, at open; refreshes never call it.** A per-frame
   `SCShareableContent::get` (~70ms) is prohibited; steady-state refresh is a lock-free pop
   of an already-delivered frame. Per-window geometry/title questions use CoreGraphics
   by-id (~hundreds of µs), never ScreenCaptureKit.
7. **`ScreenRecordingAccessNotGranted` means "not granted", never "refused".**
   `CGPreflightScreenCaptureAccess` answers `false` identically for a process never asked
   and one refused, so the notice **instructs, never accuses**. `SCShareableContent::get`
   (at stream open) is what raises the permission prompt; the preflight only classifies.
8. **`WindowIdentification::Fallback` is a selection outcome, not a capture failure** — it
   must never select a capture-failure notice. The permission notice selects only on
   `CaptureFailure::ScreenRecordingAccessNotGranted`.
9. **A capture failure/attempt diagnostic never retains an `Arc<Desktop>`.** The worker
   result is split on receipt (`into_diagnostic_and_desktop_result`); the retained
   `completed_attempts` deque is bounded (drop-oldest) so it cannot grow without limit.
10. **Attract per-frame budget is `ATTRACT_FRAME_INTERVAL = 33ms`.** Full-area band
    painting, footer rendering, and currency marks all fit inside it: no second capture, no
    reduction, no per-frame allocation, no per-frame `String`. Work done on load, on
    overlay open, or on resize is not bound by it.
11. **Non-empty / non-contradictory-by-construction domain types.** The window
    search is one `WindowIdentificationState` with two projections, not five loose fields
    that could disagree; `CaptureWindowTarget` replaced a bare `Option<u32>` capture
    target; `TerminalWindowSearchOutcome` replaced a bare `Option<u32>` lookup result. The
    capture *target* and the capture *record* (`CaptureAttemptWindowSelection`) stay
    separate — an attempt asked for `TerminalWindowHeuristic` may still report
    `Selected{method: ClosestSizeMatch}`, and one asked for `PreferWindow` may report
    `SelectionNotReached`.
12. **Theme content belongs to the app, not the framework.** No new color is hardcoded in
    `tui_pane`; band/text/pixel rendering goes through `theme::blend_color` and the theme
    accessors.
13. **`tui_pane` must not leak backdrop symbols when the feature is off**, and the whole
    `backdrop` tree must compile on Linux — variant *construction*, not just definition,
    has to be target-independent (see gotchas).

---

## Calibration / gotchas

**Stream constants (`backdrop/desktop/platform/macos/stream.rs`).**
- `OPEN_LOCK_FILE = "tui_pane-desktop-capture-open.lock"` (joined onto `$TMPDIR`).
- `OPEN_DEADLINE = 8s` — bounds one open (shareable-content read + start confirmation)
  under the open lock. Deliberately below twice the monitor's 5s `CAPTURE_ATTEMPT_DEADLINE`,
  so a slow first open costs at most one tolerated stall, not a worker replacement.
- `OPEN_POLL_INTERVAL = 100ms` — park between open-future polls (bounds staleness of the
  deadline / stream-stopped checks).
- `FIRST_FRAME_DEADLINE = 3s`, `FRAME_POLL_INTERVAL = 25ms` — a freshly opened stream is
  polled up to 3s for its first frame (the cross-process *open* lock is already released by
  then, though the process-local registry mutex is held for the whole call); later pulls
  over a static desktop return nothing and fall back to `last`.
- `STREAM_BUFFER_CAPACITY = 2` (newest-wins), `STREAM_QUEUE_DEPTH = 3`, `STREAM_MAX_FPS = 2`
  — fine at the backdrop's ~1Hz refresh.
- `BYTES_PER_PIXEL = 4` (BGRA in; converted to RGBA by `bgra_rows_to_rgba`, which drops
  any per-row `CoreVideo` padding beyond `width*4`).

**Monitor constants (`backdrop/constants.rs`).**
- `CAPTURE_ATTEMPT_DEADLINE = 5s`, `MAX_CAPTURE_WORKER_REPLACEMENTS = 3`,
  `MAX_RETAINED_CAPTURE_ATTEMPT_DIAGNOSTICS = 64`.
- `CAPTURE_REFRESH = 1000ms` (routine cadence), `CAPTURE_RETRY = 150ms` (a still-unusable
  capture retries sooner than the routine cycle, but not every frame — each attempt costs
  the worker the same long round trip).
- `IDENTIFY_PASSES = 10`, `IDENTIFY_RETRY = 500ms`, `IDENTIFY_MARKER = "tui-pane-window-"`.
- `SAMPLES_PER_CELL = 4`, `EMULATOR_NAME_FLOOR = 5`, `POSITION_TOLERANCE = 200.0`.

**Attract / notice constants (`cargo-tile/src/constants.rs`).**
- `ATTRACT_FRAME_INTERVAL = 33ms`, `PROBE_THRESHOLD = 33ms`, `ATTRACT_BACKDROP_GRACE = 10s`
  (a missing backdrop is silent until 10s elapses, except a replaced-worker stall which is
  reported ahead of both grace and current-backdrop suppression).

**Band constants.** `BAND_BEHIND_FADE = 64` (not an alias of `TEXT_BEHIND_FADE =
128`; the band has no per-cell ink outside its strip, so a halfway blend washes out the
variation the background exists to show). `BAND_EDGE_FALLOFF_CELLS = 3` (guarded by a
compile-time assert; the ramp is continuous only because `glyph_strength` takes the
minimum of the two boundary distances).

**Gotchas.**
- **The wedged capture thread leaks by design.** `doom-fish-utils`' `SyncCompletion`
  exposes only `wait(self)` with no timeout, so a ScreenCaptureKit call that never returns
  cannot be bounded at the call site — the bound lives in the monitor (deadline + replace),
  and the abandoned thread is unrecoverable.
- **`draw_backdrop_notice` runs every frame from `render.rs` regardless of whether the
  attract screen is up.** Any notice arm placed ahead of the grace-period arms must test
  `AttractScreenVisibility` itself, or it paints across the user's working panes — which is
  why visibility is the first input to `classify_backdrop_notice`.
- **Linux dead-code gate.** `backdrop/**` compiles only under the `backdrop` feature, and
  anything reachable solely from the `#[cfg(target_os = "macos")]` module is dead code on
  Linux, where CI denies it. Moving a type out of the macOS module for testing is exactly
  what trips it; the check is `cargo clippy --target x86_64-unknown-linux-gnu -p tui_pane
  --all-features -- -D warnings`. Variant *construction* (not just definition) must be
  target-independent — `TerminalWindowSearchOutcome::Found` and `WindowTitle` variants
  carry `#[cfg_attr(not(macos), expect(dead_code))]` for this reason. macOS reports nothing
  and no `verify.sh` line catches it.
- **`verify.sh` cannot see backdrop code.** `tui_pane`'s `default = ["clipboard"]` and
  `verify.sh` composes no `--features`, so `check|test|lint tui_pane` build with backdrop
  off and never run its unit tests. Only `cargo-tile` enables the feature; behavior that
  must actually execute needs a cargo-tile-side test driving the framework API, or `cargo
  test -p tui_pane --features backdrop`.
- **`CARGO_TILE_FRAME_LOG` is off by default** and each process truncates the file on its
  first write — two instances sharing one path erase each other. The neutral
  `CaptureUnavailable` line names this variable rather than promising a recording.

---

## Why

- **Persistent multi-client SCStream instead of per-refresh screenshots.** A fresh
  in-process screenshot each refresh issues a window-server composite call every time;
  when several cargo-tile instances overlap on the window server (macOS 26), those calls
  wedge process-to-process — an instance's capture confirms its start but never delivers.
  A ScreenCaptureKit session is multi-client, so one persistent stream per display lets
  every process sample the shared display independently, and the one wedge-prone call
  (`SCShareableContent::get` at open) is paid once, serialized across processes by the
  advisory lock, and bounded by the park-timeout driver so it can never hang the worker.
  Steady-state refresh becomes a lock-free frame pop with zero window-server calls.
- **Point-size capture.** Asking the stream for the display's point dimensions makes the
  returned image one pixel per point, so a single cell measured in points reduces the image
  and places the window — no second unit, and the cell size stays independent of the
  fallible window match.
- **A failure-staging enum (`CaptureFailure`).** Callers keep drawing the last good
  desktop while surfacing *why* the newest attempt failed; a `Copy`, field-less enum is
  cheap to send from the worker and retain as state, and it lets the notice classifier and
  the probe log tell one precise story (permission vs wedge vs no-window vs reduce) instead
  of a generic "capture failed".
- **A four-input, total notice classifier.** What the reader is told about capture is a
  pure function of visibility, grace, current-backdrop presence, and status — settled in
  one place, exhaustively tested, and impossible to contradict frame-to-frame. Visibility
  is first so the same code path that runs on every frame never paints over working panes.
