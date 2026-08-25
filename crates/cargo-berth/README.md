# cargo-berth

`cargo-berth` coordinates path ownership and merge order between Git worktrees.
It keeps an append-only journal in the repository's common Git directory, so
every worktree sees the same reservations.

It does not choose an order for you. It records explicit answers, checks them
before integration, and shows the resulting state on a board.

## First use

Install the binary, initialize one repository, reserve your paths, inspect the
board, complete and commit the work, checkpoint it, update trunk, and retire the
reservation:

```console
$ cargo install cargo-berth
$ cargo berth init
$ cargo berth claim crates/parser src/main.rs --run 01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1b --why "update the parser"
$ cargo berth board
$ cargo berth release <reservation-id>
$ cargo berth integrate <reservation-id>
$ cargo berth release <reservation-id>
$ cargo berth release <reservation-id>
```

`cargo berth ...` and `cargo-berth ...` invoke the same executable. `init`
creates `.git/cargo-berth/journal.ndjson`, writes
`.claude/config/berth.toml`, and installs two managed hooks when their names are
available: a `reference-transaction` trunk gate and a non-blocking
`post-commit` drift warning. `integrate` and `release` do not combine lifecycle
transitions: `release` on an active reservation checkpoints the current commit,
while `integrate` updates configured trunk to the current worktree `HEAD`. The
first `release` after that update can append the
new integration evidence and leave the reservation outstanding. Run `release`
again to record the terminal `integrated` disposition. If another stateful read
already materialized that evidence, the first post-integration `release` records
the disposition instead.

This transcript came from the installed `cargo-berth 0.1.0-dev` in a scratch
repository. The protected work was committed between `board` and the first
`release`:

```console
$ cargo berth init
Initialized the cargo-berth ledger.
$ cargo berth claim crates/parser src/main.rs --run 01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1b --why "update the parser"
Claimed 2 reservation scope(s) as 01a0371f-ff36-75a3-ad13-b1dee3820f97.
$ cargo berth board
The reservation board was read. Use `cargo-berth board --json` to inspect it.
$ cargo berth release 01a0371f-ff36-75a3-ad13-b1dee3820f97
Reservation 01a0371f-ff36-75a3-ad13-b1dee3820f97 is outstanding at protected tip 292471ef2254a985665228c46355571f54e4148a.
$ cargo berth integrate 01a0371f-ff36-75a3-ad13-b1dee3820f97
Integrated reservation 01a0371f-ff36-75a3-ad13-b1dee3820f97; the ordering gate was clear.
$ cargo berth release 01a0371f-ff36-75a3-ad13-b1dee3820f97
Reservation 01a0371f-ff36-75a3-ad13-b1dee3820f97 has integration evidence in trunk commit 292471ef2254a985665228c46355571f54e4148a.
$ cargo berth release 01a0371f-ff36-75a3-ad13-b1dee3820f97
Reservation 01a0371f-ff36-75a3-ad13-b1dee3820f97 recorded disposition Integrated.
$ cargo berth board --json | jq -c '{status, lifecycle: .payload.data.resolved.entries[0].lifecycle}'
{"status":"board_ready","lifecycle":{"stage":"released","disposition":{"kind":"integrated"}}}
```

The full verb set is:

- `init`: initialize the ledger and hooks, repair the projection, or perform a
  confirmed reinitialization.
- `claim`: reserve paths and, when needed, propose one answer for an overlap.
- `check`: ask whether proposed file paths collide with a foreign reservation.
- `drift`: compare changed paths with reservation scopes.
- `board`: inspect current constraints, answers, incidents, and audit history.
- `sequence`: turn a deferred overlap into a directed ordering edge.
- `integrate`: update configured trunk to the current worktree `HEAD`;
  `--force --why <text>` records an explicit permit when ordering or deferral
  holds remain.
- `release`: checkpoint an active reservation, materialize current integration
  evidence for an outstanding one, or record its verified terminal disposition.
- `resolve`: answer an incursion or record an explicit recovery disposition.
- `renew`: refresh reservation activity without changing scopes or edges.

