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
first column is the agent's pid. Within a machine the rows run oldest first. A
machine with no agents keeps its heading (`natedev · no agents`) and has no
label row; one that has not answered yet says `scanning`; one whose probe failed
shows the reason in the `unreachable` color: `unreachable` (ssh exit 255),
`cargo-handler not installed` (exit 127), `probe version N, expected M`,
`timed out` (no answer within 10 seconds), or the probe's exit status. This
machine is scanned every 2 seconds and each remote every 5; a remote whose probe
is still out is skipped until it answers.

Only top-level agents are listed -- the ones a person started, not the ones an
agent started:

- **Claude Code**: each `~/.claude/sessions/<pid>.json` whose process is alive
  and is `claude`. The row shows the session's name (or the start of its session
  id), its status (`busy`, `shell`, `idle`, or `—` when the record has none),
  the age of the process, the desktop its window is on (see
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
 pid 3266367 · claude · busy · 23h · natedev · berth_fix
 ~/rust/tool-based-ui-trunk
 launched by boss of bosses

 pid      kind        name                                         age
 2371669  shell       Launch the Phase 1 implementation seat       12m
 2372720    codex     app-server                                   12m
 —            thread  tool-based-ui-trunk-impl                     11m
 —        subagent    Review the permission queue                  5m 3s
 2424763    shell     cargo nextest run -p hana_video --no-fail-…  45s
 3337048  session     tool-based-ui-arrange                        30s
```

Every agent someone can talk to gets a cell, titled in its top border with its
name (`tool-based-ui-trunk` above): each
top-level agent, and each session another agent opened in tmux. The cells
follow the machines in summary order; within a machine each top-level agent
comes oldest first, followed by the sessions it opened, depth first and oldest
first. The header gives the agent's pid, program, status, age, machine and
desktop, colored as the summary colors them, then its directory, and for a launched
session the agent that opened it. Below it is a table of everything the agent
started that is still running, each row's `kind` indented two cells under the
row that started it, with `—` for a row that has no process of its own. An
agent running nothing says `nothing running`. A name too long for the room the
other columns leave is cut and ends in `…`.

Each kind of row comes from its own place:

- **shell**: a command the agent's shell tool is running: a child of the
  `claude` process running Claude Code's shell wrapper
  (`zsh -c "source …/shell-snapshots/snapshot-… && eval '<command>'"`). It is
  named by the `description` of the Bash call in the session's transcript whose
  command is exactly the wrapper's, written at most 5 seconds after the shell
  started; a call written without a description, or none found, leaves the
  command itself on one line. A call found in a subagent's transcript puts the
  shell under that subagent.
- **claude** / **codex**: a Claude Code or Codex process found below a shell,
  one level under it. A `codex app-server` is named `app-server`, another Codex
  by its arguments, and a Claude Code process by its session's name. Each is
  laid out the same way in turn. A Codex app server moved out from under the
  shell that started it is found by the `CLAUDE_PID` in its environment, and
  sits directly under the agent that variable names.
- **thread**: a thread a Codex app server holds open, on Linux only: read from
  the conversation files under `~/.codex/sessions/` among the server's open
  files, and named from Codex's thread database.
- **subagent**: a subagent running inside the agent, read from the
  `subagents/` directory beside its transcript. A subagent whose transcript has
  ended its turn, or gone 30 minutes unwritten, is no longer listed.
- **session**: an agent this one opened in tmux, which has its own cell.

### launched sessions

A tmux server runs apart from whoever started it, so nothing in the process
tree says which agent opened a session it holds. For each agent held by tmux,
the transcripts of the Claude Code agents started before it are read for Bash
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

[tiles]
initial_rows = 4                 # rows the first column grows to before the grid squares up

[machines]
remote = []                      # ssh host names the summary probes, e.g. ["mac"]
```

The settings overlay edits `remote` under **Machines** as a typed list; a
change applies from the next probe.

Four themes are compiled in: `Default Dark`, `Default Light`,
`High Contrast Dark` and `High Contrast Light`. They live in
[`src/theme.rs`](src/theme.rs), mirrored as TOML under [`themes/`](themes/).
For custom colors, copy [`themes/starter.toml`](themes/starter.toml) into
`themes/` under the config directory and point `dark_theme` at
`Cargo Handler Dark`.

The summary reads six keys from each variant's `[variants.roles]`: `claude`
and `codex` color the agent column, `busy`, `shell` and `idle` the status
column (a missing or unknown status uses `idle`), and `unreachable` a remote's
failure reason. A custom variant that leaves one out takes it from the built-in
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
