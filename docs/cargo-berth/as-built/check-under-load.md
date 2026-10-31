# cargo-berth: edit check under load

## What it is

`cargo-berth check` is the pre-edit question both agent harnesses ask: may this session write these paths? When the caller's selected reservation already covers every requested path and the lock-free snapshot shows no conflict, the check answers `clear` / `already_held` at once. It takes no mutation lock, runs no reconcile and makes no coordination change; a read-only replay can still rewrite its replay checkpoint (`ledger/replay_checkpoint.rs`). When a reconcile is needed, it observes each holder worktree on its own thread instead of one after another. That observation is prepared outside the mutation lock and validated against the journal and session-mapping versions when it publishes; after `MUTATION_PREPARATION_ATTEMPTS` (8) invalidations it prepares under the lock (`ledger/preparation.rs`). A reconcile failure inside `check` answers with its own envelope: at the lock deadline that is `contention`, exit 6.

The problem was measured on natedev in `hana_catalyst-inspectors` (17 worktrees, load average 63–127). Every check reconciled whenever any foreign reservation was unreleased, which was always true there. One traced run waited 2.6 s for `mutation.lock`, held it for about 10 s, and ended at 14.3 s. Of the time under the lock, 5.5 s was 11 sequential `git status --porcelain -z` calls (0.15–1.32 s each); `update-ref --stdin` took 1.2 s and the rev-lists about 1.5 s. Other runs ended at 5.8 s, at 23 s, or at the 10 s lock deadline. Checks queued behind each other's reconciles, so edit hooks reached their 25 s limit and blocked clean edits.

## How it works

### Two callers, one engine

- **Claude Code.** `hook/pre_tool_use.rs::execute` resolves the edit target, `set_current_dir`s to the repository root, selects the payload's harness session id, and builds a `CheckRequest` with `CheckReservationSelection::SessionMappingOrSingleActive`. It calls `check::execute`. `render_pre_tool_use_answer` maps the exit code to a hook result:
  - `Clear` allows the edit.
  - `LedgerUnreadable` (4) fails open: the edit is allowed with a notice.
  - `BlockedByOverlap`, `BlockedByOrdering`, `UsageError`, `BlockedByContention` (6) and `TerminalViewFailed` refuse. The hook writes the engine's rendered blocks to stderr and exits 2.
- **Codex seats.** The Codex pre-edit hook (`~/.claude/scripts/hooks/pre-tool-use-berth.py`) runs `~/.claude/scripts/direct/phase_reservation.py`, which runs `cargo-berth check <scopes> --json` from the repository root with `CARGO_BERTH_SESSION_ID=<director session>.phase-<n>` and `CHECK_TIMEOUT_SECONDS = 25`. The CLI parses a bare check to `SessionMappingOrNew` and `--reservation <id>` to `Explicit(id)`.

Both callers reach `verb/check.rs`:

```rust
pub(crate) fn execute(
    check_request: CheckRequest,
    recovery_command_line: &RecoveryCommandLine,
) -> OutputEnvelope
```

### Data flow in `check::execute`

`execute` runs `execute_with_capacity_recovery` with a `ClaimCapacityRecovery` and adds the releases it committed to the answer. The capacity scan at `maximum_reservations` is described in [worktree-coordination.md](worktree-coordination.md).

`execute_with_capacity_recovery` calls `RepositorySnapshot::discover` once. Every decision pass, the reconcile and the locked acquisition take that snapshot. Each `decide` pass reads `check_repository_observation`, which is `RepositorySnapshot::fresh_observation`: it reads status, HEAD and the worktree list again, and keeps the object reader, the merge-pair cache, the trunk-ancestry cache and the `git merge-tree --stdin` sessions. So one check runs each merge pair and each trunk history read once.

1. **`decide`, lock-free.**
   - `Ledger::read_for_edit_check(invocation_directory)` returns an `EditCheckLedgerSnapshot`: the replayed `RetainedReservationSet` (or its replay error), the `WorktreeContext` and the full `BerthConfig`. It is read without git, locking, repair or publication; replay goes through the checkpoint.
   - `decide` reads `PathCase` and builds two antichains: exact-file scopes for acquisition and minimal scopes for refusal.
   - It resolves identity once through `ledger::resolve_identity` and observes `ActingHeadContainment::observe_for_edit`. That observation judges only the holders whose protection touches the requested scopes. A disjoint holder costs no history read and no merge pair, because containment can only remove protection.
   - It validates the coordination identity and the worktree occupancy. A stale or released session mapping fails here with `invalid_input` / `stale_session_mapping`, exit 5, before any lock.
   - It computes `conflicts_for_edit`.
   - With no conflicts and a parseable `configuration.repository_trunk()`, it calls `claim::selected_reservation_check_acquisition`. Otherwise the acquisition is `NeedsLockedDecision`.
   - It sets `MergeObservationRequirement::Required` when any reservation from another worktree is not `Released`.
