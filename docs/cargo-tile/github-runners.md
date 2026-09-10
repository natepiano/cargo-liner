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

  - `crates/cargo-tile/src/progress.rs` — reads the capture root. `Progress`
    `:85`; `Phase` `:115`; `RunState` `:141`; `RunLiveness` `:175` with `From<bool>`
    `:183`; `RegistrationGeneration` `:189` with `names_log` `:199`;
    `LiveRegistrations` `:226` and `RegistrationEvidence` `:235`; `Capture`
    `:263` — the pid-keyed map of readable runs, field `readings` — with
    `take(liveness)` `:274`, `take_from(root, liveness)` `:279`,
    `take_roots(roots, liveness)` `:285` — the single per-scan removal allowance
    across every root — `scan_root` `:300`, `discard` `:367` and `read(pid)`
    `:389`, a map lookup that performs no filesystem access; `root()` `:394` —
    resolves `CARGO_TILE_ROOT` or the default, the single site phase 5 turns
    into a list; `live_runs` `:402` — reads `<root>/state/pids` through the
    access layer and returns `LiveRegistrations { generations, evidence }`, so
    one pid carries every generation registered under it and completeness is
    tracked separately from the live set; `parse_state` `:499`. Inline
    `#[cfg(test)]` module `:636` (57 tests). `tail` no longer exists — reading a
    log goes through `capture_root::ScanEntry::read_log` `:499`, and reading a
    registration through `read_registration` `:507`.

    Two orderings are invariants, not implementation detail, and phase 3 moved
    the first into the access layer. `RootScan::open` samples the log inventory
    **before** it opens or samples registrations, and `Capture::scan_root` reads
    that sample **before** it consults liveness; collecting into a `Vec` is the
    sample, and opening an iterator first does not preserve the boundary.
    Reversing either lets a run that publishes mid-scan lose its log to the
    orphan sweep, after which cargo reopens the log outside the shim's setup
    subshell under the caller's umask — `0066` on the runner units — and the
    operator cannot read it for the life of that run. `RegistrationGeneration::Calendar(String)` is built from any
    nonempty filename suffix, so it classifies a filename and proves nothing
    about process identity; treat it as a candidate, never as verification.

  - `crates/cargo-tile/src/processes.rs` — the two-phase process scan and row
    construction, 79 KB, the largest non-render file. `cargo_split(argv) ->
    Option<(usize, Option<String>)>` `:1211`, which short-circuits at
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
  reach any crate item. Every test of a crate item is therefore an inline
  `#[cfg(test)]` module inside the file it tests: `processes.rs:1298` (47),
  `render.rs:2009` (43), `progress.rs:594` (37), `hook.rs:289` (20),
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

### Phase 4 — Registration identity: verify before it counts, and before it is deleted  · status: todo

#### Work Order

**Goal:** a registration is accepted only when the process now holding its pid is provably the one that wrote it, and no artifact is deleted without fresh evidence its writer ended.

**Spec:**

*1. A live pid does not prove the registration belongs to that run.* Today liveness means only that some process occupies the registered pid. A registration left behind by a SIGKILLed run plus pid reuse would display a finished command as running with no cargo process anywhere. Root ownership does not establish the association; root-qualified keys separate files but not process instances; and mtime supplies a display timestamp rather than proof of identity. The reverse direction fails the same way: a shim that registers *after* the process snapshot reads as absent, and cleanup in an owned root then deletes a live run's registration.

A random token, an inode number and a ctime all prove only that a *file* persisted — none of them associates that file with the process now holding its pid. The evidence that does is the stamp the kernel assigns at process creation, qualified by which boot it belongs to:

- **Linux:** the boot ID from `/proc/sys/kernel/random/boot_id`, plus field 22 of `/proc/<pid>/stat`. That field is readable across the uid boundary. Keep it in **ticks** — do not convert through sysinfo's rounded seconds, which loses the resolution the comparison needs.
- **macOS:** the boot-session identity from `kern.boottime`, plus `KERN_PROC_PID`'s `p_starttime`. sysinfo's macOS fallback initialises `start_time` to zero, so a start-time-versus-mtime comparison cannot carry it there.

*How the macOS stamp is obtained.* Both values are `sysctl`, and **`rustix` does not implement `sysctl`** — the crate lists it as unsupported at `not_implemented.rs:292`, and `rustix::system` carries only `uname`, a Linux-only `sysinfo()`, and hostname setters. `sysinfo` is already ruled out above. So `birth_stamp/macos.rs` calls `libc::sysctl` directly and is the **only** file in this plan that writes `unsafe`, under a per-item `#[allow(unsafe_code, reason = "...")]` with a `// SAFETY:` comment on each block — the escape the workspace manifest's own comment sanctions (`FFI-only; opt back in per item with a reasoned allow`). Do not shell out to `sysctl(8)` or `ps(1)` instead: the reader runs this per registration per scan at up to four scans a second, and a subprocess there is not affordable. `kern.boottime` cannot change within a boot, so read it once and cache it; `KERN_PROC_PID` is per process and read per verification. `birth_stamp/linux.rs` needs none of this — `/proc` reads are ordinary file I/O.

*Units, and the macOS resolution trap.* Phase 2's shim is POSIX `sh` and can only obtain the macOS birth through `ps -o lstart= -p $$`, which has **one-second resolution**, while `p_starttime` is a `timeval` carrying microseconds. Truncate the live reading to whole seconds before comparing. The residual exposure is a pid reused inside the same second as the original registration, which is narrower than the pid-reuse window this evidence exists to close and is the best the shim side can supply. Linux has no such gap: both sides carry ticks.

*The stamp is normalized where it is written, not where it is read.* What phase 2 shipped on Darwin is the raw `ps -o lstart=` text, which Apple renders through `localtime` under the caller's locale, so the same instant serializes differently on two machines and the reader cannot recover the instant from the text. Reading it is not made correct by any parser, so the shim stops emitting it: field 4 becomes **a decimal integer on both platforms** — clock ticks since boot on Linux, whole epoch seconds on macOS — obtained by running both `ps` and the conversion under `LC_ALL=C` (`LC_ALL=C ps -o lstart= -p $$` through `LC_ALL=C date -j -f '%a %b %e %H:%M:%S %Y' ... +%s`), with the whole conversion inside the setup subshell so a failure yields an empty field rather than a partial one. Field 3 still says which boot the number belongs to. A record whose field 4 is anything but a decimal integer is a record written before this normalization: report it `Unknown` rather than reading it as though its format were known. Exercise a writer and a reader under differing locale and timezone settings without altering the environment cargo inherits — the conversion's environment is scoped to the conversion.

