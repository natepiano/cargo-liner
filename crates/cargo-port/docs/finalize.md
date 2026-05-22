# Finalize framework move

Branch: `refactor/finalize-framework` (renamed from `refactor/style`).

This plan finalizes the move of all non-domain code out of `src/` into
`tui_pane/`. The earlier commit on this branch handled theme machinery
(watch, registry loader, resolver, runtime, OS appearance poller).
This plan covers everything else: hit-test infrastructure
cleanup, generic diagnostics, keymap infrastructure, keymap UI plus
click dispatch, and input event dispatch.

## Delivery model

The whole refactor lands as **one commit** on `refactor/finalize-framework`:

> `refactor(tui_pane): finalize framework move`

The phases below describe logical work streams for review and
ordering inside that single commit — they are not separate commits.
Tests + clippy + fmt run once at the end, before push. Rollback is a
single `git revert <hash>`.

### Pre-merge checklist

- `cargo nextest run --workspace` green.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` clean.
- `cargo +nightly fmt --all` clean.
- Manual smoke test of editor launch (no automated test covers crossterm-driven editor invocation).
- Type-boundary check (see below) reviewed.

## Cross-cutting policies

Apply these everywhere.

### Type boundary (no cycles)

Every trait method in `tui_pane` returns and accepts types defined in
`tui_pane` or `std` — never types from the app crate. If the framework
needs an app type, the app erases it (`&dyn SomeTrait`, an enum the
framework owns, or `Box<dyn Any>`) before the framework sees it. This
prevents `framework → app → framework` cycles.

Apply the check to **all** boundary code, not just trait methods:
helper functions in moved files (`viewport_mut_for`, `set_hovered`,
etc.) must also accept/return framework types only. Audit during the
keymap-UI/interaction trait design (Phase 4).

### Re-export policy

The submodules (`theme/`, `keymap/`, `overlays/`, etc.) are the stable
API. The crate-root re-exports in `tui_pane/src/lib.rs` are a
convenience surface and may move. Each new `pub use` must be justified
by a comment near the re-export or in the commit body. When in
doubt, leave the import path explicit
(`tui_pane::keymap::ResolvedKeymap` rather than
`tui_pane::ResolvedKeymap`).

### Module placement

Convention in `tui_pane/src/`:

- Directories for feature groups (`theme/`, `keymap/`, `overlays/`).
- Single-file modules at top level for isolated utilities (`activity.rs`, `copy.rs`, `util.rs`, `watched_file.rs`).
- **Exception:** observability / monitoring primitives live under `tui_pane/src/diagnostics/` (per Q3). `cpu.rs` is housed there even though it's also a render primitive — diagnostics is the chosen umbrella for "framework-provided observability." If the umbrella ever grows to a third file, revisit.

### Test migration

For each moved file, classify its `#[cfg(test)] mod tests`:

- Pure logic tests (input → output): move with the file.
- Tests that construct an `App` (e.g. via `crate::tui::test_support::make_app`): leave in the app crate as integration tests under `tests/`. The framework should not depend on app-side test scaffolding.
- Tests that read `App` state directly without `make_app`: rewrite to use a small trait mock the framework exposes (or move them to app-side integration tests).

The commit body lists which tests moved, which were rewritten, and which stayed.

## Inventory

Domain refs = matches against
`crate::{scan,ci,lint,project,http,config,themes,enrichment,cache_paths,watcher,constants}`.

