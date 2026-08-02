# Compile Visibility

> **Status: IMPLEMENTATION PLAN — phased, delegate-ready.** Add an opt-in, scope-correct view of live Cargo builds to the existing Output pane, including safe selected-build and scope-wide termination.

## Delegation Context

- **Project:** `cargo-port` — Rust TUI for inspecting and managing Rust projects; `tui_pane` is its in-workspace UI framework crate.
- **Project started:** 2026-08-01T15:16:25-04:00
- **Stack:** Rust 2024; `ratatui` 0.30.2; `crossterm` 0.29.0; `sysinfo` 0.39.6; `cargo_metadata` 0.23.1; `crossbeam-channel`; local `tui_pane` 0.6.0-dev.
- **Layout:** `src/process_observation/` (new shared OS-process observation capability); `src/process_termination/` (new identity-bound signaling capability and nonblocking worker); `src/build_monitor/` (new build discovery, attribution, snapshots, and termination transactions); `src/tui/compile_visibility/` plus `src/tui/workspace_index.rs` (new selected-scope/App adapters); `src/project/` (workspace index and worktree ownership); `src/tui/running_targets/` (existing consumer migrated to shared observer/index); `src/tui/state/`, `src/tui/terminal/`, and `src/tui/app/` (owned-run lifecycle, channels, scheduling, modal transactions); `src/tui/panes/output/` plus pane layout/render/input files (monitor presentation); framework keymap and tests/assets (portable shortcuts and labels).
- **Key files:**
  - `Cargo.toml` — crate metadata and process/TUI dependency versions.
  - `src/main.rs` — top-level module declarations.
  - `src/process_observation/` — new `ProcessObserver`, strong or insufficient identity evidence, named field-observation states, refresh-plan scheduling, and validated ancestry; it never signals processes.
  - `src/process_termination/` — new `ProcessTerminator`, identity-bound platform capabilities, descendant revalidation, and nonblocking bounded signaling worker.
  - `src/build_monitor/` — new build-session classifier, compiler/package attribution, immutable monitor snapshots, opaque actionable authority, and bounded termination transaction ownership.
  - `src/project/cargo/metadata_store.rs` — current `WorkspaceMetadataStore`, workspace roots, target directories, package/source records, metadata generations.
  - `src/project/cargo/mod.rs` and `src/project/mod.rs` — workspace-index exports.
  - `src/project/root_item.rs` and `src/project/git/worktree_group.rs` — checkout/worktree-group membership and canonical live paths.
  - `src/tui/project_list/visible_rows.rs`, `src/tui/project_list/list.rs`, and `src/tui/project_list/mod.rs` — typed selected-row kinds and row-to-project/worktree resolution.
  - `src/tui/running_targets/state.rs`, `src/tui/running_targets/app_tick.rs`, `src/tui/running_targets/constants.rs`, and `src/tui/running_targets/mod.rs` — existing one-second Running-target process consumer to preserve while extracting shared observation/indexing.
  - `src/tui/startup_services.rs` — existing process-poll startup suppression/test effects.
  - `src/tui/app/mod.rs`, `src/tui/app/construct.rs`, `src/tui/app/confirm_action.rs`, and `src/tui/app/async_tasks/poll.rs` — App-owned capabilities, construction, modal confirmation, correlated result reconciliation, pane visibility/focus.
  - `src/tui/background.rs`, `src/tui/messages.rs`, and `src/tui/terminal/processes.rs` — owned Cargo launch, isolated process group, captured output, and session-aware messages.
  - `src/tui/terminal/event_loop.rs` and `src/tui/terminal/frame_metrics.rs` — combined process deadlines/refresh work, event-loop wakeups, and performance accounting.
  - `src/tui/state/inflight.rs` and `src/tui/state/mod.rs` — replace anonymous example-run fields with the sole `OwnedRun` aggregate and monotonic `OwnedRunId`.
  - `src/tui/workspace_index.rs` — shared App adapter exposing current, retained-last-accepted, or uninitialized workspace-index readiness to consumers.
  - `src/tui/compile_visibility/` — new `CompileVisibilityState`, row-kind-aware `MonitorScopeKey`, named scope resolution, scope revisions, generation invalidation, and App-facing actions.
  - `src/tui/app_render_state.rs` and `src/tui/render_context.rs` — monitor/owned-run render borrows.
  - `src/tui/panes/output/mod.rs`, `src/tui/panes/output/pane.rs`, `src/tui/panes/output/render.rs`, and `src/tui/panes/output/selection.rs` — `OutputPresentation`, columns, typed cursor, navigation, hit rectangles, captured-output-only selection/copy.
  - `src/tui/panes/layout.rs`, `src/tui/panes/system.rs`, `src/tui/panes/spec.rs`, and `src/tui/panes/mod.rs` — Output visibility, tabbability, layout ownership, and pane facade.
  - `src/tui/render.rs`, `src/tui/hit_test.rs`, and `src/tui/interaction.rs` — presentation-driven bottom row, monitor/confirmation rendering, identity-bearing mouse hits.
  - `src/tui/keymap/actions.rs`, `src/tui/keymap/canonical.rs`, `src/tui/keymap/load.rs`, `src/tui/keymap/resolved.rs`, and `src/tui/keymap/constants.rs` — action enums, portable Alt parsing/storage, conflict validation, defaults/migrations.
  - `src/tui/integration/framework_keymap/app_context.rs`, `src/tui/integration/framework_keymap/navigation.rs`, `src/tui/integration/framework_keymap/output_pane.rs`, `src/tui/integration/framework_keymap/builder.rs`, and `src/tui/integration/framework_keymap/mod.rs` — global toggle and Output-scoped kill actions through the framework keymap.
  - `src/tui/input/dispatch.rs` and `src/tui/keymap_ui/controller.rs` — action-aware Output preflight, modal input priority, dispatch, and keymap UI labels.
  - `tests/assets/default-keymap.toml` — pinned generated default-keymap fixture.
- **Build:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-port`
- **Test:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-port`
- **Lint:** `bash ~/.claude/scripts/delegate/verify.sh lint cargo-port`
- **Style:** `zsh ~/.claude/scripts/rust_style/load-rust-style.sh --scope edit --project-root /Users/natemccoy/rust/cargo-port`
- **Invariants:** Compile visibility starts disabled, is not persisted, and while off owns no compile-specific deadline, new refresh demand, command parsing, classification, snapshot, tombstone, generation, or late-result acceptance; cancellation prevents an already-observing shared worker request from entering compile classification after toggle/scope invalidation while preserving any coalesced Running Targets result. Existing Running Targets retains its one-second behavior and owned-target Output behavior remains unchanged. One App-owned, dedicated-worker `ProcessRefreshExecutor` owns exactly one `ProcessObserver`, coalesces simultaneous consumer demand into one refresh cycle, and returns `CompletedProcessRefreshExecution` with independent Running and compile-consumer outcomes; one App-owned revision-keyed `CargoWorkspaceIndex` serves Running Targets and Build Monitor without launching Cargo when monitoring starts. The shared index explicitly reports `Current`, `RetainedLastAccepted`, or `Uninitialized`; consumers preserve the last accepted index on refresh failure, and only an uninitialized index may use a named fallback. `ProjectListRevision` changes only when visible ownership content changes; selected-row identity is separate monitor-scope input. Scope is a typed row-kind-aware `MonitorScopeKey` over sorted canonical checkout/workspace roots plus metadata/project-list revision; workspace members resolve to their owning workspace, groups differ from primary checkout rows, non-Rust scopes are empty, and a changed key makes old data immediately non-actionable. Build sessions and activity rows are keyed by exec-sensitive `ProcessIncarnation`, while termination authorization retains strong `ProcessIdentity`; neither uses a bare PID. Weak, stale, inferred, ambiguous, or unattributed evidence is observed-only, and system-wide cache-daemon ambiguity is rendered once without guessing. Process observation and termination are separate: `ProcessObserver` produces immutable evidence and capabilities, while `ProcessTerminator` performs identity-revalidated signaling off the TUI event loop. External termination requires an identity-bound platform capability and opaque frozen scope/identity authorization; never signal an ambient process group, Cargo Port, shell/LLM ancestors, cache daemons, divergent nested sessions, or target-directory-only compiler matches. Selected-scope kill refuses partial actionability, never absorbs builds started after confirmation, and bounded leaf-before-root termination reports already gone, gone after signaling, survivors, and errors truthfully without claiming causation it cannot prove or automatically using `SIGKILL`. `OwnedRun` solely owns lifecycle/output; every message carries `OwnedRunId`; its observed activity is joined, not copied, and pinned owned output can coexist with external columns while remaining outside unrelated scope-wide kills. A single `OutputPresentation` controls rendering, layout, focus, tabbability, labels, copy, and hit testing; typed cursors permit visual selection/Ctrl-A only in owned captured output, while columns/navigation preserve stable identities and Tab/Shift-Tab preflight falls through at session boundaries. Defaults are framework-keymap actions only: global `Shift-C`, Output `alt-k` for selected build, and `alt-shift-k` for all scoped builds; render `Option-K`/`Option-Shift-K` on macOS and `Alt-K`/`Alt-Shift-K` elsewhere, with no raw `KeyCode` dispatch outside the keymap. Open termination confirmation is modal above Output/global input. Preserve one Cargo Port-owned run at a time, strict workspace lints/missing docs, `RUSTC_WRAPPER`, nightly formatting for this `natepiano` origin, and inline focused tests plus 1,000/5,000-process refresh benchmarks proving no persistent monitor-off CPU work.

## Phases

### Phase 1 — Shared Cargo workspace index · status: done

