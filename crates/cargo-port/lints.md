# Lints Roadmap

This is the execution plan for finishing `Lints`.

Already done:
- lint watching and summary history
- settings UI for projects, commands, and history budget
- retained-history accounting and pruning
- user-facing rename from `Port Report` to `Lints`

What remains is primarily about making lint history useful to inspect and act on.

## Milestone 1: Real Historical Artifacts

Outcome:
- each historical lint run has stable archived command output
- history rows no longer point only at rolling `*-latest.log` files
- budget pruning operates on complete historical units

Scope:
- archive per-run command logs keyed by run id and command
- store stable output paths in run metadata
- keep `latest` logs only as convenience pointers
- prune archived logs and matching metadata together

Definition of done:
- selecting an older run can still find its output
- history-budget enforcement removes the oldest complete retained runs first
- no stale references remain after pruning

Why first:
- drill-down is not very valuable without stable historical output
- this is the storage model the rest of the feature should build on

## Milestone 2: Open Command Output

Outcome:
- `Enter` on a historical run opens all its command outputs in the editor

Scope:
- add an `Enter` action on run history rows
- open each command's archived output in a separate editor tab (one open command per file)
- target the editor instance for the current project
- surface clear messages when output was pruned or never archived
- add a fast path for opening the latest failed command output
- improve retained-history UX:
  - show usage versus budget clearly
  - indicate when pruning occurred
  - indicate when live artifacts alone exceed the configured budget

Definition of done:
- a user can go from run summary to reading output in the editor with a single keypress
- users can understand why output is or is not available
- the budget feels observable rather than mysterious

Why second:
- once historical artifacts are stable, opening output becomes trustworthy
- this is the point where Lints becomes a daily debugging tool instead of a passive report

## Deferred

These should wait until the milestones above are complete:
- manual rerun/retry for the selected project
- history filtering by status
- history search
- export or copy command output
