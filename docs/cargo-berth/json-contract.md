# cargo-berth JSON and journal contract

The wire contract for tools that consume `cargo-berth` output: the envelope
`cargo berth board --json` emits, and the journal records the ledger appends.
Both are stable — additions arrive as new variants, not renamed fields.

For what the tool is and how to use it, see the [README](../../crates/cargo-berth/README.md).

## The JSON envelope

This is an actual first read from a scratch repository. It contains a live
predecessor, a waiting edge with its user action, stale alerts with their
`renew` actions, and one recovered bypass:

```json
{
  "verb": "board",
  "status": "board_ready",
  "exit_code": 0,
  "reservations": [
    "01a036fa-b70a-7e72-89ae-0facf1976ed1",
    "01a036fb-1629-7712-96b7-1672b64a151f"
  ],
  "blocked_by": [],
  "message": "The reservation board was read. Use `cargo-berth board --json` to inspect it.",
  "payload": {
    "kind": "board",
    "data": {
      "journal_position": { "generation": 3, "journal_byte_offset": 2820 },
      "recovered_bypasses_this_invocation": [
        "cargo-berth-pending-bypass-readme-example.json"
      ],
      "integration_order": "constraints_recorded",
      "ready_now": {
        "journal_position": { "generation": 3, "journal_byte_offset": 2820 },
        "entries": [{
          "relation": "unordered",
          "reservation": {
            "reservation_id": "01a036fa-b70a-7e72-89ae-0facf1976ed1",
            "holder": {
              "worktree_id": "01a036fa-b6a0-7a03-b2fb-81799cb93e3a",
              "worktree_root": "/private/tmp/cargo-berth-phase11-collision.ay9US3/worktrees/holder",
              "branch": { "kind": "attached", "reference": "refs/heads/holder" },
              "liveness": "live"
            },
            "source": { "kind": "work_plan", "plan": "docs/work.md", "phase": "holder" },
            "purpose": { "kind": "explained", "explanation": "update the shared API" },
            "scopes": [{ "path": "crates/shared", "kind": "tree" }],
            "lifecycle": { "stage": "active" },
            "integration_evidence": { "kind": "active_work" },
            "edit_blocking_status": "blocking",
            "visibility": "active_constraint",
            "freshness": { "status": "stale", "last_activity_at": "2020-01-01T00:00:00.000Z" },
            "ahead_behind_main": { "status": "counts", "ahead": 0, "behind": 0 }
          }
        }]
      },
      "waiting": {
        "journal_position": { "generation": 3, "journal_byte_offset": 2820 },
        "entries": [{
          "edge_id": "01a036fb-1629-7712-96b7-168de72d8c2f",
          "predecessor": "01a036fa-b70a-7e72-89ae-0facf1976ed1",
          "successor": "01a036fb-1629-7712-96b7-1672b64a151f",
          "scopes": [{ "path": "crates/shared", "kind": "tree" }],
          "reason": "the holder API must land first",
          "action": {
            "reason": "predecessor_checkpoint",
            "instruction": "wait for the predecessor to reach a checkpoint; nobody can act yet"
          },
          "provenance": "acquisition",
          "declaration_event_id": "01a036fb-1629-7712-96b7-1699922daa50"
        }]
      },
      "settled_ordering_constraints": { "journal_position": { "generation": 3, "journal_byte_offset": 2820 }, "entries": [] },
      "unresolved_overlaps": { "journal_position": { "generation": 3, "journal_byte_offset": 2820 }, "entries": [] },
      "recorded_overlap_answers": {
        "journal_position": { "generation": 3, "journal_byte_offset": 2820 },
        "entries": [{
          "answer": "sequence",
          "reservation_id": "01a036fb-1629-7712-96b7-1672b64a151f",
          "blocker": "01a036fa-b70a-7e72-89ae-0facf1976ed1",
          "direction": "holder_before_requester",
          "exact_approved_scopes": [{
            "reservation_id": "01a036fa-b70a-7e72-89ae-0facf1976ed1",
            "scope_revision": [{ "path": "crates/shared", "kind": "tree" }],
            "scopes": [{ "path": "crates/shared", "kind": "tree" }]
          }],
          "authorization_reason": "the holder API must land first",
          "acquisition": { "origin": "claim" },
          "consequence": {
            "state": "holding",
            "action": {
              "reason": "predecessor_checkpoint",
              "instruction": "wait for the predecessor to reach a checkpoint; nobody can act yet"
            }
          }
        }]
      },
      "unconstrained_reservations": { "journal_position": { "generation": 3, "journal_byte_offset": 2820 }, "entries": [] },
      "resolved": { "journal_position": { "generation": 3, "journal_byte_offset": 2820 }, "entries": [] },
      "available_forced_permits": { "journal_position": { "generation": 3, "journal_byte_offset": 2820 }, "entries": [] },
      "bypass_audit": {
        "journal_position": { "generation": 3, "journal_byte_offset": 2820 },
        "entries": [{
          "kind": "environment_override",
          "override_name": "CARGO_BERTH_BYPASS=1",
          "occurrences": [{ "status": "unknown" }],
          "grouped_reference_transactions": 1,
          "skipped_holds": "override_preceded_ledger_read"
        }]
      },
      "outstanding_incursions": { "journal_position": { "generation": 3, "journal_byte_offset": 2820 }, "entries": [] },
      "recorded_incursion_answers": { "journal_position": { "generation": 3, "journal_byte_offset": 2820 }, "entries": [] },
      "alerts": {
        "journal_position": { "generation": 3, "journal_byte_offset": 2820 },
        "entries": [{
          "kind": "stale_reservation",
          "reservation_id": "01a036fa-b70a-7e72-89ae-0facf1976ed1",
          "freshness": { "status": "stale", "last_activity_at": "2020-01-01T00:00:00.000Z" },
          "resolution": { "action": "renew", "reservation_id": "01a036fa-b70a-7e72-89ae-0facf1976ed1" }
        }, {
          "kind": "stale_reservation",
          "reservation_id": "01a036fb-1629-7712-96b7-1672b64a151f",
          "freshness": { "status": "stale", "last_activity_at": "2020-01-01T00:00:00.000Z" },
          "resolution": { "action": "renew", "reservation_id": "01a036fb-1629-7712-96b7-1672b64a151f" }
        }]
      },
      "git_cost": {
        "trunk_resolution_calls": 1,
        "worktree_list_calls": 1,
        "reservation_evidence_revalidations": 0,
        "protected_predecessor_ancestry_queries": 0,
        "worktree_ahead_behind_computations": 0,
        "orphan_recovery_evidence_queries": 0
      }
    },
    "alerts": []
  }
}
```

