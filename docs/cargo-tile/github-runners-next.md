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
      capture.rs  root_scan/{mod.rs, inspected_directory.rs, root_history.rs, sweep_authority.rs}
      census/{mod.rs, process_identity.rs, direct_capture.rs, invocation_cpu_accounting.rs, command_text.rs, scan.rs}
      progress/{mod.rs, capture.rs, capture_roots.rs, capture_read.rs, capture_diagnostic.rs, registered_runs.rs}
      render.rs  settings.rs  constants.rs
      birth_stamp/{mod.rs, process_lifetime.rs, kernel_observation.rs, linux.rs, macos.rs}
    tests/
      shim_registration.rs                     #[path] modules over src/ + PTY + Python
      summary_totals.rs                        #[path] modules over src/ (Phase 5)
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
  - `crates/cargo-tile/src/hook.rs` — shim installation, account discovery, credential switching. Phase 4 made every one of these operation-neutral, so the install-specific names earlier phases used are gone. `HookAccount` `:76`, `HookOperation` `:89` with `completion` `:110`, `AccountHookOutcome` `:127`, `ToolchainHookOutcome` `:138`, `ToolchainHookReport` `:200` with `parse` `:209` and `protocol` `:277`, `AccountHookReport` `:314` with `incomplete` `:327` and `from_output` `:337`, `Hook` `:400`, `Startup` `:419`, `at_startup` `:465`, `HookState` `:514`, `HookOperationOutcome` `:531`, `HookDiscovery` `:556`, `Hook::reports` `:572` (an iterator — callers print each row before the next toolchain starts), `Hook::all` `:601`, `Hook::in_rustup_home` `:604`, `Hook::at` `:630`, `Hook::state` `:660` (the only unlocked inspection), `Hook::perform` `:678`, `Hook::install` `:702`, `Hook::ensure` `:741`, `Hook::remove` `:764`, `HookInstallationLock` `:780`, `StagedExecutable` `:819`, `stage_executable` `:837`, `system_accounts` `:864`, `account_groups` `:911`, the private `account_groups_with` `:941`, `run_account_hooks` `:986`, `run_account_hooks_with` `:1002`, `run_account_hook` `:1025`, `bounded_account_groups` `:1064` (macOS refusal), `account_credentials` `:1090`, `inspect_shim` `:1191`. Also holds `SHIM_SOURCE`, `SHIM_MARKER`, and three `unsafe_code` allowances at `:861`, `:908`, `:1087`.
  - `crates/cargo-tile/src/cargo-capture-shim.sh` — POSIX `sh` capture shim. `capture_parent=/tmp/cargo-tile` `:53` (the fixed shared location), `case $first in` `:80` (subcommand classification; `install` takes the capture path), publication framing `printf '%s\000' cargo-tile-v3 …` `:299` (Phase 8 raised the framing from `v2`).
  - `crates/cargo-tile/src/root_scan/mod.rs` — hub: `RootScan:58`, `sweep:209` (returns `SweepCounts`), `access:240`, `revalidate_paths:268`, the `mod` block and re-exports; `SweepCounts` and `RegistrationObservation` have no hub re-export
  - `crates/cargo-tile/src/root_scan/inspected_directory.rs` — `CaptureFailure` consumer at `:57`; `SharedDirectoryState:133`, `SharedCaptureDirectory::inspect:153`, Linux `Inventory::sample:289`, Darwin `Inventory::sample:311`; the FIFO test allowance is at `:708`
  - `crates/cargo-tile/src/root_scan/root_history.rs` — `RootHistory` and `TreeIdentity`
  - `crates/cargo-tile/src/root_scan/sweep_authority.rs` — `SweepAuthority:31`, `SweepAuthority::sweep:64` (at `:69–74` it counts every non-`Complete` `Enumeration` outcome of the registration and log samples directly into `incomplete_inventories`), payload-free `SweepAdmissionRefusal:150` with exactly three variants `ForeignRoot`, `EffectiveUserUnavailable`, `AccessFailure`, `SweepCounts:198` (`pub(crate)`, four private fields, `#[cfg(test)]` accessors `removed_files`, `skipped_pair_attempts`, `incomplete_inventories`, `unavailable_roots`); no `enumeration_refusal` function, no `EnumerationIncomplete` sweep variant, no ACL inspector, and no production `unsafe` allowance remains
  - `crates/cargo-tile/src/render.rs` — grid and popup drawing, and the owner of the render-only types: `CaptureAccount:161`, `CaptureContext:170`, `CounterState:181`, `impl From<&CaptureLookup> for CounterState` `:199` (the only lookup-to-gauge conversion; `CaptureLookup::working` and `RunState::working` no longer exist), `SummaryDetail:219`; `summary_rows` `:828`, `heading_gauge` and `draw_settings` located by symbol. Also holds `GroupingIdentity`.
  - `crates/cargo-tile/src/census/` — the process census as six files: `mod.rs` (hub; thirteen re-exports for consumers outside `census`), `process_identity.rs`, `direct_capture.rs`, `invocation_cpu_accounting.rs` (`Measurement<T>`, `MeasurementAbsence`, `InvocationMeasurements`, the five renamed CPU accounting types), `command_text.rs` (`CommandText`, `RowAbsence`, `CargoArgvAbsence`, the subcommand outcome types), and `scan.rs` (`Census` with every scan, attribution, row, and grouping method, `CargoAncestry`, the two `cfg(test)` group adapters). Inline tests sit beside the code they exercise.
  - `crates/cargo-tile/src/progress/` — the former `progress.rs` is deleted. `mod.rs` (62 lines) is the hub: `Progress`, the five `mod` declarations, and its inline percentage test; it re-exports none of the relocated types, so consumers import each from its owning submodule. `capture.rs` (1,999 lines with tests, about 523 production lines, one coupled cluster): `CaptureRootIndex`, `CaptureKey`, `CaptureGeneration`, `CaptureSelection`, `ConfirmedCapture`, `Capture`, `LogWriter`, `legacy_read:447`, `log_pid:508`. `capture_roots.rs` (281): `CaptureRoot:55` with `account`, `CaptureCleanup`, `CaptureParent`, `CaptureRoots:102` with `from_parent`, `AccountCaptureDirectory` with `inspect`, `RootReadStatus`, `AccountName`. `capture_read.rs` (619): `Phase`, `RunState`, `CaptureLookup`, `CaptureRead:76`, `Counter`, `CurrentProgress::{Active, RetiredCounter, Unrecognized}`, `TailCounter`, `ParsedCounter`, `LeadingNumber::{Digits, NoLeadingDigits, Overflow}`, `parse_state:160`, `last_counter:201`, `counter_at:276`, `leading_number:319`; the `CaptureRead` conversion folds both parser absences into `NoCurrentProgress`. `capture_diagnostic.rs` (68): `CaptureDiagnostic` (its `EnumerationIncomplete` variant survives and reaches Settings), `CaptureFailure`, `PathFailure`. `registered_runs.rs` (202): `RegisteredRuns`, `RegisteredRun`, `RegistrationName`, `registered_runs:84`.
  - `crates/cargo-tile/src/app.rs` — application state. `App.capture_note` `:187` holds a `CaptureStartupNotice` (defined `:153`), initialized to `CaptureStartupNotice::Quiet` at `:236` and assigned at `:1067`; `settings::rows` (`settings.rs:158`) consumes it at `settings.rs:388` and `:396`. Phase 8 replaced the former `Option<String>` with that semantic type; the field name still says `note` while the type says notice.
  - `crates/cargo-tile/src/navigation.rs` — `settings_overlay` `:48`: keyboard navigation mutates the settings row viewport directly.
  - `crates/cargo-tile/src/interaction.rs` — `handle_click` `:45`, `overlay_row` `:54`: a click sets the settings row the same way.
  - `crates/cargo-tile/tests/summary_totals.rs` — Phase 5's integration binary. Carries its own copy of the `#[path]` block (29 declarations from `:3`, naming `census/mod.rs`, `progress/mod.rs`, `root_scan/mod.rs`; `shim_registration.rs` and `capture_root_acl.rs` carry the same three), and drives private assembly through the two `cfg(test)` adapters rather than through the PTY.
  - `crates/cargo-tile/src/registration.rs` — `cargo-tile-v2` NUL framing. `Registration::parse` `:31`, `ParseError` `:229`.
  - `crates/cargo-tile/src/birth_stamp/mod.rs` — hub: `BirthStamp:31`, `compare:91`, the `mod` block and re-exports; every pre-existing consumer export keeps its name
  - `crates/cargo-tile/src/birth_stamp/process_lifetime.rs` — `ProcessLifetime:15`, `LifetimeEvidence:43`
  - `crates/cargo-tile/src/birth_stamp/kernel_observation.rs` — `PathFailure` consumer at `:18`; `KernelObservation:34`, `IdentityEvidence:59`, `Verification:78`, `observe:88`, `boot_verification:111` (re-exported from the hub), `observe_process:135`
  - `crates/cargo-tile/src/birth_stamp/macos.rs` — Darwin `observe` `:48`, reaching methods private to the parent module; `allow(unsafe_code)` sysctl at `:118`. Does not compile on Linux.
  - `crates/cargo-tile/src/constants.rs` — every constant with its rationale; carries `cfg(target_os)` gates. New ACL and framing-version constants go here.
  - `crates/tui_pane/src/overlays/settings.rs` — row-to-rendered-line map for mouse selection. `line_targets` field `:107`, `set_line_targets` `:220`, `row_at` `:238`, `line_for_selection` `:249`. **Drift:** the plan cites `:238` for both `row_at` and `line_for_selection`; `:238` is `row_at`, `line_for_selection` is at `:249`.
  - `crates/cargo-berth/src/gate/install.rs` — `EXECUTABLE_ENVIRONMENT: &str = "CARGO_BERTH_EXECUTABLE"` `:32`, the executable override readers resolve through.
  - `crates/cargo-berth/src/ledger/journal.rs` — `JournalOperation::MergeExtentObserved` `:362` (the record older binaries reject), `JournalOperation::EvidenceRevalidated` `:422` (carries `status: IntegrationEvidenceStatus` and `edit_blocking_status: EditBlockingStatus`).
  - `crates/cargo-berth/src/reconcile.rs` — `prepare_reconciliation_transaction` `:968`, which calls `derive_merge_extents` (defined `:1040`) at `:1005` and only then `append_evidence_operations` (defined `:1977`) at `:1014`; `prepare_gate_reconciliation` `:1365` calls the same appender at `:1412`; `append_evidence_and_retention` `:1902`. **Phase 9 reversed the former order: every evidence record is now built after extent derivation**, so it reads the extent the transaction is about to persist rather than the previous one.
  - `crates/cargo-berth/src/reservation/lifecycle.rs` — `IntegrationEvidenceStatus::edit_blocking_status` `:168`, the checkpoint-only derivation, now `#[cfg(test)]` (`:165`): Phase 9 removed its last production caller and it survives as the historical-replay comparison only.
  - `crates/cargo-berth/src/reservation/record.rs` — `Reservation::edit_blocking_status` `:363`, the decision live blocking checks use and the one every persisted evidence record now derives from; `Reservation::with_merge_extent` `:281` produces the post-transaction reservation it is asked about.
  - `crates/cargo-berth/src/verb/release.rs` — `outstanding_operation` `:450`, whose unreadable-trunk early return at `:458` persists an `ObjectUnknown` record through `evidence_operation` `:668`; `released_evidence_operation` `:572`, `already_settled_operation` `:619`.
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
  - `cargo-tile` — `crates/cargo-tile/tests/`. Integration binaries: `shim_registration` (embedded Python 3 harness driving the built binary under a PTY; `#[path]`-includes all of `src/` plus `support/shared_capture.rs`), `shim_modes` (Rust only), `cli_lifecycle` (Rust only), `summary_totals` (Phase 5; carries its own full `#[path]` block), `capture_root_acl` (Phase 7; whole-file `#![cfg(target_os = "macos")]`). `support/shared_capture.rs` is **not** its own binary. The crate is binary-only, so unit tests of crate items are inline `#[cfg(test)]` modules in `src/`.
  - `cargo-berth` — `crates/cargo-berth/tests/`. Integration binaries: `answers`, `board`, `drift`, `edges`, `engine_instructions`, `front_end_corpus`, `gate`, `hooks`, `ledger`, `lifecycle`, `liveness`, `output_contract`, `overlap`, `presentation`, `reader_compat` (Phase 9). Shared helpers in `tests/support/` (`reader_compat_hooks.rs`, `#[path]`-included by both `reader_compat` and `hooks`); fixture data in `tests/fixtures/front_end_corpus.json` and the frozen ledger bundle under `tests/fixtures/reader_compat/`. All Rust; no Python harness.
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
  - **Registration framing.** Records are NUL-framed with the `cargo-tile-v3` magic (`REGISTRATION_MAGIC`, `constants.rs:980`); generation and log fields must be single basenames and the argument count must match. Phase 8 raised the framing from v2, which survives as `REGISTRATION_V2_MAGIC` (`:990`) so a record written by an older shim is recognized as skew rather than as corruption. `Registration::parse` yields `Versioned(RegistrationCandidate)` or `Legacy`; a legacy record may annotate a row and never sources one.
  - **Identity proof.** `RegistrationCandidate::verify_observation(pid, &KernelObservation)` is the only route to `VerifiedRegistration`. Production `KernelObservation` values come only from `birth_stamp::observe(pid)` and stay pid-bound with private fields; arbitrary observation injection exists only under `cfg(test)`. `Verification::Unknown` authorizes neither a row nor a deletion.
  - **Who may unlink a registration.** Only the reader's own uid directory is swept (`CaptureCleanup::Here`, `effective_user()` equal to the directory's uid). A foreign-owned directory is reported and never read. Another account's dead captures are reaped by that account's next shim invocation.
  - **Sweep admission.** `RootScan::access` admits a sweep when the effective uid owns the root, `state`, and `state/pids`, the retained and reopened directory identities agree, and scan continuity is established; directory write grants and incomplete inventories do not veto independently proved pairs, each unlink requires fresh registration/liveness evidence and file revalidation, and sweep outcomes produce no Settings cleanup text.
  - **Directory opening.** Every directory is opened `O_NOFOLLOW` relative to a held handle. Ancestor symlinks are resolved once by `canonical_capture_path`; the final component never is.
  - **Admin install protocol.** `install --all-accounts` requires root, reads the account database with `getpwent`, and stages a root-owned `0755` copy of the running executable under the sticky `/tmp` parent. Since Phase 4 the same protocol carries `install`, `uninstall`, and `status`: the child runs once per account with `HOME`/`RUSTUP_HOME` set and the hidden `--account-hook-report` flag (`constants.rs:813`), printing one `<toolchain>\t<result>` line per toolchain and flushing each row before the next toolchain starts, so a terminated child still reports what it finished. Credentials are applied together in `pre_exec`: `setgroups` first, then `setgid`, then `setuid`. No root-owned file operation reaches an account's tree. `account_credentials` (`hook.rs:1090`) resolves the account's full membership through `account_groups` (`:911`), which grows its `getgrouplist` buffer geometrically from 32 entries to a 65,536-entry ceiling: glibc reports the required total through the count pointer, Darwin can leave the supplied count unchanged, and both cases resolve. Beyond the ceiling, a failure the count cannot size, or a negative count, it returns an error naming the account rather than a truncated list. On macOS `bounded_account_groups` (`:1064`) then refuses a resolved membership larger than the runtime `setgroups` limit, naming the account, the resolved count, the primary gid, and the limit; it never shortens the list, because the kernel treats a supplementary-group list as the account's whole membership rather than a cache. Later accounts are still processed. `Hook::install` and `Hook::remove` each take `HookInstallationLock` (exclusive creation of `cargo-tile-shim.lock`, removed on drop) and inspect state under it; `Hook::state` is the only unlocked read. `Hook::remove` answers `Ok(HookOperationOutcome::Orphaned)` for a shim whose real cargo is gone — a rendered row and an incomplete removal, not an error.
  - **Exit-status asymmetry, deliberate.** `install` reports a failing toolchain, continues, and exits **zero**, because runner job-start hooks call it and must not fail over capture setup. `uninstall` exits **nonzero** on a removal failure and must keep doing so, or automation reads a partial uninstall as complete.
  - **Settings pane.** Does no filesystem access; everything it shows was observed on the scanner worker.
  - **Platform.** `birth_stamp/macos.rs` and the Darwin directory enumeration do not compile on Linux; Linux CI compiles but cannot exercise the sysctl path or the twenty native `capture_root_acl` cases. No ACL inspector remains; the only non-Linux root-scan code is the Darwin `Inventory::sample` selection in `root_scan/inspected_directory.rs:311`. `unsafe_code` is denied workspace-wide with per-item `allow(reason)` plus a `// SAFETY:` comment at `hook.rs:861/908/1087` and `birth_stamp/macos.rs:118`; the root-scan module's only allowance is the test-module FIFO at `root_scan/inspected_directory.rs:708`. The crate does compile and test natively on macOS, and the workflow carries a `macos-latest` job (`.github/workflows/ci.yml:348`), so a macOS-only target has a native route as well as CI. **The native `cargo-tile` package suite is not green on macOS today:** three `shim_registration` cases — `fifo_removal_failure_preserves_original_cargo_and_unowned_directory`, `publication_links_a_complete_tmp_file_into_place`, and `stale_invocation_fifo_still_publishes_registration_and_captures_stderr` — fail on the current tree and equally on a `git archive` of the Phase 2 checkpoint (1449 passed / 3 failed on each). They predate Phase 3, no phase owns them, and they are recorded as a next item; any phase whose gate names a green native suite depends on that item first.
  - **Constants.** Every constant lives in `crates/cargo-tile/src/constants.rs` with a rationale. No magic values inline.
  - **Test layout.** `cargo-tile` is binary-only: crate-item tests are inline `#[cfg(test)]` modules, and `tests/` reaches sources through `#[path]` modules and drives the built binary. Two uids are never required by a test. Any file move must carry the `#[path]` declarations at the top of **every** harness that has them — `shim_registration.rs:3`, `summary_totals.rs:3`, and `capture_root_acl.rs` once Phase 7 creates it — not just the first. Each is a full copy of the block, so a rename missed in one of them breaks that binary alone.
  - **Scratch checkouts get their own target directory.** A scratch tree built beside the live one (a `git archive` of an earlier commit, a control build, an older executable for a compatibility check) must be given its own `CARGO_TARGET_DIR`. Sharing one makes a focused build reuse a stale binary from the other tree, so a result is attributed to the wrong source. Observed in Phase 5, where a focused check reused a stale test binary and failed against unchanged source.
  - **Package formatting is one owner's, after the writers pause.** `verify.sh lint <pkg>` formats the whole package, not the caller's files, so a lint run by one lane rewrites another lane's open files and the file partition stops holding. Assign package formatting and lint to a single owner, to be done only once every editing lane has paused; reconcile whatever it changes through each file's own owner before the combined gate. A writing seat's Verification section lists `check` and its scoped `test` lines and never `lint`.
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

