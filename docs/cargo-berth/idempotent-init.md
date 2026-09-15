# idempotent-init — `cargo berth init` enrolls the worktrees already in flight

> **Status: DESIGN PLAN — not yet phased for delegation.** `init` brings every live worktree under coordination in one pass, including work committed before the install, and can be re-run at any time to enroll worktrees it has not seen.

## Problem

`cargo berth init` assumes a repository with no work in progress. It creates the ledger, writes the configuration, installs the two managed hooks, and stops. On a repository that already has several worktrees with active branches, this leaves four defects.

1. **Committed work is never covered.** A first-touch claim records the current `HEAD` as its phase start (`src/verb/claim.rs:651`), and a post-write claim acquires only paths still modified in the working tree (`post_write_claim_subject`, `src/drift/observation.rs:337`). Every commit a branch made before the install sits below the phase start and outside every later drift comparison. Two branches that both committed changes to the same file before the install produce no claim, no incursion, and no ordering edge. The collision surfaces at merge time, which is the failure the tool exists to prevent.
2. **Ownership goes to whoever acts first.** Uncommitted edits in two worktrees to one file are claimed by the first worktree whose pre-edit hook, post-Bash drift, or post-commit drift runs. The order depends on which session happens to issue a tool call first, not on which work came first.
3. **The second worktree is accused of an incursion it did not commit.** A working-tree path is classified against the ledger's present holders (as-built doc, "A working-tree path was written just now"). The second worktree's pre-install edit is therefore reported as an entry into the first worktree's reservation, with the instruction to stop and resolve. The as-built rule for committed paths already states the principle this breaks: a write cannot enter a claim that did not exist when the write happened.
4. **`init` in a linked worktree leaves its siblings unconfigured.** `BerthConfig::initialize` writes to the invoking worktree's root (`src/config.rs:111`), while `WorktreeContext::configuration_lookup` (`src/ledger/worktree_context.rs:255`) has a linked worktree read its own file and then the main worktree's. A config written into one linked worktree is invisible to every other linked worktree. Their hooks report `Unconfigured`, which renders nothing, so the gap is silent.

The trunk gate does not help: a branch with no reservation has nothing to hold, so `GateDecision::Clear` applies (`src/gate/decision.rs:77`), and the shipped `gate_mode` is `observe`.

## Target behavior

After this plan, `cargo berth init` on a repository with work in flight:

- writes the configuration where every worktree reads it;
- enrolls each live registered worktree that has never held a reservation, with one reservation covering its full footprint since its merge-base with trunk;
- records every overlap between enrolled footprints as an unanswered question that holds integration and leaves editing open for both sides;
- reports each enrolled worktree, each worktree it could not fully enroll with its reason and recovery, and each overlap with a ready-to-run answer command;
- is idempotent: a second run enrolls only worktrees still never enrolled, finishes any interrupted or partial enrollment, changes nothing else, and reports the same overlaps if they remain unanswered.

A worktree `init` could not enroll gets the same enrollment at its first contact with the engine.

## Design

The design has one state model (D2) and one way enrollment reaches the journal (D4). Every other section is written in their terms.

### D1. Configuration and policy location

**Where the file goes.** One policy selection, resolved once, holds the config path, the policy location, and the parsed config; both the baked `__POLICY_WORKTREE__` value and the baked trunk derive from it.

- Main worktree detection uses git's reported main worktree, not a `.git` directory name, so separate-git-dir layouts work.
- A non-bare repository's shared location is the main worktree root.
- A bare repository's shared location is `<common-git-dir>/cargo-berth/berth.toml`.
- An existing file in a linked worktree is still validated and still wins on read. When `init` migrates a linked-only config to the shared location, the effective policy (trunk, `gate_mode`, limits) is carried over, never replaced by defaults, and any disagreement is reported.

**Publication.** `BerthConfig::initialize` today opens the destination with `create_new` before writing, and an empty or partial file parses with defaults, including `gate_mode = observe`. The fix lives in `config.rs`: write and sync the complete config to a sibling temporary file, link it into place without replacing a concurrent winner, sync the directory, then validate whichever file won.

