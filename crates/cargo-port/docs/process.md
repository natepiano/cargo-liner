# Detect running cargo targets + render them in the Targets pane

> **Resumption note**: This doc is the single source of truth for this work. It covers both the detection capability and the UI integration. A compacted session resuming from this doc should not need additional context.

## Context

The Targets pane (`src/tui/panes/targets.rs`) renders a 3-column table — Target / Source / Kind — of every runnable cargo target (binary, example, bench) for the currently selected project. The pane gives no indication today of whether a target is **currently executing** on the host.

This work delivers two layered changes:

1. **Detection capability** — a poller that maps "this target" → "is its process alive, and at which PID".
2. **UI integration** — running rows sort to the top of each kind section, and the target name is suffixed with a green ` (r)` marker. The marker survives name truncation.

The detection layer was specced in a prior iteration of this doc; the UI layer was specced in conversation after the table grew its Source column. Both are now folded into this single plan.

## Detection: reverse-direction matching on exe paths

For each running OS process, sysinfo returns the absolute exe path. We already know each loaded project's `target_directory` (cached in `MetadataStore`). The poller iterates processes once per tick, and for any exe under a known `target_directory`, parses the path tail to classify it as a bin / example / bench of that project.

This beats the alternatives:

- **Per-target candidate paths + glob for benches**: requires per-bench `read_dir` calls and is fragile across rebuilds.
- **Basename matching**: false positives across workspaces that share target names (`server`, `main`).
- **Shelling to `pgrep`/`ps`**: fork/exec storm and parsing fragility.

One sysinfo refresh per tick produces the full system-wide running set; pane render-time lookups are O(1) per row.

### Parse rules

For an exe path under `<target_dir>/` with the remainder split into segments:

| Tail segments | Classification |
|---|---|
| `debug`/`release`, `<name>` | bin named `<name>` |
| `debug`/`release`, `examples`, `<name>` | example named `<name>` |
| `debug`/`release`, `deps`, basename matching `^<name>-[0-9a-f]{16,}$` | bench candidate; keep only if `<name>` is in the project's known bench set |

Anything else under `target/` (test bins under `deps/`, `build/`, `incremental/`) is ignored. The bench-name set is the safety net against ambiguity (`my-bench-1234567890abcdef` could parse two ways; only the form whose `<name>` is a declared bench wins; if both are declared, longest valid name wins — pin this in the test).

Both the `target_dir` and the sysinfo-reported `exe_path` are canonicalized before the `starts_with` check. On macOS libproc returns the real exe inode; on Linux `/proc/<pid>/exe` resolves the symlink — canonicalizing both sides keeps the comparison stable regardless of platform or user-created symlinks under `target/`.

`cargo run` itself never matches — its exe is `~/.cargo/bin/cargo`, not under any project's `target/`. The child binary is what shows up under `target/`.

### Known limitations (document in user-facing release notes)

- **Cross-compile / `CARGO_TARGET_*_RUNNER`**: when a runner wraps the binary (e.g. `qemu-arm`, custom test runners), the OS-level exe is the runner, not the target — won't be detected.
- **Sandboxed children** (devcontainer, Docker, podman, remote SSH): processes outside the host's PID namespace are invisible to sysinfo.
- **Cross-uid processes**: sysinfo's `exe()` returns `None` when the OS denies the read. Targets the current user launched are always readable.
- **Snapshot staleness**: `is_running` reflects the last tick, so the indicator may be up to `poll_interval` stale.
- **Shared `target-dir` via `.cargo/config.toml`**: when multiple workspaces point at the same `[build] target-dir`, a single running binary matches against every workspace that claims that target_dir. The pane filters by its own target set, so this only causes false positives when two workspaces declare a target of the same name.

## Detection: module layout

### New file: `src/tui/running_targets.rs`

Mirrors the `CpuPoller` pattern from `src/tui/cpu.rs:71-130` — main-thread cadence poller, no background task. Visibility follows the `pub(super)` convention used by `CpuPoller`.

