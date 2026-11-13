# Agent hierarchy and worker identity

## What it is

cargo-handler joins live sessions to the staff forest.
Managed sessions and worker threads appear under their managers across the displayed machines.
The census owns liveness; the staff file owns names, roles, and management links.

Paths below are relative to `crates/cargo-handler/src/` unless they start with `crates/`.

| File | Responsibility |
| --- | --- |
| `census/hierarchy.rs` | Staff parsing, validation, membership, and hierarchy publication. |
| `census/session_identity.rs` | Proven Claude, Codex, and Astra session identities. |
| `census/placement.rs` | Cross-machine placement, role labels, and dispatch pairing. |
| `census/worker_dispatch/` | Launcher parsing, unit ownership, worker names, and selection keys. |
| `census/scan.rs` | One staff snapshot for each scan. |
| `census/schedule.rs` | Local publications and remote probes. |
| `census/probe.rs` | Compatible transport of identities, roles, and links. |
| `agent_cell.rs` | Tree construction and agent grouping. |

See [startup](startup.md) for process discovery and names.
See [tile layout](tile-layout.md) for tree columns and summary membership.
See [activity](activity.md) for detached runs and compaction.

## How it works

### Staff source and validation

`[census] staff_file` uses `StaffFilePath`.
Its default is `~/.local/state/staff/staff.json`.
An empty path disables staff discovery.
Settings replace the shared `StaffFileSetting`; the next scan reads the new source.

`StaffFileReader` shares its cache through `Arc<Mutex<StaffFileCache>>`.
It checks the path and modification time before parsing.
An unchanged revision returns the same `Arc<StaffRoster>`.
Each `StaffSnapshot` binds that roster to its `HierarchySource`.
Names and links in one scan therefore use the same revision.

The reader accepts schema 2.
Known kinds are `producer`, `showrunner`, `unit`, `worker`, and `agent`.
Each known row supplies `id`, `name`, `parent`, and `current_session`.
Worker rows can also supply `role` and `home`.
Unknown kinds contribute no rows.

`ValidatedStaffForest` checks identities, parent kinds, and cycles before publication.
Staff IDs and session IDs must each be unique and nonempty.
Producers, showrunners, and agents can be roots.
A producer or showrunner can manage a showrunner, unit, or worker.
A unit can manage a worker.
Missing parents and invalid parent chains reject the whole forest.

`StaffRoster` indexes members by staff ID and by session ID.
`member(session)` returns `StaffMemberLookup::{Unavailable, Root, Managed}`.
`workers(manager)` returns only workers of that manager's current session.
`StaffWorkerRole::{Known(WorkerRole), Unknown}` converts the optional JSON role at the boundary.
Known values are `impl`, `test`, `review`, `fix`, `astra-designer`, and `astra-reviewer`.
Other values publish `HierarchyRole::Worker`.
Worker names do not determine roles.

Missing, unreadable, or invalid staff files supply an empty roster.
Invalid files never publish a partial forest.
Access failures clear the cache for another attempt.
A parsed invalid revision stays empty until its modification time changes.
Local file failure does not retain an older forest or show an error notice.

### Publication and transport

`LocalAnswers::publish_staff` sends the snapshot that supplied the scan's names.
The first scan publishes classified rows before detail reads.
Completed publication also sends the classified-stage update before the completed-stage update.
Production discovery reads the staff file directly.
The asynchronous hierarchy request machinery remains for scheduler fixtures.
It does not imply a configured hierarchy shell command.

`HierarchySnapshot { edges, notice }` keeps links and discovery health together.
`HierarchyNotice` supports `Clear`, `Failed`, and `NoSessionMatched`.
A match can use a scanned manager, member, or hosted thread.
Remote failure handling retains the host's last successful edges.
Removing a remote removes its retained edges.

Probe schema 3 separates visible `rows` from transport-only `thread_hosts`.
`Agent::CodexThreadHost` transports threads without adding a visible agent.
Optional fields include `codex_session`, `astra_member`, `astra_edges`, and `hierarchy_notice`.
The legacy `hierarchy_failed` flag preserves failure handling for older receivers.
Astra edges retain a generic worker edge beside the extension.

### Proven identities and placement

`SessionIdentity` is `Unrecorded`, `Claude`, `Codex(ProvenCodexThread)`, or `Astra(AstraSession)`.
`SessionIdentity::ids()` supplies the identity for staff matching.
`ProvenCodexThread::ResumeArgument` requires `codex resume <full UUID>`.
`OpenStaffRollout` requires exactly one open rollout whose thread belongs to the staff snapshot.
The open-file proof uses `/proc/<pid>/fd` on Linux.
Working directory and start time cannot prove a pane worker's identity.

`codex::resume_request` skips known value options with their values.
It also skips unknown switches and `--name=value` arguments.
New value options must enter `CODEX_VALUE_OPTIONS`.
Otherwise their values can occupy the subcommand position.

`AgentPlacement::from_machines` builds a borrowed forest from the current machine answers.
`Placement` distinguishes `Root`, `Managed`, and `Launched` rows.
Manager lookup accepts proven Codex identities as well as Claude identities.
Unresolved links leave launcher placement or root placement intact.
Conflicting resolving links use deterministic manager order.
Cycle checks restore launcher placement and rescue otherwise unreachable roots.

`ManagedThread` moves a hosted thread under its manager once per thread ID.
Placement prefers a visible owner over a transport carrier.
A thread claimed by placement leaves the owner's ordinary work list.
`RoleLabel` distinguishes stated roles, inferred manager roles, `ReviewFallback`, and unlabeled sessions.
Accepted incoming roles take precedence over inference.
Plan-mode Claude processes use `ReviewFallback` only without a stronger mapping.