**Gate context.** The hook changes into its policy worktree, and gate evaluation calls `WorktreeContext::discover`, which needs a checkout's `.git` entry. The policy location becomes a typed value, worktree-backed or common-directory-backed, carrying the exact config path and the common git dir through gate policy loading and rewrite handling. The issuing worktree is resolved separately, for actor identity only. A removed linked worktree, or a bare repository with no checkout at the policy location, no longer breaks the gate.

### D2. Enrollment state model

Enrollment state is a replay projection owned by one module. Adoption (D8) and reservation lifecycle are separate projections; identity ambiguity (D3) is an observation result, not a state.

```rust
enum EnrollmentState {
    /// No enrollment history. Eligible.
    Unseen,
    /// A batch naming this worktree is open and not yet completed.
    Writing(BatchId),
    /// Enrollment is recorded but coverage is not complete.
    Waiting(EnrollmentWait),
    /// Footprint fully accounted for. Never enrolled again.
    Accounted(EnrollmentReceipt),
}

struct EnrollmentWait {
    cause: WaitCause, // MissingTrunk | UnrelatedHistory | CapacityRefused { group: RefusalGroupId }
    coverage: WaitCoverage, // Unclaimed | Partial(ReservationId)
}
```

| Entry record or result | State | Integration hold | Eligible for enrollment work |
| --- | --- | --- | --- |
| No history; observation failed or footprint proven empty | `Unseen` | none | yes |
| Batch opened naming this worktree | `Writing` | pending hold (D4) | resume only |
| Batch completed, full footprint | `Accounted` | ordinary reservation holds | no |
| Batch completed, dirty paths known, committed half unknown | `Waiting(Partial)` | held until completion | completion only |
| Awaiting-footprint record (clean, trunk missing or unrelated) | `Waiting(Unclaimed)` | none (nothing claimed) | yes |
| Capacity refusal record | `Waiting(Unclaimed, CapacityRefused)` | none | whole group only (D7) |
| Legacy `Claim` actor, `RebindWorktree`/`RelocateWorktree` destination | `Accounted` | ordinary | no |
| Terminal resolution of a `Partial` reservation (release, abandon) | `Accounted` | ordinary | no |

"Never held a reservation" stays the idempotence key: a worktree whose reservation was released, integrated, or abandoned has had its committed work accounted for. A released `Partial` reservation ends the obligation rather than blocking the user's release.

Unborn `HEAD` produces no record and stays `Unseen`.

### D3. Candidates and observation

**Discovery.** One `git worktree list --porcelain` read through `WorktreeRegistry` (`src/worktree/liveness.rs:89`). A worktree is a candidate when its `WorktreeRegistrationState` is `Available` or `Locked`, its `RegisteredWorktreeLocation` is `Discovered`, and D2 makes it eligible. A `Prunable` registration or an `Unavailable` location is reported with its own reason.

**Identity.** Worktree ids are created at one shared boundary, `create_or_read_worktree_id` (`src/ledger/identity.rs:103`), which every command reaches through `resolve_identity`, including `board` reconciliation. Two fixes live there:

- The file is published atomically: write and sync a temporary file in the same directory, then link it into place without replacing an existing id.
- When the file is missing, creation runs under the mutation lock and first compares the registration (administrative locator, root) with historical bindings in replay. A registration not proven new is `identity_ambiguous`: no id is created, and the report names `cargo berth identity rebind-worktree --from <previous-worktree-id>`. That command appends a worktree-level rebind that works when every historical reservation is released, unlike `resolve --recovered` (`recovery.rs:572`), which rejects released reservations. Existing-id reads keep today's cost.

**Footprint.** Per candidate, outside the lock:

- Trunk and `HEAD` object ids are resolved once and used for every read, the phase start, and the recorded fields.
- *Committed:* `git::unmerged_branch_paths(trunk, head)` (`src/git/reachability.rs:276`).
- *Working tree:* the tracked-and-untracked status read drift uses (`observe_working_tree_status`, `src/drift/observation.rs:533`), run in that worktree's root.
- The footprint is the union, as exact file scopes, captured as one immutable observation revision.

**Stability and failure.** `HEAD`, index, and in-progress operation state (`rebase-merge`, `rebase-apply`, `MERGE_HEAD`, `CHERRY_PICK_HEAD`, `REVERT_HEAD`) are checked before and after observation. A change triggers re-observation up to a named attempt limit. On enrollment paths only, every git command, the shared registry read included, runs under a deadline; expiry terminates and reaps the child, and completion returns immediately. Repository-wide failures abort. Per-candidate failures (`operation_in_progress`, `unstable`, `timed_out`, `git_failure`, `unsupported_path`) leave that candidate `Unseen` while healthy candidates proceed.

**Empty-footprint cache.** A clean status cannot show an empty committed footprint, because a clean branch ahead of trunk also reports nothing. A proven-empty observation is cached in a disposable file beside the projection, keyed by worktree id, `HEAD` object id, trunk ref name, and trunk object id. A hit costs ref resolution plus one status read. The cache only skips git reads; it never decides enrollment state.

### D4. Publication protocol

Every enrollment, from `init` or from first contact, reaches the journal the same way. Nothing an enrollment writes becomes mutable reservation or graph state until its batch completes.

1. **Gate first.** Hooks are prepared outside the lock and activated before the first enrollment record is appended, so an interrupted batch is never left without the gate that reads its hold. When activation fails, the report says the hold is recorded but not enforceable.
2. **Open.** A fixed-size opening record names the batch id and its attempt. While the batch manifest is incomplete, the gate holds every trunk update (a repository hold). *Call: in enforce mode an `init` interrupted in this short window blocks trunk updates until any `init` or contact resumes it; the alternative is an unprotected window.*
3. **Manifest.** Bounded fragments list each participant: worktree id, pre-allocated reservation id, and captured integration subject (`HEAD` object id, with branch or detached recorded for display). Once the manifest is complete, the repository hold narrows to participant holds. The gate projects pending participants directly into its inputs, beside `entering_reservations` (`src/gate/decision.rs:298`), using the same reachability test on the captured object id; a participant whose head was unavailable holds conservatively, as `IntegrationSubject::WorktreeHeadUnavailable` does today. No reservation facts are fabricated. Admitted pending participants count against `maximum_reservations` in every acquisition path; completion converts the allocation without counting it twice.
4. **Payload.** Bounded fragments carry each participant's logical claim, its complete merge extent, and each overlap pair binding (D6). Scope sets are staged once under a content id and referenced from claims, extents, and bindings, so a scope set shared by many pairs is stored and decoded once. The encoder yields a private validated record type holding final bytes including the newline, consumed without re-serializing.
5. **Complete.** A completion record activates every claim, extent, and pair of the batch together in the reservation and graph projections. Adoption (D8) and session publication are possible only after completion.

**Actors.** A batch is one `Ledger::transact` hold with a per-record actor. No record whose actor worktree differs from the invoker's publishes into the invoking process's session mapping (`session::apply_journal_event` publishes for `Claim` and `Widen` today); `identity_inputs` still records the invoker.

**Resume.** Staged fragments are immutable and bound to their observation revision. Under the lock, a resumer revalidates capacity, identity bindings, and concurrent claims. If every payload fragment is durable, it appends completion. If not, it re-observes and appends a superseding attempt of the same batch that keeps the pre-allocated reservation ids. Fault tests restart, retry, and attempt integration after every record boundary, including batches with no overlaps and detached participants.

