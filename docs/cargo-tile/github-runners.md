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
    Result<CargoArguments, RowAbsence>` `:1610`, which reports an unavailable
    argv as its own outcome — the macOS empty-argv gate. `row(...) ->
    Result<CargoProcess, RowAbsence>` `:1427`: `path` degrades a missing cwd to
    `UNRESOLVED_PATH` via `map_or_else` (`:1436-1437`) while `command:
    command_text(process.cmd(), home)?` (`:1453`) fails the whole row — the
    asymmetry phase 9's per-field merge resolves. `CpuSmoothing` `:709`;
    `Census` `:794` with `take` `:807`, phase 7's collection boundary; `is_shim`
    `:1005`; `drawn_parent` `:1070`; `captured_run` `:1181`, returning
    `CaptureLookup`; `annotate_capture` `:1245`, which selects a confirmed
    registration by pid alone; the exclusion retain `:617`, whose `.is_ok()`
    discards the `RowAbsence` distinction; `Capture::take()` at `:631`;
    `aggregate_compilers` `:1383` and `aggregate_cpu` `:1413`, phase 7's
    aggregation boundary and in this file rather than in `render.rs`;
    `registration_directory` `:1511`, which collapses a directory to `~` only
    when the record's writer home matches the scanner's own. Inline
    `#[cfg(test)]` blocks `:242` and `:1713`.

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
    Vec<PathGroup<'a>>` `:1353`** — groups by comparing
    `row.process.directory_identity` (`:1356-1361`), the **raw** directory
    identity rather than the rendered text, so two directories that render alike
    never merge on their display strings. That is the site phase 9 keys on
    verified account and root as well, so rows under different roots stay
    separate. **Pin selection at `:1401` still chooses by display text**
    (`row.process.path == path`), which is why grouping identity and pin identity
    migrate together. Its doc `:1343-1352` records that path order never moves on
    its own and that the linear search is deliberate. Phase 7's group totals are
    here: the sites summing `cpu` / `managed` across a group's members, each of
    which now propagates an unavailable member into the total. Inline
    `#[cfg(test)]` module `:2038` (`mod tests` `:2044`, 56 tests, layout and
    eliding).

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
    `TrackedRow` `:26`, `TrackedGroup` `:126`, `Roster` `:282`, `assign_families()`
    `:388` — the parent/child grouping phase 7's unknown-measurement rules pass
    through: a family survives an unavailable managed count, can be established
    from a retained child's parent link, and is released only on a measured zero.
    Inline tests `:458` (27 tests).

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
  an inline `#[cfg(test)]` module inside the file it tests: `processes.rs:1713`
  (79), `progress.rs:844` (73), `render.rs:2038` (56), `hook.rs:348` (33),
  `terminal.rs:672` (9), `cli.rs:163` (6), `config.rs:283` (5), plus modules in
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

### Phase 7 — An unknown measurement never renders as a measured zero  · status: done

#### As-built

`processes.rs` defines the measurement vocabulary every consumer reads: `Measurement<T>` = `Reading(T) | Unavailable(MeasurementAbsence)`, `MeasurementAbsence` = `FirstObservation | ReadFailed | Unproven`, `CompilerObservation` = `Unknown | None | Running(Compiler)`, and `CpuPublication` = `NeverPublished | Published(Instant)` in place of `CpuSmoothing`'s former `taken` optional. `measure_cpu` classifies at collection: a sysinfo rate counts as a fresh computation only when the process's *previous* accumulated counter is positive; a positive rate on an unchanged counter and a counter that moved backwards both yield `Unproven`; a zero rate against nonzero accumulated time is a measured zero and still renders `0%`. A pre-refresh snapshot taken before `refresh_processes_specifics` supplies each process's baseline, so classification never feeds itself. `Add for Measurement<T>` propagates absence, so one unavailable member makes a group total unavailable; `aggregate_cpu` and `cpu_label` are `pub(crate)` so display tests exercise production aggregation, while `aggregate_compilers` stays private. Smoothing drops a published reading the moment a sample goes unavailable rather than holding it to the next reporting deadline, and recovery restarts from the new value. The CPU, compiler and managed columns print the single shared literal `UNAVAILABLE_MEASUREMENT` (`--`); the compiler column keeps blank for "no compile running" distinct from `--`.

**Files:**
- `crates/cargo-tile/src/processes.rs` — the four measurement types, `Census::take`, `measure_cpu`, `CpuSmoothing` (`settled` and `reported` pid-keyed maps plus `publication`), `aggregate_cpu` / `aggregate_compilers`, and the inline collection-boundary tests
- `crates/cargo-tile/src/render.rs` — per-row and per-total rendering of an unavailable value, plus the inline display tests
- `crates/cargo-tile/src/roster.rs` — `assign_families`, which acts on the managed count as a `Measurement`
- `crates/cargo-tile/src/constants.rs` — `UNAVAILABLE_MEASUREMENT`
- `crates/cargo-tile/src/terminal.rs` — two `CargoProcess` row literals, one carrying readings and one carrying unavailable measurements

**Binds later work:** the type names above are the ones downstream rows and constraints spell out. CPU history is three fields, not one — re-keying it off the bare pid reaches `settled`, `reported` and `publication` alike. `UNAVAILABLE_MEASUREMENT` is the only unavailable marker; later rows reuse it rather than adding a second. `CargoProcess.parent: Option<u32>` is still a bare optional, untouched here.

