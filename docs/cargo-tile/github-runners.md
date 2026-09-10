# cargo-tile and the local GitHub Actions runners

> **Status: IMPLEMENTATION PLAN — phased, delegate-ready.**

## Delegation Context

- **Project:** `cargo-tile`, a workspace member of the `cargo-liner` repository: a
  terminal UI that tiles every live cargo invocation on the machine into a grid,
  each cell showing one run's progress, duration and output. This plan gives it
  visibility into runs started by the local GitHub Actions runner accounts on the
  same box. Working tree is the `cargo-tile-runner` worktree, branch
  `enh/tile-runner`.
- **Project started:** 2026-09-09T23:17:04.446+00:00

- **Stack:** Rust, edition 2024, resolver 3. `ratatui` 0.30.2 and `crossterm`
  0.29 over the workspace's own `tui_pane` crate; `sysinfo` 0.39.6 for the
  process table; `serde` + `toml` for `config.toml`; `clap` 4.6 for the CLI;
  `chrono`, `dirs`, `uuid`, `unicode-width`. Dev-dependency: `tempfile`.
  **`cargo-tile` is a binary-only crate** — `src/main.rs`, no `lib.rs` — which
  governs how it can be tested (see **Test lanes**). The capture shim is POSIX
  `sh`, embedded into the binary by `include_str!`.

  Direct syscalls: **`rustix = "1.1.4"` is already in `[workspace.dependencies]`**
  (consumed by `cargo-port` with `features = ["process"]`). Phase 3 adds `rustix =
  { workspace = true, features = ["fs"] }` to this crate's `[dependencies]` — one
  line, no new workspace dependency and no version decision. rustix is safe
  bindings to POSIX syscalls, with a pure-Rust `linux_raw` backend on Linux and a
  `libc` backend on macOS; its `openat` / `fstat` / `O_NOFOLLOW` surface needs no
  `unsafe`, which is what makes it the right choice under `unsafe_code = "deny"`.

  **rustix does not cover everything this plan needs.** It has no `sysctl` — the
  crate declares it unsupported at `not_implemented.rs:292`, and `rustix::system`
  offers only `uname`, a Linux-only `sysinfo()`, and hostname setters. Phase 4's
  macOS birth stamp reads `kern.boottime` and `KERN_PROC_PID`, both of which are
  sysctl, so that submodule alone uses `libc` and `unsafe` under a per-item
  reasoned `allow` (see **Invariants**). No other phase does.

- **Layout:** only what the phases touch.

  ```
  Cargo.toml                                   workspace deps + workspace lints
  crates/cargo-tile/
    Cargo.toml                                 crate manifest (phase 3 edits)
    src/
      main.rs                                  module declaration hub — every
                                               new module is declared here
      cargo-capture-shim.sh                    the POSIX sh shim (phase 2)
      hook.rs   cli.rs                         shim install/repair (phase 1)
      config.rs                                config.toml (phase 5)
      progress.rs                              capture root reader (phases 3-5, 8, 9)
      processes.rs                             process table -> rows (phases 5-9)
      render.rs                                frame rendering (phases 7, 9)
      settings.rs  terminal.rs                 overlay rows, event loop (phase 6)
      constants.rs                             every constant, documented
      roster.rs  app.rs  tiles.rs  capture.rs  read-only context for this plan
    tests/                                     DOES NOT EXIST — phase 2 creates it
  ```

- **Key files:**

  - `crates/cargo-tile/src/main.rs` — the crate's **module declaration hub**, 33
    lines: a flat `mod` list (`:4-29`) and `fn main() -> ExitCode {
    cli::Cli::parse_arguments().run() }` (`:33`). Every new module in phases 3 and
    4 must be declared here, which makes it a hub file those phases assign to
    exactly one writer.

  - `crates/cargo-tile/src/progress.rs` — reads the capture root, 38 KB. `Progress`
    `:85`; `Phase` `:115`; `RunState` `:141`; `RunLiveness` `:175` with `From<bool>`
    `:183`; `Capture` `:194` — the pid-keyed map of readable runs — with
    `take(liveness)` `:214`, `take_from(root, liveness)` `:225` and `read(pid)`
    `:282`; `root()` `:289` — resolves `CARGO_TILE_ROOT` or the default, the single
    site phase 5 turns into a list; `live_runs` `:295` — enumerates
    `<root>/state/pids`. Inline `#[cfg(test)]` module `:513` (31 tests).

  - `crates/cargo-tile/src/processes.rs` — the two-phase process scan and row
    construction, 79 KB, the largest non-render file. `cargo_split(argv) ->
    Option<(usize, Option<String>)>` `:1215`, which short-circuits at
    `argv.first()?` — the macOS empty-argv gate. `row(...) -> Option<CargoProcess>`
    `:1069-1083`: `path` degrades a missing cwd to `UNRESOLVED_PATH` via
    `map_or_else` (`:1070-1073`) while `command: command_text(process.cmd(),
    home)?` (`:1083`) fails the whole row — the asymmetry phase 9's per-field merge
    resolves. `is_shim` `:752`; `captured_run` `:918`; the exclusion retain `:440`;
    the liveness callback `:462`. Inline `#[cfg(test)]` modules `:162` and `:1298`
    (47 tests).

  - `crates/cargo-tile/src/hook.rs` — shim install / remove / repair, 39 KB, and
    **phase 1 rewrote most of it**. `const SHIM_SOURCE: &str =
    include_str!("cargo-capture-shim.sh")` **`:53`**; the shim it embeds must keep
    `SHIM_MARKER` within its first `SHIM_MARKER_SEARCH_BYTES` bytes, which is how
    an installed shim is recognised. `at_startup() -> io::Result<Startup>` `:107`
    over `stand_up` `:111`; `Startup` `:75` (`installed`, `refreshed`, `orphaned`,
    `failed`). **`HookState` `:127` now has four variants** — `Installed` `:130`,
    `Absent` `:132`, `Repairable` `:135` (a saved `cargo-tile-real` with no
    `cargo` beside it — an install interrupted partway), `Orphaned` `:139`.
    `Change` `:144` — `Installed` `:146`, `AlreadyCurrent` `:151` (the installed
    shim is already the current one and nothing was written), `Removed` `:153`,
    `AlreadyAbsent` `:155`, `Orphaned` `:158`. `Hook::all() ->
    io::Result<Vec<Self>>` `:166` reads `rustup_home()?.join(TOOLCHAINS_DIR)`;
    `Hook::at(toolchain)` `:177` **also discovers a toolchain holding only the
    saved cargo**, so a half-installed toolchain is visible rather than skipped;
    `state()` `:196`. **`install()` `:217` takes the installation lock as its
    first statement**, before inspecting state, and holds it through shim
    publication; its `Repairable` arm writes the shim directly instead of moving
    the saved cargo back first. `ensure() -> io::Result<Change>` `:247` is now
    exactly `install()`. `write_shim()` `:257` (staging path, then rename);
    **`remove()` `:264` takes the same lock on the same path**, so an install and
    an uninstall cannot interleave and destroy the saved cargo.
    `HookInstallationLock` `:284` — exclusive creation of
    `<toolchain>/bin/cargo-tile-shim.lock`, bounded retries, released on every
    exit path by `Drop` `:318`; when the retries run out the error carries
    `SHIM_LOCK_RECOVERY`. `rustup_home()` `:323` — prefers `RUSTUP_HOME` over
    `$HOME/.rustup`, which is what makes the `nologin` runner accounts work.
    Inline `#[cfg(test)]` module `:348` (38 tests).

  - `crates/cargo-tile/src/cli.rs` — argument parsing and subcommand dispatch, 10 KB.
    `Cli::parse_arguments().run()` is the whole of `main`. `fn install()` `:108`
    loops `Hook::all()?` through `hook.ensure()`, **reports a toolchain that fails
    and carries on to the next**, and exits successfully whatever any single
    toolchain did — one unwritable toolchain never fails the command.
    `fn uninstall()` `:127` **still stops at the first toolchain that errors**;
    that is pre-existing behaviour phase 1 left alone, and it is why an orphaned
    toolchain can leave later ones still installed. `fn status()` `:136` prints a
    line per toolchain, including `interrupted install -- run cargo-tile install
    to restore cargo` for a half-installed one. `describe(change)` `:152` maps
    each outcome to its line. Inline `#[cfg(test)]` module `:163` (8 tests).

  - `crates/cargo-tile/src/config.rs` — `config.toml` parse and restate, 12 KB.
    `AppearanceConfig` `:34`; `CommandsConfig` `:62` with `excluded: Vec<String>`
    `:71` — **the precedent phase 5's `capture.roots` follows**, including how a
    missing key defaults (`DEFAULT_EXCLUDED`, `:85`); `TilesConfig` `:100`;
    `CaptureConfig` `:139` with `auto_install: bool` `:146` and its `Default`
    `:152`; `Config` `:161` with `capture: CaptureConfig` `:165`; `LoadedConfig`
    `:174` (the parsed config plus any parse error, held in `App`). Inline
    `#[cfg(test)]` module `:276` — only 2 tests, `a_complete_config_is_left_alone`
    and `a_config_missing_a_section_gains_it`; the second asserts the restated file
    `contains("[capture]")` (`:308`), so adding a key to `[capture]` touches it.

  - `crates/cargo-tile/src/render.rs` — frame rendering, 118 KB, the largest file.
    **`fn group_by_path<'a>(rows: &[&'a TrackedRow], pinned: Option<&str>) ->
    Vec<PathGroup<'a>>` `:1347`** — groups by comparing `group.path ==
    row.process.path` (`:1350-1352`), i.e. **on the display-text working directory
    and nothing else**. That is the site phase 9 keys on verified account + root
    instead, and the reason two accounts' `~/x` would otherwise merge into one
    heading. Its doc `:1340-1346` records that path order never moves on its own
    and that the linear search is deliberate. Phase 7's group-total reach is here:
    the three sites summing `cpu` / `managed` across a group's members. Inline
    `#[cfg(test)]` module `:2009` (43 tests, layout and eliding).

  - `crates/cargo-tile/src/settings.rs` — the settings overlay's rows, 11 KB.
    **`pub(crate) fn rows(app: &App) -> SettingsRows` `:98`** is the only entry
    point; `SettingsRows` `:58` (`rows`, `widest_row`, `ids`). **It does no
    filesystem access today** — every row is read out of `app` (`loaded_config`,
    `startup_note`, `capture_note`, `sccache`) and pushed through `push_stepper`.
    Phase 6 must keep that true: root status is rendered from observations the
    scan already retained, never read at render time.

  - `crates/cargo-tile/src/terminal.rs` — the event loop and the scan worker, 34 KB.
    `drain_scans(app, &scans) -> bool` **`:401`**, called from the loop at `:311`;
    its `bool` is "redraw needed". `spawn_input_thread()` `:432`. The scan worker
    sleeps `PROCESS_POLL_MILLIS` **after** each scan, so the scan rate is
    `1 / (0.250 + scan_seconds)` — at most 4/s, never exactly 4.

  - `crates/cargo-tile/src/capture.rs` — 6 KB, turns `hook::at_startup()` into
    toasts and the settings notice. `stand_up(app)` `:23` returns early unless
    `config.capture.auto_install`; `report(app, startup)` `:31` pushes the
    installed / refreshed / orphaned / failed toasts and sets `app.capture_note`.
    Read-only for this plan — it is what makes phase 1's outcomes visible, and its
    `refreshed` list is what stops being populated once a current shim is left
    alone. Inline `#[cfg(test)]` module `:94` (6 tests).

  - `crates/cargo-tile/src/roster.rs` — the retained view across scans, 33 KB.
    `TrackedRow` `:25`, `TrackedGroup` `:125`, `Roster` `:281`, `assign_families()`
    `:382` — the parent/child grouping phase 7's unknown-measurement rules pass
    through. Inline tests `:439` (23 tests).

  - `crates/cargo-tile/src/app.rs` — 10 KB, the application state every renderer
    reads: `AppPaneId` `:46`, `AppOverlay` `:59`, `Updates` `:99` with `toggled()`
    `:109`. Holds `loaded_config`, `startup_note`, `capture_note`, `sccache`.

  - `crates/cargo-tile/src/tiles.rs` — 106 KB, the grid geometry: how many cells
    the pane holds, where each sits, the motion between arrangements. Pure
    functions (`columns`, `shares`, `cell_wants`). Read-only for this plan; it is
    named only because a registration-sourced row becomes a cell like any other.

  - `crates/cargo-tile/src/constants.rs` — 48 KB, every constant with its
    rationale. The ones the phases use, with values:

    | constant | value |
    | --- | --- |
    | `CAPTURE_ROOT` `:694` | `"/tmp/cargo-tile"` |
    | `CAPTURE_ROOT_ENV` `:697` | `"CARGO_TILE_ROOT"` |
    | `CAPTURE_LIVE_RUNS_DIR` `:691` | `"state/pids"` |
    | `CAPTURE_SWEEP_LIMIT` `:743` | `512` |
    | `RUN_LOG_PREFIX` `:745` | `"run-"` |
    | `RUN_LOG_SUFFIX` `:747` | `".log"` |
    | `RUN_LOG_TAIL_BYTES` `:731` | `64 * 1024` |
    | `PID_SEPARATOR` `:700` | `'-'` |
    | `PROCESS_POLL_MILLIS` `:501` | `250` |
    | `UNRESOLVED_PATH` `:256` | `"unavailable"` |
    | `UNRESOLVED_TIME` `:530` | `"--:--"` |
    | `STATE_BLOCKED` `:704` | `"blocked"` |
    | `STATE_COLUMN` `:592` | `5` |
    | `SHIM_MARKER` `:658` | `"cargo-tile-capture-shim"` |
    | `SHIM_MODE` `:661` | `0o755` |
    | `SHIM_MARKER_SEARCH_BYTES` `:674` | `1024` |
    | `SHIM_STAGING_NAME` `:679` | `"cargo-tile-shim.staging"` |
    | `SHIM_LOCK_NAME` `:682` | `"cargo-tile-shim.lock"` |
    | `SHIM_LOCK_RECOVERY` `:685` | what to do about a lock left behind |
    | `SHIM_LOCK_RETRY_ATTEMPTS` `:689` | `10` |
    | `SHIM_LOCK_RETRY_DELAY` `:691` | `10ms` |
    | `REAL_CARGO_NAME` `:652` | `"cargo-tile-real"` |
    | `CARGO_NAME` `:650` | `"cargo"` |
    | `TOOLCHAINS_DIR` `:663` | `"toolchains"` |
    | `TOOLCHAIN_BIN_DIR` `:665` | `"bin"` |
    | `RUSTUP_DIRNAME` `:667` | `".rustup"` |
    | `RUSTUP_HOME_ENV` `:670` | `"RUSTUP_HOME"` |

    Every new constant this plan introduces goes here with the same doc-comment
    discipline; none is defined at its use site.

  - `crates/cargo-tile/src/cargo-capture-shim.sh` — the shim, 11.6 KB of POSIX
    `sh`, phase 2's whole subject. Line map: header and rationale `:1-17`; `set -u`
    `:18`; self-resolution through symlinks `:20-31` (`readlink -f` is GNU-only and
    deliberately unused), `real="$self_dir/cargo-tile-real"` `:31`, and the
    missing-real-cargo bail at `:33`; the `CARGO` re-export `:37-50` (naming the
    shim, not the real binary, so `cargo-clippy` and `cargo-nextest` come back
    through it; not `CARGO_`-prefixed, so sccache does not hash it); `root` and
    `pids` `:53-54`; `capture=1` `:56`; the subcommand gate `:58-99`; the
    nesting-by-ancestry gate `:100-132`; the uncaptured `exec "$real" "$@"` at
    `:138`; `mkdir -p "$pids"` `:139`; the log name `:143`; the registration write
    `:153` / `:155`; `cleanup()` and its traps `:158-171` — **`rm -f "$log"` means
    a run takes its log with it, so only a run killed outright leaves one
    behind**; the pty capture path `:229` / `:237` (`script -q`); the no-terminal
    stderr-FIFO path `:282-291`.

