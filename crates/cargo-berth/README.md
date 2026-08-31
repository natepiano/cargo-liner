# cargo-berth

`cargo-berth` coordinates path ownership and merge order between Git worktrees.
It keeps an append-only journal in the repository's common Git directory, so
every worktree sees the same reservations.

It does not choose an order for you. It records explicit answers, checks them
before integration, and shows the resulting state on a board.

## First use

Install the binary, initialize one repository, claim the paths you are about to
touch, do the work, then walk the reservation through its lifecycle:

```console
$ cargo install cargo-berth
$ cargo berth init
$ cargo berth claim crates/parser src/main.rs --run 01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1b --why "update the parser"
$ cargo berth board
# ... edit and commit ...
$ cargo berth release <reservation-id>     # checkpoint: pins the current commit
$ cargo berth integrate <reservation-id>   # moves trunk to this worktree's HEAD
$ cargo berth release <reservation-id>     # records the integrated disposition
```

`cargo berth ...` and `cargo-berth ...` invoke the same executable.

`init` creates `.git/cargo-berth/journal.ndjson`, writes
`.claude/config/berth.toml`, and installs two managed hooks when their names are
available: a `reference-transaction` trunk gate and a non-blocking `post-commit`
drift warning.

**`release` is a lifecycle walk, not a single act**, which is why it appears
more than once above. Called on an active reservation it *checkpoints* — pinning
the commit that later evidence is judged against — and leaves the reservation
outstanding. Called again after `integrate` has moved trunk, it records the
terminal `integrated` disposition. A third call is sometimes needed: if the run
after `integrate` only materialized the new evidence, the call after that records
the disposition. If some other stateful read already materialized that evidence,
the first post-integration `release` records the disposition directly. Run it
until the board shows a disposition; repeating it is safe.

`integrate` never checkpoints and `release` never moves trunk. Keeping them
separate is what lets a reservation hold a pinned commit while trunk moves
underneath it.

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

requester$ cargo berth claim tree:crates/shared --run 01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1c --plan docs/work.md --phase requester --why "consume the shared API" --after 01a0370c-860c-7d90-bd62-3497dd7063aa --overlap-why "the holder API must land first" --proposal '<the token from the exit-3 payload, byte for byte>'
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
cargo-berth could not complete the post-commit drift check. Changed paths outside.txt were not widened because attribution is ambiguous among reservations 01a036fd-c494-73c3-8999-9682008496f1, 01a036fd-c539-76f2-9121-2b4d79bf3075. Run `cargo-berth drift --reservation <id>` with one listed reservation. Run `cargo-berth drift --full` by hand; this commit remains in place.
```

That is the intended non-mutating outcome: the commit exists, no reservation
changed, and the paths can be attributed by hand. An unidentified coordination
run is reported only by the command, because there is no identity to journal:

```text
Changed paths outside.txt were not widened because no coordination run was identified. Set CARGO_BERTH_RUN to the run that owns the target reservation, then run `cargo-berth drift --reservation <id>`.
Incursion 01a036fe-2b98-7e11-8ebb-af53d83640bf: reservation 01a036fd-cbbc-7ce3-8a72-6f1ee19c4693 entered shared/entered.txt held by foreign reservation(s) 01a036fd-cd09-7c10-b121-354321926b1b. Stop and resolve the overlap with `cargo-berth resolve 01a036fd-cbbc-7ce3-8a72-6f1ee19c4693 --incursion 01a036fe-2b98-7e11-8ebb-af53d83640bf` before making more changes.
Incursion 01a036fe-2b98-7e11-8ebb-af664b883b7c: reservation 01a036fd-cc5e-7b12-bcf8-e4f18ca90755 entered shared/entered.txt held by foreign reservation(s) 01a036fd-cd09-7c10-b121-354321926b1b. Stop and resolve the overlap with `cargo-berth resolve 01a036fd-cc5e-7b12-bcf8-e4f18ca90755 --incursion 01a036fe-2b98-7e11-8ebb-af664b883b7c` before making more changes.
```

`CARGO_BERTH_BYPASS=1` skips the whole post-commit check.

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

The full envelope schema — every payload variant and every journal record type —
is in [JSON and journal contract](https://github.com/natepiano/cargo-liner/blob/main/docs/cargo-berth/json-contract.md).

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

## Recovery and identity

How edit authorization resolves, how bypasses are audited, and how to recover a
damaged ledger are in [Operations](https://github.com/natepiano/cargo-liner/blob/main/docs/cargo-berth/operations.md).

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
`exit_code`, `reservations`, `blocked_by`, and `message`, followed by a typed
`payload`. The payload variants are in
[JSON and journal contract](https://github.com/natepiano/cargo-liner/blob/main/docs/cargo-berth/json-contract.md).

## Where the rest of the documentation lives

This crate is one member of a workspace, and the longer references sit in the
repository at `docs/cargo-berth/` rather than inside the published package. The
links below are absolute so they resolve from crates.io as well as a checkout.

- [JSON and journal contract](https://github.com/natepiano/cargo-liner/blob/main/docs/cargo-berth/json-contract.md) — the envelope `--json`
  emits, every payload variant, and every journal record type.
- [Operations](https://github.com/natepiano/cargo-liner/blob/main/docs/cargo-berth/operations.md) — how edit authorization resolves, how
  bypasses are audited, and how to recover a damaged ledger.

## Deliberate omissions

`cargo-berth` does not choose merge order, read plans, track phases, parse work
orders, or coordinate paths across repositories. One ledger belongs to one Git
repository. A harness can supply plan and phase labels as provenance, but the
binary treats them only as text.
