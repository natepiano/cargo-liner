# cargo-port — as-built overview

Read this map before changing cargo-port.

In every section, file paths are relative to crates/cargo-port/src unless they start with crates/ or docs/; those paths are relative to the repository. Runtime file locations are written as prose. Command blocks use absolute paths or paths created in that block.

## Module map

| Location | Job, types and entry points |
| --- | --- |
| `main.rs` | Declares the modules; `main` calls `tui::run`. |
| `cache_paths.rs` | App cache locations; `default_app_cache_root`, `ci_cache_root` and `lint_runs_root_for` return `AbsolutePath`. |
| `channel.rs` | Crossbeam exports `Sender`, `Receiver`, `Select` and `unbounded` for sources that wake the UI. |
| `ci.rs` | GitHub run data: `OwnerRepo`, `CiRun`, `CiJob`, `CiStatus`; `parse_owner_repo` and `build_ci_run`. |
| `config.rs` | `CargoPortConfig` and `CargoPortConfigurationPathResolution`; `config_path`, `CargoPortConfig::from_table`, `normalize_config`, `set_active_config`. |
| `constants.rs` | Shared names, defaults, concurrency limits and timing constants; no state or entry function. |
| `enrichment.rs` | Local disk, language, test-count and crates.io work after discovery; `enrich` and `spawn_language_scan`. |
| `http/` | `HttpClient`, `ServiceSignal`, `GitHubRateLimit`; `HttpClient::new` creates the authenticated client. Actions, PRs, crates.io, auth and quotas have separate files. |
| `lint/` | `LintRuns`, `LintStatus`, `RuntimeHandle`; `spawn`, `read_history`, `reclaim_project_cache`. Runtime, disk records, triggers and cache reclamation live here. |
| `process_observation/` | `ProcessIdentity` and `VerifiedProcessIdentity`; `observe_current_process_identity` and `revalidate_strong_process_identity` bind signals to one process lifetime. |
| `project/` | `RootItem`, `ProjectEntry`, `ProjectInfo`, `RustProject`, `WorkspaceMetadataStore`; `from_cargo_toml` parses manifests. Cargo and Git models live in their own subdirectories. |
| `scan/` | `BackgroundMsg`, `MetadataDispatchContext`, `ExcludeDirs`; `resolve_include_dirs`, `spawn_streaming_scan`, `build_tree` and `spawn_cargo_metadata_refresh`. |
| `sccache.rs` | `SccacheSummary`, `StatsResult`; `read_summary` and `read_stats` run sccache and parse its output. |
| `support.rs` | OS process helpers: `terminate_process_group`, `hard_kill_process_group`, `process_exists`; no owned app state. |
| `test_support.rs` | Test runtime, Git fixtures and allocation checks; `test_runtime`, `init_git_repo`, `count_allocations`, `ThreadCountingAllocator`. |
| `themes/` | App palettes and user-theme location; `builtins` returns framework `ThemeVariant` values; `themes_dir` resolves the directory. |
| `tui/` | App, UI state, input and rendering; exports terminal `run`. See the next table. |
| `watcher/` | `WatcherMsg`, `WatchRequest`, `WatcherLoopState`; `spawn_watcher` registers roots, reads filesystem events and sends `BackgroundMsg` results. |

### TUI modules

