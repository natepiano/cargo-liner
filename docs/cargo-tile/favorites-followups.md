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
  - `crates/cargo-tile/src/attract/mod.rs` — `AttractSettings`, `current_settings()` (`:614`), `AttractMode::draw` index-then-match (`:343`), `identified: Option<bool>` (`:456`, written `:886`), `keyed_mode` (`:602`), `backdrop_overdue` (`:1141`), `render` passing one `Backdrop` to all three renderers (`:1157`), automatic-attract steering regression test (`:1773`). 1913 lines, tests at 1182.
  - `crates/cargo-tile/src/attract/{moving_band,moving_text,pixelate,held_key}.rs` — key bindings only; none of them render.
  - `crates/cargo-tile/src/render.rs` — the pane background the band currently falls back to. 3177 lines, tests at 2033.
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

### Phase 2 — Window identification says whether it is still trying  · status: todo

#### Work Order

**Goal:** `BackdropMonitor::identify` reports its progress instead of returning the same `false` for "retrying" and "gave up".

**Spec:**

`identify() -> bool` (`monitor.rs:250`) returns `false` when attempts are exhausted (`:254`), when a retry is merely paced by the backoff (`:261`–265), and when the marker title fails to write (`:299`). A caller cannot tell a search still running from one that has given up and fallen back to frontmost or size selection.

Replace the return with a public `WindowIdentification` enum in `backdrop/monitor.rs`: not attempted, pending, identified, and fallback — the last meaning pinning is exhausted and frontmost or size selection is in use. `Fallback` describes window *selection*, not capture: it must not read as, or be reported as, a capture failure. Re-export it with the other backdrop types, feature-gated.

Every branch of `identify` maps to exactly one variant, and this list is the mapping — none of it is left to the implementer's reading:

- `pinned` already settled (`:251`–253) → `Identified`.
- attempts already exhausted on entry (`:254`) → `Fallback`.
- a retry inside `IDENTIFY_RETRY` (`:261`–265) → `Pending`.
- the emulator's own position query answering (`:275`–279) → `Identified`.
- the marker title failing to write (`:299`) → `Pending`. Attempts remain, so this is a lost race rather than a surrender.
- the last pass finding no window (`:313`–322, `attempts >= IDENTIFY_PASSES` with nothing found) → `Fallback` **on that same pass**. It must not answer `Pending` and make the caller wait another frame to learn the search is over.
- any earlier pass finding no window → `Pending`.
- before any pass has run → `NotAttempted`.

`identify` writes to the terminal and calls the window server, so nothing can drive it from a test. Extract the mapping as a pure private function over the state a pass ends in — passes made, whether a window was found, whether a pass ran at all — and call it from `identify`, so the production path and the test read the same code. A seam nothing calls in production proves nothing.

On the `cargo-tile` side, `Attract::identified: Option<bool>` (`attract/mod.rs:463`, initialized `:512`, written `:897`–898) becomes a field holding the reported `WindowIdentification`. Note that the original complaint about this field was wrong and the Spec is not a bug fix: `Option<bool>` has three representable values and all three are in use — `None` before any report, `Some(false)` unsettled, `Some(true)` settled. The gain here is that the states are named at the boundary that produces them. Preserve the existing behavior at every read site.

**Files:**
- `crates/tui_pane/src/backdrop/monitor.rs` — `WindowIdentification`, `identify` returns it, backoff and exhaustion map to distinct variants
- `crates/tui_pane/src/backdrop/mod.rs` — re-export
- `crates/tui_pane/src/lib.rs` — feature-gated re-export
- `crates/cargo-tile/src/attract/mod.rs` — `identified` holds the reported value; every read site preserved

