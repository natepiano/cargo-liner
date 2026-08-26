# Handoff — issue2 remaining work

Branch `refactor/cargo-berth-drift-split`. Everything below `b51e3f8f` is
committed and green: `cargo nextest run --workspace` passes 2836 tests and
`cargo clippy --all-targets --all-features -- -D warnings` is clean, both under
**git 2.55.0**.

Delete this file when the work below is picked up.

## Environment changed this session

`brew install git` put **git 2.55.0** on `PATH` at `/opt/homebrew/bin/git`;
Apple Git-155 remains at `/usr/bin/git` behind it. This matters: the whole
`hook-phase-issue` defect was invisible under the old 2.50.1, and the berth
suite now exercises the `preparing` phase for real. Do not assume a green run on
a machine still using Apple git proves anything about that path.

The installed `cargo-berth` on `PATH` was reinstalled from this branch
(`cargo install --path crates/cargo-berth --locked`). It carries the
worktree-identity and incursion fixes, so the PostToolUse hook works again.

## Closed this session

- `reservation-issue.md` — worktree-identity edit blocking. Deleted; its two
  secondary defects live in `berth-plan-next.md`.
- `hook-phase-issue.md` — both parts. Deleted.
- `issue2.md` §2 item 1 — incursion dedup. See the divergence note below.

## Still open in `issue2.md`

### §1 item 1 — re-anchor `phase_start_head` on a branch rewrite — DONE (`832dd9f4`)

Closed. Two things were wrong, not one, and the second is not in `issue2.md`:

1. Nothing emitted `JournalOperation::Resnapshot { Active }`, so a rebase left
   the anchor describing a history the branch no longer had.
2. Git runs `post-commit` for **every** replayed commit during a rebase. Even
   with the re-anchor in place, drift ran on each of them and acquired the new
   base's paths before the branch reference moved at the end. `drift` now
   stands aside while `rebase-merge`/`rebase-apply` exists.

**The anchor recommendation this file previously gave was wrong.** It proposed
`proposed~N` (`N = rev-list --count phase_start_head..previous`) clamped so it
is never an ancestor of `merge-base(proposed, trunk)`. A fixture in which the
rebase drops a commit whose patch already reached the new base lands one commit
too far back, and the clamp does not catch it, because the wrong answer sits
*above* the merge-base.

Counting cannot work, and neither can patch identity on its own: a dropped
commit and a replayed commit both have upstream patch-equivalents, so
`--left-only --cherry-pick` returned zero dropped commits on that fixture.
Only **position** separates them — the replayed commits are contiguous at the
tip. `git::rewritten_phase_anchor` takes the equivalent set from
`rev-list --cherry-mark --left-right --no-merges <previous>...<proposed> ^<phase_start>`,
walks `rev-list --first-parent` down from the new tip while each commit is in
that set, and anchors beneath the last one. Exact on all three fixtures:
single-phase, clean two-phase, and the drop case.

### §1 item 2 — do not widen onto a path the worktree did not change

**Do not implement this as a straight copy of `256a04b8`.** That commit filtered
the post-write first-touch claim through `cache_value.modified_paths()`, the
working-tree fingerprint. Applying the same filter to widening would break
legitimate widening from the **committed** component: a phase that commits work and
leaves a clean tree must still widen onto what it committed, and `issue2.md`'s own
completion condition demands that ("a widening driven by real uncommitted **or
committed** work in the worktree is unchanged").

Correct decomposition:

- Widening from the cheap/working-tree components **should** intersect with
  `modified_paths()`. A path that went dirty→clean appears in the cheap symmetric
  difference, and widening onto it acquires a block on a restored file. That is the
  `256a04b8` rule and it is a real defect independent of any rebase.
- Widening from the **committed** component cannot be fixed by working-tree state at
  all. It is only correct once the anchor is correct — that dependency on item 1
  is now discharged.

### §1 item 3 — name the commits an incursion came from

Not started. Target `drift/report.rs`, `output.rs`, and the PostToolUse shim's
`post_write_incursion` rendering.

### §2 item 2 — say how many outstanding incidents a notice stands for

Not started, and small. `outstanding_incursion_incidents` already exists on
`RetainedReservationSet` (`reservation/mod.rs`). The notice names one incident id
and reads as though answering it ends the matter.

## Divergence from the issue's diagnosis — §2 item 1

Recorded here because a future reader will otherwise "fix" the same thing again.

`issue2.md` §2 attributes the repeated incident minting to the
`ClaimSource::FirstTouch` condition on `outstanding_incursion_covers`
(`drift/classification.rs`), claiming an explicit-claim reservation has no dedup.
**The live journal says otherwise.** Every duplicate incident is immediately
preceded by a `resolve_incursion` of the identical one before it:

```
18:44:59 incursion caa5   18:45:07 resolve caa5
18:45:12 incursion fc08   18:45:28 resolve fc08
18:45:30 incursion 41fa   18:45:42 resolve 41fa
```

and where nothing was resolved, nothing was re-minted — `776a` stood outstanding
alone for ninety minutes. `observe_incursion` already deduped outstanding incidents
regardless of claim source; what it did not do was treat a *resolved* incident as
final, which is the loop fixed in `4be01466`.

`outstanding_incursion_covers` requires
`incident.reservation_id() != current_reservation_id`, so it suppresses one
reservation against *another* and can never suppress a reservation's own repeat.
Its claim-source condition was **deliberately left alone** — changing it would alter
which reservations suppress each other, and there is no evidence that behaviour is
wrong.

What was genuinely left was the superset case, fixed in `386286b4`: coverage is now
decided per path rather than by whole-set equality.

## Live state worth knowing

Two incursion incidents are still outstanding in `/Users/natemccoy/rust/cargo-liner`
(`01a03fce-f5a6-…` and `01a03fda-1948-…`) and surface on every SessionStart. They
are real records of the rebase defect, not noise from the fixed loop, and will
stop being minted afresh now that `832dd9f4` has landed — but the existing ones
still need a disposition, or they stay in the notice.
