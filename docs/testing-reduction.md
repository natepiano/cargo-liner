# Testing reduction

> **Status: IMPLEMENTATION PLAN — phased, delegate-ready.** Same behavior coverage for the cargo-liner workspace test suite at a fraction of its local and CI cost.

## Delegation Context

- **Project:** cargo-liner, a Rust workspace of cargo tools sharing the `tui_pane` ratatui framework (`/home/natepiano/rust/cargo-liner`). Every touched crate is binary-only:
  - cargo-berth: git-worktree path reservation and integration-ordering engine.
  - cargo-tile: terminal UI grid of the cargo invocations running on the machine, fed by a capture shim.
  - cargo-mend: visibility auditing and fixes, run as `RUSTC_WORKSPACE_WRAPPER`.
  - cargo-port: TUI dashboard for Rust projects.
- **Project started:** 2026-09-14T21:05:47.496+00:00
- **Stack:**
  - Rust edition 2024, resolver 3, rustup stable. `.cargo/config.toml` sets `RUSTC_BOOTSTRAP=1` for mend's `rustc_private`.
  - cargo-nextest; CI Linux jobs run under `nix develop -c`.
  - sysinfo 0.39.6 (tile census); ratatui 0.30.2 `TestBackend` via `App::new_for_test`; tempfile.
  - `git` CLI in berth tests (dev-dep `cargo-berth-test-support`) with `#!/bin/sh` hooks and wrappers.
  - Python 3 reader harness and `cargo-capture-shim.sh` in tile tests.
  - `dorny/paths-filter@v4` in CI.
- **Layout:**
  - `.config/nextest.toml`, `.github/workflows/ci.yml`
  - `docs/testing-reduction-ledger.md`, `docs/testing-reduction-measurements.md` (both created in phase 1)
  - `crates/cargo-berth/{src/{ledger,gate,scope,reservation},tests}`
  - `crates/cargo-tile/{src/{census,progress,root_scan},tests/support}`
  - `crates/cargo-mend/{src/{fixes/runner,selection},tests/{diagnostics,support}}`
  - `crates/cargo-port/src/{watcher,lint/runtime,tui/app}`
- **Key files:**
  - `.github/workflows/ci.yml`:
    - the `changes` job's `shared` anchor (lacks `.config/**`);
    - nextest steps: `test` job (`nix develop -c cargo nextest run --all-features --workspace --exclude cargo-mend --tests`), `macos` job (`cargo nextest run --no-fail-fast -p cargo-tile --bins --tests`), `mend_test` job (`cargo nextest run -p cargo-mend --all-features --tests`);
    - `windows` job checks cargo-port and tui_pane only.
  - `.config/nextest.toml` — one override: filter `package(cargo-tile) & binary(shim_registration) & test(=tests::reader_keeps_parent_family_across_its_only_childs_source_switch)`, `threads-required = "num-cpus"`.
  - cargo-berth `src/`:
    - `src/ledger/constants.rs:22` — `MUTATING_VERB_CONTENTION_TOLERANCE` (10 s); `:26` `MUTATION_LOCK_READY_PATH_ENVIRONMENT` = `CARGO_BERTH_TEST_MUTATION_LOCK_READY_PATH`.
    - `src/ledger/lock.rs:26` — `MutationLock::acquire`; ready-path read at `:42`; 100 ms timeout unit test at `:155`.
    - `src/ledger/handle.rs:292` — `Ledger::try_transact`.
    - `src/cli.rs:1511` — `TOTAL_GATE_DEADLINE` local to `run_reference_transaction`; unit tests at `:2408`, `:2547`.
    - `src/coordination_identity.rs:754` — `validate_coordination_identity`.
    - `src/scope/mod.rs:294`, `:302`; `src/reservation/retention.rs:1785`; `src/presentation.rs:372`; `src/output_contract.rs`.
    - `src/ledger/journal.rs:1994`, `:2076`; `src/config.rs:437`, `:469`, `:508`; `src/drift/git_output.rs:536`.
    - Test support: `src/ledger/test_support.rs`, `src/board/test_support.rs`.
  - cargo-berth `tests/` — `lifecycle.rs`, `drift.rs`, `gate.rs`, `hooks.rs`, `liveness.rs`, `edges.rs`, `board.rs`, `answers.rs`, `overlap.rs`, `ledger.rs`, `presentation.rs`, `output_contract.rs`.
  - cargo-tile entry and targets — `src/main.rs` declares 29 `mod`s; no `[lib]`. Auto-discovered integration binaries: `capture_root_acl` (macOS-only via `#![cfg]`), `cli_lifecycle`, `shim_modes`, `shim_registration`, `summary_totals`.
  - `crates/cargo-tile/tests/shim_registration.rs` (3,798 lines):
    - `#[path]` src modules `:3-60`; `#[path] support/shared_capture.rs` `:62-68` (uses `crate::hook::*`, `crate::constants::*`); `support/rows_readout.rs` `:70-72` (uses `crate::census::*`, `crate::render`, `crate::progress::capture_read`);
    - one `mod tests` at `:115` with private imports `:143-160`;
    - harness `const READER_SCENARIO_SCRIPT: &str = r#"…"#` `:166-1954`; shim `include_str!("../src/cargo-capture-shim.sh")` `:1976`;
    - the harness re-executes the test binary: `[binary, '--exact', 'tests::cpu_scan_child', '--nocapture']` `:1534`, and `os.execve(binary, [binary, '--exact', 'reader_child', '--nocapture'])` `:1550`;
    - `terminal_snapshot` `:828`; resize-on-clip regex `:993`; pane visibility check `:982`; byte-split frame regression `:1248`; unconditional `read_terminal(1)` `:1557`; `sleep 1` `:2063`;
    - reader scenarios `:2405-2906`; `App::new_for_test` pattern `:2577`; oversized-newer test `:2697`; `cfg(linux)`/`cfg(macos)` `:2911`, `:2924`;
    - wire tests `:2934-3753` (`versioned_fields_preserve_original_argument_bytes` `:2934` uses `REGISTRATION_MAGIC`).
  - `crates/cargo-tile/tests/summary_totals.rs` (760 lines):
    - `#[path]` `:3-60`; private imports `:84-97` (`census::{CargoProcess,CompilerObservation,InvocationId,Measurement}`, `invocation_cpu_accounting::MeasurementAbsence`, `process_identity::ProcessIdentity`, `scan::groups_with_{cpu,cpu_counters,registration_rows}_for_test`, `cli::Cli`, `render::summary_cpu_for_test`, `roster::{Roster,TrackedGroup}`);
    - `SummaryTree::new` spawns six `sh` `:111-141`; `cli_rejects_unknown_process_arguments` `:322`.
  - `crates/cargo-tile/tests/capture_root_acl.rs` — `#[path]` `:5-66`, `#[expect(dead_code)]` on `mod cli`, 22 macOS ACL tests; target `src/root_scan/sweep_authority.rs`.
  - cargo-tile `src/`:
    - `src/terminal.rs:147` `run_with_capture_parent`; `:538` `force_repaint`.
    - `src/census/scan.rs`: `Census::take` `:525`, `detached_compilers` `:1040`, `forwards_json_capture_arguments` `:1939`, `groups_with_cpu_for_test` `:2379`, `groups_with_cpu_counters_for_test` `:2391`, `groups_with_registration_rows_for_test` `:2451`, `metadata_process` `:2550`, `fixture_groups` `:2581`, `census_of` `:3992`; unit tests cited by line in phases 4–5.
    - `src/census/invocation_cpu_accounting.rs:424` `cargo_target_directory`; `:582` `reidentify_owner`.
    - `src/progress/capture.rs` (tests `:601`, `:681`, `:726`, `:1255`, `:1472`, `:1505`, `:1731`, `:1753`, `:1763`).
    - `src/registration.rs:487`, `:592`, `:609`, `:624`; `src/root_scan/inspected_directory.rs:830`; `src/settings.rs:950`, `:963`, `:1198`, `:1243`.
    - `src/render.rs:643` `draw_rows_readout` (format `content rows: N`, optional width label, `  r/c: H/W`); render tests cited by line in phases 4–5.
    - `src/hook.rs:2405-2502` startup outcome.
  - cargo-mend:
    - `[[test]] name="diagnostics" path="tests/diagnostics/mod.rs"` (17 modules plus `#[path="../support/mod.rs"] mod support`); support `tests/support/{mod,diagnostics,mend_json,report}.rs`; `mend_json.rs:23-24` strips `CARGO_TARGET_DIR`.
    - `tests/cli_smoke.rs:362` (workspace run).
    - `src/fixes/runner/plan.rs:29` (discovery check); `src/fixes/runner/apply.rs:15` (`MendRunner::apply`, empty fix set returns before validation), `:53` (validation check); `src/selection/metadata.rs:169` (default check plan).
    - `tests/diagnostics/rendering.rs:9` is the only test without a crate compile; `:642` shows reuse for identical runs.
  - cargo-port:
    - `src/watcher/runtime.rs` sleeps `:1307`, `:1315`; `register_watch_roots_reports_elapsed_for_representative_roots` `:2879`; `init_git_repo` `:2916`.
    - `src/lint/runtime/supervisor.rs` test `:1128`, 1 s deadline `:1164`.
    - `src/tui/app/mod.rs` trio `:15203`, `:15262`, `:15326`; 150 ms quiet window `:15283`; `init_git_project` `:16200`.
    - `src/test_support.rs`.