Compare the live process's current stamp against fields 3 and 4 of the registration Phase 2 writes. Equal on both → the registration belongs to that process. Different → the pid was reused and the registration is stale. Either field empty, or the observation denied or incomplete → `Unknown`.

*2. Three states, never two.* Model the outcome as `Confirmed | Ended | Unknown`. `Unknown` is never rendered as "running" and never authorises a deletion. A record written before this evidence existed — a legacy registration with no magic, or one whose identity fields are empty — is display-only: it may annotate a row the process table already built, and it may never source an active row.

*3. The deletion rule covers logs too.* Registration enumeration and log enumeration are not one atomic snapshot. A shim can publish its registration and create its log in the interval between them; the reader then meets a log whose pid was absent from the earlier set and deletes a running capture's output. A second race survives directory-relative deletion: after inspecting `state/pids/<name>` the reader can unlink a *replacement* that has since taken that name, and re-checking the inode narrows the interval without closing it. Every artifact — logs included — needs fresh evidence its writer ended before removal, and uncertainty preserves it. Phase 2's exclusive-create publication closes the **overwrite** race and only that one: a name cannot be taken while a record still holds it. It does not close **reuse**. One scan can verify `<pid>.<generation>`, anything can unlink it, a new invocation can publish that same name, and the first scan's delayed unlink then removes a running run's record — a calendar generation makes that collision reachable whenever a pid recurs inside one stamp resolution. So a verified name is not a licence to delete: **delete only what this scan verified and, immediately before the unlink, re-confirmed still carries the identity it verified.** Identity here is the record's own boot-and-birth pair, not the name and not the inode, so a republished record fails the re-confirmation and survives. The same rule covers logs, which carry no identity of their own and are therefore deleted only alongside the registration whose field 5 names them. *Alongside* is a constraint on the sweep's ordering and its allowance, not only on its evidence: `OwnedRoot::sweep` removes every registration before it considers any log and can spend the whole per-scan allowance on registrations, which would strand logs whose only deletion evidence — the registration naming them — has just been removed. Remove the pair together or not at all: the registration is retained until its log is gone, and a pair that does not fit in the remaining allowance is left whole for the next scan. A legacy bare-`<pid>` record gets deferred cleanup rather than an immediate delete, and a snapshot miss is re-verified before any delete.

Re-confirmation narrows the interval between the final read and the unlink; it does not make the two operations atomic, because the name can be taken again inside it. Close the remaining window at the publisher instead of guarding it at the reader: a generation must not repeat, so the shim's `date +%Y%m%d-%H%M%S` suffix — one-second resolution, and therefore repeatable whenever a pid recurs inside the same second — gains a per-invocation distinguishing component that no concurrent or later invocation can reproduce. A name that is never reused makes a delayed unlink harmless by construction rather than by timing. Phase 8 builds invocation identity on this, so the value must be decided here, where the writer changes.

*4. Parse the versioned record.* Read the NUL-framed fields Phase 2 defines, under the registration byte cap from Phase 3. Phase 2's inline fixtures write **empty** registration files, because nothing read them; replace them here with three kinds that exercise the parser — a valid record, a well-formed record whose identity cannot be verified, and a malformed one. Reject a malformed record individually — a bad record must not discard the other registrations and must not disturb the process-table rows that already exist. Records in the old `<cwd>\tcargo <args>` format are display-only text, since their argument boundaries cannot be recovered.

*5. Bind the log to the registration's generation.* Part of this already exists and must be described before it is replaced: phase 2 shipped `RegistrationGeneration` (`progress.rs:189`) and `names_log` (`:199`), which protect every explicitly registered generation and already select the exact filename when a pid carries a single versioned registration. What phase 2 did **not** ship is any reading of the record's contents — the reader still never opens a registration body, so nothing today parses fields or verifies a stale pid. This phase replaces filename inference with verified record contents; it does not introduce generation association from nothing.

A1's identity establishes which process owns a registration but not which log belongs to that registration's run. "Newest filename for this pid" has no correct answer during the window where a new registration exists and its log does not — the reader selects the previous run's log for a reused pid. Take the exact log basename from field 5, validate it is a single filename with no directory part, and open it relative to the inspected root. That removes foreign log enumeration and `newer()` selection for new-format records entirely, and it needs a root-layer API phase 3 did not ship: `ScanEntry` (`capture_root.rs:471`) opens only entries the scan already sampled, so opening a log named by field 5 requires a validated-basename read on `OwnedRoot` that does not depend on that log appearing in the sampled inventory. Dropping the unconditional log sampling in `RootScan::open` (`capture_root.rs:135`) is part of the same change. Phase 9 additionally needs the registration's modification time as a display timestamp, and `read_registration` (`capture_root.rs:507`) returns bytes alone: return an observation carrying the bytes and the metadata read from one opened descriptor, so a record's contents can never be paired with a replacement's timestamp. Root replacement or a generation change discards the association; a temporarily missing log is retried on the next scan while its registration stays valid; directory timestamps never decide whether that named file is reopened, since timestamp resolution is finite and a rename over an existing name can leave them equal.

*6. The record carries the working directory as it is, and the reader shortens it for display.* The shim collapses `$HOME` to `~` before serializing (`cargo-capture-shim.sh:206`), which makes two invocations under different `HOME` values encode two different absolute directories identically — under one account and one root, and phase 9 groups rows by directory identity. Identity is decided where it is known, so the record carries the raw absolute path and the `~` collapse becomes a transform the reader applies when it renders. `tests/shim_registration.rs` asserts the collapsed field today and changes with the writer; that assertion moves to the reader's display path, which is the one shipped phase-2 test expectation this phase alters. A phase 2 record carrying only the collapsed form is directory-identity-unavailable, never a distinct directory.

