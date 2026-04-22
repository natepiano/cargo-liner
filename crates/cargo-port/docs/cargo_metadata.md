# Cargo Metadata Integration Plan

## Problem To Fix

Cargo-port today reconstructs Cargo project state by hand-parsing `Cargo.toml`, walking `examples/` and `benches/` on disk, and matching workspace-member globs with a bespoke pattern matcher. This has three concrete costs:

1. **Incorrect target-dir assumptions.** Cargo resolves the target directory via (in precedence order) `--target-dir`, `CARGO_TARGET_DIR`, `build.target-dir` in `.cargo/config.toml` (project tree → `~/.cargo/config.toml`), then the default `<project_root>/target/`. Worktrees commonly share a target dir to avoid recompiling the dep graph N times. Cargo-port does not resolve any of this.
   - `src/tui/app/query.rs:181` — `start_clean()` checks `project_path.as_path().join("target").exists()`. If a project redirects its target dir, the "Already clean" toast fires incorrectly.
   - `src/watcher.rs:542` — the file watcher decides whether an event is a `target/` event using `starts_with(entry.abs_path.join("target"))`; redirected target dirs fall through and trigger spurious re-lints.
   - `src/scan.rs:979, src/scan.rs:1262, src/lint/trigger.rs:63` — all hardcode the string `"target"` to filter walks. Fine for default layouts, wrong when the user redirects.
   - `src/enrichment.rs:43, src/scan.rs:1061, src/scan.rs:1485, src/watcher.rs:1039, src/tui/terminal.rs:586` — disk-usage callers walk the project tree with `scan::dir_size`. Out-of-tree target directories are silently missed from the reported size; in-tree target directories are included without attribution.
   - `src/tui/terminal.rs:486` — `cargo clean` shell-out inherits the right dir via cargo itself, but the app has no idea *which* directory was cleaned or who else shared it.
2. **Duplicated, drifting Cargo.toml parsing.** `src/project/cargo.rs:71-157` hand-parses `package.name`, `package.version` (including workspace inheritance at `:86-100`), `package.description`, `package.publish` (including the `publish = ["registry"]` allowlist form at `:111-115`), `[lib]`, `[[bin]]`, `[[example]]`, `[[bench]]`, `[[test]]`. `src/scan.rs:566-627` hand-parses `[workspace].members` / `[workspace].exclude`. `src/scan.rs:630-638` (`normalize_workspace_path`) and `src/scan.rs:640-682` (`workspace_pattern_matches*`) implement a hand-rolled glob engine. Edition, license, homepage, and repository are never parsed even though the UI could use them.
3. **Filesystem target discovery disagrees with cargo.** `src/project/cargo.rs:205-245, :272-319, :323-370, :372-388` discover examples, benches, and tests by walking directories when the manifest is silent. Cargo's actual discovery rules (name collisions, excluded targets, `autoexamples = false`, named vs. multi-file examples) are subtly different; the pane can show targets cargo refuses to build and vice-versa.

The `cargo_metadata` crate shells out to `cargo metadata` and deserializes the result into typed structs. It resolves `target_directory` for us, enumerates workspace members authoritatively, and returns the true set of build targets with `kind` (`bin`, `example`, `test`, `bench`, `lib`, `proc-macro`, `cdylib`, `rlib`, …) and `src_path`.

## Goals

- **Phase 1:** adopt `cargo_metadata` as the single source of truth for Cargo project structure and resolved target directory. Retire hand-rolled parsers *where safe*. No user-visible behavior changes except "more accurate."
- **Phase 2:** build a correct clean action on top of the Phase 1 data: per-worktree clean with share-detection, and a deduped fan-out group clean.

## Non-Goals

- Replacing lint, CI, crates.io enrichment, or GitHub integrations. Orthogonal.
- Walking the dependency graph or reading `Cargo.lock`. `--no-deps` keeps us fast.
- Replacing worktree detection (`src/project/git.rs:906-948`). Orthogonal.

## Phase 0 — Refactor `StartupPhaseTracker` Before Adding The Metadata Phase

The current `StartupPhaseTracker` at `src/tui/app/types.rs:73-94` has per-phase named fields (`disk_expected`, `disk_seen`, `disk_complete_at`, `disk_toast`, then the same shape repeated for `git_*`, `repo_*`, `lint_*`). Adding `cargo_metadata` as a fifth phase by mirroring that pattern would duplicate the completion and toast logic again. Phase 0 collapses the duplication first so Phase 1's metadata phase slots into shared code.

### Shape

