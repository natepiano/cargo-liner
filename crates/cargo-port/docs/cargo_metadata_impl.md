# Cargo Metadata — Implementation Plan

Companion to `docs/cargo_metadata.md` (the design plan). This doc covers *how* to land the design in shippable increments, and what context is needed at each step. Read it alongside the design plan — it does not re-specify behavior or rationale.

## Reading Order

1. `docs/cargo_metadata.md` — design plan (authoritative on behavior).
2. This doc — sequencing, code-touch lists, per-step verification.
3. The codebase modules named in each step.

When the two docs disagree, the design plan wins; open a PR to reconcile this one.

## Progress Snapshot

As of the latest commit on `enh/cargo-metadata`:

| Step | Status | Commits |
|---|---|---|
| 0 — `PhaseState` refactor | ✅ shipped | `a595aeb` |
| 1a — Plumbing (dep, store, dispatch, handler, resolve helpers, toasts) | ✅ shipped | `c050152` + `bcf64fe` |
| 1b 7a — Watcher classifier + in-tree refresh | ✅ shipped | `0293570` |
| 1b 7b — Ancestor `.cargo/` watch-set | ✅ shipped (pure helpers `708954b`, integration `7a78e80`) |
| 2 — Resolved target dir at path-check sites | ✅ shipped | `4a50b1c` |
| 3a — Targets pane from `PackageRecord.targets` | ✅ shipped | `43230d5` |
| 3b — Retire hand-parsed Targets fallback | ✅ shipped (Targets pane `d435c40`; detail-pane version/description `12b8163`; `Cargo` field stamp from snapshot `3b6afc9`) |
| 4 — Package pane fields (edition/license/homepage/repository) | ✅ shipped | `c6ab1f3` (render-buffer viewport fix `2d9a86a`) |
| 5a — Single-pass walker yields in-target/non-target split | ✅ shipped | `94ef21e` |
| 5b — Detail pane disk breakdown | ✅ shipped (in-tree split `7308b3b`; cached out-of-tree walk `77e7dff`) |
| 6a/6b — `TargetDirIndex` + `build_clean_plan` pure types | ✅ shipped | `3cd343a` |
| 6c — Index maintenance + `App::clean_selection` + gating fix | ✅ shipped | `cfb67c7` |
| 6d — Confirm dialog lists affected siblings + nested collapse | ✅ shipped | `54c7501` |
| 6e — Async clean-confirm re-fingerprint UX | ✅ shipped (`2cc2dd3`; scoped: `Option<AbsolutePath>` state on App rather than a full ConfirmAction shape change — expand if WorktreeGroup cleans need per-state rendering.) |
| 7 — Group-level fan-out + `DeletedWorktree` skip | ✅ shipped | `1e35e5d` |
| 7b — Ancestor placeholder → CargoDir promotion on `mkdir .cargo` | ✅ shipped | `3aaf07e` |
| Clippy `-D warnings` pass | ✅ shipped (`0ff0cae`, re-verified after every subsequent commit) |

All design-plan items and the "narrow follow-ups" that were carried
alongside Steps 3b / 5b / 7b are landed. No open follow-ups tracked
against this doc.

## Shippable Increments

Each step is a single PR. Tests land in the same PR as the behavior (per design plan → **Testing**).

### Step 0 — `PhaseState` refactor

See design plan → **Phase 0** for the full shape and rationale.

Context:

- Current shape: `src/tui/app/types.rs:73-94`.
- `initialize_startup_phase_tracker` at `src/tui/app/async_tasks.rs:699`.
- `maybe_log_startup_phase_completions` at `src/tui/app/async_tasks.rs:815`.
- Grouped-toast pattern to mirror when adding the metadata phase: the "Calculating disk usage" call at `src/tui/app/async_tasks.rs:774`.
- No consumers outside `async_tasks.rs` and `types.rs` at read time.

Work:

- Implement Phase 0 per design plan → **Phase 0 → Shape**.
- Sequencing constraint: the `metadata` field is declared in the enum variant now (because it lives next to its siblings), but the `StartupPhaseTracker.metadata` field is **not** added and not populated until Step 1.

Verification:

