# Source module restructure plan

Target: tighten the directory layout of `src/` so every file lives either at
the crate root (genuinely top-level concerns) or inside a directory submodule
that groups files by responsibility. No flat sprawl of unrelated single-file
modules at any layer. After placement is settled, split the three remaining
over-large files along their natural seams.

## Phase overview

| Phase | What                                                       | Risk    | Rough size |
|-------|------------------------------------------------------------|---------|------------|
| 1     | Placement — group `tui/` and `project/` files into dirs    | Low     | ~80–120 import updates, 6 commits |
| 2     | Split `src/scan.rs` (1988 prod lines)                      | Medium  | New `src/scan/` dir with 6–7 files |
| 3     | Split `src/tui/project_list.rs` (2352 lines)               | Medium  | New `src/tui/project_list/` dir with 4 files |
| 4     | Move `src/tui/interaction.rs` test module out              | Low     | One-file move; production stays one file |
| 5     | Fix `lint/types.rs` → `tui_pane::Icon` domain-to-UI leak   | Low     | One new adapter file in `tui/integration/` |
| —     | Optional: `tui/app/api.rs` stable surface                  | n/a     | Speculative; revisit later |

Phases are ordered to minimise re-shuffling: placement (1) makes the
directories that 2–5 fill. Within phases 2–4 the order is independent. Phase
5 can land any time after phase 1.

---

## Phase 1 — Placement

### Proposed layout

```
src/
├── main.rs
├── cache_paths.rs
├── ci.rs
├── config.rs
├── constants.rs
├── enrichment.rs
├── http.rs
├── keymap.rs
├── perf_log.rs
├── scan.rs
├── test_support.rs
├── watcher.rs
│
├── lint/                       (unchanged)
│
├── project/
│   ├── mod.rs
│   ├── info.rs
│   ├── member_group.rs
│   ├── non_rust.rs
│   ├── paths.rs
│   ├── project_entry.rs
│   ├── project_fields.rs
│   ├── root_item.rs
│   ├── vendored_package.rs
│   │
│   ├── cargo/                  (NEW)
│   │   ├── mod.rs
│   │   ├── cargo.rs
│   │   ├── metadata_store.rs   (renamed from cargo_metadata_store.rs)
│   │   ├── package.rs
│   │   ├── rust_info.rs
│   │   ├── rust_project.rs
│   │   └── workspace.rs
│   │
│   └── git/                    (NEW)
│       ├── mod.rs
│       ├── git.rs
│       ├── submodule.rs
│       └── worktree_group.rs
│
└── tui/
    ├── mod.rs
    ├── background.rs
    ├── constants.rs
    ├── cpu.rs
    ├── finder.rs
    ├── input.rs
    ├── interaction.rs
    ├── keymap_ui.rs
    ├── project_list.rs
    ├── render.rs
    ├── settings.rs
    ├── terminal.rs
    ├── test_support.rs
    │
    ├── app/                    (unchanged internally)
    ├── columns/                (unchanged)
    ├── overlays/               (now also owns popup.rs)
    ├── pane/                   (unchanged)
    ├── panes/                  (unchanged)
    │
    ├── state/                  (NEW)
    │   ├── mod.rs
    │   ├── ci.rs               (was ci_state.rs)
    │   ├── config.rs           (was config_state.rs)
    │   ├── inflight.rs         (was tui/inflight.rs)
    │   ├── keymap.rs           (was keymap_state.rs)
    │   ├── lint.rs             (was lint_state.rs)
    │   ├── net.rs              (was net_state.rs)
    │   └── scan.rs             (was scan_state.rs)
    │
    ├── integration/            (NEW)
    │   ├── mod.rs
    │   ├── config_reload.rs
    │   ├── framework_keymap.rs
    │   └── toast_adapters.rs
    │
    └── support/                (NEW)
        ├── mod.rs
        ├── duration.rs         (was duration_fmt.rs)
        ├── running_tracker.rs
        └── watched_file.rs
```

### Moves, with rationale

#### `src/project/` → `cargo/` + `git/`

The 18 files in `project/` split cleanly along a domain seam that today is
only visible by reading filenames.

