# cargo-berth

## What it is

`cargo-berth` is a reservation engine for repositories worked on through several git worktrees at once. When more than one worktree edits the same repository, two failures recur: two workers edit the same file and discover it at merge time, and work lands on trunk in an order that breaks a dependency neither worker recorded. `cargo-berth` gives those facts a home. A worktree announces the repo-relative paths it is about to touch, the tool answers whether those paths are free, and if they are it records the claim in the same locked step that decided it. Ordering between overlapping reservations is recorded as edges, integration is gated on the predecessor actually reaching trunk, and both answers are enforced by git hooks rather than by convention. The interface is repo-relative paths and reservation ids; there is no separate board file to keep in sync, because every fact the tool reports is either journalled or recomputed from git.

## How it works

### Storage

All coordination state lives in a `cargo-berth` directory under the repository's common git dir, so every worktree of one repository shares one ledger and a clone gets a fresh one. `ledger/constants.rs` names the layout:

| File | Role |
| --- | --- |
| `journal.ndjson` | Append-only NDJSON record of every mutation. Truth. |
| `reservations.json` | Materialized projection of the journal. Disposable cache. |
| `reservations.json.tmp` | Staging path for the atomic rename that publishes a projection. |
| `mutation.lock` | The single file lock every mutating verb takes. |
| `repo-instance-id` | Identifies the ledger's repository instance. |
| `cargo-berth-worktree-id` | Per-worktree identity file, written in the worktree's git dir. |
| `cargo-berth-run-id` | Coordination-run marker, plus a `retiring` suffix during teardown. |
| `session-identities.json` | Harness session id to reservation id mapping. |

The projection is never authoritative. It carries a generation counter and is rebuilt by replaying the journal whenever it is missing, stale, unparseable, or ahead of the journal. `MINIMUM_SUPPORTED_SCHEMA_VERSION` is 1 and `CURRENT_SCHEMA_VERSION` is 2: new records and projections are written at 2, and records at 1 still decode.

### The transaction surface

`ledger/mod.rs` exposes exactly one way to change state:

```rust
pub(crate) fn transact<Rejection>(
    &self,
    worktree_id: WorktreeId,
    coordination_run_id: CoordinationRunId,
    validate: impl FnOnce(ReplayedLedgerState<'_>) -> TransactionValidation<Rejection>,
) -> Result<LedgerTransactionOutcome<Rejection>, LedgerTransactionError>
```

The sequence inside is fixed: acquire the mutation lock, replay the journal to a `ReplayedLedgerState`, hand that state to the caller's `validate` closure, and — only if the closure returns an accepting `TransactionValidation` — append the record it produced. `transact_with_committed_action` extends the same hold to git side effects that must not be observed apart from the record, so the ref write and the journal append cannot be seen out of order.

`MutationLock::acquire` polls with backoff from `MUTATION_LOCK_INITIAL_RETRY_INTERVAL` (50 ms) to `MUTATION_LOCK_MAXIMUM_RETRY_INTERVAL` (1 s), giving up after `MUTATING_VERB_CONTENTION_TOLERANCE` (10 s) with exit 6. The lock is `std::fs::File::lock`; there is no advisory protocol layered on top and no lock-free append path. `PIPE_BUF` atomicity governs pipes, not regular files, so no record small enough exists to make an unlocked append safe.

Read paths do not take the lock. `Ledger::read_for_edit_check` replays without locking and without invoking git, which is what makes the hot `check` path cheap.

Projection maintenance is `ProjectionSynchronization`, and `ProjectionError::CacheAhead` is the specific case where the cache claims a generation the journal cannot justify — treated as corruption of the cache, never of the journal, and repaired by rebuild.

### The journal operation union

`JournalOperation` carries sixteen variants: `Claim`, `Widen`, `Checkpoint`, `Resnapshot`, `Renew`, `Release`, `ReplaceReleaseDisposition`, `EvidenceRevalidated`, `ResolveDefer`, `Incursion`, `ResolveIncursion`, `ForcedIntegrationPermit`, `ConsumeForcedIntegrationPermit`, `Bypass`, `RebindWorktree`, and `RelocateWorktree`. Every record also carries its actor — worktree id and coordination run id — and a `RecordedAt`. A record may not exceed `MAXIMUM_JOURNAL_RECORD_BYTES` (16 KiB) including its terminating newline; the writer refuses rather than emitting a line a reader could not decode.

