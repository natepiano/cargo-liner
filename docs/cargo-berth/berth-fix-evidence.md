# cargo-berth fix list — field evidence

Raw evidence behind `berth-fix.md`. Appendix A is a journal-level investigation of
a reservation that blocked forever (2026-08-26); Appendix B is a measurement of
git-hook invocation cost from the same day. Both are preserved verbatim from the
original investigations. The implementation plan cites them; nothing here is a
work item.

# Appendix A — the released reservation that blocked forever

Verbatim from the 2026-08-26 investigation. Supports items 1, 2, and 3.


> Observed 2026-08-26 while running `/plan:delegate` phase 1 of the cargo-tile
> favorites plan in the `cargo-tile-favorites` worktree. Every fact below is
> from `.git/cargo-berth/journal.ndjson` and a `board --json` read taken at the
> time; nothing here is inferred from prose or from the hook text alone.

#### Why this matters

Berth spent this session telling a delegate run to STOP over a conflict that did
not exist, naming a resolution command that then reported the incident was
already resolved. Only `gate_mode = "observe"` kept it from hard-stopping the
phase. A coordination tool whose blocking signal is wrong 13 times out of 13 is
a tool people learn to ignore, and an ignored gate is worse than no gate — it
carries the authority of a check without the substance of one.

#### The cast

| Id | What it is |
|---|---|
| `01a03f1a-3d5b-78b1-b676-9804e2804afb` | the phantom blocker. Held by `/Users/natemccoy/rust/cargo-liner` on `main`. Called **R** below. |
| `01a03f60-2e8b-77c2-858f-476ee413d81c` | the phase-1 reservation in `/Users/natemccoy/rust/cargo-tile-favorites`. |
| `01a04086-c7b1-7b00-bfba-758abe313627` | the incursion incident raised against phase 1. |
| `01a03f08-e197-7a83-9b7c-bc7c555d0c00` | worktree id of `cargo-liner` (main checkout). |
| `01a03f1f-6d9c-7383-8389-a6fd541e79d5` | worktree id of `cargo-tile-favorites`. |

#### Defect 1 — a reservation released as `integrated` keeps blocking (critical)

R's complete journal history:

| Time (UTC) | Op | Detail |
|---|---|---|
| 17:24:39.387 | `claim` | `desktop.rs`, `query.rs`, `overlays/keymap.rs` |
| 17:27:05.850 | `widen` | later board shows scopes grown to include `band.rs`, `pixels.rs` |
| 17:29:02.044 | `checkpoint` | `protected_tip: 252cea74fb383a625379f1390d69f22b8a7dd2f0` |
| 17:29:02.837 | `evidence_revalidated` | |
| 17:29:09.597 | `release` | **`disposition: {kind: integrated}`** |

R is done. Checkpointed, revalidated, released, integrated — the full happy path,
completed in under five minutes.

**Twenty-five minutes later it started blocking, and never stopped.** R appears as
the `foreign_reservation_ids` holder on **13 incursion events** spanning 6h34m:

```
17:54:18Z  wt 01a03f08 (cargo-liner)          band.rs, desktop.rs, pixels.rs, query.rs, overlays/keymap.rs
18:44:59Z  wt 01a03f08                        desktop.rs
18:45:12Z  wt 01a03f08                        desktop.rs
18:45:30Z  wt 01a03f08                        desktop.rs
18:45:43Z  wt 01a03f08                        desktop.rs
20:18:37Z  wt 01a03f08                        .claude/commands/sync.md, desktop.rs
20:19:29Z  wt 01a03f08                        .claude/commands/sync.md, desktop.rs, query.rs
20:36:55Z  wt 01a03f08                        desktop.rs
20:37:12Z  wt 01a03f08                        desktop.rs, query.rs
20:42:03Z  wt 01a03f08                        desktop.rs
20:43:26Z  wt 01a03f08                        desktop.rs, query.rs
20:54:13Z  wt 01a03f1f (cargo-tile-favorites) cargo-berth/tests/board.rs, backdrop/constants.rs, desktop.rs, query.rs, process.rs
00:02:49Z  wt 01a03f1f (cargo-tile-favorites) band.rs, pixels.rs          <- the one that stopped phase 1
```

R is the incurring party on **zero** events and the blocking holder on **all 13**.