#### Work Order

**Goal:** Running Targets reads workspace ownership and target-directory data through one App-owned, revision-keyed `CargoWorkspaceIndex` with no visible behavior change.

**Spec:**

- Extract canonical workspace roots, target directories, workspace-member ownership, package/source identity, and accepted metadata revision from `WorkspaceMetadataStore` into an App-owned `CargoWorkspaceIndex`.
- Rebuild the index only after an accepted metadata revision or visible-target/project-list revision. Never rebuild it on a generic event-loop wake.
- Preserve the current `cargo metadata --no-deps` behavior. The index must expose workspace data without implying that registry and Git dependency records are complete.
- Represent canonical checkout/workspace roots explicitly so later consumers do not use string-prefix scope tests.
- Move the existing Running Targets metadata lookups onto the shared index and preserve its current results and one-second process cadence.

**Files:**

- `src/project/cargo/metadata_store.rs` — expose the accepted metadata inputs and revision needed by the shared index.
- `src/project/cargo/mod.rs` — define or export `CargoWorkspaceIndex` and its revision-keyed views.
- `src/project/mod.rs` — export the shared project capability.
- `src/tui/app/mod.rs` — own the index at App scope.
- `src/tui/app/construct.rs` — construct the index from accepted project metadata.
- `src/tui/running_targets/state.rs` — consume shared workspace/target data.
- `src/tui/running_targets/mod.rs` — adjust the Running Targets facade and tests.

**Constraints from prior phases:** None.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-port`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-port`, and `bash ~/.claude/scripts/delegate/verify.sh lint cargo-port` are green; focused tests prove revision changes rebuild the index, ordinary ticks do not, canonical worktree/workspace roots remain distinct, and Running Targets results are unchanged.

#### Retrospective

**What worked:**

- `CargoWorkspaceIndex` now owns revision-keyed checkout, Cargo-workspace, member, package, target-source, and live target-directory identities for App consumers.
- Running Targets reads the shared index without changing its one-second process cadence, and all 1,015 tests plus warning-denied lint remain green.

**What deviated from the plan:**

- The metadata root split reached existing scan, target-pane, package-pane, and tree-mutation callers beyond the initial Files list so checkout roots and Cargo-reported workspace roots stayed truthful end to end.
- Running Targets needed named current, retained-last-accepted, and uninitialized index states plus a cadence readiness gate before Phase 1 filesystem attribution work.

**Surprises:**

- Shared target directories and canonical source/owner collisions required retaining all workspace candidates; exact ownership can remain ambiguous across workspaces or across packages with the same target identity.
- Selection-only navigation must not advance `ProjectListRevision`; the revision represents visible ownership content, while Phase 5 owns selected-row scope identity.

**Implications for remaining phases:**

- Later process and build classification must consume exact PackageId/target identities and named canonical-resolution or ambiguity states instead of reconstructing ownership with path prefixes.
- Phase 3 must preserve the pre-attribution one-second readiness gate, retained-last-accepted index behavior, and conservative omission of cross-workspace Running Targets ambiguity while moving host process observation.
- Phase 5 must keep selected-row scope generation separate from Phase 1's content-only `ProjectListRevision`.

#### Phase 1 Review

- Phases 2–3 now model weak process evidence and preserve Running Targets index readiness, cadence ordering, ambiguity omission, and its existing identity-revalidated termination path.
- Phases 4–5 now use semantic owned-run lifecycle and monitor-scope/index-readiness states, with selection identity separate from content-only `ProjectListRevision`.
- Phase 6 now reuses `MonitorScopeKey` and `cargo_metadata::PackageId`, consumes exact Phase 1 identities, and keeps cache/ledger mutation outside its pure classifier.
- Phase 7 was inserted as the measured execution-architecture gate; the former Phases 7–12 became Phases 8–13.
- Phase 8 now consumes the chosen executor and revises live target-directory resolution when directories or symlink targets change without a metadata/list revision.
- Phase 9 now renders every named scope-availability state distinctly and non-actionably.
- Phases 11–13 now separate observation from nonblocking termination, retain opaque authority through confirmations, and report already-gone, gone-after-signal, survivor, and error outcomes without unprovable kill claims.
- No user decisions were required; these edits preserve the existing behavior and safety invariants while making each remaining Work Order self-contained.

### Phase 2 — Strong process observation foundation · status: done

#### Work Order

**Goal:** A host-only `ProcessObserver` can produce immutable process snapshots with strong lifetime and exec-incarnation identity without changing any existing consumer.

**Spec:**

- Add `src/process_observation/` and declare it from `src/main.rs`.
- Define `ProcessIdentity { pid: u32, creation_token: PlatformCreationToken }` using the strongest available process creation token on each supported platform. A bare PID is never a strong identity.
- Convert platform discovery into `ObservedProcessIdentity::{Strong(ProcessIdentity), Insufficient(InsufficientProcessIdentity)}` at the OS boundary. Insufficient evidence stays visible for diagnostics but can never enter a strong-identity collection or action-bearing API.
- Define `ProcessIncarnation { identity: ProcessIdentity, executable_argv_fingerprint: ProcessFingerprint }`. An executable/argument fingerprint change invalidates executable, command, cwd, classification, scope, ancestry, and termination evidence even though lifetime identity is unchanged.
- Represent executable, argv, cwd, and parent discovery with named `ProcessFieldObservation<T>` states such as observed, unavailable, and invalidated; validated parentage likewise distinguishes root, validated edge, unavailable parent, and rejected edge. Domain-owned snapshot records do not use bare `Option<T>` for evidence state.
- Own one `sysinfo::System` inside `ProcessObserver`; expose immutable snapshots and targeted refresh input rather than the mutable system object.
- Provide validated, depth-capped parent walks in which every edge carries the current parent and child `ProcessIdentity`.
- Cache parsed process incarnations for new, changed, or still-unclassified Cargo/compiler/wrapper candidates. Evict entries absent from the latest successful full snapshot.
- Keep observation and termination capabilities separate. This phase exposes no public signal action.
- Add platform-focused tests for PID reuse rejection, invalid parent chains, same-PID exec invalidation, and snapshot eviction.

**Files:**

- `Cargo.toml` — add only process-platform dependencies required by the typed identity implementation.
- `src/main.rs` — declare the shared process-observation module.
- `src/process_observation/mod.rs` — public host-only capability and immutable snapshot API.
- `src/process_observation/identity.rs` — strong/insufficient lifetime evidence, exec incarnation, and platform token types.
- `src/process_observation/snapshot.rs` — named field observations, process records, validated ancestry, refresh inputs, and caches.

**Constraints from prior phases:** Phase 1 established App-owned shared capabilities and exact canonical checkout/workspace/package/target identities; `ProcessObserver` must remain independent of Cargo metadata, `cargo_metadata::PackageId`, and project-list types.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-port`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-port`, and `bash ~/.claude/scripts/delegate/verify.sh lint cargo-port` are green; focused tests prove strong versus insufficient identity, unavailable executable/argv/cwd/parent states, exec invalidation, depth-capped parent validation, rejected edges, and cache eviction.

#### Retrospective

**What worked:**

- `ProcessObserver` now owns one private `sysinfo::System` and returns immutable full or targeted snapshots with strong lifetime, exec-sensitive incarnation, named field, and validated ancestry evidence.
- Platform-focused identity, coherence, parentage, cache, and live same-PID exec coverage passes with 1,094 cargo-port tests; the real TUI started, changed project selection, and quit cleanly.

**What deviated from the plan:**

- macOS needed a native `proc_pid_rusage` monotonic start value, bracketed by processkit lifetime anchors, to supply both the strongest creation token and truthful creation ordering.
- Full-refresh cache reconciliation required an explicit directly-sampled PID set plus one final direct lookup for each omitted cached PID, independent of parent-only observations.

**Surprises:**

- Direct and reported-parent identities form one chronological evidence stream; later insufficient evidence must block strong admission while distinguishing proven exit from transient identity unavailability.
- Executable, argv, and cwd coherence must be sampled together, while parent evidence is bracketed and reconciled independently even when those fields are unstable.

**Implications for remaining phases:**

- Every later consumer must use `ProcessIdentity` or `ProcessIncarnation`, never a PID, and must preserve named insufficient, unavailable, invalidated, and rejected states without promoting them to action authority.
- Phase 3 must schedule one shared full or targeted `ProcessObserver` refresh and consume immutable snapshots without reintroducing a private `sysinfo::System` or merging observation with signaling.
- Later cache, classification, ownership, and termination work must treat the latest chronological identity evidence and exec fingerprint as the invalidation boundary.

#### Phase 2 Review

- Phase 3 now measures observer-only cost before migrating Running Targets, owns the sole observer behind a semantic synchronous/worker executor, distinguishes completed-empty refreshes from failures, and treats one coalesced consumer cycle separately from Phase 2's internal coherence samples.
- Phase 3 now preserves identity-bound CPU/history metrics and moves legacy Running Targets signaling into its own typed termination boundary outside `ProcessObserver`.
- Phase 4 now stores identity and owned-group authority only in lifecycle states where they exist, with every direct consumer included in its Work Order.
- Phases 6 and 9 now key sessions, activities, and cursor rows by `ProcessIncarnation`, consume Phase 2's sole candidate-incarnation cache, and invalidate prior selection/actionability on same-PID exec.
- Phase 7 now extends the Phase 3 executor with measured classification cost while preserving sole observer ownership and semantic execution outcomes.
- Phases 8 and 11 now distinguish executor failure from per-process insufficiency and completed-empty observation, preserve internal coherence sampling, and reject same-PID exec before action.
- No user decisions were required; these were mechanical consequences of Phase 2's shipped identity, coherence, cache, and observation-only guarantees.

