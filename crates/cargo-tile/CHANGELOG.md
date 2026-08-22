# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-08-21

### Added
- Establish cargo-tile as the starting point for a new `tui_pane` application, tagged `app-template-v1`. The crate is a complete TUI with no application in it: framework globals, the settings / keymap / global-shortcuts overlays, live theming, a status line, restart in place, demand-driven rendering, and input on its own thread. See `crates/tui_pane/docs/as-built/app-template.md`.
- Edit key bindings from the keymap overlay: Enter on a row captures the next keypress, checks it against every binding in force, writes `keymap.toml`, and reloads. The `?` overlay hands off to the same editor with the selected row already open.
- Show the cargo invocations running on this machine, one row per invocation, grouped by the working directory they were started from and ordered by path. A process is classified as cargo by its argv rather than its process name, so a wrapper or a renamed binary is still attributed correctly, and a start-time tie between two candidates resolves toward the newer pid.
- Tile the pane into an animated grid of cells, one per running invocation. The layout is a pure function of the cell count, growing by greedy fill up to `initial_rows` squared and then toward the next square. A cell moving between columns is drawn as two placements -- the piece leaving the old column and the piece arriving in the new one -- so it reads as sliding rather than jumping, and transition progress is fixed-point so the animation is deterministic. A command finishing in the middle empties its cell and the grid closes up around it, with focus following its cell.
- Show the running cargo-tile version in the pane title.
- Own theme content in the app rather than the framework: the crate ships its own theme variants, and its grid draws on the shared pane border in the inactive shade regardless of focus.
