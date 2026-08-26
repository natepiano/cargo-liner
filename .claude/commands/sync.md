---
description: Read the cargo-berth board, claim paths named directly, check phase drift, and operate reservation sequencing and integration without teaching the engine any plan format.
---

# Sync

`/sync` reads the cargo-berth board, claims paths named directly, checks phase
drift, and operates reservation sequencing and integration. `cargo-berth` never
reads Markdown, phases, or repository-specific structure.

Use the installed executable on `PATH` only. Every invocation goes through
`claim_state.py`, which sets `CARGO_BERTH_SESSION_ID` from
`CLAUDE_CODE_SESSION_ID`, validates the frozen JSON envelope, checks that its
status and exit code agree with the process exit code, and invokes the engine
once. Never build, install, or retry the binary here.

Set the module path for every command:

```sh
PYTHONPATH="$HOME/.claude/scripts" python3 -m berth.claim_state ...
```

`--cwd "$PWD"` names the invocation directory, which may be any directory inside
the repository. The coordinator resolves the repository top level with Git
before invoking the engine. A directory outside a Git repository is an error.

## Commands

### `/sync board`

Run exactly:

```sh
PYTHONPATH="$HOME/.claude/scripts" python3 -m berth.claim_state board --cwd "$PWD"
```

The coordinator invokes `cargo-berth board --json` exactly once. Never invoke
bare `board`: it can open the full-screen terminal or print only a pointer. Show
`state.rendered_markdown`, not a pass-through of the engine's prose. Its JSON
pointer rendering includes every envelope field, every field under
`payload.data`, every additive alert, and a separate list of every `action`,
`instruction`, `flag`, `flags`, `resolve_flag`, and `resolution` value. Do not
drop a field or user action.

`recovered_bypasses_this_invocation` is a one-read notice: show it on the read
that adopted the pending marker. It is empty on the next read, while the durable
entry remains under `bypass_audit`. Do not describe disappearance of the notice
as disappearance of the audit fact.

### `/sync claim <file:path|tree:path>...`

Name one or more repository-relative paths directly. `--plan <plan> --phase
<phase>` may be supplied together to retain Work Plan provenance; omit both for
an explicit source. A reservation over the paths a phase edits now comes into
being on first touch, so an explicit claim is for someone who wants to reserve
paths ahead of touching them.

```sh
PYTHONPATH="$HOME/.claude/scripts" python3 -m berth.claim_state claim \
  --cwd "$PWD" <file:path|tree:path>...
```

The command has one authorization state machine:

1. **Blocked.** A neutral claim returning exit `1` has
   `state.kind = blocked`. Show `state.rendered_markdown`; it is the shared
   refusal rendering used by the `PreToolUse` shim. It names every holder's
   reservation id, run id, branch or detached head, claim time, purpose, exact
   shared scopes, and activity as either active or gone quiet with its last
   activity time. It renders a `work_plan` source with plan and phase, an
   `explicit` source as an explicit claim with no plan or phase, and a
   `first_touch` source as a first-touch claim with no plan or phase. Never print
   an empty plan/phase field or turn `first_touch` into `explicit`.

   Read `state.scope_facts.kind`, never the presence of a sibling key. An
   `exact_requested_scopes` value carries the blocked check's scopes under
   `requested_scopes`; `holder_shared_scopes_only` means a blocked claim carries
   the requested-path facts through each holder's exact `overlapping_scopes`.

   Ask the user for exactly one answer for one holder. Answers 1 through 4
   require a non-empty reason and are flags on `claim`, never separate verbs:

   Read each `state.answers[].kind`, never the presence of a flag key. A
   `reasoned_claim` answer carries `selection`, the coordinator's spelling of
   that answer; `leave_alone` carries neither an engine action nor a reason
   requirement. The coordinator invocation below is the only way to submit an
   answer.

   1. **Land before the holder** — coordinator answer `--answer before
      --blocker <reservation-id> --overlap-reason "<the user's non-empty
      reason>"`. The requester takes the paths and integrates first; the holder is held
      until the requester is on trunk. Use when the holder will build on this
      requester's change.
   2. **Land after the holder** — coordinator answer `--answer after --blocker
      <reservation-id> --overlap-reason "<the user's non-empty reason>"`.
      The requester takes the paths and integrates second; it remains held until
      the holder's protected tip is on trunk and is an ancestor of the
      requester's `HEAD`. Use when the requester will build on the holder.
   3. **Defer the order** — coordinator answer `--answer defer --blocker
      <reservation-id> --overlap-reason "<the user's non-empty reason>"`.
      The requester takes the paths, no ordering edge is added, and the unresolved overlap
      stays visible on `cargo-berth board --json` until someone later sequences
      it.
   4. **Override** — coordinator answer `--answer override --blocker
      <reservation-id> --overlap-reason "<the user's non-empty reason>"`.
      The requester takes the paths, no ordering edge is added, and the override plus its
      reason stays visible on `cargo-berth board --json`.
   5. **Leave it alone.** Run no engine command, append nothing, and work
      elsewhere.

   Only `before` and `after` add an ordering edge. `defer` and `override` do not;
   the board record is the durable proof that the answer was recorded. The
   trunk-gate bypass is not an edit answer and cannot permit the blocked edit.

   An agent never chooses an answer, invents its reason, or answers its own
   block. If more than one blocker remains, require the user to narrow the
   requested scopes; a proposal binds exactly one named blocker.