```rust
use std::collections::HashMap;
use std::collections::HashSet;
use std::time::Duration;
use std::time::Instant;

use sysinfo::Pid;

use crate::project::paths::AbsolutePath;
use crate::tui::panes::RunTargetKind;

pub(super) struct RunningTargetsPoller {
    system:        sysinfo::System,
    last_poll:     Option<Instant>,
    poll_interval: Duration,
    snapshot:      RunningTargets,
}

#[derive(Default)]
pub(super) struct RunningTargets {
    by_key: HashMap<RunningKey, Vec<Pid>>,
}

/// `kind` is one of the three runnable target kinds: Binary, Example, Bench.
/// Tests-as-binaries are not modeled at this layer.
/// `name` matches cargo's filesystem-level target name (case-sensitive on Unix).
#[derive(Hash, Eq, PartialEq, Clone)]
pub(super) struct RunningKey {
    pub target_dir: AbsolutePath,   // canonicalized workspace target_directory
    pub kind:       RunTargetKind,
    pub name:       String,
}

/// Temporary slice assembled immediately before `tick`. Do not cache across
/// renders or metadata refreshes — `bench_names` is borrowed from
/// `PackageRecord`s that may be replaced when cargo metadata refreshes.
pub(super) struct ProjectTargetSlice<'a> {
    pub target_dir:  &'a AbsolutePath,
    pub bench_names: &'a HashSet<String>,
}

impl RunningTargetsPoller {
    pub(super) fn new(poll_interval: Duration) -> Self { /* ... */ }

    /// Refresh if due. Always returns the current snapshot, so the pane has
    /// stable access regardless of cadence.
    pub(super) fn tick(
        &mut self,
        now: Instant,
        projects: &[ProjectTargetSlice<'_>],
    ) -> &RunningTargets;

    pub(super) fn snapshot(&self) -> &RunningTargets { &self.snapshot }
}

impl RunningTargets {
    pub(super) fn pids(&self, key: &RunningKey) -> &[Pid];
    pub(super) fn is_running(&self, key: &RunningKey) -> bool;
}
```

Refresh call: `ProcessRefreshKind::nothing().with_exe(UpdateKind::Always)` over `ProcessesToUpdate::All` (sysinfo 0.38.4). Verify the exact parameter order of `System::refresh_processes_specifics(update, remove_dead, refresh_kind)` at implementation time. Use a separate `sysinfo::System` from `CpuPoller`'s — the CPU instance is CPU-only-refresh (`src/tui/cpu.rs:80-82`) and conflating costs would change its profile.

### Owner: `Panes`, not `App`

Verified: `CpuPoller` is owned by `CpuPane` (`src/tui/panes/pane_impls.rs:125,140,148,154`) and ticked via `app.panes.cpu_tick(...)` from `src/tui/terminal.rs:260`. Follow the same pattern — but the running poller serves every pane that shows targets and isn't bound to one pane instance, so place it directly on `Panes` (next to `CpuPane`) with a public `running_targets_tick` paralleling `Panes::cpu_tick` at `src/tui/panes/system.rs:104`.

### Tick site

Add adjacent to the existing `app.panes.cpu_tick(Instant::now())` at `src/tui/terminal.rs:260`:

```rust
let slices = app.build_running_target_slices();
app.panes.running_targets_tick(now, &slices);
```

### Slice index + cache

New helper `App::build_running_target_slices` returns a borrowed `Vec<ProjectTargetSlice<'_>>` over a cached owned index. The owned index is rebuilt **only when cargo metadata arrives**, not every tick. Rebuild happens in `handle_cargo_metadata_msg` (`src/tui/app/async_tasks/dispatch.rs`).

Per-project rebuild:

- Project root → `app.scan.resolve_target_dir(root)` (`src/tui/state/scan.rs:120`) → workspace `target_directory`.
- Bench names come from `TargetsData::from_workspace_metadata` at `src/tui/panes/pane_data/mod.rs:681` — iterate the cached `PackageRecord`s for each workspace and collect bench `name`s into a `HashSet<String>`.
- Deduplicate by `target_dir`: workspace members share one target_dir; emit one entry per target_dir with the union of bench names across its members.
- Canonicalize each `target_dir` once during rebuild; store the canonicalized form on the owned slice. Per-tick path: zero syscalls outside sysinfo itself.

### `PaneRenderCtx` wiring

Add `running_targets: &'a RunningTargets` to `PaneRenderCtx` (`src/tui/pane/mod.rs`). Borrow chain: `App` → `Panes` (owns poller + snapshot) → `PaneRenderCtx` (borrows snapshot). Mutation happens only in `running_targets_tick`, which runs before any pane render call on the same frame.

### Configuration

Mirror `CpuConfig`. Add `RunningTargetsConfig { poll_interval: Duration }` (default 1s). The poller takes it via `RunningTargetsPoller::new(cfg.poll_interval)`. Reload follows the same path `CpuPoller::new(cfg)` uses at `pane_impls.rs:154`.

