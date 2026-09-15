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
- reports each enrolled worktree, each skipped worktree with its reason, and each overlap with a ready-to-run answer command;
- is idempotent: a second run enrolls only worktrees still never enrolled, changes nothing else, and reports the same overlaps if they remain unanswered.

A worktree `init` could not enroll gets the same enrollment at its first contact with the engine.

## Design

### D1. Configuration goes to the main worktree

`Ledger::initialize` resolves the configuration root through the same rule `configuration_lookup` reads: a linked worktree of a non-bare repository writes to the main worktree root; a main worktree writes to itself; a linked worktree of a bare repository writes to itself. An existing file in the invoking linked worktree is still validated and still wins on read, and `init` reports which file it created or found.

The `reference-transaction` hook bakes a policy worktree into its script (`src/gate/install.rs`, `__POLICY_WORKTREE__`). It should name the same root the configuration went to, so a removed linked worktree cannot leave the gate reading policy from a directory that no longer exists.

### D2. Enrollment

**Which worktrees.** One `git worktree list --porcelain` read through the existing `WorktreeRegistry` (`src/worktree/liveness.rs:89`). A worktree is a candidate when its registration is `Available` or `Locked`, its location is `Discovered`, and the journal holds no `Claim` whose actor is that worktree's id. `Prunable` and `Unavailable` registrations are skipped and reported.

"Never held a reservation" is the idempotence key, and it has to be a journal fact, not a live-state fact. A worktree whose earlier reservation was released, integrated, or abandoned has already had its committed work accounted for; enrolling it again would claim finished work.

**Worktree identity.** `create_or_read_worktree_id` (`src/ledger/identity.rs:103`) already creates the id file on first use in a worktree's administrative directory with `create_new`. Enrollment calls it for each candidate's administrative directory, so `init` running in one worktree assigns ids to the others with the same race safety.

**Footprint.** For each candidate:

- *Committed.* `git::unmerged_branch_paths(trunk, head)` (`src/git/reachability.rs:276`): one `git diff --merge-base --name-only -z --no-renames` per worktree, net of paths the branch changed and restored.
- *Working tree.* The same tracked-and-untracked status read drift uses (`observe_working_tree_status`, `src/drift/observation.rs:533`), run with that worktree's root as the working directory.

The footprint is the union, as exact file scopes. An empty footprint enrolls nothing: a worktree sitting clean at trunk has nothing to protect and will first-touch normally.

**Phase start.** The merge-base of trunk and the worktree's `HEAD`. `ProtectedPhaseStartHead` already accepts a caller-supplied commit (`PhaseStartSelection::Protected`, `src/verb/claim.rs:132`). Drift from that anchor covers the branch's committed work, and a later rebase re-anchors it through the existing `Resnapshot` path.

**The claim record.** One `JournalOperation::Claim` per enrolled worktree, with:

- `source`: a new `ClaimSource::Enrolled` variant. Enrollment is neither a first touch nor a caller's stated claim, and the board, the alerts, and a reader of the journal need to tell it apart. Per the as-built invariant, a new state is a new variant.
- `purpose`: an explanation naming enrollment at `init` and the branch or detached head.
- `coordination_identity_provenance`: `NotPresented`. The occupancy rule then never counts the enrolled reservation against a session that later presents `--run` in that worktree, and post-commit selection finds it as the sole active candidate, so drift widens it without further input.
- `worktree_root`, `worktree_administrative_locator`, `head_snapshot`, `trunk_at_claim`: read for the candidate worktree, not the invoking one. `ClaimRepositoryFacts::read` (`src/verb/claim.rs:1556`) is keyed on a `WorktreeContext`, and the registry already holds one per discovered registration.
- `authorization`: see D3.
- The actor is the candidate's worktree id with an engine-issued coordination run id, and `identity_inputs` records the invoking process as usual. The record therefore states that the reservation belongs to the enrolled worktree and that `init`, run elsewhere, created it.

