# Tile fixes

## What it is

The tile unit ran the `remove-running-section-tile-fixes` plan from 2026-10-08 to 2026-11-22. The plan fixed and extended cargo-tile, the grid and status line in `tui_pane`, the capture shim, and parts of cargo-berth, cargo-handler and cargo-port. All built work is on main at a9dea4906.

This page lists what each finished phase built, the phases that were not built, and the open follow-ups. The full per-phase as-built notes (files, gates, review rounds) are in the last plan doc: `git show 55f173926:docs/cargo-port/remove-running-section-tile-fixes.md`.

This page also describes native capture startup and cleanup in the reader ownership tests.

## Built

Each line is the behavior that one phase made true. The phase id finds its notes in the plan doc above.

- A cell in flight stands on the painted ground (Phase 1).
- Touching cells and columns share one line through a motion (Phase 2).
- A closing column empties in one motion (Phase 3).
- `auto` follows the system's light or dark setting (Phase 4).
- A floor for cells, and the summary alone below it (Phase 5).
- Headings and ancestry are whole or marked (Phase 6).
- Review repairs: wrapping, headings and the toast (Phase 7).
- A cell shows a process or is not drawn (Phase 8).
- A cut command fills its column, and a cut cell fills its rows (Phase 9).
- The summary takes the rows it asks for, not a second position (Phase 10).
- The summary shows alone when it shows more there, and a view setting picks the rule (Phase 11).
- A moving piece is drawn only at heights its content fills (Phase 12).
- One key's change moves in one motion (Phase 13).
- One column grid, and every row clear of the right frame (Phase 14).
- The toast reads in plain words, and a word that fits a line is never split (Phase 15).
- Readable inks, and a foot that fades (Phase 16).
- The named slow tests run in under a second (Phase 17).
- Toast timing is a named state (Phase 18).
- One reading per command, one ancestry form, and a titled summary (Phase 19).
- Each reading in one place, and every mark where its cut is (Phase 20).
- Hidden commands are hidden always (Phase 21).
- Arguments the user chooses, and two keys that show more (Phase 23).
- Reader and named slow tests hold under load (Phase 26).
- The CPU a row shows is the CPU its processes used (Phase 27).
- Move-only splits (Phase 29).
- The census scan file splits by type cluster (Phase 29a).
- CPU fix 1 on main (Phase 32m).
- Capture shim cost on main (Phase 30m).
- Summary arguments on main (Phase 35m).
- Directory header rule on main (Phase 36m).
- The exec census test finds its program on PATH (Phase 41).
- The exec census tests pass on the GitHub macOS runner (Phase 41c).
- A captured command shows once, with the work under it (Phase 44).
- A captured command keeps one row from its start (Phase 44b).
- The reader uses less CPU, on one process identity (Phase 43a).
- Census CPU falls toward its goal (Phase 43b).
- Mac tests start without a stall (Phase 43e).
- Capture text and constants follow the style guide (Phase 43d).
- Mac reader tests stay under 1 s under load (Phase 43h).
- The Mac census reads executable names through libproc (Phase 43f).
- The install lifecycle test leaves no process behind on the Mac (Phase 43g).
- Intermittent test faults get their owners, and two more slow tests go under 1 s (Phase 43i).
- The Mac source-switch reader test runs under 1 s (Phase 43j).
- A released phase's commits raise no incursion against the next reservation (Phase 64).
- The cargo-berth template setup survives a slow git protocol reply on the CI runner (Phase 65).
- The `shim_modes` native mirror test requires native capture and reports the fallback cause on failure (Phase 66).
- Old test fixtures no longer fill the Mac cache (Phase 67).
- cargo-berth tests run under 1 s on the Mac (Phase 68).
- A plan shrink edit joins the worktree's active reservation (Phase 70).
- A capture shim lock that no install holds does not block (Phase 71).
- Every notice closes with its [x] and with Esc (Phase 72).
- The PTY helper fallback test passes on Linux (Phase 63b).
- A build in a slot shows the slot name, its holder and its purpose (Phase 69).
- A staff file that does not parse keeps its own state (Phase 69a).
- A new cell's text starts near its final color (Phase 38).
- The mem value counts every compile's memory (Phase 39).
- Capture falls back to plain cargo when its stderr mirror cannot run (Phase 40).
- A delayed cell decides its entry on the frame its text first shows (Phase 42).
- A command cell's account and directory are its border title (Phase 47).
- The border title is only the account and directory; progress is the first body row (Phase 47a).
- The cell border is plain; the account and directory are the first body row (Phase 47b).
- The attract screen is a cargo-tile feature, on by default (Phase 52).
- Enter shows the focused cell on the full screen, and Esc returns to the grid (Phase 46).
- A checkpoint release frees only its own phase's reservation (Phase 56).
- Three cargo-berth lock tests pass under load (Phase 57).
- cargo-berth tests run under a second (Phase 58).
- The remaining cargo-berth tests in Phase 58 files run under a second (Phase 58a).
- A reader scenario stops its processes and removes its folder on every exit path (Phase 59).
- cargo-berth tests stay under a second when they run together (Phase 58b).
- Phase 23's follow-ups: rows name what runs, and all arguments fit the screen (Phase 32).
- The capture shim skips every option that cargo accepts before its subcommand (Phase 32d).
- No tile frame draws for more than 4 ms (Phase 32b).
- The last tile frames over 4 ms of CPU (Phase 32e).
- Tile frames stay under 4 ms after layout, view and value changes (Phase 32h).
- A reservation whose holder worktree id changed can be released (Phase 58c).
- Two cargo-berth tests fail only some of the time on the Mac (Phase 58d).
- Two more cargo-berth lock tests fail under load on the Mac (Phase 58g).
- A session's reservation in another worktree accepts `--reservation` (Phase 58i).
- Sixteen cargo-berth drift, phase and presentation tests run under 1 s (Phase 58j).
- Sixteen more cargo-berth tests and two cargo-tile tests run under 1 s (Phase 58k).
- cargo-liner tests ignore inherited Git command settings (Phase 58n).
- The cargo-berth rebase hooks run faster (Phase 58m).
- Before-run templates restore at their canonical path (Phase 58l).
- A merged reservation whose worktree is gone can be recorded as integrated (Phase 58f).
- Two cargo-berth board tests pass in any checkout (Phase 58h).
- A reservation's merge extent holds only the branch's own changes (Phase 58e).
- The toast first-sentence test fails when a word breaks across rows (Phase 60).
- All arguments keep the crate name, and status words match the key names (Phase 32c).
- The hidden-process row reads whole and matches its cell (Phase 32g).
- A nextest run shows its tests, and CPU counts only what each process adds (Phase 33).
- Space pauses the display (Phase 34).
- Phase 35's overlay items: a clipped-pane test that reads cells, a readable selected row, and settings values that wrap (Phase 35o).
- Two lifecycle tests leave no child process or pipe (Phase 61).
- Each app shows its own disk reads per second after its CPU (Phase 62).
- Reads/s stays beside CPU at 100 columns or more (Phase 62a).
- Four more cargo-berth edges and hooks tests run under 1 s (Phase 62b).
- cargo-tile's status-row tests follow the Reads/s fit order (Phase 62c).
- Two shim mode tests pass under load (Phase 63).
- The shim signal tests wait on an event and name their cause (Phase 63a).
- The p and h shortcuts show with the other global shortcuts (Phase 48).
- The summary tile shows free disk space in GiB after mem (Phase 49).
- The reader SIGKILL scenario passes in a full Mac run (Phase 54).
- A CI nextest run shows its progress meter (Phase 50).
- A reader scenario ends its fake cargo processes (Phase 51).
- A lifecycle test leaves no process running (Phase 53).