| Location | Job, types and entry points |
| --- | --- |
| `tui/app/` | `App` coordinates subsystems. `App::new` uses the builder in `tui/app/construct.rs`; navigation, dismissal and async handlers have separate files. |
| `tui/app_render_state.rs` | Borrowed render inputs: `RenderBorrows`, `RenderRegistry`, `FinderSplit`, `OverlayRenderInputs`; no persistent state. |
| `tui/background.rs` | `Background` owns result channels and the watcher handle; `register_item_background_services` registers projects. |
| `tui/columns/` | `ProjectRow`, `ProjectListWidths`, `LintCell`; `build_row_cells` and `row_to_line` format project columns. |
| `tui/constants.rs` | UI timings, log names and labels; no state or entry function. |
| `tui/dismiss_target.rs` | `DismissTarget` distinguishes a toast from a deleted project; consumed by `App::dismiss`. |
| `tui/finder/` | `FinderItem`, `build_finder_index`, `dispatch_finder_action`, `render_finder_pane_body`: search indexing, actions and drawing. |
| `tui/hit_test.rs` | `HoverTarget` and `HittableId` describe mouse targets and drawing order; interaction resolves them. |
| `tui/input/` | Key dispatch and editor/terminal launches; `handle_app_surface_key`, `open_in_editor`, `open_terminal`; consumes `App` and `KeyBind`. |
| `tui/integration/` | Adapts app actions to the framework; `AppPaneId`, `AppNavigation`, `build_framework_keymap`, `collect_reload_actions`. |
| `tui/interaction.rs` | `ClickMode`, `handle_click`, `hit_test_at`, `apply_hovered_pane_row`: mouse routing, selection and focus. |
| `tui/keymap/` | `ResolvedKeymap`, `ScopeMap`, action enums; `keymap_path`, `load_keymap`, `load_keymap_from_str` parse and resolve bindings. |
| `tui/keymap_ui/` | Keymap UI policy; `vim_mode_conflicts` and `is_generated_vim_extra` operate on framework key sequences. |
| `tui/messages.rs` | `CiFetchMsg`, `CleanMsg`, `OwnedRunEvent`: result protocols consumed by the async pollers. |
| `tui/overlays/` | `Overlays` owns finder/sccache visibility and return focus; `open_finder`, `close_finder`, `open_sccache`; `FinderPane` owns finder view state. |
| `tui/panes/` | `Panes`, pane structs, `PaneRenderCtx` consumers and layout; `resolve_layout` and body renderers draw app content. |
| `tui/project_list/` | `VisibleRow`, `ExpandKey`, `ProjectList` methods; `compute_visible_rows`, `recompute_visibility` and selection/expansion methods derive navigable rows. |
| `tui/project_list_state.rs` | Defines `ProjectList`: ordered roots, cursor, finder, expansion, storage and row/width caches. Methods live in `tui/project_list/`. |
| `tui/render.rs` | `ui` lays out and draws panes, overlays and status bar; `render_tiled_panes` dispatches pane rendering. |
| `tui/render_context.rs` | `PaneRenderCtx` borrows shared model and render values; pane renderers read it. |
| `tui/sccache/` | `SccachePane`, `SccacheStatusLine`; `refresh_summary_if_due`, `apply_summary`, `open_sccache_stats_overlay`, `render_sccache_popup`. |
| `tui/settings.rs` | `StartupSettings`, `SettingOption`; `load_cargo_port_settings_for_startup` loads settings; settings handlers save through framework `SettingsStore`. |
| `tui/startup_services.rs` | `StartupServices`, `StartupEnvironment`, `WatcherHandle`; service methods choose real startup work or quiet test substitutes. |
| `tui/state/` | `Net`, `Lint`, `Ci`, `Scan`, `Config`, `Keymap`, `Inflight`, Git/sync trackers; constructors and subsystem methods own state changes. |
| `tui/terminal/` | `run`, `event_loop`, `spawn_input_thread`; terminal setup/restore, frame timing, tree persistence and child processes. |
| `tui/test_support.rs` | `TestApp`, `FixtureDirs`; `make_app`, `make_app_with_config`, `make_local_startup_test_app` isolate app tests. |
| `tui/theme_roles.rs` | App color roles; `apply_role_defaults_to_registry` and `apply_role_defaults_to_theme` complete framework themes. |

## Data flows

### Startup and rows

