# Phase 1: Migrate CI State to Hierarchy

## Goal

Split the current `CiState` enum into persistent project data (on the hierarchy)
and transient runtime state (on `App`). Remove
`ci_state: HashMap<AbsolutePath, CiState>` from `App`.

## Core principle: separate data from runtime

`CiState` currently combines project data (CI runs, totals) with fetch lifecycle
state (`Fetching` vs `Loaded`) in a single enum. These have different ownership:

- **CI runs are project data** — they belong on the hierarchy node, just like
  stars and crates.io metadata.
- **Fetch-in-progress is runtime state** — it's transient coordination between
  the UI and async tasks, same category as `pending_ci_fetch`, channels, and
  toasts.

## Design decisions

### CI runs go on the hierarchy as `ProjectCiData`

A new enum on `ProjectInfo`, alongside `GitHubInfo`:

```rust
pub(crate) struct ProjectCiInfo {
    pub runs:         Vec<CiRun>,
    pub github_total: u32,
    pub exhausted:    bool,
}

pub(crate) enum ProjectCiData {
    Unfetched,
    Loaded(ProjectCiInfo),
}
```

`ProjectInfo` gains `ci_data: ProjectCiData`. `Unfetched` means "haven't heard
from GitHub yet" (show loading indicator). `Loaded(ProjectCiInfo)` means we
have a definitive answer — even if `runs` is empty, we know there are no CI
runs.

### Fetch state stays on `App` via `CiFetchTracker`

Replace the `CiState` enum with a newtype-wrapped set:

```rust
pub struct CiFetchTracker {
    inner: HashSet<AbsolutePath>,
}

impl CiFetchTracker {
    pub fn start(&mut self, path: AbsolutePath) { self.inner.insert(path); }
    pub fn complete(&mut self, path: &AbsolutePath) -> bool { self.inner.remove(path) }
    pub fn is_fetching(&self, path: &AbsolutePath) -> bool { self.inner.contains(path) }
    pub fn clear(&mut self) { self.inner.clear(); }
}
```

Named operations instead of raw `HashSet` manipulation. Makes forgotten removals
easy to audit — `complete()` is the only removal path.

### `ci_display_modes` stays on `App`

`ci_display_modes: HashMap<AbsolutePath, CiRunDisplayMode>` is a UI preference
(branch-only vs all runs), not project data. Same category as `ci_pane` (scroll
position). No migration needed.

### Simplify branch filtering

Current behavior: the branch/all toggle only appears when the current branch
differs from the default branch. New behavior:

- **Always filter by current branch by default**, regardless of whether it's the
  default branch.
- **Toggle is always available** if the project has a known branch.
- **Empty state**: when filtering by current branch yields zero runs, show
  "no CI runs for branch X" rather than silently falling back to all runs.
  The user can toggle to "all" to see other branches.

This eliminates the `default_branch` comparison in `branch_only_ci_filter` —
the function just returns the current branch (if known).

### Skip unpublishable crates (no change)

Already handled in the previous phase. No CI equivalent needed.

## What stays on `App` (not migrating)

These are runtime/communication/UI state:

- `ci_fetch_tracker: CiFetchTracker` (new, replaces `CiState`)
- `ci_display_modes: HashMap<AbsolutePath, CiRunDisplayMode>`
- `pending_ci_fetch: Option<PendingCiFetch>`
- `ci_fetch_tx` / `ci_fetch_rx`
- `ci_fetch_toast: Option<ToastTaskId>`
- `ci_pane: Pane`
- `repo_fetch_cache: RepoCache` (keyed by owner/repo, not path)

## Step 1: Use the existing hierarchy accessor

Use `ProjectList::at_path_mut()` for CI hierarchy writes. If a semantic alias
such as `project_info_at_path_mut()` becomes useful later, it can be added as a
thin wrapper, but no new accessor is required for this phase.

## Step 2: Add `ProjectCiData`/`ProjectCiInfo` to `ProjectInfo`

### New types (`src/project/info.rs` or its own file)

```rust
pub(crate) struct ProjectCiInfo {
    pub runs:         Vec<CiRun>,
    pub github_total: u32,
    pub exhausted:    bool,
}

pub(crate) enum ProjectCiData {
    Unfetched,
    Loaded(ProjectCiInfo),
}
```

### `ProjectInfo`

```rust
pub(crate) struct ProjectInfo {
    pub disk_usage_bytes: Option<u64>,
    pub local_git_state:  LocalGitState,
    pub github_info:      Option<GitHubInfo>,
    pub ci_data:          ProjectCiData,
    pub visibility:       Visibility,
    pub worktree_health:  WorktreeHealth,
    pub submodules:       Vec<SubmoduleInfo>,
}
```

Default should remain `Unfetched`.

## Step 3: Add `CiFetchTracker` and replace `CiState`

- Add `CiFetchTracker` newtype (wrapping `HashSet<AbsolutePath>`)
- Remove `ci_state: HashMap<AbsolutePath, CiState>` from `App`
- Add `ci_fetch_tracker: CiFetchTracker` to `App`
- Delete the `CiState` enum from `types.rs`
- Update `is_fetching` checks to use `ci_fetch_tracker.is_fetching(path)`

## Step 4: Update write sites

### Initial scan (`insert_ci_runs`)

