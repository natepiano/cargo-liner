# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- First version, at `0.1.0-dev`: a terminal UI on `tui_pane`, reached as `cargo-handler` or `cargo handler`.
- A tile grid opening on a summary cell that says there is nothing to show yet. `+` opens a cell, `-` closes an empty one, the arrows and Tab move the focus ring, and a click focuses the cell under it.
- The attract screen from `tui_pane`: it comes on over the idle grid after a quiet spell, and on `a`. `r` draws one at random and `u` undoes that replacement.
- Attract favorites: `ctrl-s` saves the parameters on screen, `ctrl-o` opens the saved list, `m` shows one at random. They are kept in `favorites.toml`.
- The framework overlays: `s` settings (appearance, initial rows, file paths, notices), ctrl-k keymap editor, `?` shortcuts.
- `config.toml`, `keymap.toml` and `themes/` under `<os config dir>/cargo-handler/`, with four built-in themes mirrored as TOML under `themes/`.
- On KDE Plasma the attract screen draws the windows behind the terminal once `assets/cargo-handler.desktop.in` is installed: `scripts/install-desktop-entry.sh --crate cargo-handler`, or the script given a binary named `cargo-handler`. `KWin` grants window capture by executable path, so without the entry the backdrop is the wallpaper.
