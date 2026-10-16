# cargo-berth: one reservation per phase

## What it is

One unit director can run several phases of a plan side by side in one worktree, each with its own cargo-berth reservation and its own checkpoint. All of a director's reservations share one coordination run. Each phase runs its checks under its own harness session id, `CARGO_BERTH_SESSION_ID=<director sid>.phase-<N>`, so a phase's paths stay in that phase's reservation and release with that phase's checkpoint instead of folding into whichever phase opened first. Overlap answers can name the phase they belong to, and an overlap refusal names the holder's run so it never reads as the requester's.

## How it works

### The caller pattern

- Open a phase: `CARGO_BERTH_SESSION_ID=<sid>.phase-<N> cargo-berth check <paths>`. The first check under that session opens a new reservation in the director's run; later checks under it widen that reservation.
- Answer an overlap for one phase: `cargo-berth claim <paths> --reservation <id> --<before|after|defer|override> <holder> --overlap-why "<why>"`.
- Checkpoint one phase: `cargo-berth drift --full --reservation <id>`, then release it.
- The director's own Bash calls go through the post-tool-use hook, which never widens one phase with another phase's files.

The director skills (`/unit:direct`, `/unit:checkpoint`) carry this pattern; the crate only supplies the selections below.

### Identity

`ledger::resolve_identity` resolves the run once per invocation: session mapping, then `CARGO_BERTH_RUN`, then the worktree's coordination-run marker. A phase session id that has no mapping yet falls through to the marker, so it resolves to the director's run. That is what puts every phase reservation in the same run.

### Bare `check`: mapping, else a new reservation

`CheckArguments::into_check_request` (`src/cli.rs`) maps a bare `check` to `CheckReservationSelection::SessionMappingOrNew` and `check --reservation <id>` to `Explicit(id)`. The pre-tool-use edit hook builds its own `CheckRequest` with `SessionMappingOrSingleActive`.

```rust
// src/verb/claim/mod.rs
pub(crate) enum CheckReservationSelection {
    SessionMappingOrSingleActive, // edit hook
    SessionMappingOrNew,          // bare check
    Explicit(ReservationId),      // check --reservation
}
```

Under the ledger lock, `select_first_touch_reservation_reuse` reads `FirstTouchSessionReservationMapping` (`src/session/mod.rs`) and decides:

| Selection | Mapping state | Result |
| --- | --- | --- |
| `Explicit(id)` | any | `from_explicit_selection`; an id not active for the acting run and worktree is `IneligibleExplicitReservation` |
| any | `Mapped` to an active reservation of the acting run | that reservation (`from_locked_session_mapping`) |
| `SessionMappingOrNew` | `AvailableSessionWithoutMapping` | `append_unheld_first_touch_scopes`: a new reservation in the acting run |
| any other | `AvailableSessionWithoutMapping` or `HarnessSessionUnavailable` | `from_unmapped_active_run_reservations`: none opens one, one is reused, several is `ambiguous_active_run_reservations` |

A mapping that names another run falls through to the unmapped rule, so it never redirects a widen onto that run's reservation.

### Sibling-held paths

`partition_first_touch_protected_scopes` treats every active reservation of the same run and worktree as one party: a path any sibling declares is covered and is never added to a second reservation. `append_unheld_first_touch_scopes` applies it to a new phase:

- `sibling_held_scopes` lists, per sibling, the requested paths it already holds. They come back as `acquisition.already_held: Vec<SiblingHeldScopes { reservation_id, scopes }>` (absent when empty), and the message names them.
- The residual paths become the new reservation (`FirstTouchReservationReuse::AppendRequired`, appended by `append_first_touch_claim`).
- When siblings hold every path, the check reports `already_held` under the sibling holding the first requested path, appends nothing, and returns `AlreadyHeldSessionMapping::PreserveUnmappedSession`. The publication is `SessionIdentityMappingPublication::LeftUnmapped` (`{"status": "left_unmapped"}`), so a later bare check under that session still opens new work.
- A mapped or explicit selection whose paths are all held uses `PublishSelectedReservation`: it maps the session to the selected reservation and reports `published`.

