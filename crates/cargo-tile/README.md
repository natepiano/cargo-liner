# cargo-tile

A starting template for a terminal UI built on
[`tui_pane`](../tui_pane), the `ratatui` pane framework this workspace shares
with [`cargo-port`](../cargo-port).

```bash
cargo run -p cargo-tile
```

Once installed it answers to both `cargo-tile` and `cargo tile` — cargo runs any
binary on the path whose name starts with `cargo-` as a subcommand of its own,
and the two spellings take the same arguments.

It takes over the terminal (alternate screen, raw mode) and draws one content
pane above the framework status line. The pane tiles into an animated grid of the
cargo invocations running on this machine; the sections below describe how it
behaves, and `starting a new TUI from this` covers replacing it.

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
immediately. The remaining rows report what the config holds, where each file
lives and what happened at startup; they are inert. A setting whose values are
not a fixed set -- a list of subcommands, say -- is reported rather than
stepped, and edited in `config.toml`, whose path the overlay gives.

Every setting appears in that file whether or not it was ever set: a config
written before a setting existed is rewritten at startup with the section it was
missing, at its default. A file that fails to parse is left as it is, for
whoever wrote the typo to fix.

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
runs `cargo check` per crate -- is one row there, and its own cell carries the
`runs` column counting what it is managing. The summary leaves out `runs` and
`compiler` both: a row there stands for a whole command, and those two describe
a single invocation. A shim that merely wraps cargo is not a manager and does not
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

Some commands never finish. A terminal UI reached as a cargo subcommand is open
all day, and while it is only sitting there it has no reading, no compiler and
no duration worth reading — so its own cell holds one row saying no more than
the summary's line for it already does, and that is a cell no build is getting.
Name those subcommands and the grid withholds the cell until they have work
under them:

```toml
[commands]
hidden_when_idle = ["port"]
```

The summary is not affected — a line there saying the command is running is the
whole of what it has to say, and it keeps it. The settings overlay reports the
list under **Commands**. The cell opens the moment the command drives a cargo
invocation, carrying that invocation under it, and closes through the usual fade
once the invocation ends. The list is only for commands that outlast their work:
anything that finishes on its own already leaves the grid by finishing.

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

### build progress

A compiling command shows how far along it is on the heading over it, in the
summary and in the command's own cell alike:

```
~/rust/nateroids ━━━━━━━╌╌╌╌╌╌╌╌╌╌╌╌╌  36%
```

A heading stands over every command started from one directory, so it can only
carry one reading. Where two of them are compiling in the same directory at
once, the table falls back to a `state` column, which has room for a reading on
every row:

```
pid    start  dur   state        command
41883  14:02  1:07  ████░░░ 36%  cargo build --release
```

The reading sits at the right of that field and the fill runs under it, as a
background rather than a run of glyphs, so a finished build reads `100%` on a
solid ground instead of beside one. The cell the fill has reached is drawn in
eighths: whole cells alone would move the bar once every ninth of the build,
and the reading is right-aligned, so that cell is free for most of a run.

The same column says when a command is not building at all. Cargo locks the
build directory, so a second command against the same target waits rather than
fails -- it prints `Blocking waiting for file lock on build directory` and then
nothing, which from outside is a row with a pid, a climbing duration, and no
reading, exactly like a build that has not reached its first unit yet. The
column separates them:

```
pid    start  dur   state        command
41883  14:02  1:07  ████░░░ 36%  cargo build --release
41902  14:03  0:41  blocked      cargo check --workspace
```

A wait is per row and a heading is per directory, so a blocked row brings the
column in even where every reading is already on a heading. It does not take a
heading from a reading, though: it has none of its own to put there.

The number is cargo's own. While it compiles, cargo draws
`Building [========>    ] 149/403: globset, regex-automata`, and those two
counts are units of its build plan finished and planned — a unit being one
compilation of one crate target. Nothing here estimates anything. A unit that is
already fresh counts as finished the moment cargo checks it, so an incremental
build opens near its total rather than at zero.

Reading it takes a capture, and that is what the shim is for:

```bash
cargo-tile install      # put the shim in front of cargo
cargo-tile status       # report what stands in front of each toolchain
cargo-tile uninstall    # give cargo its name back
```

`cargo tile install` and the rest work the same way.

The rows in the grid are found by scanning the process table, so they belong to
other terminals — and a process's output belongs to the terminal that started
it. Nothing outside can read it. So `cargo-tile install` moves each toolchain's
real cargo aside to `cargo-tile-real` and puts a small script in its place,
which runs the real binary under a pty and mirrors the output to
`/tmp/cargo-tile/run-<timestamp>-<pid>.log`. The grid reads the last counter out
of the tail of that log.

Without the shim nothing breaks: the `state` column simply stays out and
headings draw no rule. What a command is doing is the only thing it adds.

A run with no terminal — one started by a script, or with its output piped —
gets a bar too, but by a different route: cargo draws no progress at all
without a tty unless asked, so the shim asks, and passes a width because cargo
rejects `always` without one. Only stderr is copied there, so piped stdout
stays byte for byte what the caller expects.

Worth knowing before installing it:

- **It stands in front of every cargo run on the machine**, not just the ones
  you are watching. It changes nothing about what cargo does, prints, or exits
  with — it only copies the output aside.
- **A run already going cannot be captured.** The shim is only there for
  processes it starts, so anything mid-flight when you install shows in the grid
  without a bar until it is run again. Installing during a build is otherwise
  safe: a running cargo holds its binary open, and moving that file aside does
  not disturb it.
- **Query invocations are passed straight through** — `cargo metadata`,
  `--version`, `--message-format=json`, and the rest. They compile nothing, and
  rust-analyzer issues them constantly. So is `cargo tile` itself: capturing it
  would run the grid under `script` and log every redraw of it.
- **A nested cargo does not open a second capture.** A build script, or cargo
  driving cargo, is already inside the outer run.
- **`rustup update` replaces the shim** with a fresh cargo. Run
  `cargo-tile install` again — it is safe to repeat, and repairing that is the
  same command.
- **The real binary is only ever moved, never written over**, and anything
  holding the name without the shim's marker in it is treated as the real cargo.
  That is what makes installing twice harmless.

The shim is POSIX `sh`, and runs on macOS and Linux. The one real difference
between them is `script` itself: the BSD one takes a command and its arguments,
util-linux's takes a single command line after `-c` and needs `-e` to exit with
the child's status. The shim asks which is present — only util-linux answers
`--version` — and calls it accordingly. Where there is no `script` at all it
falls back to the no-terminal route described above rather than giving up.

A run that reached no unit and waited on no lock deletes its own log as it
ends: there is nothing in it the grid could have read, and an editor issues one
such check on every save. What is left behind is the logs of runs that actually
reported something, and whatever cleans `/tmp` on the system is what bounds
those: macOS sweeps files after a few days, and many Linux systems clear it at
boot. A run counts as live only while its marker file under
`/tmp/cargo-tile/state/pids/` exists, so deleting the logs is safe at any
time. `CARGO_TILE_ROOT` moves the whole
directory, which is how a second grid runs on captures of its own.

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