**Gotchas:** sysinfo's CPU rate is trustworthy only when the previous accumulated counter is positive — macOS `compute_cpu_usage` assigns `cpu_usage` solely inside that condition and otherwise leaves the earlier rate in place, and Linux returns early while both old counters are zero, so any future consumer of that rate needs the same precondition. `accumulated_cpu_time()` is quantized (integer milliseconds on macOS, whole clock ticks on Linux), so it may be read as evidence but never asserted as proof of a failed measurement. `CpuBaseline` pairs pid with a whole-second start time, so a same-second pid replacement with a nondecreasing counter still passes as a continuing process. The crate is binary-only: there is no library target for `tests/` to link against, and every test of a crate item is an inline `#[cfg(test)]` module in the file it covers.

**Ruled out:** banning `accumulated_cpu_time()` outright — the counter must be read, only failure claims drawn from it are forbidden; making every equal-counter sample unavailable — a zero rate against nonzero accumulated time is a measured zero; giving a promoted managed summary row its subtree's CPU total — such a row reports its own pid's share by pre-existing design; separating a same-second pid replacement here — that needs process-lifetime identity evidence this code does not collect.

### Phase 8 — Invocation identity and capture membership  · status: done

#### As-built

- `InvocationId::{Captured(RunId), Process(ProcessIdentity)}` (`processes.rs:89`) keys capture lookup, deduplication, roster, tiles, CPU smoothing and render, with the displayed pid carried separately. `RunId` holds root index, `RootIncarnation`, shim pid, publication generation and `BirthStamp`; `ProcessIdentity` is `Known { pid, lifetime: ProcessLifetime }` or `Unavailable { pid, observation: Uuid }`, and `ProcessIdentities::observe` retains an unavailable identity only while its pid stays continuously present.
- `CaptureMembership::{Enclosing(RunId), Outside}` (`:176`) keeps membership distinct from identity, so a nested invocation keeps its own command and directory. `Census::direct_capture` (`:1497`) returns `DirectAssociation::{Direct, None}` and its parent walk crosses only `capture_wrappers` (`:981`) — processes whose argv forwards the registered cargo command — so an exec-replaced `cargo run` application cannot hand nested cargo children the enclosing run's identity.
- Ownership resolves through `Capture::select(pid) -> CaptureSelection::{Unregistered, Selected(CaptureKey), Ambiguous}` (`progress.rs:713`); `Capture::keys` (`:753`) is `#[cfg(test)]` and inspects retained readings only. `CaptureKey` (`:280`) is root, pid, incarnation, generation and birth, and `scan_root` (`:611`) reads each log pathname once per scan through one shared map, so a legacy and a versioned record naming one pathname produce a single read.
- `RootIncarnation(Uuid)` (`capture_root.rs:484`) changes only when the directory object itself is replaced, kept apart from the cleanup-continuity signal; the private `RootHistory` (`:396`) pins one `OwnedFd` per observed root so inode reuse cannot read as continuity, and exposes no cleanup capability.
- Bare optionals became named states: `RowLifetime::{Live, Finished(Instant)}`, `FamilyHead::{NoChildren, Heads}`, `ParentFamily::{NoFamily, Member}` (`roster.rs:28`), `VisibleParent::{Invocation { id, pid }, Ancestor(u32), None}` (`processes.rs:185`), `CounterState::{Working, Blocked, NoCurrentProgress, Unavailable, Unregistered}` (`progress.rs:172`), `FocusLocation::{Cell, Departing}` (`tiles.rs:216`), `AncestryLevel::{Ancestor, Elided}` (`render.rs:973`).
- `VersionedRegistration` is now `RegistrationCandidate` (`registration.rs:85`) with `arguments()` (`:119`) a production accessor; the two former `DirectoryIdentity` types are `WorkingDirectoryIdentity` (`registration.rs:158`) and the private `InspectedDirectoryMetadata` (`capture_root.rs:488`). `is_shim` (`processes.rs:1226`) delegates to `observed_shim_match` and requires observed equality, so two unavailable cwds no longer match. `CpuSmoothing` (`:885`) and `CpuBaseline` key on invocation identity and lifetime evidence, and `process_details` (`:824`) builds a fresh `System` per scan.

**Files:**
- `crates/cargo-tile/src/processes.rs` — the identity types, membership, visible parent, the census wrapper set and direct association, CPU smoothing and baseline
- `crates/cargo-tile/src/progress.rs` — `CaptureKey`, `CaptureSelection`, `Capture::select`, `CounterState`, per-scan log read deduplication
- `crates/cargo-tile/src/capture_root.rs` — `RootIncarnation`, `RootHistory`, `InspectedDirectoryMetadata`
- `crates/cargo-tile/src/registration.rs` — `RegistrationCandidate`, `WorkingDirectoryIdentity`, the production `arguments()`
- `crates/cargo-tile/src/{roster,tiles,render,terminal}.rs` — consumers re-keyed off the bare pid, each with its named states
- `crates/cargo-tile/tests/shim_registration.rs` — pty regressions for nested invocation identity, exclusion and ambiguity; the harness locates a pane by its fixture markers before counting headings

**Binds later work:** ownership is taken through `Capture::select`, never `Capture::keys`, and `Ambiguous` means no owner rather than "pick one" — the registration-sourced row phase must not collapse it into `Unregistered`. A process reached through a non-forwarding parent is `CaptureMembership::Enclosing`, never the run's own identity. `RootIncarnation` belongs in any grouping or pin key retained across scans, and grouping keys on `WorkingDirectoryIdentity`. `arguments()` was opened for the registration-sourced row builder. The two-scan source-transition acceptance test is still owed by that phase, and both halves — registration-sourced and process-sourced identity — now resolve to one value.