Opening a phase past `maximum_reservations` returns `CheckPayload::ReservationLimitReached { maximum }` through `OutputEnvelope::check_reservation_limit_reached`: status `reservation_limit_reached`, exit 1, no claim, no mapping.

### Post-tool-use drift: only the sole candidate

`post_tool_use_drift_request` (`src/hook/post_tool_use.rs`) runs cheap drift as `EveryActiveForPostCommit { widening: PostCommitWideningSelection::SoleRunCandidate }`.

```rust
// src/drift/selection.rs
pub(crate) enum PostCommitWideningSelection {
    Explicit(ReservationId),
    SessionMappingOrSingleCandidate, // explicit drift, post-commit drift
    SoleRunCandidate,                // post-tool-use hook
}
```

`resolve_post_commit` builds the candidates: active reservations of the acting run in the worktree. `SoleRunCandidate` ignores the session mapping. With one candidate it widens that one; with none it widens nothing; with several, unheld changed paths come back as `DriftPathAttributionOutcome::Ambiguous` naming the candidate ids. `SessionMappingOrSingleCandidate` first takes the mapped reservation when it is a candidate, so a phase session running `drift` attributes its own files.

A path a sibling already holds is `DriftBlockingCoverage::NoForeignStanding` in `drift_coverage_probe`, so drift never widens one phase onto a path the other declared.

### Checkpoint drift beside a released sibling

A full checkpoint drift for phase B can see phase A's last commit inside B's phase range after A checkpointed and released. `unclaimed_path_can_widen` (`src/drift/classification.rs`) keeps B from adopting those paths: a committed-only path widens only when the HEAD commit introduced it and `released_sibling_checkpoint_covers` finds no released reservation of the same run and worktree that covers the path and whose `protected_tip` is the full comparison's anchored target (`ObservedDriftChanges::compares_to_checkpoint`, `src/drift/observation.rs`). Dirty edits to those paths, and later commits that change them, still widen B.

### `claim --reservation`: answer for one phase

`ClaimArguments` carries `--reservation`, which clap makes conflict with `--target`, `--head`, `--why`, `--plan` and `--phase`. `ClaimRequest.reservation_selection` is:

```rust
pub(crate) enum ClaimReservationSelection {
    NewReservation,           // ordinary claim, unchanged
    Existing(ReservationId),  // claim --reservation <id>
}
```

`claim::acquire` sends `Existing` to `src/verb/claim/named.rs`:

- `validate` selects with the same `from_explicit_selection` rule as `check --reservation`. An unknown, inactive, foreign-run or foreign-worktree id is `ClaimRejection::IneligibleExplicitReservation` (`invalid_input`, exit 5) and changes neither the journal nor the mapping.
- Conflicts come from `conflicts_for_claim`. `authorization` binds the answer through `bind_claim_authorization` with an `OrderingContext` whose `reservation_selection` is `Existing(id)`, so `replaced_orderings` (`--replace`) considers only edges incident on the selected reservation. `--defer`, and a conflict answered by the repository's `default_answer`, bind through `bind_answer_authorizations`.
- `coverage` adds only paths neither the selected reservation nor an active sibling holds, and reports sibling coverage in `already_held`.
- `mutation` chooses the journal record:

| Answer | Paths to add | Record | `acquisition.kind` | Envelope status |
| --- | --- | --- | --- | --- |
| none, `--before`, `--after`, `--override` | some | `Widen { cause: Explicit { reason: "claim --reservation" } }` | `widened` | `widened` |
| none, `--before`, `--after`, `--override` | none | `Answer { reservation_id: Some(id) }` | `already_held` | `answered` |
| `--defer` or default answer | any | `Answer { reservation_id: Some(id) }` | `answered` | `answered` |