**`project/cargo/`** — how cargo-port understands a Rust project on disk:
`cargo.rs`, `cargo_metadata_store.rs` (renamed to `metadata_store.rs`),
`package.rs`, `rust_info.rs`, `rust_project.rs`, `workspace.rs`.

**`project/git/`** — how cargo-port understands the repository state around
it: `git.rs`, `submodule.rs`, `worktree_group.rs`.

**Stays at `project/` root** — the project-level abstractions that aggregate
both sides: `info.rs`, `member_group.rs`, `non_rust.rs`, `paths.rs`,
`project_entry.rs`, `project_fields.rs`, `root_item.rs`, `vendored_package.rs`.

The `cargo_metadata_store.rs` → `cargo/metadata_store.rs` rename drops the
redundant `cargo_` prefix now that the directory provides the namespace.

#### `src/tui/state/`

Seven files match the same pattern: an owned subsystem state struct held as a
field on `App`, with no behavior beyond housekeeping. They hold; they don't
drive. Moves (the `_state` suffix drops since the directory name supplies it):

- `tui/ci_state.rs`     → `tui/state/ci.rs`
- `tui/config_state.rs` → `tui/state/config.rs`
- `tui/inflight.rs`     → `tui/state/inflight.rs`
- `tui/keymap_state.rs` → `tui/state/keymap.rs`
- `tui/lint_state.rs`   → `tui/state/lint.rs`
- `tui/net_state.rs`    → `tui/state/net.rs`
- `tui/scan_state.rs`   → `tui/state/scan.rs`

`Inflight` belongs in `state/` because it is App-owned in-flight bookkeeping
that pairs with `Lint` (running tracker) and `Ci` (fetch lifecycle). It is
not channel infrastructure.

#### `src/tui/background.rs` (stays flat)

`background.rs` is one file owning the four `mpsc` channel pairs plus the
watcher sender — pure channel infrastructure. With `inflight.rs` moving into
`state/`, there is no second file to group with. A single-file directory
would be noise.

#### `src/tui/integration/`

Three files bridge cargo-port domain types into the generic `tui_pane`
framework — the wiring layer where this binary plugs into the library:

- `tui/config_reload.rs`    → `tui/integration/config_reload.rs`
- `tui/framework_keymap.rs` → `tui/integration/framework_keymap.rs`
- `tui/toast_adapters.rs`   → `tui/integration/toast_adapters.rs`

#### `src/tui/support/`

Three plain helpers with no subsystem affiliation and no framework glue role:

- `tui/duration_fmt.rs`     → `tui/support/duration.rs`
- `tui/running_tracker.rs`  → `tui/support/running_tracker.rs`
- `tui/watched_file.rs`     → `tui/support/watched_file.rs`

(Project convention: shared-helper modules are named `support`, not `util`.)

#### `tui/popup.rs` → `tui/overlays/popup.rs`

Overlay frame chrome (borders, centering). Three of its four consumers
(`finder`, `keymap_ui`, `settings`) are overlay screens; the fourth
(`render.rs`) only calls into it to draw overlay frames. It belongs next to
the overlay implementations, not in a generic helper bucket.

### What stays where

**Crate root files** — `cache_paths`, `ci`, `config`, `constants`,
`enrichment`, `http`, `keymap`, `perf_log`, `scan`, `watcher`,
`test_support`, `main`. Each is a single cross-cutting concern at the binary
level. Not flat sprawl; top-level entry points.

**`tui/` files that stay flat** — `background.rs`, `constants.rs`, `cpu.rs`,
`finder.rs`, `input.rs`, `interaction.rs`, `keymap_ui.rs`, `project_list.rs`,
`render.rs`, `settings.rs`, `terminal.rs`, `test_support.rs`. Each is either
a single coherent screen or a top-level TUI concern (event adapter, frame
dispatcher, crossterm loop, CPU sampler bundled with its rendering).

**`tui/app/`** — `app/ci.rs`, `app/construct.rs`, `app/dismiss.rs`,
`app/phase_state.rs`, `app/startup.rs`, `app/target_index.rs`,
`app/types.rs` are all method bundles on `App` plus its internal state
enums. The `async_tasks/`, `navigation/`, `query/`, `tests/`
subdirectories are correctly grouped already.

