# cargo-tile

A starting template for a terminal UI built on
[`tui_pane`](../tui_pane), the `ratatui` pane framework this workspace shares
with [`cargo-port`](../cargo-port).

```bash
cargo run -p cargo-tile
```

It takes over the terminal (alternate screen, raw mode) and draws one content
pane above the framework status line. The pane is a placeholder — replacing it
is where a new app starts.

## keys

Every binding below comes from `tui_pane::GlobalAction`'s defaults, and every
one of them is listed in the app itself: `?` opens the shortcut overlay in the
bottom-right corner, and ctrl-k opens the full keymap viewer.

| key | action |
| --- | --- |
| `?` | global shortcuts overlay |
| ctrl-k | keymap editor — every registered action, the key it resolves to, and Enter to rebind it |
| `s` | settings overlay |
| `q` | quit |
| `R` | restart — re-runs this binary with the same arguments |
| `x` / Esc | dismiss the open overlay |
| Tab / shift-Tab | cycle panes |
| ↑ ↓ | move the selection in the open overlay |
| ← → | change the selected setting |
| Enter | rebind the selected row (keymap and `?` overlays); step the selected setting forward |
| Space | step the selected setting forward |

In the settings overlay the three `[appearance]` rows are steppers, drawn as
`< value >`: stepping one writes `config.toml` and swaps the active theme
immediately. The remaining rows report where each file lives and what happened
at startup; they are inert.

## starting a new TUI from this

This crate is the workspace's TUI template — a complete `tui_pane` application
with no application in it. The tag `app-template-v1` marks the commit to start
from; `crates/tui_pane/docs/as-built/app-template.md` records what it contains
and what it deliberately leaves out.

Rendering is demand-driven — a frame is painted only when an event arrives, so
an idle app costs nothing. An app with live data marks itself dirty when that
data changes.

| to add | do this |
| --- | --- |
| a pane | add an `AppPaneId` variant (`app.rs`), give it a `Pane<App>` host and a `register_pane` call (`keymap.rs`), list it in `APP_PANE_DISPLAY_ORDER` so its shortcuts reach the keymap overlay, and lay it out in `render::draw_panes` |
| a global shortcut | add a variant to `AppGlobalAction` via `tui_pane::action_enum!`, list it in `Globals::render_order`, bind a default in `Globals::defaults`, and handle it in `globals::dispatch` (`globals.rs`) |
| a status-line slot | push a `StatusLineGlobal` in `render::draw_status_line` |
| a setting | add a row and a `SettingId` in `settings.rs` |

`AppGlobalAction` starts as an enum with no variants, so the app-globals scope
is empty until you give it one. That is why its `Action` methods are written by
hand rather than through `action_enum!` — the macro requires at least one
variant.

## configuration

Three files, all optional, under `<os config dir>/cargo-tile/` — on macOS that
is `~/Library/Application Support/cargo-tile/`, on Linux `~/.config/cargo-tile/`:

| file | purpose |
| --- | --- |
| `config.toml` | which theme to use |
| `themes/*.toml` | custom color themes |
| `keymap.toml` | key binding overrides |

### colors

`config.toml` selects a theme per appearance:

```toml
[appearance]
mode        = "auto"          # auto follows the terminal; light / dark pin one
light_theme = "Default Light"
dark_theme  = "Default Dark"
```

Four variants are compiled in: `Default Dark`, `Default Light`,
`High Contrast Dark`, and `High Contrast Light`. They are cargo-tile's own —
`tui_pane` supplies the theme machinery and none of the colors — and live in
[`src/theme/builtins.rs`](src/theme/builtins.rs), mirrored as TOML under
[`themes/`](themes/) so they can be read and copied.

For custom colors, copy [`themes/starter.toml`](themes/starter.toml) into
`themes/` under the config directory and set `dark_theme` (or `light_theme`) to
the variant name inside it. Theme files are strict: every section must be
present, and unknown keys are rejected. A theme id that matches nothing falls
back to another variant of the same appearance and the substitution is reported
in the settings overlay.

The grid draws in `pane_chrome.inactive_border`: tiles are peers, so none of
them is focused. `pane_chrome.active_border` is what a tile would light up to
if one ever carried focus.

### keys

`keymap.toml` overrides bindings by action name — for example, moving settings
onto `,`:

```toml
[global]
open_settings = ","
```

Unknown entries are skipped rather than failing startup, so a keymap written
against an older version still loads.

Editing it by hand is optional: Enter on a row in the keymap overlay (or in the
`?` overlay) captures the next keypress, checks it against every binding
already in force, and writes this file. The framework runs that whole flow —
`App` supplies only where the file lives and how to rebuild the keymap
afterwards, through `tui_pane::KeymapEditContext`.
