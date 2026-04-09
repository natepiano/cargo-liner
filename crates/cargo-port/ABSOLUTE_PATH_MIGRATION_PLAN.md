# AbsolutePath Migration Plan

## Goal

Finish the intended migration so project identity is represented by `AbsolutePath`
where it is semantically an absolute project root, and reserve display paths for UI
rendering only.

That means:

- `RustProject` and `NonRustProject` store typed absolute paths, not raw `PathBuf`
- selection, dedup, expansion restore, and project lookup use absolute identity
- `display_path()` remains available for rendering and labels, not app logic

## Current Gaps

The type exists already in [src/project.rs](src/project.rs), but the project model
still stores raw `PathBuf`:

- `RustProject.path` and `worktree_primary_abs_path`
- `NonRustProject.path`

The bigger migration gap is in TUI state and lookup code, which still uses
display-path strings as identity in several places:

- selection sync and persisted selection
- expand/collapse restore
- vendored/member classification
- lint registration lookup and discovery dedup

Repo-relative git paths are not part of this migration and should remain as-is.

## Phase 1: Tighten The Project Model

### Changes

- change `RustProject.path` from `PathBuf` to `AbsolutePath`
- change `NonRustProject.path` from `PathBuf` to `AbsolutePath`
- change `worktree_primary_abs_path` from `Option<PathBuf>` to
  `Option<AbsolutePath>`
- update constructors, clones, and accessors in `src/project.rs`
- prefer explicit accessors:
  - `abs_path(&self) -> &AbsolutePath`
  - `path(&self) -> &Path` only as an interop helper
- move `display_path()` to derive from the stored `AbsolutePath`

### Important Constraint

`AbsolutePath` currently wraps `PathBuf` without enforcing absoluteness. That makes
the type advisory instead of real.

Tighten construction by doing one of:

- add `AbsolutePath::new(path: PathBuf) -> Result<Self, _>`
- add `AbsolutePath::assert(path: PathBuf) -> Self`
- keep `From<PathBuf>` only if it asserts `path.is_absolute()`

The invariant should be explicit and local.

### Files

- `src/project.rs`
- `src/scan.rs`
- `src/tui/app/tests.rs`
- any direct constructors in tests and fixtures

### Acceptance

- no project identity field in `RustProject` or `NonRustProject` is raw `PathBuf`
- all project constructors require or produce `AbsolutePath`
- no code relies on unchecked project-root `PathBuf` storage

## Phase 2: Convert TUI Selection State To Absolute Identity

### Changes

- change `SelectionPaths` to store absolute identity instead of `Option<String>`
- persist last-selected as an absolute path string on disk
- make selection sync compare absolute paths, not display strings
- keep display strings derived on demand for rendering only

### Files

- `src/tui/app/types.rs`
- `src/tui/app/query.rs`
- `src/tui/app/navigation.rs`
- `src/tui/terminal.rs`

### Notes

This is the highest-value behavioral cleanup.

Today `sync_selected_project()` still keys off `selected_display_path()`, and the
persistence file in `last_selected.txt` also stores the display string. That keeps
home-relative formatting coupled to project identity.

### Acceptance

- selection state is keyed by absolute path
- changing how display paths are formatted does not affect selection restoration
- persisted last-selected state survives only on absolute identity

## Phase 3: Replace Display-Path Lookup Helpers

### Changes

- replace helpers like `is_vendored_path(&str)` with path-based versions
- replace `is_workspace_member_path(&str)` with path-based versions
- remove lookup code that scans for `display_path() == target`
- when expanding/selecting by a target, use absolute path inputs

### Files

- `src/tui/app/query.rs`
- `src/tui/app/navigation.rs`
- `src/project_list.rs` if helper interfaces need adjustment

### Acceptance

- no app-logic helper classifies projects by display string
- vendored/member checks are stable even if display formatting changes
- expand/collapse restore works by absolute path

## Phase 4: Remove Display-Path Logic From Async And Lint Wiring

### Changes

- stop registering lint projects via display-path-driven lookup
- remove `register_lint_for_path(&str)` or convert it to absolute-path input
- dedup discovered projects by absolute path only
- keep `project_path` strings in lint runtime only if they are truly labels

### Files

- `src/tui/app/async_tasks.rs`
- `src/lint/runtime.rs`
- `src/watcher.rs` if any discovery paths still round-trip through display strings

### Notes

`RegisterProjectRequest.project_path` may still be useful as a human-readable label,
but it should not be required for identity or lookup. If retained, rename it to make
that role obvious, for example `project_label`.

### Acceptance

- async flows do not recover project identity from display strings
- lint registration and unregistering operate on absolute identity
- discovered-project dedup does not allocate home-relative strings

## Phase 5: Make The Type Boundary Explicit

### Changes

- prefer returning `DisplayPath` instead of plain `String` for display-only APIs
- use `AbsolutePath` in maps, sets, and task-tracking keys where the value is meant
  to identify a project root
- leave transient filesystem and external API interop as `Path` or `PathBuf`

### Files

- `src/project.rs`
- `src/project_list.rs`
- `src/tui/toasts/manager.rs`
- TUI render/detail/finder code where display values are produced

### Acceptance

- the codebase communicates intent through types:
  - `AbsolutePath` for identity
  - `DisplayPath` for rendering
  - `Path` or `PathBuf` for generic filesystem interop

## Compatibility Decisions

### Persisted Selection

Decide one:

1. Migrate old `last_selected.txt` contents if they start with `~/`
2. Treat old display-path persistence as incompatible and clear it on first load

Option 2 is simpler and probably acceptable unless preserving selection across this
upgrade matters.

### Test Updates

Many tests currently construct projects with fake `~/...` paths. Those should stop
standing in for absolute identity.

Recommended approach:

- use absolute fixture paths for identity-heavy tests
- keep dedicated tests for `home_relative_path()` and display formatting separately

## Suggested Order

1. Tighten `AbsolutePath` construction and convert project model fields
2. Convert selection state and persisted last-selected identity
3. Replace expand/select helpers with path-based variants
4. Remove display-string lookup from async/lint/discovery flows
5. Tighten API boundaries with `DisplayPath` where useful
6. Run targeted tests after each phase

## Targeted Test Coverage

- selection survives tree rebuild by absolute path
- expand/collapse restore survives display formatting changes
- search confirm selects the intended project by absolute path
- vendored/member classification uses absolute path only
- lint registration and unregistering do not depend on display strings
- discovered-project dedup rejects same absolute project with different display form

## Definition Of Done

- `RustProject` and `NonRustProject` use `AbsolutePath` for project identity fields
- project-identity logic no longer uses display strings
- display formatting changes cannot break selection, lookup, or dedup
- repo-relative git internals remain unchanged
- remaining `PathBuf` usage is generic interop, not leaked project identity
