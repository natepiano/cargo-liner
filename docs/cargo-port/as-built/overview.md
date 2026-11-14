# cargo-port overview

cargo-port is a terminal application for inspecting Rust projects and managing their local actions.
It uses the shared `tui_pane` framework.
The application stack includes Rust 2024, ratatui, crossterm, tokio, reqwest, and notify.

Paths below are relative to `crates/cargo-port/src/` unless they begin with `crates/` or `docs/`.

## Ownership and invariants

1. The port unit owns cargo-port enhancements, fixes, and its project documentation.
2. Shared framework changes belong in `tui_pane`, rather than application copies of framework behavior.
3. Coordinate shared-file ownership before changing another unit's active files.
4. Preserve the assigned owner of style-production findings.
5. Every test must complete in less than one second.
6. Keep a Deleted project visible and struck through until the user dismisses it or rescans.
7. Preserve Dismissed state through late background replies.
8. Keep state changes with their subsystem owner.

Process observation does not enumerate every process.
The owned-run path reads one PID and verifies its creation identity before signaling.
The CPU pane samples aggregate CPU data.
The backdrop feature is optional and is not enabled by cargo-port itself.

## Module map

| Location | Role |
| --- | --- |
| `main.rs` | Starts the application and configures the GNU/Linux allocator limit. |
| `config.rs` | Resolves configuration, validates values, and supplies active configuration. |
| `cache_paths.rs` | Resolves typed application, CI, and lint cache paths. |
| `channel.rs` | Supplies the crossbeam channel and selection types. |
| `ci.rs` | Defines GitHub runs, jobs, status, and repository identities. |
| `http/` | Owns authentication, requests, quota caches, and service state. |
| `lint/` | Owns lint workers, triggers, status, history, and cache reclamation. |
| `project/` | Owns roots, project entries, Cargo metadata, Git data, and visibility. |
| `scan/` | Discovers projects and schedules metadata and enrichment jobs. |
| `watcher/` | Registers subscriptions, filters events, and schedules refreshes. |
| `enrichment.rs` | Measures disk, source, language, and test data after discovery. |
| `process_observation/` | Measures disk reads of the current task. |
| `sccache.rs` | Runs sccache queries and parses their results. |
| `support.rs` | Supplies operating-system process helpers. |
| `themes/` | Defines application palettes and user-theme paths. |
| `test_support.rs` | Supplies private fixtures, Git seeds, and measurement helpers. |

## TUI map

| Location | Role |
| --- | --- |
| `tui/app/` | Coordinates startup, navigation, dismissal, and asynchronous results. |
| `tui/app/construct.rs` | Builds `App` with the selected startup services. |
| `tui/background.rs` | Owns channels and the watcher handle. |
| `tui/state/` | Owns Scan, Net, Lint, CI, configuration, and inflight action state. |
| `tui/project_list_state.rs` | Defines `ProjectList` and its derived caches. |
| `tui/project_list/` | Implements indexed lookup, grouping, expansion, visibility, and selection. |
| `tui/columns/` | Measures and fits Projects columns. |
| `tui/panes/` | Owns pane data, viewports, body renderers, and geometry. |
| `tui/render/mod.rs` | Draws panes, overlays, toasts, and the status line. |
| `tui/render_context.rs` | Supplies borrowed inputs to pane renderers. |
| `tui/app_render_state.rs` | Splits borrowed application state for rendering. |
| `tui/app/render_registry.rs` | Supplies the framework's pane registry. |
| `tui/terminal/` | Owns terminal setup, event processing, child commands, and tree persistence. |
| `tui/input/` | Routes keys and launches editors or terminals. |
| `tui/interaction.rs` | Routes clicks, focus, and hover interactions. |
| `tui/hit_test.rs` | Defines `HoverTarget` and hit-test order. |
| `tui/integration/` | Adapts application actions and panes to framework interfaces. |
| `tui/keymap/` | Loads defaults, migrations, overrides, and resolved actions. |
| `tui/keymap_ui/` | Supplies keymap editor policy. |
| `tui/overlays/` | Owns Finder and sccache visibility and return focus. |
| `tui/finder/` | Builds search results and dispatches selected results. |
| `tui/sccache/` | Owns summary polling and the statistics popup. |
| `tui/settings.rs` | Supplies application settings to the framework settings interface. |
| `tui/startup_services.rs` | Selects production services or quiet test substitutes. |
| `tui/messages.rs` | Defines asynchronous completion protocols. |
| `tui/theme_roles.rs` | Supplies application colors and the final selection pass. |
| `tui/dismiss_target.rs` | Distinguishes toast and Deleted-project dismissal. |
| `tui/test_support/` | Supplies deterministic application and drawn-buffer fixtures. |