Every section carries the same `journal_position`; a consumer can reject a mix
of generations or offsets. Reservation rows occur at
`ready_now.entries[].reservation`, `unconstrained_reservations.entries[]`, and
`resolved.entries[]`. Their complete tagged alternatives are:

- `holder.branch.kind`: `attached` with `reference`, or `detached` with `head`.
- `source.kind`: `work_plan` with `plan` and `phase`, or `explicit`.
- `purpose.kind`: `explained` with `explanation`, or
  `not_provided_by_caller`.
- `lifecycle.stage`: `active`; `outstanding` with `protected_tip`; or `released`
  with `disposition`. The disposition uses `kind = integrated`,
  `rewritten_integration`, `abandoned`, or `retired_orphan`; the last three add
  the scalar `evidence` field.
- `integration_evidence.kind`: `active_work`; `released_without_checkpoint`; or
  `current` with a nested `status`. That nested value has its own `status`
  discriminator: `integration_evidence.status.status` is `not_integrated`;
  `integrated` with `trunk_oid` and `proof`; `trunk_rewritten`; or
  `object_unknown`. Integrated `proof` is `protected_tip_ancestor` or
  `scoped_patch_equivalent`.
- `freshness.status`: `fresh` or `stale`, both with `last_activity_at`.
- `ahead_behind_main.status`: `counts` with `ahead` and `behind`; `unrelated`;
  or `unavailable`.

