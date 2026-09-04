# berth-fix — next items

Work identified but not built. Each item names what it changes and the evidence
that produced it.

## Make `maximum_reservations` match the successor-verdict retention bound

`config.rs:158` parses `maximum_reservations` as an unrestricted `u32` and
`ParsedConfigValues::finish` applies it with no upper bound. A predecessor's
`RetainedSuccessorScopedPatchTargetVerdicts` and
`SuccessorScopedPatchTargetEvaluationSchedule` each retain at most
`SUCCESSOR_SCOPED_PATCH_TARGET_RETENTION_LIMIT = 512` entries
(`reservation/constants.rs:9`), evicting oldest-first at
`reservation/mod.rs:262-264` and `:330-332`. Nothing ties the two numbers
together.

Configure `maximum_reservations` above roughly 513 and a single predecessor can
carry more live successor heads than either retained structure can cover.
Verdicts for stable heads are then evicted and recomputed, and because the
round-robin admits one cold comparison per reconciliation, the set of proven
successors never closes: at least one edge keeps reporting
`awaiting_successor_incorporation` on every pass. That is the conservative
direction and never a false release, but at that configuration the permanent
block the successor-equivalence proof exists to close reopens.

Unreachable at the shipped default of 128, which allows at most 127 successors
per predecessor. The work is a validated configuration bound — reject or clamp a
`maximum_reservations` the retention limit cannot serve, and say so in the config
error — not a change to either retained structure, whose boundedness is required.
`config.rs` owns it.

Verified against the live tree.

## Keep the configured trunk and managed hook synchronized after a proven trunk rename

The detached refresh replaces the managed `reference-transaction` hook after a proven trunk rename but leaves `.claude/config/berth.toml` naming the deleted branch, so a later `cargo berth init` silently restores the stale hook value.

When the detached refresh finds one uniquely proven rename target, atomically compare-and-replace the configured `trunk` from the deleted branch to that target before installing the refreshed hook. Do not overwrite a concurrent configuration change; if configuration replacement or hook installation fails, retain the previous hook, emit a diagnostic, and preserve its stale-reference fail-safe.

Acceptance renames `main` to `renamed` and proves both `berth.toml` and the managed hook name `renamed`, a subsequent `cargo berth init` leaves both unchanged, ambiguous or unproven targets change neither, and configuration-write failure or a concurrent trunk edit never installs a hook that disagrees with the authoritative configuration.

Confirmed by two independent reviews and reproduced in smoke testing.

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
rather than merely unwritten. This is a journal record-layout change: the existing
answered incidents store paths and holders as independent arrays and must remain
replayable, so the change carries a migration or a versioned reader.

Acceptance replays a journal containing pre-change incidents alongside
post-change ones, confirms coverage decisions are unchanged for both, and shows
the union combination no longer compiles.

Surfaced while investigating why an answered incursion kept reappearing.

## Publish the engine and its wrappers as one atomic version

Every Claude session can execute a berth hook while an installation is being
refreshed. Publishing each file with its own rename prevents partial file
contents, but still permits a live invocation to reach an engine from a
different contract version than the one its run started against.

A wrapper holds no contract knowledge — each is a `PATH` check plus
`exec cargo-berth hook <event>` — so there is nothing for it to disagree with
the engine about. What has to publish atomically is the `cargo-berth` binary
itself, the three wrappers, `install.sh`, and the Python test suite.

Stage and validate one immutable versioned bundle holding those files, and edit
or remove nothing inside a published bundle. Registered hook paths remain stable
bootstraps outside the bundles: each reads the active bundle identifier exactly
once, resolves it to an absolute path, and executes the engine from that same
captured bundle. Publish by atomically replacing the single active bundle
identifier in the same directory, retaining the previous bundle while an
invocation may still hold its path and for rollback.

Acceptance holds old hook invocations open across publication and starts
concurrent new ones throughout it. Every invocation observes either the complete
old bundle or the complete new bundle; none observes partial contents or a
wrapper/engine version mixture. Failures before the active-version switch leave
the old bundle active, and failures after it can restore the previous identifier with one
atomic replacement.

Surfaced when an install rewrote the canonical PostToolUse wrapper while three
sessions were live.