- **Test lanes:**
  - cargo-berth: `crates/cargo-berth/tests/`
  - cargo-tile: `crates/cargo-tile/tests/`
  - cargo-mend: `crates/cargo-mend/tests/`
  - cargo-port: none
- **Build:** `bash ~/.claude/scripts/delegate/verify.sh check <pkg>` (`<pkg>` ∈ cargo-berth, cargo-tile, cargo-mend, cargo-port)
- **Test:** `bash ~/.claude/scripts/delegate/verify.sh test <pkg>`; one integration binary: `bash ~/.claude/scripts/delegate/verify.sh test <pkg> <int_test>`
- **Lint:** `bash ~/.claude/scripts/delegate/verify.sh lint <pkg>`
- **Style:** run-end /clippy style-only auto-proceed
- **Invariants:**
  - **Coverage (user, 2026-09-14: "don't sacrifice major test paths").**
    - A change either makes a test cheaper with the same assertions, or removes a test whose exact code path, assertion strength and platforms another named test already asserts.
    - Missing assertions transfer into the survivor before the removal.
    - A scenario without equivalent cover moves down to a unit test; it is never deleted.
    - Every deletion row in a Work Order is re-verified assertion by assertion by the phase; a row whose cover is weaker becomes a move.
  - **Coverage ledger.** Every removal, merge, matrix shrink, dropped compile configuration or move appends a row to `docs/testing-reduction-ledger.md`:
    - columns `Item | Kind | File:line before | Behavior path | Now asserted by | Platforms and features | CI job`;
    - Kind ∈ test, scenario, assertion, matrix cell, compile configuration, move;
    - "Now asserted by" is a file:line plus the assertion, never blank;
    - each phase reconciles its rows against the diff and `cargo nextest list -p <pkg>` before and after.
  - **CI (user, 2026-09-14: "any changes you make need to work in ci also").**
    - Test-only overrides are plain environment variables that work on Linux, macOS and Windows.
    - Nothing depends on natedev's NixOS layout (no envfs, `unshare`, `/dev/shm`).
    - Phases do not push. The run's final push must show these jobs concluding `success` (a `skipped` required job does not count): Test Suite (berth, tile, port), macOS cargo-tile (tile), cargo-mend Test Suite (mend), Windows Build Check (port only).
  - **macOS.** Phases touching cargo-tile verify on the Mac:
    1. `rsync -a --delete --exclude target/ ./ mac:~/tmp/cargo-liner-phase-verify/` with the sandbox disabled;
    2. run `ssh -o BatchMode=yes -o ConnectTimeout=10 mac 'cd ~/tmp/cargo-liner-phase-verify && cargo nextest run --no-fail-fast -p cargo-tile --bins --tests && cargo clippy -p cargo-tile --bins --tests -- -D warnings'`;
    3. if the Mac is unreachable, record that in the phase result and continue.
  - **Measurement.**
    - Never use summed per-test time as a cost measure: it is inflated up to 55x under load.
    - Record the phase 1 protocol at the end of each phase into `docs/testing-reduction-measurements.md`, with the machine state (envfs on/off), compared to the previous phase's row under the same state.
    - The envfs replacement on natedev is the user's `rebuild`, outside every phase; no phase waits on it.
  - **Flakes.** Binaries a phase changes run three consecutive green full-workspace nextest runs (`cargo nextest run --workspace --all-targets`) before the phase closes.
  - **Scheduling.** `reader_keeps_parent_family_across_its_only_childs_source_switch` keeps `threads-required = "num-cpus"` in every phase; only its filter changes when it relocates.
  - **Code.**
    - Format with `cargo +nightly fmt`; taplo checks TOML.
    - Workspace clippy `all`/`pedantic`/`nursery`/`cargo` denied, plus `unwrap_used`, `expect_used`, `panic`, `unreachable`, `allow_attributes_without_reason`, `self_named_module_files`, `undocumented_unsafe_blocks`. Test modules carry a reasoned `#[allow]`.
    - CI runs `cargo mend --workspace --all-targets --fail-on-warn` (file-textual: it reads macOS `cfg` code on Linux) and "Stable Without Bootstrap" (`RUSTC_BOOTSTRAP=0`) for non-mend crates.
    - No new `pub` surface for tests: test support stays `pub(crate)` or `#[cfg(test)]`.
    - macOS-only cfg code in cargo-tile lives in `src/{constants.rs,hook.rs,census/scan.rs,birth_stamp/{mod,kernel_observation,process_lifetime}.rs}` and `tests/capture_root_acl.rs`.

## Phases

### Phase 1 — Stop tests waiting on the clock, plus CI and ledger groundwork  · status: done

#### Work Order

**Goal:** No cargo-berth or cargo-port test waits out a real deadline or proves absence with a time window; CI runs tests on nextest-config changes with build and execution as separate steps; the ledger and measurements docs exist.

**Spec:**

**Part — CI change detection, split build/execution steps, and measurement baseline**

- `.github/workflows/ci.yml`, `changes` job, `dorny/paths-filter` `shared` anchor: add `'.config/**'` so a `.config/nextest.toml` change triggers both `stable_crates` and `cargo_mend`.
- Split each nextest step into a build step and an execution step with identical flags, so compile/link time and execution time are separate step durations:
  - `test` job: `nix develop -c cargo nextest run --all-features --workspace --exclude cargo-mend --tests --no-run` (step name `Build test targets`), then the existing `Run test targets`. Keep the existing `env:` block (`WGPU_BACKEND`, `WINIT_UNIX_BACKEND`, `RUSTFLAGS`) on both steps, or the second rebuilds.
  - `macos` job: `cargo nextest run --no-fail-fast -p cargo-tile --bins --tests --no-run`, then the existing test step.
  - `mend_test` job: `cargo nextest run -p cargo-mend --all-features --tests --no-run`, then the existing step.
- Create `docs/testing-reduction-ledger.md` with a one-paragraph header naming the rule and the empty table `| Item | Kind | File:line before | Behavior path | Now asserted by | Platforms and features | CI job |`.
- Create `docs/testing-reduction-measurements.md`. Other seats edit code during this phase, so the baseline is the pre-change record, not a fresh run:
  - design-audit figures on the pre-change tree: whole workspace 90.9 s wall / 488 s CPU including builds; cargo-berth alone 64.7 s under envfs versus 22.9 s with `/bin` bypassing envfs;
  - CI step durations of Test Suite, macOS cargo-tile and cargo-mend Test Suite from `gh run view 34887513301 --repo natepiano/cargo-liner --json jobs` with the sandbox disabled (mend jobs may be `skipped` in that run; say so).
- Write the protocol every phase records at its end into the measurements doc:
  - machine state line: `envfs: on|off` (`findmnt /bin`), `rustc -V`, profile `dev`, warm cache;
  - build: `command time -v cargo nextest run --workspace --all-targets --no-run`, wall and user+sys CPU;
  - execution: `command time -v cargo nextest run --workspace --all-targets` three times, median and range of wall and CPU, nextest summary line;
  - per package (cargo-berth, cargo-tile, cargo-mend, cargo-port): `cargo nextest run -p <pkg> --all-targets` once, wall, CPU, test count, longest test.
- After every seat has posted `done`, run the protocol once and record it as this phase's row.
- Run long commands in the background and wait for their notification.

**Part — cargo-berth test-only deadline overrides**