Run `cargo berth <verb> --help` for every argument and flag. The help surface is
kept in step with this guide.

## Enforcement boundaries

The trunk gate is a Git `reference-transaction` hook. Once installed and set to
enforce, it rejects a trunk update whose reservation order has not been
satisfied. A caller does not need an editor integration for that protection.
The shipped configuration uses `gate_mode = "observe"`: violations are reported
and the update is permitted. Set `gate_mode = "enforce"` to reject them.

Editing has a different boundary. `cargo-berth` does not block a keystroke. A
Claude Code `PreToolUse` hook can call `cargo-berth check` before a write, but
that harness hook is not installed by this crate. A general Git user instead
gets the managed `post-commit` warning. It runs after the commit already exists,
never rejects the commit, names the paths that strayed and the foreign holders
they reached, and leaves the decision with the user. Run `cargo berth drift` for
the same check on demand. `CARGO_BERTH_BYPASS=1` skips the post-commit check.

Permissive overlap answers also have a stated limit. An answer takes two
deliberate invocations: the first returns a scoped proposal at exit 3, and a
second invocation applies that exact token. The resulting fact records the
submitting repository, worktree, and coordination run, the reason, and the exact
overlap. It guarantees that the answer was deliberate, reasoned, limited to the
conflict shown, attributed to the submitting coordination identity, and visible
on the board. It does not identify a person or prove that a human supplied the
answer. A published binary has nowhere to send an escalation that its caller
cannot also read. The invoking harness is responsible for enforcing a
human-in-the-loop rule.

The gate has four intentional permit paths:

- Observe mode reports a violation and permits it. Choose `gate_mode =
  "enforce"` when rejection is required.
- An unreadable `.claude/config/berth.toml` permits the update and explains why.
  Restore the configuration file.
- A missing or unstartable `cargo-berth` executable permits the update and
  explains why. Restore the executable and rerun `cargo berth init`.
- An unrelated `reference-transaction` hook is preserved, not replaced. `init`
  succeeds, but that gate is inactive. Its text message and JSON `hooks` payload
  report the inactive hook. Move the existing hook aside first, run `cargo berth
  init` so it can install the managed hook, then combine the saved hook with the
  installed managed hook in a wrapper. Do not rerun `init` after installing the
  wrapper: a wrapper without the managed marker is preserved and reported
  inactive, while a wrapper containing the marker is replaced by the managed
  hook.

For example, the last case reports:

```text
Initialized the cargo-berth ledger. Hook 'reference-transaction' is occupied by an unmanaged hook, so cargo-berth protection for that hook is not active. Incorporate the existing hook in a wrapper or move it aside, then rerun cargo berth init.
```

Journal loss follows a stricter rule. `integrate` and the trunk gate fail closed
on an absent, corrupt, or unknown-epoch journal because losing the journal can
erase an approved merge order. An unreadable configuration file instead means
the gate cannot determine repository policy, so it permits and explains. These
are different inputs: coordination facts are retained conservatively, while an
unavailable policy file never silently selects enforce mode.

An unresolvable trunk or protected tip produces
`IntegrationEvidenceStatus::ObjectUnknown`. It creates a holding violation and
keeps edit checks blocked. Under the shipped `observe` mode, `integrate` reports
that violation and still updates trunk; `enforce` mode rejects the update. The
violation never expires. Restore the missing Git object and let reconciliation
revalidate the evidence.

## Claims and collisions

A bare path passed to `claim` reserves that path and its whole subtree. A bare
path passed to `check` asks about exactly one file. Prefix either command's
argument with `file:` or `tree:` to override its default. Overlap follows path
components, so `crates/foo` and `crates/foobar` do not overlap. A path need not
exist before it is claimed. Comparisons ignore case when Git reports
`core.ignoreCase=true` for the repository.

Repository manifests such as `Cargo.toml`, `Cargo.lock`, and individual files
under `.claude/config` use ordinary exact exclusive claims. Paths touched only
for verification do not need claims.

Here is an actual collision from two linked worktrees. Branch `holder` owns
`tree:crates/shared`; branch `requester` asks for the same tree. The UUIDs and
proposal token below are the binary's output from that repository.

