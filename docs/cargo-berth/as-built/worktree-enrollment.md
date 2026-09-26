# Worktree enrollment in `cargo berth init`

## What it is

`cargo berth init` can be run in a repository where work is already under way in several worktrees. It writes the configuration at the main worktree root, where every worktree reads it. It then gives each live worktree that has never held a reservation one reservation covering its current work: every path merging its branch into its selected target would change or conflict on, plus its staged, unstaged and untracked paths. When two worktrees' reservations share paths, both sides can keep editing, but integration is held until the user picks an order with `sequence`. Running `init` again is safe: it enrolls only worktrees that still have no reservation history, and it reports the overlaps that are still unresolved.

## How it works

### Configuration location

- `BerthConfig::initialize(&ConfigurationLookup)` decides where to write:
  - For `ConfigurationLookup::OwnThenMain`, it writes `.claude/config/berth.toml` at `main_repository_root`.
  - For `ConfigurationLookup::Own`, it writes at the invoking root.
  - `WorktreeContext::configuration_lookup()` returns `OwnThenMain` only for a linked worktree whose common git directory is named `.git`; the main root is that directory's parent. Everything else gets `Own`.
- If a valid file already exists at the target, it is kept (`InitializationState::Existing`), and the linked worktree's file is never read.
- If the target is missing and the invoking linked worktree has its own valid file, that file's `trunk`, `gate_mode`, `maximum_reservations` and `maximum_ordering_edges` are written into the new main file. Otherwise defaults are written. The linked file is left alone. Its limits and gate mode take precedence locally, while the main file alone supplies `trunk` once it exists.
- `read_file` returns a private `ConfigurationFilePresence::{Missing, Present(BerthConfig)}`. `read` checks the worktree's own file first; when the main file exists, its `trunk` replaces the linked value. Without a main file, the own file supplies the fallback.
- `BerthConfig::relative_path()` (`.claude/config/berth.toml` relative to a worktree root) is `pub(crate)` so drift observation can compare against it.
- `Ledger::initialize` calls `WorktreeContext::discover` before taking the initialization lock and passes `configuration_lookup()` to `BerthConfig::initialize`.

### Enrollment flow

`initialize_ledger` in `cli.rs` calls `worktree::enroll_worktrees(&WorktreeContext, &BerthConfig) -> Result<WorktreeEnrollmentReport, LedgerError>` after configuration and hooks are set up. An `Err` (opening or reading the ledger, reading `PathCase`) turns into `initialization_error`. Anything that goes wrong for a single worktree goes into `report.failures` instead.

1. **Read state once.** It reads validated events and the worktree registry. If `WorktreeRegistry::read` fails, it records one `git_failure` for the invoking root and still reports unresolved overlaps.
2. **List candidates.** `WorktreeRegistry::enrollment_candidates()` returns a `WorktreeEnrollmentCandidate` per registration from that one read:
   - `Eligible(WorktreeContext)` for `Available` or `Locked` registrations that were `Discovered`.
   - `Unavailable { root }` for everything else, reported as `unavailable`.
   - If `ledger::worktree_identity` fails for a candidate, that is also reported as `unavailable`.
3. **Check history.** `reservation_history(events, worktree_id)` returns `AlreadyReserved` if any `JournalOperation::Claim` has that worktree as actor, whatever its source or lifecycle. Otherwise it returns `NeverReserved`.
4. **Observe the footprint** (`observe_footprint`, all Git, no lock):
   - Any entry in `OPERATION_IN_PROGRESS_MARKERS` present in the administrative directory → `operation_in_progress`.
   - HEAD via `git::head_object_id`, then `read_enrollment_target`: `ledger::resolve_claim_target` with `TargetSelectionRequest::AutomaticAcquisition` selects `branch.<name>.cargoBerthTarget`, else the repository trunk (always for a detached HEAD). The trunk comes from `BerthConfig::repository_trunk()`; `enroll_worktrees` checks it once before any footprint, so a trunk that is not a local branch fails `init`, and `read_enrollment_target` repeats the check as `configuration`. A setting naming the claimant's own branch or a branch that does not resolve falls back to the trunk and records `fallback: Some(TargetFallback { requested, reason })`. The target commit comes from `git::branch_object_id`. A `GitError::CommandFailed` here → `no_merge_base`; other errors → `git_failure`.
   - `git merge-base target head`. Exit code `MERGE_BASE_NO_COMMON_ANCESTOR_EXIT_CODE` (1) → `no_merge_base`; any other failure → `git_failure`.
   - Paths = `git::unmerged_branch_paths(root, target, head)` ∪ `drift::observe_merge_working_tree(root)` tracked ∪ untracked. The result becomes exact file scopes via `DeclaredReservationScopeSet::from_file_paths(..).into_exact_file_antichain(path_case)`. No paths → `FootprintObservation::Empty`, which is neither enrolled nor reported.
   - `git::head_attachment` gives `ClaimHeadSnapshot::Branch { full_ref, head }` or `Detached { head }`, and the display string (the full ref, or `detached at <sha>`).
   - Footprints are also observed for worktrees that already have history. Those are used only to compute the pairwise overlap checks below, and their observation failures are dropped silently.