**Constraints from prior phases:** Phase 1 added `CaptureFailure` and `BackdropStatus` to `backdrop/desktop.rs` and `backdrop/monitor.rs`, re-exported through `backdrop/mod.rs` and `lib.rs` under `#[cfg(feature = "backdrop")]`, and made `BackdropMonitor` hold the last successful desktop separately from the latest attempt status. `WindowIdentification` is a third, independent report on the same monitor — do not fold it into `BackdropStatus`, which is about capture, not window selection. Phase 1 left `identify` itself untouched: the window it settles on lives in `pinned` (`monitor.rs:118`), and the window id capture actually uses is computed inside `Desktop::capture` (`desktop.rs:491`–496) and never returned to any caller.

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh check cargo-tile`
- `bash ~/.claude/scripts/delegate/verify.sh test cargo-tile` — the attract tests exercise `identified` through the consumer that enables the feature
- `bash ~/.claude/scripts/delegate/verify.sh lint cargo-tile`
- `bash ~/.claude/scripts/delegate/verify.sh test tui_pane`
- A cargo-tile test asserting that a paced retry and an exhausted search are distinguishable at `Attract`'s field, which `Option<bool>` could not express by name.
- Unit tests in `monitor.rs` over the pure mapping function, one per transition listed in the Spec, including that the last failing pass reports `Fallback` rather than `Pending`. `monitor.rs` has no test module today, so this adds one. Per Invariant 1 these compile but do not execute until the final workspace gate — name them in the report so that gate covers them.

**Pending decision: two questions about the backdrop's public vocabulary and shape, both cheapest to settle before this phase writes more code against it**

Actual problem:
**(a) Names.** The architect review of Phase 1 holds that `BackdropStatus` (`tui_pane/src/backdrop/monitor.rs:103`) and `CaptureFailure::PermissionDenied` (`desktop.rs:90`) both promise more than they mean, and that Phases 2, 3 and 14 will spread them. `BackdropStatus` reports the newest capture *attempt*, not whether a backdrop is available — a monitor whose status is failed still renders its last good desktop. `PermissionDenied` is set by classifying an already-failed shareable-content query with `CGPreflightScreenCaptureAccess`, and that call answers `false` for a process that has never been asked exactly as it does for one that refused, so the variant means "Screen Recording access is not granted", never "the user denied it".

**(b) The capture boundary.** Phase 14 must record "each instance's selected window id" and today cannot: the id capture actually uses is resolved inside `Desktop::capture` and never leaves it (`desktop.rs:491`–496). Separately, three owned values spell window selection as a bare option whose `None` means "nothing settled, use frontmost or size" — `BackdropMonitor::pinned` (`monitor.rs:118`), `Request::window` (`:154`), and the `pinned` parameter of `Desktop::capture` (`desktop.rs:468`). This phase's `WindowIdentification` names the *search's* progress; it does not name what the capture then selected, and as written `Identified` need not even carry the id it settled on.

What exists now:
- `pub enum BackdropStatus { WaitingForFirstResult, Ready, Failed(CaptureFailure) }` and `CaptureFailure::PermissionDenied`, documented as "The shareable-content query failed and this process lacks Screen Recording access.", both re-exported from `backdrop/mod.rs` and `lib.rs`.
- Nothing outside `tui_pane` reads either name yet; `cargo-tile` first consumes them in Phase 3.
- `identify` sets `self.pinned`, and this phase turns its `bool` into `WindowIdentification` with no requirement that `Identified` carry a window id.
- `capture(metrics, pinned: Option<u32>)` resolves the pinned id, falls back to frontmost, then to size, and returns a `Desktop` that says nothing about which window was chosen.
- Phase 14's Spec asks for the selected id by name as reproduction evidence; Phase 3's Spec logs only the failing `CaptureFailure` stage and the identification outcome.

What should change:
- **(a)** Rename the variant to say access is not granted rather than that permission was denied, and decide the same question for the type — for example `ScreenRecordingAccessNotGranted` and `LatestCaptureAttempt`. The architect's own suggestions (`LatestCaptureAttemptStatus`, `ShareableContentQueryFailedWithoutScreenRecordingAccess`) put a whole sentence in the identifier and read worse than what they replace. Or keep both names and rely on the doc comments, accepting that Phase 3's notice logic and Phase 14's diagnosis each read a variant that means something narrower than it says.
- **(b) Narrow:** require `Identified { window_id }` to carry the settled id, and have the capture outcome report the id it actually used, so Phase 3 can log it. The public surface grows by one field and one accessor, and nothing existing changes shape. **Wide:** additionally replace all three bare options with one named selection type distinguishing an exact pinned window, a search still running, and an exhausted fallback, threading it through `Desktop::capture`'s signature — the type-design answer, and a changed public signature every backdrop caller compiles against.

Recommendation:
**(a)** Rename the variant; leave the type alone. `PermissionDenied` is the one that will actually mislead — it is the variant Phase 3 selects the "open System Settings" text on, and sending a user who has simply never been asked to a settings pane with nothing to change is the exact defect this plan exists to fix. `BackdropStatus` is read as `monitor.status()` at every call site, where the receiver already answers "status of what", so renaming it costs the same edit and buys much less. Either way this is cheapest now: no consumer outside the crate exists yet, and every later phase adds one. If approved, it is a whole-word rename across the workspace, which your editor applies faster and more accurately than an edit pass.

**(b)** Take the narrow change in this phase and leave the wide one out. The narrow change is precisely what Phase 14 is blocked on, and it is additive. The wide change rewrites the capture entry point while Phase 14 still has no reproduction — and Phase 14 may yet conclude that Phase 1's exclusion-id deduplication already fixed the second-window failure, at which point what shape this seam wants is a different question from the one it looks like today.

### Phase 3 — The attract notice names the real cause  · status: todo

#### Work Order

**Goal:** A missing desktop capture reports the permission only when the permission is actually denied.

**Spec:**

`ATTRACT_NO_BACKDROP_NOTICE` (`cargo-tile/src/constants.rs:39`) reads `attract: no desktop capture -- allow Screen Recording for this terminal in System Settings > Privacy & Security`. `Attract::backdrop_overdue` (`attract/mod.rs:1141`) knows only that the grace period elapsed with no backdrop, so every cause inherits that text — including the reproduced case where a second window of an already-permitted iTerm2 gets no capture and the user is sent to a settings pane with nothing to change.

Consume the `BackdropStatus` Phase 1 exposed. Show the existing permission notice only when the latest attempt failed at the Screen Recording access stage, and add a second constant beside it for every other cause: it must not name a setting, and it must not imply the user did something wrong — the capture is unavailable and the reason is recorded, that is all the user can act on. The access variant covers a process that has never been asked as well as one that refused, so even the permission text reads as an instruction, never an accusation. Keep both inside the existing grace period: `backdrop_overdue` (`attract/mod.rs:1141`) deliberately waits so a slow capture is not called a missing one, and that stays.

The choice is not two booleans. Name it — a private enum in `attract/mod.rs` with one variant per outcome, and one pure function mapping the inputs to it. The inputs are whether the grace period has elapsed, whether a current backdrop exists, and the latest `BackdropStatus`. The complete mapping:

- grace period not yet elapsed → no notice, whatever the status.
- overdue, no current backdrop, still waiting for a first result → the generic notice.
- overdue, no current backdrop, failed at the access stage → the permission notice.
- overdue, no current backdrop, failed at any other stage → the generic notice.
- overdue, no current backdrop, status ready → the generic notice. A capture that succeeded but has not been placed yet is not a permission problem.
- a current backdrop exists → no notice, even when the newest attempt failed. The user is looking at a desktop; telling them there is none is wrong.

`BackdropMonitor` calls the window server and cannot be constructed in a state a test chooses, so this function takes those three inputs as plain values rather than reading the monitor. Call it from the render path — a classifier nothing uses in production proves nothing — and test it directly.

Log the failing `CaptureFailure` stage and the `WindowIdentification` Phase 2 reports where cargo-tile already logs attract diagnostics, so the two-window reproduction in the final phase has evidence to read. If Phase 2's pending decision resolves in favour of the capture outcome reporting the window id it used, log that too — Phase 14's Spec asks for it by name. Log on transition only: an unchanged stage is not logged again, because this path runs every 33ms (Invariant 3). The user-facing notice stays one short line.

**Files:**
- `crates/cargo-tile/src/constants.rs` — second notice constant beside `ATTRACT_NO_BACKDROP_NOTICE:39`
- `crates/cargo-tile/src/attract/mod.rs` — `backdrop_overdue` selects on `BackdropStatus`; the failing stage reaches the diagnostic log
- `crates/cargo-tile/src/render.rs` — only if the status line renders the notice through it

**Constraints from prior phases:** Phase 1 gave `BackdropMonitor` a `BackdropStatus` — waiting for a first result, ready, or failed carrying a `CaptureFailure` whose variants name the stage — and a last successful desktop held separately, so a failure never removes a desktop already on screen. There is no preflight: `SCShareableContent::get()` runs first, because that is the call that raises the macOS permission prompt, and `CGPreflightScreenCaptureAccess` classifies only a query that already failed. That call answers `false` for a process never asked exactly as for one that refused, so the access variant means "not granted". A later success clears a stored failure. Phase 2 added `WindowIdentification`, whose `Fallback` variant means window *selection* fell back to frontmost or size matching; it is not a capture failure and must not select the failure notice. Phase 2 also carries a pending decision on whether the capture outcome reports the window id it used, which is the only thing that could put that id in this phase's log.

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh test cargo-tile` — with tests over the pure classifier covering every outcome in the Spec's mapping, including that a current backdrop suppresses the notice while the newest attempt is failed, that a ready-but-unplaced capture does not select the permission text, and that nothing appears before the grace period elapses
- `bash ~/.claude/scripts/delegate/verify.sh lint cargo-tile`