Shortening is the writer's information, so serializing the raw path alone does not let the reader reproduce it. The shim collapses against the writer's `HOME`; the reader's helper (`processes.rs:1128`) collapses against the scanner's, and an owner uid does not recover a custom `HOME`. Carry the writer's home prefix as its own field beside the raw path. A record that supplies one renders `~/x`; a record that does not, or one whose prefix does not match the scanner's own, renders the absolute path rather than a `~` that would name a different directory. Phase 9's headings depend on this and its table is corrected to match.

*7. Verification produces a type only verification can construct.* This phase exists to make identity provable, and a value any filename can construct is not proof: `RegistrationGeneration::Calendar(String)` (`progress.rs:189`) accepts any nonempty suffix, yet phases 8 and 9 build capture membership and display rows on it. Introduce a verified-registration type whose only constructor is a completed identity check — private fields, fallible construction, no `From<&str>` — carrying the record's parsed fields and its `Confirmed` outcome. `Ended` and `Unknown` produce no such value, which is what stops an unverified registration from reaching a row by construction rather than by discipline. Phase 8 consumes it through explicitly named direct-versus-enclosing capture relationships; phase 9 accepts it for row construction. Give the same treatment to the optional results at the external-process boundary — `CargoProcess.state`, `row()` and `cargo_split()` — where unavailable metadata, deliberate exclusion, absent capture membership and unreadable capture data all arrive today as one `None`. Four outcomes need four names, and an unreadable capture is a status this phase retains rather than collapses, which is what phase 6 renders.

The same collapse begins one level lower, and fixing only the upper three leaves the information already destroyed before they see it: `Capture::read` returns `Option<RunState>` and answers `None` for a log this scan could not open, a log it read that carried no parseable progress, and a pid it never sampled at all. It is the origin of the unreadable-versus-empty ambiguity, so it changes here with the other three rather than being wrapped by them — three outcomes, named, with the read failure carried rather than discarded.

Those three describe a record that exists. Whether a pid has a capture at all is a separate question, and one type cannot answer both without reintroducing the collapse: model the lookup as `Unregistered | Registered(CaptureRead)` and the read as `Progress | NoCurrentProgress | Unreadable`. Phase 6 renders the unreadable case and phase 8 consumes membership, so both consume these rather than re-deriving them. The same treatment applies to the domain optionals this phase already touches: `cargo_split` (`processes.rs:1211`) carries an inner `Option<String>` that distinguishes two argument layouts, and that distinction needs a name of its own. Replacing only the outer results leaves the contract half-applied.

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
- `crates/cargo-tile/src/capture_root.rs` — the validated-basename log read, the registration observation carrying bytes and metadata together, paired removal, and the allowance rule for retained staging files
- `crates/cargo-tile/src/processes.rs` — the liveness call site, `cargo_split`'s argument-layout type, and the display-shortening helper
- `crates/cargo-tile/src/render.rs` — the rendering matches and signatures that change with `CargoProcess.state`
- `crates/cargo-tile/src/roster.rs` — the fixtures built on that state type
- `crates/cargo-tile/src/main.rs` — the `mod` declarations for `birth_stamp` and `registration`

**Seats:** 2 writers + 1 tester — this phase now has a real tester lane, which the original three-writer split did not. The deletion, timestamp and directory-identity work below reaches back into the shipped publisher and the shipped reader together, so those two move as one writer's set rather than two; and phase 2's files under `tests/` drive the shim as a process, which is exactly where publication, compatibility and permission behavior can be exercised independently of the crate's private items.
- `impl` — `crates/cargo-tile/src/birth_stamp/` and its two platform submodules, `crates/cargo-tile/src/registration.rs`, `crates/cargo-tile/src/constants.rs`; hub: `crates/cargo-tile/src/main.rs` **and** `crates/cargo-tile/Cargo.toml` (the macOS-gated `libc` entry). Owns the verification interfaces and both hubs, and lands the `mod` lines for both new top-level modules first so the other writer can compile. This seat owns the only `unsafe` in the plan, so it also owns the `allow` and the `// SAFETY:` comments; a Linux-hosted delegate cannot compile `macos.rs` and must reason it through rather than iterating against the compiler.
- `test` — opens as impl: `crates/cargo-tile/src/cargo-capture-shim.sh`, `crates/cargo-tile/src/progress.rs`, `crates/cargo-tile/src/capture_root.rs`, every affected part of `crates/cargo-tile/src/processes.rs`, `crates/cargo-tile/src/render.rs`, and `crates/cargo-tile/src/roster.rs`, including their inline tests. The publisher and the reader change together here; one writer holding both is what keeps the record format from drifting between them. The state type reaches rendering and the fixtures, so those travel with it rather than being left without an owner.
- `review` — opens as test: `crates/cargo-tile/tests/shim_registration.rs` and `crates/cargo-tile/tests/shim_modes.rs`. Exercises publication, backward compatibility with phase 2 records, and the permission invariants, against the built shim rather than against crate items.

