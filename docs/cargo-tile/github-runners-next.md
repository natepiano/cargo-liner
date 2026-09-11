# cargo-tile and the local GitHub Actions runners — Next

> **Status: IMPLEMENTATION PLAN — phased, delegate-ready.**

## Delegation Context

- **Project:** `cargo-liner` workspace (`resolver = "3"`, members `crates/*`). Three members are touched:
  - `cargo-tile` (`crates/cargo-tile`, v0.2.78-dev) — binary-only terminal grid of every live cargo invocation on the machine; owns the capture shim, the shared capture directory reader, registration parsing, birth stamps, the account install/uninstall/status hooks, and the settings pane.
  - `cargo-berth` (`crates/cargo-berth`, v0.1.0-dev) — git-worktree reservation engine; owns the ledger journal, reconciliation, reservation lifecycle/records, board reporting, and the JSON output contract.
  - `tui_pane` (`crates/tui_pane`, v0.8.0-dev) — reusable ratatui pane framework consumed by both binaries; owns the settings overlay's row-to-rendered-line map.
- **Project started:** 2026-09-11T09:46:02-04:00

- **Stack:** Rust, edition 2024 (`workspace.package.edition`). `ratatui` 0.30.2 + `crossterm` 0.29.0 for the TUI, `tui_pane` (path dep, `backdrop` feature in `cargo-tile`) for panes/overlays, `rustix` 1.1.4 (`fs`, `process`) and `libc` 0.2.189 (unix-only) for the account, group, and kernel calls, `sysinfo` 0.39.6 for the process table, `uuid` 1 (v7), `serde`/`serde_json`/`schemars` for the cargo-berth contract, `tempfile` for test fixtures. The capture shim is POSIX `sh` embedded in the binary as `SHIM_SOURCE`. Workspace lints deny `unsafe_code`, `missing_docs`, and clippy `all`/`cargo`/`nursery`/`pedantic` plus per-rule denies (`unwrap_used`, `expect_used`, `panic`, `unreachable`, `self_named_module_files`, `undocumented_unsafe_blocks`, `allow_attributes_without_reason`). Toolchain notes: tests run with `cargo nextest`, never plain `cargo test`; formatting is `cargo +nightly fmt` only, because stable rustfmt ignores the nightly options in `rustfmt.toml`. Integration tests in `cargo-tile` drive the built binary under a PTY with embedded Python 3.

- **Layout:**
  ```
  Cargo.toml                                   workspace manifest, members = crates/*
  rustfmt.toml                                 nightly-only options
  crates/cargo-tile/
    Cargo.toml
    src/
      cargo-capture-shim.sh                    POSIX sh shim, embedded as SHIM_SOURCE
      cli.rs  hook.rs  registration.rs
      capture.rs  capture_root.rs
      processes.rs  progress.rs
      render.rs  settings.rs  constants.rs
      birth_stamp/{mod.rs, linux.rs, macos.rs}
    tests/
      shim_registration.rs                     #[path] modules over src/ + PTY + Python
      shim_modes.rs  cli_lifecycle.rs
      support/shared_capture.rs                #[path] module of shim_registration only
  crates/cargo-berth/
    Cargo.toml
    src/
      reconcile.rs  output.rs
      gate/install.rs
      ledger/journal.rs
      reservation/{lifecycle.rs, record.rs}
      board/{report.rs, rows.rs}
      verb/{board.rs, release.rs}
    tests/                                     board.rs hooks.rs lifecycle.rs output_contract.rs overlap.rs + 9 more
  crates/tui_pane/
    Cargo.toml
    src/overlays/settings.rs
    tests/                                     framework_bar.rs framework_toasts.rs macro_use.rs
  docs/cargo-berth/generated/output-contract.json   checked-in JSON schema contract
  docs/cargo-tile/as-built/github-runners.md        as-built these items follow from
  ```

- **Key files:** (line refs verified against HEAD `3fb313b0`; the `cargo-tile` `cli.rs` and `hook.rs` refs re-verified against the tree Phase 4 left; drift noted inline)
  - `crates/cargo-tile/src/cli.rs` — command entry points. `RequestedAction` `:82` with its `From<Cli>` `:94`; `Cli::run` `:124`; `report()` `:159` turns a returned `io::Result` into the exit status; `install()` `:171`; `account_hook_report()` `:190` (the child path, writing and flushing one protocol row per toolchain); `all_accounts()` `:200`; `uninstall()` `:238`; `status()` `:255`; `print_local_report` `:269`.
  - `crates/cargo-tile/src/hook.rs` — shim installation, account discovery, credential switching. Phase 4 made every one of these operation-neutral, so the install-specific names earlier phases used are gone. `HookAccount` `:74`, `HookOperation` `:87` with `completion` `:108`, `AccountHookOutcome` `:125`, `ToolchainHookOutcome` `:136`, `ToolchainHookReport` `:188` with `parse` `:197` and `protocol` `:247`, `AccountHookReport` `:273` with `incomplete` `:286` and `from_output` `:296`, `Hook` `:359`, `Startup` `:378`, `at_startup` `:410`, `HookState` `:449`, `HookOperationOutcome` `:466`, `HookDiscovery` `:484`, `Hook::reports` `:500` (an iterator — callers print each row before the next toolchain starts), `Hook::all` `:529`, `Hook::in_rustup_home` `:532`, `Hook::at` `:558`, `Hook::state` `:588` (the only unlocked inspection), `Hook::perform` `:606`, `Hook::install` `:630`, `Hook::ensure` `:660`, `Hook::remove` `:683`, `HookInstallationLock` `:699`, `StagedExecutable` `:738`, `stage_executable` `:756`, `system_accounts` `:783`, `account_groups` `:830`, the private `account_groups_with` `:860`, `run_account_hooks` `:905`, `run_account_hooks_with` `:921`, `run_account_hook` `:944`, `bounded_account_groups` `:983` (macOS refusal), `account_credentials` `:1009`, `inspect_shim` `:1061`. Also holds `SHIM_SOURCE`, `SHIM_MARKER`, and three `unsafe_code` allowances at `:780`, `:827`, `:1006`.
  - `crates/cargo-tile/src/cargo-capture-shim.sh` — POSIX `sh` capture shim. `capture_parent=/tmp/cargo-tile` `:53` (the fixed shared location), `case $first in` `:80` (subcommand classification; `install` takes the capture path), publication framing `printf '%s\000' cargo-tile-v2 …` `:298`.
  - `crates/cargo-tile/src/capture_root.rs` — root access layer, 1023 non-test lines, anchor type `RootScan`. `CleanupRefusal` `:195`, `RootScan::cleanup_refusals` `:422`, private `RootScan::access` `:471`, `RootScan::revalidate_paths` `:487`, `InspectedDirectoryMetadata::refusals` `:637`, `InspectedDirectory::inspect` `:679`, `OwnedRoot::sweep` `:923`. One `allow(unsafe_code)` at `:1395`.
  - `crates/cargo-tile/src/settings.rs` — settings-pane rendering, no filesystem access. `push_value` `:385`, `capture_root_status` `:451`, `capture_diagnostic` `:623`, `cleanup_refusal` `:666`.
  - `crates/cargo-tile/src/render.rs` — grid and popup drawing. `summary_rows` `:678`, `heading_gauge` `:1894`, `draw_settings` `:2128`. Also holds `GroupingIdentity`.
  - `crates/cargo-tile/src/processes.rs` — 2848 non-test lines, 44 top-level types. `Measurement<T>` `:221`, `CaptureDiagnostic` `:806`, `Census::attribute_cpu` `:1459`, `Census::groups` `:1996` with the per-row CPU overwrite at `:2032`, `Census::group` `:2182`, `aggregate_cpu` `:2401`.
  - `crates/cargo-tile/src/progress.rs` — 1217 non-test lines, 23 top-level types. `registered_runs` `:939`, `parse_state` `:1086`, `last_counter` `:1120`. Also holds `CaptureRoots::from_parent`.
  - `crates/cargo-tile/src/registration.rs` — `cargo-tile-v2` NUL framing. `Registration::parse` `:31`, `ParseError` `:229`.
  - `crates/cargo-tile/src/birth_stamp/mod.rs` — `BirthStamp::compare` `:87`, `ProcessLifetime` `:106`, `ProcessLifetime::macos` `:116`, `LifetimeEvidence` `:134`, `IdentityEvidence` `:166`, `KernelObservation` `:197`, `Verification` `:222`, `observe` `:232`, `observe_process` `:279`.
  - `crates/cargo-tile/src/birth_stamp/macos.rs` — Darwin `observe` `:48`, reaching methods private to the parent module; `allow(unsafe_code)` sysctl at `:118`. Does not compile on Linux.
  - `crates/cargo-tile/src/constants.rs` — every constant with its rationale; carries `cfg(target_os)` gates. New ACL and framing-version constants go here.
  - `crates/tui_pane/src/overlays/settings.rs` — row-to-rendered-line map for mouse selection. `line_targets` field `:107`, `set_line_targets` `:220`, `row_at` `:238`, `line_for_selection` `:249`. **Drift:** the plan cites `:238` for both `row_at` and `line_for_selection`; `:238` is `row_at`, `line_for_selection` is at `:249`.
  - `crates/cargo-berth/src/gate/install.rs` — `EXECUTABLE_ENVIRONMENT: &str = "CARGO_BERTH_EXECUTABLE"` `:32`, the executable override readers resolve through.
  - `crates/cargo-berth/src/ledger/journal.rs` — `JournalOperation::MergeExtentObserved` `:362` (the record older binaries reject), `JournalOperation::EvidenceRevalidated` `:422` (carries `status: IntegrationEvidenceStatus` and `edit_blocking_status: EditBlockingStatus`).
  - `crates/cargo-berth/src/reconcile.rs` — `prepare_reconciliation_transaction` `:969`, `derive_merge_extents` `:1039`, `append_evidence_and_retention` `:1899`. The evidence operation is constructed before `derive_merge_extents` runs.
  - `crates/cargo-berth/src/reservation/lifecycle.rs` — `IntegrationEvidenceStatus::edit_blocking_status` `:167`, the checkpoint-only derivation.
  - `crates/cargo-berth/src/reservation/record.rs` — `Reservation::edit_blocking_status` `:355`, the decision live blocking checks use.
  - `crates/cargo-berth/src/verb/release.rs` — `outstanding_operation` `:451`.
  - `crates/cargo-berth/src/board/report.rs` — reservation-snapshot report structure (no line cited).
  - `crates/cargo-berth/src/board/rows.rs` — board placement; keeps waiting successors and unresolved-overlap endpoints out of snapshot sections (no line cited).
  - `crates/cargo-berth/src/verb/board.rs` — `board --reservation <id> --json` path (no line cited).
  - `crates/cargo-berth/src/output.rs` — output selector enums feeding the schema (no line cited).
  - `crates/cargo-berth/src/output_contract.rs` — generates the checked-in contract; regenerate by setting `CARGO_BERTH_REGENERATE_OUTPUT_CONTRACT` and running its inline test (`:189`–`:205`).
  - `docs/cargo-berth/generated/output-contract.json` — checked-in JSON schema, 551 KB. Asserted by `crates/cargo-berth/tests/output_contract.rs:12` and `crates/cargo-berth/tests/overlap.rs:770` via `include_str!`; any board-report field change must be regenerated here.
  - `crates/cargo-tile/tests/shim_registration.rs` — PTY + embedded Python integration binary. `#[path = "../src/…"]` module declarations `:3`–`:59`; `#[path = "support/shared_capture.rs"]` `:63` with `mod shared_capture;` `:68`; `def wait_for(predicate, description, diagnostics=None)` `:239`; `def read_terminal` `:412`; `def terminal_snapshot` `:426`; `def fixture_panes(rendered, markers)` `:499` — the one definition of a matching command pane; `def fixture_pane` `:503` asserts `fixture must occupy one command pane` at `:505` on a screen already known ready; `def wait_for_fixture_pane(markers)` `:508` — polls within the `wait_for` deadline, one PTY drain and one snapshot per poll, until exactly one command pane holds every marker, returns that screen, and on expiry reports the matching-pane count with the final screen; `def assert_fixture_pane_readiness` `:663` — deterministic delayed and never-ready cases under fake time (scenarios `pane-readiness-delayed`, `pane-readiness-never`); `def reader_has_scanned` `:904`; `reader_pane_readiness_waits_for_all_fixture_markers_in_one_pane` `:1667`; `reader_keeps_application_spawned_cargo_when_run_is_excluded` `:1833`. No `CARGO_TILE_ROOT` reference remains under `tests/`.
  - `crates/cargo-tile/tests/support/shared_capture.rs` — cross-account scenarios. **Verified: it is not a test binary of its own.** It is a `#[path]`-included module of exactly one integration test binary, `shim_registration` (`shim_registration.rs:63`), and is included nowhere else, so any gate over it names `<int_test>` = `shim_registration`. `resolved_account_groups_include_the_callers_primary_gid` `:196` (Phase 3 made it return `io::Result<()>` and compare the whole sorted list against `id -G <caller>`), `injected_account_reports_child_errors_and_continues_other_toolchains` `:321`.
  - `crates/cargo-tile/tests/shim_modes.rs` — shim terminal/no-terminal modes (Rust only; no environment scrub of `CARGO_TILE_ROOT`).
  - `crates/cargo-berth/tests/hooks.rs` — hook protocol fixtures; the expected SessionStart and PostToolUse response contents the reader-upgrade inventory compares against (no line cited).
  - `crates/cargo-berth/tests/board.rs` — board reporting assertions (no line cited).
  - `crates/cargo-berth/tests/lifecycle.rs` — reconciliation and release lifecycle assertions (no line cited).

