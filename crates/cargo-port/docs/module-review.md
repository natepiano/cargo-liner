# `src/` restructure plan

The current placement is mostly sound: domain modules (`lint/`, `project/`, `scan/`) are cleanly isolated from `tui/`; the sibling crate `tui_pane` has a clean unidirectional boundary; the largest subsystems already use `mod.rs` facades with focused submodules. The work this plan describes is a single placement commit (Phase 1) that fixes a naming collision, a layer inversion, and two undersized integration files — followed by a series of file-split phases that resolve the over-large files surfaced against `when-to-split-a-module.md`.

## Phase overview

| Phase | What | Risk | Rough size |
|-------|------|------|------------|
| 1 | Placement: rename `panes/support.rs`, move `DismissTarget` into `pane/`, consolidate two tiny `integration/` files | Low | ~12 import edits, one commit |
| 2 | Split `src/watcher.rs` (4155 lines) into `watcher/{mod,debounce,disk,git,paths}` | Medium | 4–6 leaf imports across crate, one commit |
| 3 | Split `src/tui/panes/pane_data.rs` (was `panes/support.rs`, 1913 lines) by pane domain | Medium | ~10 caller imports inside `panes/`, one commit |
| 4 | Split `src/tui/finder.rs` (1108 lines) into `finder/{mod,index,dispatch}` | Low | 3 caller imports, one commit |
| 5 | Split `src/tui/input.rs` (873 lines) into `input/{mod,keys,editor_terminal,mouse}` | Low | 1 public entry point, one commit |
| 6 | Split `src/http.rs` (981 lines) into `http/{mod,github,crates_io,rate_limit}` | Low | 2 caller imports (test_support + watcher), one commit |
| 7 | Split `src/keymap.rs` (1579 lines) into `keymap/{mod,parse,display}` | Medium | 20+ callers, but only the file path changes; one commit |
| 8 | Split `src/tui/keymap_ui.rs` (1165 lines) into `keymap_ui/{mod,view,settings_popup}` | Low | 3 caller imports, one commit |

Each phase lands as a single commit once `cargo build` + `cargo nextest run` are green at the end. Cargo checkpoints between sequencing steps within a phase are in-flight only — no intermediate commits.

---

## Phase 1 — Placement

### Proposed layout

```
src/
├── main.rs
├── cache_paths.rs
├── ci.rs
├── config.rs                          (split deferred — see Phase deferred list)
├── constants.rs
├── enrichment.rs
├── http.rs                            (split in Phase 6)
├── keymap.rs                          (split in Phase 7)
├── perf_log.rs
├── test_support.rs
├── watcher.rs                         (split in Phase 2)
│
├── lint/                              ← unchanged
├── scan/                              ← unchanged
├── project/                           ← unchanged
│   ├── cargo/                         ← unchanged
│   └── git/                           ← unchanged
│
└── tui/
    ├── mod.rs
    ├── background.rs
    ├── constants.rs
    ├── cpu.rs
    ├── finder.rs                      (split in Phase 4)
    ├── input.rs                       (split in Phase 5)
    ├── interaction.rs
    ├── keymap_ui.rs                   (split in Phase 8)
    ├── render.rs
    ├── settings.rs
    ├── terminal.rs
    ├── test_support.rs
    │
    ├── app/                           ← unchanged except DismissTarget leaves
    │   ├── mod.rs
    │   ├── ci.rs
    │   ├── construct.rs
    │   ├── dismiss.rs                 (now only owns the dismiss *methods*; type moves to pane/)
    │   ├── phase_state.rs
    │   ├── startup.rs
    │   ├── target_index.rs
    │   ├── types.rs
    │   ├── async_tasks/               ← unchanged
    │   ├── navigation/                ← unchanged
    │   └── tests/                     ← unchanged
    │
    ├── columns/                       ← unchanged
    │
    ├── integration/
    │   ├── mod.rs
    │   ├── config_reload.rs           ← unchanged
    │   ├── framework_keymap.rs        ← unchanged
    │   └── lint_icon.rs               (renamed from lint_display.rs; toast_adapters.rs folded in)
    │
    ├── overlays/                      ← unchanged
    │
    ├── pane/                          ← gains DismissTarget
    │   ├── mod.rs
    │   ├── chrome.rs
    │   ├── dispatch.rs
    │   ├── dismiss.rs                 (NEW — owns DismissTarget enum)
    │   ├── layout.rs
    │   ├── rules.rs
    │   ├── state.rs
    │   └── title.rs
    │
    ├── panes/
    │   ├── mod.rs
    │   ├── actions.rs
    │   ├── ci.rs
    │   ├── constants.rs
    │   ├── cpu.rs
    │   ├── data.rs
    │   ├── git.rs
    │   ├── lang.rs
    │   ├── layout.rs
    │   ├── lints.rs
    │   ├── output.rs
    │   ├── package.rs
    │   ├── pane_data.rs               (renamed from support.rs — split in Phase 3)
    │   ├── pane_impls.rs
    │   ├── project_list.rs
    │   ├── spec.rs
    │   ├── system.rs
    │   ├── targets.rs
    │   ├── tests.rs
    │   └── widths.rs
    │
    ├── project_list/                  ← unchanged
    │
    ├── state/                         ← unchanged
    │
    └── support/                       ← unchanged
```