1. `main` → `tui::run` in `tui/terminal/run.rs` → `load_cargo_port_settings_for_startup` → `CargoPortConfig::from_table` → `normalize_config` → `set_active_config`.
2. `resolve_include_dirs` → `spawn_streaming_scan` starts discovery before `App::new`. `AppBuilder::open_channels` → `run_startup` loads `themes::builtins` and user files into `ThemeRegistry`, resolves appearance, starts lint/watcher services → `build` creates `App`.
3. `phase1_discover` in `scan/discovery.rs` walks include directories and parses manifests into `RootItem::Rust`. When non-Rust inclusion is on, a folder with a .git directory and no Cargo.toml becomes `RootItem::NonRust` through `discover_non_rust_project`. `build_tree` in `scan/tree/build.rs` nests workspace members in `MemberGroup`, extracts vendored packages and merges Git checkouts into `RootItem::Worktrees` / `WorktreeGroup`.
4. `RootItem::list_order` in `project/root_item.rs` orders roots by case-insensitive directory leaf, then absolute path. Members sort by package name; `inline_dirs` controls their grouping.
5. `BackgroundMsg::ScanResult` → `App::handle_scan_result` in `tui/app/async_tasks/dispatch.rs` → `ProjectList::new` through `TreeMutation` → `recompute_visibility` / `compute_visible_rows` → `VisibleRow`. Detail, disk and width caches refresh before drawing.
6. `spawn_cargo_metadata_tree` in `scan/cargo_metadata.rs` → `BackgroundMsg::CargoMetadata` → `handle_cargo_metadata_msg` in `tui/app/async_tasks/metadata_handlers.rs` → generation-checked `WorkspaceMetadataStore`. `ensure_detail_cached` in `tui/app/navigation/cache.rs` builds the selected package, Git and target content.

### Watcher, deletion and dismissal

1. `register_watch_roots` in `watcher/roots.rs` registers recursive include roots; `register_cargo_home_watch` adds Cargo config coverage. `watcher_loop` in `watcher/runtime.rs` accepts project registration through `WatcherMsg` and buffers notify events until registration completes.
2. `process_notify_events` → `handle_notify_event` in `watcher/events.rs` reads event paths and kinds. Source/manifest events can call `RuntimeHandle::lint_trigger`; manifest, lockfile and Cargo config changes dispatch metadata refreshes. Target-layout paths also call `spawn_project_refresh` in `watcher/probe.rs`.
3. Git index/HEAD/refs/ignore events → `enqueue_git_refresh` / `fire_git_updates` in `watcher/refresh.rs` → `BackgroundMsg::RepoInfo` and `CheckoutInfo` → repo handlers update project Git state. Shared Git directories combine worktree refreshes; each checkout gets its own status probe.
4. Ordinary project events → `schedule_disk_refresh` / `fire_disk_updates` → `BackgroundMsg::DiskUsage`. Build-directory events update disk usage without scheduling ordinary Git refreshes. New or missing root candidates → `probe_new_projects` → `ProjectDiscovered`, `ProjectRefreshed`, `DiskUsageBatch` or zero-byte `DiskUsage`.
5. `handle_disk_usage_msg` → `apply_disk_usage` in `tui/app/async_tasks/disk_handlers.rs` updates `ProjectInfo`: zero bytes plus a missing path sets `Visibility::Deleted` and unregisters lint; positive bytes restore `Visibility::Visible`. Disk caches and storage totals refresh.
6. `Visibility::Deleted` stays in `compute_visible_rows`; project drawing strikes it through. A deleted project's row does not leave the list on a watcher event. One exception: `App::rescan` in `tui/app/async_tasks/tree.rs` clears every root (a config reload that asks for a full rescan calls it), and discovery cannot restore a missing project, so its row is gone. The user dismisses it with default `x` or its close box → `App::dismiss` in `tui/app/dismiss.rs` → `Visibility::Dismissed`, lint-cache reclamation and row-cache refresh. `Dismissed` rows are filtered out.

### Async dispatch and services