- **Test lanes:**

  **`crates/cargo-tile` has no `tests/` directory, and cannot usefully have one
  for Rust code.** It is a binary-only crate (`src/main.rs`, no `lib.rs`), so an
  integration test in `tests/` has no library target to link against and cannot
  reach any crate item. Every one of the crate's ~400 tests is therefore an
  inline `#[cfg(test)]` module inside the file it tests: `processes.rs:1298` (47),
  `render.rs:2009` (43), `progress.rs:513` (31), `hook.rs:289` (20),
  `terminal.rs:670` (5), `cli.rs:149` (4), `config.rs:276` (2), plus modules in
  `roster.rs`, `capture.rs`, `globals.rs`, `sccache.rs` and the `attract`,
  `favorites` and `theme` trees.

  The consequence for **Seats**: a tester writing tests for crate-internal work
  would be editing the same file as its impl writer, so in every phase except
  phase 2 the `test` seat **opens as impl** and each writer writes the inline
  tests for the file it owns.

  **The one real lane is the shim.** `cargo-capture-shim.sh` is a standalone
  POSIX `sh` script, so an integration test can exercise it by running
  `Command::new("sh")` against `concat!(env!("CARGO_MANIFEST_DIR"),
  "/src/cargo-capture-shim.sh")` — no crate linkage of any kind. **Phase 2 creates
  `crates/cargo-tile/tests/` for exactly this**, which is also what makes a
  single-file phase divisible into three seats. `tempfile` is already a
  dev-dependency, so the test files need no manifest change. Later phases must not
  read the existence of `tests/` as a lane for crate-internal work.

  **`cargo-berth` has a real one, and phases 10 and 11 are in it.**
  `crates/cargo-berth/tests/` holds fifteen integration targets — among them
  `lifecycle.rs`, `ledger.rs`, `board.rs`, `edges.rs`, `drift.rs`, `overlap.rs`,
  `hooks.rs` and `output_contract.rs` — plus a `fixtures/` tree, and they drive
  the built binary through the `cargo-berth-test-support` crate rather than
  linking library items. So both cargo-berth phases have a genuine `test` seat,
  which is what their **Seats** fields open. `cargo-port`, `cargo-mend` and
  `tui_pane` also have `tests/` directories; no phase touches those crates.

- **Build:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-tile`
- **Test:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-tile`
- **Lint:** `bash ~/.claude/scripts/delegate/verify.sh lint cargo-tile`

  **Phases 10 and 11 edit `cargo-berth`, not `cargo-tile`, and use the same three
  commands against that package instead:** `verify.sh check cargo-berth`,
  `verify.sh test cargo-berth`, `verify.sh lint cargo-berth`. A phase that edits
  cargo-berth and runs only the cargo-tile gate has compiled none of its own
  work. Either phase may re-run one integration target alone with
  `verify.sh test cargo-berth <int_test>`.

  These are the only build, test and lint commands a phase may run. The delegate
  composes no cargo flags. Phase 2 additionally may re-run its own integration
  test alone with `verify.sh test cargo-tile <int_test>`. Workspace-wide breadth
  (`--all-targets`, examples, `verify.sh final`) belongs to the final gate after
  the last phase, never to a phase. Exit code `3` means a sandbox failure — re-run
  the same command unsandboxed; it is never a code defect.

- **Style:** `run-end /clippy style-only auto-proceed`

- **Invariants:**

  - **`unsafe_code = "deny"`**, set in the root `Cargo.toml` under
    `[workspace.lints.rust]`, and inherited by `crates/cargo-tile` via `[lints]
    workspace = true`. The manifest's own comment gives the escape: `FFI-only;
    opt back in per item with a reasoned allow`. **Exactly one file in this plan
    takes that escape** — `birth_stamp/macos.rs`, because the sysctl it needs has
    no safe wrapper anywhere in the dependency tree. Everything else, phase 3's
    whole directory-handle layer included, is safe `rustix`. A delegate writing
    `unsafe` in any other file has taken a wrong turn. Where the `allow` is used
    it is per item with a `reason = "..."`, and every block carries a `// SAFETY:`
    comment, since `undocumented_unsafe_blocks` is denied as well.
  - **`cargo-tile` is a Unix-only binary in fact, whatever the code suggests.**
    `hook.rs:29` imports `std::os::unix::fs::PermissionsExt` with no `cfg` gate,
    so the crate has not compiled for Windows for as long as that file has
    existed. The `#[cfg(windows)]` restart branch at `terminal.rs:663` is
    unreachable, and the Windows CI job (`.github/workflows/ci.yml:371`) checks
    only `cargo-port` and `tui_pane`. No phase may spend effort on Windows
    portability, and no phase may add `cfg(windows)` ceremony to justify a
    Unix-only call. Removing the dead branch is out of scope for this plan.
  - **Clippy is denied at `all`, `cargo`, `nursery` and `pedantic`**, plus explicit
    per-rule denies: `unwrap_used`, `expect_used`, `panic`, `unreachable`,
    `undocumented_unsafe_blocks`, `allow_attributes_without_reason`,
    `self_named_module_files`. What that costs this plan: **no `unwrap` or
    `expect` outside test modules**; every `allow` carries a `reason = "..."`;
    every new multi-file module uses the `module/mod.rs` directory form, not
    `module.rs` beside a `module/`; test modules opt back in with
    `#[allow(clippy::expect_used, reason = "tests should panic on unexpected
    values")]`.
  - **`missing_docs = "deny"`** — every module, type, field, variant and function
    carries a doc comment. The existing files set the register: prose that says
    why, not what.
  - **Constants live in `constants.rs`**, each with a doc comment giving its
    rationale. No magic numbers or literal strings at use sites.
  - **The shim must never alter what cargo does, prints, or exits with.** It runs
    in front of every cargo invocation on the machine. Any capture-setup failure
    degrades to running cargo unchanged.
  - **The shim is POSIX `sh`**, deliberately: `/bin/sh` is `dash` on Debian and
    `bash` in POSIX mode on macOS, and nothing in it may depend on which. No
    bashisms, no `readlink -f`, no `local`.
  - **Never edit `/etc/nixos`.** Reads are fine at any time. It is one shared
    working tree and index between the `natedev` session and the local-runners
    session, and two authors collide on the index. Every NixOS-side change in the
    configuration contract — the runner units' `CARGO_TILE_ROOT`, the tmpfiles
    rules for the per-account roots, the job-started hook — is made by the
    `natedev` session, not by a phase delegate.
  - **No agent session rebuilds or switches NixOS generations.** The user does that.
  - **The natedev scan-path gap is unreproduced and is not a phase.** On the
    natedev Linux box no runner cargo has yet appeared in the grid, and the cause
    is not established. The predicted symptom of the known cross-uid limits on
    Linux is a row with `path: "unavailable"` — **not an empty grid** — because
    `/proc/<pid>/cmdline` is world-readable while `/proc/<pid>/cwd` is gated by
    `PTRACE_MODE_READ_FSCREDS`. Diagnosing it needs a live CI job, which no phase
    has. No phase may build against a theory of this gap, and no phase's
    acceptance gate may depend on reproducing it. It is expected to be answered by
    observation once phases 5 and 9 put the runner roots on screen.
  - **CI log growth is unsized.** `<root>/run-*.log` is deleted by the shim's EXIT
    trap, so only runs killed outright leave logs behind; how often that happens
    on the runners is unmeasured. Phase 3's sweep budget is scan-local and bounded
    by design, so this does not block any phase — it is a thing to watch, not a
    thing to solve here.
  - **Two uids are never required by a test.** Every acceptance gate must pass for
    a single unprivileged user in a temp directory. Cross-account behaviour is
    asserted through the ownership and permission checks the code makes, not by
    creating a second account.

## Phases

### Phase 1 — Toolchain hook: repair a half-installed toolchain, stop rewriting a current shim  · status: done

#### As-built

`HookState` (`hook.rs:127`) carries `Installed`, `Absent`, `Repairable` and `Orphaned`; `state()` (`:196`) returns `Repairable` when a toolchain's `bin` lacks `cargo` while the saved `cargo-tile-real` is present, and `Hook::at` (`:177`) discovers such a toolchain rather than skipping it. `status` names the interrupted install and the command that repairs it. `Change` (`hook.rs:144`) carries `Installed`, `AlreadyCurrent`, `Removed`, `AlreadyAbsent` and `Orphaned`: `install()` (`:217`) compares the installed shim against the compiled-in `SHIM_SOURCE` (`:53`) and returns `AlreadyCurrent` without writing when they match, leaving the shim's mtime untouched; `ensure()` (`:247`) is `install()`. Repair writes the shim directly over the missing `cargo` and leaves `cargo-tile-real` where it sits. `HookInstallationLock` (`hook.rs:284`) serializes installation per toolchain via a lock file created beside the shim by exclusive create: `acquire` (`:292`) retries `SHIM_LOCK_RETRY_ATTEMPTS` times at `SHIM_LOCK_RETRY_DELAY`, the guard's `Drop` (`:318`) removes the file on every exit path including error, and `remove()` (`:264`) binds the same lock before inspecting state, so uninstall runs inside the same boundary as install. An attempt that exhausts its retries reports the lock path and the recovery instruction `SHIM_LOCK_RECOVERY` (`constants.rs:685`). The CLI's `install` (`cli.rs:108`) reports each toolchain's failure, continues to the next, and exits zero.

**Files:**
- `crates/cargo-tile/src/hook.rs` — the toolchain hook: shim source, discovery, the four states, the five outcomes, install/ensure/remove, and the per-toolchain installation lock; 38 inline tests.
- `crates/cargo-tile/src/cli.rs` — `install`, `uninstall`, `status`, `describe`; per-toolchain failure reporting with a zero exit; 8 inline tests.
- `crates/cargo-tile/src/constants.rs` — `SHIM_LOCK_NAME` (`:682`), `SHIM_LOCK_RECOVERY` (`:685`), `SHIM_LOCK_RETRY_ATTEMPTS` (`:689`, 10), `SHIM_LOCK_RETRY_DELAY` (`:691`, 10ms).

**Binds later work:** A toolchain's `bin` can hold a third cargo-tile file beside `cargo` and `cargo-tile-real` — the lock, present only while an install runs — so code that enumerates a toolchain directory, or reasons about what cargo-tile put there, must expect it. `HookState` has four variants and `Change` five outcomes; exhaustive matches on either gain arms. A zero exit paired with per-toolchain failure reporting is what keeps a runner job from failing over capture setup, and a later setup boundary must not reintroduce a fail-fast loop.

**Gotchas:** Discovery of a half-installed toolchain is safe only because installs serialize — a second installer that can see the half-installed state, without the lock, saves a shim as the real cargo; the two mechanisms cannot be separated. Two runner units start jobs on the same box, so two concurrent installs against one toolchain is the expected case, not an exotic race. `uninstall` (`cli.rs:127`) still abandons its whole loop on the first toolchain that errors, leaving the rest untouched; `install` no longer behaves this way. The crate is binary-only: no `lib.rs`, no linkable integration-test target, every test an inline `#[cfg(test)]` module inside a production file. It also forbids literals at use sites, so any new constant goes in `constants.rs`.

