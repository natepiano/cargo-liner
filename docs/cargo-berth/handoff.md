# Handoff — issue2, closed

Branch `refactor/cargo-berth-drift-split`. Everything through `234469fe` is
committed and green: `cargo nextest run --workspace` passes 2842 tests and
`cargo clippy --all-targets --all-features -- -D warnings` is clean, both under
**git 2.55.0**.

Every `issue2.md` item is closed. What remains here is the record of two
findings a future reader would otherwise re-derive, and one live disposition
the user still owes. Delete this file once that disposition is recorded.

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

## Closed since — every `issue2.md` item

- **§1 item 1 — re-anchor `phase_start_head` on a branch rewrite** (`832dd9f4`).
  Two defects, not one. Nothing emitted `Resnapshot { Active }`, and git runs
  `post-commit` for **every** commit a rebase replays, so even with the
  re-anchor in place drift acquired the new base's paths before the branch
  reference moved at the end. Drift now stands aside while `rebase-merge` or
  `rebase-apply` exists.

  **The anchor formula this file recommended was wrong.** It proposed
  `proposed~N` (`N = rev-list --count phase_start_head..previous`) clamped
  against `merge-base(proposed, trunk)`. A fixture where the rebase drops a
  commit whose patch already reached the new base lands one commit too far
  back, and the clamp does not catch it because the wrong answer sits *above*
  the merge-base. Counting cannot work and neither can patch identity alone: a
  dropped commit and a replayed commit both have upstream equivalents, so
  `--left-only --cherry-pick` reported zero dropped commits on that fixture.
  Only position separates them, and `git::rewritten_phase_anchor` uses it.

- **§1 item 2 — do not widen onto a path the worktree did not change**
  (`1c2ec0d6`). `ObservedDriftChanges::carries_work` gates the widening arm
  only; incursion and collision keep the unfiltered set. A full comparison
  answers unconditionally, because each of its components is a positive
  statement about the present and so cannot name a restored path.

- **§1 item 3 — name the commits an incursion came from** (`a46e2719`).
  `DriftEffect::Incursion` carries an `IncursionCommit` per commit, with an
  origin of `phase_authored`, `already_on_trunk`, or `unknown`. Resolved after
  the transaction commits, so no git call runs under the mutation lock.

- **§2 item 2 — say how many outstanding incidents a notice stands for**
  (`234469fe`). Needed more than a count: no disposition cleared a *set*, so
  `resolve <id> --every-incursion` is new.

`issue2.md` is untracked and its reporter may add more items; it was left in
place rather than deleted.

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