```console
requester$ cargo berth claim tree:crates/shared --run 01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1c --plan docs/work.md --phase requester --why "consume the shared API"
Reservation 01a0370c-860c-7d90-bd62-3497dd7063aa on refs/heads/holder (plan docs/work.md, phase holder, update the shared API) holds overlapping paths for 01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1b; reduce the requested scopes or coordinate with the holder, then retry.
[exit 1]

requester$ cargo berth claim tree:crates/shared --run 01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1c --plan docs/work.md --phase requester --why "consume the shared API" --after 01a0370c-860c-7d90-bd62-3497dd7063aa --overlap-why "the holder API must land first"
User authorization is required before this overlap can be recorded: editing proceeds on the shown scopes and integration enforces the selected order. Review every holder, shared scope, plan, phase, direction, and reason in the payload, then rerun this claim with --proposal '{"requester":{"coordination_identity":{"status":"presented","coordination_run_id":"01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1c"},"worktree_id":"01a0370c-8679-7211-825a-c5ab5ea91162","source":{"kind":"work_plan","plan":"docs/work.md","phase":"requester"},"purpose":{"kind":"explained","explanation":"consume the shared API"}},"authorization_reason":"the holder API must land first","candidate_scopes":[{"path":"crates/shared","kind":"tree"}],"answer":{"kind":"sequence","blocker":"01a0370c-860c-7d90-bd62-3497dd7063aa","direction":"holder_before_requester"},"overlaps":[{"reservation_id":"01a0370c-860c-7d90-bd62-3497dd7063aa","scope_revision":[{"path":"crates/shared","kind":"tree"}],"scopes":[{"path":"crates/shared","kind":"tree"}]}]}'.
Holder 01a0370c-860c-7d90-bd62-3497dd7063aa: plan docs/work.md, phase holder; shared scopes: tree:crates/shared; direction: holder 01a0370c-860c-7d90-bd62-3497dd7063aa before requester; reason: the holder API must land first; consequence: editing proceeds on the shown scopes and integration enforces the selected order.
[exit 3]

requester$ cargo berth claim tree:crates/shared --run 01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1c --plan docs/work.md --phase requester --why "consume the shared API" --after 01a0370c-860c-7d90-bd62-3497dd7063aa --overlap-why "the holder API must land first" --proposal '{"requester":{"coordination_identity":{"status":"presented","coordination_run_id":"01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1c"},"worktree_id":"01a0370c-8679-7211-825a-c5ab5ea91162","source":{"kind":"work_plan","plan":"docs/work.md","phase":"requester"},"purpose":{"kind":"explained","explanation":"consume the shared API"}},"authorization_reason":"the holder API must land first","candidate_scopes":[{"path":"crates/shared","kind":"tree"}],"answer":{"kind":"sequence","blocker":"01a0370c-860c-7d90-bd62-3497dd7063aa","direction":"holder_before_requester"},"overlaps":[{"reservation_id":"01a0370c-860c-7d90-bd62-3497dd7063aa","scope_revision":[{"path":"crates/shared","kind":"tree"}],"scopes":[{"path":"crates/shared","kind":"tree"}]}]}'
Claimed 1 reservation scope(s) as 01a0370c-890c-7312-969f-0af2605dfc82.
[exit 0]
```

The four answers are:

- `--before <holder>`: the requester integrates before the holder.
- `--after <holder>`: the requester integrates after the holder.
- `--defer <holder>`: neither may integrate until a later `sequence` command
  supplies an order.
- `--override <holder>`: editing is authorized without an integration order.

Each answer requires `--overlap-why <text>`. The exit-3 proposal changes no
state; only a separate invocation carrying the byte-for-byte token can append
the answer.

## Drift and the post-commit warning

`cargo berth drift` defaults to the cheap comparison. It compares the current
tracked-status and untracked-path fingerprint with the preceding observation,
answering “which path memberships changed since the last check?” It normally
runs two Git queries. If no usable fingerprint exists, it falls back to the full
comparison.