- **Test lanes:**
  - `cargo-tile` — `crates/cargo-tile/tests/`. Integration binaries: `shim_registration` (embedded Python 3 harness driving the built binary under a PTY; `#[path]`-includes all of `src/` plus `support/shared_capture.rs`), `shim_modes` (Rust only), `cli_lifecycle` (Rust only). `support/shared_capture.rs` is **not** its own binary. The crate is binary-only, so unit tests of crate items are inline `#[cfg(test)]` modules in `src/`.
  - `cargo-berth` — `crates/cargo-berth/tests/`. Integration binaries: `answers`, `board`, `drift`, `edges`, `engine_instructions`, `front_end_corpus`, `gate`, `hooks`, `ledger`, `lifecycle`, `liveness`, `output_contract`, `overlap`, `presentation`. Fixture data in `tests/fixtures/front_end_corpus.json`. All Rust; no Python harness.
  - `tui_pane` — `crates/tui_pane/tests/`. Integration binaries: `framework_bar`, `framework_toasts`, `macro_use`. All Rust.

- **Build:**
  - `cargo-tile` — `bash ~/.claude/scripts/delegate/verify.sh check cargo-tile`
  - `cargo-berth` — `bash ~/.claude/scripts/delegate/verify.sh check cargo-berth`
  - `tui_pane` — `bash ~/.claude/scripts/delegate/verify.sh check tui_pane`

- **Test:**
  - `cargo-tile` — `bash ~/.claude/scripts/delegate/verify.sh test cargo-tile`
  - `cargo-berth` — `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth`
  - `tui_pane` — `bash ~/.claude/scripts/delegate/verify.sh test tui_pane`

- **Lint:**
  - `cargo-tile` — `bash ~/.claude/scripts/delegate/verify.sh lint cargo-tile`
  - `cargo-berth` — `bash ~/.claude/scripts/delegate/verify.sh lint cargo-berth`
  - `tui_pane` — `bash ~/.claude/scripts/delegate/verify.sh lint tui_pane`

- **Style:** `run-end /clippy style-only auto-proceed`

- **Invariants:**
  - **Shim behavior.** The shim never alters what cargo does, prints, or exits with; every setup failure degrades to `exec "$real" "$@"`. It stays POSIX `sh` — no bashisms, no `readlink -f`, no `local`.
  - **Fixed capture location.** The shared capture parent is hard-coded at `/tmp/cargo-tile` (`cargo-capture-shim.sh:53`), mode `1777`, with one `0755` per-uid child `/tmp/cargo-tile/<uid>`. `CARGO_TILE_ROOT` is removed and read by nothing; the `[capture] roots` config key is parsed and ignored.
  - **Publication order.** The shim publishes the registration (via `ln` onto `<pid>.<generation>`, never `mv -f`) before it creates the log; the reader samples registrations when it opens the root and collects the sample before consulting liveness. Reversing either end reintroduces the `0600` unreadable-log defect.
  - **Registration framing.** Records are NUL-framed with the `cargo-tile-v2` magic; generation and log fields must be single basenames and the argument count must match. `Registration::parse` yields `Versioned(RegistrationCandidate)` or `Legacy`; a legacy record may annotate a row and never sources one.
  - **Identity proof.** `RegistrationCandidate::verify_observation(pid, &KernelObservation)` is the only route to `VerifiedRegistration`. Production `KernelObservation` values come only from `birth_stamp::observe(pid)` and stay pid-bound with private fields; arbitrary observation injection exists only under `cfg(test)`. `Verification::Unknown` authorizes neither a row nor a deletion.
  - **Who may unlink a registration.** Only the reader's own uid directory is swept (`CaptureCleanup::Here`, `effective_user()` equal to the directory's uid). A foreign-owned directory is reported and never read. Another account's dead captures are reaped by that account's next shim invocation.
  - **Ownership prerequisite for sweeping.** `RootScan::access` returns `RootAccess::Owned` only when `cleanup_refusals()` is empty: root, `state`, and `pids` owned by the effective user with neither `WGRP` nor `WOTH` set, the effective uid readable, both enumerations `Enumeration::Complete`, continuity holding, and `revalidate_paths` matching reopened handles against the held ones. `access` stays private. A sweep removes `<pid>.<generation>` and the log the record names as a pair, decided by fresh kernel evidence immediately before the unlink, never by age or name alone.
  - **Directory opening.** Every directory is opened `O_NOFOLLOW` relative to a held handle. Ancestor symlinks are resolved once by `canonical_capture_path`; the final component never is.
  - **Admin install protocol.** `install --all-accounts` requires root, reads the account database with `getpwent`, and stages a root-owned `0755` copy of the running executable under the sticky `/tmp` parent. Since Phase 4 the same protocol carries `install`, `uninstall`, and `status`: the child runs once per account with `HOME`/`RUSTUP_HOME` set and the hidden `--account-hook-report` flag (`constants.rs:762`), printing one `<toolchain>\t<result>` line per toolchain and flushing each row before the next toolchain starts, so a terminated child still reports what it finished. Credentials are applied together in `pre_exec`: `setgroups` first, then `setgid`, then `setuid`. No root-owned file operation reaches an account's tree. `account_credentials` (`hook.rs:1009`) resolves the account's full membership through `account_groups` (`:830`), which grows its `getgrouplist` buffer geometrically from 32 entries to a 65,536-entry ceiling: glibc reports the required total through the count pointer, Darwin can leave the supplied count unchanged, and both cases resolve. Beyond the ceiling, a failure the count cannot size, or a negative count, it returns an error naming the account rather than a truncated list. On macOS `bounded_account_groups` (`:983`) then refuses a resolved membership larger than the runtime `setgroups` limit, naming the account, the resolved count, the primary gid, and the limit; it never shortens the list, because the kernel treats a supplementary-group list as the account's whole membership rather than a cache. Later accounts are still processed. `Hook::install` and `Hook::remove` each take `HookInstallationLock` (exclusive creation of `cargo-tile-shim.lock`, removed on drop) and inspect state under it; `Hook::state` is the only unlocked read. `Hook::remove` answers `Ok(HookOperationOutcome::Orphaned)` for a shim whose real cargo is gone — a rendered row and an incomplete removal, not an error.
  - **Exit-status asymmetry, deliberate.** `install` reports a failing toolchain, continues, and exits **zero**, because runner job-start hooks call it and must not fail over capture setup. `uninstall` exits **nonzero** on a removal failure and must keep doing so, or automation reads a partial uninstall as complete.
  - **Settings pane.** Does no filesystem access; everything it shows was observed on the scanner worker.
  - **Platform.** `birth_stamp/macos.rs` and the Darwin directory enumeration do not compile on Linux; Linux CI compiles but cannot exercise the macOS ACL or sysctl paths. `unsafe_code` is denied workspace-wide with per-item `allow(reason)` plus a `// SAFETY:` comment at `capture_root.rs:1395`, `hook.rs:780/827/1006`, and `birth_stamp/macos.rs:118`. The crate does compile and test natively on macOS, and the workflow carries a `macos-latest` job (`.github/workflows/ci.yml:348`), so a macOS-only target has a native route as well as CI. **The native `cargo-tile` package suite is not green on macOS today:** three `shim_registration` cases — `fifo_removal_failure_preserves_original_cargo_and_unowned_directory`, `publication_links_a_complete_tmp_file_into_place`, and `stale_invocation_fifo_still_publishes_registration_and_captures_stderr` — fail on the current tree and equally on a `git archive` of the Phase 2 checkpoint (1449 passed / 3 failed on each). They predate Phase 3, no phase owns them, and they are recorded as a next item; any phase whose gate names a green native suite depends on that item first.
  - **Constants.** Every constant lives in `crates/cargo-tile/src/constants.rs` with a rationale. No magic values inline.
  - **Test layout.** `cargo-tile` is binary-only: crate-item tests are inline `#[cfg(test)]` modules, and `tests/` reaches sources through `#[path]` modules and drives the built binary. Two uids are never required by a test. Any file move must carry the `#[path]` declarations at the top of `shim_registration.rs` with it.
  - **Verification gates are per package.** Use the `verify.sh` lines above, never raw cargo, never workspace-wide breadth in a phase gate. Tests run under `cargo nextest`; formatting is `cargo +nightly fmt` only.
  - **Style rules that bind these phases.** One type cluster per file and submodules named for their anchor type (`~/rust/nate_style/rust/split-by-type-ownership.md`, `name-submodules-after-anchor-types.md`); a flat file splits when two or more of the criteria in `when-to-split-a-module.md` hold, with ~500 non-test lines as the line-count criterion; a `mod.rs` is a table of contents (`module-roots-as-table-of-contents.md`); the split replaces the flat file rather than leaving a `#[path]` shell. Forbidden words at `~/rust/nate_style/rust/forbidden-words.md` apply to code, comments, identifiers, and commits.
  - **Type design.** No `Option<T>` in domain-owned types or APIs where a semantic type states what presence and absence mean; convert at the foreign-API boundary (`~/.claude/docs/type_design.md`). A three-state outcome (established / refused / inspection failed) is a named enum, not a `Result<bool, _>`.
  - **cargo-berth ledger compatibility.** Reconciliation appends `merge_extent_observed` on the first `board`, `check`, or `drift` against a ledger, and every older binary then rejects the whole ledger. Historical journal records stay readable and unchanged. Any board-report field addition must be regenerated into `docs/cargo-berth/generated/output-contract.json`.