**Size and memory.** No record exceeds `MAXIMUM_JOURNAL_RECORD_BYTES` (16 KiB, `src/ledger/constants.rs:19`), whatever the footprint, participant count, or pair count. Replay buffers an incomplete batch until completion or supersession. Reconciliation's `MergeExtentObserved`, whose key embeds the full working-tree fingerprint (`src/reservation/merge_extent.rs:14`), moves to the same fragment path; the prior complete extent stays in force until its replacement completes.

**Legacy records.** Existing `Claim`, `Widen`, and `MergeExtentObserved` records replay as immediately complete logical operations.

### D5. The enrolled reservation

Each completed participant materializes one reservation equivalent to a `Claim` with:

- `source`: a new `ClaimSource::Enrolled` variant, so the board, alerts, and journal readers can tell enrollment from first touch and explicit claims.
- `purpose`: names enrollment and the branch or detached head.
- `phase_start_head`: the merge-base of trunk and the captured `HEAD` (`PhaseStartSelection::Protected`, `src/verb/claim.rs:132`). A later rebase re-anchors through the existing `Resnapshot` path.
- `worktree_root`, `worktree_administrative_locator`, `head_snapshot`, `trunk_at_claim`: read for the candidate, through `ClaimRepositoryFacts::read` keyed on the candidate's `WorktreeContext` (`src/verb/claim.rs:1556`).
- Actor: the candidate's worktree id with an engine-issued run (D8).
- Merge protection: the complete extent from the captured footprint, published in the batch, so siblings are protected at completion and not only after reconciliation. No extra git reads.

**Partial reservations.** When trunk is missing or history is unrelated and the worktree has dirty paths, the reservation claims those paths with the captured `HEAD` as an explicitly provisional phase start and the existing `Unavailable` merge protection. It is `Waiting(Partial)` and keeps its integration hold. Completion, at a later `init` or contact once a merge-base exists, appends scope and authorization updates, a `Resnapshot` to the real merge-base, and a successful extent on the same reservation id, then marks the footprint accounted. A terminal resolution before then ends the obligation (D2).

### D6. Overlaps between enrolled footprints

A batch shares one replay, so no participant came first. An overlap between two participants is neither an incursion nor an answer the user gave.

**The authorization.** A new `ConflictAuthorization` variant, since the as-built answer set is closed. Each unordered reservation pair is declared exactly once in the batch, avoiding `AmbiguousDeferral`, and projects into the existing deferral collection in `edge/graph.rs` with a typed cause, `Enrollment` or `UserDeferred { reason }`, sharing hold and resolution logic.

- **Editing.** Both parties keep editing the shared paths. `conflicts_for_authorized_edit` (`src/reservation/retention.rs:478`) filters it through `authorizes` in both directions.
- **Grant binding.** The grant binds to the pair and its recorded shared scope set, not the counterpart's whole scope revision, so unrelated footprint growth does not re-block it (`AuthorizedOverlap::covers` compares the whole revision today). New overlaps get ordinary treatment; existing answers keep today's behavior.
- **Integration.** Both reservations are held, as a pending `Defer` is (`IntegrationHold::DeferredOverlap`, `src/edge/graph.rs:224`). Observe mode reports; enforce mode rejects.
- **No incursion.** No `Incursion` record is written, and later drift in either worktree does not report the pre-existing overlap, because each side is covered by its own reservation and the authorization.

**Resolution.** One prepared deferral resolution with `Ordered { first, then }` and `Unordered` variants, validated under the lock against the pair's declaration, endpoints, shared scopes, actor, and reason. `sequence <a> <b> --why <text>` produces `Ordered`, which alone runs edge validation (cycles, duplicate direction) and allocates an edge. `sequence <a> <b> --no-order --why <text>` produces `Unordered`, which retires the pair with no edge and does not count against `maximum_ordering_edges`. Replay's retirement step is shared. `sequence` keeps its single-invocation shape.

**Capacity.** Pair count is quadratic: 33 mutually overlapping worktrees yield 528 pairs against the default 512 edges. `init` reports prospective pairs and remaining edge capacity, and names the setting when ordering every pair would exceed it.

