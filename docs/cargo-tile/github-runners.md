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
    Result<CargoArguments, RowAbsence>` `:2037`, which reports an unavailable
    argv as its own outcome — the macOS empty-argv gate. `row(...) ->
    Result<CargoProcess, RowAbsence>` `:1851`: `path` degrades a missing cwd to
    `UNRESOLVED_PATH` via `map_or_else` (`:1862-1863`) while `command:
    command_text(process.cmd(), home)?` (`:1880`) fails the whole row — the
    asymmetry phase 9's per-field merge resolves. `CpuSmoothing` `:885`;
    `Census` `:973` with `take` `:998`, phase 7's collection boundary; `is_shim`
    `:1226`; `drawn_parent` `:1291`, returning `VisibleParent`; `captured_run`
    `:1410`, returning `NearestRegistration`; `annotate_capture` `:1594`, which
    takes ownership through `Capture::select` rather than by pid alone; the
    exclusion retain `names_cargo` `:2066`, whose `.is_ok()` discards the
    `RowAbsence` distinction; `Capture::take()` at `:789`;
    `aggregate_compilers` `:1807` and `aggregate_cpu` `:1837`, phase 7's
    aggregation boundary and in this file rather than in `render.rs`;
    `registration_directory` `:1938`, which collapses a directory to `~` only
    when the record's writer home matches the scanner's own. Inline
    `#[cfg(test)]` block `:2140`.

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

### Phase 9 — The registration becomes a row source  · status: done

#### As-built

- A confirmed registration with no directly representing process row sources its own grid row: the raw directory, the `cargo <args>` command, the shim pid from the filename, and `RunStart` from the registration file's mtime. A failed mtime read is a named unavailable start; it renders the unavailable duration text and sorts deterministically. `RowProvenance` names a row as registration-sourced, process-captured, or uncaptured/enclosing. `DirectAssociation`, read through `Capture::row_source(pid)`, is the direct-representation outcome the builder consumes. Only a selected and confirmed key sources a row; `Ambiguous`, a selected-but-unconfirmed key, and a proof under a non-selected root source nothing. A `CaptureMembership::Enclosing` descendant never suppresses the invocation's row; a process row directly representing the same `RunId` does.
- Precedence is per field. A process row with an unavailable cwd takes its directory from the matching registration before any field becomes display text, keeping its own pid and measured values. Only the directly represented invocation inherits.
- `GroupingIdentity`, `GroupQualification`, `PinnedGroup` and `GroupDirectory` key on the verified account uid, the root index and incarnation, and the raw working directory. Headings carry a `[account]` prefix, or `[uid]` when `AccountName::resolve(RootOwner, &Users)` resolves no name (`CaptureAccount`); the lookup never affects grouping. A directory collapses to `~` only when the writer home equals `ScannerHome`.
- `AssociationSelection`, `SelectedProof`, `UnusedCapture` and `UnusedCaptureReason` carry which publication was selected and why the others went unused, ambiguity included, into `settings::capture_root_status`, naming `<pid>.<generation>` basenames (`REGISTRATION_SEPARATOR`). Ambiguity recovers on a later scan once the competing publication is gone.
- `assemble_groups` merges registration-sourced parents with process-sourced children into one tree that keeps focus and family identity across a source transition in either direction. Every parent with children reports `managed = Reading(count)`, counted transitively along the final parent links; a childless registration row keeps `Unavailable(Unproven)` for cpu and managed and `CompilerObservation::Unknown`. `GroupAbsence` names no such process, no identity, and excluded; `WorkingDirectoryObservation` replaces the optional cwd at the shim-match boundary.
- The shim rewrites argv for a non-terminal JSON capture (`--message-format=json` added, standalone `--quiet`/`-q` removed, both before `--`), so `forwards_json_capture_arguments` accepts that argv as the registered invocation and the heading renders the registered argv. Constants: `CARGO_JSON_FORMAT_PREFIX`, `CARGO_MESSAGE_FORMAT_FLAG`, `CARGO_MESSAGE_FORMAT_JSON_PREFIX`, `CARGO_QUIET_FLAGS`.

**Files:**
- `crates/cargo-tile/src/processes.rs` — row provenance, direct association, fallback rows, per-field merge, JSON-rewrite match, mixed-source assembly and managed counting
- `crates/cargo-tile/src/progress.rs` — `row_source`, `registered_pids`, the selection-reporting types
- `crates/cargo-tile/src/render.rs` — grouping identity, account-prefixed headings, pin identity
- `crates/cargo-tile/src/settings.rs` — selection and ambiguity diagnostics in the root status
- `crates/cargo-tile/src/constants.rs` — heading prefix literals and the JSON-rewrite argument constants
- `crates/cargo-tile/src/roster.rs`, `crates/cargo-tile/src/terminal.rs` — row fixtures on the current schema; the terminal test parser keeps ANSI foregrounds
- `crates/cargo-tile/tests/shim_registration.rs` — production-binary regressions: source switch, fallback rows, quiet-JSON and rejected rewrites, family colour, root diagnostics

