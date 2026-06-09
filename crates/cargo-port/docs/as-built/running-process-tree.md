# Running Process Tree As Built

Status: implemented.

This document records the current Running-list CPU smoothing and process-outline
behavior. The feature is implemented in:

- `src/tui/running_targets/mod.rs`
- `src/tui/running_targets/app_tick.rs`
- `src/tui/panes/targets/running_subpane.rs`
- `src/tui/panes/actions.rs`
- `src/tui/panes/pane_impls.rs`

## Summary

The Targets pane's Running sub-pane shows one row per shown OS process:

- tracked Cargo targets that match known bin, example, or bench artifacts;
- installed binaries in the Cargo install bin directory, displayed with the
  `cargo` profile;
- untracked child processes whose parent chain reaches a tracked instance, such
  as `cargo` and `rustc` subprocesses spawned by `cargo mend`.

CPU values are smoothed with a five-sample rolling mean. Memory stays
instantaneous.

## CPU Smoothing

`RunningTargetsPoller` owns `cpu_history: HashMap<u32, RollingMean>`.
`RollingMean` is defined in `tui_pane::diagnostics` and caps its window at
`CPU_SMOOTHING_WINDOW_POLLS = 5`.

Each poll:

1. reads the latest `sysinfo` CPU sample for a PID;
2. pushes the sample into that PID's rolling window;
3. stores the mean in `RunningInstance.cpu_percent` or
   `ChildProcess.cpu_percent`;
4. retains CPU history only for PIDs still shown in the current snapshot;
5. evicts history explicitly when `drop_instances` removes a killed PID.

The first sample is the mean of one sample, not a zero-diluted value.

## Process Discovery

`App::running_targets_tick` builds `ProjectTargetSlice` values from the current
metadata cache and the visible Targets pane. Each slice carries:

- canonical workspace `target_directory`;
- workspace root fallback;
- known bench and binary target names;
- member manifest directories keyed by `(RunTargetKind, target name)`.

`RunningTargetsPoller::tick` refreshes process exe, cwd, CPU, and memory data.
It classifies running executables as:

- `target/debug/<bin>` or `target/release/<bin>`;
- `target/<profile>/examples/<example>`;
- `target/<profile>/deps/<bench>-<hash>`, when the bench name is known;
- an installed binary under the Cargo install bin directory, when its file stem
  matches a known bin target.

Relative executable paths are resolved against process cwd before matching.
Build-script and other unrecognized artifacts are ignored.

## Parent Links

The poller uses `Process::parent()` and `Process::start_time()` from the same
sysinfo process table. Parent walks are validated so PID reuse does not produce
bad outline links:

- the walk has a hard depth cap;
- self-parent links stop the walk;
- any parent whose start time is after its child is rejected as a reused PID;
- a chain that leaves the process table stops.

For untracked child processes, a process joins the outline when its ancestor
chain reaches any tracked PID. Its outline parent is its direct OS parent, which
is also shown because the full chain between the child and tracked root is kept.

For tracked target instances, current code is more conservative: a tracked
instance nests under a tracked ancestor only when the nearest tracked ancestor
has the same `RunningKey`. This keeps unrelated targets launched by an installed
`cargo-port` process visible as top-level rows instead of hiding them inside the
collapsed installed-`cargo` group.

## Row Ordering

`build_running_rows` creates `RunningRow` values from the poller snapshot.
Rows are ordered as follows:

1. tracked instances are collected;
2. duplicate installed-binary attributions are deduplicated by PID, keeping the
   lowest path attribution for stability;
3. installed `cargo` profile roots sort before other roots;
4. rows within those groups sort by `first_seen`, then PID;
5. child processes are appended and then the whole set is rewritten into tree
   order.

Tree order is depth-first. Children render directly below their parent and carry
their depth for indentation.

## Collapsing And Aggregates

`TargetsPane` owns `expanded_parents: HashSet<u32>`. Missing means collapsed,
so outline parents are collapsed by default.

`build_running_list` converts the full `RunningRow` list into the currently
navigable list:

- collapsed parents hide their whole subtree;
- expanded parents show their children;
- installed `cargo` roots and everything they spawned fold under a leading
  `cargo` header row controlled by `CargoGroup`;
- the `cargo` header has no PID and is not killable.

Collapsed parent rows display aggregate CPU and memory for their subtree.
Expanded parent rows display their own metrics. Child rows display their own
metrics and leave Profile and Path blank.

## Rendering

`render_running_subpane` draws:

1. a full-width divider titled `Running (N)` or `Running (i of N)`;
2. the column header;
3. the visible list window.

Columns are:

```text
Target | Profile | PID | CPU | MEM | Path
```

The Target column uses outline glyphs for parents and children. The target name
column reserves enough width for a future one-digit child-count suffix so fixed
metric columns do not jump when the first child appears. The Path column is
left-truncated with an ellipsis so the rightmost path segment remains visible.

## Kill Behavior

`resolve_kill_request` returns a request only for a `RunningListRow::Instance`.
Table rows and the `cargo` header return `None`.

The request carries:

- label, including profile for target rows or `(process)` for child rows;
- PID;
- create time.

The confirm body shows the label and `pid <pid> · started <age> ago`.
`RunningTargetsPoller::kill(pid, create_time)` refreshes that one PID and checks
the live process start time before sending `SIGTERM`, so a reused PID is skipped
instead of killed.

After a confirmed kill, `drop_instances` removes the PID from the current
snapshot and evicts first-seen and CPU-history entries.