## Phases

### Phase 1 — The reader harness waits for the complete command pane, and the tests stop scrubbing `CARGO_TILE_ROOT`  · status: done

#### As-built

- The Python harness in `shim_registration.rs` defines `fixture_panes(rendered, markers)` as the one definition of a command pane holding every marker; `fixture_pane` asserts through it and is for a screen already known ready.
- `wait_for_fixture_pane(markers)` polls within the `wait_for` deadline (10s, 0.02s retry), draining the PTY for 0.1s and reconstructing one snapshot per poll, until exactly one command pane holds every marker, and returns that screen. On expiry the assertion message carries the matching-pane count and the final screen, supplied through the optional `diagnostics` callable on `wait_for`.
- The nested, excluded, exec-nested, and exec-excluded reader scenarios wait with it and read the returned screen for their row, pid, exclusion, cleanup, and capture-pulse assertions. `reader_has_scanned` keeps its staging-specific `live[1].name` marker and screen-level summary condition.
- `assert_fixture_pane_readiness()` drives two deterministic scenarios under fake time, `pane-readiness-delayed` (summary-only, split, and duplicate panes rejected before one complete pane is accepted) and `pane-readiness-never` (final counts of zero and two, final screen reported, one snapshot per drain), from the Rust tests `reader_pane_readiness_waits_for_all_fixture_markers_in_one_pane` and `reader_pane_readiness_reports_the_final_screen_on_timeout`.
- No test references `CARGO_TILE_ROOT`; the shim never reads it.

**Files:**
- `crates/cargo-tile/tests/shim_registration.rs` — the readiness predicate, its deterministic cases, and the two readiness test entry points
- `crates/cargo-tile/tests/shim_modes.rs` — the shim-mode fixture; its environment carries no `CARGO_TILE_ROOT` scrub

**Binds later work:** New harness scenarios that wait for a nested fixture (the registration framing v3 phase and the cargo install progress phase) call `wait_for_fixture_pane(markers)` and assert on the screen it returns; `fixture_pane` is for a screen already known ready; no test scrubs `CARGO_TILE_ROOT`.

**Gotchas:** Fixture writers live about 20s (1000 × 0.02s sleeps); the two 10s waits plus the 1s settle read already reach about 21s at their limits, so a new wait replaces an existing one and never joins the chain. The exec-excluded scenario keys on `first[1].name`, never `enclosing[1].name`; the summary label is a separate pane and stays a screen-level condition.

**Ruled out:** A real-time never-ready test (about 100s across ten invocations), replaced by fake time; extending the nested wait's deadline, which would outlive the fixture writers.

### Phase 2 — `uninstall` and the admin install report every toolchain by name  · status: done

#### As-built

`hook.rs` defines `ToolchainInstallOutcome` (`Installed`, `Refreshed`, `AlreadyInstalled`, `Orphaned`, `Failed(String)`) and `ToolchainInstallReport { toolchain, outcome }`, whose `TryFrom<&str>` decodes the child's `name\t<state>` and `name\terror\t<message>` lines and whose `Display` renders them back. `AccountInstallReport` keeps `toolchains: Vec<ToolchainInstallReport>` beside its `AccountInstallOutcome`; `AccountInstallReport::from_output(account, &Output)` is the only decoder (`impl From<Output> for AccountInstallOutcome` is gone) and retains the valid rows past a malformed line and past an unsuccessful child exit. Any orphan, failure, malformed line, or child failure summarizes the account as `Skipped` with joined reasons; an empty report as `Skipped("no toolchains")`; any install or refresh as `Installed`; else `AlreadyInstalled`. `install_account` returns the report and `install_all_accounts` (`cli.rs`) prints it through `Display`: an account line, then one indented line per named toolchain, totals unchanged. `uninstall` attempts every discovered toolchain, prints each failure as `cargo-tile: <toolchain>: <error>` to stderr, then returns an error naming the count, so the nonzero exit follows a complete attempt; `Hook::remove`'s orphan error carries no toolchain name, the reporting layer supplies it.

**Files:**
- `crates/cargo-tile/src/hook.rs` — the report types, the decoder, install and removal.
- `crates/cargo-tile/src/cli.rs` — the administrative renderer and the uninstall loop; `install_account_report` and `describe_account_install` emit the child protocol.
- `crates/cargo-tile/tests/support/shared_capture.rs` — regressions over the child protocol and the administrative report.

**Binds later work:** Account results come from `AccountInstallReport.toolchains` and its `Display`, not a per-account `AccountInstallOutcome`; the decoder keeps rows past a malformed line and past an unsuccessful child exit; `install` exits zero on a failing toolchain (job-start hooks invoke it) while `uninstall` exits nonzero, deliberately.

**Gotchas:** `install_all_accounts` in `cli.rs`, not anything in `hook.rs`, is the human renderer and the only production match on `AccountInstallOutcome` outside `hook.rs` — a per-toolchain outcome that never reaches it is invisible to an operator. The child emits per-toolchain errors on stdout with a successful exit; only a discovery failure uses stderr and an unsuccessful exit.

**Ruled out:** `impl From<Output> for AccountInstallOutcome` beside the new decoder — it dropped every row after a malformed line and on an unsuccessful child exit. Summarizing an account from the first orphan alone — a fixture requires a later toolchain error to survive in the skip reason.

### Phase 3 — `account_groups` proves its resize path  · status: done

#### As-built

`account_groups(name: &str, primary_gid: u32) -> io::Result<Vec<u32>>` (`crates/cargo-tile/src/hook.rs:614`) keeps its signature and its macOS element conversions — `libc::c_int::try_from(primary_gid)` at entry, `u32::try_from(group)` at exit. The `getgrouplist` call sits behind a private generic `account_groups_with<Group: Copy>(primary_gid, resolve)` (`:644`) that takes the group-list call as a parameter, so the resize path runs under unit tests on either platform without touching the real account database. The resize is a bounded loop, not a two-call sequence: the count is reset to the buffer length before every call; a reported count larger than the buffer resizes to exactly that; a zero, smaller, or unchanged count multiplies the buffer by `ACCOUNT_GROUPS_GROWTH_FACTOR`; a negative count returns `io::ErrorKind::InvalidData` with no further call; repeated overflow at `ACCOUNT_GROUPS_MAX_CAPACITY`, or a required size above it, returns an error naming the ceiling. Seven inline `#[cfg(test)]` cases in `hook.rs` cover the glibc reply, the Darwin reply, growth when no larger size arrives, the ceiling, a required size above the ceiling, and the negative count. `resolved_account_groups_include_the_callers_primary_gid` (`crates/cargo-tile/tests/support/shared_capture.rs:196`) returns `io::Result<()>`, keeps its primary-gid assertion, and compares the whole sorted result for the calling account against `id -G <caller>`.

**Files:**
- `crates/cargo-tile/src/hook.rs` — `account_groups`, the private `account_groups_with` generic, and the seven inline cases
- `crates/cargo-tile/src/constants.rs` — `ACCOUNT_GROUPS_INITIAL_CAPACITY = 32`, `ACCOUNT_GROUPS_GROWTH_FACTOR = 2`, `ACCOUNT_GROUPS_MAX_CAPACITY = 65_536` (65,536 entries caps each libc group buffer at 256 KiB), each with its rationale
- `crates/cargo-tile/tests/support/shared_capture.rs` — the full-list comparison against `id -G`

**Binds later work:** The ceiling error names the account and the 65,536-entry limit and reaches an operator through `account_credentials` (`hook.rs:740`) → `install_account` (`:705`) → the account report's `Display` (`:231`), so the administrative multi-account operations own proving such an account is classified incomplete with its reason retained. Darwin's `setgroups` limit is 16 supplementary entries: resolution succeeding for a larger account does not mean credentials can be applied to it. The crate compiles and tests natively on macOS — copy the tree to the Mac and run the package suite — which is the check route for any macOS-behavior work, subject to the pre-existing failures below. `account_groups` may call the resolver more than twice; its signature, result, and error kinds are unchanged and no consumer counts calls.

**Gotchas:**
- The two libcs do not report the required count the same way: glibc writes the required total through the count pointer, Darwin can leave the supplied count at the buffer capacity. A strategy that trusts the reported count alone errors on Darwin instead of growing, which is why the loop resets the count before every call and multiplies when the reply supplies no larger size.
- Three `shim_registration` cases fail on macOS and fail equally on the preceding checkpoint — `fifo_removal_failure_preserves_original_cargo_and_unowned_directory`, `publication_links_a_complete_tmp_file_into_place`, `stale_invocation_fifo_still_publishes_registration_and_captures_stderr`. They concern the capture shim's argument forwarding and FIFO publication on Darwin, not group resolution, so any gate naming a green native macOS suite is blocked on them. Linux runs 1454/1454; macOS 1449/1452.
- A macOS account above 32 groups has never been installed end to end: it needs an administrator to create the account and a privileged multi-account install.

**Ruled out:**
- A `cfg(target_os)` branch for the two count contracts — one loop satisfies both libcs, so the platform difference the code carries stays the group element type alone.
- An unbounded retry — a libc that keeps failing without ever supplying a usable size would spin, so the loop stops at `ACCOUNT_GROUPS_MAX_CAPACITY` with an error naming it.

### Phase 4 — `uninstall --all-accounts` and `status --all-accounts`  · status: done

#### As-built

- `HookOperation` (`Install`, `Uninstall`, `Status`) drives one shared account protocol; the hidden child flag is `--account-hook-report` (`ACCOUNT_HOOK_REPORT_FLAG`), and `run_account_hooks` / `run_account_hooks_with` / `run_account_hook` spawn the per-account children for every operation. The containers are operation-neutral — `HookAccount`, `ToolchainHookReport` (`parse`, `protocol`), `AccountHookReport` (`incomplete`, `from_output`), `AccountHookOutcome`, `ToolchainHookOutcome`, `HookOperationOutcome` — and `HookOperation::completion` reduces a slice of account reports into the command's exit. Clap's optional subcommand stays inside the argument adapter and converts immediately into `RequestedAction` with an explicit `Grid` case.
- `Hook::at` answers `HookDiscovery` — `Discovered`, `AbsentCargo`, `InspectionFailed { path, error }` — in place of `Option<Hook>`, so the path and the error survive. `Hook::reports(operation) -> io::Result<impl Iterator<Item = ToolchainHookReport>>` yields each toolchain as it completes: the three local paths consume it through `.inspect(print_local_report)`, the account child through `account_hook_report`, which writes and flushes one protocol row before the next toolchain starts, so a command stopped part-way still shows the work that finished.
- `Hook::state` is the only unlocked inspection; `Hook::install` and `Hook::remove` acquire `HookInstallationLock` and classify state under it, so a mutation contending with another installer waits and reports a lock failure naming the lock path instead of a stale state. `Hook::remove` answers `Ok(HookOperationOutcome::Orphaned)` for a shim whose real cargo is gone — a rendered row and an incomplete removal, never an error.
- macOS refuses an over-limit account by name rather than shortening its membership: `bounded_account_groups(primary_gid, groups, limit) -> io::Result<Vec<u32>>` returns `InvalidInput` naming the resolved count, the primary gid, and the runtime `setgroups` limit, before the child is spawned; that account is reported incomplete and later accounts still run. An accepted list keeps every resolved group, the primary gid included, and a failed or non-positive limit leaves membership intact. The credential callback is `impl FnMut(&mut Command, &HookAccount) -> io::Result<()>`; `AccountGroupNote` and `CredentialGroups` do not exist.

