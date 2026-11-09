# Conversation activity and detached runs

## What it is

Each conversation row shows one activity word.
The census combines conversation events, live detached runs, and compaction markers before rendering.
A worker can remain visible while a detached run is its only live source.

Paths below are relative to `crates/cargo-handler/src/`.

| File | Responsibility |
| --- | --- |
| `census/activity.rs` | Activity vocabulary, transcript activity, detached records, and live process evidence. |
| `census/compaction_markers.rs` | Compaction markers and native completion records. |
| `census/scan.rs` | Worker joins, activity precedence, and missing worker rows. |
| `census/tree.rs` | `RunThread` rows and process folding. |
| `census/tier.rs` | Retained rollout state and conversation prompts. |
| `census/transcript.rs` | Incremental records and subagent roles. |
| `census/probe.rs` | Activity compatibility fields. |
| `agent_cell.rs` | Status text, countdowns, and Astra status precedence. |

## How it works

### Activity resolution

`AgentActivity` is `Unreported`, `Idle`, or `Active(Activity)`.
`Activity` includes thinking, compacting, writing, reading, git, delegating, web, running, asking, waiting, and verification.
`VerifyStep` names fmt, clippy, nextest, build, check, doc, mend, and taplo.
`RunLocation::Mac` adds the displayed Mac suffix.
Status sorting uses the displayed activity text.

Claude activity comes from its transcript's tool and turn records.
Codex activity comes from rollout function calls and turn records.
A subagent uses its own transcript.
An open turn without a later tool word shows `thinking`.
A completed or aborted turn shows `idle`.
`ServiceTier::Fast` adds `fast`; other tier states add no suffix.

A live detached run normally supplies the verification step or `waiting` before ordinary conversation activity.
Among competing runs, the latest start wins, with supervisor pid as the tie breaker.
An active compaction marker takes precedence over live-run activity.
This rule applies to ordinary worker writes, `AstraMembership::activity`, and `TreeLine::status`.
Astra pane idle or unknown state cannot erase a live run.

### Detached records and worker rows

`RunRecords` caches records under discovered session `mesh_runs` directories.
`RunRecord::{Ignored, Running(DetachedRun)}` retains running records with valid live supervisor evidence.
The supervisor must name the exact record.
A command need not be a recognized verification command.
`RequestedStep::Unknown` shows `waiting`.
A script hidden inside `bash -c` does not acquire a verification step from its filename.
A completed record releases its activity.

`WorkerReference::{Staff, LegacyThread}` distinguishes stable staff IDs from direct legacy thread IDs.
`RunWorkerReference::{Identified, NamedSeat}` also accepts run records that name only a seat.
`LiveWorkerRun::worker` resolves `RunWorkerIdentity::{Session, NamedSeat, Unrecorded}` once for the scan.
A named seat must match one worker under the run's checked director.
The recorded owner in `progress_history_state.json` and current staff membership supply that director.
The legacy director file remains a fallback.
Claude and Codex rows can both be directors.

`WorkerRun`, `RunThread`, and `append_run_threads` preserve a missing worker while its run remains live.
They add only identities that other live sources have not supplied.
Run supervisors and command processes fold into the worker row.
The director keeps its own activity.
`VerificationProcesses::live_runs` shares descendant indices rather than repeatedly walking each process subtree.
Removed records and departed sessions leave the cache.
See [hierarchy](hierarchy.md) for unit candidate checks and dispatch pairing.

### Compaction

`CompactionMarkers` reads `~/.local/state/compaction/<sid>.json`.
The hook producer owns marker creation and deletion.
cargo-handler never deletes a marker.
The payload identifies the session, family, and start time.
`CompactionRead::MARKER_BYTE_LIMIT` rejects marker payloads larger than 4096 bytes.
`CompactionMarkerStatus::{Closed, Open { started }}` is shared by transcript discovery and activity resolution.

A matching open marker starts `Activity::Compacting`.
A missing marker or a native completion record after its start ends that activity.
`CompactionCompletion`, `CompactionLineKind`, and `CompactionEnd` retain completion meaning without boolean flags.
`find_compaction_transcripts` finds Codex worker rollouts that the ordinary thread cache does not retain.
It shares the per-scan marker state.
Codex home discovery uses the current user's `~/.codex*` homes.
A rejected repeated compact can leave a marker until the next prompt clears it.

### Subagents and timers

`SubagentRole` is a closed enum.
It includes Writer, Reviewer, Impl, Test, Fix, Capture, Lint, Survey, Plan, Explore, Showrunner, Agent, and Unrecorded.
The metadata agent type wins before a named teammate or description token.
Tokens match whole words with supported inflections.
A generic description containing only `agent` supplies no role.
Unknown metadata can still identify a generic Agent.
`SubagentParent::{OwningSession, RecordedSubagent}` preserves nested subagents.
An absent or finished parent returns the subagent to its owning session.

`ShellPurpose::{Timer, Command}` comes from `ShellTimerObservation`, rather than the displayed command or description.
Only `ObservedCountdown` gives the role `timer`.
`ShellState::CountingDown(PositiveRemainingTime)` draws remaining digits.
Expiry changes the status to `running` with the Busy ink.
An untimed row named `sleep 300`, or `sleep infinity`, has no timer role.
The type column identifies shell rows once.

### Wire compatibility

Probe schema 3 keeps existing enum forms readable by older receivers.
A compacting activity serializes as legacy `thinking` plus `compacting: true`.
The current decoder restores `Activity::Compacting` from that extension.
The same compatibility rule applies to Astra run activity.
Subagent role data travels beside the unchanged `subagent` child kind.
Legacy specialist strings convert into the closed role enum at the wire boundary.

## Invariants and gotchas

1. Attribute detached work through the run record, rather than the worker's process ancestry.
2. Require a live supervisor that names the exact record.
3. Retain unknown commands as waiting runs.
4. Apply compaction precedence in all three activity consumers.
5. Preserve one visible row for each resolved worker identity.
6. Add compatible probe extensions instead of new legacy enum values.

A staff row alone does not create a live worker.
A legacy run without staff membership cannot recover membership from the removed mesh roster.
Activity readers can grow their cold tail to find decisive records.
Model metadata readers use separate fixed bounds when effort is absent.
See [startup](startup.md) for those read contracts.

## Why

Detached processes can outlive their launch call and lose process ancestry to the worker.
Records preserve attribution across that separation.
Compaction uses explicit marker and completion evidence because ordinary thinking records cannot distinguish it.