**Constraints from prior phases:**
- Phase 2 defines the record: NUL-terminated fields in the order magic, generation, boot identity, birth stamp, log basename, working directory, argument count, arguments. The magic is the literal `cargo-tile-v2`. Fields 3 and 4 are empty when the shim could not obtain them.
- Phase 2's Linux birth stamp is field 22 of `/proc/$$/stat` in ticks — the shim's own pid, the one the registration is filed under, never `self` inside its setup subshell; the macOS one is `ps -o lstart= -p $$`. The reader must compare against the same units, and against the process named by the registration's filename.
- Phase 3 supplies the directory handles, the byte caps, and `OwnedRoot<'scan>`; all opens and deletions here go through them.
- Phase 3 already added `rustix` to `crates/cargo-tile/Cargo.toml`. This phase adds only the macOS-gated `libc` entry beneath it; do not restate or alter the `rustix` line. rustix covers phase 3's file surface and **not** `sysctl`, which is why this phase reaches past it on macOS alone.
- Phase 3's enumeration accepts both `<pid>.<generation>` and legacy bare `<pid>` filenames.
- `live_runs` (`progress.rs:402`) returns `LiveRegistrations { generations, evidence }`, where `generations` is the `HashMap<u32, HashSet<RegistrationGeneration>>` association and `evidence` records whether every published registration was readable. It already opens and reads registration bodies; what is missing is parsing, verification, and retaining the outcome. Verification replaces the *contents* of that association, not its shape, and must not discard either field.
- `Capture.readings` holds the per-scan capture results (`scan_root:300`, `read:389`, `root:394`, `parse_state:499`, inline tests from `:631`). `tail` no longer exists; reading a log goes through `capture_root::ScanEntry::read_log` (`capture_root.rs:499`).
- `a_disappearing_registration_preserves_sibling_readings_and_disables_cleanup` (`progress.rs:868`) must keep passing: replacing the association's contents may not discard readable siblings, and the completeness evidence stays separate from the live set.
- `RegistrationGeneration::Calendar(String)` (`progress.rs:189`) is constructed from any nonempty filename suffix. It is a candidate, not proof — it carries neither validated calendar syntax nor process identity, which is what this phase exists to add.
- Phase 3 recognizes `.tmp` staging files and counts them against its sweep budget but never deletes one, because age cannot establish that a paused writer ended. Deletion eligibility for a staging file belongs here, decided by the same identity evidence as every other artifact. Forward progress past one belongs here too, and it is the more pressing half: `staging_files_consume_the_shared_budget_and_remain_in_place` (`progress.rs:1362`) shows a full allowance of staging files being inspected and kept, which spends the entire per-scan allowance without removing anything. Repeated scans then repeat that inspection, so an eligible log behind them is never reached and a later owned root never receives any allowance at all. Inspecting a file that is retained is not a removal and must not draw on the removal allowance.
- The liveness answer the sweep consumes comes from a snapshot taken before the scan begins. `processes.rs` refreshes the whole process table once at the top of each scan, then `Capture::take` answers every pid from that one snapshot, so a run starting after the refresh reads as absent and its live registration and log are swept — the failure phase 3's sample-before-liveness order exists to prevent, reintroduced one level up. Only the false-*ended* direction does damage; a false-*running* merely preserves a dead run's artifacts for one more pass. Absence from a snapshot is therefore not by itself deletion evidence: an artifact whose pid the snapshot does not list is re-verified against the live process immediately before its unlink, exactly as item 3 requires of a pid the snapshot does list. The seat holding `processes.rs` owns this alongside the reader.

**Acceptance gate:** Build, Test and Lint green. A test asserting a registration whose birth stamp does not match the live pid's is `Ended`, not `Confirmed`. A test asserting an `Unknown` outcome authorises no deletion. A test asserting a log created between the two enumerations is not swept. A deterministic regression, for registrations and for logs, that verifies a name, removes it, republishes it, and asserts the pending delete does not remove the republished artifact. A test asserting a record whose birth field is not a decimal integer is `Unknown`. A test asserting a record round-trips the raw working directory and that the `~` collapse happens on the display path. A test asserting the verified-registration type cannot be constructed from an `Ended` or `Unknown` outcome. A test asserting a legacy record annotates but never sources a row. A test asserting the comparison truncates to whole seconds on the macOS path and does not on the Linux path, written so it runs on either host. A test asserting a registration republished *after* its final identity re-confirmation and before the unlink survives — the case re-reading identity cannot close, and the one the non-repeating generation does. A test asserting two invocations of the same pid inside one second receive different generations. A test asserting the allowance is exhausted between a registration removal and its log removal leaves the pair intact rather than the log stranded, and one asserting a failed log unlink retains its registration. A test asserting repeated scans over a full allowance of retained staging files still reach an eligible log behind them and still leave allowance for a second owned root, without deleting any staging file. A test asserting a registration and its log published after the process-table refresh but before `RootScan::open` are not deleted, whether fresh verification confirms the writer or returns `Unknown`. A test asserting a named log outside the sampled inventory is opened, and one asserting a record's bytes are never paired with a replacement's timestamp. A test asserting each of the four lookup and read outcomes is distinguishable, including `Finished` while the application is still running. A test asserting a record carrying a writer home prefix that differs from the scanner's renders the absolute path rather than a `~` collapse.

`unsafe` appears in `birth_stamp/macos.rs` and nowhere else, each block under a `reason`-carrying `allow` and a `// SAFETY:` comment.

---

### Phase 5 — One root becomes a list  · status: todo

#### Work Order

**Goal:** the grid reads the capture roots of other accounts on the same machine, configured once and resolved once.

**Spec:**

`root()` (`progress.rs:394`) resolves exactly one root: `CARGO_TILE_ROOT` or the `/tmp` default. It becomes a list — the reader's own root, unchanged, plus any additional roots from configuration:

```toml
[capture]
auto_install = true
roots        = ["/var/lib/hana-ci/hana-linux-1/cargo-tile",
                "/var/lib/hana-ci/hana-linux-2/cargo-tile"]
```

`CaptureConfig` already carries `auto_install` (`config.rs:139-146`); `CommandsConfig::excluded` (`config.rs:71`) is the precedent for a user-supplied list and for snapshotting it once at scanner start. The key is additive and defaults empty, so a machine that sets nothing behaves exactly as it does now.

`CARGO_TILE_ROOT` says where a process **writes**. Nothing in it says where the grid **reads**, which is why discovery belongs in configuration rather than in another environment variable. The reading half needs nothing from the nix modules — no file for them to write or own.

Deserialize `capture.roots` as `Vec<PathBuf>`; resolve, validate absolute, and deduplicate **once at scanner start**, not per scan. Intern the resolved roots so a root index rather than a path goes into keys.

*Root-qualified capture identity.* `Capture.readings` is keyed by bare `u32`. With more than one root, two roots can hold a registration under the same pid number, so the key must carry the root as well. Qualify the capture map on the interned root index plus the pid. Do not attempt the wider identity change here — Phase 8 carries root-qualified identity through the roster, families and tiles; this phase's obligation is that two roots holding the same pid number cannot collide in the capture map.

The interned root table stores identity only — **never** deletion authority. `OwnedRoot<'scan>` from Phase 3 borrows the scan; interning must not give it a longer life.

The sweep budget from Phase 3 is already scan-local and threaded across roots; adding roots must not reset or multiply it. `Capture::take_roots` (`progress.rs:285`) is where that single allowance is established, and `take_from` (`:279`) is its per-root step. Iterating roots means calling `take_roots` once with the resolved list, never calling `take_from` once per root — the latter creates one full allowance per root and undoes the bound entirely.