**Files:**
- `crates/cargo-tile/src/hook.rs` — the operation-neutral protocol, discovery, locking, and the Darwin refusal
- `crates/cargo-tile/src/cli.rs` — `RequestedAction`, the three local paths, the account child path, `print_local_report`
- `crates/cargo-tile/src/constants.rs` — `ACCOUNT_HOOK_REPORT_FLAG` alongside the three group-buffer constants
- `crates/cargo-tile/tests/cli_lifecycle.rs`, `crates/cargo-tile/tests/support/shared_capture.rs` — lifecycle and shared-capture coverage, including the child-termination and refusal-injection cases
- `crates/cargo-tile/README.md` — the macOS group-limit section, which describes refusal, why a list is never shortened, and reducing membership as the recovery

**Binds later work:** `Hook::reports` is lazy — a new caller must consume it, printing or forwarding each item before pulling the next, or an interrupted command reports nothing for completed work. A mutating operation acquires `HookInstallationLock` before inspecting state and adds no unlocked precheck. An over-limit Darwin account is refused, never shortened, and `bounded_account_groups` is the single place that decision is made. `status --all-accounts` reports from the shim marker and the real cargo's presence; it reads no framing version and compares no bytes. The Darwin refusal has unit and integration coverage but has never been observed against a real account above the limit, which needs root and such an account.

**Gotchas:**
- The local install path ends `.inspect(print_local_report).count()` and genuinely iterates only because a `filter_map` iterator is not `ExactSizeIterator`; should that chain become one, `count()` short-circuits and the local path stops printing with no other symptom.
- A supplementary-group list is the account's whole membership as far as the kernel is concerned, not a cache over the membership service; shortening one costs the account every access it holds through a dropped group.
- `tests/shim_registration.rs` compiles `tests/support/shared_capture.rs` as a `#[path]` module, so a package test result taken while that file is being rewritten is early signal, not a gate; `README.md` is pulled in by no `include_str!`, so editing it implicates no cargo line.

**Ruled out:**
- Truncating an over-limit Darwin membership with a per-account note — it silently costs the account every access held through a dropped group.
- Collecting every toolchain report before printing — a terminated child then reports nothing for work it had finished.
- Rewording the refusal's `could not resolve {name}'s groups: {error}` wrapper — the sentence after the colon already carries the account name, both counts, the limit, and the recovery.

### Phase 5 — A promoted summary row reports its whole subtree  · status: todo

#### Work Order

**Goal:** A summary row drawn in place of its hidden driver reports the CPU of every cargo invocation beneath it exactly once, and an unavailable nested contribution makes the total unavailable rather than a number.

**Spec:**
`summary_rows` (`crates/cargo-tile/src/render.rs:678`) promotes a managed row in place of its hidden driver. `Census::attribute_cpu` (`crates/cargo-tile/src/processes.rs:1459`) charges non-cargo descendants (compilers, build scripts) to their nearest cargo invocation, so a promoted row's own bucket already holds them. What it omits is every separately attributed nested cargo invocation hidden beneath it. `Census::groups` (`:1996`) overwrites each managed row's CPU with its own attributed share after assembly (about `:2032`), so a repair in `Census::group` (`:2182`) alone is undone. `aggregate_cpu` (`:2401`) and `Measurement<T>` (`:221`) already propagate `Measurement::Unavailable`.

Compute, on the worker after group assembly, each promoted row's subtree total: its own invocation bucket plus each nested cargo attribution in its subtree exactly once, excluding the hidden driver and any sibling subtree, with `Measurement::Unavailable` propagating so an unavailable nested contribution makes the total unavailable. Added work and storage are linear in the assembled invocations and their parent links; reuse the membership traversal `groups` already performs. The command view's per-row measurements are unchanged; `summary_rows` reads the prepared total instead of the row's own share.

**Files:**
- `crates/cargo-tile/src/processes.rs` — subtree totals after assembly in `Census::groups`; the total carried on the managed row
- `crates/cargo-tile/src/render.rs` — `summary_rows` (`:678`) reads the prepared total
- `crates/cargo-tile/src/roster.rs`, `crates/cargo-tile/src/terminal.rs` — their `CargoProcess` literals carry the new subtree total
- `crates/cargo-tile/tests/summary_totals.rs` — new: a `#[path]`-included harness (copy the include block from `tests/shim_registration.rs:3`) driving a hidden driver with two promoted children and a deeper nested cargo, one unavailable, through `Census::groups` and `summary_rows`

**Test access:** `Census`, `Census::groups` (`:1996`), and `summary_rows` are private, so copying the include block does not by itself let the tester call them or inject a deterministic unavailable attribution. Drive the assertions through interfaces the harness can already reach, or through narrowly scoped `cfg(test)` adapters the production writer owns and lands before the tester needs them; visibility does not widen for production callers.

**Seats:** 1 writer + 1 tester + reserve — `render.rs` only reads what `processes.rs` computes, and the other `CargoProcess` constructor sites move with the field.
- `impl` — `crates/cargo-tile/src/processes.rs`, `crates/cargo-tile/src/render.rs`, `crates/cargo-tile/src/roster.rs`, `crates/cargo-tile/src/terminal.rs`, and the `cfg(test)` adapters the gate needs; hub: `crates/cargo-tile/src/processes.rs`
- `test` — `crates/cargo-tile/tests/summary_totals.rs`: the tree above, asserting each promoted total includes its descendants once, excludes the sibling subtree, and is unavailable for an unavailable descendant, and that the command view's per-row measurements are unchanged
- `review` — reserve

**Constraints from prior phases:** none that bind; Phases 2–4 touched `cli.rs`, `hook.rs`, `constants.rs`, and the support tests only.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-tile`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-tile`, `bash ~/.claude/scripts/delegate/verify.sh lint cargo-tile` green; `bash ~/.claude/scripts/delegate/verify.sh test cargo-tile summary_totals` green.

### Phase 6 — Every Settings account line is reachable  · status: todo

#### Work Order

**Goal:** In the Settings popup every account line, every wrapped diagnostic line, and every settings row below them can be scrolled to and selected, on any terminal height.

**Spec:**
`draw_settings` (`crates/cargo-tile/src/render.rs:2128`) draws the capture section — the shared directory and one line per account directory — as a paragraph that does not follow the settings viewport, so rows past the bottom edge cannot be reached. `capture_root_status` (`crates/cargo-tile/src/settings.rs:451`) and `push_value` (`:385`) build one selectable row per account holding its diagnostics and associations, so one row wraps to as many lines as its diagnostics need and can itself be taller than the popup. `SettingsPane::row_at` (`crates/tui_pane/src/overlays/settings.rs:238`) and `line_for_selection` (`:249`) map selectable rows to rendered lines for mouse selection.

With enough account directories to overflow a small terminal, navigation reveals every account line and the settings rows below them, keeps the selected row visible after resizing or status updates, and leaves the capture lines inert (not editable). With one account whose wrapped status is taller than the popup, scrolling exposes every continuation line, and clicking a visible continuation line selects that account. Positioning reuses the rendered lines and the line-target map already built, bounded to one linear pass over rendered lines per frame, with no extra filesystem read or formatting pass. The viewport offset lives with the pane's selection state, not recomputed from scratch per key.

Continuation lines already carry their row target: `push_wrapped_value_row` (`crates/tui_pane/src/overlays/settings.rs:659`) stamps every wrapped line of a row with that row's payload. So a click on a visible continuation line already selects its row, and that requirement is regression coverage the tester asserts rather than behavior the writer builds — implementation concentrates on the viewport and the scroll offset.

The line-target map this phase reworks is the plan's `Option<T>` invariant in the open: `line_targets: Vec<Option<SettingsRowPayload>>` (`crates/tui_pane/src/overlays/settings.rs:107`, filled by `set_line_targets` `:220`) uses `None` for a rendered line that belongs to no selectable row; `line_target` (`:226`), `row_at` (`:238`), and `line_for_selection` (`:249`) answer `Option<usize>` for three different absences — a rendered line that is decoration, a position outside every row, and a selection with no rendered line yet. Give each a named outcome saying what its absence means, and carry the conversion through to the domain-owned boundary rather than stopping at the overlay: `SettingsRow.payload: Option<SettingsRowPayload>` (`crates/tui_pane/src/settings_store/row.rs:13`) currently stands for both a section heading that is decoration and a row whose identity nobody supplied, and that ambiguity is what forces the positional fallback. Name row identity explicitly there, keep the conversion at the caller that indexes a `Vec` or reads a mouse position, and re-export whatever the new outcomes need.

Changing `row_at` reaches the framework's hit testing (`crates/tui_pane/src/framework/mod.rs:406`), which maps its `Option` into `FrameworkHit::ModalMissed`; that caller and any other consumer of the three outcomes belong to the pane writer, not to the cargo-tile writer.

**Files:**
- `crates/cargo-tile/src/render.rs` — `draw_settings` (`:2128`): viewport-to-rendered-line positioning
- `crates/cargo-tile/src/settings.rs` — `capture_root_status` (`:451`), `push_value` (`:385`)
- `crates/tui_pane/src/overlays/settings.rs` — `line_target` (`:226`), `row_at` (`:238`), `line_for_selection` (`:249`): scroll offset; the named outcomes replacing the three `Option` answers, and the `line_targets` element type (`:107`, filled at `:220`)
- `crates/tui_pane/src/settings_store/row.rs` — `SettingsRow.payload` (`:13`): explicit row identity in place of the `Option`
- `crates/tui_pane/src/framework/mod.rs` — the settings hit test (`:406`) consuming the new `row_at` outcome
- `crates/tui_pane/src/lib.rs`, `crates/tui_pane/src/overlays/mod.rs`, `crates/tui_pane/src/settings_store/mod.rs` — exports the new outcomes need
- `crates/tui_pane/tests/settings_scroll.rs` — new: pane-level scrolling and selection tests

**Seats:** 2 writers + 1 tester — the pane logic and the cargo-tile rendering are separate crates.
- `impl` — `crates/cargo-tile/src/render.rs`, `crates/cargo-tile/src/settings.rs`; hub: `crates/cargo-tile/src/render.rs`
- `review` — opens as impl: the `tui_pane` settings overlay, `settings_store/row.rs`, the affected framework callers and exports; hub: `crates/tui_pane/src/overlays/settings.rs`
- `test` — `crates/tui_pane/tests/settings_scroll.rs`: many short rows overflowing the height, one row taller than the height, selection kept visible across a resize, and a click on a continuation line selecting its row through the framework hit test