## Startup and asynchronous data

1. Resolve configuration and scan roots before application construction.
2. Start streaming discovery and build `App` through `AppBuilder`.
3. Discover Rust projects, optional non-Rust repositories, workspace members, and linked checkouts.
4. Install discovered roots and derive `VisibleRow` values.
5. Accept generation-checked Cargo metadata and background enrichment results.
6. Rebuild selected detail data when its cache key changes.

`RootItem` orders roots by directory name and absolute path.
Members sort by package name.
`inline_dirs` controls member grouping.
An empty include list scans nothing; the current directory is not an implicit scan root.

`App` drains bounded message batches.
Disk, metadata, repository, lint, crates.io, and service handlers each update their own data through owner methods.
`Net` holds the HTTP client and repository request state.
`Scan` holds discovery generations and the metadata store.
`Panes` holds viewports and the detail cache stamp.
`Inflight` owns the clean and owned-run lifecycles.

A rejected Cargo manifest uses `CargoMetadataError::ManifestRejected`.
`ManifestMetadataStates` in `tui/app/manifest_metadata.rs` retains its accepted outcome across rescans.
`ProjectMetadataState::{NotLoaded, Available, Failed}` controls the row marker.
A failure shows one ` !` marker and the first Cargo error in Package.
It does not create a rejected-manifest toast on every scan.
Launch, I/O, and timeout failures retain their separate error handling.

## Configuration, themes, and tests

`CARGO_PORT_CONFIG_DIR` overrides the configuration directory.
An empty override is an error.
Otherwise the application uses the platform configuration directory plus `cargo-port`.
The same root supplies configuration, keymap, and theme files.
Missing configuration receives defaults.
User themes replace matching built-in names without requiring a seeded themes directory.

File-only scan, watcher, HTTP, and lint settings control operational costs.
Appearance settings select themes, tint, transparency, and framework toast settings.
`crates/cargo-port/tests/assets/default-config.toml` and `crates/cargo-port/tests/assets/default-keymap.toml` record the shipped defaults.

Application fixtures disable unrequested startup services.
Use explicit startup fixtures for tests that need watcher, HTTP, lint, or CPU work.
Theme overrides restore through a Drop fixture.
Tests use private temporary directories and shared Git seeds.
A test must not depend on the developer's Cargo home or configuration.

For a live check, use a managed build and a private scratch fixture.
Set an absolute scratch `XDG_CONFIG_HOME` and a private cache root.
Remove an inherited `CARGO_PORT_CONFIG_DIR` override.
Scan only the fixture project parent.
Give the application terminal input from `/dev/tty` when a Python script uses standard input.

For an idle CPU measurement, finish startup before sampling.
Require ten consecutive one-second samples below 5% of one core before each 60-second window.
Treat failure to reach that state within five minutes as a failed measurement.
Count any later background work in the measurement.

## Feature references

| Area | Contract |
| --- | --- |
| Watches and deletion | [watcher.md](watcher.md) |
| Frames and invalidation | [rendering.md](rendering.md) |
| Workers and memory | [background-work.md](background-work.md) |
| Project rows and columns | [project-list.md](project-list.md) |
| Layout, colors, and overlays | [presentation.md](presentation.md) |
| Package values and statistics | [package-pane.md](package-pane.md) |
| Git data and quota resets | [git-pane.md](git-pane.md) |
| CI rows and titles | [ci-pane.md](ci-pane.md) |
| Targets, Finder binaries, and owned runs | [targets-pane.md](targets-pane.md) |
| Lint history and blocked runs | [lint-file-lock-spinner.md](lint-file-lock-spinner.md) |
| Shared frame contracts | [Framework presentation](../../tui_pane/as-built/pane-presentation.md) |
| Shared toast rules | [Toast scheduling and rendering](../../tui_pane/as-built/toast-visual-deadline.md) |
| Mend safety | [Import fixes](../../cargo-mend/as-built/import-fixes.md) and [compiler validation](../../cargo-mend/as-built/compiler-validation.md) |