Eleven of those blocked `cargo-liner` itself — the very worktree that had already
checkpointed and released R. A worktree being blocked by its own completed,
integrated reservation is the clearest possible statement that release is not
actually clearing the scope.

Board state at the time of the phase-1 block:

```json
"reservation_id": "01a03f1a-3d5b-78b1-b676-9804e2804afb",
"lifecycle": { "stage": "released", "disposition": { "kind": "integrated" } },
"integration_evidence": { "kind": "current", "status": { "status": "trunk_rewritten" } },
"edit_blocking_status": "blocking",
"visibility": "reblocked_active_constraint"
```

`stage: released` and `edit_blocking_status: blocking` in the same record is the
contradiction to fix.

##### On `trunk_rewritten`

R's protected tip `252cea74` still exists as a commit object but is reachable
from **no ref** — `git merge-base --is-ancestor` says no for both `main` and the
favorites branch, and `git branch -a --contains` returns zero refs. It was
orphaned by a `git rebase` of `feat/cargo-tile-favorites` onto `main`.

So berth's `trunk_rewritten` detection is factually correct. The bug is what it
does with that fact: it flips a `released` + `integrated` reservation to
`reblocked_active_constraint`.

**Losing the checkpoint commit to a rebase does not un-merge the work.** The
disposition already recorded the outcome as `integrated`. Re-deriving a blocking
state from vanished evidence, after a terminal disposition was recorded, turns
every rebase into a permanent phantom blocker with no operator path to clear it.

Note the timeline forbids blaming the rebase alone: the first phantom block was
17:54:18Z, and the rebase that orphaned `252cea74` happened at roughly 23:45Z.
R was already blocking **six hours before** its protected tip became unreachable.
`trunk_rewritten` compounds this defect; it did not cause it.

**Suggested fix:** a terminal disposition wins over recomputed evidence. Once
`lifecycle.stage = released` with `disposition.kind = integrated`, the scope is
released for edit-blocking purposes permanently, whatever later happens to the
protected tip. If lost evidence must still be surfaced, surface it as an alert,
never as `edit_blocking_status: blocking`.

#### Defect 2 — `resolve` is journalled against the wrong worktree (important)

The resolve was invoked from `/Users/natemccoy/rust/cargo-tile-favorites`:

```sh
cd /Users/natemccoy/rust/cargo-tile-favorites
cargo-berth resolve 01a03f60-2e8b-77c2-858f-476ee413d81c \
  --incursion 01a04086-c7b1-7b00-bfba-758abe313627 --json
```

The two journal events for that incident:

```json
{ "at": "2026-08-27T00:02:49.929Z", "op": "incursion",
  "actor": { "worktree": "01a03f1f-6d9c-7383-8389-a6fd541e79d5",
             "run": "01a03f60-2e87-7b93-b933-e3dc5e9211d9" },
  "incident_id": "01a04086-c7b1-7b00-bfba-758abe313627" }

{ "at": "2026-08-27T00:04:18.340Z", "op": "resolve_incursion",
  "actor": { "worktree": "01a03f08-e197-7a83-9b7c-bc7c555d0c00",
             "run": "01a03f63-03e7-7fb2-ae63-5b297177f59f" },
  "incident_id": "01a04086-c7b1-7b00-bfba-758abe313627" }
```

The `incursion` is attributed correctly to `cargo-tile-favorites`. The
`resolve_incursion` 89 seconds later is attributed to **`cargo-liner`** — the
main checkout, which ran no such command.

Both attributed values match the main checkout's files exactly:

- journalled worktree `01a03f08-e197-…` = contents of `.git/cargo-berth-worktree-id`
- journalled run `01a03f63-03e7-…` = contents of `.git/cargo-berth-run-id`

while the correct per-worktree values sit in
`.git/worktrees/cargo-tile-favorites/cargo-berth-{worktree,run}-id` and hold
`01a03f1f-6d9c-…` / `01a03f60-2e87-…` — the pair the `incursion` event used
correctly minutes earlier.

**Hypothesis, not verified in source:** the resolve path reads identity from
`$GIT_COMMON_DIR/cargo-berth-*-id` where the incursion path reads it from
`$GIT_DIR/cargo-berth-*-id`. In a worktree those differ; in the main checkout
they coincide, which is why this never shows up when testing from the primary
checkout.

This is the same misattribution family already recorded in `issue2.md` (rebase
attributing upstream commits to the rebasing worktree). Worth checking whether
one shared identity-resolution helper fixes all of them.