### Moves, with rationale

#### 1. Rename `tui/panes/support.rs` → `tui/panes/pane_data.rs`

`tui/panes/support.rs` (1913 lines) defines per-pane detail data — `PackageData`, `GitData`, `CiData`, `LintsData`, `TargetsData`, plus the `DetailField` enum and builders. None of it is generic "support" code. Meanwhile `tui/support/` (sibling under `tui/`) holds genuinely generic utilities (`WatchedFile`, `RunningTracker`, `format_progressive`). The two names collide and the contents don't match either label.

`pane_data.rs` describes what the file contains. The rename also positions the file accurately for Phase 3, where it gets split by pane domain.

#### 2. Move `DismissTarget` from `tui/app/dismiss.rs` to `tui/pane/dismiss.rs`

`DismissTarget` is currently defined at `tui/app/dismiss.rs:1` and consumed by `tui/pane/dispatch.rs:24`. That is a layer inversion: `pane/` is the primitive layer (trait infrastructure for any pane), `app/` is the aggregator. A primitive must not import from the aggregate it is embedded in.

The fix: the *enum* `DismissTarget` (the data describing what a pane can be dismissed against) belongs in `pane/`; the *method* `App::dismiss(target)` and `App::dismiss_target_for_row(row)` continue to live in `app/dismiss.rs` and now `use crate::tui::pane::DismissTarget`. The split matches `DismissTarget`'s actual coupling — it names pane-level concerns (rows, focus, scroll), not app-level orchestration.

#### 3. Fold `tui/integration/toast_adapters.rs` (27 lines) into `tui/integration/framework_keymap.rs`; rename `lint_display.rs` (39 lines) to `lint_icon.rs`

`tui/integration/toast_adapters.rs` (27 lines) contains exactly two helpers: `path_key()` and `owner_repo_key()`. They map cargo-port domain types to `tui_pane::TrackedItemKey`. That is a one-line job per type and lives more naturally next to the other framework-adapter functions in `framework_keymap.rs`.