```rust
#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
pub enum PhaseKind {
    Disk,
    Git,
    Repo,
    Lint,
    Metadata,       // added in Phase 1; declared here for the refactor target
}

pub enum PhaseState<K> {
    Keyed(KeyedPhase<K>),
    Counted(CountedPhase),
}

pub struct KeyedPhase<K> {
    pub expected:    Option<HashSet<K>>, // None = Unknown (not-yet-initialized)
    pub seen:        HashSet<K>,
    pub complete_at: Option<Instant>,
    pub toast:       Option<ToastTaskId>,
}

pub struct CountedPhase {
    pub expected:    Option<usize>,      // None = Unknown
    pub seen:        usize,
    pub complete_at: Option<Instant>,
    pub toast:       Option<ToastTaskId>,
}

pub struct StartupPhaseTracker {
    pub scan_complete_at:    Option<Instant>,
    pub startup_toast:       Option<ToastTaskId>,
    pub startup_complete_at: Option<Instant>,

    pub disk:          PhaseState<AbsolutePath>, // Keyed
    pub git:           PhaseState<AbsolutePath>, // Keyed
    pub repo:          PhaseState<OwnerRepo>,    // Keyed
    pub lint:          PhaseState<AbsolutePath>, // Keyed
    pub lint_startup:  PhaseState<!>,            // Counted — no key type
    pub metadata:      PhaseState<AbsolutePath>, // Keyed; added in Phase 1
}
```

The `enum` split is deliberate: a `KeyedPhase<K>` tracks a finite set of identities; a `CountedPhase` tracks cardinality only. Hiding both behind a single struct with an unused `HashSet<()>` invites silent drift (someone adds `.insert(())` for symmetry and the counter semantics degrade). The split makes the invariant a type-system property.

`Option<HashSet<K>>` on `KeyedPhase` preserves the `Unknown` vs `Set(empty)` distinction the current tracker tracks via `Option<HashSet<_>>` at `src/tui/app/types.rs:75, 87`. `CountedPhase` covers `lint_startup_expected: Option<usize>` / `lint_startup_seen: usize` at `:90-92`.

Shared helpers operate on the enum:

```rust
pub fn is_complete<K: Hash + Eq>(s: &PhaseState<K>) -> bool {
    match s {
        PhaseState::Keyed(k) => matches!(&k.expected, Some(e) if k.seen.len() == e.len()),
        PhaseState::Counted(c) => matches!(c.expected, Some(n) if c.seen == n),
    }
}
```

### Work

- Add `PhaseExpectation<K>` and `PhaseState<K>`.
- Replace every named field group on `StartupPhaseTracker` with a typed `PhaseState`, including `lint_startup` (which uses `PhaseState<()>` with `PhaseExpectation::Count`).
- Extract completion / toast update logic from `initialize_startup_phase_tracker` and `maybe_log_startup_phase_completions` (`src/tui/app/async_tasks.rs:698-810`) into helpers over `&mut PhaseState<K>`.
- No behavior change. The tracker's external semantics (`is_scan_complete`, toast grouping, duration display) are unchanged.

### Adding the `Metadata` phase

Phase 1 adds `PhaseKind::Metadata` and a `metadata: PhaseState<AbsolutePath>` field populated with one entry per detected workspace root. `BackgroundMsg::CargoMetadata` marks `seen` on arrival. Completion, toast display, and duration rendering come from the shared helpers — no metadata-specific branches.

## Phase 1 — Adopt `cargo_metadata` For Accuracy

### New dependency

```toml
cargo_metadata = "0.22"
```

Invocation: `MetadataCommand::new().current_dir(root).no_deps().other_options(vec!["--offline".into()]).exec()`. `--no-deps` skips transitive dependency resolution (the step that normally triggers network I/O); `--offline` is a belt-and-suspenders guarantee that scanning never reaches out. Scans must never surprise the user with outbound traffic.

**What `--offline` does *not* break.** With `--no-deps`, cargo does not resolve transitive dependencies and does not consult the registry index. A cold registry cache is fine. The combination fails only on manifests whose own declared dependencies cannot be validated locally — typically broken path deps or missing git deps. Those failures surface through the normal error-toast path and are indistinguishable to the user from malformed-manifest failures; the fix is the same (correct the manifest or fetch the missing dep with an explicit `cargo fetch`).

Runtime cost is one `cargo metadata --no-deps` invocation per *workspace root* (not per package). On large workspaces cargo parses the whole tree once and returns all members; we piggyback.

### Where metadata lives

One snapshot per workspace, keyed by `workspace_root`. Held in `App` as a `WorkspaceMetadataStore`. `RustInfo` / `Package` carry only a lightweight reference (`WorkspaceMetadataHandle { workspace_root: AbsolutePath, package_id: PackageId }`) so the workspace snapshot is not duplicated across every member.

**Access pattern.** Resolution is a single helper on `App`:

```rust
impl App {
    pub fn resolve_metadata(&self, handle: &WorkspaceMetadataHandle) -> Option<&PackageRecord>;
    pub fn resolve_target_dir(&self, root: &AbsolutePath) -> Option<&AbsolutePath>;
}
```

Render and query code calls these two helpers; nothing else threads `&WorkspaceMetadataStore` around. Both return `Option`, and every caller must tolerate `None` — that is the first-paint state before metadata arrives and the permanent state for projects whose `cargo metadata` call fails. The rule is: **UI must render with `None`.** See the fallback section below for what that looks like.