### Phase 5 — A promoted summary row reports its whole subtree  · status: done

#### As-built

`Census::groups` (`crates/cargo-tile/src/processes.rs:2003`) prepares, after group assembly, each promoted row's subtree CPU total — its own invocation bucket plus every nested cargo attribution beneath it exactly once, excluding the hidden driver and any sibling subtree. `summary_rows` (`crates/cargo-tile/src/render.rs:684`) draws that prepared total in place of the row's own share; command-view per-row measurements are unchanged.

`subtree_cpu` (`processes.rs:2430`) is a private free function with one call site, taking `&HashSet<InvocationId>`. It seeds a member from its pid bucket only when that member's invocation identity is in the set, and `Measurement::Unavailable(MeasurementAbsence::Unproven)` otherwise. `groups` builds that `process_rows` set at `:2022`; both the subtree seed (`:2029`) and the per-row CPU overwrite (`:2044`) gate on it, so an unestablished contribution anywhere in a subtree makes the whole total unavailable rather than a number. Aggregation keys on invocation identity, never pid.

Private assembly is reachable from an integration binary through two test-only adapters: `groups_with_registration_rows_for_test(system, parents, shares, registration_rows, omitted_process_pids)` (`:2932`) and `groups_with_cpu_for_test` (`:2922`), now a wrapper over it. `Census` carries a `#[cfg(test)] registration_rows: Vec<CargoProcess>` field whose rows `groups` appends *after* the `process_rows` snapshot — that ordering is what makes an injected row unproven. No production visibility widens.