**`lint/`** — already a flat, single-purpose module. Subdividing would
split files that work as one runner.

### Module re-exports

Each new directory uses a `mod.rs` that re-exports its members at the
visibility the parent expects, so consumer `use` paths stay short.

- `tui/state/mod.rs`
  ```rust
  mod ci; mod config; mod inflight; mod keymap; mod lint; mod net; mod scan;
  pub(super) use ci::Ci;
  pub(super) use config::Config;
  pub(super) use inflight::Inflight;
  pub(super) use keymap::Keymap;
  pub(super) use lint::Lint;
  pub(super) use net::Net;
  pub(super) use scan::Scan;
  ```
  `tui/app/mod.rs` imports become `use super::state::{Ci, Config, …}`.

- `tui/integration/mod.rs` re-exports the public types from
  `config_reload`, `framework_keymap`, `toast_adapters` at `pub(super)` so
  `crate::tui::framework_keymap::…`-style imports collapse to
  `crate::tui::integration::…`.

- `tui/support/mod.rs` declares `mod duration; mod running_tracker; mod
  watched_file;` with `pub(super) use` re-exports for `RunningTracker`,
  `WatchedFile`, and the duration formatters. Callers use
  `super::support::…`.

- `project/cargo/mod.rs` and `project/git/mod.rs` re-export every current
  `pub(super)` / `pub(crate)` type at the directory level so
  `project/mod.rs`'s `pub(crate) use` lines change to
  `pub(crate) use cargo::metadata_store::…` (etc.) and external imports
  through `crate::project::…` keep working unchanged.

### Sequencing

State and integration files import from `support`, so `support` lands first.

1. `tui/support/` — move + rename; update every `super::running_tracker`,
   `super::watched_file`, `super::duration_fmt` import.
2. `tui/state/` — move + rename; absorbs `tui/inflight.rs`; updates the
   `tui/app/mod.rs` field-declaration imports.
3. `tui/overlays/popup.rs` — move `tui/popup.rs` into `overlays/`; four
   consumers update their imports.
4. `tui/integration/` — move; no renames.
5. `project/cargo/` — move + `cargo_metadata_store.rs` →
   `metadata_store.rs` rename; update `project/mod.rs` and every
   cross-import.
6. `project/git/` — move.

Each step is one commit. `cargo build` + `cargo nextest run` between
steps.

---

## Phase 2 — Split `src/scan.rs`

`src/scan.rs` is 2319 lines, 1988 of which are production code (tests start
at line 1989). It mixes the `BackgroundMsg` dispatch contract with five
unrelated subsystems that happen to share the file because they all run on
background threads.

### Target layout

```
src/scan/
├── mod.rs            (BackgroundMsg, service signal emitters, top-level API)
├── ci_cache.rs       (CI run cache: load_all/save/fetch/merge, exhausted flag)
├── discovery.rs      (project discovery, project details, repo cache, phase1)
├── cargo_metadata.rs (streaming scan context, metadata refresh, fingerprints)
├── disk_usage.rs     (disk usage tree, dir sizes, target/non-target split)
├── language_stats.rs (tokei integration)
└── tree.rs           (tree building, worktree merging, vendored extraction,
                       member grouping, workspace patterns)
```

### What goes where