### Phase 4 — The moving band paints the desktop across the pane  · status: todo

#### Work Order

**Goal:** The field the band has crossed shows the desktop rather than flat theme color, and the band's leading and trailing edges fade into it instead of cutting off.

**Spec:**

The capture is already routed: `Attract::render` passes one `Backdrop` to all three renderers (`attract/mod.rs:1157`) and `TravelingBand::render` samples `backdrop.color_at` for every covered cell (`band.rs:622`). `attract/moving_band.rs` holds key bindings and draws nothing — do not edit it for this. Three things in the renderer produce the reported appearance:

1. Cells outside the strip are deliberately left untouched (`band.rs:603`), so everything the band has passed over falls back to the pane background from `render.rs`. That is the flat field.
2. Inside the strip the background is blended halfway toward the existing background through `BAND_BEHIND_FADE`, inherited from `TEXT_BEHIND_FADE = 128` (`band.rs:643`, `backdrop/constants.rs:23`, `:269`). Desktop variation survives at half strength.
3. The edge treatment is one cell wide — `coverage` fades only the partially entered boundary cell (`band.rs:615`).

Change the composition in `TravelingBand::render`: paint the sampled desktop across the full area first, then draw the glyph strip over it. `DriftingText` at `text.rs:552` is the reference — it already paints every cell from the backdrop and its blend at `:572` is the pattern to follow. Then fade glyph ink toward the sampled background across a designed multi-cell leading and trailing falloff rather than the single fractional cell, so the strip reads as passing over the desktop.