The remaining row enums are scalar strings:
`edit_blocking_status = blocking | clear`, `visibility = active_constraint |
reblocked_active_constraint | resolved_audit`, and `holder.liveness = live |
unavailable | orphan_candidate | orphaned | unknown`.

`edit_blocking_status` is a lifecycle-derived projection: active reservations
are `blocking`, outstanding reservations follow their integration evidence, and
released reservations are always `clear`. Consequently, newly assembled rows
never pair `lifecycle.stage = released` with `edit_blocking_status = blocking`;
the `reblocked_active_constraint` wire value remains reserved for v1
compatibility but is not emitted for released reservations.

The sections mean:

- `ready_now`: non-resolved endpoints involved in any recorded ordering edge,
  including an edge now listed under `settled_ordering_constraints`, except a
  current waiting successor or either endpoint of an unresolved deferral. Each
  entry wraps the row with `relation = "unordered"`. Non-resolved rows are
  active or outstanding reservations.
- `waiting`: holding edges. Its action is `predecessor_checkpoint`,
  `predecessor_not_integrated`, `trunk_evidence_rewritten`,
  `predecessor_object_unknown`, or `successor_must_incorporate_predecessor`.
  The actions respectively tell the user to wait for a checkpoint, wait for the
  checkpoint to reach trunk, record rewritten evidence with the supplied
  `resolve_flag`, restore the missing Git object, or incorporate the predecessor.
- `settled_ordering_constraints`: cancelled, fulfilled, or inactive-successor
  edges, tagged `cancelled_constraint_ended`,
  `fulfilled_successor_contains_predecessor`, or
  `successor_no_longer_active`.
- `unresolved_overlaps`: deferred pairs that still require a `sequence` answer.
- `recorded_overlap_answers`: durable `sequence`, `defer`, `override`,
  `ordering_created_from_deferral`, `existing_answers_cover_every_overlap`, and
  `widen_without_foreign_overlap` answers with their exact scopes and effects.
- `unconstrained_reservations`: non-resolved rows not involved in a recorded
  ordering edge or unresolved deferral. These are `active` and `outstanding`
  rows; a `released` reservation is always resolved audit history.
- `resolved`: released reservation audit history. Its four dispositions are
  `integrated`, `rewritten_integration`, `abandoned`, and `retired_orphan`.
  Orphan retirement remains distinct from deliberate abandonment.
- `available_forced_permits`: unused force permits and the skipped holds they
  can authorize.
- `bypass_audit`: durable force and environment-override history. It remains
  visible after any one-time notice is consumed.
- `outstanding_incursions`: incidents awaiting the row's supplied `flag`, which
  names `resolve <reservation-id> --incursion <incident-id>`.
- `recorded_incursion_answers`: durable resolutions for those incidents.
- `alerts`: lost integration evidence and its `resolve_integrated_as` action;
  orphan recovery evidence and its `recover` or `retire_or_abandon` action;
  stale reservations and their
  `resolution.action = "renew"`; or bypasses not yet recorded and an instruction
  for restoring the journal audit path. A lost-evidence alert identifies the
  released reservation, protected tip, current evidence status, and whether
  trunk must resolve before the operator can confirm integration. The orphan
  action names either `resolve --recovered` or the explicit retire/abandon
  flags; the stale action names the reservation for `renew`; the bypass alert
  names the recovery step.
- `git_cost`: exact Git-call counts used to build this board.

`integration_order` is `undeclared` or `constraints_recorded`.
`recovered_bypasses_this_invocation` is a
`RecoveredBypassesThisInvocation` list of pending-bypass marker ids. It is a
notice reported once by the read that imported the marker into the journal and
deleted it; the next board read returns an empty list. The corresponding
`bypass_audit` entry remains as durable history.

