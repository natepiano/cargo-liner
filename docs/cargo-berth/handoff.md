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

### §1 item 1 — re-anchor `phase_start_head` on a branch rewrite (the big one)

**The machinery already exists and nothing triggers it.**
`JournalOperation::Resnapshot { reservation_id, snapshot }` with
`ReservationSnapshot::Active { claim_snapshot }` assigns
`reservation.phase_start_head` during replay
(`reservation/mod.rs`, `apply_resnapshot`). Its journal doc comment reads
verbatim *"Replace the comparison points after a rebase or trunk rewrite."*
Today only `verb/release.rs` emits `Resnapshot`, and only the `Outstanding`
variant. Nothing ever emits `Active`.

**The signal is observable.** Probed against real git 2.55.0 — a `git rebase main`
on a feature branch produces exactly one transaction on the concrete ref:

```
preparing / prepared / committed    77a60cb → 3307898    refs/heads/feature
```

and sets `ORIG_HEAD` to the pre-rebase tip (`0000… → 77a60cb`) at the start.
A rewrite is distinguishable from an ordinary commit because `previous` is **not**
an ancestor of `proposed`.

**A reservation knows its branch.** The claim records
`head_snapshot: {"kind": "branch", "full_ref": "refs/heads/phase", "head": <oid>}`,
so a rewritten `refs/heads/<name>` can be matched to the Active reservations on it.

**The obstacle.** `evaluate_reference_transaction` only considers the configured
trunk ref — non-trunk updates fall into `ReferenceUpdateGateSubject::NotMainEntry`
and are dropped. Re-anchoring requires the hook to act on the *acting worktree's*
branch too, which is a genuine widening of the gate's role. Note the generated
hook `cd`s to `policy_worktree` (the main worktree) before running, so the acting
worktree is **not** available from the process cwd — the ref name is the only
handle on which worktree was rewritten. `issue2.md`'s own "Checked and not a
defect" section confirms the `cd` is deliberate.

**What the new anchor should be — undecided, and this is the one real design
question.** Options considered, none yet chosen:

- `merge-base(proposed, trunk)`. Correct for the reported case: after
  `git rebase main` it is main's tip, so the range covers only the phase's replayed
  commits. Wrong for two sequential phases on one branch, where it reaches back past
  phase 1 and attributes phase 1's commits to phase 2.
- `proposed~N` where `N = rev-list --count phase_start_head..previous`. Handles the
  multi-phase case correctly, but goes too far back when a rebase drops commits as
  already-applied, re-introducing the defect in smaller form.
- **Recommended:** `proposed~N`, clamped so it is never an ancestor of
  `merge-base(proposed, trunk)`; fall back to the merge-base when the count cannot
  be computed. This is a recommendation, not a verified choice — prove it with a
  fixture that rebases a two-phase branch before committing to it.

Whatever is chosen, the issue requires the new anchor be journalled with the ref
transaction that moved it, so the change is auditable rather than inferred.

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
  all. It is only correct once the anchor is correct, so it depends on item 1.

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
and surface on every SessionStart. They are real records of the rebase defect, not
noise from the fixed loop, and will stop being minted afresh now — but the existing
ones still need a disposition, or they stay in the notice.