**Ruled out:** Restoring `cargo-tile-real` to `cargo` before installing over it — the round trip is the window in which a second installer saves a shim as the real cargo. Adding a file-locking dependency for the per-toolchain lock — the crate's existing exclusive-create idiom already serves. Deferring stale-lock recovery guidance — the instruction ships with the diagnostic that names the lock.

### Phase 2 — The capture shim: setup boundary, explicit modes, versioned registration  · status: todo

#### Work Order

**Goal:** the shim publishes a complete, unambiguous registration with an explicit mode, and no capture-setup failure can alter or fail the cargo invocation underneath it.

**Spec:**

Five changes to `cargo-capture-shim.sh` that share one control flow, plus one
change to the reader that **has to land with them**. Phase 1 refreshes an
out-of-date shim automatically at startup, so the moment this binary ships every
toolchain on the machine begins publishing the new registration filename — and
the reader in `progress.rs` accepts only an all-digits filename today, so it
would see none of them and the sweep would treat their live logs as orphans.
Item *6* is that reader change; land it before or with the shim edit, never
after.

*1. A setup-failure policy that cannot reach cargo.* `: > "$log"` (`cargo-capture-shim.sh`, the log pre-creation) is a redirection on `:`, a POSIX **special built-in**, and a redirection failure on a special built-in exits a non-interactive shell — before the umask is restored and before cargo runs. The shim's own header commits it to `/bin/sh` as dash on Debian and bash in POSIX mode on macOS, which are exactly the shells where this applies; it does not reproduce on NixOS only because `/bin/sh` there is bash outside POSIX mode. Use `true > "$log"`.

That single fix is not sufficient. `$HOME` still terminates the shell when unset under `set -u`; the traps currently arrive after the artifacts exist; a same-directory rename can fail with `ENOSPC` or `EROFS` after the record is written, and atomic publication supplies no cleanup; and the FIFO is created **after** the argument rewriting, so its failure cannot deliver the original arguments. Give the shim one policy: **any capture-setup failure runs cargo with its original arguments and the caller's original umask.** A monitoring feature must never fail a build.

Install the parent's cleanup and signal traps **before** any artifact exists, then contain the permission window in a setup subshell, which must install its own traps because caught traps reset on entry:

```sh
fifo=
cleanup() {
    rm -f "$temporary" "$registration" "$log"
    if [ -n "$fifo" ]; then rm -f "$fifo"; fi
}
trap cleanup 0
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

setup_capture() (
    trap 'exit 129' HUP
    trap 'exit 130' INT
    trap 'exit 143' TERM
    umask 0027 || exit 1
    mkdir -p "$pids" || exit 1
    if [ -z "${CARGO_TILE_ROOT-}" ]; then
        chmod 0700 "$root" || exit 1
    fi

    directory=${PWD-}
    if [ -n "${HOME-}" ]; then
        case $directory in
            "$HOME") directory='~' ;;
            "$HOME"/*) directory="~${directory#"$HOME"}" ;;
        esac
    fi

    printf '%s\000' cargo-tile-v2 "$generation" "$boot" "$birth" \
        "$log_basename" "$directory" "$#" "$@" > "$temporary" || exit 1
    chmod 0640 "$temporary" || exit 1
    true > "$log" || exit 1
    mv -f "$temporary" "$registration" || exit 1
)

if setup_capture "$@"; then
    :
else
    result=$?
    cleanup
    trap - 0
    case $result in
        129|130|143) exit "$result" ;;
    esac
    exec "$real" "$@"
fi
```

Create the FIFO, where the no-terminal path needs one, **before** exporting capture settings or rewriting arguments, under the same failure policy. Give the staging file a name the reader ignores (a `.tmp` suffix), because a SIGKILL or a read-only remount can leave one behind and bounded owner cleanup has to recognise it later. The subshell is the containment mechanism: a fatal expansion error stays inside it and the parent's umask never changes. Its cost is one short-lived child and one wait per captured invocation.

