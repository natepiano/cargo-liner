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
    tests/
      shim_modes.rs                          built-binary shim modes (phase 2)
      shim_registration.rs                   built-binary reader harness (phases 2, 4)
  ```

- **Key files:**

  - `crates/cargo-tile/src/main.rs` — the crate's **module declaration hub**, 33
    lines: a flat `mod` list (`:4-29`) and `fn main() -> ExitCode {
    cli::Cli::parse_arguments().run() }` (`:33`). Every new module in phases 3 and
    4 must be declared here, which makes it a hub file those phases assign to
    exactly one writer.

  - `crates/cargo-tile/src/progress.rs` — reads the capture root. `Progress`
    `:100`; `Phase` `:130`; `RunState` `:156`; `CaptureLookup` `:182` —
    `Unregistered | Registered(CaptureRead)`; `CaptureRead` `:202` —
    `Progress(RunState) | NoCurrentProgress | Unreadable(CaptureFailure)`;
    `CaptureFailure` `:222`, which retains the error kind and message;
    `RegisteredRuns` `:395`; `RegistrationEvidence` `:418`; `ConfirmedCapture`
    `:427` — a verified registration with its descriptor-bound modification time
    held as `Result<SystemTime, CaptureFailure>`, so verification can succeed
    while the timestamp read fails; `RegistrationName` `:437` — `Legacy |
    Generated | Staging | Unrelated`, a classification of a filename that proves
    nothing about process identity; `Capture` `:460`, holding two `CaptureKey`-keyed stores, `readings` (read outcomes) and `confirmed` (verified registrations),
    with `take()` `:471`, `take_from(root, observe)` `:498`,
    `take_roots(roots, observe)` `:507` — the single per-scan removal allowance
    across every root — `scan_root` `:564`, `read(key) -> CaptureLookup` `:673`,
    a map lookup that performs no filesystem access, and `confirmed() ->
    &[ConfirmedCapture]` `:689`; `CaptureRoots::resolve(configured)` `:303` — interns the configured list
    against `CARGO_TILE_ROOT` and the default once per scanner start, yielding a
    `CaptureRootIndex` `:255` per root; **`fn root()` no longer exists**, phase 5
    deleted it;
    `registered_runs(scan, observe)` `:756` — reads `<root>/state/pids` through
    the access layer, verifies each record against fresh kernel evidence for that
    pid, and returns `RegisteredRuns`; `parse_state` `:903`. Inline
    `#[cfg(test)]` module `:1040`. Reading a log goes through
    `capture_root::ScanEntry::read_log` `:632`, and reading a registration
    through `read_registration` `:640`.

    One ordering is an invariant rather than implementation detail, and phase 4
    moved the sweep onto identity. `RootScan::open` (`capture_root.rs:156`)
    samples the **registration** inventory when it opens the root; the log
    inventory is a `OnceLock` sampled lazily and only for legacy annotation,
    because a versioned record names its own log. Cleanup removes a registration
    and the log it names as a pair, decided by fresh kernel evidence for that pid
    rather than by file age, so a run publishing mid-scan cannot lose its log to
    an orphan sweep — which would leave cargo reopening the log outside the
    shim's setup subshell under the caller's umask, `0066` on the runner units,
    unreadable by the operator for the life of that run. Any change here keeps
    the named-log, paired-removal, and retained-staging budget regressions
    passing unchanged.

  - `crates/cargo-tile/src/processes.rs` — the two-phase process scan and row
    construction, the largest non-render file. `cargo_split(argv) ->
    Result<CargoArguments, RowAbsence>` `:1422`, which reports an unavailable
    argv as its own outcome — the macOS empty-argv gate. `row(...) ->
    Result<CargoProcess, RowAbsence>` `:1239`: `path` degrades a missing cwd to
    `UNRESOLVED_PATH` via `map_or_else` (`:1248-1249`) while `command:
    command_text(process.cmd(), home)?` (`:1265`) fails the whole row — the
    asymmetry phase 9's per-field merge resolves. `CpuSmoothing` `:610`;
    `Census` `:682` with `take` `:698`, phase 7's collection boundary; `is_shim`
    `:844`; `drawn_parent` `:909`; `captured_run` `:1020`, returning
    `CaptureLookup`; `annotate_capture` `:1084`, which selects a confirmed
    registration by pid alone; the exclusion retain `:545`, whose `.is_ok()`
    discards the `RowAbsence` distinction; `Capture::take()` at `:559`;
    `aggregate_compilers` `:1214` and `aggregate_cpu` `:1234`, phase 7's
    aggregation boundary and in this file rather than in `render.rs`;
    `registration_directory` `:1323`, which collapses a directory to `~` only
    when the record's writer home matches the scanner's own. Inline
    `#[cfg(test)]` blocks `:175` and `:1525`.

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
    **`pub(crate) fn rows(app: &App) -> SettingsRows` `:145`** is the only entry
    point; `SettingsRows` `:105` (`rows`, `widest_row`, `ids`). **It does no
    filesystem access today** — every row is read out of `app` (`loaded_config`,
    `startup_note`, `capture_note`, `sccache`, `root_status`) and pushed through `push_stepper`.
    Phase 6 kept that true and added to it: `push_capture_roots` `:200` renders
    every configured root out of `app.root_status` through `capture_root_status`
    `:437`, observations the scan already retained rather than anything read at
    render time. Later phases keep it true.

  - `crates/cargo-tile/src/terminal.rs` — the event loop and the scan worker, 34 KB.
    `drain_scans(app, &scans) -> bool` **`:401`**, called from the loop at `:311`;
    its `bool` is "redraw needed". `spawn_input_thread()` `:434`. The scan worker
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
    reads: `AppPaneId` `:47`, `AppOverlay` `:60`, `Updates` `:100` with `toggled()`
    `:109`. Holds `loaded_config`, `startup_note`, `capture_note`, `sccache`, and — from
    phase 6 — `root_status: Vec<RootStatus>` `:171`, the per-root read outcome the
    settings overlay renders and `drain_scans` compares for redraw.

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

  **`crates/cargo-tile` has two lanes, and which one a phase gets depends on
  what its gate can observe.** It is a binary-only crate (`src/main.rs`, no
  `lib.rs`), so an integration test in `tests/` has no library target to link
  against and can reach no crate item. Every test of a crate item is therefore
  an inline `#[cfg(test)]` module inside the file it tests: `processes.rs:1399`
  (58), `progress.rs:844` (61), `render.rs:2030` (48), `hook.rs:348` (33),
  `terminal.rs:670` (6), `cli.rs:163` (6), `config.rs:283` (5), plus modules in
  `roster.rs`, `capture.rs`, `globals.rs`, `sccache.rs` and the `attract`,
  `favorites` and `theme` trees.

  **`crates/cargo-tile/tests/` exists and is a real lane.** `shim_modes.rs`
  (19 tests) and `shim_registration.rs` (24 tests) drive the **built binary**
  as a process under a PTY harness — `reader_regression` at
  `shim_registration.rs:652` — so they need no crate item and stay disjoint
  from every source file a writer holds.

  The consequence for **Seats**: a phase whose gate is observable from outside
  the binary — an install/status/uninstall lifecycle, a shim publication, a
  rendered heading — gives its `test` or `review` seat that real tester lane,
  as phases 6, 8 and 9 each do. A phase whose gate reaches only crate items has
  no tester lane at all: its `test` seat **opens as a writer** and each writer
  writes the inline tests for the file it owns.

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

### Phase 2 — The capture shim: setup boundary, explicit modes, versioned registration  · status: done

#### As-built

`cargo-capture-shim.sh` runs every capture-setup step inside a `setup_capture` subshell that installs its own `HUP`/`INT`/`TERM` traps; the parent installs its `cleanup` and signal traps before any artifact exists. Any setup failure cleans up and `exec`s the real cargo with the original arguments under the caller's original umask, so no capture failure can alter or fail the build. The subshell also contains a fatal expansion error, which is why the parent's umask is never modified; `true > "$log"` replaces `: > "$log"`, since a redirection failure on the special built-in `:` exits a POSIX shell outright.

A scoped `umask 0027` covers only the shim's own writes — directories `0750`, files `0640` — and the shim corrects the modes of the directories it owns beneath the root, one level at a time and never recursively, because `mkdir -p` applies a umask only to a directory it actually creates. The built-in `/tmp/cargo-tile` root is created and corrected to `0700`; a root named through `CARGO_TILE_ROOT` keeps the mode its deployment gave it. The staging file gets an explicit `chmod 0640` because `mktemp` creates `0600` whatever the umask.

The registration is a NUL-framed record written by `printf` straight to a `.tmp` staging file — never through a shell variable, which cannot hold a NUL: `cargo-tile-v2`, generation, boot identity, birth stamp, log basename, `$HOME`-collapsed working directory, argument count, then each argument verbatim. The birth stamp is field 22 of `/proc/$$/stat` (parsed past the last `)`) on Linux and `ps -o lstart= -p $$` on macOS; identity fields are empty when unobtainable. Publication is `ln` from the exclusively created staging file onto `<pid>.<generation>`, so a name already held is a setup failure like any other, and it happens strictly before the log is created. The shim removes an existing FIFO at its own pid's name before `mkfifo`, since owning the pid means the name is stale; a removal failure preserves cargo's original invocation and leaves an unowned directory untouched.

On the reader side, `RegistrationGeneration` classifies a registration filename in both the bare all-digits and `<pid>.<generation>` forms, and `live_runs` returns `HashMap<u32, HashSet<RegistrationGeneration>>`, so one pid carries every generation registered under it and each generation's log is protected separately. `Capture::take_from` collects the capture directory into a `Vec<fs::DirEntry>` strictly before it samples liveness. Nothing reads a record body yet — association is by filename only.

**Files:**
- `crates/cargo-tile/src/cargo-capture-shim.sh` — the writer: setup boundary, capture-path selection, publication, cleanup.
- `crates/cargo-tile/src/progress.rs` — the reader: `RegistrationGeneration`, `Capture`, the sampling order, the sweep, log association by filename.
- `crates/cargo-tile/src/constants.rs` — the record's literals and limits, including the registration-filename separator.
- `crates/cargo-tile/src/hook.rs` — shim installation and toolchain handling; its test asserts the durable fact that the shim removes its own log on exit.
- `crates/cargo-tile/tests/shim_modes.rs` — capture paths and permission boundaries, against the built shim.
- `crates/cargo-tile/tests/shim_registration.rs` — publication, stale-pid recovery, record framing.

**Binds later work:** A sweep removes `<pid>.<generation>` by the full name it read and proved ended, never by pid alone. Exclusive creation bounds what that name can mean only so far: it prevents a name being taken while a record still holds it, and does **not** prevent the same name being published again once something has removed it. Deleting by an exact name is therefore not by itself safe, and what makes it safe is still open. The record body is still unparsed, so verifying that a registration belongs to the process now holding its pid remains open. The two files under `tests/` are the shim's regression surface — anything touching the shim runs `verify.sh test cargo-tile` against them. "A hardened root access layer, and `progress.rs` onto it" must carry `RegistrationGeneration` across the migration.