2. **Proposal awaiting approval.** After the user supplies the answer, blocker,
   and reason, run one answered invocation, for example:

   ```sh
   PYTHONPATH="$HOME/.claude/scripts" python3 -m berth.claim_state claim \
     --cwd "$PWD" <file:path|tree:path>... \
     --answer after --blocker <reservation-id> \
     --overlap-reason "<the user's non-empty reason>"
   ```

   Exit `3` yields `state.kind = proposal_awaiting_approval`. Show
   `state.rendered_markdown`; it includes every holder fact from the refusal,
   the exact shared scopes, selected direction, reason, consequence, proposal,
   and transient token, but not the five-answer menu. A proposal binds exactly
   one displayed conflict, and its consequence must agree with its selected
   answer. Retain `proposal_token` only as transient conversation state and ask
   for approval of the answer already selected; do not ask for a different
   answer. Stop. Never mint and spend a token in the same turn.

3. **Claimed.** Only after a later, explicit approval, repeat the same answered
   invocation with the exact token:

   ```sh
   PYTHONPATH="$HOME/.claude/scripts" python3 -m berth.claim_state claim \
     --cwd "$PWD" <file:path|tree:path>... \
     --answer after --blocker <reservation-id> \
     --overlap-reason "<the same user-supplied reason>" \
     --proposal <exact-token>
   ```

   Exit `0` yields `state.kind = claimed`. Show `state.rendered_markdown` and
   return `reservation_id` explicitly.
   If `session_mapping_publication.status = unavailable`, this is degraded
   success: the journal and reservation are durable, so report its diagnostic,
   continue, and name the reservation id explicitly from then on.

A token-bearing exit `3` is stale: discard the old token, render the refreshed
proposal, and require a new explicit approval. Exit `1` means the named conflict
changed: return to **Blocked** with current facts. Exit `5` is malformed input,
not staleness; discard the token, show the structured invalid-input diagnostic,
and restart from an answered invocation without a token. Never apply a token in
the turn that selects an answer or writes a reason.

`/plan:delegate` uses first-touch claiming at its edit boundary and does not
reproduce proposal logic or read a second parser.

### `/sync check`

This answers “did anything stray outside what was claimed?” Run drift's full
phase-start comparison explicitly:

```sh
PYTHONPATH="$HOME/.claude/scripts" python3 -m berth.claim_state invoke \
  --cwd "$PWD" --expected-verb drift -- drift --full --json
```

`--full` is mandatory. The engine's fingerprint path issues three `git diff`
commands and one `git ls-files` command. Do not use drift's cheap default, whose
`status` plus `ls-files` comparison answers only whether anything changed since
the last observation.

The engine's `check` verb remains reachable for its different question — whether
a proposed path or edit collides with live reservations:

```sh
# /sync check-path <file:path|tree:path>...
PYTHONPATH="$HOME/.claude/scripts" python3 -m berth.claim_state invoke \
  --cwd "$PWD" --expected-verb check -- check <paths...> --json

# /sync check-edit uses the same check invocation for paths an edit would touch.
```

Never pass a run id to `check`; the engine resolves edit authorization itself.
On a blocked check, show `state.rendered_markdown`; it is the same five-answer
refusal shown by `/sync claim` and the edit shim. Read
`state.scope_facts.kind = exact_requested_scopes` and show its
`requested_scopes`; do not infer this variant from key presence. On a clear check,
`state.kind = edit_authorized` and the state names whether acquisition was
`appended`, `widened`, or `already_held`. If its session mapping is unavailable,
show its nonblocking degraded-success diagnostic and reservation id and continue.

### `/sync release`, `/sync sequence`, and `/sync integrate`

Map directly to the phase-1 verbs and always request JSON:

```sh
PYTHONPATH="$HOME/.claude/scripts" python3 -m berth.claim_state invoke \
  --cwd "$PWD" --expected-verb release -- release <reservation-id> --json

PYTHONPATH="$HOME/.claude/scripts" python3 -m berth.claim_state invoke \
  --cwd "$PWD" --expected-verb sequence -- \
  sequence <first-reservation-id> <then-reservation-id> --why "<user reason>" --json

PYTHONPATH="$HOME/.claude/scripts" python3 -m berth.claim_state invoke \
  --cwd "$PWD" --expected-verb integrate -- integrate <reservation-id> --json
```

Do not silently choose `integrate --force`; it skips recorded holds and requires
the user's explicit direction and reason.

If `sequence` or `integrate` rejects an `inactive_session_mapping`, say that the
harness session names an inactive coordination run, name the run and requested
reservation(s), and tell the user to restart the coordination run or use the
explicit active reservation. Do not call it a stale worktree marker;
`inactive_marker_run` is a different rejection: name its coordination run and
tell the user to remove or replace the stale worktree coordination marker.

## Terminal outcomes

These rules bind every command before its ordinary transition:

- `unconfigured` at exit `4` with `no_facts` is terminal. Print the exact
  expected `.claude/config/berth.toml` path from the diagnostic and say to run
  `cargo-berth init`. Do not enter board, claim, release, sequence, integrate, or
  drift logic.
- `ledger_unreadable` at exit `4` is a genuine ledger/configuration failure, not
  `unconfigured`; report its diagnostic and establish no facts.
- Exit `6` means the engine's single ten-second deadline is exhausted. Report
  “the ledger is busy, try again” and name the exact command to rerun. Do not
  retry, clear, or describe the ledger as unreadable.
- Exit `5` is invalid input. Exit `7` belongs to the terminal board and is
  unreachable because `/sync board` always uses `--json`.

State is pulled when the user asks. There is no emit ritual.
