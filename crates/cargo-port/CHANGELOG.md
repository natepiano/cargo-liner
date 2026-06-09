# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.3] - 2026-06-09

### Added
- Add click-to-toggle behavior for expandable project tree and Running outline rows

### Changed
- Mark the primary checkout in expanded worktree groups with `(p)`
- Sort expanded worktree-group checkouts alphabetically

### Fixed
- Keep linked worktree lint status visible in the project list after worktree refreshes
- Clear lint history when dismissing deleted project rows
- Let lint include filters match a linked worktree's primary checkout even when the worktree folder has an unrelated name

## [0.1.2] - 2026-06-08

### Added
- Add Rust language-pane child rows for code, unit tests, integration tests, examples, and benches, with distinct subtotal coloring
- Persist the project tree's expand/collapse state (at every nesting depth) alongside the selected project, restored on the next launch and preserved across rescans (stored in `tree_state.toml`)

### Fixed
- Isolate launched examples so stopping them from the Output pane does not also quit cargo-port
- Keep the project-list Lint column showing the running spinner when async lint-history hydration lands during an active lint run
- Avoid rerunning lint commands during rescan; startup now hydrates terminal-only cached lint status instead of feeding the running Lints toast
- Re-lint projects whose source changed since their last run once startup finishes, so a stale lint status is no longer shown as current; never-linted projects still respect the discovery-lint setting
- Prevent Startup from entering its close countdown until startup-owned GitHub and crates.io work is terminal
- Hold the Startup GitHub row open until every spawned repo-fetch worker reports, so a fetch that queues just as startup finishes stays inside the panel instead of leaking a standalone "Retrieving GitHub repo details" toast
- Show incremental Languages startup progress while language stats scan files
- Keep the completed Startup panel green during its close countdown instead of sending it through the task-toast fade path
- Avoid a full terminal clear when stopping a running example from the Output pane
- Fix Esc handling so framework overlays close before the output pane when both are visible

## [0.1.1] - 2026-06-06

### Fixed
- Fix Targets pane Source and Kind column alignment so Kind sits against the pane edge
- Fix workspace-root target Source labels to show package names instead of `workspace`

## [0.1.0] - 2026-06-05

### Added
- Add CPU, GPU, and sccache diagnostics panes with background polling and platform-specific GPU detection
- Add a runtime theme system with built-in dark, light, and high-contrast themes, user theme hot-reload, appearance settings, terminal background matching, and focused-pane tinting
- Add open pull request display, deleted-pull-request toasts, pull request check polling, and animated pull request check status
- Add global shortcuts overlay, vim paging, chord keymaps, tab traversal, edge-scroll navigation, and selection copy support
- Add cargo metadata-backed package and target details, workspace target aggregation, running-target markers, and richer clean-plan confirmation
- Add GitHub rate-limit and service-recovery status in the Git pane, including persistent recovery toasts and automatic refetch after service recovery
- Add richer worktree, submodule, and vendored-package handling, including submodule Git state and vendored workspace member rows
- Add Git pane branch relation labels, bisect progress, aligned remote/worktree sync columns, and clearer `gh` status handling
- Add Package detail test counts, rustdoc doctest counts, and crates.io version, prerelease, and download stats
- Add output-pane line selection, yank, vim visual selection, mouse drag selection, and sanitized copied output
- Add a collapsible Running sub-pane with per-process CPU/memory, process outlines, PID-specific kill controls, and CPU/GPU smoothing
- Add a consolidated startup progress panel and first-run Include dirs hint
- Add cache root editing in Settings and clearer CPU utilization threshold names

### Changed
- Convert the app into a Cargo workspace and extract reusable TUI framework code into the new `tui_pane` crate
- Rework pane rendering, keymaps, overlays, settings, toasts, themes, and hit testing around shared framework APIs
- Replace hand-parsed Cargo target and package fallbacks with `cargo_metadata`-driven records
- Improve app responsiveness with cached scroll hot paths, event-driven rendering, background diagnostics polling, and higher scan concurrency
- Improve clean behavior with target-directory indexing, async re-fingerprinting, affected-sibling confirmation, and target vs non-target disk breakdowns
- Improve startup responsiveness by loading lint history off-thread and persisting archived run sizes
- Resize top-row panes from rendered content so middle-row panes get more usable space

### Fixed
- Fix lint runtime reliability across interrupted runs, deleted worktrees, Windows execution, Linux file-read events, cache pruning, and cloned runtime handles
- Fix CI and pull request rendering edge cases, including skipped vs cancelled runs, manual fetch and clear, pull request snapshot loading, pull request tables, CI column layout, and inherited CI on vendored rows
- Fix finder and keymap behavior for visible-entry indexing, path separator matching, TOML override preservation, keymap popup height, and app-global shortcut registration
- Fix toast lifecycle bugs for cargo clean completion, task countdowns, startup disk scans, duplicate CI toasts, char-boundary panics, and elapsed-time display noise
- Fix live project and worktree state for refreshed worktree groups, deleted project cleanup, inline dirs, newly discovered project metadata, and member vendored details
- Fix CI branch/all filtering, CI header alignment, and vendored/member rows inheriting worktree git deltas
- Fix worktree-group target display, fitted target columns, and launched targets disappearing from the Running pane
- Fix startup crates.io fetch tracking and prevent standalone network toasts from leaking during startup
- Fix detail-pane overflow pagination, keymap popup sizing, language table colors, and terminal-control leakage in target output

## [0.0.3] - 2026-04-16