**Gotchas:**
- The shim's publication order and the reader's sampling order are one invariant read from two ends, and neither may be swapped. When a live run's log is swept, cargo reopens it through `script`/`tee -a` outside the setup subshell under the caller's umask — `0066` on the runner units — so the log lands `0600` and the operator cannot read it for the life of that run, with no message saying why.
- `read_dir` returns a lazy iterator. Collecting it is the sample; opening it is not. Code that keeps the calls in order but drops the collection reintroduces the defect while looking correct.
- `RegistrationGeneration::Calendar(String)` accepts any nonempty filename suffix. It is a candidate, never proof of process identity.
- The record's directory field is `$HOME`-collapsed at write time and a test asserts that form, so the raw absolute path is not recoverable from a record.
- The macOS birth stamp is unnormalized `ps -o lstart=` output, which Apple formats through `localtime`; it is not comparable across locales or timezones.
- `cargo-tile` has no library target, so the files under `tests/` reach the shim as a subprocess and cannot touch crate items.
- A test must never run the repository copy of the shim in place: it resolves its own directory and expects `cargo-tile-real` as a sibling, so doing so exercises only the bail-out path.

**Ruled out:**
- Overriding the runner unit's `UMask=0066` — it reaches every file the runner writes, `.credentials` included.
- `mv -f` publication — it silently replaces a name a live run still owns.
- Recursive mode correction beneath the root — one level, only directories the shim owns.
- `/proc/self/stat` for the birth stamp — inside the setup subshell `self` is the short-lived child, not the pid the registration is filed under.
- The `<cwd>\tcargo <args>` line format — space-joined argv loses argument boundaries and a tab or newline inside an argument breaks the framing.
- Running the application smoke against real toolchains — `install` moves each toolchain's real cargo aside machine-wide.
- A stylistic-only change inside the shim with no behavioral consequence.

### Phase 3 — A hardened root access layer, and `progress.rs` onto it  · status: done

#### As-built

- `RootScan::open` opens a capture root through directory handles — root, then `state`, then `pids`, each relative to the previous handle with `RDONLY | DIRECTORY | NOFOLLOW | CLOEXEC` — and samples the log inventory before it opens or samples registrations.
- `RootScan::access` returns `RootAccess::Owned | Foreign`, granting the cleanup capability only when root continuity holds across scans, both enumerations (`Enumeration::{Complete, Incomplete, Failed}`) are complete, the root and its `state`/`pids` directories are owned by the effective user with no group or other write bit, and revalidation succeeds.
- `DirectoryIdentity::exclusive_owner` applies one ownership rule on every platform — owner is the effective user and neither group nor other may write — and documents in a comment that a macOS ACL can grant write access without changing those bits.
- `effective_user()` caches its first result in a `OnceLock`, retaining a failed read as `EffectiveUser::Unavailable` so a transient failure is never retried into a sweep.
- `ScanEntry::read_log` and `read_registration` replace the former `tail`, reading through the directory handle under `Read::take` byte caps.
- `live_runs` returns `LiveRegistrations { generations: HashMap<u32, HashSet<RegistrationGeneration>>, evidence: RegistrationEvidence }`, separating the live set from whether the evidence authorizes deletion; `RegistrationEvidence::Incomplete` withholds the sweep while every successfully sampled reading is still retained and displayed.
- `Capture` stores per-scan `readings`; `Capture::read` is a map lookup that performs no filesystem access, so a capture referenced by many rows is read once per scan.
- `Capture::take_roots` establishes one removal allowance shared across every owned root for a scan, counting registrations, logs, and recognized staging files together, replacing the earlier per-root budget reset.
- `.tmp` staging files are recognized during enumeration and counted against the sweep budget; nothing in this phase deletes them.

**Files:**
- `crates/cargo-tile/src/capture_root.rs` — new: the root access layer — directory-handle traversal, the ownership rule, enumeration outcomes, bounded reads, and the sweep
- `crates/cargo-tile/src/progress.rs` — the reader rewired onto that layer: `root()`, `live_runs`, `take_roots`, per-scan `readings`
- `crates/cargo-tile/src/constants.rs` — the access layer's own limits and the corrected `CAPTURE_SWEEP_LIMIT` sizing comment
- `crates/cargo-tile/src/main.rs`, `Cargo.toml` — the `mod capture_root` declaration and `rustix = { workspace = true, features = ["fs"] }`

**Binds later work:** `RootScan::open` samples the log inventory before registrations, and the reader collects that sample into a `Vec` before consulting liveness — both orderings must survive any future scanner entry point. `live_runs`'s map carries `HashSet<RegistrationGeneration>` per pid: one pid can hold several live generations, each protecting its own log. `RootHistory`/`RootContinuity` lives in thread-local state and must stay reachable through any new scanner entry point. Staging files are recognized and budget-counted but not deletable by anything here — age cannot establish that a paused writer ended, so eligibility needs registration identity, which this layer does not evaluate. `Capture::read` still returns `Option<RunState>`, collapsing unreadable, read-with-nothing-to-report, and never-sampled into one value. Four conditions currently render identically to a healthy or empty root, with no distinct operator-facing surface yet: incomplete or failed enumeration, incomplete registration evidence, a foreign root (itself collapsing five distinct causes), and an unavailable effective-user read. The in-use `rustix` has no `sysctl` binding.

**Gotchas:** Reversing either sampling order lets a run publishing mid-scan lose its log; cargo then reopens the log outside the shim's setup subshell, under the caller's umask (`0066` on the runner units), leaving it unreadable by the operator for the rest of that run. A single failed registration read must not abandon the whole root: a cargo run exiting normally unlinks its own registration between the sample and the read, so this is the ordinary case, not an edge case — treating it as fatal would blank every neighboring run's progress whenever any build finished. `cargo-tile` has no library target, so every test for this layer is an inline `#[cfg(test)]` module inside the production file — no test can be added without holding that file. Caching the effective uid for the process lifetime is permanent: if the very first read fails, cleanup is disabled for the whole session with nothing on screen to explain why. The runner cache directories are `drwxr-x---` (group read+execute, no write), so every removal a non-owning reader attempts fails on every entry, permanently — this is the ordinary case on the runner machine, not a hypothetical.

**Ruled out:** a platform gate returning `Foreign` for every non-Linux root, since no Mac would ever sweep — one ownership rule applied uniformly replaces it. A trusted-prefix list preserving macOS `/tmp` — unnecessary once the ownership rule is uniform. Adding `rustix`'s `process` feature for `geteuid` — the effective uid comes from the `sysinfo` dependency already in use. Abandoning a capture root entirely when a single registration read fails. A `bool` marking whether the live set was complete — completeness is carried by the `RegistrationEvidence` type instead.

### Phase 4 — Registration identity: verify before it counts, and before it is deleted  · status: done

#### As-built

`registration::Registration::parse` yields `VersionedRegistration` (private fields, `:85`) or `LegacyRegistration` (`:173`); `verify_observation` (`:127`) is the only route to `VerifiedRegistration` (`:203`) and takes a `birth_stamp::KernelObservation` whose pid is checked before its identity, so a live process cannot adopt another's record. The outcome is `Confirmed | Ended | Unknown`: `Unknown` never renders as running and authorizes no deletion, and a legacy record annotates a row the process table already built but never sources one. `birth_stamp::observe(pid)` returns a `KernelObservation` carrying the pid it was taken for — Linux from `/proc/<pid>/stat` start ticks qualified by the boot id, macOS from `KERN_PROC_PID` and `kern.boottime` through `libc::sysctl` (target-gated dependency, the crate's only `unsafe`, per-item `allow` with a `// SAFETY:` comment). `Capture` (`progress.rs:295`) holds two stores, `readings` and `confirmed`; `confirmed()` (`:417`) hands out `ConfirmedCapture` (`:264`) whose modification time is descriptor-bound and typed `Result<SystemTime, CaptureFailure>`. Read failures survive as values: `CaptureLookup` (`:171`) is `Unregistered | Registered(CaptureRead)` and `CaptureRead` (`:191`) is `Progress | NoCurrentProgress | Unreadable(CaptureFailure)`. Cleanup removes a registration and the log its field 5 names as a pair, decided by identity evidence re-confirmed immediately before the unlink rather than by file age or by name; a `.tmp` staging file whose writer the kernel confirms ended is eligible, and inspecting a retained file no longer spends the per-scan removal allowance. The shim publishes a non-repeating generation and writes field 4 as a decimal integer on both platforms, reading its own start time under `LC_ALL=C TZ=UTC0`. Rows group on `CargoProcess.directory_identity`, the raw absolute directory; `registration_directory` (`processes.rs:1179`) collapses it to `~` only where the record's writer-home field matches the scanner's own.

**Files:**
- `crates/cargo-tile/src/registration.rs` — record parsing and the verified proof type
- `crates/cargo-tile/src/birth_stamp/{mod,linux,macos}.rs` — kernel birth evidence and its two platform readers
- `crates/cargo-tile/src/progress.rs` — the two capture stores, the read outcomes, the paired sweep
- `crates/cargo-tile/src/capture_root.rs` — validated-basename log read, registration observation carrying bytes and metadata from one descriptor, removal allowance
- `crates/cargo-tile/src/processes.rs` — verified annotation, `cargo_split`'s argument-layout type, directory display
- `crates/cargo-tile/src/render.rs` — grouping on raw directory identity
- `crates/cargo-tile/src/roster.rs` — row fixtures carrying the identity field
- `crates/cargo-tile/src/cargo-capture-shim.sh` — non-repeating generation, fixed locale and timezone
- `crates/cargo-tile/tests/shim_registration.rs` — production-binary regressions driven through a pty against the shim's real publications

**Binds later work:** `VerifiedRegistration` is the only proof of identity — capture membership and row construction consume it, never the candidate; `KernelObservation`'s private fields come from `observe(pid)` alone. `CaptureLookup` and `CaptureRead` are consumed rather than re-derived: the unreadable case belongs to the work rendering configured-root status, membership to the work building invocation identity on the shim's non-repeating generation. `CargoProcess.directory_identity` and `render::group_by_path` already exist, so the row-grouping and headings work adds account and root qualification instead of introducing the key. Both capture stores carry the root; annotation cannot cross roots.

**Gotchas:** The state column ignores a package-cache lock wait by design — only `waiting for file lock on build directory` draws a blocked row (`constants.rs:895-901`). The summary pane carries the outermost invocation per directory, so anything spawned under `cargo nextest` appears only in the command pane. The macOS birth truncates to whole seconds deliberately: it confirms a claimed identity still holds, and does not separate two processes born in the same second. `effective_user()` and each platform's boot read cache their failure in a `OnceLock` for the life of the process; nothing retries without a restart. `birth_stamp/macos.rs` is `cfg`-gated off on Linux and CI has no macOS runner, so it compiles nowhere in this repository's automation.

**Ruled out:** sysinfo for start times — rounded seconds on Linux, zero on the macOS fallback. `rustix` for the macOS reads — `sysctl` is unimplemented there. Shelling out to `sysctl(8)` or `ps(1)` — a subprocess per registration per scan at four scans a second. Serializing raw `ps -o lstart=` text — rendered through `localtime` under the writer's locale, so no reader can recover the instant. Deleting a staging file on age — age cannot establish that a paused writer ended. Treating a filename suffix as identity — `RegistrationName` classifies a name and proves nothing about the process holding the pid.