### Phase 3 — Shared refresh scheduling and Running Targets migration · status: done

#### Work Order

**Goal:** Running Targets uses the App-owned `ProcessRefreshExecutor`, preserving its one-second behavior while establishing one measured, coalesced process-refresh scheduler over the sole observer.

**Spec:**

- Before moving Running Targets, benchmark Phase 2 full and targeted observer refreshes with repeatable 1,000- and 5,000-process fixtures against a 15 ms event-loop allocation. Define one App-owned `ProcessRefreshExecutor` that owns exactly one `ProcessObserver`; keep synchronous execution only when repeated 5,000-process samples remain at or below 15 ms, otherwise move the observer and its private `System`/incarnation cache behind a dedicated worker and `crossbeam-channel` in this phase.
- Move the host-facing process-table work out of `RunningTargetsPoller` and onto `ProcessRefreshExecutor`; retain a Running-target facade for view-specific state. Do not expose observer references or mutable cache state that would prevent worker ownership.
- Define typed refresh demand for Running, compile monitoring, or both. `ProcessRefreshExecutor::next_deadline()` and `refresh_due(now)` combine required fields and perform at most one coalesced refresh cycle per due instant, with no duplicate cycle per consumer. Phase 2's repeated fresh field samples and bracketed identity observations remain internal to that one logical cycle and are not counted as duplicate consumer refreshes.
- Represent execution as `ProcessRefreshExecutionOutcome::{Completed(ProcessObservationSnapshot), Failed(ProcessRefreshExecutionFailure)}` or an equally semantic aggregate; a completed empty snapshot is not a failure, and no domain-owned API returns a bare `Option<ProcessObservationSnapshot>`.
- Running CPU/history sampling keeps its current one-second cadence even when a later compile-monitor consumer requests identity refreshes more often.
- Preserve a separate identity-bound metrics observation for Running Targets name/CPU/memory over the long-lived `System`; execute it once per due Running cycle without weakening Phase 2's repeated executable/argv/cwd coherence sampling.
- The terminal event loop waits on the minimum animation/process deadline. Process refresh is not represented as an animation.
- Preserve startup/test suppression semantics in `startup_services.rs` and all existing Running-target visibility, CPU/history, and kill behavior.
- Preserve `WorkspaceIndexRefreshState::{Current, RetainedLastAccepted, Uninitialized}` exactly. Evaluate the one-second cadence/readiness gate before filesystem attribution, use the last accepted index after refresh failure, allow visible-target fallback only while uninitialized, and continue omitting cross-workspace ambiguous exact owners.
- Move the existing Running Targets kill path through a typed `RunningTargetTerminationCapability` in `src/tui/running_targets/termination.rs` that consumes strong identity revalidation and owns the existing signaling behavior. Neither an observer snapshot nor a bare PID/create-time pair is action authority; this capability preserves existing behavior only and remains separate from later build-monitor termination authority.
- Instrument refresh duration through existing frame metrics so Phase 7 can compare the synchronous path against its frame budget.

**Files:**

- `src/process_observation/mod.rs` — add typed consumers, deadlines, and combined refresh plans.
- `src/process_observation/identity.rs` — expose the strong identity revalidation used by the preserved Running Targets termination capability.
- `src/process_observation/snapshot.rs` — support coalesced consumer demand while preserving repeated coherent field sampling and immutable execution outcomes.
- `src/process_observation/benchmarks.rs` — repeatable 1,000/5,000-process observer timing fixtures and report harness.
- `src/tui/app/mod.rs` — own and expose the executor without exposing its observer/cache internals.
- `src/tui/app/construct.rs` — initialize the measured synchronous or worker-backed executor.
- `src/tui/running_targets/state.rs` — replace host snapshot ownership with view state over observer records.
- `src/tui/running_targets/app_tick.rs` — request one-second Running refreshes.
- `src/tui/running_targets/constants.rs` — retain the current cadence.
- `src/tui/running_targets/mod.rs` — adapt exports and tests.
- `src/tui/running_targets/termination.rs` — own identity-revalidated legacy Running Targets signaling outside `ProcessObserver`.
- `src/tui/panes/system.rs` — preserve Running Targets kill/action routing while process ownership moves.
- `src/tui/startup_services.rs` — preserve suppression/test effects.
- `src/tui/terminal/event_loop.rs` — wait on the minimum animation/process deadline.
- `src/tui/terminal/frame_metrics.rs` — record observer work separately from rendering.
- `src/tui/background.rs` — own the dedicated observer worker when the 15 ms gate rejects synchronous execution.
- `src/tui/messages.rs` — carry correlated immutable refresh requests/results when the worker path is selected.

**Constraints from prior phases:** Use Phase 1's exact `CargoWorkspaceIndex` identities and named refresh states for project data. Phase 2's `ProcessObserver` owns one private long-lived `sysinfo::System`, incarnation cache, immutable full/targeted snapshots, chronological direct/reported-parent identity evidence, and independent coherent-field/parent sampling. Preserve the Phase 1 cadence-before-attribution gate, retained accepted index, uninitialized-only fallback, and conservative cross-workspace ambiguity omission. Running Targets must not recover its own `sysinfo::System`; `ProcessObserver` remains observation-only, and the preserved Running termination capability is not build-monitor authority.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-port`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-port`, and `bash ~/.claude/scripts/delegate/verify.sh lint cargo-port` are green; observer-only 1,000/5,000-process timing is recorded and selects synchronous execution only at or below 15 ms; existing Running Targets tests pass; focused tests prove simultaneous demands cause one coalesced cycle, completed-empty and failed execution remain distinct, repeated field coherence remains intact, one-second identity-bound CPU/history cadence and readiness ordering are preserved, retained/uninitialized index paths remain distinct, cross-workspace ambiguous targets stay omitted, PID reuse is rejected before Running Targets termination, and no compile deadline exists yet.

#### Retrospective

**What worked:**

- Repeatable 1,000/5,000-process measurements selected the dedicated worker, and `ProcessRefreshExecutor` now owns the sole observer behind correlated, coalesced demand.
- Running Targets consumes immutable snapshots at its existing one-second cadence, retains current/retained/uninitialized index behavior, and signals only through `RunningTargetTerminationCapability` after strong-identity revalidation.

**What deviated from the plan:**

- Successful execution became `CompletedProcessRefreshExecution`, which keeps elapsed observer time beside the immutable snapshot; failures carry no fabricated completion duration.
- The long-lived metrics cache gained a private `RunningMetricsProcessTable` and named CPU-continuity states so purging replaced, unbound, or identity-insufficient records does not visibly reset unrelated stable CPU samples.

**Surprises:**

- Refreshing identity fields on the same long-lived sysinfo table disturbed CPU timing, so full discovery and repeated executable/argv/cwd sampling use short-lived systems while one private table owns CPU/memory baselines.
- Counting a semantic metrics-cycle call was not enough to prove one raw OS refresh; the final structure counts each legal process-table refresh invocation at the deepest private boundary.

**Implications for remaining phases:**

- Phase 5 must replace the private Running Targets index-readiness adapter with the shared adapter without changing cadence-before-attribution behavior.
- Phases 7–8 must extend and consume the existing dedicated executor, `CompletedProcessRefreshExecution`, named failure timing, and one-counted-refresh boundary rather than create another observer or timing channel.
- Later build termination must remain separate from the legacy Running Targets capability and perform its own immediate strong-identity revalidation.

#### Phase 3 Review

- Phase 4 now creates identity-verified owned-group authority, replaces optional package selection with semantic invocation state, covers every direct lifecycle consumer, and preserves Phase 3 worker messages/deadlines while refactoring shared files.
- Phase 5 now replaces Running Targets' private index-readiness adapter without changing cadence-before-attribution, retained-index, ambiguity, or one-refresh metrics behavior.
- Phase 6 now defines `BuildClassificationCoordinator` as the owner of dependency-manifest and first-seen support state around the pure classifier.
- Phase 7 now extends the already-selected dedicated worker, places the coordinator beside the sole observer, returns independent compile outcomes inside `CompletedProcessRefreshExecution`, supports cooperative generation cancellation, and moves shared reconciliation to a neutral App adapter.
- Phase 8 now uses a 500 ms semantic disabled/due/in-flight schedule, ages monitor data for compile-only or whole-cycle failures without discarding successful Running results, and cancels in-flight classification after shared observation.
- Phase 9 now retains cursor identity only by exec-sensitive activity/session IDs.
- Phases 11–13 now carry authority-bearing owned/external actionability and distinct opaque selected/scope termination aggregates created only by `BuildMonitor`; Phase 13's keymap fixture correctly points back to Phase 10 defaults.
- No user decisions were required; every change follows Phase 3's measured worker choice, monitor-off invariant, exec-sensitive identity boundary, and existing termination-safety requirements.

### Phase 4 — Correlated Cargo Port-owned runs · status: todo

#### Work Order

**Goal:** The existing single Cargo Port-owned target run has a stable ID and owns all lifecycle/output state, with late asynchronous messages unable to mutate a later run.

**Spec:**