`cargo berth drift --full` compares committed, staged, unstaged, and untracked
paths with the reservation's protected phase-start commit. It answers “what has
changed since this reservation began?” It runs four Git queries for one
reservation and one extra committed-diff query for each additional reservation.
Both modes reconcile first and acquire the mutation lock; the cheap fingerprint
alone is not the complete command cost.

On a small scratch repository, five measured complete default calls took 0.12,
0.09, 0.10, 0.11, and 0.10 seconds. Those measurements include process startup,
reconciliation, lock acquisition, and comparison. A 0.20-second upper bound is
the integration budget derived from that measurement; repository size and Git
performance can increase it.

Use `--reservation <id>` to select a reservation. Without it, `drift` first uses
the harness session mapping, then accepts an implicit selection only when one
active reservation matches. Ambiguity is a usage error that names every
candidate:

```text
drift is ambiguous; choose one active reservation with --reservation: 01a036fd-c494-73c3-8999-9682008496f1, 01a036fd-c539-76f2-9121-2b4d79bf3075
[exit 5]
```

A result can have three consequences:

- An auto-widen appends scopes to the selected reservation.
- An incursion records that a local reservation entered paths held by a foreign
  reservation. The incident has a durable id and remains outstanding until
  `cargo berth resolve <reservation-id> --incursion <incident-id>` records its
  disposition.
- A collision refuses to widen. It reports the foreign holders and changes no
  reservation.

The `post-commit` hook examines every active reservation held by the invoking
worktree for incursions and collisions, without mutating those results. It can
auto-widen exactly one reservation: the session-mapped reservation, an explicit
`--reservation`, or the sole active candidate. If it cannot identify one, it
widens nothing and names `drift --reservation <id>`:

```text
Widened reservation 01a036fd-c84d-72c2-a2a6-44e0e7148f26 to cover file:outside.txt.
```

That is the single-reservation outcome. A two-reservation worktree reports
every incursion, leaves each scope unchanged when it cannot select a widening
subject, and points at the explicit command:

```text
cargo-berth could not complete the post-commit drift check. Changed paths outside.txt were not widened because attribution is ambiguous among reservations 01a036fd-c494-73c3-8999-9682008496f1, 01a036fd-c539-76f2-9121-2b4d79bf3075. Run drift --reservation <id> with one listed reservation. Run `cargo-berth drift --full` by hand; this commit remains in place.
```

That is the intended non-mutating outcome: the commit exists, no reservation
changed, and the paths can be attributed by hand. An unidentified coordination
run is reported only by the command, because there is no identity to journal:

```text
Changed paths outside.txt were not widened because no coordination run was identified. Set CARGO_BERTH_RUN to the run that owns the target reservation, then run drift --reservation <id>.
Incursion 01a036fe-2b98-7e11-8ebb-af53d83640bf: reservation 01a036fd-cbbc-7ce3-8a72-6f1ee19c4693 entered shared/entered.txt held by foreign reservation(s) 01a036fd-cd09-7c10-b121-354321926b1b. Stop and resolve the overlap with `resolve 01a036fd-cbbc-7ce3-8a72-6f1ee19c4693 --incursion 01a036fe-2b98-7e11-8ebb-af53d83640bf` before making more changes.
Incursion 01a036fe-2b98-7e11-8ebb-af664b883b7c: reservation 01a036fd-cc5e-7b12-bcf8-e4f18ca90755 entered shared/entered.txt held by foreign reservation(s) 01a036fd-cd09-7c10-b121-354321926b1b. Stop and resolve the overlap with `resolve 01a036fd-cc5e-7b12-bcf8-e4f18ca90755 --incursion 01a036fe-2b98-7e11-8ebb-af664b883b7c` before making more changes.
```

`CARGO_BERTH_BYPASS=1` skips the whole post-commit check.

## Harness identity

`session-identities.json` sits beside the journal. A harness supplies
`CARGO_BERTH_SESSION_ID`; the file maps that key to one coordination run and one
active reservation. A later claim in the same harness session replaces the
earlier reservation mapping. The mapping is best-effort auxiliary state, not a
journal-rebuildable projection: the harness session id comes only from the
environment of the process applying a new event and is absent from journal
records. If the mapping is deleted or corrupt, use `CARGO_BERTH_RUN` and an
explicit reservation id until a later claim under that harness session writes a
new mapping.

