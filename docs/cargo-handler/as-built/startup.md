# Startup, census inputs, and session names

## What it is

cargo-handler draws a scanning frame before census answers arrive.
The census reads process, account, conversation, and window data outside rendering.
It retains readers between scans to avoid repeated process discovery and whole-file reads.

Paths below are relative to `crates/cargo-handler/src/` unless they start with `crates/`.
See [hierarchy](hierarchy.md) for staff ownership and publication.
See [activity](activity.md) for activity precedence and detached runs.
See [tile layout](tile-layout.md) for redraw decisions.

## How it works

### Startup and scheduling

`terminal::run` initializes `startup_log`, loads configuration, installs the theme, and reads system appearance.
It builds `App` and creates `Ticker` around the census receiver.
The appearance read completes before the first frame.
The first frame can show each configured machine as `scanning`.

`census::schedule::spawn` receives `RemoteMachines`, `StaffFileSetting`, `AccountLabels`, `CodexStartWindow`, and `HerdrReadTimeout`.
It creates one shared `StaffFileReader` and configures `LocalScanner` with it.
The scheduler dispatches due remote probes before starting a due local scan.
Each remote has its own probe thread.
`InFlight` prevents concurrent probes of one host.
Local scans run one at a time on a scan worker.
The local cadence is two seconds; the remote cadence is five seconds.
Remote settings use a shared snapshot, so added hosts enter a later round.
Answers from removed hosts do not restore their rows.

`LocalScanner::scan_staged` reads sessions and processes, refreshes Codex log evidence, and takes one staff snapshot.
It classifies rows and attaches proven pane identities before publication.
Only the first scan publishes incomplete classified rows.
Detail reads then attach transcripts, tiers, children, run activity, desktops, and branches.
The scanner applies staff names after fallback names in both publications.
`Ticker::drain_census` applies updates and requests a new body only when displayed state changes.

### Accounts and process identity

`[accounts]` is an `AccountsConfig` map from configuration homes to display labels.
`AccountLabels::new` normalizes homes against the scanning machine's home directory.
`AccountLabels::resolve` constructs `AccountHome` with a matching home and label.
Without a configured label, the last path component supplies the label without its leading dot.

`ConfigHome::{Unread, Default(Program), Named(PathBuf)}` records process environment evidence.
Claude uses `CLAUDE_CONFIG_DIR`; Codex uses `CODEX_HOME`.
An empty variable selects the program's default home.
Unread process environments remain unknown.
`Account::{Unrecorded, Home(AccountHome)}` carries the result on census rows.
Shell and subagent display accounts use the owning conversation's account.
Hosted Codex threads keep their own account.

Each machine resolves its own labels before sending a probe.
The wire represents an account as null or a home-and-label object.
Missing account fields decode as `Unrecorded` under schema 3.
The `[accounts]` map loads at startup.
Changes require a restart on the machine that owns the rows.

`AgentRow::started` can move to the transcript's earlier session start.
`process_started` remains the process start.
`AgentCell { machine, pid, process_started }` therefore survives detail attachment and distinguishes reused pids.
Older probes without `process_started` use `started` at the wire boundary.

### Codex names and account boundaries

`census/codex.rs` reads each account's thread database and its log database.
`ThreadsLayout` selects the supported database layout once per open.
Interactive rows use `thread_source` when available, with legacy originator handling for older layouts.
By-ID requests accept worker threads as well as interactive threads.

`CodexLogs` reads new log records incrementally from the highest numbered `logs_<n>.sqlite`.
`ConfirmedSelection` accepts connection-specific start, resume, or shell-snapshot evidence.
The reader resolves thread IDs through the account's database.
An unproven shared-daemon connection supplies no thread name.
Argument text and a start-time window alone cannot establish that connection.
Pending pid records remain until a later process snapshot can check their lifetime.
Replaced thread databases reopen.

Each `CodexThread` retains `CodexAccountHome` from its source database.
`thread_matches_home` compares that home with the process account.
A shared rollout path does not transfer account ownership.
`SessionNames` uses the thread's own `session_index.jsonl` rename, then its database name.
`SessionName::{Named, Unnamed}` separates absence from a recorded name.
The configured `codex_thread_start_window` defaults to 60 seconds for the remaining discovery paths.
It does not replace identity proof.

`census/herdr.rs` obtains the socket from `HERDR_SOCKET_PATH` in the pane process environment.
A subscription worker maintains a shared pane snapshot.
The scanner links a Codex process through the foreground pid or pane-shell ancestry.
A pane with multiple Codex windows supplies no fallback name.
Name precedence is staff name, proven thread name, herdr pane name or tab label, then pid.
Only terminal titles lose the ` | ` suffix.
`attach_codex_names` applies the resolved fallback names before staff names.

### Probe waits