```rust
// src/project/cargo_metadata_store.rs (new)
pub struct WorkspaceMetadataStore {
    by_root: HashMap<AbsolutePath, WorkspaceSnapshot>,
}

pub struct WorkspaceSnapshot {
    pub workspace_root:    AbsolutePath,
    pub target_directory:  AbsolutePath,       // resolved
    pub packages:          HashMap<PackageId, PackageRecord>,
    pub workspace_members: Vec<PackageId>,     // authoritative
    pub fetched_at:        SystemTime,
    pub fingerprint:       ManifestFingerprint, // snapshot-input, see below
}

pub struct PackageRecord {
    pub id:            PackageId,
    pub name:          String,
    pub version:       semver::Version,
    pub edition:       String,
    pub description:   Option<String>,
    pub license:       Option<String>,
    pub homepage:      Option<String>,
    pub repository:    Option<String>,
    pub manifest_path: AbsolutePath,
    pub targets:       Vec<TargetRecord>,
    pub publish:       PublishPolicy,          // preserves ["registry"] allowlist form
}

pub enum PublishPolicy {
    Any,                          // default
    Never,                        // publish = false
    Registries(Vec<String>),      // publish = ["crates-io", ...]
}

pub struct TargetRecord {
    pub name:              String,
    pub kinds:             Vec<TargetKind>,
    pub src_path:          AbsolutePath,
    pub edition:           String,
    pub required_features: Vec<String>,
}
```

`Submodule` and `Vendored` variants are treated as **independent snapshots** when they contain a `Cargo.toml`. If they fail to produce metadata (common for vendored stubs), they fall back to the hand-rolled parser like any other metadata-less project.

### New async message

```rust
// src/scan.rs
pub enum BackgroundMsg {
    // ...
    CargoMetadata {
        workspace_root: AbsolutePath,
        fingerprint:    ManifestFingerprint,  // captured BEFORE spawn
        result:         Result<WorkspaceSnapshot, CargoMetadataError>,
    },
}
```

- Dispatched once per detected workspace root.
- Runs on the blocking pool. Concurrency capped at `num_cpus`.
- Timeout: 10s.

### `ManifestFingerprint`

```rust
pub struct ManifestFingerprint {
    pub manifest:        FileStamp,                       // <workspace_root>/Cargo.toml
    pub lockfile:        Option<FileStamp>,               // <workspace_root>/Cargo.lock
    pub rust_toolchain:  Option<FileStamp>,               // rust-toolchain[.toml] — can shift target_directory
    pub configs:         BTreeMap<PathBuf, Option<FileStamp>>, // every ancestor .cargo/config[.toml] path; None = absent
}

pub struct FileStamp {
    pub mtime:        SystemTime,
    pub len:          u64,
    pub content_hash: [u8; 32], // blake3 of file bytes — authoritative for equality
}
```

Equality uses `content_hash` only. `(mtime, len)` is a cheap pre-check that skips the hash when the file clearly changed; it does not participate in equality.

Inode is deliberately omitted. Editors (vim `writebackup`, VS Code atomic save) write to a temp path and `rename(2)` on commit, which changes the inode on every save even when the bytes are identical. Using inode in equality would invalidate the snapshot on every no-op save and defeat the point of hashing. `content_hash` covers identity; hashing a few KB of TOML is a sub-ms cost.

Ancestor config walk:

- Start at workspace root, ascend to `CARGO_HOME` (resolved from env at startup; default `~/.cargo`).
- At each level, check both `config.toml` and the legacy extensionless `config`.
- Stop at `CARGO_HOME`; do not walk past it.
- Record every candidate path in the `configs` map — present files as `Some(FileStamp)`, absent files as `None`. A `None → Some` transition (a file being created) is itself a diff and invalidates the snapshot.

### In-flight race handling

Fingerprint is captured before spawn. When the result arrives, recompute the fingerprint. If it matches, commit; if not, discard and re-dispatch. To prevent unbounded retry loops on rapid edits, each workspace carries a monotonic `dispatch_generation: u64`. A spawn captures the current generation; completion commits only if the captured generation still equals the live generation at merge time. In-flight work stamped with an older generation is dropped rather than recomputed, so a save-storm converges to a single accepted result once edits settle.

### Watcher integration

`src/lint/trigger.rs` already classifies `Cargo.toml` and `Cargo.lock` events. Extend its classifier to additionally emit a `MetadataRefresh` signal for:

- `Cargo.toml`, `Cargo.lock`
- `rust-toolchain`, `rust-toolchain.toml`
- `.cargo/config.toml` *within the project tree*

`.cargo/config.toml` files above the project tree are not watched; those changes are picked up opportunistically via the fingerprint walk the next time any project-local file change triggers a refresh, or on a lazy TTL (30 minutes) re-fingerprint pass. This is an accepted staleness window; a full home-dir watcher is out of scope.

### Call-site migrations

Each step is independently shippable. Steps 2+ all require a **fallback path** back to the hand-rolled parser so first-paint works before metadata has arrived.