`ci_state.rs:87` currently inserts `CiState::Loaded` into the HashMap. Replace
with: look up the project via `ProjectList::at_path_mut()`, set
`ci_data = ProjectCiData::Loaded(ProjectCiInfo { runs, github_total, exhausted })`.

If the path is inactive, reset hierarchy CI data back to `ProjectCiData::Unfetched`.

### Fetch start (`terminal.rs`)

Currently sets `CiState::Fetching`. Replace with:
`self.ci_fetch_tracker.start(path)`. The existing runs on the hierarchy node are
untouched — they remain readable during the fetch.

### Fetch complete (`handle_ci_fetch_complete`)

`ci_state.rs:112` currently merges runs and sets `CiState::Loaded`. Replace
with: look up the hierarchy node via `ProjectList::at_path_mut()`, merge runs
into `ci_data`, update `exhausted` and `github_total`, then call
`self.ci_fetch_tracker.complete(path)`.

**Guard against rescan race**: if the hierarchy node no longer exists (rescan
rebuilt the tree while a fetch was in flight), skip the write silently. The
channels are not rebuilt on rescan, so in-flight fetches can still deliver
results to stale paths. Add a comment explaining the discard.

**Multi-owner paths**: a single GitHub repo can map to multiple hierarchy paths
(worktree entries). Loop over all owner paths and update each via
`ProjectList::at_path_mut()`. Skip any path that no longer exists. Document
that all owner paths for a repo should be updated together.

### Clear CI cache (`detail/interaction.rs`)

The current clear-cache path resets `CiState::Loaded` to an empty run list while
preserving `github_total`. Replace that with hierarchy writes through
`ProjectList::at_path_mut()`, setting each owner path to
`ProjectCiData::Loaded(ProjectCiInfo { runs: Vec::new(), exhausted: false, github_total: prev_total })`.

### Prune inactive project state (`query.rs`)

The current inactive-path cleanup removes CI entries from the `HashMap`. Keep
the cleanup behavior, but reset hierarchy-owned CI data back to
`ProjectCiData::Unfetched` for inactive paths instead of deleting anything.

### Rescan

The `ci_state.clear()` call goes away. Rescan rebuilds hierarchy from scratch
(fresh nodes with `ProjectCiData::Unfetched`). Clear `ci_fetch_tracker` since
in-flight requests target stale paths.

## Step 5: Update read sites

### `ci_state_for(path)` (`query.rs`)

Replace with `ci_data_for(path)` that reads CI data from the hierarchy node.
Fetching status comes from `ci_fetch_tracker.is_fetching(path)` separately —
callers that previously checked `CiState::is_fetching()` must be updated to use
the tracker instead.

### `ci_runs_for_display_inner` (`ci_state.rs`)

Read runs from hierarchy `ProjectCiData` instead of `CiState`. Filtering logic
is updated per Step 6.

### `latest_ci_run_for_path` (`ci_state.rs`)

Same — read from hierarchy.

### CI pane data + rendering (`detail/model.rs`, `detail/interaction.rs`, `panes/ci.rs`)

Update pane data construction, CI interaction, and the pane renderer to read CI
runs/totals from the hierarchy. Fetch spinner/status reads `ci_fetch_tracker`.

### Main view (`query.rs`, `render.rs`, pane callers)

Update `selected_ci_state()` callers to use hierarchy-backed CI data plus the
fetch tracker.

## Step 6: Simplify branch filtering

### `branch_only_ci_filter` (`ci_state.rs`)

Currently:

```rust
fn branch_only_ci_filter(&self, path: &Path) -> Option<&str> {
    let git = self.git_info_for(path)?;
    let branch = git.branch.as_deref()?;
    let default_branch = git.default_branch.as_deref()?;
    (branch != default_branch).then_some(branch)
}
```

Replace with:

```rust
fn current_branch_for(&self, path: &Path) -> Option<&str> {
    self.git_info_for(path)?.branch.as_deref()
}
```

No `default_branch` comparison. Always filter by current branch.

### `ci_toggle_available_for_inner`

Toggle is available whenever we know the current branch:

```rust
pub(super) fn ci_toggle_available_for_inner(&self, path: &Path) -> bool {
    self.current_branch_for(path).is_some()
}
```

### Empty state when no runs match current branch

When filtering by current branch yields zero results, show "no CI runs for
branch X" instead of silently falling back to all runs. The user can toggle
to "all" if they want to see other branches.

### Fetch-complete toast delta logic (`async_tasks.rs`)

The async toast completion path currently computes `before`/`after` run counts
from `ci_state`. Replace those reads with hierarchy-backed CI counts so the
"N new runs fetched" toast remains accurate after the migration.

## Step 7: Update tests

Tests in `src/tui/app/tests/state.rs` directly assert against `app.ci_state`
(the HashMap). Update them to read CI data via `ProjectList::at_path()` and
check fetch status via `ci_fetch_tracker`. Rewrite assertions to use hierarchy
accessors.

## Step 8: Remove dead code

- Delete `CiState` from `types.rs`
- Delete `ci_state`, `ci_state_mut()`, and any `selected_ci_state()`-style
  compatibility helpers that only exist for the old model
- Remove old initializer state from `construct.rs`
- Remove `HashMap`-based CI cleanup paths after they are replaced with hierarchy
  resets to `ProjectCiData::Unfetched`

## Step 9: Verify

- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo +nightly fmt --all`
- `cargo nextest run`
- `cargo install --path .`