- Two environment variables, one per deadline:
  - `CARGO_BERTH_TEST_LOCK_CONTENTION_TOLERANCE_MS` shortens `MUTATING_VERB_CONTENTION_TOLERANCE` (`src/ledger/constants.rs:22`) as consumed by `MutationLock::acquire` (`src/ledger/lock.rs:26`) via `Ledger::try_transact` (`src/ledger/handle.rs:292`).
  - `CARGO_BERTH_TEST_GATE_DEADLINE_MS` shortens `TOTAL_GATE_DEADLINE` (`src/cli.rs:1511`).
  - Name constants for both beside `MUTATION_LOCK_READY_PATH_ENVIRONMENT` in `src/ledger/constants.rs`; the gate one may live in `cli.rs` where its deadline lives.
- One private resolver per deadline, `fn(Duration) -> Duration`:
  - Under `cfg(debug_assertions)`: read the variable once per acquisition (lock) or once per gate invocation (gate), parse as milliseconds, and return `min(parsed, supplied)`.
  - Never lengthen the deadline. Never replace `Duration::ZERO`: the bypass audit's nonblocking acquire stays zero.
  - An absent or unparsable variable returns the supplied duration.
  - Under `not(debug_assertions)`: return the supplied duration without reading the environment.
- Diagnostics keep formatting from the production constants, so messages still say "10-second" under the override.
- Unit tests in `src/`:
  - resolver shorten-only, zero-preserving, absent/unparsable cases;
  - the lock and gate diagnostic text built from the constants, asserted without waiting.
- Integration tests that wait today (about 104 s of slots; `lifecycle.rs:853` waits twice, for contended `init` then `release`):
  - `lifecycle.rs:853`, `drift.rs:3481`, `hooks.rs:321`, `hooks.rs:3023`, `liveness.rs:928`, `liveness.rs:974`: set `CARGO_BERTH_TEST_LOCK_CONTENTION_TOLERANCE_MS` (e.g. 300) on the contending child command only. Keep the normal gate budget. Keep waiting on `CARGO_BERTH_TEST_MUTATION_LOCK_READY_PATH` before timing, so contention is proven.
  - `gate.rs:2470` (assertion at `:2496-2498`, today 9–15 s): the contention case uses the lock variable only.
  - Add an outer-gate-timeout integration test in `tests/gate.rs`: a deliberately blocked worker (hold the mutation lock across the gate, or pause via the ready-path signal) with `CARGO_BERTH_TEST_GATE_DEADLINE_MS` set. It asserts the outer-timeout diagnostic specifically. If the outer timeout and lock contention produce an identical diagnostic and exit code today, record that in the phase result and assert the distinguishing exit code plus message.
  - Every rewritten timing assertion keeps both bounds: `elapsed >= override` and `elapsed < override + SCHEDULING_ALLOWANCE`, where the allowance is a named test constant (e.g. 5 s) documented as CI scheduling headroom and well under 10 s.
  - Covered consumers: `init`, an ordinary mutating verb, `release`, and the gate outer deadline.
