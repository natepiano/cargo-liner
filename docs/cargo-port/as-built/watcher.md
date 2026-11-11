# Project watches and deletion

cargo-port watches project sources and discovery paths without watching build output.
The watcher sends typed results to the application.
The project list owns the visible deletion state.

Paths below are relative to `crates/cargo-port/src/`.

## Registration and events

`watcher/runtime.rs` selects `WatchRegistration` once when it creates the watcher.
`watcher/roots.rs` stores the selection in `RegisteredRoots`.

| Backend | Registration |
| --- | --- |
| `RecursiveRoot` | macOS uses recursive roots through FSEvents. |
| `PerFolder` | Other platforms use individual folder watches. Linux uses inotify. |

Under `PerFolder`, `WatchedFolderRole` identifies `Project`, `Discovery`, `Candidate`, or `ConfigSource` subscriptions.
`Subscription::Unwatched` identifies a path without a subscription.
`Subscription::Watched(role)` identifies its subscribed role.
A folder without a project below it needs no general discovery watch.

A new folder below a Discovery subscription gets a Candidate subscription.
The project probe promotes that subscription or removes it.
A restored folder receives subscriptions for its known projects.
`complete_startup_registration` reconciles changes between discovery and registration before it replays buffered events.

Under `RecursiveRoot`, configured roots remain discovery boundaries.
Only successfully registered ancestors supply recursive coverage.
A failed ancestor registration therefore does not suppress a child's registration.
Overlapping roots do not need duplicate recursive subscriptions.
Create, remove, and rename events do not change recursive registrations.

`watcher/events.rs` routes events by their subscribed role.
Git events outside the checkout still reach Git refreshes.
These include linked-worktree Git files and the containing repository's configuration.

## Exclusions and disk measurements

`WatchSkipSet` in `watcher/skip.rs` owns exclusions.
It excludes `target/`, known Cargo target directories, and `.git/objects`.
`skips_resolved` checks canonical paths, including target-directory symlinks.
Both registration and event filtering apply these exclusions.

`replace_skip_set` removes obsolete individual subscriptions when target exclusions change.
Recursive registrations keep their roots and replace their event filter.
`ExclusionTransition` distinguishes the exclusion changes.
Cargo-home watches use the same exclusions.
`CargoHome::{Directory, Unavailable}` records the resolved Cargo home.
The runtime resolves it once and passes it to registration.

`watcher/refresh.rs` measures disk use independently of build-output events.
The default poll interval is 30 seconds.
`DiskProbeScope` selects `ProjectTree`, `TreeAndLinkedTarget`, or `TreeAndExternalTarget`.
A build can therefore change the Disk column up to one poll interval later.

## Deletion and dismissal

1. The watcher reports measurements and missing projects through background messages.
2. `tui/app/async_tasks/disk_handlers.rs` updates the project's measurement and visibility.
3. `Visibility::Deleted` keeps the row visible and permits dismissal.
4. Dismissal changes the state to `Visibility::Dismissed` and removes the visible row.
5. A full rescan clears the old roots and installs only discovered roots.

Deleted rows strike through their name and value text.
Their Disk cell supplies the dismissal control.
Late zero or positive disk replies do not restore a dismissed row.
Individual refreshes preserve relevant deletion states.
A full rescan deliberately does not preserve Deleted rows.

`tui/app/selection_paths.rs` records explicit selection changes.
A rescan restores the saved choice when it remains selectable.
Otherwise it selects the first visible row without saving that fallback as the user's choice.

## Invariants and gotchas

1. Subscribe once per required path and backend mode.
2. Derive recursive coverage from successful subscriptions.
3. Apply canonical target exclusions to aliases.
4. Keep Deleted rows until dismissal or rescan.
5. Preserve Dismissed state through late replies.
6. Retire removed projects from watcher maps and pending disk work.

A project created inside an existing unsubscribed folder needs a full rescan.
An external `.cargo/` folder deleted and recreated receives its subscription again at the next rescan.
Tests pass a private Cargo home instead of reading the developer's Cargo home.

The role model limits watch cost while preserving source, metadata, and Git refreshes.
The disk poll supplies build-output measurements without subscribing to large build trees.