`Claim` carries the origin of the reservation as `ClaimSource::{WorkPlan, FirstTouch, Explicit}` — a claim made under a named plan and phase, one minted by first touch, or one a user stated outright. `Widen` carries a reason distinguishing drift-driven widening from an explicit one.

### Scopes and overlap

A reservation scope is a set of repo-relative paths. `scope/` validates them purely lexically — no filesystem probe, so a scope for a file that does not exist yet is legal, which is what claiming a file you are about to create requires. Overlap is computed on path components, not string prefixes, so `src/foo` and `src/foobar` are disjoint while `src/foo` and `src/foo/bar.rs` overlap by ancestry. `PathCase` is derived from git's `core.ignoreCase` so a case-insensitive checkout treats `Src/main.rs` and `src/main.rs` as the same path. Scope sets are reduced to a minimal antichain: a path that an ancestor already covers is dropped rather than stored twice.

### The command surface

`main.rs` is `cli::Cli::parse_arguments().run()` returning an `ExitCode`. `cli.rs` holds the whole clap surface; `verb/` holds `board`, `check`, `claim`, `drift`, `integrate`, `release`, and `sequence`; `recovery.rs` holds `resolve` and `renew`; and `init` is implemented in `cli.rs` itself.

| Verb | What it does |
| --- | --- |
| `init` | Creates the ledger, writes configuration if absent, installs the managed hooks. `InitializationRequest::{Initialize, RepairProjection, ReinitializeAfterReview}` selects between first setup, cache repair, and a deliberate re-run over an existing install. |
| `check` | Asks whether paths are free, and on a clear answer claims them in the same transaction. |
| `claim` | Creates a reservation explicitly, with a stated purpose. |
| `board` | Renders current state as a terminal view, plain text, or `--json`. |
| `drift` | Compares recorded scope against what the worktree actually touched. |
| `sequence` | Records an ordering edge between two overlapping reservations. |
| `integrate` | Gates a merge to trunk on predecessor evidence. |
| `release` | Retires a reservation with a disposition. |
| `resolve` | Acts on an alert — an orphaned reservation, an incursion. |
| `renew` | Refreshes a reservation's activity timestamp. |

There is no `reserve` verb. `claim` is what creates a reservation; `check` is what creates one on first touch.

A hidden `__reference-transaction` subcommand exists solely for the git hook to invoke; it is not part of the user surface.

`CommandExecution` separates `Response` from `BoardTerminalRestored`, so a command whose output is a restored terminal is not mistaken for one that produced a payload. `TOTAL_GATE_DEADLINE` is `Duration::from_secs(10)`.

### The output envelope

Every command returns an `OutputEnvelope` with six frozen fields plus a `payload`. `OutputPayload` is a struct: `#[serde(flatten)] facts: OutputFacts` and `alerts: Vec<Alert>`. `OutputFacts` is tagged `kind`/`data` across thirteen variants — `NoFacts`, `Init`, `ProjectionRepair`, `Reinitialize`, `Board`, `Check`, `Claim`, `Drift`, `Release`, `Sequence`, `Integrate`, `Resolve`, `Renew` — so a consumer switches on `kind` and reads `data` directly. Alerts travel with the facts on every envelope rather than being a payload variant, because an alert is orthogonal to what the command was asked to do.

`OutputStatus` names each terminal state — `Clear`, `Claimed`, `Widened`, `Incursion`, `DriftCollision`, `BlockedByOverlap`, `BlockedByOrdering`, `NeedsUserAuthorization`, `Contention`, `Sequenced`, `OrderingCycle`, `Integrated`, `TrunkRewritten`, `Released`, `Recovered`, `Renewed`, and the rest — so the status is readable without parsing prose.

Exit codes are a `#[repr(u8)]` enum in `exit.rs` with a serde round-trip through `u8`:

```rust
pub(crate) enum BerthExit {
    Clear = 0, BlockedByOverlap = 1, BlockedByOrdering = 2,
    NeedsUserAuthorization = 3, LedgerUnreadable = 4, UsageError = 5,
    BlockedByContention = 6, TerminalViewFailed = 7,
}
```

### Enrollment

`Enrollment<T>` is `Enrolled(T)` or `Unconfigured { expected_configuration_path }`. Absence of configuration is a state the tool reports, not an error it fails on: a repository that has never run `init` gets told where the file would go. Configuration lives at `<root>/.claude/config/berth.toml`, parsed by a hand-rolled subset reader. `BerthConfig` defaults are `trunk = "main"`, `maximum_reservations = 128`, `maximum_ordering_edges = 512`, and `gate_mode = Observe`. `InitializationState::{Created, Existing}` distinguishes a file the run wrote from one it found.

### Identity

Three identities compose. A `WorktreeId` is stable per worktree. A `CoordinationRunId` identifies one run of a harness. A `ReservationId` identifies the claim. `EditAuthorization` resolves who is editing, in a fixed order:

```rust
pub(crate) enum EditAuthorization {
    Session { .. },
    Environment(..),
    Marker { .. },
    Unidentified,
}
```

`resolve_from_sources` consults the harness session mapping first, then `CARGO_BERTH_RUN`, then the on-disk coordination-run marker, and falls back to `Unidentified`. `session/mod.rs` keeps `session-identities.json` current: `apply_journal_event` publishes a mapping on `Claim` and `Widen`, and retires every entry pointing at the reservation on `Checkpoint` and `Release`. Marker removal reports `CoordinationRunMarkerRemoval::{Removed, AlreadyAbsent, PreservedDifferentRun, PreservedMalformed}` — a marker belonging to another run or one that cannot be parsed is left in place rather than deleted.

Worktree identity is persistent, not derived from the environment. Two runs distinguished only by `CARGO_BERTH_RUN` inside the same worktree collapse into one actor once that variable is unset, because the on-disk marker files outlive it. A genuinely foreign holder requires a real `git worktree add`.

### Claiming and the first-touch path

`check` is the common path and is built to be cheap:

```rust
pub(crate) fn execute(check_request: CheckRequest) -> OutputEnvelope

fn decide(invocation_directory: &Path, declared_scopes: DeclaredReservationScopeSet)
    -> Result<Enrollment<CheckDecision>, CheckDecisionError>
```

`decide` runs lock-free and git-free through `read_for_edit_check`. If the paths carry no conflict, `check` calls `acquire_first_touch` with `FirstTouchConflictHandling::RefuseRequest` — a single `transact` that re-replays under the lock, re-validates against the same generation, and appends the `Claim` only if the answer is still clear. The decision and the record are one atomic step; there is no window between reading clear and writing the claim.

If the answer is blocked, `reconcile_and_retry` runs reconciliation once and re-decides. A reconciliation failure is swallowed and the block stands: a failure to prove a reservation dead is never grounds to treat it as dead.

`claim` takes the explicit path, with `conflicts_for_claim`; `check` uses `conflicts_for_edit`, which additionally consults `EditAuthorization` so a run that already holds the path is not blocked by itself. The first-touch path must go through an authorization-filtering conflict query — one that ignores recorded overlap answers will refuse edits the answers already permitted.

### Lifecycle and git evidence

`reservation/lifecycle.rs` keeps four orthogonal types rather than one fused stage enum: `ReservationLifecycle`, `IntegrationEvidenceStatus`, `EditBlockingStatus`, and `ReleaseDisposition`. Neither `lifecycle.rs` nor `evidence.rs` uses `Option` in these types — every state that could be "not known yet" is a named variant instead.

`ReservationLifecycle` has exactly three variants: `Active`, `Outstanding { protected_tip }`, and `Released { disposition }`. The `release` verb on an active reservation produces `Outstanding`, not `Released`.

`reservation/mod.rs` holds `RetainedReservationSet::replay`, the only path by which live reservation state is derived. Every consumer — board, gate, drift, integration — reads the same replay rather than maintaining a parallel view.