- Replace anonymous single-run fields in `Inflight` with one `OwnedRun` aggregate carrying monotonic `OwnedRunId` and semantic `OwnedRunLifecycle` variants. Queued/starting variants own pending launch data without pretending a root exists; running/stopping variants own the verified root `ProcessIdentity`, isolated process group, and a new opaque `OwnedProcessGroupTerminationCapability`; retained success, gone-after-signal, and failure variants own their outcome and retained output. Do not model these states as a cluster of bare `Option<T>` fields.
- Replace `PendingExampleRun.package_name: Option<String>` at the lifecycle boundary with `CargoPackageInvocation::{WorkspaceDefault, Package(String)}` (or an equally semantic type) so queued launch state states how Cargo arguments are constructed without interpreting presence or absence.
- After spawning the isolated process group, observe and revalidate its strong Phase 2 `ProcessIdentity` before constructing `OwnedProcessGroupTerminationCapability` and entering a live lifecycle state. If strong identity cannot be established, fail the launch and clean up the child/group; do not expose a PID-only stop path or fabricate live authority.
- `OwnedRun` is the sole owner of owned lifecycle, state-specific identity/termination authority, outcome, and output. Other state may retain only `OwnedRunId` plus immutable observations.
- Tag every output, progress, started, and finished message with `OwnedRunId`; reconciliation ignores messages whose ID is not the current run.
- Preserve the current one-owned-run concurrency limit, isolated process group, clear/close lifecycle, stopping behavior, output retention, visual-selection frozen snapshots, and monitor-off rendering byte-for-byte.
- Expose owned run identity, lifecycle, and output by reference for later Output presentation; do not copy output into process snapshots.

**Files:**

- `src/tui/state/inflight.rs` — define `OwnedRunId`/`OwnedRun` and own the aggregate.
- `src/tui/state/mod.rs` — export the owned-run API.
- `src/tui/background.rs` — carry the run ID through background work.
- `src/tui/messages.rs` — add run ID to all owned-run messages.
- `src/process_observation/identity.rs` — supply strong identity observation/revalidation required before a run becomes live.
- `src/tui/terminal/processes.rs` — return verified root/process-group identity and tag captured output.
- `src/tui/terminal/mod.rs` — expose the identity-verified launch boundary.
- `src/tui/app/async_tasks/poll.rs` — reconcile only matching run messages.
- `src/tui/app/mod.rs` — replace direct anonymous run-field consumers with the lifecycle aggregate.
- `src/tui/terminal/event_loop.rs` — consume state-specific owned lifecycle/deadline data.
- `src/tui/panes/pane_data/pending.rs` — replace optional package launch data with semantic Cargo package invocation.
- `src/tui/panes/actions.rs` — construct semantic owned-run launch requests.
- `src/tui/panes/output/mod.rs` — consume owned output through the new aggregate without changing presentation.
- `src/tui/panes/output/render.rs` — render retained and live owned states through state-specific lifecycle data.
- `src/tui/input/dispatch.rs` — start/stop only through the correlated owned-run lifecycle.
- `src/tui/integration/framework_keymap/output_pane.rs` — preserve existing Output action availability against the new lifecycle.
- `src/tui/render.rs` — preserve current owned-run status and Output rendering.

**Constraints from prior phases:** Use Phase 2 strong `ProcessIdentity` only in lifecycle variants where the owned root has actually been verified; queued and starting states cannot hide absent identity in `Option`. The current code has only a PID-bearing process-group stop path, so this phase must create the opaque owned-group capability after identity verification rather than assume it already exists. Preserve Phase 3's App-owned dedicated `ProcessRefreshExecutor`, `ProcessRefreshMsg` correlation, result receiver, shared deadline access, and Running Targets reconciliation while refactoring the same App/background/messages/event-loop files.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-port`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-port`, and `bash ~/.claude/scripts/delegate/verify.sh lint cargo-port` are green; focused tests prove queued/starting states carry no fabricated root or bare optional authority; workspace-default and named-package invocations construct the correct Cargo arguments; identity-unavailable launch fails and cleans up without exposing live authority; only live/stopping states own verified identity and opaque group termination capability; late output/progress/started/finished messages from run N cannot mutate run N+1; Phase 3 refresh messages and deadlines still reconcile; and all existing launch, stop, clear, copy, visual-selection, and retained-output behavior remains unchanged.

### Phase 5 — Typed monitor scope and state shell · status: todo

#### Work Order

**Goal:** The selected project-list row resolves to a stable, row-kind-aware compile-monitor scope, and monitor state can be on or off without polling or rendering builds yet.

**Spec:**

- Add `src/tui/compile_visibility/` with `CompileVisibilityState::{Off, On(ActiveMonitorState)}`. Only `On` may own a scope key, external snapshot, tombstone, classifier generation, monitor deadline, or late-result acceptance; toggling off drops the entire aggregate.
- Define `MonitorScopeKey` from selected row kind, sorted canonical checkout/workspace roots, metadata revision, and project-list revision. A worktree-group row and its primary checkout row remain different scopes even when they share a path.
- Add a shared App-facing workspace-index adapter with `WorkspaceIndexReadiness::{Current, RetainedLastAccepted, Uninitialized}` so Running Targets and compile visibility consume the same readiness decision instead of duplicating private logic.
- Resolve the selected row as `MonitorScopeResolution::{Ready(MonitorScopeKey), EmptyNonRust, PendingIndex, AmbiguousOwnership, UnresolvedPath}` or an equally semantic exhaustive type. A bare `Option<MonitorScopeKey>` is not permitted; only `Ready` can become actionable.
- Resolve package/workspace rows to the owning workspace checkout; linked-worktree checkout rows include only that checkout; worktree-group rows include the primary and every represented live linked checkout; vendored packages/submodules use their own Cargo workspace when metadata proves one and otherwise their containing checkout; non-Rust rows produce an empty scope.
- Define monotonic `CompileMonitorGeneration` and advance it on toggle/scope replacement. A selection, membership, metadata, or project-list revision change immediately replaces the scope key, makes the prior snapshot non-actionable, and leaves the new state pending until its first matching snapshot. Late results carry the generation and are ignored after replacement.
- Do not launch Cargo or refresh process data when resolving scope.

**Files:**

- `src/main.rs` — declare the compile-visibility adapter.
- `src/tui/compile_visibility/mod.rs` — state shell, toggle lifecycle, generations, and exports.
- `src/tui/compile_visibility/scope.rs` — `MonitorScopeKey` and selected-row resolution.
- `src/tui/workspace_index.rs` — shared App adapter for accepted-index readiness and query access.
- `src/tui/mod.rs` — declare the shared workspace-index adapter.
- `src/project/cargo/workspace_index.rs` — consume exact index identities and expose any missing named query result required by scope resolution.
- `src/project/cargo/workspace_index_api_tests.rs` — prove the shared scope queries preserve exact ownership and ambiguity.
- `src/project/root_item.rs` — expose canonical checkout ownership required by scope construction.
- `src/project/git/worktree_group.rs` — expose represented live checkout roots.
- `src/tui/project_list/visible_rows.rs` — expose typed row kind.
- `src/tui/project_list/list.rs` — provide current selected row and revision.
- `src/tui/project_list/mod.rs` — export typed selection data.
- `src/tui/app/mod.rs` — own `CompileVisibilityState`, initially `Off`.
- `src/tui/running_targets/app_tick.rs` — consume the shared readiness adapter instead of private index-selection logic.
- `src/tui/running_targets/state.rs` — consume shared index query results without weakening ambiguity states.

**Constraints from prior phases:** Read exact canonical workspace/member/package/target data only from Phase 1's revision-keyed index and preserve its current/retained/uninitialized semantics. Replace the private readiness adapter in `running_targets/app_tick.rs` with the shared adapter without moving filesystem attribution ahead of the existing one-second cadence/readiness gate, weakening retained-last-accepted behavior, or admitting cross-workspace ambiguous owners. `ProjectListRevision` changes only for visible ownership content; selected-row identity is a separate scope input. Process identities and snapshots from Phases 2–3 must not leak into stale scope state. Phase 3's dedicated executor, `CompletedProcessRefreshExecution`, failure timing, and one-counted raw Running metrics refresh boundary remain unchanged. Phase 4 owned output remains independent.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-port`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-port`, and `bash ~/.claude/scripts/delegate/verify.sh lint cargo-port` are green; focused tests cover primary, linked, grouped, workspace-member, vendored/submodule, and non-Rust selections; distinguish current, retained, uninitialized, empty, pending, ambiguous, and unresolved states; prove group and primary keys differ; prove selection changes scope without revising visible content; prove scope/toggle changes invalidate prior generations immediately; prove `Off` owns no monitor deadline or snapshot; and prove Running Targets still evaluates cadence/readiness before attribution, retains the last accepted index, omits cross-workspace ambiguity, and performs one raw metrics refresh per due cycle.

### Phase 6 — Cargo build and compiler classification · status: todo

#### Work Order

**Goal:** Pure build-monitor classification converts an immutable system snapshot plus workspace index into stable build sessions and active compile units without guessing ambiguous attribution.

**Spec:**