5. **Enroll each `NeverReserved` footprint** (`enroll_candidate`):
   - Outside the lock:
     - Replay `RetainedReservationSet`.
     - Call `ActingHeadContainment::observe_at_head(&reservations, &context, worktree_id, &target, &repository_trunk, &head)`, with the candidate's selected target as the acting target and the configured repository trunk.
     - Compute `mutual_remainders` against every other observed footprint. For each pair, `MutualWorkRemainder { candidate_paths, holder_paths }` holds what each side would still bring to the other's HEAD: `unmerged_branch_paths(other_head, this_head)` plus its working-tree paths.
     - Build the `CanonicalWorktreeRoot` and the purpose `Enroll existing work on <branch>`.
     - Generate a new `ReservationId` and `CoordinationRunId`.
   - `Ledger::transact(worktree_id, run, validate)` then does all of the following under the lock:
     - Re-checks history on the locked replay. `AlreadyReserved` means a skip, not a failure.
     - Replays with `.with_acting_head_containment(containment)`, which sets `ForeignProtectionPolicy::ExcludeContainedWork`.
     - `enrollment_authorization` runs `conflicts_for_claim(&scopes, worktree_id, path_case)` and narrows each conflict's `overlapping_scopes` to scopes found in both sides of that holder's `MutualWorkRemainder`. A conflict with nothing left is dropped. A holder with no remainder entry (not observed this run) keeps its full overlap. The remaining conflicts become `AuthorizedOverlap::from(&conflict)`, then `ConflictAuthorization::Enrollment { overlaps }`, or `NoConflict` if none remain.
     - Appends `JournalOperation::Claim` with:
       - `source: ClaimSource::Enrolled`
       - `scopes` = the full target-relative footprint (not narrowed)
       - `trunk_at_claim` = the selected target's commit
       - `target` = the selected `ClaimTarget`, with its fallback when automatic selection fell back
       - `head_snapshot` from the same resolution
       - `phase_start_head` = merge base with the target
       - `coordination_identity_provenance: NotPresented`
   - `LedgerTransactionError::CorrectableInput` (the 16 KiB record limit) → `record_too_large`; other transaction errors → `git_failure`.
   - After the append, `WorktreeContext::publish_coordination_run_marker(run)`. A session in that worktree then joins the run through the marker and reuses the reservation without appending anything.
6. **Report.** `report.overlaps = unresolved_enrollment_overlaps(re-read events)`. This walks the journal in order: every `Claim` or `Widen` with an `Enrollment` authorization adds one `EnrollmentOverlap` per bound counterpart, and a `ResolveDefer` whose `(deferred, blocker)` equals `(first, second)` removes it. `sequence` writes the stored deferral's orientation into `ResolveDefer` (`prepare_deferred_edge` uses `deferred_overlap_between(..).endpoints`), so either command order clears the pair.

### Types