Integration evidence is git, not a flag. A reservation's protected tip is pinned by a retention ref at `refs/cargo-berth/reservations/<id>`, so the commit survives branch deletion and `git gc --prune=now`. Evidence questions are ancestry queries against that ref. `git/` wraps `std::process::Command`; there is no git library dependency and no libgit2.

### Answering an overlap

An overlap has a bounded answer set:

```rust
pub(crate) enum ConflictAuthorization {
    NoConflict,
    Sequence { overlaps, blocker, direction, edge_id, reason },
    Defer { .. },
    Override { .. },
    ExistingAnswersCoverEveryOverlap { overlaps },
}
```

All four user-selectable answers — before, after, defer, override — exist so that **both** parties may continue editing the overlapping path. Only the integration order differs between them. An answer never revokes edit access.

Answering is a two-invocation handshake. The first invocation exits 3 with a proposal and an `OverlapProposalToken`. The token is transient — nothing is journalled for it — and the second invocation recomputes the proposal under the lock and matches it against the token. If state moved in between, the token no longer matches and the user is asked again against the new facts. `from_approved_proposal` is the only way an approved proposal becomes an authorization.

### The edge graph

`edge/` stores ordering edges and derives readiness rather than storing it:

```rust
pub(crate) enum EdgeReadiness {
    Holding { hold: EdgeHold },
    Cancelled,
    Fulfilled,
}
```

`holds_successor()` is one structural match over that enum, so there is exactly one place that decides whether an edge blocks. `EdgeHold` carries `UnintegratedPredecessorEvidence` — the concrete git fact that keeps the hold alive. Readiness asked about a reservation that the snapshot does not carry produces `MissingReadinessFact`, which fails closed with exit 4 rather than assuming the edge is clear. Cycles are refused at `sequence` time.

### Liveness and reconciliation

`worktree/` classifies whether a reservation's holder still exists. `reconcile.rs` runs the reconciliation pass that turns a dead holder into an alert. Nothing is auto-removed: reconciliation records what it observed and raises an alert; retiring a reservation always takes a user action.

### Alerts and recovery

`Alert` currently carries `OrphanedOutstanding(OrphanedOutstandingAlert)` — a protected reservation with no validated worktree holder. Alerts travel on every envelope. The board's `BoardAlert` adds the board-only views `StaleReservation` and `UnrecordedBypasses`. An orphan alert carries everything needed to act: `protected_tip`, `BoardBranchRefStatus`, `ObjectAvailability`, `BoardRetentionRefStatus`, `RecoverabilityVerdict`, `OrphanRecoveryConsequence`, and the `OrphanResolutionAction` that would clear it. `recovery.rs` implements `resolve` and `renew` against those verdicts.

### Drift

`drift/` answers whether a worktree edited outside its recorded scope. Two comparison modes exist behind `DriftComparisonChoice`: a cheap fingerprint delta costing two git calls, and a full phase-start comparison costing three plus one committed-diff call per additional reservation. The outcomes are silent (nothing moved), auto-widen (the path is free and joins the scope), incursion (the path belongs to someone else), and collision (two reservations both touched it). `DriftPathAttributionOutcome` names all six results including the wire tags `first_touch_claimed` and `post_write_incursion`. `PostWriteFreePathProtection` covers the case where a write landed on a path nobody had claimed. `DriftEffect` is what the run actually applied.

Drift selects only `Active` reservations. An `Outstanding` reservation is past the point where widening its scope is meaningful.

### The trunk gate

`gate/` enforces on git's `reference-transaction` hook, so any ref update reaching trunk is evaluated regardless of which porcelain command produced it. `GateMode` is `Observe` or `Enforce`; `Observe` records and warns, `Enforce` refuses. `GateDecision` has five variants and every one carries the generation it was decided against. `evaluate_reference_transaction` handles the hook path and `evaluate_integration` the verb path, sharing the readiness derivation.

`gate/install.rs` manages exactly two hooks:

```rust
const MANAGED_HOOKS: &[ManagedHook] = &[REFERENCE_TRANSACTION_HOOK, POST_COMMIT_HOOK];
```

