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
remote machines in the config. Every cell carries the framework's readout on
its last row.

## summary

```text
 natedev · 3 agents
 pid      agent   name                                  status  age     directory
 1579022  claude  boss of bosses                        idle    21h     ~/rust/hana_catalyst/docs/hana
 2747564  claude  tmp cleanup then merge to berth and…  shell   2h 29m  ~/rust/cargo-handler
 4039085  codex   codex test                            —       12m     ~/rust/handler

 mac · 1 agent
 pid      agent   name                                  status  age     directory
 12055    claude  natemccoy-30                          idle    23h     ~
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
  the age of the process, and the session's directory.
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
`codex` process) or a tmux server, since those sessions are driven by something
other than a person at a terminal.

### remote machines

A remote is an ssh host name. cargo-handler runs
`ssh -o BatchMode=yes -o ConnectTimeout=5 <host> cargo-handler probe`, sharing
one connection per host through a control socket under `~/.ssh/` (left out when
that path would be too long for ssh). `probe` prints the host's own census as
one line of JSON and exits, so cargo-handler must be installed on the remote and
on the `PATH` ssh gives a non-interactive command -- `cargo install` puts it in
`~/.cargo/bin/`.

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
