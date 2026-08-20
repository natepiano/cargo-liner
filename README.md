# cargo-liner

[![CI](https://github.com/natepiano/cargo-liner/actions/workflows/ci.yml/badge.svg)](https://github.com/natepiano/cargo-liner/actions/workflows/ci.yml)

A fleet of cargo tools, and the terminal-UI framework they share.

## workspace members

- [cargo-port](crates/cargo-port) — a terminal dashboard for your Rust workspaces
  and projects [![crates.io](https://img.shields.io/crates/v/cargo-port.svg)](https://crates.io/crates/cargo-port)
- [cargo-tile](crates/cargo-tile) — a terminal UI cargo tool (early skeleton,
  unpublished)
- [tui_pane](crates/tui_pane) — reusable `ratatui` pane framework: keymaps, status
  bar, framework panes [![crates.io](https://img.shields.io/crates/v/tui_pane.svg)](https://crates.io/crates/tui_pane)

Each crate carries its own version, changelog, and release cadence. See
[docs/cargo-liner-consolidation.md](docs/cargo-liner-consolidation.md) for how the
workspace is put together and how releases work.
