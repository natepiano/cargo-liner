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

## Refuse to install a managed hook that points inside a `target/` directory

`init` writes the resolved path of whichever executable invoked it into both
managed hooks. Run it once from a development build and both hooks hard-code
that build's path — `<worktree>/target/debug/cargo-berth`.

`.git/hooks` lives in the common git directory, so it is shared by every
worktree of the repository. A development build in one worktree therefore
becomes the hook binary for all of them, and nothing reports that it happened:
while the development build agrees with the installed one, the hooks look
correct.

The divergence surfaces as an unrelated failure. When the bounded-projection
work removed `events` from the projection and raised the projection schema
version, the first commit after that build landed wrote the new projection into
the shared ledger, and every session still on the installed binary failed with a
deserialization error naming a missing field. The cause is the hook target, not
the format change; nothing in the message points at it.

Prefer a stable installed path when one resolves, and refuse to install a hook
whose target lies inside a `target/` directory unless an explicit flag opts in.
Report the chosen hook target in `init`'s payload either way, so a development
target is visible at the moment it is installed rather than at the moment it
breaks a sibling worktree.

Acceptance installs hooks from a `target/debug` executable and confirms the
install is refused with the target named, confirms the same install succeeds
under the opt-in flag, and confirms `init` reports the hook target it wrote.

Surfaced by diagnosing two shared-ledger outages during the Phase 15 checkpoint;
both were the drift-split worktree's debug build running as the repository's
commit hook.

## Publish the engine, shims, and generated consumers as one atomic version

Every Claude session can execute a berth shim while an installation is being
refreshed. Publishing each file with its own rename prevents partial file
contents, but still permits a complete shim to read a validator or invoke an
engine from a different contract version.

Stage and validate one immutable versioned bundle containing the `cargo-berth`
binary, every registered shim, and every generated consumer artifact, including
`generated/envelope_validation.jq` and `generated/status_payload_tables.py`.
Nothing inside a published bundle is edited or removed in place.

Registered hook paths resolve a stable launcher. The launcher reads the active
bundle identifier exactly once, resolves that bundle to an absolute path, and
executes its shim; the shim reads generated consumers and invokes `cargo-berth`
from that same captured bundle. Publish by atomically replacing the single active
bundle identifier in the same directory, retaining the previous bundle while an
invocation may still hold its path and for rollback.

Acceptance holds old invocations open across publication and starts concurrent
new invocations throughout it. Every invocation observes either the complete old
bundle or the complete new bundle; none observes partial contents, a missing
generated directory, or a shim/validator/engine version mixture. Failures before
the active-version switch leave the old bundle active, and failures after it can
restore the previous identifier with one atomic replacement.

Surfaced when a Phase 16 timing run edited the canonical PostToolUse shim while
three sessions were live.
