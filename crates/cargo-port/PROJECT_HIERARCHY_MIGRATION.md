# Project Hierarchy Migration Plan

## Goal

Finish the migration to a single hierarchy-backed data/view model:

- `ProjectList` is the source of truth for project data.
- Leaf project identity is absolute-path based.
- `DisplayPath` is render-only.
- View generation reads from hierarchy data, not synchronized copies.

## Rules

### 1. One source of truth

If a value is leaf-attached project data, it should live on the hierarchy node,
not in a separately synchronized `HashMap<AbsolutePath, _>` on `App`.

### 2. Absolute path is identity

Lookup, selection, navigation, mutation, persistence, and reconciliation should
use absolute paths.

`DisplayPath` is not an identifier. It is only a rendered label.

### 3. Derived caches are allowed

Derived caches and render indexes are fine if they are clearly disposable and
rebuildable from the hierarchy.

The problem is not "any cached structure". The problem is maintaining multiple
authoritative data sources for the same project state.

## Remaining work

### 1. CI state

`ci_state: HashMap<AbsolutePath, CiState>` — split into:

- **`ProjectCiInfo` on hierarchy** (`ProjectInfo.ci_data`): runs, total, exhausted flag
- **`ci_fetching: HashSet<AbsolutePath>` on App**: transient fetch-in-progress tracking
- **`ci_display_modes` stays on App**: UI preference, same category as `ci_pane`

Also simplifies branch filtering: always filter by current branch (no
`default_branch` comparison), toggle always available, and show an in-panel
empty state when the current branch has no runs instead of silently falling
back to all runs. See `PHASE1_PLAN.md` for full design.

### 2. Worktree-group versus leaf visibility

`WorktreeGroup` variants in `src/project/worktree_group.rs` carry a separate
`visibility` field independent of leaf `ProjectInfo` visibility.

Design question to settle before acting:

- Is group visibility truly independent UI structure state?
- Or should group-level hiding/collapsing be derived from child visibility?

### 3. Some tests still encode old display-path assumptions

Several tests still assert on `display_path()` strings:

- `src/tui/app/tests/state.rs` — lint runtime tests compare `project_label`
- `src/tui/app/tests/panes.rs` — uses `selected_display_path()` string comparison
- `src/tui/app/tests/search.rs` — asserts `hit.display_path == member_path`
- `src/tui/app/tests/background.rs` — uses `item.display_path()`

Migrate these to path identity assertions once the production APIs they test are
migrated. Keep display-path assertions only where rendering behavior is under
test.

## Recommended order

### Phase 1: Re-evaluate CI ownership (§1)

### Phase 2: Settle group visibility design (§2)

### Ongoing: Update tests (§3)

## Things that do not need migration

These are allowed to remain as derived caches/indexes:

- `cached_visible_rows`
- `cached_root_sorted`
- `cached_child_sorted`
- fit-width build outputs
- search token indexes
- lint rollup indexes (`lint_rollup_status`, `lint_rollup_paths`,
  `lint_rollup_keys_by_path`)

## Definition of done

The migration is done when:

- all project identity lookups use absolute paths
- `DisplayPath` is only used for rendering/presentation
- leaf project data lives on the hierarchy
- remaining caches are clearly derived, not authoritative
- no UI behavior depends on reconciling parallel data sources for the same
  project field