- `cargo nextest run` passes existing tests unchanged.
- New unit tests per design plan → **Testing → Phase 0**.
- Render-snapshot test: build a tracker with keyed + counted phases populated, render through the existing toast pipeline, assert output string matches a committed fixture (covers "startup toast grouping/duration unchanged" automatically instead of relying on eyeballing).

### Step 1 — Plumbing only

See design plan → **Phase 1 → New dependency**, **Where metadata lives**, **`ManifestFingerprint`**, **In-flight race handling**, **Ancestor config watching**, **Observability**, **Metadata errors**.

Context:

- Async task plumbing: `src/scan.rs` (`BackgroundMsg`, `emit_*` helpers), `src/tui/app/async_tasks.rs` (message handling).
- Toast API: `src/tui/toasts/manager.rs` (`push_task`, `finish_task`, tracked items).
- Existing watcher: `src/watcher.rs`, `src/lint/trigger.rs:53-94` (classifier).

Work, in order:

1. Add `cargo_metadata = "0.22"` to `Cargo.toml`.
2. Create `src/project/cargo_metadata_store.rs` with `WorkspaceMetadataStore`, `WorkspaceSnapshot`, `PackageRecord`, `TargetRecord`, `PublishPolicy`, `WorkspaceMetadataHandle`, `ManifestFingerprint`, `FileStamp`.
3. Add `App::resolve_metadata` and `App::resolve_target_dir` helpers. Both return `Option`; no caller may unwrap.
4. Add the `metadata: PhaseState<AbsolutePath>` field to `StartupPhaseTracker` (the enum variant was declared in Step 0; this step attaches it to the tracker struct and populates it during scan).
5. Add `BackgroundMsg::CargoMetadata { workspace_root, fingerprint, result }`; dispatch one per detected workspace root during scan.
6. Implement fingerprint capture-before-spawn + dispatch-generation coalescing per design plan.
7a. **Classifier extension** in `src/lint/trigger.rs:53-94`: Create/Modify/Delete events on `Cargo.toml`, `Cargo.lock`, `rust-toolchain[.toml]`, and `config` / `config.toml` under any watched `.cargo/` directory → emit a metadata-refresh signal alongside the existing lint classification.
7b. **Ancestor-directory watch-set management** in `src/watcher.rs`: introduce a per-project "ancestor `.cargo/` watch set" computed at project-add time (walk from project root to `CARGO_HOME`, collect each `.cargo/` directory that exists, plus one parent placeholder when `.cargo/` is absent). Watch the directories, not individual files. On project add/remove, diff the global watch set (union across all projects) and register/unregister notifications. On Create events with basename `.cargo`, re-diff the set before falling through to the fingerprint path. This is a new subsystem; it is not a one-line classifier tweak.
8. Wire observability toasts:
   - **Startup grouped toast**: mirror `src/tui/app/async_tasks.rs:774` (`self.start_task_toast("Calculating disk usage", …)`) with a sibling call that pushes `"Running cargo metadata"` as the label and one tracked item per workspace root. Tracked items render via the existing `tracked_item_line` at `src/tui/toasts/render.rs:381`; no new formatting code — the design plan's millisecond duration comes for free from `format_elapsed`.
   - **Post-startup individual toasts**: `push_task("cargo metadata", <workspace path>, 1)`, finished on success with `finish_task`, dismissed on failure (replaced by the error toast).

No UI-visible behavior changes from Step 1 except the new toasts. Nothing consumes the snapshots yet — `resolve_*` helpers return `Some` but nothing calls them.

**Optional split into Step 1a / Step 1b.** Step 1 is intentionally large because every piece is pure plumbing with no consumers — each is trivially verifiable in isolation, but the whole stack must exist before Step 2 can use it. Default: ship as one PR. If diff size starts hurting review, split:

- **Step 1a:** items 1–6 and 8 — `cargo_metadata` dep, store + types, `App::resolve_*`, `BackgroundMsg::CargoMetadata`, fingerprint with content hash + dispatch-generation, race guard, `metadata` phase field, observability toasts. Metadata dispatches once at initial scan; there is no refresh behavior yet.
- **Step 1b:** items 7a and 7b — watcher classifier extension and the ancestor `.cargo/` watch-set subsystem. Layers refresh-on-edit behavior on top of the Step 1a plumbing.