Why not auto-record `Defer`: every existing answer is a deliberate, attributed decision, and the board presents it as one. An engine-authored `Defer` would appear as a choice nobody made. Why editing stays open: refusing edits in two worktrees on the day of install is the lockout the observe-mode default exists to avoid, and both sides already hold the edits. Holding integration is what protects trunk.

### D7. Entry points and first contact

A worktree can miss `init`: it was unconfigured, its location was unreadable, or it was added later from a branch with commits ahead of trunk. Every engine entry in the worktree asks D2 first, from replay, before any refusal, unchanged return, or board read: `check` from the pre-edit hook (including the blocked path through `check::reconcile_and_retry`), post-Bash drift, post-commit drift (including an absent identity file in `comparable_worktree`), `session-start` (which has no first-touch branch today), and explicit `claim`.

**One module.** A private `enrollment` module takes explicit candidate contexts and captured observations, expands a contact into its recorded capacity group when there is one, prepares and validates under the lock, and returns a typed disposition. It shares claim construction with first touch; the caller owns publication. `init` passes every candidate; first contact passes its own worktree. `acquire_first_touch_with_reservation_selection` is not called per candidate, because it discovers cwd, resolves ambient identity, and opens its own transaction.

**The disposition governs acquisition.**

| Disposition | Pre-edit `check` | Post-write hooks | Explicit `claim` |
| --- | --- | --- | --- |
| `Accounted` / `Enrolled(id)` | Ordinary rules against the enrolled reservation | Ordinary drift | Ordinary rules |
| `EmptyFootprint` | Ordinary first touch | Ordinary first touch | Ordinary claim |
| `Writing`, `Waiting`, `CapacityRefused`, per-candidate failure, `identity_ambiguous` | Conflicts against existing holders still refuse; otherwise the edit is permitted with no claim appended, and the reason renders | Advisory, no claim appended, exit 0 | Refused with the reason and its recovery |

*Call: a failed or refused enrollment never falls through to an individual claim, because that claim would make the worktree look accounted with its committed footprint uncovered, and would let hook order take a capacity slot from the refused group. The edit itself stays open, and it enters the footprint of the next successful enrollment.*

**Pre-edit target.** The enrollment and the requested edit scopes are decided separately in the same transaction. The footprint and its overlaps are published even when the extra target is refused; the target joins the enrolled reservation only when ordinary `check` validation permits it.

**First explicit `claim`.** In an `Unseen` worktree with a non-empty footprint, the requested scopes join the enrolled reservation in one acquisition and the command returns that one reservation id. The caller's purpose and identity are recorded with the adoption (D8). Later `claim` calls create separate reservations as today.

**Asymmetric overlaps.** At first contact the other side's reservation already exists. The D6 authorization still applies, because the enrolling worktree's committed work predates the incumbent's claim just as it would at `init`. Paths written after enrollment are ordinary drift.

**Capacity group.** When `init` cannot fit every candidate, it enrolls none and records a refusal naming the group. A contact in any member re-evaluates capacity for the whole remaining group, never for itself alone, and revalidates membership under the lock.

### D8. Adoption

Ownership of an enrolled reservation is a replay projection:

```rust
enum EnrollmentOwnership {
    /// Created by enrollment; no identified caller has adopted it.
    EngineIssued,
    /// Adopted by the recorded run.
    Identified { run: CoordinationRunId },
}
```

- A local caller with no resolved identity (no presented run, session mapping, or marker) acts through the engine-issued run. Selection accepts the engine-issued run for such callers in the reservation's own worktree. No record is appended.
- The first local caller with a resolved identity appends one adoption record under the mutation lock, naming its run and keeping the enrollment attribution. Identity precedence stays as today (`src/ledger/authorization.rs:157`, session before environment). Mapping publication happens here.
- After adoption, the durable adoption fact enforces the owner: a different identified run is refused with the existing occupancy refusal, whatever the reservation's recorded presentation provenance, and later sessions inherit the owner through normal precedence. One append, no git.