`ManagedHookInstallation` and `ManagedHookActivationOutcome` report what installation did. Each hook body carries a marker comment identifying it as managed, so a subsequent run can recognize its own file and refuse to overwrite a hand-written one. Installed hooks get `EXECUTABLE_PERMISSIONS` (`0o755`). The post-commit hook runs `CARGO_BERTH_POST_COMMIT=1 <executable> drift --full` and always exits 0; if the executable is missing or non-executable it prints a message telling the user to run `cargo-berth drift --full` by hand and states that the commit remains in place.

Forced permits are one-use: `ForcedIntegrationPermit` grants and `ConsumeForcedIntegrationPermit` spends. `Bypass` records a gate bypass. `CARGO_BERTH_BYPASS=1` is evaluated before any ledger read, so an unreadable ledger can still be bypassed — and both managed hooks honor it in their first lines.

### The board

`board/mod.rs` builds a `BoardModel` with sixteen top-level fields, and `board/tui.rs` renders it with ratatui, crossterm, and `tui_pane` across six panes. Three output modes exist: the terminal view, plain text, and `--json` carrying the full model. The model includes a `git_cost` block with six dimensions — trunk resolution calls, worktree list calls, reservation evidence revalidations, protected predecessor ancestry queries, worktree ahead/behind computations, and orphan recovery evidence queries — so the cost of rendering the board is visible in its own output. A terminal that cannot be driven exits 7 rather than reusing a data error code.

### Environment variables

| Variable | Effect |
| --- | --- |
| `CARGO_BERTH_RUN` | Supplies the coordination run id, consulted after the session mapping. |
| `CARGO_BERTH_BYPASS=1` | Skips gate evaluation; read before any ledger access, and honored by both hooks. |
| `CARGO_BERTH_POST_COMMIT=1` | Marks a `drift` run as hook-invoked, selecting warning rendering. |
| `CARGO_BERTH_TEST_MUTATION_LOCK_READY_PATH` | Test-only signal that makes a waiting lock acquisition observable. |

## Invariants

- The journal is truth; `reservations.json` is a cache. Any code that reads the projection must tolerate rebuilding it, and no code may treat a projection value as authoritative over a replay.
- Every mutation goes through `Ledger::transact` or `transact_with_committed_action`. A new verb does not get its own append path.
- Decision and record happen in one lock hold against one generation. Reading a clear answer and appending in a separate step is unsound and is not permitted anywhere.
- The mutation lock is the only anti-interleaving mechanism. No record size makes an unlocked append safe.
- Journal records never exceed `MAXIMUM_JOURNAL_RECORD_BYTES`, counting the newline.
- Records are append-only. Nothing is edited or deleted in place; a correction is a new record — `ReplaceReleaseDisposition` corrects a disposition, it does not rewrite one.
- New journal operations write `CURRENT_SCHEMA_VERSION` and must decode records back to `MINIMUM_SUPPORTED_SCHEMA_VERSION`.
- Overlap is decided on path components, never string prefixes, and always through the `PathCase` derived from `core.ignoreCase`.
- Scope validation stays lexical. No path check may require the file to exist.
- Scope sets stay a minimal antichain.
- All four overlap answers leave both parties able to edit the shared path. Any conflict query on the first-touch path must apply authorization filtering, or recorded answers are silently ignored.
- `RetainedReservationSet::replay` is the only path by which live reservation state is derived.
- `lifecycle.rs` and `evidence.rs` carry no `Option`. A new state is a new variant.
- The four lifecycle types stay orthogonal. Fusing them into one stage enum is not an available simplification.
- Integration evidence is a git ancestry query against the retention ref, never a stored boolean.
- Every reservation with a protected tip has a retention ref, and the ref is written inside the same lock hold as the record that justifies it.
- Edge readiness is derived, never stored. `holds_successor()` remains the single decision point.
- A readiness question about an absent snapshot entry fails closed with exit 4.
- Ordering edges never form a cycle; `sequence` refuses one.
- The overlap answer set is closed. A new answer is a new `ConflictAuthorization` variant, not a flag on an existing one.
- Overlap proposal tokens are never journalled and are always re-derived and matched under the lock.
- Nothing is auto-removed. Reconciliation raises an alert; a user action retires a reservation.
- A reconciliation failure never relaxes a block.
- Editing fails open and integration fails closed when the ledger is unreadable.
- `CARGO_BERTH_BYPASS` is read before any ledger access so a broken ledger can never trap a user.
- A bypass is recorded, never forgiven — `Bypass` records go in the journal and surface on the board.
- Forced permits are single-use and their consumption is journalled.
- Every `GateDecision` carries the generation it was decided against.
- Both managed hooks honor `CARGO_BERTH_BYPASS` in their first lines and carry their marker comment; installation never silently replaces an unmanaged hook.
- The post-commit hook always exits 0. Drift warns, it never blocks a commit.
- Alerts are a field on every envelope, orthogonal to `OutputFacts`.
- Exit codes are stable and each names one condition. Reusing a code for a new condition is not allowed; `TerminalViewFailed` stays distinct from `LedgerUnreadable`.
- The six frozen envelope fields and the `kind`/`data` tagging of `OutputFacts` are wire contract.
- `Enrollment` keeps "not configured" as a reported state, never an error.
- Git is reached only through `git/`, as `std::process::Command`. No git library is introduced.
- The session identity mapping is maintained only by `apply_journal_event`, and only `Claim`, `Widen`, `Checkpoint`, and `Release` move it.
- A coordination-run marker belonging to a different run, or one that cannot be parsed, is preserved rather than removed.