## Native capture startup

`capture_terminal` in `crates/cargo-tile/src/hook.rs` selects PTY capture when `script` is executable on `PATH` and all three standard streams are terminals. For piped capture, it checks stdout and stderr with `rustix::io::fcntl_getfd`. It uses `/dev/fd` metadata to count free descriptors in `MIRROR_DESCRIPTOR_RANGE` (3 through 9). Piped capture requires `MIN_MIRROR_FREE_DESCRIPTORS` (2).

If a check fails, the shim runs the original cargo command before it publishes capture files. It sets `CARGOTILE_CAPTURE_FALLBACK` without changing terminal output. The `terminal:` error identifies the failed stream check or gives the free descriptor count, range, and minimum.

Inherited descriptors can take the mirror's spare descriptors on macOS. In `crates/cargo-tile/tests/shim_modes.rs`, `mirror_startup_command` uses `/bin/sh` to close descriptors 3 through 9 before it applies the test setup. The shell then sources the shim or starts the native entry. `OccupiedDescriptors` fills the range and checks that the fallback reports zero free descriptors. `DescriptorLimit` uses a limit of 4 on macOS to fail publication and 6 on Linux to fail pipe setup. The publication race test in `hook.rs` closes descriptors 3 through 8 and uses descriptor 9 for cargo stdout.

