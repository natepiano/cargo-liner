# Workspace split: extract `pane_kit` from `cargo-port`

## Goal

Split today's single-crate `cargo-port` into a Cargo workspace with two members:

- `crates/cargo-port/` — the application binary. Domain pipelines (project discovery, lint runs, CI fetches, scanning, watching, enrichment), plus the cargo-port-specific TUI assembly on top of `pane_kit`.
- `crates/pane_kit/` — a reusable library for building ratatui TUIs composed of structured panes. No knowledge of cargo-port, projects, lints, CI, or any other domain.

The split's purpose is **compiler-enforced layering**. Today, generic TUI primitives and app-specific TUI code live in the same `src/tui/` module and import each other freely. That's fine for a single binary but defeats reuse: any other TUI built on top would inherit the cargo-port domain types whether it wanted them or not. A separate library crate makes the boundary a build error instead of a convention.

## Final layout

```
cargo-port/                              ← workspace root (this repo)
  Cargo.toml                             ← [workspace] manifest
  Cargo.lock
  docs/
  crates/
    cargo-port/
      Cargo.toml                         ← [package] name = "cargo-port"  (bin)
      src/
        main.rs
        cache_paths.rs, ci.rs, config.rs, constants.rs, enrichment.rs,
        http.rs, keymap.rs, perf_log.rs, project_list.rs, scan.rs,
        test_support.rs, watcher.rs
        lint/                            (unchanged)
        project/                         (unchanged)
        tui/                             (app-specific TUI only — see below)
    pane_kit/
      Cargo.toml                         ← [package] name = "pane_kit"    (lib)
      src/
        lib.rs
        pane/, columns/, toasts/        (with leaks resolved; see below)
        animation.rs, duration_fmt.rs, popup.rs, running_tracker.rs,
        watched_file.rs, terminal.rs, render.rs, input.rs, interaction.rs,
        background.rs, inflight.rs, selection.rs, finder.rs,
        keymap_state.rs, keymap_ui.rs, shortcuts.rs
```

## What goes where (today's coupling, audited)

A `rg "^use crate::(project|lint|ci|...)"` over `src/tui/` produced the categorization below. "Coupled" means the file imports an app-side type today and needs a boundary fix before it can move into `pane_kit`.

### Trivially generic (no app imports — move as-is)

Files: `animation.rs`, `duration_fmt.rs`, `watched_file.rs`.

Subtree: `pane/{chrome, layout, mod, rules, state, title}` (uses only chrome colors that become `Theme` slots and `tui::interaction::ToastHitbox`, both addressed below).

`tui/constants.rs` (the chrome-color palette) becomes `pane_kit::Theme` field defaults — see "Color handling" below. The crate-root `src/constants.rs` (`APP_NAME`, `GITHUB_API_BASE`, `GIT_*`, `SYNC_*`, `SERVICE_RETRY_SECS`, cache dir names) stays in cargo-port; nothing there is generic.

### Coupled but extractable (need a small boundary type before moving)

| File | App imports today | Boundary fix |
|---|---|---|
| `tui/columns/widths.rs` | none | Truly generic — moves to `pane_kit::columns::Widths`. |
| `tui/toasts/manager.rs` | `super::constants::{TOAST_*}` chrome plus cargo-port git-status constants for some toast styling | Move chrome into `Theme`; toast content is caller-pre-rendered `Span`s. After this fix, `toasts/manager.rs` is moveable. |
| `tui/running_tracker.rs` | `super::toasts::ToastTaskId` | Either extract `ToastTaskId` into `pane_kit::toasts` (it's a thin newtype) and `running_tracker` follows, or leave both in cargo-port. Default: extract `ToastTaskId`, move both. |
| `tui/popup.rs` | `super::render`, `super::constants::TITLE_COLOR` | Inline or extract the small `render` helper popup uses (likely a few lines), then popup moves. `TITLE_COLOR` becomes a `Theme` slot. |
| `tui/keymap_state.rs`, `tui/shortcuts.rs` | `keymap::*Action` enums | Generic over an `Action` trait (`toml_key()`, `description()`, `ALL: &[Self]`, scope name). Concrete enums stay in cargo-port's `keymap.rs`. |
| `tui/background.rs` | `scan::BackgroundMsg`, `watcher::WatcherMsg` | **Reclassify as Stays.** It's a thin channel multiplexer for cargo-port's two specific message types; the generic primitive (an `mpsc::Receiver` select) is already in `tokio`. No need to re-export. |

### Heavily coupled (the App seam — toolkit + thin convenience trait)

A `rg "^use (crate|super)::(panes|app|settings)"` over `render`/`terminal`/`input`/`interaction`/`finder` shows the coupling is deep:

```
super::app::{App, CleanSelection, ConfirmAction, HoveredPaneRow, PendingClean, PollBackgroundStats, ...}
super::panes::{CiFetchKind, HoverTarget, LayoutCache, PaneBehavior, PaneId,
               PendingCiFetch, PendingExampleRun, RunTargetKind}
super::settings
super::finder
super::interaction
```

These are not domain types — they're cargo-port-invented TUI-level identities (which pane has focus, which row is hovered, the user's pending confirmation). Forcing them onto every consumer of `pane_kit` via associated types on a heavy `App` trait would make the library a cargo-port-flavored framework instead of a reusable toolkit.