## Calibration and gotchas

| Value | Setting |
| --- | --- |
| `MUTATING_VERB_CONTENTION_TOLERANCE` | 10 s before exit 6 |
| `MUTATION_LOCK_INITIAL_RETRY_INTERVAL` | 50 ms |
| `MUTATION_LOCK_MAXIMUM_RETRY_INTERVAL` | 1 s |
| `MAXIMUM_JOURNAL_RECORD_BYTES` | 16 KiB, newline included |
| `TOTAL_GATE_DEADLINE` | 10 s |
| `STALE_AFTER` | 24 h of owner inactivity before a reservation reads stale |
| `HarnessSessionId::MAXIMUM_CHARACTERS` | 256 |
| `SymbolicReferenceDepth::MAXIMUM` | 32 |
| `EXECUTABLE_PERMISSIONS` | `0o755` |
| `maximum_reservations` | 128 |
| `maximum_ordering_edges` | 512 |
| Cheap drift comparison | 2 git calls |
| Full drift comparison | 3 git calls plus one committed-diff call per additional reservation |
| `DELETE_CONTROL_BYTE` | `0x7f`, rejected in ref names |

- Reservation freshness is computed from owner activity events only. Unrelated journal traffic from other worktrees does not refresh a reservation, so a busy repository does not mask an abandoned claim.
- `check` runs without the lock and without git. Adding a git call to `decide` changes the cost of the most frequent operation in the system.
- `DriftPathAttributionOutcome::{Ambiguous, CoordinationRunRequired}` exist only in the command payload and are never journalled — they describe a question the run could not answer, not a fact about the repository.
- The cheap drift comparison detects that something moved; it cannot attribute which reservation moved it. Attribution needs the full comparison.
- Drift runs after the write, not before. A path is discovered as touched, then classified.
- Case-insensitive checkouts are common on macOS. Any path comparison written without `PathCase` will pass on Linux CI and fail on a developer machine.
- `git gc --prune=now` will drop a reservation's tip if the retention ref is missing, which is why the ref is written under the same lock as the record.
- Branch deletion is expected and normal; the retention ref is what makes evidence survive it.
- A projection that is ahead of the journal (`ProjectionError::CacheAhead`) indicates a stale or foreign cache, not journal damage. The response is rebuild, never truncate.
- A clone starts with no ledger. Ledger state is deliberately not committed to the repository.
- A second worktree needs `.claude/config/berth.toml` present, or the board reports `unconfigured`. That file is not tracked, so `git worktree add` does not bring it along.
- The `reference-transaction` hook fires for every ref update including ones no porcelain command names. Filtering on branch name alone is not sufficient; the trunk name comes from configuration.
- Exit 3 is not a failure. It means the tool needs an answer and has produced a proposal; the caller re-invokes with the token.
- An overlap proposal token that no longer matches is the correct outcome when state moved, not an error to retry through.
- The board's `git_cost` block exists because board rendering is the most git-expensive operation; a change that adds queries shows up there.
- `Alert::recovery_evidence_query_count` returns 4, 3, or 2 depending on branch status, so alert-heavy boards cost proportionally more.
- The board TUI requires a real terminal. Piping it exits 7, distinct from any data problem. Under a sandbox that blocks `openpty`, the terminal-attached tests fail for that reason alone.
- Configuration parsing accepts only the subset of TOML the tool needs. Unusual TOML syntax in `berth.toml` will not be understood.
- The shipped dependency set is `clap`, `crossterm`, `ratatui`, `serde`, `serde_json`, `tui_pane`, `unicode-width`, and `uuid`, with `tempfile` for tests. There is no TOML crate, no time crate, and no error-handling crate.
- Reservation ids are UUID v7, so id ordering is roughly creation ordering — useful for reading a journal, not a substitute for the recorded timestamp.
- The `__reference-transaction` subcommand is hidden and its argument handling is driven by git's hook protocol, not by user ergonomics.
- Workspace-wide builds enable `clap/wrap_help` through an unrelated member's dependency, which rewraps `long_about` text. Help-text assertions must normalize whitespace, and package-scoped verification cannot observe this class of defect.