| Type | Location | Role |
|---|---|---|
| `WorktreeEnrollmentReport { enrolled, overlaps, failures }` | `worktree/enrollment.rs` | Added to `InitializationPayload.enrollment` (`#[serde(default)]`) |
| `EnrolledWorktree { reservation_id, worktree_root, branch }` | same | One per new reservation |
| `EnrollmentOverlap { first_reservation_id, second_reservation_id, shared_scopes, sequence_commands: [String; 2] }` | same | Commands are `cargo berth sequence <a> <b> --why 'Order enrolled work'` in both orders |
| `WorktreeEnrollmentFailure { worktree_root, reason, diagnostic }` | same | Wire labels come from `WorktreeEnrollmentFailureReason::as_str`: `operation_in_progress`, `no_merge_base`, `git_failure`, `configuration`, `unavailable`, `record_too_large` |
| `ClaimSource::Enrolled` (wire `{"kind":"enrolled"}`) | `ledger/journal.rs` | Permanent marker of an enrolled claim |
| `ConflictAuthorization::Enrollment { overlaps: AuthorizedOverlapSet }` | `answer/conflict_authorization.rs` | Binds each counterpart and the shared scopes recorded at enrollment; has no blocker or reason field |
| `OverlapAuthorizationReason::enrollment()` | `answer/proposal.rs`, text in `answer/constants.rs` (`ENROLLMENT_AUTHORIZATION_REASON`) | Engine-written reason on enrollment deferrals; the wording is not a contract |
| `edge::DeferralOrigin { UserAnswer, Enrollment }` | `edge/mod.rs` | Stored on `DeferredOverlap` and `IntegrationDeferralConstraint.origin` |
| `ForeignProtectionPolicy { FullProtection, ExcludeContainedWork(ActingHeadContainment) }` | `reservation/retention.rs` | Replaces `Option<ActingHeadContainment>` |
| `WorktreeEnrollmentCandidate { Eligible(WorktreeContext), Unavailable { root } }` | `worktree/liveness.rs` | Candidate list from one registry read |

### How enrollment touches existing subsystems

- **Edit coverage.** `ConflictAuthorization::covers` sends `Enrollment` to `AuthorizedOverlap::covers_shared_scope`: the counterpart id matches and the scope is among the recorded scopes, and `scope_revision` is ignored. Every other variant uses `AuthorizedOverlap::covers`, which is revision equality plus `covers_shared_scope`. Edit checks and widening go through the existing `reservations_authorize_scope`.
- **Ordering graph.** `OrderingGraph::apply_authorization` adds one deferral per distinct counterpart through `add_deferral(.., DeferralOrigin::Enrollment)`; `Defer` uses the same helper with `UserAnswer`. The existing `IntegrationHold::DeferredOverlap` holds integration: observe mode reports it, enforce mode rejects it. `sequence <first> <then> --why` resolves the deferral in either order.
- **Board.** Reads `DeferralOrigin` from the constraint projection. `UnresolvedOverlap.origin` is `"user_answer"` or `"enrollment"`. `RecordedAnswer::Enrollment { reservation_id, exact_approved_scopes, acquisition: AnswerAcquisition::Enrollment, consequence }` lists only the pairs not yet resolved. A resolved pair shows up as `OrderingCreatedFromDeferral`, and its deferral reasons are taken from the projection's enrollment deferrals, one per pair in journal order.
- **Automatic run ending** (`reconcile.rs` `append_merged_run_endings`). A reservation with `ClaimSource::Enrolled` counts as having done work even when its merge extent was already observed `Empty`. It therefore ends once its checkout is clean at a head that its target contains. `ClaimSource::Cover` counts the same way.
- **Cover claims** (`cover_claim`). Reconciliation reads an integration branch's checkout through the same `observe_target_footprint`, given the covered branch's own target and commit, and returns a `ClaimSource::Cover` claim with the same field rules, or nothing for an `Empty` footprint.
- **Session mapping** (`session/mod.rs` `apply_journal_event`). A `Claim` with `source: Enrolled` publishes no session mapping, so the session running `init` is not mapped to other worktrees' reservations.
- **Merge observation** (`drift/observation.rs` `observe_merge_working_tree`). Drops a staged, unstaged, or untracked path equal to `BerthConfig::relative_path()`: an uncommitted configuration edit is repository setup, not lane work, and counting a tracked edit kept a merged lane from ending its run. Committed changes to that file still count through `git::unmerged_branch_paths`.
- **Branch paths** (`git/reachability.rs` `unmerged_branch_paths`). Runs `git merge-tree --write-tree --name-only --no-messages -z <base> <head>`, accepting exit 0 (clean) and 1 (conflicted). `<base>` is the selected target's commit for a footprint and the other worktree's HEAD for a mutual remainder. The first NUL field is the result tree; the rest are conflicted paths. The answer is `git diff --name-only -z --no-renames <base> <result tree>` plus those conflicted paths. `merge-tree` builds a virtual base when criss-cross merges leave several merge bases, where `git diff --merge-base` failed with "multiple merge bases found". A modify/delete conflict keeps the base's file in the result tree, so only the conflict list names it. Each read is two git invocations, counted by `UNMERGED_BRANCH_PATH_GIT_QUERIES` in `merge_extent_path_queries`.
- **Containment** (`reservation/containment.rs`). `ActingHeadContainment::observe_at_head` reuses a HEAD the caller already resolved, and `observe` delegates to it; both take the acting target and the repository trunk. For a foreign holder recorded against a different target, committed protection is limited to `unmerged_branch_paths(acting target tip, holder head)` intersected with the acting-HEAD remainder; its dirty paths always count. `MergeExtent` has no `protected_key()`; the `Protected` / `Unavailable { retained_evidence: Protected }` states are matched explicitly.
- **Scope normalization.** `verb/claim.rs` treats `FirstTouch | Enrolled` with `into_exact_file_antichain`.
- **Text output** (`output.rs` `append_enrollment_message`). One line per enrolled worktree, overlap (with both commands) and failure. When all three lists are empty, only the original `init` line prints. `source_description` shows `enrolled worktree changes`.

