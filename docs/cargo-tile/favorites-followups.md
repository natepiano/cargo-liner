# cargo-tile favorites — follow-up work

> **Status: IMPLEMENTATION PLAN — phased, delegate-ready.** Repairs and completions that came out of the shipped favorites feature: desktop capture that reports why it failed, an attract band that shows the desktop, an overlay whose rows can be selected and deleted for what they are, and documentation that matches the code.

## Delegation Context

- **Project:** `cargo-liner` workspace, `members = ["crates/*"]`, resolver 3. `cargo-tile` — a terminal-UI cargo tool whose idle attract screen draws desktop-derived animations and persists steerable parameter favorites. `tui_pane` — the reusable ratatui pane framework (keymap, status bar, overlays, toasts, theme machinery, and the optional `backdrop` desktop-capture and attract renderers) shared by `cargo-tile` and `cargo-port`.
- **Project started:** 2026-08-28T07:32:52.840+00:00
- **Stack:** Rust, edition 2024. Workspace-pinned: `ratatui 0.30.2`, `crossterm 0.29.0`, `toml 1.1.4`, `screencapturekit 8.0.1` (features `["macos_14_0"]`, optional, macOS-only), `objc2-core-graphics 0.3.2` (optional, macOS-only, features `CGDirectDisplay`/`CGGeometry`/`CGWindow`/`std`), `objc2-core-foundation 0.3.2`, `serde 1`, `unicode-width 0.2.2`, `uuid 1`, `chrono 0.4.45`, `tempfile 3.27.0` (dev). `tui_pane` also carries `crossbeam-channel`, `tokio`, `thiserror`, `tracing`, `dark-light`, `dirs`, `sysinfo`. rustfmt: `max_width = 100`, `imports_granularity = "Item"`, `group_imports = "StdExternalCrate"`, `wrap_comments = true`, `struct_field_align_threshold = 50`.
- **Layout:**
  - `crates/tui_pane/src/backdrop/` — `mod`, `desktop`, `monitor`, `band`, `text`, `pixels`, `query`, `constants`, `random`, entirely behind `#[cfg(feature = "backdrop")]` (`lib.rs:9`).
  - `crates/tui_pane/src/toasts/` — 16 top-level files plus `render/{mod,card,drawing,format,layout}.rs`.
  - `crates/tui_pane/src/theme/` — machinery only, plus `theme/testdata/`.
  - `crates/cargo-tile/src/` — flat modules (`favorites.rs`, `favorites_overlay.rs`, `globals.rs`, `constants.rs`, `terminal.rs`, `render.rs`, `app.rs`, `keymap.rs`, `config.rs`) plus `attract/{mod,moving_band,moving_text,pixelate,held_key}.rs` and `theme/{mod,builtins}.rs`; `crates/cargo-tile/themes/` holds shipped theme `.toml`s.
  - `crates/cargo-port/src/tui/app/{mod,constants}.rs` — the only cargo-port files this plan touches.
  - Neither `tui_pane` nor `cargo-tile` has an `examples/` directory; no `required-features` anywhere.
- **Key files:**
  - `crates/tui_pane/src/backdrop/mod.rs` — `Backdrop` (per-cell desktop color), public re-export surface for every backdrop type including `CaptureFailure` and `BackdropStatus`; 209 lines, tests at 145.
  - `crates/tui_pane/src/backdrop/desktop.rs` — `Desktop` capture. `CaptureFailure` names the stage that failed (`:90`) and carries the two classification helpers every former swallow site now uses; `capture` returns `Result<Desktop, CaptureFailure>` (`:468`). `SCShareableContent::get()` runs first and `shareable_content_failure` (`:447`, called `:473`) classifies a failed query from `screen_capture_access_is_granted` (`:443`). Pinned-id resolution falls back to frontmost, then size (`:491`–494); the id it settles on stays local to that function (`:496`). Exclusion list deduplicated by window id through `deduplicate_windows_by_id` (`:456`, called `:534`). `reduce_capture` (`:890`, called `:567`). 1632 lines, tests at 1370 (inside `mod platform`) and 1519.
  - `crates/tui_pane/src/backdrop/monitor.rs` — `BackdropMonitor` (`:90`–145): per-instance channels and workers, `pinned: Option<u32>` (`:118`), last successful desktop (`:101`) and latest attempt status (`:103`) held separately, `status()` accessor (`:414`). `Request::window: Option<u32>` (`:154`) carries the window to capture behind. `identify() -> bool` (`:250`) returns `false` alike for exhausted attempts (`:254`), a merely-paced retry (`:261`–265), and a failed marker-title write (`:299`); the exhaustion branch (`:313`–322) restores the title and leaves `pinned` unset. The worker forwards both capture outcomes. 459 lines, no test module.
  - `crates/tui_pane/src/backdrop/band.rs` — `TravelingBand`; cells outside the strip untouched (`:603`); one-cell `coverage` edge fade (`:615`); `backdrop.color_at` per covered cell (`:622`); background blend via `BAND_BEHIND_FADE` (`:643`). 1901 lines, tests at 656.
  - `crates/tui_pane/src/backdrop/text.rs` — `DriftingText`; paints every cell from the backdrop (`:552`, blend `:572`) — the reference composition for the band change. 1939 lines, tests at 1036.
  - `crates/tui_pane/src/backdrop/pixels.rs` — `ResolvingPixels`; also paints every cell, `PIXEL_BEHIND_FADE`. 1474 lines, tests at 909.
  - `crates/tui_pane/src/backdrop/query.rs` — xterm position query pinning which emulator window this process draws in. 309 lines, tests at 230.
  - `crates/tui_pane/src/backdrop/constants.rs` — `TEXT_BEHIND_FADE: u8 = 128` (`:269`), `BAND_BEHIND_FADE = TEXT_BEHIND_FADE` (`:23`), `PIXEL_BEHIND_FADE` (`:194`), `CHURN_CELLS_PER_FRAME` (`:27`), `DEFAULT_BAND_SPEED` (`:30`). 532 lines.
  - `crates/tui_pane/src/toasts/toast.rs` — `Toast`, `ToastPhase`, `created_at` (`:107`), `min_height()` (`:221`), `current_visible_lines` = `floor(elapsed/line_ms)+1` clamped up to `min_height` (`:223`–238), `target_height` (`:252`), exit arithmetic (`:245`). 305 lines.
  - `crates/tui_pane/src/toasts/manager.rs` — `Toasts`, `ToastSpec`, `ToastCommand`, `active_now()`; owns push and wrapping. 584 lines, tests at 107.
  - `crates/tui_pane/src/toasts/settings.rs` — `ToastSettings`, `animation.entrance_duration`/`exit_duration`, `ToastDuration`, `ToastPlacement`. 376 lines.
  - `crates/tui_pane/src/toasts/mod.rs` — 44 lines, the module's export list.
  - `crates/tui_pane/src/toasts/{lifecycle,body,view,slots}.rs`, `toasts/render/*` — phase transitions and expiry, wrapping width, hitboxes, slot layout, card drawing.
  - `crates/tui_pane/src/keymap/global_action.rs` — `GlobalAction::Dismiss` default bound to `'x'` (`:70`, `:241`), help text (`:122`).
  - `crates/tui_pane/src/lib.rs` — `mod backdrop` and every backdrop re-export gated `#[cfg(feature = "backdrop")]` (`:9`–62).
  - `crates/cargo-tile/src/favorites.rs` — model, parser, lock, writer. `UnrecognizedFavoriteValue` (`:160`), recognitions sorted independently of raw `tables` (`:244`), duplicate-id row demoted carrying its id (`:250`), `push` settings-match dedup (`:264`), `remove(FavoriteId)` (`:296`, `:510`) re-reading under lock (`:596`), `#[cfg(test)] parse_rows_for_overlay_test` (`:516`), exhaustive `AttractSettings` constructors (`:762`). **1039 non-test lines**; `#[cfg(test)]` at 1040.
  - `crates/cargo-tile/src/favorites_overlay.rs` — `column_descriptors` (`:348`), `FavoritesSurfaceBindings::footer` unconditional move/load/delete (`:526`), `finish_navigation` indexing blanks and headings (`:616`), `FavoritesOverlay` state (`:683`), `open` calling `favorites::load` (`:713`), `schedule_timed_toast` call (`:1180`), `FavoriteSelection` (`:1314`), two-cell marker prefix in `rendered_line` (`:1323`), `build_line_plan` (`:1375`), `Attract:` section heading (`:1455`), `append_unrecognized` emitting `CachedOverlayLine::Static` (`:1508`), `favorite_cells` (`:1728`), private `mode_label` (`:1755`). **1809 non-test lines**; `#[cfg(test)]` at 1810.
  - `crates/cargo-tile/src/globals.rs` — app-globals scope; `schedule_timed_toast` pairings (`:155`, `:230`); private `mode_label` (`:239`). 746 lines, tests at 247.
  - `crates/cargo-tile/src/constants.rs` — `ATTRACT_NO_BACKDROP_NOTICE` (`:39`), `ATTRACT_FRAME_INTERVAL` (`:346`), `PROBE_THRESHOLD` (`:354`). 810 lines, no test module.
  - `crates/cargo-tile/src/terminal.rs` — input dispatch ladder; `ToastVisualTimeline` (`:105`, impl `:120`), `ToastVisualSchedule` (`:204`, `record` `:234`), overlay consumes every key (`:720`), `keyed_mode` caller (`:744`), `Dismiss` dispatch (`:801`, `:821`). 1146 lines, tests at 859 (schedule tests `:971`, `:1066`, `:1114`).
  - `crates/cargo-tile/src/app.rs` — `App` holds `toast_visual_schedule` (`:133`, init `:188`), `schedule_timed_toast` (`:217`), imports (`:30`).
  - `crates/cargo-tile/src/attract/mod.rs` — `AttractSettings`, `current_settings()` (`:697`), `AttractMode::draw` index-then-match (`:427`), `noted_backdrop: BackdropDiagnostic` written on transition in `identify` (`:964`), `keyed_mode` (`:680`), `backdrop_notice(now) -> BackdropNotice` (`:1227`), `render` passing one `Backdrop` to all three renderers (`:1251`), automatic-attract steering regression test in the test module. 2120 lines, tests at 1280.
  - `crates/cargo-tile/src/attract/{moving_band,moving_text,pixelate,held_key}.rs` — key bindings only; none of them render.
  - `crates/cargo-tile/src/render.rs` — the pane background the band currently falls back to; `draw_backdrop_notice` (`:216`) writes the attract notice on the body's last row, called from the attract branch (`:184`). 3178 lines.
  - `crates/cargo-tile/src/keymap.rs` — keymap assembly; `x` dismiss arrives from `GlobalAction` defaults.
  - `crates/cargo-tile/src/config.rs` — `<os config dir>/cargo-tile/` paths.
  - `crates/cargo-tile/README.md` — 475 lines; `AppGlobalAction` "starts with no variants" (`:70`–73), `## configuration` "Three files" (`:75`–78), `### keys` (`:430`), attract-keys claim (`:465`–469).
  - `crates/cargo-port/src/tui/app/mod.rs` — `animation_timeout` (`:452`), `is_animating` (`:465`) whose last clause `!self.framework.toasts.active_now().is_empty()` (`:470`) keeps the 80ms heartbeat alive for a toast's whole lifetime; `ANIMATION_TICK` import (`:125`).
  - `crates/cargo-port/src/tui/app/constants.rs` — `ANIMATION_TICK: Duration = Duration::from_millis(80)` (`:3`).
