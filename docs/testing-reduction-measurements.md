# Testing reduction measurements

Use measured command wall time and user + system CPU time. Summed per-test durations are not a cost measure: concurrent scheduling inflated them by up to 55× in the design audit. Compare phases only under the same machine state. The user's envfs replacement is outside this project and does not block a phase.

## Pre-change baseline

These are the design-audit records on the pre-change tree, not fresh runs taken while phase 1 peers were editing.

| Scope | Machine state | Wall | CPU (user + system) | Qualification |
| --- | --- | ---: | ---: | --- |
| Whole workspace | Design-audit state; envfs not separately recorded | 90.9 s | 488 s | Includes builds; build and execution not separated |
| cargo-berth | envfs on | 64.7 s | Not recorded | Package run |
| cargo-berth | `/bin` bypassing envfs | 22.9 s | Not recorded | Package run; not a system rebuild |

Historical CI data comes from `gh run view 34887513301 --repo natepiano/cargo-liner --json jobs`, read without a filesystem sandbox on 2026-09-14. [Run 34887513301](https://github.com/natepiano/cargo-liner/actions/runs/34887513301) predates the build/execution step split, so its nextest step durations include test compilation and linking.

| Job | Conclusion | Job elapsed | Relevant step | Step duration |
| --- | --- | ---: | --- | ---: |
| Test Suite | success | 511 s | Run test targets (19:35:50–19:40:58 UTC) | 308 s |
| macOS cargo-tile | success | 382 s | Build cargo-tile (19:32:59–19:34:36 UTC) | 97 s |
| macOS cargo-tile | success | 382 s | Test cargo-tile (19:34:36–19:38:11 UTC) | 215 s |
| macOS cargo-tile | success | 382 s | Lint cargo-tile (19:38:11–19:38:42 UTC) | 31 s |
| cargo-mend Test Suite | skipped | Not applicable | No steps executed | Not measured |

## End-of-phase protocol

After both peer seats post `done`, record machine state: `envfs: on|off` from `findmnt /bin`, the full `rustc -V` output, profile `dev`, and warm cache. Run long commands in the background and collect their exit status and output. Keep raw output, including GNU time's user, system, and elapsed fields and nextest's summary. Do not mix results from different machine states.

1. Build: `command time -v cargo nextest run --workspace --all-targets --no-run`; record wall and user + system CPU.
2. Execution: `command time -v cargo nextest run --workspace --all-targets` three consecutive times. Record each run's wall, CPU, and nextest summary; report median and range of wall and CPU. All three runs must be green before closing the phase.
3. Packages: run `command time -v cargo nextest run -p <pkg> --all-targets` once for each of `cargo-berth`, `cargo-tile`, `cargo-mend`, and `cargo-port`. Record wall, CPU, test count, and the longest test's name and duration.
4. Reconcile the diff and coverage ledger with `cargo nextest list -p <pkg>` before and after. Compare against the previous phase under the same machine state. The pre-change figures above cannot establish a build/execution split retroactively.

## Phase 1

Observed machine state during implementation: `envfs: on` (`findmnt /bin`: `/bin envfs fuse`), `rustc 1.98.1 (48a229cea 2026-09-01)`. Measurement profile: `dev`, warm cache.

The original final Verification section allowed only scoped berth verification and prohibited raw Cargo. A follow-up instruction explicitly authorized before/after nextest inventories and the release resolver check; those are now complete below. The separate workspace build, three full-workspace executions, and four package wall/CPU measurements remain pending for an orchestrator run. The scoped debug gate is not a substitute for this protocol.

| Phase | envfs | Build wall / CPU | Execution wall median (range) | Execution CPU median (range) | Full-workspace summaries | Comparison |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | on | Pending authorized protocol run | Pending | Pending | Pending | No comparable phase row yet |

Release checks are provided as `ledger::lock::tests::lock_contention_tolerance_ignores_override_in_release` and `cli::tests::gate_deadline_ignores_override_in_release`, gated by `cfg(test)` plus `not(debug_assertions)`. The subsequently authorized command `cargo test --release -p cargo-berth ignores_override_in_release` exited 0: both release-only resolver tests passed, zero failed or ignored; all unrelated tests were filtered out. Compilation took 42.78 s as reported by Cargo. This check proves each environment variable is ignored in release mode, including when its value is zero.

The gate outer timeout and lock timeout share the contention exit code but have distinct diagnostics: `10-second total deadline` versus `10-second lock deadline`. Both messages use the production duration constants even when debug overrides shorten the effective deadline. `Ledger::try_transact` still supplies `Duration::ZERO`; the resolver is applied inside `MutationLock::acquire`, so every caller receives the same shorten-only policy without changing the bypass-audit call site.

The readiness file is written after the lock acquisition timer starts. Integration elapsed measurement therefore begins immediately before launching the child; readiness is confirmed before collecting output and checking both bounds. Starting the test timer after polling observes readiness would incorrectly subtract part of the deadline. Fixture setup is outside the timed section. Contention budgets are 300 ms with a named 5 s scheduling allowance; the new outer-gate case uses 1000 ms with the same allowance.

Intermediate scoped verification: berth peer reported 634/634 tests, zero skipped, nextest summary 74.056 s; all rewritten timed sections passed. Several complete test scenarios exceeded 10 s, so the literal no-test-at-or-above-10-seconds acceptance criterion remains unmet. The full-test duration includes setup and other work outside the shortened contention interval; this observation is not a pre-phase comparison.

Cargo-port peer's final scoped verification (after its final source edits): 1170 passed, zero skipped, nextest summary 2.240 s; longest test `watcher::runtime::tests::source_events_schedule_lint_run_through_main_runtime`, 1.514 s. Its nightly format and clippy gates passed. These scoped results do not provide the protocol's wall/CPU package measurements.

CI YAML parsing passed with an existing cached PyYAML package supplied through `PYTHONPATH` (bytecode writes disabled). A structural check confirmed identical build/execution flags and environments for all three pairs, and `.config/**` in the shared filter. No CI jobs were triggered by this phase.

### Final scoped gate after both peers posted done

`bash ~/.claude/scripts/delegate/verify.sh test cargo-berth` exited 0. Nextest run `c9be6133-9591-4bf3-8aa3-f5dddc4c9de5` reported `Summary [72.549s] 634 tests run: 634 passed, 0 skipped`. Longest remaining berth test: `edges::predecessor_graph_has_fixed_cold_cost`, **50.459 s**. This is the observed per-test duration, not a summed cost estimate. The literal acceptance criterion that every berth test takes less than 10 s is unmet, although every shortened deadline assertion passed.

The final berth lint retry exited 0, with nightly formatting and clippy both executed and no skipped checks. Its first attempt had failed writing a missing `zbus_macros` build fingerprint; the same unrestricted invocation passed on retry. No pre-existing-failure claim is made.

The source diff inventory reconciles to three added debug unit tests (two resolver tests and one diagnostic test), two additional release-only resolver tests, one renamed gate contention test, one added outer-timeout integration test, and one deleted assertion-free cargo-port timing-print test. All other existing test names remain. The coverage ledger contains 14 rows. The subsequently authorized before/after nextest inventories confirm this source inventory, as recorded below.

No machine-state-matched phase performance improvement is claimed: the full measurement protocol has not run. The orchestrator must run the separate workspace build, three consecutive green full-workspace executions, four package measurements before accepting phase 1. Native macOS/Windows and the required post-push CI conclusions were not verified by this phase.

### Authorized inventory follow-up

Both inventories used `cargo nextest list -p <package> --message-format json`, with the before source extracted from commit `ea7d986ac61700c48765a0ad5eff7d949e8a2173` into `/tmp/cargo-liner-phase1-inventory-au9sr8ck` and the after source taken from the current workspace. Generated test executables were invalidated before switching source trees to prevent the shared target directory from supplying a stale binary. The final inventories were rebuilt from the named source trees and all commands exited 0. Raw JSON is retained as `berth-before.json`, `berth-after.json`, `port-before.json`, and `port-after.json` in that snapshot directory.

| Package | Before | After | Exact inventory change |
| --- | ---: | ---: | --- |
| cargo-berth | 630 | 634 | Renamed `hook_lock_contention_uses_one_ten_second_deadline` to `hook_lock_contention_uses_one_shortened_lock_deadline`; added `hook_outer_gate_deadline_expires_while_worker_waits_on_the_mutation_lock`, both debug resolver tests, and the diagnostic unit test |
| cargo-port | 1171 | 1170 | Removed only `watcher::runtime::tests::register_watch_roots_reports_elapsed_for_representative_roots`; no additions or renames |

All seven rewritten berth integration tests remain, with the one documented rename. The release-only resolver cases are correctly absent from these debug inventories and passed in the separate release command.
