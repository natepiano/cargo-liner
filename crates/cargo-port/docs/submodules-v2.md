# Submodule pane: give submodules their own `GitRepo`

## Problem

The submodule pane silently renders the **parent repo's** metadata for every repo-level field. `build_pane_data_for_submodule` (`src/tui/panes/pane_data/mod.rs:1200`) calls `build_git_detail_fields(app, submodule.path)`, which does two lookups:

- `app.project_list.git_info_for(abs_path)` — returns the submodule's own `CheckoutInfo` (branch, status, last commit). Correct.
- `app.project_list.entry_containing(abs_path)` — returns the **parent** `ProjectEntry`, because submodules are nested inside a parent's working tree and aren't top-level entries. So `Stars`, `Incept`, `About`, `Fetched`, `local_main_branch`, and the entire `Remotes` table all come from the wrong repo.

A second, smaller bug: `CheckoutInfo.branch` is `Option<String>` filled from `git rev-parse --abbrev-ref HEAD`, which returns the literal string `"HEAD"` when detached. A detached submodule renders `Branch HEAD` and titles `Git - HEAD`. The same issue affects any detached checkout, not just submodules.

A third gap: `.gitmodules` records a tracking branch, and the parent repo records a pinned commit (`git ls-tree HEAD`). Both are already parsed into `Submodule { branch, commit }` (`src/project/git/submodule.rs:28–30`) but never surfaced.

## Core observation

A submodule **is a separate git repo** sitting on disk. The existing `GitRepo` / `RepoInfo` / `CheckoutInfo` model already describes everything a repo needs. The submodule isn't getting that treatment because it isn't a top-level `ProjectEntry`. Attach a `GitRepo` directly to `Submodule` and the existing detection pipeline does the work.

The submodule **also** has two parent-side facts that no normal repo has — the `.gitmodules` tracking branch and the parent-recorded pinned commit — but those already live on `Submodule` itself. They just need new render fields.

## Target data model

```rust
struct Submodule {
    name:          String,
    path:          AbsolutePath,
    relative_path: String,
    url:           Option<String>,
    branch:        Option<String>,   // .gitmodules tracking branch (existing)
    commit:        Option<String>,   // parent-pinned SHA (existing)
    info:          ProjectInfo,      // existing
    git_repo:      Option<GitRepo>,  // NEW — same model as ProjectEntry
}

enum HeadState {
    Unborn,
    Branch(String),
    Detached { short_sha: String },
}

struct CheckoutInfo {
    status:              GitStatus,
    head:                HeadState,           // was: branch: Option<String>
    last_commit:         Option<String>,
    ahead_behind_local:  Option<(usize, usize)>,
    primary_tracked_ref: Option<String>,
}

struct RemoteInfo {
    // existing fields...
    push: PushState,                          // NEW
}
enum PushState {
    Enabled { push_url: String },             // resolved push URL (== fetch URL if no override)
    Disabled { reason: PushDisabledReason },
}
enum PushDisabledReason {
    KnownSentinel(KnownSentinel),  // pushurl == well-known disable sentinel
    OtherSentinel(String),         // pushurl is set but not in the known list
    NoPushUrl,                     // pushurl is empty where required
}
enum KnownSentinel { Disabled, NoPush, DoNotPush }
```

`HeadState` and `PushState`/`PushDisabledReason`/`KnownSentinel` derive `Serialize`/`Deserialize` so `CheckoutInfo` and `RemoteInfo` keep round-tripping through the existing serde flow.

Stage 1 also reworks `GitData` to use `HeadState` directly (removing the duplication between `data.branch` and `data.head`):

```rust
struct GitData {
    // existing fields, but:
    head: Option<HeadState>,        // was: branch: Option<String>
    // ...new submodule-only fields below...
}
```

Two new optional `GitData` fields populated only by the submodule pane builder:

```rust
struct GitData {
    // ...
    pinned_commit: Option<String>,  // from Submodule.commit
    tracks:        Option<String>,  // from Submodule.branch (.gitmodules)
}
```

Parent-repo identity ("Submodule of …") is rendered as a **second line in the existing About section** rather than as a flat field. That keeps the flat field list short and matches the mockup. Implementation: extend `GitAboutCtx` (`src/tui/panes/git.rs:715`) with `parent_repo: Option<&'a str>` and emit a second line after `description` when present. No new `DetailField` variant is needed for the parent.