*2. The permission window.* Both runner units carry `UMask=0066` (`lib.mkDefault` in nixpkgs' `github-runner/service.nix`); the macOS daemon runs at `Umask = 63`, the same value in decimal. Under `0066` a directory the shim creates lands `0711` and a file lands `0600` — the operator cannot read the file at all. Overriding the unit's umask would reach every file the runner writes, its `.credentials` among them, to buy a monitoring feature; the narrow fix is a scoped `umask 0027` window covering only the shim's own writes.

| umask in force | directory | file |
| --- | --- | --- |
| `0066` (the unit) | `0711` | `0600` — the operator cannot read it |
| `0027` (the window) | `0750` | `0640` — group reads, world does not |

`natepiano` is a member of both runner groups (988 `hana-linux-1`, 987 `hana-linux-2`) and `admin` gives the equivalent read on the Mac, so `0640` is exactly the reach required on the CI accounts. Two ordering constraints, both real:

- **Restore before the `script` call and before the passthrough `exec`.** A umask set once at the top of the shim is inherited by the cargo it execs and by every rustc beneath it, repainting the whole target tree. The `exec "$real"` on the `mkdir` failure path is inside the window and needs the restore too — the subshell above gives this for free, since the parent's umask is never modified.
- **Pre-create the log rather than letting `script` create it.** Both implementations truncate an existing file rather than recreating it, so `true > "$log"` needs no per-platform branch and the `0640` survives.

- **Correct the modes of the directories the shim itself created.** `mkdir -p`
  applies the umask only to a directory it actually creates; one that already
  exists keeps whatever mode it was made with. A runner account that has already
  run cargo under `0066` therefore has `state` and `state/pids` sitting at
  `0711`, and the new `0027` window leaves them exactly there — new registrations
  land `0640` inside a directory the operator still cannot list, which is the
  same failure this item exists to fix. The shim corrects the modes of the
  directories it owns beneath the root, one level at a time and never
  recursively, under the same failure policy as everything else in the window.

`mktemp` creates `0600` whatever the umask, and rename preserves the mode, which is why the staging file needs the explicit `chmod 0640`.

*3. The desktop root is owner-only.* Where the effective root is the built-in `/tmp/cargo-tile` — that is, `CARGO_TILE_ROOT` is unset — the shim creates it `0700` and corrects an existing directory to `0700`, one level and never recursively. A root a deployment named through `CARGO_TILE_ROOT` keeps the mode that deployment gave it: the nix tmpfiles entries make the runner roots `0750 <runner> <runner>`, and group membership is the read path. A failed `chmod` follows the same policy as every other setup failure.

*4. The registration format.* The shim currently writes `printf '%s\tcargo %s\n' "$directory" "$*"`, which joins the arguments with a space: `cargo run -- 'a b'` and `cargo run -- a b` produce identical records. `CommandText.arguments` promises one entry per argv word and the summary's flag trimming relies on it, so splitting the record on whitespace would violate that promise; a tab or newline inside an argument breaks the `<cwd>\tcargo <args>` framing outright; and the writer opens the final filename directly, so a reader can see an empty or partial record.

The replacement is NUL-framed, versioned, and published by same-directory rename — one rename per run, not per poll. Fields in order, each terminated by NUL:

| # | field | value |
| --- | --- | --- |
| 1 | magic | the literal `cargo-tile-v2` |
| 2 | generation | the same token the log basename already carries. **It is not an epoch second:** the log name is `run-$(date +%Y%m%d-%H%M%S)-$$.log`, a calendar timestamp at one-second resolution |
| 3 | boot identity | Linux: contents of `/proc/sys/kernel/random/boot_id`. macOS: `sysctl -n kern.boottime`. Empty when unobtainable |
| 4 | birth stamp | Linux: field 22 of **`/proc/$$/stat`**, in ticks, parsed by discarding everything through the **last** `)` so a `comm` containing spaces cannot shift the field. macOS: `ps -o lstart= -p $$`. Empty when unobtainable |
| 5 | log basename | the exact basename of this run's log, no directory part |
| 6 | working directory | as today, `$HOME` already collapsed to `~` |
| 7 | argument count | `$#` |
| 8.. | arguments | `"$@"`, one field each, verbatim |

Never hold the framed record in a shell variable — a NUL cannot survive one. `printf` writes it directly to the staging file.

The registration **filename** becomes `<pid>.<generation>` rather than bare `<pid>`. That alone does **not** make the name unreusable: the generation is a one-second calendar stamp, so a pid reused inside the same second — or any second at all after a clock step backwards — reproduces a name a previous run already used, and `mv -f` would silently replace whatever holds it. Two rules close that, and both are the contract Phase 4's deletion argument rests on:

- **Publish by exclusive creation, not by overwrite.** Link the staging file into place with `ln "$temporary" "$registration"`, which fails when the name exists — the one atomic exclusive-create primitive POSIX `sh` actually has — then remove the staging file. A name already taken is a setup failure like any other: clean up and run cargo uncaptured. A run can therefore never replace a registration another run still owns, including one a sweep has already chosen as a deletion candidate.
- **Delete only the exact name that was verified.** A sweep removes `<pid>.<generation>` by the full name it read and proved ended, never by pid alone. A registration that reappears under the same name between enumeration and deletion is a different record, and the exclusive-create rule is what guarantees it could only have appeared after the first was gone.

The log filename already carries the generation and does not change.

Read the birth stamp from **`/proc/$$/stat`, not `/proc/self/stat`**. The setup work runs inside a subshell, so `self` there is the short-lived child; `$$` in POSIX `sh` stays the parent shim's pid, which is the pid the registration is named for and the one the reader will check. Reading `self` publishes a stamp for a process that has already exited by the time anyone compares it, and every registration then reads as a reused pid.

Fields 3 and 4 are what lets the reader prove the registration belongs to the process now holding that pid rather than to a reused pid. When either is empty the reader treats the record as unverifiable — Phase 4 defines what that means. Obtain them inside the setup subshell so a failure follows the same policy.

*5. The header comment.* The shim's header says the grid reads the working directory and command from the registration. Phase 9 makes that true; today it is not. State it as what the format is *for*, without claiming the reader does it yet.

*6. The reader accepts both filenames — and this lands first.* `live_runs`
(`progress.rs:302`) reads each entry in `state/pids` and parses the whole
filename as a `u32`, discarding anything that does not parse. A `<pid>.<generation>`
name fails that parse, so every registration this phase publishes would be
invisible: the run would not count as live, and `Capture::take_from`'s sweep —
which deletes the log of any run it cannot see — would delete the logs of runs
still writing to them. Teach the reader to take the pid from either form (all
digits, or digits up to the first separator) and to carry the generation with it
where present, so a mixed directory of old and new registrations reads
correctly. The separator is a documented constant like every other literal in
this crate. Phase 4 builds verification on top of this; here the requirement is
only that the new name is understood and no live log is swept.

**Files:**
- `crates/cargo-tile/src/cargo-capture-shim.sh` — the whole shim change; it is embedded by `include_str!` (`hook.rs:53`), so no registration or build step is needed for it to ship
- `crates/cargo-tile/src/progress.rs` — item *6*: `live_runs` (`:302`) accepts both filename forms; writes its cases into the existing `#[cfg(test)]` module (`:513`)
- `crates/cargo-tile/src/constants.rs` — the registration-filename separator, and any other literal item *6* needs
- `crates/cargo-tile/src/hook.rs` — **its `#[cfg(test)]` module only**, no production code: the assertion at `:410` requires the shim to contain the literal `rm -f "$log"`, and this phase's combined `cleanup()` writes `rm -f "$temporary" "$registration" "$log"` instead. Update that assertion to the durable fact it is there to protect — the shim removes its own log on exit — rather than deleting it
- `crates/cargo-tile/tests/shim_modes.rs` — **new**, and creates the `tests/` directory
- `crates/cargo-tile/tests/shim_registration.rs` — **new**

**Seats:** 1 writer + 2 testers — this phase **creates the crate's only real test lane**, which is what makes it divisible. The shim is a standalone POSIX `sh` script, so a test reaches it with `Command::new("sh")` — no crate linkage, so a binary-only crate is no obstacle. `tempfile` is already a dev-dependency. The two test files are new and disjoint from each other and from the writer's files.

**Neither test file may run the repository copy of the script in place.** The shim resolves its own directory and expects `cargo-tile-real` as a sibling, so pointing `sh` at `crates/cargo-tile/src/cargo-capture-shim.sh` makes it look for `crates/cargo-tile/src/cargo-tile-real`, which does not exist, and every such test exercises only the bail-out path. Each test copies the shim into a temporary directory laid out like an installed toolchain, beside an executable stand-in for the real cargo that records how it was called. The default-root cases (`CARGO_TILE_ROOT` unset) additionally need their own `HOME` and root so they never touch the developer's `/tmp/cargo-tile`.
- `impl` — `crates/cargo-tile/src/cargo-capture-shim.sh`, `crates/cargo-tile/src/progress.rs`, `crates/cargo-tile/src/constants.rs`, and the `#[cfg(test)]` module of `crates/cargo-tile/src/hook.rs`; hub: `crates/cargo-tile/src/constants.rs`. The shim's format and the reader that parses it are one decision and stay in one head
- `test` — creates `crates/cargo-tile/tests/shim_modes.rs`: cargo's observed umask and the resulting artifact modes for a caller umask of `0022`, `0066` and `0077`, across both the pty and the no-terminal capture path; the `0700` default-root correction against a set and an unset `CARGO_TILE_ROOT`; and the degradation cases — an unwritable root, and `HOME` unset
- `review` — opens as test: creates `crates/cargo-tile/tests/shim_registration.rs`: the registration format from the Spec's field table — field order, NUL framing, the `cargo-tile-v2` magic, argument boundaries preserved across `cargo run -- 'a b'` versus `cargo run -- a b`, the `<pid>.<generation>` filename, the `.tmp` staging suffix, and the log basename matching the log actually written

**Constraints from prior phases:**

- **Phase 1 ships a changed shim to every toolchain by itself.** `install()` compares the installed shim against `SHIM_SOURCE` and rewrites it when they differ, and `at_startup()` runs that on launch, so this phase's new format goes live on the next start of the application with no operator action. That is why item *6* cannot land after the shim edit.
- `SHIM_SOURCE` is `hook.rs:53`, and the shim must keep `SHIM_MARKER` within its first `SHIM_MARKER_SEARCH_BYTES` bytes or an installed shim stops being recognised as one.
- `HookState` has four variants — `Installed`, `Absent`, `Repairable`, `Orphaned` — and a toolchain holding only the saved cargo is now discovered rather than skipped. `Change::AlreadyCurrent` means the installed shim already matched and nothing was written.
- `cargo tile install` reports a toolchain that fails and continues to the next, and exits successfully whatever any single toolchain did. `cargo tile uninstall` still stops at the first error.
- Install and remove serialise on `<toolchain>/bin/cargo-tile-shim.lock`. Nothing in this phase touches that lock, but a test that installs shims must not assume the file is absent afterwards on a failure path.

**Acceptance gate:** Build, Test and Lint green. Tests asserting cargo's observed umask and the artifact modes for `0022`, `0066` and `0077` on both capture paths. A test asserting an existing `0711` `state/pids` hierarchy is corrected to a mode the owner's group can read, while cargo still observes the caller's original umask. A test asserting `cargo run -- 'a b'` and `cargo run -- a b` produce different registrations. A test asserting a setup failure (an unwritable root) still runs cargo with its original arguments and the caller's umask, and exits with cargo's status. A test asserting `HOME` unset does not abort the shim. A test asserting a registration cannot be published over a name that already exists, and that cargo then runs uncaptured. A test asserting the emitted birth stamp names the shim process itself — the pid the registration is filed under — and not a child of it. A test asserting a `state/pids` directory holding both an old all-digits registration and a new `<pid>.<generation>` one yields both runs as live, and that a scan across it deletes neither run's log.

---

### Phase 3 — A hardened root access layer, and `progress.rs` onto it  · status: todo

#### Work Order

**Goal:** the scanner reads a capture root through directory handles with bounded, type-checked reads, never sweeps a root it does not own, and cannot mistake an unreadable directory for an empty one.

**Spec:**

*1. The reads are unsafe today.* `tail` (`progress.rs:353`) uses `File::open`, which follows symlinks, checks no file type, and then `read_to_end`s after a seek. A FIFO blocks at open; a symlink to `/dev/zero` reports length 0, seeks to 0 and reads without bound. The scan runs on a worker thread, which contains neither failure — one foreign entry stops every local row updating, or exhausts memory.

*2. One directory-handle contract.* Hardening the final `open` is not enough: `O_NOFOLLOW` on `root/state/pids/<name>` still follows a replaced `state` or `pids`, and checking an old descriptor's metadata does not detect a replacement mounted at the configured pathname. Inspection, ownership classification, retained entries and deletion must all refer to the same directory:

- Open the effective root, then derive ownership from `fstat` on **that descriptor**. Define root-symlink handling explicitly and preserve trusted system prefixes such as macOS `/tmp`.
- Open `state`, then `pids`, each relative to the previous handle with `RDONLY | DIRECTORY | NOFOLLOW | CLOEXEC`. Enumerate through those handles and **retain enumeration errors** rather than discarding them.
- Open entry **basenames** relative to those handles with `RDONLY | NOFOLLOW | NONBLOCK | CLOEXEC`, check the opened file is a regular file, and enforce byte caps with `Read::take` independently of metadata: the existing tail cap for logs, a documented cap plus one byte for registrations so an oversize record is detectable. `O_NONBLOCK` keeps a FIFO open from blocking; it imposes no deadline on ordinary file I/O, so the caps are the real bound.
- Reopen the configured pathname each scan and compare device/inode identity and ownership against the previous scan. Replacement or access failure invalidates that root's retained entries and disables its sweep; descriptors from a previous root are never combined with paths from its replacement.
- Retain the existing lossy UTF-8 decode for log contents.

*3. Deletion is a capability, not a flag.* Both removal paths take an ordinary `&Path`, so an ownership boolean leaves every future caller responsible for remembering the rule. Represent roots as `Owned(OwnedRoot<'scan>) | Foreign(ForeignRoot<'scan>)`, where `OwnedRoot` is private, non-clonable, constructible only after a successful `fstat` and an `st_uid == euid` comparison, and **borrows the scan** — the inspected directory handles and that scan's enumeration success — so a cached capability cannot outlive the check that justified it. Both removal paths are reachable only through `OwnedRoot` and take their targets from its own scan entries. One internal enum dispatch per root, no trait, no per-entry allocation.

*4. Ownership is necessary but not sufficient.* An owned root does not establish ownership of `state/pids`, nor stop that path redirecting through a symlink. Worse, `live_runs` (`progress.rs:302-319`) turns a failed directory read into an **empty** live set, and `take_from` then treats every log as belonging to a dead run and deletes it: a root that stays readable while `state/pids` does not is a path to destroying live capture data in a root the reader owns. Represent an unavailable live set separately from an empty one, and make a complete and successful registration enumeration necessary before any sweep. A failed ownership check disables sweeping for that root without affecting the others. Decline to sweep any directory another account can write.

*5. One sweep budget, scan-local, counting both artifact types.* `Capture::discard` (`progress.rs:272-278`) increments `swept` **before** `fs::remove_file`, so entries it cannot delete consume the budget anyway and a foreign root starves the owned root's sweep on every pass. `live_runs` removes stale registrations the same way and discards the error identically — and those removals carry **no budget at all**. For `R` stale registrations and `D` discardable logs, one pass attempts `R + min(D, CAPTURE_SWEEP_LIMIT)` removals. Thread **one** budget through registration and log cleanup across all owned roots: initialise it before the scan iterates roots and pass it through both cleanup paths. Resetting it inside `take_from` multiplies the allowance by the number of roots; persisting it across scans eventually stops cleanup altogether. Foreign roots consume none of it. Correct `CAPTURE_SWEEP_LIMIT`'s documented sizing rationale, which counts logs only.

This is guaranteed rather than hypothetical on the runner machine: the runner cache directories are `drwxr-x---`, group read and execute with no write, so every removal a reader running as `natepiano` attempts fails, on every entry, permanently.

*6. Three stale doc comments, not two.* `progress.rs` contradicts itself about whether logs are deleted, and the wrong half is repeated:

| line | says | correct? |
| --- | --- | --- |
| `progress.rs:300` | "every run since the last reboot" | yes |
| `progress.rs:326` | "Logs are never deleted, so the capture directory holds every run since the machine was set up" | no |
| `progress.rs:607` | "Logs are never deleted and pids come round again" | no |

The shim's `cleanup()` does `rm -f "$log"` with `trap cleanup EXIT`, and `trap 'exit 130' INT` / `trap 'exit 143' TERM` route signals through it, so a run takes its log with it and a cancelled Actions job — which takes SIGTERM first — cleans up after itself. What survives is the logs of runs **killed outright**. Correct both wrong comments; the claim was read as a sizing input and produced a CI log-growth estimate wrong by orders of magnitude.

The scan rate this file reasons with is also wrong in one place: the worker sleeps 250 ms **after** completing each scan (`processes.rs:380`), so the frequency is `1 / (0.250 + scan_seconds)` — at most four per second, not exactly four.

**Files:**
- `crates/cargo-tile/src/capture_root.rs` — **new**: the root access layer. Root open, the ownership check, `openat` traversal, bounded entry reads, and the `OwnedRoot<'scan>` / `ForeignRoot<'scan>` capability types. One file, not a directory module — it has no submodules, and `self_named_module_files` is denied.
- `crates/cargo-tile/src/progress.rs` — `root()`, `live_runs`, `take_from`, `discard`, `tail` rewired onto the layer; the budget threaded; the three doc corrections
- `crates/cargo-tile/Cargo.toml` — one line: `rustix = { workspace = true, features = ["fs"] }`. `rustix 1.1.4` is already in `[workspace.dependencies]`, so there is no version to choose; `unsafe_code = "deny"` is why this is `rustix` and not `libc`.
- `crates/cargo-tile/src/main.rs` — the `mod` declaration for the new module

**Seats:** 2 writers + reserve — there are two independent halves here, not three: the new access module with the manifest and module declaration it needs, and the `progress.rs` rewiring onto it. `cargo-tile` is a binary-only crate, so it has no linkable integration-test target and its tests are inline `#[cfg(test)]` modules inside the files being edited (Delegation Context → **Test lanes**). Each writer writes the inline tests for the file it owns; no seat here can open as `test`. A third writer would be waiting to enter a file one of the other two already holds.
- `impl` — `crates/cargo-tile/src/capture_root.rs` and its inline `#[cfg(test)]` tests, `crates/cargo-tile/src/constants.rs` (the access layer's own limits, and the `CAPTURE_SWEEP_LIMIT` documentation this phase changes); hub: `crates/cargo-tile/src/main.rs` (the `mod` line) **and** `crates/cargo-tile/Cargo.toml` (`rustix = { workspace = true, features = ["fs"] }` — `rustix 1.1.4` is already in `[workspace.dependencies]`, so this is one line and no version decision). Land both hub edits first: the second writer cannot compile until the module is declared.
- `test` — opens as impl: `crates/cargo-tile/src/progress.rs` — `root()` (`:289`), `live_runs` (`:302`), `take_from` (`:225`), the sweep and the tail read, rewired onto the layer, plus the three stale doc corrections; writes its cases into the existing `#[cfg(test)]` module (`:513`)
- `review` — reserve until the module declaration lands, then opens as impl on whichever half is behind. `capture_root.rs` owns the symlink, FIFO, byte-cap and ownership tests; `progress.rs` owns the unavailable-versus-empty live set and the scan-local budget shared across roots.

**Constraints from prior phases:**
- Phase 2 publishes registrations under `<pid>.<generation>` filenames and logs under their existing `run-<generation>-<pid>` names. Enumeration must accept both that form and the legacy bare `<pid>`, and parse the pid as the segment before the first `.`.
- Phase 2's staging files carry a `.tmp` suffix. The reader ignores them during enumeration; bounded owner cleanup may remove them.

**Acceptance gate:** Build, Test and Lint green. Tests for each item in the tester's line above. A test asserting one scan across two owned roots attempts at most `CAPTURE_SWEEP_LIMIT` removals in total, not that many per root — and that the budget counts every kind of removal the sweep makes together: registrations, logs, and staging files old enough to be swept. A budget counted per artifact kind is three budgets, which is what the constant exists to prevent. No test may require a second uid.

---

### Phase 4 — Registration identity: verify before it counts, and before it is deleted  · status: todo

#### Work Order

**Goal:** a registration is accepted only when the process now holding its pid is provably the one that wrote it, and no artifact is deleted without fresh evidence its writer ended.

**Spec:**

*1. A live pid does not prove the registration belongs to that run.* Today liveness means only that some process occupies the registered pid. A registration left behind by a SIGKILLed run plus pid reuse would display a finished command as running with no cargo process anywhere. Root ownership does not establish the association; root-qualified keys separate files but not process instances; and mtime supplies a display timestamp rather than proof of identity. The reverse direction fails the same way: a shim that registers *after* the process snapshot reads as absent, and cleanup in an owned root then deletes a live run's registration.

A random token, an inode number and a ctime all prove only that a *file* persisted — none of them associates that file with the process now holding its pid. The evidence that does is the stamp the kernel assigns at process creation, qualified by which boot it belongs to:

- **Linux:** the boot ID from `/proc/sys/kernel/random/boot_id`, plus field 22 of `/proc/<pid>/stat`. That field is readable across the uid boundary. Keep it in **ticks** — do not convert through sysinfo's rounded seconds, which loses the resolution the comparison needs.
- **macOS:** the boot-session identity from `kern.boottime`, plus `KERN_PROC_PID`'s `p_starttime`. sysinfo's macOS fallback initialises `start_time` to zero, so a start-time-versus-mtime comparison cannot carry it there.

*How the macOS stamp is obtained.* Both values are `sysctl`, and **`rustix` does not implement `sysctl`** — the crate lists it as unsupported at `not_implemented.rs:292`, and `rustix::system` carries only `uname`, a Linux-only `sysinfo()`, and hostname setters. `sysinfo` is already ruled out above. So `birth_stamp/macos.rs` calls `libc::sysctl` directly and is the **only** file in this plan that writes `unsafe`, under a per-item `#[allow(unsafe_code, reason = "...")]` with a `// SAFETY:` comment on each block — the escape the workspace manifest's own comment sanctions (`FFI-only; opt back in per item with a reasoned allow`). Do not shell out to `sysctl(8)` or `ps(1)` instead: the reader runs this per registration per scan at up to four scans a second, and a subprocess there is not affordable. `kern.boottime` cannot change within a boot, so read it once and cache it; `KERN_PROC_PID` is per process and read per verification. `birth_stamp/linux.rs` needs none of this — `/proc` reads are ordinary file I/O.

*Units, and the macOS resolution trap.* Phase 2's shim is POSIX `sh` and can only obtain the macOS birth through `ps -o lstart= -p $$`, which has **one-second resolution**, while `p_starttime` is a `timeval` carrying microseconds. Truncate the live reading to whole seconds before comparing, and parse `lstart`'s fixed format rather than assuming a locale. The residual exposure is a pid reused inside the same second as the original registration, which is narrower than the pid-reuse window this evidence exists to close and is the best the shim side can supply. Linux has no such gap: both sides carry ticks.

Compare the live process's current stamp against fields 3 and 4 of the registration Phase 2 writes. Equal on both → the registration belongs to that process. Different → the pid was reused and the registration is stale. Either field empty, or the observation denied or incomplete → `Unknown`.

*2. Three states, never two.* Model the outcome as `Confirmed | Ended | Unknown`. `Unknown` is never rendered as "running" and never authorises a deletion. A record written before this evidence existed — a legacy registration with no magic, or one whose identity fields are empty — is display-only: it may annotate a row the process table already built, and it may never source an active row.

*3. The deletion rule covers logs too.* Registration enumeration and log enumeration are not one atomic snapshot. A shim can publish its registration and create its log in the interval between them; the reader then meets a log whose pid was absent from the earlier set and deletes a running capture's output. A second race survives directory-relative deletion: after inspecting `state/pids/<name>` the reader can unlink a *replacement* that has since taken that name, and re-checking the inode narrows the interval without closing it. Every artifact — logs included — needs fresh evidence its writer ended before removal, and uncertainty preserves it. Phase 2's exclusive-create publication is what closes the second race for new records — a `<pid>.<generation>` name cannot be taken while a record still holds it, so a name that reappears is provably a later record rather than a replacement of the one just inspected, and deletion by that exact full name is therefore safe; a legacy bare-`<pid>` record gets deferred cleanup rather than an immediate delete. A snapshot miss must be re-verified before any delete.

*4. Parse the versioned record.* Read the NUL-framed fields Phase 2 defines, under the registration byte cap from Phase 3. Reject a malformed record individually — a bad record must not discard the other registrations and must not disturb the process-table rows that already exist. Records in the old `<cwd>\tcargo <args>` format are display-only text, since their argument boundaries cannot be recovered.

*5. Bind the log to the registration's generation.* A1's identity establishes which process owns a registration but not which log belongs to that registration's run. "Newest filename for this pid" has no correct answer during the window where a new registration exists and its log does not — the reader selects the previous run's log for a reused pid. Take the exact log basename from field 5, validate it is a single filename with no directory part, and open it relative to the inspected root. That removes foreign log enumeration and `newer()` selection for new-format records entirely. Root replacement or a generation change discards the association; a temporarily missing log is retried on the next scan while its registration stays valid; directory timestamps never decide whether that named file is reopened, since timestamp resolution is finite and a rename over an existing name can leave them equal.

**Files:**
- `crates/cargo-tile/src/birth_stamp/mod.rs` — **new**: the boot-qualified birth stamp and the platform-independent comparison. Directory form because it has submodules, which `self_named_module_files` requires.
- `crates/cargo-tile/src/birth_stamp/linux.rs` — **new**: boot id from `/proc/sys/kernel/random/boot_id`, birth from field 22 of `/proc/<pid>/stat`
- `crates/cargo-tile/src/birth_stamp/macos.rs` — **new**: boot time from `kern.boottime`, birth from `KERN_PROC_PID`'s `p_starttime`, via `libc::sysctl`. The one file in this plan carrying `unsafe`.
- `crates/cargo-tile/Cargo.toml` — `libc` for the macOS submodule only:
  ```toml
  [target.'cfg(target_os = "macos")'.dependencies]
  libc = { workspace = true }
  ```
  `libc = "0.2.189"` is already in `[workspace.dependencies]`, so there is no version to choose, and target-gating keeps a Linux build free of a direct `libc` dependency.
- `crates/cargo-tile/src/registration.rs` — **new**: parsing and verifying the `cargo-tile-v2` record
- `crates/cargo-tile/src/progress.rs` — acceptance, the three-state outcome, the deletion rule, log association by basename
- `crates/cargo-tile/src/main.rs` — the `mod` declarations for `birth_stamp` and `registration`

**Seats:** 3 writers — splits by module: the platform birth stamp, the record parser, and `progress.rs` acceptance are three separate files. `cargo-tile` is a binary-only crate, so it has no linkable integration-test target and its tests are inline `#[cfg(test)]` modules inside the files being edited (Delegation Context → **Test lanes**). Each writer writes the inline tests for the file it owns; no seat here can open as `test`.
- `impl` — `crates/cargo-tile/src/birth_stamp/mod.rs` and its two platform submodules, plus `crates/cargo-tile/src/constants.rs` (the tick and clock literals both platforms need); hub: `crates/cargo-tile/src/main.rs` **and** `crates/cargo-tile/Cargo.toml` (the macOS-gated `libc` entry). Lands the `mod` lines for **both** new top-level modules first, so the other two writers can compile. This seat owns the only `unsafe` in the plan, so it also owns the `allow` and the `// SAFETY:` comments; a Linux-hosted delegate cannot compile `macos.rs` and must reason it through rather than iterating against the compiler.
- `test` — opens as impl: `crates/cargo-tile/src/registration.rs` — the field table, NUL framing, rejection of a malformed or short record, `Unknown` for an empty identity field, and refusal of a log basename containing a directory separator
- `review` — opens as impl: `crates/cargo-tile/src/progress.rs` — acceptance, the `Confirmed | Ended | Unknown` outcome, the deletion rule, and log association by basename

**Constraints from prior phases:**
- Phase 2 defines the record: NUL-terminated fields in the order magic, generation, boot identity, birth stamp, log basename, working directory, argument count, arguments. The magic is the literal `cargo-tile-v2`. Fields 3 and 4 are empty when the shim could not obtain them.
- Phase 2's Linux birth stamp is field 22 of `/proc/$$/stat` in ticks — the shim's own pid, the one the registration is filed under, never `self` inside its setup subshell; the macOS one is `ps -o lstart= -p $$`. The reader must compare against the same units, and against the process named by the registration's filename.
- Phase 3 supplies the directory handles, the byte caps, and `OwnedRoot<'scan>`; all opens and deletions here go through them.
- Phase 3 already added `rustix` to `crates/cargo-tile/Cargo.toml`. This phase adds only the macOS-gated `libc` entry beneath it; do not restate or alter the `rustix` line. rustix covers phase 3's file surface and **not** `sysctl`, which is why this phase reaches past it on macOS alone.
- Phase 3's enumeration accepts both `<pid>.<generation>` and legacy bare `<pid>` filenames.

**Acceptance gate:** Build, Test and Lint green. A test asserting a registration whose birth stamp does not match the live pid's is `Ended`, not `Confirmed`. A test asserting an `Unknown` outcome authorises no deletion. A test asserting a log created between the two enumerations is not swept. A test asserting a legacy record annotates but never sources a row. A test asserting the comparison truncates to whole seconds on the macOS path and does not on the Linux path, written so it runs on either host. `unsafe` appears in `birth_stamp/macos.rs` and nowhere else, each block under a `reason`-carrying `allow` and a `// SAFETY:` comment.

---

### Phase 5 — One root becomes a list  · status: todo

#### Work Order

**Goal:** the grid reads the capture roots of other accounts on the same machine, configured once and resolved once.

**Spec:**

`root()` (`progress.rs:289`) resolves exactly one root: `CARGO_TILE_ROOT` or the `/tmp` default. It becomes a list — the reader's own root, unchanged, plus any additional roots from configuration:

```toml
[capture]
auto_install = true
roots        = ["/var/lib/hana-ci/hana-linux-1/cargo-tile",
                "/var/lib/hana-ci/hana-linux-2/cargo-tile"]
```

`CaptureConfig` already carries `auto_install` (`config.rs:139-146`); `CommandsConfig::excluded` (`config.rs:71`) is the precedent for a user-supplied list and for snapshotting it once at scanner start. The key is additive and defaults empty, so a machine that sets nothing behaves exactly as it does now.

`CARGO_TILE_ROOT` says where a process **writes**. Nothing in it says where the grid **reads**, which is why discovery belongs in configuration rather than in another environment variable. The reading half needs nothing from the nix modules — no file for them to write or own.

Deserialize `capture.roots` as `Vec<PathBuf>`; resolve, validate absolute, and deduplicate **once at scanner start**, not per scan. Intern the resolved roots so a root index rather than a path goes into keys.

*Root-qualified capture identity.* `Capture.logs` is keyed by bare `u32`. With more than one root, two roots can hold a registration under the same pid number, so the key must carry the root as well. Qualify the capture map on the interned root index plus the pid. Do not attempt the wider identity change here — Phase 8 carries root-qualified identity through the roster, families and tiles; this phase's obligation is that two roots holding the same pid number cannot collide in the capture map.

The interned root table stores identity only — **never** deletion authority. `OwnedRoot<'scan>` from Phase 3 borrows the scan; interning must not give it a longer life.

The sweep budget from Phase 3 is already scan-local and threaded across roots; adding roots must not reset or multiply it.

**Files:**
- `crates/cargo-tile/src/config.rs` — `capture.roots` on `CaptureConfig`
- `crates/cargo-tile/src/progress.rs` — `root()` becomes the resolved list; the capture map key gains the root index
- `crates/cargo-tile/src/processes.rs` — the scanner resolves and interns at start and iterates roots per scan
- `crates/cargo-tile/src/terminal.rs` — `:209`, the scanner's actual caller, which changes with the resolved-once signature
- `crates/cargo-tile/src/constants.rs` — the `capture.roots` key and its default

**Seats:** 3 writers — splits by file: the config key, the capture map, and the scanner are independent edits. `cargo-tile` is a binary-only crate, so it has no linkable integration-test target and its tests are inline `#[cfg(test)]` modules inside the files being edited (Delegation Context → **Test lanes**). Each writer writes the inline tests for the file it owns; no seat here can open as `test`.
- `impl` — `crates/cargo-tile/src/progress.rs` — `root()` becomes the resolved list and the capture map key gains the root index, **and each resolved root records where it came from** (the built-in default, the environment variable, or a named `capture.roots` entry) because phase 6 renders that distinction and cannot recover it afterwards; a configured path that does not exist is **retained** in the list rather than dropped, so phase 6 can say it is missing instead of showing nothing. Also `crates/cargo-tile/src/constants.rs` for the new keys and defaults; hub: `crates/cargo-tile/src/progress.rs` (the root-qualified capture key type both other files read)
- `test` — opens as impl: `crates/cargo-tile/src/processes.rs` — resolve and intern once at scanner start, iterate roots per scan — **and `crates/cargo-tile/src/terminal.rs`**, whose `:209` call is the scanner's actual caller and changes with the resolved-once signature; writes into the existing `#[cfg(test)]` module (`:670`)
- `review` — opens as impl: `crates/cargo-tile/src/config.rs` — `capture.roots` on `CaptureConfig` (`:139`), following the `CommandsConfig::excluded` precedent (`:71`, `:85`). Its `#[cfg(test)]` module (`:276`) has only 2 tests and one of them asserts the restated file `contains("[capture]")` (`:308`), so this seat updates that test as part of the change.

**Constraints from prior phases:**
- Phase 3 supplies `OwnedRoot<'scan>` / `ForeignRoot<'scan>`, the per-scan ownership recheck, and the scan-local sweep budget. Roots are opened and classified through it.
- Phase 4 supplies acceptance and the log association; both are per root and must not assume a single root.
- The runner roots this key names are `/var/lib/hana-ci/hana-linux-1/cargo-tile` and `/var/lib/hana-ci/hana-linux-2/cargo-tile`, created `0750 <runner> <runner>` by nix tmpfiles, with `CARGO_TILE_ROOT` set in each unit's environment. They are foreign to the reader and will never be swept.

**Acceptance gate:** Build, Test and Lint green. A test asserting a config with no `capture.roots` produces exactly the single root the code resolves today. A test asserting a registration under the same pid number in two roots yields two capture entries. A test asserting root resolution happens once, not per scan. A test asserting a configured root that does not exist is still present in the resolved list, carrying the source it came from, rather than being dropped — phase 6 has nothing to report about a root that was silently discarded.

---

### Phase 6 — Configured roots get a visible status  · status: todo

#### Work Order

**Goal:** an operator can see, without leaving the grid, which capture roots are being read, who owns them, and why one is producing nothing.

**Spec:**

Without this, a misspelled root, or one whose path a nix change moved, produces exactly the idle grid the feature exists to remove.

*Resolution versus continuing access are different things.* Keep resolution and deduplication at startup, but **retain missing configured paths** rather than rejecting them — a root that does not exist yet must be able to recover when it appears. Refresh ownership and access observations with each scan, composing with Phase 3's per-scan ownership recheck.

*The status has to reach the screen.* `Scan` carries no root diagnostics today, and `drain_scans` (`terminal.rs:401`) requests a redraw only when the command groups change — so a root that becomes unreadable while the grid is empty leaves its displayed status frozen. Carry root status through `Scan`, retain it in `App`, and trigger a redraw when it changes. Perform **no filesystem reads** in `settings::rows` (`settings.rs`); it renders what the scan observed.

*One read-only settings row per effective root*, the implicit environment or default root included, naming:

- its source — `default`, `environment`, or `config`
- the owner account or uid
- the cleanup policy
- the status
- the absolute path

Use specific wording rather than a boolean: `readable — no active captures`, `active — 2 captures`, `missing directory`, `permission denied: state/pids`, `partial — 1 unreadable log`. A foreign root reads `read-only`; a failed ownership check reads `ownership unavailable — cleanup disabled`. Readable-but-empty must be distinguishable from a failed read, and an unused default root stays quiet.

**Files:**
- `crates/cargo-tile/src/processes.rs` — root status on `Scan`
- `crates/cargo-tile/src/terminal.rs` — `drain_scans` redraws when root status changes
- `crates/cargo-tile/src/settings.rs` — the read-only rows, rendered from retained observations only
- `crates/cargo-tile/src/app.rs` — holds the retained root status the overlay reads. The Delegation Context lists this file as read-only for the plan; this phase is the exception, because a status `settings.rs` may not read from disk has to reach it through `App`
- `crates/cargo-tile/src/constants.rs` — the status strings

**Seats:** 3 writers — splits by file: producing the status, propagating a redraw, and rendering the rows. `cargo-tile` is a binary-only crate, so it has no linkable integration-test target and its tests are inline `#[cfg(test)]` modules inside the files being edited (Delegation Context → **Test lanes**). Each writer writes the inline tests for the file it owns; no seat here can open as `test`.
- `impl` — `crates/cargo-tile/src/processes.rs` — root status on `Scan`; hub: `crates/cargo-tile/src/processes.rs` (the status type the other two read)
- `test` — opens as impl: `crates/cargo-tile/src/terminal.rs` — `drain_scans` (`:401`) redraws when root status changes even though the command groups did not; writes into the existing `#[cfg(test)]` module (`:670`). **Also `crates/cargo-tile/src/app.rs`**, which has to hold the retained root status for `settings.rs` to read: the Delegation Context lists `app.rs` as read-only for this plan, and this phase is the exception
- `review` — opens as impl: `crates/cargo-tile/src/settings.rs` — the read-only rows off `rows(app)` (`:98`), plus `crates/cargo-tile/src/constants.rs` for the status strings. **This seat must not introduce filesystem access**: `settings.rs` performs none today, and every value it shows comes from what the scan already retained on `app`.

**Constraints from prior phases:**
- Phase 5 supplies the resolved, deduplicated, interned root list and the `default` / `environment` / `config` distinction.
- Phase 3 supplies per-scan ownership and enumeration outcomes, including the unavailable-versus-empty live set that `permission denied: state/pids` and `partial` report.

**Acceptance gate:** Build, Test and Lint green. A test asserting `settings::rows` performs no filesystem access. A test asserting a root whose status changes while the command groups do not still requests a redraw. A test asserting a configured root that is missing at first and appears later changes its displayed status, with the command groups unchanged across both scans. A test per status string.

---

### Phase 7 — An unknown measurement never renders as a measured zero  · status: todo

#### Work Order

**Goal:** a value the scanner could not read is displayed as unavailable rather than as zero, and a group total containing one is itself unknown.

**Spec:**

`cpu` is a formatted `String` and `managed` a `usize`, so neither can say "not available"; a missing CPU sample becomes zero before the row is built, so changing row fields alone would still preserve a fabricated reading. `compiler: Option` is already spoken for — `None` there means no compiler is running, which is a measurement, not an absence.

- Give the formatted CPU and the managed count a **semantic measurement type** that
  names the two outcomes — a reading, or the reason there is none — rather than a
  bare `Option` on a domain field. `None` at a row says only "absent" and leaves
  every consumer to re-derive what that means; the type has to carry it, because
  the group total, the smoother and family assignment each act on it differently.
- Give compiler observation three states: `Unknown | None | Running`.
- Preserve availability **through collection and smoothing**, not by restoring it at the row.

An `Option` at the row is not sufficient on its own. The group total sums the members it can read and silently skips the ones it cannot, so a group holding an unknown contributor reports a figure lower than reality and indistinguishable from a measured one. **An unknown contributor makes the group total unknown.** Smoothing must stop publishing its previous value once a sample goes unavailable, rather than holding the last reading indefinitely.

Reach: `Census::take`, `CpuSmoothing`, `Roster::assign_families` (the `managed > 0` test), and three call sites in `render.rs`.

An active runner must never read `0%` because the value could not be read. This phase lands before the registration-sourced row so that row can be built on the optional types from the start rather than converted afterwards.

**Files:**
- `crates/cargo-tile/src/processes.rs` — `Census::take`, `CpuSmoothing`, the row fields, and the group totals `aggregate_cpu` / `aggregate_compilers` (`:1033`)
- `crates/cargo-tile/src/render.rs` — how an unavailable value renders
- `crates/cargo-tile/src/roster.rs` — `assign_families` (`:382`), whose `managed > 0` comparison and inline fixtures both change with the measurement type
- `crates/cargo-tile/src/constants.rs` — the unavailable-value presentation literals

**Seats:** 3 writers — the measurement type reaches three files, not two: collection and smoothing, rendering, and family assignment. `cargo-tile` is a binary-only crate, so it has no linkable integration-test target and its tests are inline `#[cfg(test)]` modules inside the files being edited (Delegation Context → **Test lanes**). Each writer writes the inline tests for the file it owns; no seat here can open as `test`.
- `impl` — `crates/cargo-tile/src/processes.rs` — `Census::take`, `CpuSmoothing`, the row fields, **and the group totals: `aggregate_cpu` and `aggregate_compilers` (`:1033`) live here, not in `render.rs`**; plus `crates/cargo-tile/src/constants.rs`; hub: `crates/cargo-tile/src/processes.rs` (the measurement type all three files read)
- `test` — opens as impl: `crates/cargo-tile/src/render.rs` — how an unavailable value renders wherever a total or a per-row figure is drawn; writes into the existing `#[cfg(test)]` module (`:2009`). **The Work Order previously named three CPU/managed summations in this file; they are not here** — the arithmetic is in `processes.rs` and this seat renders its result
- `review` — opens as impl: `crates/cargo-tile/src/roster.rs` — `assign_families` (`:382`) compares `managed > 0` directly and its inline fixtures construct measurements by hand, so both change with the type; writes into the existing `#[cfg(test)]` module (`:439`)

**Constraints from prior phases:** none binding — this phase is independent of the capture work and touches no root, registration, or shim code.

**Acceptance gate:** Build, Test and Lint green. A test asserting an unavailable CPU sample renders as unavailable rather than `0%`. A test asserting a group total containing an unknown contributor is unknown. A test asserting `CpuSmoothing` stops publishing a stale value.

---

### Phase 8 — Invocation identity and capture membership  · status: todo

#### Work Order

**Goal:** every invocation carries a stable identity that survives a change of source between scans, and each capture is read once per scan rather than once per row.

**Spec:**

*1. Identity must reach past `Capture.logs`.* Qualifying only the log map leaves `read(pid)` ambiguous, and group identity, roster matching, family colours and tile identity all still key on a bare pid. There is a second problem underneath: a registration row uses the **shim** pid while its preferred process row uses the **cargo** pid, so a run that switches source between scans makes the roster retire one tile and open another.

Carry a stable captured-invocation identity of root, shim pid and run generation through capture lookup, deduplication, roster and tiles, with the **displayed** pid kept separate from the identity.

*2. Identity is not membership.* One capture legitimately supplies progress to several invocations — the shim skips nested captures on purpose — so giving every associated row the same run identity merges distinct commands into one. Model it as `InvocationId::Captured(RunId) | Process(ProcessIdentity)`. Only the invocation a registration **directly** represents takes the working directory and arguments from it; a nested invocation keeps its own identity and merely references the enclosing `RunId`.

One `RunId` inside `Captured` cannot carry both of those relationships: the same value would mean "this registration describes me" for one row and "I ran underneath that registration" for another, and every consumer would then re-derive which it is holding. Give the direct case its own type — a registration that Phase 4 verified, holding the working directory and arguments it is allowed to supply — and let membership in an enclosing capture be a separate field that carries only the `RunId` it is nested under. Row creation in Phase 9 takes the verified-registration type and nothing else, so a nested invocation cannot reach the fields it must not inherit, and no row-building path needs a runtime check to keep the two apart.

*3. `is_shim` accepts absence as evidence.* `is_shim` (`processes.rs:752`) compares `subcommand(outer.cmd()) == subcommand(inner.cmd()) && outer.cwd() == inner.cwd()`. When both sides are `None` the cwd test passes, so two unavailable fields count as a match — and cwd is exactly the field the runner machine denies. Require observed equality, not matching absence.

*4. Membership is resolved before progress is parsed.* `captured_run` (`processes.rs:918`) establishes association only when parsing yields a `RunState`, so a valid registration whose log has printed nothing yet reads as no association at all. Resolve membership independently of progress parsing.

*5. Read each capture once per scan.* Every row calls `captured_run`, and every hit opens, stats, seeks, reads and parses the log again — ten rows sharing one capture at the full tail cap read and parse 640 KiB per scan, and rows from the same scan can disagree about progress because they read at different instants. Phase 3's hardened open would repeat on each of those reads. Resolve membership through the identity above, read each referenced capture **once per scan** into the scan's capture data — caching the unsuccessful reads too — and copy the resulting outcome into matching rows; `RunState` is already `Copy`. One stored result per capture, and no cache needing invalidation across scans.

What the cache stores is a named capture-read outcome, not `Option<RunState>`. Three things happen when a capture is read and the rows need to tell them apart: progress was parsed, the log opened but has printed nothing to parse yet, and the read itself did not succeed. A bare `None` collapses the last two, and the second is the ordinary state of a run that has just started — so a row cannot distinguish "no progress yet" from "this capture is unreadable" without asking the filesystem again, which is the per-row read this item removes. Name the three outcomes in the type the scan stores, and let the row decide what to display from the outcome rather than from an absence.

*6. Order within the scan.* Verify registrations against Phase 4's evidence, prune, establish direct association, resolve each invocation's command, and only then decide row eligibility.

*7. Exclusion applies to one set only.* `commands.excluded` prunes the process row at `processes.rs:440`, but the shim's hardcoded skip list covers only `metadata`/`tile`/`port`/`berth` and friends — a user-configured `excluded = ["clippy"]` still gets a registration. "No process-table row" therefore includes rows removed **on purpose**, and Phase 9's fallback would resurrect them, breaking an existing configuration contract for local captures as well as CI ones. Apply exclusion to both sources before fallback selection and attribution, keep deliberate exclusion distinct from unavailable metadata, and apply it **only** to the row-owning and attribution set — never to `Census.parents` and never to the capture liveness set, or an excluded live capture becomes a sweep candidate and loses artifacts it is still writing.

**Files:**
- `crates/cargo-tile/src/processes.rs` — `InvocationId`, `is_shim` (`:752`), `captured_run` (`:918`), the exclusion retain at `:440`, the liveness callback at `:462`, scan-scoped capture reads
- `crates/cargo-tile/src/progress.rs` — capture lookup keyed by the new identity
- `crates/cargo-tile/src/roster.rs` — matching (`:194`) and the family maps, both keyed on the pid today
- `crates/cargo-tile/src/tiles.rs` — the demand and content identifiers, likewise
- `crates/cargo-tile/src/render.rs` — the renderer entry point that still takes a pid
- `crates/cargo-tile/src/constants.rs` — any literal the identity or the scan cache needs

  `roster.rs`, `tiles.rs` and `render.rs` are listed read-only in the Delegation Context and in this Work Order's earlier constraint. **That does not hold for this phase**: the identity these files key on is exactly what the phase replaces, so a change that stops at `processes.rs` and `progress.rs` leaves PID-based identity live in the consumers and the phase does not achieve its goal.

**Seats:** 3 writers — the identity reaches further than the two files this phase originally named: `roster.rs`, `tiles.rs` and `render.rs` all still key on the pid, so a third writer takes those consumers while the other two establish the identity and the capture lookup. `cargo-tile` is a binary-only crate, so it has no linkable integration-test target and its tests are inline `#[cfg(test)]` modules inside the files being edited (Delegation Context → **Test lanes**). Each writer writes the inline tests for the file it owns; no seat here can open as `test`.
- `impl` — `crates/cargo-tile/src/processes.rs` — `InvocationId`, `is_shim` (`:752`), `captured_run` (`:918`), the exclusion retain (`:440`), the liveness callback (`:462`), scan-scoped capture reads; plus `crates/cargo-tile/src/constants.rs`; hub: `crates/cargo-tile/src/processes.rs` (`InvocationId` is defined here and read by all three files). Land the type first — the other two cannot compile without it
- `test` — opens as impl: `crates/cargo-tile/src/progress.rs` — capture lookup keyed by the new identity, and the once-per-scan read that replaces the per-row read; writes into the existing `#[cfg(test)]` module (`:513`)
- `review` — opens as impl: `crates/cargo-tile/src/roster.rs` (matching at `:194` and the family maps), `crates/cargo-tile/src/tiles.rs` (the demand and content identifiers) and `crates/cargo-tile/src/render.rs` (the renderer entry point that takes a pid). The `is_shim` defect — two unavailable cwds comparing equal — is the highest-value single test in the phase and belongs with the `processes.rs` writer

**Constraints from prior phases:**
- Phase 5 supplies the interned root index and the root-qualified capture map key; the identity here extends that key rather than replacing it.
- Phase 4 supplies verification and the `Confirmed | Ended | Unknown` outcome; membership resolution consumes it and must not re-derive it.
- Phase 7 supplies the optional CPU, optional managed count, and three-state compiler observation; rows built here use them directly.

**Acceptance gate:** Build, Test and Lint green. A test asserting a run that switches between a registration source and a process source across two scans keeps one tile. A test asserting a capture referenced by many rows is read once per scan. A test asserting an excluded command produces no row but is still counted live for sweep purposes.

---

### Phase 9 — The registration becomes a row source  · status: todo

#### Work Order

**Goal:** a cargo run the process table cannot describe appears on the grid, labelled by the account that owns it.

**Spec:**

This is the item the feature exists for. The shim already writes the field that is missing and the grid discards it: `live_runs` (`progress.rs:302`) parses only the **filename** as a pid and never opens the file. So a registration can *supply* a row rather than only annotate one, and an unreadable argv costs no columns.

*1. The new direction.* Today the linkage runs one way: `captured_run` (`processes.rs:918`) walks **upward** from a cargo pid to find a registered ancestor and read its state. That walk stays as it is for every row the process table can already build. What is new is the other direction, for a run the process table cannot describe: the registration names a live shim pid, and the two fields the row is missing are in the file. Give `Capture` a sibling to `read(pid) -> Option<RunState>` that returns the parsed registration — working directory and command text — and let a row be built from it.

That sibling returns a named outcome, not a bare optional. Its caller has to separate three cases and cannot do so from an absence: the registration parsed and Phase 4 confirmed it, so it may source a row; it parsed but verification came back `Unknown` or the record is a legacy one, so it may annotate an existing row and nothing more; or it could not be read at all. The middle case is the one a bare `None` hides, and it is the case that decides whether a row exists. Return the verified-registration type Phase 8 defines for the first case, and name the other two, so the row builder reads the outcome rather than testing for emptiness.

| field | source | note |
| --- | --- | --- |
| `path` | registration | the shim already collapsed `$HOME` to `~` |
| `command` | registration | `cargo <args>`, as typed, one field per argument |
| `pid` | the registration's filename | the shim's pid, a live process |
| `started` | the registration file's **mtime** | see below |
| `cpu`, `compiler`, `managed` | process table where it answers | absent rather than wrong |

Using the registration's mtime as the run start avoids the one call least certain to answer across a uid boundary. The shim writes the file once, at the moment the run begins, and never rewrites it; the reader already stats the entry. Whether sysinfo's macOS backup path carries a usable start time is untested, so nothing here may depend on it.

*2. Precedence is per field, never per row.* Preferring a whole process-table row discards a working directory the registration has, because `row()` (`processes.rs:1069-1083`) degrades a missing cwd to `"unavailable"` rather than failing. That is the ordinary runner-machine case, not a corner: there argv is readable and cwd is denied, so the row survives with `path: "unavailable"` while the registration holds the real directory. The process row keeps its real pid, its tree and its measured values; an absent cwd is filled from the matching registration; and the merge happens **before** any field becomes display text.

Only the invocation a registration directly represents takes cwd and arguments from it — Phase 8's `InvocationId::Captured` distinction. A nested invocation does not inherit them.

*3. The account has to be visible.* `CargoProcess.path` is already display text and the summary groups by that string (`render.rs:1347`), so two accounts both reporting `~/x` share one directory heading and one progress gauge — and the `~` in a foreign registration means the **writer's** home, so a `~` from a runner root means `/run/github-runner/hana-linux-1`, which an operator reads as their own.

Group on the verified account, the root identity **and** the working directory rather than on the display text. Each of the three separates runs the other two leave merged. Dropping the directory merges every run one account has going in different checkouts into a single heading and a single gauge, which is the more common shape on a build machine than the collision this item started from; dropping the account or the root merges the two accounts back together. The directory in the key is the identity the registration carries, not the `~`-collapsed text the heading shows. Family colour cannot substitute for any of it, since it means process ancestry and is absent for a command with no children. Present the account as a **prefix in the heading**: `[hana-linux-1] ~/x` in tile and summary headings. It appears once per heading, where the grouping already lives, rather than spending width on every row. Derive the account name from the root's owner, not from the path text.

**Files:**
- `crates/cargo-tile/src/progress.rs` — the registration accessor beside `read`
- `crates/cargo-tile/src/processes.rs` — the registration-sourced row, the per-field merge, `row()` (`:1069-1083`)
- `crates/cargo-tile/src/render.rs` — grouping on account, root and working directory (`:1347`), the heading prefix
- `crates/cargo-tile/src/constants.rs` — the heading prefix's presentation literals

**Seats:** 3 writers — splits by file; the behaviour is a merge across three existing modules. `cargo-tile` is a binary-only crate, so it has no linkable integration-test target and its tests are inline `#[cfg(test)]` modules inside the files being edited (Delegation Context → **Test lanes**). Each writer writes the inline tests for the file it owns; no seat here can open as `test`.
- `impl` — `crates/cargo-tile/src/processes.rs` — the registration-sourced row and the per-field merge at `row()` (`:1069-1083`), where `path` degrades to `UNRESOLVED_PATH` but `command` fails the whole row; hub: `crates/cargo-tile/src/processes.rs` (the row builder the other two feed and read)
- `test` — opens as impl: `crates/cargo-tile/src/render.rs` — `group_by_path` (`:1347`) keys on verified account, root and working directory instead of display text, and headings gain the account prefix; plus `crates/cargo-tile/src/constants.rs` for the prefix's presentation literals, which are this writer's alone in this phase; writes into the existing `#[cfg(test)]` module (`:2009`)
- `review` — opens as impl: `crates/cargo-tile/src/progress.rs` — the registration accessor beside `read` (`:282`)

**Constraints from prior phases:**
- Phase 4 supplies the parsed record: working directory, argument count and arguments as separate fields, and the `Confirmed | Ended | Unknown` outcome. Only `Confirmed` may source a row; `Unknown` and legacy records annotate only.
- Phase 8 supplies `InvocationId::Captured(RunId) | Process(ProcessIdentity)`, the resolved membership, the scan-scoped capture reads, and the exclusion ordering. A row may not be built for an excluded command.
- Phase 7 supplies the optional CPU and managed count and the three-state compiler observation; a registration-sourced row leaves all three unavailable rather than zero.
- Phase 6 supplies the owner account per root, which the heading prefix and the grouping key both use.
- Phase 5 supplies the interned root index that qualifies the identity.

**Acceptance gate:** Build, Test and Lint green. A test asserting a registration with no matching process-table row produces a row carrying its working directory and command. A test asserting a process row with an unavailable cwd and a matching registration renders the registration's directory while keeping the process row's pid and measured values. A test asserting two roots reporting the same `~/x` produce two headings, each prefixed with its own account. A test asserting two working directories under one account and one root produce two headings rather than one. A test asserting an `Unknown` registration sources no row.

### Phase 10 — A reservation retires when its work reaches trunk  · status: todo

#### Work Order

**Goal:** A reservation whose commits are in trunk and whose worktree is clean
retires on its own, so finished work stops holding files against everyone else.

**Spec:**

Today a reservation is held until something explicitly releases it. When a run
ends without that release, the claim outlives the work forever. Observed
2026-09-09: reservation `01a06fde`, opened 2026-09-05 by the `cargo-liner`
worktree, still held 40+ files four days after its work was committed and
pushed and its worktree returned to clean. It blocked an unrelated phase in a
second worktree and raised incursion `01a0887c`. The two worktrees had separate
working trees on different branches and could not collide on disk.

Retention exists so an interrupted run cannot silently drop its protection.
That reasoning does not survive integration: once the work is in trunk there is
nothing left to protect and the claim is pure obstruction. The cost lands on
whoever comes next, who did nothing wrong and has no way to tell a live claim
from a dead one.

`resolve --integrated-as <TRUNK_OID>` already exists, but it is a manual
assertion, and its own help warns that a wrong commit releases an unresolved
reservation. So the safe path is the one nobody runs. Make it automatic and
evidence-based instead.

Retire a reservation during board evaluation when **both** hold:

1. every commit the reservation records is an ancestor of trunk, and
2. its worktree is clean — no uncommitted change inside its own scopes.

Both, never either alone. A clean worktree whose commits are not in trunk is
unintegrated work, and unreachable commits with a dirty tree are work in
progress; retiring either would discard exactly what retention protects.

The ancestry machinery already exists: the board reports
`protected_predecessor_ancestry_queries` and
`worktree_ahead_behind_computations` in its git cost, so this reads existing
capability rather than adding a git path. Keep the per-invocation git cost
bounded — the board is read constantly and must not become an O(reservations)
ancestry walk. Reuse the batched ancestry the board already issues rather than
adding a query per reservation, and hold the bound across the whole read: the
cost that matters is reservations multiplied by worktrees, since each verified
holder needs its own cleanliness observation.

*Where the rule runs, and what it may read.* Three placements here are not
interchangeable, and getting them wrong puts a durable mutation outside the lock
that protects it:

- `retention.rs` **replays journal operations**. The rule that decides a
  reservation is retirable is not a replay step, and a replay function is not
  where a new board-time decision goes.
- `lifecycle.rs` owns the disposition and its permitted transitions.
- The automatic mutation itself belongs in **locked reconciliation** —
  `prepare_reconciliation_transaction` (`reconcile.rs:949`) — where a board read
  already holds the lock and already produces journal operations. A retirement
  decided outside it races every other board reader.
- `git/mod.rs` only re-exports; the ancestry queries are implemented in
  `git/reachability.rs`, which is the file this phase actually reads and extends.

The cleanliness half needs an observation that does not exist yet. Today's
worktree observations carry liveness and HEAD — enough to say a worktree is
still there and where it points, and nothing about its index. "No uncommitted
change inside its own scopes" needs **staged, unstaged and untracked** paths
observed per verified holder, intersected with that holder's scope set. Add
those observations rather than inferring cleanliness from HEAD.

Evidence that cannot be obtained is not evidence of cleanliness. A worktree that
cannot be observed — gone, unreadable, a git invocation that failed — makes the
reservation **ineligible** for automatic retirement, and it stays held. The
whole rule is that retirement follows proof; an unanswered question is not proof.

Record the retirement as its own disposition — retired-on-integration — so the
journal still shows where the work went. It is not an abandonment and must not
be reported as one: nothing was discarded.

*The transition has to be durable, and today's replay refuses it.* Two rules in
the shipped code stand between this phase and a recorded retirement, and both
are there deliberately:

- `ReservationLifecycle::release` (`lifecycle.rs:85`) rejects a release from
  `Active` with `ReleaseRequiresCheckpoint`. A reservation that never
  checkpointed cannot take the ordinary integrated release path at all — and a
  reservation retiring on integration evidence may well be one that never
  checkpointed.
- `apply_release` (`retention.rs:1047`) rejects
  `ReleaseDisposition::Integrated` unless the reservation already carries
  `IntegrationEvidenceStatus::Integrated { .. }`, with
  `IntegratedReleaseWithoutEvidence`.

The only two dispositions that release from `Active` today are `Abandoned` and
`RetiredOrphan`, both of which go through `release_after_user_confirmation` —
which is exactly why a checkpoint-free release currently means "the user threw
this away" or "the user confirmed an orphan". Automatic retirement is neither,
so it cannot borrow either path.

Define the transition so that all of this holds:

1. The retirement **records the evidence it acted on** — the verified commits
   and the trunk commit they are ancestors of — into the journal alongside the
   disposition, not only the fact of retirement.
2. **Replay reconstructs it.** Reading the journal back produces the same
   retired reservation with the same evidence, without the replay path needing
   to re-run git.
3. The recorded evidence **participates in later revalidation** the way other
   integration evidence does, rather than being a terminal note nothing checks
   again.
4. **Successor retention refs are preserved.** A reservation retiring must not
   drop the retention refs a successor still depends on
   (`RetentionRefStatus`, `alert.rs:263`).
5. **Obsolete identity mappings retire with it.** `verb/release.rs` publishes a
   session identity mapping on release (`SessionIdentityMappingPublication`);
   the automatic path owes the same bookkeeping, or a retired reservation leaves
   a mapping pointing at nothing.

Leave the squash and cherry-pick case alone. Where the recorded commits are not
ancestors of trunk but the work did reach it in rewritten form, the tool cannot
prove integration, and `--integrated-as` stays the manual answer for it.

**Files:**
- `crates/cargo-berth/src/reservation/retention.rs` — replay of the new operation (`apply_release`, `:1047`).
- `crates/cargo-berth/src/reservation/lifecycle.rs` — the disposition and its permitted transition (`release`, `:85`; `ReleaseDisposition`, `:189`).
- `crates/cargo-berth/src/reservation/record.rs` — the recorded evidence on the reservation (`:49`).
- `crates/cargo-berth/src/ledger/journal.rs`, `crates/cargo-berth/src/ledger/projection.rs` — the durable operation and its projection.
- `crates/cargo-berth/src/reconcile.rs` — `prepare_reconciliation_transaction` (`:949`), where the board-time decision is made under the lock.
- `crates/cargo-berth/src/git/reachability.rs` — the batched ancestry queries this reuses; `crates/cargo-berth/src/git/mod.rs` only re-exports them.
- `crates/cargo-berth/src/worktree/` — the staged, unstaged and untracked observations the cleanliness check needs.
- `crates/cargo-berth/src/board/`, `crates/cargo-berth/src/edge/`, `crates/cargo-berth/src/alert.rs`, `crates/cargo-berth/src/recovery.rs`, `crates/cargo-berth/src/verb/release.rs` — successor retention refs and the identity-mapping bookkeeping the automatic path inherits.
- `crates/cargo-berth/src/output.rs`, `crates/cargo-berth/src/output_contract.rs`, `crates/cargo-berth/src/constants.rs` — how the new disposition reports.
- `crates/cargo-berth/tests/lifecycle.rs`, `crates/cargo-berth/tests/ledger.rs`, `crates/cargo-berth/tests/board.rs`, `crates/cargo-berth/tests/edges.rs`, `crates/cargo-berth/tests/output_contract.rs` — the lane.

**Seats:** `2 writers + 1 tester`, split between the durable lifecycle and everything that observes or reports it. `cargo-berth` has a real integration-test lane (Delegation Context → **Test lanes**), and the rule above is concrete enough to test before the code exists, so the tester opens as `test`.
- `impl` — opens as `impl`. Owns `crates/cargo-berth/src/reservation/` (`retention.rs`, `lifecycle.rs`, `record.rs`), `crates/cargo-berth/src/ledger/journal.rs` and `crates/cargo-berth/src/ledger/projection.rs`; hub: `crates/cargo-berth/src/reservation/record.rs` — the lifecycle and durable-evidence hub the other writer reads. Land the disposition and its journal operation first; the observation side cannot compile against a transition that does not exist yet
- `review` — opens as writer: `crates/cargo-berth/src/reconcile.rs`, `crates/cargo-berth/src/git/`, `crates/cargo-berth/src/worktree/`, `crates/cargo-berth/src/board/`, `crates/cargo-berth/src/edge/`, `crates/cargo-berth/src/alert.rs`, `crates/cargo-berth/src/recovery.rs`, `crates/cargo-berth/src/verb/release.rs`, `crates/cargo-berth/src/output.rs`, `crates/cargo-berth/src/output_contract.rs`, `crates/cargo-berth/src/constants.rs` — the board-time decision, the observations it reads, and how the result reports
- `test` — opens as `test`. Owns `crates/cargo-berth/tests/lifecycle.rs`, `crates/cargo-berth/tests/ledger.rs`, `crates/cargo-berth/tests/board.rs`, `crates/cargo-berth/tests/edges.rs` and `crates/cargo-berth/tests/output_contract.rs`

**Constraints from prior phases:** None. This phase is independent of phases 1-9
and may run before or after them.

**Acceptance gate:** `verify.sh check cargo-berth`, `verify.sh test cargo-berth`
and `verify.sh lint cargo-berth` green — this phase edits no `cargo-tile` file,
so the Delegation Context's default `cargo-tile` gate would pass without
compiling anything this phase wrote. A test that a reservation whose
commits are ancestors of trunk and whose worktree is clean is retired without an
operator disposition. A test that the same reservation with an uncommitted
change inside its own scopes is **not** retired. A test that a clean worktree
whose commits are not ancestors of trunk is **not** retired. A test that the
recorded disposition reads as retired-on-integration and not as an abandonment.
A test that retirement releases the files for a second worktree that previously
raised an incursion against them. A test that a worktree whose cleanliness
cannot be observed is left held rather than retired. A test that reading the
board repeatedly produces exactly one retirement transition, not one per read. A
test that replaying the journal reconstructs the retired reservation and its
recorded evidence without re-running git. A test that a trunk rewrite after
retirement is handled without the recorded evidence going stale unnoticed. A
test that an incursion previously recorded against the reservation receives its
own disposition rather than disappearing with the retirement.

**Pending decision: what counts as proof that a reservation's work reached trunk**

Actual problem:
The rule above says "every commit the reservation records is an ancestor of
trunk", and `Reservation` (`reservation/record.rs:49`) records no commit list. It
holds `phase_start_head` (the acquisition baseline), `head_snapshot`,
`retained_protected_tip` (the eventual checkpoint tip) and
`integration_trunk_snapshot`. Something has to stand in for "the reservation's
work", and the wrong stand-in retires live claims.

What exists now:
- A reservation that has checkpointed has a protected tip, and reconciliation
  already makes it nonblocking once integration is proved — including through
  the existing scoped-patch equivalence path. That part of the problem is
  already solved; what this phase adds is **terminal** retirement, and handling
  reservations that are still `Active`.
- A reservation still `Active` has never checkpointed, so it has no protected
  integration subject at all. Its only commit-shaped fact is the baseline it
  acquired at.
- Testing the baseline, or testing an empty set, both answer "yes, integrated"
  for a reservation that has not yet made a single edit — the ancestry question
  is trivially true when there is nothing to ask about. Combined with a clean
  worktree, that retires a freshly opened claim before its first edit, which is
  the precise failure this phase must not introduce.

What should change:
- Settle what the ancestry test is applied to, for each of the two shapes: a
  reservation with a protected tip, and one still `Active` with none.
- Whatever the answer, the emptiness case is not eligible: a reservation with no
  observed work is held, never retired.
- Acceptance has to include a fresh claim with no edits yet, and a reservation
  carrying commits made after its last checkpoint, since those are the two
  states a baseline test silently passes.

Recommendation:
Require the reservation to have produced observable work before the ancestry
question is asked at all, and derive that work from the reservation's own
worktree rather than inventing a commit list on the record: the commits between
`phase_start_head` and the worktree's current HEAD, restricted to the
reservation's scopes. An empty result means nothing to integrate and the
reservation stays held — never retired. That keeps a still-`Active` reservation
eligible once it has actually committed something, without it having
checkpointed, and it leaves the checkpointed case reading its protected tip as
it does today.

Any answer must also satisfy the durable-transition requirements above: the
evidence it acts on is recorded in the journal, replay reconstructs it without
re-running git, it revalidates later, successor retention refs survive, and the
identity mapping retires with the reservation.

### Phase 11 — Drift widening stays inside the work it protects  · status: todo

#### Work Order

**Goal:** A long-lived reservation stops accreting files it never worked on, so
a claim keeps meaning what it says.

**Spec:**

A reservation widens its scopes when drift shows it touched something new. That
is right in itself — a claim has to cover what the work actually did. What is
missing is any tie back to the work being protected, so on a busy branch the
claim grows without limit.

Observed on reservation `01a06fde`: 45 recorded overlap answers, every one
`widen_without_foreign_overlap` with cause `drift`. A reservation protecting a
change in `crates/cargo-berth` finished up owning the whole of
`crates/cargo-tile`, plus `crates/cargo-mend/CHANGELOG.md` and two root handoff
files. By the end its scope list described the repository, not the work.

Bound the widening. The rule is a design decision this phase must settle and
record, not one to pick silently; evaluate at least these and say why the chosen
one wins:

- confine widening to the crates the reservation's own commits touch;
- expire scopes no commit of this reservation ever modified;
- require a purpose at first touch and hold widening to it.

Note that `01a06fde` carried `purpose: not_provided_by_caller`. That is not
incidental — with no purpose recorded there is nothing for a widen to be checked
against, which is why the third option is about more than reporting.

Whatever rule lands, a widen that the rule refuses must fail visibly rather than
silently narrowing the claim: a claim that quietly stops covering what the work
touched is worse than one that grows, because the protection disappears without
anyone being told.

*Where a widen actually comes from, and where it lands.* The bound has to be
enforced at both producers, and neither of them is the file this phase first
named:

- `JournalOperation::Widen` is emitted from **two** places —
  `drift/classification.rs:258`, the drift path this incident came through, and
  `widen_first_touch_reservation` (`verb/claim.rs:1151`, emitting at `:1176`),
  the first-touch reuse path. A bound enforced in one is a bound the other walks
  around.
- The scope set is mutated by `apply_widen` (`reservation/retention.rs:958`) on
  replay. That is where a widen becomes a wider claim.
- `drift/selection.rs` chooses which reservations are subjects of the
  comparison, and `reservation/partition.rs` defines coverage and binding —
  `is_foreign`, `authorizes`, `reservations_authorize_scope`. Neither is the
  mutation, and a change made only in them changes what is compared rather than
  what is claimed.

*A refusal is a result, not an absence.* Carry the refusal as its own outcome
through classification, reporting, output and execution rather than dropping the
operation. One consequence in particular has to be handled: the drift comparison
publishes a working-tree fingerprint (`drift/fingerprint.rs`,
`publish_fingerprint`) so the next run can compare cheaply. If a refused widen
still publishes the fingerprint, the next invocation sees no drift for those
paths and the refusal never surfaces again — the change is hidden rather than
reported. A refused widen must leave the next comparison able to see the same
paths.

**Files:**
- `crates/cargo-berth/src/drift/classification.rs` — the drift producer of `JournalOperation::Widen` (`:258`) and where a refusal is classified.
- `crates/cargo-berth/src/drift/selection.rs` — which reservations are subjects of the comparison.
- `crates/cargo-berth/src/drift/fingerprint.rs` — the published working-tree fingerprint a refusal must not hide behind.
- `crates/cargo-berth/src/drift/observation.rs` — what `phase_start..HEAD` does and does not establish (`:194`).
- `crates/cargo-berth/src/verb/claim.rs` — the first-touch reuse producer, `widen_first_touch_reservation` (`:1151`, emitting at `:1176`).
- `crates/cargo-berth/src/reservation/retention.rs` — `apply_widen` (`:958`), the scope-set mutation.
- `crates/cargo-berth/src/reservation/partition.rs` — coverage and binding, which the bound is expressed against.
- `crates/cargo-berth/src/ledger/journal.rs` — the `Widen` operation itself (`:362`) and any refusal it carries.
- `crates/cargo-berth/src/output.rs`, `crates/cargo-berth/src/cli.rs` — how a refused widen reports and how execution ends.
- `crates/cargo-berth/tests/drift.rs`, `crates/cargo-berth/tests/overlap.rs`, `crates/cargo-berth/tests/hooks.rs`, `crates/cargo-berth/tests/output_contract.rs` — the lane.

**Seats:** `1 writer + 1 tester + reserve`, because the bound is one rule that
both producers and the replay have to agree on, and splitting it would put
halves of one judgment in different heads. `cargo-berth` has a real
integration-test lane (Delegation Context → **Test lanes**), so the tester opens
as `test`.
- `impl` — opens as `impl`. Owns `crates/cargo-berth/src/drift/`, `crates/cargo-berth/src/reservation/`, `crates/cargo-berth/src/verb/claim.rs`, `crates/cargo-berth/src/ledger/journal.rs`, `crates/cargo-berth/src/output.rs` and `crates/cargo-berth/src/cli.rs`; hub: the widening policy itself, which both producers and `apply_widen` read
- `test` — opens as `test`. Owns `crates/cargo-berth/tests/drift.rs`, `crates/cargo-berth/tests/overlap.rs`, `crates/cargo-berth/tests/hooks.rs` and `crates/cargo-berth/tests/output_contract.rs`
- `review` — reserve. Reads the chosen rule against the recorded incident and against both widen producers

**Constraints from prior phases:**
- Phase 10 supplies retirement-on-integration. A reservation that retires when
  its work lands is exposed to far less drift, so this phase bounds what remains
  rather than carrying the whole problem alone. Do not treat phase 10 as making
  this unnecessary: a claim held across a long-running branch still widens.

**Acceptance gate:** `verify.sh check cargo-berth`, `verify.sh test cargo-berth`
and `verify.sh lint cargo-berth` green — this phase edits no `cargo-tile` file,
so the Delegation Context's default `cargo-tile` gate would pass without
compiling anything this phase wrote. A test that drift outside the
bound does not widen the reservation. A test that drift inside it still does —
the protection this exists for must survive the fix. A test that a refused widen
is reported rather than silently dropped. A test reproducing the observed shape:
a reservation whose commits touch one crate does not come to own a second crate
through repeated drift answers. A test that the same bound refuses the same
widen on the first-touch reuse path, not only on the drift path. A test that a
refused widen leaves the next comparison able to see the same paths rather than
being hidden by a published fingerprint.

**Pending decision: which rule bounds a widen, and what it does before there is anything to bound it against**

Actual problem:
The Spec above lists three candidate rules and says the phase must settle one.
Two of the three do not survive contact with the shipped code, and the third has
no executable meaning yet, so the choice cannot be left to the delegate.

What exists now:
- `drift/observation.rs:194` states outright that a reservation's
  `phase_start..HEAD` range "is a comparison, not an authorship record": once a
  second run commits onto the same branch in the same worktree, the incumbent's
  earlier paths sit inside the range too. Deriving "the crates this
  reservation's own commits touch" from that same range is therefore circular —
  the range grows with other runs' work, so the bound widens for the same reason
  the claim does.
- Classification already excludes restored paths and historical committed-only
  paths outside HEAD's own changes. So part of what reservation `01a06fde` looks
  like — unrestricted accretion — is already constrained today, and the rule
  only has to bound what gets past those exclusions.
- Expiring scopes no commit ever modified removes protection from work that is
  edited but not yet committed. That is exactly the window a claim exists to
  cover, and `01a06fde` spent days in it.
- `purpose` is free text and was `not_provided_by_caller` on the observed
  reservation. Free text supplies no boundary anything can evaluate, and a rule
  that requires one has to say what happens when it is absent.

What should change:
- Pick the rule, in terms the code can evaluate, and say what it is applied to
  when the reservation has committed nothing yet, when the touched path is a
  shared root file belonging to no crate, and when no purpose was recorded.
- Say what a claim protects during the uncommitted window, since the answer
  cannot be "nothing".
- Resolving this may add or move files in the Files list above; the production
  ownership recorded there stands independently of which rule wins.

Recommendation:
Bound the widen by what the acting run can be attributed, using the same
attribution `observation.rs` already defines — HEAD's own commit plus the
working tree — rather than the full `phase_start..HEAD` range, and take the
bound as the set of crates that attribution covers. That reuses a distinction
the code already draws for this exact reason instead of adding a second one, and
it sidesteps the circularity, since the attributed set does not grow when
another run commits. For the three edge shapes: a reservation with nothing
attributed yet widens freely within its own worktree, which is the pre-commit
window and the thing retention exists to cover; a shared root file is attributed
to the acting run the same way any other path is, and is not treated as a crate;
and a missing purpose does not block a widen, because purpose is reporting, not
policy. Do not make purpose the boundary until it is something other than free
text.
