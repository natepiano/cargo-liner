# berth-fix — next items

Work this plan surfaced but does not implement. Each item names what it changes
and the evidence that produced it.

## Bound the ledger projection so it stops carrying the whole journal

`ledger/projection.rs:40` stores `events: Vec<JournalEvent>` — a full copy of the
journal — and `Projection::from_replay` clones every event on each publish, which
then serializes and fsyncs on every reconciliation. Reconciliation runs on the
`PostToolUse` path, so the per-edit cost of publishing grows with the total number
of journal events for the life of the repository.

The growth is inherent to the append-only journal and predates this plan, but
Phases 3 and 4 raise the event rate. `ScopedPatchEquivalenceChecked` records one
event per `(reservation, trunk target)` even when the reported status does not
change. `SuccessorScopedPatchEquivalenceChecked` records one event per
`(predecessor, proof subject, successor head)`, and
`SuccessorScopedPatchComparisonAttempted` records every transient successor
comparison that cannot be cached. The successor cache's 512-entry verdict and
attempt retention bounds replayed state; it does not bound durable journal or
projection bytes.

Phase 6 also raises the serialized size of every new event by attaching
`identity_inputs`: the invocation directory and four environment values each
retain at most 256 JSON-content bytes, or a bounded state carrying only an
observed byte count. These inputs are durable audit evidence for the unresolved
worktree-attribution incident. They remain in the append-only journal for its
lifetime, but they are not replay state and must not be copied into a bounded
projection.

No phase owns this. The plan-wide invariant (`berth-fix.md:90`) constrains Git
subprocess counts, not ledger size, and bounded per-reservation replay state
cannot bound a projection that stores every `JournalEvent`. Replace
`Projection.events` with bounded replay state while retaining the journal offset,
fingerprint, generation, and enough materialized facts to preserve replay
behavior. Do not rewrite or compact the append-only journal.

Acceptance compares projection publication and reload from a small journal and a
long journal carrying equivalent live state: serialized projection size and
publish work remain bounded by live replay state rather than event count, while
the journal retains every Phase 6 `identity_inputs` record unchanged.

Surfaced by the Phase 3 closure review, pass 5; strengthened by the Phase 4
retrospective and Phase 6 identity instrumentation.

## Make `maximum_reservations` truthful against the successor-verdict retention bound

`config.rs:158` parses `maximum_reservations` as an unrestricted `u32` and
`ParsedConfigValues::finish` applies it with no upper bound. A predecessor's
successor-equivalence cache and comparison-attempt history each retain at most
`SUCCESSOR_SCOPED_PATCH_TARGET_RETENTION_LIMIT = 512` entries
(`reservation/constants.rs:8`), evicting oldest-first at `reservation/mod.rs:257`
and `:325`. Nothing ties the two numbers together.

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