Board `lost_integration_evidence` entries use this tagged form:

```json
{
  "kind": "lost_integration_evidence",
  "reservation_id": "01a036fa-b70a-7e72-89ae-0facf1976ed1",
  "protected_tip": "1111111111111111111111111111111111111111",
  "evidence_status": { "status": "trunk_rewritten" },
  "recovery": {
    "kind": "verify_resolved_trunk",
    "trunk_oid": "2222222222222222222222222222222222222222",
    "action": {
      "action": "resolve_integrated_as",
      "reservation_id": "01a036fa-b70a-7e72-89ae-0facf1976ed1"
    }
  }
}
```

`evidence_status.status` is `not_integrated`, `trunk_rewritten`, or
`object_unknown`. `recovery.kind = verify_resolved_trunk` includes the resolved
`trunk_oid`. `recovery.kind = resolve_trunk_first` omits `trunk_oid` and requires
the configured trunk to resolve before the action is usable. Both alternatives
carry `action.action = resolve_integrated_as` and its `reservation_id`.
Released rows remain `edit_blocking_status = clear` in every alternative.

The envelope-level `payload.alerts[]` form carries the same fields under
`{ "kind": "lost_integration_evidence", "data": { ... } }`. Board alerts are
flattened under `payload.data.alerts.entries[]` as shown above.

Two similar answer tags are intentionally separate parts of the frozen schema.
Journal, claim, and widen payloads use
`authorization.kind = "existing_answers_cover_every_overlap"` for
`ConflictAuthorization::ExistingAnswersCoverEveryOverlap`. Board JSON uses
`recorded_overlap_answers.entries[].answer =
"existing_answers_cover_every_overlap"` for
`RecordedAnswer::ExistingAnswersCoverEveryOverlap`.

## Named reservation lifecycle

`cargo-berth board --reservation <reservation-id> --json` reads one retained
reservation independently of complete-board placement. A known reservation
returns `verb = "board"`, `status = "board_ready"`, `exit_code = 0`, and
`payload.kind = "reservation"`. The envelope's `reservations` array contains
only the requested id, `blocked_by` is empty, and the payload echoes the id:

```json
{
  "kind": "reservation",
  "data": {
    "reservation_id": "01a036fb-1629-7712-96b7-1672b64a151f",
    "lifecycle": {
      "status": "outstanding",
      "protected_tip": "1111111111111111111111111111111111111111"
    }
  },
  "alerts": []
}
```

`lifecycle` is exactly one of these tagged alternatives:

- `status = "active"`, with no additional fields;
- `status = "outstanding"`, with `protected_tip`;
- `status = "released_after_checkpoint"`, with `protected_tip` and
  `disposition`;
- `status = "released_without_checkpoint"`, with `disposition`.

The disposition is the same tagged value used by board rows: `kind =
"integrated"`; or `rewritten_integration`, `abandoned`, or `retired_orphan`
with `evidence`. The lifecycle payload deliberately omits current integration
evidence. A waiting successor and either endpoint of an unresolved overlap can
therefore be selected even though the complete board omits their reservation
rows. The selector has no terminal representation and requires `--json`.

An unknown id returns `status = "invalid_input"`, `exit_code = 5`, and the same
envelope reservation-id rules. Its typed payload is:

```json
{
  "kind": "reservation",
  "data": {
    "status": "unknown_reservation",
    "reservation_id": "01a036fb-1629-7712-96b7-1672b64a15ff"
  },
  "alerts": []
}
```

Adding this payload kind does not change the serialized bytes of plain
`cargo-berth board --json`; that command continues to return `payload.kind =
"board"` and the complete board object documented above.

## Coordination identity rejections