Tune the background blend so desktop variation is visible rather than washed halfway out. `BAND_BEHIND_FADE` currently aliases `TEXT_BEHIND_FADE`; give the band its own value if the two want different strengths, and say in the constant's doc comment why they differ.

Colors go through `theme::blend_color` and the theme accessors — no new hardcoded color in `tui_pane` (Invariant 5). The band already scans width × height per frame with a cached lookup and two integer blends per covered cell; painting the full area matches what text and pixelate already do. No second capture, no reduction, no per-frame allocation (Invariant 3).

**Files:**
- `crates/tui_pane/src/backdrop/band.rs` — full-area composition, multi-cell edge falloff, glyph-versus-background derivation
- `crates/tui_pane/src/backdrop/constants.rs` — band fade strength and any new falloff width constant, each documented

**Constraints from prior phases:** none of Phases 1–3 change the render path. Phase 1's `Result` return on `Desktop::capture` does not reach `TravelingBand::render`, which receives an already-built `Backdrop`.

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh check cargo-tile`
- `bash ~/.claude/scripts/delegate/verify.sh lint cargo-tile`
- `bash ~/.claude/scripts/delegate/verify.sh test tui_pane`
- Buffer tests in `band.rs` over a synthetic multicolor backdrop proving that adjacent cell backgrounds under the band stay visibly different from each other, that cells outside the strip carry desktop color rather than the pane background, and that edge cells approach their own underlying color across more than one cell. Per Invariant 1 these compile but run at the final workspace gate — name them.
- Hands-on: run `cargo tile`, press `a`, and look at the band.

### Phase 5 — The toast owns its next visual-change deadline  · status: todo

#### Work Order

**Goal:** A toast that cannot change what is on screen stops asking the event loop for frames, and the arithmetic that decides this lives in one place.

**Spec:**

`ToastVisualTimeline` (`terminal.rs:105`, impl `:120`) asks for 8ms frames from `pushed_at` through the entrance, and for an ordinary single-line toast none of them can change anything. `current_visible_lines` computes `floor(elapsed / line_ms) + 1` and clamps upward to `min_height` (`toast.rs:223`–238). With `min_height == 3` the rendered height is 3 at steps 0, 1 and 2 and first becomes 4 at step 3, so the earliest possible change is `pushed_at + min_height * entrance_line_ms` — one interval later than a naive reading. At 150ms per line that is 450ms, up to 57 redraws that cannot alter the toast. When `target_height == min_height`, the common case, there is no entrance change at all.

Model whether an entrance interval exists rather than computing a start that may not be one: an entrance is either absent — begin in the static phase and schedule only expiry — or scheduled with a start and an end. Exit boundaries are unchanged.

Put the deadline in `tui_pane::toasts::manager`, which owns `created_at`, the phase, the wrapping and the minimum height: one query returning the next moment a toast's rendering can change — the next line-height boundary, the exit boundary, or expiry. `Toasts` already tracks active toasts, so this is a scan over them, constant work per toast, no allocation.

Then delete the duplicates. `cargo-tile` reconstructs target height and durations at `terminal.rs:120` and `:277` and pairs every push with `schedule_timed_toast` by hand (`globals.rs:155`, `:230`, `favorites_overlay.rs:1180`, `app.rs:217`); `ToastVisualSchedule` (`terminal.rs:204`) and `ToastVisualTimeline` both go, and `App::toast_visual_schedule` (`app.rs:133`, `:188`) with them. `cargo-port` has the mirror-image workaround: `is_animating` (`app/mod.rs:465`) returns true whenever any toast is active (`:470`), keeping the 80ms `ANIMATION_TICK` heartbeat alive for the toast's whole lifetime including its static interval — replace that clause with the framework deadline in `animation_timeout` (`:452`).

Both callers must keep working: this is one atomic change across three crates, which is why it is one phase.

**Files:**
- `crates/tui_pane/src/toasts/manager.rs` — the next-visual-change deadline over active toasts
- `crates/tui_pane/src/toasts/toast.rs` — entrance modelled as absent or scheduled; the corrected first-change boundary
- `crates/tui_pane/src/toasts/mod.rs` — export the deadline API
- `crates/cargo-tile/src/terminal.rs` — delete `ToastVisualTimeline` (`:105`) and `ToastVisualSchedule` (`:204`); consume the framework deadline; the schedule tests at `:971`, `:1066`, `:1114` move to the new surface
- `crates/cargo-tile/src/app.rs` — drop `toast_visual_schedule` (`:133`, `:188`, `:217`) and its import (`:30`)
- `crates/cargo-tile/src/globals.rs` — `schedule_timed_toast` pairings at `:155`, `:230`
- `crates/cargo-tile/src/favorites_overlay.rs` — the pairing at `:1180`
- `crates/cargo-port/src/tui/app/mod.rs` — `animation_timeout` (`:452`) uses the deadline; drop the always-animating toast clause (`:470`)

**Constraints from prior phases:** none — the toast path is independent of Phases 1–4. This phase touches `favorites_overlay.rs` only to remove a `schedule_timed_toast` call site; it must not restructure that file, which Phase 8 splits.

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh test tui_pane` — toasts are default-feature code, so this gate really runs them
- `bash ~/.claude/scripts/delegate/verify.sh test cargo-tile`
- `bash ~/.claude/scripts/delegate/verify.sh test cargo-port`
- `bash ~/.claude/scripts/delegate/verify.sh lint tui_pane`, `lint cargo-tile`, `lint cargo-port`
- Tests proving a single-line toast requests no entrance frame before expiry, a multi-line toast's first repaint lands on `pushed_at + min_height * entrance_line_ms`, and exit boundaries are unchanged.

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
- No file outside `crates/cargo-tile/src/favorites/` changes, since `mod.rs` re-exports the same names.

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

