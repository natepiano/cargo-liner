# Accounts table

## What it is

The summary shows stored account usage and role counts from the accounts command.
The command owns the numbers and role assignments.
cargo-handler formats, sorts, and fits the supplied account groups.

Paths below are relative to `crates/cargo-handler/src/`.

| File | Responsibility |
| --- | --- |
| `census/accounts.rs` | JSON conversion, count arithmetic, monitor, and retained readings. |
| `census/command_inputs.rs` | Revision and time inputs for command reuse. |
| `summary/accounts.rs` | Account groups, fitting, formatting, selection spans, and reset colors. |
| `summary/mod.rs` | Summary layout and the accounts section pin. |
| `sort.rs` | `AccountsColumn` and `AccountsSort`. |
| `interaction.rs` | Pinned viewport mapping and account selection. |

## How it works

### Source and state

`[census] accounts_command` selects the JSON source.
The standard command is `~/.claude/scripts/lib/py ~/.claude/scripts/accounts/accounts.py status --json --stored`.
`AccountsMonitor` runs it on a refresh thread.
An empty command creates no thread.
The UI only drains completed answers through `refresh`.

`AccountsTable` distinguishes no configuration, the first pending read, success, failure, and failure with retained rows.
A failed command or malformed answer retains the last reading when one exists.
The table shows an accounts failure note and supplied `problems` below its rows.

`Reading<T>` distinguishes `Known` from `Unavailable`.
`Credits` distinguishes an amount, unlimited credit, and no reading.
`CreditPermission` records whether the account may spend credits.
`login_ends_at` is optional on the wire; an older report without the key gives `Unavailable`.
A malformed value fails the reading, as a malformed `resets_at` does.
`AccountReadStatus` controls the green or red usage-read mark.
A missing `read_ok` becomes Failed.
A stale or unread account label uses readable subdued ink.
`read_at` still supplies freshness without a visible read-age column.

The monitor checks inputs every 60 seconds.
`InputDrivenCommand` recognizes the standard stored-accounts command in either flag order.
It reuses unchanged results for at most 300 seconds.
File revisions and stored reset or route-expiry boundaries invalidate the answer sooner.
Non-file inputs still require the fallback refresh.
Unrecognized command forms run without this reuse policy.
`hierarchy::run_output` bounds execution to ten seconds and stdout to one MiB.
It suppresses child stderr and terminates the process group on failure.

### Role counts and rows

`AccountRoleUsage` associates each supplied role with `Reading<RoleSessionCounts>`.
`RoleSessionCounts` requires busy, idle, paused, exited, and total as `u64` values.
`RoleCountsWire` accepts extra fields but rejects the retired `shell` key.
The reader ignores the old `starting` and `waiting` keys as extra fields.
Missing or invalid required counts cannot become zero.
`checked_add` reports `RoleCountOverflow`.
The conversion into `Reading<RoleSessionCounts>` makes overflow unavailable.

The columns follow this order:

| Column | Meaning |
| --- | --- |
| Read mark | Usage-read health, without a header or sort key. |
| account | The supplied account label. |
| left | Weekly quota remaining. |
| role | The supplied role. |
| busy | Busy sessions for that role. |
| idle | Idle sessions for that role. |
| paused | Paused sessions for that role. |
| exited | Exited sessions for that role. |
| total | Total sessions for that role. |
| resets | The stored reset time. |
| login ends | The date the stored Claude login ends, in the `resets` date form. Codex rows and rows with no value are empty. |
| credits | Available credits. |
| enabled | Credit permission. |

`AccountGroup` draws account details on its first role row.
Further rows contain only the role and its counts.
An account without roles still has its account row.
All role rows share one selectable account span.
Sorting moves the whole account group.
The final total row sums supplied counts and has no selection identity.
Counts never come from a second census calculation.

`AccountsSort::SourceOrder` preserves source order until a header is selected.
`ColumnSort<AccountsColumn>` toggles direction on a repeated selection.
Missing readings stay last in both directions.
Equal values preserve source order, including positive and negative zero.

### Fit, pinning, and reset shades

One `AccountsFit` selects columns, widths, and `AccountsLayout::{Full, TwoLine, Stacked}`.
It drops enabled, then login ends, then credits, before shortening role text to a six-cell floor.
It then chooses TwoLine or Stacked if Full cannot fit.
The account, quota, role, counts, and reset information remain.
`AccountLines` supplies the same layout to height, draw, selection, and header lookup.
Wrapped text keeps words and numbers whole.
Separators remain with the preceding word.
Stacked layout adds no separate sortable header.

`SectionPin` keeps the accounts heading and applicable header lines above the scrolling body.
`ViewportMapping` serves drawing, navigation, selection, and clicks.
Pinned headers sort; the pinned heading selects no account.
A wrapped account's entire visible span receives its selection band.

Distinct reset timestamps determine rank.
`ResetRank::positions` gives equal times the same shade.
`ResetRank::color` uses `reset_nearest` for the first quarter and `reset_farthest` for the last quarter.
The middle uses their sRGB midpoint.
A single reset uses `reset_nearest`.
`readable_status_style` corrects contrast after rank resolution.
The failed-read mark keeps the error color.
Missing reset times stay blank and do not change ranks.
See [colors](colors.md) for contrast and palette constraints.

## Invariants and gotchas

1. Preserve the accounts command as the source of counts and roles.
2. Keep continuation rows with their account during sorting and selection.
3. Use the same fitted lines for height, drawing, and input.
4. Keep unavailable readings distinct from zero.
5. Preserve pinned headers when the body scrolls.

A full single-line accounts summary can require 90 columns.
Narrower widths can require a different layout without dropping accounts.
Quota below ten percent uses the readable warning ink.
Reset times use local weekday and time formatting with minute rounding.
Changing the accounts column set requires no saved-sort migration; accounts sorts are not persisted.

## Why

Stored JSON avoids parsing board text or recreating account policy.
Owned fitted lines prevent wrapped tables from disagreeing with their height and click regions.
Three reset steps provide clearer rank differences than a continuous ramp with many similar shades.
