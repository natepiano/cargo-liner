# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Show how far along a compiling invocation is. The reading is cargo's own: while it compiles it draws `Building [========>    ] 149/403: globset, regex-automata`, and those are units of its build plan finished and planned, so nothing is estimated. The summary grows a `done` column drawing the percentage beside a six-cell bar at eighth-block resolution, so it still moves on every unit, and a command's own cell rules the same reading along its working-directory heading. The column joins only where some command on the cell has a capture behind it, so a narrow tile spends no width on a column of dashes.
- Capture cargo's output so that progress can be read at all, through a shim cargo-tile installs itself: `cargo-tile install` moves each toolchain's real cargo aside to `cargo-tile-real` and takes its name, running it under a pty and mirroring the output to `/tmp/cargo-tile/run-<timestamp>-<pid>.log`. `cargo-tile status` reports what stands in front of each toolchain and `cargo-tile uninstall` gives cargo its name back. The rows in the grid are found by scanning the process table and so belong to other terminals, and a process's output belongs to the terminal that started it -- which is why reaching it takes standing in front of cargo rather than asking it for anything.
- Answer to `cargo tile` as well as `cargo-tile`. Cargo runs any `cargo-`prefixed binary on the path as a subcommand, handing it that subcommand name ahead of every other argument, so the command line drops that word and both spellings take the same arguments. This kept working when argument parsing arrived: before it there was none, and the extra word went unread.
- Report progress for runs with no terminal as well, by asking cargo for a progress bar it would otherwise draw only for a tty. Cargo refuses `always` unless a width comes with it, so the shim sets both.

### Notes
- Installing the shim is always explicit, never done on startup: it stands in front of every cargo invocation on the machine. It changes nothing about what cargo does, prints, or exits with. Query invocations (`cargo metadata`, `--version`, `--message-format=json`) pass straight through, as does `cargo tile` itself -- capturing the grid would run a terminal UI under `script` and copy every redraw of it into a log -- and a nested cargo -- a build script, or cargo driving cargo -- does not open a second capture.
- A run already going cannot be captured. A shim is only ever there for the processes it starts, so anything mid-flight when it is installed shows in the grid without a bar until it is run again. Installing while a build runs is otherwise safe: a running cargo holds its binary open, so moving that file aside does not disturb it.
- `rustup update` replaces the shim with a fresh cargo. Running `cargo-tile install` again repairs it, and is safe to repeat: the real binary is only ever moved, never written over, and anything holding the name without the shim's marker in it is treated as the real cargo.
- The shim is POSIX `sh` and runs on macOS and Linux. The two `script` implementations disagree -- the BSD one takes a command and its arguments, util-linux's takes a single command line after `-c` and needs `-e` to exit with the child's status -- so the shim settles which is present before calling it, and falls back to the no-terminal path where there is no `script` at all.
- Without the shim installed nothing breaks -- the `done` column stays out of the summary and headings draw no rule.

## [0.1.0] - 2026-08-21

### Added
- Establish cargo-tile as the starting point for a new `tui_pane` application, tagged `app-template-v1`. The crate is a complete TUI with no application in it: framework globals, the settings / keymap / global-shortcuts overlays, live theming, a status line, restart in place, demand-driven rendering, and input on its own thread. See `crates/tui_pane/docs/as-built/app-template.md`.
- Edit key bindings from the keymap overlay: Enter on a row captures the next keypress, checks it against every binding in force, writes `keymap.toml`, and reloads. The `?` overlay hands off to the same editor with the selected row already open.
- Show the cargo invocations running on this machine, one row per invocation, grouped by the working directory they were started from and ordered by path. A process is classified as cargo by its argv rather than its process name, so a wrapper or a renamed binary is still attributed correctly, and a start-time tie between two candidates resolves toward the newer pid.
- Tile the pane into an animated grid of cells, one per running invocation. The layout is a pure function of the cell count, growing by greedy fill up to `initial_rows` squared and then toward the next square. A cell moving between columns is drawn as two placements -- the piece leaving the old column and the piece arriving in the new one -- so it reads as sliding rather than jumping, and transition progress is fixed-point so the animation is deterministic. A command finishing in the middle empties its cell and the grid closes up around it, with focus following its cell.
- Show the running cargo-tile version in the pane title.
- Own theme content in the app rather than the framework: the crate ships its own theme variants, and its grid draws on the shared pane border in the inactive shade regardless of focus.