**Decision: toolkit of primitives + one thin convenience trait.** `pane_kit` exports the reusable widgets and state primitives, plus a minimal optional `App` trait for consumers who want the run loop hidden:

```rust
// pane_kit
pub trait App {
    fn draw(&mut self, frame: &mut Frame);
    fn handle_event(&mut self, event: Event) -> Flow;
    fn tick(&mut self) -> Flow { Flow::Continue }
}

pub enum Flow { Continue, Quit }

pub fn run<A: App>(app: A) -> io::Result<ExitCode> { /* generic loop */ }
```

Three methods, zero associated types, zero cargo-port concepts in the signatures. Idiomatic Rust pattern (mirrors `tokio::main`, `axum::serve`, `clap`'s builder+derive duo): a powerful toolkit with a one-liner convenience for the common case. New consumers can adopt incrementally — start with primitives, add `pane_kit::run` later.

What `pane_kit` exports:

- Chrome and layout: `pane::{Chrome, Layout, Rules, Title, State}`, `columns::Widths`
- Standalone widgets: `popup`, `animation`, `running_tracker`, `watched_file`, `toasts` (after the `manager.rs` boundary fix)
- Generic primitives: `RunningTracker<K>` (inside `running_tracker`), `fuzzy::{search, highlight_spans}` (extracted from `finder.rs`; generic over `T: AsRef<str>`)
- Theming: `Theme` (see "Color handling" below)
- Keymap machinery: `Action` trait + `ScopeMap<A>` + `ResolvedKeymap<A>` + `keymap_state` + `shortcuts` (generic over `A: Action`)
- Optional run loop: `App` trait + `run<A: App>` function

Per-file disposition:

| File | Disposition |
|---|---|
| `tui/render.rs` | **Stays** in `cargo-port`. Imports `super::settings`, `super::finder`, `super::interaction`, `super::panes::*`, `super::app::*` — every one is app-side. Cargo-port's render code is what its `App::draw` impl calls. |
| `tui/terminal.rs` | **Stays** in `cargo-port`. Cargo-port's loop has unusual needs (HttpClient, perf_log, multiplexed scan/watcher channels, signal handling) and is the wrong shape to force through `pane_kit::run`. The generic `pane_kit::run` exists for new consumers; cargo-port keeps its bespoke loop. |
| `tui/input.rs` | **Stays.** Imports `super::panes::*` and project types directly. |
| `tui/interaction.rs` (1529 lines) | **Stays.** Reaches into `app::{ConfirmAction, ExpandKey, DismissTarget}` and `settings::SettingOption`. |
| `tui/background.rs` | **Stays.** Multiplexes `scan::BackgroundMsg` + `watcher::WatcherMsg`. |
| `tui/finder.rs` | **Stays** as the cargo-port fuzzy-finder. The genericizable core (`search_finder` over `nucleo_matcher`, `highlighted_spans`) extracts to `pane_kit::fuzzy::{search, highlight_spans}` in Phase 0b. The state struct (`FinderState`, currently in `app/types.rs`) and the index builder (`build_finder_index` walking `RootItem`/`Workspace`/`Package`/`VendoredPackage`) stay app-side — they're project-domain. |
| `tui/columns/mod.rs` | **Stays.** Exports `ProjectRow<'a>`, `LintCell`, `ProjectListWidths`, `build_row_cells`, `build_summary_cells`, `build_group_header_cells`, plus a hardcoded 8-column project-list schema. Twenty-plus call sites in `panes/project_list.rs`. Only `columns/widths.rs` (the generic widths observer) moves to `pane_kit::columns::Widths`. |
| `tui/keymap_ui.rs` | **Stays.** Imports `super::app::App` directly for ~9 different concerns (selection state, `current_keymap`, `keymap_path`, `inline_error`, `sync_keymap_stamp`, viewport, focus state, mode flags). The `Action` trait alone doesn't address this; only `ScopeMap`/`ResolvedKeymap` move to `pane_kit`, and `keymap_ui` becomes a cargo-port file that uses them. |
| `tui/selection.rs` | **Stays.** Imports six app types (`ExpandKey`, `FinderState`, `ProjectListWidths`, `SelectionPaths`, `SelectionSync`, `VisibleRow`) plus `crate::project_list::ProjectList`. Genericizing this is a months-of-design undertaking; not in scope. |
| `tui/inflight.rs` | **Stays.** Imports `app::PendingClean`, `panes::PendingCiFetch`, `panes::PendingExampleRun`, plus `RunningTracker<AbsolutePath>`. Same conclusion as `selection.rs`: not a small boundary fix. The generic `RunningTracker<K>` it depends on does move to `pane_kit` (via `running_tracker.rs`). |
| `tui/mod.rs` | Rewritten: `pub use pane_kit::{Icon, LINT_SPINNER}` (re-exported for cargo-port-side callers that haven't migrated their imports yet) and removes `pub use terminal::run` (terminal stays — `cargo-port::tui::run` is unchanged). |

Cargo-port itself does *not* implement `pane_kit::App` — it keeps its own loop. The `App` trait and `run` function are part of `pane_kit`'s public API for the benefit of future consumers, not for cargo-port. (If, after the split lands, cargo-port's loop turns out to be reshapable through the generic `run`, that's a follow-up.)

### Stays in `cargo-port` (app-specific, no question)

- All of `tui/app/` (composition root)
- All of `tui/panes/` (concrete pane kinds: `ci`, `cpu`, `git`, `lang`, `lints`, `output`, `package`, `project_list`, `system`, `targets`)
- `tui/*_state.rs` mirrors: `ci_state`, `config_state`, `lint_state`, `net_state`, `scan_state`
- `tui/cpu.rs`, `tui/config_reload.rs`, `tui/settings.rs`

### Color handling: chrome vs content

Today's `tui/constants.rs` mixes two different concerns:

- **Chrome colors** — colors `pane_kit` widgets apply to themselves: pane borders (active/inactive), titles, focus ring, popup backgrounds, toast borders, spinners, accent. The library *will* draw these; it has to get the color from somewhere.
- **Content colors** — colors the app applies to data inside the chrome: "this worktree is in sync (green)", "this lint failed (red)", "this CI run was cancelled (gray)". These encode cargo-port domain meaning. (These already live in `src/constants.rs` at the crate root, not in `tui/constants.rs`.)

**Decision:**

- `pane_kit` exports a `Theme` struct with ~20 named slots. Today's chrome palette in `tui/constants.rs` already maps to: `accent`, `border_active`, `border_inactive`, `title_active` (=`TITLE_COLOR`), `title_inactive`, `label`, `focus_active`, `focus_hover`, `focus_remembered`, `column_header`, `discovery_shimmer`, `error`, `warning`, `inline_error`, `success`, `secondary_text`, `status_bar`, `finder_match_bg`, plus toast-related slots. Caller passes a `Theme` at app construction. `Theme::default()` matches cargo-port's current palette so the migration is one line. Note: a few colors today are dual-use (e.g. `ERROR_COLOR` is used by both inline errors and toast borders) — the `Theme` struct gives them named-by-purpose slots so consumers can override one without affecting the other.
- All chrome code in `pane_kit` (pane chrome, popup, toasts, animations, columns separators/headers) reads colors from `&self.theme` instead of constants.
- Content cells stay caller-pre-rendered as `Span`s — even with a theme, "what color does git-modified get" is a domain question, not a theme question.

This adds one type and ~10 minutes of refactoring inside `pane_kit`'s widgets in exchange for the library actually being reusable by other consumers, not just cosmetically separate.

### Top-level `src/*.rs` audit (none move)

Audited every non-tui file at the crate root for genericity. None should move to `pane_kit`:

| File | Why it stays |
|---|---|
| `http.rs` (977 lines) | Hardcoded for GitHub + crates.io APIs (`GITHUB_API_BASE`, `CRATES_IO_API_BASE`, `GhRun`, `GqlCheckRun`). Not a generic rate-limited client. |
| `watcher.rs` (4141 lines) | Orchestrates cargo-port project re-enrichment via `enrichment`, `WorkspaceMetadataStore`, `RootItem`. Not a generic `notify` wrapper. |
| `keymap.rs` (1654 lines) | Defines the cargo-port action enums (`CiRunsAction`, `GitAction`, `LintsAction`, `PackageAction`, `ProjectListAction`, `TargetsAction`, `GlobalAction`). The reusable resolution/display machinery moves via the `Action` trait introduced in Phase 0, but the concrete enums are app-domain. |
| `cache_paths.rs` (84 lines) | Computes cargo-port's cache locations (`CI_CACHE_DIR`, `LINTS_CACHE_DIR`, `APP_NAME`). Not a generic XDG helper. |
| `perf_log.rs` (72 lines) | Small enough that genericizing the `AbsolutePath` key type isn't worth the surgery. |
| `constants.rs` (93 lines) | All cargo-port-specific (`APP_NAME`, `GITHUB_API_BASE`, `GIT_*` git-status colors, `SYNC_*`, `SERVICE_RETRY_SECS`, cache dir names). The TUI styling palette that moves to `pane_kit` lives in `src/tui/constants.rs`, not this file. |
| `ci.rs`, `config.rs`, `enrichment.rs`, `project_list.rs`, `scan.rs`, `test_support.rs` | Domain pipelines and types; obviously app-side. |

## Phased execution

This is one big change shipped together — the phases below are an **execution checklist for correctness**, not separate PRs. Each phase ends at `cargo build && cargo nextest run` green so the work can be paused and resumed at a known-good point, but the whole sequence lands in a single PR (or one PR per phase if the change feels too big to review at once — caller's choice, not a planning constraint).

### Phase 0a — Introduce `Action` trait

In today's `src/`, define an `Action` trait (display name, scope, etc.) and implement it for every `keymap::*Action` enum. Rewrite `tui/keymap_ui.rs` and `tui/shortcuts.rs` to be generic over `A: Action` instead of pattern-matching each concrete enum. `tui/keymap_state.rs` follows. **No files move yet.**

Verify: keymap rendering, shortcut bar, and key dispatch unchanged.

### Phase 0b — Extract fuzzy primitives

Inside `tui/finder.rs`, isolate the genericizable matcher pieces — `search_finder` (~60 lines wrapping `nucleo_matcher`) and `highlighted_spans` (~70 lines) — into a `tui::fuzzy` module generic over `T: AsRef<str>`. Concrete `finder.rs` calls into them. Note: `FinderState` (in `app/types.rs`) and `build_finder_index` stay where they are — they're project-domain, not generic.

Verify: finder open/close/search/select/highlight unchanged.

### Phase 0c — Boundary work for moveable files

Refactor:
- `popup.rs`: inline the small `super::render` helper it depends on (or extract that helper to a sibling module that moves with popup).
- `running_tracker.rs`: extract `ToastTaskId` (a thin newtype currently in `toasts/manager.rs`) into a place both files can share post-split.
- `toasts/manager.rs`: switch from importing chrome constants directly to taking colors from a `Theme` reference; toast content becomes caller-pre-rendered `Span`s.
- Define the `Action` trait in `tui/keymap.rs`-side and implement for every `*Action` enum. Rewrite `keymap_state.rs` and `shortcuts.rs` generic over `A: Action`. (`keymap_ui.rs` is *not* genericized — it stays cargo-port-side per the disposition table.)

Verify: TUI looks and behaves identically.

### Phase 1 — Workspace skeleton, one member

Convert the root `Cargo.toml` to `[workspace]` with `members = ["crates/cargo-port"]`. Move all of today's `src/`, `Cargo.toml` `[package]`/`[dependencies]` into `crates/cargo-port/`. Promote shared dep versions to `[workspace.dependencies]`.

**Important:** do NOT promote `[lints]` to the workspace yet — leave them on the cargo-port member. Promoting `missing_docs = "deny"` to the workspace would force every `pub` item in `pane_kit` to carry rustdoc starting at Phase 3. Promote workspace lints in Phase 6 once `pane_kit` is fully documented.

Verify: `cargo install --path crates/cargo-port` works, `cargo nextest run` green, binary behavior unchanged.

### Phase 2 — Add empty `pane_kit`

Create `crates/pane_kit/Cargo.toml` and `crates/pane_kit/src/lib.rs` (empty `//! pane_kit`). Add `pane_kit = { path = "../pane_kit" }` as a dep of `cargo-port`. `pane_kit` declares its own `[lints]` block — use cargo-port's strict set minus `missing_docs` until Phase 6.

### Phase 3 — Move trivially-generic files

Move: `animation.rs`, `duration_fmt.rs`, `watched_file.rs`, `pane/{chrome, layout, mod, rules, state, title}`, `columns/widths.rs`, today's `tui/constants.rs` chrome colors as `Theme` field defaults. Update every `use crate::tui::xyz` in cargo-port to `use pane_kit::xyz`.

### Phase 4 — Move boundary-fixed files

Move the files refactored in Phase 0a/0b/0c: `popup`, `running_tracker` (with extracted `ToastTaskId`), `toasts/{format, mod, render, manager}`, `keymap_state`, `shortcuts`, the `Action` trait + `ScopeMap`/`ResolvedKeymap` machinery, the `fuzzy::{search, highlight_spans}` primitive. Update imports across cargo-port to point at `pane_kit::*`.

### Phase 5 — Audit what (if anything) of the run loop can move

Re-examine `terminal`, `render`, `input`, `interaction`, `background`, `finder`, `selection`, `inflight`, `keymap_ui`, `columns/mod.rs` against the realized `pane_kit` API. Default disposition (per the toolkit decision in §App seam): **all stay in cargo-port**. If a small helper (e.g. terminal init/restore, signal handling, frame-pacing) is genuinely generic, extract that helper only.

### Phase 6 — Tighten, document, promote workspace lints

Add rustdoc to every `pub` item in `pane_kit`. Promote `[lints.rust]` (including `missing_docs = "deny"`) and `[lints.clippy]` to `[workspace.lints]` and switch members to `lints.workspace = true`. Audit `pub(crate)` vs `pub`. Delete dead re-exports in `cargo-port`. Update `docs/app-api.md`.

## Risks and open questions

1. **App seam decision: locked.** `pane_kit` is a toolkit of primitives plus a thin three-method `App` trait and a `run<A: App>` helper. No associated types on the trait. Cargo-port keeps its own loop and does not implement `App` itself.
2. **`tui/app/tests/`** — those tests reach into both halves. They stay in `cargo-port` (testing the composed app). Anything that ends up needing to test `pane_kit` primitives in isolation gets a unit test inside `pane_kit/src/<file>.rs` instead of moving from `app/tests/`.
3. **`test_support.rs`** is `#[cfg(test)] mod test_support;` in `main.rs` (verified). If `pane_kit` needs test helpers, give it its own private `test_support`; do not try to share across crates without a third `*-test-support` crate. Defer that decision until something actually needs to be shared.
4. **`missing_docs` timing.** Inside the one big change, defer promoting `missing_docs = "deny"` to workspace lints until the documentation pass at the end. Until then, `pane_kit`'s `[lints]` block uses cargo-port's strict set minus `missing_docs`. Don't drop `missing_docs` enforcement entirely — public library code should have docs.
5. **`cargo install --path .`** convention changes to `cargo install --path crates/cargo-port`. Update the auto-memory entry `feedback_cargo_install.md` after Phase 1.
6. **`cargo-mend` workspace behavior.** CI runs `cargo install cargo-mend` then `cargo mend --fail-on-warn` from the workspace root. Verify cargo-mend handles a workspace with the bin member at `crates/cargo-port/` before merging Phase 1, or pin its invocation to the member directory.
7. **`perf_log` coupling to `terminal`.** Resolved by the App-seam decision: `terminal.rs` stays in cargo-port and keeps its `crate::perf_log` import unchanged. `perf_log` itself stays at the cargo-port crate root.
8. **Doctests.** Once `pane_kit` is a lib crate, every `///` example becomes a doctest. Plan accordingly when writing rustdoc in Phase 6 — examples that need a running terminal are not realistic doctests; use `no_run` or `ignore` annotations.
9. **Shared `target/` and `Cargo.lock`.** Workspace members share `target/` and `Cargo.lock` at the workspace root by default. Both desirable; just confirm CI cache keys still hit (they probably do — the lockfile path is unchanged from the workspace-root perspective).

## Out of band

This split does not solve and is not blocked on:

- The `tui/pane/` vs `tui/panes/` singular/plural naming collision. Resolved naturally by the split: the singular `pane/` subtree moves to `pane_kit::pane`, and `panes/` (concrete pane kinds) stays as `cargo-port::tui::panes`. Two crates, no ambiguity.
- Grouping the `*_state.rs` mirrors under `tui/state/` in `cargo-port` (independent cleanup; do whenever).