Add an opaque locator type minted while loading, before the display sort, carrying the raw table index plus enough of the table's own content to re-verify it identifies the same table after the locked re-read. Give removal a target that distinguishes a recognized `FavoriteId` from an unrecognized locator rather than overloading `FavoriteId`. When the locator no longer identifies exactly one table, refuse the removal and report it — a concurrent edit fails loudly rather than deleting the wrong row.

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
- No file outside `crates/cargo-tile/src/favorites_overlay/` changes.

### Phase 9 — Rows know what they are, and the footer offers only what will work  · status: todo

#### Work Order

**Goal:** Every row the cursor can reach shows that it is selected, a broken row can be deleted, and the footer names only keys that will act.

**Spec:**

Three complaints that rewrite the same row contract — the cached line variants, the selection type, the navigation indices and the footer. Built separately they would conflict three times over, which is why they are one phase.

**Rows do not show the cursor.** Scrolling past the last recognized favorite carries the view into the unrecognized block, where presses are absorbed and no row takes the highlight. Nothing is broken underneath: `saved_count` counts recognized rows only, so the viewport bounds the selection to them, and `append_unrecognized` emits `CachedOverlayLine::Static`, which `rendered_line` returns with neither the `"▸ "` marker nor `selection_style`. Navigation is also coarser than it looks — `finish_navigation` puts *every* line after the last favorite into the index, blank lines and headings included.

**The footer advertises what the selection cannot do.** `FavoritesSurfaceBindings::footer` formats move, load and delete unconditionally; only horizontal paging is conditional.

Build them together:

- Give the cached line a row **kind** — static, recognized, or diagnostic — carrying the row's identity: a `FavoriteId` for recognized, the Phase 7 locator for diagnostic. Keep selection and currency out of the variant; they are render-time state, not row identity.
- Put only real rows in the navigation index — recognized and diagnostic — never blank lines or headings.
- Make the selection type distinguish recognized, diagnostic and nothing, replacing today's recognized-or-nothing `FavoriteSelection`.
- Render selection styling on diagnostic rows exactly as on recognized ones. They are selectable.
- Delete reaches both kinds, routing a diagnostic row through the Phase 7 removal target. Load stays recognized-only and refuses on a diagnostic row. Confirm before deleting a diagnostic row: it may be valid data written by a newer cargo-tile rather than a broken one.
- Derive the footer from the selection kind — load and delete on a recognized row, delete only on a diagnostic row, neither on nothing — and drop the movement hint when there is only one navigation position. Cache the footer and rebuild it when bindings, page, content state or selection kind change, rather than reconstructing the `String` every render (Invariant 3).

**Files:**
- `crates/cargo-tile/src/favorites_overlay/line_plan.rs` — row kinds on the cached line, navigation index over real rows only, selection styling for diagnostic rows, `append_unrecognized` emitting diagnostic rows
- `crates/cargo-tile/src/favorites_overlay/bindings.rs` — capability-derived footer, cached
- `crates/cargo-tile/src/favorites_overlay/mod.rs` — the selection type, delete routing by kind, load refusal, the confirmation for a diagnostic deletion
- `crates/cargo-tile/src/favorites_overlay/content.rs` — carry the locator onto the unrecognized row view

**Constraints from prior phases:** Phase 7 added an opaque locator minted before the display sort and a removal target enum distinguishing a recognized `FavoriteId` from an unrecognized locator, both exported from `favorites/mod.rs`; the unrecognized removal path re-verifies under the lock and refuses when the locator no longer identifies exactly one table — surface that refusal to the user rather than swallowing it. Phase 8 split the overlay into `favorites_overlay/{mod,content,bindings,line_plan}.rs` with `line_plan` depending on `content` and `bindings`; the line refs in this Spec are to the pre-split file, so locate by item name.

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh test cargo-tile` with tests covering: a diagnostic row rendering with the selection marker and style; the navigation index skipping blanks and headings; delete on a diagnostic row reaching the locator path; load on a diagnostic row refusing; and the footer omitting load on a diagnostic row and both actions on nothing
- `bash ~/.claude/scripts/delegate/verify.sh lint cargo-tile`
- Hands-on: with a malformed entry in `favorites.toml`, open `ctrl-o`, arrow into the diagnostic block, watch the row highlight, and delete it.

### Phase 10 — A row that matches the running parameters is marked  · status: todo

#### Work Order

**Goal:** Opening the overlay shows which saved rows match what the attract screen is running.

**Spec:**

Opening the favorites table while the attract screen runs a set of parameters that exactly matches a saved favorite gives no sign of it.

`rendered_line` writes a two-cell prefix — `"▸ "` when selected, `"  "` otherwise — which is one glyph plus a separator. Selection and currency are independent: the running row may or may not be under the cursor and both must be visible at once. Widen the prefix to three cells: selection, currency, separator. The four combinations are one value, not two booleans, so the two columns cannot disagree — neither, selected, current, or both, each mapping to a fixed three-character string. Update the width budget accordingly.

`Attract::current_settings()` (`attract/mod.rs:614`) supplies the comparison. Snapshot it when the overlay opens, compare it against each recognized row once while building the line plan, and cache the result on the view. The overlay consumes every key while open (`terminal.rs:720`), so steering cannot stale the snapshot; a resize that reclamps settings recomputes it. This is one comparison per recognized row on open and on resize, none per frame, no allocation.

**What the mark claims.** `Attract` keeps settings, not the `FavoriteId` they came from — loading returns `AttractSettings` alone — and a hand-edited file can hold several rows with equal settings and different ids, which `push` does not normalize. So the honest claim is "this row matches the current parameters", and **every** matching row is marked. Say so where the user can see it — a legend in the heading or footer. A single authoritative "the favorite that is running" would mean carrying `FavoriteId` provenance through load, steer, randomize, undo and save and clearing it on every edit; that is a different, larger feature and is not this phase.

The comparison is derived `PartialEq` on `AttractSettings`, the same equality `FavoriteRows::push` already uses to recognize a repeat save. A new settings field breaks the exhaustive constructors, so it cannot slip past this silently; a separate comparison key would duplicate the persistence schema.

**Files:**
- `crates/cargo-tile/src/favorites_overlay/line_plan.rs` — three-cell prefix, the four-state marker value, width budget, per-row match cached on the view
- `crates/cargo-tile/src/favorites_overlay/mod.rs` — snapshot `current_settings()` on open and on resize; the legend
- `crates/cargo-tile/src/favorites_overlay/content.rs` — the match flag on the recognized row view

**Constraints from prior phases:** Phase 9 gave the cached line a row kind carrying identity, put only real rows in the navigation index, made the selection type distinguish recognized from diagnostic, and kept selection out of the row variant as render-time state — currency joins it there. The prefix Phase 9 renders is still two cells; this phase widens it. Diagnostic rows never carry a currency mark: they have no settings to compare.

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh test cargo-tile` with tests covering all four selected/current combinations rendering distinct three-cell prefixes, every matching row marked when two rows share settings, no mark on a diagnostic row, and the width budget accounting for the wider prefix
- `bash ~/.claude/scripts/delegate/verify.sh lint cargo-tile`
- Hands-on: steer the attract screen to a saved favorite's parameters, open `ctrl-o`, see the mark.