- Add domain identifiers and snapshots: `BuildSessionId(ProcessIncarnation)`, `CompileActivityId(ProcessIncarnation)`, Phase 5's `MonitorScopeKey`, `ScopeAttribution::{Confirmed, Inferred}`, `CompilerAssociation::{Confirmed, UniqueHeuristic, Ambiguous { candidates }, Unmatched}`, `MonitorSnapshot::{Pending, Fresh, Stale, Unavailable}`, and presentation-only session/activity records. Reuse Phase 1's `cargo_metadata::PackageId`; do not introduce duplicate `BuildScopeId` or `PackageId` wrappers. Raw PIDs never stand alone in an actionable type; `ProcessIdentity` remains available inside later signaling authorization but does not key a session or row across exec.
- Discover the outermost recognized root build in a validated Cargo process chain. Normalize rustup proxies, built-in/configured aliases, `cargo-*` plugins, and nested Cargo. Immediately recognize `build`, `check`, `clippy`, `fix`, `run`, `rustc`, `rustdoc`, `test`, `nextest`, `bench`, and `doc`; deny known metadata/fetch/management commands unless a live compiler descendant proves a build.
- Resolve scope for every Cargo node before normalizing. A nested Cargo belongs to the outer root only when confirmed scope and termination boundary match; a plugin/alias entering another checkout becomes a separate session. Discover compatible roots system-wide before filtering the selected scope.
- Resolve root scope in order: Cargo Port-owned PID/launch directory; `--manifest-path` or absolute manifest argument; cwd plus nearest containing manifest; uniquely matching compiler output directory. Canonicalize both sides and never use string-prefix matching alone.
- Associate `rustc`, `clippy-driver`, `rustdoc`, build-script, and linker descendants by validated parent chain. For cache-daemon parentage, use `(target directory, profile/build directory, target triple)` only when it selects one compatible live session across the entire system. Render ambiguous units once in a scope-level, non-actionable attribution-unavailable section with candidate sessions.
- Derive compile units primarily from `--crate-name`, primary input, `--out-dir`, target triple, flags, and strong compiler identity. Resolve workspace packages from the shared index; for dependencies absent from `no_deps`, parse the nearest package manifest once and cache package identity by canonical source root plus manifest stamp. Reparse after change/removal; otherwise use `CompilerCrateIdentity::{WorkspacePackage(cargo_metadata::PackageId), DependencyPackage(DependencyPackageIdentity), CrateNameFallback(CompilerCrateName)}` or an equally semantic fallback type.
- Keep classification pure. Define a stateful `BuildClassificationCoordinator` that owns dependency-manifest caches and the first-seen ledger, prepares immutable `BuildClassificationInput` containing the process snapshot, workspace-index view, dependency-manifest snapshot, and first-seen snapshot, invokes the pure classifier to produce immutable `BuildClassificationOutput`, then updates those support structures outside the pure classify call. This phase defines and tests the coordinator without assigning it an App runtime owner; Phase 7 moves its sole runtime instance beside `ProcessObserver` on the dedicated executor worker.
- Consume Phase 2's immutable unclassified Cargo/compiler/wrapper candidate-incarnation evidence as part of `BuildClassificationInput`; do not repeat candidate parsing or introduce a second owner for that cache. Classification adds Cargo/build semantics while Phase 2 remains the sole exec-bound candidate-cache owner.
- Use session key `BuildSessionId(ProcessIncarnation)` and activity key `CompileActivityId(ProcessIncarnation)`; target directory and profile are attributes. Resolve profile from explicit `--profile`/`--release`, then output directories, then metadata defaults, preserving custom/unknown labels. Order sessions and units by first-seen then process incarnation.

**Files:**

- `src/build_monitor/mod.rs` — domain exports and classification entry point.
- `src/build_monitor/model.rs` — typed IDs, attribution, activity, snapshots, and non-actionable presentation records.
- `src/build_monitor/classify.rs` — Cargo root normalization, scope resolution, compiler association, unit/profile/package derivation, and caches.
- `src/build_monitor/coordinator.rs` — own dependency-manifest cache and first-seen state around the pure classifier boundary.
- `src/main.rs` — declare the build-monitor domain.
- `src/process_observation/snapshot.rs` — expose immutable executable, argv, cwd, ancestry, creation, and unclassified candidate-incarnation evidence required by pure classification without exposing mutable cache ownership.
- `src/project/cargo/metadata_store.rs` — expose manifest/source stamps without adding dependency metadata commands.
- `src/project/cargo/mod.rs` — supply index queries used by classification.
- `src/project/cargo/workspace_index.rs` — supply exact package, target, workspace, and ambiguity queries used by classification.
- `src/project/cargo/workspace_index_api_tests.rs` — prove classification-facing queries retain all exact candidates.
- `src/tui/workspace_index.rs` — supply named readiness and immutable index views to classification callers.
- `Cargo.toml` — add only parsing/platform dependencies required by the classifier.

**Constraints from prior phases:** Consume Phase 1's exact `CargoWorkspaceIndex` identities and existing `cargo_metadata::PackageId`. Phase 2 supplies named strong/insufficient immutable observations, `ProcessIncarnation` as the exec-sensitive classification boundary, and the sole unclassified Cargo/compiler/wrapper candidate cache; a changed executable/argv fingerprint invalidates classification, scope, ancestry, selection, and actionability within the same process lifetime. Consume Phase 4's owned identity and Phase 5's canonical `MonitorScopeKey` plus named scope/index readiness. Cross-workspace exact ambiguity remains non-actionable. Classification creates no signal authority, owns no mutable observer/cache state, and launches no Cargo command. `BuildClassificationCoordinator` owns classification support state only; it never owns or reaches into `ProcessObserver`.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-port`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-port`, and `bash ~/.claude/scripts/delegate/verify.sh lint cargo-port` are green; pure tests cover root commands, non-build commands, proxies/aliases/plugins/nested and divergent scopes, sibling roots, direct compiler/build-script/linker children, cache-daemon and cross-workspace ambiguity, debug/release/custom profiles, PID reuse and exec transitions, same-PID exec producing new session/activity IDs and invalidating prior classification, the sole Phase 2 candidate-cache owner, every named scope/index readiness state, dependency manifest caching/invalidation, exact workspace package IDs, semantic dependency/crate-name fallback, and no-deps fallback; coordinator tests prove dependency/first-seen mutation stays outside the pure classify call and immutable input/output snapshots cross the boundary.

### Phase 7 — Worker-side classification integration · status: todo

#### Work Order

**Goal:** The dedicated-worker `ProcessRefreshExecutor` incorporates classification and shared App reconciliation before lifecycle polling is added, preserving one observer owner and independent consumer outcomes.

**Spec:**

- Extend the Phase 3 repeatable 1,000- and 5,000-process benchmarks to cover observer refresh plus Phase 6 classification-input preparation/classification, including representative Cargo, compiler, wrapper, and unrelated processes; report classification's incremental cost separately from the observer-only baseline. Timing remains recorded evidence rather than a flaky CI threshold.
- Keep the architecture Phase 3 selected: one App-owned `ProcessRefreshExecutor` uses its dedicated worker, and the worker owns the sole `ProcessObserver` plus the sole runtime `BuildClassificationCoordinator`. Do not add a synchronous production branch, another observer, another classifier-support owner, or another timing channel.
- Requests carry refresh correlation, the semantic refresh plan, immutable workspace-index/scope/generation/owned-run evidence, and a compile-generation cancellation capability. Mutable App state and observer internals never cross into the worker.
- Extend `CompletedProcessRefreshExecution` with `CompileClassificationExecution::{NotRequested, Completed(BuildClassificationOutput), Failed(BuildClassificationExecutionFailure), Cancelled(CompileMonitorGeneration)}` or an equally semantic product. A successful process observation remains available to Running Targets when compile classification fails or is cancelled; only `ProcessRefreshExecutionOutcome::Failed(NoCompletedRefresh)` represents a cycle with no completed observation.
- Check compile cancellation after process observation and immediately before classification-input preparation/parsing. Toggle or scope invalidation cancels the matching monitor generation so an in-flight combined request skips compile parsing/classification while retaining and returning any due Running result.
- Move shared executor deadline access, request dispatch, receiver access, result correlation, and consumer-outcome reconciliation from `running_targets/app_tick.rs` into a neutral App adapter in `src/tui/process_refresh.rs`. Leave Running-specific cadence, index readiness, attribution, metrics, and view-state application in `running_targets/app_tick.rs`.
- Preserve Phase 3's `CompletedProcessRefreshExecution` duration, named no-completion failure timing, and one-counted raw Running metrics refresh boundary. This phase adds no compile-monitor deadline or lifecycle; Phase 8 schedules compile demand through this adapter without reopening architecture.

**Files:**

- `src/process_observation/mod.rs` — expose combined observation work through the executor boundary.
- `src/process_observation/executor.rs` — extend the existing dedicated worker request/result and cooperative cancellation boundary.
- `src/process_observation/snapshot.rs` — build immutable refresh inputs/results for measurement or worker transfer.
- `src/build_monitor/classify.rs` — accept the immutable classification input measured by the executor.
- `src/build_monitor/coordinator.rs` — move the sole runtime classification-support state beside the worker-owned observer.
- `src/build_monitor/benchmarks.rs` — repeatable 1,000/5,000-process fixtures and timing report harness.
- `src/build_monitor/model.rs` — carry semantic compile-classification outcomes, generation cancellation, and immutable results.
- `src/build_monitor/mod.rs` — export `ProcessRefreshExecutor`-facing classification APIs.
- `src/tui/process_refresh.rs` — neutral App adapter for shared deadline, dispatch, receiver, correlation, and consumer reconciliation.
- `src/tui/mod.rs` — declare the neutral process-refresh adapter.
- `src/tui/running_targets/app_tick.rs` — retain only Running-specific demand and snapshot application after shared orchestration moves.
- `src/tui/terminal/frame_metrics.rs` — use the existing 30 ms slow-frame boundary and record refresh cost separately.
- `src/tui/terminal/event_loop.rs` — host only the selected executor integration, without adding a compile deadline.
- `src/tui/background.rs` — extend the existing dedicated refresh worker with classification coordination.
- `src/tui/messages.rs` — extend immutable correlated worker requests/results with separate consumer outcomes.
- `src/tui/app/mod.rs` — keep App ownership at the executor boundary while observer ownership stays inside it.
- `src/tui/app/construct.rs` — construct the dedicated executor and worker-owned classifier coordinator without exposing either mutable owner.
- `src/tui/app/async_tasks/poll.rs` — route correlated worker results through the neutral adapter.