##### Phase 6 reproduction result — not reproduced

The Phase 6 fixture used the recorded invocation route from the linked
`cargo-tile-favorites` worktree and wrote the recorded marker pairs into the
same main and linked administrative-directory locations:

- main: worktree `01a03f08-e197-7a83-9b7c-bc7c555d0c00`, run
  `01a03f63-03e7-7fb2-ae63-5b297177f59f`
- linked: worktree `01a03f1f-6d9c-7383-8389-a6fd541e79d5`, run
  `01a03f60-2e87-7b93-b933-e3dc5e9211d9`

The fixture first recorded an incursion from the linked worktree, then invoked
`cargo-berth resolve <reservation> --incursion <incident> --json` there. Both
events carried the linked pair. The fixture passed before any production source
changed, so the common-directory hypothesis did not reproduce the incident.

The incident record contains no `CARGO_BERTH_SESSION_ID`, `CARGO_BERTH_RUN`,
`GIT_DIR`, or `GIT_COMMON_DIR` values. Phase 6 therefore retained the evidence
as an unexplained historical misattribution and added `identity_inputs` to every
new journal event. It records the invocation directory and those four process
environment values, including explicit `unset` and non-UTF-8 states. A future
recurrence will show whether the command ran in the reported directory and
which session, run, and Git inputs were present.

Worktree actor resolution uses the canonical invocation directory's filesystem
metadata. A relative `gitdir:` locator is resolved from the worktree root, and a
relative `commondir` locator is resolved from the per-worktree administrative
directory. `GIT_DIR` and `GIT_COMMON_DIR`, whether absolute or relative, do not
override those actor paths; their raw values are recorded for diagnosis. When
the Git variables are absent, the same filesystem traversal remains in use.

#### Defect 3 — resolving a live incident reports `invalid_input` (important)

The invocation above returned:

```json
{ "envelope": { "exit_code": 5, "status": "invalid_input",
    "message": "incursion incident 01a04086-c7b1-7b00-bfba-758abe313627 is already resolved",
    "payload": { "kind": "no_facts" } },
  "invocation": { "attempts": 1, "process_exit": 5 } }
```

There is exactly **one** `resolve_incursion` event in the journal for this
incident, timestamped 00:04:18.340Z — within seconds of that invocation, and
attributed per Defect 2. No other session was running a resolve.

So the caller was told its operation was rejected as invalid input while the
journal recorded the resolution. Whichever way round it happened — write then
misreport, or a validation pass that reads its own just-written state — the
caller cannot distinguish "you already did this" from "someone else did this"
from "this succeeded and I am describing it badly", and an exit code of 5 on a
successful mutation will make any wrapper treat a completed resolve as a failure.

**Suggested fix:** a resolve that ends with the incident resolved and this caller
responsible is exit 0 with a payload describing what it did. Reserve
`invalid_input` for an incident that was genuinely resolved by a *different*
actor before this call, and say who in the payload.

#### Defect 4 — the hook keeps printing STOP for a resolved incident (minor)

After the resolve at 00:04:18Z, the PostToolUse hook continued to emit:

```
INCURSION: reservation 01a03f60-2e8b-77c2-858f-476ee413d81c entered
crates/tui_pane/src/backdrop/band.rs, crates/tui_pane/src/backdrop/pixels.rs,
held by 01a03f1a-3d5b-78b1-b676-9804e2804afb; incident
01a04086-c7b1-7b00-bfba-758abe313627. STOP. Resolve with `cargo-berth resolve …`
before making more changes.
```

on a subsequent tool call, naming the incident the journal had already recorded
as resolved and telling the operator to run the command that had just returned
exit 5. This is downstream of Defects 1 and 3 and probably clears with them, but
the STOP text should be gated on the incident's live resolved state regardless.

#### What this cost

Nothing in the tree, because `gate_mode = "observe"` in
`.claude/config/berth.toml` meant the block was advisory. The phase-1 delegate
completed all its edits, ran 398 tests green and passed lint while the hook was
printing STOP.

Under `gate_mode = "enforce"` this would have hard-stopped phase 1 partway
through — after `band.rs` and `pixels.rs` were already modified — over a
reservation whose work merged six hours earlier, with the documented recovery
command returning `invalid_input`.

The operator cost was the whole investigation this document came out of.

#### Checked and not a defect

