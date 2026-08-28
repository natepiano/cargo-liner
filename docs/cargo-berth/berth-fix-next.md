# berth-fix — next items

Work this plan surfaced but does not implement. Each item names what it changes
and the evidence that produced it.

## Make `maximum_reservations` truthful against the successor-verdict retention bound

`config.rs:158` parses `maximum_reservations` as an unrestricted `u32` and
`ParsedConfigValues::finish` applies it with no upper bound. A predecessor's
successor-equivalence cache and comparison-attempt history each retain at most
`SUCCESSOR_SCOPED_PATCH_TARGET_RETENTION_LIMIT = 512` entries
(`reservation/constants.rs:9`), evicting oldest-first at
`reservation/mod.rs:262-264` and `:330-332`. Nothing ties the two numbers
together.

Configure `maximum_reservations` above roughly 513 and a single predecessor can
carry more live successor heads than its cache can retain. Verdicts for stable
heads are then evicted and recomputed, and because the round-robin admits one
cold comparison per reconciliation, the set of proven successors never closes:
at least one edge keeps reporting `awaiting_successor_incorporation` on every
pass. That is the conservative direction and never a false release, but at that
configuration the permanent block Phase 4 exists to close reopens.

Unreachable at the shipped default of 128, which allows at most 127 successors
per predecessor. The work is a validated configuration bound — reject or clamp a
`maximum_reservations` the retention limit cannot serve, and say so in the config
error — not a change to the successor cache, whose bound the Phase 4 Work Order
required. `config.rs` owns it; no phase does.

Surfaced by the Phase 4 architect review and verified against the live tree.

## Keep the configured trunk and managed hook synchronized after a proven trunk rename

Phase 8 refreshes the managed `reference-transaction` hook after a proven trunk rename but leaves `.claude/config/berth.toml` naming the deleted branch, so a later `cargo berth init` silently restores the stale hook value.

When the detached refresh finds one uniquely proven rename target, atomically compare-and-replace the configured `trunk` from the deleted branch to that target before installing the refreshed hook. Do not overwrite a concurrent configuration change; if configuration replacement or hook installation fails, retain the previous hook, emit a diagnostic, and preserve its stale-reference fail-safe.

Acceptance renames `main` to `renamed` and proves both `berth.toml` and the managed hook name `renamed`, a subsequent `cargo berth init` leaves both unchanged, ambiguous or unproven targets change neither, and configuration-write failure or a concurrent trunk edit never installs a hook that disagrees with the authoritative configuration.

Surfaced by the Phase 8 state-and-consequence audit and confirmed independently
by the Phase 8 architect review; reproduced during phase 8 smoke.

## Pair entered paths with their holders in the incursion observation signature

`RetainedReservationSet::observe_incursion` takes `foreign_reservation_ids:
&ForeignReservationIdSet` and `paths: &IncursionPathSet` as two independent
parameters, and `IncursionIncident` stores them the same way. The true relation is
per path: each entered path is blocked by the holders that actually claim it. Two
independent sets cannot express that, so a caller passing the union of several
paths' holders type-checks while recording a combination no single path exhibits.

That defect shipped. `drift/classification.rs` accumulated every entered path's
blockers into one list and reported them under one holder set. Because
`incursion_path_coverage` decides coverage one path at a time and requires each
observed holder to appear in the retained incident, an already-answered path
stopped matching its own incident as soon as an unrelated path contributed a new
holder — and was raised again. Journal evidence: `crates/cargo-berth/tests/board.rs`
was answered twice against one holder, then raised a third time under incident
`01a0492e-84a1-7ae1-a0c8-d4c01db1c36c` once `Cargo.lock` added a second holder to
the same drift run.

The repair grouped entered paths by their own holder set before observing, so the
flat pair is accurate by construction. The signature still permits the invalid
combination; only the caller's discipline prevents it, and
`observe_incursion`'s own contract — an answered path stays answered — silently
fails for any future caller that passes a union.

Replace the parameter pair and the retained incident's two fields with one type
carrying each path alongside its blocking holders, so a union is unrepresentable
rather than merely unwritten. This is a journal record-shape change: the existing
answered incidents store paths and holders as independent arrays and must remain
replayable, so the change carries a migration or a versioned reader.

Acceptance replays a journal containing pre-change incidents alongside
post-change ones, confirms coverage decisions are unchanged for both, and shows
the union combination no longer compiles.

Surfaced while investigating why an answered incursion kept reappearing, after
the Phase 14 checkpoint.