**Gotchas:**
- `UpdateKind::OnlyIfNotSet` never re-reads a populated field, so a cached `System` carries a dead process's cwd, argv and exe across a pid replacement inside one second. The detail pass builds its own `System` for that reason.
- `ProcessIdentity::Unavailable` must not generate fresh evidence per scan: regenerating turns one live process with an unreadable lifetime into a phantom ended row plus a new row on every poll. Disappearance from a scan is what ends the identity.
- `BirthStamp::macos` truncates to whole seconds so it compares equal to `ps -o lstart=`. The publication generation, not the birth stamp, is what separates two invocations born inside one second.
- CPU reads as unavailable for any process whose accumulated counter has not advanced between two 250ms samples. This predates the identity migration and is unchanged by it.
- `birth_stamp/macos.rs` has never compiled on this machine; the two-platform identity claim holds only once the macOS CI job is green.

**Ruled out:** deriving root incarnation from the cleanup `RootContinuity` signal, where a recovered permission or a moved mode bit would discard every retained association; letting `CaptureKey`'s derived `Ord` decide ownership, which lets a filename string pick the live generation among equally confirmed registrations; `Option<RunId>` for enclosing membership, and a per-row capture cache, since no row reopens a log.

### Phase 9 — The registration becomes a row source  · status: todo

#### Work Order

**Goal:** a cargo run the process table cannot describe appears on the grid, labelled by the account that owns it.

**Spec:**

This is the item the feature exists for. Phase 4 already opens and verifies the registration — `registered_runs` (`progress.rs:885`) reads each record against fresh kernel evidence, and `Capture::confirmed()` (`:769`) hands out the verified ones — so the fields the grid needs are in hand. What is missing is a row built from them: a registration can *supply* a row rather than only annotate one, and an unreadable argv costs no columns.

*1. The new direction.* Today the linkage runs one way: `captured_run` (`processes.rs:1410`) walks **upward** from a cargo pid to find a registered ancestor and read its state. That walk stays as it is for every row the process table can already build. What is new is the other direction, for a run the process table cannot describe: the registration names a live shim pid, and the fields the row is missing are already parsed. `Capture::confirmed()` (`progress.rs:769`) is the source — it hands out `ConfirmedCapture` (`:474`), each a verified registration with its file's modification time — and `Capture::read` (`:702`) already returns a named outcome rather than an optional. Build rows from `confirmed()`, taking the direct-association type phase 8 defines rather than reaching for the candidate record.

Three cases still stay apart at the row builder, and phase 4's outcomes already name them: the registration was confirmed, so it may source a row; it parsed but verification came back `Unknown`, or the record is a legacy one, so it may annotate an existing row and nothing more; or it could not be read at all. The middle case is the one that decides whether a row exists, so the row builder reads the outcome rather than testing for emptiness.

| field | source | note |
| --- | --- | --- |
| `path` | registration | the raw absolute directory; the reader shortens it to `~` only when the record's writer-home field matches the scanner's own, and otherwise renders it in full |
| `command` | registration | `cargo <args>`, as typed, one field per argument |
| `pid` | the registration's filename | the shim's pid, a live process |
| `started` | the registration file's **mtime** | see below |
| `cpu`, `compiler`, `managed` | process table where it answers | absent rather than wrong |

Using the registration's mtime as the run start avoids the one call least certain to answer across a uid boundary. The shim writes the file once, at the moment the run begins, and never rewrites it; the reader already stats the entry. Whether sysinfo's macOS backup path carries a usable start time is untested, so nothing here may depend on it.

`ConfirmedCapture` (`progress.rs:474`) already holds that time as `Result<SystemTime, CaptureFailure>`, taken from the same descriptor as the bytes, because verification is allowed to succeed while the timestamp read fails. The fallback row therefore has to survive a failed timestamp: name the unavailable case, decide what the duration column shows for it, and give it a deterministic place in the ordering rather than letting it sort against a missing value. Keep `registration_bytes_and_timestamp_always_come_from_the_same_descriptor` passing unchanged.

*A verified registration and a readable log are separate facts, and the row source depends on only the first.* `scan_root` retains a `ConfirmedCapture` (`progress.rs:611`) before it ever reads that registration's log, while `RootStatus.confirmed` counts only the publications whose logs came back readable — the shipped test `mixed_root_counts_only_confirmed_captures_and_names_each_other_artifact` (`settings.rs:951`) asserts two retained confirmations against a displayed active count of one, and that difference is deliberate. A row sourced here follows the retained proof rather than the count: a verified registration whose log is unreadable still supplies its directory, command, pid and start time, and the root diagnostic naming the unreadable log stays exactly where phase 6 put it.

*2. Precedence is per field, never per row.* Preferring a whole process-table row discards a working directory the registration has, because `row()` (`processes.rs:1851`) degrades a missing cwd to `UNRESOLVED_PATH` (`:1863`) rather than failing. That is the ordinary runner-machine case, not a corner: there argv is readable and cwd is denied, so the row survives with `path: "unavailable"` while the registration holds the real directory. The process row keeps its real pid, its tree and its measured values; an absent cwd is filled from the matching registration; and the merge happens **before** any field becomes display text.

Only the invocation a registration directly represents takes cwd and arguments from it — Phase 8's `InvocationId::Captured` distinction. A nested invocation does not inherit them.