`tui/integration/lint_display.rs` (39 lines) holds a single function `icon_for(LintStatusKind) -> tui_pane::Icon`. Keeping the file is fine; "display" is vague — `lint_icon.rs` describes the actual content. (Alternative considered: fold it into `framework_keymap.rs` too. Rejected because the lint-icon mapping is a domain → framework data translation, not a key-binding adapter — they don't share imports or callers.)

### What stays where

- `src/config.rs` (1144 lines): single coherent domain (TOML schema + serde impls for the full app config). A future split could separate `keyboard` config from the main schema, but no agent in pass 1 identified two domains here.
- `src/tui/app/mod.rs` (1197 lines, of which only 103 are production): production section is the App struct and its mutation-guard pattern docs. The rest is `#[cfg(test)]`. Not a split candidate.
- `src/tui/settings.rs` (1699 lines, of which only 103 are production): same pattern as `app/mod.rs` — small production module with a large inline test block. Tests could be extracted to `tests/settings/` later, but it is not a *placement* concern.
- `src/tui/columns/mod.rs` (1177 lines, 744 production): column layout and cell rendering are mutually dependent — width recompute reads column rules, and rendering writes back the chosen widths. Splitting would force `pub`-widening on the shared types without reducing the import surface.
- `src/tui/pane/` vs `src/tui/panes/` (singular vs plural): the naming is paired but the contents are correctly separated (`pane/` = trait machinery + chrome; `panes/` = concrete `impl Pane` for each visible pane). A rename to `pane_frame/` was considered and rejected — the cost (caller import churn across ~40 files) exceeds the clarity gain.
- `src/tui/app/async_tasks/`: 14 files under one directory, split between handlers (8 files) and coordinators (4 files). Agent C proposed nesting handlers in their own subdir; not enough cohesion gain to justify the import churn.

### Module re-exports

#### `src/tui/pane/mod.rs`

Add re-export for the relocated `DismissTarget`:

```rust
// existing
mod chrome;
mod dispatch;
mod layout;
mod rules;
mod state;
mod title;

// new
mod dismiss;

pub use chrome::PaneChrome;
pub use dispatch::Pane;
pub use dispatch::Hittable;
pub use dispatch::PaneRenderCtx;
pub use state::PaneFocusState;
// ... existing exports

pub use dismiss::DismissTarget;   // NEW
```

#### `src/tui/app/dismiss.rs`

Replace the `pub enum DismissTarget { ... }` definition with `use crate::tui::pane::DismissTarget;`. The `App::dismiss(target)` and `App::dismiss_target_for_row(row)` methods stay; only the type definition moves.

#### `src/tui/app/mod.rs`

Drop the line `pub(super) use dismiss::DismissTarget;`. Callers that reached `DismissTarget` via `crate::tui::app::DismissTarget` now reach it via `crate::tui::pane::DismissTarget`. The 8 known call sites (per `rg`):

```
src/tui/pane/dispatch.rs        : already in pane/, becomes super::DismissTarget
src/tui/panes/project_list.rs   : crate::tui::app::DismissTarget → crate::tui::pane::DismissTarget
src/tui/panes/pane_impls.rs     : crate::tui::app::DismissTarget → crate::tui::pane::DismissTarget
src/tui/interaction.rs          : super::app::DismissTarget       → super::pane::DismissTarget
src/tui/project_list/mod.rs     : super::app::DismissTarget       → crate::tui::pane::DismissTarget
src/tui/app/tests/interaction.rs: crate::tui::app::DismissTarget → crate::tui::pane::DismissTarget
src/tui/app/tests/mod.rs        : super::DismissTarget            → crate::tui::pane::DismissTarget
src/tui/app/tests/rows.rs       : (uses through method calls; no direct import)
```

#### `src/tui/integration/mod.rs`

```rust
// before
mod config_reload;
mod framework_keymap;
mod lint_display;
mod toast_adapters;

pub(super) use config_reload::*;
pub(super) use framework_keymap::*;
pub(super) use lint_display::icon_for;
pub(super) use toast_adapters::owner_repo_key;
pub(super) use toast_adapters::path_key;

// after
mod config_reload;
mod framework_keymap;
mod lint_icon;

pub(super) use config_reload::*;
pub(super) use framework_keymap::*;
pub(super) use framework_keymap::owner_repo_key;
pub(super) use framework_keymap::path_key;
pub(super) use lint_icon::icon_for;
```

#### `src/tui/panes/mod.rs`

```rust
// before
mod support;
pub(super) use support::*;

// after
mod pane_data;
pub(super) use pane_data::*;
```

### Sequencing

Each step is an in-flight checkpoint — run `cargo build` then `cargo nextest run` between steps. The whole phase lands as one commit once the final checkpoint is green.

1. **Rename `tui/panes/support.rs` → `tui/panes/pane_data.rs`.** Update `tui/panes/mod.rs` (`mod support;` → `mod pane_data;` and the re-export glob). No call-site imports change — every consumer reached the contents through `pub(super) use support::*;`. Checkpoint: build + tests.

2. **Create `tui/pane/dismiss.rs` containing the `DismissTarget` enum** (move the definition out of `tui/app/dismiss.rs`). Add `mod dismiss;` and `pub use dismiss::DismissTarget;` to `tui/pane/mod.rs`. Leave the old import in `tui/app/dismiss.rs` for the duration of this step — `pub use crate::tui::pane::DismissTarget;` at the bottom of the file so `crate::tui::app::DismissTarget` keeps resolving. Checkpoint: build (no caller edits yet — should compile clean).

3. **Update the 7 caller import sites** (see list above) to point at `crate::tui::pane::DismissTarget`. Remove the temporary `pub use` from `tui/app/dismiss.rs`. Remove `pub(super) use dismiss::DismissTarget;` from `tui/app/mod.rs`. Checkpoint: build + tests.

4. **Fold `tui/integration/toast_adapters.rs` into `framework_keymap.rs`.** Append the two functions (`path_key`, `owner_repo_key`) and their tests to `framework_keymap.rs`. Delete `toast_adapters.rs`. Update `tui/integration/mod.rs` (drop `mod toast_adapters;` and the two re-exports, add re-exports from `framework_keymap`). Checkpoint: build + tests.

5. **Rename `tui/integration/lint_display.rs` → `tui/integration/lint_icon.rs`.** Update `tui/integration/mod.rs` (`mod lint_display;` → `mod lint_icon;`). The single re-export `pub(super) use … icon_for;` keeps working since the symbol name doesn't change. Checkpoint: build + tests.

6. **Final cargo checkpoint.** Run `cargo build` and `cargo nextest run` once more. Commit.

---

---

## Phase 2 — Split `src/watcher.rs` (4155 lines)

`watcher.rs` mixes four responsibilities: filesystem-event reception via `notify`, debounce state (`WatchState`, `WatcherLoopState`), per-domain refresh dispatch (disk-usage updates, git-info updates), and root-registration bookkeeping (`RegisteredRoots`, cargo-home registration). Both `criterion 3` (size) and `criterion 2` (mixed domains) of `when-to-split-a-module.md` are met. 65% of total lines are tests, but production alone is ~1430 lines.

### Target layout

```
src/watcher/
├── mod.rs                  ~250 lines: WatchRequest, WatcherMsg, spawn_watcher, WatcherLoopContext
├── roots.rs                ~180 lines: RegisteredRoots, register_watch_roots, cargo-home registration, WatchRootRegistrationFailure
├── debounce.rs             ~200 lines: WatchState, WatcherLoopState, is_ready_to_launch, drain helpers
├── events.rs               ~350 lines: process_notify_events, drain_notify_events, replay_buffered_events, handle_notify_event, handle_event, EventContext, WatcherDispatchContext
├── git_refresh.rs          ~250 lines: fire_git_updates, spawn_git_refresh, enqueue_git_refresh, is_fast/internal/worktree_git_*_event, git_refresh_key, emit_root_git_info_refresh
├── disk_refresh.rs         ~120 lines: fire_disk_updates, spawn_disk_update, schedule_disk_refresh, handle_disk_completion, is_target_event_for, is_target_metadata_event
└── probe.rs                ~150 lines: probe_new_projects, project_level_dir, probe_project, spawn_project_refresh, spawn_project_refresh_after
```

The `#[cfg(test)] mod tests` block from `watcher.rs` moves to `watcher/tests.rs` (single file) rather than spreading across submodules — the existing tests exercise the loop end-to-end and don't decompose along the new boundaries.

### Sequencing

1. Create `watcher/mod.rs` shell, move `WatchRequest`, `WatcherMsg`, `spawn_watcher`, `WatcherLoopContext` in. Delete `src/watcher.rs`. Build checkpoint.
2. Extract `roots.rs` (no internal deps).
3. Extract `debounce.rs` (no internal deps).
4. Extract `disk_refresh.rs` (depends on `debounce`).
5. Extract `git_refresh.rs` (depends on `debounce`).
6. Extract `events.rs` (depends on `disk_refresh`, `git_refresh`, `debounce`).
7. Extract `probe.rs` (called from `events.rs`).
8. Move tests to `watcher/tests.rs`. Build + `cargo nextest run`. Commit.

External callers (`tui/background.rs`, `tui/app/construct.rs`, `tui/app/async_tasks/lint_runtime.rs`) reach `WatchRequest`, `WatcherMsg`, and `spawn_watcher` — all re-exported by `watcher/mod.rs`. Zero caller edits.

---

## Phase 3 — Split `src/tui/panes/pane_data.rs` (1913 lines, after Phase 1 rename)

The file defines per-pane detail data — five independent type clusters (PackageData, GitData, CiData, LintsData, TargetsData) plus the `DetailField` enum and per-domain builder functions. `criterion 1` (multiple type clusters) + `criterion 3` (size) met.

### Target layout

```
src/tui/panes/pane_data/
├── mod.rs                  ~120 lines: DetailPaneData facade, build_pane_data, build_pane_data_for_member, build_pane_data_for_workspace_ref. Re-exports each domain's public types.
├── detail_field.rs         ~200 lines: DetailField enum, field-formatting helpers shared by package and git renderers
├── package.rs              ~350 lines: PackageData, package_fields_from_data, builders
├── git.rs                  ~450 lines: GitData, GitRow, RemoteRow, WorktreeInfo, git_fields_from_data, git_row_at
├── ci.rs                   ~200 lines: CiData, CiEmptyState, CiFetchKind, build_ci_data
├── lints.rs                ~100 lines: LintsData, lint-row formatting
└── targets.rs              ~250 lines: TargetsData, TargetEntry, BuildMode, builders
```

`detail_field` is the only internal dependency (package.rs and git.rs both use `DetailField`). All five domain submodules depend only on `detail_field` and external types from `project/`, `lint/`, `ci`, `tui/state`.

### Sequencing

1. Create `pane_data/mod.rs` and `pane_data/detail_field.rs`. Move `DetailField` and its helpers.
2. Extract `pane_data/ci.rs`, `lints.rs`, `targets.rs` (independent of `detail_field`). Checkpoint after each.
3. Extract `pane_data/package.rs` and `pane_data/git.rs` (depend on `detail_field`).
4. Replace the existing `panes/pane_data.rs` with the new `pane_data/mod.rs` directory. Build + `cargo nextest run`. Commit.

`tui/panes/mod.rs` keeps its `mod pane_data; pub(super) use pane_data::*;` pattern. Zero caller edits across the 11 files in `tui/panes/`.

---

## Phase 4 — Split `src/tui/finder.rs` (1108 lines)

Three responsibilities: building the searchable index, fuzzy-search dispatch, and rendering the finder popup. `criterion 2` (mixed domains) + `criterion 3` met.

### Target layout

```
src/tui/finder/
├── mod.rs                  ~250 lines: FinderItem, FinderKind, FINDER_COLUMN_COUNT, FINDER_HEADERS, public entry points (open_finder, close_finder)
├── index.rs                ~450 lines: build_finder_index, add_workspace_items, add_package_items, add_vendored_items_typed, build_search_tokens
└── dispatch.rs             ~400 lines: refresh_finder_results, highlighted_spans, confirm_finder, dispatch_finder_action, navigate_to_target, render_finder_popup
```

`index` produces a `Vec<FinderItem>`; `dispatch` consumes it. `mod` re-exports `FinderItem` and the public entry points so callers (`tui/render.rs`, `tui/input.rs`, `tui/app/types.rs`, `tui/integration/framework_keymap.rs`) keep their existing `use crate::tui::finder::FinderItem;` paths.

### Sequencing

1. Create `finder/mod.rs` with `FinderItem`, `FinderKind`, constants, and `open_finder`/`close_finder`.
2. Extract `finder/index.rs`. Checkpoint.
3. Extract `finder/dispatch.rs`. Build + `cargo nextest run`. Commit.

---

## Phase 5 — Split `src/tui/input.rs` (873 lines)

Four functional regions: key dispatch (key normalization + per-pane action routing), mouse handling (click + scroll + position tracking), editor/terminal spawning (open-in-editor, shell helpers), and overlay event routing (framework / keymap / finder overlays). `criterion 2` + `criterion 3` met.

### Target layout

```
src/tui/input/
├── mod.rs                  ~200 lines: handle_event entry point, overlay dispatch, LAST_MOUSE_POS static
├── keys.rs                 ~350 lines: handle_key_event, key normalization, per-pane action routing
├── mouse.rs                ~150 lines: handle_mouse_event, scroll_pane_at, click dispatch
└── editor_terminal.rs      ~200 lines: open_path_in_editor, spawn_terminal_command, shell-detection helpers
```

`handle_event` in `mod.rs` is the single public entry point; the three submodules are private helpers reached only from `mod.rs`. No caller changes.

### Sequencing

1. Create `input/mod.rs` with the `handle_event` entry point and overlay routing.
2. Extract `input/editor_terminal.rs` (no internal deps).
3. Extract `input/mouse.rs` (no internal deps).
4. Extract `input/keys.rs` (no internal deps). Build + `cargo nextest run`. Commit.

---

## Phase 6 — Split `src/http.rs` (981 lines, 737 production)

The file mixes shared infrastructure (`HttpClient`, retry, error types), rate-limit parsing (`RateLimitBucket`, `RateLimitQuota`, header parsing), GitHub REST + GraphQL methods, and crates.io methods. `criterion 2` + `criterion 3` met.

### Target layout

```
src/http/
├── mod.rs                  ~200 lines: HttpClient struct, ServiceKind, ServiceSignal, retry loop, public constructor
├── rate_limit.rs           ~250 lines: RateLimitBucket, RateLimitQuota, GitHubRateLimit, header parsing
├── github.rs               ~300 lines: GitHub REST endpoints, GraphQL execution, paging helpers
└── crates_io.rs            ~80 lines: crates.io REST endpoints
```

Dependency order: `rate_limit` and `crates_io` are leaves; `github` uses `rate_limit`; `mod` orchestrates all three. The 244-line inline `#[cfg(test)] mod tests` block is GitHub-and-rate-limit focused — it moves to `github.rs` and `rate_limit.rs` along with the production code each section exercises.

External callers (~16 files) import `HttpClient`, `ServiceKind`, `ServiceSignal`, `GitHubRateLimit` — all re-exported by `mod.rs`. No caller edits.

### Sequencing

1. Create `http/mod.rs` shell with `HttpClient`, `ServiceKind`, `ServiceSignal`, retry loop.
2. Extract `http/rate_limit.rs` (leaf).
3. Extract `http/crates_io.rs` (leaf).
4. Extract `http/github.rs` (depends on `rate_limit`).
5. Distribute the inline tests to their domain submodules. Build + `cargo nextest run`. Commit.

---

## Phase 7 — Split `src/keymap.rs` (1579 lines, 1004 production)

Three responsibilities: types (`KeyBind`, `Keymap`, `ScopeMap`, `ResolvedKeymap`, action enums), TOML parse/normalize (load, default-merge, serialize-back), and display formatting (glyph rendering, human-readable strings). `criterion 1` (multiple type clusters) + `criterion 3` met.

The nine pane-specific action enums (`CiRunsAction`, `GitAction`, `LintsAction`, `OutputAction`, `PackageAction`, `ProjectListAction`, `TargetsAction`, `FinderAction`, `LangAction`) **stay in `keymap/mod.rs`** for now. Moving them to `tui/panes/*.rs` was considered: it would put each action enum next to its dispatcher, but it would also force `tui/panes/` (which currently has no public types reaching outside `tui/`) to be imported by `tui/integration/framework_keymap.rs` and the keymap parser. The current direction (parser depends on action types defined alongside it) is the right one — defer the pane-action relocation as a follow-up, scoped against the same plan if it comes up later.

### Target layout

```
src/keymap/
├── mod.rs                  ~400 lines: KeyBind, Keymap, ScopeMap, ResolvedKeymap, action enums (Global + 9 pane-specific), Registering, Configuring, VimMode, Navigation
├── parse.rs                ~400 lines: load_from_path, parse_toml, merge_with_defaults, write_back, normalize
└── display.rs              ~200 lines: KeyBind::Display, KeyBind::pretty_string, glyph rendering
```

`parse` and `display` depend on the types in `mod.rs`; neither depends on the other. The 575-line inline test block stays in whichever submodule's behavior it exercises (parse tests → `parse.rs`, display tests → `display.rs`).

### Sequencing

1. Create `keymap/mod.rs` shell with all type definitions. Delete `src/keymap.rs`.
2. Extract `keymap/display.rs`. Checkpoint.
3. Extract `keymap/parse.rs`. Distribute inline tests. Build + `cargo nextest run`. Commit.

20+ callers import via `use crate::keymap::*` — file-path change only, no symbol rewrites.

---

## Phase 8 — Split `src/tui/keymap_ui.rs` (1165 lines, 1057 production)

Two responsibilities: the keymap-popup rendering pipeline (rows, lines, columns, scroll state) and the rebinding workflow (capture, conflict detection, apply, save-to-disk via TOML write-back). `criterion 1` + `criterion 3` met.

### Target layout

```
src/tui/keymap_ui/
├── mod.rs                  ~150 lines: KeymapUi struct, public entry points (open_keymap, close_keymap), state coordination
├── view.rs                 ~450 lines: build_rows, build_lines, KeymapLines, column layout, scroll dispatch
└── rebind.rs               ~450 lines: handle_captured_bind, conflict detection, apply_rebind, save_keymap_to_disk, write_navigation_section, write_app_pane_sections
```

The "settings popup integration" surface earlier mentioned by pass 2 turned out to be small (~50 lines) and lives in `mod.rs` alongside the public entry points — not enough material for its own submodule.

`view` and `rebind` both depend on `mod.rs` types; neither depends on the other.

### Sequencing

1. Create `keymap_ui/mod.rs` with KeymapUi struct and entry points.
2. Extract `view.rs`.
3. Extract `rebind.rs`. Build + `cargo nextest run`. Commit.

---

## Deferred

- **`src/tui/project_list/mod.rs` further split** (1824 lines): pass 1 flagged this for tree-extraction; investigation showed `regroup_workspace`, `linked_worktree_identity`, and `try_insert_member` already live in `grouping.rs`. The remaining 1824 lines are the `ProjectList` struct, its accessors, and mutation methods — one coherent thing. Revisit only if a second domain develops (e.g., disk-cache logic outgrows its current section).
- **Pane-action relocation**: move the nine pane-specific action enums from `keymap/mod.rs` to their respective `tui/panes/*.rs` files. Tradeoff: better locality of pane behavior, but adds an import edge `keymap → tui` that today doesn't exist. Wait until the cost is concrete (e.g., a pane gains a sub-action enum that doesn't fit in a single file).
- **Test extraction from `src/tui/settings.rs`**: 1562 production lines is still above the 500-line line but earlier passes found no second domain. Revisit if the production half grows.
- **`src/config.rs` (730 production lines, single domain)**: no second cluster identified. Revisit only if a second config domain is added.
- **`src/tui/app/async_tasks/` handler-vs-coordinator regrouping**: defer; the directory is large but each file is small and the role-split (handlers vs dispatch/poll/tree) is already clear from filenames.