Resolution must not lose the path the operator wrote. Phase 6 reports a configured root by the pathname from the config file, so canonicalization, deduplication and interning all retain the original string alongside the resolved path.

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
- Phase 3's `RootHistory` is thread-local and preserves observations between scans, which is what lets a root replaced at the same pathname suppress its first sweep. Routing the scanner through a new entry point must keep that history reachable and must keep `replacing_the_root_suppresses_its_first_sweep` and the access-recovery regressions passing unchanged.
- Phase 3 resolves the final path component without following a symlink, and its ownership check is mode-based: owner is the effective user, and neither group nor other may write. On macOS an ACL can grant another account write access without changing those bits, and that residual exposure is documented at `capture_root.rs:333`. Both policies apply unchanged to every root in the list; a configured root does not get a weaker check for being configured.
- Phase 4 supplies acceptance and the log association; both are per root and must not assume a single root.
- The writer and the reader disagree today about an **empty** `CARGO_TILE_ROOT`. The shim (`cargo-capture-shim.sh:53`) treats empty as the default, and phase 2's `RootSelection::Empty` tests assert that; `root()` (`progress.rs:343`) returns an empty path for it. Resolve it one way here: unset **and** empty both fall back to the default, and the resolved root records `default` as its source in both cases. Do not preserve the disagreement into a list.
- The runner roots this key names are `/var/lib/hana-ci/hana-linux-1/cargo-tile` and `/var/lib/hana-ci/hana-linux-2/cargo-tile`, created `0750 <runner> <runner>` by nix tmpfiles, with `CARGO_TILE_ROOT` set in each unit's environment. They are foreign to the reader and will never be swept.

**Acceptance gate:** Build, Test and Lint green. Three cases for the environment variable — unset, set and empty, set and nonempty — asserting the first two resolve to the default with source `default`, and the third to the override with source `environment`. A test asserting a config with no `capture.roots` produces exactly the single root the code resolves today. A test asserting a registration under the same pid number in two roots yields two capture entries. A test asserting root resolution happens once, not per scan. A test asserting a configured root that does not exist is still present in the resolved list, carrying the source it came from, rather than being dropped — phase 6 has nothing to report about a root that was silently discarded. A test asserting the resolved entry retains the pathname as configured, not only its canonical form. A test asserting two roots share one per-scan removal allowance rather than receiving one each.

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
- `crates/cargo-tile/src/capture_root.rs` — owner identity reported separately from cleanup eligibility, with path-qualified refusal reasons in place of one private `Foreign`
- `crates/cargo-tile/src/constants.rs` — the status strings
- `crates/cargo-tile/tests/cli_lifecycle.rs` — **new**: the built-binary lifecycle check, against fixture-only toolchain and root environments

**Seats:** 2 writers + 1 tester — the crate is binary-only and its unit tests are inline `#[cfg(test)]` modules inside the files being edited, so the two writers each write the inline tests for the files they own. The tester lane is real here even so: this phase's CLI gate drives the **built binary** as a process, which needs no crate items, and `tests/shim_registration.rs:39` already demonstrates that shape.
- `impl` — `crates/cargo-tile/src/capture_root.rs`, `crates/cargo-tile/src/progress.rs`, `crates/cargo-tile/src/processes.rs`, `crates/cargo-tile/src/terminal.rs` and `crates/cargo-tile/src/app.rs`, with their inline tests — producing the diagnostics and carrying them to the screen: the split classification, the read outcomes behind it, root status on `Scan`, the `drain_scans` (`:401`) redraw when status changes while the command groups do not, and the retained status `App` holds. The Delegation Context lists `app.rs` as read-only for this plan and this phase is the exception. Hub: `crates/cargo-tile/src/processes.rs` (the `Scan` status type the renderer reads). Producing a diagnostic and transporting it are one change; splitting them across two writers puts the same type on both lines.
- `test` — opens as impl: `crates/cargo-tile/src/settings.rs` — the read-only rows off `rows(app)` (`:98`) — plus `crates/cargo-tile/src/constants.rs` for the status strings, and the rendering tests for both. **This seat must not introduce filesystem access**: `settings.rs` performs none today, and every value it shows comes from what the scan already retained on `app`.
- `review` — opens as test: `crates/cargo-tile/tests/cli_lifecycle.rs`, new, exercising the built binary against isolated toolchains. Owns the `install` / `status` / shim / `uninstall` gate below end to end, and claims no writer file.

**Constraints from prior phases:**
- Phase 5 supplies the resolved, deduplicated, interned root list and the `default` / `environment` / `config` distinction.
- Phase 3 supplies per-scan ownership and enumeration outcomes, including the unavailable-versus-empty live set that `permission denied: state/pids` and `partial` report. It produces **five distinct refusal conditions that today all look identical to an empty root**, and this phase owes each one its own wording:
  - an enumeration that failed or reached its bound (`Enumeration::{Failed, Incomplete}`, `capture_root.rs`) — the root is readable but the list is short, so runs are missing and nothing is ever cleaned up;
  - a registration inventory that was incomplete even though directory enumeration succeeded (`RegistrationEvidence::Incomplete`, `progress.rs`) — every run displays, and cleanup silently stops;
  - a root genuinely owned by another account — readable, read-only, and correct; there is nothing for the operator to fix;
  - a root the operator *can* fix, whose mode bits grant group or other write — indistinguishable from the previous case today, and the only one worth acting on;
  - an effective-user read that failed. `effective_user()` (`capture_root.rs:677`) caches its first result in a `OnceLock` **including the failure**, so one failed read at startup disables cleanup for the entire session with no retry. This one is new as of phase 3's repair round and currently has no surface anywhere — it is the condition most likely to be reported as "it just stopped tidying up".

  `RootScan::access` (`capture_root.rs:206`) collapses all five into one private `RootAccess::Foreign`, so that value cannot become the displayed classification and this phase cannot render the list above without changing it. Split the two questions the name currently conflates: who owns the root, and whether this scan may delete in it. Report the reason path-qualified — the failing directory named, not the root alone — so `permission denied: state/pids` says which directory. That makes `capture_root.rs` and its inline tests this phase's to edit.