**Files:**
- `crates/cargo-tile/src/processes.rs` — subtree totals in `Census::groups`, `subtree_cpu`, the `cfg(test)` field and both adapters
- `crates/cargo-tile/src/render.rs` — `summary_rows` reads the prepared total
- `crates/cargo-tile/src/roster.rs`, `crates/cargo-tile/src/terminal.rs` — `CargoProcess` construction sites carrying the subtree total
- `crates/cargo-tile/tests/summary_totals.rs` — integration binary: three regressions covering descendants counted once, a sibling subtree excluded, and an unavailable descendant

**Binds later work:** `subtree_cpu` stays beside its single call site, the invocation-identity gate stays on both call sites, registration injection stays after the `process_rows` snapshot, and the `cfg(test)` field and both adapters travel with the `Census` cluster — the `processes.rs` split inherits all of it, with the three regressions still passing. `tests/summary_totals.rs` carries its own full copy of the `#[path]` block (29 declarations from `:3`), so a file rename or split must update every harness's block, not only `shim_registration.rs`'s; any gate naming the `cargo-tile` package now builds this target too. Observing the total as a number in the running grid falls to the rollout phase, where "observed" and "blocked on CPU availability" are distinct results.

**Gotchas:**
- Two invocation identities can share one pid when a registration fallback and a real process invocation differ only by command; anything keyed on pid charges that bucket twice.
- A scratch checkout beside the live tree needs its own `CARGO_TARGET_DIR`; sharing one makes a focused run reuse a stale test binary and fail against unchanged source.
- Per-process CPU reads unavailable for every cargo invocation on this Linux host, on this tree and the pre-phase tree alike; the blank column is not a regression from this work.

**Ruled out:** Repairing the command row group lead's pid-summed double count here — command rows stay unchanged and that double count pre-dates this work.

### Phase 6 — Every Settings account line is reachable  · status: done

#### As-built

The Settings popup scrolls: every account line, every wrapped diagnostic continuation line, and every settings row below them is reachable at any terminal height, and the selected row stays visible across navigation and resize. `SettingsPane::render_lines(&mut self, frame, lines)` is the pane's drawing entry point — it slices the prepared lines to the configured viewport, paints them, and records the offset it painted with; both consumers draw through it. Hit testing resolves against that recorded paint offset rather than the live viewport offset, so a click arriving in the same input batch as a navigation key selects the row actually on screen; before the first paint every click misses. The overlay's three optional answers are named outcomes — `SettingsLineTarget::{Row, Decoration}`, `SettingsSelectionLine::{Rendered, NotRendered}`, and `SettingsRowHit::{Row, Missed}`, which the framework hit test turns into a modal miss — and row identity at the store boundary is `SettingsRowIdentity::{Decoration, Selectable(SettingsRowPayload)}` in place of an optional payload that stood for both a heading and a missing id. `SettingsPane::new` stays a `const fn`; the row-line ranges are a `BTreeMap`.

**Files:**
- `crates/tui_pane/src/overlays/settings.rs` — the pane: viewport, recorded draw offset, `render_lines`, the three named outcomes, `const fn new`
- `crates/tui_pane/src/settings_store/row.rs` — `SettingsRowIdentity` and the documented rule that a selectable payload is the row's zero-based position among selectable rows
- `crates/tui_pane/src/framework/mod.rs` — the settings hit test consuming `SettingsRowHit`
- `crates/cargo-tile/src/render.rs` — the settings draw: geometry, scroll update, then `render_lines`
- `crates/cargo-tile/src/navigation.rs`, `crates/cargo-tile/src/interaction.rs` — the keyboard and click paths into the settings viewport
- `crates/cargo-tile/src/settings.rs` — the capture section's row text
- `crates/cargo-port/src/tui/settings.rs` — the second consumer, drawing through the shared entry point
- `crates/tui_pane/tests/settings_scroll.rs` — pane-level scrolling, selection, and the recorded-offset click case
- `crates/cargo-tile/tests/shim_registration.rs` — the PTY scenario proving the drawn popup follows the offset

**Binds later work:** A new diagnostic string in the Settings popup needs no positioning work — a long string wraps into continuation lines that each carry their row's identity — but its row must be numbered by the row builder, never by a hand-chosen payload.

**Gotchas:**
- The recorded offset is stamped by the content paint, so any consumer that paints prepared settings lines some other way has hit testing that silently misses. Paint through `SettingsPane::render_lines`.
- A selectable row's payload must equal its zero-based position among selectable rows, decoration excluded. The positional fallback that used to hide a wrong payload is gone, so a hand-chosen id highlights and scrolls to the wrong row.
- `HashMap::new` is not `const`, so a `HashMap` field in a pane costs the `const` constructor; `BTreeMap` carries the same map and keeps it.

**Ruled out:** Wrapping the mutable viewport accessor to force offset recording through it — it breaks the `const` callers in the wheel-dispatch and hover paths.

### Phase 7 — The ownership check closes the macOS ACL hole  · status: done

#### As-built

- `AclWriteAccess` in `capture_root.rs` states a directory's ACL write evidence with no bare `Option`: `NoNonOwnerWriteGrant` exists on every platform, `NonOwnerWriteGrant` and `InspectionFailed(CaptureFailure)` only on macOS. On Linux the permitting state is the only state and no ACL is read.
- The Darwin inspector reads the ACL through the held directory descriptor (`acl_get_fd`, `acl_get_entry`, `acl_get_tag_type`, `acl_get_permset`, `acl_get_perm_np`, `acl_get_qualifier`, `mbr_uuid_to_id`, `acl_free`) under one narrowly scoped `allow(unsafe_code)`, freeing the ACL on every exit path and each qualifier after its membership lookup. Only an allow entry whose principal is a user id equal to the directory's owner is exempt; any other principal holding any write-class permission is a grant.
- `InspectedDirectory` carries an `acl_write_access` field and `InspectedDirectoryMetadata::refusals` takes an `&AclWriteAccess`. Inspection runs when a directory is opened and again on every revalidation, so a grant added after open is caught before any unlink. `RootOwner` continues to carry ownership on its own — no ACL state replaces it.
- `CleanupRefusal` gained a macOS-only `AclWritableByOthers(PathBuf)`, rendered by `cleanup_refusal` in `settings.rs` as `ACL grants non-owner write access — cleanup disabled: <path>`. An inspection failure surfaces instead as `Access(PathFailure)`, carrying the path and the original diagnostic while owner and file list survive.
- `revalidate_paths` returns `Result<(), CleanupRefusal>`; the private `revalidate` wrapper is gone and both unlink sites consume the typed refusal directly.
- `tests/capture_root_acl.rs` is a whole-file macOS-gated integration target of fifteen cases driving real ACLs through `/bin/chmod` `+a`, `+a#`, `-a#`, and `-N`, asserting mode bits stay `0o700`. Test-only reach into production is two `cfg(all(test, target_os = "macos"))` adapters — `RootScan::fail_acl_inspection_for_test` and `cleanup_refusal_for_test` — because the target cannot otherwise inject an inspection failure or reach a private renderer.

**Files:**
- `crates/cargo-tile/src/capture_root.rs` — the root scan, the ACL evidence type, the native inspector, and the typed revalidation
- `crates/cargo-tile/src/constants.rs` — the `CAPTURE_ACL_*` tag, iterator, principal, and write-mask constants, the refusal wording, and two `cfg(all(test, target_os = "macos"))` fixture arrays
- `crates/cargo-tile/src/settings.rs` — the only exhaustive match on `CleanupRefusal`, plus a macOS test-only renderer adapter
- `crates/cargo-tile/tests/capture_root_acl.rs` — the fifteen ACL cases

**Binds later work:** `cleanup_refusal` is the only exhaustive match on `CleanupRefusal`; any second exhaustive match must gate the macOS-only arm or it compiles on Linux and fails on macOS alone. `revalidate_paths`'s typed `Result<(), CleanupRefusal>` is what the sweep path consumes. `acl_write_access` and `refusals`'s `&AclWriteAccess` parameter are hub-adjacent signatures inside `capture_root.rs`: the phase that splits the capture-root module keeps the native inspector with its unsafe allowance together and leaves the `CAPTURE_ACL_*` constants in `constants.rs`. `tests/capture_root_acl.rs` uses the crate's `#[path = "../src/…"]` module preamble, so a phase adding a module to `src/` may need to add it there too. Only a native macOS run naming all fifteen cases is evidence about this target.

**Gotchas:**
- An empty ACL is not an absent one: an absent ACL returns a null pointer with `ENOENT`, while an explicitly emptied ACL is a live allocation whose first `acl_get_entry` reports exhaustion. Both reduce to `NoNonOwnerWriteGrant`. `/bin/chmod -a# 0` on the sole entry leaves a zero-entry ACL attached; `-N` removes the ACL itself. The two are distinct fixture operations.
- Darwin's `acl_get_perm_np` accepts a combined multi-bit permission mask and answers whether any bit is present. No Linux check can confirm this.
- `tests/capture_root_acl.rs` carries a whole-file `#![cfg(target_os = "macos")]`, so a Linux gate compiles none of its bodies and a focused Linux run exits 4 for zero tests. A green Linux gate is not evidence about that file.
- Every harness carrying a `#[path]` include block carries a full copy of it, and this one sits two lines lower than the other two because of its whole-file attribute: `birth_stamp` at `:9`, `capture_root` at `:13`, `processes` at `:39`, `progress` at `:41`.
- A refusal first observed during a sweep blocks the unlink immediately but reaches Settings one scan late: `progress.rs:707` assigns `status.cleanup` before `sweep_ended` runs. This is not specific to ACL refusals — `Changed` and `Access` already behaved this way.

**Ruled out:**
- Fixing the one-scan-late refusal here — the fix changes when the status snapshot is taken relative to sweeping, which affects every refusal kind and predates this work.
- Production adapters to reach a descriptor-query `EINVAL` case or a group gid numerically equal to the owner uid — the behavior is fail-closed in both directions.
- Checking the ACL principal against the owner's uid alone without the tag kind — an allow entry for a group whose gid equals the owner's uid would then read as the owner.

### Phase 8 — Registration framing v3 with a version-skew diagnostic  · status: done

#### As-built