1. `App::poll_background` in `tui/app/async_tasks/poll.rs` drains a bounded batch of `BackgroundMsg` values → `handle_bg_msg`. `update_generations_for_msg` invalidates selected details only when a message can affect them.
2. Dispatch groups: `tui/app/async_tasks/disk_handlers.rs` handles disk/storage; `tui/app/async_tasks/metadata_handlers.rs` handles Cargo metadata and redirected targets; `tui/app/async_tasks/repo_handlers.rs` handles Git, repo metadata, PRs, discovery and refresh; `tui/app/async_tasks/lint_handlers.rs` handles lint status/cache; `tui/app/async_tasks/crates_io_handlers.rs` handles crate fetch progress and releases; `tui/app/async_tasks/service_handlers.rs` handles reachability and quota signals. Language/test batches update `ProjectList` directly.
3. `poll_ci_fetches` consumes `CiFetchMsg::Complete`; `poll_clean_msgs` consumes `CleanMsg::Finished`; `poll_owned_run_events` applies `OwnedRunEvent` start/output/progress/termination/finish messages to `Inflight`. `tui/app/async_tasks/running_toasts.rs` and `tui/app/async_tasks/startup_phase/` track task progress.
4. `fetch_project_details` / `spawn_repo_fetch_for_git_info` → `HttpClient` in `http/client.rs` → `http/github_actions.rs`, `http/github_pull_requests.rs` and `http/crates_io.rs` → typed background replies. `HttpClient::new` runs `gh auth token` once; `http/auth.rs` classifies the result. `http/rate_limit.rs` parses separate core/GraphQL quotas and rate-limit responses; service handlers and `tui/app/async_tasks/recovery.rs` manage retry/recovery. `scan/ci_cache.rs` merges cached and fetched `CiRun` values; `ci.rs` builds job/run status.
5. Lint registration and triggers → `lint/runtime/supervisor.rs` → per-project `ProjectWorker` → `lint/runtime/command.rs` runs configured commands under a run lock and writes history/status → `BackgroundMsg::LintStatus` → `Lint` and project lint records. Pause and idle timing belong to the runtime.
6. `refresh_summary_if_due` in `tui/sccache/status_line.rs` → worker `sccache::read_summary` → `BackgroundMsg::SccacheSummary` → `apply_summary`. The overlay separately requests `read_stats` and receives `BackgroundMsg::SccacheStats` with a request id.

### Render loop and framework

1. `event_loop` in `tui/terminal/event_loop.rs` drains input and background messages, ticks CPU data and refreshes visible-row, disk, width and detail caches → `draw_frame` → `ui` in `tui/render.rs`.
2. `render_tiled_panes` → `RenderRegistry` / `PaneRenderCtx` → pane renderers. `render_project_list_pane_body` in `tui/panes/project_list/tree_render.rs` uses `render_tree_items` and column builders to draw `VisibleRow` data.
3. After drawing, `spawn_pending_background_tasks` starts queued CI/clean/owned-run work. `wait_for_event` selects input, result and CPU channels until an event or `App::animation_timeout` (animation tick or idle heartbeat). Idle ticks also support config/keymap/theme reload checks.
4. cargo-port owns project models, service work, actions, pane content and palettes. The workspace crate `crates/tui_pane/` owns reusable framework focus, key routing, viewports, pane chrome/layout tools, settings, themes and toasts. `tui/integration/framework_keymap/` supplies app hosts; `tui/app/render_registry.rs` supplies pane lookup.

## State ownership

| Owner | State and writers |
| --- | --- |
| `App` in `tui/app/mod.rs` | Subsystems, framework, startup progress and confirmation state. Named app methods coordinate changes across owners. |
| `ProjectList` / `ProjectEntry` | Root hierarchy and all project data, selection, expansion, finder and derived row/width caches. Async handlers and navigation methods write through list methods; `TreeMutation` clears dependent pane caches and recomputes visibility. |
| `Panes` in `tui/panes/system.rs` | Pane instances, layout and detail stamp. `set_detail_data` updates package/Git/targets together. Each pane owns its viewport/focus; renderers update geometry and actions change selection. |
| `PackagePane`, `GitPane`, `TargetsPane` | Selected content and hit-test geometry; Git also owns worktree summary cache. `LangPane` owns view state and reads model language data. `CpuPane` owns its monitor and sampling state. |
| `ProjectListPane`, `OutputPane` | Project list geometry/dismiss targets; Output selection and scrolling. The project model stays in `ProjectList`; owned-run output stays in `Inflight`. |
| `Ci`, `Lint` in `tui/state/` | CI and lint panes own their content, viewport and focus here. `Ci` also owns fetch/display tracking; `Lint` owns runtime handle, running-task trackers, pause toast and cache usage. Async handlers update both. |
| `Net` in `tui/state/net.rs` | Shared `HttpClient`, GitHub repo cache/in-flight fetches/PR polls, service availability and network-task progress. Repo/service handlers mutate it; the HTTP client shares quota state. |
| `Scan`, `Config`, `Keymap`, `ThemeRuntime` | Scan generations, metadata store and shimmers; watched config/keymap and diagnostics; theme directory watch. Scan handlers and `tui/app/async_tasks/config.rs` change them. Active themes/registry live in framework global theme state. |
| `Inflight`, `Background`, lint runtime | Pending actions, clean/owned-run lifecycle; channels/watcher handle; supervisor-owned worker map and child processes. `OwnedRunProcessActor` serializes owned-run reaping/signals. `SccacheStatusLine` owns its summary and poll state on `App`. |