Two new `DetailField` variants: `Pinned` and `Tracks`. `git_fields_from_data` pushes them when `Some(_)`, in that order, between `GitPath` and `VsLocal`.

### Why these two rather than an `is_submodule` flag

A boolean `is_submodule` would push branching into every renderer. Optional fields keep render logic uniform: a field is present and rendered, or absent and skipped — same pattern `Stars`, `Inception`, `LastFetched` already use.

## Sanity check: per-repo vs per-checkout vs per-parent-relationship

| Field                | Kind                       | Home                |
| -------------------- | -------------------------- | ------------------- |
| `head`               | per-checkout               | `CheckoutInfo`      |
| `status`             | per-checkout               | `CheckoutInfo`      |
| `last_commit`        | per-checkout               | `CheckoutInfo`      |
| `ahead_behind_local` | per-checkout               | `CheckoutInfo`      |
| `primary_tracked_ref`| per-checkout               | `CheckoutInfo`      |
| `remotes`            | per-repo                   | `RepoInfo`          |
| `first_commit`       | per-repo                   | `RepoInfo`          |
| `last_fetched`       | per-repo                   | `RepoInfo`          |
| `default_branch`     | per-repo                   | `RepoInfo`          |
| `local_main_branch`  | per-repo                   | `RepoInfo`          |
| `github_info`        | per-repo                   | `GitRepo`           |
| `pinned_commit`      | per-parent-relationship    | `Submodule.commit`  |
| `tracks`             | per-parent-relationship    | `Submodule.branch`  |
| `parent_repo`        | per-parent-relationship    | derived at render   |
| `push` (per remote)  | per-repo                   | `RemoteInfo`        |

The submodule has all three categories; a non-submodule project has only the first two.

## Rendering — before vs after

Concrete example: parent repo `gltf-ibl-sampler-egui` on `master`; submodule `glTF-IBL-Sampler` detached at `26847464`, `.gitmodules` says `branch = lite`, remote configured with `pushurl = DISABLED`.

**Today:**

```
┌ Git - HEAD ─────────────────────────────────────────┐
│ <parent repo's GitHub description>             ⚠ bug│
├─────────────────────────────────────────────────────┤
│  Branch              HEAD                      ⚠ bug│
│  Git Path            modified                       │
│  vs local main       3↑ 0↓                     ⚠ bug│
│  Stars               142                       ⚠ bug│
│  Incept              2019-04-15                ⚠ bug│
│  Latest              2024-12-01                     │
│  Fetched             1h ago                    ⚠ bug│
│  Rate limit core     4912/5000                      │
│  Rate limit GraphQL  4870/5000                      │
│ ─ Remotes ───                                  ⚠ bug│
│   ⇉ origin   parent-org/gltf-ibl-sampler-egui       │
│              origin/master                  ✓ in sync│
└─────────────────────────────────────────────────────┘
   ⚠ = parent repo's data leaking into the submodule pane
```

**Proposed:**

```
┌ Git - detached @ 26847464 ──────────────────────────┐
│ Sample implementation of glTF IBL filtering.        │
│ Submodule of ~/rust/gltf-ibl-sampler-egui           │
├─────────────────────────────────────────────────────┤
│  Head                detached @ 26847464            │
│  Tracks              lite  (from .gitmodules)       │
│  Pinned              26847464  (parent HEAD)        │
│  Git Path            modified                       │
│  Latest              2024-12-01                     │
│  Stars               18                             │
│  Incept              2017-08-22                     │
│  Fetched             1h ago                         │
│  Rate limit core     4912/5000                      │
│  Rate limit GraphQL  4870/5000                      │
│ ─ Remotes ───                                       │
│   ⇉ origin   pcwalton/glTF-IBL-Sampler              │
│              origin/lite        ↛ push disabled     │
└─────────────────────────────────────────────────────┘
```

When the submodule is later switched onto a local branch `mac-vkhelper`:

```
┌ Git - mac-vkhelper ─────────────────────────────────┐
│ ...                                                 │
│  Head                mac-vkhelper                   │
│  Tracks              lite  (from .gitmodules)       │
│  Pinned              26847464  (parent HEAD)        │
│  Git Path            modified                       │
│  ...                                                │
└─────────────────────────────────────────────────────┘
```

`DetailField::Branch` is renamed to `DetailField::Head` (label `"Head"`) so the label matches the typed value. The git-pane title routes through `HeadState`:

- `HeadState::Branch(name)` → `Git - <name>`
- `HeadState::Detached { short_sha }` → `Git - detached @ <short_sha>`
- `HeadState::Unborn` → `Git`

## Staging

Each stage compiles and passes tests on its own. Four stages.

### Stage 1 — Introduce `HeadState` (replaces `CheckoutInfo.branch: Option<String>`)

This stage stands alone — it's the smallest cross-cutting type change and unblocks both the title fix and the submodule pane work.

- Add the `HeadState` enum to `src/project/git/checkout.rs`, deriving `Debug, Clone, Serialize, Deserialize`.
- Change `CheckoutInfo.branch: Option<String>` → `CheckoutInfo.head: HeadState`.
- `CheckoutInfo::get` resolves `HEAD` in two steps. Run `rev-parse --abbrev-ref HEAD`:
  - Empty output → `HeadState::Unborn` (no commits yet — git emits nothing).
  - `"HEAD"` → run `rev-parse --short=8 HEAD`; if that succeeds emit `HeadState::Detached { short_sha }`. If that *also* fails (corrupt repo), emit `HeadState::Unborn`.
  - Any other string → `HeadState::Branch(name)`.
  Always use `--short=8` (not `--short`) so the SHA length matches `ls_tree_submodule_commits` (`submodule.rs:161`).
- Update every reader. Call sites: `git_panel_title` (`src/tui/panes/git.rs:630`), `build_git_detail_fields` (`pane_data/mod.rs:1008`), `worktrees_from_item` (`pane_data/mod.rs:1115`), tests in `src/tui/app/tests/*`. Also: `GitData.branch: Option<String>` becomes `GitData.head: Option<HeadState>` (was duplicated alongside `CheckoutInfo.head`) — update `build_git_detail_fields` to copy the typed value through unchanged. Add a `HeadState::branch_name(&self) -> Option<&str>` helper for the `local_main_branch` comparison case.
- `DetailField::Branch` is renamed to `DetailField::Head` — three edits: enum variant (`pane_data/mod.rs:273`), label string `"Branch"` → `"Head"` (`pane_data/mod.rs:304`), and the `git_value` match arm (`pane_data/mod.rs:375`). The new `git_value` for `Head` matches on `HeadState`: `Branch(name)` → `name` (with `(HEAD)` suffix when default), `Detached { short_sha }` → `"detached @ {short_sha}"`, `Unborn` → `"unborn"`.
- `git_panel_title` (`src/tui/panes/git.rs:630`) matches on `data.head` directly: `Branch(name)` → `" Git - <name> "`, `Detached { short_sha }` → `" Git - detached @ <short_sha> "`, `Unborn` or `None` → plain `Git`.
- Add `CheckoutInfo::for_tests()` (none exists today; needed by all subsequent stages).
- Tests: detached-HEAD title and `Head` field rendering; branch-head title rendering; unborn-repo `Unborn` emission; `HeadState::branch_name()` correctness; serde round-trip of all three variants.

**Expected LOC:** ~150–200. **Files:** ~10. **User-visible:** detached HEADs now render `detached @ <sha>` everywhere (panes + title) instead of the literal string `"HEAD"`.

### Stage 2 — Attach `git_repo: Option<GitRepo>` to `Submodule`

Now the structural fix: submodules carry their own repo metadata.