*3. The account has to be visible.* Phase 4 already settled the two premises this item started from: `group_by_path` (`render.rs:1372`) groups on `row.process.directory_identity`, the raw identity rather than display text, and `registration_directory` (`processes.rs:1938`) collapses a directory to `~` only when the record's writer home matches the scanner's own. What is still merged is what a directory alone cannot separate: two accounts, or two roots, holding the same absolute directory. So the remaining work here is the fallback rows, the per-field merge, account and root qualification, the heading prefix, and pin identity — and the existing different-HOME, raw-byte and unavailable-directory regressions stay passing through all of it.

Extend the grouping key to the verified account and the root identity alongside the working directory it already carries. Each of the three separates runs the other two leave merged. Dropping the directory merges every run one account has going in different checkouts into a single heading and a single gauge, which is the more common shape on a build machine than the collision this item started from; dropping the account or the root merges the two accounts back together. The directory in the key is the identity the registration carries, not the `~`-collapsed text the heading shows. Family colour cannot substitute for any of it, since it means process ancestry and is absent for a command with no children. Present the account as a **prefix in the heading**: `[hana-linux-1] /run/github-runner/hana-linux-1/x` in tile and summary headings, absolute wherever the writer's home differs from the scanner's, and `~`-collapsed only where they match. It appears once per heading, where the grouping already lives, rather than spending width on every row. Derive the account name from the root's owner uid, not from the path text, and never let a name that could not be resolved render as an account with no name.

*4. A fallback row is for a run with no process row, never a second view of one that has one.* `Capture::confirmed()` (`progress.rs:769`) hands out proofs from every root, and a pid can be confirmed under a root the process-row association did not select. Build a fallback row **only** where the process table describes the shim pid nowhere. A non-selected proof for a pid that already has a row never becomes a second row and never overwrites that row's own root's metadata — one live invocation is one tile. **Phase 6's root status does not already explain every one of those, and this Work Order must stop assuming it does.** `Census::associate_status` (`processes.rs:1428`) walks the process rows that exist and collects a suppressed proof only where the registration it selected came back **unconfirmed**. Two cases fall outside that, and they are exactly the two this phase creates: a pid confirmed under more than one root, where the selected proof is itself confirmed, and a fallback row with no process-table entry at all. Neither inherits any record of which proof was selected or why the others went unused. Reporting the final selection therefore belongs here — carry it through `CaptureAssociation`, the surface phase 6 already renders, so one explanation reaches the operator whichever way the row was sourced. This phase owes the row-count rule, the reported selection, and the tests for both.

*5. Ownership is direct representation, never shared membership.* Phase 8 made this precise and the row builder has to follow it. `Capture::select` (`progress.rs:713`) returns `CaptureSelection`, and only its selected **and** confirmed key may source a fallback row. `Ambiguous` — two confirmed generations competing for one pid — sources nothing, and neither does a selected key whose reading came back unconfirmed; in neither case may the builder reach past the selection to another root's proof. `Capture::keys` (`progress.rs:753`) is `#[cfg(test)]` and stays that way: fixtures inspect retained readings, ownership goes through `select`.

The suppression rule in item 4 — "the process table describes the shim pid nowhere" — means **direct representation**, not shared capture. `direct_capture` (`processes.rs:1497`) grants ownership only across recorded command-forwarding wrappers; an exec-replaced application is a boundary, and `CaptureMembership::Enclosing` supplies no directory and no argument fields. So a descendant can share an invocation's capture without representing it, and a fallback row is suppressed only where some process row directly represents the same `RunId` and is itself eligible for display. Keep `direct_capture_crosses_only_observed_command_forwarders` (`processes.rs:2469`) and the application-spawned and excluded-parent reader regressions passing unchanged.

*6. Ambiguity keeps its explanation.* Today `captured_run` (`processes.rs:1410`) converts `Ambiguous` into `NearestRegistration::Unregistered`, so `associate_status` (`processes.rs:1428`) records no association at all and the operator sees a row with no progress and no reason for it. Item 4 already owes the reported selection; this is the mechanism that makes it deliverable. Retain a named ambiguity outcome through selection reporting, carrying the competing registration identities, and render it through `settings::capture_root_status` (`settings.rs:437`) — the surface phase 6 already draws. Both shapes get it: an ambiguity over a pid that has a process row, and an ambiguity with no row at all. A later scan in which the competing publication disappears recovers to the ordinary confirmed case and redraws.

*7. A mixed-source tree is one tree.* `Census::groups` and `Census::group` (`processes.rs:1638`, `:1668`) build exclusively from surviving process pids, and `Roster::observe` (`roster.rs:376`) matches whole groups before it matches their children. Appending independent fallback groups afterwards can therefore move an existing child from its process-sourced group into a registration-sourced one and leave its previous row fading, even though the invocation identity never changed. Assembly of registration-sourced parents with process-sourced children belongs here: one tile keeps its focus and family identity across a transition in either direction, `VisibleParent` ancestry stays correct, and no child is drawn twice. The ancestry presentation itself is already shipped and needs no change.

*8. Uncaptured and enclosing rows keep their own provenance.* `CargoProcess` (`processes.rs:264`) also represents invocations with no capture at all, and `Process` identities whose membership is `Enclosing`. Enclosing membership may supply the capture and account qualification the grouping key needs, while the nested row keeps its own working directory and its own command. A row outside every capture group groups on what it has, and neither a root nor an account is invented for it. Give that provenance a **named type** carrying which of the three shapes a row is, rather than optional account and root fields on the row.