**Gotchas:**
- The parent walk in `assemble_groups` advances past an already-visited parent and only suppresses the repeated count; breaking there sends both rows of a cyclic ancestry into each other's family and discards both.
- The reader fixture does not set `NO_COLOR`, and the terminal test parser retains ANSI foregrounds, so family colour is asserted through the binary.
- `reader_keeps_application_spawned_cargo_when_run_is_excluded` is timing-sensitive: about one flake in three package runs, green on rerun.
- macOS and cross-uid behaviour is unverified: every test here runs on Linux under one uid.

**Ruled out:**
- A second row for a pid confirmed under a root the process row did not select: one live invocation is one tile.
- A roster-side workaround for the managed count: the count is set where the tree is assembled.
- sysinfo's process start time as the run start: the registration mtime is the one value the reader already has across a uid boundary, and the macOS backup path is untested.
- Family colour as a grouping component: it means process ancestry and is absent for a command with no children.

### Phase 10 — A claim covers the merge it prevents, and nothing else  · status: done

#### As-built

- A reservation carries two independent extents, and each refusal ground in `is_foreign_to_coordination_run_in_worktree` (`reservation/record.rs`) reads only its own. The **merge extent** is what a foreign checkout is refused: the paths the holder's branch still holds unmerged relative to trunk, derived on every read and never accumulated. Its basis is the diff from the merge base of trunk and `HEAD` to `HEAD` (`git/reachability.rs`, anchored on the merge base so an integrated holder behind an advancing trunk derives empty), unioned with the checkout's staged, unstaged and untracked paths from `observe_working_tree_status`. Once the branch merges, the extent derives empty on its own; there is no retirement verb.
- The **race extent** is what a second coordination run in the same checkout is refused: the paths the holder's run declared or first-touched. It is the only extent a widen grows, from all three producers: the drift widen (`drift/classification.rs`), the first-touch reuse widen (`verb/claim.rs`) and `apply_widen` on replay (`reservation/retention.rs`). It is dropped when the run ends. Run end is checkpoint and release, computed independently of merge emptiness: a freshly claimed clean branch has an empty merge extent and a live race extent from its first moment.
- `reservation/merge_extent.rs` defines `ReservationProtection::{Clear, Protected(scopes)}` and `Reservation::protection_in_worktree(WorktreeId)`. The merge extent has its own type because `ReservationScopeSet` refuses empty construction and successful emptiness is the ordinary end state. Per reservation the wire state is `protected { scopes }`, `empty`, `unavailable { retained_evidence }` or `not_derived { protection }`: an unreadable checkout keeps the last derived evidence and goes on protecting, and a claim that has never derived reports its declared protection.
- Reconciliation (`prepare_reconciliation_transaction` in `reconcile.rs`, under the lock) derives the extent on every `board`, `check` and `drift` read and persists each derivation as the journal operation `merge_extent_observed`, cached on (trunk tip, `HEAD`, working-tree fingerprint). Trunk is in the key because trunk advancing under a fixed, clean `HEAD` is exactly the case that empties the extent. Replay reconstructs the reservation and its race extent without running git.
- Same-checkout holders answer race protection only; foreign checkouts answer merge protection. `AuthorizedEditingIdentity::Unidentified { worktree_id }` carries the caller's checkout so a new run there is not refused its own branch's paths.
- A foreign checkout holding several unreleased claims reports ONE merge conflict carrying the normalized union of every edit-blocking holder's protection, attributed to the oldest contributing holder: `RetainedReservationSet::protection_for_conflict(holder, acting_worktree, path_case)` returns `ConflictProtection::{Clear, Protected { representative, contributors, scopes }}` (`reservation/record.rs`), and `conflicts_with_holders` dedupes per checkout onto the representative. The operator answers with `--override <representative>`; `reservations_authorize_scope` binds the forward answer to the representative id and the union revision, and the reciprocal check consults every contributor's authorizations.
- Git subprocesses drop `GIT_DIR`, `GIT_WORK_TREE`, `GIT_INDEX_FILE`, `GIT_COMMON_DIR` and `GIT_PREFIX` (`git/command.rs`) so a hook in one checkout derives other checkouts against their own `HEAD`. The hook resolves its executable through `CARGO_BERTH_EXECUTABLE`, falling back to `current_exe`.
- A refused widen deletes the cached drift fingerprint, so the next comparison sees the same paths and the refusal surfaces again. Explicit release re-snapshots and stays outstanding while the merge extent is `Protected`. Both hook renderers report `Alert::MergeExtentUnavailable`, and the reconcile alert loop skips released reservations. Successor retention refs survive an emptied merge extent. Merge-extent state is visible in `board --json` under `merge_extent` and in a refused check's `blocked_by`.
- Squash and cherry-pick integrations stay manual: `resolve --integrated-as <TRUNK_OID>` with its existing warning. The specification tests live in `tests/lifecycle.rs::merge_extent`; legacy fixtures carry real dirty or unmerged work wherever an overlap is expected.