- Add a release-build check that the variable is ignored: a unit test under `#[cfg(all(test, not(debug_assertions)))]`, which the normal debug run compiles out. State in the phase result how it was run (`cargo test --release -p cargo-berth <name>` once).
- Out of scope (author's call): `CARGO_BERTH_TEST_MUTATION_LOCK_READY_PATH` stays unguarded as today.
- Record in the measurements doc the longest remaining cargo-berth test after this phase.
- Ledger: one row per rewritten timing test, Kind `assertion`, recording old bound → new bound pair, "Now asserted by" the same test.

**Part — cargo-port timing waits**

- Replace the two sleeps in `src/watcher/runtime.rs:1307` and `:1315` (`POLL_INTERVAL` 500 ms plus 100 ms) with a join or signal on the work they wait for.
- Replace the 1 s window in the `src/lint/runtime/supervisor.rs:1128` test (deadline at `:1164`) that proves nothing ran with a deterministic check, such as a counter or channel asserted empty after the supervisor settles. The same applies to the 150 ms quiet window at `src/tui/app/mod.rs:15283`.
- Give the lint-registration trio (`src/tui/app/mod.rs:15203`, `:15262`, `:15326`) one shared fixture.
- Merge the duplicate git helpers `init_git_project` (`src/tui/app/mod.rs:16200`) and `init_git_repo` (`src/watcher/runtime.rs:2916`) into `src/test_support.rs`, using 3 git spawns instead of 5.
- Delete `register_watch_roots_reports_elapsed_for_representative_roots` (`src/watcher/runtime.rs:2879`): it prints a timing and asserts nothing. Verify there is no assertion before deleting.
- tui_pane is out of scope (author's call: 6 s).
- Ledger rows for the deletion and merges.

**Files:**
- `.github/workflows/ci.yml`
- `docs/testing-reduction-ledger.md`
- `docs/testing-reduction-measurements.md`
- `crates/cargo-berth/src/ledger/constants.rs`
- `crates/cargo-berth/src/ledger/lock.rs`
- `crates/cargo-berth/src/ledger/handle.rs`
- `crates/cargo-berth/src/cli.rs`
- `crates/cargo-berth/tests/lifecycle.rs`
- `crates/cargo-berth/tests/drift.rs`
- `crates/cargo-berth/tests/hooks.rs`
- `crates/cargo-berth/tests/liveness.rs`
- `crates/cargo-berth/tests/gate.rs`
- `crates/cargo-port/src/watcher/runtime.rs`
- `crates/cargo-port/src/lint/runtime/supervisor.rs`
- `crates/cargo-port/src/tui/app/mod.rs`
- `crates/cargo-port/src/test_support.rs`

**Seats:** 2 writers + 1 tester. Split by crate: cargo-berth production plus CI and docs, cargo-berth integration tests, cargo-port.
- `impl` — `crates/cargo-berth/src/ledger/constants.rs`, `src/ledger/lock.rs`, `src/ledger/handle.rs`, `src/cli.rs` (resolvers and unit tests), `.github/workflows/ci.yml`, `docs/testing-reduction-ledger.md`, `docs/testing-reduction-measurements.md`; hub: `src/ledger/constants.rs` (variable names), `docs/testing-reduction-ledger.md` (peers send rows); runs the end-of-phase measurement
- `test` — `crates/cargo-berth/tests/lifecycle.rs`, `tests/drift.rs`, `tests/hooks.rs`, `tests/liveness.rs`, `tests/gate.rs`: timing rewrites and the new outer-timeout test, from the variable names and bounds in the Spec
- `review` — opens as impl; `crates/cargo-port/src/watcher/runtime.rs`, `src/lint/runtime/supervisor.rs`, `src/tui/app/mod.rs`, `src/test_support.rs`; hub: `src/test_support.rs`

**Constraints from prior phases:**
none.

**Acceptance gate:**
- `python3 -c 'import yaml,sys; yaml.safe_load(open(".github/workflows/ci.yml"))'` exits 0; `git diff .github/workflows/ci.yml` shows only the filter line and the three step splits.
- `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth`
- `bash ~/.claude/scripts/delegate/verify.sh lint cargo-berth`
- `bash ~/.claude/scripts/delegate/verify.sh test cargo-port`
- `bash ~/.claude/scripts/delegate/verify.sh lint cargo-port`
- Every rewritten cargo-berth contention test finishes in well under 10 s (each timed child observes the 300 ms override); resolver unit tests prove shorten-only and zero preservation; outer-timeout test green. The edge cold-cost and gate rebase-timing tests stay long here; phase 3 shrinks their scenarios.
- No `thread::sleep` remains in the changed cargo-port tests.
- Measurements doc holds the baseline and this phase's protocol row; ledger rows present.

### Phase 2 — Compile each test once  · status: done

#### Work Order

**Goal:** cargo-tile's `summary_totals`, `capture_root_acl` and `shim_registration` integration binaries and cargo-berth's `output_contract` binary are gone; their tests run once inside the owning crate's bin test binary, and shim wire tests sleep only where birth-time resolution needs it.

**Spec:**

**Part — cargo-tile: relocate summary_totals and capture_root_acl into src**

- Why: both binaries `#[path]`-include every `src` module under `cfg(test)`, so all 732 unit tests compile and run again inside each. A library target cannot replace this: the tests use `pub(crate)` and `#[cfg(test)]` items, and test support must not become public.
- `tests/summary_totals.rs` (13 tests plus `cli_rejects_unknown_process_arguments` `:322`):
  - Move the CLI test into `src/cli.rs`'s test module.
  - Move the remaining tests, their `SummaryTree` live `sh` fixture (`:111-141`) and helpers into `src/census/summary_totals_tests.rs`, declared `#[cfg(test)] mod summary_totals_tests;` in the census module root. Imports become `crate::…`/`super::…` paths for the items at `:84-97`. Keep the live fixture; phase 4 replaces it.
  - Delete `tests/summary_totals.rs`.
- `tests/capture_root_acl.rs` (22 ACL tests, macOS-only):
  - Move into `src/root_scan/sweep_authority.rs` under `#[cfg(all(test, target_os = "macos"))] mod acl_tests` (or merge into its existing test module under that cfg). The `CAPTURE_ACL_TEST_*` constants move with them.
  - Delete `tests/capture_root_acl.rs` and its `#[expect(dead_code)]` on `mod cli`.
- Test names change module path; nothing else asserts on them.
- Record per-binary counts before and after with `cargo nextest list -p cargo-tile`. Expected: `summary_totals` and `capture_root_acl` absent; bin grows by 14 on Linux and by 36 on macOS; the distinct-test total is unchanged. Each moved test is a ledger row of Kind `move`.
- macOS verification per Invariants is mandatory here: the ACL tests compile only on macOS.

**Part — cargo-tile: relocate shim_registration into src**

- Nearly every test in `tests/shim_registration.rs` uses crate-private items. Examples: `REGISTRATION_MAGIC` at `:2953`; `support/shared_capture.rs` uses `crate::hook::*` and `crate::constants::*`; `support/rows_readout.rs` uses `crate::census::*`, `crate::render` and `crate::progress::capture_read`. So the whole file relocates into a test-only module tree, declared in `src/main.rs` as `#[cfg(test)] mod shim_registration;` (`self_named_module_files` is denied, so use `mod.rs` layout):
  - `src/shim_registration/mod.rs` — the `mod tests` content (`:115` onward) minus harness text; imports rewritten from `super::{…}` (`:143-160`) to `crate::…`.
  - `src/shim_registration/reader_scenario.py` — the Python harness moved verbatim out of `READER_SCENARIO_SCRIPT` (`:166-1954`); the Rust side holds `const READER_SCENARIO_SCRIPT: &str = include_str!("reader_scenario.py");`.
  - `src/shim_registration/shared_capture.rs` and `src/shim_registration/rows_readout.rs` — moved from `tests/support/`, with `crate::` paths.
  - The shim stays `include_str!` of `src/cargo-capture-shim.sh` with the path adjusted.
- Re-execution selectors (`:1534`, `:1550`) name tests by full path in the bin test binary. `tests::cpu_scan_child` becomes `shim_registration::tests::cpu_scan_child` (or the path the move produces), and `reader_child` becomes its full path; `--exact` requires the full name. Verify by running one reader scenario.
- `cfg(target_os = "linux")` / `cfg(target_os = "macos")` blocks (`:2911`, `:2924`) keep their gates.
- `.config/nextest.toml` override filter changes to `package(cargo-tile) & kind(bin) & test(=<new full path of reader_keeps_parent_family_across_its_only_childs_source_switch>)`, still `threads-required = "num-cpus"`. Update its comment only for the binary change.
- Delete `tests/shim_registration.rs`, `tests/support/shared_capture.rs`, `tests/support/rows_readout.rs`, and `tests/support/` if empty (check `cli_lifecycle.rs` and `shim_modes.rs` do not use it first).
- No assertion, scenario, sleep or timeout changes in this phase.
- Record per-binary counts before and after with `cargo nextest list -p cargo-tile`. Expected: `shim_registration` absent; the bin gains its 123 non-copied tests; the distinct-test total is unchanged. One ledger row of Kind `move` for the binary, listing the new module path.
- Run the flake invariant (three full-workspace runs), since the exclusive-slot filter changed.

**Part — cargo-berth output_contract move**

- Move `tests/output_contract.rs` (one test, no processes) into the test module of `src/output_contract.rs`; delete the integration file (removes one binary).
- Ledger: one row of Kind `move`.

**Part — shim wire sleep**

- Shim wire tests (23): each passes through a fixed `sleep 1` (`shim_registration.rs:2063`, about 38 s summed; do this after the move, in `src/shim_registration/wire.rs`), which exists for 1-second birth-time resolution. Only the macOS `ps lstart` path and the birth-comparison tests need it. Make the sleep a per-test option, on only for those tests; list which tests keep it in the phase result.
- Ledger: one row (Kind `assertion`) for the sleep change listing affected tests.

**Files:**
- `crates/cargo-tile/tests/summary_totals.rs`
- `crates/cargo-tile/src/census/summary_totals_tests.rs`
- `crates/cargo-tile/src/census/mod.rs`
- `crates/cargo-tile/src/census.rs`
- `crates/cargo-tile/src/cli.rs`
- `crates/cargo-tile/tests/capture_root_acl.rs`
- `crates/cargo-tile/src/root_scan/sweep_authority.rs`
- `docs/testing-reduction-ledger.md`
- `crates/cargo-tile/tests/shim_registration.rs`
- `crates/cargo-tile/tests/support/shared_capture.rs`
- `crates/cargo-tile/tests/support/rows_readout.rs`
- `crates/cargo-tile/src/shim_registration/mod.rs`
- `crates/cargo-tile/src/shim_registration/reader_scenario.py`
- `crates/cargo-tile/src/shim_registration/shared_capture.rs`
- `crates/cargo-tile/src/shim_registration/rows_readout.rs`
- `crates/cargo-tile/src/main.rs`
- `.config/nextest.toml`
- `crates/cargo-berth/tests/output_contract.rs`
- `crates/cargo-berth/src/output_contract.rs`
- `crates/cargo-tile/src/shim_registration/wire.rs`

**Seats:** 2 writers + reserve. Split by binary: the reader half of `shim_registration`, the wire half, and the small moves.
- `impl` — `crates/cargo-tile/tests/shim_registration.rs` deletion, `src/shim_registration/mod.rs` (reader scenarios), `src/shim_registration/reader_scenario.py`, `src/shim_registration/rows_readout.rs`, `tests/support/rows_readout.rs` deletion, `src/main.rs`, `.config/nextest.toml`; hub: `src/shim_registration/mod.rs`, `src/main.rs`
- `test` — opens as impl; `crates/cargo-tile/tests/summary_totals.rs`, `src/census/summary_totals_tests.rs`, `src/census/mod.rs`, `src/cli.rs`, `tests/capture_root_acl.rs`, `src/root_scan/sweep_authority.rs`, `crates/cargo-berth/tests/output_contract.rs`, `crates/cargo-berth/src/output_contract.rs`, `docs/testing-reduction-ledger.md`; hub: `docs/testing-reduction-ledger.md`; runs the macOS verification
- `review` — opens as impl; `src/shim_registration/wire.rs` (wire tests, declared by impl), the per-test sleep option, `src/shim_registration/shared_capture.rs`, `tests/support/shared_capture.rs` deletion

**Constraints from prior phases:**
- Phase 1 created `docs/testing-reduction-ledger.md` and `docs/testing-reduction-measurements.md` (baseline and protocol); append to both.
- Phase 1 split CI nextest steps into `--no-run` build plus execution, and `.config/**` changes trigger all test jobs.
- Phase 1 shortened cargo-berth lock and gate deadlines via `CARGO_BERTH_TEST_LOCK_CONTENTION_TOLERANCE_MS` and `CARGO_BERTH_TEST_GATE_DEADLINE_MS` (debug builds, shorten-only).

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh test cargo-tile`
- `bash ~/.claude/scripts/delegate/verify.sh lint cargo-tile`
- `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth`
- `bash ~/.claude/scripts/delegate/verify.sh lint cargo-berth`
- `cargo nextest list` shows no `summary_totals`, `capture_root_acl`, `shim_registration` or `output_contract` binary, the exclusive-slot test under its new path, and unchanged distinct-test totals.
- Three consecutive green full-workspace runs.
- macOS nextest and clippy green, or recorded unreachable.
- Measurements row and ledger rows present.

### Phase 3 — cargo-berth: fewer, smaller scenarios  · status: done

#### Work Order

**Goal:** cargo-berth's matrices shrink to discriminating inputs, identity rejection is one unit table plus one integration test per verb, same-path scenario families are table-driven, and exact duplicates are gone, with every assertion preserved.

**Spec:**

**Part — cargo-berth measurement matrices**

| Test | Today | Change |
|---|---|---|
| `edges.rs:836` `predecessor_graph_has_fixed_cold_cost` | same git sequence at n=1 and n=20 (23 worktrees) | n ∈ {2, 4} |
| `edges.rs:747` `successor_round_robin_has_fixed_cold_cost_and_covers_every_head` | n=20, 20 cached reads | n=4, 2 cached reads |
| `edges.rs:619` `rewritten_successor_content_is_cached_for_fulfilled_and_holding_edges` | 20 replays × 2 fixtures | 2 replays |
| `gate.rs:2536` `git_hook_post_commit_path_and_commit_cardinality_matrix_is_fixed` | {1,4,33}×{1,14,100} + 7 cases, about 460 commits | {0,1,2}² plus the 7 named cases; fold in `gate.rs:2507` (reservations 1 vs 20 → {1,3}) |
| `gate.rs:1448` `filtered_three_commit_rebases_report_median_and_maximum_wall_time` | 30 timed rebases; `run_three_commit_rebase_sample` (`gate.rs:4345`) asserts each rebase succeeds with hooks disabled, bypassed, live | drop repetitions and the timing report; keep one live-hook and one bypassed rebase asserting success. `gate.rs:1170` spies the hook executable (`committed_feature_rebase_hook_phases` `:4002`), so it does not cover live success |
| `board.rs:3043`, `:2438`, `:2624` | read loops of 20 / 20 / 8 | loops of 2 (mirror test `board.rs:2512` already uses 1) |
| `board.rs:2697` `proof_subject_changes_force_rechecks_at_an_unchanged_target` | 2×3 repos; each cell checks a fresh git comparison, resulting `integrated`/`trunk_rewritten` status, and a second journal record (helper `board.rs:3712`) | keep all three mutations × both prior verdicts, over one shared fixture per verdict with explicit transitions; `retention.rs:1785` covers replay from an integrated verdict only |
| `board.rs:3102`, `:3158` | 4 repos each | N ∈ {1,2}; the one-reservation baseline result may be shared inside one test, but distinct and duplicate proof cases each keep a fresh cold repository (the first board query persists a `scoped_patch_equivalence_checked` record) |
| `gate.rs:1077` retention-ref cardinality | 1 vs 20 refs | {1,3} |

- `drift.rs:1499` (19 valid anchors): first grep cargo-berth `src/` for a batch size or limit near 20 on the path it exercises. Shrink only if none exists; otherwise leave it and record why in the phase result.
- An invariant test must still fail if cost became linear: with n ∈ {2,4}, the test asserts equal git-call sequences or counts, not durations. Confirm each shrunk test's assertion is count- or sequence-based. If one compares durations, keep a ratio that separates n=2 from n=4 and record it.
- Ledger: one row per removed matrix cell group (Kind `matrix cell`) and the gate.rs:1448 repetitions (Kind `assertion`), "Now asserted by" the shrunk test's assertion line.

**Part — cargo-berth identity rejection: unit table and one integration test per verb**

- `validate_coordination_identity` (`src/coordination_identity.rs:754`) is re-tested once per verb and file:
  - `edges.rs:193`, `:251`, `:310`
  - `liveness.rs:783`, `:825`, `:895`
  - `answers.rs:1117`, `:1193`, `:1249`
  - `gate.rs:482`, `:555`, `:607`, `:659`
  - `drift.rs:183`
  - `hooks.rs:671`, `:848`, `:2731`
- No unit test covers how the three rejection kinds are chosen. Add a table-driven unit test in `src/coordination_identity.rs` covering each kind and the precedence between them.
- For each verb, keep one integration test that runs all three kinds in one repository (one fixture, three invocations), asserting each kind's exit code and diagnostic exactly as the tests being merged assert them. Before merging, list each merged test's assertions and confirm the survivor carries them.
- Merged tests run their cases in sequence; record the survivor's duration and confirm it stays below the package's longest test.
- The shared assertion helper `assert_integration_identity_rejection` exists near `gate.rs:4415`; reuse it.
- Ledger: one row per removed integration test (Kind `test`), "Now asserted by" the survivor's line and the unit table.

**Part — cargo-berth hooks, drift and edges scenario families**

- Merging saves time only by folding cases into one test over one fixture. nextest runs each test in its own process, so a shared helper does not reduce construction across tests; the constructors at `board.rs:3801` and `gate.rs:2970` already build fresh repositories per call. For each merged batch, write into the phase result:
  - its cases and each case's surviving assertion;
  - fixture constructions before → after;
  - mutation/reset requirements;
  - the merged test's duration versus the package's longest test. A merge must not raise package wall time.
- Hooks merges:
  - `hooks.rs:3321`/`:3357` and `:3339`/`:3376` are the same evidence-loss fixture, run once per hook: parameterize over the hook.
  - The orphan trio `:2939`/`:3069`/`:3196` becomes one test.
  - `:2906` is covered by `:2692` and `:2846`: re-verify assertion by assertion, then delete.
  - `:3282` folds into `:3271`.
- Table-drive the same-path groups:
  - edges readiness `:522`/`:571`/`:916`/`:962`/`:1214` (`:1202` builds 4 repositories);
  - the drift refused-second-run group `:1720–:2460` (11 tests, about 6 distinct paths);
  - hooks path normalization `:402`/`:424`/`:449`/`:512`/`:589`.
- Scenario fixtures inside merged tests: `rewritten_reservation_fixture` (17 builds, 4 distinct variants), the gate `deferred_pair` (6 tests), and the edges predecessor/successor pair. Each becomes an owned fixture with named variants and explicit transitions inside the merged test.
- Destructive gate variants (configuration edits, hook replacement, worktree removal, e.g. the unavailable worktree test near `gate.rs:2138`) stay independently initialized unless an exact restoration is demonstrated.
- Line numbers predate this phase's other edits; locate tests by name.
- Ledger: one row per removed or merged test.

**Part — cargo-berth subsets, restated unit tests, and output_contract move**

- Subsets (transfer first, then delete):

  | Removed | Covered by | Transfer first |
  |---|---|---|
  | `drift.rs:1292` | `:1397` | — |
  | `drift.rs:1436` | `:1499` | — |
  | `drift.rs:2764` | `:2728` | exactly one journal widening for the named reservation |
  | `drift.rs:2844` | `:2941` foreign arm | the authorized overlap's scope path; extend `recorded_widen_authorization` (`:3074`), which retains only holder ids today |
  | `gate.rs:1655` | `:1683–:1695` | the `refs/heads/topic` case |
  | `presentation.rs:345` | `:320` and `src/presentation.rs:372` | — |
  | `ledger.rs:96` | `lifecycle.rs:649` | — |

- `ledger.rs:179` stays: it rebuilds through ordinary `init`; `lifecycle.rs:2270` runs `init --repair-projection`, a separate CLI branch.
- Integration tests restating unit tests:

  | Integration test | Outcome |
  |---|---|
  | `overlap.rs:161` | delete; covered by `src/scope/mod.rs:302` |
  | `overlap.rs:122` | add a unit case for file scope versus tree scope over descendants beside `src/scope/mod.rs:294`, then delete |
  | `liveness.rs:1075` | move its incomplete and conflicting `--retire-orphan` argument cases, with their diagnostics, into `src/cli.rs` unit tests beside `:2408`; merge its exit-code assertion into an existing liveness usage-error integration test |
  | `answers.rs:1559` | move the valid-token-without-answer case and its diagnostic into `src/cli.rs` beside `:2547`; merge its exit-behavior assertion into an existing answers usage-error integration test |
  | `overlap.rs:941`, `:965`, `:991` | keep: real worktree configuration discovery, command outcomes, and refusal to replay after configuration disappears; may merge into one test over one fixture |
  | `ledger.rs:399` | keep: exit code 4 and the `ledger_unreadable`/`no_facts` envelope |
  | `drift.rs:3213` | keep: a real git rename through incremental drift |

- Line numbers predate this phase's other edits; locate tests by name.
- Ledger: one row per removal, transfer (Kind `assertion`), and move.

**Files:**
- `crates/cargo-berth/tests/edges.rs`
- `crates/cargo-berth/tests/gate.rs`
- `crates/cargo-berth/tests/board.rs`
- `crates/cargo-berth/tests/drift.rs`
- `docs/testing-reduction-ledger.md`
- `crates/cargo-berth/src/coordination_identity.rs`
- `crates/cargo-berth/tests/liveness.rs`
- `crates/cargo-berth/tests/answers.rs`
- `crates/cargo-berth/tests/hooks.rs`
- `crates/cargo-berth/tests/presentation.rs`
- `crates/cargo-berth/tests/ledger.rs`
- `crates/cargo-berth/tests/overlap.rs`
- `crates/cargo-berth/tests/lifecycle.rs`
- `crates/cargo-berth/src/scope/mod.rs`
- `crates/cargo-berth/src/cli.rs`
- `crates/cargo-berth/tests/output_contract.rs`
- `crates/cargo-berth/src/output_contract.rs`

**Seats:** 3 writers. Test-only work split by file group; the ledger has one owner.
- `impl` — `crates/cargo-berth/tests/gate.rs`, `tests/drift.rs`, `src/coordination_identity.rs` (unit table), `docs/testing-reduction-ledger.md`; hub: `docs/testing-reduction-ledger.md`
- `test` — opens as impl; `tests/hooks.rs`, `tests/liveness.rs`, `tests/answers.rs`, `src/cli.rs` (moved parser cases); `tests/lifecycle.rs` read only
- `review` — opens as impl; `tests/edges.rs`, `tests/board.rs`, `tests/overlap.rs`, `tests/presentation.rs`, `tests/ledger.rs`, `src/scope/mod.rs`

**Constraints from prior phases:**
- Phase 1: lock and gate deadlines are shortened in tests via `CARGO_BERTH_TEST_LOCK_CONTENTION_TOLERANCE_MS` / `CARGO_BERTH_TEST_GATE_DEADLINE_MS`; do not reintroduce real 10 s waits. Line numbers in `lifecycle.rs`, `drift.rs`, `hooks.rs`, `liveness.rs` and `gate.rs` moved; locate tests by name.
- Phase 2 moved `tests/output_contract.rs` into `src/output_contract.rs`.
- Ledger and measurements docs exist (phase 1).

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth`
- `bash ~/.claude/scripts/delegate/verify.sh lint cargo-berth`
- Each shrunk invariant test asserts counts or sequences; one identity-rejection integration test per verb; unit table green.
- Phase result lists each merged batch's cases, fixture counts and durations; cargo-berth execution wall is not higher than after phase 1.
- Every deleted test has a ledger row naming a file:line assertion; rows reconcile with `cargo nextest list -p cargo-berth` before and after.
- Three consecutive green full-workspace runs; measurements row present.

### Phase 4 — cargo-tile: synthetic census and a lean reader harness  · status: todo

#### Work Order

**Goal:** Census tests run over private observation records instead of spawned processes; covered reader scenarios are deleted, app-level ones are unit tests, and the harness parses incrementally with the kept readers serialized.

**Spec:**

**Part — cargo-tile synthetic census: observation records through the production scan**

- Today `groups_with_cpu_for_test` (`scan.rs:2379`), `groups_with_cpu_counters_for_test` (`:2391`), `groups_with_registration_rows_for_test` (`:2451`) and `fixture_groups` (`:2581`) take a live `&sysinfo::System`, forcing real processes (`metadata_process` `:2550`). They also overwrite discovery results from argv and return only the final sample's groups.
- Define private (`pub(crate)` at most) observation records consumed by the same functions in production and tests: `Census::take` (`:525`), `detached_compilers` (`:1040`), `cargo_target_directory` (`invocation_cpu_accounting.rs:424`), and the assembly they feed. Each record carries:
  - pid and parent pid;
  - native process name and argv, observed independently, each possibly unavailable;
  - cwd and executable metadata, possibly unavailable;
  - user id (detached-compiler attribution matches observed uids);
  - the environment variables target selection reads;
  - the existing `LifetimeEvidence` and `Measurement` types for lifetime and CPU.
- Production builds the records from its existing two-stage OS scan and ordered native CPU reads, keeping their order. Records borrow from the scan (no string or path clones). No process-source trait. If a PID index is materialized, it is one allocation per scan; measure scan time before and after with an existing census benchmark or a timed unit loop, and record the figure.
- Tests construct records directly. Provide a sequence fixture that:
  - keeps accounting and identity state across samples;
  - takes a fresh record set per sample;
  - returns every sample's groups, so tests assert each sample.
- Replace the four `*_for_test`/`fixture_groups` helpers with this path; delete the argv-overwrite of discovery results.
- Convert callers:
  - `src/census/summary_totals_tests.rs` (phase 2): `SummaryTree::new` stops spawning six `sh`;
  - the scan unit tests that used the helpers.

  Assertions stay identical.
- No reader scenario changes in this phase.

**Part — cargo-tile app-level moves, covered deletions, and shim sleep**

- Kept end-to-end (do not touch): `child-source-switch` (`reader_keeps_parent_family_across_its_only_childs_source_switch`), `locale`, `root-headings`, `settings-scroll-burst`, `cpu-cache-server`, `quiet-json-long`, `excluded`.
- Moves needing app entry points:

  | Scenario | Unit test | Needs |
  |---|---|---|
  | `settings-scroll` | `TestBackend` over `App::new_for_test` (pattern formerly at `shim_registration.rs:2577`, now in `src/shim_registration/mod.rs`) | none |
  | `startup-newer`, `startup-newer-failure`, `startup-truncated` | toast show and expiry over `App::new_for_test` | a `#[cfg(test)]` way to inject a hook startup outcome (`hook.rs:2405-2502`) |

- Delete, each re-verified assertion by assertion first. A row whose cover is weaker becomes a move in this phase.

  | Scenario | Covered by |
  |---|---|
  | `fallback-source-switch` | `scan.rs:3008`, `:3087`; `render.rs:3716` |
  | `version-mixed` | `registration.rs:624`; `progress/capture.rs:601` |
  | `fallback-ambiguous` | `scan.rs:2866`; `settings.rs:1198` |
  | `fallback-excluded` | `scan.rs:2797` |
  | `fallback-foreign-owned` | `progress/capture.rs:1255` |
  | `fallback-unreadable-log` | `scan.rs:2976`; `settings.rs:1243` |
  | `staging` | `progress/capture.rs:1731`, `:1753`, `:1763` |
  | `two-directories` | `render.rs:3601`, `:3901` |
  | `fallback-other-home` | `scan.rs:4437`, `:4462` |
  | `grouping` | `render.rs:3869`; `scan.rs:4392` |
  | `fallback` | `scan.rs:2599` |
  | `fallback-unknown` | `scan.rs:2866` |
  | `fallback-summary` | `render.rs:2647`; `src/shim_registration/rows_readout.rs` |
  | `oversized_newer_registration_reports_its_version_before_its_size` | `registration.rs:592` |
  | `cli_rejects_unknown_process_arguments` (the harness copy, formerly `shim_registration.rs:89`) | the phase 2 test in `src/cli.rs` |

- Harness-only scenarios go with the scenarios they served: `grouping-earlier-pane`, `pane-readiness-delayed`, `pane-readiness-never`, `cache-readiness-delayed`, `cache-readiness-never`. Remove harness helper code only these used.
- Line numbers in `scan.rs`, `render.rs` and `capture.rs` predate this phase's census change; locate by test name if they moved.
- Ledger: one row per deleted or moved scenario.

**Part — cargo-tile harness incremental parsing and reader scheduling**

- `terminal_snapshot` (formerly `shim_registration.rs:828`) re-tokenizes the whole transcript on every poll. Replace it with a parser whose state persists across polls:
  - feed only newly appended bytes;
  - publish a snapshot only at a completed frame; during an incomplete draw keep returning the previous completed snapshot.
- Do not trim at the last full-screen clear. It can select an unfinished redraw and discard the completed screen before it, and `force_repaint` (`src/terminal.rs:538`) redraws without clearing.
- Extend the byte-split frame regression (formerly `:1248`) with:
  - an unfinished full-screen clear;
  - a resize;
  - colour (SGR) assertions;
  - repeated redraws without clears.
- Drop the unconditional `read_terminal(1)` (formerly `:1557`).
- The resize-on-clip regex (formerly `:993`) must match the `draw_rows_readout` format (`src/render.rs:643`: `content rows: N`, optional width label, `  r/c: H/W`). Exercise it once against real output. If it never matches, fix it; if its branch is unreachable, delete it. Record which.
- `.config/nextest.toml`:
  - Keep the `threads-required = "num-cpus"` override on `reader_keeps_parent_family_across_its_only_childs_source_switch`.
  - Add `[test-groups.cargo-tile-readers] max-threads = 1` and an override assigning the other six kept reader tests (`locale`, `root-headings`, `settings-scroll-burst`, `cpu-cache-server`, `quiet-json-long`, `excluded`) by full test path to that group.
  - Comment: nextest leaves tests outside a group unaffected, so the exclusive slot stays for the pane-capacity-sensitive test.
- Record in the measurements doc the group's combined duration and full-workspace execution under the new schedule.

**Files:**
- `crates/cargo-tile/src/census/scan.rs`
- `crates/cargo-tile/src/census/invocation_cpu_accounting.rs`
- `crates/cargo-tile/src/census/summary_totals_tests.rs`
- `crates/cargo-tile/src/census/mod.rs`
- `crates/cargo-tile/src/render.rs`
- `crates/cargo-tile/src/shim_registration/reader_scenario.py`
- `crates/cargo-tile/src/shim_registration/mod.rs`
- `crates/cargo-tile/src/shim_registration/wire.rs`
- `crates/cargo-tile/src/hook.rs`
- `crates/cargo-tile/src/app.rs`
- `docs/testing-reduction-ledger.md`
- `.config/nextest.toml`
- `docs/testing-reduction-measurements.md`

**Seats:** 2 writers + 1 tester. Split into census production code, the harness, and the app-level unit tests.
- `impl` — `crates/cargo-tile/src/census/scan.rs`, `src/census/invocation_cpu_accounting.rs`, `src/census/mod.rs`, `src/render.rs`, `src/census/summary_totals_tests.rs`; hub: `src/census/mod.rs`
- `test` — settings-scroll and startup toast unit tests in `src/shim_registration/app_scenarios.rs` (declared by review), `src/hook.rs` startup injection, `src/app.rs` only if needed; the sequence-fixture self-test goes to impl
- `review` — opens as impl; `src/shim_registration/reader_scenario.py`, `src/shim_registration/mod.rs` (deletions, extended frame regression, `app_scenarios` declaration), `.config/nextest.toml`, `docs/testing-reduction-ledger.md`, `docs/testing-reduction-measurements.md`; hub: `src/shim_registration/mod.rs`, `docs/testing-reduction-ledger.md`

**Constraints from prior phases:**
- Phase 2 moved summary tests to `src/census/summary_totals_tests.rs` (live `sh` fixture) and the CLI test to `src/cli.rs`; the harness and wire tests live in `src/shim_registration/` (`mod.rs`, `reader_scenario.py`, `shared_capture.rs`, `rows_readout.rs`, `wire.rs`); the exclusive-slot filter targets the bin binary; wire tests already sleep only where needed.
- Ledger and measurements docs exist (phase 1); CI `.config/**` changes trigger all test jobs.

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh test cargo-tile`
- `bash ~/.claude/scripts/delegate/verify.sh lint cargo-tile`
- `rg 'Command::new\("sh"\)' crates/cargo-tile/src/census` returns nothing; scan-time figure recorded.
- Extended frame regression green; the covered and app-level scenarios are absent from the harness.
- Three consecutive green full-workspace runs with the new schedule.
- macOS nextest and clippy green, or recorded unreachable.
- Measurements and ledger rows present.

### Phase 5 — cargo-tile: remaining reader scenarios become unit tests  · status: todo

#### Work Order

**Goal:** The capture-scan and scan-selection reader scenarios become unit tests carrying every assertion, leaving the harness with exactly the seven kept end-to-end scenarios.

**Spec:**

**Part — cargo-tile capture-scan scenario moves**

- Each unit test carries every assertion of its scenario and runs on Linux and macOS. First read the scenario's assertions in `src/shim_registration/reader_scenario.py` and its Rust wrapper in `src/shim_registration/mod.rs`, then write the unit test, then remove the scenario.

  | Scenario | The unit test must assert |
  |---|---|
  | `version-newer-live`, `version-newer-ended`, `version-newer-oversized` | repeated sweeps of an ended sibling leave the unsupported registration and its log byte-identical; plus the existing parser and settings assertions (`registration.rs:592`, `:609`; `inspected_directory.rs:830`; `settings.rs:963`) |
  | `version-malformed` | the same byte-identical preservation of malformed artifacts across sweeps, plus sibling readability (`progress/capture.rs:1505`) |
  | `forged` | a registration and log whose identity mismatches the live process are removed; `registration.rs:487` and `progress/capture.rs:1472` assert the opposite case (uncertain evidence prevents removal) |
  | `ambiguous-generation`, `unverifiable-generation` | competing metadata yields neither command nor progress, and progress returns once the competing publication is removed (`progress/capture.rs:681`, `:726` cover ambiguity only) |

- Place tests beside `progress/capture.rs`'s sweep tests, using temp directories and the phase 4 observation records for liveness.
- Remove each moved scenario from the harness and its Rust test wrapper.
- Ledger: one row per scenario, Kind `scenario` → `move`, "Now asserted by" the unit test lines.

**Part — cargo-tile scan-selection scenario moves**

- Read each scenario's assertions first; each unit test carries every one, on Linux and macOS.

  | Scenario | The unit test must assert | Where |
  |---|---|---|
  | `nested`, `exec-nested`, `exec-excluded` | distinct nested rows survive selection with their own commands and progress; for `exec-excluded`, captured writes continue after cleanup (`scan.rs:3483`, `:3677`, `:3896` end with nested argv unavailable) | beside those tests |
  | `quiet-json-short`, `quiet-json-separate`, `rejected-rewrite` (non-json and post-separator), `rejected-rewrite-unrelated` | one reader row for an accepted rewrite; two distinct rows for a rejected one (`scan.rs:3815`, `:3851` test only the comparison predicate) | beside `scan.rs:3815` |
  | same five | the shell's executed argv for each case | new cases in `src/shim_registration/wire.rs` |
  | `root-duplicate`, `fallback-root-duplicate`, `fallback-selected-unknown` | a directory whose claimed uid differs from its owner is rejected (`scan.rs:2909`, `:4186`, `:2942`, `:4246` test precedence between accepted roots) | beside those tests |
  | `summary-root-headings` | foreign-owned attribution is rejected in the summary (`render.rs:3645`, `:3788` build accepted rows) | render tests |
  | `cpu-cache-identity-recovery` | across multiple samples, a detached compiler with a retained target and established CPU credit keeps its CPU through registration-identity recovery, without repeating prior history (`scan.rs:4804` has no detached compiler; `reidentify_owner` `invocation_cpu_accounting.rs:582` transfers history, targets and compiler ownership separately) | beside `scan.rs:4804`, sequence fixture |
  | `cpu-cache-ambiguous`, `cpu-cache-excluded` | shared-target refusal; excluded requester (records with `--target-dir` argv and a detached-compiler parent, per `census_of` `scan.rs:3992`) | beside `scan.rs:4897` |
  | `cpu-turnover` | every sample as the process set changes | beside `scan.rs:5032-5165`, sequence fixture |
  | `fallback-nested-source-switch` | the source switch plus the nested assertion | beside `scan.rs:3008`, `:3194` |

- Remove each moved scenario from `reader_scenario.py` and its wrapper in `src/shim_registration/mod.rs`.
- Ledger: one row per scenario.

**Part — kept set**

- After this phase the harness lists exactly the seven kept scenarios: `child-source-switch`, `locale`, `root-headings`, `settings-scroll-burst`, `cpu-cache-server`, `quiet-json-long`, `excluded`.

**Files:**
- `crates/cargo-tile/src/progress/capture.rs`
- `crates/cargo-tile/src/registration.rs`
- `crates/cargo-tile/src/shim_registration/reader_scenario.py`
- `crates/cargo-tile/src/shim_registration/mod.rs`
- `docs/testing-reduction-ledger.md`
- `crates/cargo-tile/src/census/scan.rs`
- `crates/cargo-tile/src/census/invocation_cpu_accounting.rs`
- `crates/cargo-tile/src/render.rs`
- `crates/cargo-tile/src/shim_registration/wire.rs`

**Seats:** 2 writers + 1 tester. Unit tests split by file; harness removal has one owner.
- `impl` — `crates/cargo-tile/src/shim_registration/reader_scenario.py`, `src/shim_registration/mod.rs`, `src/shim_registration/wire.rs`, `src/render.rs`, `src/registration.rs`, `docs/testing-reduction-ledger.md`; hub: `docs/testing-reduction-ledger.md`; removes each scenario once its unit test is green
- `test` — `src/progress/capture.rs` unit tests (capture-scan table) and `src/census/scan.rs` tests for the nested, rewrite and root-duplicate rows
- `review` — opens as impl; `src/census/scan_cpu_scenario_tests.rs` (CPU and `fallback-nested-source-switch` rows, declared from `scan.rs`'s test module by test), `src/census/invocation_cpu_accounting.rs`

**Constraints from prior phases:**
- Phase 2: harness at `src/shim_registration/reader_scenario.py`; re-exec selectors use full bin-test paths; wire tests in `src/shim_registration/wire.rs`.
- Phase 4: private observation records and a sequence fixture that returns every sample feed `Census::take`'s stages; the `*_for_test` helpers are gone; covered and app-level scenarios are already removed; the kept readers run in the `cargo-tile-readers` group.
- Ledger doc exists (phase 1).

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh test cargo-tile`
- `bash ~/.claude/scripts/delegate/verify.sh lint cargo-tile`
- The harness lists exactly the seven kept scenarios; ledger reconciles every removed scenario with unit test lines.
- Three consecutive green full-workspace runs.
- macOS nextest green, or recorded unreachable.
- Measurements row present.

### Phase 6 — cargo-mend: batch diagnostics fixtures  · status: todo

#### Work Order

**Goal:** cargo-mend's import, path and visibility diagnostics run their compatible cases in batched fixture crates, keeping every per-case assertion, every negative `--fix` byte assertion, and every compile configuration validation does not already run.

**Spec:**

**Part — cargo-mend diagnostics batching: import and path fixture families**

- Cost model:
  - Each diagnostics test compiles a temporary crate cold (`tests/support/mend_json.rs:23-24` strips `CARGO_TARGET_DIR`).
  - One mend run is one `cargo check` with mend as `RUSTC_WORKSPACE_WRAPPER`.
  - `--fix` with a nonempty fix set is at least two checks (`src/fixes/runner/plan.rs:29` discovery, `apply.rs:53` validation). With an empty set, `MendRunner::apply` (`apply.rs:15`) returns before validation.
  - A plain `cargo check` cannot reuse mend's artifacts, because cargo fingerprints the wrapper.
  - A shared target dir does not help: no dependencies to reuse, and cargo's build lock would serialize tests.
- Batching rules:
  - **Sibling modules in one crate:** one mend run covers them, safe only where the analysis stays inside the module.
  - **Members of one workspace run with `--workspace`** (as `tests/cli_smoke.rs:362`): isolated crates checked in parallel; group members by `mend.toml` setting, one explicit configuration per run.
  - **Fixture identity:** every batched case gets a unique module or member directory, and assertions match the complete reported path including member prefix. Suffix matching (as `forbidden_pub_in_crate`'s `assert_codes` does) is not enough, because fixtures share suffixes like `src/a/b/c.rs`.
  - **Per-case assertions survive:** codes, counts, help text, fixability and resulting file contents, negative cases included. An aggregate count across cases never replaces them.
  - **Never batched:** rollback and failure tests; crate-wide reach, crate-root globs and `unused_pub`; lib-versus-bin, feature and `cfg` cases.
  - **Independent batches stay separate tests** so nextest parallelizes them. Record per batch: cases, mend invocation count, duration.
- Targets:

  | File | Summed today | Change |
  |---|---|---|
  | `prefer_module_import` | 73 s | batch the negative family (`:431`, `:477`, `:523`, `:569`, `:615`, `:961`, `:1069`, `:1127`, `:1168`, `:1235`, `:1451`, `:1535`, `:1778`, `:1820`, `:2574`, `:2939`); merge the dry-run / read-only / clean trio `:653`/`:715`/`:772`; drop the `:5` pre-check |
  | `inline_path_fixes` | 71 s | one crate for the negatives (`:1009`…`:2521`); one multi-module `--fix` for the single-rewrite positives (`:14`…`:2642`); merge the byte-identical trio `:1569`/`:1627`/`:1673`. Inventory the 8 plain `cargo check` calls by target and feature arguments and drop only those whose exact configuration mend's validation compile already runs. Keep the all-features check (`:364`) and the separate `x`, `y` and combined feature builds (`:729`): mend's default check plan (`src/selection/metadata.rs:169`) runs none of them |
  | `imports_at_top` | 11 s | four single-file fix runs become one crate with four files; `moves_cfg_gated_use_carrying_its_gate` (`:120`) and `moves_use_from_cfg_gated_block_carrying_the_gate` (`:202`) stay separate |
  | `import_fixes` | 19 s | the super-path trio `:590`/`:655`/`:715` becomes one fixture |

- Confirm whether a `--fix` after a `--json` run reuses cached findings (`rendering.rs:642` shows reuse for identical runs). If it does, fold paired runs in these files; record the result either way.
- Ledger: rows for merged tests, dropped compile configurations (Kind `compile configuration`, with the configuration named), and the dropped pre-check.

**Part — cargo-mend diagnostics batching: visibility fixture families**

- The cost model, batching rules, fixture-identity rule and never-batched list above apply unchanged, along with the full-path assertion helper and batch fixture builder in `tests/support/diagnostics.rs`.
- Targets:

  | File | Summed today | Change |
  |---|---|---|
  | `forbidden_pub_in_crate` | 51 s | per-setting loops (`:572`, `:653`, `:698`, `:733`) run one workspace per setting. Each setting keeps a real `--fix` run, and the forbidden and permitted arms of `the_exact_boundary_rewrite_is_offered_only_under_required` (`:572`) keep their assertions that declaration and facade bytes are unchanged after it. Replace suffix matching in `assert_codes` (`:2011`) with full-path matching for batched members |
  | `pub_use_fixes` | 43 s | grouped dry-run/apply pairs (`:758`/`:806`, `:870`, `:936`, `:1006`, `:1119`, `:1169`, `:1385`, `:1440`, `:1500`) become one dry run plus one apply over a multi-facade fixture |
  | `narrow_pub_crate` | 32 s | batch the spelling trio `:483`/`:536`/`:584`; keep only the `:1499` feature checks validation does not compile |
  | `allowances`, `facade_subjects`, `overbroad_pub_crate` | 77 s | batch read-only scenarios by config; `overbroad_pub_crate` field pairs `:899`/`:1120`, `:949`/`:1213` |

- Record per file summed and wall time before and after, and the cargo-mend Test Suite estimate. The design audit estimated 422 s summed for all diagnostics falling to about 130–150 s before retained feature builds and `--fix` runs.
- Ledger rows as above.

**Files:**
- `crates/cargo-mend/tests/diagnostics/prefer_module_import.rs`
- `crates/cargo-mend/tests/diagnostics/inline_path_fixes.rs`
- `crates/cargo-mend/tests/diagnostics/imports_at_top.rs`
- `crates/cargo-mend/tests/diagnostics/import_fixes.rs`
- `crates/cargo-mend/tests/support/diagnostics.rs`
- `crates/cargo-mend/tests/support/mend_json.rs`
- `docs/testing-reduction-ledger.md`
- `crates/cargo-mend/tests/diagnostics/forbidden_pub_in_crate.rs`
- `crates/cargo-mend/tests/diagnostics/pub_use_fixes.rs`
- `crates/cargo-mend/tests/diagnostics/narrow_pub_crate.rs`
- `crates/cargo-mend/tests/diagnostics/allowances.rs`
- `crates/cargo-mend/tests/diagnostics/facade_subjects.rs`
- `crates/cargo-mend/tests/diagnostics/overbroad_pub_crate.rs`
- `docs/testing-reduction-measurements.md`

**Seats:** 3 writers. Test-only work split by diagnostics file; the support helpers have one owner, who lands them first.
- `impl` — `crates/cargo-mend/tests/support/diagnostics.rs`, `tests/support/mend_json.rs`, `tests/diagnostics/prefer_module_import.rs`, `tests/diagnostics/forbidden_pub_in_crate.rs`, `docs/testing-reduction-ledger.md`, `docs/testing-reduction-measurements.md`; hub: `tests/support/diagnostics.rs` (full-path assertion helper and batch fixture builder), `docs/testing-reduction-ledger.md`
- `test` — opens as impl; `tests/diagnostics/inline_path_fixes.rs`, `tests/diagnostics/imports_at_top.rs`, `tests/diagnostics/import_fixes.rs`
- `review` — opens as impl; `tests/diagnostics/pub_use_fixes.rs`, `tests/diagnostics/narrow_pub_crate.rs`, `tests/diagnostics/allowances.rs`, `tests/diagnostics/facade_subjects.rs`, `tests/diagnostics/overbroad_pub_crate.rs`

**Constraints from prior phases:**
- CI: cargo-mend runs only in `cargo-mend Test Suite`, with a separate `--no-run` build step (phase 1).
- Ledger and measurements docs exist (phase 1).

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh test cargo-mend diagnostics`
- `bash ~/.claude/scripts/delegate/verify.sh lint cargo-mend`
- The all-features and `x`/`y`/combined feature builds still run; `the_exact_boundary_rewrite_is_offered_only_under_required` still runs `--fix` under forbidden and permitted with unchanged-byte assertions.
- Per-file summed and wall time recorded before and after; measurements and ledger rows present.