**Files:**
- `crates/cargo-tile/src/progress.rs` — the row-sourcing accessor beside `confirmed` (`:689`)
- `crates/cargo-tile/src/processes.rs` — the registration-sourced row, the per-field merge, `row()` (`:1427`), and `registration_directory` (`:1511`)
- `crates/cargo-tile/src/roster.rs` — the literal row fixtures (`:821`), which change with every identity field a row gains
- `crates/cargo-tile/src/render.rs` — grouping on account, root and working directory (`:1372`), pin selection (`:1420`), the heading prefix
- `crates/cargo-tile/src/constants.rs` — the heading prefix's presentation literals
- `crates/cargo-tile/src/settings.rs` — `capture_root_status` (`:437`), where the selection explanation and the ambiguity outcome reach the operator
- `crates/cargo-tile/src/terminal.rs` — **three** `CargoProcess` row literals: the base fixture in `scan_process` (`:753`), the reused-pid replacement (`:782`) and the unavailable update (`:818`), all built from the row schema this phase changes
- `crates/cargo-tile/tests/shim_registration.rs` — root-qualified headings and reader regressions against the production binary

**Seats:** 2 writers + 1 tester — splits by file: the row source and its merge, the renderer's grouping and headings, and a real tester lane on the production-binary harness at `reader_regression` (`tests/shim_registration.rs:811`), which phase 4 proved out. Each writer writes the inline `#[cfg(test)]` tests for the files it owns. Paths below are relative to `crates/cargo-tile`.
- `impl` — opens as impl: `src/processes.rs`, `src/progress.rs`, and `src/roster.rs` including every row fixture — the registration-sourced row, the per-field merge at `row()` (`:1427`) where `path` degrades to `UNRESOLVED_PATH` but `command` fails the whole row, and the row-sourcing accessor beside `confirmed`; hub: `src/processes.rs`, the row builder both writers feed and read. Owns the source-merge tests. Keeps the `roster.rs` fixtures current through every repair round, plus `src/terminal.rs` — its three `CargoProcess` row literals (`:753`, `:782`, `:818`) are built from the row schema this seat changes and cannot be migrated by anyone else
- `test` — opens as impl: `src/render.rs` — `group_by_path` (`:1372`) keys on verified account, root, root incarnation and working directory, headings gain the account prefix, and pin selection (`:1420`) migrates with the key — plus `src/constants.rs` for the prefix literals and `src/settings.rs` for `capture_root_status` (`:437`), which is where the selection explanation and the ambiguity outcome reach the operator; both are this writer's alone in this phase. Writes into the existing `#[cfg(test)]` module (`:2074`). **The row and association schema is settled by the `impl` seat before this writer implements its consumers.**
- `review` — opens as test: `tests/shim_registration.rs` — root-qualified headings and reader regressions driven through the production binary, on the harness `reader_regression` (`:811`); claims no writer file