### Polling thread: sync (decided)

The poller runs synchronously on the render thread, mirroring `CpuPoller`. Profiling step 1 below is the kill-switch: if `tick` exceeds the ~20 ms budget in steady state, or if any visible frame glitching appears during the manual smoke test, migrate to a background task — new `BackgroundMsg::RunningTargets(RunningTargets)` variant in `src/scan/mod.rs`, poller spawned at construction, results consumed via the existing background-message channel handled in `src/tui/app/async_tasks/dispatch.rs`.

## UI integration: sort + `(r)` marker

### Current render path

- `build_target_list_from_data` (`src/tui/panes/pane_data/mod.rs:207`) currently does `entries.extend(data.binaries.iter().cloned()); ...examples; ...benches;` and returns the flat list. `TargetsData` (`pane_data/mod.rs:661-665`) holds three `Vec<TargetEntry>` fields (binaries / examples / benches).
- `TargetEntry` (`pane_data/mod.rs:178-184`): `{ name, display_name, kind, source }`. No running flag.
- `render_targets_pane_body` (`src/tui/panes/targets.rs:44`) → `compute_layout` → `build_rows` (`targets.rs:182-208`). The row's name cell is `Cell::from(format!(" {display}"))` where `display = truncate_with_ellipsis(&entry.display_name, layout.name_max, "…")`.
- Column layout: `Layout { kind, source, name_max }` (`targets.rs:69-73`). `name_max = content_width − kind − source − col_spacing*2 − leading_pad(1)`.
- Existing helper: `truncate_with_ellipsis(text, max_width, ellipsis)` at `src/tui/render.rs:532`.

### Rendering choice: render-time check, no field on `TargetEntry`

The running flag is computed at render time inside `targets.rs`, not persisted on `TargetEntry`. Reasons:

- Keeps `TargetEntry` purely metadata-derived; running state is volatile.
- Confines the change to `targets.rs` + `pane_data/mod.rs` (sort) + `render.rs` (helper).
- `build_target_list_from_data` already returns a fresh `Vec` per render — adding a sort pre-pass is cheap.

### Sort: running-first within each section

Change `build_target_list_from_data` to accept the running snapshot, and stable-sort each section running-first before extending. Signature:

```rust
pub fn build_target_list_from_data(
    data: &TargetsData,
    running_for: &dyn Fn(&TargetEntry) -> bool,
) -> Vec<TargetEntry>
```

Caller in `render_targets_with_data` constructs the closure once per render:

```rust
let target_dir = ctx.running_targets_dir_for(data); // helper that resolves
                                                    // the project's canonical
                                                    // target_dir; see below
let running_for = |e: &TargetEntry| match &target_dir {
    Some(dir) => ctx.running_targets.is_running(&RunningKey {
        target_dir: dir.clone(),
        kind:       e.kind,
        name:       e.name.clone(),
    }),
    None => false,
};
let entries = panes::build_target_list_from_data(data, &running_for);
```

`build_target_list_from_data` body:

```rust
let mut binaries = data.binaries.clone();
let mut examples = data.examples.clone();
let mut benches  = data.benches.clone();
let stable_running_first = |xs: &mut Vec<TargetEntry>| {
    xs.sort_by_key(|e| !running_for(e));   // false (running) sorts before true
};
stable_running_first(&mut binaries);
stable_running_first(&mut examples);
stable_running_first(&mut benches);
let mut entries = Vec::with_capacity(binaries.len() + examples.len() + benches.len());
entries.extend(binaries);
entries.extend(examples);
entries.extend(benches);
entries
```

**Cursor-jump caveat**: the pane's cursor is an index into the entry list. When a target starts/stops running, the sort order changes and the highlighted row jumps to whatever target now occupies that index. Acceptable for v1; if users report the wrong row becoming selected after a run/stop, switch to tracking the cursor by `(kind, name)` across renders. Note this in release notes.

### `(r)` marker rendering

The marker appears as a fixed 4-character suffix `" (r)"` styled with `tui_pane::success_color()` (the same green CPU and CI passing use). It is rendered **only when the row is running**.

The name cell becomes a `Line<'_>` with two `Span`s when running:

```rust
let cell = if entry_running {
    let suffix = " (r)";
    let (visible_name, suffix_text) =
        render::truncate_with_suffix(&entry.display_name, suffix, layout.name_max, "…");
    Cell::from(Line::from(vec![
        Span::raw(format!(" {visible_name}")),
        Span::styled(suffix_text, Style::default().fg(success_color())),
    ]))
} else {
    let display = render::truncate_with_ellipsis(&entry.display_name, layout.name_max, "…");
    Cell::from(format!(" {display}"))
};
```