Registration records are framed `cargo-tile-v3` and `SUPPORTED_REGISTRATION_VERSION` is 3. `Registration::parse` classifies the bounded opening header through `check_version` — which scans at most `REGISTRATION_VERSION_HEADER_BYTES + 1` bytes for the field separator — before it consults payload size or any field: a header naming a version above the supported one yields `ParseError::UnsupportedVersion { encountered: u64 }` whatever the payload's size, supported, malformed and legacy records over `CAPTURE_REGISTRATION_BYTES` still yield `ParseError::TooLarge`, and a record with no separator in the window falls through to the legacy `v2` parse. `capture_root::read_registration_file` reads one byte past the cap, classifies the bytes it already holds, and returns them for a newer header rather than the generic unreadable error, so the version reaches the reader.

`progress::registered_runs` maps that error to `CaptureDiagnostic::UnsupportedRegistrationVersion { path, encountered, supported }` and continues **before** inserting into the generation map `sweep_ended` consults, so an unsupported record never becomes a `RegisteredRun`, never gains cleanup authority, and neither it nor its log is unlinked; `settings::capture_diagnostic` renders the registration path, both versions, and upgrade-and-restart guidance.

The installed shim carries a version line prefixed `# cargo-tile-shim-version: `. `hook::shim_version` inspects the opening `SHIM_MARKER_SEARCH_BYTES` and requires a declaration to terminate inside that window; an unterminated declaration, or a trailing fragment that is a proper prefix of `SHIM_VERSION_PREFIX`, is `io::ErrorKind::InvalidData` carrying `incomplete shim version line`. LF and CRLF both parse; duplicate and malformed declarations keep their prior handling. Under the installation lock, `install` and `ensure` answer `HookOperationOutcome::DowngradeRefused { installed, supported }` for a shim declaring a newer version, leaving its bytes, its saved real cargo, their modes and their mtimes untouched and no staging or lock file behind; the CLI prints `downgrade refused -- newer shim vN kept; this reader supports vM; upgrade and restart the reader`.

`App.capture_note` is `CaptureStartupNotice` with four cases — `Quiet`, `InstallationFailed`, `NewerShimKept`, `NewerShimKeptWithFailures { kept, failures }` — fed by `Startup.kept_newer: Vec<NewerShim>`, where `NewerShim` carries the toolchain, the installed version and the supported one. `capture::stand_up` produces it; the grid's startup notice and the Settings pane both render it, so a kept newer shim is distinguishable from an installation failure.

**Files:**
- `crates/cargo-tile/src/registration.rs` — the framed record parser; owns `check_version`, `pub(crate)` for its one in-crate consumer
- `crates/cargo-tile/src/capture_root.rs` — the descriptor read that classifies before rejecting size
- `crates/cargo-tile/src/progress.rs` — the version diagnostic and the retention that precedes the generation map
- `crates/cargo-tile/src/hook.rs` — shim version inspection and the install refusal
- `crates/cargo-tile/src/app.rs` — `CaptureStartupNotice`
- `crates/cargo-tile/src/capture.rs`, `settings.rs`, `cli.rs`, `processes.rs`, `constants.rs`, `cargo-capture-shim.sh` — the notice's producer and renderers, the CLI reporting, and the shim's own version line
- `crates/cargo-tile/tests/shim_registration.rs`, `crates/cargo-tile/tests/support/shared_capture.rs`, `crates/cargo-tile/tests/shim_modes.rs` — the scenarios and their fixtures

**Binds later work:** `capture_root.rs` imports `registration::{check_version, ParseError}` inside `read_registration_file`; both belong to the directory-inspection cluster and travel with it for the lane that moves that file into `crates/cargo-tile/src/root_scan/`. Rollout order for the native smoke: readers are deployed and restarted before the `v3` shim is installed.

**Gotchas:**
- A bounded read is not a bounded parse: classify the header from the bounded opening first, decide about the payload second. The inverse ordering read a shim declaring 30 as 3, and reported a newer oversized record as unreadable.
- `shim_version` refuses a trailing fragment that is a proper prefix of `SHIM_VERSION_PREFIX` — a bare `#` ending the inspected opening is enough, even when a complete declaration was already read earlier in the window. The shim's declaration sits near byte 44 and `SHIM_MARKER_SEARCH_BYTES` is 1024, so this does not fire today; a future shim edit that lands a comment `#` exactly at the boundary would make install refuse a correct shim.
- The truncated-declaration refusal carries no upgrade guidance, because a reader that cannot read the version cannot know the shim is newer; this matches how malformed and duplicate declarations report.
- `RegistrationObservation.bytes` may hold a truncated payload for a newer-framed record over the cap. Only the version dispatch reads it, and such a record never becomes a `RegisteredRun`.

**Ruled out:**
- Parsing a truncated version declaration from its visible prefix — it reads a newer shim as an older one and destroys it.
- A sleep, a retry, or a serialized resize-then-input step in the settings fixture — it avoids a suspected schedule instead of repairing input loss, whose cause is Crossterm 0.29 returning from a resize without retaining its position in the `EPOLLET` readiness batch.

### Phase 9 — cargo-berth readiness: reader compatibility, reservation extents, and an accurate evidence decision  · status: done

#### As-built

`board --reservation <id> --json` reports `race_extent` and `merge_extent` under `payload.data` in the snapshot sections' shape; an unresolvable trunk yields `unavailable` carrying the previously observed extent as `retained_evidence`. The success variant is boxed (`Snapshot(Box<ReservationReportSnapshot>)`) for the enum-size lint; the JSON shape is unchanged because the variant is untagged.

Every persisted evidence record's `edit_blocking_status` equals the live answer at the moment it is written: `append_evidence_operations` (`reconcile.rs:1977`) runs after `derive_merge_extents`, from `prepare_reconciliation_transaction` (`:1014`) and `prepare_gate_reconciliation` (`:1412`), and derives through `Reservation::edit_blocking_status` (`record.rs:363`) applied to the post-transaction reservation built by `with_merge_extent` (`record.rs:281`). `append_evidence_operations` partitions the four scoped-patch scheduling operations out, appends evidence, then restores them, so a plan's durable retry priorities survive the reordering. `IntegrationEvidenceStatus::edit_blocking_status` (`lifecycle.rs:168`) answered `Clear` from checkpoint integration alone; it is now `#[cfg(test)]` (`:165`), reachable by no production caller and surviving only as the historical-replay comparison. `release::outstanding_operation` (`release.rs:450`) uses the same derivation; its unreadable-trunk return (`:458`) keeps `object_unknown` visible and lets the retained merge extent's protection alone decide clear versus blocking, where it previously forced `blocking`. Historical records replay byte-unchanged.

`tests/reader_compat.rs` drives a chosen executable against a frozen ledger restored from a Git bundle into its own temporary tree and compares five results — `board`, `check`, and `drift` responses plus both hook payloads — against recorded expectations; the restored root is canonicalized before comparison so a temporary root reached through a symlinked or `..`-bearing parent still matches. Expected-response construction lives in `tests/support/reader_compat_hooks.rs`, `#[path]`-included by both `reader_compat.rs` and `hooks.rs`, whose assertions are unchanged.

**Files:**
- `crates/cargo-berth/src/board/report.rs`, `src/output.rs`, `src/verb/board.rs` — the extents on a single reservation's report and its output types
- `crates/cargo-berth/src/reconcile.rs` — evidence appended after extent derivation, scheduling operations preserved across the reorder
- `crates/cargo-berth/src/reservation/record.rs` — `edit_blocking_status`, the one derivation every persisted record uses, and `with_merge_extent`
- `crates/cargo-berth/src/reservation/lifecycle.rs` — the historical checkpoint-only derivation, test-only
- `crates/cargo-berth/src/verb/release.rs` — `outstanding_operation`, its unreadable-trunk return, `evidence_operation` (`:668`)
- `crates/cargo-berth/tests/reader_compat.rs`, `tests/support/reader_compat_hooks.rs`, `tests/fixtures/reader_compat/` (with `README.md`) — the compatibility target, its shared helpers, the frozen bundle
- `crates/cargo-berth/tests/board.rs`, `tests/lifecycle.rs`, `tests/hooks.rs` — coverage, including four cases for an outstanding reservation whose trunk cannot be read
- `docs/cargo-berth/generated/output-contract.json` — regenerated for the two new report fields

**Binds later work:** The reader rollout runs `reader_compat` against every inventoried reader with `CARGO_BERTH_EXECUTABLE` set explicitly, recording five results per executable path; its negative control is a binary built from `a42fd209268cb18462001617b337fe6413a4cd06`, the parent of the commit that introduced `MergeExtentObserved`, which rejects all five with `journal record 2 is corrupt: unknown variant merge_extent_observed`. Every reader that can reconcile or release must be Phase 9-inclusive by build provenance before a new binary touches a shared ledger — `reader_compat` alone does not prove it. A ledger spans both evidence derivations: records written earlier keep their old value and replay unchanged, a property the rollout reads over rather than reconciles. No lane opened the shared `cargo-liner` ledger.

**Gotchas:**
- `CARGO_BERTH_EXECUTABLE` unset is not an error — it silently selects the freshly built binary (`reader_compat.rs:115`), so an unset run looks like a pass while reporting on the wrong binary.
- `reader_compat` passing is not proof a reader is Phase 9-inclusive: the frozen fixture exercises neither `board --reservation` nor an evidence transition, so a reader that merely decodes the merge-extent record passes while still carrying both repaired defects.
- An evidence record describes its own moment; the newest record is not automatically the current live decision — compare against the live answer.
- A release against an unreadable trunk does not reach `outstanding_operation` on its own: `release::execute` returns the reconciliation result first, so a test must materialize `object_unknown` with a board read before calling release.
- A test fixture must commit its berth configuration before making worktrees — an untracked configuration file adds merge protection and fails setup assertions.
- `verify.sh lint <pkg>` formats the whole package, rewriting files outside the caller's scope; package formatting belongs to one owner after every editing lane pauses.
- The two unreadable-trunk lifecycle cases declare different scopes (`file:checkpoint.rs` clear, `file:declared.rs` blocking), so the pair is not a single-variable control; an Outstanding reservation's persisted decision reads only `merge_extent.protection()`, and the asymmetry proves the declaration-only path stays editable. Align on the next round that touches the file.