Claim, check, drift, and sequence return a shared identity rejection with
`status = "invalid_input"`, `exit_code = 5`, and
`payload.kind = "coordination_identity"`. Integration retains its verb payload:
`payload.kind = "integrate"`, `payload.data.status = "rejected"`, and the same
rejection object at `payload.data.reason`.

The rejection object is one of these tagged alternatives:

- `kind = "stale_session_mapping"` carries `coordination_run_id`,
  `reservation_id`, and `recovery_actions`.
- `kind = "stale_marker_run"` carries `coordination_run_id`,
  `issuing_worktree_id`, `issuing_root`, and `recovery_actions`.
- `kind = "session_worktree_mismatch"` carries `coordination_run_id`,
  `reservation_id`, `holding_worktree_id`, `issuing_worktree_id`,
  `holding_root`, `issuing_root`, and `recovery_actions`.

`recovery_actions` is always a non-empty array. Every action contains a
non-empty `argv` array holding the complete executable and arguments, plus a
canonical absolute `cwd`. Every published `argv` is directly executable without
argument substitution. An action whose complete command cannot be represented
as text is omitted instead of being published in a degraded form. The action
alternatives are:

- `clear_session_mapping`, whose command is
  `["cargo-berth", "identity", "clear-session", "--json"]`;
- `reconcile_and_sweep_marker`, whose command is
  `["cargo-berth", "board", "--json"]`;
- `rerun_from_holding_worktree`, whose command is the original process argv and
  whose `cwd` is `holding_root`;
- `claim_separately_here`, whose command is
  `["cargo-berth", "identity", "clear-session", "--json"]` and whose `cwd` is
  `issuing_root`. After it succeeds, the caller starts a separate harness
  session, claims work in that checkout, and reruns the rejected command.

A stale session mapping has only `clear_session_mapping`; a stale marker has
only `reconcile_and_sweep_marker`; and a worktree mismatch has both
`rerun_from_holding_worktree` and `claim_separately_here`, in that order. A
consumer executes the supplied argv in the supplied cwd without adding flags or
paths. A managed reference-transaction hook cannot replay Git's stdin-backed
private command. Its mismatch response therefore supplies only
`clear_session_mapping`; after the repair succeeds, the user retries the
original Git command. If the original process argv contains an argument that is
not text, `rerun_from_holding_worktree` is omitted and the mismatch response
supplies only the always-runnable `claim_separately_here` action.

`cargo-berth identity clear-session --json` returns `verb = "identity"` and
`payload.kind = "identity"`. A removed or already-absent mapping returns
`status = "session_mapping_cleared"`, `exit_code = 0`, and
`payload.data.status = "session_mapping_removed"` or
`"session_mapping_already_absent"`. When no usable `CARGO_BERTH_SESSION_ID` is
available, the repair did not run: the response has
`status = "session_mapping_unavailable"`, `exit_code = 5`, and
`payload.data.status = "current_session_unavailable"`. The command removes only
the mapping selected by the current `CARGO_BERTH_SESSION_ID`; it does not remove
other session mappings or alter reservation or journal state.

## Drift outcomes

A drift response carries `payload.kind = "drift"`. Each
`payload.data.results[]` entry uses `status = "unchanged"`, `changed`, or
`phase_start_object_unknown`. The unreadable-phase-start form carries
`reservation_id` and `phase_start`, and means Git could not read the baseline
required for that reservation's full comparison. It is a blocking result: when
no higher-priority drift status is present, the envelope has
`status = "object_unknown"` and `exit_code = 1`. It is never represented as an
empty committed-path set, does not append a drift consequence to the journal,
and does not publish a fingerprint cache entry.

## Resolve incursion outcomes

A one-incident `resolve` response carries `payload.kind = "resolve"`. Its
`payload.data.status` reports whether this invocation appended the disposition
or found a durable disposition from a coordination actor:

- `recorded_now` carries `reservation_id` and `incident_id`. The envelope has
  `status = "incursion_resolved"` and `exit_code = 0`.
