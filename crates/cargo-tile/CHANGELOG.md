# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- On KDE Plasma the attract screen animates over what is really behind the window -- the other windows standing there, over the actual wallpaper. `KWin` grants the `org.kde.KWin.ScreenShot2` interface by executable path and prompts for nothing, so `scripts/install-desktop-entry.sh` installs the entry that asks for it; without it the attract screen behaves as before.
- The grid puts the capture shim in front of cargo as it opens, so progress bars work from the first launch and come back after `rustup update`. `[capture] auto_install = false` leaves the shim to the subcommands; removing it is never automatic.
- The shim is written beside `cargo` and renamed across, never over the file already there, so a run part way through keeps the script it opened.
- A saved favorite matching the running attract screen is marked `●`, and the popup's title says how many rows are saved and what the mark means.
- `commands.excluded` names cargo subcommands the scan drops entirely, defaulting to `["berth"]`, whose hook fires several times a second. The capture shim skips the same commands.
- `u` restores the full attract configuration displaced by the latest random draw or favorite load.
- `r` draws a fresh attract mode and all of its parameters at random.
- `m` loads a random saved favorite; unusable ones open the diagnostic overlay instead.
- `enter` loads the selected favorite, reporting terminal-sized adjustments without rewriting the row; `x` deletes, with a fade that finishes even when the overlay closes mid-way.
- `ctrl-o` opens a mode-grouped scrolling favorites table whose key labels follow rebinding, paging parameter columns on narrow terminals.
- `ctrl-s` saves the current attract parameters as a favorite, even while the animation is hidden.
- Attract favorites persist in a lossless `favorites.toml`: UUID-addressed rows, tolerant of newer values, locked mutations, atomic replacement.
- The keymap overlay draws its sections side by side where the terminal is wide enough, since three attract screens no longer fit one column.
- A third attract screen on `3`: the desktop drawn as itself with a band of coarseness sweeping across it, taking the picture to blocks and giving it back. Arrows point the sweep, `>`/`<` pace it, `+`/`-` size the blocks, `[`/`]` narrow and widen the band, `v` cycles how a block hands its cells back, `t` draws cells solid or shaded.
- The drifting attract screen fills its cells with bars, and `t` swaps between those and characters. Bars draw part way into a cell, so slow lines travel rather than step, and turning carries the field over instead of dealing it afresh.
- `p` toggles how much of each command a cell spells out: short names every step of the process tree and nothing more, full spells the lines out, with `tree` in the status line while it does.
- A second attract screen on `2`, and the one the app opens with: the window filled with characters in the colours of the desktop behind them, every line drifting on its own. Arrows point the drift, `<`/`>` change speed, `v` lets the lines come apart, `[`/`]` draw their speeds together or apart, dealt in lanes so neighbouring lines travel together.
- The attract band sets off left to right with both edges fraying, so everything the animation does is on screen before a key is pressed. `v` cycles which edges fray and `[`/`]` change how fast.
- The settings overlay moves on the rebindable navigation keys rather than hard-wired arrows, so `[navigation]` in `keymap.toml` says what walks the list.
- Steer the attract band while it is shown: arrows send it, `>`/`<` (or `.`/`,`) change speed, `+`/`-` (or `=`) widen and thin it, further per press while held. The keys live under `[attract_moving_band]` so a later animation binds the same keys to its own meanings, and are live only when the screen was asked for with `a`.
- `a` shows the attract screen on demand, replacing the grid rather than drawing over it, with `attract` in the status line; the panes come back once the strip has faded out.
- An attract screen while no cargo is running: the desktop behind the window, captured with the window's own contents left out, drawn as a band of characters travelling across the terminal. It follows a drag, since only the offset into the capture is re-read. macOS only, and only once Screen Recording is allowed.
- Keep the head of a cell's chain when the block is squeezed below the chain's length and there is no room to elide: the head says an editor or an agent was behind the command. A driver's cell keeps its foot instead.
- Rows tied on start time read by pid, so a cargo draws above the cargo it launched.
- A pid with no family is drawn in the ordinary text colour rather than the column headers' own, where it disappeared.
- Family colours stay clear of the column-header and error colours, measured against the theme in force rather than fixed at startup.
- `f` holds the whole display still and lets it go again, with `frozen` in the status line. The scanners keep going and their output is discarded, so letting go shows what is running then.
- A `parent` column beside `pid` names the nearest ancestor the cell actually draws -- never the pty and shim the capture opened. A pid with cargo under it is drawn in a colour of its own, and every `parent` cell pointing at it shares that colour, handed out against the families on screen from the solarized accent set.
- The summary cell's top border reports what sccache is doing -- hit rate, hits and misses, disk against the eviction ceiling -- only where a server is already running, with fields dropping out in order as the cell narrows.
- How far along a compiling invocation is, from cargo's own unit counter, drawn on the working-directory heading so it costs the table no width.
- Capture cargo's output through a shim: `cargo-tile install` moves each toolchain's real cargo aside, runs it under a pty and mirrors output to `/tmp/cargo-tile/`. `status` reports what stands in front of each toolchain, `uninstall` undoes it.
- Answer to `cargo tile` as well as `cargo-tile`.
- Report progress for runs with no terminal, by asking cargo for the bar it would otherwise draw only for a tty.
- Say when a command is `blocked` on another cargo's build-directory lock, which from outside looks like a build that has not reached its first unit.
- Withhold a command's own cell while nothing runs under it, for the subcommands in `commands.hidden_when_idle` (`port` to begin with). The summary line stays.
- List `commands.hidden_when_idle` in the settings overlay, and write the config file at startup where it does not name every setting.
- A `cpu` column: the whole command's share on `top`'s scale, settled over a two-second window and held a second at a time.
- Head a command's own cell with the parent chain above it, outermost first, one space deeper per level. The processes that only passed the command through are left out, except whatever stands at the foot -- which is what tells a hand-typed command from one an agent ran. Never more than half the cell.
- Draw a `commands.hidden_when_idle` command as the last step of its own cell's chain rather than a table row, since `cargo port` is open all day and compiles nothing.
- Write along each cell's foot what its contents ask for against the size it was given -- `content rows: ##  r/c: ##/##` -- green while it fits, red once it does not.
- Report test progress as well as build progress: `cargo nextest run` counts tests the way cargo counts units, and the heading says which count is on screen. Runs with no terminal read the count from nextest's per-test lines.