## Configuration and themes

`config.rs` resolves config.toml beneath CARGO_PORT_CONFIG_DIR when set, otherwise beneath dirs::config_dir()/cargo-port. An empty override is an error. Settings startup seeds a missing file; `normalize_config` normalizes lint commands/cache units, crate release groups, branch names, CPU limits and shimmer duration.

Sections are cache (root), crates_io (release groups), cpu (sampling/thresholds), mouse (scroll), tui (include/exclude/inline directories, navigation, editor/terminal, branches, CI count), lint (eligibility, commands, idle delay, cache), debug (forced quota exhaustion), appearance (mode/themes/tint/transparency) and framework toasts. `resolve_include_dirs` expands home paths, joins relative entries to the home directory and canonicalizes existing paths. Empty include_dirs scans nothing; cwd is not the scan root.

Theme templates are `crates/cargo-port/themes/default_dark.toml`, `crates/cargo-port/themes/default_light.toml` and `crates/cargo-port/themes/high_contrast.toml`. `themes/builtins.rs` compiles the palettes; user TOML files under the resolved configuration root's themes directory replace matching names. The app does not seed that directory.

The same root holds keymap.toml. `tui/keymap/load.rs` loads defaults, migrations and user bindings; actions, canonical keys, resolved bindings and scopes are in adjacent files. `crates/cargo-port/tests/assets/default-keymap.toml` and `crates/cargo-port/tests/assets/default-config.toml` record defaults.

## Tests and live check

Tests mostly live in inline test modules beside code. Separate files include `tui/settings_scroll_tests.rs`, `tui/panes/output/presentation_tests.rs` and `tui/render/status_bar_contrast_tests.rs`; `crates/cargo-port/tests/` holds default-file assets. `make_app` and `make_app_with_config` in `tui/test_support.rs` disable unrequested startup services and isolate files. `count_allocations` in `test_support.rs` counts allocations on the calling thread. `init_git_repo` installs real loose objects from `test_support/git_seed/` to avoid repeated Git setup commands.

No test takes a second. Use the managed checks:

```sh
bash ~/.claude/scripts/direct/verify.sh lint cargo-port
bash ~/.claude/scripts/direct/verify.sh test cargo-port
```

For a live check on natedev/Linux, build first (the first command below prints `binary: <path>`). The commands below create an absolute scratch XDG_CONFIG_HOME, removes an inherited configuration override, sets a scratch cache root and scans only the scratch project parent:

```sh
bash ~/.claude/scripts/direct/verify.sh build cargo-port
port_scratch=$(mktemp -d /tmp/cargo-port-live.XXXXXX)
mkdir -p "$port_scratch/config/cargo-port" "$port_scratch/projects/map-check/src"
cat > "$port_scratch/projects/map-check/Cargo.toml" <<'TOML'
[package]
name = "map-check"
version = "0.1.0"
edition = "2024"
TOML
printf 'pub fn check() {}\n' > "$port_scratch/projects/map-check/src/lib.rs"
cat > "$port_scratch/config/cargo-port/config.toml" <<TOML
[tui]
include_dirs = ["$port_scratch/projects"]
[cache]
root = "$port_scratch/cache"
[lint]
enabled = false
TOML
env -u CARGO_PORT_CONFIG_DIR XDG_CONFIG_HOME="$port_scratch/config" \
  <binary path>
```

In a terminal, confirm map-check appears in the project list, then quit with q. Config and keymap resolve under the scratch configuration directory; saved tree state uses the scratch cache root.

Projects under ~/rust/repro are change-review repros made by scratch.py --visible.

Related detail: [Targets pane](targets-pane.md) and [Lint file-lock spinner](lint-file-lock-spinner.md).