- `already_recorded_by_same_coordination_actor` carries `reservation_id` and
  `incident_id`. It means the resolution event's recorded worktree and
  coordination-run ids equal the invoking actor's resolved ids. The envelope
  has `status = "incursion_resolved"` and `exit_code = 0`.
- `already_recorded_by_different_coordination_actor` carries `reservation_id`,
  `incident_id`, `resolving_worktree_id`,
  `resolving_coordination_run_id`, `resolution_event_id`, and `resolved_at`.
  The envelope has `status = "invalid_input"` and `exit_code = 5`.

The actor comparison describes only the ids recorded in the journal. It does
not assert that two calls were the same process or that a historical actor was
physically attributed to the correct worktree. A same-actor repeat does not
append another journal event.

The historical `incursion_resolved` payload alternative remains decodable for
wire compatibility. `every_incursion_resolved` remains the successful payload
for `resolve --every-incursion` and carries `reservation_id` plus
`incident_ids`.

## The journal record

`.git/cargo-berth/journal.ndjson` contains one complete JSON object per line.
Every record has this envelope:

- `schema_version`: the integer `2` on every new record. Records carrying the
  integer `1` are historical and still accepted; `1` is the oldest version this
  binary decodes.
- `event_id`: the record's UUID-v7 string.
- `actor`: `{ "repository": <uuid-v7>, "worktree": <uuid-v7>, "run":
  <uuid-v7> }`.
- `identity_inputs`: the process inputs captured for actor diagnosis. Historical
  records written before this field omit it. Every new mutation writes the
  `recorded` form described below.
- `at`: an RFC 3339 UTC string with millisecond precision.
- `projection_generation`: the integer generation published by this append.
- `op`: the operation discriminator. Operation fields are flattened into the
  same object as the envelope; there is no nested operation object.

Reservation, event, incident, edge, permit, repository, worktree, and
coordination-run ids are UUID-v7 strings. Pending-marker ids and bypassed-merge
identities are opaque strings. Git object ids are full lowercase SHA-1 or
SHA-256 hex strings. A `scope` is `{ "path": <repository-relative string>,
"kind": "file" | "tree" }`; fields named `scopes`, `added_scopes`,
`scope_revision`, and overlap `scopes` are arrays of that object.

`identity_inputs` has this form on every new journal mutation:

```json
{
  "status": "recorded",
  "invocation_directory": {
    "status": "utf8",
    "path": "/Users/example/rust/cargo-tile-favorites"
  },
  "cargo_berth_session_id": { "status": "utf8", "value": "session-4134" },
  "cargo_berth_run": { "status": "unset" },
  "git_dir": { "status": "utf8", "value": ".git/worktrees/favorites" },
  "git_common_dir": { "status": "utf8", "value": ".git" }
}
```

Each environment field is `unset`, `utf8` with its exact raw `value`,
`too_long` with `observed_bytes`, or `non_utf8`. `invocation_directory` is
`utf8` with `path`, `too_long` with `observed_bytes`, `non_utf8`, or
`unavailable` with `diagnostic`. A `utf8` path or value is retained only when
its JSON-encoded string contents are at most 256 bytes; `observed_bytes` is the
raw UTF-8 byte length before JSON escaping. Actor resolution canonicalizes the
invocation directory and follows its `.git` filesystem metadata. A relative
`gitdir:` locator is relative to the worktree root; a relative `commondir`
locator is relative to the per-worktree administrative directory. Supplied
`GIT_DIR` and `GIT_COMMON_DIR` values do not override that actor resolution,
including when they are relative; they are recorded as process inputs for
diagnosis.

The v1 operation union is:

| `op` | Operation fields |
| --- | --- |
| `claim` | `reservation_id`, `scopes`, `source`, `purpose`, `trunk_at_claim`, `head_snapshot`, `phase_start_head`, `worktree_root`, `worktree_administrative_locator`, `authorization` |
| `widen` | `reservation_id`, `added_scopes`, `cause`, `authorization`, `edit_blocking_status` |
| `checkpoint` | `reservation_id`, `protected_tip`, `trunk_snapshot` |
| `resnapshot` | `reservation_id`, `snapshot` |
| `renew` | `reservation_id` |
| `release` | `reservation_id`, `disposition` |
| `replace_release_disposition` | `reservation_id`, `superseded`, `replacement` |
| `evidence_revalidated` | `reservation_id`, `status`, `edit_blocking_status` |
| `scoped_patch_equivalence_checked` | `reservation_id`, `subject`, `target`, `verdict` |
| `scoped_patch_comparison_attempted` | `reservation_id`, `subject`, `target` |
| `successor_scoped_patch_equivalence_checked` | `predecessor_reservation_id`, `subject`, `successor_head`, `verdict` |
| `successor_scoped_patch_comparison_attempted` | `predecessor_reservation_id`, `subject`, `successor_head` |
| `resolve_defer` | `deferred_reservation_id`, `blocker_reservation_id`, `edge_id`, `direction`, `reason` |
| `incursion` | `incident_id`, `reservation_id`, `foreign_reservation_ids`, `paths` |
| `resolve_incursion` | `incident_id` |
| `forced_integration_permit` | `permit_id`, `reservation_id`, `reason`, `skipped_holds` |
| `consume_forced_integration_permit` | `permit_id`, `reservation_id` |
| `bypass` | `action`, `cause`, `occurrence_time`, `recording` |
| `rebind_worktree` | `reservation_id`, `previous_worktree_id`, `current_worktree_id`, `current_worktree_root`, `current_worktree_administrative_locator` |
| `relocate_worktree` | `reservation_id`, `worktree_id`, `previous_root`, `current_root` |

These operation fields use the following tagged values:

- `source` is `{ "kind": "explicit" }` or `{ "kind": "work_plan",
  "plan": <string>, "phase": <string> }`.
- `purpose` is `{ "kind": "not_provided_by_caller" }` or `{ "kind":
  "explained", "explanation": <non-empty string> }`.
- `head_snapshot` is `{ "kind": "branch", "full_ref": <refs/... string>,
  "head": <oid> }` or `{ "kind": "detached", "head": <oid> }`.
- `trunk_at_claim` is untagged: either a bare git object id string, meaning the
  configured trunk resolved to that commit, or `{ "reference": <refs/... string>
  }`, meaning the configured trunk reference existed but resolved to no commit.
  Records written before the object form was widened are bare oid strings and
  decode unchanged. An unresolved trunk reference is a recorded observation, not
  a corrupt journal: replay accepts it and later commands answer from the
  reservation's lifecycle instead of reporting the ledger unreadable.
- `authorization.kind` is `no_conflict`; `sequence` with `overlaps`, `blocker`,
  `direction`, `edge_id`, and `reason`; `defer` or `override` with `overlaps`,
  `blocker`, and `reason`; or `existing_answers_cover_every_overlap` with
  `overlaps`. `direction` is `requester_before_holder` or
  `holder_before_requester`. Each `overlaps` entry is `{ "reservation_id":
  <uuid-v7>, "scope_revision": [scope...], "scopes": [scope...] }`.
- Widen `cause.kind` is `drift` or `explicit`; `explicit` adds `reason`.
- `edit_blocking_status` is `blocking` or `clear`. The field remains in v1
  `widen` and `evidence_revalidated` records for compatibility and audit, but
  replay treats it as informational. Effective blocking is derived from the
  replayed lifecycle and integration evidence, and a released reservation is
  always effectively `clear` even if a historical record says `blocking`.
- `snapshot.stage` is `active` with `claim_snapshot`, or `outstanding` with
  `protected_tip` and `trunk_oid`.
  A resnapshot can update only an active claim snapshot or an outstanding
  protected tip. Legacy records that resnapshot an already released
  reservation are consumed without reopening its terminal lifecycle.