| File / dir                       | Lines | Domain refs | Phase | Notes |
| -------------------------------- | ----- | ----------- | ----- | ----- |
| `src/tui/pane/{dismiss,dispatch,mod}.rs` | 10 + 159 + 33 = 202 | 0 (DismissTarget carries `AbsolutePath`) | 1 | Cleanup only — see Phase 1. |
| `src/tui/interaction.rs`         | 201   | 0 (reads `App.{project_list, framework.toasts, overlays, panes}` directly) | 4 | Moves alongside keymap_ui behind a separate `InteractionContext` trait. |
| `src/perf_log.rs`                | 72    | 1 (`AbsolutePath`)        | 2 | `tui_pane/src/diagnostics/perf_log.rs`. Takes `&Path` args. |
| `src/tui/cpu.rs`                 | 463   | 1 (`config::CpuConfig`)   | 2 | `tui_pane/src/diagnostics/cpu.rs`. sysinfo always-on dep (per Q1). |
| `src/tui/keymap/{load,parse,resolved,scope_map,key_bind}.rs` | ~big | 0 in code paths; load.rs has cfg(test) refs to `config::NavigationKeys`, `constants::{APP_NAME, KEYMAP_FILE}`, `project::AbsolutePath` | 3 | Audit-driven move. See Phase 3. |
| `src/tui/keymap_ui/`             | 957   | 0 | 4 | Behind `KeymapUiContext` trait. |
| `src/tui/input/mod.rs`           | 737   | 1 (`project::AbsolutePath`) | 5 | Dispatch goes to framework; action handlers stay in app. |
| `src/tui/input/editor_terminal.rs` | 218 | 4 (`project::{AbsolutePath, ProjectFields, RootItem, RustProject}`) | 5 | Stays in app — domain-locked. |

### Out of scope (rationale)

These have low surface coupling but stay in the app:

- `src/tui/overlays/` (174 lines, 0 grep refs) — `Overlays` composes cargo-port-specific overlay state: `FinderMode` (cargo-port's fuzzy finder feature), `finder_return` focus restoration, `inline_error` for settings UI, the Finder pane viewport. Framework owns the primitives (`PopupFrame`, `FocusedPane<T>`, `KeymapPane`, `SettingsPane`); this struct is the app's composition layer.
- `src/tui/background.rs` (241 lines, 7 domain refs) — wraps the cargo-port `BackgroundMsg` channel.
- `src/tui/finder/` (201 lines, 10 domain refs) — couples deeply to project list state.
- `src/tui/running_targets.rs` (283 lines) — cargo target lifecycle state. Also uses `sysinfo` directly — affects Phase 2's dep accounting (see Phase 6).
- `src/tui/settings.rs` (1964 lines) — cargo-port's settings UI built on the framework's settings primitives.
- `src/tui/test_support.rs` — couples to `App` builders.
- `src/tui/{render, terminal, app, state, integration, panes, project_list, columns, constants}` — composition or domain UI.
- `src/tui/keymap/actions.rs` — invokes `tui_pane::action_enum!` to define cargo-port-specific action enums.

## Phase 1 — Simplify pane re-export stack (cleanup only)

No new framework code. The framework already has `Hittable`, `HitTestRegistry`, and the `AppContext::AppPaneId` associated type. What this phase does:

- `src/tui/pane/mod.rs` currently re-exports a stack of `tui_pane::*` names so internal callers can write `crate::tui::pane::Hittable`. Delete the re-export stack; callers import from `tui_pane::` directly.
- `src/tui/pane/dismiss.rs` (10 lines, `DismissTarget` only) folds into `dispatch.rs`. Two-file pane module collapses to one.
- `src/tui/pane/dispatch.rs` keeps its cargo-port content: `HittableId`, `HoverTarget`, `HITTABLE_Z_ORDER`, `PaneRenderCtx`, plus the trait impls (`HitTestRegistry for App`, `InputContext for App`). All stay in app — they are cargo-port's specific instantiation of the framework's generic types.

Net effect: one fewer file in `src/tui/pane/` and a simpler import surface. No framework code lands.

## Phase 2 — Diagnostics

### 2a. `src/perf_log.rs` → `tui_pane/src/diagnostics/perf_log.rs`

Moves: constants `SLOW_FRAME_MS`, `SLOW_BG_BATCH_MS`, `SLOW_INPUT_EVENT_MS`; the `ms()` saturating cast helper; the tracing subscriber installer.

Decoupling:

- Replace `OnceLock<AbsolutePath>` with `OnceLock<PathBuf>` (or remove the cache and let the caller hold it).
- New init signature: `pub fn init(current_log_path: &Path, previous_log_path: &Path)`. The app's `terminal::run` computes the paths from its config and passes them in.
- Remove the hardcoded `"cargo-port-tui-perf.log"` filename.

### 2b. `src/tui/cpu.rs` → `tui_pane/src/diagnostics/cpu.rs`

Moves: `CpuCoreUsage`, `CpuUsage`, the sysinfo sampler, the gradient rendering using the framework's theme accessors.

Decoupling:

- Constructor today takes `&CpuConfig`. New signature takes three explicit args: `poll_interval_ms: u64`, `green_max_percent: u8`, `yellow_max_percent: u8`. The app converts from its `CpuConfig` at the call site.
- `severity()` helper gets the same treatment.

Dep consequence: `sysinfo` becomes a `tui_pane` dependency (always-on, per Q1). The framework provides sampling + rendering; the app controls layout placement.

## Phase 3 — Keymap infrastructure / actions split

`tui_pane/src/keymap/` already exists with a long file list (`action_enum`, `bindings`, `builder/`, `global_action`, `globals`, `key_bind`, `key_outcome`, `key_sequence`, `load`, `mod`, `navigation`, `runtime_scope`, `scope_map`, `shortcuts`, `vim`). The app side has `actions`, `key_bind`, `load`, `parse`, `resolved`, `scope_map`. File-name overlap: `key_bind`, `load`, `scope_map`.

### Pre-phase audit (done before writing the move)

For each app-side file, classify:

| File | Framework counterpart? | Action |
| ---- | ---------------------- | ------ |
| `key_bind.rs` | yes — same name, field-name divergence (app uses `modifiers`, framework uses `mods`); app version is `pub(crate)`, framework is `pub` | Confirm semantics are identical (normalization, ordering). Delete app version, repoint imports to `tui_pane::KeyBind`. |
| `load.rs` | yes | Diff both, pick the canonical one (likely framework's), delete the other, migrate cfg(test) refs to integration tests under `tests/keymap_load.rs` in the app. |
| `scope_map.rs` | yes | Diff both, reconcile, delete app version. |
| `parse.rs` | no | Move app version into framework verbatim. |
| `resolved.rs` | no | Move app version into framework verbatim. |
| `actions.rs` | no — invokes `action_enum!` macro to define domain action enums | Stays in app. |

The audit results land in the commit body. If any diff turns out to be semantically incompatible (rather than cosmetic), escalate — don't fix mid-move.

### cfg(test) coupling in load.rs

`src/tui/keymap/load.rs` has cfg(test) references to `config::NavigationKeys`, `constants::{APP_NAME, KEYMAP_FILE}`, `project::AbsolutePath`. Move the function bodies to the framework; **leave the tests behind** as a new app-side `tests/keymap_load.rs` that exercises the framework function with cargo-port fixtures. Cleaner than parametrizing the tests.

## Phase 4 — Keymap UI + click dispatch

Combines `src/tui/keymap_ui/` (957 lines) and `src/tui/interaction.rs` (201 lines). Two narrow traits — not one wide one.

### Trait design (do this first)

The two files have **different** access patterns:

- `keymap_ui` reads: `ResolvedKeymap`, current scope, capture state, overlays' `inline_error`. Pure rendering.
- `interaction.rs` reads: `project_list.set_cursor`, `framework.toasts`, `overlays.is_finder_open`, panes' viewports, focus management.

Define two separate traits in `tui_pane`:

- **`KeymapUiContext`** — keymap rendering surface: resolved keymap reads, current scope, capture state, overlay error state.
- **`InteractionContext`** — click/scroll dispatch surface: pane cursor setting, viewport access, toast dismissal, finder query, focus mutation.

The app's `App` struct implements both. Single-responsibility wins here: a future client could adopt one without the other, and each trait stays small and reviewable. (`AppContext` continues to host the truly universal methods like `framework()`, `framework_mut()`, `set_focus()`.)

Before writing any move, draft both trait signatures by grepping each file for `app.<field>` accesses, categorizing as trait method vs. internal helper. Capture the draft in the commit body or a short audit file.

### Steps

- Land both traits in `tui_pane`.
- Move `src/tui/keymap_ui/{mod,view}.rs` to `tui_pane/src/keymap_ui/`; rewrite to use `KeymapUiContext`.
- Move `src/tui/interaction.rs` to `tui_pane/src/interaction.rs`; rewrite to use `InteractionContext`. Apply the type-boundary check to its helper functions (`viewport_mut_for`, `set_hovered`, etc.) — they must accept/return framework types only.
- Move the entangled `Renderable<PaneRenderCtx<'_>> for KeymapPane` impl out of `src/tui/overlays/pane_impls.rs` into `tui_pane/src/keymap_ui/` (or wherever `keymap_ui` lives). After this, `pane_impls.rs` keeps only the `FinderPane` impls and the `Renderable for SettingsPane` impl (settings rendering stays in app).
- App implements `KeymapUiContext` and `InteractionContext` for `App`.
- Delete `src/tui/keymap_ui/` and `src/tui/interaction.rs`.

### Test migration

- Pure-logic tests in `keymap_ui/mod.rs` (e.g. `keymap_header_line_uses_section_name`, `keymap_popup_height_is_bounded_on_tall_terminals`) move with the file.
- The `keymap_lines_track_selectable_rows_only` test (and anything else that calls `make_app`) stays in the app under `tests/keymap_ui.rs`.
- `interaction.rs` currently has no test module; nothing to migrate.

## Phase 5 — Input event loop

`src/tui/input/mod.rs` (737 lines) drives crossterm event polling and key dispatch. `editor_terminal.rs` (218 lines) opens an external editor — domain-locked (reads `RootItem::Rust(RustProject::Workspace(_))`).

### Split

- **Move to `tui_pane/src/input/`** (or a sensible existing module): the generic event loop, the key→action dispatch table machinery, scroll/click handling.
- **Stays in app:** `editor_terminal.rs`. The framework's input dispatch returns an `Action` enum; the app's action-handler code matches on it and calls `editor_terminal::launch` directly when the editor action fires. The framework never names the editor concept.

This keeps `AppContext` clean — no framework trait method for editor launch. The app-domain action handler stays in the app where it belongs.

### Pre-phase domain audit

The inventory previously called input/ "1 domain ref" — that was wrong. `editor_terminal.rs` has 4 distinct `crate::project::*` types. Confirm by grepping before writing the move. Document any other domain refs in the commit body.

## Phase 6 — Cleanup

### Dep audit

After the full move:

- `tui_pane/Cargo.toml` gains `sysinfo` (per Q1, always-on).
- `cargo-port/Cargo.toml` keeps `sysinfo` — `src/tui/running_targets.rs` still uses it directly. **Do not remove sysinfo from cargo-port.**
- Check whether any other dep (e.g. `crossterm` features) shifts. Verify by removing candidates and running `cargo check --workspace --all-features`.

### Doc updates

- `docs/style/frontend-boundaries.md` — add concrete examples from any tricky case the move surfaced.

### `src/themes/mod.rs`

After all phases, this module holds only `themes_dir()` (uses `APP_NAME`) plus its test override. App-specific — stays. Could merge into `src/config.rs`; decision deferred (small enough that a dedicated module is not harmful).

## Resolved questions (from earlier walkthrough)

- **Q1 — RESOLVED: always-on.** Phase 2b adds `sysinfo` to `tui_pane/Cargo.toml` as a regular dep, not a feature flag.
- **Q2 — RESOLVED: fold into Phase 4.** `interaction.rs` moves alongside the `keymap_ui` trait work in Phase 4. (Plan now uses two narrow traits — `KeymapUiContext` and `InteractionContext` — rather than one wide trait; see Phase 4.)
- **Q3 — RESOLVED: `tui_pane/src/diagnostics/`.** `perf_log.rs` and `cpu.rs` go under a new `diagnostics/` subdir.
- **Q4 — RESOLVED: cancel Phase 1b.** `overlays/` stays in the app. The `Overlays` struct composes cargo-port-specific UI state.

## Decided during second review (delegable design calls)

- **Phase 4 trait design — two narrow traits, not one wide one.** `KeymapUiContext` covers keymap rendering surface only; `InteractionContext` covers click/scroll dispatch surface only. Single-responsibility, narrower contracts, future clients can adopt one without the other.
- **Phase 5 editor launch — no framework trait method.** The framework's input dispatch returns an `Action` enum; the app's action handlers stay in app and call `editor_terminal::launch` directly. Keeps `AppContext` from accumulating domain-specific verbs.
