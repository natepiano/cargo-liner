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

The projection is never authoritative, and it carries only what is read back from it: schema version, repository instance id, generation, journal end offset, and journal fingerprint. It holds no copy of the events, so the cost of publishing one is set by live replay state rather than by how many records the journal has ever carried.

The two schemas version independently. The journal's `CURRENT_SCHEMA_VERSION` is 2 with `MINIMUM_SUPPORTED_SCHEMA_VERSION` 1 — new records are written at 2 and records at 1 still decode — while the projection owns `CURRENT_PROJECTION_SCHEMA_VERSION`, at 3. `read_once` reads a small `ProjectionSchemaHeader` and validates its version before decoding the rest, mirroring the journal's own header-first read. `read_validated` answers `ProjectionSynchronization::RebuildRequired` for a missing file, an older schema version, and a version too new for this binary to decode, so an unreadable cache is discarded and rebuilt rather than failing the command. Malformed bytes, a repository-identity mismatch, a cache ahead of the journal, and a fingerprint mismatch stay fatal, because none of them establishes that the file is a readable cache for *this* repository.

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

`JournalOperation` carries twenty variants: `Claim`, `Widen`, `Checkpoint`, `Resnapshot`, `Renew`, `Release`, `ReplaceReleaseDisposition`, `EvidenceRevalidated`, `ResolveDefer`, `Incursion`, `ResolveIncursion`, `ForcedIntegrationPermit`, `ConsumeForcedIntegrationPermit`, `Bypass`, `RebindWorktree`, `RelocateWorktree`, and the four that carry integration proof across a restart — `ScopedPatchEquivalenceChecked`, `ScopedPatchComparisonAttempted`, `SuccessorScopedPatchEquivalenceChecked`, and `SuccessorScopedPatchComparisonAttempted`. Every record also carries its actor — worktree id and coordination run id — and a `RecordedAt`.

Every record also carries `identity_inputs`: the process inputs available when its actor was resolved — the invocation directory plus `CARGO_BERTH_SESSION_ID`, `CARGO_BERTH_RUN`, `GIT_DIR`, and `GIT_COMMON_DIR`. Each is a tagged state rather than a bare string (the directory as `utf8`/`too_long`/`non_utf8`/`unavailable`, each environment value as `unset`/`utf8`/`too_long`/`non_utf8`), each is bounded at `MAXIMUM_RECORDED_IDENTITY_INPUT_VALUE_BYTES` (256 JSON-content bytes) with `too_long` retaining only `observed_bytes`, and the field is additive: records written before it omit it. These bytes are journal evidence, not replay state. A record may not exceed `MAXIMUM_JOURNAL_RECORD_BYTES` (16 KiB) including its terminating newline; the writer refuses rather than emitting a line a reader could not decode.

`Claim` carries the origin of the reservation as `ClaimSource::{WorkPlan, FirstTouch, Explicit}` — a claim made under a named plan and phase, one created by first touch, or one a user stated outright. `Widen` carries a reason distinguishing drift-driven widening from an explicit one.

### Scopes and overlap

A reservation scope is a set of repo-relative paths. `scope/` validates them purely lexically — no filesystem probe, so a scope for a file that does not exist yet is legal, which is what claiming a file you are about to create requires. Overlap is computed on path components, not string prefixes, so `src/foo` and `src/foobar` are disjoint while `src/foo` and `src/foo/bar.rs` overlap by ancestry. `PathCase` is derived from git's `core.ignoreCase` so a case-insensitive checkout treats `Src/main.rs` and `src/main.rs` as the same path. Scope sets are reduced to a minimal antichain: a path that an ancestor already covers is dropped rather than stored twice.

### The command surface

`main.rs` is `cli::Cli::parse_arguments().run()` returning an `ExitCode`. `cli.rs` holds the whole clap surface; `verb/` holds `board`, `check`, `claim`, `drift`, `integrate`, `release`, and `sequence`; `hook/` holds the three harness hook events; `recovery.rs` holds `resolve` and `renew`; and `init` and `identity` are implemented in `cli.rs` itself.

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
| `hook` | Answers one harness hook event — `pre-tool-use`, `post-tool-use`, `session-start` — from a raw payload on standard input. |
| `identity` | Manages the current process's disposable coordination identity; `clear-session` is its one subcommand. |

There is no `reserve` verb. `claim` is what creates a reservation; `check` is what creates one on first touch.

Two hidden subcommands exist solely for git to invoke — `__reference-transaction` and `__refresh-managed-hook-after-trunk-deletion` — and are not part of the user surface. `cli.rs` carries a route table over all sixteen entries, and a unit test that reads no standard input asserts each entry's real output ownership.

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

### The harness hook surface

`cargo-berth hook` is a public verb with three subcommands, one per harness event: `pre-tool-use`, `post-tool-use`, and `session-start`. Each reads one raw JSON payload on standard input and writes the response object that event's protocol defines, so the engine — not a front end — decides every byte a user reads.

`hook/mod.rs` holds only what all three share. `hook/process_binding.rs` makes the two decisions every event makes before any repository work starts: `HookWorkingDirectorySelection` chooses the directory whose repository owns the answer, and `HarnessSessionIdentityAvailability` (`Available | Unusable`) decides whether the process can select a disposable harness-session mapping. `hook/context_notice.rs` owns the single stdout object every event publishes when work continues; the three differ only in whether stating continuation means anything for them, which is the one thing a caller chooses. Each event's module carries only the response fields and decisions belonging to its own event.

Dispatch keeps the hook's write out of the dispatch that selected it. `Command::execute` returns `CommandOutputOwnership::HookOwnsItsResponse(HookCommand)` with nothing written, and `Cli::run` calls `HookCommand::write_response()` one frame later, so a caller that only needs to know which hook answers an invocation never reads standard input. `CommandResultReporting` has three answers: `Envelope` for the verbs that produce one, `HookProtocol(HookCommand)` for these three, and `GitHookProtocol` for the two git-invoked private commands, which return before any envelope exists.

Payload parts are domain types with named absent cases, never a bare `Option<T>`; optionals live only inside the private serde boundary structs. A payload whose `cwd` or `session_id` is present but not a string is invalid rather than coerced to an empty string — an empty `cwd` silently observes a different repository, and an empty `session_id` silently attributes drift to another session's reservation. An absent or invalid payload session id publishes a no-session selection that blocks the ambient `CARGO_BERTH_SESSION_ID` fallback, in all three events.

`EngineAnswerOccasion` — `DirectInvocation`, `CompletedBashCall`, `OpeningSession` — is recorded once per process by the hook that owns it, and the first record stands. The verb decides which words a condition gets; the occasion decides which event those words name, because `check`, `board`, and `drift` are each shared between a hook and a person running the verb by hand.

**`hook pre-tool-use`** is the only blocking event. It answers the protocol directly: nothing on a silent allow, an allow-notice object on stdout when the presentation carries blocks, the refusal detail on stderr with exit 2. Edit-target resolution is a two-type split — `PayloadEditTarget { Named | NotNamed { reason } }` resolving into `ResolvedEditTarget { WithinRepository | OutsideCoordinationDomain | Unresolved { reason } }` — so `execute()` carries no impossible arm. The path arrives exactly as the payload named it, `..` components included, because only the filesystem can say what a `..` means: `alias/../held.rs` is `held.rs` when `alias` is a real directory and something else entirely when `alias` is a symlink. `CoordinationDomain` keeps the repository root in both the filesystem's canonical namespace and the payload's. Canonical placement runs first, so a file reached through a symlink keeps the single coordination identity it has always had; only when canonicalization lands outside the repository does the payload-namespace comparison run, because a symlinked directory inside the worktree pointing elsewhere is still an edit inside this worktree. `WorktreeRelativeEditName` rebuilds the worktree-relative name from `Component::Normal` alone, so a surviving `..` answers `NamesNoWorktreeFile` and the hook refuses visibly rather than coordinating a name no write ever reaches.