### Phase 11 — Saving says whether it added a favorite or refreshed one  · status: todo

#### Work Order

**Goal:** Re-saving parameters that are already stored says so, instead of implying a second row was added.

**Spec:**

`FavoriteRows::push` already distinguishes the two: an exact settings match keeps the existing row's id and updates only its timestamp; a new set appends a row. The public result throws that distinction away, so both paths toast the same "saved" text, and a user who saves twice and then opens the overlay finds one row and no explanation. Nothing is wrong with the file — this is the dedup working — the confirmation is what misleads.

Return a named outcome — added or refreshed — from the save entry point, and give each its own confirmation text through the existing toast path. The branch that decides it has already done the equality comparison, so this costs one enum variant.

**Files:**
- `crates/cargo-tile/src/favorites/rows.rs` — `push` returns the named outcome
- `crates/cargo-tile/src/favorites/file.rs` — the save entry point carries it out
- `crates/cargo-tile/src/favorites/mod.rs` — export
- `crates/cargo-tile/src/globals.rs` — the `ctrl-s` handler selects the confirmation text

**Constraints from prior phases:** Phase 6 put `push` in `favorites/rows.rs` and the save entry point in `favorites/file.rs`. Phase 5 removed `schedule_timed_toast` from `globals.rs` in favor of the framework-owned toast deadline — push the confirmation through whatever that phase left in place. Phase 10 uses the same `AttractSettings` equality to decide the currency mark; the two must agree, so do not introduce a second comparison.

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh test cargo-tile` with tests proving a first save reports added, an identical second save reports refreshed and leaves one row, and the refreshed row's timestamp moved
- `bash ~/.claude/scripts/delegate/verify.sh lint cargo-tile`

### Phase 12 — A column heading carries its own value  · status: todo

#### Work Order

**Goal:** Reordering the overlay's parameter columns cannot put a value under the wrong heading.

**Spec:**

`column_descriptors(mode)` and `favorite_cells(settings)` are independent vectors matched only by index, so reordering one silently misaligns every row with no compiler complaint. Give `ParameterColumnDescriptor` the function that renders its own column's value and delete the parallel vector. It runs while rebuilding the plan, not per frame (Invariant 3).

`AttractMode::draw` (`attract/mod.rs:343`) has the same shape: it computes an index bounded by `AttractMode::ALL`, then maps `0`, `1` and everything else through a separate match, so a fourth mode would silently draw as `Pixelate`. Index `ALL` directly so adding a mode updates selection by construction. No trait and no generic — the array length is already the bound.

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
- The attract section says steering keys do not work when the screen appears on its own (`:465`–469). `keyed_mode` returns a mode when the screen was requested **or** has fully arrived at `faded == 0` (`attract/mod.rs:602`), and a regression test covers it (`:1773`).
- The template section says `AppGlobalAction` starts with no variants (`:70`–73); it now has many.

Add a favorites section beside the existing attract one, in the same voice: what a favorite stores (the attract mode and its steerable parameters, not the animation's instantaneous position); where the file lives (`<os config dir>/cargo-tile/favorites.toml`); `ctrl-s` to save and `ctrl-o` to open the table; `m` for a random favorite, `r` to randomize the current parameters, `u` to undo the last replacement; in the overlay, arrows to move, enter to load, `x` to delete, left/right to page the parameter columns, esc to close; what the three-cell row prefix means, including the currency mark; that rows this version cannot read are kept, shown as diagnostics, and can be deleted; and that saving the same parameters twice refreshes the existing row rather than adding one. State that the listed keys are defaults and can be rebound.

Also filter the `x` Dismiss row out of cargo-tile's rendered keymap. `GlobalAction::Dismiss` is bound to `'x'` as a shared framework default (`global_action.rs:70`, `:241`) that `cargo-port` relies on, while cargo-tile's tests require `x` not to close framework overlays — so the fix is in cargo-tile's keymap assembly, not in the shared default (Invariant 2 reasoning applies: do not change what the other consumer depends on).

**Files:**
- `crates/cargo-tile/README.md` — favorites section; configuration table; attract-steering paragraph; the `AppGlobalAction` claim
- `crates/cargo-tile/src/keymap.rs` — filter the inactive `x` Dismiss row from the rendered keymap

**Constraints from prior phases:** Phase 9 made diagnostic rows selectable and deletable and gave the footer capability-derived hints; Phase 10 widened the row prefix to three cells and defined the currency mark as "matches the current parameters", marking every matching row rather than one authoritative running favorite; Phase 11 named the save outcome added versus refreshed. Document what those phases actually shipped — read them before writing the section rather than describing the plan.

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh test cargo-tile` — the keymap filter has a test; the README has no test, so verify it by reading each corrected claim against the code it describes and saying so in the report
- `bash ~/.claude/scripts/delegate/verify.sh lint cargo-tile`

