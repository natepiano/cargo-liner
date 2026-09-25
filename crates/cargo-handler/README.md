# cargo-handler

A terminal UI cargo tool, built on [`tui_pane`](../tui_pane), the `ratatui`
pane framework this workspace shares with [`cargo-tile`](../cargo-tile) and
[`cargo-port`](../cargo-port).

```bash
cargo install --path crates/cargo-handler
cargo handler
```

Once installed it answers to both `cargo-handler` and `cargo handler` -- cargo
runs any binary on the path whose name starts with `cargo-` as a subcommand of
its own. It takes no arguments beyond `--help` and `--version`.

It takes over the terminal (alternate screen, raw mode) and draws a tile grid
above the framework status line. The grid opens on the summary cell, which says
there is nothing to show yet. Every cell carries the framework's readout on its
last row.

## keys

Every binding is listed in the app itself: `?` opens the shortcut overlay, and
ctrl-k opens the full keymap editor, where Enter rebinds the selected row.

### App Shortcuts

| key | action |
| --- | --- |
| `+` | add a tile |
| `-` | remove an empty tile |
| ← → ↑ ↓ | move the focus ring between tiles |
| `a` | show or hide the attract screen |
| `r` | draw a random attract screen |
| `u` | undo the latest attract replacement |
| ctrl-s | save the attract parameters on screen as a favorite |
| ctrl-o | open the saved favorites |
| `m` | show a random favorite |

### framework

| key | action |
| --- | --- |
| `?` | global shortcuts overlay |
| ctrl-k | keymap editor |
| `s` | settings overlay |
| `q` | quit |
| `R` | restart -- re-runs this binary with the same arguments |
| Tab / shift-Tab | move the focus ring through the tiles |
| Esc | close the open overlay |

### attract screen and favorites

While the attract screen was asked for with `a`, it takes the keys it binds
ahead of the grid: `1` `2` `3` pick the animation, the arrows point it, `<` `>`
change its speed, and each animation has keys of its own under
`[attract_moving_band]`, `[attract_moving_text]` and `[attract_pixelate]` in
`keymap.toml`. The favorites overlay binds its keys under `[favorites]`: Enter
loads the selected row, `x` deletes it, Esc closes the overlay.

## configuration

Everything lives under `<os config dir>/cargo-handler/` -- on macOS
`~/Library/Application Support/cargo-handler/`, on Linux
`~/.config/cargo-handler/`:

| file | purpose |
| --- | --- |
| `config.toml` | theme selection, the iTerm2 profile, and how the grid grows |
| `keymap.toml` | key binding overrides, written by the keymap editor |
| `favorites.toml` | saved attract modes and parameters |
| `themes/*.toml` | custom color themes |

`config.toml` is not written until a setting is stepped in the settings
overlay. Its defaults:

```toml
[appearance]
mode           = "auto"          # auto follows the terminal; light / dark pin one
light_theme    = "Default Light"
dark_theme     = "Default Dark"
iterm2_profile = "cargo-handler" # "" to leave the iTerm2 session alone

[tiles]
initial_rows = 4                 # rows the first column grows to before the grid squares up
```

Four themes are compiled in: `Default Dark`, `Default Light`,
`High Contrast Dark` and `High Contrast Light`. They live in
[`src/theme.rs`](src/theme.rs), mirrored as TOML under [`themes/`](themes/).
For custom colors, copy [`themes/starter.toml`](themes/starter.toml) into
`themes/` under the config directory and point `dark_theme` at
`Cargo Handler Dark`.

A stale entry in `keymap.toml` is skipped rather than refusing to start.