The boundary is clean because Step 1a is self-contained: metadata fetched once at startup, never refreshed. Boring, works, ships. Step 1b is additive. Use this split only if the reviewer flags the combined diff as too large; otherwise one PR is simpler.

Verification:

- Fingerprint unit tests per design plan → **Testing → Phase 1 plumbing**.
- Integration test: launch against a fixture workspace, confirm `WorkspaceSnapshot` populates and a toast appears with `Nms` duration.
- Manual: launch TUI, observe grouped metadata toast at startup showing each workspace with spinner + ms.

### Step 2 — Resolved target dir at path-check sites

See design plan → **Call-site migrations → step 2**.

Context (verified file:line):

- `src/tui/app/query.rs:186` — `start_clean` existence check.
- `src/watcher.rs:542` — target-path classifier (`is_target_event`).
- `src/scan.rs:979, :1262`, `src/lint/trigger.rs:61-64` — directory-walk skip lists.
- `src/tui/render.rs:267-283` — `render_confirm_popup`. Today it only renders the string `"Run cargo clean?"` (no path). Step 2 extends the popup signature to accept and render the resolved target dir; **this is a surface extension, not a copy swap.**

Work:

- At every path-check site above, prefer `resolve_target_dir(&path)` over `project.join("target")`. `resolve_target_dir` accepts any project path and walks ancestors to the owning workspace root internally, so callers pass whatever path they already have (project root, worktree entry, etc.). Fall back to the literal when `None`.
- Extend `render_confirm_popup` to render the resolved path on its own line below the prompt. Thread the path from `ConfirmAction::Clean(abs_path)` through to the popup call.
- Disk usage is **not** touched in this step (Step 5 handles that).

Verification:

- Fixture unit tests per walk-skip site: construct paths that fall inside the resolved target dir and assert classification as "skip."
- Fixture test for `start_clean`: project whose resolved target is out-of-tree and present on disk — must *not* toast "Already clean."
- Render-snapshot test for `render_confirm_popup` with a resolved path, asserting the path appears.
- Manual (one case only): set `CARGO_TARGET_DIR=/tmp/target` in `~/.cargo/config.toml`, launch, trigger clean, confirm dialog shows `/tmp/target`.

### Step 3 — Workspace members + Targets pane (atomic)

See design plan → **Call-site migrations → step 3** and → **Retired code**.

Context:

- `src/scan.rs:479-564` — `build_tree`, the place members currently plug in via hand-rolled globs.
- `src/project/cargo.rs:71-388` — hand-rolled parsers to delete.
- `src/scan.rs:566-682` — hand-rolled glob matcher to delete.
- `src/tui/panes/support.rs:687-701` — project-type classification.

**Callers of the retired code that also need to be updated:**