### Phase 5 — One root becomes a list  · status: done

#### As-built

- `capture.roots` is a list. `CaptureRoots::resolve` (`progress.rs:288`) resolves, validates, deduplicates and interns the configured entries plus the `CARGO_TILE_ROOT` override and the built-in default, once. `fn root()` no longer exists.
- `CaptureRoot` (`:253`) carries `path: Result<PathBuf, CaptureFailure>` beside `sources: Vec<CaptureRootSource>` — `Default`, `Environment { path }`, `Configuration { entry, path }` — so a directory named twice keeps both spellings on one deduplicated entry. Unset and empty `CARGO_TILE_ROOT` both resolve to the default with source `Default`.
- Capture identity is root-qualified. `CaptureKey { root, pid }` (`:244`) keys both pid stores; `Capture::read` (`:577`) takes that key rather than a pid, `confirmed` (`:593`) returns a `ConfirmedCapture` carrying it, and `take_roots` (`:467`) walks every root through `scan_root` (`:488`) under one per-scan removal allowance.
- `spawn(config: &Config)` (`processes.rs:375`) snapshots configuration on the caller and returns; the worker resolves the root list once before its scan loop, so a slow filesystem cannot hold up terminal startup.

**Files:**
- `crates/cargo-tile/src/progress.rs` — root list, resolution, interning, `CaptureKey`, both keyed stores, and their inline tests
- `crates/cargo-tile/src/processes.rs` — worker-side resolution, root-qualified association and annotation
- `crates/cargo-tile/src/config.rs` — `capture.roots` as a list on `CaptureConfig`
- `crates/cargo-tile/src/terminal.rs` — the `CargoProcess` row literal at `:758` builds a `CaptureKey`, calls `Capture::read`, imports `DirectoryIdentity`
- `crates/cargo-tile/src/constants.rs` — the `capture.roots` key and the root literals

**Binds later work:** `CaptureKey { root, pid }` keys both `readings` and the `ConfirmedCapture` store, so consumers key on it rather than a bare pid. `CaptureRoots::resolve` (`progress.rs:288`) is the only resolution entry point and runs inside the scanner worker, not on its caller. A deduplicated root retains every `CaptureRootSource` that named it, so one row can report several spellings of the same directory. `captured_run` (`processes.rs:940`) holds two precedence rules — nearest registered ancestor first, then the lowest root index for that pid — and an unconfirmed reading in the preferred root suppresses another root's confirmed proof rather than borrowing it; the regression at `processes.rs:1687` pins that. A missing configured root is reopened every scan and recovers on its own; a root retained as `Err(CaptureFailure)` (a relative `capture.roots` entry) cannot, because resolution runs once — whereas a relative `CARGO_TILE_ROOT` is made absolute by `resolve_environment` (`:293`).

**Gotchas:**
- `Components` drops a `.` but keeps `ParentDir`, so a root spelled `<root>/state/..` has no `file_name()`. `intern` matches on `components().next_back()` and pops the canonicalized parent for a trailing `..`, which resolves an ancestor symlink before the `..` applies. The final component is never canonicalized: each root is opened there with `O_NOFOLLOW` and its ownership rechecked every scan.
- `take_roots` skips a retained validation failure silently, alongside roots it cannot open.
- The crate is binary-only, so every test of a crate item is an inline `#[cfg(test)]` module inside the file under test and there is no separate test target.

**Ruled out:** canonicalizing the whole configured path, which would resolve the final component and defeat the `O_NOFOLLOW` check; dropping a configured root that fails to resolve, since retaining it is what allows reporting it; a public resolver API or a status row for worker-side resolution, the helper being private with no observable effect beyond startup not waiting; a second row for a pid confirmed under a non-selected root, one live invocation being one tile.

### Phase 6 — Configured roots get a visible status  · status: done

#### As-built

Every effective capture root carries a read outcome that reaches the settings popup with no filesystem access at render time. `RootStatus` (`processes.rs:368`) holds the root, its owner, its cleanup refusals, its read state, a count of confirmed published registrations, its retained diagnostics and its associations; it travels on `Scan` into `App.root_status` (`app.rs:171`), where `push_capture_roots` (`settings.rs:200`) renders one inert read-only row per root through `capture_root_status` (`:437`). `drain_scans` (`terminal.rs:401`) requests a redraw when a root's status changes even though the command groups do not.

`RootReadStatus` (`processes.rs:387`) is `Readable | DefaultNotCreated | Unavailable(PathFailure) | Invalid(CaptureFailure)`: `DefaultNotCreated` is an implicit default root that does not exist yet and renders with no cleanup line, `Unavailable` a retryable access failure, `Invalid` a configured entry that failed startup validation whose row says a corrected entry plus a restart is the retry, because resolution runs once. `CleanupRefusal` (`capture_root.rs:76`) replaces one private `Foreign` classification with seven named conditions, each carrying the directory it refused, and `RootOwner` (`:67`) reports ownership as `Uid(u32) | Unavailable` independently of whether cleanup may proceed. `CaptureDiagnostic` (`processes.rs:400`) has eleven variants over enumeration, registration, annotation, identity, staging, log and boot observations; `CaptureAssociation` names which root supplied a run's association and whether another root's confirmed proof went unused. `birth_stamp::boot()` is a shared accessor on both `linux.rs` and `macos.rs` over one process-lifetime cache, so the verifier and its diagnostic never disagree about the boot read.

**Files:**
- `crates/cargo-tile/src/processes.rs` — `RootStatus`, `RootReadStatus`, `CaptureDiagnostic`, `CaptureAssociation`; the hub the renderer matches on
- `crates/cargo-tile/src/capture_root.rs` — `RootOwner`, `CleanupRefusal`, `DirectoryIdentity::refusals` (`:458`), `RootScan::cleanup_refusals` (`:279`) gating a private `access` (`:328`)
- `crates/cargo-tile/src/progress.rs` — the per-root scan producing the diagnostics and the confirmed count
- `crates/cargo-tile/src/settings.rs` — the read-only rows, one rendering test per condition
- `crates/cargo-tile/src/terminal.rs` — the status-change redraw with unchanged command groups
- `crates/cargo-tile/src/app.rs` — `root_status: Vec<RootStatus>`, retained between scans
- `crates/cargo-tile/src/birth_stamp/` — the shared cached boot accessor on both platforms
- `crates/cargo-tile/tests/cli_lifecycle.rs` — the built binary through `install`, `status`, a shim execution and `uninstall` against fixture-only `RUSTUP_HOME`, `HOME` and `CARGO_TILE_ROOT`; never touches a real toolchain
- `.github/workflows/ci.yml` — the `macos-latest` build, test and lint job at `:348`, gated on `needs: changes`

**Binds later work:** `Scan` is the transport and `App.root_status` the retained store, so a renderer added to `settings::rows(app)` inherits a state distinguishable from a measured value and performs no filesystem access; `drain_scans` (`terminal.rs:401`) already redraws on a root-status change with unchanged command groups. `RootStatus.confirmed` counts confirmed published registrations only — every other artifact class earns its own `CaptureDiagnostic` variant instead of folding into that number. `RootOwner` is a uid, not an account name: whatever needs a display name resolves it itself, scanner-side, keeping the number separate from the name. `CaptureAssociation` reports the final selection, and `Census::associate_status` collects suppressed proofs only when the selected registration is unconfirmed. Anything keying on process identity depends on `birth_stamp::observe`, whose macOS half is still unverified.

**Gotchas:**
- The `macos-latest` job is committed but has never run — the branch is unpushed — so `birth_stamp/macos.rs`, the crate's only `unsafe`, has never been compiled, linted or run by any gate.
- The boot read is a process-lifetime `OnceLock` shared by the verifier and its diagnostic, so a failed boot read is permanent for the session on both platforms. `IdentityUnknown` (retried next scan) and `IdentityBlockedByBoot` (blocked until restart) stay separate states for that reason: folding them together promises a retry that cannot happen. `effective_user()` (`capture_root.rs:712`) caches its failure the same way, which is why `EffectiveUserUnavailable` says a restart is the retry.
- `registered_runs` keys generations by pid alone, and the legacy and versioned spellings for one pid resolve to the same file, so both arms record a diagnostic for one path; the reading beside it was already deduplicated, the diagnostic now deduplicates by path.
- `settings.rs` performs no whitespace splitting anywhere — `capture_sources` (`:416`) joins with an explicit separator and renders paths through `Display`. The opposite claim looks plausible to a reader of that function and is wrong.
- The settings popup cannot scroll: `draw_settings` (`render.rs:1984`) clamps the popup height and configures a viewport, but `:2027` renders the whole paragraph without applying it, so rows past the bottom edge are unreachable.

**Ruled out:**
- Routing a missing default root through the general access-failure arm — it emits a missing-directory line plus a cleanup refusal, the opposite of an unused default root staying quiet.
- Resolving the owner uid to an account name here; `RootOwner` stays numeric and `settings.rs:589` renders the number.
- Folding a retained validation failure into the missing-root wording; it cannot recover, and its row shows the original `capture.roots` spelling rather than an absolute path.

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

*Availability has to come from somewhere, and today it is destroyed before the types could carry it.* `Census::take` (`processes.rs:698`) receives a plain `f32`. On macOS sysinfo initialises a process's CPU usage to zero (`sysinfo-0.39.6/src/unix/apple/macos/process.rs:57`) and its task-info reader discards the syscall result (`:336`), so a failed measurement and a genuinely idle process both arrive as `0.0` and no type introduced downstream can separate them. Establish the distinction where the sample is collected, without reaching for a platform syscall. **The obvious candidate does not work and must not be used:** `Process::accumulated_cpu_time()` is quantized to integer milliseconds on macOS (`sysinfo-0.39.6/src/unix/apple/macos/process.rs:440`) and derived from whole clock ticks on Linux (`src/unix/linux/process.rs:547`), so a process that was measured perfectly well can report zero accumulated time against a nonzero `run_time()` — the identical pairing a failed measurement produces. Find evidence that actually separates the two. Where none exists, the outcome is a third state that says the measurement is unproven, worded so it never asserts a failure it cannot demonstrate; the Goal is that an unknown never renders as a measured zero, not that every unknown be classified. A zero rate against nonzero accumulated time remains a measured zero and renders as one. A process observed for the first time is unavailable too — a rate needs two samples — and that is a separate reason from a failed read, so name both. This keeps the plan's rule that `birth_stamp/macos.rs` is the only file writing `unsafe`; no additional probing is introduced.

An `Option` at the row is not sufficient on its own. The group total sums the members it can read and silently skips the ones it cannot, so a group holding an unknown contributor reports a figure lower than reality and indistinguishable from a measured one. **An unknown contributor makes the group total unknown.** Smoothing must stop publishing its previous value once a sample goes unavailable, rather than holding the last reading indefinitely.

`CpuSmoothing`'s `taken` field (`processes.rs:620`) is itself one of the optionals this phase exists to remove: it distinguishes a smoother that has never published from one holding a previous reading, and those are different states with different consequences once a sample can go unavailable. Give it a name rather than an `Option`.