### Constants

- `worktree/constants.rs`: `ENROLLMENT_SEQUENCE_REASON`, `MERGE_BASE_NO_COMMON_ANCESTOR_EXIT_CODE`, `OPERATION_IN_PROGRESS_MARKERS` (`rebase-merge`, `rebase-apply`, `MERGE_HEAD`, `CHERRY_PICK_HEAD`, `REVERT_HEAD`).
- `answer/constants.rs`: `ENROLLMENT_AUTHORIZATION_REASON`.

## Invariants

- The journal is the source of truth. Whether a worktree is enrolled, and which overlaps are unresolved, are both answered by replaying it, never by stored side state.
- Any `Claim` by a worktree, in any state, rules it out of enrollment for good. Enrollment never appends to a worktree that has history.
- Each candidate gets its own `Ledger::transact`: one lock hold that decides and records. No Git runs inside the lock. The footprint, containment and pairwise checks are all computed before it. The history check is repeated on the locked replay.
- One candidate's failure never stops the others.
- `Enrollment` authorization never takes away edit access, and existing answer variants keep revision-equality coverage.
- An enrolled reservation ends like any other run: checkpoint and release, or automatically once its work reaches its target. Journal history is never removed.
- Wire changes are additive only. Older journals replay unchanged, `tests/fixtures/reader_compat` bytes stay the same, `InitializationPayload.enrollment` stays serde-default, and `docs/cargo-berth/generated/output-contract.json` must match the generator (regenerate with `CARGO_BERTH_REGENERATE_OUTPUT_CONTRACT=1` on `generated_artifacts_are_reproducible_from_the_checked_in_contract`).
- A journal record is at most 16 KiB. An enrolled claim that does not fit is reported as `record_too_large`, never split.
- `AuthorizedOverlap` still requires `scope_revision` on the wire, even though `Enrollment` coverage ignores it.
- Adding a variant to `ClaimSource` or `ConflictAuthorization` means updating its exhaustive match sites, including `verb/claim.rs` and `output.rs`.

## Calibration / gotchas