Edit authorization resolves in this exact order:

1. `CARGO_BERTH_SESSION_ID` through `session-identities.json`.
2. The explicit `CARGO_BERTH_RUN` environment override.
3. The worktree's `cargo-berth-run-id` marker file.
4. An unidentified result.

`CARGO_BERTH_RUN` therefore outranks the marker, but not a valid session
mapping. A successful command can still report that its mapping was not
published:

```text
Claimed 1 reservation scope(s) as 01a036fe-9e14-74b2-9985-be69adb82532, but the harness session mapping could not be published: session identity mapping I/O failed: Is a directory (os error 21). Later session-keyed drift checks may require an explicit coordination run and reservation.
```

The claim is durable. Supply its run and reservation explicitly afterwards. A
mapping that points at a reservation no longer active gets a different error:

```text
harness session mapping for coordination run 01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1b no longer names an active reservation in this worktree
[exit 5]
```

Remove or repair the stale session mapping, set `CARGO_BERTH_RUN` to the named
run, and pass `--reservation <id>` where the verb accepts it. Recovery is
explicit; the command never retries under a different identity silently.

## Board

`board` reconciles the journal with Git before rendering. It has three output
modes:

- Bare `cargo berth board` opens a full-screen view only when both standard
  input and standard output are terminals. A normal quit restores the terminal,
  exits 0, and prints nothing afterwards.
- Bare `cargo berth board` with either stream redirected prints one pointer line
  plus any recovered-bypass notices. It does not print board rows. Redirected
  standard input therefore selects this mode even when standard output remains
  attached to a terminal.
- `cargo berth board --json` is the only mode that emits board facts. Use it for
  every script.

The terminal panes are **Overview**, **Reservations**, **Integration
constraints**, **Answers and bypasses**, **Incursions**, and **Alerts**. Press
`q` to quit, `Tab` or `Shift-Tab` to move between panes, arrows to scroll or pan,
`Home`/`End` to jump, and `PageUp`/`PageDown` to page.

If the terminal fails before its first frame, the command exits 0 and its
envelope still carries the board with a terminal diagnostic; rerun with
`--json`. If it fails after a frame was visible, it prints no facts and exits 7;
the user already saw the board, and should rerun with `--json`. Text mode has no
status field, so scripts must use the exit code and JSON status.

The redirected form was observed as:

```text
The reservation board was read. Use `cargo-berth board --json` to inspect it.
Recovered bypass marker cargo-berth-pending-bypass-redirect-example.json: a bypass recorded earlier while the journal was unwritable has now been filed in the journal.
```

The interactive run displayed the six panes and, after `q`, restored the
terminal and emitted no line. The corresponding `--json` run emitted the facts
below.

### JSON contract

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
  `integrated` with `trunk_oid`; `trunk_rewritten`; or `object_unknown`.
- `freshness.status`: `fresh` or `stale`, both with `last_activity_at`.
- `ahead_behind_main.status`: `counts` with `ahead` and `behind`; `unrelated`;
  or `unavailable`.

The remaining row enums are scalar strings:
`edit_blocking_status = blocking | clear`, `visibility = active_constraint |
reblocked_active_constraint | resolved_audit`, and `holder.liveness = live |
unavailable | orphan_candidate | orphaned | unknown`.

The sections mean:

- `ready_now`: non-resolved endpoints involved in any recorded ordering edge,
  including an edge now listed under `settled_ordering_constraints`, except a
  current waiting successor or either endpoint of an unresolved deferral. Each
  entry wraps the row with `relation = "unordered"`. Non-resolved rows are
  active, outstanding, or released rows whose evidence made them edit-blocking
  again.
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
  ordering edge or unresolved deferral. This can include `active`, `outstanding`,
  and reblocked `released` rows.
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
- `alerts`: orphan recovery evidence and its `recover` or
  `retire_or_abandon` action; stale reservations and their
  `resolution.action = "renew"`; or bypasses not yet recorded and an instruction
  for restoring the journal audit path. The orphan action names either
  `resolve --recovered` or the explicit retire/abandon flags; the stale action
  names the reservation for `renew`; the bypass alert names the recovery step.
