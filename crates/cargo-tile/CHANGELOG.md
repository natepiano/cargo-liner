# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Report what sccache is doing along the summary cell's top border: hit rate, the hits and misses behind it, and the disk the cache occupies against the ceiling it evicts at. Only when a server is already running -- reading the stats would otherwise start one -- and the fields drop out in order as the cell narrows.
- Show how far along a compiling invocation is, from cargo's own unit counter. The reading rules the working-directory heading over the command, in the summary and in the command's own cell alike, which costs the table no width.
- Capture cargo's output so progress can be read at all, through a shim: `cargo-tile install` moves each toolchain's real cargo aside, runs it under a pty, and mirrors the output to `/tmp/cargo-tile/`. `status` reports what stands in front of each toolchain and `uninstall` undoes it.
- Answer to `cargo tile` as well as `cargo-tile`.
- Report progress for runs with no terminal too, by asking cargo for the bar it would otherwise draw only for a tty.
- Say when a command is `blocked` on another cargo's build-directory lock, which from outside looks exactly like a build that has not reached its first unit.
- Withhold a command's own cell while nothing is running under it, for the subcommands named in `commands.hidden_when_idle` -- `port` to begin with. The summary line stays.
- List `commands.hidden_when_idle` in the settings overlay, and write the config file at startup when it does not already name every setting.
- Carry a `cpu` column. It is the whole command's share rather than the cargo process's own -- `top`'s scale, so a build across eight cores reads past 700% -- settled over a two-second window and held for a second at a time.
- Report test progress, not just build progress. `cargo nextest run` counts its tests the way cargo counts units, and the working-directory heading names which count is on screen: `building`, then `testing`. Runs with no terminal report too -- nextest draws no bar there, so the count is read from its per-test lines instead.

### Fixed
- Report the state of an invocation the lead command is driving, not just the lead's own. A nested cargo -- `cargo doc` under a `cargo port` lint run, say -- waiting on the build-directory lock now says `blocked` on its own row; before, every row but the lead was left blank. A row with no capture of its own stays blank rather than borrowing the reading from the row above it.

### Changed
- Draw every reading on the working-directory heading and nowhere else. The `state` column used to grow a per-row bar wherever one directory had two commands reporting at once -- but nothing in a Rust build gets past the build-directory lock, so the second command is waiting rather than compiling and that case does not arise. The column now says one thing, `blocked`, and joins the table only when a row is waiting.
- Make Tab and Shift-Tab walk the tile grid. The status line advertised them all along, but the app registers one pane, so the step had nowhere to go and did nothing. They now step the focus ring cell by cell in the grid's own order, wrapping at either end, where the arrows read it as rows and columns.
- Fade a finished row into the cell it stands on, rather than switching it to grey and holding that shade until it vanishes. The heading and column labels travel with the least-faded row under them.
- Hold the summary cell's title off the corner glyph, with a space ahead of the word.
- Wrap a command line too long for its column instead of cutting it off at the edge.
- Close a cell by having the grid come together over it, rather than trading the hole through every cell after it.
- Draw a cell crossing between columns in the columns as they stand partway through the move, rather than as they were before it.
- Keep a cell carried by a closing column from travelling up or down as that column is pushed off the edge.
- Rename the `sub` column to `runs`.
- Put `command` ahead of `compiler` and `runs` in a command's own cell, so the command line starts in the same column it does in the summary.
- Leave `compiler` and `runs` out of the summary, where a row stands for a whole command rather than for one invocation.
- Start the working-directory headings and the rows under them one space in from the cell border, rather than two and four.
- Reject arguments that are not a subcommand. 0.1.0 parsed no command line at all, so anything after the binary name was ignored.
- Head a command's own cell with the directory the command itself runs in. A test run drives cargo in a temporary directory per case; those sorted ahead of a home-relative path and pushed the run being watched below the fold of its own cell.

### Notes
- Installing the shim is always explicit, never done on startup. It stands in front of every cargo run on the machine and changes nothing about what cargo does, prints, or exits with. Query invocations, this workspace's own terminal UIs, and nested cargos pass straight through.
- A `--message-format=json` run is captured down the no-terminal path, which mirrors stderr alone, so the JSON on stdout reaches its caller byte for byte.
- The shim publishes itself as `CARGO`, which keeps the tools that honour that over the path -- `cargo-clippy`, `cargo-nextest` -- coming back through it.
- A run already going cannot be captured. Installing during a build is otherwise safe: a running cargo holds its binary open.
- `rustup update` replaces the shim with a fresh cargo. Running `cargo-tile install` again repairs it, and is safe to repeat.
- The shim is POSIX `sh` and takes either `script` implementation, both told to flush the log on every write.
- A run that reached no unit and waited on no lock deletes its own log as it ends, having recorded nothing the grid could read.
- Without the shim installed nothing breaks: the `state` column stays out and headings draw no rule.

## [0.1.0] - 2026-08-21

### Added
- Establish cargo-tile as the starting point for a new `tui_pane` application, tagged `app-template-v1`. The crate is a complete TUI with no application in it: framework globals, the settings / keymap / global-shortcuts overlays, live theming, a status line, restart in place, demand-driven rendering, and input on its own thread. See `crates/tui_pane/docs/as-built/app-template.md`.
- Edit key bindings from the keymap overlay: Enter on a row captures the next keypress, checks it against every binding in force, writes `keymap.toml`, and reloads. The `?` overlay hands off to the same editor with the selected row already open.
- Show the cargo invocations running on this machine, one row per invocation, grouped by the working directory they were started from and ordered by path. A process is classified as cargo by its argv rather than its process name, so a wrapper or a renamed binary is still attributed correctly, and a start-time tie between two candidates resolves toward the newer pid.
- Tile the pane into an animated grid of cells, one per running invocation. The layout is a pure function of the cell count, growing by greedy fill up to `initial_rows` squared and then toward the next square. A cell moving between columns is drawn as two placements -- the piece leaving the old column and the piece arriving in the new one -- so it reads as sliding rather than jumping, and transition progress is fixed-point so the animation is deterministic. A command finishing in the middle empties its cell and the grid closes up around it, with focus following its cell.
- Show the running cargo-tile version in the pane title.
- Own theme content in the app rather than the framework: the crate ships its own theme variants, and its grid draws on the shared pane border in the inactive shade regardless of focus.