## Reader ownership test cleanup

`assert_scenario_releases_resources` in `crates/cargo-tile/src/shim_registration/reader_scenario_cases.py` checks that the writer and server groups end and the scenario folder is removed. It covers success, assertion failure, deadline expiry, and `SIGKILL` of the scenario or its caller. The test allows `OWNERSHIP_EXIT_SECONDS` (5 seconds) for the selected exit.

Cleanup closes the lifetime and release pipes so the owner can finish even if the test has no group IDs. Only the caller `SIGKILL` case sends `SIGKILL` to the intermediate caller. The test waits for the owner to close its output pipes. `OWNERSHIP_CLEANUP_SECONDS` is half of `HARD_STOP_GRACE_SECONDS` (2.5 of 5 seconds). This leaves time to signal the recorded writer and server groups before the hard stop. The final cleanup always sends `SIGKILL` to those recorded groups and ignores groups that have already ended.

On a cleanup timeout, the test closes its copies of the output pipes and leaves the owner running. The owner can still end its groups. The timeout names the exit path and includes the owner PID when the test starts the owner directly. The test raises `TimeoutError` or adds the message as a note to the earlier error.

## Not built

| Phase | Goal | Result |
| --- | --- | --- |
| 22 | No cell stands empty: an ended command leaves at once, and a drawn cell shows its rows | dropped: showrunner, 2026-11-06: not landed; the defects that remain live in Phases 31 and 32 |
| 24 | One lifecycle for every invocation | dropped: showrunner, 2026-11-06: not landed; the defects that remain live in Phases 31 and 32 |
| 25 | One invocation, one place | dropped: showrunner, 2026-11-06: not landed; Phase 28, its rebuild, was dropped because the user did not ask for it |
| 28 | Phase 25's one-place work returns on main | dropped: showrunner, 2026-10-29: the user did not ask for it; four tries broke live fill |
| 45 | A new cell's text brightens from 75% to its final color | merged: into Phase 42 |
| 43c | No cargo-tile test takes a second or more | parked: director, 2026-11-07: not landed at the 3-round repair cap; the head wins on natedev, 4 and 1 slow tests against 6, and loses on the Mac, 44 and 47 against 15 and 8; the work continues in Phase 43e |
| 68b | Heavy cargo-berth scenario tests run under 1 s alone on the Mac | dropped: review failed after 3 repair rounds; showrunner rule, 2026-11-18 |
| 68a | Two cargo-tile reader tests run under 1 s at normal load | dropped: user via showrunner, 2026-11-17 |
| 43 | The reader uses less CPU under load | folded: into Phase 43a |
| 30 | The capture shim costs one process per cargo call | folded: into Phase 30m |
| 31 | Ended cells leave whole | dropped: user via showrunner, 2026-11-17 |
| 32i | The last tile frames over 4 ms after 32h | dropped: user via showrunner, 2026-11-17 |
| 32j | Tile frames at 40 and 80 rows are no slower than main | dropped: user via showrunner, 2026-11-17 |
| 32f | Active crate labels read on a light background | dropped: user via showrunner, 2026-11-17 |
| 35 | Phase 20's follow-ups: one table walk, plain caches, and whole marks | dropped: user, 2026-11-19: not wanted; Phase 35o keeps items 9, 35 and 38 |
| 36 | Every cell moves toward one target layout | dropped: user, 2026-11-19: tile movement is good enough for now |
| 37a | Seven slow tests run under 1 s on the Mac | dropped: user, 2026-11-19: not wanted; the full Mac suite ran the six hook tests under 1 s at load 53 |
| 37 | Slow tests start first | dropped: user, 2026-11-19: cargo-liner test time blocks no work |

## Open follow-ups

These items came from reviews and live checks. No phase owns them. The phase numbers refer to the plan doc above.