- **Build:** `bash ~/.claude/scripts/delegate/verify.sh check <pkg>`
- **Test:** `bash ~/.claude/scripts/delegate/verify.sh test <pkg>`
- **Lint:** `bash ~/.claude/scripts/delegate/verify.sh lint <pkg>`
- **Style:** `run-end /clippy style-only auto-proceed`
- **Invariants:**
  1. **`verify.sh` cannot compile `tui_pane`'s backdrop code.** `tui_pane`'s `default = ["clipboard"]`; `backdrop` is opt-in and `verify.sh` composes no `--features`. So `check|test|lint tui_pane` build with backdrop **off** and never see `backdrop/**` or run its `#[cfg(test)]` modules. Only `cargo-tile` enables the feature (`crates/cargo-tile/Cargo.toml:31`), so **every backdrop phase gates through `check cargo-tile` and `lint cargo-tile`**, and any behavior that must actually *run* per phase needs a cargo-tile-side test driving the framework API. New `tui_pane` unit tests under `backdrop/` compile but do not execute until the final workspace gate — a phase adding them says so and names them.
  2. **`cargo-port` does not enable `backdrop`.** It takes default features. A backdrop change must prove (a) `tui_pane` still builds and tests green with the feature off — no `use` or re-export may leak outside `#[cfg(feature = "backdrop")]` — and (b) `cargo-port` still checks and tests. It must **not** claim cargo-port exercises the capture path; cargo-port cannot reach it.
  3. **Attract cadence is `ATTRACT_FRAME_INTERVAL = 33ms`** (`cargo-tile/src/constants.rs:346`). Every per-frame path — full-area band painting, footer rendering, currency marks — fits inside it: no second capture, no reduction, no per-frame allocation. Work done on load, on overlay open, or on resize is not bound by it.
  4. **Workspace lints bind both crates** (`[lints] workspace = true`). `clippy::all`/`cargo`/`nursery`/`pedantic` denied at priority -1, plus `unwrap_used`, `expect_used`, `panic`, `unreachable`, `allow_attributes_without_reason`, `undocumented_unsafe_blocks`, `self_named_module_files`. `missing_docs = "deny"` and `unsafe_code = "deny"` in `[workspace.lints.rust]`. Consequences: a module with submodules is `foo/mod.rs`, never `foo.rs` beside `foo/`; every new public and module item carries a doc comment; an FFI call opts back into `unsafe` with a reasoned `#[expect]`. Test modules opt back in with `#[expect(clippy::expect_used, clippy::panic, reason = "…")]` — the pattern at `favorites.rs:1041` and `favorites_overlay.rs:1811`; moved tests carry their `#[expect]` block with them.
  5. **Themes: the framework supplies machinery, the app owns content.** `tui_pane/src/theme/` holds the registry, resolver, loader, watcher, blend helpers and exactly one fallback palette; its module doc states theme *content* belongs to the client app. No new color is hardcoded in `tui_pane` — band, text and pixel rendering go through `theme::blend_color` and the theme accessors, as they do today.
  6. **Phases do not bump versions.** `cargo-tile` is `0.2.70-dev`, `tui_pane` and `cargo-port` `0.8.0-dev`. One patch bump plus `cargo install --path crates/cargo-tile` happens at the final gate, not per phase.
  7. **Do not edit `tui_pane = { path = "crates/tui_pane" }`** in the workspace manifest; the release branch rewrites that line. `.cargo/config.toml` sets `RUSTC_BOOTSTRAP = "1"` and the macOS rustflags ScreenCaptureKit's Swift shims need — leave both alone.
  8. **Rejected, do not reintroduce.** Renaming `parse_rows_for_overlay_test` (it is `#[cfg(test)]`; the name is accurate). Renaming either private `mode_label` (they are in separate modules; neither can affect the other). Replacing `keyed_mode() -> Option<AttractMode>` with a routing enum (`None` means no mode takes this key, which is what an option says). Giving `moving_text.rs` the band's background treatment (it already paints every cell).
  9. **Builds of these crates must run unsandboxed.** ScreenCaptureKit's build scripts invoke Swift Package Manager, which applies its own `sandbox-exec`; macOS sandboxes do not nest, so a sandboxed build fails at `sandbox_apply` with a panic that names Swift and never names the sandbox. That is an environment failure, never a code or dependency defect.

## Phases

### Phase 1 — A failed desktop capture carries the stage that failed  · status: done

#### As-built

`Desktop::capture` returns `Result<Desktop, CaptureFailure>` on both the macOS and non-macOS paths. `CaptureFailure` is a public field-less enum naming the stage that failed — unsupported platform, shareable-content query, Screen Recording access not granted, terminal window not found, display not found, screenshot, pixel extraction, image reduction — with two helpers, `classify_result` and `classify_option`, standing where every `.ok()?` used to.

`SCShareableContent::get()` runs first and `shareable_content_failure` classifies a query that has already failed, reading `screen_capture_access_is_granted` (a direct `CGPreflightScreenCaptureAccess` call). Exclusion window ids are deduplicated by `deduplicate_windows_by_id` before the `SCContentFilter` is built. `reduce_capture` folds the three reduction steps behind one seam.

