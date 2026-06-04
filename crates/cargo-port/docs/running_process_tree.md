# Running list: CPU smoothing + sub-process outline

Status: complete 2026-06-04 — all four phases shipped
Authored: 2026-06-03 — follow-on to `docs/sub_panes_and_running.md` (complete)
Revised: 2026-06-04 — Phase 4 widened the outline from same-target wrapper
nesting to **full descendant subtrees**: every process a tracked instance
spawns (tracked or not — `cargo`, `rustc`, wrappers) joins the outline.

## Goal

Two refinements to the Running sub-pane, both driven by watching `cargo mend`
runs in the live list:

1. **Steady CPU readings.** The CPU column oscillates (4% ↔ 23% for
   cargo-port) because both the sampler and the sampled process run ~1 s
   cycles: `process.cpu_usage()` is the average over the window between two
   poller refreshes, and how much of a work burst lands inside a given window
   varies with phase alignment. Show a moving average over the last N polls
   instead of the instantaneous sample.
2. **Sub-process outline.** One `cargo mend` run is several OS processes (an
   orchestrator plus one `RUSTC_WORKSPACE_WRAPPER` re-invocation per workspace
   target, joined through an untracked `cargo` intermediate). Today they render
   as N identical sibling rows. Nest children under their parent with the
   project list's expand/collapse idiom, collapsed by default, with the
   collapsed parent row showing the subtree's aggregate CPU/MEM.

## Data sources

- **ppid.** sysinfo populates `Process::parent()` from `libc::proc_bsdinfo`
  during the base per-process fetch — the same call that provides
  `start_time()` for the kill guard. It is present on every refresh regardless
  of `ProcessRefreshKind` flags, so the tree costs no new syscalls.
- **Start-time ordering as the reuse guard.** A parent precedes its child, so
  any hop in the ancestor walk whose parent started *after* its child is a
  reused PID — treat the row as top-level.

## Target behaviour

### CPU column

The CPU cell shows the mean of the instance's last `N = 5` poll samples (at
the 1 s poll cadence: the average over the last 5 s). A new instance averages
over however many samples exist, so its first reading is the raw sample, not
a zero-diluted mean. Memory stays instantaneous — it is a level, not a rate,
and does not alias.

### Outline

*(As revised by Phase 4.)* The outline shows every process a tracked
instance spawned. A process joins when its ancestor chain — walked through
`parent()` links, start-time-validated per hop against PID reuse,
depth-capped — reaches a tracked instance; its outline parent is its direct
OS parent. Top level is what was started independently. The `cargo` group's
count-prefix invariant holds because segment membership follows the subtree
root: installed roots sort first and their subtrees stay contiguous.

Rows render in tree order — children directly under their parent,
depth-indented with a `└` marker:

```
├─ Running (6) ──────────────────────────────────────────────────────┤
│ Target          Profile  PID     CPU   MEM       Path              │
│ ▼ cargo (6)                                                        │
│ cargo-port      cargo     4821    6%   312 MiB   ~/rust/cargo-port │
│ ▶ cargo-mend (4) cargo    5120   64%   2.1 GiB   ~/rust/cargo-mend │
│ my_app          debug     6233   12%   201 MiB   ~/rust/my_app     │
```

Expanded (`▼` on the parent), the parent row shows its **own** metrics and
each child shows its own:

```
│ ▼ cargo-mend (4) cargo    5120    2%   90 MiB    ~/rust/cargo-mend │
│   └ cargo-mend   cargo    5121   34%   801 MiB   ~/rust/cargo-mend │
│   └ cargo-mend   cargo    5122   28%   773 MiB   ~/rust/cargo-mend │
```

Behaviour rules:

- **Collapsed by default**, like the `cargo` group. Collapsed parent rows show
  the subtree-aggregate CPU/MEM and a `(N)` child count; expanded parents show
  their own metrics.
- **Toggle idiom matches the `cargo` group**: `Enter` on a parent row toggles;
  `Right` expands a collapsed parent; `Left` collapses — directly on the
  parent, or from a child row by handing the highlight to the parent.
- **Kill stays per-instance.** `K` on a parent row (collapsed or not) kills
  that one PID; hidden children are not killed implicitly.
- **The divider count is all instances**, hidden or not — same convention as
  the collapsed `cargo` group.
- **Orphaning is visible by design.** When a parent exits while children
  live, the OS reparents them to launchd: a tracked orphan pops out to top
  level; an untracked orphan's chain no longer reaches a tracked instance,
  so its row leaves the outline on the next poll.
- **Cross-target nesting is in** *(Phase 4 revision — previously deferred)*:
  a tracked target spawned by another tracked target nests under it; top
  level means started independently.

## Data model

- `RunningTargetsPoller` gains `cpu_history: HashMap<u32, VecDeque<f32>>`
  (capped at `CPU_SMOOTHING_WINDOW_POLLS = 5`), maintained exactly like
  `first_seen`: fed during the poll loop, retained against live PIDs, evicted
  by `drop_instances`.
- `RunningInstance` gains `parent_pid: Option<u32>` — the direct OS parent
  when the instance descends from another tracked instance (Phase 4
  semantics; Phases 2–3 shipped a same-key, same-profile restriction since
  deleted), resolved after the snapshot rebuild from the sysinfo table the
  poller already holds. The walk is a pure function over a
  `pid -> (parent, start_time)` lookup so tests fixture it with a map
  instead of a live process table.