### Fixed
- Running several instances at once no longer kills the desktop backdrop: each display is captured through one persistent multi-client ScreenCaptureKit stream that excludes the terminal's own windows.
- An ordinary start no longer reports a stalled desktop capture -- a first capture of a display can take seconds, so the screen waits. The notice remains for a worker actually abandoned and replaced.
- The attract screen's notice names the real cause: the Settings instruction appears only where Screen Recording is denied.
- A favorites row the parser cannot read can be deleted, and one that moved underneath you is refused with a message rather than deleted wrongly.
- A wedged desktop capture no longer strands the backdrop: an attempt is given up after five seconds and a worker missing a second deadline is replaced, up to three times since its last result.
- Toasts repaint from the framework's next-visual-change deadline, so an expiring toast no longer holds the app awake.
- The attract screen comes on when the grid goes idle: settling which window the app is drawn in asked ScreenCaptureKit from the drawing thread, seventy milliseconds a call, so the first seconds of a run drew no frames at all.
- The display no longer freezes on startup: the window-position query and the keyboard thread both read the terminal, and losing the race for one byte parked the app in a read nothing was coming for.
- Capture logs are deleted once their run ends. The directory had reached 60,872 files and 426 MB, costing a 166 ms `read_dir` per scan and leaving 27% of pids carrying an older run's log.
- The frame log records what the attract screen decided -- roster, reader, whether a capture arrived -- so a screen that never comes on says which it was.
- The attract screen says when it has no desktop to draw, after a couple of seconds, rather than drawing nothing and reading as a screen that never started.
- A run reads the log that belongs to it -- the newest for its pid -- rather than whichever the directory listed first, which could report a test run from four days ago standing at 100%.
- A finished build stops reporting the reading it stopped at: cargo's own `Finished` past the counter retires it, while a counter past that one is a test runner's and still stands.
- A run stops reading as `blocked` once its lock comes free: the wait counts only while it is the last thing written, and waits on the package cache no longer show at all.
- A cell's process chain is drawn to the row count it asked for, shortened only where the grid gave the cell less than it asked for.
- The attract screen finds iTerm2's windows again: `TERM_PROGRAM` names a bundle, `iTerm.app`, and the `.app` was compared as part of the name, matching nothing.
- The attract screen falls back to the nearest display, not the primary, when the window's centre lands on no display.
- The moving-band screen draws the desktop across the whole of each cell rather than through the character's ink alone.
- The attract screen draws at the right scale on a Retina display: the terminal reports its text area in pixels where everything it is measured against is in points, so every cell came out twice its size.
- The attract screen no longer leaves its bottom row unpainted when the window stands the display's full height.
- The attract screen draws the desktop behind its own window rather than a sibling terminal window of the same size, which titling both windows yourself used to defeat.
- The drifting screen no longer reads as grey bars over the desktop: the whole cell carries the desktop's colour, with the character standing brighter in it.
- The attract screen no longer draws the desktop from behind another application's window, which an emulator hosting its sessions in a server process used to leave it doing.
- The grid no longer re-reads the whole capture directory several times a second: a registration whose process is gone is cleared away, where one stale file had every log since the last reboot read on every scan.
- The attract screen draws the display the terminal is on rather than the primary: window and display positions were being read in two different coordinate spaces.
- The attract screen no longer follows another application onto its display. The window is found by the number it was pinned at startup, and windows standing on top of the terminal are left out of the capture.
- The attract band no longer leaves a run of empty grid down the window while both edges fray: every offset begins its lap somewhere else.
- The attract screen finishes handing the terminal over instead of turning around part way, and comes back only once the grid has stood empty for a few seconds.
- The attract screen takes its steering keys once it is what the display is showing, not only when asked for with `a`; an arrow used to reach a focus ring nobody could see.
- A cell no longer asks for rows it goes on to elide: the demand measures through the same fit the draw uses.
- A command too wide for its cell wraps down the process chain instead of being cut at the edge, breaking at whitespace and indenting to the column the command started at.
- Cells no longer exchange places when the order changes: an overtaken cell closes and comes back in behind the one that passed it.
- `a` puts the attract screen away over an empty grid, rather than being overruled on the same frame by the roster.
- Hold the render loop to a fixed schedule rather than restarting the interval from the top of each frame, which alternated the period by several milliseconds.
- Stop forcing a full-screen repaint every two seconds while the attract screen is up, which read as a tear; one fires as soon as the strip leaves.
- The attract band no longer jumps once a second or draws the desktop behind a different window: two same-size windows cannot be told apart by size, so the window is settled once, by title.
- The attract band no longer stutters once a second: where the window stands is asked from its own thread rather than queued behind the display capture.
- Bring the grid back when the attract screen finishes leaving, rather than leaving the panes empty until something unrelated asks for a repaint.
- Let go of a frozen display without the attract band jumping forward: it now stands still for as long as the display does.
- Order the cells by when the work in them started, the order every table already reads in, so a `cargo port` open since the morning sorts by the run inside it.
- Wait out the rest of a frame after drawing one rather than a whole fresh interval on top of it, which made the period a different length every frame.
- Say `blocked` for the checks rust-analyzer issues: the shim drops `--quiet` from a `--message-format=json` run with no terminal, leaving stdout byte for byte what the caller parses.
- Report a nested cargo's wait on its own row: every row now reads the nearest capture at or above it.
- Read a process's parent from the kernel where sysinfo left it unset, so a cell's chain no longer stops at `/usr/bin/login` -- which is root's, and whether the chain stopped there was a race.
- Report the state of an invocation the lead command is driving, not just the lead's own. A row with no capture of its own stays blank rather than borrowing the reading above it.