`BackdropMonitor` holds the newest successful desktop (`LastSuccessfulDesktop`) separately from the newest attempt's outcome (`BackdropStatus`: waiting for a first result, ready, or failed carrying a `CaptureFailure`), and its worker channel carries `Result<Desktop, CaptureFailure>` so failures reach the monitor instead of being dropped. `status()` exposes the latest attempt; a later success clears a stored failure, and a failure never removes a desktop already on screen. `CaptureFailure` and `BackdropStatus` are re-exported from `backdrop/mod.rs` and `lib.rs` under `#[cfg(feature = "backdrop")]`.

**Files:**
- `crates/tui_pane/src/backdrop/desktop.rs` — `CaptureFailure` and its two classification helpers, the `Result` capture path, access classification, exclusion-id deduplication, `reduce_capture`
- `crates/tui_pane/src/backdrop/monitor.rs` — `BackdropStatus`, `LastSuccessfulDesktop`, the outcome-carrying worker channel, the `status()` accessor
- `crates/tui_pane/src/backdrop/mod.rs` — re-exports for both new types
- `crates/tui_pane/src/lib.rs` — feature-gated re-exports alongside the existing backdrop ones

**Binds later work:** `SCShareableContent::get()` runs before any access check, because it is the call that raises the macOS permission prompt; `CGPreflightScreenCaptureAccess` classifies only a query that already failed. That call answers `false` for a process never asked exactly as for one that refused, so the access variant means "not granted", not "the user said no" — the attract notice must instruct rather than accuse. `BackdropStatus` describes the newest attempt, not availability: a monitor whose status is failed still renders its last good desktop, so a notice keyed on status alone is wrong while content is on screen. The window id capture actually selects is resolved inside `Desktop::capture` and never returned, which is what the second-window reproduction needs and cannot yet read.

**Gotchas:**
- `CGPreflightScreenCaptureAccess` never prompts and cannot distinguish a refusal from a process that has never been asked.
- In `objc2-core-graphics` 0.3.2 that call is declared safe; wrapping it in `unsafe` trips the workspace lints rather than satisfying them.
- Only two of the eight classification sites can be reached without a window server. The rest are proven by the shape of the code, not by a test.
- Per Invariant 1 the new `desktop.rs` tests compile but do not execute until the final workspace gate: `failed_shareable_content_query_with_access_reports_query_failure`, `failed_shareable_content_query_without_access_reports_permission_denial`, `image_reduction_rejects_a_cell_too_large_for_the_image`, `image_reduction_returns_the_implied_grid_and_colors`, and the retained `exclusion_windows_are_deduplicated_by_id_in_original_order`.

**Ruled out:**
- A permission preflight *before* the capture attempt — it removes the only call that ever asks the user for access, stranding a process that has never been asked.
- An `#[expect(unsafe_code)]` block and `// SAFETY:` comment around the preflight call — the function is safe.
- Tests asserting the generic `classify_*` helpers rather than the production classification path; they restate their own argument.

### Phase 2 — Window identification says whether it is still trying  · status: done

#### As-built

`BackdropMonitor::identify` (`monitor.rs:288`) returns `WindowIdentification` (`monitor.rs:52`) rather than a bare boolean: `NotAttempted`, `Pending`, `Identified { window_id: u32 }`, or `Fallback`. `Fallback` means pinning is exhausted and capture is using frontmost-or-size selection — it describes window *selection*, not capture, and must never be read or reported as a capture failure.

The report is decided by a pure `const fn window_identification(attempts_consumed: u32, WindowSearchOutcome) -> WindowIdentification` (`monitor.rs:475`) called at every exit of `identify`, so it depends only on attempts already spent: `0` is not attempted, below `IDENTIFY_PASSES` is pending, at or above it is fallback, and a found window is identified whatever the count. `identify` increments before the marker-title write, so a failed write on the final allowance reports `Fallback` on that same call rather than a frame later. `WindowSearchOutcome` (`monitor.rs:80`) — `NotFound` or `Found { window_id: u32 }` — is private to the module.

A successful capture reports the window id it used. `BackdropMonitor::captured_window_id()` (`monitor.rs:460`) returns `LastSuccessfulCaptureWindowId` (`monitor.rs:70`) — `WaitingForFirstSuccess` or `Available { window_id: u32 }` — reading the retained last successful desktop, so a later capture failure does not discard the id. `Desktop::window_id()` (`desktop.rs:208`) stays `pub(super) -> u32`. Both public types are re-exported through `backdrop/mod.rs` and `lib.rs` under `#[cfg(feature = "backdrop")]`.

`CaptureFailure::ScreenRecordingAccessNotGranted` (`desktop.rs:40`) replaces `PermissionDenied`; its doc comment states that the access check answers alike for a process that has never been asked and one that refused. No `PermissionDenied` remains in the workspace. `BackdropStatus` keeps its name — it is read as `monitor.status()`, where the receiver already says what it is the status of.

cargo-tile's `Attract` holds `window_identification: WindowIdentification` (`attract/mod.rs:463`, initialized `NotAttempted` at `:512`) and writes one probe line only when the report changes (`:892`–897), because that path runs every 33ms.

**Files:**
- `crates/tui_pane/src/backdrop/monitor.rs` — both public report types, the private search outcome, the pure mapping and its five tests, `identify`, `captured_window_id`
- `crates/tui_pane/src/backdrop/desktop.rs` — the renamed access variant; `Desktop` holds and exposes the window id its capture used
- `crates/tui_pane/src/backdrop/mod.rs`, `crates/tui_pane/src/lib.rs` — feature-gated re-exports of both public types
- `crates/cargo-tile/src/attract/mod.rs` — the named identification field and its transition-only probe line

**Binds later work:** The window id a capture used is reached only through `BackdropMonitor::captured_window_id()`, which answers `WaitingForFirstSuccess` or `Available { window_id: u32 }` — a consumer that logs or reports the id renders both, and a window that never captures reports the waiting state, which is itself diagnostic evidence. `WindowIdentification::Fallback` is a selection outcome and must not select a capture-failure notice. The permission notice selects on `CaptureFailure::ScreenRecordingAccessNotGranted`. The attract probe already notes the identification report on transition, so a consumer adding capture diagnostics extends that site rather than adding a second transition check. `BackdropMonitor::pinned`, `Request::window`, and `Desktop::capture`'s `pinned` parameter are all still `Option<u32>`, deliberately left for the phase that replaces them with one named selection type.

**Gotchas:**
- `verify.sh test tui_pane` never compiles `backdrop/**` (the feature is opt-in), and `verify.sh test cargo-tile` only *compiles* the mapping tests. They execute only under a backdrop-enabled `tui_pane` test run — name them at the final workspace gate.
- `WindowIdentification::NotAttempted` is unreachable from `identify` itself: every path returns `Found` or has spent at least one attempt. It exists for the cargo-tile field's initial value, which is what "before any pass has run" means.
- The mapping's inputs are coarser than the call sites that reach it. Naming a test after a call site rather than after the input it passes produces silent duplicates the moment a distinguishing argument is removed.

**Ruled out:**
- Folding window identification into `BackdropStatus` — that type is about capture, this is about selection.
- Returning a bare `Option<u32>` from `captured_window_id`, or widening `Desktop::window_id()` past `pub(super)`.
- Keeping a private per-call-site pass argument in the mapping — removing it is what let the report key on spent attempts alone.

### Phase 3 — The attract notice names the real cause  · status: done

#### As-built

- `classify_backdrop_notice(BackdropGracePeriod, CurrentBackdrop, BackdropStatus) -> BackdropNotice` in `attract/mod.rs` is a pure, total mapping over grace-period elapsed state, current-backdrop presence, and the latest capture status; called every frame from the render path.
- `BackdropNotice::{None, ScreenRecordingAccessInstruction, CaptureUnavailable}` is `pub(crate)` (not private — `render.rs` is a sibling module and names the variants directly). The Settings instruction fires only on `CaptureFailure::ScreenRecordingAccessNotGranted`; every other failure, a still-waiting status, or a ready-but-unplaced capture takes the neutral `CaptureUnavailable` line; a current backdrop suppresses either notice even when the newest capture attempt failed.
- `Attract::backdrop_overdue` is gone, replaced by `Attract::backdrop_notice(now) -> BackdropNotice` (`attract/mod.rs:1227`). The bare `backdrop_missing_since: Option<Instant>` field is gone, replaced by `BackdropWait::{NotWaiting, WaitingSince(Instant)}`.
- `BackdropDiagnostic { window_identification, backdrop_status, captured_window_id }` replaces the bare `WindowIdentification` field on `Attract`. The transition probe inside `identify` (`attract/mod.rs:964`) logs `backdrop: report=… capture_status=… captured_window_id=…` whenever the discriminant of any of the three fields changes — not on every frame.
- `render.rs` draws the notice via `draw_backdrop_notice` (`:216`, called from `:184`), on the body's last row, after the animation draws.