**Notice.** A running session in a sibling worktree never sees `init` output. At adoption, and at every pre-edit and session-start contact while ownership is `EngineIssued`, the hook renders the reservation id, open enrollment overlaps with their actions, and whether the gate reports or rejects, all from replay.

### D9. Output

`OutputFacts::Init` gains an enrollment section, and hooks and the board use the same outcome table. The wire contract grows by addition only; `docs/cargo-berth/generated/output-contract.json` is regenerated.

| Outcome | Reason | State after | Coverage | Retried when | Recovery shown |
| --- | --- | --- | --- | --- | --- |
| enrolled | — | `Accounted` | full | never | none |
| already enrolled | `already_enrolled` | `Accounted` | unchanged | never | none |
| nothing to cover | `empty_footprint` | `Unseen` | none needed | `HEAD` or trunk moves | none |
| partial | `missing_trunk`, `unrelated_history` | `Waiting(Partial)` | dirty paths, integration held | a merge-base exists | trunk setting and config path, or release |
| awaiting | `missing_trunk`, `unrelated_history` | `Waiting(Unclaimed)` | none | a merge-base exists | trunk setting and config path |
| interrupted | `writing` | `Writing` | held, not active | next `init` or contact | `cargo berth init` |
| capacity refused | `capacity_refused` | `Waiting(Unclaimed)` | none | capacity fits the group | `maximum_reservations`, required value, config path |
| not enrolled | `unborn_head` | `Unseen` | none | after the first commit | none |
| not enrolled | `operation_in_progress` | `Unseen` | none | operation finishes | finish the rebase, merge, cherry-pick, or revert |
| not enrolled | `unstable`, `timed_out`, `git_failure` | `Unseen` | none | next contact | the git command and its error |
| not enrolled | `unsupported_path` | `Unseen` | none | path changes | the offending path |
| not enrolled | `identity_ambiguous` | unchanged | none | after repair | `cargo berth identity rebind-worktree --from <id>` |
| skipped | `prunable`, `location_unavailable` | — | none | registration returns | `git worktree prune` where applicable |

- Each enrolled worktree lists reservation id, root, branch or detached head, phase start, and scope count.
- Each enrollment overlap lists both reservation ids, the shared scopes, and executable `sequence` actions for both orders and `--no-order`, each built by a dedicated constructor with full argv and canonical `cwd`. An action needing user text for `--why` is a template, kept distinct from an executable action. Tests run each emitted command unchanged.
- The report states whether the gate reports or rejects, and whether each hold is enforceable (D4 step 1).
- The board shows `Writing` and `Waiting` worktrees even when they hold no reservation. Unresolved-overlap rows carry both endpoints' branch, root, purpose, and source from the built snapshots, with the enrollment cause rendered distinctly from user answers.
- Hook presentations stay silent for benign unchanged outcomes. The single legacy `init` line remains only when there were no candidates at all.

### D10. Named cases

| Case | Behavior |
| --- | --- |
| Worktree on trunk with dirty edits | Footprint is the working-tree paths; phase start is `HEAD`. |
| Detached `HEAD` | Enrolled; pending hold uses the captured object id. |
| Trunk missing or unrelated history, dirty | `Waiting(Partial)`, completed later on the same reservation id. |
| Trunk missing or unrelated history, clean | `Waiting(Unclaimed)`. |
| Unborn `HEAD` | `Unseen`, reported. |
| Rebase, merge, cherry-pick, or revert in progress | `Unseen`, reported; enrolls at first contact after it finishes. |
| Prunable registration | Skipped and reported. |
| Main worktree of a bare repository | Not a worktree; nothing to enroll. |
| Branch already integrated into trunk | Committed half empty; only dirty paths enroll. |
| Earlier reservation released | `Accounted`; never enrolled again. |
| Worktree id file deleted after enrollment | `identity_ambiguous`; no new id; rebind command reported. |
| `init` interrupted | `Writing`; any `init` or contact resumes it on the same reservation ids. |
| Capacity for one of two candidates | Neither enrolls; neither can take the slot alone. |
| First engine command is `claim` on a branch with commits | One reservation: footprint plus requested scopes. |
| First edit target held by a sibling | Enrollment published; the edit refused as today. |
| `init` re-run with nothing new | No record appended; open enrollment overlaps repeated. |
| Case-insensitive checkout | Footprint scopes pass through `PathCase`. |