**One transaction.** All enrollment claims are decided and appended under a single `Ledger::transact` hold, against one replay. Git reads happen before the lock, as incursion attribution already does; the validation closure re-checks under the lock that no candidate acquired a `Claim` in the meantime and drops any that did.

**Record size.** A long-lived branch can touch hundreds of files, and a record is capped at `MAXIMUM_JOURNAL_RECORD_BYTES` (16 KiB, `src/ledger/constants.rs:19`). Enrollment must never fail the install on this. When the exact-file scope set would exceed the cap, collapse the deepest directories with the most member paths into `tree:` scopes until it fits. A tree scope over-claims, and the report names every collapsed directory so the user can narrow it. A reservation split across several claim records is rejected as an alternative: the post-commit widening target stops being unique, which reintroduces the ambiguity message on every commit.

**Reservation limit.** Enrollment counts against `maximum_reservations`. If the candidates exceed the remaining capacity, enroll none, report the count, and name the setting. A partial enrollment would make ownership depend on registry order, which is defect 2 again.

### D3. Overlaps between enrolled footprints

All enrollment claims share one replay, so no enrolled worktree came first. An overlap between two of them is recorded as neither an incursion nor an answer the user gave.

Add a `ConflictAuthorization` variant for an overlap found at enrollment. The as-built rule is that the overlap answer set is closed and a new answer is a new variant. Its semantics:

- **Editing.** Both parties may keep editing the shared paths, the same guarantee all four existing answers give. `conflicts_for_authorized_edit` (`src/reservation/retention.rs:478`) filters it through `authorizes` like any recorded answer, in both directions.
- **Integration.** It holds both reservations, exactly as a pending `Defer` does (`IntegrationHold::DeferredOverlap`, `src/edge/graph.rs:224`). In observe mode the gate reports it; in enforce mode it rejects.
- **Resolution.** `cargo berth sequence` resolves it into an ordering edge through the existing `ResolveDefer` path, and `--override` semantics remain available through a second `sequence` form if the user wants no order. The board lists it under "Unresolved overlaps" with a cause that says it was found at enrollment.
- **No incursion.** Enrollment writes no `Incursion` record for these paths, and later drift in either worktree does not report the pre-existing overlap as an incursion, because each side's paths are covered by its own reservation and the recorded authorization.

Why not auto-record `Defer`: every existing answer is a deliberate, attributed, two-invocation decision, and the board presents it as one. An engine-authored `Defer` would appear on the board as a choice nobody made.

Why editing stays open: refusing edits in two worktrees on the day of install is the lockout the observe-mode default exists to avoid, and both sides already hold the edits. Holding integration is what protects trunk.

### D4. First-contact enrollment

A worktree can miss `init`: it was unconfigured at the time, its registration was `Locked` with an unreadable location, or it was added later from a branch that already had commits ahead of trunk. The first engine entry that would first-touch in a never-enrolled worktree — `check` from the pre-edit hook, post-Bash drift, post-commit drift, or `session-start` reconciliation — runs D2 for that one worktree instead of the plain first touch.

For a fresh worktree created from trunk, the merge-base equals `HEAD` and the committed footprint is empty, so the result is identical to today's first touch. The difference appears only where committed work exists.

An overlap found here is not symmetric: the other side's reservation already exists. The same D3 authorization applies, because the enrolled worktree's committed work predates the incumbent's claim just as it would at `init`. Paths the enrolling worktree writes after enrollment are ordinary drift.

The `check` path is lock-free and git-free by design (as-built: "Adding a git call to `decide` changes the cost of the most frequent operation"). Enrollment therefore runs only on the branch where `check` already escalates to `acquire_first_touch`, and only when the journal holds no `Claim` for this worktree — a replay fact available without git. Once enrolled, the worktree never pays the cost again.

### D5. Output