2. **Fast return.** `CheckDecision.acquisition` is a `CheckAcquisitionDecision`:
   ```rust
   pub(super) enum CheckAcquisitionDecision {
       HeldBySelectedReservation { scopes: ReservationScopeSet, acquisition: ReservationAcquisition },
       NeedsLockedDecision,
   }
   ```
   `HeldBySelectedReservation` returns `OutputEnvelope::clear_check(scopes, acquisition, None)` before any other branch.
3. **`Required`** goes to `reconcile_and_retry`.
4. **`NotNeeded` with no conflicts** goes to the locked `acquire_first_touch`. A `Blocked` answer there falls to `reconcile_and_retry`.
5. **Any other case** goes to `reconcile_and_retry`.

`reconcile_and_retry(invocation_directory, repository, declared_scopes, reservation_selection, recovery_command_line, capacity_recovery)`:

1. Runs `reconcile::reconcile_for_check(repository)`.
2. On failure, returns `error.into_output(CommandVerb::Check)`.
3. Otherwise calls `decide` again.
4. With no conflicts, or a configured `default_answer`, runs the locked `acquire_first_touch`; otherwise records the refusal through `refusal::record` and answers `blocked_check`. Reconcile alerts, retention maintenance and automatic releases are attached to either answer.

### Fast-path selection (`verb/claim/acquisition.rs`)

```rust
pub(super) fn selected_reservation_check_acquisition(
    reservations: &RetainedReservationSet,
    repository_trunk: &IntegrationTarget,
    scopes: ReservationScopeSet,
    selection: CheckReservationSelection,
    identity: ResolvedEditAuthorization,
    path_case: PathCase,
) -> CheckAcquisitionDecision
```

| Selection | Identity | Result |
| --- | --- | --- |
| `SessionMappingOrSingleActive` / `SessionMappingOrNew` | `EditAuthorization::Session { reservation_id, .. }` | the mapped reservation |
| same | anything else (unmapped session, `CARGO_BERTH_RUN`, marker) | `NeedsLockedDecision` |
| `Explicit(id)` | `Session { reservation_id }` with `reservation_id == id` | `id` |
| `Explicit(id)` | harness session `Unavailable` | `id` |
| `Explicit(id)` | any other identity (the mapping would be created or moved) | `NeedsLockedDecision` |

The selected reservation must pass `FirstTouchReservationSelection::from_explicit_selection`, which requires it to be active for the acting run and worktree. Then `partition_first_touch_protected_scopes` must return `AlreadyHeld`: every requested exact-file scope is covered by an active reservation of the selected reservation's run and worktree, siblings included.

The answer is built by the helpers the locked path uses:

- `already_held_acquisition` records `kind: AlreadyHeld`, the selected reservation's id, its target and its `phase_start_head`, with `already_held: vec![]`.
- `first_touch_acquisition` sets `CoordinationRunMarkerPublication::Published`.
- `selection.reported_session_mapping_publication(SessionIdentityMappingPublication::Published)` sets the mapping publication; `Explicit` maps through `for_explicit_reservation_selection`.

The message is "No foreign reservation overlaps the requested paths; the acting run already holds them." (`output.rs::clear_check`).

### Concurrent holder observation (`reconcile.rs`)

The reconcile transaction's preparation (`transact_reconciliation`, outside the lock unless preparation fell back under it) calls `complete_reconciliation_plan`, which calls `derive_merge_extents`. That delegates to:

```rust
fn derive_merge_extents_with_observer(
    reservations: &RetainedReservationSet,
    snapshot: &RepositorySnapshot,
    planned: &[JournalOperation],
    git_cost: &mut MergeExtentGitCost,
    observe: impl Fn(&Reservation, &RetainedReservationSet, &RepositorySnapshot,
                     &[JournalOperation], &mut MergeExtentGitCost)
        -> Result<HolderMergeProtection, String> + Sync,
) -> Result<MergeExtentReconciliation, ReservationReplayError>
```