### New helper: `truncate_with_suffix` in `src/tui/render.rs`

Adjacent to `truncate_with_ellipsis` at line 532. Sig:

```rust
/// Truncate `text` so that `text + suffix` fits within `max_width` columns.
///
/// Returns `(visible_name, suffix_text)`. The visible name has the ellipsis
/// inside it if truncation occurred; the suffix is returned separately so the
/// caller can style it.
///
/// Edge cases:
/// - If `max_width >= text.width() + suffix.width()`: returns
///   `(text.to_string(), suffix.to_string())` — nothing truncated.
/// - If `suffix.width() >= max_width`: returns
///   `(String::new(), truncate_with_ellipsis(suffix, max_width, ellipsis))` —
///   too narrow for the name; render only the (truncated) suffix.
/// - If `0 < max_width - suffix.width() < ellipsis.width()`: returns
///   `(String::new(), suffix.to_string())` — drop the name entirely; keep the
///   suffix verbatim.
/// - Otherwise: truncates `text` to `max_width - suffix.width()` using
///   `truncate_with_ellipsis`, returns `(truncated_text, suffix.to_string())`.
pub(super) fn truncate_with_suffix(
    text: &str,
    suffix: &str,
    max_width: usize,
    ellipsis: &str,
) -> (String, String)
```

Returning a tuple (visible name, suffix) lets the caller style the suffix as its own `Span`. Width math uses unicode display width (consistent with the existing `truncate_with_ellipsis` which uses `unicode_width::UnicodeWidthStr`).

### Resolving the project's `target_dir` at render time

`PaneRenderCtx` doesn't know which project the Targets pane is rendering. Two routes:

- **Pre-compute on `Panes::set_detail_data`**: when detail data is set for a project, also stash the resolved canonical `target_dir` (or `None`). `PaneRenderCtx` exposes it via a new field or accessor. Cleanest — single resolution per project switch.
- **Resolve per-render** in `render_targets_with_data`: call `app.scan.resolve_target_dir(...)` from the render path. Adds an `App` borrow to the ctx, which it doesn't currently have.

Route 1 is preferred. Implementation: extend `Panes::set_detail_data` (`src/tui/panes/system.rs:72-83`) to accept the resolved `target_dir`, store it next to `TargetsData`, and expose it on `PaneRenderCtx` as `running_targets_dir: Option<&'a AbsolutePath>`.

## Critical files (verified line numbers as of this writing)

| File | Purpose |
|---|---|
| `src/tui/cpu.rs:71-130` | Pattern source for cadence poller (`CpuPoller`) |
| `src/tui/panes/pane_impls.rs:125,140,148,154` | Pattern for placing a poller on Panes and ticking it; reload reconstructs the poller |
| `src/tui/panes/system.rs:72-83,104` | `Panes::set_detail_data` (extend with `target_dir`) and `Panes::cpu_tick` (add sibling `running_targets_tick`) |
| `src/tui/terminal.rs:260` | Tick site — `app.panes.cpu_tick(Instant::now())`; add `running_targets_tick` here |
| `src/tui/running_targets.rs` | New module |
| `src/tui/state/scan.rs:120` | `Scan::resolve_target_dir(path) -> Option<AbsolutePath>` — call via `app.scan.resolve_target_dir(...)` |
| `src/project/cargo/metadata_store.rs:65` | `MetadataStore::resolved_target_dir` — underlying lookup; must honor `.cargo/config.toml [build] target-dir` (Verification step 1) |
| `src/tui/panes/pane_data/mod.rs:178-184,207,661-665,681` | `TargetEntry` (`name, display_name, kind, source`); `build_target_list_from_data` (change signature); `TargetsData` ({binaries, examples, benches}); `from_workspace_metadata` (read-only reuse) |
| `src/tui/pane/mod.rs` | `PaneRenderCtx` — add `running_targets: &'a RunningTargets` and `running_targets_dir: Option<&'a AbsolutePath>` |
| `src/tui/panes/targets.rs:44,69-73,182-208` | `render_targets_pane_body`, `Layout`, `build_rows` — render `(r)` suffix; use `truncate_with_suffix` for running rows |
| `src/tui/render.rs:532` | `truncate_with_ellipsis` — add sibling `truncate_with_suffix` |
| `src/tui/app/async_tasks/dispatch.rs` | `handle_cargo_metadata_msg` — invalidate / rebuild the running-target slice cache |
| `src/scan/mod.rs` (fallback) | If profiling forces a switch to async polling: add `BackgroundMsg::RunningTargets(RunningTargets)` variant; handler in `dispatch.rs` |