**Ruled out:**
- Editing an existing ledger to remove the merge-extent record — the remaining readers get upgraded instead.
- Deferring the unreadable-trunk lifecycle coverage to the rollout phase — it is this phase's own surface.
- Giving `CommandText::subcommand` all three absence outcomes — two are settled by `cargo_split` before the type is built.

### Phase 10 — A `cargo install` shows its build progress like a `cargo build`  · status: done

#### As-built

A `cargo install` reports the compiling counter a `cargo build` reports, and no source changed to make it so. The capture shim routes `install` onto the capture path alongside `build` (`cargo-capture-shim.sh`, the `case $first` classifier), writes a registration whose fields differ from a build's in argv alone, and mirrors cargo's `Building [...] n/m` output into the run log. `parse_state` and `last_counter` in `progress.rs` key on that counter rather than on the subcommand, and `heading_gauge` in `render.rs` draws the bar from it, so an install row reaches the grid by the route a build row takes.

Confirmed against the current tree for `cargo build`, for `cargo install --path .` inside the project, and for `cargo install --path <fixture>` from a directory that is not a project, each with and without a terminal attached: every case renders the `building` heading with its gauge above a correctly named row.

**Files:** none — no source changed.

**Gotchas:** A cargo-tile predating Phase 8 writes `cargo-tile-v2` registration framing. A machine reporting missing install progress is checked against its installed cargo-tile version before the capture path is suspected.

**Ruled out:**
- An install-versus-build regression test — the two subcommands share one path from classification through the gauge, so it covers no branch the existing build coverage already covers.
- Changing the shim classifier, the counter parser, or the heading gauge — each was observed working for `install` against the current tree.

### Phase 11 — A cargo row reports the processor time its invocation actually causes  · status: done

#### As-built

- Invocation CPU is a cumulative measurement. `CpuAccumulator`, `InvocationCpu`, `InvocationCpuSample`, and `InvocationWork` (`crates/cargo-tile/src/processes.rs`) sum processor time accumulated across the invocation between scans, including time reaped from exited children. Linux reads `utime+stime+cutime+cstime` from `/proc/<pid>/stat` through `linux_cpu_time`, with `LinuxCpuSample` verifying the read; other platforms retain each descendant's last observed total after it disappears. A new or idle descendant never blanks the row; `Unavailable` means the invocation's own counter could not be read. `measure_cpu`'s evidence rules for the invocation's own counter are unchanged: a zero previous counter, a decrease within one lifetime, and an unchanged counter under a claimed positive rate are unproven.
- Compiler-cache server work is attributed by target directory. A `rustc` outside any cargo tree names its `--out-dir`; `Census::observe_targets`, `detached_compilers`, and `cpu_assignment` charge it to the one invocation whose target directory contains that path. `CompileOwner` (`Unknown`, `Unique`, `Ambiguous`) yields no charge when two invocations compete, `CpuAssignment` names the outcome, `CompilerCreditRetention` keeps a live compiler with its first owner across census gaps, and `CompilePathAbsence` converts the external path absence at the boundary. Ownership includes rowless cargo invocations; ancestry takes precedence over detached attribution; a detached compiler needs matching account and target evidence. `Census::attribute(&details, smoothing, now)` takes the detailed `System` because target directories come from process command lines. `CpuSmoothing` retains invocation counters, target evidence, and compiler ownership across scans; `settle`, `CpuBaseline`, `CpuPublication`, and `cpu_label` operate on the invocation-level measurement.
- Group leads charge a shared pid once. `Census::groups` snapshots process-row invocation identities before registration-row injection; command and unpromoted-summary leads charge each contributing pid once (`aggregate_cpu`, `subtree_cpu`), while a promoted subtree total stays unavailable for an unproven invocation identity.
- crossterm carries the `use-dev-tty` feature (`crates/cargo-tile/Cargo.toml`), selecting the level-triggered Unix input backend so a keystroke arriving together with a terminal resize reaches the application. Windows input is unchanged.
- The rows readout owns its row: contents and readout partition the interior (`draw_with_readout`, `content_area`, `rows_readout_height` in `render.rs`), the readout row is counted in every tile's height demand, and a one-row interior belongs to the readout alone. `draw_cell_for_test` (`#[cfg(test)] pub(crate)`) exposes the production drawing sequence to the readout fixture.
- `constants.rs` holds `COMPILER_PROCESS_NAMES`, `RUSTC_BINARY`, `RUSTC_OUT_DIR_FLAG`, `CARGO_TARGET_DIR_FLAG`, `CARGO_TARGET_DIR_ENV`, `CPU_AUXV_PATH`, `CPU_AUXV_CLOCK_TICKS`, `CPU_AUXV_ENTRY_WORDS`, `CPU_STAT_TIME_INDEX`, `CPU_STAT_TIME_FIELDS`.

**Files:**
- `crates/cargo-tile/src/processes.rs` — CPU measurement and attribution types (`CpuBaseline`, `CpuPublication`, `CpuSmoothing`, `Measurement`, `CpuAccumulator`, `InvocationCpu`, `InvocationCpuSample`, `InvocationWork`, `CompileOwner`, `CpuAssignment`, `CompilerCreditRetention`, `CompilePathAbsence`, `LinuxCpuSample`; free functions `compile_owner`, `argument_path`, `process_path`, `process_argument_path`, `cargo_target_directory`, `linux_clock_ticks`, `linux_cpu_ticks`, `linux_cpu_time`); `Census` CPU methods (`attribute_cpu_with`, `live_cargo`, `cpu_ancestry`, `cpu_owners`, `cpu_assignment`, `collect_cpu_work`, `exclude_nested_cpu`, `compiler_credit_retention`, `cpu_identity`, `process_cpu_time`, `observe_targets`, `detached_compilers`); group totals (`groups`, `aggregate_cpu`, `subtree_cpu`)
- `crates/cargo-tile/src/constants.rs` — compiler names, target-directory flag and env names, `/proc` auxv and stat positions
- `crates/cargo-tile/src/render.rs` — content/readout partition, `draw_cell_for_test`
- `crates/cargo-tile/Cargo.toml`, `Cargo.lock` — crossterm `use-dev-tty`
- `crates/cargo-tile/tests/shim_registration.rs` — CPU turnover, cache attribution, competing/excluded owner, identity recovery, counter retention, and resize-and-key cases; includes `tests/support/rows_readout.rs`
- `crates/cargo-tile/tests/summary_totals.rs` — shared-pid, non-process contribution, and missing nested attribution cases
- `crates/cargo-tile/tests/support/rows_readout.rs` — readout collision fixture at 298x10/11/12 interiors; imports `processes` and `progress`; included only by `shim_registration.rs`

**Binds later work:** The `processes.rs` split carries the CPU-measurement cluster and the `Census` CPU methods listed above together and preserves the accounting rules: cumulative reads including reaped children, ancestry before detached attribution, no charge for a contested compile, `Census::attribute` taking `System`, a shared pid charged once per lead. `tests/support/rows_readout.rs` imports `processes` and `progress` and is included only by `shim_registration.rs`. `draw_cell_for_test` in `render.rs` stays `#[cfg(test)] pub(crate)`. crossterm's `use-dev-tty` feature stays; the Mac preflight and the rollout reader build with it. Linux per-process CPU readings populate on this host, so the promoted-summary-row observation runs during rollout.

**Gotchas:**
- `CpuSmoothing` owns far more than smoothing — accumulator state, target evidence, and compiler ownership — and `Census::attribute` reads it; renaming it and the four accumulator/history/baseline/contribution types is an open decision for the `processes.rs` split.
- A contested compile is refused silently: the row still shows its own tree's time, and nothing surfaces the refusal to the user.
- crossterm's default edge-triggered Unix backend discards a terminal-readiness token when a resize signal arrives in the same batch; only the `use-dev-tty` feature prevents the lost keystroke.

**Ruled out:** A source-side crossterm repair in `terminal.rs` (a replacement readiness handler on the render thread) — crossterm's own first readiness check can discard the key before it installs, and the render thread can block on a partial escape sequence; charging a contested compiler to one of its competing invocations — no charge is the only answer that never overstates a row.

### Phase 12 — The sweep proves each file rather than the directory, so cleanup has nothing to report  · status: done

#### As-built