### Row columns and details

The tile tree draws account, session, role, activity, and status in that order.
Optional type, model, effort, branch, and directory columns follow.
The session column retains its tree guides.
Recorded accounts remain visible when one account supplies every row.
Narrow tiles remove optional columns before they truncate session names.
Directory, branch, activity, and account leave in that relative order.

`RowRole` limits named roles to `unit`, `showrunner`, `worker`, and `producer`.
Subagents and threads show `worker`.
Shells and unlabeled sessions show an empty role.
Only a producer record in the staff source supplies the producer role.
Activity names the assigned work, including `impl`, `test`, `fix`, and `reviewer`.
A timer shows `timer` and its countdown in Activity.
A compacting session shows `compacting` instead of its assigned job.
The existing census compaction detector supplies that state for every session role.

Each account row supplies `sessions: [{session, state}]` beside its `role_counts`.
The reader merges these lists into one session map.
`SessionState` accepts `busy`, `starting`, `waiting`, `idle`, `paused`, and `exited`.
The external accounts classifier owns these meanings.
Rust reads its result without repeating its classification rules.
Claude rows use their `session_id` string as the lookup key.
Codex and Astra rows use their proven thread ID.
Hosted Codex threads use `ChildRow::thread_id`.
A missing top-level session entry produces `ReportedSessionState::Unavailable` and an empty status cell.
A running Claude subagent shows `busy`.
A Codex row without a thread ID shows `idle` with a live process, or `exited` without one.
Conflicting account entries remove only that session status.
Retained account readings retain their session states after a refresh failure.
A live shell shows `busy` independently of its countdown.
A reported `waiting` status uses the `waiting` theme role in every tile.
The full session heading uses the same ink.
The theme files supply its bright yellow color for dark and light appearances.
Other statuses and the hierarchy activity column keep their existing inks.

Age remains a computed sort value.
Showrunner and unassigned tiles start in session name order.
Other trees and the top-level agents table keep their age defaults.
Neither table draws an age column or an age sort marker.
A name default retains its session header arrow.
The details view groups values into agent, hierarchy, location, and now sections.
Its hierarchy section shows the accepted showrunner, unit, and worker names.
Unassigned sessions and shells show their full name instead.
The details view shows the long age last in the now section.
Detailed state words, such as `thinking` and `clippy`, appear as `state:` in details.

### Dispatch pairing and names

`ParsedDispatch` parses launcher commands during scanning.
The lexer supports bash and zsh quoting, ANSI-C strings, environment prefixes, and supported literal assignments.
Option parsing stops at the first positional job argument.
Unsupported shell work keeps its shell row.
Prompt titles come from `JobText` and the prompt-opening parser, rather than embedded diff text.

`UnitDirectorSource` reads `owner_staff_id`, then `main_agent.session_id`, from `progress_history_state.json`.
The legacy `director` file remains a fallback.
Current staff membership resolves the director's current session.
`StaffWorkers::for_directory` accepts only workers under that checked director.
`WorkerNames` caches candidates per directory and staff snapshot.
`UnitOwnershipRevision` invalidates them when an ownership file changes.
Seat document revisions refresh launcher claims separately.
A directory without a checked director supplies no candidates.

`WorkerIdentity` separates Codex threads from Claude sessions.
Claude seat joins also require the durable worker document and a live claim.
`DispatchPairing` pairs the launcher and worker within the tile.
The launcher disappears only when its exact worker resolves there.
Its kept descendants appear under the worker.
Unresolved, foreign-tile, and other-seat dispatches keep their shell rows.

`ResolvedWorkerDispatch` retains the display seat separately from `DispatchStaffMembership`.
`selection_key()` uses the stable staff ID, with the seat as an older-payload fallback.
Selection therefore survives staff renames and subsequent sorting.
The wire writes legacy roles in `role` and current roles in `staff_role`.
`staff_id` carries the stable identity.
Decode prefers `staff_role`, then the legacy role, then legacy seat inference.

### Astra membership

`AstraMembership` requires a known Astra role and an absolute home in the staff row.
`SessionIdentity::with_astra` attaches it only to the matching proven thread.
Missing membership fields preserve ordinary staff placement and the live session.
They do not remove the worker.
`PaneWork` supplies pane activity; `AstraRunActivity` supplies live detached-run activity.
Closed panes disappear unless another live source retains their rows.
`ThreadRequester::Astra` joins the same per-home thread request pass as other requesters.

## Invariants and gotchas

1. Use one staff revision for names, roles, and links in each scan.
2. Preserve process evidence before assigning a pane thread to a staff member.
3. Keep candidate lookup within the recorded unit.
4. Keep stable selection keys separate from display names.
5. Add new probe roles through extensions that older receivers can ignore.
6. Keep staff membership separate from process liveness.

cargo-handler reads no `mesh_roster.json` or `role_map_cache.json`.
`WorkerReference::{Staff, LegacyThread}` remains necessary for run records.
The legacy thread variant identifies a thread directly; it does not supply deleted roster membership.
Older staff documents without roles show generic workers until their producer supplies roles.

## Why

The staff producer already owns the management forest.
A checked snapshot avoids separate readers that can disagree about names and roles.
Unit-scoped candidates prevent same-name workers in different units from becoming ambiguous.
Explicit process proof prevents one Codex window from taking another window's identity.