- The selected reservation keeps its recorded target, phase-start head, purpose and plan/phase. Widening needs no reservation slot, so it succeeds at `maximum_reservations`.
- `consume` publishes the selected reservation to the session mapping for `already_held` and `answered` results itself, because `apply_journal_event` publishes nothing for an `Answer` event; a `Widen` publishes through the journal event as usual.
- A repeated `--before` or `--after` that would add an endpoint pair the ordering graph already holds (`OrderingGraph::contains_sequence`) refuses as `duplicate_ordering_edge`, exit 2, before any journal write (`OutputEnvelope::claim_duplicate_ordering`).

Replay binds an `Answer` event's authorizations to its `reservation_id`, and the board's replaced-edge view (`replaced_edges`, `src/board/answers.rs`) reads `Answer` events through `reservation_answers`.

Without `--reservation`, a permissive claim opens a new reservation and an ordinary `--defer` binds to the run's oldest active reservation in the worktree, as before.

### Overlap refusal text

`overlap_holder_description` (`src/output.rs`) builds the message of a blocked `check` or `claim`; `blocked_message` appends the first-touch disposition after it. Each holder's coordination run sits beside its reservation id:

- One holder: `Reservation <H> (coordination run <run>) on <branch> (<source>, <purpose>) holds paths that overlap the requested paths; reduce the requested scopes or coordinate with the holder, then retry.`
- Several holders: `<n> reservations hold paths that overlap the requested paths: reservation <H> (coordination run <run>) on <branch> (<source>, <purpose>); …; reduce the requested scopes or coordinate with the holders, then retry.` Entries are joined by `; `.

`ReservationConflict.holder_run_id` supplies the run. The README's collision example carries the same text.

### Key files

| File | Role |
| --- | --- |
| `src/cli.rs` | bare `check` selection; `--reservation` on `claim` |
| `src/verb/claim/mod.rs` | `CheckReservationSelection`, `ClaimReservationSelection`, `ReservationAcquisition`, `SiblingHeldScopes`, `append_unheld_first_touch_scopes`, `partition_first_touch_protected_scopes`, `AlreadyHeldSessionMapping` |
| `src/verb/claim/named.rs` | named claim: validate, coverage, mutation, consume |
| `src/session/mod.rs` | `FirstTouchSessionReservationMapping`, `SessionIdentityMappingPublication::LeftUnmapped`, `apply_journal_event` |
| `src/drift/selection.rs`, `src/hook/post_tool_use.rs` | `SoleRunCandidate` |
| `src/drift/classification.rs`, `src/drift/observation.rs` | released-sibling checkpoint guard |
| `src/edge/graph.rs` | `contains_sequence` |
| `src/output.rs` | `overlap_holder_description`, `selected_claim`, `check_reservation_limit_reached`, `claim_duplicate_ordering` |
| `tests/parallel_phases.rs` | two-phase end-to-end, `left_unmapped`, sibling coverage, limit, released-sibling checkpoint, named claims |
| `tests/hooks.rs`, `tests/lifecycle.rs`, `tests/overlap.rs` | hook selection, single-phase lifecycle, refusal text |

`tests/parallel_phases.rs` holds the shared helpers `initialized_repository`, `run_berth_with_session`, `run_post_tool_use`, `journal_events`, `open_phase`, `phase_board`, `session_mappings`, `SESSION_MAPPING_PATH` and `CONFIGURATION_PATH`.

## Invariants

- A bare `check` under a usable, unmapped session id never joins an existing reservation; it opens a new one in the acting run or reports `already_held` and stays unmapped. A `check` with no usable session id still joins the run's only active reservation.
- The edit hook keeps `SessionMappingOrSingleActive`. Changing what bare `check` selects must not change the edit hook.
- A path an active reservation of the same run and worktree declares is never added to a sibling, by `check`, named `claim` or drift.
- `left_unmapped` writes no mapping. A fresh session whose paths siblings already hold must stay free to open its own reservation later.
- The post-tool-use hook never reads the session mapping to choose a widening target. Explicit drift and post-commit drift keep honoring it, so a phase session can attribute its own files (`session_mapping_attributes_post_commit_widening_with_two_active_reservations` in `tests/drift.rs` pins this).
- A mapping naming another run never redirects a widen.
- A named claim never changes the selected reservation's target, phase-start head, purpose or plan/phase, and touches no sibling's edges.
- A refused named claim (ineligible id, duplicate edge) writes no journal record and moves no mapping.
- A blocked overlap message names each holder's run beside the holder's id, and never names the requester's run.
- `already_held` on the wire is additive and absent when empty; `answered` and `left_unmapped` are values in `docs/cargo-berth/generated/output-contract.json`.

