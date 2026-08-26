# cargo-berth operations

Identity resolution, bypass auditing, and recovery from a damaged ledger.
Reach for this when something has gone wrong or when wiring `cargo-berth` into
a harness.

For what the tool is and how to use it, see the [README](../../crates/cargo-berth/README.md).

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

## Bypass audit

`CARGO_BERTH_BYPASS=1` permits a trunk update no matter what other gate input is
broken. The override is tested before the tool reads the config, journal, or Git
evidence. The tool records an audit fact in the journal when writable, or writes
a pending marker for a later session to recover. The fact names
`CARGO_BERTH_BYPASS=1` and when it was taken. It has no reason and names no
specific hold that it skipped, because nothing else had been read yet. If
neither destination is writable, the tool warns and still permits the update.

## Recovery

`init` has three branches:

- Plain `cargo berth init` creates a missing ledger and default config and
  installs or refreshes each managed hook without replacing an unmanaged hook.
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

A malformed v1 record looks like this and is the case for confirmed
reinitialization:

```text
The reservation ledger could not be read: journal replay failed: journal record 1 is corrupt: missing field `event_id` at line 1 column 44
[exit 4]
```

A record from a newer schema is different:

```text
The reservation ledger could not be read: journal replay failed: journal schema version 2 is unsupported
[exit 4]
```

Upgrade `cargo-berth` for an unsupported schema version. Never reinitialize that
journal merely because the current executable is older.

`resolve` records one of these explicit decisions:

- `--recovered` rebinds a reservation to the worktree running the command.
- `--integrated-as <trunk-oid>` records a verified alternate commit already
  reachable from trunk.
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

