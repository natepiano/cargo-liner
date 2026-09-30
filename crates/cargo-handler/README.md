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
its own. Beyond `--help` and `--version` it takes one hidden subcommand,
`probe`, which the summary runs on remote machines (see below).

It takes over the terminal (alternate screen, raw mode) and draws a tile grid
above the framework status line. The grid opens on the summary cell, which
lists the Claude Code and Codex agents running on this machine and on the
remote machines in the config, and each agent gets a cell of its own showing
what it is running. Every cell carries the framework's readout on its last row.

## summary

```text
 natedev · 3 agents
 pid      agent   name                                  status  age     desktop        directory
 1579022  claude  boss of bosses                        idle    21h     boss           ~/rust/hana_catalyst/docs/hana
 2747564  claude  tmp cleanup then merge to berth and…  shell   2h 29m  berth_fix      ~/rust/cargo-handler
 4039085  codex   codex test                            —       12m     cargo handler  ~/rust/handler

 mac · 1 agent
 pid      agent   name                                  status  age     desktop        directory
 12055    claude  natemccoy-30                          idle    23h     —              ~
```

This machine comes first, under its short host name, then each remote machine in
config order. Each machine's heading is followed by its own column-label row,
with the columns sized across every machine so they line up down the cell; the
first column is the agent's pid. Within a machine the rows run oldest first.
Each name is drawn in the color of the agent's cell (see
[agent cells](#agent-cells)). A machine with no agents keeps its heading
(`natedev · no agents`) and has no label row; one that has not answered yet
says `scanning`; one whose probe failed shows the reason in the `unreachable`
color: `unreachable` (ssh exit 255), `cargo-handler not installed` (exit 127),
`probe version N, expected M`, `timed out` (no answer within 10 seconds), or
the probe's exit status. This machine is scanned every 2 seconds and each
remote every 5; a remote whose probe is still out is skipped until it answers.

Only top-level agents are listed -- the ones a person started, not the ones an
agent started:

- **Claude Code**: each `~/.claude/sessions/<pid>.json` whose process is alive
  and is `claude`. The row shows the session's name (or the start of its session
  id), its status (`busy`, `shell`, `idle`, or `—` when the record has none),
  the session's age -- from its transcript's first line, so a session resumed
  or restarted keeps its age -- the desktop its window is on (see
  [desktops](#desktops)), and the session's directory.
- **Codex**: an interactive `codex` process -- not `codex app-server`, which
  runs for another program. Its name is that of the thread it started with:
  the thread's name, else its first prompt on one line, cut at 80 characters.
  The thread is read from `~/.codex/state_<n>.sqlite`: the `codex-tui` thread
  created in the process's directory from 1 second before it started to 60
  seconds after, and when several `codex` processes share a directory, the
  thread goes to the one that started last. Without a thread the name is the
  command line after `codex`, or `pid <n>`. The one exception is the ChatGPT
  desktop app on macOS: its `codex app-server` child is listed once, named
  `ChatGPT`.

A process is dropped when any of its ancestors is another agent (a `claude` or
`codex` process): it shows up in that agent's cell instead. An agent held by a
tmux server is listed unless its launcher is found (see
[launched sessions](#launched-sessions)); an agent another agent opened in tmux
is not top level, so the summary leaves it out and it gets a cell under its
launcher's.

## agent cells

```text
 pid 1579022 · claude · busy · 23h · natedev · —
 main · ~/rust/cargo-liner
 ├─ tool-based-ui-trunk    busy  enh/trunk    ~/rust/ui-trunk    21h
 │  └─ trunk-impl          idle  —            ~/scratch/impl     2h
 └─ tool-based-ui-arrange  idle  enh/arrange  ~/rust/ui-arrange  3h

 pid      via         runs     name                             age
 4000001  shell       command  Run the tests                    2m
 3266367  session     claude   tool-based-ui-trunk              21h
 —          subagent  claude   Review the permission queue      5m 3s
 2424763      shell   command  cargo nextest run -p hana_video  45s
 3400000    session   claude   trunk-impl                       2h
 3337048  session     claude   tool-based-ui-arrange            3h
 468060     detached  codex    app-server                       1h
```

Each top-level agent gets a cell, titled in its top border with its name
(`boss of bosses` above), and the cell holds every session the agent opened in
tmux, sessions those opened included. The cells follow the machines in summary
order, each machine's top-level agents that opened sessions first, then the
rest, each oldest first. Every agent takes the next
color of a rainbow in cell order -- the cell's agent, then its sessions depth
first and oldest first -- red, orange, yellow, green, cyan, blue, violet, then
red again. The cell's title and the agent's name in the summary are drawn in
its color, so a summary row and its cell pair up at a glance, and a session's
name is drawn in its own color wherever the cell shows it. The header gives the agent's pid, program, status, age, machine and
desktop, colored as the summary colors them, then its branch and directory, `<branch> · <directory>`, and for an agent another
agent opened the one that opened it. Below that hangs the tree of the agent's
sessions: a line to each after `├─`, `└─` and `│` glyphs, giving its name,
status, branch, directory and age, with `—` for a session outside a repository.
The branch is read from the `HEAD` of the repository the directory sits in,
following a worktree's `.git` file; a detached `HEAD` shows the start of its
commit. Below it is a table of everything the agent
started that is still running, what each session runs nested under the row
naming the session. Each row's `via` says how the agent holds it,
indented two cells under the row that started it; `runs` says what it is,
`command` for a shell and otherwise the program, `claude` or `codex`, drawn in
that program's color. A row that has no process of its own shows `—` for its
pid. An agent running nothing says `nothing running`.

A cell is laid out for its width, spending rows where a narrow cell would
otherwise cut its header, its branch and directory, a session's line or a name
short. The same cell 38 columns wide, cut here after its first rows:

```text
 agent    claude
 pid      1579022
 status   busy
 age      23h
 machine  natedev
 desktop  —
 main · ~/rust/cargo-liner
 ├─ tool-based-ui-trunk
 │  busy · 21h
 │  enh/trunk · ~/rust/ui-trunk
 │  └─ trunk-impl
 │     idle · 2h
 │     ~/scratch/impl
 └─ tool-based-ui-arrange
    idle · 3h
    enh/arrange · ~/rust/ui-arrange

 shell · command · 2m · pid 4000001
   Run the tests
 session · claude · 21h · pid 3266367
   tool-based-ui-trunk
   subagent · claude · 5m 3s
     Review the permission queue
     shell · command · 45s
     pid 2424763
       cargo nextest run -p hana_video
```

Where the header's one line would be cut, it stands as a block, one fact to a
line after a label column: the agent, its pid, status, age, machine and
desktop, each value in the color the line gives it. A directory
too long for its line breaks after a `/`, onto as many lines as it takes. Where
the table would cut a name, each row stands as an entry of its own: its `via`,
indented as the table indents it, then its `runs`, its age and its pid when it
has one, and below that its name in full, indented under the `via` and broken
before a space or after a `/` when it is still too long.

The label color is kept for labels: the block's label column, the table's
column headers and `launched by`. Values, the `pid` before a number, the ` · `
between facts and notes such as `nothing running` are drawn in plain text or
their own colors, never in it, and every built-in theme gives labels a color
nothing else uses.

Each kind of row comes from its own place:

- **shell** running **command**: a command the agent's shell tool is running: a child of the
  `claude` process running Claude Code's shell wrapper
  (`zsh -c "source …/shell-snapshots/snapshot-… && eval '<command>'"`). It is
  named by the `description` of the Bash call in the session's transcript whose
  command is exactly the wrapper's, written at most 5 seconds after the shell
  started; a call written without a description, or none found, leaves the
  command itself on one line. A call found in a subagent's transcript puts the
  shell under that subagent.
- **shell** running **claude** / **codex**: a Claude Code or Codex process
  found below a shell, one level under it. A `codex app-server` is named
  `app-server`, another Codex by its arguments, and a Claude Code process by
  its session's name. Each is laid out the same way in turn.
- **direct**: a Claude Code or Codex process the agent started itself rather
  than through a shell, such as a `codex mcp-server`, named and laid out as one
  below a shell is.
- **detached**: a Codex app server moved out from under the shell that
  started it, found by the `CLAUDE_PID` in its environment. It sits directly
  under the agent that variable names.
- **thread** running **codex**: a thread a Codex app server holds open, on Linux only: read from
  the conversation files under `~/.codex/sessions/` among the server's open
  files, and named from Codex's thread database.
- **subagent** running **claude**: a subagent running inside the agent, read from the
  `subagents/` directory beside its transcript. A subagent whose transcript has
  ended its turn, or gone 30 minutes unwritten, is no longer listed.
- **session**: an agent this one opened in tmux, which has its own cell,
  running its program.

### launched sessions

A tmux server runs apart from whoever started it, so nothing in the process
tree says which agent opened a session it holds. For each agent held by tmux,
the transcripts of the other Claude Code agents -- one whose process restarted
since, as a session resumed in a new process does, included -- are read for Bash
calls from 10 minutes before it started to 5 seconds after that run tmux
`new-session`. A call whose `new-session` commands all name their session with
a literal `-s` other than the session's own name is passed over, unless the
call mentions that name elsewhere, as Claude Code's `-n` would. Among the rest,
a call naming the session or its directory wins, and then the latest. The
agent that wrote the winning call is the launcher. This has limits:

- A launch not written in the transcript of a running agent -- one typed at a
  terminal, run from a script started elsewhere, or made by an agent that has
  since exited -- finds no launcher, and the session is listed as top level.
- A launch that names its session through a variable, as a loop does, is
  matched by time alone: another agent's unnamed `new-session` written closer
  to the session's start wins it.
- A session with no name of its own is matched by time alone as well.

### remote machines

A remote is an ssh host name. cargo-handler runs
`ssh -o BatchMode=yes -o ConnectTimeout=5 <host> cargo-handler probe`, sharing
one connection per host through a control socket under `~/.ssh/` (left out when
that path would be too long for ssh). `probe` prints the host's own census as
one line of JSON and exits, so cargo-handler must be installed on the remote and
on the `PATH` ssh gives a non-interactive command -- `cargo install` puts it in
`~/.cargo/bin/`.

### desktops

The `desktop` column, and the last part of each agent cell's first header line,
name the KDE virtual desktop the agent's terminal window is on: `cargo handler`,
`berth_fix`, `bevy 0.20.0-rc.1`. A window on several desktops lists them joined
with `, `, and one on every desktop says `all desktops`. At most every 5
seconds, and only while there are agents, cargo-handler runs one `KWin` script
over the session bus that lists every window's pid, caption and desktops. The
window is matched to the agent this way:

- The agent's terminal is the nearest process at or above it that owns a
  window. A terminal with one window gives that window.
- A terminal that owns several windows, such as a single-instance Ghostty, gives
  the one whose caption is the session's name after the status glyph Claude Code
  writes ahead of it (`✳ boss of bosses`, `◐ enh/handler`), or, for Codex, the
  thread's name ahead of ` | ` and the directory's (`codex test | handler`). No
  such caption, or more than one, gives `—`. A session whose window shows
  another tab's title, or a title that is not the session's name, gives `—`
  too.
- An agent a tmux server holds is found through the pane it runs in, that pane's
  session, the clients showing that session (`tmux list-panes -a` and
  `tmux list-clients`), and each client's terminal window, by the same two
  rules. A detached session gives `—`. Only the default tmux socket is read,
  through the program the server was started as when that is an absolute path,
  since tmux need not be on `PATH`.

Off Linux, outside KDE Plasma, or where `KWin` does not answer, every desktop is
`—`. The probe carries each row's `desktop`, so a remote Linux machine under KDE
fills it in; the Mac shows `—`.

## keys

Every binding is listed in the app itself: `?` opens the shortcut overlay, and
ctrl-k opens the full keymap editor, where Enter rebinds the selected row.

### App Shortcuts

| key | action |
| --- | --- |
| `+` | add a tile |
| `-` | remove an empty tile |
| ← → ↑ ↓ | move the focus ring between tiles |
| `a` | show the attract screen, or give the grid back until a few idle seconds pass |
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

The attract screen comes on by itself only when no machine lists an agent,
after a few quiet seconds; an agent sitting `idle` still counts, since it is
waiting on its reader. `a` shows it on demand at any time.

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
| `config.toml` | theme selection, the iTerm2 profile, how the grid grows, and the remote machines |
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
transparent    = true            # false paints the grid solid in the theme's background

[tiles]
initial_rows  = 4                # rows the first column grows to before the grid squares up
fill          = "redistribute"   # redistribute / add_new: how the cells spread over the columns
widen_summary = true             # the summary reaches across columns until its widest line fits

[machines]
remote = []                      # ssh host names the summary probes, e.g. ["mac"]
```

`fill` decides how many cells each column holds; the number of columns is the
same either way. `redistribute` keeps every column within one cell of the
others, the taller ones on the left: twelve cells stand as three columns of
four, the thirteenth opens a fourth column and deals the thirteen out as four,
three, three and three, and the fourteenth makes that four, four, three and
three. `add_new` fills a column at a time, so the thirteenth cell stands alone
in the fourth column, the whole height of the grid.

`widen_summary` lets the summary reach past its own column when its widest line
-- a machine's heading, or an agent's row with its directory written out in full
-- does not fit there. It keeps the height its own column gives it and takes the
top of each next column in turn until that line fits or it reaches the right
edge; the cells of a column it covers divide what is left below it, and a column
whose cells would no longer fit below it stops the summary there. Off, the
summary stays in its own column and cuts a long directory short. The settings
overlay steps `initial rows`, `fill` and `widen summary` under **Tiles**.

`transparent` paints nothing under the grid, so a transparent terminal window
shows the desktop behind every cell, and the focused cell's border lights to
mark focus. Off, the grid is painted solid in the theme's background while its
cells are shown, and focus is a tint under the focused cell's contents. The
settings overlay steps it under **Appearance**.

The settings overlay edits `remote` under **Machines** as a typed list; a
change applies from the next probe.

Four themes are compiled in: `Default Dark`, `Default Light`,
`High Contrast Dark` and `High Contrast Light`. They live in
[`src/theme.rs`](src/theme.rs), mirrored as TOML under [`themes/`](themes/).
For custom colors, copy [`themes/starter.toml`](themes/starter.toml) into
`themes/` under the config directory and point `dark_theme` at
`Cargo Handler Dark`.

The summary reads thirteen keys from each variant's `[variants.roles]`:
`claude` and `codex` color the agent column, `busy`, `shell` and `idle` the
status column (a missing or unknown status uses `idle`), `unreachable` a
remote's failure reason, and `rainbow_red`, `rainbow_orange`,
`rainbow_yellow`, `rainbow_green`, `rainbow_cyan`, `rainbow_blue` and
`rainbow_violet` the colors the agent cells take in turn, for the name column
and each cell's title. A custom variant that leaves one out takes it from the built-in
it replaces, or, under a new name, from `Default Dark` or `Default Light` to
match its appearance.

A stale entry in `keymap.toml` is skipped rather than refusing to start.

### KDE Plasma

On KDE Plasma under Wayland, the attract screen draws the windows standing
behind the terminal only when KWin lets cargo-handler capture them, and KWin
decides that from a desktop entry naming the binary's exact path. Without the
entry the backdrop falls back to the wallpaper, and nothing on screen says why.
Install the entry from a checkout of this repository:

```bash
scripts/install-desktop-entry.sh --crate cargo-handler   # the cargo-handler on PATH
scripts/install-desktop-entry.sh target/release/cargo-handler cargo-handler-dev
```

The script installs cargo-tile's entry by default. It picks cargo-handler's
from a binary named `cargo-handler`, so a build under any other name needs
`--crate cargo-handler` as well. Run it again after the binary moves or is
reinstalled, and give each build an entry name of its own.
