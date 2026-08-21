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

The grid draws in `pane_chrome.inactive_border`, focus or no focus. A border
is a cell two tiles share, so lighting it for one takes the boundary away from
the other; the focused tile is marked by the background tint under its contents
instead. There is no focused-border colour in the theme.

Both states are painted, not just the focused one. A cell with no background of
its own is the terminal's *default* background, which a transparent window
composites differently from a painted cell -- so leaving unfocused tiles bare
would make focus read as a difference in opacity rather than of colour. Focus is
carried by how far each tile's tint is pushed from the theme background.

### iTerm2

iTerm2 keeps window transparency, blur, and the background image on the profile,
where no escape sequence can reach them. So cargo-tile asks the session to wear
a profile that already carries them, and gives it back on exit:

```toml
[appearance]
iterm2_profile = "cargo-tile"   # "" to leave the session alone
```

Make a profile by that name in iTerm2 (Settings -> Profiles -> **+**) and set it
up however you like. To see the tints above through a transparent window, untick
**"Only the default background color uses transparency"** there -- ticked, it
means painted cells are opaque, and every cell in the grid is painted.

The profile to return to is read from `ITERM_PROFILE`, which iTerm2 sets in
every session. If it is missing, cargo-tile does not switch at all rather than
risk leaving the shell somewhere it cannot be brought back from. Every terminal
that is not iTerm2 is left alone.

### the grid

Cell one is the summary: one row per cargo command running. A command that
drives other cargo commands -- `cargo mend` running a `cargo nextest` suite that
runs `cargo check` per crate -- is one row there, with the `sub` column counting
what it is managing. A shim that merely wraps cargo is not a manager and does not
get its own row; it collapses onto the cargo actually doing the work. A tool
installed as an external subcommand counts as the command it is: `cargo nextest
run` replaces the `cargo` process with `cargo-nextest` rather than spawning it,
so the name is all that is left to recognise it by.

The summary leaves out `--manifest-path` and the path behind it. Every row there
already sits under the working directory heading its group, and the manifest is
an absolute path -- long enough to push the subcommand off the edge of a narrow
cell to say again what the header just said. A command's own cell shows the whole
line.

Every summary row also gets its own cell, carrying the invocations that row
stands for: one for a plain command, many for a manager. `+` opens an empty cell
at the end and `-` closes one: the focused cell when `+` opened it, otherwise the
last one still standing empty. Only an empty cell goes -- a cell carrying a
command is the display itself, and the summary is not removable at all. A command
arriving takes the first empty cell waiting, or opens its own. `initial_rows` sets
how many rows a column fills before the next one opens, and the motion follows
from that.

```toml
[tiles]
initial_rows = 3
fade_seconds = 3
```

`fade_seconds` is how long a finished invocation stays on screen, greyed, before
it goes -- and before the cell it was holding closes and the cells after it move
up. Zero drops it on the scan that notices.

The grid moves one cell at a time. A command finishing in the middle empties its
cell where it stood, and the hole trades places with the cell after it, then the
one after that, until it reaches the end and the grid closes up -- a change
propagates through the cells rather than sliding all of them past each other at
once. A whole suite ending at once is more steps than anyone would watch, so past
a point the grid gives that up and settles the rest in one move.

The summary holds focus to begin with, and the arrow keys move it: left and right
between columns, up and down within one. Clicking a cell is the other way onto
it. The focused cell is the one drawn in the theme's active shade, and it closes
its own corners: the borders it shares with its neighbours are still one line
each, but the four ends of them are that cell's corners rather than the junctions
they would otherwise read as, so the focused cell reads as one closed box sitting
inside the grid. Focus follows its cell as the grid closes up around it, and falls
back to the summary when that cell goes.

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