- `git_cost`: exact Git-call counts used to build this board.

`integration_order` is `undeclared` or `constraints_recorded`.
`recovered_bypasses_this_invocation` is a
`RecoveredBypassesThisInvocation` list of pending-bypass marker ids. It is a
notice reported once by the read that imported the marker into the journal and
deleted it; the next board read returns an empty list. The corresponding
`bypass_audit` entry remains as durable history.

Two similar answer tags are intentionally separate parts of the frozen schema.
Journal, claim, and widen payloads use
`authorization.kind = "existing_answers_cover_every_overlap"` for
`ConflictAuthorization::ExistingAnswersCoverEveryOverlap`. Board JSON uses
`recorded_overlap_answers.entries[].answer =
"existing_answers_cover_every_overlap"` for
`RecordedAnswer::ExistingAnswersCoverEveryOverlap`.

### Journal contract

`.git/cargo-berth/journal.ndjson` contains one complete JSON object per line.
Every v1 record has this envelope:

- `schema_version`: the integer `1`.
- `event_id`: the record's UUID-v7 string.
- `actor`: `{ "repository": <uuid-v7>, "worktree": <uuid-v7>, "run":
  <uuid-v7> }`.
- `at`: an RFC 3339 UTC string with millisecond precision.
- `projection_generation`: the integer generation published by this append.
- `op`: the operation discriminator. Operation fields are flattened into the
  same object as the envelope; there is no nested operation object.

Reservation, event, incident, edge, permit, repository, worktree, and
coordination-run ids are UUID-v7 strings. Pending-marker ids and bypassed-merge
identities are opaque strings. Git object ids are full lowercase SHA-1 or
SHA-256 hex strings. A `scope` is `{ "path": <repository-relative string>,
"kind": "file" | "tree" }`; fields named `scopes`, `added_scopes`,
`scope_revision`, and overlap `scopes` are arrays of that object. The v1
operation union is:

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
- `authorization.kind` is `no_conflict`; `sequence` with `overlaps`, `blocker`,
  `direction`, `edge_id`, and `reason`; `defer` or `override` with `overlaps`,
  `blocker`, and `reason`; or `existing_answers_cover_every_overlap` with
  `overlaps`. `direction` is `requester_before_holder` or
  `holder_before_requester`. Each `overlaps` entry is `{ "reservation_id":
  <uuid-v7>, "scope_revision": [scope...], "scopes": [scope...] }`.
- Widen `cause.kind` is `drift` or `explicit`; `explicit` adds `reason`.
- `edit_blocking_status` is `blocking` or `clear`.
- `snapshot.stage` is `active` with `claim_snapshot`, or `outstanding` with
  `protected_tip` and `trunk_oid`.
- A release disposition is `{ "kind": "integrated" }`, or has `kind` equal to
  `rewritten_integration`, `abandoned`, or `retired_orphan` plus a scalar
  `evidence` field containing the commit or reason. `superseded` and
  `replacement` use the same format.
- Integration evidence `status.status` is `not_integrated`; `integrated` with
  `trunk_oid`; `trunk_rewritten`; or `object_unknown`.
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

An unknown `schema_version` or `op`, an omitted required field, an empty field
whose type is documented as non-empty, or an invalid tagged alternative makes
the journal unreadable; an older binary never skips an operation it cannot
replay.

## Configuration

`cargo berth init` writes `.claude/config/berth.toml` with these fields:

```toml
trunk = "main"
maximum_reservations = 128
maximum_ordering_edges = 512
gate_mode = "observe"
```

- `trunk`: the local branch whose update counts as integration.
- `maximum_reservations`: the maximum number of live reservations retained at
  once.
- `maximum_ordering_edges`: the maximum number of live ordering constraints.
- `gate_mode`: `observe` reports and permits; `enforce` reports and rejects.