- `src/watcher.rs:797` — project-refresh fallback calls `from_cargo_toml`. Replace with a snapshot re-dispatch (`BackgroundMsg::CargoMetadata`) and a bootstrap-probe read for immediate classification.
- `src/project_list.rs:682` — consumes `scan::normalize_workspace_path`. Either retain `normalize_workspace_path` as a pure public helper (it's just a path normalizer, independent of the glob engine) or inline its three lines at the call site.
- `src/tui/app/tests/mod.rs:483` — test calls `from_cargo_toml` directly. Replace with a snapshot fixture builder (simplest: construct a `WorkspaceSnapshot` manually in the test).
- `src/scan.rs:985, :1284` — internal `from_cargo_toml` calls inside `build_tree`-adjacent code. Replace with snapshot reads + the bootstrap probe.

Work:

- Add the minimal `[workspace]` presence probe to the scan bootstrap — the only surviving hand-parse, authoritative per design plan → **Minimal bootstrap parser**.
- Replace workspace-member detection with `WorkspaceSnapshot.workspace_members` when `store.by_root.contains_key(&workspace_root)`.
- Replace Targets-pane data with filters over `PackageRecord.targets`. Grouping by subdirectory derived from `TargetRecord.src_path` relative to the package manifest.
- Project-type classification becomes a function of `Vec<TargetKind>` across packages.
- Without a snapshot: workspace rows render `Loading members…`; targets pane shows `Loading…`.
- Delete the retired code. Run `cargo build` after each deletion — the four sites above will fail to compile until replaced.

Verification:

- Fixture tests per design plan → **Testing → Phase 1 migrations**.
- Regression test: the four caller sites above have a corresponding unit test or snapshot test that passes on the replacement code.
- Manual: open workspace with 10+ members, confirm tree renders correctly without flicker across the snapshot-arrival transition.

### Step 4 — Package pane fields

See design plan → **Call-site migrations → step 4**.

Context:

- `src/tui/panes/support.rs:687-701` — current package field rendering.
- `src/tui/panes/` — existing render-snapshot test harness (look for existing `insta`-style or manual render tests to follow the convention).

Work:

- Read `edition`, `license`, `homepage`, `repository` off `PackageRecord`. These are already parsed by `cargo_metadata`; nothing to extract manually.
- Update the pane rendering to show the new fields. When any is missing from the manifest, render `—`.

Verification:

- Render-snapshot tests for the package detail pane: one fixture with all four new fields populated, one fixture with each field individually missing, one fixture with all missing. Assert the rendered output matches committed snapshots.
- Manual: eyeball on a real project.

### Step 5 — Disk usage split (Phase 1.5)

See design plan → **Disk usage: physical bytes, broken down in the detail pane** and its **Recomputation strategy** subsection.

**Sequencing: Step 5 lands *after* Step 6.** This swap versus the design plan's "Migration Order" is an implementation convenience. Step 5's detail-pane "shared with …" pointer reads from `TargetDirIndex`, which is built in Step 6. Landing Step 5 first would require a temporary "(owner unknown)" placeholder that later gets replaced, duplicating render work. Landing Step 6 first costs nothing: Phase 2 doesn't read the disk breakdown. The design plan's behavior is unchanged; only shipping order flips.

Context:

- Disk enrichment: `src/enrichment.rs:42-48`, `src/scan.rs:462-470` (`dir_size`), `src/scan.rs:1485, :1502, :1548` (batch walker), `src/tui/app/async_tasks.rs:1557` (`apply_disk_usage`).
- `ProjectInfo.disk_usage_bytes` is read at ~20 call sites across `render.rs`, `snapshots.rs`, `query.rs`, `panes/support.rs`, `project/{non_rust,package,root_item}.rs`, and tests. Run `rg -n "disk_usage_bytes" src` before starting.
- Detail pane render: `src/tui/panes/support.rs` (workspace/package/worktree detail section).

Work:

- **Pin the `disk_usage_bytes` formula**: the list column remains "physical bytes rooted at this project's path," defined as `in_project_non_target + in_project_target` — the same number as today for owners (target is in-tree) and naturally smaller for sharers (their `in_project_target` = 0, because a sharer has no physical target under its root). Every one of the ~20 readers keeps working unchanged; no consumer code is touched in Step 5.
- Extend the walker to accumulate the two sub-counters (`in_project_non_target`, `in_project_target`) in a single pass and store them on `ProjectInfo` alongside the existing total.
- Add a second, cached walk keyed by resolved `target_directory` for out-of-tree targets. Store the cached result on `WorkspaceSnapshot`. Invalidate on (a) target-dir mtime change, (b) clean completion for that dir, (c) snapshot re-fetch.
- Detail pane replaces the single `Disk` row with the breakdown block (see design plan for the exact format). Owner vs sharer determination from `TargetDirIndex` (available because Step 6 already landed).

Verification:

- Fixture tests per design plan → **Testing → Phase 1.5 disk breakdown**.
- Regression test: a walker-change-only fixture that asserts `disk_usage_bytes` totals stay bit-identical to the pre-refactor value for every in-tree-target case.
- Manual: in-tree target, out-of-tree owner, out-of-tree sharer all render correctly.

### Step 6 — `TargetDirIndex` + per-worktree clean

See design plan → **Phase 2**.

Context:

- `src/tui/panes/actions.rs:101-109` (request_clean).
- `src/tui/input.rs:599-607` (project-list Clean action).
- `src/tui/render.rs:638-652` (status-bar shortcut gate).
- `src/tui/app/navigation.rs:289-296` (selected_item — the origin of the gating bug).

Work:

1. Create `src/tui/app/target_index.rs` with `TargetDirIndex` (forward + reverse maps), `TargetDirMember`, `MemberKind`.
2. Maintain the index from the `BackgroundMsg::CargoMetadata` handler: on every accept, `remove` the project from any stale bucket and `upsert` under the new target dir. Also call `remove` when scan drops a project from the set.
3. Add `build_clean_plan` as a free function. `CleanSelection` enum, `CleanPlan`, `CleanTarget`, `CleanMethod` — exactly as in the design plan. Selection exclusion for group cleans (per design plan).
4. Add `App::clean_selection`. Replace the three gating sites to route through it.
5. Update the confirm-dialog rendering: always list affected checkouts explicitly, cap at 5 with `+N more`, collapse nested (Submodule/Vendored) entries.
6. Implement the async clean-confirm re-fingerprint UX per design plan: dialog opens disabled with `Verifying target dir…` while metadata re-runs if the fingerprint drifted; transitions to the refreshed plan on arrival; on error shows Retry/Cancel only.

Verification:

- Test suite per design plan → **Testing → Phase 2 clean** (all variants, including selection exclusion, `CleanMethod` exhaustiveness, and the three clean-confirm states).
- Manual: worktree group with partial sharing, trigger clean from group and from a single worktree entry; confirm dialog copy matches design plan.

### Step 7 — Group-level fan-out clean

See design plan → **Group-level clean (worktree group header)**.

Context: same as Step 6; this step only extends `build_clean_plan` and the confirm dialog.

Work:

- Group-level selection path in `build_clean_plan` (dedupe, one `CleanTarget` per unique resolved dir, `covering_projects` listed per target).
- Deleted worktrees (directories missing on disk but listed by git) go into `CleanPlan.skipped` with reason `DeletedWorktree` without aborting the plan.
- Confirm dialog format from the design plan.

Verification:

- Fixtures from design plan → **Testing → Phase 2 clean**.
- Render-snapshot test for the group-clean confirm dialog: fixture with a 3-worktree group where 2 share and 1 is solo, asserting the exact dialog text matches the committed snapshot (catches copy regressions without eyeballing).
- Manual: run the scenario end-to-end against a real worktree group.

## Cross-cutting Rules

- **No silent panics.** Every `unwrap` / `expect` in new code must either be provably infallible at the call site (document the reason inline) or converted to error-returning code. In async handlers, errors become `Metadata errors` toasts (design plan).
- **No blocking on the event loop.** `cargo metadata` always dispatches via `BackgroundMsg`; the TUI event loop never blocks on an `.exec()` call. The clean-confirm "verifying" state is the one place the UI pauses on a metadata result, and it remains non-blocking from the event-loop's perspective.
- **LSP-first navigation.** Per repo CLAUDE.md, use LSP go-to-definition / find-references when landing these changes; grep is a fallback.
- **`cargo build && cargo +nightly fmt` after each step.** Per repo CLAUDE.md.
- **`cargo install --path .` after successful steps.** Per user memory.
- **Tests in the same PR.** No "will add tests later."

## Rollback Posture

Each step is independently shippable.

- Step 0 → `git revert` the PR; behavior is unchanged.
- Step 1 → `git revert` the PR. Snapshots stop being produced, `resolve_*` goes away, downstream callers never existed yet. No feature flag is wired; claiming otherwise would require `#[cfg]` gates on the phase-tracker field, the `BackgroundMsg` variant, the dispatch site, and the watcher extensions — not worth it for plumbing this early. Revert is the tool.
- Step 2+ → `git revert` the PR; the fallback path still exists because it's the same branch the `None` case takes.

The retirement of hand-rolled parsing in Step 3 is the only irreversible-by-revert step: after it lands, `resolve_metadata` returning `None` means "Loading…" placeholders, not hand-rolled values. If Step 3 needs rollback, restore `src/project/cargo.rs` from git history.

## Open Work Not Covered Here

- Precise per-worktree clean inside a shared target dir: explicitly rejected in design plan. If the policy changes later, a new PR reopens the conversation; not this plan's problem.
- README / help-copy updates for `CARGO_TARGET_DIR` leak and shared-target semantics: follow-up, not a blocker for any step above.