`LocalScanner::scan` creates one `HerdrProbeDeadline` before scanning.
It keeps the completed answer and waits only for its own herdr generation.
A missing herdr service ends the wait immediately.
The TUI uses `scan_staged` and does not wait for an initial herdr answer.

| Setting | Default | Bound |
| --- | --- | --- |
| `census.herdr_probe_wait` | 12000 ms | Zero through 18000 ms. |
| `census.herdr_read_timeout` | 1000 ms | Positive values through `PROBE_TIMEOUT`. |
| `PROBE_TIMEOUT` | 20 seconds | Includes scan and SSH allowance. |

The deadline bounds the added wait, rather than total scan duration.
A scan that exceeds it accepts only an already queued answer.
The probe writes refused configuration diagnostics to stderr and still emits one JSON report on stdout.

### Incremental conversations and metadata

`JsonlCursor::{NeverRead, AfterHead, Following}` retains read position and incomplete bytes for each conversation path.
`read_until` returns complete records and actual traffic through `ReadBatch`.
Cold activity tails start at 64 KiB and grow backward by four until their decisive fields exist.
The ranges do not overlap.
`RolloutRead` reads the head only until session metadata and the first task are known.
Later reads consume appended bytes.
Truncated files reset their cursors.
No fixed tail cap may silently discard a required activity value.

Model metadata follows a different bounded rule.
`TranscriptModels` reads at most a 64 KiB Claude tail or a 64 KiB Codex head and tail initially.
It never searches backward until an optional effort field appears.
Unchanged `FileRevision` values require no read.
Replacement or truncation resets through `RecordChange::Replaced`.
The decoder skips message bodies.

`ConversationModel` supplies `ModelName` and `ReasoningEffort` on agent and child rows.
Known model names include Opus, Sonnet, Haiku, Fable, Astra, and Sol.
Effort values are Low, Medium, High, XHigh, Max, and Unrecorded.
Claude records use assistant or system metadata.
Codex records use `turn_context` or `thread_settings_applied` metadata.
Values belong to each conversation, including nested subagents.
`Unrecorded` fields remain blank and can be absent from probes.
`ShortModelName` checks unknown model names before display.

`OpenThreadRows::read_scan` collects complete sets for `ThreadRequester::{PaneWorkers, AppServer, Astra}`.
Each account home is read once for the union of live IDs.
Empty sets release old requests.
Database or WAL changes and new IDs trigger queries.
Departed IDs and homes leave the cache.
`ThreadRequester::Compaction` serves the fallback `CompactionThreads` cache for workers without retained pane rollouts.

### Watches, timing, and theme notes

The scanner retains process rows and refreshes changed identities and details.
Same-pid exec changes must update names.
Linux uses process evidence; macOS also uses libproc for process revisions.
Directory watches cover transcript subagents, seat documents, and run records.
Watch failure uses a 30-second recovery scan and a two-second retry.
Process recovery also runs every 30 seconds.
`retain_session_caches` removes departed run, subagent, and worker-directory state.
Live dispatch directories remain retained even when they have no run records.

`CARGO_HANDLER_FRAME_LOG` enables append-only milestone and stage timing through `startup_log.rs`.
Events distinguish publication from UI application.
`StartupProbe` records the first flushed frame and attract identification.
An unset variable disables timing work.

`ThemeInstallation` and `StartupThemeNotes` retain missing-theme, rejected-file, and appearance diagnostics.
Later appearance answers do not erase rejected-file notes.
Theme installation accepts `ThemeSource::{BuiltinsOnly, Directory}`.
Shared `LoadedConfig` rewrites merge known values into the TOML document.
Unknown keys, comments, ordering, and value decoration remain.
Malformed files remain untouched.
Theme-note fixtures own a `TempDir` through normal completion and unwind.
They do not retain temporary folders in statics.

## Invariants and gotchas

1. Keep process discovery and conversation reads outside rendering.
2. Preserve one account database boundary for each Codex thread.
3. Keep model metadata reads bounded when fields are absent.
4. Supply every requester's complete set to the shared thread read.
5. Release watches and cached records when their owners leave.
6. Fix both grid time and status uptime in paired frame tests.

`draw_at_for_test` fixes both clocks for frame comparisons.
The shared `census-test-support` fixture provides portable same-pid exec evidence.
Render fixtures use controlled desktop sources to avoid real desktop queries.
Large metadata fixtures must exceed the asserted byte budget without needing multi-megabyte padding.
Temporary Astra databases disable SQLite synchronous durability in their test fixture.
These fixtures preserve all behavior assertions.
On Apple Silicon, CPU measurements must convert `proc_pid_rusage` ticks through `mach_timebase_info`.
Background children use null or piped stderr.
External editor and shell launches deliberately inherit the terminal.

## Why

Early classification makes agents visible before slower details arrive.
Retained cursors and watches reduce repeated work without changing census results.
Account ownership and connection proof prevent names from leaking between windows or accounts.
Bounded metadata reads prevent optional fields from causing whole-file scans.
