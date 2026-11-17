# Session states

## What it is

The accounts command reports the state of each session.
cargo-handler reads only those session states from its answer.
The status column uses them, and the hierarchy notes report the health of the source.
The handler shows no accounts table.
The hana console accounts page shows account usage.

Paths below are relative to `crates/cargo-handler/src/`.

| File | Responsibility |
| --- | --- |
| `census/accounts.rs` | `SessionStates` decoding, `AccountsSource`, and `AccountsMonitor`. |
| `census/command_inputs.rs` | Revision and time inputs for command reuse. |

## How it works

### Source and state

`[census] accounts_command` selects the JSON source.
The standard command is `~/.claude/scripts/lib/py ~/.claude/scripts/accounts/accounts.py status --json --stored`.
`AccountsMonitor` runs it on a refresh thread.
An empty command creates no thread.
The UI only drains completed answers through `refresh`.

`AccountsSource` has five states:

| State | Meaning |
| --- | --- |
| `NotConfigured` | The command is empty. |
| `AwaitingFirstReading` | The first answer has not come. |
| `Ready` | The last answer decoded. |
| `Failed` | No answer decoded yet, and the last read failed. |
| `FailedKeepingLast` | A read failed after a good answer. The good answer stays in use. |

`AccountsSource::session_states` gives the usable states, or an empty map.
`AccountsSource::health` gives the source health to the census.
A failure keeps its reason, and the hierarchy note shows it.

### Decoding

`SessionStates::parse` decodes the report once at the command boundary.
It reads `accounts[].sessions[]`, each a `{session, state}` pair.
It ignores every other field of the report.
An empty session ID or malformed JSON fails the reading.
A session that two accounts report in different states has no state.

`SessionState` has four values: `busy`, `idle`, `paused`, and `exited`.
The reader reads an old `starting` or `waiting` word as `busy`.
`ReportedSessionState::Unavailable` marks a session with no entry.
A missing entry never becomes `idle`.
A listed Codex process with no recorded thread reads as `idle`.

### Refresh

The monitor checks inputs every 60 seconds.
`InputDrivenCommand` recognizes the standard stored-accounts command in either flag order.
It reuses unchanged results for at most 300 seconds.
File revisions and stored reset or route-expiry boundaries invalidate the answer sooner.
Non-file inputs still require the fallback refresh.
Unrecognized command forms run without this reuse policy.
`hierarchy::run_output` bounds execution to ten seconds and stdout to one MiB.
It suppresses child stderr and terminates the process group on failure.

## Invariants and gotchas

1. Keep the accounts command as the one source of session states.
2. Keep an unavailable state distinct from `idle`.
3. Keep the last good states across failures.

Account labels and their ink do not come from this source.
`[accounts]` in the config maps homes to labels; see [startup](startup.md).
`FooterColors::account` gives the label ink; see [colors](colors.md).

## Why

The accounts command owns the session classification.
Rust reads its result without repeating its rules.
The hana console accounts page replaced the summary accounts table.