**`hook post-tool-use`** reports on a Bash call that has already completed, so nothing it says can block and every route ends at exit 0. It performs the drift comparison and, when the answer depends on it, reads the live incursion board in the same process — the harness is never asked to run a second command to finish an answer. `PostToolUseAnswer` is `Silent` or `Stated { summary, detail }`; `LiveIncursionState` is `Read` or `Unverifiable`; `PostToolUseObservableToolCall` separates a Bash call from a payload this verb was invoked on by mistake. Invalid payload and unavailable working directory fail open with distinct messages under `continue: true`.

**`hook session-start`** is advisory: it starts no work and blocks none, so every route ends at exit 0 and the only question it answers is what the reader is told. It reads `BoardModel::envelope_presentation` and branches on `RenderedBlocks` / `NothingToShow` / `NotProvided`, so an unconfigured repository and an unreadable ledger are distinguishable without classifying envelope facts. Its response omits the continuation field, because a session-start response cannot stop anything and the harness continues by default.

`EnvelopePresentation` is what makes silence a state rather than an emptiness check. Three variants — `NotProvided`, `NothingToShow`, and `RenderedBlocks { blocks: NonEmptyRenderedBlocks }` — where the rendered-blocks payload has a private field and a fallible constructor, so an empty rendered-blocks payload is unconstructible. `NothingToShow` serializes as the frozen `{"kind":"rendered_blocks","blocks":[]}` and deserializes back. An alert reaches a user as a rendered block on the presentation a hook publishes; nothing downstream re-derives an alert from wire facts.

`HookFacingCondition` names the conditions a hook-facing verb states in its own words rather than leaving a hook to classify a wire status: `Unconfigured`, `OutsideCoordinationDomain`, `LedgerUnreadable`, `Contention`, and `InvalidInput`. The first two render nothing. A directory under no git worktree is not a ledger the tool opened and failed to read; it is a place where there is no ledger to read and no repair anyone can perform, so `LedgerError::RepositoryNotFound` selects `LedgerReadFailureAudience::DirectCallerOnly` — the hook stays quiet, and a person who ran the verb and asked a question still reads the sentence on an unchanged envelope.

### The installed front end

The installed front end is three shell wrappers — `berth_pre_edit.sh`, `berth_post_bash.sh`, and `berth_session_start.sh` — and each decides exactly one thing: whether `cargo-berth` is on `PATH`. If it is, the wrapper `exec`s `cargo-berth hook <event>`, so stdout, stderr, and exit status pass through byte for byte and the response is identical to invoking the engine directly. The one policy each wrapper states alone is its binary-absent failure mode: pre-edit refuses with exit 2 and a stderr notice; the other two state the problem and exit 0, since neither can refuse what it reports on. Both of those notices are static JSON written with `printf` rather than composed through `jq`, so they hold when nothing else on the path does.

`tests/fixtures/front_end_corpus.json` records what the three installed hooks printed for real engine responses, and `tests/front_end_corpus.rs` is an independent oracle over it: nothing there regenerates the fixture, relaxes a comparison, or drops an entry because nothing drives it any more. Every entry is either driven by a named test or carries the measured reason it cannot be, asserted as a partition rather than a count, and `MINIMUM_FROZEN_CORPUS_ENTRIES` (50) is a ratchet so a deletion cannot balance the partition back to green.

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

`ledger::resolve_identity(&WorktreeContext)` is the single entry point for actor identity. It returns a `ResolvedJournalMutationActor` carrying the worktree id, the coordination run id, and the `EditAuthorization` resolved in that same read, and every journal-mutating path routes through it — claim, check, release, sequence, gate, permit, recovery, reconcile, and drift. Identity is resolved exactly once per invocation, because a second read can disagree with the first when a concurrent release retires the session mapping and marker in between. `WorktreeContext` distinguishes its two directories by type: `WorktreeAdministrativeDirectory` owns the worktree and run identity markers, `SharedLedgerDirectory` owns the journal and session mappings.

Worktree identity is persistent, not derived from the environment. Every identified `EditAuthorization` variant therefore carries the worktree id alongside the run, created on first use by `create_or_read_worktree_id` so an invocation that precedes any claim still resolves it.

The worktree, not the coordination run, is the coordination unit, and **one coordination run occupies a worktree at a time**. Two runs inside one worktree share one filesystem, one index, and one branch, so they cannot produce the merge collision a reservation exists to prevent — which is why edit blocking does not compare the run on its own. What they can produce is two runs editing one checkout, and that is refused by the occupancy rule below rather than by overlap.

Foreignness is one predicate, `Reservation::is_foreign_to_coordination_run_in_worktree`, and `AuthorizedEditingIdentity::is_foreign`, `conflicts_for_drift`, `blocking_coverage_for_drift`, and `bind_widened_scopes` all read it. A holder in another worktree is foreign. A holder in the acting worktree is foreign only while it *occupies* that worktree, which `Reservation::occupies_worktree_for_another_coordination_run` decides on three terms, all three required:

- **another run** — a holder belonging to the acting run is never foreign to it.
- **`Active`** — an `Outstanding` holder has released and is only awaiting integration, so a later session in the same checkout must be free to edit the paths its predecessor left behind. A run term with no lifecycle term made a worktree block itself, and that defect is the reason the run was dropped from this comparison once.
- **`CoordinationIdentityProvenance::Presented`** — the refusal holds only between two runs that **both** presented a coordination identity. A holder claimed under an identity the engine created for itself, recorded `NotPresented`, never occupies: post-commit drift first-touches that way, and counting it would lock a checkout out against its own `--run`. `Unknown` predates the record and declines for the same reason, so upgrading a repository never arrives as a lockout. The two variants stay distinct: not knowing is not the same as knowing nobody presented one.

`RetainedReservationSet::active_reservation_held_by_another_run(worktree_id, coordination_run_id)` asks that predicate over live state and answers with the incumbent — `Option<&Reservation>`, not a boolean — because the refusal has to name the reservation id, the run, and the worktree a reader acts on, and a yes-or-no answer would send the caller back for facts the search already held. The provenance term is durable rather than derived at read time: the `Claim` record carries `coordination_identity_provenance`, written with `#[serde(default)]`, so records that predate the field decode as `Unknown`.

