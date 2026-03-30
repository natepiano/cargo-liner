# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed
- Detect new projects added to the watch directory at runtime

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