Reach: `Census::take` (`:698`), `CpuSmoothing` (`:610`), the group totals `aggregate_compilers` (`:1214`) and `aggregate_cpu` (`:1234`), `Roster::assign_families` (`roster.rs:382`, the `managed > 0` test), and the call sites in `render.rs`.

An active runner must never read `0%` because the value could not be read. This phase lands before the registration-sourced row so that row can be built on the optional types from the start rather than converted afterwards.

**Files:**
- `crates/cargo-tile/src/processes.rs` — `Census::take` (`:698`), `CpuSmoothing` (`:610`), the row fields, and the group totals `aggregate_compilers` (`:1214`) / `aggregate_cpu` (`:1234`)
- `crates/cargo-tile/src/render.rs` — how an unavailable value renders
- `crates/cargo-tile/src/roster.rs` — `assign_families` (`:382`), whose `managed > 0` comparison and inline fixtures both change with the measurement type
- `crates/cargo-tile/src/constants.rs` — the unavailable-value presentation literals
- `crates/cargo-tile/src/terminal.rs` — the `CargoProcess` row literal (`:869`), which constructs the measurement fields by hand

**Seats:** 3 writers + 0 testers — the measurement type reaches three files, not two: collection and smoothing, rendering, and family assignment. `cargo-tile` is a binary-only crate, so it has no linkable integration-test target and its tests are inline `#[cfg(test)]` modules inside the files being edited (Delegation Context → **Test lanes**). Each writer writes the inline tests for the file it owns; no seat here can open as `test`.
- `impl` — `crates/cargo-tile/src/processes.rs` — `Census::take`, `CpuSmoothing`, the row fields, **and the group totals: `aggregate_compilers` (`:1214`) and `aggregate_cpu` (`:1234`) live here, not in `render.rs`**; plus `crates/cargo-tile/src/constants.rs`; hub: `crates/cargo-tile/src/processes.rs` (the measurement type all three files read)
- `test` — opens as impl: `crates/cargo-tile/src/render.rs` — how an unavailable value renders wherever a total or a per-row figure is drawn; writes into the existing `#[cfg(test)]` module (`:2030`). **The Work Order previously named three CPU/managed summations in this file; they are not here** — the arithmetic is in `processes.rs` and this seat renders its result
- `review` — opens as impl: `crates/cargo-tile/src/roster.rs` — `assign_families` (`:382`) compares `managed > 0` directly and its inline fixtures construct measurements by hand, so both change with the type; writes into the existing `#[cfg(test)]` module (`:439`), plus `crates/cargo-tile/src/terminal.rs` — phase 5 added a `CargoProcess` row literal there (`:869`) carrying `cpu: String`, `compiler: Option` and `managed: usize`, which every measurement-type change in this phase reaches

**Constraints from prior phases:**
- None binding from the capture work — this phase touches no root, registration, or shim code, and can run independently of phases 4 through 6.
- The plan permits `unsafe` in `birth_stamp/macos.rs` alone, under a per-item reasoned `allow`. This phase adds none: the availability distinction above is drawn from sysinfo values the crate already collects.

**Acceptance gate:** Build, Test and Lint green. A test asserting an unavailable CPU sample renders as unavailable rather than `0%`. A test asserting a group total containing an unknown contributor is unknown. A test asserting `CpuSmoothing` stops publishing a stale value. Three tests at the collection boundary, distinguishing a first observation, a failed measurement, and a measured zero from one another. A test asserting a readable process whose quantized accumulated CPU time is zero is not reported as a failed measurement. A test asserting a reading that becomes unavailable before the next smoothing publication is published as unavailable rather than as the retained previous value. A test asserting a never-published smoother is distinguishable from one holding a previous reading.

---

### Phase 8 — Invocation identity and capture membership  · status: todo

#### Work Order

**Goal:** every invocation carries a stable identity that survives a change of source between scans, and each capture is read once per scan rather than once per row.

**Spec:**

*1. Identity must reach past `Capture.logs`.* Qualifying only the log map leaves `read(pid)` ambiguous, and group identity, roster matching, family colours and tile identity all still key on a bare pid. There is a second problem underneath: a registration row uses the **shim** pid while its preferred process row uses the **cargo** pid, so a run that switches source between scans makes the roster retire one tile and open another.

Carry a stable captured-invocation identity of root, shim pid and run generation through capture lookup, deduplication, roster and tiles, with the **displayed** pid kept separate from the identity.

*A generation is a filename, not a lifetime.* `RegistrationName::Generated { generation, .. }` (`progress.rs:437`) classifies a filename suffix and proves nothing on its own. Phase 4 made that suffix non-repeating — `same_pid_and_calendar_second_receive_different_generations` (`tests/shim_registration.rs:1019`) holds the publisher to it — and supplied the boot-qualified birth stamp, so `(root, shim pid, generation)` no longer repeats at publication. What this phase owes is the consumer side, and it needs two values kept apart:

- **Comparison precision.** `BirthStamp::macos` (`birth_stamp/mod.rs:78`) truncates the birth to whole seconds, deliberately: the shim can read only `ps -o lstart=` at second resolution and the two have to compare equal, which `macos_discards_fractional_seconds_but_linux_keeps_individual_ticks` (`:361`) fixes in place. On macOS the birth stamp is therefore evidence that a claimed identity still holds — not a value that separates two processes born inside one second.
- **Process-lifetime identity.** `RunId` is the captured invocation's non-repeating value, and the non-repeating generation is what makes it so, with the birth stamp qualifying rather than replacing it. **`ProcessIdentity` cannot be built the same way, and this Work Order must stop saying it can.** An uncaptured process has no registration at all, and a nested invocation is deliberately given none — the shim skips publication once it detects an enclosing capture (`cargo-capture-shim.sh:133`) — so for those rows there is no generation to distinguish anything. The only other evidence available is the birth stamp, and on macOS `birth_stamp/macos.rs:45` truncates it to whole seconds, which is the precision the captured case can afford to lose and this one cannot. Define `ProcessIdentity`'s evidence separately from the registration generation: name what actually separates two processes where no generation exists, and where nothing does, name an unavailable-lifetime state rather than a value that might collide. Never merge two rows on unavailable evidence, and never let an unavailable lifetime read as a distinct one.

Root replacement invalidates every retained association keyed on a root, and this phase needs something it can observe that through. `RootScan::open` (`capture_root.rs:156`) already computes replacement once per scan, but the continuity it derives stays private to that module. Expose a **root incarnation** — an identity value that changes when the root at a pathname is replaced — without exporting any cleanup capability alongside it. **The continuity it already computes is not that value.** `RootScan::open` compares a `TreeIdentity` (`capture_root.rs:178`) carrying ownership and mode metadata alongside the directory identity, and it reports `RootContinuity::Changed` when access is merely recovered or a mode bit moves. Those signals exist to decide **cleanup eligibility**, where any change at all is a reason to refuse, and reusing that classification as the incarnation would give a permission change the same consequences as a replaced directory: every retained association discarded because a mode bit moved. Derive incarnation from evidence that the directory itself was replaced, keep it distinct from the cleanup-continuity signal, and let each rest on its own evidence. `CpuSmoothing`'s history (`processes.rs:610`) is keyed on the bare pid and belongs in the same migration: otherwise a reused pid inherits the previous invocation's smoothed CPU.

*The proof type already exists; consume it.* `VersionedRegistration` (`registration.rs:85`) is the **candidate** — what parsing produces, with private fields, proving nothing about the process. `VerifiedRegistration` (`:203`) is the proof, and `verify_observation` (`:127`) is the only way to obtain one, from kernel evidence taken for that pid. The direct-association type this phase defines wraps `VerifiedRegistration` and adds its own guarantee; it introduces neither a second verification type nor a second parser. Rename the candidate for the state it is in — `RegistrationCandidate` — so the two read apart at every call site. Production access to the record's arguments belongs here too: `arguments()` (`registration.rs:119-120`) is `#[cfg(test)]` today and phase 9's row builder needs it, so this phase's `registration.rs` owner makes it a production accessor.

*2. Identity is not membership.* One capture legitimately supplies progress to several invocations — the shim skips nested captures on purpose — so giving every associated row the same run identity merges distinct commands into one. Model it as `InvocationId::Captured(RunId) | Process(ProcessIdentity)`. Only the invocation a registration **directly** represents takes the working directory and arguments from it; a nested invocation keeps its own identity and merely references the enclosing `RunId`. That reference is itself a membership question with two named answers, not an `Option<RunId>`: a row either sits inside an enclosing capture or does not, and the second is not a missing value.

`drawn_parent` (`processes.rs:909`) conflates the same two meanings today — an optional pid standing for both "no visible parent" and "the parent is not drawn" — and gains a named visible-parent type in this phase alongside the identity it keys on.

One `RunId` inside `Captured` cannot carry both of those relationships: the same value would mean "this registration describes me" for one row and "I ran underneath that registration" for another, and every consumer would then re-derive which it is holding. Give the direct case its own type — a registration that Phase 4 verified, holding the working directory and arguments it is allowed to supply — and let membership in an enclosing capture be a separate field that carries only the `RunId` it is nested under. Row creation in Phase 9 takes the verified-registration type and nothing else, so a nested invocation cannot reach the fields it must not inherit, and no row-building path needs a runtime check to keep the two apart.

*3. `is_shim` accepts absence as evidence.* `is_shim` (`processes.rs:844`) compares `subcommand(outer.cmd()) == subcommand(inner.cmd()) && outer.cwd() == inner.cwd()`. When both sides are `None` the cwd test passes, so two unavailable fields count as a match — and cwd is exactly the field the runner machine denies. Require observed equality, not matching absence.

*4. Membership is resolved before progress is parsed — and already is.* `captured_run` (`processes.rs:1020`) now stops at the nearest registered capture whether or not its log parses, and `nearest_empty_capture_does_not_inherit_an_enclosing_progress_reading` (`:2078`) and `nearest_unreadable_capture_does_not_inherit_an_enclosing_progress_reading` (`:2094`) hold it there. What remains is narrower: carry invocation identity through that association, and keep **direct** association distinct from **enclosing** membership, preserving the existing `CaptureLookup` / `CaptureRead` outcomes and the once-per-scan reads behind them.

*5. Consume the scan's capture results; add no per-row cache.* This item asked for a once-per-scan read because every row was believed to reopen its log. **Phase 3 already shipped that.** `Capture::scan_root` (`progress.rs:564`) reads each selected log during the scan and stores the result; `Capture::read` (`:673`), now taking a `CaptureKey`, is a map lookup documented as performing no later filesystem access, and `captured_run` (`processes.rs:1020`) walks parents over that lookup alone. `a_capture_keeps_its_reading_without_reopening_a_replaced_log` (`progress.rs:2043`) holds it in place. There is no per-row I/O left to remove, and **the constraint this phase carries forward is that no row may reopen a log** — not that reading is already deduplicated everywhere, because it is not. `scan_root` reads a selected log once per **registration**, so a legacy record and a versioned record naming one pathname each read it, and only then does `readings.entry(key).or_insert(reading)` (`progress.rs:620`) discard the second result; `record_log_failure` (`:717`) deduplicates diagnostics and not reads. The fixture `legacy_and_versioned_records_report_one_shared_unreadable_log` (`progress.rs:1303`) exercises exactly that double read today. This phase owns a **scan-local, root-qualified record of which log pathnames have already been read**, so one pathname is read once per scan however many registrations name it. Its tests have to separate the two cases that look alike: several registrations aliasing a single log, which collapse to one read, and genuinely different generation logs under one root, which do not.