**Constraints from prior phases:** none that bind; Phase 5 changed `summary_rows` in `render.rs`, not `draw_settings`.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-tile`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-tile`, `bash ~/.claude/scripts/delegate/verify.sh lint cargo-tile`, `bash ~/.claude/scripts/delegate/verify.sh check tui_pane`, `bash ~/.claude/scripts/delegate/verify.sh test tui_pane`, `bash ~/.claude/scripts/delegate/verify.sh lint tui_pane` green; `bash ~/.claude/scripts/delegate/verify.sh test tui_pane settings_scroll` green.

### Phase 7 — The ownership check closes the macOS ACL hole  · status: todo

#### Work Order

**Goal:** On macOS, a non-owner ACL granting write access on the root, state, or pids directory disables sweeping with a refusal naming the directory and the reason, and an ACL added after the scan opened the directory is caught before any unlink.

**Spec:**
The ownership prerequisite (`InspectedDirectoryMetadata::refusals`, `crates/cargo-tile/src/capture_root.rs:637`; metadata sampled in `InspectedDirectory::inspect` `:679`) checks uid and mode bits alone; `RootScan::cleanup_refusals` (`:422`) gates private `RootScan::access` (`:471`). `RootScan::revalidate_paths` (`:487`) and `OwnedRoot::sweep` (`:923`) recheck only filesystem identity, owner, and mode before each unlink. `RegistrationCandidate::verify_observation` does not close the gap.

Read the ACL through the held directory descriptor on macOS (the `acl_get_fd` / `acl_get_entry` / `acl_get_permset` family from `<sys/acl.h>`, declared in a small `extern "C"` block if the `libc` crate lacks them) and reduce it to one of three explicit states: **no non-owner write grant present**, non-owner write access present, or inspection failed. Name the first state for what it actually proves — the specified read-only non-owner control must still sweep, so the check establishes the absence of non-owner write grants, not owner-only access; the separately observed `RootOwner` continues to carry ownership on its own and neither state replaces it. An absent or empty ACL is the first state, not an inspection failure: Darwin's entry iteration returns zero when it hands back an entry and `-1` with `EINVAL` at exhaustion (`acl_get_entry`, Apple's `Libc/posix1e/acl_entry.c`), so exhaustion on the first call means there is nothing to refuse, while any other error is the third state. Write access means an allow entry for a principal other than the owner carrying any write-class permission (`ACL_WRITE_DATA`/`ACL_ADD_FILE`, `ACL_APPEND_DATA`/`ACL_ADD_SUBDIRECTORY`, `ACL_DELETE`, `ACL_DELETE_CHILD`, `ACL_WRITE_ATTRIBUTES`, `ACL_WRITE_EXTATTRIBUTES`, `ACL_WRITE_SECURITY`, `ACL_CHANGE_OWNER`). Only the first state passes the private cleanup gate. Non-owner write access becomes a new path-qualified `CleanupRefusal` variant (`:195`) rendered by `cleanup_refusal` (`crates/cargo-tile/src/settings.rs:666`); inspection failure becomes `CleanupRefusal::Access(PathFailure)` for that path. The same ACL check is part of `revalidate_paths`. On Linux the state is always the first one (no ACL read). Cost: one native ACL query per directory at inspection and at revalidation, no subprocess per registration.

**Files:**
- `crates/cargo-tile/src/capture_root.rs` — the ACL state type, `refusals`, `inspect`, `revalidate_paths`, the new `CleanupRefusal` variant
- `crates/cargo-tile/src/settings.rs` — `cleanup_refusal` (`:666`) renders the new variant
- `crates/cargo-tile/src/constants.rs` — the write-class ACL permission mask, with its rationale
- `crates/cargo-tile/tests/capture_root_acl.rs` — new, `#[cfg(target_os = "macos")]`: `#[path]`-included harness (copy the include block from `tests/shim_registration.rs:3`)

**Seats:** 1 writer + 1 tester — one module owns the check; the tests are a separate macOS-only target.
- `impl` — `crates/cargo-tile/src/capture_root.rs`, `crates/cargo-tile/src/settings.rs`, `crates/cargo-tile/src/constants.rs`; hub: `crates/cargo-tile/src/capture_root.rs`
- `test` — `crates/cargo-tile/tests/capture_root_acl.rs`: a grant on each of the root, state, and pids directories, a grant added after the directory was opened, an inspection failure, a read-only non-owner ACL control that still sweeps, a directory with no ACL and a directory with an empty ACL both sweeping, ownership still reported independently of the ACL state, and the refusal rendering naming its directory; on Linux the file compiles to nothing
- `review` — reserve

**Constraints from prior phases:** none that bind; no earlier phase touched `capture_root.rs`. Phase 3 proved the crate compiles and tests natively on macOS, so this phase's macOS-only target has a native route as well as the workflow's `macos-latest` job (`.github/workflows/ci.yml:348`); both are subject to the three pre-existing `shim_registration` failures recorded in Delegation Context, which this target does not touch.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-tile`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-tile`, `bash ~/.claude/scripts/delegate/verify.sh lint cargo-tile` green on Linux. Smoke (orchestrator, macOS): `bash ~/.claude/scripts/delegate/verify.sh test cargo-tile capture_root_acl` green on the Mac, natively or through the `macos-latest` job.

### Phase 8 — Registration framing v3 with a version-skew diagnostic  · status: todo

#### Work Order

**Goal:** A reader that meets a registration framed by a newer shim reports a diagnostic naming the observed and supported versions, never unlinks it, and the install refuses to replace a newer shim with an older one.

**Spec:**
The shim's macOS boot field changed from a `kern.boottime` timeval to the boot-session UUID without a format version bump; the framing string is `cargo-tile-v2` (`crates/cargo-tile/src/cargo-capture-shim.sh:298`; `Registration::parse`, `crates/cargo-tile/src/registration.rs:31`). A reader built before `BirthStamp::compare` (`crates/cargo-tile/src/birth_stamp/mod.rs:87`) learned to answer `Unknown` for that mismatch treats every live capture as ended; identify that reader revision from `git log -S kern.boottime -- crates/cargo-tile/src` and record it in the diagnostic text's documentation. Today `parse` checks the payload field count before the magic, so a newer layout surfaces as an ordinary framing error, and an unknown magic becomes `ParseError::Magic` (`:229`), which `registered_runs` (`crates/cargo-tile/src/progress.rs:939`) reports as the generic `RegistrationInvalid` `CaptureDiagnostic` (`crates/cargo-tile/src/processes.rs:806`).

Bump the framing to `cargo-tile-v3` for the changed boot-field semantics; the reader keeps decoding `v2`. `parse` inspects the bounded version header first: a `cargo-tile-v<N>` magic with `N` above the supported version returns a distinct unsupported-version `ParseError` carrying the encountered version, before any version-specific field is read. `registered_runs` turns it into a new path-qualified `CaptureDiagnostic` variant naming the encountered and supported versions, rendered by `capture_diagnostic` (`crates/cargo-tile/src/settings.rs:623`) with the instruction to upgrade and restart the reader. Such a record never becomes a `RegistrationCandidate` or a sweep candidate, and its registration and log are preserved whether or not the writer pid is alive. `Hook::install` (`crates/cargo-tile/src/hook.rs:630`) compares shim contents: add a version line to the shim header and make install refuse to replace a shim whose version is newer than its own, reporting it as a distinct `HookOperationOutcome` variant. Rollout order (Phase 17): readers deployed and restarted before the new shim is installed.

The refusal has a second consumer, and it is the one a user meets first. Starting the grid calls `capture::stand_up` (`crates/cargo-tile/src/terminal.rs:149`), and `capture.auto_install` defaults to true (`constants.rs:838`), so an older reader started on a machine that already carries the newer shim performs the same install and meets the same refusal. Routing it through the existing failure collection (`crates/cargo-tile/src/capture.rs:81`) would make the startup toast say `not installed`, which is untrue — the newer shim is installed and capture works; discarding it would hide the refusal entirely. Give it its own startup handling: a toast and a Settings line saying the newer shim was kept and this reader is the older one, with upgrading the reader as the recovery. The refused shim's bytes are unchanged by startup.

**Files:**
- `crates/cargo-tile/src/registration.rs` — version header parse, the unsupported-version error
- `crates/cargo-tile/src/progress.rs` — `registered_runs` (`:939`) maps it to the diagnostic; never a candidate
- `crates/cargo-tile/src/processes.rs` — `CaptureDiagnostic` (`:806`) variant
- `crates/cargo-tile/src/settings.rs` — `capture_diagnostic` (`:623`)
- `crates/cargo-tile/src/cargo-capture-shim.sh` — framing `v3`, version line in the header
- `crates/cargo-tile/src/hook.rs` — `Hook::install` (`:411`) version comparison and outcome
- `crates/cargo-tile/src/cli.rs` — child protocol encoding and ordinary install result rendering
- `crates/cargo-tile/src/capture.rs` — `stand_up` (`:81`) handles the downgrade refusal as its own startup note, not as a failure
- `crates/cargo-tile/src/constants.rs` — the supported framing version and the shim version-line constants
- `crates/cargo-tile/tests/shim_registration.rs` — scenarios
- `crates/cargo-tile/tests/support/shared_capture.rs` — downgrade refusal through the account child, decoder, and administrative report

**Seats:** 2 writers + 1 tester — reader and installer changes have disjoint owners.
- `impl` — `crates/cargo-tile/src/registration.rs`, `crates/cargo-tile/src/progress.rs`, `crates/cargo-tile/src/processes.rs`, `crates/cargo-tile/src/settings.rs`, `crates/cargo-tile/src/constants.rs`; hub: `crates/cargo-tile/src/registration.rs` (the error type both sides name)
- `review` — opens as impl: `crates/cargo-tile/src/cargo-capture-shim.sh`, `crates/cargo-tile/src/hook.rs`, `crates/cargo-tile/src/cli.rs`, `crates/cargo-tile/src/capture.rs`; owns the downgrade outcome through child encoding, decoding, account reduction, startup handling, and rendering
- `test` — `crates/cargo-tile/tests/shim_registration.rs`, `crates/cargo-tile/tests/support/shared_capture.rs`: a newer header with a different payload layout, an absent writer pid, retention across scans, a legacy `v2` record still read, a malformed supported record as a separate case, mixed `v2`/`v3` publications during live captures, install refusing to downgrade a newer shim, a downgrade refusal beside a successful installation through the account protocol and administrative report, and an older reader starting with `capture.auto_install` on against a newer installed shim — the shim's bytes unchanged and the startup note saying it was kept rather than `not installed`