**Constraints from prior phases:** Phase 2 supplies named observation evidence, exec-sensitive incarnations, immutable snapshots, and one mutable observer/cache owner. Phase 3's measurements selected the dedicated worker and shipped sole `ProcessRefreshExecutor` ownership, coalesced refresh plans, `CompletedProcessRefreshExecution`, named `NoCompletedRefresh` failure timing, and exactly one raw Running metrics refresh per due cycle. Phase 5 supplies named index/scope readiness. Phase 6 supplies the pure classifier, immutable `BuildClassificationInput`, and `BuildClassificationCoordinator`; the worker owns the coordinator's dependency cache and first-seen ledger beside `ProcessObserver`, while requests contain only immutable consumer evidence. Preserve Running Targets cadence/readiness and add no compile polling yet.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-port`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-port`, and `bash ~/.claude/scripts/delegate/verify.sh lint cargo-port` are green; combined 1,000/5,000-process benchmarks and incremental classification timing are recorded; deterministic tests prove the dedicated worker solely owns `ProcessObserver` and `BuildClassificationCoordinator`, requests carry no mutable App/observer state, result correlation/order is exact, successful Running observation survives compile-classification failure, cancellation after observation skips compile parsing/classification while preserving a due Running result, one combined due cycle performs one counted raw Running metrics refresh, neutral reconciliation does not depend on the Running Targets adapter, and no compile deadline exists yet.

### Phase 8 — Conditional monitor polling and lifecycle · status: todo

#### Work Order

**Goal:** Enabling compile visibility produces fresh scoped monitor snapshots on a bounded cadence, while disabling it removes all compile-specific polling and idle work.

**Spec:**

- Add `BuildMonitor` state over the pure classifier. It retains only live session/unit identities, explicit owned association, termination tombstones added in later phases, and the latest presentation snapshot; it does not accumulate external history.
- Define `COMPILE_MONITOR_REFRESH_INTERVAL` as 500 ms in `src/tui/compile_visibility/constants.rs`. Model scheduling as `CompileMonitorRefreshSchedule::{Disabled, DueAt(Instant), InFlight { generation: CompileMonitorGeneration, rearm_at: Instant }}` or equally semantic enabled/disabled/in-flight states; do not use one `NotScheduled` state for disabled, consumed, and pending work. Rearm from the interval boundary, coalescing missed instants into one next demand rather than building a queue.
- While enabled, request command-line/process fields through the combined Phase 3 refresh plan and execute through Phase 7's dedicated-worker `ProcessRefreshExecutor`; perform one coalesced refresh cycle per interval with no duplicate cycle per workspace, column, or consumer. Phase 2's internal repeated field samples and identity brackets remain intact.
- Track live target-directory resolution as a typed state and revision, rechecked on each due poll. A previously missing target directory appearing, or a symlink being created/retargeted, invalidates affected classification and actionability even when metadata, project-list content, and selected scope are unchanged.
- The neutral Phase 7 App adapter contributes no compile deadline while monitor state is `Off`. On a due instant shared with Running Targets, union fields into one coalesced cycle while preserving the Running one-second identity-bound CPU/history sample and one-counted raw metrics refresh.
- A completed compile-classification result must carry monitor generation and exact `MonitorScopeKey`. Ignore mismatches. On scope change show `Pending`. `CompileClassificationExecution::Failed` and whole-cycle `ProcessRefreshExecutionOutcome::Failed(NoCompletedRefresh)` age the last good monitor snapshot to visibly `Stale` and non-actionable for one interval, then `Unavailable`; completed empty classification and per-process insufficient evidence do not. A compile-classification failure inside `CompletedProcessRefreshExecution` must not discard or suppress its successful Running snapshot/metrics outcome.
- On toggle off or scope/generation replacement, cancel matching in-flight compile work. The worker checks cancellation after shared process observation and before classification-input preparation/parsing, returns the cancelled compile outcome for rejection, and still returns any coalesced due Running outcome; no later compile result is accepted and no new compile demand is scheduled.
- A root Cargo process anchors a session through gaps with no live compiler. Report evidence-backed compiling, build-script, linking, owned Cargo-lock wait, and running-target states; report external no-child gaps only as active.
- Associate an owned run with exactly one observed session by matching its verified root `ProcessIdentity` to the current exec-sensitive `BuildSessionId(ProcessIncarnation)`, then retain only `OwnedRunId`; never copy owned output into snapshots. External completed sessions disappear.
- Prove no persistent idle CPU work while off and no compile-specific request/result acceptance after a toggle or scope generation change.

**Files:**

- `src/build_monitor/mod.rs` — `BuildMonitor` lifecycle and snapshot API.
- `src/build_monitor/poll.rs` — conditional refresh requests, classification, failure aging, and generation correlation.
- `src/build_monitor/model.rs` — fresh/stale/unavailable presentation states and stable first-seen ordering.
- `src/process_observation/mod.rs` — add optional compile consumer/deadline.
- `src/tui/compile_visibility/mod.rs` — connect enabled scope/generation to monitor polling.
- `src/tui/compile_visibility/constants.rs` — own the 500 ms compile refresh interval.
- `src/tui/process_refresh.rs` — combine compile/Running demand, deadlines, cancellation, and independent result reconciliation.
- `src/tui/app/mod.rs` — own `BuildMonitor`.
- `src/tui/app/construct.rs` — initialize it without enabling it.
- `src/tui/app/async_tasks/poll.rs` — reconcile generation-tagged results returned by the dedicated worker.
- `src/tui/terminal/event_loop.rs` — include the optional monitor deadline.
- `src/tui/terminal/frame_metrics.rs` — record and assert bounded refresh work.
- `src/tui/startup_services.rs` — ensure disabled/test startup creates no monitor work.

**Constraints from prior phases:** Phase 1 supplies exact index identities and current/retained/uninitialized readiness. Phase 3 supplies cadence-before-filesystem ordering, the dedicated executor, `CompletedProcessRefreshExecution`, named no-completion failures, and one-counted raw Running metrics refresh. Phase 5 owns named scope resolution and enablement/scope generation. Phase 6 supplies pure classification, `BuildClassificationCoordinator`, exec-sensitive session/activity IDs, and ambiguity omission. Phase 7 owns worker-side classification, cooperative generation cancellation, separate `CompileClassificationExecution`, and the neutral shared App adapter; extend those boundaries rather than introducing another observer, coordinator, result receiver, or timing channel. Phase 4 owns run output and semantic lifecycle. Do not add rendering or termination authority.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-port`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-port`, and `bash ~/.claude/scripts/delegate/verify.sh lint cargo-port` are green; tests prove disabled means no compile deadline/new refresh/parsing/result acceptance; 500 ms deadlines rearm exactly without queued catch-up; simultaneous Running one-second and compile due work performs one coalesced cycle and one counted raw Running metrics refresh; compile-classification failure ages monitor data without discarding a successful Running result; whole-cycle failure ages stale data to non-actionable then unavailable; completed empty classification does not; toggle/scope cancellation during an in-flight combined request skips compile parsing/classification, preserves due Running output, and rejects the cancelled generation; target-directory appearance and symlink retargeting revise live resolution without metadata/list changes; same-PID exec invalidates prior session/activity actionability; owned activity joins once; and selection changes never affect external processes.

### Phase 9 — Output monitor presentation and columns · status: todo

#### Work Order

**Goal:** The existing Output pane can render monitor empty, single-column, multi-column, and owned-output states from one presentation model.

**Spec:**

- Define `OutputPresentation::{Hidden, OwnedOnly(OwnedRunId), Monitor(MonitorColumns), MonitorWithOwned { columns, owned }}` and make layout, visibility, focus reconciliation, tabbability, bottom action labels, copy availability, hit testing, and rendering derive from this one value.
- When monitoring is enabled, the Output pane remains rendered and tabbable with a visible `Build monitor on` indicator even when the selected scope is pending, empty, or unavailable.
- Render Phase 5 scope resolution distinctly: pending index, empty non-Rust selection, ambiguous ownership, and unresolved path each have a truthful non-actionable empty-state message rather than collapsing into no sessions.
- One root Cargo invocation is one stable column. A single session uses full width with no divider. Multiple sessions split equally to a readable minimum width; when they do not fit, render a horizontally windowed subset and keep the selected column visible.
- Column headers show operative Cargo command/selectors, checkout/workspace path, resolved profile, root PID, elapsed time, and state. Render active compiler/build/link rows, plus the scope-level unattributed section, as selectable presentation rows.
- An owned session body is one sequence: activity rows, a non-selectable Output separator, then captured Cargo/target output. Once the target runs, show running state and output; after completion pin existing output with done/killed marker until existing clear/close removes it, even outside selected scope. Exclude that out-of-scope pin from the selected scope's columns.
- Introduce typed cursor targets `Empty`, `Header`, `Activity(CompileActivityId)`, `Unattributed(CompileActivityId)`, and `CapturedOutput(OutputSelection)`. The captured-output variant is constructible only for an owned column. Output hit results carry exec-sensitive `BuildSessionId` plus header/activity/output-row identity; empty monitor has a full-pane focusable hit.
- Retain a selected activity by `CompileActivityId(ProcessIncarnation)` and a selected column by `BuildSessionId(ProcessIncarnation)`; never retain cursor identity by `ProcessIdentity` alone. On unit exit choose the row now at prior index, then previous, then header. On session exit choose the session now at prior ordered index, then preceding, then pinned owned, then empty. Scope invalidation retains only a still-present pinned owned selection.
- Render only visible columns/rows while retaining off-screen model state. External rows are live samples, never reconstructed output or synthetic logs.