What remains is consumption. Phase 4 names the lookup and read outcomes — `Unregistered | Registered(CaptureRead)` over `Progress | NoCurrentProgress | Unreadable`. Rows built here read those outcomes and must not re-derive them or collapse them back into an absence. `NoCurrentProgress` is named for what it means rather than for when it happens: `parse_state` (`progress.rs:903`) yields nothing for an empty log, for ordinary output carrying no counter, and — the one that misleads — for a run that has printed `Finished` while its `cargo run` application is still alive and still the row's subject. "Nothing printed yet" describes only the first. A capture that has no process-table row at all keeps its unreadable status visible, because phase 6 reports it before phase 9's fallback rows exist.

*6. Order within the scan.* Verify registrations against Phase 4's evidence, prune, establish direct association, resolve each invocation's command, and only then decide row eligibility.

*7. Exclusion applies to one set only.* `commands.excluded` prunes the process row at `processes.rs:545`, where `.is_ok()` throws away the `RowAbsence` that says why a row is absent — and group construction's `.ok()?` does the same, so the ordering below is what preserves the distinction — but the shim's hardcoded skip list covers only `metadata`/`tile`/`port`/`berth` and friends — a user-configured `excluded = ["clippy"]` still gets a registration. "No process-table row" therefore includes rows removed **on purpose**, and Phase 9's fallback would resurrect them, breaking an existing configuration contract for local captures as well as CI ones. Apply exclusion to both sources before fallback selection and attribution, keep deliberate exclusion distinct from unavailable metadata, and apply it **only** to the row-owning and attribution set — never to `Census.parents` and never to the capture liveness set, or an excluded live capture becomes a sweep candidate and loses artifacts it is still writing.

*8. Named states on the roster row, not bare optionals.* `TrackedRow` (`roster.rs:25`) carries `ended: Option<Instant>`, `family: Option<usize>` and `parent_family: Option<usize>`, and this phase's migration is where they change. `ended` is a lifetime — a row is live, or it finished at an instant. `family` and `parent_family` are role membership, which a row either holds or does not because it has no children to colour. Give each a named type saying that, per the Type Design Contract, rather than leaving three optionals whose meanings a reader has to recover from callers. `drawn_parent` (`processes.rs:909`) gets its named visible-parent type in the same pass, as item 2 says.

The consumer files carry four more of these, and they belong to the seat that owns them rather than to the identity writer. `tiles.rs:223` names a held cell `Held` — a name stating no role — and encodes which cell within the hold has focus as `focused: Option<usize>`; `TileGrid.transition` (`:270`) keeps motion as an optional; `focused_cell` (`:811`) uses absence for a slot whose focus is departing, which is a location, not a missing value; and `render.rs:1036` returns `Vec<Option<&Ancestor>>` where absence means an ancestry segment deliberately elided, not an ancestor nobody could find. Give each a named layout, focus-location, motion and ancestry-level state per the Type Design Contract, in the same migration that re-keys these files off the bare pid.

*9. Two `DirectoryIdentity` types, two meanings.* `registration.rs:154` names the **working directory** a run reported; `capture_root.rs:428` names **inspected filesystem metadata** for an open directory handle. Phases 6, 8 and 9 all touch both, and nothing in either name says which is meant. Rename each for its role here. Phase 6 ships the second one's diagnostics first, so this phase migrates what phase 6 already built rather than renaming ahead of it, and phase 9 keys grouping on the renamed first one.

*10.. Two capture-facing boundaries the migration must also name.* `CapturedRun` (`processes.rs:438`) is the **unverified** nearest-registration lookup — its `Unregistered` arm is a real outcome — so its name states no guarantee and must not be read as this phase's verified direct association; rename it for what it is and keep the two distinct. Separately, `RunState::working` (`progress.rs:172`) and `CaptureLookup::working` (`:191`) return a domain-owned `Option<(Phase, Progress)>` consumed at `render.rs:1766`, with no semantic replacement assigned anywhere in this plan. Make the gauge consume named counter states instead, preserving the blocked, unavailable and unregistered behavior it has today. The measurement, roster, home-boundary and pinning replacements specified elsewhere in this phase stay as written.

**Files:**
- `crates/cargo-tile/src/processes.rs` — `InvocationId`, `is_shim` (`:844`), `captured_run` (`:1020`), the exclusion retain at `:545`, `Capture::take(roots)` at `:559`, `drawn_parent` (`:909`), and `CpuSmoothing`'s pid-keyed history (`:610`)
- `crates/cargo-tile/src/progress.rs` — capture lookup keyed by the new identity (`read:673`, `confirmed:689`, `registered_runs:756`). **`root:433` is gone** — phase 5 deleted `fn root()`; resolution lives in `CaptureRoots::resolve` (`:303`)
- `crates/cargo-tile/src/registration.rs` — the candidate renamed for its state, and `arguments()` (`:119-120`) made a production accessor
- `crates/cargo-tile/src/capture_root.rs` — the observable root incarnation, exposed without cleanup capability
- `crates/cargo-tile/src/birth_stamp/` — comparison precision kept distinct from process-lifetime identity
- `crates/cargo-tile/src/roster.rs` — matching (`:194`), the family maps, and `TrackedRow`'s three optionals (`:25`)
- `crates/cargo-tile/src/tiles.rs` — the demand and content identifiers, likewise
- `crates/cargo-tile/src/render.rs` — the renderer entry point that still takes a pid
- `crates/cargo-tile/src/constants.rs` — any literal the identity needs
- `crates/cargo-tile/src/terminal.rs` — the `CargoProcess` row literal (`:869`), which constructs `CaptureKey`, calls `Capture::read` and imports `DirectoryIdentity`
- `crates/cargo-tile/tests/shim_registration.rs` — nested invocation identity and exclusion regressions against the production binary

  `roster.rs`, `tiles.rs` and `render.rs` are listed read-only in the Delegation Context and in this Work Order's earlier constraint. **That does not hold for this phase**: the identity these files key on is exactly what the phase replaces, so a change that stops at `processes.rs` and `progress.rs` leaves PID-based identity live in the consumers and the phase does not achieve its goal.

**Seats:** 2 writers + 1 tester — splits by file: the identity type travels with the lookup it keys, the consumers form the second set, and the production-binary harness at `tests/shim_registration.rs:652` gives the tester a real lane against the shim's own publications, which phase 4 proved out. Each writer writes the inline `#[cfg(test)]` tests for the files it owns. Paths below are relative to `crates/cargo-tile`.
- `impl` — opens as impl: `src/processes.rs`, `src/progress.rs`, `src/registration.rs`, `src/birth_stamp/`, `src/capture_root.rs`, `src/constants.rs`; hub: `src/processes.rs`, where `InvocationId` is defined and from which the consumer set reads it. Land the type first — the consumers cannot compile without it. The `is_shim` defect, two unavailable cwds comparing equal, is the highest-value single test in the phase and belongs here
- `test` — opens as impl: `src/roster.rs` including every row fixture, `src/tiles.rs`, and `src/render.rs` — the consumers keyed on a bare pid today, plus `TrackedRow`'s three named states, and `src/terminal.rs` — its row literal (`:869`) constructs `CaptureKey`, calls `Capture::read` and imports `DirectoryIdentity`, so it migrates with the consumers
- `review` — opens as test: `tests/shim_registration.rs` — nested invocation identity and exclusion regressions driven through the production binary; claims no writer file. **Also owns one repair:** `tests/shim_registration.rs:240` selects the first command-pane header and then requires that pane to hold the fixture writers, so an unrelated pane rendered first makes it fail and pass on retry. Locate the pane by its fixture markers before counting headings, with an unrelated earlier pane present in the regression. Phase 9 inherits the repaired harness

**Constraints from prior phases:**
- Phase 5's verified anchors, replacing the stale ones this Work Order carried: `Capture::read` (`progress.rs:673`) **takes a `CaptureKey`, not a pid**; `Capture::confirmed` (`:689`); `registered_runs` (`:756`); `take_roots` (`:507`); `scan_root` (`:564`); `ConfirmedCapture` (`:427`), which now carries `key: CaptureKey`; `CaptureRootSource` (`:268`); `CaptureRoots::resolve` (`:303`). In `processes.rs`: exclusion retain `:545`, the `Capture::take(roots)` call `:559`, `is_shim` (`:844`), `drawn_parent` (`:909`), `captured_run` (`:1020`), `annotate_capture` (`:1084`), `CapturedRun` (`:438`), and `spawn(config: &Config)` (`:452`) whose only production caller is `terminal.rs:209`. `drain_scans` remains at `terminal.rs:401`.
- **`fn root()` no longer exists.** Phase 5 deleted it; `CaptureRoots::resolve` (`progress.rs:303`) resolves the list in its place. Any lookup this phase re-keys starts from `CaptureKey { root, pid }`, never from a bare pid.
- **Two precedence rules already hold and must both survive the identity migration.** `captured_run` (`processes.rs:1020`) selects the nearest registered ancestor first, and only then the lowest root index for that pid; own-root precedence does not override a nearer ancestor. The regression at `processes.rs:1869` holds the second rule in place — an unconfirmed reading in the preferred root stops another root's verified metadata from annotating the row. Keep both, and keep that regression passing.
- Phase 5 added a `CargoProcess` row literal to `terminal.rs:869` which constructs `CaptureKey` directly, calls `Capture::read`, and imports `DirectoryIdentity`. Every key, type-name and identity change in this phase reaches it.
- **This phase's platform guarantee rests on phase 6's macOS job.** Phase 6 adds a `macos-latest` build, test and lint job to `.github/workflows/ci.yml` and repairs whatever it surfaces in `birth_stamp/macos.rs`, which until then no gate had ever compiled. A two-platform identity guarantee stated here is true only once that job is green; if it is not, restate the claim as Linux-only rather than asserting it.
- Phase 5 supplies the interned root index and the root-qualified capture map key; the identity here extends that key rather than replacing it.
- Phase 4 supplies verification and the `Confirmed | Ended | Unknown` outcome, reached through `verify_observation` (`registration.rs:127`) against `birth_stamp::KernelObservation` — private fields, produced in production only by `birth_stamp::observe(pid)`. Membership resolution consumes that outcome and never re-derives it. The proof it yields is `VerifiedRegistration` (`registration.rs:203`); `VersionedRegistration` (`:85`) is the candidate this phase renames.
- Phase 7 supplies the CPU measurement, the managed-process count, and the three-state compiler observation as **named semantic types**, each distinguishing an unavailable measurement from a measured zero. Rows built here use those types directly and must not reintroduce a bare `Option` for any of the three; phase 7's Work Order names them, and this phase uses those names rather than restating them as "optional".
- **Phase 6's root-state contract survives this migration unchanged, and phase 6 owns its surfaces.** `RootReadStatus` (`processes.rs:387`) separates a readable root, a default root deliberately not created and reported quietly, a retryable `Unavailable(PathFailure)`, and an `Invalid(CaptureFailure)` from startup validation that needs a corrected configuration and a restart. `CleanupRefusal` (`capture_root.rs:76`) separately reports foreign ownership as informational, writable mode bits as repairable, effective-user failure as session-long, and access, incomplete-inventory and continuity refusals with the path each affects. `RootStatus` travels on `Scan` into `App` and is read by settings and by `drain_scans`. This phase re-keys identity underneath all of it and builds no replacement surface; keep these tests passing as written: `settings.rs:749`, `:799`, `:853`, `:1063`, `:1137`, and `terminal.rs:744` and `:782`.
- **Diagnostic lifetimes are part of that contract, not incidental detail.** `Capture::take` (`progress.rs:471`) adds the cached boot diagnostic after scanning, and `record_boot_verification` (`:478`) replaces a retryable `IdentityUnknown` with a session-long `IdentityBlockedByBoot` drawn from that same cached boot result — so a boot failure never promises recovery on the next scan. The other retained observations keep enumeration failures, unreadable and invalid registrations, annotation-only and unverifiable artifacts, staging files and unreadable logs apart, and settings shows their paths and retention consequences whether or not any row exists for them. These diagnostics deduplicate **by path**; re-keying capture identity must not collapse that, and these tests stay green: `progress.rs:1303`, `:1333`, `:1421`, and `settings.rs:991`, `:1019`, `:1047`.
- **An unreadable capture with no process rows already recovers and redraws.** `terminal.rs:782` is that regression. It is an existing guarantee this phase preserves, not work this phase takes on, and phase 9 reuses the same `Scan.root_status` transport rather than rebuilding it.