**Constraints from prior phases:** Phase 4 shipped the operation-neutral hook protocol these changes extend, and these are the names that now exist — no rename remains to anticipate. `HookAccount` (`hook.rs:74`) is the account, `HookOperation` (`:87`) the verb, `ToolchainHookReport` (`:188`) one toolchain's row, `AccountHookReport` (`:273`) one account's reduction, and `HookOperationOutcome` (`:466`) the per-toolchain result the new downgrade variant joins. `Hook::reports` (`:500`) returns an iterator and **must stay one**: `cli.rs` consumes it through `.inspect(print_local_report)` and `account_hook_report` (`cli.rs:190`) writes and flushes each protocol row before the next toolchain starts, so a terminated child still reports the work it finished. Collecting before printing reintroduces the defect Phase 4's review opened as a blocker. `Hook::state` (`:588`) is the unlocked status read; `Hook::install` (`:630`) and `Hook::remove` (`:683`) each take `HookInstallationLock` and inspect state **under** it, so bind the shim version comparison to the same lock. `run_account_hooks` (`:905`) drives the account children behind the hidden `--account-hook-report` flag (`constants.rs:762`, `ACCOUNT_HOOK_REPORT_FLAG`). `Hook::remove` answers `Ok(HookOperationOutcome::Orphaned)` for a shim whose real cargo is gone: keep its rendered row and its incomplete-removal classification, and do not reduce it to an error. Phase 4 also deleted `AccountGroupNote` and `CredentialGroups` and made the credential callback `impl FnMut(&mut Command, &HookAccount) -> io::Result<()>`; those are implementation-only and need no surface here. Carry downgrade refusal through the operation result, child encoder, toolchain decoder, account failure reduction, startup handling, and human renderers. A refused downgrade retains its named toolchain row and prevents an unqualified installed account summary even beside successful installations. Phase 5 changed `processes.rs` group assembly, not `CaptureDiagnostic`. Phase 1 replaced the harness's nested readiness wait: a scenario that waits for a nested fixture calls `wait_for_fixture_pane(markers)` (`shim_registration.rs:508`), which polls until exactly one command pane holds every marker and returns that screen, and asserts rows on the screen it returns; `fixture_pane` (`:503`) is for a screen already known ready; no test scrubs `CARGO_TILE_ROOT`.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-tile`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-tile`, `bash ~/.claude/scripts/delegate/verify.sh lint cargo-tile` green; `bash ~/.claude/scripts/delegate/verify.sh test cargo-tile shim_registration` green with the scenarios above, including a downgrade refusal beside a successful installation in both report orders, asserting unchanged newer-shim bytes, the distinct named refusal in the administrative output, an account summary that does not claim successful installation, and an older reader's startup against a newer installed shim leaving those bytes unchanged and reporting the refusal as a kept-newer-shim note rather than `not installed`. Smoke (orchestrator, macOS, Phase 17): a native capture from the `v3` shim is read live.

### Phase 9 — cargo-berth: a fixture ledger with a merge-extent record and content-checked hook responses  · status: todo

#### Work Order

**Goal:** A repeatable check proves that a given `cargo-berth` executable reads a ledger carrying a `merge_extent_observed` record and that both hooks return their expected response contents against it.

**Spec:**
Reconciliation appends `JournalOperation::MergeExtentObserved` (`crates/cargo-berth/src/ledger/journal.rs:362`) on the first `board`, `check`, or `drift` against a ledger; an older binary then refuses the whole ledger (`journal record N is corrupt: unknown variant merge_extent_observed`). The SessionStart and PostToolUse hooks exit zero on a ledger failure, so an exit code proves nothing; an empty ledger does not exercise the incompatibility. Hooks resolve their executable through `CARGO_BERTH_EXECUTABLE` (`crates/cargo-berth/src/gate/install.rs:32`), `PATH`, and the cargo-home fallback.

Add a fixture ledger under `crates/cargo-berth/tests/fixtures/` that contains a real `merge_extent_observed` record (generate it by driving reconciliation in a temporary repository with the current binary; commit the resulting ledger bytes). Add a test target that, for an executable path taken from `CARGO_BERTH_EXECUTABLE` when set and the freshly built binary otherwise, runs `board`, `check`, and `drift` against the fixture and asserts the response payloads, then drives SessionStart and PostToolUse and compares their response contents with the expected fixtures in the style of `crates/cargo-berth/tests/hooks.rs`, failing on any ledger error in the output. This is the tool Phase 17's rollout inventory runs against every installed reader.

`hooks.rs` is a separate integration binary and its response helpers (around `crates/cargo-berth/tests/hooks.rs:2458`, `hook_feedback` and its neighbours) are private to it, so the new target cannot call them directly however the two are arranged. Extract the expected-response construction into a small shared module under `crates/cargo-berth/tests/support/`, included by both binaries through `#[path]`, and keep the extraction behavior-preserving: `hooks.rs` keeps its current assertions and simply calls the extracted helpers. The helper that invokes a reader takes the executable path explicitly rather than resolving one of its own, since choosing the executable is the whole point of the new target.

**Files:**
- `crates/cargo-berth/tests/fixtures/` — the fixture ledger
- `crates/cargo-berth/tests/reader_compat.rs` — new: the executable-parameterized reader and hook checks
- `crates/cargo-berth/tests/support/reader_compat_hooks.rs` — new: the extracted expected-response helpers, with explicit executable selection, `#[path]`-included by both binaries
- `crates/cargo-berth/tests/hooks.rs` — calls the extracted helpers; assertions unchanged

**Seats:** 2 writers + reserve — the fixture and the CLI checks split from the helper extraction and the hook-content assertions.
- `impl` — `crates/cargo-berth/tests/fixtures/`, `crates/cargo-berth/tests/reader_compat.rs`, including the calls into the shared helpers; hub: `crates/cargo-berth/tests/reader_compat.rs`
- `test` — opens as impl: `crates/cargo-berth/tests/hooks.rs` and the new `crates/cargo-berth/tests/support/reader_compat_hooks.rs`; owns the extraction and the hook-content assertions
- `review` — reserve

**Constraints from prior phases:** none; first cargo-berth phase.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-berth`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth`, `bash ~/.claude/scripts/delegate/verify.sh lint cargo-berth` green; `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth reader_compat` green; with `CARGO_BERTH_EXECUTABLE` pointing at a binary built from `hana` main `a95922f9` (a pre-record reader), the target fails naming the corrupt-record error.

### Phase 10 — A waiting or deferred reservation exposes its extents on request  · status: todo

#### Work Order

**Goal:** `board --reservation <id> --json` reports a reservation's `race_extent` and `merge_extent` beside its lifecycle, for waiting successors and unresolved-overlap endpoints too.

**Spec:**
Board placement (`crates/cargo-berth/src/board/rows.rs`) keeps waiting successors and unresolved-overlap endpoints out of every reservation-snapshot section, and `board --reservation <id> --json` reports lifecycle only, so an operator holding one of those reservations cannot see its protected paths or retained evidence.

`board --reservation <id> --json` carries `race_extent` and `merge_extent`, each in the same shape the snapshot sections use, including `unavailable` with retained evidence and `not_derived` with declared protection. `crates/cargo-berth/src/output.rs` gains the fields and `docs/cargo-berth/generated/output-contract.json` is regenerated by setting `CARGO_BERTH_REGENERATE_OUTPUT_CONTRACT` and running the inline test in `crates/cargo-berth/src/output_contract.rs` (`:189`–`:205`), which `crates/cargo-berth/tests/output_contract.rs:12` then asserts against. Develop and test against isolated ledgers only (the shared ledger waits on the Phase 17 rollout).

**Files:**
- `crates/cargo-berth/src/board/report.rs` — the per-reservation report gains extents
- `crates/cargo-berth/src/verb/board.rs` — `--reservation` path fills them
- `crates/cargo-berth/src/output.rs` — output types
- `docs/cargo-berth/generated/output-contract.json` — regenerated
- `crates/cargo-berth/tests/board.rs` — coverage

**Seats:** 1 writer + 1 tester.
- `impl` — `crates/cargo-berth/src/board/report.rs`, `crates/cargo-berth/src/verb/board.rs`, `crates/cargo-berth/src/output.rs`, `docs/cargo-berth/generated/output-contract.json`; hub: `crates/cargo-berth/src/output.rs`
- `test` — `crates/cargo-berth/tests/board.rs`: waiting successors and both unresolved-overlap endpoints expose protected, empty, and unavailable extents alongside their lifecycle
- `review` — reserve

**Constraints from prior phases:** Phase 9 added `tests/reader_compat.rs` and a fixture ledger; a new journal operation must not be introduced here (readers are upgraded in Phase 17).

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-berth`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth`, `bash ~/.claude/scripts/delegate/verify.sh lint cargo-berth` green; `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth board` and `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth output_contract` green.

### Phase 11 — The durable evidence record carries the effective edit-blocking decision  · status: todo

#### Work Order

**Goal:** The `evidence_revalidated` journal record's edit-blocking field always equals the live blocking answer at the moment the record is written.

**Spec:**
`JournalOperation::EvidenceRevalidated` (`crates/cargo-berth/src/ledger/journal.rs:422`) already carries `status: IntegrationEvidenceStatus` and `edit_blocking_status: EditBlockingStatus`. The second is derived by `IntegrationEvidenceStatus::edit_blocking_status` (`crates/cargo-berth/src/reservation/lifecycle.rs:167`), which answers `Clear` from checkpoint integration alone, so with an integrated checkpoint followed by later unmerged work the persisted field reads `clear` while the reservation correctly keeps blocking through its merge extent. `Reservation::edit_blocking_status` (`crates/cargo-berth/src/reservation/record.rs:355`) is the decision live checks use, but `prepare_reconciliation_transaction` (`crates/cargo-berth/src/reconcile.rs:969`) constructs the evidence operation (`append_evidence_and_retention`, `:1899`) before `derive_merge_extents` (`:1039`) runs, so calling it there would read the previous extent.

Build each new evidence record from the checkpoint evidence and the reservation state after the transaction's planned lifecycle and merge-extent updates, through `Reservation::edit_blocking_status`, with no additional Git queries; checkpoint evidence alone never supplies the effective decision. The release path (`outstanding_operation`, `crates/cargo-berth/src/verb/release.rs:451`) does the same. Historical records remain readable and unchanged. Isolated ledgers only.

**Files:**
- `crates/cargo-berth/src/reconcile.rs` — evidence record built after extent derivation
- `crates/cargo-berth/src/reservation/lifecycle.rs` — `edit_blocking_status` no longer the source of the persisted field
- `crates/cargo-berth/src/reservation/record.rs` — the decision function, if its inputs need the planned state
- `crates/cargo-berth/src/verb/release.rs` — `outstanding_operation`
- `crates/cargo-berth/tests/lifecycle.rs` — coverage

**Seats:** 1 writer + 1 tester.
- `impl` — `crates/cargo-berth/src/reconcile.rs`, `crates/cargo-berth/src/reservation/lifecycle.rs`, `crates/cargo-berth/src/reservation/record.rs`, `crates/cargo-berth/src/verb/release.rs`; hub: `crates/cargo-berth/src/reconcile.rs`
- `test` — `crates/cargo-berth/tests/lifecycle.rs`: reconciliation and release separately — clear to protected, protected to clear, the existing later-work regression, an extent change within the transaction, unavailable extents with retained evidence — each asserting the persisted decision equals the live answer at that record
- `review` — reserve

**Constraints from prior phases:** Phase 10 added extent fields to the board output; the journal record shape is unchanged by it and by this phase.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-berth`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth`, `bash ~/.claude/scripts/delegate/verify.sh lint cargo-berth` green; `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth lifecycle` green.

### Phase 12 — A `cargo install` shows its build progress like a `cargo build`  · status: todo

**Blocked by:** G1 — the orchestrator records the Mac reproduction (toolchain, terminal mode, progress settings in effect, the registration, the captured output) into this Spec before dispatch; it needs ssh to the Mac, which needs 1Password unlocked.

#### Work Order

**Goal:** A `cargo install` row in cargo tile carries the same compiling counter a `cargo build` row does.