### Added
- Built-in lint runs with configurable commands, project filters, archived history, and an editor shortcut for opening saved run output
- Customizable `keymap.toml` shortcuts with hot-reload, conflict diagnostics, and an in-app keymap overlay for browsing and rebinding actions
- Vendored crates and Git submodules now appear as child entries in the project tree with their own navigation and detail views
- Workspace worktrees are now grouped under their primary checkout with expandable member hierarchies and rolled-up git and lint state
- A dedicated Languages pane now shows per-project language breakdowns with icons, file counts, code, comments, and blank-line totals
- Git status now includes local-main sync indicators, clearer unpublished-branch states, and a refresh action for fetching newly created CI runs
- A configurable terminal shortcut opens a shell at the selected project, and config/keymap overlays can be opened directly in the editor
- Broken worktrees are detected and highlighted in the tree, and build output now preserves ANSI colors

### Changed
- `cargo-port` is now TUI-only, with the old `list` and `ci` subcommands removed
- Startup, scanning, and CI fetching are substantially faster through background discovery, batched HTTP and GraphQL requests, and decoupled git and disk refreshes
- The TUI layout was reworked into dedicated Package, Type, Languages, Git, Lint Runs, and CI panes with per-pane scrolling, hover, and focus behavior
- Git and CI details are now scoped to the correct branch-owning row, with configurable primary-branch comparisons instead of relying on remote-only state
- Lint data now lives under the shared cache root with stable archived run directories and cache-size-based pruning

### Fixed
- New projects are detected while the app is running, and deleted projects are excluded from project counts and worktree totals
- Workspace membership and worktree grouping now stay correct for nested members, linked worktrees, and vendored children during live updates
- Git state now refreshes correctly after commits, index updates, linked worktree metadata changes, and worktree cleanup events
- Startup and priority background work no longer block the UI thread, improving responsiveness during scans and metadata refreshes
- Lint timestamps, unpublished-branch CI messaging, toast rendering, and pane hover and selection behavior were corrected across the redesigned interface

## [0.0.2] - 2026-03-30

### Added
- Add `Pane` abstraction to preserve last row position when returning to a pane via Tab, with per-pane cursor, length, content area, and scroll offset
- Detect new projects added under the scan root while running
- Strikethrough styling for projects whose directories are deleted from disk

### Changed
- Unify disk and new-project watchers into a single scan-root watcher (macOS FSEvents). Linux users may hit inotify watch limits with large directory trees.

### Fixed
- Expand arrow shown on projects with only vendored crates and no workspace members or worktrees

## [0.0.1] - 2026-03-28

### Project Discovery & Organization
- Recursively scans a directory tree to discover Rust projects and optionally non-Rust git repos
- Organizes projects into a hierarchical tree: workspaces, member groups, and individual packages
- Detects git worktrees and nests them under their primary checkout
- Identifies vendored crates
- Groups workspace members by subdirectory with configurable inline dirs that flatten into the parent
- Configurable directory exclusions

### Per-Project Metadata
- Parses `Cargo.toml` for name, version, description, and workspace status
- Detects project types: binary, library, proc-macro, build script
- Auto-discovers examples (grouped by subdirectory), benchmarks, and test targets
- Computes disk usage per project with percentile-based color gradient (green to red)
- Detects git origin type: local (no remote), clone, or fork (has upstream)
- Extracts branch, owner, repo URL, first commit date, and last commit date
- Fetches latest stable version from crates.io
- Fetches GitHub star count

### GitHub Actions CI Integration
- Fetches recent CI runs per project via `gh` CLI with configurable run count
- Displays per-job status across categorized columns (fmt, taplo, clippy, mend, build, test, bench)
- Shows wall-clock duration for each run
- Disk-based cache with merge strategy that works offline with cached data
- Pagination to load older runs on demand
- Open any run directly in the browser

### Interactive TUI
- **Project list** (left panel): expandable/collapsible tree with columns for disk usage, CI status, origin type, and language icon
- **Detail panel** (right, top): three-column view — Package (name, path, types, disk, version, description, crates.io version, vendored crates), Git (branch, origin, owner, repo, stars, inception/latest dates), and Targets (runnable binaries, examples, benchmarks)
- **CI/Output panel** (right, bottom): CI run history table or live output from a running target
- **Status bar**: context-sensitive keyboard shortcut hints

### Navigation & Search
- Keyboard-driven navigation with expand/collapse, Home/End, Tab to cycle panels
- Universal finder (`/`): fuzzy search popup across all projects, binaries, examples, and benchmarks with color-coded type, project, branch, and directory context
- Project list filtering with real-time fuzzy matching and match count

### Actions
- Open any project in your configured editor
- Open a project's `Cargo.toml` in your editor from any Cargo.toml-derived field in the detail panel
- Run binaries, examples, and benchmarks in debug or release mode with live output streaming
- Kill running processes and view output history
- Run `cargo clean` with confirmation dialog
- Rescan the filesystem
- Clear cached CI data per project

### CLI Modes
- `cargo-port list [--json] [--members]`: table or JSON output of discovered projects
- `cargo-port ci [--branch] [-n count] [--json]`: CI run history for a specific repo
- Default (no subcommand): launches the TUI

### Configuration
- Persistent config at platform-specific path, auto-created on first run
- In-TUI settings popup (`s`) for: invert scroll, CI run count, inline dirs, exclude dirs, include non-Rust, and editor
- All settings persist to `config.toml`

### Resilience
- Offline-first: CI data served from cache when network is unavailable, with a one-time notification
- No panics: `unwrap()` denied project-wide via clippy config