**Acceptance gate:** Build, Test and Lint green. Unit coverage that the identity survives a source transition — the same invocation identified from a registration and from a process resolves to one identity. The complete two-scan acceptance test for the source switch itself belongs to phase 9, which is where registration-sourced rows first exist; do not write a version of it here that needs a row builder this phase does not have. A test asserting the lookup and read outcomes phase 4 names stay distinct where rows consume them, including a log that has printed `Finished` while its application is still running. A test asserting two invocations reusing one pid neither share a tile nor inherit each other's smoothed CPU. A test asserting a capture with no process-table row keeps its unreadable status visible. A test asserting an excluded command produces no row but is still counted live for sweep purposes. A test asserting a root replaced at an unchanged pathname invalidates every retained association keyed on it. A test asserting two macOS births inside one second still yield distinct identities, because the generation and not the truncated birth stamp is what separates them. A test asserting identical pid and birth values with different generations yield distinct `RunId`s — the publisher half is already covered at `tests/shim_registration.rs:1019`, and this is the consumer half. A test asserting a record with unavailable lifetime evidence never merges with another row. A test asserting identical root, pid and birth values with **different generations** retain their own readings and their own confirmed metadata in both capture stores — distinct `RunId`s alone do not prove this, because the stores are what a collision would corrupt. A test asserting the nearest-registration lookup's unverified outcome never satisfies a caller that requires the verified direct association. A test asserting one log pathname named by both a legacy and a versioned registration is read once per scan, paired with one asserting two different generation logs under a single root are read separately. A test asserting two uncaptured or nested invocations sharing a pid and a birth second, neither carrying a generation, do not merge into one row. A test asserting a permission change on a directory that was never replaced preserves its incarnation and every association retained against it — the companion to the replaced-root case above, which must still invalidate them.

---

### Phase 9 — The registration becomes a row source  · status: todo

#### Work Order

**Goal:** a cargo run the process table cannot describe appears on the grid, labelled by the account that owns it.

**Spec:**

This is the item the feature exists for. Phase 4 already opens and verifies the registration — `registered_runs` (`progress.rs:756`) reads each record against fresh kernel evidence, and `Capture::confirmed()` (`:689`) hands out the verified ones — so the fields the grid needs are in hand. What is missing is a row built from them: a registration can *supply* a row rather than only annotate one, and an unreadable argv costs no columns.

*1. The new direction.* Today the linkage runs one way: `captured_run` (`processes.rs:1020`) walks **upward** from a cargo pid to find a registered ancestor and read its state. That walk stays as it is for every row the process table can already build. What is new is the other direction, for a run the process table cannot describe: the registration names a live shim pid, and the fields the row is missing are already parsed. `Capture::confirmed()` (`progress.rs:689`) is the source — it hands out `ConfirmedCapture` (`:427`), each a verified registration with its file's modification time — and `Capture::read` (`:673`) already returns a named outcome rather than an optional. Build rows from `confirmed()`, taking the direct-association type phase 8 defines rather than reaching for the candidate record.

Three cases still stay apart at the row builder, and phase 4's outcomes already name them: the registration was confirmed, so it may source a row; it parsed but verification came back `Unknown`, or the record is a legacy one, so it may annotate an existing row and nothing more; or it could not be read at all. The middle case is the one that decides whether a row exists, so the row builder reads the outcome rather than testing for emptiness.

| field | source | note |
| --- | --- | --- |
| `path` | registration | the raw absolute directory; the reader shortens it to `~` only when the record's writer-home field matches the scanner's own, and otherwise renders it in full |
| `command` | registration | `cargo <args>`, as typed, one field per argument |
| `pid` | the registration's filename | the shim's pid, a live process |
| `started` | the registration file's **mtime** | see below |
| `cpu`, `compiler`, `managed` | process table where it answers | absent rather than wrong |

Using the registration's mtime as the run start avoids the one call least certain to answer across a uid boundary. The shim writes the file once, at the moment the run begins, and never rewrites it; the reader already stats the entry. Whether sysinfo's macOS backup path carries a usable start time is untested, so nothing here may depend on it.

`ConfirmedCapture` (`progress.rs:427`) already holds that time as `Result<SystemTime, CaptureFailure>`, taken from the same descriptor as the bytes, because verification is allowed to succeed while the timestamp read fails. The fallback row therefore has to survive a failed timestamp: name the unavailable case, decide what the duration column shows for it, and give it a deterministic place in the ordering rather than letting it sort against a missing value. Keep `registration_bytes_and_timestamp_always_come_from_the_same_descriptor` passing unchanged.

*A verified registration and a readable log are separate facts, and the row source depends on only the first.* `scan_root` retains a `ConfirmedCapture` (`progress.rs:594`) before it ever reads that registration's log, while `RootStatus.confirmed` counts only the publications whose logs came back readable — the shipped test at `progress.rs:1258` asserts two retained confirmations against a displayed active count of one, and that difference is deliberate. A row sourced here follows the retained proof rather than the count: a verified registration whose log is unreadable still supplies its directory, command, pid and start time, and the root diagnostic naming the unreadable log stays exactly where phase 6 put it.

*2. Precedence is per field, never per row.* Preferring a whole process-table row discards a working directory the registration has, because `row()` (`processes.rs:1239`) degrades a missing cwd to `UNRESOLVED_PATH` (`:1248-1249`) rather than failing. That is the ordinary runner-machine case, not a corner: there argv is readable and cwd is denied, so the row survives with `path: "unavailable"` while the registration holds the real directory. The process row keeps its real pid, its tree and its measured values; an absent cwd is filled from the matching registration; and the merge happens **before** any field becomes display text.

Only the invocation a registration directly represents takes cwd and arguments from it — Phase 8's `InvocationId::Captured` distinction. A nested invocation does not inherit them.

*3. The account has to be visible.* Phase 4 already settled the two premises this item started from: `group_by_path` (`render.rs:1350`) groups on `row.process.directory_identity`, the raw identity rather than display text, and `registration_directory` (`processes.rs:1323`) collapses a directory to `~` only when the record's writer home matches the scanner's own. What is still merged is what a directory alone cannot separate: two accounts, or two roots, holding the same absolute directory. So the remaining work here is the fallback rows, the per-field merge, account and root qualification, the heading prefix, and pin identity — and the existing different-HOME, raw-byte and unavailable-directory regressions stay passing through all of it.

Extend the grouping key to the verified account and the root identity alongside the working directory it already carries. Each of the three separates runs the other two leave merged. Dropping the directory merges every run one account has going in different checkouts into a single heading and a single gauge, which is the more common shape on a build machine than the collision this item started from; dropping the account or the root merges the two accounts back together. The directory in the key is the identity the registration carries, not the `~`-collapsed text the heading shows. Family colour cannot substitute for any of it, since it means process ancestry and is absent for a command with no children. Present the account as a **prefix in the heading**: `[hana-linux-1] /run/github-runner/hana-linux-1/x` in tile and summary headings, absolute wherever the writer's home differs from the scanner's, and `~`-collapsed only where they match. It appears once per heading, where the grouping already lives, rather than spending width on every row. Derive the account name from the root's owner uid, not from the path text, and never let a name that could not be resolved render as an account with no name.

*4. A fallback row is for a run with no process row, never a second view of one that has one.* `Capture::confirmed()` (`progress.rs:689`) hands out proofs from every root, and a pid can be confirmed under a root the process-row association did not select. Build a fallback row **only** where the process table describes the shim pid nowhere. A non-selected proof for a pid that already has a row never becomes a second row and never overwrites that row's own root's metadata — one live invocation is one tile. **Phase 6's root status does not already explain every one of those, and this Work Order must stop assuming it does.** `Census::associate_status` (`processes.rs:1036`) walks the process rows that exist and collects a suppressed proof only where the registration it selected came back **unconfirmed**. Two cases fall outside that, and they are exactly the two this phase creates: a pid confirmed under more than one root, where the selected proof is itself confirmed, and a fallback row with no process-table entry at all. Neither inherits any record of which proof was selected or why the others went unused. Reporting the final selection therefore belongs here — carry it through `CaptureAssociation`, the surface phase 6 already renders, so one explanation reaches the operator whichever way the row was sourced. This phase owes the row-count rule, the reported selection, and the tests for both.

**Files:**
- `crates/cargo-tile/src/progress.rs` — the row-sourcing accessor beside `confirmed` (`:689`)
- `crates/cargo-tile/src/processes.rs` — the registration-sourced row, the per-field merge, `row()` (`:1239`), and `registration_directory` (`:1323`)
- `crates/cargo-tile/src/roster.rs` — the literal row fixtures (`:527`), which change with every identity field a row gains
- `crates/cargo-tile/src/render.rs` — grouping on account, root and working directory (`:1350`), pin selection (`:1398`), the heading prefix
- `crates/cargo-tile/src/constants.rs` — the heading prefix's presentation literals
- `crates/cargo-tile/src/terminal.rs` — the `CargoProcess` row literal (`:869`), built from the row schema this phase changes
- `crates/cargo-tile/tests/shim_registration.rs` — root-qualified headings and reader regressions against the production binary