- A release disposition is `{ "kind": "integrated" }`, or has `kind` equal to
  `rewritten_integration`, `abandoned`, or `retired_orphan` plus a scalar
  `evidence` field containing the commit or reason. `superseded` and
  `replacement` use the same format.
- Integration evidence `status.status` is `not_integrated`; `integrated` with
  `trunk_oid` and `proof`; `trunk_rewritten`; or `object_unknown`. Integrated
  `proof` is `protected_tip_ancestor` or `scoped_patch_equivalent`. Records
  written before `proof` was added decode as `protected_tip_ancestor`.
- `scoped_patch_equivalence_checked` records the durable content cache. Its
  positive integer `subject` identifies the reservation's current baseline,
  protected content, and scopes; `target` is the checked trunk object id; and
  `verdict` is `integrated`, `not_integrated`, or `trunk_rewritten`. The subject
  starts at `1` and
  advances whenever an input to the scoped comparison changes: its baseline,
  protected or release-revalidation tip, or scopes. The advancing operations are
  `widen`; `resnapshot` for both active baselines and outstanding protected tips;
  an initial `release` whose disposition is `rewritten_integration`; and
  `replace_release_disposition`. Replay treats a reservation with no applicable
  record as `unchecked`. Both negative verdicts retain the immutable `Different`
  comparison; reconciliation maps that comparison through the reservation's
  current integration context. Git failures reported as `object_unknown` are
  transient and never produce this operation.
- `scoped_patch_comparison_attempted` records scheduling state for a comparison
  that produced transient `object_unknown`. Its `subject` and `target` use the same identities as
  `scoped_patch_equivalence_checked`. Reconciliation runs the least-recently
  attempted uncached subject first at each target, so every subject receives a
  comparison while the transient failure remains eligible for later retries.
- `successor_scoped_patch_equivalence_checked` records the separate durable
  successor-incorporation cache. `predecessor_reservation_id` owns the protected
  content, `subject` identifies that predecessor's current baseline, content, and
  scopes, and `successor_head` is the immutable target checked by git. `verdict`
  is `equivalent` or `different`; both outcomes are cached. Entries are invalidated
  by a predecessor subject revision and retained under a bounded successor-target
  limit independent of the trunk-target cache.
- `successor_scoped_patch_comparison_attempted` records a successor comparison
  that produced transient `object_unknown`, which is never cached. One shared
  fixed budget admits one cold successor comparison per reconciliation. Pending
  heads are ordered by their persisted attempt generation, and every transient
  attempt records a new generation so an unavailable head rotates behind other
  pending heads instead of starving them. A deferred head remains not incorporated.
- Incursion `foreign_reservation_ids` and `paths` are non-empty arrays of
  reservation-id strings and repository-relative path strings, respectively.
- `skipped_holds.kind` is `ordering_edges` with non-empty `edges`; `deferrals`
  with non-empty `deferrals`; or `ordering_edges_and_deferrals` with both.
  An edge is `{ "edge_id": <uuid-v7>, "predecessor": <uuid-v7> }`; a
  deferral is `{ "declaration_event_id": <uuid-v7>, "deferred": <uuid-v7>,
  "blocker": <uuid-v7> }`.
- Bypass `action` is `integration` or `editing`. `cause.kind` is
  `environment_override` with `bypassed_merge`, or `forced_integration` with
  `permit_id` and `reason`. `occurrence_time.status` is `event_recorded_at`,
  `known` with `at`, or `unavailable`. `recording.kind` is `direct` or
  `pending_marker` with `marker_id`.

An unknown `schema_version` or `op`, an omitted field required for that record,
an empty field whose type is documented as non-empty, or an invalid tagged
alternative makes the journal unreadable. The only backward-compatible envelope
omission is `identity_inputs` on records written before identity instrumentation;
an older binary never skips an operation it cannot replay.