- **An ended command is never admitted (F021)** — Owner: Phase 31. Target: roster admission and `tiles/grid/tile_grid.rs`. An unshown Closing group takes no slot. A group keeps its cell while another invocation remains live. Completion: roster-to-grid tests cover both cases. Preserve the evidence in `scratchpad/shots22_reply.txt`, CHECK 2. Phase 22 did not land.
- **Column-crossing motion preserves row identity (F020 fallback)** — Target: "Every cell moves toward one target layout", Spec item 22. Port the distinct-row regression from `phases/22/review_findings_5.txt` item 2. Crossing content keeps its sliding row identity or is omitted. Content never starts again at its clip. The motion rewrite owns the implementation and acceptance test.
- **Free rows while a command waits** — Target: the grid admission path. Need: round 6 capture: empty bordered rows stand while a live command waits for a cell, up to 2.14 s (t80 12.56 to 14.71 s, 17 rows, p22k14), d40 1.52 s, t40 1.45 s, d80 1.03 s; main 0.16 s. Rule: a waiting command takes free rows within one motion. Source: showrunner 08:00 2026-10-18 (current follow-up item); evidence `scratchpad/shots25_reply.txt`. Related to Phase 24 item 1's summary cut.
- **A late ancestry row does not grow an ended cell** — Target: the ended snapshot in `crates/cargo-tile/src/roster.rs`. Need: a python ancestry row that arrives after the polled end grows the ended cell from 6 to 7 rows (shots33: t40 p22k13 +0.016 s, d40 p22k12 +0.279 s). Main also grows ended cells after the end. Rule: the ended snapshot's ancestry is fixed at the frame the end is seen; a row seen in the last scan before the end can join only while the cell is live. Completion: a test where an ancestry row first read after the end never changes the ended cell's height. Source: showrunner 2026-10-19, round 13 ruling: follow-up. Related to Phase 22 item 14's frozen `Ended` rows.
- **Cache-server CPU retention with the strict CPU oracle** — Target: Phase 33, Spec items 2 and 5. Change the product retention rule and the oracle retention branch together. Keep integer scan and publication times, smoothing, hold replay, unique contributions and the exact charged pid set. Keep the compiler stopped through two later completed scans, then read its independent kernel CPU counter. Preserve the strict counter bound, staged Mac reader, both client-pid readiness waits, named scenario waits and PTY close before `waitpid`. Repair only timing regressions caused by this accounting change.
- **A stalled scanner shows stale data in either view** — Target: `crates/cargo-tile/src/census/`, `terminal.rs`, and `render.rs::draw_status_line`. Need: Mac binary 0.2.78-dev showed no builds or tests during a captured offloaded hana test after about 13.5 h. No `panic.log` appeared. The suspected cause is a scanner stall without a panic. Rule 1: Bound blocking scan reads, including sysinfo, proc/libproc, process-path reads, and `FreeDiskSpace::read` through `statvfs`. Move unbounded reads off the publication path. Rule 2: One named publication-health state distinguishes current and overdue observations. An overdue scan retains all last-published data, including disk, and shows a stale mark in Grid and Expanded views. The mark uses the shared status styling. Red-first tests stop publication in each view and read the mark from the buffer. Include a blocked disk read in these tests. Rule 3: Each scan over a few seconds logs its step beside `panic.log`. The line names the pid, or the filesystem path for a filesystem read. Ask builds-unit only if more Mac data is necessary. Source: showrunner 2026-10-19; expanded-view amendment after Phase 46; filesystem-read amendment after Phase 49. Related to Phase 24 item 3's publication time bound.
- **A reader test that keeps a parent family across a source switch runs under 1 s** — Target: `shim_registration::reader_scenarios`. Need: `reader_keeps_parent_family_across_its_only_childs_source_switch` took 1.022 s on natedev at load 27.08, under 32, in one of three runs. Completion: the test runs under 1 s in the natedev full suite at load under 32. Fix it with the Phase 43j source-switch reader work. Source: showrunner 2026-11-16. Revealed by: Phase 66.
- **The macOS CI runner keeps native capture in the mirror test** — Target: the capture fallback in `crates/cargo-tile/src/hook.rs`. Need: GitHub run 43717017527 ran `a_native_mirror_keeps_cargo_streams_without_shell_utilities` with uncaptured cargo; the cause is unknown. Each fallback now sets `CARGOTILE_CAPTURE_FALLBACK` to `<path>: <error>`. Completion: the next CI failure names the path, and that path is fixed. Source: showrunner 2026-11-16. Revealed by: Phase 66.