## Gotchas

- Replay rejects a `DuplicateEdge` (`EdgeReplayError::DuplicateEdge`). A named claim must refuse a repeated edge before writing the journal; a record that slipped through would make the journal unreplayable.
- The director's real session gets mapped whenever a `Widen` or `Claim` is applied under it, including the post-tool-use hook's sole-candidate widen. That mapping no longer steers the post-tool-use hook, but a hand-run bare `drift` under `CLAUDE_CODE_SESSION_ID` and the edit hook still follow it. A director names the reservation with `--reservation` instead of relying on its own session.
- With two phases active and the director's session unmapped, an Edit or Write under that session gets `ambiguous_active_run_reservations` from the edit hook; the recovery is `check --reservation <id>`.
- `left_unmapped` reports `already_held` under the sibling holding the first requested path only. Other siblings show up in `already_held` entries, not in `reservation_id`.
- Timing under load misleads. Hook-band tests swing from about 0.3 s to 2–3 s, and the overlap message tests from under 0.1 s alone to 1.5–3.3 s in a loaded full run, unchanged tests included. Time a test alone before calling it slow. Named-claim fixtures were trimmed to at most 0.566 s.

## Why

- **A session id is the phase boundary.** Before this, a bare `check` from any session with one active reservation widened it (`SingleActiveRunReservation`) and mapped the new session onto it, so a second phase's paths released with the first phase's checkpoint. The run cannot distinguish phases; the session id can, and the director already has one per phase by suffixing its own.
- **Only a usable, unmapped session opens new work.** A hand-run `check` with no session has nothing to key a new phase on, so joining the sole reservation stays the right answer there. The edit hook keeps its old selection because its session is the director's real one, not a phase.
- **A new phase takes only free paths.** Reusing `partition_first_touch_protected_scopes` keeps one owner per path within a run. Overlapping phases are ordered through overlap answers, not by giving both phases the path.
- **`left_unmapped` instead of mapping to the sibling.** Mapping a fresh phase session onto the sibling that holds its paths would make its next check widen that sibling, which is the defect this design removes.
- **The post-tool-use hook ignores the mapping.** It runs under the director's real session, which gets mapped as soon as any widen lands under it; from then on every unheld changed path widened that one phase even with another active. Reporting ambiguity names the candidates and moves nothing. Explicit and post-commit drift keep the mapping because there the session is the phase session naming its phase.
- **A released sibling's checkpoint stays its own.** A phase range keeps every commit until the phase advances, so the sibling's final commit sits in the next checkpoint's comparison; adopting those paths would claim finished work for the wrong phase.
- **`claim --reservation` exists** because an ordinary permissive claim opens a new reservation and an ordinary `--defer` binds to the oldest active reservation, so with two phases active neither could attach an answer to the phase that needs it.
- **Answer-only records publish the mapping in `consume`.** `apply_journal_event` deliberately publishes nothing for `Answer`, since an ordinary answer has no session to map; the named claim is the one caller that knows the reservation the session meant.
- **The holder's run sits beside the holder's id.** The earlier text, `holds overlapping paths for <run>`, put the run after "for", where it read as the requester's run.
- **Ruled out: deduplicating repeated edges on a plain claim.** Each plain claim opens a distinct reservation, so its edge has a new endpoint and no duplicate arises; only the named claim can repeat a pair.
- **Ruled out: making the post-tool-use hook choose among phases.** The hook runs under the director's real session and carries no phase identity, so no rule there can tell which phase wrote a file; the director names the reservation instead.