**Constraints from prior phases:**
- Phase 5 supplies the root-qualified capture key. `Capture::confirmed()` (`progress.rs:769`) returns proofs from **every** root, while the process-row association selects one root per pid, so the two sets can disagree about the same shim pid. **A fallback row exists only for a shim pid the process table cannot describe at all.** A confirmed proof in a root the process row did not select never creates a second row: one live invocation is one tile, and a second row for it is precisely the duplicate phase 5's deduplication exists to prevent. Where the two disagree, phase 6's root status is the surface that explains it.
- Phase 5 verified anchors: `Capture::read` (`progress.rs:702`) **takes a `CaptureKey`, not a pid**; `Capture::confirmed` (`:689`); `ConfirmedCapture` (`:474`), which now carries `key: CaptureKey`; `row()` (`processes.rs:1851`); `registration_directory` (`processes.rs:1938`); `group_by_path` and pin selection stay in `render.rs`.
- Phase 5 added a `CargoProcess` literal to `terminal.rs`; phase 7 split it and changed its schema, and phase 8 added a third: the base fixture in `scan_process` (`:753`), the reused-pid replacement (`:782`) and the unavailable update (`:818`) now carry `cpu: Measurement<String>`, `compiler: CompilerObservation` and `managed: Measurement<usize>` — **not** the `String` / `Option` / `usize` this Work Order previously stated. Any row-schema change here has to migrate all three.
- Phase 4 supplies the parsed record: the raw absolute working directory, the writer's home prefix as its own field, argument count and arguments as separate fields, and the `Confirmed | Ended | Unknown` outcome. Only `Confirmed` may source a row; `Unknown` and legacy records annotate only. The type a row is built from is `VerifiedRegistration` (`registration.rs:207`), reached through phase 8's direct-association wrapper, never the candidate record. The `~` collapse happens here, on the display path, and only when the record's prefix matches the scanner's own — otherwise the heading shows the absolute path rather than a `~` naming a different directory.
- `group_by_path` (`render.rs:1372`) takes `pinned: Option<&str>` and pin selection (`:1420`) chooses by display text, so grouping identity and pin identity migrate together. A pinned directory carries the full grouping identity — account, root and raw directory — not the rendered string; otherwise two distinct directories that render alike pin the wrong group. Give that state a **name** rather than an `Option<GroupingIdentity>`: a cell either heads with a pinned group or it does not, and the second is a state, not a missing value. The scanner's own home has the same shape — convert `dirs::home_dir()` once into a named scanner-home observation instead of threading `Option<&Path>` through `registration_directory` (`processes.rs:1938`) and `home_relative`.
- Phase 8 renames the two `DirectoryIdentity` types for their roles — the working directory a run reported, and inspected filesystem metadata for an open handle. Use those names here; the grouping key takes the first.
- Phase 8 supplies `InvocationId::Captured(RunId) | Process(ProcessIdentity)`, the resolved membership, the scan-scoped capture reads, and the exclusion ordering. A row may not be built for an excluded command.
- **Phase 7 supplies three named measurement types.** `Measurement<T>` is `Reading(T) | Unavailable(MeasurementAbsence)`; `MeasurementAbsence` is `FirstObservation | ReadFailed | Unproven`; `CompilerObservation` is `Unknown | None | Running(Compiler)`, where `None` is an **observed** absence and `Unknown` is no observation at all. A row carries `cpu: Measurement<String>` and `managed: Measurement<usize>`. A registration-sourced row has no process to measure, so it supplies `Unavailable(Unproven)` for both and `CompilerObservation::Unknown` — never a zero, and never a bare `Option`. Each renders as `UNAVAILABLE_MEASUREMENT` (`constants.rs`).
- **Phase 6 supplies a numeric owner, not an account name.** `RootOwner` (`capture_root.rs:67`) is `Uid(u32) | Unavailable`, and settings deliberately renders the number (`settings.rs:589`) rather than looking anything up. The `[hana-linux-1]` heading this phase specifies therefore needs scanner-side account-name resolution that no earlier phase built, and building it is this phase's work rather than an inherited fact. Keep the two values apart: the uid is the identity the grouping key qualifies on, and the name is display text derived from it, and the account-name lookup does not affect grouping identity at all: rows sharing a uid, a root identity and a raw working directory group together, while rows under different roots stay separate whatever the lookup answers. The lookup can also come back with nothing — a uid whose passwd entry was removed with the runner account is the ordinary case, not a rare one — so name that outcome rather than substituting an empty string, and let the heading fall back to the number the settings surface already shows.
- Phase 5 supplies the interned root index that qualifies the identity.
- **The transport for everything above already exists.** Phase 6 carries `RootStatus` on `Scan` into `App`, and `unreadable_capture_recovers_and_redraws_without_any_process_row` (`terminal.rs:888`) is the shipped regression for an unreadable capture recovering and requesting a redraw with no process rows at all. Reuse that path for the selection reporting and the account this phase adds; do not build a second one alongside it.
- **Phase 8 supplies scanner continuity, and it is independent of row eligibility.** `ProcessIdentities` (`processes.rs:132`) persists across scans; its map is replaced from the **complete** census, and two consecutive unavailable lifetime observations reuse a token only while the pid is still present. Observe identities before any row filtering, never after it, or a filtered-out process loses continuity and reappears as a new row. `process_details` (`processes.rs:824`) builds a **fresh** `System` on every scan by design — `UpdateKind::OnlyIfNotSet` never re-reads a field that is already populated, so a reused `System` serves the first scan's metadata forever. CPU continuity keys on `LifetimeEvidence`, never on the unavailable row token. None of this needs a display surface: what the operator observes is stable rows, current metadata, and an unavailable CPU reading where continuity is unproved. Keep the two unavailable-lifetime roster regressions, the fresh-details regression, the replacement-baseline regression and the three smoothing regressions passing unchanged.
- **Phase 8 pins each observed root against inode reuse, and grouping identity has to carry the incarnation.** `RootHistory` (`capture_root.rs:396`) holds an incarnation anchor per observed pathname, keeping the previously observed directory object allocated while device and inode identity are compared. That is independent of ownership, permissions and cleanup eligibility, and it costs one retained descriptor per observed pathname — access failures included — until the pathname's anchor is replaced or the scanner thread exits; the previous anchor drops on replacement. A pinning failure already surfaces as a root-access failure, so no descriptor-specific display is owed. What this phase owes is the qualification: the grouping key and the pin identity take the root **index and incarnation**, not the index alone, so a replaced root never groups with the directory it replaced. Keep `permission_changes_preserve_keys_but_replaced_roots_invalidate_them` (`progress.rs:1424`) passing and add its retained-row grouping counterpart.
- **Two row-building boundaries still carry bare optionals, and they are this phase's to close.** `observed_shim_match` (`processes.rs:1794`) accepts a domain-owned `Option<&Path>` cwd observation: convert an external absence into a named observation at the boundary. `Census::group` (`processes.rs:1668`) returns `Option<CargoGroup>`, which erases three different outcomes — no such process, no identity, and a deliberately excluded command — into one empty answer: retain a named group-construction outcome instead. Both changes land inside this phase's field-merge and eligibility work. Keep `shim_detection_requires_observed_cwds_and_subcommands` passing: a legitimate row survives unavailable metadata, while a deliberately excluded command stays absent.
- **Phase 8's proof, read-deduplication and counter machinery is finished; this phase preserves it rather than extending it.** `RegistrationCandidate`, its production `arguments()` accessor, `DirectCapture` (`processes.rs:635`), `WorkingDirectoryIdentity` and `InspectedDirectoryMetadata` already carry the distinct parsing, ownership, working-directory and filesystem-metadata roles, and none of them needs an external label. The scan-local log-read map is implementation-only and complete: keep the paired read-count tests at `progress.rs:1184` and `:1222` passing, and add no row-level cache and no reopened logs. `CounterState` already reaches the gauge, and its rendering regression at `render.rs:2156` deliberately draws **no** gauge for every non-counter state — preserve that, the blocked row label, and the unreadable-log Settings diagnostics. Nothing in the phase 8 retrospective calls for a different absence gauge.