`OutputFacts::Init` gains an enrollment section: each enrolled worktree with its reservation id, root, branch, phase start, scope count, and any collapsed directories; each skipped worktree with a named reason (`prunable`, `location_unavailable`, `already_enrolled`, `no_merge_base`, `empty_footprint`); and each enrollment overlap with the two reservation ids, the shared scopes, and a runnable `sequence` argv. The wire contract grows by addition only, and `docs/cargo-berth/generated/output-contract.json` is regenerated.

The text message leads with counts, then one line per overlap. A run that enrolls nothing and finds no open enrollment overlap keeps today's single line.

### D6. Named cases

| Case | Behavior |
| --- | --- |
| Worktree checked out on trunk with dirty edits | Footprint is the working-tree paths; phase start is `HEAD`. |
| Detached `HEAD` | Enrolled; `head_snapshot` is `Detached`. |
| No merge-base with trunk, or trunk ref missing | Enroll working-tree paths only, phase start `HEAD`, and report `no_merge_base` for the committed half. |
| Rebase or merge in progress | Skip and report; enroll at first contact after it finishes. Drift already stands aside while `rebase-merge` or `rebase-apply` exists. |
| Prunable registration | Skip and report. No reservation for a checkout that is not there. |
| Main worktree of a bare repository | Not a worktree; nothing to enroll. |
| Branch already integrated into trunk | `unmerged_branch_paths` returns an empty set; only dirty paths enroll. |
| Worktree whose earlier reservation was released | Journal holds a `Claim` for it; never enrolled again. |
| `init` re-run with nothing new | No record appended; output repeats open enrollment overlaps. |
| Case-insensitive checkout | Footprint scopes pass through `PathCase` like every other claim. |

## Invariants this plan keeps

- The journal is truth; enrollment state is derived from replay, never from a marker file.
- Decision and record in one lock hold. No git inside the lock.
- All four existing overlap answers are unchanged. The new variant never revokes edit access.
- Nothing is auto-removed. An enrolled reservation is retired only by a user action.
- `check` stays lock-free and git-free on its clear path.
- The post-commit hook still always exits 0.
- Older journals replay unchanged; `ClaimSource::Enrolled` and the new authorization are additive.
- Bytes under `tests/fixtures/reader_compat` stay unchanged.

## Phases

1. **Configuration root and policy worktree (D1).** `init` from a linked worktree writes to the main worktree and bakes the main root into the hook. Tests: `init` in a linked worktree, then a sibling linked worktree reads the configuration and its pre-edit hook coordinates.
2. **Enrollment record types (D2 record, D3 variant).** `ClaimSource::Enrolled`, the enrollment overlap authorization, replay, board rows, output contract regeneration. No producer yet. Tests: replay round-trip, reader-compat unchanged, edit filter authorizes both directions, gate holds under both modes, `sequence` resolves it.
3. **`init` enrollment (D2, D5, D6).** Registry read, footprint, single transaction, record-size collapse, reservation limit, output. Tests in `tests/lifecycle.rs` or a new `tests/enrollment.rs`: three worktrees with committed and dirty overlapping work; second `init` is a no-op; released worktree is not re-enrolled; collapse fires on a 2,000-path branch; limit refusal enrolls none.
4. **First-contact enrollment (D4).** Pre-edit, post-Bash, post-commit, and session-start entry points. Tests: a worktree added after `init` from a branch with commits ahead enrolls its committed footprint on first edit; a fresh worktree from trunk produces the same record fields as today's first touch apart from `source`; `check` on an enrolled worktree issues no git call.
5. **Documentation.** README "First use" gains the in-flight case; as-built doc and `json-contract.md` updated.

## Out of scope

- Coordinating worktrees across clones or machines. One ledger per repository instance, as today.
- Inferring ordering between enrolled overlaps from commit dates or merge-bases. The user answers.
- Enrolling stashed changes. A stash lives in the repository, not in a worktree, and cannot be attributed to one.