**Files:**
- `crates/cargo-tile/src/attract/mod.rs` — the notice classifier, `BackdropWait`, `BackdropDiagnostic`, the transition probe, five tests.
- `crates/cargo-tile/src/constants.rs` — both notice strings, including `ATTRACT_BACKDROP_UNAVAILABLE_NOTICE`.
- `crates/cargo-tile/src/render.rs` — `draw_backdrop_notice`, the notice row on the body's last line.

**Binds later work:** The `backdrop:` probe line (`report=`, `capture_status=`, `captured_window_id=`, emitted only on a field transition) is the evidence the two-window capture reproduction work reads; it requires `CARGO_TILE_FRAME_LOG` set to a distinct path per process instance, since each process truncates that file on its own first write and two instances sharing one path erase each other.

**Gotchas:**
- `BackdropNotice` is `pub(crate)`, not private — cross-module callers must name the variants.
- The transition probe compares `std::mem::discriminant`, not the value: a change of instant alone is not a transition, only waiting↔not-waiting is.
- `probe::note` records nothing unless `CARGO_TILE_FRAME_LOG` is set.
- The `CaptureFailure` test list (`CAPTURE_FAILURES`) enumerates all eight variants with an exhaustive inner `match` guarding a ninth.

**Ruled out:**
- A second transition-check site for capture status — extending the existing identification probe keeps the evidence on one line instead of two.
- Promising a recording in the neutral notice's text — the frame log is off by default, so the line names the environment variable that enables it instead.

### Phase 4 — The moving band paints the desktop across the pane  · status: done

#### As-built

`TravelingBand::render` paints a desktop-derived background into every cell the backdrop has a sample for, then draws the strip's glyphs over that field — no cell is left at the flat pane background. Glyph ink is derived separately from geometric coverage by the private `TravelingBand::glyph_strength`, which ramps both strip boundaries across several cells and caps the result by the cell's own sub-cell coverage, so travel stays smooth while the edges fade into the desktop. `BAND_BEHIND_FADE` is a standalone `64` rather than an alias of `TEXT_BEHIND_FADE`, keeping three quarters of the sampled desktop colour so neighbouring cells stay visibly distinct across the painted field. `BAND_EDGE_FALLOFF_CELLS` is `3`, guarded by a compile-time assertion because the falloff divides by it. Colours still go through `theme::blend_color` and the theme accessors; the renderer takes no second capture and allocates nothing per frame. Three tests over a multicolour synthetic backdrop hold the contract: `adjacent_covered_cells_keep_their_own_desktop_backgrounds`, `cells_outside_the_strip_are_painted_from_the_desktop`, and `both_strip_edges_fade_glyph_ink_across_multiple_cells`.

**Files:**
- `crates/tui_pane/src/backdrop/band.rs` — the strip renderer, its coverage and ink-strength geometry, and the band tests.
- `crates/tui_pane/src/backdrop/constants.rs` — the band's own background fade and edge falloff, each documenting why it differs from the text field's.
- `crates/tui_pane/src/backdrop/text.rs` — module and render docs, which no longer distinguish the two animations by whether cells are painted; the distinction is now where the glyphs go.

**Gotchas:** `coverage` and `glyph_strength` answer different questions and must stay separate — coverage is the cell's geometric share of the strip and carries sub-cell travel, ink strength is the boundary ramp; collapsing them back into one value reintroduces whole-cell stepping. The ramp is continuous only because `glyph_strength` takes the minimum of the two boundary distances, and its `else { depth }` fallback is reached only where the leading-edge distance already governs that minimum. `backdrop/**` sits behind the opt-in `backdrop` feature, so `verify.sh test tui_pane` compiles none of it — every change here also needs `cargo test -p tui_pane --features backdrop`.

**Ruled out:** Aliasing `BAND_BEHIND_FADE` to `TEXT_BEHIND_FADE` — the text field restores desktop colour through per-cell ink and the band has none outside its strip, so the halfway blend washed out the variation the background exists to show. An abrupt ink loss for cells straddling the ring wrap — an exhaustive sweep of the falloff arithmetic shows no mostly-covered cell receives zero ink and no step between neighbouring covered cells exceeds the designed 85/255 three-cell ramp.

### Phase 5 — The toast owns its next visual-change deadline  · status: done

#### As-built

`Toasts::next_visual_change_deadline(now) -> ToastVisualDeadline` answers, over all active toasts, the earliest instant any of them can next look different: entrance and exit line-height boundaries, lifetime expiry, the per-second countdown, spinner frames, the elapsed readout, the per-item and whole-card linger fades, and tracked-item removal. `ToastVisualDeadline::{NoVisualChangeScheduled, At(Instant)}` is public and exported at the crate root; a reported deadline is always strictly in the future, and the aggregate is floored at `constants::FRAME_POLL_MILLIS` (8ms). A toast's entrance is `ToastEntranceSchedule::{Absent, Scheduled { starts_at, ends_at }}` and its phase is `ToastPhase::{Entering { starts_at, ends_at }, Static, Exiting { started_at }}` — both crate-internal, and neither is a bare `Option<Instant>`, so "no entrance" and "an entrance" cannot collapse into one answer. The first entrance change lands at `created_at + min_height * entrance_line_ms`, one interval later than a naive reading. `cargo-tile` consumes the framework deadline and holds no scheduling arithmetic of its own; `cargo-port`'s `animation_timeout` takes the deadline as a minimum against its 80ms tick, and `is_animating` no longer reports true merely because a toast exists.

**Files:**
- `crates/tui_pane/src/toasts/manager.rs` — `ToastVisualDeadline`, the aggregation and its 8ms floor, `set_settings` refreshing both entrance schedules and `item_linger`
- `crates/tui_pane/src/toasts/toast.rs` — the phase model and every per-toast deadline helper
- `crates/tui_pane/src/toasts/lifecycle.rs` — mutation entry points refresh the entrance schedule; `prune` handles all three phases
- `crates/tui_pane/src/activity.rs` — `FrameCycle::next_frame_boundary`, the spinner's exact next-frame instant
- `crates/tui_pane/src/toasts/render/format.rs` — `fade_level`, shared between rendering and deadline scheduling
- `crates/cargo-tile/src/terminal.rs`, `app.rs`, `globals.rs`, `favorites_overlay.rs` — the local scheduling duplicates removed
- `crates/cargo-port/src/tui/app/mod.rs` — `animation_timeout` consumes the deadline

**Binds later work:** `globals.rs` now pushes with `app.framework.toasts.push_timed(...)` and no paired scheduling call, so the save-confirmation toast is a push and nothing more.

**Gotchas:**
- Anything a toast renders from `now` must have a deadline, or it silently degrades to the consumer's idle heartbeat instead of failing.
- Exactness is bounded below by the render cadence: `format_elapsed` shows whole milliseconds under ten seconds, so an exact elapsed deadline asks for ~1000 repaints a second. The floor belongs on the aggregate, not on individual boundaries.
- `prune_tracked_items` reads live settings while the scheduler reads the toast's stored `item_linger`; `set_settings` must keep the two equal.
- The linger-fade boundary search depends on `fade_level` being monotonic in its argument.

**Ruled out:**
- `const` on `Framework::set_toast_settings` and `Toasts::set_settings` — both now do per-toast instant arithmetic and text wrapping, and no caller needs a const context.
- Coarsening `format_elapsed`'s rendered precision, or flooring the repaint rate in the consumer, to fix the millisecond wake — the framework advertised a cadence no display can use, so the framework bounds it.

### Phase 6 — Split `favorites.rs` by ownership  · status: todo

#### Work Order

**Goal:** `favorites.rs` becomes a module whose parts are named for what they own, with no behavior change.

**Spec:**

1,039 non-test lines (`#[cfg(test)]` at 1040) carry the settings and row model, recognition and sorting, file-state and mutation entry points, retry and error reporting, locking, reading, serialization and atomic replacement. That is more top-level clusters than the overlay has, and Phase 7 adds the delicate locator work to it.

Move only. No signature changes, no behavior changes, no renames beyond what the move requires:

- `crates/cargo-tile/src/favorites/rows.rs` — `AttractSettings`, `Favorite`, `FavoriteId`, raw tables, `recognize_favorite`, `FavoriteRowRecognition`, `UnrecognizedFavoriteValue`, `refresh_recognitions`, `compare_recognitions`, serialization, `FavoriteRows` and its mutations including `push` (`:264`)
- `crates/cargo-tile/src/favorites/file.rs` — `FavoritesLocation`, file states, `load`, `read_rows`, `push`/`remove` entry points (`:296`, `:510`), the lock, read-modify-write, atomic replacement (`:596`)
- `crates/cargo-tile/src/favorites/mod.rs` — module declarations and the existing crate-facing exports, unchanged so no caller edits

`self_named_module_files` is denied, so this must be `favorites/mod.rs` and the old `favorites.rs` is deleted — not `favorites.rs` sitting beside a `favorites/` directory (Invariant 4).

Tests travel with the types they cover; the `#[expect(clippy::expect_used, clippy::panic, reason = "…")]` block at `:1041` is reproduced in each destination test module. `parse_rows_for_overlay_test` (`:516`) stays `#[cfg(test)]` and keeps its name — it is test-only and the name is accurate (Invariant 8); move it beside the parse code its callers reach.

**Files:**
- `crates/cargo-tile/src/favorites.rs` — deleted
- `crates/cargo-tile/src/favorites/mod.rs` — declarations and crate-facing exports
- `crates/cargo-tile/src/favorites/rows.rs` — model, recognition, sorting, serialization, row mutations
- `crates/cargo-tile/src/favorites/file.rs` — file states, entry points, lock, read-modify-write, atomic replacement

**Constraints from prior phases:** Phase 5 removed a `schedule_timed_toast` call site from `favorites_overlay.rs` but changed nothing in `favorites.rs`.

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh test cargo-tile` — every existing favorites test passes unchanged; a moved test that needed editing means the move was not a move
- `bash ~/.claude/scripts/delegate/verify.sh lint cargo-tile`
- The only paths that change are `crates/cargo-tile/src/favorites/` and the deletion of `crates/cargo-tile/src/favorites.rs`, which the move requires. No other path changes, since `mod.rs` re-exports the same names.

### Phase 7 — An unrecognized row gets a locator that survives a concurrent edit  · status: todo

#### Work Order

**Goal:** A row the parser could not read can be named precisely enough to delete, and a stale name refuses instead of deleting the wrong row.

**Spec:**

Deleting a broken favorite from the overlay is the decided behavior, and none of the obvious ways to name the row are safe:

- `UnrecognizedFavoriteValue` carries only the first failing key and its spelling (`:160` pre-split), so two malformed rows can produce byte-identical diagnostics.
- Recognitions are sorted independently of the raw `tables` vector (`:244`), so a displayed index is not a storage index.
- `remove` accepts a `FavoriteId` and re-reads the file under the lock before mutating (`:296`, `:596`), so an index captured at display time can point at a different table by the time the write happens.
- A **duplicate-id row is demoted into the unrecognized set carrying that id** (`:250`). Deleting "by its id" would delete the first *recognized* row instead. This is the failure that makes the naive implementation destructive.

Deleting an unrecognized row means removing its whole raw `[[favorite]]` table, not the one field that failed to parse.

Add `UnrecognizedFavoriteRemovalLocator`, an opaque locator minted while loading, before the display sort, carrying the raw table index plus enough of the table's own content to re-verify it identifies the same table after the locked re-read. Give removal `FavoriteRemovalTarget`, distinguishing a recognized `FavoriteId` from an unrecognized locator rather than overloading `FavoriteId`. Both names are contract surface for Phase 9; do not rename them there. When the locator no longer identifies exactly one table, refuse the removal and report it — a concurrent edit fails loudly rather than deleting the wrong row.

The locator is minted on load and checked only during deletion, so it costs nothing per frame (Invariant 3).

This phase adds the persistence surface only. The overlay does not use it until Phase 9.

**Files:**
- `crates/cargo-tile/src/favorites/rows.rs` — the locator type, minted before `refresh_recognitions` sorts, carried on the unrecognized recognition
- `crates/cargo-tile/src/favorites/file.rs` — the removal target enum, the unrecognized removal path, re-verification under the existing lock, the refusal outcome
- `crates/cargo-tile/src/favorites/mod.rs` — export the new types

**Constraints from prior phases:** Phase 6 split `favorites.rs` into `favorites/{mod,rows,file}.rs` with the crate-facing exports unchanged in `mod.rs`. The model, recognition, sorting and `push` live in `rows.rs`; the lock, read-modify-write and atomic replacement live in `file.rs`. Line references above are to the pre-split file — locate by item name.

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh test cargo-tile` with tests covering: deletion succeeding after another process inserts a row *ahead* of the target; deletion of a duplicate-id row leaving its recognized twin intact; two rows with identical diagnostics deleted one at a time; and a refusal when the file changed such that the locator no longer matches exactly one table
- `bash ~/.claude/scripts/delegate/verify.sh lint cargo-tile`

### Phase 8 — Split `favorites_overlay.rs` with its dependency direction stated  · status: todo

#### Work Order

**Goal:** The overlay becomes four modules ordered by what depends on what, with no behavior change, before the work that rewrites its rows.

**Spec:**

1,809 non-test lines followed by a single 34-test module (`#[cfg(test)]` at 1810). The clusters are real but **not peers** — splitting them as equals produces circular imports or a wave of `pub(super)`. `FavoritesOverlay` directly owns the bindings, the line plan, the width cache, the notice and the removal state (`:683`), and `build_line_plan` consumes both content and bindings (`:1375`). Split with the direction stated:

- `crates/cargo-tile/src/favorites_overlay/mod.rs` — `FavoritesOverlay`, its state and outcome types, `FavoriteRemovalCommitState`, `FavoritesOverlayNotice`, action dispatch, rendering coordination, and the re-exports. The module-anchor exception applies to the type sharing the module's name.
- `content.rs` — `FavoritesOverlayContent`, `FavoriteRowsView`, `FavoriteRowView`, `UnrecognizedFavoritesView`, row lifecycle, and the conversion-time formatting the row constructor calls (`:309`)
- `bindings.rs` — `FavoritesSurfaceBindings`, `ModeColumnBindings`, `ParameterColumnDescriptor`, `column_descriptors` (`:348`), `footer` (`:526`), the private `mode_label` (`:1755`) which keeps its name
- `line_plan.rs` — `CachedOverlayLine`, `CachedLinePlan`, `CachedSurfaceWidth`, `FavoriteSectionTableLayout`, `finish_navigation` (`:616`), `build_line_plan` (`:1375`), `rendered_line` (`:1323`), `append_unrecognized` (`:1508`), `favorite_cells` (`:1728`)

`line_plan` may depend on `content` and `bindings`; `FavoritesOverlay` may depend on all three. Nothing depends on `mod.rs`.

Move only — no signature changes, no behavior changes. `self_named_module_files` is denied, so the old `favorites_overlay.rs` is deleted (Invariant 4). Tests move down with the types they cover; cross-cutting tests stay in `mod.rs`; no `tests.rs`. Each destination test module reproduces the `#[expect(clippy::expect_used, clippy::panic, reason = "…")]` block from `:1811`. Prefer private, then `pub(super)`, only where an actual caller requires it.

**Files:**
- `crates/cargo-tile/src/favorites_overlay.rs` — deleted
- `crates/cargo-tile/src/favorites_overlay/mod.rs` — overlay state machine, dispatch, re-exports
- `crates/cargo-tile/src/favorites_overlay/content.rs` — displayed content and row views
- `crates/cargo-tile/src/favorites_overlay/bindings.rs` — binding resolution and footer construction
- `crates/cargo-tile/src/favorites_overlay/line_plan.rs` — cached lines, navigation, layout, rendering