- `RootScan::access(&self, EffectiveUser) -> Result<SweepAuthority<'_>, CleanupRefusal>` grants a sweep when the effective uid matches the root owner, the tree's dev/ino/owner identity is unchanged since `open`, and `state/pids` is reachable; `RootScan::sweep(&self, &mut SweepBudget, impl FnMut(ScanEntry<'_>) -> SweepDisposition) -> SweepCounts` turns a refusal into one skip and never reports it. `InspectedDirectoryMetadata::same_directory` compares dev, ino, and owner only.
- `SweepAuthority::sweep_pair` proves each candidate from its own descriptor: `SweepFileInspection::inspect` opens it relative to the retained directory handle with `NOFOLLOW|NONBLOCK|CLOEXEC` and `fstat`s it (regular, `st_uid` == uid, no group/other write bit, `st_nlink == 1`); the callback reads the proved registration through `RegistrationReadPurpose::SweepRevalidation`; `SweepEligibleFile::revalidate` compares the held dev/ino with `statat(SYMLINK_NOFOLLOW)` immediately before each `unlinkat`; the log is unlinked before the registration. An unproved candidate stays in place and is counted, never reported, so a directory another account can write into decides nothing.
- `SweepCounts { removed, skipped }` is worker-local and nothing renders it; `removed` counts files, `skipped` counts retained pairs, unenumerated portions, and a refused root. `CleanupRefusal` is private with `Foreign(PathBuf)`, `EffectiveUserUnavailable`, `Access(PathFailure)`, and `EnumerationIncomplete(PathBuf)`; no consumer reads the payloads.
- `progress.rs` has no whole-scan sweep gate and no cleanup status: an unreadable registration is a per-file diagnostic in the Settings row and its siblings still sweep. The Settings account row carries no cleanup text; `AccountCaptureDirectory` has no `cleanup` field; every production `CAPTURE_ACL_*`/`CAPTURE_CLEANUP_*` constant is gone. Removed with them: `WritableByOthers`, `AclWritableByOthers`, `AclWriteAccess`, `InspectedDirectory::acl_write_access`, `InspectedDirectoryMetadata::refusals`, `settings::cleanup_refusal`, and `RegistrationEvidence`.
- `CpuAccumulator::update(&mut self, &HashMap<ProcessIdentity, Duration>)` borrows its map and the CPU-percentage subtraction is `saturating_sub`.

**Files:**
- `crates/cargo-tile/src/capture_root.rs` — `RootScan::access`/`sweep`, `SweepAuthority`, `SweepFileInspection`, `SweepEligibleFile`, `RegistrationReadPurpose`, `SweepCounts`, `CleanupRefusal`; inline tests for foreign, writable, hard-linked, symlinked, and replaced candidates
- `crates/cargo-tile/src/progress.rs` — `sweep_ended` calls the sweep without a whole-scan gate; unreadable registrations stay per-file diagnostics
- `crates/cargo-tile/src/settings.rs` — account row without cleanup text
- `crates/cargo-tile/src/processes.rs` — `AccountCaptureDirectory` without a cleanup field; `CpuAccumulator::update(&HashMap)`
- `crates/cargo-tile/src/constants.rs` — test-only ACL mode constants; production cleanup and ACL constants removed
- `crates/cargo-tile/tests/capture_root_acl.rs` — twenty macOS file-level cases
- `crates/cargo-tile/tests/support/shared_capture.rs`, `crates/cargo-tile/tests/shim_registration.rs` — assert cleanup wording absent; Settings scroll fixture opens at 10 rows

**Binds later work:** The sweep-policy cluster in `capture_root.rs` is `SweepBudget`, `SweepDisposition`, `SweepCounts`, `SweepAuthority<'scan>`, `SweepFileInspection`, `SweepEligibleFile`, `RegistrationReadPurpose`, `CleanupRefusal`; `SweepAuthority<'scan>` is established from the effective uid plus directory continuity (`RootContinuity::Established`) and `state/pids` access only. The module has no ACL inspector and no production `unsafe`; its only `unsafe` sits in the test module. The sweep-budget doc comment in `constants.rs` still describes the pre-file-proof rule. `progress.rs` carries neither `RegistrationEvidence` nor any cleanup status; `processes.rs` carries `CpuAccumulator::update(&HashMap)` with `saturating_sub`. `tests/capture_root_acl.rs` is whole-file `#![cfg(target_os = "macos")]` with twenty cases that a native run must execute by name; `CAPTURE_ACL_TEST_DIRECTORY_MODE = 0o700`, `CAPTURE_ACL_TEST_FILE_MODE = 0o600`, and `CAPTURE_ACL_TEST_WRITE_PERMISSIONS` survive under `cfg(all(test, target_os = "macos"))`. `docs/cargo-tile/as-built/github-runners.md:102` still describes the retired unreadable-registration gate.

**Gotchas:**
- The final identity check and `unlinkat` are separate syscalls: a basename replaced by another account between them inside an owned world-writable directory could be unlinked. No POSIX conditional unlink exists; never describe survival of such a replacement as guaranteed.
- No Linux gate compiles `tests/capture_root_acl.rs`; a green Linux run says nothing about it. On Darwin `rustix::fs::Mode::bits()` is `u16`, so the fixtures widen with `u32::from(write.bits())` — a fix that has not yet been compiled natively.
- A mode or group change on the capture directory between scans no longer reads as a changed directory.

**Ruled out:**
- Reporting sweep skips or refusals in Settings — cleanup has nothing to report.
- An atomic conditional unlink — none exists on POSIX; the per-file recheck is the design.

### Phase 13 — `birth_stamp` and the capture root are anchored and split  · status: done

#### As-built

- `crates/cargo-tile/src/capture_root.rs` is gone; the capture root is the `root_scan` module directory, named for its anchor type `RootScan`, with one submodule per cluster. `RootScan` keeps `open`, a private `access(&self, EffectiveUser) -> Result<SweepAuthority<'_>, SweepAdmissionRefusal>`, and `pub(crate) sweep(&self, &mut SweepBudget, impl FnMut(ScanEntry<'_>) -> SweepDisposition) -> SweepCounts`. Every consumer names `crate::root_scan::`; nothing widened visibility.
- Sweep behavior is unchanged: each candidate is proved from its own descriptor (`openat` `NOFOLLOW|NONBLOCK|CLOEXEC`, then `fstat`), the callback receives the proved registration through `RegistrationReadPurpose::SweepRevalidation`, dev/ino are rechecked immediately before each `unlinkat`, partial inventories still sweep the pairs they established, and an unreadable registration keeps its diagnostic. `SweepAuthority<'scan>`, `SweepFileInspection`, `SweepEligibleFile`, `SweepBudget`, `SweepDisposition`, and `EffectiveUser` carry their prior names.
- Sweep admission fails through `SweepAdmissionRefusal::{ForeignRoot, EffectiveUserUnavailable, AccessFailure, EnumerationIncomplete}` — `pub(super)`, payload-free; a refusal becomes `SweepCounts::unavailable_root()` and is rendered nowhere. `SweepCounts` is `pub(crate)` because `sweep` returns it, holds four private fields `removed_files`, `skipped_pair_attempts`, `incomplete_inventories`, `unavailable_roots`, and exposes `#[cfg(test)]` accessors of the same names. No Settings row or user surface reads it.
- `birth_stamp/mod.rs` holds the `mod` block, re-exports, `BirthStamp`, and its two parsing tests; `ProcessLifetime` with `LifetimeEvidence` live in `process_lifetime.rs`, and `KernelObservation` with `Observation`, `observe`, and `boot_verification` in `kernel_observation.rs`. Every export the module had before keeps its name and visibility from `birth_stamp/mod.rs`; `KernelObservation` fields stay private and arbitrary observation injection exists only under `cfg(test)`.
- `constants.rs`: `CAPTURE_DIRECTORY_CHANGED` is deleted; `CAPTURE_DIRECTORY_INCOMPLETE` is `#[cfg(test)]`; the sweep-budget doc describes the reservation rule (two units per admitted pair before its first unlink, pairs in partial inventories included).

**Files:**
- `crates/cargo-tile/src/root_scan/mod.rs` — `RootScan` (`open`, `access`, `sweep`), the `mod` block, hub re-exports, seven `RootScan` tests
- `crates/cargo-tile/src/root_scan/inspected_directory.rs` — directory inspection: `SharedDirectoryState`, `SharedCaptureDirectory`, `InspectedDirectory`, `Inventory` (Darwin `sample` at `:311`), `ScanEntry`, `RegistrationReadPurpose`, `RegistrationObservation`, `read_registration_file` with its `registration::{check_version, ParseError}` imports, the `mkfifo` test fixture and its `allow(unsafe_code)`
- `crates/cargo-tile/src/root_scan/root_history.rs` — root identity: `RootHistory`, `RootIncarnation`, `RootIncarnationAnchor`, `PreviousRoot`, `TreeIdentity`, `RegistrationIdentity`, `RootContinuity`
- `crates/cargo-tile/src/root_scan/sweep_authority.rs` — sweep policy: `SweepAuthority`, `SweepAdmissionRefusal`, `SweepBudget`, `SweepDisposition`, `SweepCounts`, `SweepFileInspection`, `SweepEligibleFile`, `EffectiveUser`, `RootOwner`, `enumeration_refusal` (`const fn`, `:319`)
- `crates/cargo-tile/src/birth_stamp/{mod.rs, process_lifetime.rs, kernel_observation.rs, linux.rs, macos.rs}` — table of contents plus `BirthStamp`; lifetime cluster; kernel-observation cluster with `boot_verification` (`:111`); platform acquisition
- `crates/cargo-tile/tests/shim_registration.rs`, `summary_totals.rs`, `capture_root_acl.rs` — `#[path]` blocks include `../src/root_scan/mod.rs`; `capture_root_acl.rs` is whole-file `#![cfg(target_os = "macos")]` and asserts counts through the four accessors

**Binds later work:** `RegistrationObservation` and `SweepCounts` are not re-exported from the `root_scan` hub — the only crate-wide handle on counts is `sweep`'s return value. `PathFailure`'s `birth_stamp` consumer is `kernel_observation.rs`; `CaptureFailure`'s is `inspected_directory.rs`. The as-built doc `docs/cargo-tile/as-built/github-runners.md` still describes `RootAccess::Owned` / `cleanup_refusals()` and is rewritten by the documentation phase to `SweepAuthority`, payload-free admission refusals, file-level proof, and the four private counters.

**Gotchas:** `enumeration_refusal` builds `SweepAdmissionRefusal::EnumerationIncomplete` inside an already-admitted sweep, so the admission type does double duty. A green Linux gate compiles none of `capture_root_acl.rs`; only native macOS execution proves the moved module and its twenty ACL cases. `inspected_directory.rs` and `sweep_authority.rs` each exceed the usual line threshold and each hold one cluster. The final basename identity check and `unlinkat` are separate syscalls, so survival of a replaced candidate is detection, never a guarantee.

**Ruled out:** renaming the sweep types again; a stricter-than-`pub(crate)` `SweepCounts` (the return type of `sweep` cannot be narrower); one file per type; any Settings or user surface for counters or refusals.

### Phase 14 — Types relocated by ownership, and `progress.rs` split  · status: done

#### As-built