In production the observer is `observe_merge_extent`; tests inject their own. `observe_holder_merge_extents`:

- Collects the distinct holder keys `(WorktreeId, recorded IntegrationTarget)` of non-released reservations, in first-seen order. One worktree with two targets is two keys.
- Inside `std::thread::scope`, spawns one thread per key and pushes every handle into a `Vec` before joining any.
- Gives each thread its own `MergeExtentGitCost`.
- Joins in first-seen order, sums `worktree_status_queries` and `path_queries` into the caller's counter, and re-raises an observer panic with `resume_unwind`.
- Returns a `HashMap<(WorktreeId, IntegrationTarget), Result<HolderMergeProtection, String>>`.

The fold that follows walks unreleased reservations in replay order, looks each holder up in the map, and gives each reservation its share of the observation through `reservation_merge_extent` (a failed observation becomes that reservation's own `unavailable` extent). It groups equal changed shares per holder into `HolderMergeExtentObserved`, then appends `ResolveIncursion` for incidents whose extent is now empty. The share split and its cache key are in [parallel-phases.md](parallel-phases.md).

### Reconcile failure envelope

```rust
impl ReconcileError {
    pub(crate) fn into_output(self, command_verb: CommandVerb) -> OutputEnvelope
}
```

| Failure | Answer |
| --- | --- |
| `Transaction(LockContention)` | `OutputEnvelope::contention`: status `contention`, exit 6, empty `blocked_by` and `reservations`; message "another cargo-berth operation is still running; wait for it to finish, then retry" |
| `RewriteSubjectChanged` | `contention` |
| `Git`, `WorktreeRegistry`, `EdgeReplay`, `MissingReadinessFact` | `ledger_unreadable`, exit 4 |
| `Config`, `Ledger` | `ledger_error` |
| `Replay` | `replay_failure` |
| `CorrectableInput` | `invalid_input` |

The hook's contention presentation leads with "cargo-berth rejected this edit because another cargo-berth operation still holds the ledger lock."

### Files

- `crates/cargo-berth/src/verb/check.rs`: `execute`, `decide`, `CheckDecision`, `MergeObservationRequirement`, `reconcile_and_retry`, `check_repository_observation`, the fast return.
- `crates/cargo-berth/src/git/snapshot.rs`: `RepositorySnapshot`, `fresh_observation`, the merge-pair and trunk-ancestry caches, and one merge-tree session per checkout.
- `crates/cargo-berth/src/reservation/containment.rs`: `ActingHeadContainment::observe_for_edit`.
- `crates/cargo-berth/src/verb/claim/acquisition.rs`: `CheckReservationSelection`, `CheckAcquisitionDecision`, `selected_reservation_check_acquisition`, and the shared `partition_first_touch_protected_scopes`, `already_held_acquisition` and `first_touch_acquisition`.
- `crates/cargo-berth/src/ledger/handle.rs`: `read_for_edit_check`, `EditCheckLedgerSnapshot::into_parts`.
- `crates/cargo-berth/src/reconcile.rs`: `reconcile_for_check`, `derive_merge_extents[_with_observer]`, `observe_holder_merge_extents`, `MergeExtentGitCost`, `ReconcileError::into_output`.
- `crates/cargo-berth/src/output.rs`: `clear_check`, `contention`.
- `crates/cargo-berth/src/hook/pre_tool_use.rs`: the Claude Code caller and its exit-code mapping.

## Invariants

- The fast path takes no lock and makes no coordination change: no journal append, no session mapping, no marker. A read-only replay may still rewrite the replay checkpoint.
- The fast path's envelope (message, scopes, acquisition, publication fields, exit code) is identical to the locked path's answer for the same state. Both are built from the same helpers; do not fork them.
- The fast path answers only with no conflict, a reservation selected through an existing mapping or a matching explicit id, and the run's active reservations in this worktree covering every exact-file scope. Every other case takes the locked path.
- Any selection that would create or move a session mapping goes through the lock; remapping is a durable write.
- Identity resolves through `ledger::resolve_identity`: `decide` resolves it on each pass, and reconciliation and acquisition resolve it again. A session mapping outranks `CARGO_BERTH_RUN` and the marker. Identity validation runs before the fast path.
- The fast-path outcome is a type (`CheckAcquisitionDecision`), not a flag.
- Concurrency does not change reconcile output: the same journal operations in the same order with the same JSON, and git cost counts equal to the serial sum. Holder dedup stays keyed on `(worktree, recorded target)` in first-seen order.
- Every observer starts before any is joined. A lazy spawn/join iterator runs them one at a time.
- A reconcile failure in `check` answers with its own envelope, never as an overlap with no holder.
- One check owns one `RepositorySnapshot`. Each merge pair, each trunk history read and each checkout's `git merge-tree --stdin` session start at most once in it. Later passes refresh only mutable facts: status, HEAD and the worktree list.
- No test may take a second.

## Calibration and gotchas

- **Concurrent Git pipe creation.** Holder observation runs Git on several threads at once. Close-on-exec closes a descriptor at exec, not at fork. A child forked on one thread while another thread sets up its pipes holds a copy of that pipe's writer until it runs exec, or for its whole life if it is a persistent reader such as `cat-file`. The reader of that pipe then does not see EOF after its own child exits.
  - `git/spawn_lock.rs` owns the one process-wide spawn lock (`PROCESS_SPAWN`, `with_spawn_lock`). Every product start takes it for pipe creation and spawn: `git/command.rs`, `git/child.rs`, and the detached managed-hook refresh in `cli.rs`.
  - `git/installation.rs` runs the installation-configuration probe as a bounded child inside the lock, once per process, and passes the file to `gix::open_opts`. `gix` starts no probe of its own. `git/read.rs` also calls `gix::open_opts` inside the lock.
  - In unit tests, `git::command::with_spawn_lock` uses the `cargo-berth-test-support` copy of the same lock, so product reads and fixture starts in one test process share it. Test support starts every fixture command through `FixtureCommand`, which takes that lock.
  - Collection releases the lock before it waits for output. After the child exits, `git/child.rs` stops the child's process group and polls the kept pipe read ends (`rustix::event::poll`, zero timeout). Pipe EOF is a kernel fact, so a slow drain thread loses no output. A writer that is still open fails the command with "child exited with inherited pipe writers still open"; a descendant that closed its pipes is not an error.
  - Tests: `installation_probe_seals_its_pipes_before_another_git_child_starts` (`git/read.rs`) pauses the probe and checks that another Git spawn cannot pass it. `fixture_commands_cannot_spawn_during_another_commands_pipe_setup` (test support `berth_command.rs`) checks the same for fixture starts. `exited_child_output_survives_a_delayed_drain` and `exited_child_output_ignores_descendants_that_closed_their_pipes` (`git/child.rs`) cover the EOF check.
- **Product-cost regressions count work.** `src/test_work.rs` has test-only, per-thread counters at the operations themselves: scope keys, pairwise comparisons, incursion reservation scans, ancestry-parent reads, history object decodes, history reads, mutation-lock attempts and mutation-lock waits. The retention, antichain, batched reachability and lock tests bound those counts, not elapsed time. `queued_writer_acquires_without_a_retry_sleep_after_release` (`ledger/lock.rs`) asserts one try-lock and one kernel-queued wait (`MutationLockRequest::{Granted, Queued}`). `check_reuses_merge_pairs_across_decision_passes` (`verb/check.rs`) asserts that a second decision pass starts no merge-tree query. Lock-contention, concurrent-progress and process-cleanup deadlines still test their own behavior.
- **Fixture isolation.** Shared Git seeds are built through `prepare_fixture_build` under `target/tmp/cargo-berth-templates`; each test receives private files, index and refs. Recorded templates are built by the nextest setup script `tests/setup-recorded-templates.sh`. Seeds cache setup history, while commands whose behavior a test asserts still execute in that test. A seed-isolation regression changes one copy's staged content and branch tip and verifies another copy remains unchanged.
- **Process-start pin.** `tests/process_start_budget.json` holds ceilings from a complete cold Mac capture of the suite and setup: 42,291 starts in all, 17,184 `product_git`, 6,215 `fixture_git`, 14,265 `other`, 3,341 `berth`, 1,258 `hook_berth`, 9 `hook_git`, 1 `berth_worker` and 18 `lock_diagnostic`, with at least 1,306 tests, plus a ceiling for each listed expensive test. `completed_process_report_obeys_budget` (`tests/process_start_budget.rs`) checks a reduced report only when `CARGO_BERTH_PROCESS_COUNT_REPORT` names one; it also requires zero product starts with no known caller. An ordinary test run cannot count its peer processes. A change that adds process starts updates the pin from a new complete capture. Product Git reads that `gix` can answer start no process: exact commit objects, commit history (`git/history.rs`) and merge-result tree paths. A repository with replacement refs, a shallow boundary or legacy grafts keeps the Git child (`GitReadRequirement::HistoryOverrides`), and revision expressions keep the Git batch reader.
- **Lock diagnostics.** On macOS a lock refusal runs one `/usr/sbin/lsof` scan, with its own `lock_diagnostic` origin and ceiling of 18. Spawn-to-reap took 133–277 ms in an instrumented capture.
- **Timings and limits.**
  - The lock deadline is `MUTATING_VERB_CONTENTION_TOLERANCE` = 60 s. `MutationLock::request` tries the lock once; when it is held, one helper thread queues a blocking `File::lock` in the kernel, so the wait does not poll.
  - The edit hook's limit and the seat launcher's check limit (`CHECK_TIMEOUT_SECONDS` in `phase_reservation.py`) are 25 s, so a request now reaches the caller's limit before the lock deadline.
  - At a load average near 30 each holder's `git status` costs about 10 ms, so the concurrency gain shows only under heavier load.
  - Concurrency launches up to one `git status` per live holder key at once, about 20 processes in hana.
- **Remaining costs.** One checkpoint decode measured 135 ms (hana: 356 MB journal, 25 MB `replay-checkpoint.json`). The fast path still pays one decode, plus `PathCase::read` and head-containment reads, which touch git only when a foreign holder keeps keyed committed protection.
- **Stale holder state.** The fast path trusts retained foreign protection. If a foreign worktree dirtied a held path since the last reconcile, this edit does not see it; the next reconcile finds it, and resolving it falls to the foreign holder as an incursion.
- **Marker and mapping report `published` without writing.** That is accurate because the mapping already exists.
- **Sibling coverage answers with the caller's reservation.** A path held by a same-run sibling answers `already_held` naming the caller's mapped reservation, not the sibling, as the locked path does.
- **`left_unmapped` only for unmapped sessions.** The locked path answers `left_unmapped` only for an unmapped session; a mapped session gets `already_held` with its own reservation, `published`.
- **Explicit selections.**
  - An explicit `--reservation` naming the session's mapped id takes the fast path.
  - With a harness session present, an explicit id that would create or move the mapping locks.
  - With no harness session id, any eligible explicit id takes the fast path, since there is no mapping to move.
- **Released mapping.** A released session mapping is not a locked-path case: identity validation answers exit 5 before any lock.
- **Other declines.** The fast path also declines when `repository_trunk()` fails to parse, and when a tree request is only partly covered; the tree case widens under the lock.
- **A lock deadline hides real conflicts.** A check whose lock-free snapshot showed a real conflict answers `contention` at the lock deadline, not `blocked_by_overlap`.
- **Git failure fails open.** A reconcile git failure (missing git, broken marker) answers `ledger_unreadable`, exit 4, which the Claude Code hook renders as fail-open with a notice.
- **Clippy.** `needless_collect` rejects the `map(spawn).collect()` form, so the spawn loop pushes handles into a `Vec` explicitly.
- **Held-lock fixture.** The test process opens `.git/cargo-berth/mutation.lock`, takes it with `File::try_lock`, and runs `check` with `CARGO_BERTH_TEST_LOCK_CONTENTION_TOLERANCE_MS`. "Took the fast path" becomes a timing-free assertion: success while the lock is held.
  - The variable only shortens the deadline, and only in debug builds.
  - `check_at_held_mutation_lock` (`tests/lifecycle.rs`) uses 50 ms with a 500 ms bound on the whole run; the hook test uses the same pair.
  - `a_session_checks_its_held_path_without_waiting_for_the_mutation_lock` uses 100 ms, asserting the held path answers under 500 ms and the unheld path waits at least the deadline.
- **Locked references.** A check with the lock free is not a locked reference, because the fast path answers first whenever it applies. A locked reference needs a state the fast path declines.
  - `fast_path_envelope_equals_the_locked_already_held_answer` gets its locked `already_held` from an explicit `--reservation` for a session not yet mapped, which must lock to publish the mapping.
  - The repeat under the held lock must equal it field for field, with journal and mappings unchanged.
- **Contended controls.** `assert_check_contention` pins status `contention`, exit 6, verb `check`, empty `blocked_by` and `reservations`, and the busy diagnostic, for the unheld control, the partial tree, the unmapped session, the explicit remap and the envelope reference.
- **Concurrency proof.** `merge_extent_observations_run_concurrently_in_holder_order` uses three holder keys plus a sibling sharing one key. The three observers meet at a `Barrier::new(3)` inside an injected observer, and the result is received with a 400 ms timeout; a serial run would deadlock at the barrier and fail at the timeout. It also checks sibling grouping, operation order and summed costs (3 status, 6 path).
- **Equivalence proof.** `concurrent_merge_extents_match_serial_targets_and_unavailable_holder` builds a real Git fixture with `merge_extent_comparison_fixture` (two holders on one worktree with different targets, plus one unavailable holder) and compares the concurrent output with `serial_merge_extent_reference`, which calls the real `observe_merge_extent` serially. Operations JSON, observed holder extents, and costs all match.
- **Hook tests.**
  - `pre_tool_use_allows_a_held_claude_edit_while_the_mutation_lock_is_held` gives the session id through the hook payload, with environment ids removed.
  - `pre_tool_use_reports_reconcile_contention_as_busy_without_naming_a_holder` asserts the refusal names neither the foreign reservation id, nor "Holder", nor "overlap".
- **Test cost.** Each of these tests takes 0.02–0.27 s.

## Why

- **Answer held paths first.** A clean edit the session already holds should not wait behind other sessions' reconciles. The locked path's answer for that state is fully determined by the snapshot, so the fast path builds the same answer without the lock.
- **A sibling-held path counts as held.** Drift treats a run's active reservations in one worktree as one party. Restricting the fast path to the selected reservation's own scopes was ruled out: the locked answer is identical, and the restriction would slow sibling-held edits.
- **No lock-free remap.** A remap needs a durable mapping write, which belongs under the lock.
- **No "already current" mapping-publication variant.** `published` is already accurate when the mapping exists.
- **Observe holders together.** The `git status` calls are independent per holder key and were the largest share of the time under the lock. Running them concurrently shortens the lock hold without changing what is recorded.
- **Observe outside the lock, validate at publish.** Preparing observation outside the lock shortens the hold; publication checks the journal and session-mapping versions, and repeated invalidation falls back to preparing under the lock. This shortens lock holds, not the caller's wait: request latency still includes the observation itself.
- **Busy answers as busy.** Reporting a reconcile failure as `blocked_by_overlap` with no holder sent the caller to resolve an overlap that did not exist. `contention`, exit 6, tells it to retry.
- **One fix point.** Both harnesses reach `check::execute`, so a change there covers Claude Code and Codex.

## Lock-hold measurements on the Mac

Each value is the longest mutation-lock hold in one command, from 20 samples per path and side at load below 10. The p95 is the nearest-rank percentile. "Before" is a tree whose writes held the lock for their whole preparation. Each "after" p95 is below 100 ms. Use the same six paths and the same sample rule to measure a change to the lock path.

| Path | Side | Samples | Median (ms) | p95 (ms) | Maximum (ms) | Load range | Expected decision |
|---|---|---:|---:|---:|---:|---|---|
| repair | before | 20 | 5498.762 | 8650.217 | 8772.874 | 2.768–9.518 | Exit 0; success |
| repair | after | 20 | 5.972 | 7.518 | 7.729 | 2.961–7.541 | Exit 0; success |
| reconcile | before | 20 | 8726.289 | 13238.593 | 16550.776 | 2.604–7.184 | Exit 0; success |
| reconcile | after | 20 | 25.889 | 33.458 | 51.062 | 1.453–9.398 | Exit 0; success |
| claim | before | 20 | 3584.033 | 5400.983 | 7373.506 | 2.555–9.800 | Exit 0; success |
| claim | after | 20 | 20.573 | 27.461 | 31.683 | 1.530–9.401 | Exit 0; success |
| post-tool-use | before | 20 | 2935.513 | 4505.761 | 4529.078 | 2.555–7.931 | Exit 0; success |
| post-tool-use | after | 20 | 20.673 | 29.381 | 46.813 | 1.487–9.209 | Exit 0; success |
| release | before | 20 | 3119.932 | 4330.964 | 18480.283 | 2.670–7.937 | Exit 0; success |
| release | after | 20 | 33.312 | 49.213 | 50.611 | 1.487–9.192 | Exit 0; success |
| gate | before | 20 | 4119.927 | 5939.525 | 6252.985 | 3.045–7.937 | Exit 0; observe-only refusal, five held reservations |
| gate | after | 20 | 22.741 | 35.000 | 41.006 | 1.487–9.329 | Exit 0; observe-only refusal, five held reservations |