**Constraints from prior phases:** Phase 5 removed the `schedule_timed_toast` call at `:1180`; it will not be present to move. Phase 6 moved the favorites model behind `crates/cargo-tile/src/favorites/` with unchanged crate-facing exports, so overlay imports of `favorites::…` still resolve. Phase 7 added a locator type and a removal target enum exported from `favorites/mod.rs`; this phase does not consume them.

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh test cargo-tile` — all 34 existing overlay tests pass unchanged
- `bash ~/.claude/scripts/delegate/verify.sh lint cargo-tile`
- The only paths that change are `crates/cargo-tile/src/favorites_overlay/` and the deletion of `crates/cargo-tile/src/favorites_overlay.rs`, which the move requires. No other path changes.

### Phase 9 — Rows know what they are, and the footer offers only what will work  · status: todo

#### Work Order

**Goal:** Every row the cursor can reach shows that it is selected, a broken row can be deleted, and the footer names only keys that will act.

**Spec:**

Three complaints that rewrite the same row contract — the cached line variants, the selection type, the navigation indices and the footer. Built separately they would conflict three times over, which is why they are one phase.

**Rows do not show the cursor.** Scrolling past the last recognized favorite carries the view into the unrecognized block, where presses are absorbed and no row takes the highlight. Nothing is broken underneath: `saved_count` counts recognized rows only, so the viewport bounds the selection to them, and `append_unrecognized` emits `CachedOverlayLine::Static`, which `rendered_line` returns with neither the `"▸ "` marker nor `selection_style`. Navigation is also coarser than it looks — `finish_navigation` puts *every* line after the last favorite into the index, blank lines and headings included.

**The footer advertises what the selection cannot do.** `FavoritesSurfaceBindings::footer` formats move, load and delete unconditionally; only horizontal paging is conditional.

Build them together:

- Separate the lines that are not rows — blanks and section headings — from the lines that are. A row carries `FavoriteRowIdentity::{Recognized(FavoriteId), Unrecognized(UnrecognizedFavoriteRemovalLocator)}`; a non-row line carries no identity at all. Do not spell this as a three-variant row kind: `static` is a fact about rendering rather than a state a row can be in, and `diagnostic` is untruthful for a row that may simply have been written by a newer cargo-tile. Keep selection and currency out of the identity; they are render-time state.
- Put only real rows in the navigation index — recognized and unrecognized — never blank lines or headings.
- Make the selection carry a `FavoriteRowIdentity` or nothing, replacing today's recognized-or-nothing `FavoriteSelection`.
- Render selection styling on unrecognized rows exactly as on recognized ones. They are selectable.
- Delete reaches both kinds, routing an unrecognized row through the Phase 7 `FavoriteRemovalTarget`. Load stays recognized-only and refuses on an unrecognized row.
- Deletion already confirms, and this phase re-keys that rather than inventing a second interaction. `FavoriteRemovalCommitState` arms on the first delete press and commits only on a second press of the same key (`favorites_overlay.rs:653`, `:829`, `:872` pre-split), and opening, reloading or moving the cursor clears it (`:732`, `:802`). Re-key it on `FavoriteRowIdentity` so it covers both kinds and so arming on one row can never commit a delete on another. While it is armed the overlay says a second press confirms; any other key cancels, writing nothing. An unrecognized row confirms for the same reason a recognized one does — it may be valid data written by a newer cargo-tile rather than a broken row.
- A refusal from the Phase 7 locator re-verification reaches the user as an overlay notice naming that the file changed and nothing was deleted. Swallowing it would leave the row on screen with no explanation.
- Derive the footer from the selection — load and delete on a recognized row, delete only on an unrecognized row, neither on nothing — and drop the movement hint when there is only one navigation position. Cache the footer and rebuild it when bindings, page, content state or selection kind change, rather than reconstructing the `String` every render (Invariant 3).

**Files:**
- `crates/cargo-tile/src/favorites_overlay/line_plan.rs` — `FavoriteRowIdentity` on the cached line and non-row lines carrying none, navigation index over real rows only, selection styling for unrecognized rows, `append_unrecognized` emitting identified rows
- `crates/cargo-tile/src/favorites_overlay/bindings.rs` — capability-derived footer, cached
- `crates/cargo-tile/src/favorites_overlay/mod.rs` — the selection type, delete routing by identity, load refusal, `FavoriteRemovalCommitState` re-keyed on `FavoriteRowIdentity`, the refusal notice
- `crates/cargo-tile/src/favorites_overlay/content.rs` — carry the locator onto the unrecognized row view

**Constraints from prior phases:** Phase 7 added `UnrecognizedFavoriteRemovalLocator`, minted before the display sort, and `FavoriteRemovalTarget` distinguishing a recognized `FavoriteId` from that locator, both exported from `favorites/mod.rs`; the unrecognized removal path re-verifies under the lock and refuses when the locator no longer identifies exactly one table — surface that refusal to the user rather than swallowing it. Phase 8 split the overlay into `favorites_overlay/{mod,content,bindings,line_plan}.rs` with `line_plan` depending on `content` and `bindings`; the line refs in this Spec are to the pre-split file, so locate by item name.

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh test cargo-tile` with tests covering: an unrecognized row rendering with the selection marker and style; the navigation index skipping blanks and headings; delete on an unrecognized row reaching the locator path; load on an unrecognized row refusing; the footer omitting load on an unrecognized row and both actions on nothing; a single delete press writing nothing and a second press deleting; a delete armed on one row not committing when the cursor has moved to another; and a locator that no longer matches leaving the file intact while the refusal reaches the overlay notice
- `bash ~/.claude/scripts/delegate/verify.sh lint cargo-tile`
- Hands-on: with a malformed entry in `favorites.toml`, open `ctrl-o`, arrow into the unrecognized block, watch the row highlight, and delete it with two presses.

### Phase 10 — A row that matches the running parameters is marked  · status: todo

#### Work Order

**Goal:** Opening the overlay shows which saved rows match what the attract screen is running.

**Spec:**

Opening the favorites table while the attract screen runs a set of parameters that exactly matches a saved favorite gives no sign of it.

`rendered_line` writes a two-cell prefix — `"▸ "` when selected, `"  "` otherwise — which is one glyph plus a separator. Selection and currency are independent: the running row may or may not be under the cursor and both must be visible at once. Widen the prefix to three cells: selection, currency, separator. The four combinations are one value, not two booleans, so the two columns cannot disagree. The three cells are selection, currency, separator, and the four strings are fixed: `"   "` for neither, `"▸  "` for selected, `" ● "` for current, `"▸● "` for both. `▸` is the marker already in use; `●` is the currency mark, single-width, so the budget grows by exactly one cell. Update the width budget accordingly.

`Attract::current_settings()` (`attract/mod.rs:614`) supplies the comparison. Snapshot it when the overlay opens, compare it against each recognized row once while building the line plan, and cache the result on the view. The overlay consumes every key while open (`terminal.rs:720`), so steering cannot stale the snapshot; a resize that reclamps settings recomputes it. This is one comparison per recognized row on open and on resize, none per frame, no allocation.

**What the mark claims.** `Attract` keeps settings, not the `FavoriteId` they came from — loading returns `AttractSettings` alone — and a hand-edited file can hold several rows with equal settings and different ids, which `push` does not normalize. So the honest claim is "this row matches the current parameters", and **every** matching row is marked. Say so where the user can see it: a legend reading `● matches the current parameters`, in the overlay heading. That exact glyph and wording are what Phase 13 documents, so a test asserts them. A single authoritative "the favorite that is running" would mean carrying `FavoriteId` provenance through load, steer, randomize, undo and save and clearing it on every edit; that is a different, larger feature and is not this phase.

The comparison is derived `PartialEq` on `AttractSettings`, the same equality `FavoriteRows::push` already uses to recognize a repeat save. A new settings field breaks the exhaustive constructors, so it cannot slip past this silently; a separate comparison key would duplicate the persistence schema.

**Files:**
- `crates/cargo-tile/src/favorites_overlay/line_plan.rs` — three-cell prefix, the four-state marker value, width budget, per-row match cached on the view
- `crates/cargo-tile/src/favorites_overlay/mod.rs` — hold the snapshot, compare it while building the plan, render the legend
- `crates/cargo-tile/src/favorites_overlay/content.rs` — the match flag on the recognized row view
- `crates/cargo-tile/src/globals.rs` — the overlay-open handler (`:117`) is the only place that holds both `App::attract` and the overlay, so the snapshot is taken there and passed into `open`
- `crates/cargo-tile/src/terminal.rs` — the resize branch (`:664`) already reclamps attract settings; it re-snapshots into the open overlay from the same place

