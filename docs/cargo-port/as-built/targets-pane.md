# Targets, binary search, and owned runs

The Targets pane lists the selected project's binaries, examples, and benches.
It launches one application-owned run and sends that run's output to Output.
It does not inspect unrelated running Cargo processes.

Paths below are relative to `crates/cargo-port/src/`.

## Entries and availability

`tui/panes/targets/data.rs` defines `TargetsData`, `TargetEntry`, `RunTargetKind`, and `TargetSource`.
`TargetsData` stores separate binary, example, and bench vectors.
`build_target_list_from_data` flattens those vectors in that order.
The title, table, copying, and launch path use the same ordering.

`TargetsData::from_workspace_metadata(&WorkspaceMetadata, &AbsolutePath) -> TargetsData` enumerates the selected packages.
Selecting a checkout root includes every workspace package.
Selecting a member includes only that member.
Every `TargetKind::Bin` becomes a Binary entry, regardless of its name.
`default-run` changes neither order nor markers.
Examples keep their directory categories; benches remain flat.
Rows sort by source kind, source label, and target name.

`TargetSourceKind::{WorkspaceRoot, Member, Worktree}` determines source order.
Root and member rows show their package names.
A checkout group aggregates available checkout targets and labels them with checkout and package names.
Metadata absence produces empty target data without a hand-parsed manifest fallback.

`TargetsPane::available_content(&ProjectList) -> Option<TargetsData>` filters entries whose owners are Deleted or Dismissed.
`has_available_targets` supplies run visibility and navigation eligibility.
A workspace selection retains targets from live owners while it removes unavailable owners.
The visible cursor stays within the filtered list.
An empty or hidden pane supplies no active target actions or row hit areas.

## Table geometry

`tui/panes/targets/render.rs` defines:

```rust
fn render_targets_pane_body(
    frame: &mut Frame,
    area: Rect,
    pane: &mut TargetsPane,
    data: &TargetsData,
    mode: ColorMode,
) -> PaneFrameChrome;
```

The renderer records a rectangle for each visible row.
Hit testing uses those rectangles.
The viewport and `TableState` share the scroll offset.
The bottom border shows the drawn page.
`TargetsData::inner_rows()` returns the header plus actual entry rows, or zero for empty data.

`compute_layout` reserves Kind and column gaps, then measures the whole target names.
Source receives the remaining budget only when its whole header also fits.
Otherwise Source is hidden and Target receives its cells.
The Source header never draws as `S…`.
Source values can shorten with `…` while the header stays whole.
Distinct path fitting prevents equal-looking checkout labels after a cut.
Target names shorten only after the available layout cannot preserve them.

`tui/panes/layout.rs` reserves preferred target width before unused Projects space grows.
The layout does not take cells from an already-shown project name.
At narrow widths, Targets can be hidden and leave the Tab order.

## Binary identity across features

`project/cargo/rust_info.rs` stores sorted `binaries: Vec<String>` and `LibraryTargetKind` on `Cargo`.
`Cargo::types()` derives Binary from the names list.
There is no separate boolean that can disagree with the list.

`tui/finder/index.rs` adds one `FinderKind::Binary` result for each binary name.
Selecting that result focuses Targets and selects that binary's row.
`tui/panes/pane_data/package_data.rs` adds `cargo.binaries().len()` to `StructureCounts`.
Structure and Targets therefore count the same binaries for package and workspace selections.
The Structure label remains `binary` for all counts.

## Launch and output

1. Select the available entry for the current viewport row.
2. Build `PendingExampleRun` with its package, target, features, and build mode.
3. Submit it to the single owned-run slot in `Inflight`.
4. Start the queued child after the frame draws.
5. Verify the child's process identity before enabling termination.
6. Route output and progress events by `OwnedRunId`.

`Enter` selects debug mode; `r` selects release mode.
`tui/terminal/processes.rs::cargo_command_for_owned_run` constructs the command.
Binaries use `cargo run --bin <name>`.
Examples use `cargo run --example <name>`.
Benches use `cargo bench --bench <name>`.
Package, feature, release, and color arguments come from the selected entry and mode.

One `OwnedRun` slot prevents concurrent owned runs.
Its nonzero identifiers do not reset or wrap.
Output retains completed lines and replaces progress lines that use carriage returns.
An event for another run cannot update the current run.
Owned output replaces the bottom Lints and CI row while that presentation is active.

Esc first respects modal and focused-text-input handling.
With an application overlay open, Esc closes that overlay and keeps the child running.
Otherwise it exits an Output selection, requests termination, or clears retained output as applicable.
A cleared Output pane returns focus to Targets when needed.

## Process identity and termination

`tui_pane` defines `ProcessIdentity` and `VerifiedProcessIdentity` in `crates/tui_pane/src/process_identity.rs`.
Identity combines a PID with an operating-system creation token.
Linux uses start ticks, Windows uses FILETIME, and macOS uses the process start token.
The macOS observation also checks process start time around the token query.

`observe_current_process_identity` verifies the new child.
`OwnedRunProcessActor` in `tui/state/owned_run_process_actor.rs` owns the child and its termination capability.
It serializes reaping and signaling on one worker.
Before each signal, it revalidates the identity.
PID reuse therefore cannot authorize a signal to another process.

On Unix the child has its own process group.
The actor requests group termination and rechecks identity before a single-PID fallback.
The actor polls only its own child, using the 25 ms reap interval.
It never enumerates the process table.

`OwnedRunTerminationToken` carries authority without exposing a PID to the application lifecycle.
A sent termination enters Stopping.
A refused, failed, replaced, or already-reaped request receives its named outcome.
The lifecycle reconciles that outcome instead of treating submission as a completed signal.

## Invariants and gotchas

1. List every binary by its Cargo target name.
2. Use the same available entries for drawing, navigation, copying, and launch.
3. Pass `--bin` even when a binary name equals its package name.
4. Preserve target names before allocating Source value width.
5. Keep Source whole or hide the entire column.
6. Revalidate process identity immediately before signaling.
7. Keep one owned run and ignore stale run identifiers.

The Targets body renderer receives filtered data explicitly.
A raw `TargetsData::has_targets` check alone does not prove that a current owner remains available.
Shared process monitoring belongs to cargo-tile, not this pane.