- **Clean counterparts.** `reservations_authorize_scope` looks at the holder's authorizations only when the requester's foreign protection is `Protected`. A clean counterpart whose merge extent is `Empty` therefore cannot edit the shared path through the other side's `Enrollment` authorization. `Defer` behaves the same way.
- **Stale holder observations.** Containment for existing holders uses their saved merge-extent key. A holder with an old saved observation can hide a new dirty overlap during enrollment, just as it does in the edit check.
- **Config exclusion scope.** Only `observe_merge_working_tree` drops the config file, from every working-tree partition. Ordinary and full drift still attribute the untracked config file, and existing tests depend on that.
- **Unborn HEAD or missing trunk.** `git rev-parse` failures come back as `GitError::CommandFailed`, not as cat-file resolution variants, and are mapped to `no_merge_base`. A `cargoBerthTarget` that does not resolve falls back to the trunk, so only an unresolvable HEAD or trunk reaches this mapping.
- **Run marker publish failure.** If publishing fails after the claim commits, running `init` again does not retry it: the worktree now has history.
- **Ended endpoints.** Unresolved overlaps are replayed from `Enrollment` authorizations minus `ResolveDefer` events, not read from the ordering graph. A pair is reported again after either reservation ends, until `sequence` resolves it.
- **Stacked worktrees.** The pairwise checks cover both candidates and observed worktrees that already have history. A worktree stacked on an in-flight branch forms no pair for the parent commits its HEAD already contains. Claim scopes themselves stay target-relative; only the recorded overlap is narrowed.
- **Locked and prunable worktrees.** Locked worktrees are enrolled. Prunable or undiscoverable registrations are reported as `unavailable`.
- **Clean repository.** Managed hooks install into git's hooks directory and the config file is excluded, so `init` creates no work of its own and enrolls nothing in a clean repository.
- **Config errors.** An invalid linked `berth.toml` makes `init` from that worktree fail before the main file exists. A present main file is accepted without reading the linked file.
- **Test fixtures.** `tests/hooks.rs` `add_worktree` copies the main configuration into the new worktree. Fixtures that need a linked-only or unconfigured worktree use `add_worktree_without_configuration` or remove the file after adding. The shared cargo target directory can serve integration binaries built in another worktree; `touch crates/cargo-berth/tests/*.rs` forces a rebuild.
- **Not automatic.** Enrollment does not happen at later hook contacts. A worktree added after `init` that already has commits needs `init` run again before its first edit.
- **Out of scope** (single user, a few worktrees): unordered overlap dismissal, capacity limits during enrollment, reporting config disagreement, crash recovery partway through `init`, squash merges, stashes, bare repositories, coordination across clones.
- **Ruled out:**
  - Letting a clean active counterpart edit through the other side's `Enrollment` authorization. It is rare, and a contained counterpart ends automatically.
  - Refreshing existing holders' merge extents from fresh footprints before recording overlaps. Enrollment follows the edit check's containment instead.
  - Retrying run-marker publication for already-enrolled worktrees. It would only matter when the marker path is blocked.
  - Extending the config-file exclusion to ordinary or full drift.
  - A separate enrolled-work evidence field in replay. The immutable `ClaimSource::Enrolled` carries that fact.
  - Passing a bare main-root path to `BerthConfig::initialize`. That loses the linked worktree's policy as a source to carry over.

## Why

- **Why `Enrollment` ignores `scope_revision`:** so that when either side widens into unrelated paths, the pair is not blocked again. A newly shared path still gets the ordinary conflict handling.
- **Why `Enrollment` is its own variant and not a `Defer`:** it binds every counterpart at once, with no single blocker and no user-written reason. `DeferralOrigin` lets the board tell engine-made deferrals from user answers without scanning the journal again.
- **Why the config goes at the main worktree root:** every worktree without its own file reads it there. Linked worktrees added later need no copy.
- **Why one transaction per candidate, with a history recheck under the lock:** one failure stays with its worktree, and a worktree that gains a reservation concurrently is skipped instead of claimed twice.
- **Why the run marker is published:** so a session in the enrolled worktree joins that run and reuses the reservation, instead of taking a first-touch claim of its own.
- **Why enrolled claims publish no session mapping:** `init` runs in one session but claims for every worktree. Mapping them would tie the invoking session to other worktrees' reservations.
- **Why `ClaimSource::Enrolled` counts as work for automatic ending:** enrollment requires a non-empty footprint, and the immutable source keeps that fact even if a later drift or gate check has already recorded an empty extent.
- **Why unresolved overlaps come from the journal and not the graph:** a pair has to keep being reported after an endpoint ends, until the user orders it.
- **Why the untracked config file is excluded from merge observation:** `init` itself creates that file. Without the exclusion, a clean repository would enroll its own configuration.