### Phase 14 — The second window's capture failure gets its actual cause  · status: todo

**Blocked by:** a live two-window reproduction. This phase cannot start until `cargo tile` runs in two windows of one terminal app with the Phase 1 status and the Phase 3 logging in place, and the failing stage has been recorded. No delegate can stage that reproduction.

#### Work Order

**Goal:** The second `cargo tile` in one terminal app gets its desktop capture.

**Spec:**

Running `cargo tile` in two windows of the same iTerm2 — an app that already has Screen Recording permission — leaves the second one with no desktop capture while the first keeps its own.

**There is no first-caller-wins path to find.** Every `BackdropMonitor::new` builds its own channels, workers, pinned window and cached desktop (`monitor.rs:90`–145), and capture resolves the pinned id across all visible windows (`desktop.rs:491`–494). Nothing is shared or app-keyed. What produced the *appearance* of ownership was diagnosis, not exclusivity: a monitor holding an earlier successful capture kept showing it while a newly started monitor that never got a first capture showed nothing, and both looked identical from outside.

Phases 1 and 3 make the real cause observable. Before writing any fix, run both instances and record, from outside the program, the windows the window server reports, each instance's selected window id, and the `CaptureFailure` stage the second instance logs. Reasoning from branches is what the last two backdrop defects both defeated; enumerate first.

Phase 1 already deduplicated the exclusion list, which was the standing suspect — a sibling terminal window above the selected one appeared both in the owned-windows set and the windows-above set. If the reproduction now succeeds, this phase is closed by evidence rather than by code, and that is a legitimate outcome to report.

Otherwise fix what the recorded stage names, in `tui_pane` where it lives.

**Files:**
- `crates/tui_pane/src/backdrop/desktop.rs` — window resolution, filter construction, or capture, as the evidence directs
- `crates/tui_pane/src/backdrop/monitor.rs` — window pinning and identification, if the evidence points there

**Constraints from prior phases:** Phase 1 typed the capture failure per stage and made `Desktop::capture` return `Result<Desktop, CaptureFailure>` (`desktop.rs:468`). It shipped **no preflight**: `SCShareableContent::get()` runs first, because that call is the one that raises the macOS permission prompt, and `CGPreflightScreenCaptureAccess` classifies only a query that already failed (`desktop.rs:443`–454, called at `:473`). That call answers `false` for a process never asked exactly as for one that refused, so the access variant means "not granted", not "the user said no"; in `objc2-core-graphics` 0.3.2 it is declared safe and carries no unsafe annotation. Phase 1 also deduplicated the exclusion window ids (`desktop.rs:456`, called at `:534`) and split the last successful desktop from the latest attempt status on `BackdropMonitor` (`monitor.rs:90`–145). The window id capture actually selects is still local to `desktop.rs:491`–496; whether it reaches a log at all is settled by Phase 2's pending decision on the capture boundary, and this phase's Spec asks for it by name. Phase 2 added `WindowIdentification`, whose `Fallback` means window selection gave up on pinning and fell back to frontmost or size matching — a strong candidate for what the second window hits. Phase 3 routes the failing stage and the identification outcome into cargo-tile's attract diagnostics, logging on transition only, which is where the reproduction reads them.

**Acceptance gate:**
- Both instances draw their own desktop capture, confirmed by eye in two simultaneous windows
- `bash ~/.claude/scripts/delegate/verify.sh check cargo-tile`, `lint cargo-tile`, `test tui_pane`, `check cargo-port`
- A regression test at whatever level the cause allows, or an explicit statement that the cause is a window-server interaction no unit test can reach