**Files:**
- `crates/cargo-berth/src/reservation/merge_extent.rs` — `ReservationProtection`, merge-extent derivation and retained evidence.
- `crates/cargo-berth/src/reservation/{record.rs, retention.rs, partition.rs, lifecycle.rs, mod.rs}` — the two extents and refusal grounds, `ConflictProtection`, `protection_for_conflict`, checkout-union conflicts, answer coverage, race-only `apply_widen`, lifecycle edit-blocking status.
- `crates/cargo-berth/src/reconcile.rs` — derivation on every read, `merge_extent_observed` persistence, alert loop.
- `crates/cargo-berth/src/ledger/{journal.rs, authorization.rs}` — the `merge_extent_observed` operation and its replay.
- `crates/cargo-berth/src/git/{command.rs, reachability.rs, constants.rs, error.rs, mod.rs}` — environment scrubbing, hook executable routing, merge-base path query.
- `crates/cargo-berth/src/drift/{classification.rs, execution.rs, fingerprint.rs, identity.rs, observation.rs, mod.rs}` — race-only drift widen, refused-widen invalidation, merge-ground comparison, status observation reused for the dirty half.
- `crates/cargo-berth/src/verb/{claim.rs, release.rs, check.rs}` — race-only first-touch widen, union conflict reporting and answers, release re-snapshot, check refusal with `blocked_by`.
- `crates/cargo-berth/src/{output.rs, alert.rs, board/alerts.rs, board/rows.rs, coordination_identity.rs, constants.rs, session/mod.rs, edge/graph.rs, worktree/liveness.rs}` — renderers, `MergeExtentUnavailable`, identity carrying the checkout, successor retention refs.
- `crates/cargo-berth/tests/{lifecycle.rs, hooks.rs, overlap.rs, presentation.rs, board.rs, drift.rs, answers.rs, edges.rs, engine_instructions.rs, gate.rs, ledger.rs, liveness.rs}` — specification tests under `lifecycle.rs::merge_extent`, regressions, fixtures with real unmerged work.
- `docs/cargo-berth/generated/output-contract.json` — regenerated by the repository generator.

**Gotchas:**
- A `cargo-berth` binary without the `merge_extent_observed` variant refuses a whole ledger once one such record exists (`journal record N is corrupt: unknown variant`). Every reader of a shared ledger, hooks included, is upgraded before the first new `board`, `check` or `drift` against it. The ledger is never hand-edited to recover an older reader.
- The wire status for successful emptiness is `empty`; `Clear` is the in-process `ReservationProtection` variant.
- `edit_blocking_status()` answers `Clear` from checkpoint integration alone, and that answer is what `evidence_revalidated` persists; the live reservation still blocks through its merge extent, so the effective decision reads the merge extent, not the persisted field.
- Without the `GIT_DIR`/`GIT_INDEX_FILE` scrub, every other checkout derives against the hook's own `HEAD`.
- The two extents stay separate, and neither ground reads the other's as a fallback. One shared set is what let a claim on one crate grow to own a second and keep holding it after its branch merged; nothing widens the merge extent, and a producer that does reintroduces accumulation.
- Waiting successors and unresolved-overlap endpoints do not expose their extents on the board.

**Ruled out:**
- Per-holder merge conflicts for one foreign checkout: N identical conflicts defeat single-answer authorization.
- Scoping the merge extent to the run's own commits: no revision range is an authorship record, and a branch is what merges, so another run's commit on the same branch enters the merge extent and stays out of the race extent.
- Treating an empty merge extent as run end: it drops race protection on a live run.
- An endpoint tree comparison against trunk's tip, or a replayed commit range: the first reports trunk-only changes, the second lists paths the branch changed and then reverted.
- An `Option` for the derived extent: never-derived and failed-first-derivation both protect and are distinct from successful emptiness.
- A separate `tests/merge_extent.rs`: the tests share the lifecycle fixtures.