**Seats:** 2 writers + 1 tester — splits by file: the row source and its merge, the renderer's grouping and headings, and a real tester lane on the production-binary harness at `tests/shim_registration.rs:652`, which phase 4 proved out. Each writer writes the inline `#[cfg(test)]` tests for the files it owns. Paths below are relative to `crates/cargo-tile`.
- `impl` — opens as impl: `src/processes.rs`, `src/progress.rs`, and `src/roster.rs` including every row fixture — the registration-sourced row, the per-field merge at `row()` (`:1239`) where `path` degrades to `UNRESOLVED_PATH` but `command` fails the whole row, and the row-sourcing accessor beside `confirmed`; hub: `src/processes.rs`, the row builder both writers feed and read. Owns the source-merge tests. Keeps the `roster.rs` fixtures current through every repair round, plus `src/terminal.rs` — its `CargoProcess` row literal (`:869`) is built from the row schema this seat changes and cannot be migrated by anyone else
- `test` — opens as impl: `src/render.rs` — `group_by_path` (`:1350`) keys on verified account, root and working directory, headings gain the account prefix, and pin selection (`:1398`) migrates with the key — plus `src/constants.rs` for the prefix literals, this writer's alone in this phase; writes into the existing `#[cfg(test)]` module (`:2030`)
- `review` — opens as test: `tests/shim_registration.rs` — root-qualified headings and reader regressions driven through the production binary; claims no writer file

**Constraints from prior phases:**
- Phase 5 supplies the root-qualified capture key. `Capture::confirmed()` (`progress.rs:689`) returns proofs from **every** root, while the process-row association selects one root per pid, so the two sets can disagree about the same shim pid. **A fallback row exists only for a shim pid the process table cannot describe at all.** A confirmed proof in a root the process row did not select never creates a second row: one live invocation is one tile, and a second row for it is precisely the duplicate phase 5's deduplication exists to prevent. Where the two disagree, phase 6's root status is the surface that explains it.
- Phase 5 verified anchors: `Capture::read` (`progress.rs:673`) **takes a `CaptureKey`, not a pid**; `Capture::confirmed` (`:689`); `ConfirmedCapture` (`:427`), which now carries `key: CaptureKey`; `row()` (`processes.rs:1239`); `registration_directory` (`processes.rs:1323`); `group_by_path` and pin selection stay in `render.rs`.
- Phase 5 added a `CargoProcess` literal to `terminal.rs:869` carrying `cpu: String`, `compiler: Option` and `managed: usize`. Any row-schema change here has to migrate it.
- Phase 4 supplies the parsed record: the raw absolute working directory, the writer's home prefix as its own field, argument count and arguments as separate fields, and the `Confirmed | Ended | Unknown` outcome. Only `Confirmed` may source a row; `Unknown` and legacy records annotate only. The type a row is built from is `VerifiedRegistration` (`registration.rs:203`), reached through phase 8's direct-association wrapper, never the candidate record. The `~` collapse happens here, on the display path, and only when the record's prefix matches the scanner's own — otherwise the heading shows the absolute path rather than a `~` naming a different directory.
- `group_by_path` (`render.rs:1350`) takes `pinned: Option<&str>` and pin selection (`:1398`) chooses by display text, so grouping identity and pin identity migrate together. A pinned directory carries the full grouping identity — account, root and raw directory — not the rendered string; otherwise two distinct directories that render alike pin the wrong group. Give that state a **name** rather than an `Option<GroupingIdentity>`: a cell either heads with a pinned group or it does not, and the second is a state, not a missing value. The scanner's own home has the same shape — convert `dirs::home_dir()` once into a named scanner-home observation instead of threading `Option<&Path>` through `registration_directory` (`processes.rs:1323`) and `home_relative`.
- Phase 8 renames the two `DirectoryIdentity` types for their roles — the working directory a run reported, and inspected filesystem metadata for an open handle. Use those names here; the grouping key takes the first.
- Phase 8 supplies `InvocationId::Captured(RunId) | Process(ProcessIdentity)`, the resolved membership, the scan-scoped capture reads, and the exclusion ordering. A row may not be built for an excluded command.
- Phase 7 supplies the CPU measurement, the managed-process count, and the three-state compiler observation as **named semantic types**. A registration-sourced row leaves all three unavailable rather than zero, and says so through those types rather than through a bare `Option`; use phase 7's names, not "optional".
- **Phase 6 supplies a numeric owner, not an account name.** `RootOwner` (`capture_root.rs:67`) is `Uid(u32) | Unavailable`, and settings deliberately renders the number (`settings.rs:589`) rather than looking anything up. The `[hana-linux-1]` heading this phase specifies therefore needs scanner-side account-name resolution that no earlier phase built, and building it is this phase's work rather than an inherited fact. Keep the two values apart: the uid is the identity the grouping key qualifies on, and the name is display text derived from it, so two roots owned by one uid group together even if the lookup answers differently for each. The lookup can also come back with nothing — a uid whose passwd entry was removed with the runner account is the ordinary case, not a rare one — so name that outcome rather than substituting an empty string, and let the heading fall back to the number the settings surface already shows.
- Phase 5 supplies the interned root index that qualifies the identity.
- **The transport for everything above already exists.** Phase 6 carries `RootStatus` on `Scan` into `App`, and `terminal.rs:782` is the shipped regression for an unreadable capture recovering and requesting a redraw with no process rows at all. Reuse that path for the selection reporting and the account this phase adds; do not build a second one alongside it.

**Acceptance gate:** Build, Test and Lint green. The complete two-scan source-switch acceptance test lands here, not in phase 8: one run observed from a registration on the first scan and from a process on the second keeps one tile with a stable identity, and an excluded command produces no row from either source. Two directory-identity tests, on phase 4's raw working directory: two distinct absolute directories that collapse to the same display text group as two, and grouping stays stable when a row's source changes. A test asserting the intended lead directory stays first when two groups render identically, across a source transition. A test asserting a record whose writer home prefix differs from the scanner's renders an absolute heading rather than a `~` collapse. A test asserting a registration with no matching process-table row produces a row carrying its working directory and command. A test asserting a process row with an unavailable cwd and a matching registration renders the registration's directory while keeping the process row's pid and measured values. A test asserting two roots reporting the same `~/x` produce two headings, each prefixed with its own account. A test asserting two working directories under one account and one root produce two headings rather than one. A test asserting the same account and the same absolute directory under two different roots produce two headings — the case that isolates the root component from the account and the directory. A test asserting a confirmed registration whose timestamp read failed still produces its row, with a named unavailable time, a defined duration display, and a deterministic place in the ordering. A test asserting an `Unknown` registration sources no row. A test asserting a confirmed registration in a root the process row did not select produces **no** second row for that pid, and that the row it already has keeps its own root's metadata. A test asserting the same pid confirmed under two roots yields one row, and a test asserting an unconfirmed reading in the selected root beside a confirmed proof in another still yields one row carrying no borrowed metadata. A test asserting that single row records which proof was selected and why the other went unused, and the same for a registration-sourced row that has no process-table entry. A test asserting a verified registration whose log is unreadable still sources its row, with the root's log diagnostic preserved and the displayed active count unchanged. A test asserting a root whose owner uid has no resolvable account name still renders its heading, prefixed with the number, and still groups apart from a root owned by a different uid.

### Phase 10 — A claim covers the merge it prevents, and nothing else  · status: todo

#### Work Order

**Goal:** A reservation protects exactly the branch's unmerged surface, stops
growing past it, and stops holding anything once the branch merges — without an
operator running a verb.

**Spec:**

*One scope set is doing two unrelated jobs.* `is_foreign_to_coordination_run_in_worktree`
(`reservation/record.rs:242`) refuses an edit on two separate grounds, and reads
the same scope set for both:

- **A different worktree** — `self.actor.worktree != worktree_id`. This is
  merge-conflict protection: two branches modifying the same file conflict when
  both reach trunk.
- **The same worktree under a different coordination run** — a direct race over
  one checkout on disk.

The two grounds have different natural extents and different natural lifetimes,
and sharing one set gives each of them the other's:

- Merge protection should last until the branch merges and cover exactly the
  branch's unmerged surface. It instead inherits accumulation, so it grows past
  that surface without bound. Observed on `01a06fde`: 45 recorded overlap
  answers, every one `widen_without_foreign_overlap` with cause `drift`, ending
  at 293 paths spanning three crates plus root files — a reservation protecting a
  change in `crates/cargo-berth` came to own the whole of `crates/cargo-tile`.
- Race protection should last as long as a run is editing and cover what that run
  has open. It instead inherits "until merged", so it outlives the run. Observed
  on the same reservation: still held days after its worktree returned to clean,
  blocking an unrelated phase in a second worktree and raising incursion
  `01a0887c`. The two worktrees had separate working trees on different branches
  and could not collide on disk at all.

One cause, both symptoms. Fix the extent and the lifetime follows.

*The rule.* A reservation's merge-protecting scope is **derived on read, never
accumulated**:

    files changed by `trunk..HEAD` in the holder's worktree
      ∪ files currently modified there (staged, unstaged, untracked)

That is the merge-conflict surface exactly. Two properties follow, and they are
the whole point:

- **It is bounded by the merge, not by the run.** It grows as the branch
  accumulates work against trunk — which is precisely what can conflict — and it
  stops holding anything once the branch merges. It does **not** exclude another
  run's commits, and no revision range could: `drift/observation.rs:194` states
  that a range "is a comparison, not an authorship record", which applies to
  `trunk..HEAD` exactly as it does to `phase_start..HEAD`. That is the intended
  behaviour rather than a limitation. Branches are what merge, so a claim covering
  the whole branch surface against trunk covers the conflict it exists to prevent;
  a claim scoped to one run's own commits would leave the rest of the branch
  unprotected while still being merged.
- **It empties itself.** Once the branch merges, `trunk..HEAD` is empty; a clean
  worktree then derives an empty scope set, and the reservation holds nothing.
  There is no retirement verb to run, no operator step to remember, and no new
  disposition to invent. Retirement stops being an action and becomes a
  consequence of the same computation.

Check the rule against the incident before building it. `cargo-liner` was on
`main`, `0 0` against `origin/main`, working tree clean, nothing uncommitted.
Unmerged surface: empty. Under this rule the claim covers zero paths, not 293,
and it reached zero on its own the moment its work landed.

*What race protection keeps.* The intra-worktree ground stays and keeps its own
extent — the paths the run declared or first-touched. It is **not** derived from
the branch: two runs sharing one checkout race over what they have open right
now, not over what the branch will eventually merge. Keep the two extents
separate on the reservation. Collapsing them back into one set is the defect this
phase removes, and a later change that "simplifies" them back together
reintroduces it whole.

*The race extent is dropped when the run ends.* This is what makes ordinary
sequential work possible, and it is the half most easily left out. A run commits
a path on Monday and finishes. A later run in the **same worktree on the same
branch** must be free to edit that same path on Tuesday: the two never overlap in
time, so there is no race, and the branch has no conflict with itself. The
merge extent still covers that path — it is unmerged work, and a *different*
worktree must still be refused it — but the intra-worktree ground reads the race
extent only, and Monday's race extent is gone.

The two grounds therefore answer opposite ways about one path, and both answers
are right: same worktree asks "is another run editing this right now", different
worktree asks "does this branch have unmerged changes here". Never let one ground
read the other's extent as a fallback.