- `RunningRow` gains `parent_pid` and a tree `depth`; `build_running_rows`
  reorders its sorted output into tree order (children after their parent,
  depth-first). Top-level rows keep today's ordering exactly — a row whose
  parent is absent from the list is top-level.
- `TargetsPane` gains `expanded_parents: HashSet<u32>` (empty = all
  collapsed); `build_running_list` skips descendants of collapsed parents.
  Stale PIDs are retained out each frame so a reused PID starts collapsed.

## Phases

Each phase ends with `cargo build && cargo +nightly fmt`, `cargo nextest run`,
`cargo mend`, and `cargo install --path .`.

### Phase 1 — CPU moving average ✅ complete (2026-06-03)

Poller-only: the history map, the mean fed into `RunningInstance.cpu_percent`,
retention + eviction, unit tests (mean, window cap, eviction). No render
change — the CPU cell already formats `cpu_percent`.

Shipped in `src/tui/running_targets.rs`: `CPU_SMOOTHING_WINDOW_POLLS`,
`cpu_history` on the poller, `smoothed_cpu` (a free function so the tick
loop's disjoint field borrows hold). The installed-bin branch feeds one
sample per OS process before the multi-project attribution loop.

### Phase 2 — parent resolution + tree-ordered rows ✅ complete (2026-06-03)

`parent_pid` on instance and row, the ancestor walk (pure, fixtured tests:
through-an-intermediate, reuse-rejection via start-time, depth cap,
self-parent), tree ordering with `depth`, and the indented `└` rendering.
Children are always visible in this phase — the outline ships expanded.

Shipped: `ParentLink` + `nearest_tracked_ancestor` + `resolve_parent_links`
(`running_targets.rs`), `tree_ordered`/`append_subtree`/`indented_name`
(`running_subpane.rs`). `tree_ordered` drains stranded rows defensively even
though the start-time-validated walk makes a parent-link cycle unreachable.

### Phase 3 — expand/collapse + aggregate metrics ✅ complete (2026-06-03)

`expanded_parents` on `TargetsPane`, collapsed-by-default list filtering, the
`▶/▼ name (N)` parent row with subtree-aggregate CPU/MEM while collapsed, the
three toggle paths (`Enter`/`Right`/`Left`) wired beside the `cargo` group's
in `actions.rs`, cursor handoff on collapse-from-child, and the PID-anchor
interaction (an anchor hidden by a collapse falls to the parent row).

Shipped: `visible_indices`/`outline_subtree_len`/`outline_name`/
`displayed_metrics` (`running_subpane.rs`), the `expanded_parents` accessors
on `TargetsPane`, and `toggle_running_parent`/`expand_running_parent`/
`collapse_running_parent` (`actions.rs`). Deviations from the plan text:

- The `(N)` subtree count renders in **both** states (the expanded mock above
  already showed it), counting the whole subtree, not direct children only.
- `Left` runs innermost-first: outline parent before the `cargo` group, so a
  wrapper row collapses its orchestrator before a second `Left` folds the
  group.
- `TargetsPane::new()` lost `const` — `HashSet::new()` is not const; the only
  caller (`Panes::new`) is non-const.
- No anchor self-heal case remained: expansion state changes only through the
  toggle paths, and the collapse-from-child path hands the highlight (and
  anchor) to the parent explicitly.

### Phase 4 — full descendant subtrees ✅ complete (2026-06-04)

Phases 2–3 nested only same-target, same-profile wrappers; the revision shows
**everything a tracked instance spawned**. The model becomes uniform:

- **Shown set** = tracked instances ∪ every process whose ancestor chain
  reaches one. Top level is what was started independently; a tracked target
  spawned *by* another tracked target nests under it (the lint-runtime's
  `cargo mend` runs nest under cargo-port; a terminal-launched `cargo mend`
  is its own top-level root).
- **Outline parent = direct OS parent.** "Descendant of tracked" is closed
  upward, so a descendant's direct parent is always itself shown — no
  nearest-shown walk needed beyond the reaches-tracked membership check
  (`shown_parent` = `nearest_tracked_ancestor(pid)?` then `links(pid).parent`).
  The same-profile restriction is deleted.
- **`ChildProcess`** (`running_targets.rs`): name (exe file name, free with
  our `with_exe` refresh), smoothed CPU, memory, `first_seen`, `create_time`,
  direct parent. Collected in a second tick pass over the process table;
  `first_seen`/`cpu_history` retention and `drop_instances` cover child PIDs.
- **`RunningRowKind`** (`running_subpane.rs`): `Target { profile,
  display_path }` or `Child` — child rows render blank Profile/Path cells,
  and the kill confirm labels them `name (process)`. `K` on any row kills
  that one PID; hidden children are never killed implicitly.
- **`cargo` segment membership moved to the subtree root**
  (`cargo_segment_len`): the header folds installed roots *with everything
  they spawned*, and its count is the folded row count, not the installed
  instance count. The divider's `Running (N)` likewise counts all shown rows.
- Collapsed-by-default now does the heavy lifting: during a mend run the
  whole compile reads as one `▶ cargo-port (N)` row whose aggregate CPU/MEM
  is the entire subtree.
- Orphaning stays visible by design: when an intermediate (`cargo`) exits,
  the OS reparents its children to launchd, their chains stop reaching a
  tracked PID, and the rows leave the outline on the next poll.