The occupancy question is asked before overlap, in one place — `coordination_identity::validate_worktree_occupancy` — and reached from `ClaimRunValidation::validate` (the `claim` path), `DriftRunValidation::authorize_scope_acquisition` (post-commit drift, which is that path's mirror of `ClaimRunValidation`), and `check::validate_edit_worktree_occupancy` (the pre-edit hook). Asking it before overlap is what makes the refusal repairable: left to the overlap pass, a second run was refused with the generic overlap answer, whose stated remedy is to record one answer for the named holder, and the `claim --override` that would record it is refused by this same rule.

Only the *holder's* provenance is read inside the predicate. The acting side of the same rule is decided by each of those three call sites instead: every one matches on the identity source first and answers without asking when the caller presented none, so only `EditAuthorization::Environment` reaches the question. A caller presenting nothing is never refused, and that narrowness is deliberate: the engine's own markerless post-commit first touch reaches this same validation, so refusing an unpresented identity would refuse the engine itself. The reach of the rule is set one step earlier still, by the resolution order — `resolve_from_sources` consults a live harness session mapping before `CARGO_BERTH_RUN`, so a caller inside a mapped session cannot present a second run and is never asked the occupancy question at all. The rule is therefore symmetric in effect while its two terms live apart, which matters before a fourth occupancy call site is added — one that omits the guard applies the rule to one side only, and the overlap chain never consults the acting side at all --- neither `AuthorizedEditingIdentity::is_foreign`, which `conflicts_for_edit` and `conflicts_for_first_touch` reach it through, nor `conflicts_for_drift`, `blocking_coverage_for_drift` and `bind_widened_scopes`, which read `is_foreign_to_coordination_run_in_worktree` directly; every hop carries a bare `CoordinationRunId`. Putting both terms in the predicate means carrying the acting side's provenance through `RetainedReservationSet` and `AuthorizedEditingIdentity`, and no caller supplies it today.

`conflicts_for_claim` and `identifies_requester` deliberately still compare the worktree alone: the first is the explicit-claim overlap query that the occupancy check already precedes, and the second decides which recorded overlap answers apply, which bind the checkout they were recorded in rather than the run that recorded them.

A genuinely foreign holder in the merge-collision sense still requires a real `git worktree add`, which is what the integration tests build through their `foreign_worktree` fixture.

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

`Released` is terminal. `resnapshot` accepts only `Outstanding`, `apply_widen` accepts only `Active`, and `apply_resnapshot` returns early for a released reservation so a legacy release-then-resnapshot journal replays to `Released` without reopening.

`edit_blocking_status` is computed, never stored. `Reservation::edit_blocking_status()` is a `const` projection of lifecycle — `Active` blocks, `Outstanding` defers to its integration evidence, `Released` is `Clear` unconditionally — and the blocking filter runs before either identity predicate, so a clear holder is dropped before foreignness is consulted. The v1 `edit_blocking_status` journal field is retained for audit and is not authoritative on replay: a journalled `Released` + `Blocking` contradiction replays to an effective `Clear`.

`reservation/mod.rs` holds `RetainedReservationSet::replay`, the only path by which live reservation state is derived. Every consumer — board, gate, drift, integration — reads the same replay rather than maintaining a parallel view.

Integration evidence is git, not a flag. A reservation's protected tip is pinned by a retention ref at `refs/cargo-berth/reservations/<id>`, so the commit survives branch deletion and `git gc --prune=now`. `git/` wraps `std::process::Command`; there is no git library dependency and no libgit2.

### Integration proof

Ancestry is the fast answer, not the only one. When a reservation's protected tip stops being an ancestor of trunk, the change the reservation made *inside its own scopes* — measured from `phase_start_head` — is compared against current trunk history. An amended, rebased, or squashed commit whose scoped content survives still certifies; the same paths carrying different content do not. `IntegrationEvidenceStatus::Integrated { trunk_oid, proof }` carries which of the two proved it, as `IntegrationProof::{ProtectedTipAncestor, ScopedPatchEquivalent}`; records written before the field decode as `ProtectedTipAncestor`.

The ancestry-success path issues no extra subprocess. The fallback batches every scope into one comparison composed of merge-base, rev-list, tree/index, merge-tree, and diff — roughly a dozen git invocations, run once per retained reservation during reconciliation. Every one routes through the typed `GitCommandExecution` boundary, so a git that could not start stays distinct from a git that ran and answered no: merge-base exit 1 (unrelated histories) and merge-tree exit 1 (conflict) both resolve to a definitive `Different`, never `Unavailable`.

The proof is cached against the pair that produced it. `IntegrationProofSubjectRevision` versions the baseline, protected content, and scopes a proof was checked under, and advances on `Widen`, `Resnapshot`, and release-disposition replacement — never on ordinary revalidation. `ScopedPatchEquivalenceCache` retains definitive verdicts (`Integrated`, `NotIntegrated`, `TrunkRewritten`) for the two most recent targets; an `ObjectUnknown` comparison is never cached, because it is a transient environment fact and storing it would make one failed subprocess durable across restarts. The cache and its schedule reach the journal as `ScopedPatchEquivalenceChecked` and `ScopedPatchComparisonAttempted`, so both survive a restart from replay alone.

Reconciliation admits **one** cold scoped comparison per trunk target per pass. Targets that lose the slot are scheduled round-robin over a bounded attempt history, so a skipped subject is preferred next pass and a subject whose comparison keeps returning `ObjectUnknown` cannot starve the others. A deferral is not neutral: `DeferredScopedPatchIntegrationStatus` decides what the materialized evidence still proves — `StillValid` only for an equivalence proof bound to the trunk actually observed, and `Degraded` to `NotIntegrated` both for a protected-tip proof reachability has just refuted and for an equivalence proof bound to an earlier target. Degradation is durable: the `EvidenceRevalidated` append precedes the schedule update, so the correct answer is the one that replays.

Successor edges use the same proof on a different axis. `SuccessorIncorporationEvidence` — `ProtectedTipAncestor`, `ScopedPatchEquivalent`, `NotIncorporated`, `ObjectUnknown` — and the per-predecessor `PredecessorSuccessorIncorporation` replaced a type named for containment that had grown a value that was not containment. `Edge::readiness` reads only the snapshot; the git work assembles every predecessor's subject into one `descendant_commits` call. Successor verdicts have their own cache, keyed by the predecessor's proof-subject revision and the successor head, with its own retention limit (512, against 2 for trunk targets) because twenty heads are twenty targets. A deferred head keeps reporting `AwaitingSuccessorIncorporation`: a deferral never reads as incorporation.

Git cost is bounded by batching, and the standard is exact argv equality at one subject and at twenty rather than a sublinear trend. Predecessor ancestry, worktree ahead/behind, retention-ref availability, and retention-ref repair are each one invocation regardless of count, and incursion attribution is three batched queries — one union-base resolution over usable phase-start anchors, one path log over every entered path, one range-membership query over every anchor — in place of a per-path loop. `IncursionAttributionAnchorState` (`UsableAncestor` / `NotAncestorOfHead` / `ObjectUnknown`) records each anchor's relation to the target, so an unreadable anchor reports nothing rather than defaulting into a false `Unchanged`.

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

`Alert` carries two variants. `OrphanedOutstanding` is a protected reservation with no validated worktree holder. `LostIntegrationEvidence` is a released reservation that no longer has affirmative integration evidence; it carries the reservation id, protected tip, evidence status, and a recovery split into `VerifyResolvedTrunk { trunk_oid, action }` and `ResolveTrunkFirst { action }`, so the unresolved-trunk case is representable without emitting an `--integrated-as <trunk-oid>` instruction nobody could run. It is derived on every reconciliation from replayed state and the already-materialized trunk, so the *first* drift envelope detecting a rewrite reports it, and the derivation is pure — it adds no git subprocess and no per-reservation cost. Alerts travel on every envelope. The board's `BoardAlert` adds the board-only views `StaleReservation` and `UnrecordedBypasses`. An orphan alert carries everything needed to act: `protected_tip`, `BoardBranchRefStatus`, `ObjectAvailability`, `BoardRetentionRefStatus`, `RecoverabilityVerdict`, `OrphanRecoveryConsequence`, and the `OrphanResolutionAction` that would clear it. `recovery.rs` implements `resolve` and `renew` against those verdicts.

### Coordination identity and recovery commands

One `validate_coordination_identity` serves the git gate and every ordinary verb. It returns a `CoordinationIdentityRejection` — stale session mapping, stale marker run, session/worktree mismatch, or worktree held by another run — each carrying a non-empty `CoordinationIdentityRecoveryActions`, so the human message and the machine payload render from one source. The fourth kind is raised by `validate_worktree_occupancy` rather than by `validate_coordination_identity` itself, because the occupancy question is asked about the *incumbent* while the other three are asked about the caller's own identity source.

**A published recovery action must be able to perform the repair it names.** `worktree_held_by_another_run` offers `ReleaseIncumbentReservation`, `cargo-berth release <incumbent-id> --json` run in the occupied worktree: `release` checkpoints an `Active` reservation into `Outstanding`, and occupancy is an `Active`-only rule, so the action ends exactly the state the refusal named. It first offered `ReconcileAndSweepMarker` by analogy with `StaleMarkerRun`, and the analogy does not transfer — the marker sweep deliberately preserves every marker whose run still holds an active reservation, which is the state being refused, so running the action and retrying reproduced the same refusal verbatim. When the incumbent is still working the remedy is a separate checkout, which the message states and no `argv` can perform.

**The occupancy refusal claims acquisition alone.** Its message once closed on a blanket statement that no reservation or edit decision changed. That holds on the pre-edit path, which refuses before deciding anything, and is false on post-commit drift, which observes and classifies first and refuses only the acquisition step — so the same invocation can report an incursion beside the refusal. One `Display` serves both call sites and neither can tell it which one it is rendering for, so it states only what holds on both: no reservation was taken and none was widened, and whatever else the invocation reported it observed and recorded regardless. The parts that survived unchanged are the ones that repair the situation — the incumbent's reservation id, the `release` action, and the separate-checkout remedy for an incumbent still working.

Every published recovery `argv` is a `RunnableRecoveryCommandLine`, produced only through a fallible conversion from the lossless `RecoveryCommandLine(Vec<OsString>)`. A command that cannot be represented as text is omitted from the action set rather than published in degraded form; `RerunFromHoldingWorktree` is the only omittable action and `ClaimSeparatelyHere` always remains, so the set is never empty and every member is directly executable. A recovery command is built from `std::env::args_os()`, so whether it is representable depends on how the process was invoked rather than on anything the ledger holds. A front end renders these actions; it does not parse `message`.

The `reference-transaction` hook exports `CARGO_BERTH_REFERENCE_TRANSACTION_ISSUING_DIRECTORY=$PWD` before it changes directory to the policy worktree, and the binary reads it into `ReferenceTransactionIssuingDirectory::{CapturedByManagedHook, MissingFromLegacyHook}`. There is no fallback to the process's own working directory. A hook installed before that export yields `MissingFromLegacyHook`, and the gate returns `GateError::LegacyReferenceTransactionHook` before resolving any worktree; the refusal carries `OutputStatus::LegacyHookOutdated` and names both repairs — rerun `cargo-berth init`, or set `CARGO_BERTH_BYPASS=1` to proceed now.

A resolve reports what it accomplished rather than that it ran: `recorded_now` when the invocation appended the disposition, `already_recorded_by_same_coordination_actor` (also exit 0) when the retained actor's worktree and run ids equal the caller's, and `already_recorded_by_different_coordination_actor` (exit 5, `invalid_input`) naming the resolving worktree, run, event id, and time in typed fields. `JournalActor::has_coordination_identity` is the single comparison: responsibility means equality of the ids the journal recorded, never sameness of process. `IncursionIncidentStatus::Resolved` retains its resolving actor, reconstructed on replay from the record's own actor, so earlier records replay unchanged and no journal lookup exists.

### Drift

`drift/` answers whether a worktree edited outside its recorded scope. Two comparison modes exist behind `DriftComparisonChoice`: a cheap fingerprint delta costing two git calls, and a full phase-start comparison whose committed history is one batched read however many reservations report. The outcomes are silent (nothing moved), auto-widen (the path is free and joins the scope), incursion (the path belongs to someone else), and collision (two reservations both touched it). `DriftPathAttributionOutcome` names all six results including the wire tags `first_touch_claimed` and `post_write_incursion`. `PostWriteFreePathProtection` covers the case where a write landed on a path nobody had claimed. `DriftEffect` is what the run actually applied.

Drift selects only `Active` reservations. An `Outstanding` reservation is past the point where widening its scope is meaningful.

**Acquisition needs a path that carries work; classification does not.** Widening adds a scope, so `ObservedDriftChanges::carries_work` gates it: a cheap comparison reports a symmetric difference, which names a path restored to its committed content alongside one that was edited, and only the fingerprint about to be cached separates them. Incursion and collision classification keep the unfiltered set, where "what moved since the last observation" is the right question. Every component of a full comparison is a positive statement about the present, so none of them can name a restored path.

**An incursion names the commits behind its entered paths.** `DriftEffect::Incursion` carries an `IncursionCommit` per commit in the reservation's phase range that introduced an entered path, each with its subject, the paths it introduced, and an `IncursionCommitOrigin` of `phase_authored`, `already_on_trunk`, or `unknown`. Only paths from the committed component are looked up, so a working-tree incursion carries none: there the write that caused it is the one the reader just made. Without this the report is not wrong about the paths, it is silent about why they moved, and a path that arrived on a commit taken from trunk reads exactly like a path the worktree wrote.

**Incursion coverage is decided one path at a time, and two suppressions answer different questions.** `RetainedReservationSet::observe_incursion` asks, per path, whether a retained incident of the *same* reservation already names that path against holders it already names: an outstanding one reports under that incident and creates nothing, a resolved one stays answered, and only genuinely new paths create an incident, naming those paths alone. Matching on whole-set equality instead created a second incident every time a straying edit grew by one path, each re-covering the previous one's paths and each needing its own disposition. `outstanding_incursion_covers` in `drift/classification.rs` answers the other question — whether a *sibling* reservation reported in the same run already stands accused of this overlap — and drops the effect outright, which is why it excludes the reporting reservation itself. Its `ClaimSource::FirstTouch` condition is deliberate rather than an oversight: two explicitly claimed reservations are separately accountable for the same entered path, which `markerless_post_commit_reports_every_incursion_without_ambiguous_widens` pins.

**The occupancy refusal withholds acquisition and nothing else.** Post-commit drift asks the occupancy question from inside the ledger transaction, at the acquisition step, in `DriftRunValidation::authorize_scope_acquisition`. Both halves of that placement matter. Asking it earlier aborts the invocation before `observation::observe` runs, so a second presented run's commits would leave no incursion record against any foreign holder anywhere in the repository. Asking it only outside the lock lets a run that raced the incumbent's claim pass the question and then acquire under it. Observation and classification therefore run for a refused run exactly as they do for an unrefused one, and the answer withholds only the widening and the post-write first touch.

**The refusal travels on the report rather than replacing it.** `DriftScopeAcquisition` is `Permitted` or `RefusedToSecondRun { rejection }`, and it is a field of `DriftReport`. A refusal that displaced the report would turn an incursion the caller must act on into a rejection that reads as "nothing happened"; one envelope states both. `DriftWideningAuthorization` is the projection of that answer onto the single thing it withholds from classification. Expressing the refusal by rewriting `ResolvedDriftSubjects::widening` instead reached further than intended: `drift/classification.rs` reads that same field as its blocker filter, so the rewrite also changed which holder a subject could be told it entered.

**A refused run's write into the incumbent's own scopes is reported, never journalled.** The cross-worktree case needs nothing extra — `subjects.reporting` names every active reservation in the worktree, the incumbent is among them, and its entry into a holder elsewhere is reported and journalled as any other incursion is. The same-worktree case is not reachable that way: the incumbent is the only subject and a reservation never enters its own scopes, while the refused run holds no reservation and so can never be a subject at all, because `JournalOperation::Incursion` carries a mandatory subject `reservation_id`. `classification::attribute_refused_run_entry` answers it on the report. It gathers the observed paths that carry work, runs them through `conflicts_for_first_touch` under the *refused* run's own identity, and on any holder replaces `DriftReport::path_attribution` with `DriftPathAttributionOutcome::IncursionDetected { paths, conflicts, protection: PostWriteFreePathProtection::NotAcquired }`. Reported and not journalled is what any writer holding no reservation already gets, which `post_write_drift_detects_but_cannot_prevent_a_foreign_incursion` pins. Without this the refused run's most direct offence — writing straight into the paths the incumbent reserved — was the one thing the engine said nothing about.

**One committed-history read answers both parties' questions.** `git::phase_committed_path_diffs` writes a single `diff-tree --stdin` input holding one pair line per readable phase start and a lone line for the observation target. Git prefixes each record with the first object of the line that produced it, so the pair records key to their anchors and the lone record keys to the target itself, and one invocation answers both what every reporting reservation's phase range carries and what this invocation's own commit introduced. `--cc` and `--root` are what keep that second record readable when the commit is a merge or the repository's first. An anchor already standing at the target is dropped from the pair lines rather than compared with itself, and the caller supplies that empty range from the target's own record — the subtlest step in the read.

**What a refused run is told about its own commit stops at that commit.** `ObservedDriftChanges::acting_run_attributable_paths` is the observed working-tree paths plus `HEAD`'s own commit, and nothing from any phase range. A range runs from its subject's phase start, so the incumbent's earlier commits sit inside it the moment a second run commits onto the same branch. A commit range is not an authorship record: attributing one to whoever ran next reports the incumbent's own writes back to the newcomer as an incursion the newcomer committed, and that accusation carries a blocking exit code, which makes it a wrong decision rather than a wrong sentence.

**A refusal is presented as a refusal, not as a widening and not as an aborted check.** Two renderings read `DriftScopeAcquisition`, and each once told the reader something a refusal contradicts. In the envelope's `presentation`, a refused acquisition is a `RenderedOutputBlock` of its own, summarised "cargo-berth refused this run's scope acquisition in a worktree another run occupies." and published ahead of any widening block rather than appended to one --- front ends render `presentation` and never parse `message`, so a refusal filed under "cargo-berth widened this worktree reservation footprint." told the reader a footprint grew on the invocation that refused to grow it. On the post-commit hook, a refusal carrying no drift effect lands in `OutputStatus::InvalidInput`, the status that also carries a request which never ran, so `post_commit_rendering` asks the payload which of the two it holds. A refused run completed its check: the stderr says so, leaves the commit in place, and offers no by-hand `cargo-berth drift --full`, an invocation this same rule would refuse the same way. A refusal that does carry a drift effect renders under that effect's status exactly as before, the rejection already being the tail of `message`.

**An outstanding-incursion notice says how large its backlog is.** Each `OutstandingIncursion` carries `outstanding_count`, the number of incidents standing for that straying reservation, and a `resolution.every_flag` naming `resolve <id> --every-incursion`, which answers all of them in one appended disposition. A notice naming one incident reads as though answering it ends the matter; resolving is per-incident, so a backlog accumulated before dedup landed would otherwise stay invisible and permanent. With a single outstanding incident the wording is what it was.

**A rewritten branch re-anchors its phases.** `<phase_start_head>..HEAD` means "what this phase changed" only while HEAD's history still contains `phase_start_head`, and a rebase makes that false. The `reference-transaction` gate detects the rewrite by asking whether the previous tip is an ancestor of the proposed one, and emits `Resnapshot { Active }` for each reservation claimed on that branch. `git::rewritten_phase_anchor` finds the replacement by position: the phase's commits survive a rebase as patch-equivalents contiguous at the tip, so the anchor is the commit beneath the last of them. Counting them is not enough and neither is patch identity alone — a rebase drops a commit whose patch already reached the new base, and the upstream commit carrying that patch is itself an equivalent, so both read a dropped commit exactly like a replayed one. Drift stands aside entirely while `rebase-merge` or `rebase-apply` exists, because git runs `post-commit` for every replayed commit and nothing re-anchors until the branch reference moves at the end.

### The trunk gate

`gate/` enforces on git's `reference-transaction` hook, so any ref update reaching trunk is evaluated regardless of which porcelain command produced it. `GateMode` is `Observe` or `Enforce`; `Observe` records and warns, `Enforce` refuses. `GateDecision` has five variants and every one carries the generation it was decided against. `evaluate_reference_transaction` handles the hook path and `evaluate_integration` the verb path, sharing the readiness derivation.

`gate/install.rs` manages exactly two hooks:

```rust
const MANAGED_HOOKS: &[ManagedHook] = &[REFERENCE_TRANSACTION_HOOK, POST_COMMIT_HOOK];
```

`ManagedHookInstallation` and `ManagedHookActivationOutcome` report what installation did. Each hook body carries a marker comment identifying it as managed, so a subsequent run can recognize its own file and refuse to overwrite a hand-written one. Installed hooks get `EXECUTABLE_PERMISSIONS` (`0o755`). The post-commit hook runs `CARGO_BERTH_POST_COMMIT=1 <executable> drift --full` and always exits 0; if the executable is missing or non-executable it prints a message telling the user to run `cargo-berth drift --full` by hand and states that the commit remains in place.

The rendered `reference-transaction` hook classifies phase/ref pairs in shell and spawns the binary only for actionable ones: `preparing`, `aborted`, and unknown phases exit before the binary; `prepared` invokes only when the transaction names the configured trunk ref exactly, as a complete third field rather than a substring; `committed` invokes for any local `refs/heads/*`, because it reanchors phase starts after a local rewrite and consumes forced-integration permits. The same filter gates the bypass recording. Two classifier stages run per fire at a cost independent of ref count: `LC_ALL=C grep -q` routes straight to the binary on any byte outside tab and printable ASCII — grep's own error exit counts as a bad byte — and then one `awk` pass classifies the surviving records. The byte scan must precede awk, because awk truncates a record at NUL. Stdin is copied to a protected temporary file and the *unchanged bytes* are redirected into the binary; a buffering failure refuses and prints a retry instruction rather than replaying a partial transaction. Anything unclassifiable invokes the binary: skipping is not a failure mode this table produces.

Trunk-rename refresh keys on the deletion alone, since `git branch -m` emits only the delete. Candidate branches are those sharing the deleted trunk's tip, and a candidate is admitted only when its newest reflog subject matches `Branch: renamed {deleted} to {candidate}` exactly; `LocalBranchRenameProof` short-circuits to `MultipleMatches` at the second proof, and zero or several proofs leave the hook untouched. The rewrite runs in the hidden `__refresh-managed-hook-after-trunk-deletion` subcommand, spawned detached because it cannot run inside the hook that triggered it, and `PendingManagedHookReplacement` keeps the swap atomic so a failed write leaves the previous hook rather than an empty permissive one.

Berth's own retention-ref writes do not pay for any of that. `GitHookExecutionPolicy` is `Enabled` by default and only the private retention-ref writes in `git/refs.rs` name `SuppressedForRetentionRef`, so `init` hook discovery and `integrate`'s trunk update still fire hooks. Suppression sets `core.hooksPath=/dev/null` through the `GIT_CONFIG_COUNT`/`GIT_CONFIG_KEY_n`/`GIT_CONFIG_VALUE_n` environment overlay rather than a `-c` argv flag, appending after whatever overlay the environment already carries, so it is invisible in recorded argv and every before/after trace stays comparable. Repair and deletion are one call issuing a single `update-ref --stdin` transaction per pass; there is no per-reservation deletion helper left to reintroduce the scaling.

Forced permits are one-use: `ForcedIntegrationPermit` grants and `ConsumeForcedIntegrationPermit` spends. `Bypass` records a gate bypass. `CARGO_BERTH_BYPASS=1` is evaluated before any ledger read, so an unreadable ledger can still be bypassed — and both managed hooks honor it in their first lines.

### The board

`board/mod.rs` builds a `BoardModel` with sixteen top-level fields, and `board/tui.rs` renders it with ratatui, crossterm, and `tui_pane` across six panes. Three output modes exist: the terminal view, plain text, and `--json` carrying the full model. The model includes a `git_cost` block with six dimensions — trunk resolution calls, worktree list calls, reservation evidence revalidations, protected predecessor ancestry queries, worktree ahead/behind computations, and orphan recovery evidence queries — so the cost of rendering the board is visible in its own output. A terminal that cannot be driven exits 7 rather than reusing a data error code.

`board --reservation <id> --json` is a placement-independent lifecycle read for one reservation, covering rows the board deliberately omits — a waiting successor, either endpoint of an unresolved overlap. The selector requires `--json` and is rejected at the command line otherwise, so it never reaches the TUI path. `ReservationLifecycleSnapshot` is projected from `Reservation::evidence_state` rather than re-matched: `Active`, `Outstanding { protected_tip }`, `ReleasedAfterCheckpoint { protected_tip, disposition }`, `ReleasedWithoutCheckpoint { disposition }`. An unknown id rejects as `ReservationLifecycleQueryRejection::UnknownReservation`, never as an absent value.

### The generated output contract

`output_contract.rs` generates the wire contract in-crate rather than from a build script, and `docs/cargo-berth/generated/output-contract.json` is the checked artifact. One test rewrites it on request; the ordinary run regenerates it in memory and byte-compares. Four declaration macros pair each variant with its pinned wire name in one list and generate both a test-visible inventory and an exhaustive match, so a status, verb, exit code, journal operation, or trunk observation cannot be added without appearing in the contract.

The contract's unit is the whole outcome tuple — verb, envelope status, exit code, payload kind, nested discriminants — because every value is individually legal and only the tuple rejects a success envelope carrying a rejection sub-status. Retained legacy outcomes are tuples marked `decodable_only` rather than omitted, which is how `reblocked_active_constraint` stays accepted as a reserved board value while never being emitted. Every `schemars` definition name is pinned to its wire name, so no Rust rename can move the generated bytes — a property a test proves by declaring a genuinely distinct type carrying the same wire name.

`ReplayFailurePayload { reason, subject, effect }` types every replay hard stop: `reason` is generated exhaustively from the replay error enums, `subject` is a three-arm identity union, and `effect` is `HardStop`. Without it the roughly twenty replay invariant failures collapse into one untyped `ledger_unreadable` envelope. `docs/cargo-berth/json-contract.md` is the document independent consumers read.

### Semantic types at the boundaries

Types name their semantic role rather than their representation, and no domain-state `Option<T>` survives at a boundary that carried one: a state that could be "not known yet" is a named variant instead.

`WorktreeComparability::{Comparable(WorktreeId), IdentityNotRecorded, DeferredPendingRewrite}` replaced a `Result<Option<WorktreeId>>`, and the middle variant is named for absence because unavailability is what the code it replaced *claimed* while swallowing every identity-read failure. `EnvironmentCoordinationRunSelection::{NotSupplied, UnusableFallbackToMarker, Identified}` is internal to `EditAuthorization::resolve_from_sources` and converted before its single authorization read, preserving the one-read guarantee. `OverlapSelection` converts all six clap optionals in one place, called once with `?`, so no downstream helper receives a raw optional and the "choose only one overlap answer" arm stays reachable. `GitCommandOutputAvailability::{Available(Output), Unavailable(io::Error)}` carries the error and its exact diagnostic through the conversion. `FilesystemReferenceResolution::{Resolved, RequiresGitResolution { rejection_if_git_reports_missing }}` has the producer pick the fallback error, so the reader matches two arms and never inspects a payload — and no wire status, payload member, or diagnostic names the fallback.

Drift's stand-aside is narrow by construction. `comparable_worktree` stands aside for exactly one case — the identity file genuinely absent, under `DriftReservationSelection::EveryActiveForPostCommit` — and propagates every other ledger error as a `DriftExecutionError`, so a malformed worktree identity fails loudly instead of reading as no drift.

`HarnessSessionId` is a type rather than a validated string: 1 to 256 **characters** — not bytes, so a 256-character multibyte id is valid — with no control characters. It travels as itself to every consumer, and none re-validates it.

### Environment variables

| Variable | Effect |
| --- | --- |
| `CARGO_BERTH_RUN` | Supplies the coordination run id, consulted after the session mapping. |
| `CARGO_BERTH_BYPASS=1` | Skips gate evaluation; read before any ledger access, and honored by both hooks. |
| `CARGO_BERTH_POST_COMMIT=1` | Marks a `drift` run as hook-invoked, selecting warning rendering. |
| `CARGO_BERTH_SESSION_ID` | Supplies the harness session id, consulted only when a hook payload named none. |
| `CARGO_BERTH_REFERENCE_TRANSACTION_ISSUING_DIRECTORY` | Exported by the managed `reference-transaction` hook before it changes directory; the gate reads the issuing checkout from it and has no fallback. |
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
- Integration evidence is derived from git, never a stored boolean: ancestry against the retention ref, or scoped patch equivalence when ancestry fails.
- A widen resets an outstanding reservation's integration evidence to `NotIntegrated`, so a proof never extends to scope it did not check.
- Only definitive scoped-patch verdicts are cached, keyed by the trunk or successor target together with the proof-subject revision. An `ObjectUnknown` comparison is transient and is never made durable.
- A deferred comparison never reads as incorporation, and a degraded verdict is journalled before the schedule update so the correct answer is the one that replays.
- Git cost per pass does not scale with paths, commits, reservations, or successor heads. The standard is exact argv equality at one subject and at twenty, measured on an unfiltered trace.
- `edit_blocking_status` is computed from lifecycle and evidence, never stored. `Released` is terminal and reports `Clear` unconditionally, so no gate may key repair eligibility on it.
- An `Outstanding` holder never blocks another run in the same worktree, and the blocking filter runs before either identity predicate, so a `Released` holder never reaches a foreignness question.
- A worktree admits one coordination run at a time, and the refusal holds only between two runs that both presented an identity: same worktree, another run, `Active`, and `Presented` on the holder, with the acting side guarded at the call site. Every site that asks the occupancy question guards that acting side itself, so a new one added without the guard applies the rule to one side only.
- `identifies_requester` and `conflicts_for_claim` compare the worktree alone, and `has_other_active_reservation` compares the run alone across every worktree. A run term on the first strands recorded overlap answers in the checkout that recorded them; a worktree term on the second changes the question from whether the run still owns live work anywhere to what one checkout may edit.
- Identity is resolved exactly once per invocation, through `resolve_identity`, and every journal record carries the bounded `identity_inputs` observed at that resolution.
- Every published recovery `argv` is directly executable. A command that cannot be represented as text is omitted from the action set rather than degraded, and the set is never empty.
- The gate reads the issuing checkout from the variable the managed hook exports, never from the process's working directory. A hook that does not export it is refused, not assumed away.
- Responsibility for a resolution is equality of the ids the journal recorded, never sameness of process.
- The projection versions independently of the journal. A cache that is unreadable but identifiably this repository's rebuilds; one whose identity or fingerprint does not match is fatal.
- Retention-ref writes suppress hooks through the environment config overlay, never through argv, so every before-and-after argv trace stays comparable.
- The `reference-transaction` dispatch table fails toward invoking the binary. Anything it cannot classify is treated as actionable.
- Every reservation with a protected tip has a retention ref, and the ref is written inside the same lock hold as the record that justifies it.
- Edge readiness is derived, never stored. `holds_successor()` remains the single decision point.
- A readiness question about an absent snapshot entry fails closed with exit 4.
- Ordering edges never form a cycle; `sequence` refuses one.
- The overlap answer set is closed. A new answer is a new `ConflictAuthorization` variant, not a flag on an existing one.
- Overlap proposal tokens are never journalled and are always re-derived and matched under the lock.
- A post-write first-touch claim acquires only paths modified at the time of the claim. Drift reporting still classifies every path that moved.
- Nothing is auto-removed. Reconciliation raises an alert; a user action retires a reservation. A live holder proven clean is no exception; the block message names the verbs that clear it.
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
- The engine writes every byte of a harness hook response. A front end may decide whether the engine can be reached; it may not rebuild, reformat, or supplement what the engine said.
- Every hook process binds its repository and harness session from the payload before any repository work starts. An absent or invalid payload session id publishes a no-session selection rather than falling through to `CARGO_BERTH_SESSION_ID`.
- `pre-tool-use` is the only hook event that can refuse. `post-tool-use` and `session-start` always exit 0, whatever they report.
- A payload-named edit path is resolved through the filesystem. Collapsing `..` textually on a path a payload named as an edit target is not permitted.
- An empty rendered-blocks presentation is unconstructible. Deliberate silence is `NothingToShow`.
- A condition a hook's reader cannot act on renders nothing. `Unconfigured` and `OutsideCoordinationDomain` are both silent to a hook while a direct caller still reads the message, and the wire fields are the same either way.
- No domain-state `Option<T>` at a boundary. A state that could be absent is a named variant, named for what it means rather than for what a failure claimed.
- The frozen front-end corpus is never shrunk to satisfy a coverage assertion, and its coverage is an asserted partition, never a count.

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
| Full drift comparison | 3 git calls, batched: one `diff-tree` over every phase start and the target, one ancestry walk, one working-tree status |
| `DELETE_CONTROL_BYTE` | `0x7f`, rejected in ref names |
| `SCOPED_PATCH_TARGET_RETENTION_LIMIT` | 2 trunk targets |
| `SUCCESSOR_SCOPED_PATCH_TARGET_RETENTION_LIMIT` | 512 successor heads |
| Cold scoped comparisons | one per target per reconciliation pass |
| Scoped-patch fallback | roughly 12 git invocations per evaluation |
| `CURRENT_PROJECTION_SCHEMA_VERSION` | 3, independent of the journal's 2 |
| `MAXIMUM_RECORDED_IDENTITY_INPUT_VALUE_BYTES` | 256 JSON-content bytes |

- Reservation freshness is computed from owner activity events only. Unrelated journal traffic from other worktrees does not refresh a reservation, so a busy repository does not mask an abandoned claim.
- `check` runs without the lock and without git. Adding a git call to `decide` changes the cost of the most frequent operation in the system.
- `DriftPathAttributionOutcome::{Ambiguous, CoordinationRunRequired}` exist only in the command payload and are never journalled — they describe a question the run could not answer, not a fact about the repository.
- The cheap drift comparison detects that something moved; it cannot attribute which reservation moved it. Attribution needs the full comparison.
- Drift runs after the write, not before. A path is discovered as touched, then classified.
- `git diff-tree --stdin` accepts a lone commit line beside its pair lines and prefixes that record with the commit's own id. One invocation answering two questions is what holds the pinned process budgets; those budget assertions are ratchets and are not relaxed to admit a second read.
- A completed-but-refused post-commit run carries `OutputStatus::ScopeAcquisitionRefused`, not `InvalidInput`: the two conditions a status must separate are a request that never ran and a run that observed everything and was refused only its acquisition. Both exit 5, so a consumer keying on the exit code alone still cannot tell them apart. That status outranks `DriftAttributionRequired` and `ObjectUnknown` in the drift status selection, because each of those names a follow-up command — `drift --reservation <id>`, `drift --full` — that this same rule refuses; ranking either first hands a refused caller a remedy that cannot succeed. Drift effects still outrank all three, so a refused run that committed an incursion is still reported as an incursion.
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
- Wall time is not driven by git process count. Fitting measured per-cell maxima against git argv gives roughly 8.6 ms per git process over a ~0.234 s intercept at zero of them, so the floor alone exceeds a 0.20 s budget and the five-git outcome measured slower than the twenty-two-git one. A plan that reaches a latency target by cutting git arity is unreachable on its own arithmetic.
- Concurrency bought the first order of magnitude on the scoped-patch reads — worst case 7.34 s to 0.28 s — and then stopped helping. What remains is per-process spawn overhead no overlap removes.
- A harness must never set an environment variable the engine reads as a scheduling switch. A measurement that changes the code path it measures is void however green it reads; that defect once serialized every timed concurrent read.
- `git::reference_lookup` returns `Missing` only on git's own exit 2. A failed `rev-parse`, a spawn failure, and malformed output each propagate as an error; collapsing any of them back into absence reintroduces a repaired defect.
- `rev-list --stdin` exits 128 on a single unknown object, blanking every result. `--ignore-missing` plus a per-item membership check confines the damage to the item actually missing.
- `anchor..HEAD` and "descendants of anchor" are different sets: a commit merged from a branch that forked before the anchor is in the first and not the second.
- Batching a per-item query silently converts a degradable failure into a fatal one. The origin query maps its failure to a named "cannot classify" state and must never propagate with `?`.
- A filter that drops unreadable anchors, combined with a collector that defaults the gap, produces a confident wrong answer instead of an error.
- `core.hooksPath = ""` does not resolve to the repository root — git rejects the empty path outright, so a hook configured that way never fires under any condition.
- An absent reflog proves nothing: reflogs can be disabled, so a missing entry leaves the hook alone rather than inferring a rename from a shared tip.
- After a proven trunk rename the hook and `.claude/config/berth.toml` disagree on the trunk name, and re-running `cargo berth init` reverts the hook to the stale configured value.
- `ReleasedWithoutCheckpoint` cannot raise the lost-evidence alert; it carries no integration status at all.
- A reported misattribution of a linked worktree's resolve does not reproduce, and was investigated on 2026-08-26 from the journalled actors, marker contents, invocation directory, and command route it recorded: the directories were passed in the correct order and a linked worktree's resolve is attributed to the linked worktree. The `identity_inputs` record on every journal record exists so a recurrence is diagnosable rather than re-investigated.
- `verify.sh test <package>` resolves lib and bin targets only and cannot see `crates/cargo-berth/tests/`, so a scoped package run reports green while every integration suite goes unrun. Name each integration target explicitly.
- A fixture edited until it passes stops proving its property. Call counts asserted without statuses, a helper filtering out the calls that had begun to scale, and a released fixture that could not show blocking status all went green while checking nothing. Every assertion change has to say what it still proves.
- `normalize_absolute_path` collapses `..` textually, which is sound only when every component left of a `..` is a real directory. It applies to a working directory the harness reports itself sitting in, never to a path a payload names as an edit target — reversed, the hook coordinates a file the write never touches while the write lands outside the repository uncoordinated.
- `Path::file_name()` is `None` for a path ending in `..`, so `<repo>/absent/../held.rs` reaches no existing ancestor and refuses visibly rather than resolving.
- A linked git worktree does not inherit `.claude/config/berth.toml`, so an unenrolled requester answers exit 0 for every edit.
- On macOS a worktree under `/tmp` is discovered as `/private/tmp/...`. The payload namespace and the canonical namespace genuinely differ, which is why `CoordinationDomain` carries both.
- `hook/mod.rs` is a shared protocol across three events. A new hook event extends it rather than growing a fourth private copy.
- Because the wrappers are pass-throughs, changing a hook's rendered text changes what users see with no front-end edit — and no front-end file to forget.
- This package's suite contains wall-clock-bounded tests. Concurrent verification runs push them past their deadlines even when compiles are serialized; those failures are contention and clear on a quiet rerun.
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

A live holder that is clean and zero commits ahead of trunk is treated the same way, and for the same reason. A clean working tree is a fact about the tree, not about the work: a stash entry lives in the repository rather than the worktree, an ignored file never appears in `git status`, and work already parked on a merged branch reads as clean while its holder still intends to hold the path. Retiring a first-touch reservation on that evidence would reintroduce exactly the inference this invariant forbids, with a fresher observation and the same failure. `active_work` therefore names the lifecycle stage the reservation is in, not an observation that a file is currently dirty. What the tool owes a blocked caller instead is the route out, so the overlap block names the disposition verbs that clear a first-touch holder.

**Fail open for editing, closed for integration.** These have asymmetric costs. If the ledger is unreadable and editing is blocked, the tool has bricked the repository for everyone — a coordination aid that stops work is worse than no aid. If the ledger is unreadable and integration proceeds, an unverified merge lands on trunk. So the cheap-to-recover direction fails open and the expensive-to-recover direction fails closed.

**Bypass is evaluated before any read.** The one situation where a user most needs to escape the tool is the one where the tool is broken. Any bypass path that first consults the ledger would fail exactly when it is needed, which is why both the binary and both hook scripts check the variable first.

**A bypass is recorded, not forgiven.** Removing the escape hatch would make the tool something people work around. Making the escape hatch invisible would make the ledger lie. Recording every bypass keeps both properties: the user can always get out, and the board always shows that they did.

**Enforcement is at the ref level.** A pre-commit hook can be skipped, and a verb-level check only covers the verbs. `reference-transaction` fires for every ref update, which means the gate sees merges, resets, and pushes made by tools that never heard of `cargo-berth`.

**The gate ships observing before it enforces.** A gate that starts refusing on day one gets disabled on day one. `Observe` produces the same records and the same warnings without blocking, so a repository can see what enforcement would have done before turning it on.

**Two release valves, not one.** Forced permits handle the case where the gate is right about the facts and wrong about this particular merge; they are one-use and journalled, so the exception does not become the norm. `CARGO_BERTH_BYPASS` handles the case where the tool itself is the problem. Collapsing them would mean either no way out when the tool is broken, or a permanent global off switch for a single exception.

**Drift detects after the write.** Intercepting writes would mean sitting in front of the editor, which is neither possible nor desirable. Comparing what the worktree actually touched against what it claimed catches the same divergence with no interception, and the cheap fingerprint comparison keeps the common case at two git calls so the check can run often.

**Moving and being modified are different questions.** The cheap comparison is a symmetric difference against the last fingerprint, so it answers "which paths moved since the last observation" — and a path that moved because it was restored to its committed content is in that answer. Reporting drift against a reservation wants exactly that question. Acquiring a new reservation does not: a restored path carries no work to protect, and the first-touch reservation it would take blocks every other worktree until a person resolves it by hand. So the post-write claim intersects the observed paths with the fingerprint it is about to cache, which lists only paths modified at the moment of the call, and both comparison modes reach the same answer for the same working tree.

**The commit-time drift check warns and never blocks.** A post-commit hook that fails leaves the user with a commit already made and a tool telling them it should not exist. Warning after the fact, and always exiting 0, keeps the commit valid and puts the finding where the user can act on it.

**The board is computed, not stored.** A maintained board file is a second copy of facts git and the journal already hold, and it is wrong the moment someone forgets to update it. Recomputing means the board cannot disagree with reality — and publishing the git cost of that recomputation keeps the price of the choice visible.

**One standalone binary.** Building this as a cargo subcommand crate keeps it installable with `cargo install`, invocable as `cargo berth`, and usable from a git hook as a plain executable. A library would have required a host; an editor plugin would have covered one editor and left the hooks unguarded.

**The engine owns the harness protocol; the front end owns nothing.** An installed front end and an installed engine upgrade at different moments. Every byte a front end composes itself is a byte the two can disagree about, and the disagreement shows up as a user reading text no version of the engine would have produced. Reducing the front end to `exec` makes that class of defect unconstructible: installing a new engine is the whole repair, and there is no front-end file that can be forgotten. The wrappers keep exactly one decision, whether the engine can be reached, because that is the one thing the engine cannot answer for itself.

**A condition the reader cannot act on is silence.** A hook speaks unprompted, after every tool call, in whatever directory a session happens to sit in. A notice there costs attention on every command and is repaid only if the reader can do something with it. A repository outside coordination and a directory under no repository at all are both conditions with no action attached, so they render nothing — while the same verb run by hand, where a person asked the question, still answers it in full. The wire fields never change between the two; only whether anyone is told unbidden.

**The occasion is recorded, not inferred.** `check`, `board`, and `drift` each serve a hook and a person running the verb directly. Letting the verb decide which event its words name would fuse two questions that move independently — which words a condition gets, and which occasion those words are for. `EngineAnswerOccasion` is recorded once by the hook that owns the process, so a response knows its occasion as a recorded fact rather than deriving one.

**Semantic names over representational ones.** `Result<Option<WorktreeId>>` says how a value is stored; it says nothing about which absence occurred, and it invites a caller to flatten several distinct absences into one benign branch. That is exactly what happened: every identity-read failure read as "unchanged". A named variant per state makes the flattening visible as a match arm someone has to write on purpose, which is why the repaired name says `IdentityNotRecorded` and not `IdentityUnavailable`.

**Integration proof survives a rewrite.** Ancestry against the protected tip is the cheap answer and the wrong *only* answer: rebasing, amending, and squashing are ordinary, and each one moves work onto trunk under a commit the reservation never named. Comparing the change the reservation made inside its own scopes asks the question that actually matters — did this work land — and answers it for a commit that no longer exists. Path existence would accept a file whose edits were later removed; whole-blob equality would reject the proof the moment trunk legitimately edits the same file again; the checkpoint's trunk snapshot as a baseline would attribute trunk's own concurrent commits to the reservation. Scoped content measured from the phase start is the predicate none of those failures reach.

**Only definitive verdicts are cached, and a deferral degrades rather than affirms.** A cache exists because the fallback comparison costs a dozen subprocesses; it is bounded because the alternative is paying that on every pass. But an `ObjectUnknown` answer is a fact about the environment, not about the repository, and storing it would make one failed subprocess durable across every future restart. The same asymmetry governs deferral: copying materialized evidence through a skipped comparison re-affirms proofs reachability has just refuted, which is a false positive — the class this system ranks strictly worse than a false negative, because a false negative holds work and a false positive lands it unverified.

**The git hook classifies before it spawns.** `reference-transaction` is the highest-frequency event git has: a three-commit rebase delivers 75 of them, of which berth wants a handful. Paying a process spawn per delivery made a rebase cost eight seconds where the same rebase without hooks cost a quarter of one, and — because the bypass check lived inside the binary — setting the escape variable still paid five of those seconds. Classifying in shell, at a fixed two-process cost independent of how many refs a transaction names, is what makes the gate affordable enough to leave on. The table fails toward invoking the binary because a false negative silently drops the trunk gate while a false positive costs one invocation.

**Recovery is an argv, not a sentence.** A front end that reads a recovery out of prose is a parser of text the engine is free to rewrite, and the failure is silent: the text changes and the front end renders nothing, or worse, something stale. Publishing `argv` and `cwd` as typed fields means the front end prints a line the user can run. It also means a command that cannot be faithfully represented as text has exactly one disposition that does not mislead — omission — since the argv is meant to be executed verbatim and a damaged one is worse than an absent one.

**The wire contract is frozen first.** Envelope fields, `kind`/`data` tagging, exit codes, and the journal operation names are consumed by hooks, scripts, and harnesses that upgrade on their own schedule. Fixing them before the behavior settles means later work adds variants rather than renaming fields.