**Files:**

- `src/tui/panes/output/mod.rs` — presentation and pane-facing API.
- `src/tui/panes/output/pane.rs` — per-column cursor/selection state and reconciliation.
- `src/tui/panes/output/render.rs` — empty/single/multiple/owned rendering and horizontal windowing.
- `src/tui/panes/output/selection.rs` — typed cursor and captured-output-only selection state.
- `src/tui/panes/layout.rs` — presentation-driven Output allocation.
- `src/tui/panes/system.rs` — presentation-driven visibility/tabbability.
- `src/tui/panes/spec.rs` — Output pane capabilities.
- `src/tui/panes/mod.rs` — facade exports.
- `src/tui/app_render_state.rs` — borrow monitor and owned-run state without copying output.
- `src/tui/render_context.rs` — pass the unified presentation inputs.
- `src/tui/render.rs` — render indicator, columns, states, and bottom labels.
- `src/tui/hit_test.rs` — identity-bearing per-column hits and empty-state focus.

**Constraints from prior phases:** Join Phase 8 immutable monitor snapshots with Phase 4 `OwnedRun` by ID. Phase 6 first-seen ordering and exec-sensitive `BuildSessionId`/`CompileActivityId`, plus Phase 5 named scope/index readiness and immediate invalidation, bind cursor fallback and empty-state rendering. A same-PID exec transition creates new session/row identities and invalidates prior cursor/actionability. Do not add key handling or termination.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-port`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-port`, and `bash ~/.claude/scripts/delegate/verify.sh lint cargo-port` are green; render/state tests prove distinct non-actionable pending-index, empty-non-Rust, ambiguous-ownership, and unresolved-path states; empty enabled focus; full-width single session; readable/windowed multiple columns; selected-column visibility; stable row/session fallback; same-PID exec invalidates prior cursor and creates new session/activity identities; unattributed section non-actionability; owned output joined once and pinned across scope changes; and monitor-off output matches prior behavior.

### Phase 10 — Monitor navigation, toggle, and owned-output coexistence · status: todo

#### Work Order

**Goal:** Users can toggle and navigate compile visibility through the framework keymap while existing Output copy, visual selection, pane snaking, and target stopping remain correct.

**Spec:**

- Add a global framework-keymap action for `Shift-C` that toggles monitoring. The state starts off each launch and is not persisted. Toggling off immediately drops external snapshots/tombstones/deadlines, stops polling, and returns to current owned-output behavior.
- Up/Down traverse the selected column's complete body; an owned column crosses activity rows into captured output while skipping the separator. Home/End/Page Up/Page Down/half-page operate vertically in that column. Left/Right and normalized Vim `h`/`l` select adjacent columns without leaving Output.
- Tab/Shift-Tab run an action-aware Output preflight before framework-global pane navigation. Traverse the complete ordered session list, including off-screen columns; with zero/one session or at the first/last boundary, fall through to normal pane-snaking order.
- Mouse clicks select column and typed row through Phase 9 hit rectangles. Horizontal paging follows selected column.
- Copying an activity row copies that row. Visual selection, drag selection, and Ctrl-A may start only within owned captured output and retain frozen-snapshot semantics. Hidden visual selections stay stored but intercept copy/Esc only when their owned output region is selected.
- `Esc` exits active owned visual selection first. While monitoring it stops an owned target only when that owned column is selected; it never stops an unselected external build.
- Add Output action variants through framework keymap only; add no raw `KeyCode` dispatch. Keep termination actions declared/resolvable but unavailable until Phases 12–13. Store defaults portably as `alt-k` and `alt-shift-k`; UI/help/status labels render `Option-K`/`Option-Shift-K` on macOS and `Alt-K`/`Alt-Shift-K` elsewhere. Document/test that macOS terminals must report Option as Meta/Alt and that users may rebind through the existing keymap.

**Files:**

- `src/tui/keymap/actions.rs` — global toggle and Output action variants.
- `src/tui/keymap/canonical.rs` — canonical portable Alt representations.
- `src/tui/keymap/load.rs` — load/migrate the defaults.
- `src/tui/keymap/resolved.rs` — resolve scope-specific shortcuts.
- `src/tui/keymap/constants.rs` — default bindings.
- `src/tui/integration/framework_keymap/app_context.rs` — global `Shift-C` action context.
- `src/tui/integration/framework_keymap/navigation.rs` — adjacent-column and pane-boundary behavior.
- `src/tui/integration/framework_keymap/output_pane.rs` — Output actions and availability.
- `src/tui/integration/framework_keymap/builder.rs` — register actions/defaults.
- `src/tui/integration/framework_keymap/mod.rs` — exports.
- `src/tui/input/dispatch.rs` — modal/Output preflight and typed dispatch.
- `src/tui/keymap_ui/controller.rs` — platform-facing modifier labels and action rows.
- `src/tui/interaction.rs` — identity-bearing mouse navigation.
- `src/tui/panes/output/pane.rs` — vertical/horizontal/tab navigation.
- `src/tui/panes/output/selection.rs` — activity copy and owned-only visual selection.
- `src/tui/app/mod.rs` — execute the toggle and reconcile focus.
- `tests/assets/default-keymap.toml` — pin `shift-c`, `alt-k`, and `alt-shift-k` defaults.

**Constraints from prior phases:** Phase 9's `OutputPresentation` is the sole source of pane/layout/action state. Phase 5 owns toggle and named scope lifecycle, Phase 8 owns conditional polling, and Phase 9 owns typed hit rectangles. Phase 4's owned stop/copy/clear behavior must remain unchanged when monitoring is off.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-port`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-port`, and `bash ~/.claude/scripts/delegate/verify.sh lint cargo-port` are green; interaction tests cover toggle lifecycle, indicator/focus, row and column traversal, tab fallthrough, Vim movement, mouse hits, output separator skipping, activity-row copy, owned-only visual selection, Esc precedence, owned run alongside external updates, portable defaults, macOS Option labels, other-platform Alt labels, and keymap conflict validation.

### Phase 11 — Termination authority and bounded transaction · status: todo

#### Work Order

**Goal:** Fresh build sessions can expose opaque typed termination authority, and `BuildMonitor` can execute a frozen, identity-revalidated bounded termination plan through a separate nonblocking terminator without UI integration.

**Spec:**

- Keep observation and termination separate. Define authority-bearing `ActionableBuild::{Owned(OwnedBuildTerminationAuthority), External(ExternalBuildTerminationAuthority)}` variants. The owned variant retains `OwnedRunId` plus Phase 4's opaque group capability; the external variant bundles confirmed scope attribution, strong root identity, lifecycle eligibility, and an identity-bound platform capability. An identifier by itself is never action authority.
- Add a separate `ProcessTerminator`. `ProcessObserver` produces immutable evidence and safe platform capabilities only; it never accepts termination plans or signals. `ProcessTerminator` executes every bounded transaction on a dedicated worker/channel path so revalidation, signaling, and deadline waits never block the TUI event loop.
- Implement platform adapters that bind signaling to the observed process object strongly enough to reject PID reuse. Use an identity-bound handle or another demonstrated safe adapter where the platform supplies one; a platform without a proven safe adapter exposes external sessions as `ObservedOnly`. Do not assume macOS is observed-only without checking available host APIs, and never fall back to a bare/racy PID action or external ambient process group.
- Only a fresh snapshot with `ActionableBuild` may construct a termination request. Pending, stale, inferred, ambiguous, unattributed, weak-identity, completed, tombstoned, or already-terminating sessions carry no action-bearing handles.
- Define `TerminationRequestId`, `SelectedBuildTerminationAuthorization`, `ScopeTerminationAuthorization`, opaque immutable execution plans, `TerminationOutcomeSummary`, and `TerminationError`. `BuildMonitor` alone creates the selected authorization from one current authority-bearing session and the scope authorization from one exact, all-actionable frozen set; UI code can retain and submit either aggregate but cannot inspect, synthesize, decompose, combine, or subset its authority.
- `BuildMonitor` owns the complete transaction: freeze authorization and scope into the appropriate aggregate, transition sessions to `Terminating`, convert the submitted aggregate into an immutable private plan for `ProcessTerminator`, and reconcile exactly one matching result. Frozen evidence remains owned after observer-cache eviction.
- At execution, re-read the process table and require every frozen identity/scope condition to remain valid. Refresh descendants between bounded passes; admit a newly spawned descendant only while its complete validated parent chain reaches a still-live frozen root. Never first admit a process after its root is gone.
- Exclude Cargo Port, shell/LLM ancestors, persistent `sccache`/`rust-cache`, separate nested sessions, scope-divergent descendants, and compiler units known only by target-directory heuristics. Signal admitted leaves before roots and keep tracking admitted descendants after root exit.
- Owned runs use their isolated process group. Serialize Unix owned-group signaling with child waiting so a reaped group leader cannot be confused with a reused group ID.
- Finish only after all frozen roots/admitted descendants are gone or a deadline returns partial failure. Distinguish already gone from gone after signaling, without claiming the signal caused an exit when observation cannot prove causation; report permission/signal errors, deadline, and survivors truthfully. Do not escalate automatically to `SIGKILL`. A failed surviving identity becomes retryable only after a new fresh observation and later confirmation.

**Files:**

