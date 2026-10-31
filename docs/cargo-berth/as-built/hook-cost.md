# cargo-berth: post-tool-use hook cost

## What it is

`cargo-berth hook post-tool-use` runs the drift check after every Bash call in every session. Its cost grows with the ledger and with the history of each worktree. The design below keeps each step close to linear and does each piece of work once per request. It changes cost only. Every answer stays identical: the same conflicts, the same scopes in the same order, the same JSON. The measured cost is in [Calibration and gotchas](#calibration-and-gotchas).

## Check under load

The pre-edit `cargo-berth check` is described in [check-under-load.md](check-under-load.md).

## How it works

### Keyed scope reduction

`crates/cargo-berth/src/scope/antichain.rs`:

- `pub(super) fn reduce(scopes: ReservationScopeSet, path_case: PathCase) -> ReservationScopeSet` keys each scope once by its path components. It collects every `Tree` key in a `HashSet`. Then it decides each scope in input order by hash lookups:
  - it is dropped when a tree sits on a strict-ancestor prefix of its key;
  - a `File` is dropped when a tree has the same key;
  - it is dropped when an earlier scope has the same key and kind (`seen_files` / `seen_trees`).
  - No pair of scopes is compared. The empty-result fallback `ReservationScopeSet::try_from(reduced).unwrap_or(scopes)` stays.
- `pub(super) fn component_keys(scope, path_case) -> Vec<String>` splits the path on `/` and maps each component through `PathCase::component_key`.
- `overlaps` and `contains` stay pairwise. They answer one pair, and other callers use them.

`crates/cargo-berth/src/scope/mod.rs`:

- `PathCase::component_key(self, &str) -> String` (`pub(super)`) returns the component unchanged under `Sensitive` and `to_lowercase` under `Insensitive`. Two components are `component_eq` exactly when their keys are equal.
- `ReservationScope::component_keys(&self, path_case: PathCase) -> Vec<String>` (`pub(crate)`) is the one way to key a scope outside the module. The reduction and the drift probe both use it.
- `DeclaredReservationScopeSet::into_minimal_antichain`, `into_exact_file_antichain` and `ReservationScopeSet::minimal_antichain` all call `antichain::reduce`.

### Drift coverage probe

`crates/cargo-berth/src/reservation/retention.rs`:

- `struct ProtectedConflictHolder<'r> { representative: &'r Reservation, scopes: ReservationScopeSet }` is one selected holder with its complete protection.
- `fn protected_conflict_holders(&self, path_case, acting_worktree, holder_is_foreign) -> Vec<ProtectedConflictHolder<'_>>` is the selection step:
  - It keeps holders whose `edit_blocking_status()` is `Blocking` and that pass the foreignness filter.
  - It keeps one holder per foreign worktree, because the merge union covers the whole checkout. Same-worktree race holders stay separate.
  - It calls `protection_for_conflict` and keeps only `ConflictProtection::Protected`. Each holder's union is reduced once there.
- `fn conflicts_overlapping_holders(holders, candidate, path_case, observed_at)` is the overlap step. It is a free function. It keys the candidate once, indexes each holder in a `KeyedConflictProtection` and asks `overlapping_scopes`. It compares no pair of scopes.
- `fn conflicts_with_holders(...)` is selection followed by overlap. `conflicts`, `bind_widened_scopes` and `conflicts_for_authorized_edit` call it. They index the holders on each call.
- `#[cfg(test)] fn conflicts_overlapping_holders_by_scan` compares every held scope with every candidate scope. It is the reference for the equivalence tests.
- `struct KeyedConflictProtection<'r>` wraps one `ProtectedConflictHolder`. Its indexes are built once:
  - `file_keys` and `tree_keys` (`HashMap<Vec<String>, Vec<usize>>`) hold exact keys of held files and trees.
  - `descendant_keys` (`BTreeMap<Vec<String>, Vec<usize>>`) holds every held key, for range lookup under a candidate tree.
  - It also caches the holder's `OverlapScopeRevision`.
- `KeyedConflictProtection::overlapping_scopes(candidate, candidate_keys)` finds overlaps three ways:
  - a held tree on the candidate key or on an ancestor prefix;
  - a held file on the exact key, for a candidate file;
  - every held key under the candidate key, for a candidate tree, through `descendant_keys.range(key..)` while `starts_with(key)` holds.
  - Matches are sorted by (held index, candidate index), which is the scan's loop order. Each match reports the candidate scope when the held scope contains it, and the held scope otherwise.
- `pub(crate) struct DriftCoverageProbe<'r> { standing_holders, foreign_holders: Vec<KeyedConflictProtection<'r>>, path_case, observed_at: RecordedAt }`.
- `RetainedReservationSet::drift_coverage_probe(&self, subject_worktree_id: WorktreeId, subject_coordination_run_id: CoordinationRunId, path_case: PathCase) -> DriftCoverageProbe<'_>` builds the probe:
  - It reads `RecordedAt::now()` once.
  - It selects standing holders, which are not foreign to the subject under `is_foreign_to_coordination_run_in_worktree`, and foreign holders, which are.
  - It indexes each holder.
- `DriftCoverageProbe::coverage(&self, candidate: &ReservationScopeSet) -> DriftBlockingCoverage` keys the candidate once. It returns `NoForeignStanding` if any standing holder overlaps. Otherwise it builds one `ReservationConflict` per overlapping foreign holder and returns `Foreign(conflicts)`, or `Unclaimed` when there are none.
- `ReservationConflict::for_overlap(holder, overlap_scope_revision, overlapping_scopes, observed_at)` in `reservation/conflict.rs` (`pub(super)`) is the one constructor. `conflicts_overlapping_holders` and `coverage` both use it, and both pass `overlapping_scopes.minimal_antichain(path_case)`.
- `blocking_coverage_for_drift` is `#[cfg(test)]` only. It builds a probe and asks it once.

`crates/cargo-berth/src/drift/classification.rs`:

- `subject_coverage_probe(reservations, subject, path_case)` calls `drift_coverage_probe(subject.actor().worktree, subject.actor().run, path_case)`.
- `PreLockForeignPathClassification::build` and `classify_locked` each build one probe per subject reservation, before `changes.visit_paths`. Every path of that subject is asked against that probe.
- The method `PreLockForeignPathClassification::blocking_coverage(&self, probe, subject, path)` and the free `blocking_coverage(probe, written_at: Option<CommitterTime>, path)` both take the probe. The free function keeps its `written_at` filter, which drops holders whose claim follows the commit that wrote the path.

`crates/cargo-berth/src/reservation/record.rs` holds `ConflictProtection::{Clear, Protected { representative, contributors, scopes }}`, `Reservation::protection_in_worktree` and `Reservation::edit_blocking_status`, which the selection step reads.

### First-parent intervals from one walk

`crates/cargo-berth/src/git/reachability.rs`, on `CommitAncestryGraph`:

- `fn first_parent_intervals_after<'c>(&self, target: &GitObjectId, candidates: impl Iterator<Item = &'c GitObjectId>) -> HashMap<GitObjectId, Vec<GitObjectId>>` walks the target's first-parent chain once. It seeds `boundaries` with each chain commit's index. For each distinct candidate it stores `chain[..boundary]`.
- `fn first_parent_ancestry_boundary(&self, candidate, chain_length, boundaries: &mut HashMap<GitObjectId, usize>) -> usize` is an iterative post-order search with `BoundaryVisit::{ParentsPending, ParentsVisited}`. A commit's boundary is the minimum of its parents' boundaries, or `chain_length` when it has none. Results are memoized across candidates, so shared ancestry is visited once. No candidate builds a full ancestor set.
- `fn classify_candidates_against_target(target, target_history, candidate_resolutions)` passes every resolved candidate that is in the graph to `first_parent_intervals_after`.
- `fn first_parent_commits_after(&self, tip, excluded_ancestor) -> Vec<GitObjectId>` reads `ancestors_including(excluded_ancestor)` and walks first parents from the tip. `earliest_containing_matches` and `descendant_commits` use it.

### One ancestor set per tip, one repository open per worker

`crates/cargo-berth/src/git/reachability.rs`, on `CommitAncestryGraph { ancestors_by_tip: Mutex<HashMap<GitObjectId, Arc<HashSet<GitObjectId>>>>, parents_by_commit }`:

- `fn ancestors_including(&self, tip) -> Arc<HashSet<GitObjectId>>` returns the tip's ancestor set from `ancestors_by_tip`. It calls `compute_ancestors` only for a tip it has not seen. Each distinct tip therefore costs one ancestor set for the life of the graph.
- One graph belongs to one request. Reachability (`descendant_commits`: is the predecessor in the successor's ancestor set?), first-parent exclusion (`first_parent_commits_after`) and merge contributions (`target_merge_contributions`) all read the same sets.

`crates/cargo-berth/src/git/read.rs`:

- `CommitRepositoryReader<'root> { root, repository: OnceCell<Result<RepositoryRead<Repository>, Arc<GitError>>> }` opens the gix repository on first use and keeps it. `commit(expression)` resolves exact object ids, full refs and `HEAD` through it. Other expressions return `GitRequired(RevisionExpression)` and open nothing.
- Each worker makes its own reader and drops it with its batch: `history::read` makes one per history read, and `OperationObjectReader::resolve` (`git/object.rs`) makes one per batch of candidates. A batch of N candidates opens the repository once, not N times.

`crates/cargo-berth/src/git/work.rs`:

- Thread-local `ReadWorkCounts { repository_opens, ancestor_sets }`. `read::open` calls `record_repository_open`, and `compute_ancestors` calls `record_ancestor_set`. Each worker owns its counts. No decision reads them.
- `request_batches_open_each_worker_once_and_compute_each_distinct_ancestry_once` (`reachability.rs`) asserts one open per worker and one ancestor set per distinct tip.

### Hook passes skip released and foreign work

- `reconcile.rs`: holder observation and `holder_group` leave out reservations whose lifecycle is `Released`. A released reservation costs no merge extent and no sibling share.
- `reconcile.rs`, `ReservationEvidenceRead`: on `CompletedBashCall` the pass reads `LiveConstraints { invoking_worktree }`. It reads git evidence for a released reservation only when the reservation is in the invoking worktree or has a nonterminal dependent. A direct invocation and a session start read `AllRetained`.
- `reconcile.rs`, `reservation_merge_extent`: `scope::ScopeContainmentIndex` indexes the reservation's scopes and its siblings' scopes by first path component and exact file path. Each observed path then tests only the trees under its own first component.
- `board/rows.rs`: `AheadBehindWorktrees::for_occasion` counts ahead/behind for every worktree on a direct invocation (`EngineAnswerOccasion::DirectInvocation`). On `CompletedBashCall` and `OpeningSession` it counts only the invoking worktree. `hook_counts_history_only_for_the_invoking_worktree` checks it.
- `scope/mod.rs`, `PathCase::component_eq`: an ASCII pair compares with `eq_ignore_ascii_case` and allocates nothing. A pair with `Σ` keeps the full `to_lowercase` compare for its final-sigma rule. Other pairs compare lowercase chars one by one.

### Incursion membership by id set

`crates/cargo-berth/src/reservation/retention.rs`:

- `incursion_incidents_of_coordination_identity(&self, reservation: &Reservation) -> impl Iterator<Item = &IncursionIncident>` collects one `HashSet<ReservationId>` of the reservations with that worktree and run, in one pass. It then filters `incursion_incidents` by membership. Retained order is kept. An incident charged to an unknown id is excluded. Its callers are in `recovery.rs` and `board/alerts.rs`.
- `observe_incursion(&self, entering, blocked_paths) -> IncursionObservation` collects the identity's incidents once into a `Vec<&IncursionIncident>`. It borrows blocked paths and clones them only into the returned `BlockedIncursionPathSet`.
- `fn incursion_path_coverage(incidents: &[&IncursionIncident], blocked: &BlockedIncursionPath) -> IncursionPathCoverage` is an associated fn. It keeps the holder-containment rule. The answer is `Outstanding(id)` at the first outstanding match, else `Answered` if any resolved incident matched, else `Uncovered`.

### Test fixtures

`crates/cargo-berth-test-support`:

- `RepositoryTemplate { target_directory, executable, name, build: fn(&Path) }` (`repository_template.rs`) builds a repository once per build of a test binary, under `CARGO_TARGET_TMPDIR/cargo-berth-templates`. `instantiate()` copies it into each test. The copy is relocated to the new root, and its repository and worktree identities are renewed.
- `IntegrationRepository::from_template(template: RepositoryTemplate) -> Self` (`integration_repository.rs`) prepares the integration pair once per template through `PreparedIntegrationPair`: the `integration` checkout and its first hooked commit. Each copy renews both checkout identities.
- `DirectorySnapshot` (`directory_snapshot.rs`) holds a directory tree in memory. `capture`, `replace_text` and `restore` let a fixture built once start several cases with no processes.

`crates/cargo-berth/tests`:

- `board.rs`:
  - The board matrices restore a captured fixture per case from `DirectorySnapshot`.
  - Setup commits whose hooks the test does not pin use `GIT.run_without_hooks`.
- `overlap.rs`: `INTEGRATION_MAIN` (name `integration-main`, built by `IntegrationRepository::build_main_template`) is derived from the file's `INITIALIZED_REPOSITORY` and used through `IntegrationRepository::from_template`.
- `tests/support/reanchoring.rs` restores the reanchoring cases from snapshots.
- `compaction.rs`: `INITIALIZED_REPOSITORY` (name `compaction-main`) wraps `build_initialized_repository`.
- `liveness.rs` uses a hookless setup commit.
- `edges.rs` runs `prune --expire=now` after `reflog expire` instead of `gc --prune=now`.
- `gate.rs`: `GATE_DEADLINE` is `Duration::from_millis(200)`. It is passed to `cargo-berth` through `CARGO_BERTH_TEST_GATE_DEADLINE_MS` (`GATE_DEADLINE_ENVIRONMENT`). Production `TOTAL_GATE_DEADLINE` in `src/cli.rs` stays 10 s. The environment value can only shorten it (`ledger::shortened_by_environment`).

## Invariants

- Every answer is unchanged: the same conflicts, the same holders in the same order, the same `overlapping_scopes`, the same scope order, the same JSON.
- Keyed `reduce` equals the pairwise rule scope for scope and in order. The pairwise rule drops a candidate when another scope contains it and either the candidate does not contain that scope, or that scope comes first. So:
  - a scope under a strict-ancestor `Tree` goes;
  - a `File` sharing a path with any `Tree` goes;
  - a repeat of an earlier (path, kind) goes;
  - a `File` never removes a `Tree`.
  - Path comparison is per component under `PathCase`.
- The reference is test-only `reduce_by_pairs` in `antichain.rs`. `reduction_matches_the_pairwise_reference_on_every_short_ordering` and `…_on_long_mixed_lists` check it under both `PathCase` values.
- Probe coverage equals the test-only scan `conflicts_overlapping_holders_by_scan` on the same holders. `keyed_drift_coverage_matches_the_scope_scan` checks it under both `PathCase` values, with case-only differences and same-path tree/file pairs.
- Each first-parent interval equals `first_parent_commits_after(target, candidate)`, including when the walk reaches a root. `batched_first_parent_intervals_match_individual_walks` checks it.
- Incident membership equals a linear scan in order, including the unknown-id case. `incursion_identity_lookup_matches_the_reservation_scan` checks it.
- One probe per subject per pass. The pre-lock and locked passes each build their own.
- One ancestor set per distinct tip per request graph, and one repository open per worker. Reachability and first-parent exclusion read the same set.
- No state lives across requests. A request's graph owns its ancestor sets, and each worker's reader owns its repository. Both drop when the request ends.
- `ReadWorkCounts` only observes. No Git decision reads it.
- No test takes a second.
- Some paths keep the scan:
  - `contains` and `overlaps` stay pairwise;
  - `first_parent_commits_after` stays for `earliest_containing_matches` and `descendant_commits`.

## Calibration and gotchas

- Never share a probe between the pre-lock and locked passes. They read different reservation sets: the snapshot, and the replay under the lock. A shared probe answers from a stale ledger.
- Journal replay rejects an incident charged to an unknown reservation. The unknown-id equivalence case therefore pushes the incident into the retained vector directly.
- `RepositoryTemplate` refuses a non-empty journal. A template may hold only setup that writes no ledger records. Hooked steps run after `instantiate`.
- Ruled out: a template cache for the whole `RewriteFixture`. Its hooked setup writes ledger records.
- Ruled out: separate work on allocation outside the incursion path.

Work-count tests (they count work, not wall-clock time):

| Test | File | Shape | Bound |
| --- | --- | --- | --- |
| `reduction_builds_each_scope_key_once_without_pairwise_comparisons` | `scope/antichain.rs` | 4,732 scopes, 364 distinct, `Insensitive` | 4,732 scope keys, 0 scope comparisons |
| `keyed_drift_queries_reuse_thirteen_holders_scope_keys` | `reservation/retention.rs` | 13 foreign holders, 4,732 held scopes, 200 candidates | at most 1,500 scope keys, 0 scope comparisons |
| `first_touch_overlap_indexes_unrelated_scope_paths` | `reservation/retention.rs` | 128 held files, 128 candidates under other roots | at most 256 scope comparisons |
| `batched_first_parent_intervals_share_ancestry_reads_across_candidates` | `git/reachability.rs` | 6,000 commits, 60 candidates | at most 6,000 ancestry parent reads |
| `incursion_observation_scans_reservations_once_for_hundreds_of_incidents` | `reservation/retention.rs` | 600 reservations, 600 incidents, 64 paths | 600 reservation reads |
| `request_batches_open_each_worker_once_and_compute_each_distinct_ancestry_once` | `git/reachability.rs` | 5 candidates over 3 distinct tips, 3 predecessor groups | 1 open per worker, 3 ancestor sets |
| `hook_counts_history_only_for_the_invoking_worktree` | `board/rows.rs` | 2 worktrees with reservations | ahead/behind for 1 worktree |

The cost-bound assertions `predecessor_graph_has_fixed_cold_cost` and `cold_proof_subjects_bound_git_evaluation_…` are kept.

Current hook cost: the complete post-Bash hook, release build, on the real cargo-liner ledger, medians of 5 interleaved runs on a loaded natedev:

- 133 ms wall.
- 140 ms CPU: 88 ms user and 51 ms system.
- 3 git children: `config`, `rev-list` and `update-ref`.

## Why

- The reduction is keyed because containment between scopes is a prefix relation on component keys. A hash lookup of each ancestor prefix answers it in time linear in total path depth. A pairwise reduction is quadratic in scope count, and one worktree's union of 13 reservations can be 4,732 scopes.
- `component_key` lowercases each component, so key equality matches `component_eq` exactly. Without that, the keyed and pairwise answers would differ under `Insensitive`.
- The probe exists because a holder's protection does not depend on the candidate path. Selection, `protection_for_conflict`, union reduction and indexing all happen once per subject. Only the overlap lookup runs per path.
- The probe is keyed by subject, not by invocation. Foreignness is relative to the subject's run and worktree. One probe for the invoking run would change which holders block every other subject.
- Selection and overlap are split so the scan and the probe share selection and `ReservationConflict::for_overlap`. Only the overlap search differs between them, and the equivalence test compares exactly that.
- Callers outside drift use the same keyed overlap search, but they index the holders on each call. They ask once per command, so a shared probe would save nothing there.
- First-parent intervals share one chain walk and one memoized boundary map. Candidates in one classification share most of their ancestry, so a per-candidate walk that builds a full ancestor set repeats that work.
- The ancestor sets live in the request's graph because one request asks for the same tips again: reachability, first-parent exclusion and merge contributions all start from them. The repository reader belongs to the worker for the same reason: each candidate needs the same repository. On the real ledger, a new ancestor set for each use and a new open for each candidate cost about 120 ms more CPU per hook call (260 ms against 140 ms) and 168 ms more wall time.
- Nothing lives across requests. A resident process that keeps the ledger and its indexes between requests would save about 5 ms CPU per request on the real ledger, which does not pay for a resident process.
- Incursion membership uses an id set. Finding each incident's reservation by a linear search over all reservations costs reservations × incidents.
- Test fixtures come from templates and snapshots. Most of each slow test's time was repeated git and `cargo-berth init` setup that writes no ledger records. Hooked steps still run per test, so every hook a test pins still runs.
- The test gate deadline is short because the tests that wait on it only need it to expire. The production deadline is unchanged.