- `crates/cargo-tile/src/progress/` replaces `progress.rs`. `mod.rs` holds `Progress` and five `mod` declarations and re-exports nothing; every consumer imports from the owning submodule.
- Twelve types left `processes.rs` for their sole constructor or single consumer: `CounterState`, `CaptureContext`, `CaptureAccount`, `SummaryDetail` in `render.rs`; `Scan` in `terminal.rs`; `CaptureAssociation`, `AssociationSelection`, `UnusedCapture`, `UnusedCaptureReason` in `settings.rs`; `AccountCaptureDirectory`, `RootReadStatus`, `CaptureDiagnostic`, `AccountName` under `progress/`. `processes.rs` keeps process identity (`InvocationId`, `RunId`, `ProcessIdentity`), capture attribution (`DirectCapture`, `DirectAssociation`, `NearestRegistration`, `SelectedProof`), the CPU cluster (`CpuBaseline`, `CpuPublication`, `CpuSmoothing`, `CpuAccumulator`, `InvocationCpuSample`, `InvocationCpu`, `InvocationWork`, `CompileOwner`, `CpuAssignment`, `CompilerCreditRetention`, `CompilePathAbsence`, `LinuxCpuSample`), and `Census`; `Census::attribute(&details, smoothing, now)` and the cumulative CPU accounting are unchanged.
- `impl From<&CaptureLookup> for CounterState` in `render.rs` is the only lookup-to-gauge conversion; `CaptureLookup::working` and `RunState::working` no longer exist. Gauge outcomes are unchanged.
- The counter parser in `progress/capture_read.rs` answers named outcomes instead of `Option`: `CurrentProgress` (`RetiredCounter` — a `Finished` marker retired a recognized counter — vs `Unrecognized` — no recognized activity), `TailCounter`, `ParsedCounter`, `LeadingNumber` (no leading digits vs digit overflow, distinct cases), and `LogWriter` (whether a log name identifies a writer, in `progress/capture.rs`). `CaptureRead` maps them onto the existing gauge states.
- `SweepAuthority::sweep` in `root_scan/sweep_authority.rs` counts every non-`Complete` `Enumeration` outcome directly into `SweepCounts::incomplete_inventories`; `enumeration_refusal` and `SweepAdmissionRefusal::EnumerationIncomplete` are removed, so `SweepAdmissionRefusal` holds only real admission failures: `ForeignRoot`, `EffectiveUserUnavailable`, `AccessFailure`. The partial-enumeration regression stays at two removals, one incomplete inventory, zero unavailable roots; a refused root counts one unavailable root with no callback and no budget consumed (`a_refused_root_counts_once_without_callbacks_or_budget_consumption`). Nothing renders these counts.

**Files:**
- `crates/cargo-tile/src/progress/mod.rs` — `Progress` and the `mod` block; no re-exports
- `crates/cargo-tile/src/progress/capture.rs` — capture assembly, selection, cleanup, `LogWriter`; one coupled cluster of about 523 production lines, above the line threshold by design
- `crates/cargo-tile/src/progress/capture_roots.rs` — `CaptureRoot`, `CaptureCleanup`, `CaptureParent`, `CaptureRoots`, `AccountCaptureDirectory`, `RootReadStatus`, `AccountName`
- `crates/cargo-tile/src/progress/capture_read.rs` — `Phase`, `RunState`, `CaptureLookup`, `CaptureRead`, `Counter`, and the parser outcomes `CurrentProgress`, `TailCounter`, `ParsedCounter`, `LeadingNumber`, with four inline parser tests
- `crates/cargo-tile/src/progress/capture_diagnostic.rs` — `CaptureDiagnostic`, `CaptureFailure`, `PathFailure` (the latter still consumed by `birth_stamp/kernel_observation.rs` through `boot_verification`, and `CaptureFailure` by `root_scan/inspected_directory.rs`)
- `crates/cargo-tile/src/progress/registered_runs.rs` — `RegisteredRuns`, `RegisteredRun`, `RegistrationName`
- `crates/cargo-tile/src/render.rs`, `settings.rs`, `terminal.rs` — home of the render-only, settings-only, and terminal-only types listed above
- `crates/cargo-tile/src/root_scan/sweep_authority.rs` — direct incomplete-inventory count, three admission refusals
- `crates/cargo-tile/tests/shim_registration.rs`, `summary_totals.rs`, `capture_root_acl.rs` — `#[path]` blocks include `../src/progress/mod.rs`

**Binds later work:** `processes.rs` is a single source file minus the twelve relocated types; its split (the `processes.rs` split phase) works from the ownership map above, with `SelectedProof`, the CPU cluster, and `Census` attribution as they stand. Every reference to progress code names a file under `crates/cargo-tile/src/progress/`. As-built documentation must describe the direct incomplete-inventory count, the three surviving admission refusals, and the surviving `CaptureDiagnostic::EnumerationIncomplete`.

**Gotchas:**
- `progress/mod.rs` re-exports nothing; a new consumer imports from the submodule that owns the type.
- `CaptureDiagnostic::EnumerationIncomplete` (a per-root read diagnostic rendered in Settings) survives; only the sweep-refusal variant of the same name is gone.
- `CaptureRead` folds `CurrentProgress::RetiredCounter` and `Unrecognized` into `CaptureRead::NoCurrentProgress`; the distinction lives only in the parser and its tests.
- `capture_root_acl.rs` is whole-file `#![cfg(target_os = "macos")]`; a green Linux gate compiles none of it.

**Ruled out:** re-exporting relocated types from `progress/mod.rs` — consumers name the owner; keeping `CaptureAssociation` or `CaptureDiagnostic` with process capture attribution — each has a single non-process consumer; splitting `progress/capture.rs` below the line threshold — it is one coupled cluster; keeping `EnumerationIncomplete` as an admission refusal — an incomplete inventory is counted inside an already admitted sweep.

### Phase 15 — `processes.rs` split  · status: done

#### As-built

- `crates/cargo-tile/src/census/` replaces `processes.rs`. `mod.rs` declares five submodules and re-exports only the thirteen items consumers outside `census` use: `command_name`, `DirectAssociation`, `SelectedProof`, `Measurement`, `InvocationId`, `VisibleParent`, `Ancestor`, `CargoGroup`, `CargoProcess`, `CompilerObservation`, `RowProvenance`, `RunStart`, `spawn_with_resolver`. Everything else is `pub(super)` or private within `census`.
- The CPU cluster keeps its renamed types — `InvocationCpuAccounting`, `RetainedSubtreeCpuTime`, `InvocationCpuBaseline::{AwaitingFirstSample, Established}`, `InvocationCpuHistory`, `InvocationCpuContributions` — and `Attributed` is now `InvocationMeasurements`: its per-invocation buckets are disjoint, and a nested cargo's bucket is its own, never folded into its parent's. CPU accounting, ancestry precedence, capture selection, and grouping bodies are unchanged apart from the named-outcome conversions below.
- `Census::owning_cargo` answers `CargoAncestry::{Owner, ParentUnavailable, WalkLimitReached}`; neither absence states that a process has no cargo ancestor, and both callers (compiler tallying, cargo-child grouping in `scan.rs`) treat either as "no owner". The `cpu_owners` / `cpu_assignment` path is separate and untouched.
- Raw argv classification draws its outcomes at `cargo_split`, which reports `CargoArgvAbsence::{ArgvUnavailable, ProgramRejected}` (the argv position lookup is inline; a bare `Option<usize>` no longer exists) with lossless `From` into `RowAbsence::{ArgvUnavailable, ProgramRejected, PolicyExcluded}` and `SubcommandAbsence::{ArgvUnavailable, ProgramRejected, NoSubcommand}`. `CommandText::subcommand` reports only `RetainedSubcommandAbsence::NoSubcommand`, the one outcome its validated contents support; `external_subcommand` reports `ExternalSubcommandAbsence::ProgramRejected`.
- `Census::groups` still snapshots process-row invocation identities before registration-row injection; `subtree_cpu` (`&HashSet<InvocationId>`) stays private beside its one call site; the `cfg(test)` field `registration_rows` and both group adapters travel with `Census`. All 123 original inline tests survive once, each beside the code it exercises, plus six new cases (three ancestry, three raw argv); the three `summary_totals.rs` double-charge cases pass unchanged.

**Files:**
- `crates/cargo-tile/src/census/mod.rs` — hub; five `mod` declarations and the thirteen re-exports above
- `crates/cargo-tile/src/census/process_identity.rs` — `InvocationId`, `RunId`, `ProcessIdentity`, `ProcessIdentities`, `CaptureMembership`, `VisibleParent`
- `crates/cargo-tile/src/census/direct_capture.rs` — `DirectCapture`, `DirectAssociation`, `NearestRegistration`, `SelectedProof`
- `crates/cargo-tile/src/census/invocation_cpu_accounting.rs` — `Measurement`, `MeasurementAbsence`, `InvocationMeasurements`, `CpuBaseline`, `CpuPublication`, the five renamed accounting types, `CompileOwner`, `CpuAssignment`, `CompilerCreditRetention`, `CompilePathAbsence`, `LinuxCpuSample`, the Linux clock helpers; one coupled cluster above the line threshold by design
- `crates/cargo-tile/src/census/command_text.rs` — `CommandText`, `ScannerHome`, `RowAbsence`, `CargoArgvAbsence`, `ArgumentLayout`, `CargoArguments`, `SubcommandAbsence`, `RetainedSubcommandAbsence`, `ExternalSubcommandAbsence`, `cargo_split`
- `crates/cargo-tile/src/census/scan.rs` — `Census` with every scan, attribution, row, and grouping method; `CargoProcess`, `CargoGroup`, `Ancestor`, `Compiler`, `CompilerObservation`, `RowProvenance`, `RunStart`, `WorkingDirectoryObservation`, `DirectoryComparison`, `GroupAbsence`, `CargoAncestry`, `subtree_cpu`, both `cfg(test)` group adapters; one coupled cluster above the line threshold by design
- `crates/cargo-tile/src/main.rs`, `render.rs`, `settings.rs`, `roster.rs`, `terminal.rs`, `tiles.rs`, `progress/capture.rs` — import from `census`
- `crates/cargo-tile/tests/shim_registration.rs`, `summary_totals.rs`, `capture_root_acl.rs` — `#[path]` blocks include `../src/census/mod.rs`; `tests/support/rows_readout.rs` imports from `crate::census`

**Gotchas:**
- An import whose only consumers are `cfg(target_os = "linux")` passes Linux lint and fails macOS lint as unused; `invocation_cpu_accounting.rs` guards five such imports with `#[cfg(target_os = "linux")]`, and the crate has no Linux-side check that catches a new one.
- `census/mod.rs` re-exports only crate-wide consumers' items; a new consumer of an internal type, fixtures included, imports from the owning submodule.
- `RowAbsence::ProgramRejected` and `PolicyExcluded` both map to `GroupAbsence::Excluded`; the distinction exists only inside `census`. `CargoAncestry`'s two absences and the `SubcommandAbsence` cases are likewise implementation-only: the rendered outcome is unchanged and their unit tests (`cargo_ancestry_*`, `command_text`, `scan`) own them.

**Ruled out:** folding command-text parsing into `scan.rs` — the scan file is already the crate's largest and the parsing cluster has its own tests and no `Census` dependency; reporting a policy exclusion from `cargo_split` — raw argv inspection runs before the exclusion list is consulted, so the case is unreachable; assigning the unavailable / rejected outcomes to `CommandText::subcommand` — those absences are settled before the type is built; re-exporting internal census types from `mod.rs` for fixtures — fixtures import from the owning submodule.