## Verification

1. **`MetadataStore` honors `.cargo/config.toml [build] target-dir`.** Read `src/project/cargo/metadata_store.rs` and the cargo-metadata ingestion path; confirm the cached `target_directory` reflects an override when one is configured. If it doesn't, fix that first — every other check assumes it does.
2. **Cost profiling** (non-optional). Time `RunningTargetsPoller::tick` on a host with several hundred processes. Budget: 5–15 ms on macOS for ~400 processes. If sustained >20 ms, switch to the async path (see "Polling thread" decision above).
3. **Unit tests — parser** (in `running_targets.rs`):
   - `<target>/debug/foo` → bin `foo`
   - `<target>/release/examples/bar` → example `bar`
   - `<target>/debug/deps/baz-0123456789abcdef` with `baz` in bench set → bench `baz`
   - `<target>/debug/deps/baz-shorthash` → unrecognized (hash too short)
   - `<target>/debug/deps/other-0123456789abcdef` with `other` not in bench set → unrecognized
   - `<target>/debug/deps/my-bench-0123456789abcdef` with both `my` and `my-bench` declared as benches → match `my-bench` (longest valid name wins; document the tiebreak rule in the test)
4. **Unit tests — `truncate_with_suffix`** (in `render.rs`):
   - `truncate_with_suffix("hello", " (r)", 20, "…")` → `("hello", " (r)")`
   - `truncate_with_suffix("very_long_name", " (r)", 10, "…")` → `("very_…", " (r)")` (6 + 4 = 10)
   - `truncate_with_suffix("a", " (r)", 4, "…")` → `("", " (r)")` (suffix exactly fits)
   - `truncate_with_suffix("a", " (r)", 3, "…")` → `("", " (…")` (suffix doesn't fit, truncate the suffix)
   - `truncate_with_suffix("a", " (r)", 0, "…")` → `("", "")` (zero width)
5. **Unit tests — sort** (in `pane_data` or `targets`):
   - Given a `TargetsData` with binaries `["a", "b", "c"]` and `running_for` returning true for `b`: result is `b, a, c` followed by examples and benches in their own running-first order.
   - Empty running set: order matches the existing flat output.
   - All running: order matches the existing flat output (stable sort preserves alphabetical).
6. **Integration test (sysinfo backed)**: build a real fixture binary into `<fixture_target>/debug/`, spawn it via `Command::new(...)`, tick the poller, assert detection, kill, tick again, assert absence. Do **not** symlink `/bin/sleep` — SIP and platform-specific symlink-exe behavior make that fragile.
7. **Manual smoke test**:
   1. `cargo install --path .`, then `cargo-port` in this repo.
   2. From another terminal, `cargo run --example <some_example>` in a project cargo-port has loaded.
   3. Confirm the example's row appears at the top of the Examples section with ` (r)` in green.
   4. Ctrl-C the example, confirm the row falls back to its alphabetical position and the marker disappears (within ~1 second).
   5. Repeat for a bin and a bench (`cargo bench --no-run`, then run the binary in `target/release/deps/` directly).
   6. Resize the pane narrow enough that target names truncate; confirm the ` (r)` suffix stays visible at the right edge with the ellipsis inside the name.
   7. Watch for any visible frame glitch during the smoke run — if any, switch to the async polling path.
8. **Debug logging on `exe() == None`** so future "why isn't my target detected" reports have a trail.

## Implementation order (one push)

1. New module `src/tui/running_targets.rs` — types, poller, parser, unit tests for the parser.
2. `Panes` field + `running_targets_tick` method + reload path.
3. `App::build_running_target_slices` + slice cache + invalidation on `handle_cargo_metadata_msg`.
4. Tick site in `terminal.rs:260`.
5. `PaneRenderCtx` fields (`running_targets`, `running_targets_dir`).
6. `Panes::set_detail_data` carries the resolved `target_dir`.
7. `truncate_with_suffix` + unit tests in `render.rs`.
8. `build_target_list_from_data` signature change + sort + unit tests.
9. `build_rows` in `targets.rs` switches to two-span rendering when running.
10. Run the full verification list (steps 1–8 above).