**Spec:**
The failing stage is not yet localized. Candidate stages, in the order to check: shim command classification in `crates/cargo-tile/src/cargo-capture-shim.sh` (`case $first` at `:80`, where `install` already takes the capture path), registration and identity verification of the captured process, the captured log itself, counter parsing in `crates/cargo-tile/src/progress.rs` (`parse_state` `:1086`, `last_counter` `:1120`, which do not filter by subcommand), and the heading gauge in `crates/cargo-tile/src/render.rs` (`heading_gauge` `:1894`, omitted when the column is too narrow). The shim removes the registration and log on exit, so evidence is captured while the install is live.

Mac reproduction (G1): <recorded by the orchestrator before dispatch>.

Change the first stage the reproduction shows failing. Add an integration test that installs an isolated binary fixture — `cargo install --path <fixture>` with its own `CARGO_INSTALL_ROOT` and `CARGO_TARGET_DIR`, a fixture whose `build.rs` waits on a release file so compilation stays live — and asserts the production reader reports `Phase::Building` (`progress.rs:136`) with the same counter it reports for `cargo build` under equivalent settings, and that completion clears the counter. The original Mac case is kept as an acceptance case; the settings pane needs no change.

**Files:**
- `crates/cargo-tile/src/cargo-capture-shim.sh` — only if classification is the failing stage
- `crates/cargo-tile/src/progress.rs` — only if parsing is the failing stage
- `crates/cargo-tile/src/render.rs` — only if the gauge is the failing stage
- `crates/cargo-tile/tests/shim_registration.rs` — the install scenario
- `crates/cargo-tile/tests/fixtures/install_progress/` — new: the fixture crate

**Seats:** 1 writer + 1 tester — the fix lands in one stage; the scenario and fixture are separate.
- `impl` — whichever of the three source files the reproduction names; hub: `crates/cargo-tile/src/progress.rs`
- `test` — `crates/cargo-tile/tests/shim_registration.rs`, `crates/cargo-tile/tests/fixtures/install_progress/`: the build-versus-install scenario
- `review` — reserve

**Constraints from prior phases:** Phase 8 bumped the shim framing to `v3` and added a version line to the shim header. Phase 1 replaced the harness's nested readiness wait: a scenario that waits for a nested fixture calls `wait_for_fixture_pane(markers)` (`shim_registration.rs:508`), which polls until exactly one command pane holds every marker and returns that screen, and asserts rows on the screen it returns; `fixture_pane` (`:503`) is for a screen already known ready; no test scrubs `CARGO_TILE_ROOT`.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-tile`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-tile`, `bash ~/.claude/scripts/delegate/verify.sh lint cargo-tile` green; `bash ~/.claude/scripts/delegate/verify.sh test cargo-tile shim_registration` green with the install scenario. Smoke (orchestrator, macOS): the recorded Mac case shows a counter.

### Phase 13 — `birth_stamp/mod.rs` keeps only the module-name type  · status: todo

#### Work Order

**Goal:** The `birth_stamp` module root is a table of contents plus `BirthStamp`; every other type lives in a cohesive leaf module with its behavior and tests.

**Spec:**
`crates/cargo-tile/src/birth_stamp/mod.rs` declares `linux` and `macos` and then defines `ProcessLifetime` (`:106`, with `ProcessLifetime::macos` `:116`), `LifetimeEvidence` (`:134`), `IdentityEvidence` (`:166`), `Observation`, `KernelObservation` (`:197`), `Verification` (`:222`), `observe` (`:232`), and `observe_process` (`:279`) beside `BirthStamp` (`:29`). `LifetimeEvidence::Available` holds a `ProcessLifetime`, `KernelObservation` holds an `Observation`, and `observe` fills `KernelObservation`'s private fields; `crates/cargo-tile/src/birth_stamp/macos.rs` (`observe` `:48`) reaches methods private to the parent.

The root keeps the `mod` block, re-exports, and `BirthStamp` with its implementation. The remaining code moves as cohesive clusters into leaf modules named for their anchor types — one keeps `ProcessLifetime` with `LifetimeEvidence`; one keeps `KernelObservation` with its `Observation` payload and the production acquisition functions — taking the free helpers and inline tests with the behavior they exercise. Not one file per type (style rule `split-by-type-ownership`). `KernelObservation`'s fields stay private, production construction stays pid-bound, and arbitrary observation injection exists only under `cfg(test)`. Check Linux and macOS visibility, including `ProcessLifetime::macos` and what `macos.rs` and `linux.rs` reach, before choosing boundaries. `tests/shim_registration.rs:7` includes `../src/birth_stamp/mod.rs` and stays valid.

**Files:**
- `crates/cargo-tile/src/birth_stamp/mod.rs` — reduced to the table of contents and `BirthStamp`
- `crates/cargo-tile/src/birth_stamp/` — new leaf modules
- `crates/cargo-tile/src/birth_stamp/macos.rs`, `crates/cargo-tile/src/birth_stamp/linux.rs` — imports and visibility

**Seats:** 1 writer; nothing splits — a module move is one writer's edit.
- `impl` — everything under `crates/cargo-tile/src/birth_stamp/`; hub: `crates/cargo-tile/src/birth_stamp/mod.rs`
- `test` — opens as test: no new behavior; confirms every inline test travelled with its type and that the macOS-only paths still compile by reading `macos.rs` against the new boundaries
- `review` — reserve

**Constraints from prior phases:** Phase 8 changed `registration.rs` and `progress.rs`, not `birth_stamp`.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-tile`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-tile`, `bash ~/.claude/scripts/delegate/verify.sh lint cargo-tile` green with the package tests unchanged. Smoke (orchestrator, macOS): `verify.sh check cargo-tile` and `verify.sh test cargo-tile` green on the Mac. **Prerequisite:** that native suite is not green today — the three pre-existing `shim_registration` failures recorded in Delegation Context must be resolved first (they are a recorded next item), or this gate cannot be met by anything this phase does.

### Phase 14 — `capture_root.rs` is anchored and split  · status: todo

#### Work Order

**Goal:** The capture-root module is a directory named for its anchor type `RootScan`, with one anchor-named submodule per type cluster, and every test still passes.

**Spec:**
`crates/cargo-tile/src/capture_root.rs` (1023 non-test lines, 25 top-level types) holds three clusters — directory inspection (`InspectedDirectory`, `Inventory`, `ScanEntry`), root identity (`RootHistory`, `TreeIdentity`, `RootIncarnation`), and sweep policy (`SweepBudget`, `SweepDisposition`, `OwnedRoot`) — under a module name that no type carries. The anchor type is `RootScan`, not `CaptureRoot`.

Move the file to a module directory named for `RootScan` (`crates/cargo-tile/src/root_scan/mod.rs` holding `RootScan`, the `mod` block, and re-exports), with each cluster an anchor-named submodule taking its inline tests. Keep the public surface: the paths `progress.rs`, `settings.rs`, `app.rs`, and `tests/shim_registration.rs:11` (`#[path = "../src/capture_root.rs"]`) use are updated to the new module, and nothing widens visibility. Test behavior and assertions are unchanged.

**Files:**
- `crates/cargo-tile/src/capture_root.rs` — becomes `crates/cargo-tile/src/root_scan/` (mod.rs plus cluster submodules)
- `crates/cargo-tile/src/main.rs` — the `mod` declaration
- `crates/cargo-tile/src/progress.rs`, `crates/cargo-tile/src/settings.rs`, `crates/cargo-tile/src/app.rs`, `crates/cargo-tile/src/cli.rs`, `crates/cargo-tile/src/processes.rs`, `crates/cargo-tile/src/render.rs`, `crates/cargo-tile/src/terminal.rs` — import paths; these are every file naming `capture_root`, confirmed against the tree, and the last four were absent from this list
- `crates/cargo-tile/tests/shim_registration.rs` — the `#[path]` include at `:11`
- `crates/cargo-tile/tests/support/shared_capture.rs` — its own `capture_root` reference at `:11`
- `crates/cargo-tile/tests/capture_root_acl.rs` — its `#[path]` include (Phase 7 copied the whole include block from `shim_registration.rs:3`)
- `crates/cargo-tile/tests/summary_totals.rs` — its `#[path]` include (Phase 5 copied the same block, so it names `capture_root.rs` too)

**Seats:** 1 writer + 1 tester + reserve; nothing splits — a module move is one writer's edit.
- `impl` — everything above, every consumer and all three copied harness include blocks included; hub: `crates/cargo-tile/src/root_scan/mod.rs`
- `test` — opens as test: no new behavior; confirms the package compiles against the new paths and that every inline test travelled with its type; no file edits
- `review` — reserve

**Constraints from prior phases:** Phase 7 added the ACL state type and a `CleanupRefusal` variant to this file, and `tests/capture_root_acl.rs` includes it by path; both move with it. Before dispatch the writer confirms the consumer list against the tree — `grep -rln capture_root` over `crates/cargo-tile/src` and `crates/cargo-tile/tests` — because Phases 5, 7, and 8 each add files to it.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-tile`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-tile`, `bash ~/.claude/scripts/delegate/verify.sh lint cargo-tile` green with the package tests unchanged; every file under `crates/cargo-tile/src/root_scan/` is under the style guide's line threshold or holds one cluster.

### Phase 15 — Types relocated by ownership, and `progress.rs` split  · status: todo

#### Work Order

**Goal:** Every type in `processes.rs` and `progress.rs` lives in the module that owns its sole constructor or single consumer, and `progress.rs` is split into anchor-named submodules.

**Spec:**
`crates/cargo-tile/src/processes.rs` (2848 non-test lines, 44 top-level types) holds process identity (`InvocationId`, `RunId`, `ProcessIdentity`), capture attribution (`CaptureAssociation`, `DirectCapture`, `NearestRegistration`), CPU measurement (`CpuBaseline`, `CpuPublication`, `CpuSmoothing`), and the `Census` scan; `crates/cargo-tile/src/progress.rs` (1217 non-test lines, 23 top-level types) holds capture-root lookup (`CaptureRoots`, `CaptureRoot`, `RegisteredRuns`) beside the `Progress` counter state.

First relocate by ownership: each type whose sole constructor or single consumer lives in another module moves there (the capture-root lookup types belong with the root scan module from Phase 14 if that is where they are constructed; check the constructor before moving). Then split what remains of `progress.rs` into `crates/cargo-tile/src/progress/mod.rs` (`Progress` and the `mod` block) plus anchor-named submodules, each under the line threshold or holding one cluster, inline tests travelling with their types. `tests/shim_registration.rs:39` (`#[path = "../src/progress.rs"]`) and `render.rs` / `settings.rs` / `app.rs` imports follow. `processes.rs` is only a source of moved-out types here; its split is Phase 16.

The type work belongs here too, because this phase owns `progress.rs` and the plan's `Option<T>` invariant has no other owner for it. `parse_state` (`:1086`) answers `Option<RunState>`, and its `None` is **two** outcomes, not one: a recognized `Finished` marker has retired a recognized counter, or the captured tail carried no activity this reader recognizes. Define the outcome around current progress evidence — the same distinction `CaptureRead::NoCurrentProgress` (`:188`) already draws — and keep both cases named and distinct; the earlier reading of this line, that absence means only an unrecognized state line, would give the replacement type a guarantee it does not hold.