### Changed
- `config.toml` is read, restated and saved by `tui_pane`'s `LoadedConfig`, and the theme is installed by `tui_pane::install_theme`; the file's sections, keys, key order, defaults and parse-error text are unchanged. The status line's styling and its `frozen`, `attract` and `tree` notes, and the keymap and `?` overlays, are drawn through `tui_pane` with the same text and colours. The settings overlay's rows, stepping, popup sizing and navigation scope come from `tui_pane` too (`SettingsRows`, `step_framework_setting`, `draw_settings`, `SettingsNavigation`), and `tiles.initial_rows` is its `InitialRows`; the rows, labels, values, widths, order, the save on every step and the `[navigation]` keymap table are unchanged. The terminal setup and restore, the frame loop, the key dispatch order, the restart and the iTerm2 profile switch run through `tui_pane::run_terminal`; frame pacing, resize handling, the restart's arguments and messages, and the frame log's byte counts are unchanged. The frame log times `tui_pane`'s `FramePhase`s and writes its notes through the runner's `FrameProbe`; `CARGO_TILE_FRAME_LOG`, the file's lines and their column order are unchanged. The freeze state, the grid's fade toward the colour it is painted on and the attract screen's ground colour come from `tui_pane`'s `Updates`, `fade_to_background` and `attract_ground`, with the same colours. The attract screen's controller, its three key scopes, the undo toast and the backdrop notice line now come from `tui_pane`'s `Attract`, `MovingBandPane`, `MovingTextPane`, `PixelatePane`, `undo_attract_replacement` and `draw_backdrop_notice`: timing, fades, frame pacing, keys, `keymap.toml` table and action names, toasts and notice text are unchanged. Favorites are read, saved and deleted by `tui_pane`; `favorites.toml`'s location, keys, spellings and row order, its lock, and the refusal texts are unchanged. The favorites overlay, its `[favorites]` keys, the `ctrl-s`, `ctrl-o` and `m` globals and the removal fade now come from `tui_pane`'s `FavoritesOverlay` and its helpers; the table, its keys and defaults, its notices and toasts, and the favorites file are unchanged. The attract screen's layers are drawn by `tui_pane`'s `draw_attract_layers`, and the frame log's `FrameProbe` supplies the unavailable-capture notice that names `CARGO_TILE_FRAME_LOG`; the frame and the notice text are unchanged.
- The tile grid and its drawing now come from `tui_pane`'s `TileGrid` and `draw_tile_grid`, keyed by invocation, and so do its clicks and Tab (`TileGridHost`, `TileGridPane`, `handle_tile_click`) and the frame's toasts and framework overlays (`render_toasts`, `draw_framework_overlay`). Layout, motion, borders, the rows readout, keys, clicks, Tab, the order the frame's toasts and overlays are drawn in, and `keymap.toml` names are unchanged.
- Saving a favorite says whether it added a row or refreshed an existing one's timestamp.
- The favorites footer offers only what the current selection can do, so `enter` is not advertised with nothing selected.
- The moving-band screen paints the desktop into every cell it has a sample for and fades the strip's edges into it.
- `x` no longer appears in the global-shortcuts list; it still dismisses and is still rebindable.
- App-owned modals consume every key and click ahead of framework overlays and the grid, which close through their own toggle or cancel binding.
- The moving-text screen's fast and slow runs of lines merge into one another rather than meeting at a hard edge.
- The moving-text attract screen starts a quarter slower.
- The moving-text screen draws its bands at varying thicknesses, so the field reads as a field instead of a ruled grid.
- The attract screens ask for thirty frames a second rather than one per poll; a frame is every cell of the window, parsed by the terminal.
- The drifting screen's fast and slow runs travel along the field at half a line a second rather than standing where they were dealt.
- The attract screen's fade takes about twice as long, so `a` reads as the screen changing hands rather than a cut.
- The attract screen arrives and leaves over bare panes: the grid keeps its frames and loses its contents for the length of the fade, so the band has a background to settle into.
- `initial_rows` says how tall the grid grows in a single column before arranging itself into a square, rather than how many rows a column fills before the next opens.
- Draw every reading on the working-directory heading and nowhere else; the `state` column now says one thing, `blocked`, and joins the table only when a row is waiting.
- Tab and Shift-Tab walk the tile grid cell by cell in the grid's own order, wrapping at either end, where the arrows read rows and columns.
- Fade a finished row into the cell it stands on rather than switching it to grey, with the heading and labels travelling with the least-faded row.
- Give a cell out of room the space its neighbours are not using, from either side, rather than dividing every column evenly. A cell keeps what it grew to until another cell encroaches; where the room does not go round, the focused cell is served first.
- Keep the summary's command line to what says something about the work: `--manifest-path`, `--message-format` and `--color` go, everything naming what is built stays, and anything past a bare `--` passes through untouched.
- Order the working directories in a cell by when their work began rather than by name, and the rows under each the same way.
- Give the summary one row per command, and for a `commands.hidden_when_idle` driver the commands it drives instead of its own row.
- Gather the summary by working directory rather than by command, so a directory where one build holds the lock and others wait reads as a single block.
- Count what a cell actually draws when working out what it asks for, wrapping included and through the same table layout the draw uses.
- Hold the summary cell's title off the corner glyph, with a space ahead of the word.
- Wrap a command line too long for its column instead of cutting it off at the edge.
- Close a cell by having the grid come together over it, rather than trading the hole through every cell after it.
- Draw a cell crossing between columns in the columns as they stand partway through the move.
- Keep a cell carried by a closing column from travelling up or down as that column is pushed off the edge.
- Give a heading's reading a tenth of a percent past a hundred units, where a whole number would sit still for several units at a time.
- Rename the `sub` column to `runs`.
- Put `command` ahead of `compiler` and `runs` in a command's own cell, so the command line starts in the same column as in the summary.
- Leave `compiler` and `runs` out of the summary, where a row stands for a whole command.
- Start the working-directory headings and their rows one space in from the cell border rather than two and four.
- Reject arguments that are not a subcommand; 0.1.0 parsed no command line at all.
- Head a command's own cell with the directory the command itself runs in, so a test run's per-case temporary directories no longer push the run being watched below the fold.