**Constraints from prior phases:** Phase 9 gave the cached line a `FavoriteRowIdentity::{Recognized(FavoriteId), Unrecognized(UnrecognizedFavoriteRemovalLocator)}` with non-row lines carrying none, put only real rows in the navigation index, made the selection carry that identity or nothing, and kept selection out of the identity as render-time state — currency joins it there. The prefix Phase 9 renders is still two cells; this phase widens it. Unrecognized rows never carry a currency mark: they have no settings to compare. `Attract::current_settings()` takes `&mut self` (`attract/mod.rs:697`), so the caller must hold the attract state mutably — which is why the snapshot is taken in `globals.rs` and `terminal.rs` rather than inside the overlay.

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh test cargo-tile` with tests covering all four selected/current combinations rendering their exact three-cell prefixes, the legend text as written above, every matching row marked when two rows share settings, no mark on an unrecognized row, the width budget accounting for the wider prefix, and a resize while the overlay is open recomputing the marks against the reclamped settings
- `bash ~/.claude/scripts/delegate/verify.sh lint cargo-tile`
- Hands-on: steer the attract screen to a saved favorite's parameters, open `ctrl-o`, see the mark.

### Phase 11 — Saving says whether it added a favorite or refreshed one  · status: todo

#### Work Order

**Goal:** Re-saving parameters that are already stored says so, instead of implying a second row was added.

**Spec:**

`FavoriteRows::push` already distinguishes the two: an exact settings match keeps the existing row's id and updates only its timestamp; a new set appends a row. The public result throws that distinction away, so both paths toast the same "saved" text, and a user who saves twice and then opens the overlay finds one row and no explanation. Nothing is wrong with the file — this is the dedup working — the confirmation is what misleads.

Return `FavoriteSaveOutcome::{Added, Refreshed}` from the save entry point, and give each its own confirmation text through the existing toast path. Not a boolean and not an `Option`: both answers are ordinary successes and the caller renders different text for each. The branch that decides it has already done the equality comparison, so this costs one enum variant.

**Files:**
- `crates/cargo-tile/src/favorites/rows.rs` — `push` returns the named outcome
- `crates/cargo-tile/src/favorites/file.rs` — the save entry point carries it out
- `crates/cargo-tile/src/favorites/mod.rs` — export
- `crates/cargo-tile/src/globals.rs` — the `ctrl-s` handler selects the confirmation text

**Constraints from prior phases:** Phase 6 put `push` in `favorites/rows.rs` and the save entry point in `favorites/file.rs`. Phase 5 removed `schedule_timed_toast` from `globals.rs` in favor of the framework-owned toast deadline: `globals.rs` now calls `app.framework.toasts.push_timed(...)` (`:147`, `:214`) with no paired scheduling call, and `Toasts::next_visual_change_deadline(now)` supplies the repaint cadence, so the confirmation is a `push_timed` and nothing else. Phase 10 uses the same `AttractSettings` equality to decide the currency mark; the two must agree, so do not introduce a second comparison.

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh test cargo-tile` with tests proving a first save reports added, an identical second save reports refreshed and leaves one row, and the refreshed row's timestamp moved
- `bash ~/.claude/scripts/delegate/verify.sh lint cargo-tile`

### Phase 12 — A column heading carries its own value  · status: todo

#### Work Order

**Goal:** Reordering the overlay's parameter columns cannot put a value under the wrong heading.

**Spec:**

`column_descriptors(mode)` and `favorite_cells(settings)` are independent vectors matched only by index, so reordering one silently misaligns every row with no compiler complaint. Give `ParameterColumnDescriptor` the function that renders its own column's value and delete the parallel vector. It runs while rebuilding the plan, not per frame (Invariant 3).

`AttractMode::draw` (`attract/mod.rs:427`) has the same shape: it computes an index bounded by `AttractMode::ALL`, then maps `0`, `1` and everything else through a separate match, so a fourth mode would silently draw as `Pixelate`. Index `ALL` directly so adding a mode updates selection by construction. No trait and no generic — the array length is already the bound.

**Files:**
- `crates/cargo-tile/src/favorites_overlay/bindings.rs` — `ParameterColumnDescriptor` carries its value renderer; `column_descriptors` updated
- `crates/cargo-tile/src/favorites_overlay/line_plan.rs` — `favorite_cells` deleted; rows render through the descriptors
- `crates/cargo-tile/src/attract/mod.rs` — `AttractMode::draw` indexes `ALL`

**Constraints from prior phases:** Phase 8 put `column_descriptors` in `bindings.rs` and `favorite_cells` in `line_plan.rs`. Phases 9 and 10 changed the row prefix and the width budget in `line_plan.rs`; the column values themselves are untouched by both, so this phase is additive to them.

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh test cargo-tile` with a test proving heading and value stay paired when the descriptor order changes, and one proving a mode added to `AttractMode::ALL` is reachable from `draw`
- `bash ~/.claude/scripts/delegate/verify.sh lint cargo-tile`

### Phase 13 — The README documents favorites and stops contradicting the code  · status: todo

#### Work Order

**Goal:** A user who reads the README can find the favorites feature, and the statements beside it are true.

**Spec:**

`crates/cargo-tile/README.md` documents the attract steering keys in detail and never uses the word "favorite"; `ctrl-s`, `ctrl-o`, `m`, `r` and `u` do not appear. Three statements already in the file are false and sit next to where the new section goes, so they are fixed in the same pass:

- `## configuration` says there are three files and omits `favorites.toml` (`:75`–78).
- The attract section says steering keys do not work when the screen appears on its own (`:465`–469). `keyed_mode` returns a mode when the screen was requested **or** has fully arrived at `faded == 0` (`attract/mod.rs:680`), and a regression test in that file's test module covers it.
- The template section says `AppGlobalAction` starts with no variants (`:70`–73); it now has many.

