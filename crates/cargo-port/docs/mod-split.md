# Splitting the three remaining god-files in `src/tui/app/`

## Why a plan first

A previous attempt split `async_tasks.rs` by skim-grouping into 3 sibling
files. The result was three new files in the 600–1500 line range — i.e.
we'd have replaced one god-file with three smaller god-files. That's
not a refactor, that's redistribution.

This plan fixes the organizing principle, then applies it to all three
remaining god-files: `async_tasks.rs` (2898 lines, 105 fns),
`navigation.rs` (1404 lines, 51 fns), `query.rs` (995 lines, 66 fns).

## Organizing principles

1. **Each god-file becomes a directory.** `async_tasks.rs` →
   `async_tasks/mod.rs` plus peer files. The directory name preserves
   how callers think about the cluster ("the async-tasks subsystem of
   `App`"), and the peers are private to that directory by default.
2. **One responsibility per peer, describable in ≤6 words.** If you
   can't name the peer's job in one short phrase ("disk-usage handlers",
   "expand/collapse", "config & keymap reload"), the seam is wrong —
   regroup.
3. **Target peer size 100–400 lines.** No new file > 500 lines without
   a hard reason. Subdivide further if you blow past it.
4. **Inbound vs outbound separation.** Methods that *consume* background
   messages live in `*_handlers.rs` files. Methods that *schedule* or
   *produce* outbound work (lint registration, repo fetch spawning, CI
   priority fetches) live in their own peers and are not mixed with
   handlers.
5. **Preserve `impl App` everywhere.** Every peer opens its own
   `impl App { ... }` block on the same struct — nothing changes about
   how callers reach these methods. The split is purely about which
   file the body lives in.
6. **No public API change.** Visibilities (`pub`, `pub(super)`, etc.)
   preserved exactly. No new re-exports, no renames, no signature
   tweaks. If a private helper was only used by one cluster of methods,
   move it into that peer; if it's used across peers, move it to
   `mod.rs` of the directory.
7. **No `mod tests` blocks to relocate.** None of the three god-files
   contain a `#[cfg(test)] mod tests { ... }` block — only a few
   `#[cfg(test)]` annotations on individual fns (`apply_tree_build`,
   `startup_lint_toast_body_for`, the inner cfg-test inside
   `spawn_service_retry`, `toasts_is_alive_for_test`). Each travels
   with its enclosing fn into the destination peer; no separate test
   relocation step needed.

## How peer counts were chosen

For each god-file, every method was placed against the App field(s) it
mutates or reads, and methods touching the same field cluster were
grouped. Peers fall out of those clusters. Peers that ended up with
fewer than 4 fns were merged into a neighboring peer if the merge
preserved a single short responsibility.

---

## Phase A — `src/tui/app/async_tasks.rs` → `src/tui/app/async_tasks/`

### Proposed peer layout (13 peers)

| Peer file              | Responsibility (≤6 words)              | Fns | Est. lines |
| ---------------------- | -------------------------------------- | --- | ---------- |
| `mod.rs`               | declares peers; private shared structs | 0   | ~30        |
| `config.rs`            | config & keymap reload                 | 11  | ~240       |
| `lint_runtime.rs`      | lint registration & runtime sync       | 13  | ~330       |
| `background_services.rs` | scheduling background services for tree | 5   | ~200       |
| `startup_phase/tracker.rs`     | phase-completion state mutations  | 12  | ~392       |
| `startup_phase/toast_bodies.rs` | phase-toast body formatters       | 7   | ~113       |
| `startup_phase/mod.rs`         | startup_phase peer declarations   | 0   | ~10        |
| `running_toasts.rs`    | sync running-task toasts               | 5   | ~110       |
| `tree.rs`              | apply tree-build; legacy expansion     | 6   | ~150       |
| `poll.rs`              | poll loops (bg/ci/example/clean)       | 8   | ~190       |
| `dispatch.rs`          | dispatch BackgroundMsg to handlers     | 3   | ~200       |
| `disk_handlers.rs`     | disk-usage handlers                    | 6   | ~70        |
| `repo_handlers.rs`     | git/repo info handlers                 | 10  | ~340       |
| `service_handlers.rs`  | service-availability handlers          | 8   | ~150       |
| `lint_handlers.rs`     | lint-status / lint-cache handlers      | 5   | ~120       |
| `metadata_handlers.rs` | cargo-metadata & lang-stats handlers   | 6   | ~210       |
| `priority_fetch.rs`    | detail-pane priority fetch             | 2   | ~42        |

**Module-scope free items (verified against source):**

| Item                          | Kind   | Sole/primary caller             | Lands in              |
| ----------------------------- | ------ | ------------------------------- | --------------------- |
| `LegacyRootExpansion`         | struct | `capture/migrate_legacy_*`      | `tree.rs`             |
| `AvailabilityKind`            | enum   | `apply_unavailability` family   | `service_handlers.rs` |
| `service_unavailable_message` | const fn | `apply_unavailability` etc.   | `service_handlers.rs` |
| `service_recovered_message`   | const fn | `mark_service_recovered` etc. | `service_handlers.rs` |
| `collect_publishable_children` | fn    | `schedule_member_crates_io_fetches` (line 513) | `background_services.rs` |
| `workspace_member_roots`      | fn     | `accept_cargo_metadata` (line 2684) | `metadata_handlers.rs` |

The earlier draft listed `push_workspace`/`push_package_vendored` as
free fns under `priority_fetch.rs` — incorrect. They're inner fns
nested *inside* `collect_publishable_children` and travel with their
parent into `background_services.rs`.

Total ≈ 2890 lines, 109 fns (source has 108 `fn`/`const fn` decls plus
the `apply_tree_build` test variant; plan absorbs all of them across the
peers above).

### Method assignment

**`config.rs`** — `record_config_reload_failure`, `load_initial_keymap`,
`maybe_reload_keymap_from_disk`, `sync_keymap_stamp`,
`show_keymap_diagnostics`, `dismiss_keymap_diagnostics`,
`maybe_reload_config_from_disk`, `save_and_apply_config`, `apply_config`,
`apply_lint_config_change`, `refresh_lint_runtime_from_config`.

**`lint_runtime.rs`** — `respawn_watcher`, `register_existing_projects`,
`finish_watcher_registration_batch`,
`respawn_watcher_and_register_existing_projects`,
`refresh_lint_runs_from_disk`, `reload_lint_history`,
`refresh_lint_cache_usage_from_disk`, `lint_runtime_root_entries`,
`lint_runtime_projects`, `sync_lint_runtime_projects`,
`register_lint_for_root_items`, `register_lint_project_if_eligible`,
`register_lint_for_path`.

**`background_services.rs`** — `register_background_services_for_tree`,
`register_item_background_services`, `schedule_startup_project_details`,
`schedule_member_crates_io_fetches`, `schedule_git_first_commit_refreshes`,
plus the free fn `collect_publishable_children` (and its two nested
inner fns `push_workspace`, `push_package_vendored`).

**`startup_phase/tracker.rs`** — `initialize_startup_phase_tracker`,
`reset_startup_phase_state`, `start_startup_toast`,
`start_startup_detail_toasts`, `log_startup_phase_plan`,
`maybe_log_startup_phase_completions`,
`maybe_complete_startup_{disk,git,repo,metadata,lints,ready}`.

**`startup_phase/toast_bodies.rs`** —
`startup_{disk,git,metadata}_toast_body`, `tracked_items_for_startup`,
`startup_remaining_toast_body`, `startup_git_directory_for_path`,
`startup_lint_toast_body_for`.

(Pre-committed to the sub-split rather than gambling on the 500-line
threshold — phase 723–1227 is exactly 504 lines, right at the boundary.)

**`running_toasts.rs`** — `sync_running_clean_toast`,
`sync_running_lint_toast`, `sync_running_repo_fetch_toast`,
`running_items_for_toast`, `sync_running_toast`.

**`tree.rs`** — `apply_tree_build` *(`#[cfg(test)]`-only — production
code path uses the inline tree-apply branch in `handle_scan_result`;
keep these in sync via a comment in `tree.rs`)*,
`capture_legacy_root_expansions`, `migrate_legacy_root_expansions`,
`rebuild_visible_rows_now`, `rescan`, `refresh_derived_state` *(line
1334; one-liner that bumps scan generation, called from `apply_config`
and `poll_background`)*, plus the private `LegacyRootExpansion` struct.

**`poll.rs`** — `poll_background`, `log_saturated_background_batch`,
`record_background_msg_kind` *(associated `const fn`, called only by
`poll_background`)*, `poll_ci_fetches`, `poll_example_msgs`,
`apply_example_progress`, `finish_example_run`, `poll_clean_msgs`.

**`dispatch.rs`** — `update_generations_for_msg`, `handle_scan_result`,
`handle_bg_msg`. (The dispatch layer that fans out to the typed
`*_handlers.rs` peers below.) Dependency note: `handle_bg_msg`'s
`BackgroundMsg::CiRuns` arm calls `self.insert_ci_runs` which lives
outside `async_tasks/`; `BackgroundMsg::Submodules` is handled inline
with no helper. Both stay as-is — this peer is the boundary against
the rest of `App`.

**`disk_handlers.rs`** — `handle_disk_usage`, `handle_disk_usage_batch`,
`handle_disk_usage_msg`, `handle_disk_usage_batch_msg`,
`apply_disk_usage_breakdown`, `apply_disk_usage`.

**`repo_handlers.rs`** — `spawn_repo_fetch_for_git_info`,
`handle_checkout_info`, `handle_repo_info`, `maybe_trigger_repo_fetch`,
`handle_git_first_commit`, `handle_repo_fetch_queued`,
`handle_repo_fetch_complete`, `handle_repo_meta`,
`handle_project_discovered`, `handle_project_refreshed`.

**`service_handlers.rs`** — `apply_service_signal`,
`handle_service_reachable`, `apply_unavailability`,
`availability_for` (line 2115; the `const fn` method that returns
`&mut ServiceAvailability`, called from `apply_unavailability`,
`push_service_unavailable_toast`, `mark_service_recovered`, and
`handle_service_reachable`), `push_service_unavailable_toast`,
`spawn_service_retry`, `spawn_rate_limit_prime`,
`mark_service_recovered`, plus the `AvailabilityKind` enum and the
const fns `service_unavailable_message` and
`service_recovered_message`.

**`lint_handlers.rs`** — `handle_crates_io_version_msg`,
`handle_lint_startup_status_msg`, `maybe_complete_startup_lint_cache`,
`handle_lint_status_msg`, `handle_lint_cache_pruned`.

**`metadata_handlers.rs`** — `handle_out_of_tree_target_size`,
`handle_language_stats_batch`, `handle_cargo_metadata_msg`,
`accept_cargo_metadata`, `apply_cargo_fields_from_workspace_metadata`,
plus the free fn `workspace_member_roots`.

**`priority_fetch.rs`** — `detail_path_is_affected`,
`maybe_priority_fetch`. (No free fns; ~42 actual lines, not the
earlier ~80 estimate.)

### Risks

- `LegacyRootExpansion` struct is private and only used by `tree.rs`
  methods — keep it inside `tree.rs`, not `mod.rs`.
- `bg_dispatch::handle_bg_msg` is a giant `match` that calls into every
  `*_handlers.rs` peer. That's expected — it's the dispatch table —
  and each match arm is one or two lines.
- `startup_phase/` is pre-committed to the sub-split (`tracker.rs` ~392
  lines / `toast_bodies.rs` ~113 lines) — no conditional. The earlier
  draft had a 500-line trigger for the sub-split; that's stale.

---

## Phase B — `src/tui/app/navigation.rs` → `src/tui/app/navigation/`

### Proposed peer layout (7 peers)

| Peer file          | Responsibility (≤6 words)             | Fns | Est. lines |
| ------------------ | ------------------------------------- | --- | ---------- |
| `mod.rs`           | declares peers                        | 0   | ~20        |
| `cache.rs`         | viewport cache ensure-fns             | 5   | ~80        |
| `pane_data.rs`     | resolve detail-pane targets           | 6   | ~205       |
| `selection.rs`     | selected row/item/path queries        | 11  | ~398       |
| `worktree_paths.rs` | worktree path resolution              | 11  | ~250       |
| `expand.rs`        | expand/collapse a single row          | 8   | ~205       |
| `movement.rs`      | cursor movement (up/down/top/bottom)  | 5   | ~75        |
| `bulk.rs`          | bulk expand/collapse + path-select    | 6   | ~225       |

Total ≈ 1408 lines, 51 fns. (`selection.rs` carries most of the file's
mass — 398 of 1404 source lines — including the long display-path
match arms; still under the 500-line peer cap.)

### Method assignment

**`cache.rs`** — `ensure_visible_rows_cached`, `visible_rows`,
`ensure_fit_widths_cached`, `ensure_disk_cache`, `ensure_detail_cached`.

**`pane_data.rs`** — `build_selected_pane_data`, `resolve_member`,
`resolve_vendored`, `worktree_member_ref`, `worktree_vendored_ref`,
`build_worktree_detail`.

**`selection.rs`** — `selected_row`, `selected_item`, `clean_selection`,
`selected_project_path`, `path_for_row`, `member_path_ref`,
`vendored_path_ref`, `selected_display_path`, `display_path_for_row`,
`abs_path_for_row`, plus the free fn `worktree_group_selection`
(navigation.rs:1393, sole caller is `clean_selection` line 340).

**`worktree_paths.rs`** — `is_inline_group`, `is_worktree_inline_group`,
`worktree_display_path`, `worktree_member_display_path`,
`worktree_vendored_display_path`, `worktree_abs_path`,
`worktree_member_abs_path`, `worktree_vendored_abs_path`,
`worktree_path_ref`, `worktree_member_path_ref`,
`worktree_vendored_path_ref`.

**`expand.rs`** — `selected_is_expandable`, `expand_key_for_row`,
`expand`, `collapse_to`, `try_collapse`, `collapse`, `collapse_row`,
`row_count`.

**`movement.rs`** — `move_up`, `move_down`, `move_to_top`,
`move_to_bottom`, `collapse_anchor_row`.

**`bulk.rs`** — `expand_all`, `collapse_all`, `expand_path_in_tree`,
`row_matches_project_path`, `select_matching_visible_row`,
`select_project_in_tree`.

**Module-scope free items (verified):**

| Item                         | Sole/primary caller             | Lands in       |
| ---------------------------- | ------------------------------- | -------------- |
| `worktree_group_selection`   | `clean_selection` (line 340)    | `selection.rs` |

### Risks

- `is_inline_group` / `is_worktree_inline_group` could plausibly live in
  `selection.rs` or `worktree_paths.rs`. Picked the latter because
  callers are exclusively the worktree-path helpers.
- `row_count` is a one-liner that conceptually belongs with movement,
  but reads as a viewport query; placed with `expand.rs` because every
  expand/collapse path uses it for clamping.

---

## Phase C — `src/tui/app/query.rs` → `src/tui/app/query/`

### Proposed peer layout (8 peers)

| Peer file               | Responsibility (≤6 words)            | Fns | Est. lines |
| ----------------------- | ------------------------------------ | --- | ---------- |
| `mod.rs`                | declares peers                       | 0   | ~20        |
| `config_accessors.rs`   | config-flag accessors                | 8   | ~50        |
| `toasts.rs`             | toast/task lifecycle                 | 14  | ~150       |
| `post_selection.rs`     | post-selection-change side-effects   | 2   | ~50        |
| `ci_queries.rs`         | CI-state queries                     | 9   | ~100       |
| `git_repo_queries.rs`   | git/repo state queries               | 10  | ~200       |
| `disk.rs`               | disk-bytes formatting                | 2   | ~30        |
| `project_predicates.rs` | path/visibility predicates           | 7   | ~120       |
| `discovery_shimmer.rs`  | discovery-shimmer animation state    | 9   | ~150       |

Total ≈ 995 lines, 66 fns — matches source. (Earlier draft claimed a
drop "because test-only helpers move to `tests/`"; not true — source
has only one `#[cfg(test)]` fn (`toasts_is_alive_for_test`, ~2 lines)
and zero `mod tests` blocks. Nothing relocates.)

### Method assignment

**`config_accessors.rs`** *(renamed from `config.rs` to avoid grep
collision with `async_tasks/config.rs`)* — `lint_enabled`,
`invert_scroll`, `include_non_rust`, `ci_run_count`, `navigation_keys`,
`editor`, `terminal_command`, `terminal_command_configured`.

**`toasts.rs`** — `toast_timeout`, `active_toasts`, `focused_toast_id`,
`toasts_is_alive_for_test`, `prune_toasts`, `show_timed_toast`,
`show_timed_warning_toast`, `start_task_toast`, `finish_task_toast`,
`set_task_tracked_items`, `mark_tracked_item_completed`, `start_clean`,
`clean_spawn_failed`, `dismiss_toast`.

**`post_selection.rs`** *(renamed from `selection.rs` to avoid grep
collision with `navigation/selection.rs`)* — `sync_selected_project`,
`enter_action`. The pair of small post-selection-change side-effect
helpers that fire when the cursor moves and the detail subsystem
needs to react.

**`ci_queries.rs`** — `selected_ci_path`, `selected_ci_runs`,
`unpublished_ci_branch_name`, `ci_for`, `ci_data_for`, `ci_info_for`,
`ci_is_fetching`, `ci_is_exhausted`, `ci_for_item`.

**`git_repo_queries.rs`** — `git_info_for`, `repo_info_for`,
`primary_url_for`, `primary_ahead_behind_for`, `fetch_url_for`,
`git_status_for`, `git_status_for_item`, `git_sync`, `git_main`,
plus the free fn `worst_git_status` (line 985).

**`disk.rs`** — `formatted_disk`, `formatted_disk_for_item`.

**`project_predicates.rs`** — `is_deleted`, `selected_project_is_deleted`,
`is_rust_at_path`, `is_vendored_path`, `is_workspace_member_path`,
`unique_item_paths`, `prune_inactive_project_state`.

**`discovery_shimmer.rs`** — `animation_elapsed`,
`discovery_shimmer_enabled`, `discovery_shimmer_duration`,
`register_discovery_shimmer`, `prune_discovery_shimmers`,
`discovery_name_segments_for_path`, `discovery_shimmer_session_for_path`,
`discovery_shimmer_session_matches`, `discovery_scope_contains`,
`discovery_parent_row`, plus all module-scope free items used
exclusively by these methods: the `DiscoveryParentRow` struct, the
`impl DiscoveryRowKind { allows_parent_kind, discriminant }` block
(line 781), and the free fns `discovery_shimmer_window_len`
(line 740), `discovery_shimmer_step_millis` (line 750),
`discovery_shimmer_phase_offset`, `package_contains_path`,
`workspace_contains_path`, `root_item_scope_contains`,
`workspace_scope_contains`, `package_scope_contains`,
`root_item_parent_row`, `workspace_parent_row`,
`package_parent_row`. *(Verified at source: `package_contains_path`
and `workspace_contains_path` are only called by other shimmer-scope
helpers, so they live here, not in `project_predicates.rs`.)*

**Module-scope free items (verified):**

All query.rs module-scope free items are absorbed by their cluster:
`worst_git_status` → `git_repo_queries.rs`; everything else
(`DiscoveryParentRow` struct, the `impl DiscoveryRowKind` block,
and the free fns `discovery_shimmer_window_len`,
`discovery_shimmer_step_millis`, `discovery_shimmer_phase_offset`,
`package_contains_path`, `workspace_contains_path`,
`root_item_scope_contains`, `workspace_scope_contains`,
`package_scope_contains`, `root_item_parent_row`,
`workspace_parent_row`, `package_parent_row`) → `discovery_shimmer.rs`.

### Risks

- `animation_elapsed` is a one-liner accessor that's only used by
  shimmer code; placed in `discovery_shimmer.rs` to keep the cluster
  cohesive.
- `enter_action` is keyboard-binding-dispatch flavor, but reads the
  current selection and short-circuits per-row, so it stays adjacent to
  `sync_selected_project`.

---

## Migration order and gating

Phases run in this order, each as a single commit:

1. **Phase C** (smallest: 995 lines, 8 peers). Surfaces mechanical
   friction — directory-creation, peer-import-pruning, fmt fallout —
   on the lowest-blast-radius file.
2. **Phase B** (medium: 1404 lines, 7 peers). Apply the lessons from C.
3. **Phase A** (largest: 2898 lines, 14 peers). Tackle last on a
   warmed-up workflow.

Earlier draft ordered A→B→C "highest test coverage first"; second
review pointed out coverage applies after each phase regardless of
order, and warming up on the easy cases is more useful than landing
the hard one first.

Per-phase gating (mechanical, no new tests):

1. Create the directory; move `god_file.rs` into `god_file/mod.rs`.
2. Walk the `mod.rs` top-to-bottom, *cutting* methods into their target
   peer files in their original order — never copy. The original
   `mod.rs` shrinks as peers fill.
3. After each peer is filled: `cargo check --workspace --all-targets`.
   Fix import errors in the peer (the original file's import block is
   superset; each peer needs only its subset).
4. After all peers filled: `cargo nextest run --workspace`. Then
   `cargo +nightly fmt --all`.
5. Final `mod.rs` should be ~30 lines: `mod` declarations, no fns.

If any peer ends up > 500 lines after migration, sub-split *before*
moving on to the next phase — don't let one bloated peer survive into
the next phase's review.

## Judgment calls deferred

These came up in review but the plan keeps them as-is — flagging so
they're not invisible:

- **`priority_fetch.rs` (~42 lines, 2 fns) and `query/disk.rs` (~30
  lines, 2 fns) are smaller than the 100–400 line target.** Could
  fold `priority_fetch.rs` into `dispatch.rs` and `disk.rs` into
  `project_predicates.rs`. Keeping them separate because the
  responsibilities are sharply different from their would-be hosts —
  fewer files isn't the goal; clear seams are. Reconsider only if
  they stay this small after the migration.
- **`navigation/cache.rs` arguably doesn't belong in `navigation/`.**
  `ensure_disk_cache` and `ensure_detail_cached` are called primarily
  from `terminal.rs` / `interaction.rs` outside this directory.
  They're "viewport prep" rather than navigation per se. Moving them
  out of the directory entirely is a follow-up; this refactor only
  splits within the existing god-files.

## Out-of-scope (call them out, do not silently expand)

- No method renames.
- No visibility tightening (that was the prior pass; this is structural
  only).
- No re-organization of `App` fields or `App` constructor.
- No tests added or removed.
- No public API change visible to other modules in `tui/`.