- **`mod.rs`** — `BackgroundMsg`, the service-signal emitters
  (`emit_service_signal`, `emit_service_recovered`, `emit_git_info`),
  `combine_service_signal`, the existing `pub(crate)` top-level entry
  points (`cache_dir`, etc. that don't belong in a subsystem).

- **`ci_cache.rs`** — currently lines 363–560: `CiFetchResult`,
  `repo_cache_dir`, `ci_cache_dir`, `ci_cache_dir_pub`, `is_exhausted`,
  `mark_exhausted`, `clear_exhausted`, `save_cached_run`, `load_cached_run`,
  `load_all_cached_runs`, `fetch_recent_runs`, `merge_runs`,
  `fetch_ci_runs_cached`, `fetch_older_runs`.

- **`tree.rs`** — currently lines 569–1050: `dir_size`, `build_tree`,
  `workspace_member_paths_new`, `workspace_member_patterns`,
  `normalize_workspace_path`, the workspace pattern matchers,
  `item_worktree_identity`, `item_is_linked`, `merge_worktrees_new`,
  `extract_vendored_new`, `group_members_new`, `cargo_project_to_item`.

- **`discovery.rs`** — currently lines 1050–1230 plus 1616–1720:
  `discover_project_item`, `fetch_project_details`, `new_repo_cache`,
  `load_cached_repo_data`, `store_cached_repo_data`,
  `invalidate_cached_repo_data`, `resolve_include_dirs`, `expand_home_path`,
  the phase-1 discovery (`Phase1DiscoverStats`, `Phase1DiscoverResult`,
  `discover_non_rust_project`, `phase1_discover`).

- **`cargo_metadata.rs`** — currently lines 1232–1620:
  `StreamingScanContext`, `spawn_streaming_scan`,
  `collect_cargo_metadata_roots`, `cargo_metadata_roots_for_item`,
  `spawn_cargo_metadata_tree`, `MetadataDispatchContext`,
  `spawn_cargo_metadata_refresh`, `CargoMetadataTaskOutput`,
  `spawn_out_of_tree_target_walk`, `sum_dir_bytes`,
  `run_cargo_metadata_for_root`, `execute_cargo_metadata`,
  `format_cargo_metadata_error`, `synthetic_fingerprint`,
  `build_workspace_metadata`.

- **`language_stats.rs`** — currently lines 1723–1820:
  `spawn_initial_language_stats`, `spawn_language_stats_tree`,
  `collect_language_stats_for_tree`, `build_language_stats`,
  `collect_language_stats_single`.

- **`disk_usage.rs`** — currently lines 1824–1980:
  `spawn_initial_disk_usage`, `DiskUsageTree`, `group_disk_usage_trees`,
  `spawn_disk_usage_tree`, `DirSizes`, `file_lives_under_target`,
  `dir_sizes_for_tree`, `disk_usage_batch_for_item`.

The existing inline `mod tests` (lines 1989–end) splits across the new
files, each test landing next to the production code it covers.

### Sequencing

The submodules have some cross-dependencies (`cargo_metadata.rs` calls into
`tree.rs`; `discovery.rs` calls into `cargo_metadata.rs`), so move the
leaves first.

1. `ci_cache.rs` — no callers inside scan; pure extraction.
2. `tree.rs` — `cargo_metadata` and `discovery` import from it.
3. `disk_usage.rs` — leaf.
4. `language_stats.rs` — leaf.
5. `cargo_metadata.rs` — depends on `tree.rs`.
6. `discovery.rs` — depends on `cargo_metadata.rs` and `tree.rs`.

Each step is one commit. `cargo build` + `cargo nextest run` between
steps. After all six, `mod.rs` retains only `BackgroundMsg` and the
service-signal layer.

---

## Phase 3 — Split `src/tui/project_list.rs`

2352 lines (no inline test module — all production). One file owns the
`IndexMap<AbsolutePath, ProjectEntry>` data structure, the visible-row
projection, the selection-mutation guard, and workspace regrouping.

### Target layout

```
src/tui/project_list/
├── mod.rs           (ProjectList struct, primary impl, public API)
├── visible_rows.rs  (VisibleRow, ExpandKey, emit_* helpers)
├── selection.rs     (SelectionMutation guard, drop logic)
└── grouping.rs      (regroup_workspace, worktree attachment,
                      member grouping helpers)
```

### What goes where

- **`mod.rs`** — `ProjectList` struct definition (currently line 70), its
  primary impl block (currently lines 83+), `Index<usize>` impl, and the
  pane-trait integration. Keep the public API surface here so `use
  crate::tui::project_list::ProjectList` keeps working unchanged.

- **`visible_rows.rs`** — currently lines 965–1224: `ExpandKey`,
  `VisibleRow`, the second `impl ProjectList` block that emits visible
  rows, `emit_groups`, `emit_vendored_rows`, `emit_submodule_rows`,
  `emit_worktree_group`, `emit_worktree_children`, `worst_git_status`,
  plus the `LegacyRootExpansion` enum and its associated `VisibleRow`
  impl (currently lines 2315+).

- **`selection.rs`** — currently lines 1361–1416: `SelectionMutation`
  struct, its impl, its `Drop` impl. Self-contained guard pattern.

- **`grouping.rs`** — currently lines 752–960: `shortest_unique_suffixes`,
  `display_path_segments`, `join_suffix`, `try_attach_worktree`,
  `item_worktree_identity`, `linked_worktree_identity`,
  `find_matching_worktree_container`, `regroup_workspace`,
  `try_insert_member`.

The two large later impl blocks at lines 1417 and 1751 stay in `mod.rs`
unless they themselves have an internal seam (TBD at extraction time).

### Sequencing

1. `grouping.rs` — free functions with no cross-dependencies; pure extract.
2. `selection.rs` — self-contained guard; references `ProjectList` but
   nothing else.
3. `visible_rows.rs` — depends on the `ProjectList` field layout but not on
   `grouping` or `selection`.

Each step is one commit. `cargo build` + `cargo nextest run` between steps.

---

## Phase 4 — Extract tests from `src/tui/interaction.rs`

The file is 1873 lines, but production stops at line 178. The remaining
~1695 lines are an inline `#[cfg(test)] mod tests` block containing
mouse/keyboard integration tests for hover, dismiss, and overlay
blocking. The production code (7 functions: `handle_click`,
`hovered_pane_row_at`, `hit_test_at`, `hit_test_toasts`, `set_pane_pos`,
`viewport_mut_for`, `apply_hovered_pane_row`) is cohesive and small
enough to stay as a single file.

### Action

Move the entire test module to `src/tui/app/tests/interaction.rs`,
matching the existing test-organization pattern (`tests/background.rs`,
`tests/panes.rs`, etc. already live there). The production module
`src/tui/interaction.rs` shrinks to ~178 lines focused purely on
hit-test and viewport routing.

This is a single-commit refactor: cut the test module, paste into
`tui/app/tests/interaction.rs`, fix imports to reach into
`crate::tui::interaction::*` and `crate::tui::*`, register in
`tui/app/tests/mod.rs`.

### What this corrects

Earlier reviews flagged `interaction.rs` for "monolithic dispatch" based
on file size. The production dispatch is in fact small and cohesive;
the size came from comprehensive tests bundled inline. Extracting them
clarifies that interaction.rs is doing one focused job.

---

## Phase 5 — Fix `lint/types.rs` → `tui_pane::Icon` leak

Today `src/lint/types.rs` (domain) imports `tui_pane::Icon` and exposes
`LintStatus::icon()` returning that icon enum. This is a domain layer
depending on the UI framework — the only such leak in the crate.

### Action

1. In `lint/types.rs`, replace `LintStatus::icon()` with a display-agnostic
   accessor — likely `kind()` returning a local `LintStatusKind` enum
   (`Running`, `Passed`, `Failed`, etc.) defined in the same file.
2. Create `src/tui/integration/lint_display.rs` containing the mapping
   from `LintStatusKind` to `tui_pane::Icon` (using the existing
   `ACTIVITY_SPINNER` and the static/animated icon variants).
3. Update every call site that uses `LintStatus::icon()` to go through
   the adapter: `lint_display::icon_for(status.kind())`.
4. Remove the `tui_pane` import from `lint/types.rs`.

Single commit. After this, `lint/` has zero `tui_pane` imports.

---

## Future / optional — `tui/app/api.rs`

Today panes and overlays import internal App types like `DismissTarget`,
`VisibleRow`, `HoveredPaneRow`, `AvailabilityStatus`, `DiscoveryRowKind`
directly from `crate::tui::app::*`. If those internals change, ~35 call
sites need updating.

A stabilizing layer would live at `src/tui/app/api.rs` re-exporting only
the types pane/overlay code is allowed to depend on, while keeping App
state-machine internals private to `app/`. Callers would import
`crate::tui::app::api::DismissTarget` to signal "this is the stable
surface."

This is speculative — worth doing only if internal-type churn becomes
painful in practice. Not scheduled.
