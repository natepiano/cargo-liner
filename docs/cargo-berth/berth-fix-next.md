# berth-fix — next items

Work this plan surfaced but does not implement. Each item names what it changes
and the evidence that produced it.

## Bound the ledger projection so it stops carrying the whole journal

`ledger/projection.rs:39` stores `events: Vec<JournalEvent>` — a full copy of the
journal — and `Projection::from_replay` clones every event on each publish, which
then serializes and fsyncs on every reconciliation. Reconciliation runs on the
`PostToolUse` path, so the per-edit cost of publishing grows with the total number
of journal events for the life of the repository.

The growth is inherent to the append-only journal and predates this plan, but
Phase 3 raises the rate: `ScopedPatchEquivalenceChecked` records one event per
`(reservation, trunk target)`, and unlike `EvidenceRevalidated` it fires even when
the reported status does not change. A repository whose trunk advances often
therefore accumulates one record per retained reservation per advance.

No phase owns this. Phase 3's invariant (`berth-fix.md:90`) constrains git
subprocess counts, not ledger size, and its bounded per-reservation retention
cannot reach durable state. The work is journal compaction or a projection that
stores replayed facts instead of raw events — a change to `ledger/`, not a repair
to any phase's diff.

Surfaced by the Phase 3 closure review, pass 5.