*Widening applies to one extent only.* `JournalOperation::Widen` is emitted from
two places — `drift/classification.rs:258`, the drift path this incident came
through, and `widen_first_touch_reservation` (`verb/claim.rs:1151`, emitting at
`:1176`), the first-touch reuse path — and `apply_widen`
(`reservation/retention.rs:958`) mutates the shared set on replay. After this
change all three grow the **race** extent only. Nothing widens the merge extent,
because nothing needs to: it is computed. A bound enforced in one producer and
not the other is a bound the other walks around, so both producers change
together.

*Where the derivation runs.* Under the lock, in `prepare_reconciliation_transaction`
(`reconcile.rs:949`), where a board read already holds the lock and already
produces journal operations. `retention.rs` replays journal operations and is not
where a board-time computation belongs; `git/mod.rs` only re-exports, and the
queries live in `git/reachability.rs`.

*Cost.* Deriving on read means git work per board read, and the board is read
constantly. Per live holder worktree that is one `trunk..HEAD` name-only query
plus one status observation — the staged, unstaged and untracked paths today's
worktree observations do not carry. Hold the cost bounded across the whole read
rather than per reservation, and key the cache on trunk as well: `WorkingTreeFingerprint`
(`drift/fingerprint.rs:18`) carries path sets alone, so a pair of `HEAD` and that
fingerprint misses a trunk that moved underneath an unchanged branch and would
serve a stale extent. The key is (trunk tip, `HEAD`, working-tree fingerprint), and
a recomputation happens only when one of the three actually moved.

*What must not regress.*

- **Uncommitted work stays covered.** The dirty half of the union is what does
  this. A run that has committed nothing still protects everything it has open —
  the pre-commit window is the thing retention exists for, and the answer there
  can never be "nothing".
- **An unanswerable derivation holds, never releases.** If the derivation cannot
  run — worktree gone, unreadable, a failed git invocation — the reservation
  keeps its last derived set and reports the failure rather than collapsing to
  empty. An unanswered question is not proof that nothing is protected.
- **A narrowing is as visible as a refusal.** A claim that quietly stops covering
  what the work touched is worse than one that grows, because the protection
  disappears with nobody told. If a refused widen still publishes the working-tree
  fingerprint, the next invocation sees no drift for those paths and the refusal
  never surfaces again. A refused widen must leave the next comparison able to
  see the same paths.
- **Successor retention refs survive.** A reservation whose merge extent empties
  must not drop retention refs a successor still depends on (`RetentionRefStatus`,
  `alert.rs:263`).
- **Obsolete identity mappings retire with it.** `verb/release.rs` publishes a
  session identity mapping on release (`SessionIdentityMappingPublication`); a
  reservation that empties owes the same bookkeeping, or it leaves a mapping
  pointing at nothing.
- **An incursion recorded against the reservation gets its own disposition**
  rather than disappearing when the claim empties.

*Squash and cherry-pick stay manual.* Where a branch's work reached trunk in
rewritten form, `trunk..HEAD` may still list paths the tool cannot prove
integrated. `resolve --integrated-as <TRUNK_OID>` stays the manual answer for
that shape, with its existing warning intact.

**Files:**
- `crates/cargo-berth/src/reservation/record.rs` — the two refusal grounds (`is_foreign_to_coordination_run_in_worktree`, `:242`) and the reservation's two extents (`:49`).
- `crates/cargo-berth/src/reservation/partition.rs` — coverage and binding (`is_foreign`, `authorizes`, `reservations_authorize_scope`), expressed against whichever extent each ground reads.
- `crates/cargo-berth/src/reservation/retention.rs` — `apply_widen` (`:958`), now mutating the race extent only.
- `crates/cargo-berth/src/reservation/lifecycle.rs` — how an emptied merge extent reads as a terminal state (`ReleaseDisposition`, `:189`).
- `crates/cargo-berth/src/reconcile.rs` — `prepare_reconciliation_transaction` (`:949`), where the derivation runs under the lock.
- `crates/cargo-berth/src/git/reachability.rs` — the `trunk..HEAD` query and the batched ancestry it joins; `crates/cargo-berth/src/git/mod.rs` only re-exports.
- `crates/cargo-berth/src/worktree/` — the staged, unstaged and untracked observations the dirty half needs.
- `crates/cargo-berth/src/drift/classification.rs` — `:258`, the drift widen producer, now bounded to the race extent.
- `crates/cargo-berth/src/verb/claim.rs` — `:1151`, emitting at `:1176`; the other widen producer, likewise bounded to the race extent.
- `crates/cargo-berth/src/drift/fingerprint.rs` — the published fingerprint the cache keys on and a refusal must not hide behind.
- `crates/cargo-berth/src/drift/observation.rs` — `:194`, what the ranges establish.
- `crates/cargo-berth/src/drift/selection.rs` — which reservations are compared.
- `crates/cargo-berth/src/ledger/journal.rs` — `Widen` at `:362`, the durable operations.
- `crates/cargo-berth/src/ledger/projection.rs` — their projection.
- `crates/cargo-berth/src/board/` — successor retention refs.
- `crates/cargo-berth/src/edge/` — successor retention refs.
- `crates/cargo-berth/src/alert.rs` — `:263`, identity-mapping bookkeeping.
- `crates/cargo-berth/src/recovery.rs` — identity-mapping bookkeeping.
- `crates/cargo-berth/src/verb/release.rs` — identity-mapping bookkeeping.
- `crates/cargo-berth/src/output.rs`, `crates/cargo-berth/src/output_contract.rs`, `crates/cargo-berth/src/constants.rs`, `crates/cargo-berth/src/cli.rs` — how a derived scope, a refused widen and an emptied claim report.
- `crates/cargo-berth/tests/drift.rs`, `crates/cargo-berth/tests/overlap.rs`, `crates/cargo-berth/tests/hooks.rs`, `crates/cargo-berth/tests/lifecycle.rs`, `crates/cargo-berth/tests/ledger.rs`, `crates/cargo-berth/tests/board.rs`, `crates/cargo-berth/tests/edges.rs`, `crates/cargo-berth/tests/output_contract.rs` — the lane.

**Seats:** `1 writer + 1 tester + reserve`. The separation of the two extents is a
single judgment that the refusal grounds, both widen producers, replay and the
board all have to agree on; splitting it puts halves of one decision in different
heads, and the file list is wide only because everything reads the same rule.
`cargo-berth` has a real integration-test lane (Delegation Context → **Test
lanes**), and the rule above is concrete enough to test before the code exists,
so the tester opens as `test`.
- `impl` — opens as `impl`. Owns `crates/cargo-berth/src/reservation/`, `crates/cargo-berth/src/reconcile.rs`, `crates/cargo-berth/src/git/`, `crates/cargo-berth/src/worktree/`, `crates/cargo-berth/src/drift/`, `crates/cargo-berth/src/verb/`, `crates/cargo-berth/src/ledger/`, `crates/cargo-berth/src/board/`, `crates/cargo-berth/src/edge/`, `crates/cargo-berth/src/alert.rs`, `crates/cargo-berth/src/recovery.rs`, `crates/cargo-berth/src/output.rs`, `crates/cargo-berth/src/output_contract.rs`, `crates/cargo-berth/src/constants.rs` and `crates/cargo-berth/src/cli.rs`; hub: `crates/cargo-berth/src/reservation/record.rs`, which carries both extents and both refusal grounds
- `test` — opens as `test`. Owns `crates/cargo-berth/tests/drift.rs`, `crates/cargo-berth/tests/overlap.rs`, `crates/cargo-berth/tests/hooks.rs`, `crates/cargo-berth/tests/lifecycle.rs`, `crates/cargo-berth/tests/ledger.rs`, `crates/cargo-berth/tests/board.rs`, `crates/cargo-berth/tests/edges.rs` and `crates/cargo-berth/tests/output_contract.rs`
- `review` — reserve. Reads the derived extent against the recorded incident and against both widen producers

**Constraints from prior phases:** None. This phase is independent of phases 1-9
and may run before or after them.

*The cache key includes trunk.* The derived extent is cached on
`(resolved trunk revision, HEAD, working-tree fingerprint)`. The first component is not optional
bookkeeping: neither of the other two moves when **trunk** advances while the holder's `HEAD` stays
fixed and its worktree stays clean, yet `trunk..HEAD` has just become empty. A key without it keeps
covering paths the branch no longer holds unmerged, and the reservation goes on refusing another
worktree work it should now allow — the over-holding this phase exists to end, reintroduced through
the cache. `crates/cargo-berth/src/drift/fingerprint.rs:18` builds its fingerprint from path sets
only and observes nothing about trunk, so this is an addition to the key rather than a change to it.

**Acceptance gate:** `verify.sh check cargo-berth`, `verify.sh test cargo-berth`
and `verify.sh lint cargo-berth` green — this phase edits no `cargo-tile` file,
so the Delegation Context's default `cargo-tile` gate would pass without
compiling anything this phase wrote.

Reproducing the incident: a test that a worktree on trunk with nothing ahead and
a clean tree derives an **empty** merge extent, so a second worktree may edit
paths that reservation previously held. A test that a reservation whose branch
touches one crate does not come to hold a second crate through repeated drift
answers.

The cache: a test where **only** trunk changes — the holder's worktree untouched, its `HEAD`
fixed — and the next board read releases the merge extent.

The derivation: a test that a path committed on this branch and not yet on trunk
is covered. A test that the same path stops being covered once the branch merges,
with no verb run and no operator step. A test that a path committed by a
*different* run onto the same branch **does** enter this reservation's merge
extent, and does **not** enter its race extent. The merge extent is derived from
`trunk..HEAD` and is branch-wide by construction — a branch is what gets merged,
so every unmerged change on it is covered whichever run made it. The
run-specific set is the race extent, and that is the one another run's commit
stays out of. Stating it of the merge extent contradicted the derivation rule
above and made the two requirements unsatisfiable together. A test that reading the board repeatedly derives the same extent and
emits at most one transition, not one per read.

Uncommitted work: a test that a reservation that has committed nothing still
covers everything modified in its worktree. A test that a staged, an unstaged and
an untracked path are each covered.

The two extents stay separate: a test that a second coordination run in the same
worktree is refused a path the first run currently has open, while the first run
is **still live**. A test that the same second run is **allowed** that same path
once the first run has ended — committed on the branch, unmerged, and the merge
extent still covering it — since sequential work on one branch is the ordinary
case and must not be refused. A test that a *different* worktree is refused that
same path at that same moment, so the two grounds are shown answering opposite
ways about one file. A test that a widen from the drift path grows the race
extent and leaves the merge extent unchanged, and the same test for the
first-touch reuse path.

Failure and visibility: a test that a worktree whose state cannot be observed
keeps its last derived extent rather than collapsing to empty. A test that a
refused widen is reported rather than silently dropped. A test that a refused
widen leaves the next comparison able to see the same paths rather than being
hidden by a published fingerprint. A test that replaying the journal reconstructs
the reservation and its race extent without re-running git. A test that an
incursion previously recorded against the reservation receives its own
disposition rather than disappearing when the claim empties.
