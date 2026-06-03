# cargo-port

[![CI](https://github.com/natepiano/cargo-port/actions/workflows/ci.yml/badge.svg)](https://github.com/natepiano/cargo-port/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/cargo-port.svg)](https://crates.io/crates/cargo-port)
[![docs.rs](https://docs.rs/cargo-port/badge.svg)](https://docs.rs/cargo-port)
[![license](https://img.shields.io/crates/l/cargo-port.svg)](LICENSE-MIT)

cargo port is a terminal dashboard for your Rust workspaces and projects. Configure it to scan one or more directories to view workspaces, crates, worktrees, vendored dependencies, targets, local lint state, GitHub CI, pull requests, and machine diagnostics in one keyboard-driven view.

See everything about your rust environment in one place. Every question answered without having to hit the cli. Dynamically updated.  Maybe you're using a coding agent, I won't judge. While your agents are doing all the work, cargo port will show you what's up!

And if you're old school, the information is dense and informative. And fast. There is no better overview of all of your project info (confidently asserted...and...PR's welcome).

<img src="assets/dashboard.gif" alt="cargo-port dashboard showing project tree, worktree details, Git status, CPU and GPU diagnostics, targets, lint runs, and CI runs" width="100%">

- **Inventory everything** - workspaces, members, linked worktrees, submodules, vendored crates, examples, benches, binaries, tests, and non-Rust git repos
- **Run and inspect targets** - launch examples, benches, and binaries in debug or release mode with live output and running-target markers
- **Track project health** - see lint status, archived lint runs, GitHub Actions history, open pull requests, PR check polling, and GitHub rate-limit state
- **Keep context visible** - inspect package metadata, target directories, language stats, worktree summaries, remotes, CI jobs, and pull request rows without leaving the TUI
- **Navigate quickly** - fuzzy search, vim-style paging, keymaps, tab traversal, global shortcuts, and selection copy
- **Themes** - light/dark/high-contrast variants, and hot-reload - there's not a lot of themes here yet, but you know, PR's welcome
## Try me

Build the current `main` branch:

```bash
git clone https://github.com/natepiano/cargo-port.git
cd cargo-port
cargo build
cargo run
```

Install the latest published crates.io release:

```bash
cargo install cargo-port
cargo port
```

### Light up the GitHub panes

CI runs, the CI status column, pull requests, repo stars/description, and rate-limit state all come from GitHub's API. cargo-port reads your auth token from the [GitHub CLI](https://cli.github.com), so install it and log in to turn those panes on:

```bash
gh auth login
```

Everything local - git status, remotes, worktrees, languages, targets, lint runs, CPU/GPU - works without it; you'll just get a warning where the GitHub data would be. The token is read once at startup, so restart cargo port after logging in.

## cargo port panes overview

The dashboard combines a project tree in the upper left with detail panes for package metadata, Git state, languages, CPU/GPU activity, targets, lint history, and CI runs - all shown based on the currently selected project in the upper left.
### Dashboard View
<img src="assets/dashboard-overview-numbered.png" alt="Numbered cargo-port dashboard overview showing each major pane" width="100%">

1. **Project tree** workspaces, members, linked worktrees, submodules, vendored crates, optional non-Rust repos, status columns, and disk rollups.
2. **Workspace details**: Cargo metadata-backed package summary, disk breakdown, lint/CI rollups, and target structure counts.
3. **Git**: branch status, sync state, remotes, worktrees, GitHub rate-limit state, and pull request rows when available.
4. **Languages**: per-project language totals by file count, code, comments, blanks, and total lines.
5. **Diagnostics**: CPU and GPU utilization, with background refresh.
6. **Targets**: examples, benches, binaries, and tests with source package and target kind.
7. **Lint runs**: local lint/watch history and cached run artifacts.
8. **CI runs**: GitHub Actions history with job-level status and duration columns.
9. **Status bar**: current mode, pane navigation, active action, and shortcut help.

### Project Tree
On startup, the configured `Include dirs` are scanned by cargo port to retrieve Cargo.toml metadata, GitHub metadata including CI runs, disk usage, local lint pass/fail status, programming language line counts and configured targets. 

The Project Tree is the main navigation point for the app - other panes adapt to show detailed information about the selected row in the tree.

We set a file watcher on these projects so any changes will invoke a lint run (if configured). New projects will automatically appear and deleted projects will disappear.  Disk space adjusts itself automatically with each changed file and recompile.

<img src="assets/pane-project-tree-numbered.png" alt="Numbered project tree pane" width="75%">

1. The title shows scanned directories and project counts for each configured scan dir.  On first run, cargo port will prompt you to define your "Include dirs" in Settings.
2. Hierarchical project list with expandable workspaces, members, worktrees, submodules, and vendored crates. You can configure whether non-Rust git projects are included. 
3. - Status columns
	1. Lint pass/fail - a lint command configurable in settings. Lint column will show an activity spinner for a currently running lint. 
	2. CI passed 🟢, failed 🔴, skipped ⚪, cancelled ⚫. Not shown if ci is not configured or if a remote isn't configured for a branch. Currently only supports GitHub.
	3. Git status - clean ✨, modified 🟠, untracked 🟢
	4. Origin sync - whether the project is synced (☑️), with configured origin or ahead/behind
	5. main sync whether a worktree checkout is synced (☑️) with the main branch or ahead/behind
	6. Disk usage - <span style="color: yellow;">Σ</span>
4. Worktree-group rows with branch and status rollups. Any row with a tree emoji (🌲) is a worktree group and appends the count of checkouts next to it (🌲:2).  You can expand it to see info about each separate checkout.
5. Total disk usage across the visible project set because, you know, rustc consumes a lot of disk.  There is a keyboard shortcut (defaults to 'c') to clean the currently selected workspace/project.

### Detail Pane
This space adapts to show a distinct details pane based on what is selected in the Project pane. You can see useful information for all of these:

- Workspace
- Package
- Worktree Group
- Non-rust Project
- Git Submodule
- Vendored crate

Below, we're showing a Worktree Group as an example as the rest will be self-explanatory when you run the app.  The Worktree Group is a good representative as it is one of the more comprehensive detail panes.

<img src="assets/pane-details-numbered.png" alt="Numbered workspace details pane" width="75%">

1. Title shows what kind of detail pane is shown - in this case it's a WorktreeGroup for the bevy_window_manager project.  The description from Cargo.toml is shown if there is one.
2. Summary of how many worktrees, the aggregate lint status across all checkouts, and the CI status from GitHub. The numbers show local cached run metadata and how many runs are there in total on GitHub.
3. Package location, disk breakdown and metadata from Cargo.toml
4. Structure indicates what targets are configured - counts of bins, libs, examples, benches, proc-macros.
5. Tests shows counts of unit, integration and doc tests as well as the count of ignored. It doesn't attempt to break it out by feature gates so it may not always match up exactly.
6. crates.io version info and download count

### Git Pane
cargo-port uses local git data plus GitHub's REST and GraphQL APIs, authenticated with the token from your `gh` login (see [Try me](#try-me)).

<img src="assets/pane-git-numbered.png" alt="Numbered Git pane" width="75%">

1. Pane title shows the current branch and the description from GitHub if it exists.
2. Branch info, status, stars. Project incept date and date of latest commit. Also date of last fetch.  Rate limits exist on GitHub so we show them in case you are hammering github with requests, you'll see it reflected here.  GitHub has different limits for core and GraphQL requests so we show both as well as a countdown timer as to when the limit will reset.
3. All configured remotes - including sync (synced/ahead/behind) status between the branch and the tracked origin.
4. All worktree checkouts. The primary checkout (where the .git directory lives) isn't tracked versus anything so just shows dashes for Tracked and Sync. Other checkouts show their branch and tracked status versus the local main. It's what i find most useful in my workflow - and I'm interested in other points of view so make an issue or a PR and we'll discuss.
### Languages Pane
Cargo port uses the tokei crate to scan project languages at startup. It doesn't refresh automatically but you can always ctrl-r to rescan (default key binding) if your curiosity is getting to you. Can you guess what famous rust game engine has these stats?

<img src="assets/pane-languages-numbered.png" alt="Numbered languages pane" width="75%">

1. Detected languages with file-type icons. Some icons are better than others. Some don't exist. Not wanting to get even deeper into fonts, emojis, and specific terminal implementations, we're doing the best we can.
2. File counts per language.
3. Code, comment, blank-line, and total-line counts.

What good is this? Is this useful information taking up value screen real estate? I don't know. But we have it so enjoy!
### CPU/GPU Pane
Dynamic character based CPU/GPU usage. If this doesn't get you out of bed in the morning, I don't know what will.  On my MacBook Pro, I have 12 cores. And it all fits in the pane.  But you might have many more. Don't worry, it will scroll - or you might turn your terminal sideways and see them all. I don't know. Is this useful? The aggregate is always interesting - but the per core?

<img src="assets/pane-diagnostics-numbered.png" alt="Numbered CPU and GPU diagnostics pane" width="20%">

1. Per-core CPU usage bars.
2. Aggregate system, user, and idle CPU percentages.
3. GPU utilization when available for the current platform. Not super tested for your particular wacky setup yet.  GPU is an aggregate right now. If you don't like this view, PR's are welcome. And I will have opinions about style.

"You will ride eternal, shiny and chrome". 
### Targets Pane

<img src="assets/pane-targets-numbered.png" alt="Numbered targets pane" width="75%">

1. Runnable target names - you can launch it by hitting enter for a debug run, 'r' for `cargo-run --release`. Once running you can type 'K' to kill (uppercase to not clash with vim bindings in case you turned them on).  If you run multiple instances, they will who as running in this pane and you can type 'K' on the parent name to killall. 
2. Source package for each target.
3. Target kind: example, bench, bin
4. Page position for long target lists - just mentioning it here but any pane that has a list that exceeds will show these markers. You can scroll through and pgup/pgdown/home/end are available and you can turn on vim bindings for limited vim navigation support.
### Lint Runs
You may optionally configure a comma delimited list of commands to run as a lint on rust projects (only).  The included directories have a watcher on them and if any rust file or Cargo.toml changes, then the lint commands will be asynchronously executed and tracked to completion. 

When finished results from historical runs will showin the Lint Runs pane. 

You can select and hit enter on a run to open the stdout from each command in your configured eidtor.

<img src="assets/pane-lint-runs-numbered.png" alt="Numbered lint runs pane" width="75%">

1. Run dates are grouped to make this pane less busy.
2. Runtime duration.
3. Cached output file size. Total Lint run storage space is configurable. Oldest runs are automatically evicted when they exceed the configured limit.
4. Pass/fail result.
### CI Runs

GitHub Actions runs are cached to disk so the dashboard stays useful offline 
<img src="assets/pane-ci-runs-numbered.png" alt="Numbered CI runs pane" width="100%">

1. CI run count and selected branch. You can filter for the current branch or show all ci runs.
2. Commit summary for each run. 
3. Branch and timestamp.
4. Job-level status columns. These are constructed from the ci runs themselves. If there are too many columns it will show the ones that have the longest durations and collapse the ones that don't. 
   
   I didn't choose to implement side scrolling. this may not work for you but you can press enter on any run and it will open a browser to take you to that run in GitHub where you can see everything you want to see.

## Navigation

Press `?` in the TUI to open the global shortcuts overlay.

- Use `/` to fuzzy-find projects, packages, examples, benches, binaries, and tests
- Use `Tab` to move between panes; optional edge-scroll can advance focus when a list hits its top or bottom
- Enable vim navigation with `navigation_keys = true` for `hjkl` movement in non-text panes
- Use chord keymaps for multi-key commands and `y` to copy the selected pane row's path, URL, or field value when available
- Open projects, config, keymaps, GitHub URLs, crates.io pages, and terminal sessions from the selected context

## GitHub, CI, and PRs



- Pull request rows show open PRs for the selected repo, including deleted/disappeared PR toasts and check polling
- GitHub API rate-limit and service recovery state are shown in the Git pane
- If `gh` is missing or unauthenticated, cargo-port warns in the UI instead of silently hiding the problem

## Themes and Diagnostics

The TUI has runtime-swappable themes and lightweight machine diagnostics.

- Built-in themes include default dark, default light, and high-contrast variants
- User themes live under the platform config directory in `cargo-port/themes/` and reload while the app is running
- `[appearance]` can follow the OS appearance or force light/dark mode
- The CPU/GPU pane refreshes in the background; GPU availability depends on platform support
- The sccache pane appears when a configured Rust compiler wrapper points at `sccache`

## Configuration

cargo-port creates a config file on first run at:
- **macOS**: `~/Library/Application Support/cargo-port/config.toml`
- **Linux**: `~/.config/cargo-port/config.toml`

### Scan directories

By default, cargo-port scans the entire scan root (defaults to `~`). To limit scanning to specific directories, set `include_dirs` in the config file or via the in-app settings editor (press `s`).

Paths can be relative to the scan root or absolute:

```toml
[tui]
include_dirs = ["rust", "projects", "/opt/work"]
```

An empty list (the default) scans the entire scan root. Changes to `include_dirs` in the settings editor trigger an automatic rescan.

### Include non-Rust projects

To also show non-Rust git repositories in the project tree:

```toml
[tui]
include_non_rust = true
```

These show up with reduced details (no types, version, examples) but can still display disk usage, git info, and CI runs.

### Navigation

```toml
[tui]
navigation_keys = true
edge_scroll = true
```

`navigation_keys` enables `hjkl` movement in non-text panes. `edge_scroll` moves focus to the adjacent pane when scrolling past the top or bottom of a list.

### Appearance

```toml
[appearance]
mode = "auto"
light_theme = "Default Light"
dark_theme = "Default Dark"
focused_pane_tint = true
```

`mode` accepts `"auto"`, `"light"`, or `"dark"`. Custom themes can be added under the platform config directory:
- **macOS**: `~/Library/Application Support/cargo-port/themes/`
- **Linux**: `~/.config/cargo-port/themes/`

### Diagnostics

```toml
[cpu]
poll_ms = 1000
green_max_percent = 60
yellow_max_percent = 85
```

CPU and GPU diagnostics refresh in the background. GPU usage is shown when cargo-port can read it from the current platform; otherwise the GPU row reports unavailable.

### Lints

Lints is cargo-port's local lint/watch runtime. When enabled, cargo-port watches only the projects you allow-list, runs configured commands when they change, and shows the current status in the project list.

Lints is off by default.

In the Settings popup (`s`), the `Lints` section exposes:
- `Enabled`
- `Projects`
- `Commands`
- `Cache size`

`Projects` is an allow-list. If it is empty, Lints watches nothing.

#### Basic config

```toml
[lint]
enabled = true
include = ["cargo-port", "bevy_lagrange"]
exclude = []
commands = []

[port_report]
cache_size = "512 MiB"
```

Notes:
- `include` entries can be bare project names, display-path prefixes, or absolute-path prefixes
- `exclude` is applied after `include`
- an empty `commands` list uses the built-in default command
- `port_report.cache_size` caps retained lint run storage across JSON history and cached artifacts; `0` and `unlimited` disable pruning

#### Commands

The released default is a single clippy command:

```toml
[lint]
enabled = true
include = ["cargo-port"]
exclude = []
commands = []

[port_report]
cache_size = "512 MiB"
```

That expands to:

```toml
[[lint.commands]]
name = "clippy"
command = "cargo clippy --workspace --all-targets --all-features --manifest-path \"$MANIFEST_PATH\" -- -D warnings"
```

If you want to override that, you can configure explicit commands:

```toml
[lint]
enabled = true
include = ["cargo-port"]

[[lint.commands]]
name = "mend"
command = "cargo mend --manifest-path \"$MANIFEST_PATH\" --all-targets"

[[lint.commands]]
name = "clippy"
command = "cargo clippy --workspace --all-targets --all-features -- -D warnings"
```

`command` is executed as a shell command in the project root, not as an implied Cargo subcommand. That means values like `cargo fmt --check`, `cargo mend --manifest-path "$MANIFEST_PATH" --all-targets`, `cargo clippy --workspace --all-targets --all-features --manifest-path "$MANIFEST_PATH" -- -D warnings`, or `something --else` are all valid.

In the Settings popup, `Commands` accepts a comma-separated list of full shell commands.

Legacy preset-style entries such as `clippy` or `mend` are normalized to their built-in command definitions when config is loaded or saved.

#### Cache size

`port_report.cache_size` accepts flexible binary-size strings such as:
- `512MiB`
- `512 MiB`
- `1.5 GiB`
- `0`
- `unlimited`

Values are normalized when config is loaded or saved. The cache size caps retained lint run storage under the shared cache root. When stored runs exceed the limit, cargo-port prunes the oldest runs first and keeps current/latest artifacts even if that live floor alone exceeds the configured size.

#### Cache location

Lints writes its cache under cargo-port's shared cache root.

By default this uses the platform cache directory:
- macOS: `~/Library/Caches/cargo-port`
- Linux: `~/.cache/cargo-port`

You can override the root:

```toml
[cache]
root = ""
```

Rules:
- empty string means use the default platform cache root
- a relative path extends the default cargo-port cache root
- an absolute path replaces it

Lint run data is stored under `lint-runs/` within the cache root. CI cache uses the same shared root under `ci/`.
## Platforms
Tested primarily on macos, limited testing on windows and linux.
## License

`cargo-port` is free, open source and permissively licensed!
Except where noted (below and/or in individual files), all code in this repository is dual-licensed under either:

* MIT License ([LICENSE-MIT](LICENSE-MIT) or [http://opensource.org/licenses/MIT](http://opensource.org/licenses/MIT))
* Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or [http://www.apache.org/licenses/LICENSE-2.0](http://www.apache.org/licenses/LICENSE-2.0))

at your option.

### Your contributions

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