*An unreadable capture is a status, not an absence.* Reading a log and `Capture::read()`
erase every read failure into `None`, so a log the reader cannot open and a healthy capture with
nothing to report arrive here as the same value. This phase owns the operator surface for that:
name the affected root and log, say the access failed, and keep a registration that cannot be
verified distinct from a root that is genuinely empty. It is the visible face of the `0600` failure
the capture ordering exists to prevent — when it does happen the operator sees nothing at all and is
left to find it with `cat`.

This requires the earlier phases to stop discarding the outcome: the hardened access layer and the
identity verification both retain read failures and carry them to the scan rather than collapsing
them, and the invocation-identity phase consumes those retained outcomes when it adds its
once-per-scan capture cache instead of re-deriving them. Their Work Orders carry that requirement.

**Acceptance gate:** Build, Test and Lint green. A test asserting `settings::rows` performs no filesystem access. A test asserting each of the five refusal conditions above — a short or failed enumeration, an incomplete registration inventory behind a complete directory enumeration, a genuinely foreign read-only root, a root whose mode bits the operator can fix, and an unavailable effective user — renders wording distinct from one another and from a root that is genuinely empty. A test asserting the effective-user wording says cleanup is disabled for the session and that restarting is what retries it, since nothing else will. A test asserting a permission failure names the directory that failed, not only the root. A test asserting a root whose status changes while the command groups do not still requests a redraw. A test asserting a configured root that is missing at first and appears later changes its displayed status, with the command groups unchanged across both scans. A test asserting an empty `CARGO_TILE_ROOT` displays the default as its source, not the environment. A test asserting a log the reader cannot open displays an access failure rather than an empty root, and that it returns to a normal status once readable, requesting a redraw on that transition with the command groups unchanged. A test asserting a registration that cannot be verified is distinct from a root with nothing in it. A test per status string.

An end-to-end CLI check also lands here, and it is the one phase 2 deferred: run the built binary against a fixture-only `RUSTUP_HOME`, `HOME` and `CARGO_TILE_ROOT`, and assert `install`, `status`, a shim execution, and `uninstall` restoring what was moved aside. `rustup_home()` (`hook.rs:323`) honours `RUSTUP_HOME` and the existing hook tests already build isolated toolchains, so this needs no second machine and must never touch a real toolchain. Phase 2's two files under `tests/` drive the shim as a process and do not cover this.

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

*Availability has to come from somewhere, and today it is destroyed before the types could carry it.* `Census::take` (`processes.rs:607`) receives a plain `f32`. On macOS sysinfo initialises a process's CPU usage to zero (`sysinfo-0.39.6/src/unix/apple/macos/process.rs:57`) and its task-info reader discards the syscall result (`:336`), so a failed measurement and a genuinely idle process both arrive as `0.0` and no type introduced downstream can separate them. Establish the distinction where the sample is collected, using evidence already available and without reaching for a platform syscall: `Process::accumulated_cpu_time()` is total CPU consumed, and a process with a nonzero `run_time()` that reports **zero accumulated CPU time** has not been measured rather than been idle. Treat that pairing as unavailable; a zero rate against nonzero accumulated time is a measured zero and renders as one. A process observed for the first time is unavailable too — a rate needs two samples — and that is a separate reason from a failed read, so name both. This keeps the plan's rule that `birth_stamp/macos.rs` is the only file writing `unsafe`; no additional probing is introduced.

An `Option` at the row is not sufficient on its own. The group total sums the members it can read and silently skips the ones it cannot, so a group holding an unknown contributor reports a figure lower than reality and indistinguishable from a measured one. **An unknown contributor makes the group total unknown.** Smoothing must stop publishing its previous value once a sample goes unavailable, rather than holding the last reading indefinitely.

`CpuSmoothing::taken` (`processes.rs:518`) is itself one of the optionals this phase exists to remove: it distinguishes a smoother that has never published from one holding a previous reading, and those are different states with different consequences once a sample can go unavailable. Give it a name rather than an `Option`.

Reach: `Census::take` (`:607`), `CpuSmoothing` (`:508`), `Roster::assign_families` (the `managed > 0` test), and three call sites in `render.rs`.

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

**Constraints from prior phases:**
- None binding from the capture work — this phase touches no root, registration, or shim code, and can run independently of phases 4 through 6.
- The plan permits `unsafe` in `birth_stamp/macos.rs` alone, under a per-item reasoned `allow`. This phase adds none: the availability distinction above is drawn from sysinfo values the crate already collects.

**Acceptance gate:** Build, Test and Lint green. A test asserting an unavailable CPU sample renders as unavailable rather than `0%`. A test asserting a group total containing an unknown contributor is unknown. A test asserting `CpuSmoothing` stops publishing a stale value. Three tests at the collection boundary, distinguishing a first observation, a failed measurement, and a measured zero from one another. A test asserting a reading that becomes unavailable before the next smoothing publication is published as unavailable rather than as the retained previous value. A test asserting a never-published smoother is distinguishable from one holding a previous reading.

---

### Phase 8 — Invocation identity and capture membership  · status: todo

#### Work Order

**Goal:** every invocation carries a stable identity that survives a change of source between scans, and each capture is read once per scan rather than once per row.

**Spec:**

*1. Identity must reach past `Capture.logs`.* Qualifying only the log map leaves `read(pid)` ambiguous, and group identity, roster matching, family colours and tile identity all still key on a bare pid. There is a second problem underneath: a registration row uses the **shim** pid while its preferred process row uses the **cargo** pid, so a run that switches source between scans makes the roster retire one tile and open another.

Carry a stable captured-invocation identity of root, shim pid and run generation through capture lookup, deduplication, roster and tiles, with the **displayed** pid kept separate from the identity.

*A generation is a filename, not a lifetime.* `RegistrationGeneration::Calendar(String)` (`progress.rs:189`) is arbitrary suffix text produced by a second-resolution stamp in the shim (`cargo-capture-shim.sh:150`), so `(root, shim pid, generation)` can repeat across two genuinely different processes whenever a pid recurs inside one second. Phase 4 makes that suffix non-repeating and supplies the boot-qualified birth stamp; qualify `RunId` and `ProcessIdentity` by that verified process lifetime rather than by the filename, and define root replacement as invalidating every retained association keyed on it. `CpuSmoothing`'s history (`processes.rs:508`) is keyed on the bare pid and belongs in the same migration — otherwise a reused pid inherits the previous invocation's smoothed CPU.