## Invariants this plan keeps

- The journal is truth. Enrollment state and ownership are replay projections. The D3 cache only ever skips a git read.
- Decision and record in one lock hold. No git inside the lock.
- Nothing an enrollment writes is mutable reservation or graph state before its batch completes.
- All four existing overlap answers are unchanged. The new variant never revokes edit access.
- Nothing is auto-removed. An enrolled reservation is retired only by a user action.
- `check::decide` stays lock-free and git-free. Enrollment, adoption, and acquisition costs are budgeted separately below.
- The post-commit hook still always exits 0; post-write hooks stay advisory.
- Older journals replay unchanged. Legacy records are immediately complete logical operations; every new variant and record is additive.
- Bytes under `tests/fixtures/reader_compat` stay unchanged.

## Cost budgets

| Entry point | `Accounted` worktree | `Writing` / `Waiting` | `Unseen` |
| --- | --- | --- | --- |
| `check::decide` | unchanged, lock-free, git-free | same | same |
| `check` command, clear | unchanged | replay fact; resume or completion only when its cause may have cleared (ref resolution first) | ref resolution + cache lookup; hit adds one status read; miss adds merge-base, diff, status |
| post-Bash, post-commit drift | unchanged | same as `check` | same as `check` |
| session-start | unchanged plus notice rendering while `EngineIssued` | same as `check` | same as `check` |
| explicit `claim` | unchanged | refusal from replay | same as `check` |
| adoption | one lock and one append, once per reservation | — | — |
| `init` | one registry read plus per-candidate observation, all under deadlines | resume from durable bytes; re-observe only for a superseding attempt | same |

The full `check` command already reconciles when a foreign non-released holder exists; that cost is unchanged. Shared scope sets keep replay proportional to distinct scope sets, not to pairs.

## Phases

1. **Configuration and policy location (D1).** Atomic config publication, shared location for linked and bare layouts, typed policy location through gate loading. Tests: sibling linked worktree reads the config after `init` in another linked worktree; bare repository enforce-mode update from a sibling after removing the checkout that ran `init`; readers and interruption during an enforce-policy migration.
2. **Identity safeguards (D3 identity).** Atomic id file; guarded creation under the lock at the shared boundary; `identity rebind-worktree`. Tests: id deletion then `board` before `init`; deletion versus removal and recreation at the same path; rebind when every historical reservation is released.
3. **Records, replay, and resolution (D2, D4 records, D5, D6, D8 records).** Enrollment projection, opening/manifest/payload/completion/supersession records, awaiting and capacity-refusal records, content-addressed scope sets, validated encoder, gate pending subjects, `ClaimSource::Enrolled`, the enrollment authorization, partial completion transition, adoption record and ownership projection, the prepared resolution with `sequence --no-order` as its producer, `MergeExtentObserved` moved to fragments. Tests: replay round-trip; reader-compat unchanged; legacy records replay as complete; activation only at completion; gate holds pending, detached, and repository-hold participants under both modes; both resolution variants after replay and repeated; large shared scope sets decode once.
4. **`init` enrollment (D3 observation, D4 producer, D7 module, D9, D10).** Hooks activated first, discovery and observation under deadlines, batch publication, capacity group, output table. Tests: three worktrees with committed and dirty overlapping work; second `init` no-op; released worktree not re-enrolled; a 2,000-path branch and a 128-participant batch within the record cap; capacity refusal; fault injection after every record boundary; `init` from a mapped session enrolling a sibling that needs many fragments, then the initiating session's next hook; stalled git process and continuous commits.
5. **First contact and adoption (D7, D8).** Every entry point, dispositions, pre-edit target separation, first explicit `claim`, empty-footprint cache, adoption and notice. Tests: a worktree added after `init` with commits ahead enrolls on first edit; a fresh worktree from trunk matches first touch except `source`, `purpose`, `phase_start_head`; an accounted worktree performs no enrollment git reads; both hook orders after a capacity refusal; a failed enrollment appends no individual claim; competing identified adopters; anonymous post-commit before an identified caller; an already-running session's next hooks render the notice; repeated clean contacts and cache invalidation on branch movement.
6. **Documentation.** README "First use" rewritten for the in-flight case using the reservation id `init` returns; `operations.md` (adoption, identity rebind, interrupted enrollment, capacity groups, configuration location); the as-built doc; `json-contract.md`.