## Why it is this way

**The ledger lives in the common git dir, not the working tree.** Coordination state is about the repository instance, not about its contents. Putting it in the working tree would make it a merge conflict, put it in diffs, and make a clone inherit another machine's reservations. Under the common git dir it is shared by every worktree of one repository and absent from a fresh clone, which is exactly the scope it needs.

**The journal is append-only and the projection is disposable.** A mutable state file has no recovery story: once it is wrong, nothing can tell you what it should have been. An append-only record can always be replayed, which makes every corruption of the cache repairable and makes the history auditable without a separate audit log. The cost is replay on every mutation, which is bounded by the journal being small and the read-only path never replaying under lock.

**One transaction function, not per-verb append paths.** The correctness argument for this system is short: everything that changes state does so under one lock, after one replay, against one generation. That argument only holds if there is one place to check. A second append path would not just add a bug, it would make the invariant unverifiable.

**Overlap is component ancestry.** String prefix matching makes `src/foo` conflict with `src/foobar`, which is wrong in a way users notice immediately and stop trusting. Component ancestry is the same rule git itself uses for pathspecs.

**Scope validation is lexical.** The most common claim is for a file the worktree is about to create. Requiring the path to exist would make the primary case fail, so path checking never touches the filesystem.

**Claiming happens on first touch.** An earlier design had each worktree declare a reservations block up front. That fails on the thing that actually happens: work discovers files. A declared block is either stale within minutes or maintained by hand at exactly the moment the user has the least attention to spare. First-touch claiming makes the reservation a consequence of the edit, so it cannot drift from what the worktree is doing. The accepted cost is that the claim is created by the same command that answers the question, which makes `check` a mutating verb and forces the acquisition into the transaction.

**Acquisition is inside the check, not after it.** Read-then-write is a race with a real window: two worktrees can both read clear and both append. Making the clear branch acquire inside the same `transact` closes it. This is the reason `check` cannot be a pure read.

**One permission rule on the first-touch path.** The authorization filter and the widened-scope binding answer the same question and must not be duplicated. Two rules on that path drift apart, and when they do the symptom is an edit refused despite a recorded answer that permits it.

**Claiming is mandatory rather than advisory.** An advisory tool is one that reports what already went wrong. Since the enforcement is at the ref level anyway, making the claim mandatory costs nothing extra and turns a report into a guarantee.

**No exception for the root manifest.** Shared files like a workspace manifest were considered for a permanent exemption, on the grounds that everyone touches them. But those are precisely the files where uncoordinated edits collide most expensively. They go through the same rule, and the answer mechanism — sequence, defer, override — is what makes that tolerable.

**Checkpoint is not release.** Reaching a good state and giving up the paths are different events. Fusing them would force a worker to either hold a reservation past its usefulness or lose the recorded tip that later evidence depends on. `Checkpoint` records the tip; `Release` gives up the scope.