*2. Identity is not membership.* One capture legitimately supplies progress to several invocations — the shim skips nested captures on purpose — so giving every associated row the same run identity merges distinct commands into one. Model it as `InvocationId::Captured(RunId) | Process(ProcessIdentity)`. Only the invocation a registration **directly** represents takes the working directory and arguments from it; a nested invocation keeps its own identity and merely references the enclosing `RunId`. That reference is itself a membership question with two named answers, not an `Option<RunId>`: a row either sits inside an enclosing capture or does not, and the second is not a missing value.

`drawn_parent` (`processes.rs:807`) has the same shape today — an optional pid standing for both "no visible parent" and "the parent is not drawn" — and gains a named visible-parent type in this phase alongside the identity it keys on.

One `RunId` inside `Captured` cannot carry both of those relationships: the same value would mean "this registration describes me" for one row and "I ran underneath that registration" for another, and every consumer would then re-derive which it is holding. Give the direct case its own type — a registration that Phase 4 verified, holding the working directory and arguments it is allowed to supply — and let membership in an enclosing capture be a separate field that carries only the `RunId` it is nested under. Row creation in Phase 9 takes the verified-registration type and nothing else, so a nested invocation cannot reach the fields it must not inherit, and no row-building path needs a runtime check to keep the two apart.

*3. `is_shim` accepts absence as evidence.* `is_shim` (`processes.rs:752`) compares `subcommand(outer.cmd()) == subcommand(inner.cmd()) && outer.cwd() == inner.cwd()`. When both sides are `None` the cwd test passes, so two unavailable fields count as a match — and cwd is exactly the field the runner machine denies. Require observed equality, not matching absence.

*4. Membership is resolved before progress is parsed.* `captured_run` (`processes.rs:918`) establishes association only when parsing yields a `RunState`, so a valid registration whose log has printed nothing yet reads as no association at all. Resolve membership independently of progress parsing.

*5. Consume the scan's capture results; do not build a second cache.* This item asked for a once-per-scan read because every row was believed to reopen its log. **Phase 3 already shipped that.** `Capture::scan_root` (`progress.rs:300`) reads each selected log during the scan and stores the result; `Capture::read` (`:389`) is a map lookup documented as performing no later filesystem access, and `captured_run` (`processes.rs:918`) walks parents over that lookup alone. `a_capture_keeps_its_reading_without_reopening_a_replaced_log` (`progress.rs:1016`) holds it in place. There is no per-row I/O left to remove and no new cache to add here.

What remains is consumption. Phase 4 names the lookup and read outcomes — `Unregistered | Registered(CaptureRead)` over `Progress | NoCurrentProgress | Unreadable`. Rows built here read those outcomes and must not re-derive them or collapse them back into an absence. `NoCurrentProgress` is named for what it means rather than for when it happens: `parse_state` (`progress.rs:499`) yields nothing for an empty log, for ordinary output carrying no counter, and — the one that misleads — for a run that has printed `Finished` while its `cargo run` application is still alive and still the row's subject. "Nothing printed yet" describes only the first. A capture that has no process-table row at all keeps its unreadable status visible, because phase 6 reports it before phase 9's fallback rows exist.

*6. Order within the scan.* Verify registrations against Phase 4's evidence, prune, establish direct association, resolve each invocation's command, and only then decide row eligibility.

*7. Exclusion applies to one set only.* `commands.excluded` prunes the process row at `processes.rs:440`, but the shim's hardcoded skip list covers only `metadata`/`tile`/`port`/`berth` and friends — a user-configured `excluded = ["clippy"]` still gets a registration. "No process-table row" therefore includes rows removed **on purpose**, and Phase 9's fallback would resurrect them, breaking an existing configuration contract for local captures as well as CI ones. Apply exclusion to both sources before fallback selection and attribution, keep deliberate exclusion distinct from unavailable metadata, and apply it **only** to the row-owning and attribution set — never to `Census.parents` and never to the capture liveness set, or an excluded live capture becomes a sweep candidate and loses artifacts it is still writing.

**Files:**
- `crates/cargo-tile/src/processes.rs` — `InvocationId`, `is_shim` (`:752`), `captured_run` (`:918`), the exclusion retain at `:440`, the liveness callback at `:462`, `drawn_parent` (`:807`), and `CpuSmoothing`'s pid-keyed history (`:508`)
- `crates/cargo-tile/src/progress.rs` — capture lookup keyed by the new identity (`read:389`, `root:394`, `live_runs:402`)
- `crates/cargo-tile/src/roster.rs` — matching (`:194`) and the family maps, both keyed on the pid today
- `crates/cargo-tile/src/tiles.rs` — the demand and content identifiers, likewise
- `crates/cargo-tile/src/render.rs` — the renderer entry point that still takes a pid
- `crates/cargo-tile/src/constants.rs` — any literal the identity or the scan cache needs

  `roster.rs`, `tiles.rs` and `render.rs` are listed read-only in the Delegation Context and in this Work Order's earlier constraint. **That does not hold for this phase**: the identity these files key on is exactly what the phase replaces, so a change that stops at `processes.rs` and `progress.rs` leaves PID-based identity live in the consumers and the phase does not achieve its goal.

**Seats:** 2 writers + 0 testers + reserve — with the capture cache removed from this phase, the identity change and the lookup it keys travel together, and the consumers form the second set. `cargo-tile` is a binary-only crate, so it has no linkable integration-test target and its tests are inline `#[cfg(test)]` modules inside the files being edited (Delegation Context → **Test lanes**). Each writer writes the inline tests for the files it owns; neither writer's lane can open as `test`.
- `impl` — `crates/cargo-tile/src/processes.rs` — `InvocationId`, `is_shim` (`:752`), `captured_run` (`:918`), the exclusion retain (`:440`), the liveness callback (`:462`), scan-scoped capture reads; plus `crates/cargo-tile/src/constants.rs`; hub: `crates/cargo-tile/src/processes.rs` (`InvocationId` is defined here and read by all three files). Land the type first — the other two cannot compile without it
- `test` — opens as impl: `crates/cargo-tile/src/roster.rs` (matching at `:194` and the family maps), `crates/cargo-tile/src/tiles.rs` (the demand and content identifiers) and `crates/cargo-tile/src/render.rs` (the renderer entry point that takes a pid), including their consumer tests
- `review` — reserve. Reviews identity lifetime and source transitions without claiming either writer's files. The `is_shim` defect — two unavailable cwds comparing equal — is the highest-value single test in the phase and belongs with the `processes.rs` writer