## Out of scope

- Coordinating worktrees across clones or machines. One ledger per repository instance, as today.
- Inferring ordering between enrolled overlaps from commit dates or merge-bases. The user answers.
- Enrolling stashed changes. A stash lives in the repository, not in a worktree, and cannot be attributed to one.

## Review history

Three team review passes on 2026-09-15 (correctness, architecture and types, risk and performance, ergonomics; the third pass added coherence and simplicity). Passes one and two recorded R1–R38 as patches; pass three found them not closed and they were consolidated into D1–D10 above. Every accepted item lives in the section named below; this table exists so a later review does not reopen them.

| Item | Subject | Now in |
| --- | --- | --- |
| R1, R31 | Enrollment before every early return, including explicit `claim` | D7 |
| R2, T8 | Pre-edit target decided separately from enrollment | D7 |
| R3, R27, T10 | Adoption with `EngineIssued` / `Identified` ownership | D8 |
| R4, R29 | Per-record actors; no foreign session publication | D4 |
| R5, R23, T4, T5 | Batch protocol, completion activation, resume, bounded opening | D4 |
| R6, R7 | Unique pairs, typed cause, pair-bound grants | D6 |
| R8, R30 | Prepared resolution, `Ordered` / `Unordered` | D6 |
| R9, R26, T13 | Fragments and shared scope sets; the claim-and-widen chain is withdrawn | D4 |
| R10, R12, R24, T2, T3 | State model; partial and awaiting; unborn head | D2, D5 |
| R11 | Idempotence key covers recovery | D2 |
| R13, R14, R33, R34 | Policy selection, shared location, atomic config, typed gate context | D1 |
| R15, R17, R36, T14 | Consistent observation, failure isolation, deadlines incl. discovery | D3 |
| R16, R32, T11 | Atomic id file, guarded creation at shared boundary, rebind command | D3 |
| R18 | Ordering capacity report | D6 |
| R19, R35, T16 | Cost budgets, empty-footprint cache, precise `check` invariant | D3, Cost budgets, Invariants |
| R20, R21, R22, T15 | Outcome table, board rows, recovery argv | D9 |
| R25 | Complete merge extent in the batch | D4, D5 |
| R28 | Adoption notice | D8 |
| R37, T7 | Capacity group; disposition governs acquisition | D7 |
| R38 | One private enrollment module | D7 |
| T1 | Gate hook active before the first record | D4 |
| T6 | Pending participants as gate subjects, detached included | D4 |
| T9 | First explicit `claim` yields one reservation | D7 |
| T12 | Pending participants consume capacity | D4 |
| T17 | Phase map with producers | Phases |

Calls made in consolidation, each a one-line revert in this doc: an interrupted opening holds all trunk updates until resumed (D4); a failed enrollment permits the edit with no individual claim (D7); a terminal resolution ends a partial reservation's obligation instead of blocking release (D2, D5); the first explicit `claim` folds into the enrolled reservation (D7).

## Proposed user decisions

None open.

- *Dropped — scope-narrowing verb for collapsed `tree:` scopes.* Exact scopes remove the over-claim (D4), so no narrowing verb is needed.