- Add `git_repo: Option<GitRepo>` to `Submodule` (`src/project/git/submodule.rs:18`). Initialize `None` at parse time; populated by detection like any other entry. Skip population when the submodule is uninitialized — see "uninitialized submodules" below.
- Plumb the submodule detection path. `get_submodules` already runs at scan time; extend it to fire `RepoInfo::get` against each submodule's working dir, storing the result in `submodule.git_repo`. Submodules suppress GitHub fetches today via exactly one guard: `if self.project_list.is_submodule_path(path) { return; }` in `repo_handlers.rs:268` (`maybe_trigger_repo_fetch`). Stage 2 removes that guard. The dedup set `App.net.repo_fetch_in_flight: HashSet<OwnerRepo>` (`src/tui/state/net.rs:133`) keys on `OwnerRepo`, so a submodule sharing an `OwnerRepo` with its parent won't trigger a duplicate fetch. CI fetch suppression for submodules stays in place per `git_repo.md` Stage 1 — Stage 2 only reverses the *GitHub metadata* suppression.
- Refresh-pipeline cost: each submodule adds one `RepoInfo::get` invocation per scan (multiple `git` shell-outs). Two mitigations available if measurements warrant: (a) route submodule `RepoInfo::get` through a `BackgroundMsg` like the existing entry detection, (b) batch `git config` reads at the `RepoInfo::get` call site (see Stage 4).
- Uninitialized submodules: if the submodule's working directory is missing or doesn't contain `.git`, skip `RepoInfo::get` and leave `submodule.git_repo = None`. Add `Submodule::is_initialized(&self) -> bool`. Pane rendering then shows `Tracks` + `Pinned` (parent-side facts that survive deinit) but no repo-level rows.
- Add a submodule-aware lookup helper on `ProjectList`. The actual store is `roots: IndexMap<AbsolutePath, ProjectEntry>` (`project_list/mod.rs:82`), not a `Vec`:
  ```rust
  pub(super) fn git_repo_for(&self, abs_path: &Path) -> Option<&GitRepo> {
      // 1. Direct entry hit: top-level project at abs_path.
      // 2. Submodule hit: walk roots.values(), check each entry's submodules.
      //    Use a longest-prefix-match so nested cases pick the most specific repo.
      // 3. Containing-entry hit: fallback to the entry that contains abs_path.
  }
  ```
  Internally use `ProjectEntry::find_submodule(&Path) -> Option<&Submodule>` so the walking lives on the entry, not the list. Order matters — direct/submodule before containing-entry, so a submodule never falls through to the parent.
- `build_git_detail_fields` keeps `app: &App, abs_path: &Path` (callers still need `app.net.rate_limit()`, `app.config`, and `git_status_for(abs_path)`), and replaces the inline `app.project_list.entry_containing(abs_path).and_then(...).git_repo` lookup with a single `app.project_list.git_repo_for(abs_path)` call. `build_pane_data_for_submodule` calls the same function — the helper hands back the submodule's own repo, not the parent's.
- Add test constructors that don't yet exist: `Submodule::for_tests()`, `RemoteInfo::for_tests()`. (`CheckoutInfo::for_tests()` already added in Stage 1.) `ProjectInfo::for_tests()` and `GitRepo::for_tests()` exist per `git_repo.md` Stage 0.

**Expected LOC:** ~300–400. **Files:** ~12–15. **User-visible:** the `⚠ bug` rows in the rendering diagram are fixed — Stars / Incept / About / Fetched / Remotes / `vs local main` now describe the submodule, not the parent.

### Stage 3 — Add submodule render fields (`Tracks`, `Pinned`, parent About line)

Pure render addition; no model changes beyond `GitData` and `DetailField`.

- Add fields to `GitData`: `tracks: Option<String>`, `pinned_commit: Option<String>`, `parent_repo: Option<String>`. The `parent_repo` field feeds the About section, not a `DetailField`.
- Add `DetailField::Tracks` and `DetailField::Pinned` with labels `"Tracks"` and `"Pinned"`.
- `git_fields_from_data` pushes them when `Some(_)`, in that order, between `GitPath` and `VsLocal`.
- `git_value` formats:
  - `Tracks` → `"<branch>  (from .gitmodules)"`.
  - `Pinned` → `"<short_sha>  (parent HEAD)"`.
- About-section rendering: extend `GitAboutCtx` (`src/tui/panes/git.rs:715`) with `parent_repo: Option<&'a str>`. When present, emit a second line `"Submodule of <parent_repo>"` after the description.
- `build_pane_data_for_submodule` populates these. Parent path is `app.project_list.entry_containing(submodule.path).map(|e| home_relative_path(e.item.path()))`. Other pane builders leave the three fields `None`.

**Expected LOC:** ~150–200. **Files:** ~4–6.

### Stage 4 — Push-state detection on `RemoteInfo`

General-purpose; applies to every repo. Title commits and PRs accordingly ("add push-state detection on RemoteInfo") rather than framing as submodule-specific.

