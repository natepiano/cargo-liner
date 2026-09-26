# cargo-berth operations

Identity resolution, bypass auditing, and recovery from a damaged ledger.
Reach for this when something has gone wrong or when wiring `cargo-berth` into
a harness.

For what the tool is and how to use it, see the [README](../../../crates/cargo-berth/README.md).

## Worktree enrollment

Run `cargo berth init` from any worktree to set up the ledger and hooks, then
enroll live worktrees with existing work and no reservation history. The
configuration lives at the main worktree's `.claude/config/berth.toml`; a linked
worktree reads that file unless it has its own. An existing main file is kept.
When the main file is missing and `init` runs in a linked worktree with its own
valid file, that file's `trunk`, `gate_mode`, `maximum_reservations`, and
`maximum_ordering_edges` seed the new main file; otherwise defaults are
written. An invalid linked file makes `init` from that worktree fail before the
main file is written. An uncommitted configuration file at that path, untracked
or tracked and modified, is excluded from enrollment and merge-extent
observations; committed changes to it still count.

Enrollment reserves the exact file paths merging the branch into its recorded target would
change or conflict on, plus its staged, unstaged, and untracked paths. Locked worktrees are eligible. Existing reservations, including
ended ones, prevent a worktree from being enrolled again. Re-run `init` after
adding a worktree with commits, before its first edit, or after fixing a failed
candidate. Enrollment at later hook contacts is not automatic.

The report gives each unresolved enrollment overlap's reservation ids, shared
paths, and two runnable `sequence` commands. Run the order you intend, with a
useful `--why` explanation. Editing remains authorized on both sides; the
overlap holds integration until sequenced (reported in observe mode, rejected
in enforce mode). Re-running `init` repeats unresolved pairs even after an
endpoint ends. Enrollment runs end through the usual checkpoint and release
workflow, or automatically when their checkout is clean with no remaining
branch change at a commit trunk contains.

Failures name the worktree root and include a diagnostic. Other candidates
continue:

| Reason | Recovery |
| --- | --- |
| `operation_in_progress` | Finish or abort the rebase, merge, cherry-pick, or revert, then retry. |
| `no_merge_base` | Ensure `HEAD` and configured trunk resolve to commits with a shared ancestor, then retry. |
| `git_failure` | Repair the Git command failure shown in the diagnostic, then retry. |
| `unavailable` | Restore access to the registered worktree, or remove its stale registration through Git. |
| `record_too_large` | Reduce the work footprint before retrying; enrollment does not split a claim across journal records. |

Enrollment publishes a coordination-run marker in each enrolled worktree and
leaves the invoking harness session mapping unchanged. An unmapped session
there joins through the marker and reuses the reservation for covered edits.

## Integration targets

Ordering edges use the shared target when both reservations are judged at the
same branch. Across targets, they hold the successor until the predecessor's
work reaches the repository trunk, unless the successor has already
incorporated it. After trunk receives the predecessor, the successor must
still incorporate that work. In enforce mode, a proposed trunk update that
brings in a held successor is rejected by the reference-transaction gate.
Board instructions and gate recovery name the shared target for a same-target
edge and the repository trunk for an edge across targets.


Reconciliation judges each reservation at its recorded target. If an unreleased lane's non-trunk target ref disappears, the `target_missing` alert supplies `cargo-berth retarget <id> --target <branch>` and the board shows that target's commit as `"unresolved"`. Until the reservation is retargeted, the alert persists and reconciliation judges that reservation at the repository trunk. A released lane keeps its proof if its target branch is later deleted; lost evidence is reported only if the proving commits leave history and fail revalidation on two passes. The trunk gate still gates the repository trunk.

`cargo-berth claim <paths> --target <local-branch>` records the integration branch for the new reservation. Without the flag, the claimant branch's `branch.<name>.cargoBerthTarget` setting in the common Git config takes precedence over the repository trunk. Detached HEAD uses the repository trunk. Explicit own-branch and unresolved targets return `invalid_input`; first touch and enrollment fall back to the trunk and report the reason in JSON.

`cargo-berth retarget <reservation> --target <local-branch>` changes an unreleased reservation's recorded target and resets its integration evidence. `init` pins old reservations that have no recorded target to the current repository trunk once; repeated `init` does not append another pin. Reconciliation and release judge each reservation at its recorded target; the trunk gate still judges the repository trunk. A malformed `--target` names why it is not a local branch, while a well-formed branch without a ref reports that it does not resolve.

`resolve <id> --integrated-as <commit>` accepts a carrying commit only when it is reachable from that reservation's target. Drift compares committed incursion origins with the acting session reservation's target, or each reporting reservation's target when no session reservation is mapped. Commits already on that target are attributed as upstream history.

A present non-trunk target needs a cover in the worktree where it is checked out. Any live reservation there aimed at the target's parent serves as the cover; otherwise reconciliation claims the target branch's full diff and dirty paths. A lane reaches a final integrated release at that target only after the cover has a fresh extent at the target's current tip. Direct `release` at an uncovered target leaves the lane outstanding and names the target; explicit `resolve --integrated-as` remains available. If the target has no registered checkout, `target_uncovered` lists its waiting reservations. A deleted target instead raises `target_missing` and is judged at the repository trunk.