### Notes
- Installing the shim is always explicit, never done on startup. It changes nothing about what cargo does, prints, or exits with; query invocations, this workspace's own terminal UIs, and nested cargos pass straight through.
- A `--message-format=json` run is captured down the no-terminal path, which mirrors stderr alone, so the JSON on stdout reaches its caller byte for byte.
- The shim publishes itself as `CARGO`, which keeps `cargo-clippy` and `cargo-nextest` coming back through it.
- A run already going cannot be captured; installing during a build is otherwise safe, since a running cargo holds its binary open.
- `rustup update` replaces the shim with a fresh cargo. Running `cargo-tile install` again repairs it, and is safe to repeat.
- The shim is POSIX `sh` and takes either `script` implementation, both told to flush the log on every write.

## [0.1.0] - 2026-08-21

### Added
- Establish cargo-tile as the starting point for a new `tui_pane` application, tagged `app-template-v1`. The crate is a complete TUI with no application in it: framework globals, the settings / keymap / global-shortcuts overlays, live theming, a status line, restart in place, demand-driven rendering, and input on its own thread. See `docs/tui_pane/as-built/app-template.md`.
- Edit key bindings from the keymap overlay: Enter on a row captures the next keypress, checks it against every binding in force, writes `keymap.toml`, and reloads. The `?` overlay hands off to the same editor with the selected row already open.
- Show the cargo invocations running on this machine, one row per invocation, grouped by the working directory they were started from and ordered by path. A process is classified as cargo by its argv rather than its process name, so a wrapper or a renamed binary is still attributed correctly, and a start-time tie between two candidates resolves toward the newer pid.
- Tile the pane into an animated grid of cells, one per running invocation. The layout is a pure function of the cell count, growing by greedy fill up to `initial_rows` squared and then toward the next square. A cell moving between columns is drawn as two placements -- the piece leaving the old column and the piece arriving in the new one -- so it reads as sliding rather than jumping, and transition progress is fixed-point so the animation is deterministic. A command finishing in the middle empties its cell and the grid closes up around it, with focus following its cell.
- Show the running cargo-tile version in the pane title.
- Own theme content in the app rather than the framework: the crate ships its own theme variants, and its grid draws on the shared pane border in the inactive shade regardless of focus.