- **`cargo-liner` really is clean.** `git status --short` empty, on `main` at
  `3b93b692`. There was never a competing editor of `band.rs` or `pixels.rs`.
  The conflict was not merely stale — it never existed.
- **The auto-widen path is correct.** The phase-1 reservation widened cleanly
  onto `random.rs`, `text.rs`, `mod.rs`, `lib.rs` and `CHANGELOG.md` as the
  delegate touched them, and reported each widen accurately.
- **The `incursion` event's own attribution is correct** — right worktree, right
  run, right paths, right foreign holder. Only `resolve_incursion` is wrong, which
  is what makes the shared-helper hypothesis in Defect 2 worth testing first.
- **`trunk_rewritten` detection is accurate.** `252cea74` genuinely is unreachable
  from every ref. The detection is right; only the conclusion drawn from it is wrong.

---

# Appendix B — the git-hook invocation cost

Verbatim from the 2026-08-26 measurements. Supports items 4, 5, 6, and 9.


Measured 2026-08-26 against the `cargo-berth` installed at
`~/.cargo/bin/cargo-berth` (built 18:56, so it contains `77530afa` and
everything before it), with `git version 2.55.0` at `/opt/homebrew/bin/git`
first on `PATH` ahead of Apple's 2.50.1.

#### Symptom

A 15-commit rebase took long enough to be worth investigating. It is not one
slow operation. Berth is attached to the highest-frequency event git has, and
it pays a process spawn for every delivery of that event, including the ones it
immediately discards.

Reduced to three commits, in this repository:

| Configuration | Time |
| --- | --- |
| `git -c core.hooksPath=/dev/null rebase …` | 0.23s |
| hooks installed, `CARGO_BERTH_BYPASS=1` | 5.44s |
| hooks installed, live | 7.97s |

The same rebase in a freshly `git init`-ed repository with no hooks is 0.03s.
`git status` here is 0.06s against 771 tracked files, so the working tree is
not the cause.

Two things follow:

- The floor is 0.23s. Everything above it is hook cost.
- **5.44 of the 7.97 seconds are spent before berth does any drift work.**
  `CARGO_BERTH_BYPASS=1` is not a fast path; see §3.

#### 1. Git delivers 25 invocations per commit, and berth wants one

Instrumented with a no-op `reference-transaction` hook logging `$1` and each
ref arriving on stdin. Three commits, rebased:

```
83 hook invocations total
  75 reference-transaction
   3 prepare-commit-msg
   3 post-commit
   1 post-rewrite
   1 post-checkout
```

The 75 break down two ways.

**By ref:**

| Ref | Calls | What it is |
| --- | --- | --- |
| `CHERRY_PICK_HEAD` | 21 | scratch state, written and deleted inside each pick |
| `AUTO_MERGE` | 21 | scratch state, per pick |
| `HEAD` | 15 | moves per pick |
| `REBASE_HEAD` | 12 | scratch state, per pick |
| `ORIG_HEAD` | 3 | set once at the start |
| `refs/heads/<branch>` | **3** | the branch the trunk gate exists to guard |

**By phase:**

| Phase | Calls |
| --- | --- |
| `preparing` | 22 |
| `prepared` | 22 |
| `committed` | 22 |
| `aborted` | 9 |

Berth acts only on `prepared` — `cli.rs:1115` returns `BerthExit::Clear` for
every other phase — and only `refs/heads/<trunk>` can matter to a trunk gate.
Of 75 invocations, **at most 3 carry information berth uses**, and the
generated hook script spawns the binary for all 75 before any of that is known.

Git 2.55 made this worse by adding `preparing`: a transaction that reported
three times under 2.50 now reports four when it aborts, and every `preparing`
fire is pure loss.

##### Fix

Both filters are shell, in the script generated by `gate/install.rs`, and
neither needs the binary to start:

```sh
[ "$1" = "prepared" ] || exit 0
```

plus a stdin check that exits when no line names `refs/heads/<trunk>`. The
first takes 75 → 22. The two together take 75 → about 1.

Filtering earlier does not weaken the gate: a phase berth already answers
`Clear` for, and a ref that is not the trunk, cannot change a gate decision.
Keep the existing "executable unavailable, permitting" behaviour on the path
that does reach the binary.

#### 2. `drift --full` scales with the incursion, one subprocess per path

`post-commit` runs `cargo-berth drift --full`. Traced by shimming `git` onto
`PATH`, with a 33-path incursion outstanding:

```
62 git subprocesses, ~1.5s
  33 log         git log --format=%H%x1f%s <base>..HEAD -- <ONE path>
  14 merge-base  git merge-base --is-ancestor <sha> HEAD
   8 rev-parse
   3 diff
   2 update-ref
   1 worktree
   1 ls-files
```

The 33 `log` calls carry an identical commit range and differ only in the
pathspec. The 14 `merge-base` calls ask about one commit each. Both are
per-item process spawns for a question one process answers.

After resolving the outstanding incursions, the same command:

```
17 git subprocesses
   8 rev-parse, 3 diff, 2 update-ref, 2 merge-base, 1 worktree, 1 ls-files
```

`log` went to zero and `merge-base` to two. That is the confirmation: **the
subprocess count is the incursion's path count.** An incursion left open makes
every later commit in the repository slower, without bound — the wrong
direction for a condition the user is being nagged to clear.

Per-commit effect of clearing them: 2.10s → 1.84s. Real, but small next to §1,
which is where the time actually goes.

##### Fix

- One `git log <base>..HEAD --name-only --format=…` replaces the N per-path
  calls; attribute paths to commits from that single output.
- One `git rev-list <base>..HEAD` replaces the N `merge-base --is-ancestor`
  calls; ancestry is set membership once the list is in hand.

Target a fixed subprocess count that does not move with the number of paths or
commits involved.

#### 3. `CARGO_BERTH_BYPASS=1` still spawns the binary every time

In the installed `reference-transaction` script, the bypass branch does not
exit early. It runs `cargo-berth __reference-transaction "$@"` to record the
bypass, on every one of the 75 invocations. That is the whole 5.44s in the
opening table — 5.2s of it paid before any drift work, in the mode whose
purpose is to not do the work.

##### Fix

Record the bypass once, on `prepared` and for the trunk ref only, under the
same filter as §1. The other invocations have nothing to record. The
marker-file branch further down the same script already gates on
`[ "$1" = "prepared" ]`, so the shape is established in the file — the binary
call above it just does not use it.

#### 4. Berth's own git calls re-enter its own hooks

`git/command.rs:15-21` builds every subprocess as
`git --no-optional-locks -C <root> …` with no hook suppression. `drift --full`
makes two `update-ref` calls, and each fires `reference-transaction`.

Verified without touching repository config, by passing `core.hooksPath`
through `GIT_CONFIG_COUNT` / `GIT_CONFIG_KEY_0` / `GIT_CONFIG_VALUE_0` so
berth's git children inherit it:

```
8 reference-transaction fires from berth's own update-ref calls
  2 preparing, 2 prepared, 2 committed, 2 aborted
```

In production those are 8 more `cargo-berth` process spawns per drift run —
berth gate-evaluating its own bookkeeping writes.

##### Fix

Suppress hooks on berth's own git invocations in `git_command`, e.g. `-c
core.hooksPath=` on the constructed `Command`. This is a correctness point as
much as a cost one: berth's internal ref writes are not user history.

#### 5. A cross-worktree run mismatch is reported as a missing reservation

Lower priority, found while working through the above.

`cargo-berth check` run from a worktree whose session maps to a reservation
held by a *different* worktree fails with:

```
harness session mapping for coordination run <run-id> no longer names an
active reservation; retry the command
```

The reservation named was alive and `active` the whole time — it simply
belonged to another worktree. The message sends the reader looking for a
deleted record instead of a worktree mismatch, and `retry the command` cannot
help, because a retry changes nothing.

##### Fix

Say what is actually true: the run's reservation is held by worktree X and the
command was issued from worktree Y. Name both. Drop the retry advice on this
path.

#### Verification

- A 3-commit rebase should land near the 0.23s floor, not 7.97s.
- Instrument with a no-op `reference-transaction` hook (log `$1` and stdin) and
  count: of 75 invocations, about 1 should reach the binary.
- Shim `git` onto `PATH` and count subprocesses in `drift --full`: the count
  must not vary with the number of paths in an outstanding incursion.
- Re-run the `GIT_CONFIG_*` probe in §4: berth's own git calls should fire the
  hook 0 times.
- Gate behaviour must not change. `preparing`, `committed`, and `aborted` still
  permit; an unknown phase still permits (`5af62641`); a missing executable
  still permits with the printed warning.