### Migrate a uniform integration trunk to per-branch targets

Run these steps once after Phases 1–4 are installed. For example, suppose `hana` currently names the integration branch and `main` is the intended repository trunk.

1. In any worktree, while `.claude/config/berth.toml` still names `hana`, run `cargo-berth init`. This pins every existing reservation, including released ones, to `hana`.
2. Run `git config branch.<lane>.cargoBerthTarget hana` for each lane. In hana, this includes `tool-based-ui-arrange`, `tool-based-ui-geometry-material`, `tool-based-ui-trunk`, and later lanes.
3. Set `trunk = "main"` in `.claude/config/berth.toml` and commit it on `main` and `hana`. Only the main worktree's copy supplies the repository trunk.
4. Run `cargo-berth retarget <id> --target main` for any live reservation in the main worktree or the `hana` worktree.
5. Run `cargo-berth board --json`. The lanes should target `hana`; the reservation in the `hana` worktree should target `main`, carry `hana`'s diff as its extent, and cover it without blocking those lanes. Reconciliation creates that cover if step 4 left no live reservation there.

Step 1 must precede step 3. Changing the trunk before the pin re-judges released lanes against `main` and raises lost-evidence alerts for their former integration proofs.

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

## One coordination run per worktree

A worktree admits one coordination run at a time. A caller that presents its own
run identity — through `CARGO_BERTH_RUN`, since a live session mapping outranks
it — in a checkout where another presented run already holds an active
reservation is refused the ability to take or widen a reservation there, at
`claim`, at the pre-edit hook, and at post-commit drift. On the post-commit path
the response has `status = "scope_acquisition_refused"` and exit 5 — the run
observed and recorded everything it found, and was refused only its acquisition.
It names the incumbent reservation and offers one runnable repair:

```text
cargo-berth release <incumbent-reservation-id> --json
```

Run it in the occupied worktree. It checkpoints the incumbent out of `Active`,
which is the state the refusal names. A marker sweep does not repair this: the
sweep preserves every marker whose run still holds an active reservation, so it
returns the caller to the same refusal. When the incumbent is still working, the
remedy is a separate checkout, and no command performs it.

The refusal withholds acquisition and nothing else. Post-commit drift observes,
classifies, and records first, so a refused invocation still reports what its
commit did — including an incursion into paths another reservation holds — and
then reports that it may take nothing here. A run that presents no identity is
never refused, so an existing repository never meets this rule as a lockout.

## Bypass audit

`CARGO_BERTH_BYPASS=1` permits a trunk update no matter what other gate input is
broken. The override is tested before the tool reads the config, journal, or Git
evidence. The tool records an audit fact in the journal when writable, or writes
a pending marker for a later session to recover. The fact names
`CARGO_BERTH_BYPASS=1` and when it was taken. It has no reason and names no
specific hold that it skipped, because nothing else had been read yet. If
neither destination is writable, the tool warns and still permits the update.
The committed `reference-transaction` hook also writes pending markers of
`kind = "branch_rewrite"` under the same file naming to carry a rewrite map to
the next reconciliation; those never count or report as bypasses.

The same variable is the escape hatch for the edit gate. The pre-edit wrapper
honors `CARGO_BERTH_BYPASS=1` before it reaches the engine, so an engine that
hangs or crashes, or is missing, never stands between a user and a write. The
wrapper allows the edit, leaves a pending marker whose `action` is `editing`,
and the next journal write imports it. The engine honors the variable the same
way when it is invoked directly.

## Recovery

`init` has three branches:

- Plain `cargo berth init` creates a missing ledger and main-worktree config,
  installs or refreshes each managed hook without replacing an unmanaged hook,
  and enrolls existing worktree work as described under
  [Worktree enrollment](#worktree-enrollment).
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

A malformed record looks like this and is the case for confirmed
reinitialization:

```text
The reservation ledger could not be read: journal replay failed: journal record 1 is corrupt: missing field `event_id` at line 1 column 44
[exit 4]
```

A record with a schema version this executable does not write is different:

```text
The reservation ledger could not be read: journal replay failed: journal schema version 2 is unsupported
[exit 4]
```

Upgrade `cargo-berth` for an unsupported schema version rather than
reinitializing that journal.

An outstanding reservation needs no command once its work reaches trunk:
ordinary reconciliation (`board`, `check`, the post-Bash hook, the trunk gate,
or `release`) settles it when git proves the whole scoped phase is on the
checked-out trunk and no reserved work remains outside that proof, including
after a rebase, amend, or reset. An orphan notice names the dispositions that
fit the orphan and the observed trunk.

`resolve` records one of these explicit decisions for what reconciliation
cannot prove:

- `--recovered` rebinds a reservation to the worktree running the command.
- `--integrated-as <trunk-oid>` records a verified alternate commit already
  reachable from the reservation's target.
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