- Add `push: PushState` to `RemoteInfo` (`src/project/git/repo.rs`). `PushState::Enabled { push_url }` carries the resolved URL (which is the fetch URL when there's no override) so the renderer doesn't re-derive it.
- Detection in `repo.rs` batches the per-remote `git config` reads: run `git config --get-regexp '^remote\\..*\\.pushurl$'` once per `RepoInfo::get`, parse all pushurl values into a `HashMap<RemoteName, String>`, then populate `push` for each remote in the existing per-remote loop. Rules:
  - No entry in the map → `Enabled { push_url: fetch_url.clone() }`.
  - Empty value → `Disabled { reason: NoPushUrl }`.
  - Value matches a known sentinel (case-insensitive: `DISABLED`, `no-push`, `do_not_push`) → `Disabled { reason: KnownSentinel(...) }`.
  - Any other value → `Enabled { push_url: value }`. Anything that looks intentionally non-routable is *not* heuristically demoted to disabled in Stage 4 — explicit sentinels only.
- Extend `RemoteRow` with a pre-formatted `push_annotation: Option<String>` so the renderer doesn't branch on `PushState` directly.
- Render in `panes/git.rs` after the `status` column: `↛ push disabled` for `KnownSentinel`/`NoPushUrl`. When room permits, append the sentinel name (e.g. `↛ push disabled (DISABLED)`).
- Out of Stage 4 (follow-up): heuristic detection of non-routable hosts (e.g. `nowhere.invalid`, loopback addresses). Add only if real users hit cases the sentinel list misses.

**Expected LOC:** ~100–150. **Files:** ~4.

## Risks and what to watch

### `HeadState` migration churn

Stage 1 touches every reader of `CheckoutInfo.branch` and `GitData.branch`. Grep finds ~12 sites. None are hot — all are render-time. The `branch_name()` helper covers the "use as string for comparison" pattern.

### Pinned-vs-HEAD divergence

`Submodule.commit` is 8 chars from `ls_tree_submodule_commits` (`submodule.rs:161`); `HeadState::Detached.short_sha` is 8 chars from `rev-parse --short=8 HEAD`. When the submodule is detached *and* the pinned commit matches the checkout, both SHAs are equal — the pane shows the same string in `Head` and `Pinned`. When they diverge (the working tree is on a different commit than the parent pins, which is the dirty-submodule case), the two SHAs disagree and the `Git Path: modified` row is the user's signal. The pane doesn't try to highlight the mismatch further in this plan; flag as a follow-up if users miss it.

### Submodule detection cost

Each submodule adds one `RepoInfo::get` invocation per scan (several git shell-outs). In a workspace with many submodules this can measurably slow scan. Mitigations: (a) route submodule `RepoInfo::get` through a `BackgroundMsg` like entry detection, (b) Stage 4's batched `git config` already amortizes the per-remote cost. Measure before deferring.

### Stale `Submodule.git_repo` after `git submodule deinit`

`get_submodules` re-runs on each scan; deinitialized submodules return `commit = None` and `is_initialized() == false`, so `RepoInfo::get` is skipped. Existing `Submodule.git_repo` (from a prior scan) is discarded with the rest of `Submodule` when `get_submodules` rebuilds the list. No persistent stale-pointer hazard.

### Submodule URL edge cases

Tests today cover the happy path. Add coverage for: `.gitmodules` present but working tree not initialized; submodule listed in `.gitmodules` but path missing from `ls-tree HEAD`; relative URLs (`url = ../other-repo`). Relative URLs are out of Stage 2 — `RepoInfo::get` against the working dir still works because it uses the actual remote config; only the GitHub-URL inference may not resolve.

### `ls_tree HEAD` silent failure

`ls_tree_submodule_commits` (`submodule.rs:140`) returns an empty map on failure with no log. A corrupt parent repo would silently render `Pinned: -`. Add a `tracing::warn!` on `output.status.success() == false` so corrupt repos show up in logs.

### Symlink / overlapping path hazards

`entry_containing` and the new `git_repo_for` use a containment check on paths. Two entries with overlapping hierarchies (symlinks, bind mounts) could resolve the wrong way. The proposal documents this as an existing assumption — non-overlapping `ProjectList` roots — and uses longest-prefix-match inside `git_repo_for` to reduce the blast radius for the submodule case.

### Tests

Constructors needed: `Submodule::for_tests()`, `CheckoutInfo::for_tests()`, `RemoteInfo::for_tests()`. Test cases:

- Detached submodule renders `detached @ <sha>` in title and `Head` field.
- Submodule pane's `Stars` / `About` / `Remotes` describe the submodule's repo, not the parent.
- A remote with `pushurl = DISABLED` renders the `↛ push disabled` annotation.
- A submodule with `branch = lite` in `.gitmodules` renders the `Tracks` row.
- A submodule deinit case: `is_initialized() == false`, no repo-level rows; `Tracks` + `Pinned` still render.
- Unborn repo: `HeadState::Unborn` correctly emitted.
- `HeadState` and `PushState` serde round-trip.

## Non-goals

- Editing submodules from the pane (init, update, sync). The pane stays read-only.
- Showing per-submodule CI. Submodules don't get a `ProjectCiData` slot per `git_repo.md`'s Stage 1 — keep that.
- Cross-repo deduplication of GitHub fetches between a submodule and a parent that point at the same `OwnerRepo`. The existing `repo_fetch_in_flight: HashSet<OwnerRepo>` handles this automatically.
- Distinguishing nested-submodule scenarios (submodules inside submodules). The proposal handles one level; deeper nesting can extend the same recursion.

## Success criteria

- Submodule pane title renders `Git - <branch>` or `Git - detached @ <sha>` — never the literal `Git - HEAD`.
- Submodule pane's `Stars`, `Incept`, `About`, `Fetched`, `Remotes`, `vs local main` describe the submodule's own repo, not the parent's.
- Submodule pane renders `Tracks` and `Pinned` rows when `.gitmodules` / parent HEAD provide values, and a "Submodule of …" line in the About section.
- A remote configured with `pushurl = DISABLED` renders an explicit push-disabled annotation in the Remotes table.
- `CheckoutInfo.branch: Option<String>` no longer exists; `CheckoutInfo.head: HeadState` replaces it. `GitData.branch` likewise becomes `GitData.head: Option<HeadState>`.
- `Submodule.git_repo: Option<GitRepo>` exists and is populated by detection for initialized submodules.

## Decisions log

Recorded from /adhoc_review of team-review findings (2026-05-18).

### 1. Submodule placement — **attached (current plan)**

Keep `git_repo: Option<GitRepo>` on the nested `Submodule`. Do **not** promote to `RootItem::Submodule`. A submodule is a child of its parent project, not a sibling — promoting would force a child relationship into a sibling slot and require every `RootItem` iterator to filter submodules out. The uniformity win is cosmetic; the semantic mismatch is real.

### 2. `GitData` submodule fields — **sum type**

Replace the three independent `Option<String>` fields (`tracks`, `pinned_commit`, `parent_repo`) with a single `Option<SubmoduleContext>`:

```rust
pub struct SubmoduleContext {
    pub tracks:        Option<String>,    // from .gitmodules `branch =`
    pub pinned_commit: String,            // from parent ls-tree HEAD; always present when populated
    pub parent_repo:   String,            // display path of parent
}

pub struct GitData {
    // ...
    pub submodule_ctx: Option<SubmoduleContext>,
}
```

`git_fields_from_data` does one `if let Some(ctx) = &data.submodule_ctx` and pushes `Tracks` / `Pinned` together. About-section rendering reads `ctx.parent_repo`. Diverges from the local "independent options" pattern, but it spells out "this whole group of fields is the submodule overlay" — which is the real intent.

### 3. `CommitId` / `BranchName` newtypes — **skip (keep bare `String`s)**

No `CommitId` or `BranchName` newtypes. The git module is small; every producer of these values is a known `git` shell-out, so the invariant violations the newtypes would prevent aren't real risks here. Newtypes would touch every git struct, every test fixture, and every render call for a small safety win. Revisit if the git module grows substantially.

### 4. `GitRepoProvider` trait — **skip (keep the `git_repo_for` helper)**

No `GitRepoProvider` trait. `build_git_detail_fields` still needs `&App` for rate limits, config, and `git_status_for(path)` — the trait would only wrap the `git_repo` accessor. With only two call sites (regular entry path and submodule path), both already knowing which `GitRepo` they want, the trait's compile-time enforcement protects against an error nobody is making. Path-based lookup matches `entry_containing` and the rest of the project-list API. Add the trait later if a third caller appears and the pattern starts to drift.

## Summary of staging

| Stage | What it does                                                  | User-visible                                  |
| ----- | ------------------------------------------------------------- | --------------------------------------------- |
| 1     | Introduce `HeadState`, replace `CheckoutInfo.branch`          | detached HEADs render correctly everywhere    |
| 2     | Attach `git_repo` to `Submodule`; reverse fetch suppression   | submodule pane shows submodule's repo data    |
| 3     | Add `Tracks` / `Pinned` / `Parent` to `GitData`               | parent-pin and tracking branch render         |
| 4     | `PushState` on `RemoteInfo`; render push-disabled annotation  | push-protected remotes flagged in any repo    |