**Constraints from prior phases:**
- Phase 5 supplies the interned root index and the root-qualified capture map key; the identity here extends that key rather than replacing it.
- Phase 4 supplies verification and the `Confirmed | Ended | Unknown` outcome; membership resolution consumes it and must not re-derive it.
- Phase 7 supplies the CPU measurement, the managed-process count, and the three-state compiler observation as **named semantic types**, each distinguishing an unavailable measurement from a measured zero. Rows built here use those types directly and must not reintroduce a bare `Option` for any of the three; phase 7's Work Order names them, and this phase uses those names rather than restating them as "optional".

**Acceptance gate:** Build, Test and Lint green. Unit coverage that the identity survives a source transition — the same invocation identified from a registration and from a process resolves to one identity. The complete two-scan acceptance test for the source switch itself belongs to phase 9, which is where registration-sourced rows first exist; do not write a version of it here that needs a row builder this phase does not have. A test asserting the lookup and read outcomes phase 4 names stay distinct where rows consume them, including a log that has printed `Finished` while its application is still running. A test asserting two invocations reusing one pid neither share a tile nor inherit each other's smoothed CPU. A test asserting a capture with no process-table row keeps its unreadable status visible. A test asserting an excluded command produces no row but is still counted live for sweep purposes.

---

### Phase 9 — The registration becomes a row source  · status: todo

#### Work Order

**Goal:** a cargo run the process table cannot describe appears on the grid, labelled by the account that owns it.

**Spec:**

This is the item the feature exists for. The shim already writes the field that is missing and the grid discards it: `live_runs` (`progress.rs:356`) parses only the **filename** as a pid and never opens the file. So a registration can *supply* a row rather than only annotate one, and an unreadable argv costs no columns.

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
- `crates/cargo-tile/src/render.rs` — grouping on account, root and working directory (`:1347`), pin selection (`:1391`), the heading prefix
- `crates/cargo-tile/src/constants.rs` — the heading prefix's presentation literals

**Seats:** 2 writers + 0 testers + reserve — phases 4 and 8 already expose verified registration data, so the accessor work that once looked like a third lane is now a few lines tightly bound to row construction and belongs with its owner. `cargo-tile` is a binary-only crate, so it has no linkable integration-test target and its tests are inline `#[cfg(test)]` modules inside the files being edited (Delegation Context → **Test lanes**). Each writer writes the inline tests for the file it owns; no seat here can open as `test`.
- `impl` — `crates/cargo-tile/src/processes.rs` **and `crates/cargo-tile/src/progress.rs`** — the registration-sourced row, the per-field merge at `row()` (`:1069-1083`) where `path` degrades to `UNRESOLVED_PATH` but `command` fails the whole row, and the registration accessor beside `read`; hub: `crates/cargo-tile/src/processes.rs` (the row builder both writers feed and read). Also maintains the literal row fixtures in `crates/cargo-tile/src/roster.rs` (`:527`), which have to change whenever a row gains an identity field. Owns the source-merge tests.
- `test` — opens as impl: `crates/cargo-tile/src/render.rs` — `group_by_path` (`:1347`) keys on verified account, root and working directory instead of display text, and headings gain the account prefix; plus `crates/cargo-tile/src/constants.rs` for the prefix's presentation literals, which are this writer's alone in this phase; writes into the existing `#[cfg(test)]` module (`:2009`)
- `review` — reserve. Reviews source transitions and exclusion behaviour without taking a share of either writer's files.

**Constraints from prior phases:**
- Phase 4 supplies the parsed record: the raw absolute working directory, the writer's home prefix as its own field, argument count and arguments as separate fields, and the `Confirmed | Ended | Unknown` outcome. Only `Confirmed` may source a row; `Unknown` and legacy records annotate only. The `~` collapse happens here, on the display path, and only when the record's prefix matches the scanner's own — otherwise the heading shows the absolute path rather than a `~` naming a different directory.
- `group_by_path` (`render.rs:1347`) takes `pinned: Option<&str>` and pin selection (`:1391`) chooses by display text, so grouping identity and pin identity must migrate together. A pinned directory carries the full grouping identity — account, root and raw directory — not the rendered string; otherwise two distinct directories that render alike pin the wrong group.
- Phase 8 supplies `InvocationId::Captured(RunId) | Process(ProcessIdentity)`, the resolved membership, the scan-scoped capture reads, and the exclusion ordering. A row may not be built for an excluded command.
- Phase 7 supplies the CPU measurement, the managed-process count, and the three-state compiler observation as **named semantic types**. A registration-sourced row leaves all three unavailable rather than zero, and says so through those types rather than through a bare `Option`; use phase 7's names, not "optional".
- Phase 6 supplies the owner account per root, which the heading prefix and the grouping key both use.
- Phase 5 supplies the interned root index that qualifies the identity.

**Acceptance gate:** Build, Test and Lint green. The complete two-scan source-switch acceptance test lands here, not in phase 8: one run observed from a registration on the first scan and from a process on the second keeps one tile with a stable identity, and an excluded command produces no row from either source. Two directory-identity tests, on phase 4's raw working directory: two distinct absolute directories that collapse to the same display text group as two, and grouping stays stable when a row's source changes. A test asserting the intended lead directory stays first when two groups render identically, across a source transition. A test asserting a record whose writer home prefix differs from the scanner's renders an absolute heading rather than a `~` collapse. A test asserting a registration with no matching process-table row produces a row carrying its working directory and command. A test asserting a process row with an unavailable cwd and a matching registration renders the registration's directory while keeping the process row's pid and measured values. A test asserting two roots reporting the same `~/x` produce two headings, each prefixed with its own account. A test asserting two working directories under one account and one root produce two headings rather than one. A test asserting an `Unknown` registration sources no row.

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