### Phase 16 — Neither binary spawns a process or rebuilds a session-bus connection once a second  · status: done

#### As-built

Linux only; the macOS backdrop path already opens once and shares. The phase exists because the two binaries together opened about a hundred fresh session-bus connections a minute: sixty `kscreen-doctor` spawns from the 1 Hz capture tick and forty `Settings.ReadOne` calls from the 1500 ms appearance poll.

- The display topology is read once and watched. A worker holds `DisplayTopology::{Unread, Watching, Recovering}` and refreshes on a KScreen change notification, not on the capture tick. `kscreen-doctor -j` runs with a socketpair stdout, polled with `try_wait` against `TOPOLOGY_READ_DEADLINE` (5 s), killed and reaped on expiry; a failed or expired read is `TopologyRead::Unreadable`, held as `Recovering(Unreadable)` and retried on `DESKTOP_RETRY_INTERVAL` (30 s). `capture` reports both `Unread` and `Recovering(Unreadable)` as the existing `CaptureFailure::DisplayNotFound`; a read that is empty is distinct from one that is unreadable.
- `display::under` returns `OutputSelection::{Containing(&Output), Nearest(&Output), NoActiveOutputs}`; an off-screen window selects the nearest output.
- `session_connection` returns `SessionBus::{Connected, Unavailable(ConnectionFailure)}` with a 30 s retry; the window and wallpaper snapshots keep their existing capture failures while the bus is unavailable and recover once it connects.
- The appearance setting is subscribed to once over a held zbus connection in `theme/poller.rs`: `AppearanceBackend` (implemented by `PortalBackend`) reads `org.freedesktop.appearance` / `color-scheme` at startup and delivers later values from the change signal. `follow` takes its callback by value (`FnMut`). Observations are `AppearanceObservation::{Unspecified, Selected(Appearance)}`, retained as `AppearanceHistory::{Unobserved, Observed(AppearanceObservation)}`; an unspecified appearance never reaches the callback, the first concrete one does, and concrete → unspecified → the same concrete value delivers again. `SubscriptionRecovery::{NeverConnected, Interrupted}` and `SubscriptionEnd` are separate types: `NeverConnected` keeps the startup fallback, `Interrupted` keeps the last delivered appearance, and each failed retry logs a `tracing::warn!` line (visible with `TUI_PANE_LOG=warn`).
- Manifests: workspace `futures-lite = "2.6.1"`; Linux `zbus` and `futures-lite` are non-optional in `tui_pane`; `dep:zbus` is no longer part of the `backdrop` feature.

**Files:**
- `crates/tui_pane/src/backdrop/desktop/platform/linux/display.rs` — `DisplayTopology`, `TopologyRead`, the bounded `kscreen-doctor` read and `KScreenDocument` parsing, `OutputSelection` and `under`, inline topology and selection tests
- `crates/tui_pane/src/backdrop/desktop/platform/linux/mod.rs` — `capture`; `session_connection` returning `SessionBus`; inline `session_bus_*` tests
- `crates/tui_pane/src/backdrop/desktop/platform/linux/window.rs` — consumes `OutputSelection` and `SessionBus`
- `crates/tui_pane/src/backdrop/desktop/platform/linux/wallpaper.rs` — consumes `SessionBus`
- `crates/tui_pane/src/backdrop/desktop/platform/linux/constants.rs` and `crates/tui_pane/src/backdrop/constants.rs` — the Linux backdrop constants, `CAPTURE_REFRESH` (still the 1000 ms capture tick) among them; `DESKTOP_RETRY_INTERVAL` lives in the Linux file and `TOPOLOGY_READ_DEADLINE` at the top of `display.rs`
- `crates/tui_pane/src/theme/poller.rs` — `follow`, `AppearanceBackend`, `PortalBackend`, `AppearanceObservation`, `AppearanceHistory`, `SubscriptionRecovery`, `SubscriptionEnd`, inline appearance tests
- `crates/tui_pane/src/theme/constants.rs` — the appearance task's constants

**Gotchas:** The first capture tick after startup can land on `DisplayTopology::Unread` and report `DisplayNotFound` once; the next tick reads the published layout. `backdrop` is off by default in `tui_pane`, so the topology, selection, and bus tests run only with that feature enabled. The subprocess deadline bounds only the read; the retry cadence is `DESKTOP_RETRY_INTERVAL`. An initial `NameOwnerChanged` emission at `AppearanceBackend::watch` would cause a reconnect loop; it has not been observed live.

**Ruled out:** subscribing through `dark-light` (its crate source has no subscribe entry point); placing the subscription in `theme/watch.rs`; a `crates/tui_pane/tests/` target for either lane (both entry points are private or callback-only); `follow` taking its callback by reference (a shared reference held across an await needs `Sync` on the public bound); a quieter repeat warning on failed appearance retries.

### Phase 17 — Rollout on both machines: readers, shims, runner services  · status: done

#### As-built

- Both machines run the rollout: every `cargo-berth` reader passes `reader_compat` (`CARGO_BERTH_EXECUTABLE` set per inventoried executable) with build provenance that includes the extent and evidence repairs; tile readers and `cargo-port` are upgraded; every runner shim carries the exact `v3` version line and `status --all-accounts` is clean. `capture.auto_install = false` and withheld runner hooks were in place per account before the first upgraded reader started (on linux-host through a NixOS pre-start helper, since removed through its flag), then restored per account; `docs/cargo-tile/as-built/github-runners.md` records the inventory, versions and that window. The stale `/var/lib/hana-ci/hana-linux-{1,2}/cargo-tile` trees are gone, the Mac plist no longer sets `CARGO_TILE_ROOT`, and the Mac holds three natemccoy shims after toolchains 1.83.0 and 1.96.0 were removed.
- CI rows for `hana-linux-1`, `hana-linux-2` and `hana-ci` appear in cargo tile from the hana repository's CI, where the three self-hosted runners are registered.
- Darwin hook children switch credentials through a parent-prepared `initgroups` before exec, so an account keeps its whole membership past the 16-group `setgroups` limit; no account is refused. A disposable 21-group account confirms it: the switched child reads through a group beyond the first 16, while an identical 16-group `setgroups` control is refused.
- A promoted CPU summary row whose Cargo parent accrues no CPU ticks of its own reports its descendants' combined CPU instead of `--`.
- Linux backdrop discovery: `TerminalWindowInventory` holds KWin UUIDs from one bounded `kdotool search --name .* getwindowid %@` run, repeated at most once per `DESKTOP_RETRY_INTERVAL`; `TerminalWindowSearch` is `NotSearched` / `Found` / `Unavailable`, and each UUID holds a `TerminalClassification` (`Unclassified` / `Known(TerminalClass)`). Capture and coordinate identification query only terminal windows plus one classifying query per new UUID; marker identification queries every held UUID; a UUID that does not answer is dropped. Failures still reach callers as the existing `CaptureFailure` values.
- The shared position worker holds a `PositionReadState` (`Unwatched` / `Holding{window, position, read_at}`) whose `WindowPosition` is `Unavailable` / `Settled(Frame)` / `Moving{frame, unchanged_since}`: a settled or unanswered window is re-read every `POSITION_IDLE_INTERVAL` (250 ms), a changed frame reads on every request until unchanged for `POSITION_SETTLE_DURATION` (500 ms), and a different window reads at once. The render thread still gets one `Option<Frame>` per request, so a move can register up to one idle interval late.
- A live idle minute on linux-host shows 2 `kdotool` children, 0 `kscreen-doctor` hits, 0 new session-bus connections and capture Ready, with `getWindowInfo` at 4.5/s (previously 27/s, with 59 `kdotool` children per minute).
- Test fixtures: native macOS `shim_registration` canonicalizes the /var alias, copies and ad-hoc signs `/bin/bash` in Darwin fixtures, and sets `PYTHONUTF8=1`; Linux cache readiness retries an unavailable `ps` within its deadline; a child-source-switch PTY grows only when the parent pane footer shows clipping.

**Files:**
- `crates/cargo-tile/src/hook.rs` — Darwin `initgroups` credential switch prepared in the parent
- `crates/cargo-tile/src/census/scan.rs`, `crates/cargo-tile/src/census/invocation_cpu_accounting.rs` — descendant CPU for a zero-tick parent
- `crates/tui_pane/src/backdrop/desktop/platform/linux/window.rs` — held window inventory and terminal classification
- `crates/tui_pane/src/backdrop/desktop/platform/linux/mod.rs` — shared bounded `DesktopSubprocess` reader and `KDO_TOOL_ACCESS`
- `crates/tui_pane/src/backdrop/monitor/mod.rs` — paced position worker
- `crates/tui_pane/src/backdrop/constants.rs` — `POSITION_IDLE_INTERVAL`, `POSITION_SETTLE_DURATION`
- `crates/cargo-tile/tests/shim_registration.rs`, `crates/cargo-tile/tests/summary_totals.rs`, `crates/cargo-tile/tests/support/shared_capture.rs` — native macOS fixtures, CPU regressions, readiness
- `crates/cargo-tile/README.md` — Darwin accounts keep full membership through `initgroups`
- `docs/cargo-tile/as-built/github-runners.md` — rollout record with per-account suppression and restoration, and the 21-group account outcome

**Gotchas:**
- Restarting a tile reader installs the shim unless `capture.auto_install` is false; suppression must precede the first upgraded reader's start.
- `status` reports `Installed` without reading the shim's framing version, and `install` exits zero despite a per-account failure; only each shim's version line proves `v3`.
- XNU hides the environment of the platform `/bin/bash` even from root; the runner Listener process carries the runner environment instead.
- macOS adds implicit groups (12/61/701/100), so a 17-group account resolves to about 21.
- An unsigned copy of `/bin/sh` or `/bin/bash` is killed on macOS, and `/bin/sh` re-execs as bash under its real name.
- Backdrop capture runs only while the attract screen shows; cargo work elsewhere on the host dismisses it.
- The installed readers on both machines predate the final tree until it is deployed.

**Ruled out:**
- Refusing over-limit Darwin accounts, or shortening their groups: `initgroups` preserves full membership.
- Reading the window position every render frame.
- Class-scoped `kdotool` discovery.
- Querying every desktop window on each capture tick.