**The answer set is closed.** Sequence, defer, override, and the case where existing answers already cover every overlap are the complete set of things a user can mean. Leaving it open-ended would push the decision into free text that nothing downstream can act on. A closed enum means the gate can enforce the answer later without re-interpreting it.

**Edge status is derived.** A stored "ready" flag is a second source of truth about whether a predecessor landed, and git already knows. Deriving readiness from ancestry means the answer cannot go stale, and it means a rewritten trunk is noticed rather than silently trusted.

**Four lifecycle types instead of one stage enum.** A fused enum forces every combination into a named state and produces states that cannot occur alongside states that can. Keeping lifecycle, integration evidence, edit blocking, and release disposition separate means each has only its own legal values, and a new value in one does not multiply against the others. The related choice to ban `Option` in those modules is the same reasoning: `None` names the absence of information without saying which absence it is.

**Exit codes separate conditions that call for different actions.** Blocked by overlap and blocked by ordering call for different responses from a caller, so they are different codes. Needing authorization is not a failure at all. Contention means retry. An unreadable ledger means repair. A terminal that will not render is not a data problem and does not share the data-error code — collapsing them would make a display problem look like corruption.

**Nothing is removed automatically.** The tool can prove a worktree is gone; it cannot prove the work is abandoned. Auto-removing a reservation on that evidence would silently discard someone's claim on the strength of an inference. An alert plus an explicit `resolve` keeps the decision with the person who has the missing context.

**Fail open for editing, closed for integration.** These have asymmetric costs. If the ledger is unreadable and editing is blocked, the tool has bricked the repository for everyone — a coordination aid that stops work is worse than no aid. If the ledger is unreadable and integration proceeds, an unverified merge lands on trunk. So the cheap-to-recover direction fails open and the expensive-to-recover direction fails closed.

**Bypass is evaluated before any read.** The one situation where a user most needs to escape the tool is the one where the tool is broken. Any bypass path that first consults the ledger would fail exactly when it is needed, which is why both the binary and both hook scripts check the variable first.

**A bypass is recorded, not forgiven.** Removing the escape hatch would make the tool something people work around. Making the escape hatch invisible would make the ledger lie. Recording every bypass keeps both properties: the user can always get out, and the board always shows that they did.

**Enforcement is at the ref level.** A pre-commit hook can be skipped, and a verb-level check only covers the verbs. `reference-transaction` fires for every ref update, which means the gate sees merges, resets, and pushes made by tools that never heard of `cargo-berth`.

**The gate ships observing before it enforces.** A gate that starts refusing on day one gets disabled on day one. `Observe` produces the same records and the same warnings without blocking, so a repository can see what enforcement would have done before turning it on.

**Two release valves, not one.** Forced permits handle the case where the gate is right about the facts and wrong about this particular merge; they are one-use and journalled, so the exception does not become the norm. `CARGO_BERTH_BYPASS` handles the case where the tool itself is the problem. Collapsing them would mean either no way out when the tool is broken, or a permanent global off switch for a single exception.

**Drift detects after the write.** Intercepting writes would mean sitting in front of the editor, which is neither possible nor desirable. Comparing what the worktree actually touched against what it claimed catches the same divergence with no interception, and the cheap fingerprint comparison keeps the common case at two git calls so the check can run often.

**The commit-time drift check warns and never blocks.** A post-commit hook that fails leaves the user with a commit already made and a tool telling them it should not exist. Warning after the fact, and always exiting 0, keeps the commit valid and puts the finding where the user can act on it.

**The board is computed, not stored.** A maintained board file is a second copy of facts git and the journal already hold, and it is wrong the moment someone forgets to update it. Recomputing means the board cannot disagree with reality — and publishing the git cost of that recomputation keeps the price of the choice visible.

**One standalone binary.** Building this as a cargo subcommand crate keeps it installable with `cargo install`, invocable as `cargo berth`, and usable from a git hook as a plain executable. A library would have required a host; an editor plugin would have covered one editor and left the hooks unguarded.

**The wire contract is frozen first.** Envelope fields, `kind`/`data` tagging, exit codes, and the journal operation names are consumed by hooks, scripts, and harnesses that upgrade on their own schedule. Fixing them before the behavior settles means later work adds variants rather than renaming fields.