**Acceptance gate:** Build, Test and Lint green. The complete two-scan source-switch acceptance test lands here, not in phase 8: one run observed from a registration on the first scan and from a process on the second keeps one tile with a stable identity, and an excluded command produces no row from either source. Two directory-identity tests, on phase 4's raw working directory: two distinct absolute directories that collapse to the same display text group as two, and grouping stays stable when a row's source changes. A test asserting the intended lead directory stays first when two groups render identically, across a source transition. A test asserting a record whose writer home prefix differs from the scanner's renders an absolute heading rather than a `~` collapse. A test asserting a registration with no matching process-table row produces a row carrying its working directory and command. A test asserting a process row with an unavailable cwd and a matching registration renders the registration's directory while keeping the process row's pid and measured values. A test asserting two roots reporting the same `~/x` produce two headings, each prefixed with its own account. A test asserting two working directories under one account and one root produce two headings rather than one. A test asserting the same account and the same absolute directory under two different roots produce two headings — the case that isolates the root component from the account and the directory. A test asserting a confirmed registration whose timestamp read failed still produces its row, with a named unavailable time, a defined duration display, and a deterministic place in the ordering. A test asserting an `Unknown` registration sources no row. A test asserting a confirmed registration in a root the process row did not select produces **no** second row for that pid, and that the row it already has keeps its own root's metadata. A test asserting the same pid confirmed under two roots yields one row, and a test asserting an unconfirmed reading in the selected root beside a confirmed proof in another still yields one row carrying no borrowed metadata. A test asserting that single row records which proof was selected and why the other went unused, and the same for a registration-sourced row that has no process-table entry. A test asserting a verified registration whose log is unreadable still sources its row, with the root's log diagnostic preserved and the displayed active count unchanged. A test asserting a root whose owner uid has no resolvable account name still renders its heading, prefixed with the number, and still groups apart from a root owned by a different uid. A test asserting the fallback row's constructor supplies `Unavailable(Unproven)` for CPU and for the managed count and `CompilerObservation::Unknown` for the compiler — no evidence exists for any of the three — and that all three render as `UNAVAILABLE_MEASUREMENT` in the command view and in the summary view alike. A test asserting an ambiguous selection — two confirmed generations for one pid — sources **no** row and no borrowed metadata, and a second asserting the same ambiguity with no process-table entry at all still reports which registrations competed. A test asserting that ambiguity recovers on a later scan once the competing publication is gone, redrawing the ordinary confirmed row. A test asserting a selected but unconfirmed key sources no row. A test asserting a descendant sharing an invocation's capture through `CaptureMembership::Enclosing` does **not** suppress that invocation's fallback row, while a process row that directly represents the same `RunId` does. A test asserting an exec-replaced application is a boundary rather than a forwarder, so it neither suppresses nor inherits. A test asserting a registration-sourced parent with process-sourced children renders one tile that keeps its focus and family identity across a transition in each direction, with correct `VisibleParent` ancestry and no child drawn twice. A test asserting a root replaced under the same pathname does not group with the directory it replaced, keyed on root incarnation rather than root index alone, and its retained rows group accordingly. A test asserting a nested row under enclosing membership takes capture and account qualification from the enclosing invocation while keeping its own working directory and command; a test asserting an uncaptured row groups on what it has, inventing neither a root nor an account; and a test asserting a row whose working directory is unavailable still renders. A test asserting the named group-construction outcome distinguishes no such process, no identity, and a deliberately excluded command.

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

*What `trunk..HEAD` means here, exactly.* The shorthand above names one specific
computation and not the two it could be confused with. The basis is the **net change the
branch would bring to trunk**: the difference between the merge base of trunk and `HEAD`
and `HEAD` itself. It is not a raw commit range replayed path by path, which would list a
path the branch changed and then reverted — a path a merge does not touch, and therefore
outside "the merge it prevents, and nothing else". Nor is it an endpoint tree comparison
against trunk's current tip, which is what `git/paths.rs:28` performs today: that reports
trunk-only changes as differences, so an integrated holder sitting behind an advancing
trunk would keep widening as trunk moved. Measured on this repository while the plan was
written: comparing the parent against the current commit produced **zero unmerged commits
and seven changed paths**, which is exactly the disagreement the basis has to settle. The
existing `git/paths.rs` machinery is reusable, but the anchor it is given is the merge
base, not trunk's tip.

*Run end is checkpoint and release, never merge emptiness.* An emptied merge extent is a
statement about the branch, and it is not a statement that the run is over. A newly claimed
clean branch has an empty merge extent from its first moment while its run holds live
editing scope, so treating emptiness as terminal would drop race protection on a run that
is still working. The authoritative run-end fact stays the one that already exists —
checkpoint and release, the boundary
`an_outstanding_holder_from_another_run_refuses_nothing` (`tests/lifecycle.rs:964`) already
covers — and worktree liveness observes checkout identity, because the hook protocol has no
session-end event to key on. The two facts are computed independently and neither is
derived from the other.