- `src/process_observation/mod.rs` — expose immutable observation evidence and safe platform capability construction without a signal API.
- `src/process_observation/identity.rs` — expose identity revalidation evidence without weakening encapsulation.
- `src/process_termination/mod.rs` — public `ProcessTerminator` worker/channel boundary and correlated request/result API.
- `src/process_termination/platform.rs` — safe identity-bound capability adapters or explicit observed-only fallback.
- `src/process_termination/transaction.rs` — descendant admission, leaf-first signaling, deadline, and truthful result model.
- `src/main.rs` — declare the process-termination module.
- `src/build_monitor/model.rs` — actionable/observed-only session types and termination lifecycle states.
- `src/build_monitor/termination.rs` — frozen authorization, request IDs, transaction ownership, and reconciliation.
- `src/build_monitor/mod.rs` — expose request construction only from fresh actionable sessions.
- `src/tui/terminal/processes.rs` — adapt existing owned process-group stop to the serialized transaction boundary.
- `src/tui/messages.rs` — carry correlated termination plan/results.
- `src/tui/background.rs` — host the dedicated nonblocking termination worker.
- `src/tui/app/async_tasks/poll.rs` — reconcile correlated termination results without event-loop blocking.
- `Cargo.toml` — add only platform dependencies needed for identity-bound external signaling.

**Constraints from prior phases:** Phase 2 defines strong/insufficient identity, exec-sensitive incarnation, chronological identity evidence, and validated ancestry; Phase 6 keys confirmed exact scope/session/activity association by `ProcessIncarnation`; Phase 8 owns fresh/stale lifecycle; Phase 4 owns isolated process groups and `OwnedProcessGroupTerminationCapability`. A same-PID exec transition invalidates classification and actionability, while opaque authorization retains the current strong `ProcessIdentity` needed for immediate revalidation. Phase 3's `RunningTargetTerminationCapability` remains a separate legacy consumer and can never be reused as build-monitor authority. `ProcessObserver` remains observation-only, all new build signaling runs through `ProcessTerminator` off the event loop, and no UI may synthesize or decompose opaque action authority.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-port`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-port`, and `bash ~/.claude/scripts/delegate/verify.sh lint cargo-port` are green; deterministic process-fixture tests prove PID reuse and same-PID exec rejection before signaling, safe-adapter or observed-only fallback, owned/external actionability always carries authority rather than an ID, only `BuildMonitor` constructs opaque selected/scope authorization aggregates, UI code cannot reconstruct/combine/subset them, exact request/result correlation, termination work stays off the event loop, immediate strong-identity revalidation, validated descendant admission, no post-root first admission, leaf-before-root order, continued tracking after root exit, exclusions, owned group/wait serialization, no automatic `SIGKILL`, and truthful already-gone/gone-after-signal/partial-failure outcomes.

### Phase 12 — Selected-build termination interaction · status: todo

#### Work Order

**Goal:** From an actionable selected Output column, `Alt-k` (`Option-K` on macOS labels) opens a modal confirmation and safely terminates that entire root build.

**Spec:**

- The selected compiler/activity row identifies cursor location only; selected-build termination always targets the owning root Cargo invocation.
- Expose the action only when the selected column's fresh session has authority-bearing `ActionableBuild::Owned` or `ActionableBuild::External`. Headers and activity rows may invoke it; unattributed, pending, stale, observed-only, completed, killed, failed-unrefreshed, and terminating sessions cannot.
- Ask `BuildMonitor` to construct one opaque `SelectedBuildTerminationAuthorization` for the selected root. Confirmation shows operative command, checkout, PID, start age, and current observed compiler-child count as separate display data while retaining that aggregate; UI code must not rebuild authority from `BuildSessionId`, scope, root identity, PID, or the display data.
- Confirmation is modal and consumes input before Output cancellation, globals, copy, or navigation: `y` submits the frozen request; `n` or `Esc` cancels; all other keys do nothing.
- Before signaling, Phase 11's submitted selected authorization requires the frozen session identity and scope still match a fresh observation. Exit becomes an already-gone toast, scope/identity mismatch rejects the request, and no replacement process at the PID is touched.
- Render `Terminating` until the correlated Phase 11 transaction completes. Retain a selected-build gone-after-signal tombstone until a new build replaces it, scope changes, or monitoring toggles off; do not label an external process “killed” when only disappearance after a signal is observed. On errors/deadline/survivors render a visible partial failure; enable retry only after a new fresh actionable snapshot and confirmation.
- Preserve existing `Esc` owned-run stop behavior outside the modal and when monitoring is off.

**Files:**

- `src/tui/app/confirm_action.rs` — selected-build confirmation payload retaining `SelectedBuildTerminationAuthorization` plus separate display data.
- `src/tui/app/mod.rs` — construct/submit requests and reconcile toasts/state.
- `src/tui/input/dispatch.rs` — modal priority and `y`/`n`/Esc handling.
- `src/tui/integration/framework_keymap/output_pane.rs` — availability and dispatch for `alt-k`.
- `src/tui/panes/output/pane.rs` — selected session/row lookup.
- `src/tui/panes/output/render.rs` — terminating, gone-after-signal, already-gone, and partial-failure markers.
- `src/tui/render.rs` — modal confirmation and status/toast presentation.
- `src/tui/messages.rs` — carry selected request/result IDs through the Phase 11 transaction channel.
- `src/build_monitor/termination.rs` — construct a one-session frozen plan.

**Constraints from prior phases:** Use Phase 10's framework action and platform label; retain and submit only Phase 11's `SelectedBuildTerminationAuthorization` without reconstructing or substituting a scope-wide aggregate. Phase 5 scope changes make an open request invalid, Phase 9 owns exec-sensitive selection identity/fallback, and the legacy Running Targets termination capability remains unrelated.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-port`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-port`, and `bash ~/.claude/scripts/delegate/verify.sh lint cargo-port` are green; interaction tests prove modal precedence, selected-authorization retention without UI authority reconstruction, exact frozen scope/session, root-not-row semantics, stale/inferred/ambiguous/weak-state unavailability, PID exit/reuse safety, truthful terminating/already-gone/gone-after-signal/partial-failure states, fresh-observation retry, and no effect on unrelated builds/cache daemons.

### Phase 13 — Scope-wide termination and end-to-end verification · status: todo

#### Work Order

**Goal:** `Alt-Shift-k` (`Option-Shift-K` on macOS labels) safely terminates exactly the live actionable roots in the selected scope, with final end-to-end proof of the complete feature.

**Spec:**

- Scope-wide termination requires a nonempty live root set and refuses to open if any represented live root is observed-only; “all” never means a silent actionable subset.
- Build the set from the current `MonitorScopeKey` only. Exclude pinned owned output outside the selected scope, completed runs, tombstones, unattributed compiler units, and duplicate/nested references to the same root.
- Ask `BuildMonitor` to create one opaque `ScopeTerminationAuthorization` from the current exact all-actionable root set. Confirmation names the selected scope and deduplicated `BuildSessionId` set as separate display data while retaining that aggregate; UI code never reassembles, combines, or subsets authority from displayed scope/session IDs. Any scope/metadata/project-list revision change invalidates the authorization.
- A build starting after confirmation is never added to destructive authority; leave it running and report that a newer build was not included. A root that already exited is `gone`, never replaced by a new process at the PID.
- Submit the opaque exact frozen-set authorization through Phase 11's one bounded transaction. Render per-root and aggregate terminating, gone-after-signal, already-gone, survivor, and error outcomes truthfully. Retain gone-after-signal tombstones until scope change, replacement build, or monitor off.
- Complete focused automated coverage for simultaneous debug/release, linked/group worktree scope, unique versus ambiguous cache-wrapper attribution, owned target plus external build/Cargo-lock wait, selected versus scope kill, and disabled polling.
- Perform live verification on macOS where available: debug and release in one checkout; builds in two linked worktrees with group versus checkout scope; `RUSTC_WRAPPER=rust-cache`/`sccache`; owned target launch beside an external build including Cargo-lock wait; selected kill preserving unrelated builds/cache daemon; scope kill affecting only deduplicated scoped roots; toggle off ceasing compile work. If an external platform adapter is intentionally unavailable, verify observed-only rendering/action unavailability rather than using unsafe fallback.

**Files:**

- `src/tui/app/confirm_action.rs` — scoped confirmation payload retaining `ScopeTerminationAuthorization` plus exact-set display data.
- `src/tui/app/mod.rs` — create, submit, and reconcile scope-wide requests.
- `src/tui/input/dispatch.rs` — modal input for the scope action.
- `src/tui/integration/framework_keymap/output_pane.rs` — availability/dispatch for `alt-shift-k`.
- `src/build_monitor/termination.rs` — deduplicate and freeze the all-scope transaction.
- `src/build_monitor/model.rs` — per-root and aggregate results/tombstones.
- `src/tui/panes/output/render.rs` — scoped transaction outcome rendering.
- `src/tui/render.rs` — scope confirmation and completion toast.
- `tests/assets/default-keymap.toml` — verify the final generated fixture matches the Phase 10 defaults.

**Constraints from prior phases:** Phase 12 establishes modal selected termination; reuse its input path while retaining Phase 11's distinct `ScopeTerminationAuthorization` for set-wide authority. Phase 11's aggregate/transaction semantics plus Phase 5 exact scope generations bind the frozen set; UI code never reconstructs, combines, or subsets it. Phase 4 pinned owned output remains outside unrelated scope-wide authority.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-port`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-port`, and `bash ~/.claude/scripts/delegate/verify.sh lint cargo-port` are green; automated tests prove all-or-refuse actionability, opaque scope-authorization retention without UI reconstruction/combination/subsetting, root deduplication, new-build exclusion, gone-versus-reused identity, pinned-owned exclusion, modal priority, truthful exact scoped outcomes, compile-monitor-off quiescence, and the generated keymap fixture matches Phase 10 defaults; the live verification matrix is completed with any platform-observed-only limitation recorded without weakening safety.