Missing fields take these defaults. Unknown fields, duplicates, invalid values,
and an unreadable file are configuration errors.

## Bypass audit

`CARGO_BERTH_BYPASS=1` permits a trunk update no matter what other gate input is
broken. The override is tested before the tool reads the config, journal, or Git
evidence. The tool records an audit fact in the journal when writable, or writes
a pending marker for a later session to recover. The fact names
`CARGO_BERTH_BYPASS=1` and when it was taken. It has no reason and names no
specific hold that it skipped, because nothing else had been read yet. If
neither destination is writable, the tool warns and still permits the update.

## Recovery

`init` has three branches:

- Plain `cargo berth init` creates a missing ledger and default config and
  installs or refreshes each managed hook without replacing an unmanaged hook.
- `cargo berth init --repair-projection` rebuilds only `reservations.json` from
  journal truth. It changes no journal record and loses nothing.
- `cargo berth init --reinitialize-after-review` is the confirmed recovery for
  a corrupt journal. It replaces journal history and the projection after the
  user has reviewed the lost order. Reservations, ordering, answers, releases,
  incursions, and bypass audit facts in that journal are lost.

The projection-only branch reports:

```text
Rebuilt reservations.json from journal truth without changing the journal.
```

A confirmed reinitialization reports exactly how much journal material was
discarded and how many pending bypass markers remain reportable, for example:

```text
Reinitialized cargo-berth after confirmed order review; discarded 45 journal bytes across 1 complete record(s). 0 environment bypass marker(s) remain reportable.
```

A malformed v1 record looks like this and is the case for confirmed
reinitialization:

```text
The reservation ledger could not be read: journal replay failed: journal record 1 is corrupt: missing field `event_id` at line 1 column 44
[exit 4]
```

A record from a newer schema is different:

```text
The reservation ledger could not be read: journal replay failed: journal schema version 2 is unsupported
[exit 4]
```

Upgrade `cargo-berth` for an unsupported schema version. Never reinitialize that
journal merely because the current executable is older.

`resolve` records one of these explicit decisions:

- `--recovered` rebinds a reservation to the worktree running the command.
- `--integrated-as <trunk-oid>` records a verified alternate commit already
  reachable from trunk.
- `--abandon --why <text>` is the only deliberate abandonment route.
- `--retire-orphan --why <text>` is the only confirmed orphan-retirement route,
  and its disposition stays distinct from abandonment after replay.
- `--incursion <incident-id>` answers the named outstanding incursion for the
  positional reservation.

`renew <reservation-id>` refreshes freshness without changing scopes, ordering
edges, or lifecycle.

Every mutating verb appends its journal record before attempting its Git side
effects. A command can therefore say that a ref write or marker retirement
failed after the checkpoint or release was already durable. The work did
happen; rerun the command or let the next reconciliation repair the Git side
effect. Do not repeat the underlying work.

Exit 6 is the opposite: another mutation held the lock until the command's
ten-second wait was exhausted, so nothing was decided. Run the command again by
hand. Do not wrap it in another retry loop that multiplies the already-spent
wait.

## Process exit codes

The meanings in this table are the executable's public contract:

| Code | Meaning |
| ---: | --- |
| 0 | The command may proceed. |
| 1 | A reservation overlap blocks the command. |
| 2 | An unsatisfied ordering edge blocks the command. |
| 3 | The command needs user authorization. |
| 4 | The ledger cannot be read. Edit paths fail open; `integrate` fails closed. |
| 5 | The command line is invalid. |
| 6 | Another mutation holds the ledger lock; retry the command. |
| 7 | The board was handed a terminal and the terminal failed. |

Every JSON response has the six common envelope fields `verb`, `status`,
`exit_code`, `reservations`, `blocked_by`, and `message`, followed by the typed
`payload` field shown above.

## Deliberate omissions

`cargo-berth` does not choose merge order, read plans, track phases, parse work
orders, or coordinate paths across repositories. One ledger belongs to one Git
repository. A harness can supply plan and phase labels as provenance, but the
binary treats them only as text.