*The derived extent needs its own states, and the reservation's scope type cannot supply
them.* `ReservationScopeSet` (`ledger/journal.rs:943`) refuses empty construction and empty
deserialization by contract, so the derived merge extent cannot reuse it: a successfully
empty extent is the ordinary end state of this phase and has to be representable. Name each
state rather than reaching for an optional: an extent successfully derived and empty, an
extent successfully derived with protected paths, a derivation that could not run with the
last derived set retained as its evidence, and the initial state before any derivation has
run. The last two are distinct and the failure rule above depends on it — a first
derivation that fails has no "last derived set", and replaying an existing journal that
never recorded one lands in the same place. Both must still protect rather than collapse to
empty.

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
- `crates/cargo-berth/src/session/mod.rs` — `apply_journal_event` (`:284`), where session identity mappings actually retire.
- `crates/cargo-berth/src/coordination_identity.rs` — `validate_worktree_occupancy` (`:725`), the separate worktree-occupancy gate, which has to agree with the chosen run-end fact.
- `crates/cargo-berth/src/git/paths.rs` — `:28`, the existing path query, reusable with the merge base as its anchor rather than trunk's tip.
- `crates/cargo-berth/src/drift/git_output.rs` — `WorkingTreeChangePartition`, the existing staged/unstaged/untracked partition.
- `crates/cargo-berth/src/output.rs`, `crates/cargo-berth/src/output_contract.rs`, `crates/cargo-berth/src/constants.rs`, `crates/cargo-berth/src/cli.rs` — how a derived scope, a refused widen and an emptied claim report.
- `crates/cargo-berth/tests/drift.rs`, `crates/cargo-berth/tests/overlap.rs`, `crates/cargo-berth/tests/hooks.rs`, `crates/cargo-berth/tests/lifecycle.rs`, `crates/cargo-berth/tests/ledger.rs`, `crates/cargo-berth/tests/board.rs`, `crates/cargo-berth/tests/edges.rs`, `crates/cargo-berth/tests/output_contract.rs` — the lane.

**Seats:** `1 writer + 1 tester + reserve`. The separation of the two extents is a
single judgment that the refusal grounds, both widen producers, replay and the
board all have to agree on; splitting it puts halves of one decision in different
heads, and the file list is wide only because everything reads the same rule.
`cargo-berth` has a real integration-test lane (Delegation Context → **Test
lanes**), and the rule above is concrete enough to test before the code exists,
so the tester opens as `test`.
- `impl` — opens as `impl`. Owns `crates/cargo-berth/src/reservation/`, `crates/cargo-berth/src/reconcile.rs`, `crates/cargo-berth/src/git/`, `crates/cargo-berth/src/worktree/`, `crates/cargo-berth/src/drift/`, `crates/cargo-berth/src/verb/`, `crates/cargo-berth/src/ledger/`, `crates/cargo-berth/src/board/`, `crates/cargo-berth/src/edge/`, `crates/cargo-berth/src/alert.rs`, `crates/cargo-berth/src/recovery.rs`, `crates/cargo-berth/src/output.rs`, `crates/cargo-berth/src/output_contract.rs`, `crates/cargo-berth/src/constants.rs` `crates/cargo-berth/src/cli.rs`, `crates/cargo-berth/src/session/` and `crates/cargo-berth/src/coordination_identity.rs`; hub: `crates/cargo-berth/src/reservation/record.rs`, which carries both extents and both refusal grounds. The added session and coordination-identity files carry the lifecycle bookkeeping the emptied claim owes, and they answer to the same run-end decision, so they cannot be split off
- `test` — opens as `test`. Owns `crates/cargo-berth/tests/drift.rs`, `crates/cargo-berth/tests/overlap.rs`, `crates/cargo-berth/tests/hooks.rs`, `crates/cargo-berth/tests/lifecycle.rs`, `crates/cargo-berth/tests/ledger.rs`, `crates/cargo-berth/tests/board.rs`, `crates/cargo-berth/tests/edges.rs` and `crates/cargo-berth/tests/output_contract.rs`
- `review` — reserve. Reads the derived extent against the recorded incident, against both widen producers, and checks that the merge extent and the run lifetime stay independent of each other

**Constraints from prior phases:** None from phases 1-9. Phase 8 satisfies none of this
phase's branch-scope derivation work and introduces no sequencing dependency, so this phase
may run before or after them. Two facts about the existing `cargo-berth` code are named
here so the Work Order stays implementable from its files alone:

- **Reuse the existing observation machinery for the dirty half.** `observe_working_tree_status` (`drift/observation.rs:521`) and `WorkingTreeChangePartition` (`drift/git_output.rs`) are private today and already produce the staged, unstaged and untracked partition the union needs. Reuse them for the per-holder status read rather than adding a second observer.
- **Mapping retirement lives in the session module, not in the release verb alone.** `verb/release.rs` publishes the mapping, but `session::apply_journal_event` (`session/mod.rs:284`) is where a mapping actually retires, so a reservation that empties has to reach that path. The separate worktree-occupancy gate `validate_worktree_occupancy` (`coordination_identity.rs:725`) reads its own provenance and must agree with the run-end fact above rather than inferring one from the merge extent.

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

The derivation basis: a test that a holder already integrated into trunk, sitting
behind an advancing trunk, derives an empty merge extent rather than growing as
trunk moves. A test that trunk-only changes on a diverged branch never enter this
reservation's merge extent. A test that a path this branch changed and then
reverted is **not** covered, because a merge does not touch it.

Run end against merge emptiness: a test that a freshly claimed clean branch — an
empty merge extent from its first moment — keeps a live race extent and keeps its
session identity mapping. A test that an ended run releases race protection even
while its branch still carries unmerged commits, so the two facts are shown moving
independently.

The extent's states: a test that a successfully derived empty extent is
representable and distinguishable from a derivation that could not run. A test
that a **first** derivation which fails protects rather than collapsing to empty,
having no previous set to retain. A test that replaying an existing journal which
never recorded a derived set reaches the same protecting state.
