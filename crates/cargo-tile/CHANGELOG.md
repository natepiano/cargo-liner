# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Establish cargo-tile as the starting point for a new `tui_pane` application, tagged `app-template-v1`. The crate is a complete TUI with no application in it: framework globals, the settings / keymap / global-shortcuts overlays, live theming, a status line, restart in place, demand-driven rendering, and input on its own thread. See `crates/tui_pane/docs/as-built/app-template.md`.
- Edit key bindings from the keymap overlay: Enter on a row captures the next keypress, checks it against every binding in force, writes `keymap.toml`, and reloads. The `?` overlay hands off to the same editor with the selected row already open.
