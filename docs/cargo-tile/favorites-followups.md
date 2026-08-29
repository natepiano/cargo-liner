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
  - `crates/cargo-tile/src/` — `favorites/{mod,rows,file}.rs` and `favorites_overlay/{mod,content,bindings,line_plan}.rs` plus flat modules (`globals.rs`, `constants.rs`, `terminal.rs`, `render.rs`, `app.rs`, `keymap.rs`, `config.rs`) plus `attract/{mod,moving_band,moving_text,pixelate,held_key}.rs` and `theme/{mod,builtins}.rs`; `crates/cargo-tile/themes/` holds shipped theme `.toml`s.
  - `crates/cargo-port/src/tui/app/{mod,constants}.rs` — the only cargo-port files this plan touches.
  - Neither `tui_pane` nor `cargo-tile` has an `examples/` directory; no `required-features` anywhere.
- **Key files:**
  - `crates/tui_pane/src/backdrop/mod.rs` — `Backdrop` (per-cell desktop color), public re-export surface for every backdrop type including `CaptureFailure` and `BackdropStatus`; 209 lines, tests at 145.
  - `crates/tui_pane/src/backdrop/desktop.rs` — `Desktop` capture. `CaptureFailure` names the stage that failed (`:90`) and carries the two classification helpers every former swallow site now uses; `capture` returns `Result<Desktop, CaptureFailure>` (`:468`). `SCShareableContent::get()` runs first and `shareable_content_failure` (`:447`, called `:473`) classifies a failed query from `screen_capture_access_is_granted` (`:443`). Pinned-id resolution falls back to frontmost, then size (`:491`–494); the id it settles on stays local to that function (`:496`). Exclusion list deduplicated by window id through `deduplicate_windows_by_id` (`:456`, called `:534`). `reduce_capture` (`:890`, called `:567`). 1632 lines, tests at 1370 (inside `mod platform`) and 1519.
  - `crates/tui_pane/src/backdrop/monitor.rs` — `BackdropMonitor` (`:90`–145): per-instance channels and workers, `pinned: Option<u32>` (`:118`), last successful desktop (`:101`) and latest attempt status (`:103`) held separately, `status()` accessor (`:414`). `Request::window: Option<u32>` (`:154`) carries the window to capture behind. `identify() -> bool` (`:250`) returns `false` alike for exhausted attempts (`:254`), a merely-paced retry (`:261`–265), and a failed marker-title write (`:299`); the exhaustion branch (`:313`–322) restores the title and leaves `pinned` unset. The worker forwards both capture outcomes. 459 lines, no test module.
  - `crates/tui_pane/src/backdrop/band.rs` — `TravelingBand`; paints the desktop across the whole pane with both ends fading, blending through the theme like the text and pixel renderers.
  - `crates/tui_pane/src/backdrop/text.rs` — `DriftingText`; paints every cell from the backdrop (`:552`, blend `:572`) — the reference composition for the band change. 1939 lines, tests at 1036.
  - `crates/tui_pane/src/backdrop/pixels.rs` — `ResolvingPixels`; also paints every cell, `PIXEL_BEHIND_FADE`. 1474 lines, tests at 909.
  - `crates/tui_pane/src/backdrop/query.rs` — xterm position query pinning which emulator window this process draws in. 309 lines, tests at 230.
  - `crates/tui_pane/src/backdrop/constants.rs` — `TEXT_BEHIND_FADE: u8 = 128` (`:269`), `BAND_BEHIND_FADE = TEXT_BEHIND_FADE` (`:23`), `PIXEL_BEHIND_FADE` (`:194`), `CHURN_CELLS_PER_FRAME` (`:27`), `DEFAULT_BAND_SPEED` (`:30`). 532 lines.
  - `crates/tui_pane/src/toasts/toast.rs` — `Toast`, `ToastPhase`, `created_at` (`:107`), `min_height()` (`:221`), `current_visible_lines` = `floor(elapsed/line_ms)+1` clamped up to `min_height` (`:223`–238), `target_height` (`:252`), exit arithmetic (`:245`). 305 lines.
  - `crates/tui_pane/src/toasts/manager.rs` — `Toasts`, `ToastSpec`, `ToastCommand`, `active_now()`; owns push and wrapping, and `next_visual_change_deadline(now) -> ToastVisualDeadline::{NoVisualChangeScheduled, At(Instant)}` — the earliest instant any active toast can next look different, floored at `constants::FRAME_POLL_MILLIS` (8ms). Exported at the `tui_pane` crate root.
  - `crates/tui_pane/src/toasts/settings.rs` — `ToastSettings`, `animation.entrance_duration`/`exit_duration`, `ToastDuration`, `ToastPlacement`. 376 lines.
  - `crates/tui_pane/src/toasts/mod.rs` — 44 lines, the module's export list.
  - `crates/tui_pane/src/toasts/{lifecycle,body,view,slots}.rs`, `toasts/render/*` — phase transitions and expiry, wrapping width, hitboxes, slot layout, card drawing.
  - `crates/tui_pane/src/keymap/global_action.rs` — `GlobalAction::Dismiss` default bound to `'x'` (`:70`, `:241`), help text (`:122`).
  - `crates/tui_pane/src/lib.rs` — `mod backdrop` and every backdrop re-export gated `#[cfg(feature = "backdrop")]` (`:9`–62).
  - `crates/cargo-tile/src/favorites/` — the model, parser, lock and writer, split by ownership with the crate-facing exports unchanged in `mod.rs`. `rows.rs` (724 lines) holds `AttractSettings` and its exhaustive constructors, `Favorite`, `FavoriteId` (`:45`, private field, cross-module construction only through `#[cfg(test)] pub(super) const fn from_uuid_for_test` at `:49`), `UnrecognizedFavoriteValue` (`:103`), `FavoriteRowRecognition::Unrecognized { diagnostic, removal_locator }` — a **struct** variant, so every match site names its fields — `UnrecognizedFavoriteRemovalLocator` (opaque: a raw table index plus a private serialized-text fingerprint, matched by text so `nan` compares equal to itself and signed zero does not), the recognition refresh that sorts independently of the raw `tables` and demotes a duplicate-id row carrying its id, `push` settings-match dedup, serialization, and `#[cfg(test)] parse_rows_for_overlay_test`. `file.rs` holds `FavoritesLocation`, the file states, `load`, `read_rows`, `FavoriteRemovalTarget::{Recognized, Unrecognized}`, `remove(FavoriteRemovalTarget)`, the lock, the locked read-modify-write and atomic replacement. `FavoritesMutationError::UnrecognizedFavoriteChanged` is the named refusal when a locator no longer identifies exactly one raw table; the edit closure returns `Result`, so a refused removal never reaches `atomic_replace` and leaves the file byte-identical.
  - `crates/cargo-tile/src/favorites_overlay/` — the overlay, split so imports run one way: `content` and `bindings` import nothing from their siblings, `line_plan` imports from both, `mod` from all three, and nothing imports `mod`. `mod.rs` holds `FavoritesOverlay` and its state (`open(&keymap, current_parameters)` and `open_file_state(state, current_parameters, &keymap)` calling `favorites::load`, `refresh_current_parameters(current_parameters)` replacing the snapshot after a coalesced resize and setting `CachedSurfaceWidth::NeedsRebuild`, `favorites_heading(saved_count)` producing the popup border title ` Favorites -- N saved -- ● matches the current parameters `, `favorite_selection`, `close_overlay_with` taking `impl FnMut(FavoriteRemovalTarget)`), `FavoriteRemovalCommitState` (the in-flight commit guard) and the deliberately separate `FavoriteDeletionConfirmationState::{NoConfirmationArmed, AwaitingSecondPress(FavoriteRowIdentity)}` (the two-press delete question), `FavoritesOverlayNotice`, `deletion_refusal_message`, `FavoritesOverlayFrameOutcome::CommitRemoval(FavoriteRemovalTarget)` — `advance` performs no file I/O and `terminal.rs` commits both recognized and unrecognized removals — `FavoritesOverlayCloseCommit` carrying `removal_targets: Vec<FavoriteRemovalTarget>`, `handle_unmapped_key`, action dispatch, render coordination and the crate-facing re-exports — which are `FavoritesOverlayContent` and `UnrecognizedFavoritesView` only, so `FavoriteRowsView` is reached as the payload of `FavoritesOverlayContent::Rows`. `content.rs` holds `FavoritesOverlayContent`, `FavoriteRowsView` (whose `from` matches `FavoriteRowRecognition::Unrecognized { diagnostic, .. }`), `FavoriteRowView`, `UnrecognizedFavoritesView`, row lifecycle, conversion-time formatting and the private `favorite_cells`. `bindings.rs` holds `FavoritesSurfaceBindings`, whose footer is cached and derived from what the selection can actually do: `SelectedFavoriteActions::{NoFavoriteSelected, DeleteOnly, LoadAndDelete}` and the private `FavoritesFooterRequest` decide the segments, `refresh_footer(navigation_position_count, last_horizontal_column_page, selected_favorite_actions)` rebuilds them into `CachedFavoritesFooter`, and `footer(&self) -> &str` only reads. Every segment is gated on `ResolvedBinding::Bound` and a paired hint (move, page) requires both halves, because `ResolvedBinding::display_short()` returns an empty string for an unbound key (`favorites/file.rs:58`-63). `ModeColumnBindings`, `ParameterColumnDescriptor` (only `heading` is `pub(super)`; `action_names` and `separator` are private), `column_descriptors` and the private `mode_label`. `line_plan.rs` holds `CachedOverlayLine::{NonRow(Line), Row { identity, current_parameters, tail }}` where `current_parameters` is `FavoriteRowCurrentParameters::{Unrecognized, Different, Matching}` decided once per plan rebuild, with `FavoriteRowIdentity::{Recognized(FavoriteId), Unrecognized(UnrecognizedFavoriteRemovalLocator)}`, `CachedLinePlan` (`selectable_line_index` private behind a `#[cfg(test)] pub(super)` accessor, `navigation_line_index` a plain `pub(super)` field), `CachedSurfaceWidth::{NeedsRebuild, Rendered(u16)}`, `FavoriteSectionTableLayout`, `FavoriteSelection::{NoRowSelected, Row(FavoriteRowIdentity)}`, `finish_navigation` cloning `selectable_line_index` so blanks and headings are never navigable, `build_line_plan(content, current_parameters, bindings, width, horizontal_page)`, the private `FavoriteRowMarker::{Neither, Selected, Current, SelectedAndCurrent}` whose `prefix()` returns the three-cell `"   "`, `"▸  "`, `" ● "` or `"▸● "` and whose `is_selected()` drives the highlight, `rendered_line` with its three-cell marker prefix — whose width every column budget takes from `constants::FAVORITE_ROW_PREFIX_WIDTH`, never a literal — which fades an unrecognized row from `error_color()` and a recognized one from `text_default()` toward `attract::ground()` — the `Attract:` section heading and `append_unrecognized` emitting identified `Row` lines that join `selectable_line_index` while the row is `Active`. Locate items by name; the split moved every line.
  - `crates/cargo-tile/src/globals.rs` — app-globals scope; toasts pushed with `app.framework.toasts.push_timed(...)` (`:149`, `:227`) and no paired scheduling call; the overlay-open handler (`:116`), which snapshots `app.attract.current_settings().into()` into `OpenFavoritesCurrentParameters` before opening — as does the fall-through path of `show_random_favorite_with`, the success path having already applied the settings; private `mode_label`.
  - `crates/cargo-tile/src/constants.rs` — `ATTRACT_NO_BACKDROP_NOTICE` (`:39`), `ATTRACT_FRAME_INTERVAL` (`:346`), `PROBE_THRESHOLD` (`:354`), `FAVORITE_ROW_PREFIX_WIDTH` (the favorites row's selection, currency and separator cells, value 3) and the separate `CURSOR_WIDTH` (value 2), which `settings.rs` also reads and which the favorites prefix therefore does not share. No test module; locate constants by name — this file is edited often enough that its line numbers drift.
  - `crates/cargo-tile/src/terminal.rs` — input dispatch ladder. The local toast-scheduling types are gone; the framework deadline supplies the repaint cadence. The overlay consumes every key in the modal branch of `handle_key`, which calls `handle_unmapped_key()` (`:526`) so any unmapped key cancels a delete confirmation, and `:291` commits `FavoritesOverlayFrameOutcome::CommitRemoval` for both recognized and unrecognized rows; `Event::Resize` records a pending area only (`:459`) and the post-drain `Resized::Yes` block acts on it (`:268`), calling the private `refresh_open_favorites_after_resize(app)` (`:468`) before `force_repaint` so an open favorites modal re-snapshots the re-clamped attract parameters; `keyed_mode` caller and `Dismiss` dispatch follow.
  - `crates/cargo-tile/src/app.rs` — `App` state; `AppOverlay::{Closed, Favorites(OpenFavoritesOverlayState)}`, where `OpenFavoritesOverlayState` pairs the overlay content with `OpenFavoritesCurrentParameters` — a copy type wrapping `AttractSettings`, with `matches(attract_settings)` and `From<AttractSettings>`, so a caller cannot pass raw settings where a snapshot is meant. The `toast_visual_schedule` field and `schedule_timed_toast` are deleted; toast repaint timing comes from `tui_pane`.
  - `crates/cargo-tile/src/attract/mod.rs` — `AttractSettings`, `current_settings()` (`:697`), `AttractMode::draw` index-then-match (`:427`), `noted_backdrop: BackdropDiagnostic` written on transition in `identify` (`:964`), `keyed_mode` (`:680`), `backdrop_notice(now) -> BackdropNotice` (`:1227`), `render` passing one `Backdrop` to all three renderers (`:1251`), automatic-attract steering regression test in the test module. 2120 lines, tests at 1280.
  - `crates/cargo-tile/src/attract/{moving_band,moving_text,pixelate,held_key}.rs` — key bindings only; none of them render.
  - `crates/cargo-tile/src/render.rs` — the pane background the band currently falls back to; `draw_backdrop_notice` (`:216`) writes the attract notice on the body's last row, called from the attract branch (`:184`). 3178 lines.
  - `crates/cargo-tile/src/keymap.rs` — keymap assembly; `x` dismiss arrives from `GlobalAction` defaults.
  - `crates/cargo-tile/src/config.rs` — `<os config dir>/cargo-tile/` paths.
  - `crates/cargo-tile/README.md` — 475 lines; `AppGlobalAction` "starts with no variants" (`:70`–73), `## configuration` "Three files" (`:75`–78), `### keys` (`:430`), attract-keys claim (`:465`–469).
  - `crates/cargo-port/src/tui/app/mod.rs` — `animation_timeout` takes the framework toast deadline as a minimum against its 80ms `ANIMATION_TICK`; `is_animating` no longer reports true merely because a toast exists.
  - `crates/cargo-port/src/tui/app/constants.rs` — `ANIMATION_TICK: Duration = Duration::from_millis(80)` (`:3`).
- **Build:** `bash ~/.claude/scripts/delegate/verify.sh check <pkg>`
- **Test:** `bash ~/.claude/scripts/delegate/verify.sh test <pkg>`
- **Lint:** `bash ~/.claude/scripts/delegate/verify.sh lint <pkg>`
- **Style:** `run-end /clippy style-only auto-proceed`
- **Invariants:**
  1. **`verify.sh` cannot compile `tui_pane`'s backdrop code.** `tui_pane`'s `default = ["clipboard"]`; `backdrop` is opt-in and `verify.sh` composes no `--features`. So `check|test|lint tui_pane` build with backdrop **off** and never see `backdrop/**` or run its `#[cfg(test)]` modules. Only `cargo-tile` enables the feature (`crates/cargo-tile/Cargo.toml:31`), so **every backdrop phase gates through `check cargo-tile` and `lint cargo-tile`**, and any behavior that must actually *run* per phase needs a cargo-tile-side test driving the framework API. New `tui_pane` unit tests under `backdrop/` compile but do not execute until the final workspace gate — a phase adding them says so and names them.
  2. **`cargo-port` does not enable `backdrop`.** It takes default features. A backdrop change must prove (a) `tui_pane` still builds and tests green with the feature off — no `use` or re-export may leak outside `#[cfg(feature = "backdrop")]` — and (b) `cargo-port` still checks and tests. It must **not** claim cargo-port exercises the capture path; cargo-port cannot reach it.
  3. **Attract cadence is `ATTRACT_FRAME_INTERVAL = 33ms`** (`cargo-tile/src/constants.rs:346`). Every per-frame path — full-area band painting, footer rendering, currency marks — fits inside it: no second capture, no reduction, no per-frame allocation. Work done on load, on overlay open, or on resize is not bound by it.
  4. **Workspace lints bind both crates** (`[lints] workspace = true`). `clippy::all`/`cargo`/`nursery`/`pedantic` denied at priority -1, plus `unwrap_used`, `expect_used`, `panic`, `unreachable`, `allow_attributes_without_reason`, `undocumented_unsafe_blocks`, `self_named_module_files`. `missing_docs = "deny"` and `unsafe_code = "deny"` in `[workspace.lints.rust]`. Consequences: a module with submodules is `foo/mod.rs`, never `foo.rs` beside `foo/`; every new public and module item carries a doc comment; an FFI call opts back into `unsafe` with a reasoned `#[expect]`. Test modules opt back in with `#[expect(clippy::expect_used, clippy::panic, reason = "…")]` — the pattern every test module in these crates uses. A test module carries **only** the lints its own module actually trips: an `#[expect]` that never fires is itself an error under denied warnings, so copying a wider block from a sibling breaks the build. Moved tests carry their own block with them.
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

### Phase 6 — Split `favorites.rs` by ownership  · status: done

#### As-built

`favorites` is a directory module of three files with an unchanged crate-facing surface: `mod.rs` declares the submodules and re-exports the same sixteen `pub(crate)` names the flat file exported, so no caller outside the module names a new path. `rows.rs` owns the model and its in-memory transformations — `AttractSettings` and its exhaustive constructors, `Favorite`, `FavoriteId`, `FavoriteRows`, `FavoriteRowRecognition`, `UnrecognizedFavoriteValue`, `recognize_favorite`, `compare_recognitions`, `table_from_favorite`, the recognition refresh that sorts independently of the raw `tables` and demotes a duplicate-id row carrying its id, and the row-level mutations. `file.rs` owns everything that touches the file — `ResolvedBinding`, `FavoritesFileState`, `FavoritesMutation`, `FavoritesMutationError`, `FavoritesRetryInstruction`, `favorite_refusal_message`, `load`, `push`, `remove`, `read_rows`, `acquire_lock`, `edit_at_location`, and `atomic_replace`. `FavoriteId`'s inner `Uuid` is private; `#[cfg(test)] pub(super) const fn from_uuid_for_test(uuid: Uuid) -> Self` on `FavoriteId` is the only cross-module construction path, consumed solely by `file.rs`'s test module. Tests live beside the code they cover, each test module carrying its own `#[expect(clippy::expect_used, clippy::panic, …)]` block; `parse_rows_for_overlay_test` keeps its name and stays `#[cfg(test)]` beside the parse code.

**Files:**
- `crates/cargo-tile/src/favorites/mod.rs` — submodule declarations and the crate-facing re-exports
- `crates/cargo-tile/src/favorites/rows.rs` — model, recognition, sorting, serialization, row mutations
- `crates/cargo-tile/src/favorites/file.rs` — file states, `push`/`remove` entry points, lock, read-modify-write, atomic replacement
- `crates/cargo-tile/src/favorites.rs` — deleted; `self_named_module_files` is denied, so the module is `favorites/mod.rs` and the flat file cannot survive beside the directory

**Binds later work:** `FavoriteId` is opaque outside `favorites` — anything locating a row by id goes through `rows.rs`, not around the newtype. The row model and the file's read-modify-write path are now in separate files, so work naming either reaches it by item name; line numbers quoted from the pre-split `favorites.rs` no longer resolve.

**Gotchas:** Splitting a file turns same-file access into cross-module access, and the reflex fix — widening a field or item — is permanent, production-visible cost paid for a compile error that only test code raised. Any widening a split forces gets checked; if only tests need it, the accessor is `#[cfg(test)]`-gated instead.

**Ruled out:** Deduplicating the test fixture constants `FIRST_ID`, `FIRST_SAVED`, `SECOND_SAVED`, defined in both test modules — each copy is used only inside its own module's fixtures, so divergence cannot change a result, and merging them buys a shared test module for three string literals.

### Phase 7 — An unrecognized row gets a locator that survives a concurrent edit  · status: done

#### As-built

A favorites row the parser could not read can be named precisely enough to delete, and a row that moved underneath the user is refused rather than deleted wrongly. `UnrecognizedFavoriteRemovalLocator` (`rows.rs`) is opaque: a raw table index plus a private serialized-text fingerprint, created during the recognition refresh before the display sort and carried on the recognition. Its `locate` trusts the recorded index the moment that table's content still matches, and otherwise falls back to a content search that resolves only when exactly one table matches. `FavoriteRowRecognition::Unrecognized { diagnostic, removal_locator }` is a struct variant, so every match site names its fields. `favorites::remove` takes `FavoriteRemovalTarget::{Recognized(FavoriteId), Unrecognized(locator)}` (`file.rs`) — no shim and no second entry point; both existing callers were moved to it. A locator that no longer identifies exactly one raw table yields `FavoritesMutationError::UnrecognizedFavoriteChanged`, and `edit_at_location`'s closure returns `Result` so a refusal aborts before `atomic_replace` and leaves the file byte-identical. Deleting an unrecognized row removes its whole raw `[[favorite]]` table. `FavoriteId` stays opaque: private field, `#[cfg(test)]` constructor only.

**Files:**
- `crates/cargo-tile/src/favorites/rows.rs` — the locator type and its fingerprint, created and inspected here; the unrecognized struct variant
- `crates/cargo-tile/src/favorites/file.rs` — `FavoriteRemovalTarget`, the unrecognized removal path, re-verification under the existing lock, the refusal error
- `crates/cargo-tile/src/favorites/mod.rs` — exports the new types
- `crates/cargo-tile/src/terminal.rs` — removal commit path, on the target type
- `crates/cargo-tile/src/favorites_overlay.rs` — close path passing `remove` as a function pointer, on the target type; names the struct variant's fields at its match site

**Binds later work:** The struct variant and the opaque locator are the surface the unrecognized-row deletion UI builds on. `UnrecognizedFavoriteChanged` is user-actionable and already flows through `favorite_refusal_message`, so it surfaces as a refusal toast wherever a removal is refused; a consumer distinguishes it from an I/O failure by matching the variant, never its message text. The locator is created on load and checked only during deletion, so it costs nothing per frame (Invariant 3).

**Gotchas:**
- Identity rests on serialized text, not on comparing parsed `toml::Table`s: `toml::Value::Float` wraps `f64`, so a row containing `nan` never equals its own snapshot (permanently undeletable) while a `-0.0` to `0.0` edit compares equal (a changed row deleted instead of refused). Both directions break silently.
- The ambiguity scan belongs only on the content-search fallback. A staleness check applied when nothing is stale is itself a defect — running the duplicate scan while the recorded index still holds the expected row makes two byte-identical broken rows block each other forever, with no concurrent edit anywhere.

**Ruled out:** A cloned `toml::Table` compared with `==` as the locator's fingerprint; deleting an unrecognized row by its id, which deletes a duplicate-id row's recognized twin instead; a temporary shim or deprecated alias for the old `remove` signature.

### Phase 8 — Split `favorites_overlay.rs` with its dependency direction stated  · status: done

#### As-built

`favorites_overlay` is a directory module of four files whose imports run one way: `content` and `bindings` import nothing from their siblings, `line_plan` imports from both, `mod` from all three, and nothing imports `mod`. The split is a move only — no signature or behavior changes; the 34 overlay tests moved with the code they cover and pass unedited. Visibility widened only where a sibling actually reads: `heading` on `ParameterColumnDescriptor`, and `lines` / `navigation_line_index` / `last_horizontal_column_page` on `CachedLinePlan`. A widening only test code needs is a `#[cfg(test)]` accessor rather than a permanent production widening.

**Files:**
- `crates/cargo-tile/src/favorites_overlay/mod.rs` — `FavoritesOverlay`, its state and outcome types, `FavoriteRemovalCommitState`, `FavoritesOverlayNotice`, action dispatch, render coordination, the crate-facing re-exports, and the cross-cutting tests
- `crates/cargo-tile/src/favorites_overlay/content.rs` — `FavoritesOverlayContent`, `FavoriteRowsView`, `FavoriteRowView`, `UnrecognizedFavoritesView`, row lifecycle, conversion-time formatting, and the private `favorite_cells`
- `crates/cargo-tile/src/favorites_overlay/bindings.rs` — `FavoritesSurfaceBindings`, `ModeColumnBindings`, `ParameterColumnDescriptor`, `column_descriptors`, `footer`, `mode_label`
- `crates/cargo-tile/src/favorites_overlay/line_plan.rs` — `CachedOverlayLine`, `CachedLinePlan`, `CachedSurfaceWidth`, `FavoriteSectionTableLayout`, `finish_navigation`, `build_line_plan`, `rendered_line`, `append_unrecognized`

**Binds later work:** `favorite_cells` is private to `content.rs` beside the row constructor that calls it — moving it to `line_plan` would make `content` depend on `line_plan`, which already depends on `content`. `ParameterColumnDescriptor` exposes only `heading` to siblings; `action_names` and `separator` are private, so a field a sibling must read is widened deliberately or reached through a `pub(super)` method. `CachedLinePlan::selectable_line_index` is private behind a `#[cfg(test)] pub(super)` accessor, while `navigation_line_index` is a plain `pub(super)` field. `FavoriteRowsView` has no `crate::favorites_overlay::` path; it is reached as the payload of the re-exported `FavoritesOverlayContent::Rows`.

**Gotchas:** Each module's test module carries only the lints that module actually trips — `clippy::expect_used` for the three leaves, plus `clippy::panic` and `clippy::unchecked_time_subtraction` for `mod.rs`. Warnings are denied and an unfulfilled `#[expect]` is itself an error, so copying the wider block into a leaf breaks the build.

**Ruled out:** Restoring the `pub(crate)` re-export of `FavoriteRowsView` — nothing imports that path, so it needs a permanent `#[expect(unused_imports)]`. Treating rustfmt's rewrap of `saved_count`'s body as a defect — it is the formatter reacting to the added `pub(super) ` prefix.

### Phase 9 — Rows know what they are, and the footer offers only what will work  · status: done

#### As-built

Every line in the favorites overlay says what it is. A row carries
`FavoriteRowIdentity::{Recognized(FavoriteId), Unrecognized(UnrecognizedFavoriteRemovalLocator)}`
as `CachedOverlayLine::Row { identity, tail }`; a blank or a heading is
`CachedOverlayLine::NonRow` and carries no identity. Only real rows enter
`selectable_line_index`, and `finish_navigation` is a plain clone of it, so the
cursor cannot land on a blank line or a section heading. `FavoriteSelection::{NoRowSelected,
Row(FavoriteRowIdentity)}` carries a row identity or nothing, and `rendered_line`
draws the marker and selection style on unrecognized rows exactly as on recognized
ones, fading an unrecognized row from `error_color()` and a recognized one from
`text_default()` toward `attract::ground()`.

Deleting takes two presses. `FavoriteDeletionConfirmationState::{NoConfirmationArmed,
AwaitingSecondPress(FavoriteRowIdentity)}` answers "has the user asked twice?" and is
separate from `FavoriteRemovalCommitState`, which guards a commit already in flight;
they clear on different events. Any action that is not `Delete` cancels the
confirmation, including a key the overlay does not map, which reaches it through
`handle_unmapped_key()`. Delete reaches both kinds of row through
`FavoriteRemovalTarget`; load stays recognized-only and refuses on an unrecognized row.

The overlay writes no files. `FavoritesOverlayFrameOutcome::CommitRemoval(FavoriteRemovalTarget)`
carries the target out to `terminal.rs`, which commits both kinds, and
`FavoritesOverlayCloseCommit` carries `removal_targets: Vec<FavoriteRemovalTarget>`.
A refusal from locator re-verification reaches the user through
`deletion_refusal_message`, which matches `FavoritesMutationError::UnrecognizedFavoriteChanged`
by variant rather than by message text.

The footer offers only what will work. `SelectedFavoriteActions::{NoFavoriteSelected,
DeleteOnly, LoadAndDelete}` and the private `FavoritesFooterRequest` decide its segments;
`refresh_footer(navigation_position_count, last_horizontal_column_page,
selected_favorite_actions)` rebuilds it into `CachedFavoritesFooter` and
`footer(&self) -> &str` only reads, so no `String` is built per frame.

**Files:**
- `crates/cargo-tile/src/favorites_overlay/line_plan.rs` — row identity, the cached line variants, the navigation index, selection styling and the removal fade.
- `crates/cargo-tile/src/favorites_overlay/bindings.rs` — the capability-derived cached footer and its request type.
- `crates/cargo-tile/src/favorites_overlay/mod.rs` — the selection type, the two removal states, delete routing by identity, the load refusal and the refusal message, `handle_unmapped_key`.
- `crates/cargo-tile/src/favorites_overlay/content.rs` — the removal locator carried onto the unrecognized row view.
- `crates/cargo-tile/src/terminal.rs` — commits `CommitRemoval` for both kinds of row and routes unmapped modal keys into the overlay.

**Binds later work:** the row marker for the running parameters keys off
`FavoriteRowIdentity`, not `FavoriteId`, because an unrecognized row has no id;
`build_line_plan` takes its content immutably and `rendered_line` receives only an
already-cached line, so per-row state is cached on the row rather than written back to a
view. Headings and blanks are `NonRow` and are excluded from navigation by construction.
The overlay notice and `deletion_refusal_message` are the pattern for surfacing a
mutation outcome, and `FavoritesMutationError` is matched by variant. Two-press delete
with any-key cancellation is user-visible behavior that the README must describe.

**Gotchas:** `ResolvedBinding::display_short()` returns an empty string for `Unbound`
(`favorites/file.rs:58`-63), so text built from a binding must match `ResolvedBinding::Bound`
first and a paired hint needs both halves bound, or it renders as a bare separator.
`CURSOR_WIDTH` (`constants.rs:206`) is shared with the settings pane (`settings.rs:19`, `:90`)
and is not a favorites-only constant. A test module inherits no lint expectations:
`line_plan.rs`'s carries `clippy::panic` beside `clippy::expect_used` because a test there
needs it, and warnings are denied so an unfulfilled `#[expect]` is itself an error.

**Ruled out:** a three-variant row kind including `static` or `diagnostic` — `static`
describes rendering rather than a state a row can be in, and `diagnostic` is untrue for a
row a newer cargo-tile may have written; collapsing the confirmation state and the
in-flight commit guard into one field.

### Phase 10 — A row that matches the running parameters is marked  · status: done

#### As-built

The favorites table marks every saved row whose parameters equal what the attract screen is currently running. `rendered_line` writes a three-cell prefix chosen by a private `FavoriteRowMarker::{Neither, Selected, Current, SelectedAndCurrent}` whose `prefix()` returns `"   "`, `"▸  "`, `" ● "` or `"▸● "` and whose `is_selected()` drives the row highlight, so selection and currency are one value and cannot disagree. The popup's border title reads ` Favorites -- N saved -- ● matches the current parameters `, built by `favorites_heading(saved_count)`.

The comparison happens once per plan rebuild, never per frame. `AppOverlay::Favorites` carries an `OpenFavoritesOverlayState` pairing the overlay content with `OpenFavoritesCurrentParameters` — a copy type wrapping `AttractSettings`, with `matches(attract_settings)` and `From<AttractSettings>`, so raw settings cannot be passed where a snapshot is meant. `build_line_plan(content, current_parameters, bindings, width, horizontal_page)` decides `FavoriteRowCurrentParameters::{Unrecognized, Different, Matching}` for each row and stores it on `CachedOverlayLine::Row` beside the identity. Unrecognized rows are never current: they have no settings to compare. The equality is the derived `PartialEq` on `AttractSettings`, the same one `FavoriteRows::push` uses to recognize a repeat save.

Both production open paths snapshot `app.attract.current_settings().into()` before opening: the `AppGlobalAction::OpenFavorites` handler and the fall-through path of `show_random_favorite_with`, whose success path has already applied the settings. A terminal resize reclamps the attract parameters, so the post-drain `Resized::Yes` block calls `refresh_open_favorites_after_resize(app)`, which re-snapshots into an open modal through `refresh_current_parameters` — once per coalesced burst, not once per resize event.

**Files:**
- `crates/cargo-tile/src/app.rs` — `AppOverlay::{Closed, Favorites(OpenFavoritesOverlayState)}`, `OpenFavoritesOverlayState`, `OpenFavoritesCurrentParameters`
- `crates/cargo-tile/src/favorites_overlay/line_plan.rs` — `FavoriteRowCurrentParameters`, the private `FavoriteRowMarker`, `CachedOverlayLine::Row { identity, current_parameters, tail }`, `CachedSurfaceWidth::{NeedsRebuild, Rendered(u16)}`, `build_line_plan`'s snapshot argument
- `crates/cargo-tile/src/favorites_overlay/mod.rs` — the snapshot-taking `open` and `open_file_state`, `refresh_current_parameters`, `favorites_heading`
- `crates/cargo-tile/src/globals.rs` — both production open paths snapshot
- `crates/cargo-tile/src/terminal.rs` — `refresh_open_favorites_after_resize`, called from the post-drain resize block
- `crates/cargo-tile/src/constants.rs` — `FAVORITE_ROW_PREFIX_WIDTH: usize = 3`
- `crates/cargo-tile/src/interaction.rs` — its test opener passes the snapshot

**Binds later work:** `build_line_plan` takes `current_parameters: &OpenFavoritesCurrentParameters` as its second argument and the per-row match is decided there, cached on the row, and never recomputed in `rendered_line`. Every favorites column budget takes the prefix width from `constants::FAVORITE_ROW_PREFIX_WIDTH`, not a literal — `visible_parameter_columns` and `format_table_line` both do. `CURSOR_WIDTH` (value 2) stays separate because `settings.rs` reads it for the settings pane. The legend text and the four prefix strings are asserted by tests and documented in the README.

**Gotchas:** `refresh_current_parameters` must also set `CachedSurfaceWidth::NeedsRebuild`; without it the resize path rebuilt the plan twice, once with the stale snapshot. `CachedSurfaceWidth::NeedsRebuild` is named to match `CachedFavoritesFooter::NeedsRebuild` in the sibling `bindings.rs`, which already meant exactly this. `Attract::current_settings()` takes `&mut self`, which is why the snapshot is taken in `globals.rs` and `terminal.rs` rather than inside the overlay. The popup's width comes from the terminal, so the ~57-cell title truncates below roughly 61 columns and the legend is the first thing lost.

**Ruled out:** a single authoritative "the favorite that is running" — it would mean carrying `FavoriteId` provenance through load, steer, randomize, undo and save and clearing it on every edit, a larger feature; two booleans or a tuple for selected/current, which could disagree; an `Option<AttractSettings>` on the outer controller, where "no overlay open" and "overlay open, no snapshot" collapse into one value; renaming or retuning `CURSOR_WIDTH`, which `settings.rs` shares; snapshotting inside the `Event::Resize` arm, which would recompute on every event of a drag burst and read settings not yet reclamped.

### Phase 11 — Saving says whether it added a favorite or refreshed one  · status: done

#### As-built

Saving a parameter set reports which of two things happened. `FavoriteRows::push(&Favorite)` returns `FavoriteSaveOutcome::{Added, Refreshed}` — `Refreshed` when an existing row's settings compare equal and only its timestamp is rewritten, `Added` when a row is appended — and `favorites::push(AttractSettings)` carries that outcome out through `push_to_location`. `push` no longer rebuilds and returns a `Favorite`; the outcome is the whole result.

`save_favorite` in `globals.rs` renders one confirmation per outcome, both naming the attract mode through `mode_label`: `Favorite added` with "<mode> parameters added to favorites", and `Favorite refreshed` with "<mode> parameters were already saved, so the existing favorite's timestamp was refreshed rather than a second row being added".

**Files:**
- `crates/cargo-tile/src/favorites/rows.rs` — `FavoriteSaveOutcome`, returned by `push`
- `crates/cargo-tile/src/favorites/file.rs` — `push` and `push_to_location` carry the outcome and take `&Favorite`
- `crates/cargo-tile/src/favorites/mod.rs` — exports `FavoriteSaveOutcome`
- `crates/cargo-tile/src/globals.rs` — the `ctrl-s` handler picks the confirmation

**Binds later work:** the dedup comparison is still the derived `PartialEq` on `AttractSettings`, the same equality Phase 10's currency mark uses; the two must keep agreeing, so no second comparison, normalization, or epsilon. The toast bodies are the wording the README documents.

**Gotchas:** `push` takes `&Favorite` because `edit_at_location`'s closure cannot move a captured value. `save_favorite` reads `settings.mode()` before handing the settings to `push`, since the outcome no longer carries them back.

**Ruled out:** a boolean or an `Option` for the outcome — both answers are ordinary successes and each renders different text; a second comparison key for dedup, which would duplicate the persistence schema.

### Phase 12 — A column heading carries its own value  · status: done

#### As-built

`ParameterColumnDescriptor` carries `value_renderer: fn(AttractSettings) -> String` beside its `heading`, reached through `pub(super) fn render_value(self, AttractSettings) -> String` so the private field stays private. `BAND_COLUMNS`, `TEXT_COLUMNS` and `PIXEL_COLUMNS` each pair a heading with the renderer for its own column, so reordering a table moves the value with its heading and no index-matched second vector exists to fall out of step.

`favorite_cells` and the `cells: Vec<String>` field on `FavoriteRowView` are gone. `line_plan.rs` derives both the measured column widths and the rendered row tail from the section's descriptors and `row.settings`; `measured_parameter_widths` and `append_section` both take `column_descriptors(section.mode)`, and `FavoriteRowsView::from` keys each section by `favorite.settings.mode()`, so a renderer never receives another mode's settings.

The six `const fn` value-name helpers — `direction_name`, `fraying_name`, `drift_name`, `text_fill_name`, `pixel_resolve_name`, `pixel_fill_name` — live in `bindings.rs` beside the renderers that call them, keeping the split's dependency direction: `bindings` imports nothing from `content`.

`AttractMode::draw` indexes `AttractMode::ALL` directly rather than mapping indices through a separate match, so a mode added to `ALL` is drawable by construction instead of needing a second edit.

**Files:**
- `crates/cargo-tile/src/favorites_overlay/bindings.rs` — the three descriptor tables, the per-column value renderers, the six value-name helpers, and the regression proving a reordered table keeps each heading with its value
- `crates/cargo-tile/src/favorites_overlay/content.rs` — row views carrying settings and timestamp only, with no rendered cells
- `crates/cargo-tile/src/favorites_overlay/line_plan.rs` — renders and measures every column through its descriptor
- `crates/cargo-tile/src/favorites_overlay/mod.rs` — imports the moved helpers from `bindings`; the load regression asserts against the selected row's settings rather than a mutated display cell
- `crates/cargo-tile/src/attract/mod.rs` — `draw` indexes `ALL`, with a regression that every mode declared in `ALL` is reachable

**Gotchas:**
- A column's value is rendered twice per plan rebuild, once to measure its width and once to draw it. Caching it back onto the row restores exactly the parallel array this phase deleted. The cost is bounded by the rebuild (Invariant 3) and by `CachedSurfaceWidth::Rendered`, never per frame.
- `AttractMode::ALL[index]` cannot panic because `INDEX_BOUND` is `NonZeroIndexBound::try_from_len(Self::ALL.len())` and `random::bounded_index` returns `candidate % bound`. Decoupling the bound from the array length reintroduces an index panic.
- A renderer handed another mode's settings returns a diagnostic string rather than panicking, because the workspace denies `panic`, `unwrap` and `expect` and the arm is unreachable by construction rather than by type.

**Ruled out:**
- Caching the rendered cells back onto the row view to avoid rendering twice — it restores the index-matched parallel array.
- A trait or generic over the descriptor tables — the array length is already the bound.

### Phase 13 — A framework shortcut can be hidden, and a refused delete says what to do  · status: done

#### As-built

`tui_pane` carries a client-supplied presentation policy: `FrameworkGlobalShortcutVisibility::{Shown, Hidden}` and `FrameworkGlobalShortcutPresentation`, a `Clone + Copy` wrapper over `fn(GlobalAction) -> FrameworkGlobalShortcutVisibility` built with `FrameworkGlobalShortcutPresentation::new`. Both are `pub` and re-exported from the crate root beside `GlobalShortcutRow`; the reader `FrameworkGlobalShortcutPresentation::visibility` is `pub(super)`, so a client installs a policy but never evaluates one.

The policy is a non-optional builder field carried through `Configuring` → `Registering` and moved onto `Keymap` by `finalize`; `KeymapBuilder::framework_global_shortcut_presentation` is a `const fn` settings-phase method. It defaults to showing every action, so a client that sets none is unchanged. `Keymap::global_shortcut_rows` filters both the navigation pair and the remaining `GlobalAction::ALL` through it; `Keymap::keymap_help_rows` deliberately does not, so a hidden action still dispatches and stays rebindable in the full editor.

cargo-tile installs a policy hiding `GlobalAction::Dismiss` from its compact shortcut popup. `deletion_refusal_message` ends the changed-file refusal with `Close and reopen favorites, then try again.`

**Files:**
- `crates/tui_pane/src/keymap/global_action.rs` — the two types and the show-everything default
- `crates/tui_pane/src/keymap/mod.rs` — the policy field, its `pub(super)` setter, and the filter in `global_shortcut_rows`
- `crates/tui_pane/src/keymap/builder/{mod,transition,finalize}.rs` — the builder field, its settings-phase method, the typestate carry, and the install
- `crates/tui_pane/src/lib.rs` — crate-root exports
- `crates/tui_pane/src/overlays/keymap_edit.rs` — a test module proving compact-row selection still resolves in the editor after a row is hidden
- `crates/cargo-tile/src/keymap.rs` — cargo-tile's policy and its no-dismiss-row assertion
- `crates/cargo-tile/src/favorites_overlay/mod.rs` — the recovery sentence and its exact-text regression

**Binds later work:** cargo-tile's compact shortcut popup no longer lists the `x` Dismiss row, so documentation must not present `x` as a cargo-tile global — needed by **The README documents favorites and stops contradicting the code**. The key stays bound and rebindable.

**Gotchas:**
- `edit_selected_global_shortcut` resolves the compact selection into the editor by `(scope, action)`, never by index, which is the only reason filtering rows cannot open the wrong entry.
- The compact overlay's viewport length comes from the same filtered `global_shortcut_rows()` it renders, so a hidden row cannot leave the selection out of bounds.
- Hiding an action is safe in cargo-tile specifically because it declares `ToastAction = NoToastAction` and registers no dismiss fallback, leaving the dismiss chain nothing to run outside an overlay. An app with actionable toasts must recheck that before copying the policy.
- `cargo-port` supplies no policy and must keep showing and dispatching `GlobalAction::Dismiss` exactly as it does today.

**Ruled out:**
- Spelling the policy as `Option<fn(..)>` or `bool` — a named two-variant type instead, so "no policy" and "a policy that hides nothing" are not two spellings of one behavior.
- Filtering `keymap_help_rows` as well — hiding a row from the quick reference must never make it unrebindable.
- Changing the shared `'x'` → `GlobalAction::Dismiss` framework default, which `cargo-port` relies on.

### Phase 14 — The README documents favorites and stops contradicting the code  · status: done

#### As-built

`crates/cargo-tile/README.md` carries a `#### favorites` subsection beside the attract documentation: what a favorite stores (the attract mode and its steerable parameters, not the animation's instantaneous position), where the file lives, the keys that save, open, load, randomize, undo and delete one, what the three-cell row prefix and the `● matches the current parameters` legend mean, how unreadable rows are kept and deleted, that saving identical parameters refreshes the existing row, and that every listed key is a rebindable default.

Four statements beside it are corrected. `## configuration` reads "Four optional configuration entries" and lists `favorites.toml`. The `AppGlobalAction` paragraph describes it as cargo-tile's populated global-shortcut enum rather than an empty one. The attract steering paragraph states `keyed_mode`'s real rule — immediate when the screen was requested, on full arrival when it came on by itself, with the grid keeping the keyboard during an automatic fade. The globals table's `` `x` / Esc `` dismiss row is gone. The band-fraying paragraph names all four `BandFraying` modes and states the two real depth bounds instead of the trailing-edge-only range it inherited.

**Files:**
- `crates/cargo-tile/README.md` — the durable explanation of favorites, and the corrected source for the configuration file set, the `AppGlobalAction` role, the attract steering rule, and the global shortcut table

**Gotchas:**
- The globals table's header promises every row comes from `tui_pane::GlobalAction`'s defaults, so any row added there must be checked against `keymap/global_action.rs`. Esc is not among those defaults (`:122`, `:293`); only `'x' => Self::Dismiss` is, and cargo-tile hides even that from its compact popup.
- Band fraying depth is bounded by `VARIABLE_TAIL_FLOOR_PERCENT = 30` and `VARIABLE_HEAD_CEILING_PERCENT = 20` (`tui_pane/src/backdrop/constants.rs`), enforced by a `const _: () = assert!(HEAD < TAIL)`. A trailing edge never eats into the last third; a leading edge stands back at most a fifth. Prose stating one range for all four modes is false for three of them.
- A section heading inserted here rescopes the paragraph below it without changing a word of that paragraph. The `keymap.toml` hand-editing paragraph must stay above the favorites heading, or it reads as a statement about `favorites.toml`.
- Unreadable favorites rows are not diagnostics: such a row may simply have been written by a newer cargo-tile, and the README says so.

**Ruled out:**
- Documenting the favorites overlay's vim aliases `k`/`j`/`h`/`l` — bound in `favorites_overlay/mod.rs`, but only the arrows were specified, and widening the documented key set was not this phase's call.

### Phase 15 — Every capture attempt records the window it aimed at  · status: todo

#### Work Order

**Goal:** Every completed desktop-capture attempt records which window it targeted and how that window was chosen, so an instance that failed before choosing one can be told from an instance that chose one and then failed.

**Spec:**

Phase 3 logs one `backdrop:` line on transition carrying `report` (the `WindowIdentification`), `capture_status` (the `BackdropStatus`, whose failed form names the `CaptureFailure` stage) and `captured_window_id` (the `LastSuccessfulCaptureWindowId`). None of the three answers the question the next phase turns on. `captured_window_id` reads `WaitingForFirstSuccess` for an instance that has never captured, which is exactly the instance under investigation, and `WindowIdentification::Fallback` says selection fell back without saying to what. `Desktop::capture` keeps the window id it settles on local to that function and returns only `Desktop` or `CaptureFailure`, and several failure stages happen *before* any window has been selected — so "which window did this instance try to capture behind, and did it get that far?" is currently unanswerable from the data path.

Build that diagnostic. It is this phase's whole deliverable; the live reproduction it exists to serve is the next phase and is blocked on a human.

**Three types, each carrying one distinction:**

1. `CaptureAttemptWindowSelection::{SelectionNotReached, Selected { window_id: u32, method: CaptureWindowSelectionMethod }}` — what a **completed** attempt selected. Not an `Option<u32>`: "failed before choosing a window" and "chose this window and then failed" are the two answers the reproduction turns on, and an option collapses them.
2. `CaptureWindowSelectionMethod::{PinnedWindow, ClosestSizeMatch { candidates: TerminalWindowCandidateSource }}` — how the id was reached — with `TerminalWindowCandidateSource::{ProcessAncestry, TerminalProgramName, FrontmostApplication}` naming where the candidate set came from. Its own enums rather than strings or integers, so a log line cannot claim a method that does not exist.

   **"Frontmost" and "size" are not competing methods, and a two-variant enum naming them as alternatives would describe a path the code does not have.** `terminal_windows` (`desktop.rs:1091`) narrows to candidate windows by process ancestry, by `TERM_PROGRAM`, or by the frontmost application; `frontmost_window` (`:1207`) then always picks the closest size match among whatever that produced, and despite its name performs no frontmost check of its own. The two stages compose rather than compete, which is why the method variant carries the candidate source inside it instead of standing beside it. Read both functions before naming the variants.
3. `LatestCaptureAttemptWindowSelection::{WaitingForFirstResult, Completed(CaptureAttemptWindowSelection)}` — what the monitor holds. The waiting state lives **here and only here**: an instance that has completed no attempt at all is exactly the instance under investigation and must not read as one that failed before selecting, but a value describing a finished attempt can never be "waiting". Putting `WaitingForFirstResult` inside the completed selection would make that impossible state constructible.

**Two channels, because they have different cadences.** Carry `CaptureAttemptWindowSelection` inside a named `CaptureAttemptResult` alongside a `CaptureAttemptSequence` newtype counter, and emit one record per **completed attempt** — every attempt, including a run of attempts whose outcome never changes, which is the sequence counter's reason to exist. The existing `backdrop:` summary line stays transition-only and unchanged in cadence; it gains the monitor's `LatestCaptureAttemptWindowSelection` and nothing else. Adding the sequence to that line instead would force one of two wrong answers: log on every attempt and change the summary's cadence, or keep it transition-only and lose exactly the repeated identical attempts the reproduction needs to count.

Carry `CaptureAttemptResult` out of `capture` on both the success and the failure path, hold the latest selection on `BackdropMonitor` beside the latest attempt status, export the public types through `backdrop/mod.rs` and `lib.rs` behind the existing feature gate, and add the latest selection to the `backdrop: …` transition line `Attract::identify` already emits.

**The per-attempt record needs a log line of its own, or the second channel does not exist.** Holding only the latest selection on the monitor and adding it to a transition-only line delivers neither cadence the next phase reads: the monitor keeps one value, and the transition line suppresses exactly the repeated identical attempts that have to be counted. So `Attract::identify` emits a second, separate `backdrop_attempt:` line through `probe::note` for **every** completed attempt it observes — carrying the `CaptureAttemptSequence`, the `CaptureAttemptWindowSelection` and the attempt's outcome — under the same `CARGO_TILE_FRAME_LOG` gate as the summary line. `BackdropMonitor` must therefore expose each completed attempt rather than only the most recent one, so that nothing is dropped between two reads; a monitor exposing only the latest cannot feed this line and does not satisfy this phase.

This is distinct from Phase 17's `CaptureWindowTarget`, which says what the capture was *asked* to do. This says what it actually did.

**Files:**
- `crates/tui_pane/src/backdrop/desktop.rs` — `CaptureAttemptWindowSelection`, `CaptureWindowSelectionMethod`, `CaptureAttemptSequence` and `CaptureAttemptResult`, carried out of `capture` on both paths
- `crates/tui_pane/src/backdrop/monitor.rs` — hold `LatestCaptureAttemptWindowSelection` beside the latest attempt status, advance the sequence, and expose both, plus each completed `CaptureAttemptResult` so no attempt is lost between two reads
- `crates/tui_pane/src/backdrop/mod.rs` and `crates/tui_pane/src/lib.rs` — export the new types behind the existing `#[cfg(feature = "backdrop")]` gate
- `crates/cargo-tile/src/attract/mod.rs` — add the latest selection to the transition log line, emit the per-attempt `backdrop_attempt:` line for every completed attempt, and cover both in that module's test module

**Constraints from prior phases:** Phase 1 typed the capture failure per stage and made `Desktop::capture` return `Result<Desktop, CaptureFailure>` (`desktop.rs:201`, platform implementation at `:470`). It shipped **no preflight**: `SCShareableContent::get()` runs first, because that call is the one that raises the macOS permission prompt, and `CGPreflightScreenCaptureAccess` classifies only a query that already failed (`desktop.rs:446`–453, called at `:475`). That call answers `false` for a process never asked exactly as for one that refused, so the access variant means "not granted", not "the user said no"; in `objc2-core-graphics` 0.3.2 it is declared safe and carries no unsafe annotation. Phase 1 also deduplicated the exclusion window ids (`desktop.rs:458`, called at `:536`) and split the last successful desktop from the latest attempt status on `BackdropMonitor` (`monitor.rs:129`–184). Phase 2 renamed the access variant to `ScreenRecordingAccessNotGranted` (`desktop.rs:40`) and made a successful capture report the window id it used: `BackdropMonitor::captured_window_id()` (`monitor.rs:457`) returns `LastSuccessfulCaptureWindowId`, either `WaitingForFirstSuccess` or `Available { window_id: u32 }` — a separate report about the last *success*, which this phase's per-attempt record does not replace. Phase 2 also added `WindowIdentification`, whose `Fallback` means window selection gave up on pinning and fell back to the candidate-set heuristic; `CaptureWindowSelectionMethod` is what names which candidate source that heuristic ran over. Phase 3 routes the failing stage and the identification outcome into cargo-tile's attract diagnostics, logging on transition only: `probe::note` emits one `backdrop: report=… capture_status=… captured_window_id=…` line from `Attract::identify` (`attract/mod.rs:960`) whenever any of the three changes, gated on `CARGO_TILE_FRAME_LOG`, and the attract screen's neutral notice names that variable rather than promising a recording an ordinary run never makes. Capture resolves the pinned id across all visible windows (`desktop.rs:494`–495); every `BackdropMonitor::new` builds its own channels, workers, pinned window and cached desktop, so nothing is shared or app-keyed between two instances.

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh check cargo-tile` — the only gate that compiles `backdrop/**` (Invariant 1)
- `bash ~/.claude/scripts/delegate/verify.sh lint cargo-tile`
- `bash ~/.claude/scripts/delegate/verify.sh test tui_pane`
- `bash ~/.claude/scripts/delegate/verify.sh check cargo-port`
- Tests proving: an attempt that fails before window selection reports `SelectionNotReached`; `PinnedWindow` and each `TerminalWindowCandidateSource` of `ClosestSizeMatch` is reachable and reported; two consecutive attempts with an identical outcome produce two `backdrop_attempt:` records with distinct sequence values rather than one; and a monitor that has completed no attempt reports `WaitingForFirstResult`
- Per Invariant 1 the listed `test tui_pane` line does not compile `backdrop/**`, so any test landing there is named in the report and run out of band with the feature enabled before this phase closes, then again at the final workspace gate; behavior that must actually execute per phase needs a cargo-tile-side test instead

### Phase 16 — The second window's capture failure gets its actual cause  · status: todo

**Blocked by:** a live two-window reproduction. This phase cannot start until `cargo tile` runs in two windows of one terminal app with the Phase 1 status, the Phase 3 logging and the Phase 15 attempt records in place, and the failing stage has been recorded. No delegate can stage that reproduction.

#### Work Order

**Goal:** The second `cargo tile` in one terminal app gets its desktop capture.

**Spec:**

Running `cargo tile` in two windows of the same iTerm2 — an app that already has Screen Recording permission — leaves the second one with no desktop capture while the first keeps its own.

**There is no first-caller-wins path to find.** Every `BackdropMonitor::new` builds its own channels, workers, pinned window and cached desktop (`monitor.rs:129`–184), and capture resolves the pinned id across all visible windows (`desktop.rs:494`–495). Nothing is shared or app-keyed. What produced the *appearance* of ownership was diagnosis, not exclusivity: a monitor holding an earlier successful capture kept showing it while a newly started monitor that never got a first capture showed nothing, and both looked identical from outside.

Phase 15 made the missing half observable. **Enumerate, then reason.** Run both instances and record, from outside the program, the windows the window server reports, alongside each instance's per-attempt `CaptureAttemptWindowSelection` — the id it targeted, and whether that id came from pinning or from a closest-size match over a candidate set, naming which source produced that set — and the `CaptureFailure` stage the second instance logs. Reasoning from branches is what the last two backdrop defects both defeated; list the real candidates first.

**Running the reproduction.** The frame log is off unless `CARGO_TILE_FRAME_LOG` names a path (`cargo-tile/src/probe.rs:76`), so neither instance records anything by default. Each process truncates the file on its own first write (`probe.rs:181`–191), so the two instances **must** be given different paths or they will erase each other's evidence. Give each instance its own path, keep both logs, and preserve every `backdrop:` line and every attempt record from both — those are the phase's evidence.

Phase 1 already deduplicated the exclusion list, which was the standing suspect — a sibling terminal window above the selected one appeared both in the owned-windows set and the windows-above set. If the reproduction now succeeds, this phase is closed by evidence rather than by code, and that is a legitimate outcome to report.

Otherwise fix what the recorded stage names, in `tui_pane` where it lives: window resolution, filter construction, or capture, as the evidence directs.

**Files:**
- `crates/tui_pane/src/backdrop/desktop.rs` — window resolution, filter construction, or capture, as the evidence directs
- `crates/tui_pane/src/backdrop/monitor.rs` — window pinning and identification, if the evidence points there

**Constraints from prior phases:** Phase 15 shipped the attempt-level diagnostic this phase reads: `CaptureAttemptResult` carries a `CaptureAttemptSequence` and a `CaptureAttemptWindowSelection::{SelectionNotReached, Selected { window_id, method }}` out of `Desktop::capture` on both paths, one record per completed attempt; `CaptureWindowSelectionMethod` is `{PinnedWindow, ClosestSizeMatch { candidates: TerminalWindowCandidateSource }}`; `BackdropMonitor` holds `LatestCaptureAttemptWindowSelection::{WaitingForFirstResult, Completed(..)}` and exposes every completed attempt; `Attract::identify`'s transition-only `backdrop:` line carries the latest selection, and its separate `backdrop_attempt:` line carries one record per completed attempt with its sequence. The per-attempt line is the one this phase counts repeated identical attempts from; the summary line will not show them. Read those records before touching the capture path — the whole point of the previous phase is that this one starts from evidence rather than from branch reading. Phase 1's per-stage `CaptureFailure`, its exclusion-id deduplication (`desktop.rs:458`, called at `:536`) and its split of the last successful desktop from the latest attempt status (`monitor.rs:129`–184) all still hold; Phase 2's `WindowIdentification` and `LastSuccessfulCaptureWindowId` (`monitor.rs:457`) are the public reports, and a second window that never captures reports `WaitingForFirstSuccess`, which is itself evidence.

**Acceptance gate:**
- Both instances draw their own desktop capture, confirmed by eye in two simultaneous windows
- `bash ~/.claude/scripts/delegate/verify.sh check cargo-tile`, `lint cargo-tile`, `test tui_pane`, `check cargo-port`
- A regression test at whatever level the cause allows, or an explicit statement that the cause is a window-server interaction no unit test can reach. Per Invariant 1 the listed `test tui_pane` line does not compile `backdrop/**`, so a test landing there is named in the report and run out of band with the feature enabled before this phase closes, then again at the final workspace gate; behavior that must actually execute per phase needs a cargo-tile-side test instead

### Phase 17 — Window selection stops being a bare optional number  · status: todo

#### Work Order

**Goal:** Named types carry window selection everywhere in `backdrop/` — identification progress on the monitor, capture target on the capture path, and search outcome on the two window lookups — leaving no owned `Option<u32>` whose absent case has to be decoded from a caller.

**Spec:**

Three owned values spell window selection as `Option<u32>`, and in each the `None` means "no window has been settled on, so fall back to the candidate-set heuristic" — a rule the type does not state and every reader has to recover from `Desktop::capture`'s body: `BackdropMonitor::pinned` (`monitor.rs:157`), `Request::window` (`:193`, built at `:431`), and the `pinned` parameter of `Desktop::capture` (`desktop.rs:201`, threaded to the platform implementation at `:470`–472).

The three sites look alike and are not one domain, so one type threaded through all of them would hand the capture path a state it cannot act on. `BackdropMonitor::pinned` is about **identification progress**: its `None` means the search is still running or has been exhausted, and only `attempts` on the monitor tells those apart. `Request::window` and `Desktop::capture`'s parameter are about the **capture target**: each means an exact id or "use the heuristic now", and neither can behave differently for a search still in flight.

So this phase replaces **all three** options, with two types rather than one:

1. Introduce one private `CaptureWindowTarget::{PreferWindow { window_id: u32 }, TerminalWindowHeuristic}` in `backdrop/` for the worker request (`Request::window`) and the capture parameter (`Desktop::capture`). The monitor derives it at the boundary where it builds the request; `Desktop::capture` matches it once at the top and its body stops re-deriving what `None` meant. `TerminalWindowHeuristic` rather than a name pairing "frontmost" with "size": those are two stages of one path, not two choices — see Phase 15's as-built record.

   Rename the worker message `Request` to `CaptureRequest` in the same pass. This phase is the one that edits its `window` field, and `Request` alone says nothing about what it requests.
2. **Replace the field `BackdropMonitor::pinned: Option<u32>` itself** with `WindowIdentification`, or an equivalent monitor-owned state that names identified, still-pending and exhausted as separate variants. Today those three are `Some(id)`, `None`, and `None` again, told apart only by consulting `attempts` — the field is the third option, and leaving it typed as one would mean the phase's goal is unmet. Do not merely derive the target from `pinned` and leave the field as it is.

Keep `CaptureWindowTarget` private to the crate: `WindowIdentification` is already the public report on the same subject, and nothing outside `tui_pane` reads the capture parameter.

**Preserve Phase 15's distinction.** `CaptureWindowTarget` says what the capture was *asked* to do; `CaptureAttemptWindowSelection` says what it actually did. They are not the same value and must not be merged: an attempt can be asked for `TerminalWindowHeuristic` and report `Selected { method: ClosestSizeMatch { .. } }`, or be asked for `PreferWindow { window_id }` and report `SelectionNotReached`. Keep the per-attempt record, its `CaptureAttemptSequence`, the `backdrop_attempt:` line and the transition line's `LatestCaptureAttemptWindowSelection` working exactly as Phase 15 shipped them.

3. **The two window lookups are the fourth and fifth options, and this phase converts them too.** `window_titled` (`desktop.rs:324`, platform implementation `:597`, non-macOS stub `:1515`) and `window_at` (`:349`, `:614`, stub `:1518`) each return `Option<u32>`, and each `None` means the search ran and found nothing — a domain answer the type declines to give. Convert both at the boundary to `TerminalWindowSearchOutcome::{NotFound, Found { window_id: u32 }}`, keeping it private to the crate for the same reason `CaptureWindowTarget` is private. Without this the phase's own claim below is false.

When this phase lands, no domain-owned window-selection `Option<u32>` remains anywhere in `backdrop/`.

Do not change what any of the three currently do. This phase is a type change with no behavior change, and its gate is that every existing test still passes unmodified.

**Files:**
- `crates/tui_pane/src/backdrop/desktop.rs` — `CaptureWindowTarget`, `capture`'s parameter, the resolution at `:494`–495, and `TerminalWindowSearchOutcome` replacing the `Option<u32>` returns of `window_titled` (`:324`, `:597`, `:1515`) and `window_at` (`:349`, `:614`, `:1518`)
- `crates/tui_pane/src/backdrop/monitor.rs` — `Request` is renamed `CaptureRequest` and its `window` field holds the target; the `pinned: Option<u32>` field is replaced by `WindowIdentification` (or an equivalent named monitor state distinguishing identified, pending and exhausted), and the target is derived from it where the request is built (`:431`)

**Constraints from prior phases:** Phase 2 made `WindowIdentification::Identified` carry the settled window id and added `LastSuccessfulCaptureWindowId` for the id a capture used, deliberately leaving these three options alone so this phase could be designed after the second-window cause was known. Both are public reports on window selection and capture; the enum this phase introduces is the private third one that the capture path itself threads. Phase 15 added the per-attempt selection record described above; it is a third, separate subject and this phase must leave it intact. Phase 16 either found the second-window cause and fixed it in `desktop.rs`/`monitor.rs`, or closed on evidence that Phase 1's exclusion-id deduplication already fixed it; read its as-built record before touching the capture path, because a fix landing there changes what the three states mean.

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh check cargo-tile` — the only gate that compiles `backdrop/**` (Invariant 1)
- `bash ~/.claude/scripts/delegate/verify.sh lint cargo-tile`
- `bash ~/.claude/scripts/delegate/verify.sh test tui_pane`
- `bash ~/.claude/scripts/delegate/verify.sh check cargo-port`
- Every existing backdrop test keeps its behavioral assertions and expected values unchanged, Phase 15's attempt-record tests included. Mechanical construction and call-site edits that replace `Some(id)`/`None` with the named variants are expected and allowed; enumerate them in the report. An assertion or an expected value that had to change means behavior changed, which this phase forbids. Per Invariant 1 the listed `test tui_pane` line compiles none of them: `check cargo-tile` proves they still build, and the feature-enabled suite is run out of band before this phase closes and again at the final workspace gate. Name in the report how many backdrop tests ran and that none were edited.