Add a favorites section beside the existing attract one, in the same voice: what a favorite stores (the attract mode and its steerable parameters, not the animation's instantaneous position); where the file lives (`<os config dir>/cargo-tile/favorites.toml`); `ctrl-s` to save and `ctrl-o` to open the table; `m` for a random favorite, `r` to randomize the current parameters, `u` to undo the last replacement; in the overlay, arrows to move, enter to load, `x` to delete, left/right to page the parameter columns, esc to close; what the three-cell row prefix means, quoting the legend `● matches the current parameters` exactly as Phase 10 renders it and saying that every matching row is marked rather than one running favorite; that rows this version cannot read are kept, shown in their own block, selectable, and deletable with two presses of the delete key; and that saving the same parameters twice refreshes the existing row rather than adding one. State that the listed keys are defaults and can be rebound.

Also filter the `x` Dismiss row out of cargo-tile's rendered keymap. `GlobalAction::Dismiss` is bound to `'x'` as a shared framework default (`global_action.rs:70`, `:241`) that `cargo-port` relies on, while cargo-tile's tests require `x` not to close framework overlays — so the fix is in cargo-tile's keymap assembly, not in the shared default (Invariant 2 reasoning applies: do not change what the other consumer depends on).

**Files:**
- `crates/cargo-tile/README.md` — favorites section; configuration table; attract-steering paragraph; the `AppGlobalAction` claim
- `crates/cargo-tile/src/keymap.rs` — filter the inactive `x` Dismiss row from the rendered keymap

**Constraints from prior phases:** Phase 9 made unrecognized rows selectable and deletable behind the existing two-press confirmation and gave the footer capability-derived hints; do not call them diagnostics in the README, since a row this version cannot read may have been written by a newer cargo-tile. Phase 10 widened the row prefix to three cells — `"   "`, `"▸  "`, `" ● "`, `"▸● "` — and renders the legend `● matches the current parameters`, marking every matching row rather than one authoritative running favorite; Phase 11 named the save outcome `FavoriteSaveOutcome::{Added, Refreshed}`. Document what those phases actually shipped — read them before writing the section rather than describing the plan.

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh test cargo-tile` — the keymap filter has a test; the README has no test, so verify it by reading each corrected claim against the code it describes and saying so in the report
- `bash ~/.claude/scripts/delegate/verify.sh lint cargo-tile`

### Phase 14 — The second window's capture failure gets its actual cause  · status: todo

**Blocked by:** a live two-window reproduction. This phase cannot start until `cargo tile` runs in two windows of one terminal app with the Phase 1 status and the Phase 3 logging in place, and the failing stage has been recorded. No delegate can stage that reproduction.

#### Work Order

**Goal:** The second `cargo tile` in one terminal app gets its desktop capture.

**Spec:**

Running `cargo tile` in two windows of the same iTerm2 — an app that already has Screen Recording permission — leaves the second one with no desktop capture while the first keeps its own.

**There is no first-caller-wins path to find.** Every `BackdropMonitor::new` builds its own channels, workers, pinned window and cached desktop (`monitor.rs:129`–184), and capture resolves the pinned id across all visible windows (`desktop.rs:494`–495). Nothing is shared or app-keyed. What produced the *appearance* of ownership was diagnosis, not exclusivity: a monitor holding an earlier successful capture kept showing it while a newly started monitor that never got a first capture showed nothing, and both looked identical from outside.

Phases 1 and 3 make the real cause observable, but not yet completely. **Instrument first, then reproduce.**

What Phase 3 already logs, on transition only, is one `backdrop:` line carrying `report` (the `WindowIdentification`), `capture_status` (the `BackdropStatus`, whose failed form names the `CaptureFailure` stage) and `captured_window_id` (the `LastSuccessfulCaptureWindowId`). What it cannot tell you is the id the *failing* instance aimed at: `captured_window_id` is `WaitingForFirstSuccess` for an instance that has never captured, which is exactly the instance under investigation, and `WindowIdentification::Fallback` says selection fell back without saying to what. Since "which window did each instance pick" is the question this phase exists to answer, add attempt-level recording before reproducing — for every capture attempt, the window id it targeted and whether that id came from the pinned selection or from the frontmost-then-size heuristic, logged on transition like the rest.

Then run both instances and record, from outside the program, the windows the window server reports, alongside each instance's targeted window id and the `CaptureFailure` stage the second instance logs. Reasoning from branches is what the last two backdrop defects both defeated; enumerate first.

**Running the reproduction.** The frame log is off unless `CARGO_TILE_FRAME_LOG` names a path (`cargo-tile/src/probe.rs:76`), so neither instance records anything by default. Each process truncates the file on its own first write (`probe.rs:181`–191), so the two instances **must** be given different paths or they will erase each other's evidence. Give each instance its own path, keep both logs, and preserve every `backdrop:` line from both — those lines and the attempt-level records above are the phase's evidence.

Phase 1 already deduplicated the exclusion list, which was the standing suspect — a sibling terminal window above the selected one appeared both in the owned-windows set and the windows-above set. If the reproduction now succeeds, this phase is closed by evidence rather than by code, and that is a legitimate outcome to report.

Otherwise fix what the recorded stage names, in `tui_pane` where it lives.

**Files:**
- `crates/tui_pane/src/backdrop/desktop.rs` — window resolution, filter construction, or capture, as the evidence directs
- `crates/tui_pane/src/backdrop/monitor.rs` — window pinning and identification, if the evidence points there

**Constraints from prior phases:** Phase 1 typed the capture failure per stage and made `Desktop::capture` return `Result<Desktop, CaptureFailure>` (`desktop.rs:201`, platform implementation at `:470`). It shipped **no preflight**: `SCShareableContent::get()` runs first, because that call is the one that raises the macOS permission prompt, and `CGPreflightScreenCaptureAccess` classifies only a query that already failed (`desktop.rs:446`–453, called at `:475`). That call answers `false` for a process never asked exactly as for one that refused, so the access variant means "not granted", not "the user said no"; in `objc2-core-graphics` 0.3.2 it is declared safe and carries no unsafe annotation. Phase 1 also deduplicated the exclusion window ids (`desktop.rs:458`, called at `:536`) and split the last successful desktop from the latest attempt status on `BackdropMonitor` (`monitor.rs:129`–184). Phase 2 renamed the access variant to `ScreenRecordingAccessNotGranted` (`desktop.rs:40`) and made a successful capture report the window id it used: `BackdropMonitor::captured_window_id()` (`monitor.rs:460`) returns `LastSuccessfulCaptureWindowId`, either `WaitingForFirstSuccess` or `Available { window_id: u32 }`. This phase's Spec asks for that id by name, and Phase 3 puts it in the log; a second window that never captures reports the waiting state, which is itself evidence. Phase 2 added `WindowIdentification`, whose `Fallback` means window selection gave up on pinning and fell back to frontmost or size matching — a strong candidate for what the second window hits. Phase 3 routes the failing stage and the identification outcome into cargo-tile's attract diagnostics, logging on transition only, which is where the reproduction reads them: `probe::note` emits one `backdrop: report=… capture_status=… captured_window_id=…` line from `Attract::identify` (`attract/mod.rs:964`) whenever any of the three changes. That call is gated on `CARGO_TILE_FRAME_LOG`, and the attract screen's own neutral notice now names that variable rather than promising a recording an ordinary run never makes.

**Acceptance gate:**
- Both instances draw their own desktop capture, confirmed by eye in two simultaneous windows
- `bash ~/.claude/scripts/delegate/verify.sh check cargo-tile`, `lint cargo-tile`, `test tui_pane`, `check cargo-port`
- A regression test at whatever level the cause allows, or an explicit statement that the cause is a window-server interaction no unit test can reach. Per Invariant 1 the listed `test tui_pane` line does not compile `backdrop/**`, so a test landing there is named in the report and run out of band with the feature enabled before this phase closes, then again at the final workspace gate; behavior that must actually execute per phase needs a cargo-tile-side test instead

### Phase 15 — Window selection stops being a bare optional number  · status: todo

#### Work Order

**Goal:** One named type carries which window the backdrop is capturing behind, replacing three owned options whose `None` means "nothing settled, use frontmost or size".

**Spec:**

Three owned values spell window selection as `Option<u32>`, and in each the `None` means "no window has been settled on, so fall back to frontmost, then size" — a rule the type does not state and every reader has to recover from `Desktop::capture`'s body: `BackdropMonitor::pinned` (`monitor.rs:157`), `Request::window` (`:193`, built at `:436`), and the `pinned` parameter of `Desktop::capture` (`desktop.rs:201`, threaded to the platform implementation at `:470`–472).

The three sites look alike and are not one domain, so one type threaded through all of them would hand the capture path a state it cannot act on. `BackdropMonitor::pinned` is about **identification progress**: its `None` means the search is still running or has been exhausted, and only `attempts` on the monitor tells those apart. `Request::window` and `Desktop::capture`'s parameter are about the **capture target**: each means an exact id or "use the heuristic now", and neither can behave differently for a search still in flight.

So introduce one private `CaptureWindowTarget::{Identified(u32), FrontmostOrSizeHeuristic}` in `backdrop/` for the worker request and the capture parameter, and leave identification progress where it belongs — in `WindowIdentification` or an equivalent monitor-owned state that names pending and exhausted separately, rather than encoding them in an option the capture path then has to interpret. The monitor derives the target at the boundary where it builds the request; `Desktop::capture` matches it once at the top and its body stops re-deriving what `None` meant.

Keep `CaptureWindowTarget` private to the crate: `WindowIdentification` is already the public report on the same subject, and nothing outside `tui_pane` reads the capture parameter.

Do not change what any of the three currently do. This phase is a type change with no behavior change, and its gate is that every existing test still passes unmodified.

**Files:**
- `crates/tui_pane/src/backdrop/desktop.rs` — `CaptureWindowTarget`, `capture`'s parameter, and the resolution at `:494`–495
- `crates/tui_pane/src/backdrop/monitor.rs` — `Request::window` holds the target; `pinned` keeps identification progress and derives the target where the request is built

**Constraints from prior phases:** Phase 2 made `WindowIdentification::Identified` carry the settled window id and added `LastSuccessfulCaptureWindowId` for the id a capture used, deliberately leaving these three options alone so this phase could be designed after the second-window cause was known. Both are public reports on window selection and capture; the enum this phase introduces is the private third one that the capture path itself threads. Phase 14 either found that cause and fixed it in `desktop.rs`/`monitor.rs`, or closed on evidence that Phase 1's exclusion-id deduplication already fixed it; read its as-built record before touching the capture path, because a fix landing there changes what the three states mean.

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh check cargo-tile` — the only gate that compiles `backdrop/**` (Invariant 1)
- `bash ~/.claude/scripts/delegate/verify.sh lint cargo-tile`
- `bash ~/.claude/scripts/delegate/verify.sh test tui_pane`
- `bash ~/.claude/scripts/delegate/verify.sh check cargo-port`
- Every existing backdrop test passes with no edit to its body. A test that had to change means behavior changed, which this phase forbids. Per Invariant 1 the listed `test tui_pane` line compiles none of them: `check cargo-tile` proves they still build, and the feature-enabled suite is run out of band before this phase closes and again at the final workspace gate. Name in the report how many backdrop tests ran and that none were edited.
