# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- First version, at `0.1.0-dev`: a terminal UI on `tui_pane`, reached as `cargo-handler` or `cargo handler`.
- A tile grid opening on the summary cell. `+` opens a cell, `-` closes an empty one, the arrows and Tab move the focus ring, and a click focuses the cell under it.
- The summary cell lists the top-level Claude Code and Codex agents on this machine and on each remote machine: pid, agent, name, status, age and directory, oldest first under a heading and a column-label row per machine. An agent started by another agent, or held by a tmux server, is left out; so is a `codex app-server`, except the one the ChatGPT desktop app runs on macOS, listed as `ChatGPT`.
- A Codex row is named for the thread its `codex` started, matched in `~/.codex/state_<n>.sqlite` by directory and start time: the thread's name, else its first prompt, else the command line after `codex` or `pid <n>`.
- `[machines] remote` in `config.toml`, edited under Machines in the settings overlay: ssh host names probed every 5 seconds with `ssh <host> cargo-handler probe`, over a shared control socket. A remote that fails keeps its heading with the reason: `unreachable`, `cargo-handler not installed`, a probe version mismatch, `timed out`, or the probe's exit status.
- A hidden `probe` subcommand that prints this machine's agents as one line of JSON.
- Theme roles `claude`, `codex`, `busy`, `shell`, `idle` and `unreachable` under `[variants.roles]`, set in all four built-in themes.
- The attract screen from `tui_pane`: it comes on over the idle grid after a quiet spell, and on `a`. `r` draws one at random and `u` undoes that replacement.
- Attract favorites: `ctrl-s` saves the parameters on screen, `ctrl-o` opens the saved list, `m` shows one at random. They are kept in `favorites.toml`.
- The framework overlays: `s` settings (appearance, initial rows, remote machines, file paths, notices), ctrl-k keymap editor, `?` shortcuts.
- `config.toml`, `keymap.toml` and `themes/` under `<os config dir>/cargo-handler/`, with four built-in themes mirrored as TOML under `themes/`.
- On KDE Plasma the attract screen draws the windows behind the terminal once `assets/cargo-handler.desktop.in` is installed: `scripts/install-desktop-entry.sh --crate cargo-handler`, or the script given a binary named `cargo-handler`. `KWin` grants window capture by executable path, so without the entry the backdrop is the wallpaper.