1. **Land the plumbing.** Dep, `WorkspaceMetadataStore`, `BackgroundMsg::CargoMetadata`, fingerprint+race handling, async dispatch. Nothing consumes it yet.
2. **Resolved target dir at path-check sites.**
   - `src/tui/app/query.rs:181` — existence check against `snapshot.target_directory` when available, else `<project>/target`.
   - `src/watcher.rs:542` — target-path prefix check against resolved dir.
   - `src/scan.rs:979, :1262` and `src/lint/trigger.rs:61-64` — keep the string-literal `"target"` skip as the fallback, **and** also skip the resolved `target_directory` when `store.by_root.contains_key(&workspace_root)`. The lint walker must honor both, otherwise out-of-tree targets that live under a watched root get linted as source.
   - Confirm dialog copy at `src/tui/render.rs:269` shows the resolved path so the user knows what's being cleaned.
   - **Disk usage is explicitly NOT changed in this step.** See below.

#### Disk usage: physical bytes, broken down in the detail pane

**Recomputation strategy.** One walk, two counters. `scan::dir_size` (or its successor) walks the project tree once per enrichment/watcher trigger and accumulates two sums: `in_project_non_target` (everything except the `target/` dir and the resolved `target_directory` if it's in-tree) and `in_project_target` (the in-tree target, if any). Out-of-tree target dirs get a separate walk keyed by resolved `target_directory`, cached on `WorkspaceSnapshot`, invalidated only when (a) the target dir's mtime changes, (b) a clean completes for that dir, or (c) the snapshot itself is re-fetched. No double-walks on the common case; no perpetually-lagging out-of-tree numbers.


The project list's `Disk` column is unchanged and unambiguous: physical bytes on disk rooted at this project's path. Nothing borrowed, nothing attributed — just what's physically there. Safe to sort, safe to sum.

The **Workspace / Package / Worktree detail pane** replaces the single `Disk` row with a breakdown.

Owner of its target dir (either in-tree, or out-of-tree with nobody else pointing at it):

```
Disk      22.3 GiB
  project     1.8 GiB
  target     20.5 GiB
```

Sharer (points at another project's target dir):

```
Disk       1.8 GiB
  project    1.8 GiB
  target       0 B   shared with ../bevy_brp_style_fix
```

Rules:

- Top-line `Disk` = `project + target` for the current row, where `target` is 0 for sharers.
- `project` = tree walk rooted at this project, excluding both the literal `target` dir and the resolved `target_directory`.
- `target` = bytes physically under `target_directory` *and* rooted at this project. 0 when the resolved target dir is out-of-tree at another project's path.
- The "shared with" pointer names the owning project. If more than one other project shares: `shared with ../bevy_brp_style_fix (+2 others)`.
- No snapshot: breakdown renders as a single `Disk` row (current behavior).

This is Phase 1.5 — after the fingerprint + resolution plumbing, before Phase 2.
3. **Workspace members AND build-target discovery (atomic).**
   - These two must ship together. If step 3 swaps members to snapshot data but step 4 still hand-rolls targets, a multi-member workspace shows rows for `b` and `c` with zero targets until the targets pass lands — which is every real-world Rust repo.
   - When a snapshot exists, use `workspace_members` directly in `build_tree` (`src/scan.rs:479-564`) **and** filter `PackageRecord.targets` for the Targets pane.
   - Derive "grouped examples by subdirectory" from `TargetRecord.src_path` relative to the package manifest.
   - Project type (Workspace/Binary/Library/ProcMacro in `src/tui/panes/support.rs:687-701`) becomes a function of `Vec<TargetKind>` across packages.
   - Without a snapshot: workspaces render as a single row; targets pane shows "Loading…". Hand-rolled parsers are deleted (see Retired code).
   - When the snapshot arrives, members appear and targets populate.
5. **Package pane fields.**
   - Point `src/tui/panes/support.rs:687-701` at `PackageRecord`. Surface edition, license, homepage, repository (currently parsed by nobody).
6. **Retire hand-rolled parsing.**
   - `src/project/cargo.rs:71-157` and all helpers listed in "Retired code" are deleted. Only the minimal `[workspace]` presence probe survives; see the Fallback policy section.

### Fallback policy (cold start and permanent failures)

`cargo metadata` is expected to complete in milliseconds per workspace. Cold-start gap between first paint and snapshot arrival is brief, but `build_tree` at `src/scan.rs:479-564` still needs to classify each discovered `Cargo.toml` as `Workspace` vs `Package` *before* any metadata arrives — otherwise workspaces render as flat rows then re-tree themselves when snapshots land, causing visible row-jumps and cursor reset.

**Minimal bootstrap parser.** One targeted read per `Cargo.toml` that checks only one thing: is a `[workspace]` table present? A single `toml::from_str` into a struct with an optional `workspace` field. No other fields extracted — we just need the classification. This bootstrap is the only piece of hand-rolled parsing retained.

Everything else falls to snapshots:

- **Name, path:** extracted by the bootstrap.
- **Version, description, edition, license, homepage, repository, targets:** **empty placeholders** (`—` / "Loading…") while the snapshot is absent.
- **Workspace members:** when a `Cargo.toml` has `[workspace]` and no snapshot yet, render the row as a workspace with a single `Loading members…` child. When the snapshot arrives, members populate. No flat-then-tree restructure.
- **Indicator:** a trailing `~` after the project name marks a row whose snapshot is absent (either still loading or metadata failed). Drops when the snapshot commits.

Row height and sort order must not depend on snapshot-only fields; the project list sorts on name/path only.

### Observability

Every `cargo metadata` invocation raises a toast via the shared `push_task` / `finish_task` API. Grouping depends on startup state:

- **During startup** (`startup_complete_at.is_none()`): metadata invocations are tracked items inside a single grouped toast `Running cargo metadata (N workspaces)`, mirroring the existing `Calculating disk usage` pattern at `src/tui/app/async_tasks.rs:774`. 50 workspaces produce one toast with 50 tracked items, not 50 toasts.
- **Post-startup**: individual toasts per invocation, keyed by workspace root, with the normal linger behavior.

**Each tracked item uses the existing spinner + elapsed renderer** (`tracked_item_line` in `src/tui/toasts/render.rs:381-423`) — same code path GitHub / lint / disk toasts already use. While a call is in flight, the item line shows the workspace path with an animated Braille spinner and the elapsed time; `format_elapsed` at `render.rs:369-378` renders durations under 10 s as `Nms` (e.g., `247ms`, `42ms`), so metadata calls display to millisecond precision with no new formatting code. On completion `finish_task` freezes the final duration next to the item with the spinner slot reserved so the number doesn't jump.

This is deliberately noisy at first. It answers two questions we can't answer from the plan alone: how often does `Cargo.lock` churn drive re-dispatch in real use, and how long does a single `cargo metadata` actually take on the user's workspaces. Once those are confirmed acceptable, post-startup toasts can be silenced behind a setting (e.g., `show_metadata_refresh_toasts = false`). Until then, they stay on.

### Metadata errors

When `cargo metadata` fails for a workspace, the affected rows render in fallback state (`~` indicator, placeholders) and a toast is raised naming the workspace and quoting the specific error:

```
Metadata failed for ~/rust/bevy_hana:
  error: failed to parse manifest at `~/rust/bevy_hana/member_x/Cargo.toml`
  expected `]`, found `=` at line 42
```

The error text is the first line of `cargo metadata`'s stderr. Toast is keyed by workspace root; a new failure for the same workspace replaces the existing toast rather than stacking.

**Interaction with the observability toast.** On failure, do **not** call `finish_task` on the observability toast (which would render it as a successful "done" next to a red error for the same event). Instead, dismiss the observability toast and let the error toast be the single record of the call. On success, `finish_task` as normal.

When any watched input for that workspace changes (manifest, lockfile, `rust-toolchain*`, tracked `.cargo/config.toml` files), the next `cargo metadata` attempt runs automatically. On success, the toast is cleared. No retry logic, no partial-snapshot synthesis — one metadata call per invalidation, one toast per failure, toast drops on recovery.

### Retired code

The following is deleted once Phase 1 ships:

- `src/project/cargo.rs:71-157` (`from_cargo_toml`) and all helpers at `:171-201`, `:205-245`, `:272-319`, `:323-370`, `:372-388`.
- `src/scan.rs:566-682` (`workspace_member_patterns*`, `normalize_workspace_path`, `workspace_pattern_matches*`).

The `toml` crate dependency stays — the minimal bootstrap parser above uses it for the `[workspace]` presence check.

### Performance

- One `cargo metadata` per workspace. Expected runtime is in milliseconds on typical workspaces; concrete numbers land via the Observability toasts (see Phase 1 Observability).
- Concurrency capped at `num_cpus` via the existing async-tasks throttle.
- Snapshots are a few KB; keeping them resident is fine for any realistic project count.

## Phase 2 — Sane Clean

### Target-dir index

Built incrementally in the TUI event loop (single-threaded — no locking) as `BackgroundMsg::CargoMetadata` messages arrive:

```rust
// src/tui/app/target_index.rs (new)
pub struct TargetDirIndex {
    by_target_dir: HashMap<AbsolutePath, Vec<TargetDirMember>>,
    by_project:    HashMap<AbsolutePath, AbsolutePath>, // reverse index: project_root → current target_dir
}

pub struct TargetDirMember {
    pub project_root: AbsolutePath,
    pub kind:         MemberKind, // Project, Submodule, Vendored
}

impl TargetDirIndex {
    /// Sets/replaces the target_dir for `member.project_root`. If the project
    /// previously lived under a different target_dir, removes the stale entry
    /// before inserting the new one. Safe to call repeatedly.
    pub fn upsert(&mut self, member: TargetDirMember, target_dir: AbsolutePath);

    /// Removes all entries for `project_root`. Called when a project disappears
    /// from the scan set.
    pub fn remove(&mut self, project_root: &AbsolutePath);

    pub fn siblings(&self, target_dir: &AbsolutePath, exclude: &[AbsolutePath]) -> Vec<&TargetDirMember>;
}

// Free function, not a method on the index.
pub fn build_clean_plan(
    index: &TargetDirIndex,
    store: &WorkspaceMetadataStore,
    selection: CleanSelection,
) -> CleanPlan;

pub enum CleanSelection {
    Project { root: AbsolutePath },
    WorktreeGroup { primary: AbsolutePath, linked: Vec<AbsolutePath> },
}

pub struct CleanPlan {
    pub targets:         Vec<CleanTarget>,       // deduped on target_directory
    pub affected_extras: Vec<AbsolutePath>,      // projects sharing a target but NOT part of the selection
    pub skipped:         Vec<(AbsolutePath, SkipReason)>, // DeletedWorktree, NoMetadata, NotRust
}

pub struct CleanTarget {
    pub target_directory:  AbsolutePath,
    pub exists_on_disk:    bool,
    pub method:            CleanMethod,
    pub covering_projects: Vec<AbsolutePath>,
}

pub enum CleanMethod {
    /// Shell out to `cargo clean` with cwd = project root. Cleans the entire
    /// target_directory resolved from that cwd. Any sibling sharing the dir
    /// is also wiped; the confirm dialog must list them.
    CargoClean { cwd: AbsolutePath },
}
```

**Why `CleanMethod` has only one variant.** Cargo's `cargo clean -p <spec>` accepts package names, not `PackageId`s, and even when a matching spec exists, `-p` cleans the package that resolves from `cwd` — it does not isolate worktree-A's artifacts from worktree-B's in a shared target dir. Artifacts are fingerprinted by source-root path inside `target/debug/.fingerprint` and `target/debug/deps`, and walking that tree is the only way to achieve per-worktree isolation.

**We are not doing that, now or later.** If worktrees share a target directory, a clean wipes the whole thing. Full stop. The confirm dialog lists every affected checkout so the user knows what's going. Users who want isolated cleans set up unshared targets. If someone wants different behavior, they can submit a PR.

`--target <triple>` handling is similarly out of scope. `cargo clean` without `--target` removes all triples; we accept that.

### Submodules and vendored projects in the index

Submodules and vendored projects almost always share the parent workspace's `target_directory`. Without special handling, `siblings()` on the parent's target returns the submodule as a "sibling" and the confirm dialog shows *"Will also affect: ../bevy/vendored/foo"* — technically true, practically confusing.

Rule: `siblings()` returns `TargetDirMember`s tagged by `MemberKind`. The confirm dialog lists `MemberKind::Project` entries plainly and groups `Submodule` / `Vendored` entries behind a collapsed *"N nested crate(s) also share this target"* row.

### Per-worktree clean

- Eligible rows: `VisibleRow::Root` (Rust), `VisibleRow::WorktreeGroupHeader`, `VisibleRow::WorktreeEntry`. Gated by `App::clean_selection(&self) -> Option<CleanSelection>`.
- **Before opening the confirm dialog**, re-fingerprint the selected workspace. If the fingerprint matches the stored snapshot, the dialog opens immediately with the cached plan. If it differs, dispatch `BackgroundMsg::CargoMetadata` and render a transient dialog labelled `Verifying target dir…` with the y/n buttons disabled; on snapshot arrival, swap the dialog contents to the refreshed plan and enable y/n. On timeout (the existing 10s cap) or failure, the dialog shows the error body and offers **Retry** / **Cancel** only — never a Clean option against a plan we couldn't verify. This is how the doc's "synchronous re-fingerprint" is implemented without blocking the TUI event loop.
- Confirm dialog — uniform rule: always state exactly what will be cleaned.
  - **Single affected checkout** (no sharing): `Clean <resolved target dir>? (y/n)`.
  - **Multiple affected checkouts** (shared target): list every affected checkout explicitly, then prompt. Example:
    ```
    This target is shared. Cleaning will affect:
      ../bevy_hana
      ../bevy_main
      ../bevy_diegetic
    Clean all? (y/n)
    ```
    Cap at 5 visible entries with a trailing `"+N more"` line when the list is longer.
  - If nested (Submodule/Vendored) entries exist: appended as a collapsed *"N nested crate(s) also share this target"* line below the checkout list.
- Execution: `CleanMethod::CargoClean { cwd }` — shell out to `cargo clean` with `cwd = project root`.

### Mixed worktree groups

A `WorktreeGroup` can contain checkouts that share a target dir plus checkouts that have their own. `build_clean_plan` handles this by deduping on `target_directory`: unique dirs produce one `CleanTarget` each; shared dirs collapse to a single `CleanTarget` whose `covering_projects` lists every worktree that points at it.

**Selection exclusion.** `build_clean_plan` computes `selection_set = {primary} ∪ linked` once at the start and excludes it when populating `affected_extras`. `covering_projects` entries that are within `selection_set` are still listed (they're *what* is being cleaned), but they are not labelled as "affected extras" — that slot is reserved for collateral projects the user did not pick. Without this, a group-level clean reports its own members as "also affected," which looks like a bug.

The confirm dialog shows one line per unique target dir. A 3-worktree group with 2 sharing and 1 solo renders as:

```
This will clean 2 target directories:
  /shared/target        (used by ../a, ../b)
  ../c/target           (used by ../c)
Clean all? (y/n)
```

### "Already clean" semantics

Cargo clean is idempotent, so the existing toast at `src/tui/app/query.rs:180` is advisory, not required. New rule: show "Already clean" only when **every** `CleanTarget` in the plan has `exists_on_disk = false` **and** `affected_extras.is_empty()`. If any sibling shares a now-empty dir, or any target dir still has bytes, we run the command anyway. This prevents the shared-target regression where the first clean removes the dir and every subsequent click wrongly toasts "Already clean."

### Group-level clean (worktree group header)

- `build_clean_plan` collects every worktree's resolved `target_directory`, dedupes, and emits one `CleanTarget` per unique dir (with `covering_projects` naming the worktrees behind it).
- Confirm dialog: *"Clean 3 target directories across 4 worktrees?"* with an expand toggle that lists the paths.
- Deleted worktrees (directories listed by git but missing on disk) are placed in `skipped` with reason `DeletedWorktree` and do not abort the plan.

### Gating fix (the bug that started this)

Three call sites gate clean on `selected_item().is_some_and(RootItem::is_rust)` — which returns `None` for any non-`Root` row. Each must route through `App::clean_selection`:

- `src/tui/panes/actions.rs:101-109` — `request_clean()`
- `src/tui/input.rs:599-607` — project-list `Clean` action
- `src/tui/render.rs:638-652` — status-bar shortcut visibility. Note: this block also feeds `shortcuts::for_status_bar`; update that signature to accept `CleanEligibility` rather than a raw `is_rust: bool`.

### Scope of share-detection

Only projects already known to cargo-port. We do not scan the filesystem for strangers. If a hundred projects share a target dir via a top-level `.cargo/config.toml`, the confirm dialog truncates the list with a `+N more` affordance instead of running away.

### Scope of config resolution

Cargo resolves `.cargo/config.toml` up to `$CARGO_HOME/config.toml` (or `$CARGO_HOME/config` — both names are legal). We delegate resolution to `cargo metadata`, which inherits that for free. Our *fingerprint walk* must match: resolve `CARGO_HOME` at startup, walk both filename variants at each level, stop at `CARGO_HOME`.

The TUI process's `CARGO_TARGET_DIR` env *does* leak into resolution (cargo reads process env at invocation). We accept this: if the user launches the TUI with an override, they get the override. Called out in the clean-confirm help copy so it isn't surprising.

### Ancestor config watching

Cargo's config lookup walks from the project root up to the filesystem root, then checks `$CARGO_HOME`. All of these `.cargo/config.toml` (and legacy `.cargo/config`) files are potential inputs to the resolved `target_directory`, so all of them must be watched.

Strategy: at project-scan time, enumerate every `.cargo/` directory between the project root and `$CARGO_HOME` (inclusive). Watch the **directories**, not the files — `notify` does not auto-subscribe to files that appear after registration, so watching only extant `config.toml` files would miss the "user creates `~/.cargo/config.toml` for the first time" case. Watching the parent directory fires a Create event when the file appears; the handler then re-fingerprints.

Watched directories are deduped across projects (ancestor paths are heavily shared). When a project is added or removed, recompute and diff the watched-directory set. No home-directory-wide watcher; the set is bounded by `sum(tree_depth for each project) + 2`. If a `.cargo/` directory itself does not exist yet, walk up one more level and watch its parent until `.cargo/` appears.

On any Create/Modify/Delete event whose filename is `config` or `config.toml`, re-fingerprint and re-dispatch `cargo metadata` for every workspace whose ancestor chain includes the changed path.

**Rewatch on `.cargo/` creation.** If a watched ancestor was a placeholder (the `.cargo/` directory didn't exist yet, so we watched its parent), a Create event whose basename is `.cargo` must both trigger a re-fingerprint *and* recompute the watched-directory set so subsequent `config.toml` creations inside that new `.cargo/` are seen. Handler: on any Create event where `basename == ".cargo"`, re-diff the watch set before falling through to the fingerprint path.

No TTL pass. The synchronous re-fingerprint on clean-confirm is kept as a redundant safety net.

## Testing

Every phase lands with tests. Not as a follow-up ticket.

- **Phase 0.** Unit tests on `PhaseState<K>` covering: `PhaseExpectation::Unknown` vs `Set(empty)` distinction; `Set` and `Count` variants both complete correctly; empty/partial/complete transitions; completion idempotency; toast-id persistence across updates. `lint_startup` tracked via the shared helpers.
- **Phase 1 plumbing.**
  - `ManifestFingerprint`: content hash defeats `(mtime, len)` ABA — test writes two files with identical `(mtime, len)` but different bytes and asserts inequality. Rename-save (write tmp + `std::fs::rename`) with identical content must *not* invalidate the snapshot (inode change ignored). New-file appearance: `configs` map transitions `None → Some(FileStamp)` and invalidates. Dispatch-generation coalesces rapid re-dispatches — simulate a save-storm and assert at most one final accepted result.
  - Integration test spawns `cargo metadata` against a fixture workspace and checks `WorkspaceSnapshot` fields.
  - `--offline` failure path: fixture with a missing path-dep asserts the error surfaces as a toast, the row enters fallback state, and a later fix + save triggers recovery (toast cleared, row committed).
- **Phase 1 migrations.** Snapshot-driven fixtures: workspace with members under `member-*/`, project with `CARGO_TARGET_DIR` override, project with `.cargo/config.toml` setting `build.target-dir`, project with `rust-toolchain.toml`. Each asserts: resolved `target_directory`, members rendered, targets pane populated.
- **Phase 1.5 disk breakdown.** Fixtures for: in-tree target, out-of-tree target (owner), out-of-tree target shared between two worktrees. Assert detail-pane breakdown numbers and the "shared with" pointer.
- **Phase 2 clean.**
  - Fixtures for: single project (no sharing), worktree group with all-unique targets, worktree group with partial share (2 share, 1 solo), worktree group with all sharing. Assert `CleanPlan.targets`, `affected_extras`, `skipped`.
  - Self-sibling exclusion: group selection of `{a, b, c}` where `a` and `b` share a target must not list `a`/`b` in `affected_extras` (they're *in* the selection).
  - `TargetDirIndex::upsert` correctness: project whose `target_directory` changes is removed from its stale bucket and inserted into the new one (no phantom entries). `remove` fully evicts.
  - Gating fix: `App::clean_selection` returns `Some` for `WorktreeEntry` / `WorktreeGroupHeader` / `Root(Rust)` and `None` for other rows.
- **Ancestor config watching.**
  - Writes a new `~/.cargo/config.toml` (directory previously existed and was empty) and asserts a Create event triggers a metadata refresh — exercises the "watch the directory, not the file" rule.
  - `mkdir ~/.cargo` when `.cargo/` did not previously exist: the Create event for `.cargo` must both re-fingerprint *and* recompute the watched-directory set. Subsequent `touch ~/.cargo/config.toml` must then trigger a second refresh.
  - Mutating an existing ancestor config triggers a refresh.
- **Toast coexistence.**
  - Startup: 50-workspace fixture produces one grouped `Running cargo metadata (N workspaces)` toast, not 50 individuals.
  - Failure: observability toast is dismissed (not `finish_task`'d) when the call errors, and the error toast is the only remaining record.
  - Recovery: a successful call after a prior failure clears the error toast.

- **`CleanMethod` single-variant invariant.** Exhaustiveness test that pattern-matches every `CleanTarget.method` produced by `build_clean_plan` against `CleanMethod::CargoClean { .. }` under `#[deny(non_exhaustive_omitted_patterns)]`. A future PR that adds a new variant to work around shared-target friction will fail this test and force the conversation back to the design doc.

- **Clean-confirm re-fingerprint.** Fixture that (a) opens a clean dialog with fingerprint unchanged and asserts it renders immediately with y/n enabled; (b) edits an ancestor config between the previous metadata call and the confirm, asserts the dialog opens in `Verifying target dir…` state with y/n disabled, then transitions to the refreshed plan on snapshot arrival; (c) forces a metadata failure and asserts the dialog shows Retry / Cancel, no Clean.

Use `cargo nextest run` to run tests.

## Migration Order

0. Refactor `StartupPhaseTracker` to `PhaseState<K>` (per Phase 0). No behavior change.
1. Plumbing: dep, store, async fetch, fingerprint (including content hash + dispatch generation), race guard, watcher extension, `CARGO_HOME` resolution + ancestor directory watching. New metadata phase wired into the shared phase tracker from Phase 0.
2. Resolved target-dir at path-check sites (`query.rs`, `watcher.rs`, lint/scan target-skip). Confirm dialog copy updated. **Disk usage is unchanged.**
3. Workspace members + Targets pane migrate to snapshots **atomically**. Glob matcher retained as cold-start fallback for members; targets show "Loading…" without a snapshot.
4. Package pane fields: edition / license / homepage / repository.
5. Phase 1.5: split `project_bytes` / `target_bytes` columns. Attribute shared targets to the owning workspace root. Never sum.
6. Phase 2: `TargetDirIndex` (with `MemberKind`), `CleanSelection`, `build_clean_plan`, gating fix, per-worktree clean with sync re-fingerprint on confirm, shared-target confirm copy.
7. Phase 2 cont'd: group-level fan-out clean with dedupe.

## Risks And Open Questions

- **Cold-start latency.** First scan dispatches one `cargo metadata` per workspace. Observability toasts report per-call duration. Concrete gate: if p95 cold-call duration exceeds 500ms on the user's tree, drop the concurrency cap from `num_cpus` to `num_cpus / 2` and re-measure.
- **`EnterWorktree` interaction.** Entering a worktree should not invalidate the parent's snapshot; only the newly-entered worktree's fingerprint is re-checked.
- **`Cargo.lock` as a fingerprint input.** Kept. Metadata is fast enough that the churn is acceptable; the toast frequency we observe post-launch will tell us if this changes.
- **Help copy.** Explain `CARGO_TARGET_DIR` env leak and shared-target semantics where the user will look (clean-confirm help tooltip, README target-dirs section).