The same treatment applies to the rest of this module's domain-owned answers, which are outcomes rather than missing values and are not foreign APIs that require `Option`: `log_pid` (`:1057`) — whether the log's name identifies a writer; `last_counter` (`:1120`) — whether the tail holds a recognized counter; `counter_at` (`:1190`) — whether a given line parses as one. Name each, preserving what its absence distinguishes, and convert at the caller. Everything else in this phase stays a move: these conversions are small enums over code the writer is already rewriting in place, so they cost one pass rather than a second one over the same files, and the phase does not split.

**Files:**
- `crates/cargo-tile/src/progress.rs` — becomes `crates/cargo-tile/src/progress/`
- `crates/cargo-tile/src/processes.rs` — types moved out by ownership
- `crates/cargo-tile/src/root_scan/` — receives the capture-root lookup types it constructs
- `crates/cargo-tile/src/render.rs`, `crates/cargo-tile/src/settings.rs`, `crates/cargo-tile/src/app.rs` — import paths
- `crates/cargo-tile/tests/shim_registration.rs`, `crates/cargo-tile/tests/summary_totals.rs`, `crates/cargo-tile/tests/capture_root_acl.rs` — `#[path]` includes; Phases 5 and 7 both copied the whole block from `shim_registration.rs:3`, so all three name `progress.rs` and `processes.rs`

**Seats:** 1 writer; nothing splits — relocation crosses every file named.
- `impl` — everything above, all three harness include blocks included; hub: `crates/cargo-tile/src/progress/mod.rs`
- `test` — opens as test: no new behavior; confirms every inline test travelled with its type
- `review` — reserve

**Constraints from prior phases:** Phase 14 renamed `capture_root` to `root_scan`; Phase 5 added subtree totals to `Census::groups`; Phase 8 added the unsupported-version diagnostic path in `registered_runs`; Phase 12 may have changed `parse_state`. All of it moves as-is.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-tile`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-tile`, `bash ~/.claude/scripts/delegate/verify.sh lint cargo-tile` green with the package tests otherwise unchanged; every file under `crates/cargo-tile/src/progress/` is under the line threshold or holds one cluster; each named outcome replacing an `Option` carries a case per distinct absence it now states, a retired-counter tail and an unrecognized tail among them.

### Phase 16 — `processes.rs` split  · status: todo

#### Work Order

**Goal:** `processes.rs` is split into anchor-named submodules — process identity, capture attribution, CPU measurement, and the `Census` scan — each under the line threshold or holding one cluster.

**Spec:**
After Phase 15's relocation, split `crates/cargo-tile/src/processes.rs` into `crates/cargo-tile/src/census/mod.rs` (the `mod` block and re-exports) plus one submodule per cluster. The directory is `census/` and the hub is `crates/cargo-tile/src/census/mod.rs`: `Census` is the scan the rest of the module is constructed around, so it is the anchor the directory takes its name from, the same rule Phase 14 applied to `RootScan`. The clusters are: process identity (`InvocationId`, `RunId`, `ProcessIdentity`), capture attribution (`CaptureAssociation`, `DirectCapture`, `NearestRegistration`), CPU measurement (`CpuBaseline`, `CpuPublication`, `CpuSmoothing`, `Measurement`), and the `Census` scan with `attribute_cpu`, `groups`, `group`, and `aggregate_cpu`. Inline tests travel with their types; visibility does not widen; `tests/shim_registration.rs:37` and `tests/summary_totals.rs` includes and every import follow.

The type work belongs here too, for the same reason it did in Phase 15. `Census::owning_cargo` (`:1611`) answers `Option<Pid>`, where the absence means no cargo ancestor was found for that process — a real attribution outcome, not a missing value. So do this module's other domain-owned answers: `CommandText::subcommand` (`:554`) — whether the command text names a subcommand at all; the argv classifiers `subcommand` (`:2779`) and `external_subcommand` (`:2817`) — whether the argument list identifies one, and whether it identifies an external one. None is a foreign API that requires `Option`. Name each outcome for what its absence states and convert at the caller. Everything else in this phase stays a split: these are small enums over code the writer is already moving in place, so they cost one pass rather than a second one over the same files, and the phase does not split.

**Files:**
- `crates/cargo-tile/src/processes.rs` — becomes `crates/cargo-tile/src/census/`
- `crates/cargo-tile/src/main.rs` — the `mod` declaration, which changes with the directory name
- `crates/cargo-tile/src/render.rs`, `crates/cargo-tile/src/settings.rs`, `crates/cargo-tile/src/app.rs`, `crates/cargo-tile/src/progress/`, `crates/cargo-tile/src/roster.rs`, `crates/cargo-tile/src/terminal.rs`, `crates/cargo-tile/src/tiles.rs` — import paths; the last three were absent from this list and each names `processes::`
- `crates/cargo-tile/tests/shim_registration.rs`, `crates/cargo-tile/tests/summary_totals.rs`, `crates/cargo-tile/tests/capture_root_acl.rs` — `#[path]` includes; all three carry a copy of the block at `shim_registration.rs:3`
- `crates/cargo-tile/tests/support/shared_capture.rs` — its own `processes` reference

**Seats:** 1 writer + 1 tester + reserve; nothing splits — one file is being divided.
- `impl` — everything above, every consumer and all three harness include blocks included; hub: `crates/cargo-tile/src/census/mod.rs`
- `test` — opens as test: no new behavior; confirms the package compiles against the new paths and that every inline test travelled with its type; no file edits
- `review` — reserve

**Constraints from prior phases:** Phase 15 moved types out of `processes.rs` by ownership and split `progress.rs`; Phase 5's subtree totals live in `Census::groups`; Phase 8's `CaptureDiagnostic` variant is in the attribution cluster. Before dispatch the writer confirms the consumer list against the tree — `grep -rln 'processes::' crates/cargo-tile/src crates/cargo-tile/tests`, plus the `mod` declaration in `main.rs` — because Phases 5 and 8 each add files to it.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-tile`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-tile`, `bash ~/.claude/scripts/delegate/verify.sh lint cargo-tile` green with the package tests unchanged; every file under the new directory is under the line threshold or holds one cluster.

### Phase 17 — Rollout on both machines: readers, shims, runner services  · status: todo

**Blocked by:** G2 — root on both machines: the linux-host session owns the NixOS runner units; the Mac's launchd plist and stale-tree deletion are the user's sudo; ssh to the Mac needs 1Password unlocked.

#### Work Order

**Goal:** Every installed `cargo-berth` reader on both machines reads the merge-extent record, every tile reader and runner shim is at the `v3` framing, the runner services reach the shared capture directory, and a CI job under each of `hana-linux-1`, `hana-linux-2`, and `hana-ci` shows its row in cargo tile.

**Spec:**
Inventory first: each machine, service label, authoritative configuration path (the NixOS runner units; the Mac runner's launchd plist), owner, and root reload steps; every `cargo-berth` reader — account, hook configuration path, resolved executable path, `CARGO_BERTH_EXECUTABLE` override — in its actual invocation environment. Run Phase 9's `reader_compat` target against every inventoried reader; upgrade each one that fails; only then let a new binary reconcile the shared ledger (where the record already exists, upgrade the remaining readers rather than editing the ledger). Deploy and restart the tile readers before the `v3` shim is installed through `install --all-accounts` and the runner hooks. Restarting a reader is itself an installation: `capture::stand_up` (`crates/cargo-tile/src/terminal.rs:149`) installs the shim on startup and `capture.auto_install` defaults to true (`constants.rs:838`), so restarting the first upgraded reader would push `v3` onto a machine whose other readers are still old — the opposite of the order this phase exists to hold. Set `auto_install = false` in each account's `config.toml` and leave the runner job-start hooks uninstalled **before** the first upgraded reader starts; keep both suppressed until every reader on that machine is upgraded and restarted; then install the `v3` shim deliberately and restore `auto_install` to its default and the hooks to their configured state. Record the suppressed and restored state per account in the rollout inventory, so a half-restored machine is visible rather than inferred. Then `status --all-accounts` (Phase 4) on each machine shows every runner toolchain installed with no orphan. Runner services: the Mac plist no longer sets `CARGO_TILE_ROOT`; the two stale `/var/lib/hana-ci/hana-linux-{1,2}/cargo-tile` trees are deleted by root; each account's installed shim is revalidated. Then a CI job under each of the three runner accounts produces its `[hana-linux-N]` or `[hana-ci]` row in cargo tile as seen from the ordinary host reader, and the rollout record (inventory, versions, dates) is appended to the as-built doc.

**Files:**
- `docs/cargo-tile/as-built/github-runners.md` — the rollout record

**Seats:** 1 writer + 1 tester + reserve — the deployment sequence and the doc are one lane; the preflight and post-deployment observations are independent of both and need no file edits.
- `impl` — machine inventory, deployment sequencing including the automatic-installation and hook suppression window and its restoration, the service changes through their authorized owners, and `docs/cargo-tile/as-built/github-runners.md`; hub: none
- `test` — the native package preflight on the Mac, `reader_compat` against every inventoried reader, verification that each account's installed shim bytes are unchanged from the moment suppression is set until the deliberate `v3` install, and the post-deployment shim-version inspection, `status --all-accounts`, and CI-row observations; no file edits
- `review` — reserve

**Constraints from prior phases:** Phase 4 provides `uninstall --all-accounts` and `status --all-accounts`; Phase 8 provides the `v3` framing and the downgrade refusal; Phase 9 provides `reader_compat`; Phases 10 and 11 must not have touched the shared ledger before this phase. Phase 3 left one deferred check that belongs to this rollout, and Phase 4 settled what it must show. An account whose resolved membership exceeds Darwin's runtime `setgroups` limit is **refused by name before its child starts** (`bounded_account_groups`, `crates/cargo-tile/src/hook.rs:983`): the list is never shortened, because a shortened supplementary-group list is the account's whole membership as far as the kernel is concerned and would silently cost it every access it holds through a dropped group. The refusal names the account, the resolved count, its primary gid, and the limit, and later accounts are still processed. Reducing the account's membership is the recovery, and it is documented in the crate README. Refusal and continuation already have unit and integration coverage; what has never been observed is the refusal against a real over-limit account, which needs an administrator to create one. That observation belongs to this run's deferred administrative smoke batch, not to a gate a delegate can pass.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth reader_compat` green against every inventoried reader; the native `cargo-tile` package suite green on the Mac before any rollout mutation (see the Delegation Context note on the three pre-existing `shim_registration` failures — they must be resolved first); `status --all-accounts` clean on both machines, **and** each inventoried shim inspected for its version line — `status` reports `Installed` from the shim marker and the real cargo's presence (`Hook::state`, `crates/cargo-tile/src/hook.rs:588`) without reading the framing version or comparing bytes, and `install` deliberately exits zero despite a per-account failure, so a partly failed refresh can leave a `v2` shim that passes `status` and still produces a CI row; the gate is each shim's version line matching the embedded shim, plus every installation report complete. Every macOS account within the runtime group limit installed with its full membership, and an account above it refused by name with its counts and its primary gid, the later accounts still processed — the over-limit case observed in the deferred administrative smoke batch, since creating such an account needs an administrator. Three CI rows observed in cargo tile; the as-built rollout record present, including the automatic-installation suppression window and its restoration per account.
