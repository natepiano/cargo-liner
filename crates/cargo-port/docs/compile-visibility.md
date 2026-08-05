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
  - `src/tui/compile_visibility/` — new `CompileVisibilityState`, row-kind-aware `MonitorScopeKey`, named scope resolution, scope revisions, generation invalidation, and App-facing actions. `MonitorScopeKey` stays inside `crate::tui` because it contains `VisibleRow`; `src/build_monitor/` sees only its roots-and-revisions projection, `BuildScopeKey`, produced by a `From` conversion.
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
- **Invariants:** Compile visibility starts disabled, is not persisted, and while off owns no compile-specific deadline, new refresh demand, command parsing, classification, snapshot, tombstone, generation, or late-result acceptance; cancellation prevents an already-observing shared worker request from entering compile classification after toggle/scope invalidation while preserving any coalesced Running Targets result. Existing Running Targets retains its one-second behavior and owned-target Output behavior remains unchanged. One App-owned, dedicated-worker `ProcessRefreshExecutor` owns exactly one `ProcessObserver`, coalesces simultaneous consumer demand into one refresh cycle, and returns `CompletedProcessRefreshExecution` with independent Running and compile-consumer outcomes; one App-owned revision-keyed `CargoWorkspaceIndex` serves Running Targets and Build Monitor without launching Cargo when monitoring starts. That index is held as an `Arc<CargoWorkspaceIndex>` and rebuilt by replacement — `rebuild_if_changed` swaps in a fresh index rather than mutating in place — so a worker request can hold the accepted index across a thread boundary without borrowing `App`; the cost is one allocation per accepted-metadata change, and an in-flight request that spans a rebuild keeps the older index, which its revision stamps and generation already identify. The shared index explicitly reports `Current`, `RetainedLastAccepted`, or `Uninitialized`; consumers preserve the last accepted index on refresh failure, and only an uninitialized index may use a named fallback. `ProjectListRevision` changes only when visible ownership content changes; selected-row identity is separate monitor-scope input. Scope is a typed row-kind-aware `MonitorScopeKey` over sorted canonical checkout/workspace roots plus metadata/project-list revision; workspace members resolve to their owning workspace, groups differ from primary checkout rows, and non-Rust scopes are empty. A changed key republishes the snapshot as pending: retained live data remains actionable only while its covered roots are unchanged, while stale data or changed covered roots are immediately non-actionable. Build sessions and activity rows are keyed by exec-sensitive `ProcessIncarnation`, while termination authorization retains strong `ProcessIdentity`; neither uses a bare PID. Weak, stale, ambiguous, or unattributed evidence is observed-only, and system-wide cache-daemon ambiguity is rendered once rather than resolved arbitrarily. A scope resolved by any of the four methods — owned launch, manifest argument, working directory, or unique output directory — is actionable; only an unresolved scope is not. Process observation and termination are separate: `ProcessObserver` produces immutable evidence and capabilities, while `ProcessTerminator` performs identity-revalidated signaling off the TUI event loop. External termination requires an identity-bound platform capability and opaque frozen scope/identity authorization; never signal an ambient process group, Cargo Port, shell/LLM ancestors, cache daemons, or divergent nested sessions. Selected-scope kill refuses partial actionability, never absorbs builds started after confirmation, and bounded leaf-before-root termination reports already gone, gone after signaling, survivors, and errors truthfully without claiming causation it cannot prove or automatically using `SIGKILL`. `OwnedRun` solely owns lifecycle/output; every message carries `OwnedRunId`; its observed activity is joined, not copied, and pinned owned output can coexist with external columns while remaining outside unrelated scope-wide kills. A single `OutputPresentation` controls rendering, layout, focus, tabbability, labels, copy, and hit testing; typed cursors permit visual selection/Ctrl-A only in owned captured output, while columns/navigation preserve stable identities and Tab/Shift-Tab preflight falls through at session boundaries. Defaults are framework-keymap actions only: global `Shift-C`, Output `alt-k` for selected build, and `alt-shift-k` for all scoped builds; render `Option-K`/`Option-Shift-K` on macOS and `Alt-K`/`Alt-Shift-K` elsewhere, with no raw `KeyCode` dispatch outside the keymap. Open termination confirmation is modal above Output/global input. Preserve one Cargo Port-owned run at a time, strict workspace lints/missing docs, `RUSTC_WRAPPER`, nightly formatting for this `natepiano` origin, and inline focused tests plus 1,000/5,000-process refresh benchmarks proving no persistent monitor-off CPU work.

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
- Phase 6 now defines `BuildClassifier` as the owner of dependency-manifest and first-seen support state around the pure classifier.
- Phase 7 now extends the already-selected dedicated worker, places the classifier beside the sole observer, returns independent compile outcomes inside `CompletedProcessRefreshExecution`, supports cooperative generation cancellation, and moves shared reconciliation to a neutral App adapter.
- Phase 8 now uses a 500 ms semantic disabled/due/in-flight schedule, ages monitor data for compile-only or whole-cycle failures without discarding successful Running results, and cancels in-flight classification after shared observation.
- Phase 9 now retains cursor identity only by exec-sensitive activity/session IDs.
- Phases 11–13 now carry authority-bearing owned/external actionability and distinct opaque selected/scope termination aggregates created only by `BuildMonitor`; Phase 13's keymap fixture correctly points back to Phase 10 defaults.
- No user decisions were required; every change follows Phase 3's measured worker choice, monitor-off invariant, exec-sensitive identity boundary, and existing termination-safety requirements.

### Phase 4 — Correlated Cargo Port-owned runs · status: done

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

#### Retrospective

**What worked:**

- `OwnedRun` now solely owns semantic queued, starting, running, stopping, retained, gone-after-signal, and failed states, with every asynchronous message correlated by monotonic `OwnedRunId`.
- `OwnedProcessGroupTerminationCapability` consumes one verified root identity, and target launches use semantic workspace-default or named-package invocation without PID-only authority.

**What deviated from the plan:**

- Retained output needed its own `OwnedRunOutputIdentity` because run N's visible output can remain while run N+1 is queued or starting.
- Failed identity verification now terminates the isolated group with `TERM` then `KILL`, waits for the owned root, and checks for group disappearance; workspace-root target rows always name their owning package.

**Surprises:**

- Closing Output during `Stopping` must clear only the captured lines and preserve the active run slot until its correlated `Finished` message arrives.
- Pipe readers flush after a stop request, so final stopping reconciliation must relocate one killed marker after all late output and progress.

**Implications for remaining phases:**

- Phase 8 must join an observed owned session by verified root identity and retain `OwnedRunId`; it cannot infer captured-output ownership from the current lifecycle ID when older output remains visible.
- Phases 9–10 must preserve output identity, stopping-slot retention, and final marker ordering when building presentation, selection, copy, close, and `Esc` behavior.
- Phases 11–13 must consume the opaque owned-group capability without reconstructing group or process identity, while keeping external termination authority separate.

#### Phase 4 Review

- Phase 5 now keys scope generation by an exact `MonitorSelectedRowIdentity`, maps every current `VisibleRow` variant, and postpones snapshot/deadline/tombstone state until Phase 8.
- Phase 6 now creates immutable live/stopping owned-root and candidate evidence, replaces saturating `OwnedRunId` allocation with explicit exhaustion, and never exposes opaque termination authority to classification.
- Phases 7–8 now carry semantic compile demand/cancellation and keep current lifecycle, verified live root, and retained-output producer identities distinct across worker polling and owned-session joins.
- Phase 9 now derives `OwnedOutputPresentation` from a semantic retained-output producer state rather than the current lifecycle ID.
- Phase 10 now preserves stopping-slot retention, late correlated pipe output, and exactly one final gone-after-signal marker.
- Phase 11 now distinguishes failed-launch `TERM`/`KILL` cleanup from user-requested termination and defers its owned-authority ownership/splitting decision to its pre-dispatch gate.
- Phases 12–13 now consume Phase 11's established authorization/transaction APIs without reimplementing selected or scope freezing.
- No user decision blocks Phases 5–10; the one unresolved destructive-authority architecture decision is deferred to Phase 11.

### Phase 5 — Typed monitor scope and state shell · status: done

#### Work Order

**Goal:** The selected project-list row resolves to a stable, row-kind-aware compile-monitor scope, and monitor state can be on or off without polling or rendering builds yet.

**Spec:**

- Add `src/tui/compile_visibility/` with `CompileVisibilityState::{Off, On(ActiveMonitorState)}`. In this phase `ActiveMonitorState` owns only selected-row identity, scope resolution, and generation; Phase 8 extends the enabled aggregate with snapshots, tombstones, deadlines, and late-result acceptance. Toggling off drops the entire enabled aggregate.
- Define `MonitorSelectedRowIdentity` from the exact typed selected row, separate from content-only `ProjectListRevision`, and include it with selected row kind, sorted canonical checkout/workspace roots, metadata revision, and project-list revision in `MonitorScopeKey`. A worktree-group row and its primary checkout row remain different scopes even when they share a path, and selecting two rows that resolve to the same workspace still advances scope generation.
- Add a shared App-facing workspace-index adapter with `WorkspaceIndexReadiness::{Current, RetainedLastAccepted, Uninitialized}` so Running Targets and compile visibility consume the same readiness decision instead of duplicating private logic.
- Resolve the selected row as `MonitorScopeResolution::{Ready(MonitorScopeKey), EmptyNonRust, PendingIndex, AmbiguousOwnership, UnresolvedPath}` or an equally semantic exhaustive type. A bare `Option<MonitorScopeKey>` is not permitted; only `Ready` can become actionable.
- Resolve package/workspace rows to the owning workspace checkout; linked-worktree checkout rows include only that checkout; worktree-group rows include the primary and every represented live linked checkout; vendored packages/submodules use their own Cargo workspace when metadata proves one and otherwise their containing checkout; non-Rust rows produce an empty scope.
- Map every current `VisibleRow` variant exhaustively: `Root` uses its root/worktree-group rule; `GroupHeader` uses its containing primary checkout; `Member` uses its owning workspace checkout; `WorktreeEntry` uses only that checkout; `WorktreeGroupHeader` and `WorktreeMember` use their containing linked checkout; `MemberVendored`, `Vendored`, `WorktreeMemberVendored`, `WorktreeVendored`, and `Submodule` use a metadata-proven nested Cargo workspace or fall back to their containing checkout. No wildcard arm may silently assign future row kinds.
- Define monotonic `CompileMonitorGeneration` and advance it on toggle/scope replacement. A selection, membership, metadata, or project-list revision change immediately replaces the scope key, makes the prior snapshot non-actionable, and leaves the new state pending until its first matching snapshot. Late results carry the generation and are ignored after replacement.
- Do not launch Cargo or refresh process data when resolving scope.

**Files:**

- `src/tui/compile_visibility/mod.rs` — state shell, toggle lifecycle, generations, and exports.
- `src/tui/compile_visibility/scope.rs` — `MonitorScopeKey` and selected-row resolution.
- `src/tui/workspace_index.rs` — shared App adapter for accepted-index readiness and query access.
- `src/tui/mod.rs` — declare the compile-visibility module and the shared workspace-index adapter.
- `src/project/cargo/workspace_index.rs` — consume exact index identities and expose any missing named query result required by scope resolution.
- `src/project/cargo/workspace_index_api_tests.rs` — prove the shared scope queries preserve exact ownership and ambiguity.
- `src/project/root_item.rs` — expose canonical checkout ownership required by scope construction.
- `src/project/git/worktree_group.rs` — expose represented visible checkout roots.
- `src/tui/project_list/visible_rows.rs` — expose typed row kind.
- `src/tui/project_list/list.rs` — provide current selected row and revision.
- `src/tui/project_list/mod.rs` — export typed selection data.
- `src/tui/app/mod.rs` — own `CompileVisibilityState`, initially `Off`.
- `src/tui/running_targets/app_tick.rs` — consume the shared readiness adapter instead of private index-selection logic.
- `src/tui/app/async_tasks/metadata_handlers.rs` — publish accepted metadata before resolving scope, then refresh once.
- `src/tui/app/async_tasks/poll.rs` — re-resolve the enabled scope when a background batch changed `ProjectListRevision`.
- `src/tui/app/construct.rs` — initialize `CompileVisibilityState::Off` and the generation counter.
- `src/project/mod.rs`, `src/project/cargo/mod.rs` — re-export the index identities scope resolution consumes.

**Constraints from prior phases:** Read exact canonical workspace/member/package/target data only from Phase 1's revision-keyed index and preserve its current/retained/uninitialized semantics. Replace the private readiness adapter in `running_targets/app_tick.rs` with the shared adapter without moving filesystem attribution ahead of the existing one-second cadence/readiness gate, weakening retained-last-accepted behavior, or admitting cross-workspace ambiguous owners. `ProjectListRevision` changes only for visible ownership content; `MonitorSelectedRowIdentity` is a separate scope input and every selection change replaces generation even when canonical roots are unchanged. Process identities and snapshots from Phases 2–3 must not leak into stale scope state. Phase 3's dedicated executor, `CompletedProcessRefreshExecution`, failure timing, and one-counted raw Running metrics refresh boundary remain unchanged. Phase 4 owned output and active-run lifecycle remain independent.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-port`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-port`, and `bash ~/.claude/scripts/delegate/verify.sh lint cargo-port` are green; focused tests cover every current `VisibleRow` variant, primary, linked, grouped, workspace-member, nested-workspace/fallback vendored/submodule, and non-Rust selections; distinguish current, retained, uninitialized, empty, pending, ambiguous, and unresolved states; prove group and primary keys differ; prove selecting two rows with the same resolved roots changes `MonitorSelectedRowIdentity` and generation without revising visible content; prove scope/toggle changes invalidate prior generations immediately; prove `Off` owns no monitor deadline or snapshot and Phase 5 creates no placeholder snapshot/deadline/tombstone state; and prove Running Targets still evaluates cadence/readiness before attribution, retains the last accepted index, omits cross-workspace ambiguity, and performs one raw metrics refresh per due cycle.

### Retrospective

**What worked:**
- `MonitorScopeResolution::{Ready, EmptyNonRust, PendingIndex, AmbiguousOwnership, UnresolvedPath}` carried every non-actionable case without a bare `Option<MonitorScopeKey>`; only `Ready` reaches `MonitorScopeActionability::Actionable`.
- Separating `MonitorSelectedRowIdentity` from `ProjectListRevision` made "two rows, same roots, still a new generation" fall out of `requires_scope_replacement` rather than needing special-case logic.
- `WorkspaceIndexReadiness` as a shared App adapter replaced Running Targets' private readiness logic with no change to its cadence, retention, or ambiguity behavior.

**What deviated from the plan:**
- The compile-visibility module is declared at `src/tui/mod.rs`, not `src/main.rs` as the Files list said; `src/tui/running_targets/state.rs` needed no change. Both corrected in the Files list.
- Four files the plan did not list were required: `async_tasks/metadata_handlers.rs`, `async_tasks/poll.rs`, `app/construct.rs`, and re-export plumbing in `src/project/mod.rs` + `src/project/cargo/mod.rs`.
- Two helpers were added that the spec did not name: `VisibleRow::node_index()` and `MonitorScopeUpdate::into_parts()`.
- `represented_live_checkout_roots` shipped as `represented_visible_checkout_roots` — its `Visibility::Visible` filter is narrower than the group's live entries, and the original name contradicted `live_entry_count` / `single_live` / `renders_as_group`, which all count `Deleted` as live.

**Surprises:**
- `MonitorScopeKey` alone is not enough to keep non-actionable resolutions distinct across selections; `MonitorScopeResolutionRevision` (project-list revision + index readiness) had to be threaded through every non-`Ready` variant, and it participates in replacement equality.
- Scope refresh needs three separate triggers, not one: selection (`sync_selected_project`), background project-list revision changes (`poll_background`), and accepted-metadata arrival. Background handlers advance `ProjectList::revision` without routing through `sync_selected_project`.
- macOS `/tmp` and `/var` symlink into `/private`, so scope fixtures must canonicalize their base directory or expectations disagree with `MonitorScopeKey`'s canonical roots.
- The phase-5 surface has no runtime caller until the keybinding lands, producing 18 `dead_code` sites suppressed per-item following the phase-1 precedent.

**Implications for remaining phases:**
- Phase 10 makes `toggle_compile_visibility` reachable, which puts scope refresh on the input-event path. Resolved: Phase 10's Constraints keep `workspace_index_readiness` on that path unchanged, with the measured per-event cost recorded there.
- Every `#[allow(dead_code, ...)]` added in this phase must be removed by whichever phase first consumes that item; the reason strings name the consuming subsystem to make that findable. Phases 6, 8, 9, and 10 now each name the suppressions their acceptance gate must delete.
- Phase 8 extends `ActiveMonitorState` with snapshots, tombstones, and deadlines. `ActiveMonitorState::new` and `replace_scope` consume `MonitorScopeUpdate` via `into_parts()`, so added state must be supplied by the caller, not cloned from the update.
- `MonitorScopeKey` cannot cross out of `crate::tui` — it contains `VisibleRow`, which is `pub(super)` there. Phase 6 defines the roots-and-revisions projection `BuildScopeKey` and a `From<&MonitorScopeKey>` conversion; that is what phases 7, 8, and 13 pass across the boundary.

### Phase 5 Review

An architect pass over the remaining phases returned eleven findings. Seven were
applied directly; three went to the user and were decided; one was mechanical.

Applied without a gate:

- Phase 8's refresh schedule loses its `Disabled` variant and moves inside
  `ActiveMonitorState` — `CompileVisibilityState::{Off, On(..)}` already makes
  "disabled owns no schedule" a type-level fact, and a parallel variant would be
  a second source of truth that can disagree with it.
- Phase 8 gains an ordering requirement: `poll_background` must call
  `ensure_visible_rows_cached()` before refreshing scope. `disk_handlers.rs:78`
  advances the project-list revision without recomputing rows, so that batch
  currently resolves scope from a stale row cache. It self-corrects next frame
  today, but phases 11–13 make scope authoritative for termination.
- Phase 8's constraints now name all three callers a change to
  `ActiveMonitorState` forces: `enable`, `replace_scope`, and
  `App::toggle_compile_visibility`. `ActiveMonitorState::new` is private and
  reachable only through them.
- Phases 6, 8, 9, and 10 each name the `dead_code` suppressions their gate must
  delete. The reason strings name subsystems, not phases, and one spans two
  phases, so nothing previously required their removal.
- Phase 9 gains `monitor_workspace_index_readiness()` as a rendering input for
  the four non-`Ready` empty states — it had no consumer in any remaining phase.
  The `Ready` asymmetry (an actionable scope records no readiness) is recorded
  there as a known limitation rather than fixed.
- Phase 6's `workspace_index.rs` Files entries narrow to the package and target
  queries that remain; Phase 5 already shipped the workspace and ambiguity paths.
- Phase 10's `src/tui/app/mod.rs` entry becomes focus reconciliation plus
  suppression removal. The toggle body and off-at-construction already shipped.

Corrected: the `**Pending decision:**` block about scope refresh on the
input-event path was attached to Phase 6, which touches no keymap or input file.
It moved to Phase 10, which adds the `Shift-C` action, and gained the fact that
`refresh_compile_monitor_scope_if_on` returns before touching the index while
the state is `Off` — nothing is paid until the user enables monitoring.

User decisions:

- **`MonitorScopeKey` cannot leave `crate::tui`.** Rejected widening `VisibleRow`
  out of `tui`; chose a separate roots-and-revisions type in `build_monitor`
  with an `impl From<&MonitorScopeKey>` boundary so the conversion and the
  dropped fields are visible at one definition. `build_monitor` has no use for
  the project list's eleven display row kinds.
- **The workspace index moves to `Arc<CargoWorkspaceIndex>`**, rebuilt by
  replacement rather than mutated in place, so a worker request can hold it
  across a thread boundary. Rejected copying a per-request snapshot: that would
  copy every 500 ms and requires predicting what Phase 6 will query. Phase 7
  owns the conversion and consumes it; Phase 6 has no caller that holds the
  index across a thread boundary.
- **A scope replacement with unchanged canonical roots retains the last good
  snapshot.** Retention is gated on comparing the roots directly, never on the
  generation or revision stamps — those advance on any background ownership
  change, which would blank the display on a 500 ms cadence during a scan.

### Phase 6 — Cargo build and compiler classification · status: done

#### Work Order

**Goal:** Pure build-monitor classification converts an immutable system snapshot plus workspace index into stable build sessions and active compile units, leaving ambiguous attribution non-actionable.

**Spec:**

- Define `BuildScopeKey` in `src/build_monitor/scope.rs` — sorted canonical checkout roots, sorted canonical workspace roots, `AcceptedCargoMetadataRevision`, and `ProjectListRevision`. It does **not** carry `MonitorSelectedRowIdentity`. It inherits `MonitorScopeKey`'s sort/dedup invariant (`src/tui/compile_visibility/scope.rs:73-78`); the conversion must not re-sort. Phase 5's `MonitorScopeKey` cannot be named outside `crate::tui` today for one reason only: `src/tui/compile_visibility/mod.rs:3` declares `mod scope;` privately and does not re-export the type. Its `VisibleRow` content is **not** the blocker — `VisibleRow` is declared `pub` (`src/tui/project_list/visible_rows.rs:35`) and merely narrowed by a `pub(super)` re-export (`src/tui/project_list/mod.rs:11`), and every `MonitorScopeKey` field is private, so a `pub(crate)` re-export would name the type crate-wide without giving `build_monitor` any path to `VisibleRow`. Widening `VisibleRow` out of `tui` was considered and rejected: it would make `build_monitor` depend on the project list's eleven display row kinds, and the row identity exists only so the `tui` side can tell whether the highlight moved. Provide the boundary as `impl From<&MonitorScopeKey> for BuildScopeKey` so the direction of the conversion and the fields that are dropped are both visible at the definition; never hand-roll the conversion at a call site. Define that impl in `src/tui/compile_visibility/mod.rs`, and expose the boundary from that module as one entry point taking a `&MonitorScopeResolution` and returning `BuildScopeActionability::{Actionable(BuildScopeKey), NotActionable}` — mirroring the shipped `MonitorScopeActionability::{Actionable, NotActionable}` rather than inventing a second vocabulary, and giving Phase 7 a two-state answer instead of five resolution states it would otherwise have to know. Reach the key through the shipped `MonitorScopeResolution::actionability()` (`src/tui/compile_visibility/scope.rs:174-187`), not by matching `Ready` at the call site: both yield the same keys, since `MonitorScopeKey::new` is private with its sole call site inside `Ready` (`:66`, `:449`) and no key can exist outside an actionable resolution — but routing through the accessor keeps the actionable/not-actionable rule in one place and gives `MonitorScopeActionability` its intended consumer. Both types are crate-local so the impl may live in any module, and this home has two consequences worth having: `mod.rs` is outside `scope.rs` and therefore cannot read `MonitorScopeKey`'s private fields, so the conversion must go through the four canonical accessors and their suppressions are consumed rather than left to a judgment call; and `MonitorScopeKey` needs no visibility widening at all, since only `BuildScopeKey` crosses out of `crate::tui`. Phase 5's constraint that `MonitorScopeKey` never leaves `crate::tui` therefore holds without a re-export. Two rows that resolve to the same roots and revisions produce equal `BuildScopeKey` values while remaining distinct `MonitorScopeKey`s — that is intended, and generation is what keeps their results apart.
- Add domain identifiers: `BuildSessionId(ProcessIncarnation)`, `CompileActivityId(ProcessIncarnation)`, `ScopeAttribution::{OwnedRoot, ManifestArgument, WorkingDirectoryManifest, UniqueOutputDirectory, Unresolved}`, `CompilerAttribution::{Confirmed, UniqueOutputMatch, Ambiguous { candidates }, Unattributed}`, and presentation-only session/activity records. `ScopeAttribution` names the method that resolved the scope rather than a confidence tier: all four resolution methods are actionable, and only `Unresolved` is not. Keep it a plain unit-variant enum with a `const fn is_resolved(self) -> bool` — the method is recorded so a misattribution is diagnosable from the pane and so one method can be demoted later without a rewrite, not so that call sites branch on it today. `Unresolved` covers a Cargo process with no relationship to anything in the project list; the Spec previously had no state for it and would have forced such a session to claim an owner. Reuse Phase 1's `cargo_metadata::PackageId`; do not introduce duplicate `BuildScopeId` or `PackageId` wrappers. Raw PIDs never stand alone in an actionable type; `ProcessIdentity` remains available inside later signaling authorization but does not key a session or row across exec.
- Make both IDs opaque so that rule is a compiler fact rather than prose: private inner field, `Eq`/`Hash`/`Ord` on the newtype, and **no** accessor returning `&ProcessIncarnation` or `&ProcessIdentity`. `ProcessIdentity` derives `Hash` and `Ord` today (`src/process_observation/identity.rs:88`), so any reach-through makes `HashMap<ProcessIdentity, _>` compile and a same-PID exec silently collide two sessions. Private inner fields also stop `CompileActivityId(session_id.0)` type-checking. Construct each only from its validated role — `BuildSessionId::from_recognized_root(&RecognizedCargoRoot)` and `CompileActivityId::from_recognized_compiler(&RecognizedCompilerProcess)`. Later phases obtain `ProcessIdentity` from the separate signaling-authority value, never from an ID.
- Never construct `CompilerAttribution::Ambiguous` at a call site: collect candidates into a private newtype and produce the enum from one slice match — `[]` is `Unattributed`, `[_]` is the unique case, anything longer is `Ambiguous`. Otherwise `Ambiguous` with zero or one candidate restates two other variants. This mirrors the shipped `CanonicalWorkspaceCandidates::ownership_evidence` (`src/project/cargo/workspace_index.rs:270-278`).
- Define immutable `OwnedRootEvidence::{NoLiveRoot, Root(LiveOwnedRoot)}` (or equally semantic states) from Phase 4's aggregate, where `LiveOwnedRoot` carries `owned_run_id`, `root_identity`, `launch_directory`, and an `OwnedRootLifecycle` field distinguishing live from stopping. Do not give live and stopping separate variants with identical payloads: for classification they are the same fact — a stopping run's compiler descendants are still live — so every call site would match `Live { .. } | Stopping { .. }` and destructure twice. The lifecycle distinction matters to the later signaling phases, which read the field. The verified `ProcessIdentity` and launch directory cross classification as immutable observation; the opaque termination capability remains private and cannot be decomposed.
- Discover the outermost recognized root build in a validated Cargo process chain. Normalize rustup proxies, `cargo-*` plugins, and nested Cargo. Classify the subcommand into `CargoSubcommandRecognition::{Build, NonBuild, Unrecognized}` and enumerate the first two so an implementer does not have to invent them:
  - `Build` — `bench`, `build`, `check`, `clippy`, `doc`, `fix`, `install`, `nextest`, `package`, `publish`, `run`, `rustc`, `rustdoc`, `test`. `install`, `package`, and `publish` belong here because they compile; a compile the user can see is a compile the user can stop, even when its output lands outside the project's target directory.
  - `NonBuild` — `add`, `clean`, `config`, `fetch`, `generate-lockfile`, `help`, `init`, `locate-project`, `login`, `logout`, `metadata`, `new`, `owner`, `pkgid`, `read-manifest`, `remove`, `search`, `tree`, `uninstall`, `update`, `vendor`, `verify-project`, `version`, `yank`.
  - `Unrecognized` — everything else, which is exactly the set that cannot be decided from argv alone: configured aliases from `.cargo/config.toml` (`cargo ck` where `ck = "check --workspace"`) and third-party plugins (`cargo miri`, `cargo llvm-cov`). Reading `.cargo/config.toml` is filesystem I/O the pure classifier must not do, and the alias table is per-checkout, so do not try to resolve these by name. Normalize only Cargo's own built-in aliases, which are fixed and need no file read: `b`, `c`, `d`, `r`, `t`, `rm`, `ver`.
- Run recognition and compiler association as two passes in that order, because the second decides part of the first. Pass one identifies Cargo roots and their subcommand recognition; pass two attaches compiler descendants. A root whose recognition is `NonBuild` or `Unrecognized` becomes a build session in pass two if and only if a live compiler descendant attaches to it — the same promotion rule the plan already applies to non-build commands, now covering aliases and plugins with no second mechanism. Promotion is sticky for the process incarnation's lifetime: once a Cargo process has been observed with a compiler descendant it stays a build session until it exits, so a gap between one crate finishing and the next starting does not make its row disappear and reappear. The visible cost is that an aliased or plugin build appears one refresh (~500 ms) after a directly recognized one.
- Resolve scope for every Cargo node before normalizing. A nested Cargo belongs to the outer root only when confirmed scope and termination boundary match; a plugin/alias entering another checkout becomes a separate session. Discover compatible roots system-wide before filtering the selected scope.
- Resolve root scope in order: verified Cargo Port-owned root `ProcessIdentity` plus launch directory; `--manifest-path` or absolute manifest argument; cwd plus nearest containing manifest; uniquely matching compiler output directory. Canonicalize both sides and never use string-prefix matching alone.
- Associate `rustc`, `clippy-driver`, `rustdoc`, build-script, and linker descendants by validated parent chain. For cache-daemon parentage, use `(target directory, build directory, target triple)` only when it selects one compatible live session across the entire system. Name those three exactly, because they are not independent: the target directory is the root Cargo writes under; the build directory is the profile-named directory immediately inside it, whose name is the custom profile's directory name rather than the profile label; and under `--target` the layout is `<target-dir>/<triple>/<profile>/`, so the triple already appears in the path between the other two. A unique match on that triple is actionable: `CompilerAttribution::UniqueOutputMatch` authorizes termination exactly as `Confirmed` does, because a target directory that exactly one live session writes to is sufficient evidence of ownership. Any candidate carrying insufficient identity — unreadable argv or cwd — degrades every uniqueness claim in that cycle to `Ambiguous`, because a candidate the system-wide test cannot see makes a "unique" match false-unique and attaches a compiler to the wrong session. When the triple selects more than one compatible live session — two checkouts sharing one target directory through `--target-dir` — do not resolve it by restricting the candidate set to the selected scope: a scope-filtered uniqueness test manufactures a unique match out of a genuinely ambiguous one, and that match authorizes a kill. Fall back instead to the other identifying evidence the compiler process already carries; its primary input path, canonicalized, resolving under exactly one known checkout root identifies the owning session regardless of a shared target directory. Any one of the association methods resolving to exactly one live session authorizes termination, and the methods are tried in order until one does; a unit stays `Ambiguous` only when none of them identifies. No session-level in-scope/out-of-scope membership state is introduced: the uniqueness test runs over every observed live session, and presentation filters by root where the rows are rendered. Render ambiguous units once in a scope-level, non-actionable attribution-unavailable section with candidate sessions.
- Derive compile units primarily from `--crate-name`, primary input, `--out-dir`, target triple, flags, and strong compiler identity. Resolve workspace packages from the shared index. Dependencies are absent from `no_deps`, so their package identity requires reading a manifest off disk — which `classify` must not do, and cannot do, because *which* manifest to read is only known after `classify` has parsed the primary input and located the source root. Resolve that by request and response across cycles rather than by reading inside the pure function: a source root absent from the input's manifest snapshot classifies this cycle as a not-yet-resolved lookup and is recorded in a request list on `BuildClassification`; the classifier reads and caches those manifests after `classify` returns; the next cycle resolves them exactly. The observable cost is one refresh in which a dependency row shows its crate name before its package identity — roughly 500 ms, once per dependency per session. Distinguish "not yet looked up" from "looked up and absent" in the cached state, or the first cycle is indistinguishable from a real miss and the lookup never retries. Cache package identity by canonical source root plus manifest stamp; reparse after change/removal; otherwise use `CompiledCrateIdentity::{WorkspacePackage(cargo_metadata::PackageId), DependencyPackage(ManifestPackageIdentity), CrateNameOnly(CompiledCrateName)}` or an equally semantic fallback type.
- Keep classification pure. Define a stateful `BuildClassifier` that owns dependency-manifest caches and the first-seen ledger, prepares immutable `BuildClassificationInput<'a>` containing the process snapshot, workspace-index view, dependency-manifest snapshot, first-seen snapshot, and the cycle instant, invokes the pure classifier to produce immutable `BuildClassification`, then updates those support structures outside the pure classify call. Phase 8 and Phase 9 both need the cycle instant and Phase 7 pins the input type across the worker boundary, so carry it from the start. This phase defines and tests the classifier without assigning it an App runtime owner; Phase 7 moves its sole runtime instance beside `ProcessObserver` on the dedicated executor worker.
- The dependency-manifest and first-seen fields of the input are shared references into the classifier's own tables, not copies. Both tables grow with the build — one entry per distinct dependency source root, one per observed process incarnation — so copying them into an owned input would reallocate thousands of map entries twenty times a minute for a function that only reads them. A shared reference is exactly as immutable to the compiler as an owned value; ownership buys no additional guarantee here, and the interior-mutability escape hatch is equally reachable through a copy. The cost is the lifetime parameter. It does not cross a thread: Phase 7 puts the classifier on the executor worker and `classify` runs on that same thread immediately after the input is built, so only the output travels back. The workspace-index field is unaffected — it is an `Arc` clone either way.
- Enforce purity through the signature, not through the implementer's discipline: `pub(super) fn classify(input: BuildClassificationInput<'_>) -> BuildClassification` is a **free function** in `classify.rs`, and the classifier plus its caches are private to `build_classifier.rs` so `classify` has no path to them. An inherent `impl BuildClassifier { fn classify(&self, …) }` satisfies the prose above and can still mutate a cache through interior mutability.
- The classifier resolves `target_directory_resolution()` once per cycle and puts the result in the input. That accessor performs filesystem syscalls on every call (`src/project/cargo/workspace_index.rs:214-220`), so reading it inside `classify` would make classification perform I/O and stop being a function of its input — which the classifier mutation test cannot detect.
- Specify eviction for both classifier-owned structures, matching the eviction Phase 2 already specifies for its candidate cache. The first-seen ledger is keyed by `ProcessIncarnation`, so a long build accumulates thousands of entries and every exec adds one without retiring its predecessor; the dependency-manifest cache grows the same way across workspace churn.
- Consume Phase 2's immutable unclassified Cargo/compiler/wrapper candidate-incarnation evidence through the process snapshot already in `BuildClassificationInput` — reach it with a `ProcessObservationSnapshot` accessor rather than repeating it as a second input field, since the input would then carry the same evidence twice. Do not repeat candidate parsing or introduce a second owner for that cache. Classification adds Cargo/build semantics while Phase 2 remains the sole exec-bound candidate-cache owner.
- Export that boundary as a named immutable `BuildCandidateIncarnations` snapshot (or equally semantic exhaustive type) from `ProcessObservationSnapshot`; do not expose the mutable `ProcessIncarnationCache`, use a bare optional candidate collection, or create a second candidate-cache owner.
- Use session key `BuildSessionId(ProcessIncarnation)` and activity key `CompileActivityId(ProcessIncarnation)`; target directory and profile are attributes. Record how the target directory was determined, not just its value: `SessionTargetDirectory::{Argument, Indexed, Unobservable}` — read from `--target-dir` on the command line, assumed to be `<checkout>/target` from the index, or undeterminable. Phase 2 observes argv, cwd, and ancestry but not the environment, so `CARGO_TARGET_DIR` and `build.target-dir` are invisible; without this state the `<checkout>/target` assumption is silently wrong for anyone who sets either, and a stale `target/` left from before they set it can match by accident. An `Unobservable` session never participates in output-directory matching in either direction — it is neither resolvable by scope method 4 nor eligible as a cache-daemon parentage candidate — so it can be neither matched wrongly nor the cause of a wrong match elsewhere. The visible cost is that a cache-daemon user with `CARGO_TARGET_DIR` set loses orphaned-compiler rows; they are omitted rather than attributed incorrectly. Resolve profile from explicit `--profile`/`--release`, then output directories, then metadata defaults, preserving custom/unknown labels. Order sessions and units by first-seen then process incarnation, and specify both halves — neither is currently implementable. The pure classifier cannot write the ledger, so everything discovered in the current cycle has no first-seen entry, which is the common case rather than the edge case: enabling the monitor mid-build, and a single `cargo build` spawning many `rustc` at once, both produce a whole cohort at once. Carry a monotonic cycle counter in the input and use it as the first-seen value for anything the ledger does not already hold. "Then process incarnation" names a struct, not an order — require a total order on `ProcessIncarnation`, which means `PlatformCreationToken` must be `Ord`.

**Files:**

- `src/build_monitor/mod.rs` — domain exports and classification entry point.
- `src/build_monitor/scope.rs` — `BuildScopeKey` and the scope-actionability boundary.
- `src/build_monitor/session.rs` — `BuildSessionId`, scope attribution, target-directory and profile state, and owned-root evidence.
- `src/build_monitor/activity.rs` — `CompileActivityId`, compiler attribution, compiled-crate identity, and the non-actionable presentation records.
- `src/build_monitor/classify.rs` — Cargo root normalization, scope resolution, compiler association, unit/profile/package derivation, and caches.
- `src/build_monitor/build_classifier.rs` — own dependency-manifest cache and first-seen state around the pure classifier boundary.
- `src/main.rs` — declare the build-monitor domain.
- `src/process_observation/snapshot.rs` — expose immutable executable, argv, cwd, ancestry, creation, and unclassified candidate-incarnation evidence required by pure classification without exposing mutable cache ownership.
- `src/project/cargo/metadata_store.rs` — expose manifest/source stamps without adding dependency metadata commands.
- `src/project/cargo/mod.rs` — supply index queries used by classification.
- `src/project/cargo/workspace_index.rs` — supply the exact package and target queries that remain, each returning a named result type rather than an `Option`: `CanonicalPackageOwnership<'a>` for package-by-canonical-member-root and `CanonicalTargetOwnership<'a>` for package-by-canonical-target-source, both `{Indexed(&'a …), Ambiguous, NotIndexed}` and both backed by `CanonicalWorkspaceCandidates`. `Option<&CargoPackageIdentity>` cannot express ambiguity, so a source path colliding across two indexed workspaces becomes either a dropped package (`None`) or a fabricated unique owner (`.first()`) — both violate this phase's own constraint that cross-workspace exact ambiguity stays non-actionable, in the data that later drives a scope-wide kill. Phase 5 already shipped `workspace_for_visible_project`, `workspace_for_workspace_root`, the `workspaces_by_root` map, and `VisibleProjectWorkspaceOwnership::{Indexed, Ambiguous, NotIndexed}`; consume those, do not re-derive the workspace and ambiguity paths.
- `src/project/cargo/workspace_index_api_tests.rs` — prove the classification-facing package/target queries retain all exact candidates.
- `src/tui/workspace_index.rs` — supply named readiness and immutable index views to classification callers. `WorkspaceIndexReadiness` keeps its three variants and its shipped `&'a CargoWorkspaceIndex` payload here; the reference-counted conversion belongs to Phase 7, because no Phase 6 caller holds the index across a thread boundary.
- `src/tui/compile_visibility/mod.rs` — the module that decides whether `MonitorScopeKey` becomes crate-visible, and one of the two candidate homes for the `From` conversion.
- `src/tui/compile_visibility/scope.rs` — `MonitorScopeKey` and its accessors; the acceptance gate already requires editing this file, and the conversion reads from it.
- `src/tui/state/inflight.rs` — expose immutable live/stopping owned-root classification evidence without exposing termination authority.
- `src/tui/state/mod.rs` — export the semantic owned-root evidence API.
- `Cargo.toml` — add only parsing/platform dependencies required by the classifier.

**Constraints from prior phases:** Consume Phase 1's exact `CargoWorkspaceIndex` identities and existing `cargo_metadata::PackageId`. Phase 2 supplies named strong/insufficient immutable observations, `ProcessIncarnation` as the exec-sensitive classification boundary, and the sole unclassified Cargo/compiler/wrapper candidate cache; a changed executable/argv fingerprint invalidates classification, scope, ancestry, selection, and actionability within the same process lifetime. Phase 4 supplies distinct current lifecycle ID, retained-output producer ID, and identity-bound opaque group authority; classification receives the current `OwnedRunId`, verified live/stopping root identity, and launch directory through semantic immutable evidence. It relies on that ID not repeating, but the allocation boundary that guarantees it is Phase 7's — Phase 6 adds no launch-side change and, having no runtime owner, cannot itself reach the reuse path. Phase 5 supplies canonical `MonitorScopeKey` plus named scope/index readiness, but `MonitorScopeKey` never leaves `crate::tui` — it reaches this phase only as `BuildScopeKey` through the `From` conversion defined in the Spec. Cross-workspace exact ambiguity remains non-actionable. Classification creates no signal authority, owns no mutable observer/cache state, and launches no Cargo command. `BuildClassifier` owns classification support state only; it never owns or reaches into `ProcessObserver`.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-port`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-port`, and `bash ~/.claude/scripts/delegate/verify.sh lint cargo-port` are green; pure tests cover each `CargoSubcommandRecognition` state, a `NonBuild` or `Unrecognized` root promoted to a build session by a live compiler descendant and staying promoted after that descendant exits while the Cargo process lives, a configured alias and a third-party plugin both landing in `Unrecognized` with no `.cargo/config.toml` read, built-in alias normalization, proxies/plugins/nested and divergent scopes, sibling roots, direct compiler/build-script/linker children, cache-daemon and cross-workspace ambiguity, debug/release/custom profiles, PID reuse and exec transitions, same-PID exec producing new session/activity IDs so that a retained ID stops matching the current snapshot (a retained ID is a lookup key, never authority — nothing makes an already-held classification value unusable, only re-resolution discharges it), named live/stopping/no-live-root owned evidence, the sole Phase 2 candidate-cache owner and immutable candidate-evidence snapshot, every named scope/index readiness state, dependency manifest caching/invalidation, exact workspace package IDs, semantic dependency/crate-name fallback, and no-deps fallback; classifier tests prove dependency/first-seen mutation stays outside the pure classify call and immutable input/output snapshots cross the boundary; classifying the same input twice produces identical output; a dependency source root absent from the manifest snapshot yields the crate-name fallback plus exactly one lookup request on the first cycle and the exact package identity with zero requests on the second, with "not yet looked up" distinguishable from "looked up and absent"; two sessions sharing one `--target-dir` produce a non-unique triple match and are still each associated with their own compiler by primary-input root, while a compiler whose primary input resolves under neither stays `Ambiguous`; two rows resolving to identical roots and revisions produce equal `BuildScopeKey` values; and session/unit ordering is stable across refreshes, including a cohort discovered in a single cycle with no prior first-seen entry.

Dead-code suppressions: this phase deletes only the ones it actually consumes, and states the disposition of the rest. Consumed by the `From` conversion: `canonical_checkout_roots`, `canonical_workspace_roots`, `accepted_cargo_metadata_revision`, and `project_list_revision` on `MonitorScopeKey`. Not consumed, and not consumable by this phase's own design — `monitor_selected_row_identity` (`src/tui/compile_visibility/scope.rs:88`) and `selected_row_kind` (`:93`), because the Spec above states that `BuildScopeKey` carries no row identity; and `required_features` (`src/project/cargo/workspace_index.rs:164`), `declared_source_path` (`:167`), and `declared_workspace_root_path` (`:199`), because the Spec mandates canonicalized comparison and the canonical accessors are already live. Delete those five, or narrow them to `#[cfg(test)]` where a test is their only caller; do not invent a production consumer and do not widen `BuildScopeKey` to manufacture one — that is the coupling the Phase 5 decision rejected. The Spec fixes the conversion's home as `src/tui/compile_visibility/mod.rs`, which is outside `scope.rs` and so cannot read the private fields — the four canonical accessors are therefore genuinely consumed and their suppressions come out. Writing the conversion inside `scope.rs` instead would read the fields directly and consume none of the six, which is why the home is fixed rather than left to the implementer. The two suppressions at `scope.rs:168` and `:175` — `MonitorScopeActionability` and `MonitorScopeResolution::actionability()` — are consumed by that entry point and come out with the other four. The gate is not green while a suppression this phase named survives without its stated disposition.

### Retrospective

**What worked:**

- Purity enforced by the signature rather than by prose: `classify` is a free
  function in `classify.rs` while the caches live in `build_classifier.rs`, so
  there was no interior-mutability escape hatch to review for. The blind review
  confirmed `classify` is a function of its immutable input.
- Opaque `BuildSessionId`/`CompileActivityId` with private inner fields stopped
  same-PID-exec collisions at compile time rather than at test time.
- The two-pass recognize-then-associate order with sticky promotion landed as
  specified and needed no rework.

**What deviated from the plan:**

- `src/main.rs` declares `#[cfg(test)] mod build_monitor;`. The whole module
  compiles into the test binary only, because this phase deliberately assigns
  classification no runtime owner and a plain `mod` would be entirely dead code
  in the shipping binary. The Files entry said only "declare the build-monitor
  domain" and did not anticipate the gate. **Phase 7 removes it** when the worker
  takes ownership.

**Surprises:**

- Build profile needed a sticky per-session ledger that the Spec did not name.
  `observed_build_directories` is rebuilt each cycle from live compilers only, so
  a build whose profile comes from `.cargo/config.toml` or an alias reported
  `Release` on cycles with a live compiler child and reverted to `Dev` on cycles
  without one. `BuildDirectoryLedger` (`build_classifier.rs:232`, LRU-capped)
  holds the last observation per session. The Spec specified stickiness for
  first-seen ordering and for promotion, but not for profile.
- The linker name table had to be consolidated into `LinkerRecognition` in
  `process_observation/snapshot.rs` and consumed cross-module from `classify.rs`.
  Two copies keyed differently — `Path::file_stem()` splits at the last dot —
  made `ld64.lld` match nothing and render as a rustc compile.
- An unreadable working directory must degrade the cycle, not skip the
  candidate: skipping removed a live build from the pane for that cycle, which
  contradicts the promotion guarantee. Scope resolution then simply yields
  `Unresolved` rather than a wrong scope.
- rustc passes `@<response-file>` instead of an argument list once the command
  line would exceed the OS limit — the default path for `link.exe` — so an
  argument-only linker test missed the entire Windows link path.

**Implications for remaining phases:**

- Phase 7 removes the `#[cfg(test)]` module gate. That also dissolves the one
  open non-gating finding: the shipping binary currently computes
  `build_candidate_incarnations` (`process_observation/snapshot.rs:1356`) about
  twenty times a minute with no reader, because every reader is inside the gated
  module.
- The classifier now owns **three** support structures, not two: the
  dependency-manifest cache, the first-seen ledger, and the build-directory
  ledger. Phase 7's Constraints paragraph named only the first two; the worker
  takes ownership of all three.
- Two test-quality gaps carry forward rather than blocking this phase: the
  cohort-ordering test (`classify_tests.rs:1228`) cannot fail if the tie-breaker
  at `classify.rs:640` were deleted, because the `BTreeMap` input already arrives
  in tie-break order; and `classifying_the_same_snapshot_twice_produces_the_same_result`
  (`classify_tests.rs:493`) uses `cargo build --release`, which returns at the
  `Argument` early-return and never exercises the build-directory ledger. The
  ledger applies a newly-learned build directory one cycle late — it converges
  and never produces a wrong attribution, but no test covers that path.

### Phase 6 Review

An architect pass over the remaining phases returned fifteen findings. Ten were
applied directly; five merged into three decisions deferred into the phases they
affect.

Applied without a gate:

- Phase 7's benchmark now charges the path canonicalization and target-directory
  resolution that `classify_cycle` performs before the pure call to
  classification rather than to the observer baseline, and builds its fixtures
  on the snapshot assembler Phase 6 shipped instead of a second one.
- Phase 7 now states that the classifier instance lives in the background
  worker, not inside the process-observation module — the classifier already
  reads types from that module, so hosting it there would make the dependency
  circular against Phase 2's neutrality constraint.
- Phase 7 records that the completed-refresh payload is boxed and must stay
  boxed once the classification result joins it.
- Phase 7 gains the collapse of the build session's two independently-unresolved
  scope fields into one exhaustive value, so "actionable implies a resolved
  root" becomes a type fact before any termination phase reads it.
- Phase 8's refresh-schedule type is renamed: Phase 3 already shipped a
  different type under the name the plan chose, exported and threaded through
  three call sites.
- Phase 8 drops a file entry that Phase 3 already satisfied.
- Phase 8's session states now require one exhaustive type of this phase's own —
  Phase 6 shipped only a per-activity compiler kind — and the Cargo-lock wait
  state is dropped rather than inferred if its only evidence source stays
  off-limits to polling.
- Phase 8's acceptance gate takes ownership of the four snapshot-and-deadline
  suppressions that no phase claimed, since it is their first reader.
- Phase 8's and Phase 9's dead-code clauses are corrected: the items they name
  carry test-only gates, not suppressions, so both gates now say what actually
  has to be removed.
- Three drifted source line references were corrected across Phases 7 and 8.

Deferred as decisions in the phase they affect:

- Phase 7 — how much of Phase 6's test-only gate this phase removes, and which
  phase proves no suppression is left. The gate is 68 attributes across ten
  files, not one module declaration, and removing it without a production caller
  converts test-gating into dead-code suppression.
- Phase 7 — what the build session exposes to the presentation and termination
  phases. It carries no operative command, no root process id, and a cycle
  counter rather than an instant, while Phases 9 and 12 specify all three, and
  Phase 11 needs a root identity the opaque session id does not expose.
- Phase 8 — which phase narrows the host-wide classification to the selected
  scope's roots. No remaining phase owns it, so the set the user sees and the
  set a scope-wide termination acts on would be computed separately.

### Phase 7 — Worker-side classification integration · status: done

#### Work Order

**Goal:** The dedicated-worker `ProcessRefreshExecutor` incorporates classification and shared App reconciliation before lifecycle polling is added, preserving one observer owner and independent consumer outcomes.

**Spec:**

- Extend the Phase 3 repeatable 1,000- and 5,000-process benchmarks to cover observer refresh plus Phase 6 classification-input preparation/classification, including representative Cargo, compiler, wrapper, and unrelated processes; report classification's incremental cost separately from the observer-only baseline. `classify_cycle` (`src/build_monitor/build_classifier.rs:456`) canonicalizes process paths and resolves indexed target directories **before** the pure `classify` call, so that filesystem work is classification cost and must be charged to the classification measurement, not left inside the observer baseline. Build the fixtures on Phase 6's `src/process_observation/snapshot_builder.rs`, which is exactly the snapshot assembler these benchmarks need; widen its `#[cfg(test)]` gate if the benchmarks are to run outside `cargo test`. Timing remains recorded evidence rather than a flaky CI threshold.
- Keep the architecture Phase 3 selected: one App-owned `ProcessRefreshExecutor` uses its dedicated worker, and the worker owns the sole `ProcessObserver` plus the sole runtime `BuildClassifier`. The `BuildClassifier` instance lives in the worker in `src/tui/background.rs`, **not** inside `src/process_observation`: `src/build_monitor/classify.rs` already imports `LinkerRecognition`, `ProcessObservationSnapshot`, `AncestryLookup`, and `ProcessIncarnation` from `process_observation`, so hosting the classifier there would complete a bidirectional dependency against Phase 2's observation-stays-neutral constraint. Do not add a synchronous production branch, another observer, another classifier-support owner, or another timing channel.
- Requests carry refresh correlation, the semantic refresh plan, immutable scope/generation/owned-run evidence, and `CompileClassificationDemand::{NotRequested, Requested { generation, scope, cancellation }}` (or an equally semantic exhaustive request state). The workspace-index evidence is an `Arc<CargoWorkspaceIndex>` clone taken at request time — **this phase** converts `App::cargo_workspace_index` (`src/tui/app/mod.rs:260`) from a by-value `CargoWorkspaceIndex` to an `Arc<CargoWorkspaceIndex>` and makes `rebuild_if_changed` swap in a fresh index rather than mutate in place, precisely so a request can hold the accepted index across the thread boundary without borrowing `App`. One allocation per accepted-metadata change; the index type still derives only `Debug, Default` and gains no `Clone`. `WorkspaceIndexReadiness` keeps its three variants and its `Clone, Copy` derive, and its two payload variants change from `&'a CargoWorkspaceIndex` to `&'a Arc<CargoWorkspaceIndex>` — a view onto the handle, not the handle. Do not make the variants own an `Arc`: that drops `Copy`, and `resolve_nested_workspace_scope` (`src/tui/compile_visibility/scope.rs:377-410`) destructures the readiness value at `:384-389` and passes the same value again at `:406`, which compiles only because it is `Copy`. With `&'a Arc<…>` the app field is already the handle, so `&self.cargo_workspace_index` produces the reference directly and every existing call site is unchanged — method calls and argument positions both deref through it. The single explicit `Arc::clone` happens here, where the request is built. A rebuild landing mid-flight leaves the in-flight request on the older index; that is safe because the request's revision stamps and generation already say which index it was asked under. `scope` is Phase 6's `BuildScopeKey`, never `MonitorScopeKey` — the latter is `pub(super)`-reachable only inside `crate::tui`. Its generation-bound cancellation capability cannot cancel another scope or generation. Mutable App state and observer internals never cross into the worker.
- Extend `CompletedProcessRefreshExecution` with `CompileClassificationExecution::{NotRequested, Completed(BuildClassification), Failed(BuildClassificationExecutionFailure), Cancelled(CompileMonitorGeneration)}` or an equally semantic product. A successful process observation remains available to Running Targets when compile classification fails or is cancelled; only `ProcessRefreshExecutionOutcome::Failed(NoCompletedRefresh)` represents a cycle with no completed observation. Phase 6 boxed `ProcessRefreshExecutionOutcome::Completed`'s payload; `BuildClassification` carries eight `Vec`s and lands in that same value, so **the box stays** — do not unbox it while adding the compile outcome.
- Check compile cancellation after process observation and immediately before classification-input preparation/parsing. Toggle or scope invalidation cancels the matching monitor generation so an in-flight combined request skips compile parsing/classification while retaining and returning any due Running result.
- Move shared executor deadline access, request dispatch, receiver access, result correlation, and consumer-outcome reconciliation from `running_targets/app_tick.rs` into a neutral App adapter in `src/tui/process_refresh.rs`. Leave Running-specific cadence, index readiness, attribution, metrics, and view-state application in `running_targets/app_tick.rs`.
- Replace Phase 4's saturating owned-run allocation with an explicit `OwnedRunIdAllocation::{Allocated(OwnedRunId), Exhausted}` boundary, and update the launch sites that create runs to handle rejection. Exhaustion rejects a new run instead of repeating an ID, so late-message, join, cursor, and later termination correlation remain unique — Phase 6's classification already relies on that uniqueness. Make the counter `NonZeroU64` so `OwnedRunId(0)` is unrepresentable and the type gains a niche. Exhaustion itself is unreachable — at one run per millisecond a `u64` lasts ~584 million years — so also close the reuse path that *is* reachable: `OwnedRun::new()` (`src/tui/state/inflight.rs:206`) is `pub(crate)` and restarts `next_id` at 1, so a second `Inflight` construction reissues IDs while messages tagged run 1 may still be in the channel, which is the exact collision `OwnedRunId` exists to prevent. Gate that `Inflight::new`/`OwnedRun::new` has exactly one production call site and the counter is never reset for the process lifetime.
- Collapse `BuildSession`'s two independently-`Unresolved` scope fields into one exhaustive `SessionScope::{Resolved { method, root }, Unresolved}` (or an equally semantic product). Phase 6 shipped `ScopeAttribution` and `SessionScopeRoot` as separate fields that each carry their own `Unresolved`, so a session can currently represent a resolved attribution method with no resolved root. Phase 13 derives destructive authority from resolved roots, so making "actionable implies a root" a type fact belongs before any termination phase reads it.
- Give `BuildSession` the data the presentation and termination phases act on, populated by classification from the root incarnation it already resolves: the operative Cargo command with its selectors, the root PID, and an observation instant for the root's first sighting so elapsed time and start age are computable. The existing `first_seen` `BuildClassificationCycle` counter stays for cycle-relative classifier logic; it is not a clock and must not be pressed into one. Add one accessor to the root `ProcessIdentity` for Phase 11's pre-signal revalidation and Phase 8's re-resolution of a retained ID against a fresh snapshot — hang it on `BuildSession`, **not** on `BuildSessionId`. An accessor on the bare ID would let any holder of an opaque identifier materialize a signalable PID with no session and no scope behind it; requiring the session means every route to root identity passes through classification output that already carries scope attribution, which is what makes the authorization scoping provable rather than asserted.
- Preserve Phase 3's `CompletedProcessRefreshExecution` duration, named no-completion failure timing, and one-counted raw Running metrics refresh boundary. This phase adds no compile-monitor deadline or lifecycle; Phase 8 schedules compile demand through this adapter without reopening architecture.

**Files:**

- `src/process_observation/mod.rs` — expose combined observation work through the executor boundary.
- `src/process_observation/executor.rs` — extend the existing dedicated worker request/result and cooperative cancellation boundary.
- `src/process_observation/snapshot.rs` — build immutable refresh inputs/results for measurement or worker transfer.
- `src/build_monitor/classify.rs` — accept the immutable classification input measured by the executor.
- `src/build_monitor/build_classifier.rs` — move the sole runtime classification-support state beside the worker-owned observer: the dependency-manifest cache, the first-seen ledger, and the `BuildDirectoryLedger`.
- `src/main.rs` — remove `#[cfg(test)]` from `mod build_monitor;` and its explanatory comment now that the worker is the module's runtime owner.
- The rest of Phase 6's test-only gate lives outside `src/build_monitor`: 68 `#[cfg(test)]` attributes across ten files, of which these carry the bulk — `src/tui/compile_visibility/mod.rs` (6, including the `From<&MonitorScopeKey>` impl and `build_scope_actionability`), `src/tui/compile_visibility/scope.rs` (11, including all four canonical-root accessors and `actionability()`), `src/tui/state/inflight.rs` (12, including `owned_root_evidence()`), `src/project/cargo/workspace_index.rs` (16), plus `src/project/mod.rs`, `src/project/cargo/mod.rs`, and `src/tui/project_list/mod.rs`. Enumerate the full set with `git log -p` over the Phase 6 checkpoint before starting; only attributes that phase added are in scope. **Remove a gate only where this phase's worker gives the item a real production caller**, and leave the rest gated — an ungated item with no caller is a `dead_code` warning whose only remedy is a suppression, which is precisely what Phase 10 forbids. Phase 8 removes the remainder when it schedules the first `Requested` demand.
- `src/build_monitor/benchmarks.rs` — repeatable 1,000/5,000-process fixtures and timing report harness, built on `src/process_observation/snapshot_builder.rs`.
- `src/process_observation/snapshot_builder.rs` — widen its `#[cfg(test)]` gate only if the benchmarks must run outside `cargo test`; otherwise reuse it as-is.
- `src/build_monitor/session.rs` — replace the two independently-`Unresolved` scope fields on `BuildSession` with one exhaustive `SessionScope`; add the operative Cargo command, root PID, and root observation instant that Phases 9 and 12 render, plus the root `ProcessIdentity` accessor, which hangs on `BuildSession` and never on the bare `BuildSessionId`.
- `src/build_monitor/execution.rs` — carry semantic compile-classification outcomes, generation cancellation, and immutable results.
- `src/build_monitor/mod.rs` — export `ProcessRefreshExecutor`-facing classification APIs.
- `src/tui/process_refresh.rs` — neutral App adapter for shared deadline, dispatch, receiver, correlation, and consumer reconciliation.
- `src/tui/mod.rs` — declare the neutral process-refresh adapter.
- `src/tui/running_targets/app_tick.rs` — retain only Running-specific demand and snapshot application after shared orchestration moves. The `Arc` conversion also breaks roughly a dozen `rebuild_count()` assertions here plus a direct field assignment at `:1387`.
- `src/tui/terminal/frame_metrics.rs` — use the existing 30 ms slow-frame boundary and record refresh cost separately.
- `src/tui/terminal/event_loop.rs` — host only the selected executor integration, without adding a compile deadline.
- `src/tui/background.rs` — extend the existing dedicated refresh worker with classification coordination.
- `src/tui/messages.rs` — extend immutable correlated worker requests/results with separate consumer outcomes.
- `src/tui/app/mod.rs` — keep App ownership at the executor boundary while observer ownership stays inside it.
- `src/tui/app/construct.rs` — construct the dedicated executor and worker-owned `BuildClassifier` without exposing either mutable owner.
- `src/tui/app/async_tasks/poll.rs` — route correlated worker results through the neutral adapter.
- `src/tui/state/inflight.rs` — supply immutable live/stopping owned-root evidence while retaining termination authority and captured output in `OwnedRun`.
- `src/tui/workspace_index.rs` — change the readiness payload variants to `&'a Arc<CargoWorkspaceIndex>` while keeping the enum `Copy`.
- `src/tui/input/dispatch.rs` and `src/tui/panes/actions.rs` — the launch sites that create owned runs; `Exhausted` forces each to handle rejection instead of receiving an ID unconditionally.

**Constraints from prior phases:** Phase 2 supplies named observation evidence, exec-sensitive incarnations, immutable snapshots, and one mutable observer/cache owner. Phase 3's measurements selected the dedicated worker and shipped sole `ProcessRefreshExecutor` ownership, coalesced refresh plans, `CompletedProcessRefreshExecution`, named `NoCompletedRefresh` failure timing, and exactly one raw Running metrics refresh per due cycle. Phase 4 supplies distinct current lifecycle, live/stopping verified-root, and retained-output producer identities; worker requests may carry immutable current/live evidence but never the opaque termination capability or captured output. Phase 5 supplies named index/scope readiness. Phase 6 supplies the pure classifier, immutable `BuildClassificationInput`, `BuildCandidateIncarnations`, and `BuildClassifier`; the worker owns **all three** of the classifier's support structures beside `ProcessObserver` — the dependency-manifest cache, the first-seen ledger, and the `BuildDirectoryLedger` (`src/build_monitor/build_classifier.rs:232`) that keeps a session's build profile stable when no live compiler child reveals the build directory this cycle — while requests contain only immutable consumer evidence. Phase 6 shipped `src/build_monitor` behind `#[cfg(test)] mod build_monitor;` in `src/main.rs`, plus 67 further `#[cfg(test)]` attributes on its supporting items across nine other files, because classification had no runtime owner; **this phase removes the gate from the items the worker actually calls** and leaves the rest gated for Phase 8, which creates the first production `Requested` demand. Removing it also resolves the standing cost that `build_candidate_incarnations` (`src/process_observation/snapshot.rs:1356`) is computed on every non-test refresh with no reader — the readers are all inside the gated module. Preserve Running Targets cadence/readiness and add no compile polling yet.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-port`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-port`, and `bash ~/.claude/scripts/delegate/verify.sh lint cargo-port` are green; combined 1,000/5,000-process benchmarks and incremental classification timing are recorded; deterministic tests prove the dedicated worker solely owns `ProcessObserver` and `BuildClassifier`, requests carry no mutable App/observer state, result correlation/order is exact, successful Running observation survives compile-classification failure, cancellation after observation skips compile parsing/classification while preserving a due Running result, one combined due cycle performs one counted raw Running metrics refresh, neutral reconciliation does not depend on the Running Targets adapter, `OwnedRunId` exhaustion is reachable from a test through a `#[cfg(test)]` allocator seed and rejects the run without reusing an ID, and no compile deadline exists yet; `src/main.rs` declares `mod build_monitor;` with no `#[cfg(test)]` gate, and every other gate this phase removes has a named production caller in the worker while no `dead_code` suppression is added anywhere to compensate for one it removed; a classified `BuildSession` carries the operative command, root PID, and an observation instant sufficient to compute elapsed time, and a test proves root `ProcessIdentity` is reachable only from a `BuildSession` produced by classification — a bare `BuildSessionId` exposes no route to it.

### Phase 7 Retrospective

**What worked:**

- The `Arc<CargoWorkspaceIndex>` conversion landed exactly as specified: `WorkspaceIndexReadiness` kept `Copy` by holding `&'a Arc<…>`, so no scope-resolution call site changed.
- Moving shared deadline/dispatch/correlation/reconciliation into `src/tui/process_refresh.rs` left `running_targets/app_tick.rs` holding only Running-specific cadence and view state, with no second observer, classifier, or timing channel.

**What deviated from the plan:**

- Owned-run rejection shipped as two types, not one: `OwnedRunIdAllocation::{Allocated, Exhausted}` stays internal to the counter, and `OwnedRunLaunchAdmission::{Queued, AlreadyActive, IdentitiesExhausted}` is what launch sites match. `AlreadyActive` was already a silent-failure path at the same call site, so folding it into the same exhaustive value was the only way to make the key press always produce a user-visible result.
- The empty-root state was made unrepresentable by `CoveredScopeRoots` + `ScopeRootCoverage::{Covered, NoRootsCovered}` in `src/build_monitor/scope.rs`, which removed the only production producer of `BuildClassificationExecutionFailure`. Both it and `CompileClassificationExecution::Failed` are therefore `#[cfg(test)]`-gated rather than suppressed, awaiting Phase 8's aging logic.
- `BuildSession` gained `first_observed_at` from a `FirstSighting { first_seen, first_observed_at }` ledger entry rather than being stamped from the current cycle instant; the cycle counter stayed a counter as the plan required, but the plan did not anticipate that the first-seen ledger had to carry the instant too.

**Surprises:**

- Classification at 5,000 processes costs ~74–89 ms per cycle, not the ~57–62 ms measured earlier. The earlier fixture attached compilers and wrappers to a checkout their parent did not own, so output-directory matching and member-root containment were timed on their miss paths. The conclusion is unchanged and better supported: classification cannot run inline against the 15 ms frame budget, and at the once-per-second refresh it is roughly 8% of one core.
- Gate removal was far more partial than the Work Order implied. The module gate on `mod build_monitor;` is gone and the worker-called items are ungated, but every `BuildSession` presentation accessor — operative command, root PID, first-observed instant, session scope, build profile — is still `#[cfg(test)]`, because their first production reader is Phase 9's column renderer, not Phase 8.
- The live smoke test runs under a pseudo-terminal with no window size, so it exercises startup, the event loop, the compile monitor, the toggle keypress, and clean exit — but not visual layout.

**Implications for remaining phases:**

- Phase 8 must delete the `#[cfg(test)]` on `BuildClassificationExecutionFailure` (`src/build_monitor/execution.rs:112`) and on `CompileClassificationExecution::Failed` (`:130`) when it supplies the first reachable failure cause; without that its own failure-aging Spec has nothing to age from.
- Phase 8's acceptance gate claim that afterwards "no compile-visibility item outside a test module is gated" is now wrong: the `BuildSession` presentation accessors belong to Phase 9. The clause needs splitting between the two phases.
- Phase 9 renders operative command, root PID, and elapsed time from accessors that exist and are gated; it owns their un-gating.
- Three non-gating nits carry forward: `allocate_id` retires one identity early (`src/tui/state/inflight.rs:261`), `classify_demand` reads cancellation a second time redundantly (`src/build_monitor/build_classifier.rs:466`), and the pre-existing `#[allow(dead_code)]` on `accepts_generation` (`src/tui/compile_visibility/mod.rs:124`) is Phase 8's to remove.

### Phase 7 Review

An architect pass over the remaining phases returned twenty findings, all
verified against shipped source. Every one was a Work Order correction; none
raised a new decision. Applied:

- Phase 8's completed compile outcome must become a struct payload carrying the
  monitor generation and the `BuildScopeKey`. It carries neither today, and its
  sibling `Cancelled` already states its generation, so the asymmetry was a type
  defect rather than a missing comment. `src/build_monitor/execution.rs` added
  to Phase 8's **Files**.
- Phase 8 gains the executor work that makes its own cadence reachable:
  un-gating `CompileMonitorRefreshSchedule::At` and adding a rearm path.
  `src/process_observation/executor.rs` added to its **Files**.
- Phase 8 now states which of the two refresh schedules is authoritative —
  `ActiveMonitorState` owns the intent, the executor's copy is derived.
- Four Phase 8 Spec bullets shrank because Phase 7 already shipped them:
  cancellation (only the `accepts_generation` half remains), per-cycle
  target-directory resolution (only the typed revision remains), per-interval
  coalescing, and no-compile-demand-while-`Off`. The last two became acceptance
  assertions instead of build work.
- Phase 8's suppression and gate inventory was wrong in most of its items; it
  now names the two `#[allow(dead_code)]` sites that actually remain in `src/`
  and the three gates whose first caller this phase supplies.
- The "nothing is gated afterwards" claim split three ways: Phase 8 keeps its
  own items, Phase 9 un-gates the `BuildSession` presentation accessors, Phase
  11 un-gates the two root-identity accessors. Matching sentences added to both
  later gates.
- Phase 8's retention rule is one `==` on `MonitorScopeKey::covered_scope_roots()`,
  and its primary case is ordinary cursor movement between two rows in the same
  workspace — not the background scan the plan cited.
- Phase 8's `BuildMonitor` bullet was reworded so it cannot be read as moving
  the classifier off the worker, and it now names the exact function that stops
  discarding the classification result.
- Phase 9's stale accessor line reference corrected, and its output-state
  replacement now states what it makes unrepresentable and that the title stays
  a semantic type rather than an `Option<String>`.
- Phase 13 now joins the monitor snapshot through the single scope entry point
  Phase 7 established, instead of constructing a `BuildScopeKey` directly and
  bypassing the actionability check.
- The two existing pending decisions were sharpened, not resolved: Phase 8's
  scope-filtering decision gained the two facts that change its cost, and Phase
  10's input-path decision now says it must be settled **before** Phase 8,
  because one of its options changes the equality Phase 8's display rule is
  built on. That prerequisite is also recorded in Phase 8's Constraints.
- No remaining phase quotes the superseded classification timing figure; no edit
  was needed there.

### Phase 8 — Conditional monitor polling and lifecycle · status: done

#### Work Order

**Goal:** Enabling compile visibility produces fresh scoped monitor snapshots on a bounded cadence, while disabling it removes all compile-specific polling and idle work.

**Spec:**

- Add `BuildMonitor`, which owns classification **results** and their lifecycle; the classifier itself stays worker-owned as `BuildClassifyingRefreshCycle` (`src/tui/background.rs:302-306`) and this phase must not move it or construct a second one. `BuildMonitor` retains only live session/unit identities, explicit owned association, termination tombstones added in later phases, and the latest presentation snapshot; it does not accumulate external history. `record_compile_classification_execution` (`src/tui/process_refresh.rs:190-215`) currently logs session and activity counts and then discards the `BuildClassification`; that function is what this phase replaces with storage into `BuildMonitor`.
- **`BuildMonitor` is the single site that narrows the host-wide classification to the selected scope, and the worker does not filter.** `BuildClassification::build_sessions()` is host-wide by Phase 6's explicit design — that is what keeps an out-of-scope owned run classifiable — and it stays that way. Before storing the presentation snapshot, `BuildMonitor` drops every session whose resolved root falls outside the current `BuildScopeKey`, so the set the Output pane renders and the set Phases 12–13 terminate are one value that cannot diverge. The filter is checkout-root containment: a session carries only a `CanonicalCheckoutRoot`, never a workspace root, so this is not the two-sided root comparison used for scope-key equality. Use `SessionScope::Resolved { root }` with `shares_resolved_root` (`src/build_monitor/session.rs:66-90`). Do **not** also filter in `classify_demand` (`src/build_monitor/build_classifier.rs:489-493`), which receives `build_scope_key` and today only logs its root counts: honoring the scope key in two places is the exact divergence this rule exists to prevent, and filtering at the worker would discard the out-of-scope owned run that host-wide classification exists to preserve. The explicit owned association survives the filter regardless of scope.
- Define `COMPILE_MONITOR_REFRESH_INTERVAL` as 500 ms in `src/tui/compile_visibility/constants.rs`. Model scheduling as `MonitorRefreshSchedule::{DueAt(Instant), InFlight { generation: CompileMonitorGeneration, rearm_at: Instant }}` or equally semantic due/in-flight states, and store it **inside `ActiveMonitorState`**. The name must not be `CompileMonitorRefreshSchedule`: Phase 3 already shipped that type at `src/process_observation/executor.rs:32` as `{At(Instant), NotScheduled}` for the shared executor deadline, exported and threaded through `app_tick.rs`, `background.rs`, and `construct.rs`. This is a second, monitor-owned concept, not a replacement for that one — Phase 5 shipped `CompileVisibilityState::{Off, On(ActiveMonitorState)}`, which drops the whole aggregate on toggle-off, so "disabled owns no schedule" is already a type-level guarantee. Do not add a parallel `Disabled` variant (it reintroduces two sources of truth that can disagree with `Off`), and do not use one `NotScheduled` state for consumed and pending work. Rearm from the interval boundary, coalescing missed instants into one next demand rather than building a queue. `ActiveMonitorState` owns the scheduling **intent** and pushes it into the executor; the executor's `CompileMonitorRefreshSchedule` is a derived dispatch deadline and is never read back as truth, so the two can never disagree about whether a refresh is owed.
- Give the executor a way to be armed at all. `CompileMonitorRefreshSchedule::At(Instant)` is still `#[cfg(test)]` (`src/process_observation/executor.rs:73-74`), `ProcessRefreshExecutor` sets the schedule only in `new` (`:305`), and `advance_dispatched_deadlines` clears it to `NotScheduled` after every compile-bearing cycle (`:478`) with no rearm path. Un-gate the variant and add a rearm method on the executor; without both, the 500 ms cadence is unreachable.
- `poll_background` (`src/tui/app/async_tasks/poll.rs:58-64`) calls `refresh_compile_monitor_scope_if_on` directly, unlike the selection and metadata triggers, which reach it after `ensure_visible_rows_cached()` via `sync_selected_project`. Of the four `mark_visible_ownership_changed` sites, `dispatch.rs` and `tree_mutation.rs` rebuild rows first and `metadata_handlers.rs` routes through `sync_selected_project`, but `disk_handlers.rs:78` advances the revision without recomputing — so that batch resolves scope from a `cached_visible_rows` that no longer matches the visibility filter. It self-corrects on the next frame today, but Phases 11–13 make scope authoritative for destructive termination: require `ensure_visible_rows_cached()` before the `poll_background` refresh, and test it.
- While enabled, request command-line/process fields through the combined Phase 3 refresh plan and execute through Phase 7's dedicated-worker `ProcessRefreshExecutor`. Phase 2's internal repeated field samples and identity brackets remain intact. Per-interval coalescing itself already ships as `due_demand` (`src/process_observation/executor.rs:439-462`), and the neutral adapter already contributes no compile deadline while monitor state is `Off` — `compile_classification_demand` returns `NotRequested` unless the state is `On` *and* the scope is `Actionable` (`src/tui/process_refresh.rs:227-237`), tested at `:440-467`. Both are Acceptance-gate assertions for this phase, not work it has to build. What this phase adds on a due instant shared with Running Targets is the union of fields into one coalesced cycle while preserving the Running one-second identity-bound CPU/history sample and one-counted raw metrics refresh.
- Add the typed live target-directory revision. The per-cycle recheck already ships: `classify_cycle` calls `IndexedWorkspaceTargetDirectories::resolve` every cycle (`src/build_monitor/build_classifier.rs:520-522`), which is why that filesystem cost is charged to classification rather than to the observer. Do not rebuild it. What remains is the typed revision that invalidates affected classification and actionability when a previously missing target directory appears, or a symlink is created or retargeted, even when metadata, project-list content, and selected scope are unchanged.
- A completed compile-classification result must carry monitor generation and the exact `BuildScopeKey` it was computed under. It carries neither today: `CompileClassificationExecution::Completed(Box<BuildClassification>)` (`src/build_monitor/execution.rs:128`) holds only the classification, `BuildClassification` (`src/build_monitor/classify.rs:479-490`) holds only `classification_cycle` and `cycle_instant`, and `classify_demand` (`src/build_monitor/build_classifier.rs:474-503`) destructures the generation and the scope key and then drops them. Replace the variant's payload with a struct payload `{ compile_monitor_generation, build_scope_key, classification }` — the sibling `Cancelled(CompileMonitorGeneration)` already states its generation, and a reader must not have to consult the originating request to learn the completed one's. A doc comment on the variant is not a substitute. Ignore mismatches. On scope change, **retain the last good monitor snapshot when the new `MonitorScopeKey`'s covered roots equal the old key's; show `Pending` only when the roots actually differ.** That comparison is a single `==`: Phase 7 made `MonitorScopeKey::covered_scope_roots()` an ungated accessor (`src/tui/compile_visibility/scope.rs:83`) returning `&CoveredScopeRoots`, which derives `PartialEq` over both sorted, deduplicated root sets. Use it; do not hand-roll two slice comparisons with their own sort assumptions. Retention needs its own state, because it is simultaneously "no snapshot for this generation yet" and "prior data still on screen", which neither `Pending` (carries nothing) nor `Stale` (data that matched the current generation and then aged) can say. Add `PendingWithRetained(RetainedMonitorData)` and do not widen `Pending` with an optional payload — during a scan the generation advances every refresh, so retention is the normal display state, not an edge one, and an optional payload would make every match site re-decide the display rule. Compare the roots directly — never infer retention from the generation or the revision stamps, which advance on any `mark_visible_ownership_changed` bump. The primary case is ordinary cursor movement, not a background event: `ActiveMonitorState::requires_replacement` (`src/tui/compile_visibility/mod.rs:44-47`) returns true when `monitor_selected_row` alone differs, and `MonitorScopeKey` embeds the selected-row identity — so arrowing between two rows in the same workspace advances the generation with both root sets unchanged. Without retention, every arrow key blanks the display, and so does a background discovery scan or a disk-usage visibility flip (`disk_handlers.rs:78`), on the 500 ms cadence, exactly while the user is waiting to see something. Carry the observation instant in every data-bearing variant so age can be re-derived at render time and at the moment a kill is authorized, rather than being implied by which variant holds the data. Retained data **is** actionable: `alt-k` keeps working while the pane shows it, because the termination path already re-resolves a retained ID against the current process snapshot before signalling — a session that ended meanwhile is simply not found. Refusing on retention would add no protection that re-resolution is not already giving, and would disable the action for the whole duration of a background scan, which is exactly when a runaway build is most likely to be the thing the user wants to stop. `CompileClassificationExecution::Failed` and whole-cycle `ProcessRefreshExecutionOutcome::Failed(NoCompletedRefresh)` age the last good monitor snapshot to visibly `Stale` and non-actionable for one interval, then `Unavailable`; completed empty classification and per-process insufficient evidence do not. A compile-classification failure inside `CompletedProcessRefreshExecution` must not discard or suppress its successful Running snapshot/metrics outcome.
- Cancellation of in-flight compile work already ships and is not this phase's build: `App::cancel_compile_classification` (`src/tui/process_refresh.rs:107`), the superseded-generation cancel in `refresh_compile_monitor_scope_if_on` (`src/tui/app/mod.rs:1302-1314`), both worker checks in `classify_demand` (`src/build_monitor/build_classifier.rs:485`, `:495`), and tests at `src/tui/process_refresh.rs:352-422` cover toggle-off and scope/generation replacement, skipping classification while still returning any coalesced due Running outcome. What this phase adds is the other half: **no later compile result is accepted and no new compile demand is scheduled** after a cancellation — the `CompileVisibilityState::accepts_generation` wiring, which is why this phase also removes that item's `dead_code` suppression.
- A root Cargo process anchors a session through gaps with no live compiler. Report evidence-backed compiling, build-script, linking, owned Cargo-lock wait, and running-target states through one exhaustive session-state type defined by this phase; Phase 6 shipped only per-activity `CompilerKind`, which names none of them. Owned Cargo-lock wait has exactly one evidence source — the owned run's captured Cargo output — and polling may not copy that output, so the owned lifecycle must surface a semantic "blocked on the Cargo lock" state that polling reads without reaching into the captured lines. If that evidence cannot be produced under those constraints, drop the lock-wait state rather than inferring it; report external no-child gaps only as active.
- Associate an owned run with exactly one observed session by matching its verified root `ProcessIdentity` to the current exec-sensitive `BuildSessionId(ProcessIncarnation)`, then retain only `OwnedRunId`; never copy owned output into snapshots. External completed sessions disappear.
- Keep three owned identities distinct: the current lifecycle `OwnedRunId`, immutable live/stopping verified-root evidence used for the session join, and `OwnedRunOutputIdentity` for captured output that may still belong to run N while run N+1 is queued or starting. Polling joins only the live/stopping root; presentation later reads captured output by its producer identity.
- Prove no persistent idle CPU work while off and no compile-specific request/result acceptance after a toggle or scope generation change.

**Files:**

- `src/build_monitor/mod.rs` — `BuildMonitor` lifecycle and snapshot API.
- `src/build_monitor/poll.rs` — conditional refresh requests, classification, failure aging, and generation correlation.
- `src/build_monitor/snapshot.rs` — define `MonitorSnapshot::{Pending, PendingWithRetained, Fresh, Stale, Unavailable}` here rather than in Phase 6: three of its four variants are only reachable from this phase's aging logic, so defining it earlier ships it as another dead-code suppression. Fresh/stale/unavailable presentation states and stable first-seen ordering.
- `src/tui/compile_visibility/mod.rs` — connect enabled scope/generation to monitor polling.
- `src/tui/compile_visibility/constants.rs` — own the 500 ms compile refresh interval.
- `src/build_monitor/execution.rs` — widen the completed compile outcome to carry the monitor generation and the `BuildScopeKey` it was classified under, and delete the `#[cfg(test)]` gates on `BuildClassificationExecutionFailure` (`:112`) and `CompileClassificationExecution::Failed` (`:130`) once this phase's failure aging supplies their first reachable cause.
- `src/process_observation/executor.rs` — un-gate `CompileMonitorRefreshSchedule::At` (`:73-74`) and add the rearm path the 500 ms cadence needs; `advance_dispatched_deadlines` (`:478`) currently clears to `NotScheduled` and nothing re-arms.
- `src/tui/process_refresh.rs` — combine compile/Running demand, deadlines, cancellation, and independent result reconciliation. `record_compile_classification_execution` (`:190-215`) is the specific function that stops logging-and-discarding and starts storing into `BuildMonitor`.
- `src/tui/app/mod.rs` — own `BuildMonitor`.
- `src/tui/app/construct.rs` — initialize it without enabling it.
- `src/tui/app/async_tasks/poll.rs` — reconcile generation-tagged results returned by the dedicated worker.
- `src/tui/terminal/event_loop.rs` — include the optional monitor deadline.
- `src/tui/terminal/frame_metrics.rs` — record and assert bounded refresh work.
- `src/tui/startup_services.rs` — ensure disabled/test startup creates no monitor work.
- `src/tui/state/inflight.rs` — expose current, live/stopping root, and retained-output producer identity as distinct borrowed/immutable states.
- `src/tui/state/mod.rs` — export the owned observation states required by polling without exposing termination authority.
- `src/build_monitor/benchmarks.rs` — add a third fixture row at a realistic build-candidate count. The two existing rows (`SMALL_FIXTURE_PROCESS_COUNT` 1_000, `LARGE_FIXTURE_PROCESS_COUNT` 5_000) yield ~260 and ~1_300 build candidates at `COMPILERS_PER_HUNDRED`/`WRAPPERS_PER_HUNDRED`/`CARGO_ROOTS_PER_HUNDRED`, both far past a real `-j16` build's ~16 rustc. Both rows classify as `RequiresDedicatedWorker`, so neither can detect a per-candidate cost regression: doubling `fs::canonicalize` cost (`src/build_monitor/build_classifier.rs:646`, up to four calls per candidate) leaves the assertion unchanged. Add a row at ~30 candidates asserting `EventLoopAffordable`, so the regression is visible at the load the monitor actually runs under.

**Constraints from prior phases:** Phase 1 supplies exact index identities and current/retained/uninitialized readiness. Phase 3 supplies cadence-before-filesystem ordering, the dedicated executor, `CompletedProcessRefreshExecution`, named no-completion failures, and one-counted raw Running metrics refresh. Phase 5 owns named scope resolution and enablement/scope generation; because `ActiveMonitorState::new` is private, adding snapshots, tombstones, or deadlines to that struct forces matching signature changes in `CompileVisibilityState::enable`, `CompileVisibilityState::replace_scope`, and `App::toggle_compile_visibility` — those three are the only ways to construct or replace it. Phase 6 supplies pure classification, `BuildClassifier`, exec-sensitive session/activity IDs, immutable owned-root/candidate evidence, and ambiguity omission. Phase 7 owns worker-side classification, generation-bound cancellation, separate `CompileClassificationExecution`, and the neutral shared App adapter; extend those boundaries rather than introducing another observer, classifier, result receiver, or timing channel. Phase 4 owns semantic lifecycle and captured output, whose producer ID may differ from the current queued/starting ID; polling never copies output or infers its producer from current lifecycle identity. Do not add rendering or termination authority. Phase 10's scope-refresh-on-input decision is settled: `workspace_index_readiness` stays on the input-event path unchanged, so `MonitorScopeResolutionRevision` equality is fixed and this phase's `PendingWithRetained` rule can be built against it. See Phase 10's Constraints for the measurement and for the two remedies that are ruled out.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-port`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-port`, and `bash ~/.claude/scripts/delegate/verify.sh lint cargo-port` are green; tests prove disabled means no compile deadline/new refresh/parsing/result acceptance; 500 ms deadlines rearm exactly without queued catch-up; simultaneous Running one-second and compile due work performs one coalesced cycle and one counted raw Running metrics refresh; compile-classification failure ages monitor data without discarding a successful Running result; whole-cycle failure ages stale data to non-actionable then unavailable; completed empty classification does not; a `ProjectListRevision` bump that leaves both canonical root sets unchanged advances the generation into `PendingWithRetained` without blanking the display, while one that changes a root shows `Pending`; retained data remains actionable and a termination requested against it re-resolves the retained ID against the current process snapshot, signalling the live session and signalling nothing when that session has already exited; every data-bearing variant carries the observation instant and age is re-derived from it rather than inferred from the variant; toggle/scope cancellation during an in-flight combined request skips compile parsing/classification, preserves due Running output, and rejects the cancelled generation; target-directory appearance and symlink retargeting revise live resolution without metadata/list changes; same-PID exec invalidates prior session/activity actionability; owned activity joins once by verified live/stopping root; run N retained output can coexist with queued/starting run N+1 without being joined to N+1; and selection changes never affect external processes. Scope filtering is proven by one pair: a live external Cargo session whose checkout root is outside the current `BuildScopeKey` is absent from the stored presentation snapshot, while an owned run whose root is equally outside it survives in that same snapshot — and a test asserts `classify_demand` still returns both, so the narrowing is observable in `BuildMonitor` and nowhere else. Also asserted rather than built, because Phase 7 already shipped them: one coalesced refresh cycle per interval with no duplicate cycle per workspace, column, or consumer (`due_demand`, `src/process_observation/executor.rs:439-462`), and no compile demand at all while monitor state is `Off` (`compile_classification_demand`, `src/tui/process_refresh.rs:227-237`).

Suppressions and gates. Exactly two `#[allow(dead_code, reason = "…")]` remain anywhere in `src/`, and this phase deletes both: `ActiveMonitorState::monitor_selected_row()` (`src/tui/compile_visibility/mod.rs:60-64`, "Reserved for Build Monitor snapshot and deadline state") and `CompileVisibilityState::accepts_generation` (`:124`, "Reserved for Build Monitor enablement lifecycle") — this phase extends `ActiveMonitorState` and owns late-result acceptance, so it is the first reader of both. Afterwards `src/` contains no `dead_code` suppression at all. Three items the plan previously assigned here are already resolved and need no action: `ActiveMonitorState::new` (now a private `fn` at `:26`) and `compile_monitor_generation()` (`:73`) carry neither a gate nor a suppression and already have production callers, and the scope-authorization items `MonitorScopeActionability` (`src/tui/compile_visibility/scope.rs:163`) and `actionability()` (`:169`) are ungated and reached in production through `build_scope_actionability` (`src/tui/compile_visibility/mod.rs:158`) from `compile_classification_demand` (`src/tui/process_refresh.rs:233`).

This phase also deletes the `#[cfg(test)]` gates whose first production caller it supplies: `monitor_scope_resolution()` (`src/tui/compile_visibility/mod.rs:68`), and `BuildClassificationExecutionFailure` (`src/build_monitor/execution.rs:112`) with `CompileClassificationExecution::Failed` (`:130`), whose reachable cause is this phase's failure aging. It does **not** delete the rest of Phase 6's gates: the `BuildSession` presentation accessors (`src/build_monitor/session.rs:444-479`) stay gated for Phase 9, which renders them, and the root-identity accessors — `SessionRootObservation::root_identity` (`:395`) and `BuildSession::root_identity` (`:461`) — stay gated for Phase 11's pre-signal revalidation. The gate for this phase is therefore: every compile-visibility item **this phase** gave a production caller is ungated, no `dead_code` suppression exists anywhere in `src/`, and no suppression was added to compensate for a removed gate.

### Phase 8 Retrospective

**What worked:**

- `BuildMonitor` really is the single scope-narrowing site. `src/build_monitor/poll.rs` holds the one filter — in-scope sessions plus `Owned` sessions wherever they are — and `classify_demand` still returns both, so the acceptance pair proving the narrowing is observable in `BuildMonitor` and nowhere else passed without argument.
- Generation gating covers both directions: a superseded `Completed` is refused and a superseded `Failed` ages nothing, each with its own test in `src/tui/process_refresh.rs`.

**What deviated from the plan:**

- The Work Order described the live target-directory recheck as behavior; it needed a type. `LiveTargetDirectoryRevision` (`src/build_monitor/scope.rs:87`) holds the resolved `CanonicalPathResolution<CanonicalTargetDirectory>` list and is a `BuildScopeKey` field, so a target directory appearing or a `target` symlink retargeting changes the key by ordinary equality rather than through a separate invalidation path.
- `MonitorSnapshot` gained an explicit `Off` variant. `Pending` was documented "Enabled, with no data" while `BuildMonitor::clear` produced it for a monitor that had just been switched off, so the type could not answer the question it existed to answer.
- `CompileVisibilityState::On` now boxes `ActiveMonitorState` (`src/tui/compile_visibility/mod.rs:164`). The variant crossed clippy's `large_enum_variant` threshold once snapshot, tombstone, and deadline state landed on it. Every read site binds by reference, so auto-deref absorbed the change.
- `BuildSessionActivity` shipped four states, not five. The lock-wait state was already sanctioned as droppable; the running-target state has no evidence source in this phase's inputs, and `src/build_monitor/snapshot.rs:25-31` documents why rather than leaving the omission silent.
- The `poll.rs` tests moved to a sibling `src/build_monitor/poll_tests.rs`.

**Surprises:**

- A background disk-usage flip cannot drop a project row. `compute_visible_rows` (`src/tui/project_list/visible_rows.rs:179`) skips only `Visibility::Dismissed`, and the progression is `Visible → Deleted → Dismissed` (`src/project/info.rs:12`) — a deleted project keeps its row until the user dismisses it. The first version of the background-poll test asserted a row count change and was therefore asserting something production never does.
- `ensure_visible_rows_cached()` is a guard, not a load-bearing rebuild. Every handler that changes rows already rebuilds them (`src/tui/app/tree_mutation.rs:88`, `src/tui/app/async_tasks/dispatch.rs:77,279`, `metadata_handlers.rs:144`, `repo_handlers.rs:566,620`). What the background path actually needs is the scope re-resolve, because a `ProjectListRevision` bump reaches the scope key without routing through `sync_selected_project`.
- The new ~30-candidate benchmark row asserts `EventLoopAffordable` correctly, but cannot detect the per-candidate regression its rationale claims: 26 candidates at a recorded 870 µs against a 15 ms budget need ~543 µs of added cost each, and `measure_fixture` reuses a warmed `BuildClassifier` across samples so a cost that caches never appears. The 1 000-process row is the sensitive one and carries no assertion.
- The whole `MonitorSnapshot` / `MonitorData` / `MonitorSessionRow` read API is `#[cfg(test)]`-gated. This phase writes it; nothing production reads it yet.

**Implications for remaining phases:**

- Phase 9 owns un-gating the snapshot read API — `MonitorSnapshot`, `MonitorData`, `RetainedMonitorData`, `MonitorSessionRow`, `MonitorSessionOwnership`, `BuildSessionActivity`, `MonitorDataActionability`, and `MonitorObservation` accessors in `src/build_monitor/snapshot.rs` — alongside the `BuildSession` presentation accessors it already owned. It must not add parallel production accessors beside gated ones.
- Phase 9's renderer must handle `MonitorSnapshot::Off` as a distinct variant from `Pending`; "no data yet" and "monitor is off" are different displays.
- Phase 13's scope-wide kill set is exactly the snapshot `BuildMonitor` stores, and the owned-session-outside-scope case that its acceptance gate must assert is the one this phase's filter deliberately preserves.
- Five non-gating items carry forward to whichever phase touches them: `MonitorData::session_row` should be `MonitorSessionRow::new`; `toggle_compile_visibility` takes an injected `Instant` while `refresh_compile_monitor_scope_if_on` reads the clock internally; the benchmark row's stated rationale needs correcting or the 1 000-process row needs an assertion; the background-poll test should assert the monitor is on so its final assertion cannot pass vacuously; and one assertion message in `src/tui/compile_visibility/scope.rs:1907` names actionability where it means currency.

### Phase 8 Review

**Code review (5 passes, 16 findings closed).** The dual review and three fix rounds landed 16 fixes, all verified closed by a closure pass. The load-bearing ones: the scope-currency test now bumps project-list visibility before comparing, so it exercises the real invalidation path rather than an unrecomputed fixture; the background-poll test asserts the monitor is on, so its final `requires_scope_replacement` assertion cannot pass vacuously; and `LiveTargetDirectoryRevision` became a real `BuildScopeKey` field compared by `==`, rather than behavior described only in prose. Both gates ran green twice — 1229/1229 tests, clippy clean. A pseudo-terminal smoke test drove the app through toggle on → off → on → quit with no panic and a clean exit.

**Architect review of the remaining phases (14 findings, all applied).** Phase 9 grew the most: the stored snapshot keeps only one collapsed `BuildSessionActivity` per session, so Phase 9 must extend `MonitorData`/`MonitorSessionRow` to retain per-activity rows and the unattributed set before it can render them; it also gained a display mapping for all six `MonitorSnapshot` variants, its missing `src/build_monitor/*` file entries, `MonitorScopeResolution` as a second render input, and a `VisualSelectionSource` type to replace a bare `Option`. Phases 12 and 13 had defined termination against "fresh" sessions, which contradicts what shipped — `actionability()` is `Actionable` for retained data too — and Phase 13's invalidation rule now keys on covered-root inequality instead of any revision change. Phase 13's kill set stopped describing a second filtering pass its own constraints forbid. Phase 10 dropped two suppressions that no longer exist and marked its toggle-off behavior assert-only. Stale line references in Phases 9, 11, and 13 were corrected against real code.

**Two decisions deferred to Phase 11.** One was already open (owning and serializing Cargo Port-owned termination authority). The new one: nothing in the plan owns per-session lifecycle state that survives a 500 ms snapshot replacement, so a `Terminating` transition and the Phase 12/13 tombstones would each be erased by the next poll cycle. Both are recorded as `**Pending decision:**` blocks in Phase 11's Work Order and stop the loop before Phase 11 dispatches.

### Phase 9 — Output monitor presentation and columns · status: done

#### Work Order

**Goal:** The existing Output pane can render monitor empty, single-column, multi-column, and owned-output states from one presentation model.

**Spec:**

- **Staleness survives scope replacement.** `MonitorSnapshot::superseded_by_scope` currently accepts `Stale(monitor_data)` as retention input and republishes it as `PendingWithRetained` (`src/build_monitor/snapshot.rs:306-318`), which `actionability()` reports as `Actionable` (`:265-275`) — so data a failed classification cycle deliberately aged to non-actionable is restored to full authority, and loses its staleness marker, by one arrow-key press within the same workspace. Phase 8 made scope replacement the common case and Phases 11–13 build termination authority directly on `actionability()`, so fix it here. `superseded_by_scope` maps `Stale(data)` to a retained-with-staleness state that `actionability()` reports `NotActionable` and the renderer draws with the stale marker; `Fresh(data)` continues to map to `PendingWithRetained`. Do not resolve this by dropping `Stale` retention to plain `Pending` — that blanks the display on every arrow-key press after any failed cycle, which is exactly the retention Phase 8 exists to provide.

- Replace representation-level output identity with `OwnedRunOutputStateRef::{Absent, Retained { producer: OwnedRunId, title, lines }}` (or equally semantic exhaustive states); no visible output may have an unknown producer. What this makes unrepresentable is "uncorrelated output that still has lines". Phase 7's `OwnedRunRetainedOutput` already avoids bare `Option` — `identity: OwnedRunOutputIdentity::{Correlated(OwnedRunId), Uncorrelated}` and `title: OwnedRunOutputTitle::{Named, Unavailable}` (`src/tui/state/inflight.rs:898-927`) — but today only the constructors keep the two apart: `:537`, `:541`, and `:667` all pass `Vec::new()` alongside `Uncorrelated`. `title` stays an `OwnedRunOutputTitle`; do not flatten it to `Option<String>` while restructuring. Define `OwnedOutputPresentation` from that retained producer independently of any current lifecycle ID, then define `OutputPresentation::{Hidden, OwnedOnly(OwnedOutputPresentation), Monitor(MonitorColumns), MonitorWithOwned { columns, owned: OwnedOutputPresentation }}` and make layout, visibility, focus reconciliation, tabbability, bottom action labels, copy availability, hit testing, and rendering derive from this one value.
- When monitoring is enabled, the Output pane remains rendered and tabbable with a visible `Build monitor on` indicator even when the selected scope is pending, empty, or unavailable.
- Render Phase 5 scope resolution distinctly: pending index, empty non-Rust selection, ambiguous ownership, and unresolved path each have a truthful non-actionable empty-state message rather than collapsing into no sessions. Each of those four carries a `MonitorScopeResolutionRevision`; read its `monitor_workspace_index_readiness()` and say in the message whether the index behind the state is current, a retained last-accepted index, or uninitialized — a pending-index state resolved off a retained index means something different to the user than one with no index at all. This is the only remaining consumer of that accessor, so removing its `#[cfg(test)]` gate is this phase's job. Known asymmetry, not to be fixed here: `MonitorScopeResolution::Ready(MonitorScopeKey)` records no readiness, so an actionable scope resolved off a retained index is indistinguishable from one resolved off a current index.
- One root Cargo invocation is one stable column. A single session uses full width with no divider. Multiple sessions split equally to a readable minimum width; when they do not fit, render a horizontally windowed subset and keep the selected column visible.
- Column headers show operative Cargo command/selectors, checkout/workspace path, resolved profile, root PID, elapsed time, and state. Render active compiler/build/link rows, plus the scope-level unattributed section, as selectable presentation rows.
- **Extend the stored snapshot to carry per-activity data; it does not today.** `MonitorSessionRow` collapses a cycle's activities into one precedence-ordered `BuildSessionActivity` (`src/build_monitor/snapshot.rs:112-141`), and `BuildMonitor::record_classification` drops the `Box<BuildClassification>` after filtering (`src/build_monitor/poll.rs:33-51`), so at render time nothing holds the individual activities or the unattributed set. The `Activity(CompileActivityId)` and `Unattributed(CompileActivityId)` cursor targets below, the activity rows above, and Phase 12's observed compiler-child count all need that data, and this phase's Constraints forbid reaching back to `BuildClassification`. Add each session's attributed activities to `MonitorSessionRow` and the scope-narrowed unattributed set to `MonitorData`, both filled in `record_classification` under the existing scope filter — the filter stays the single narrowing site, so the unattributed set is narrowed there too and is never re-derived downstream.
- **Give every `MonitorSnapshot` variant a display.** The shipped enum is `Off | Pending | PendingWithRetained | Fresh | Stale | Unavailable` (`src/build_monitor/snapshot.rs:219-236`). `Off` and `Pending` are different messages — monitor switched off versus enabled with the first cycle not back yet. `PendingWithRetained` shows the retained rows without a stale marker; `Stale` shows retained rows *with* a visible staleness marker and is non-actionable; `Unavailable` shows neither rows nor a stale marker. The retained-with-staleness state added by the first bullet is a seventh display: retained rows, the stale marker, non-actionable — visually the same as `Stale`, and it must be reachable only through a scope replacement. The acceptance gate below names a case for each.
- Name the visual-selection source instead of encoding it as presence. `OutputSelection::snapshot: Option<Rc<[String]>>` (`src/tui/panes/output/selection.rs:30`, accessor `:47`) encodes "frozen against a captured snapshot" versus "tracking live output" as `Some`/`None`, and `OutputPane::selected_range` returns a bare `Option<(usize, usize)>` (`pane.rs:199`). Replace both with exhaustive states — `VisualSelectionSource::{LiveOutput, Frozen(Rc<[String]>)}` and a named empty/selected range value — in the same restructure as `OwnedRunOutputStateRef` above.
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
- `src/tui/state/inflight.rs` — replace representation-named output correlation with semantic absent/retained producer states while preserving zero-copy output access.
- `src/tui/state/mod.rs` — export the semantic borrowed output state.
- `src/build_monitor/snapshot.rs` — extend `MonitorData`/`MonitorSessionRow` with per-activity and unattributed data; make `superseded_by_scope` preserve staleness across scope replacement instead of republishing `Stale` as actionable; and remove the `#[cfg(test)]` gates on the read API this phase is the first production reader of.
- `src/build_monitor/poll.rs` — fill the added fields inside the existing scope filter in `record_classification`.
- `src/build_monitor/mod.rs` — `BuildMonitor::monitor_snapshot()` is itself `#[cfg(test)]` (`:72-73`), and none of `MonitorData`, `MonitorSessionRow`, `BuildSessionActivity`, `MonitorSessionOwnership`, `MonitorDataActionability`, or `MonitorObservation` is re-exported. Un-gate the accessor and add the re-exports the renderer needs.
- `src/build_monitor/session.rs` — un-gate the `BuildSession` presentation accessors named in the acceptance gate.
- `src/build_monitor/activity.rs` — activity identity and kind for the rendered rows.

**Constraints from prior phases:** Phase 8's presentation snapshot is **already narrowed to the selected scope** — `BuildMonitor` is the single filtering site and the worker stays host-wide. Render what the snapshot contains; do not re-apply a scope filter here, and do not reach past the snapshot to `BuildClassification::build_sessions()`. The only exclusion left to this phase is a completed producer pin; an out-of-scope owned run deliberately survives Phase 8's filter and must still render. Join Phase 8 immutable monitor snapshots with Phase 4 `OwnedRun` by the retained output producer ID, never by `OwnedRun::identity()`: run N output may remain while run N+1 is queued or starting. Live activity joins by Phase 8's verified-root association, while an out-of-scope or completed producer remains pinned independently. Phase 6 first-seen ordering and exec-sensitive `BuildSessionId`/`CompileActivityId`, plus Phase 5 named scope/index readiness and immediate invalidation, bind cursor fallback and empty-state rendering. A same-PID exec transition creates new session/row identities and invalidates prior cursor/actionability. Do not add key handling or termination. **The renderer takes two inputs, not one:** `BuildMonitor::replace_scope` stores a plain `MonitorSnapshot::Pending` whenever the scope is `NotActionable` (`src/build_monitor/poll.rs:73-87`), so the snapshot alone cannot distinguish pending-index from empty-non-Rust from ambiguous-ownership from unresolved-path — that distinction lives only in `ActiveMonitorState::monitor_scope_resolution()` (`src/tui/compile_visibility/mod.rs:150`, already ungated). Pair `CompileVisibilityState` with `MonitorSnapshot` at the render boundary; `Pending` under an actionable scope means "first cycle not back yet", and `Pending` under a non-actionable one is whichever of the four scope-resolution messages applies.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-port`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-port`, and `bash ~/.claude/scripts/delegate/verify.sh lint cargo-port` are green; render/state tests prove distinct non-actionable pending-index, empty-non-Rust, ambiguous-ownership, and unresolved-path states; semantic absent versus retained-output producer states; run N retained output remains attributed to N while N+1 is queued/starting; empty enabled focus; full-width single session; readable/windowed multiple columns; selected-column visibility; stable row/session fallback; same-PID exec invalidates prior cursor and creates new session/activity identities; unattributed section non-actionability; owned output joined once and pinned across scope changes; and monitor-off output matches prior behavior. Each of the six `MonitorSnapshot` variants has its own render case: `Off`, `Pending` under an actionable scope, `Pending` under each non-actionable scope resolution, `PendingWithRetained` showing retained rows without a stale marker, `Stale` showing retained rows with one, and `Unavailable` showing neither. One further case covers the retention fix: age a snapshot `Fresh` → `Stale`, replace the scope, and assert the republished snapshot still reports `NotActionable` and still renders the stale marker — while the same replacement applied to a `Fresh` snapshot yields an actionable `PendingWithRetained`. Activity rows and the unattributed section are asserted from the extended snapshot, with a test proving the unattributed set is narrowed by the same `record_classification` filter as the session rows and not re-derived at render time.

This phase also removes the `#[cfg(test)]` gate — not a `dead_code` suppression — from `MonitorScopeResolutionRevision::monitor_workspace_index_readiness` (`src/tui/compile_visibility/scope.rs:157`, returning `MonitorWorkspaceIndexReadiness` at `:125`), whose empty-state rendering is specified above; from the whole snapshot read API it is the first production reader of — `MonitorSnapshot`, `MonitorData`, `RetainedMonitorData`, `MonitorSessionRow`, `MonitorSessionOwnership`, `BuildSessionActivity`, `MonitorDataActionability`, and `MonitorObservation` accessors in `src/build_monitor/snapshot.rs`, plus `BuildMonitor::monitor_snapshot()` (`src/build_monitor/mod.rs:72-73`); and from the `BuildSession` presentation accessors — `operative_cargo_command`, `cargo_subcommand_recognition`, `root_observation`, `session_target_directory`, and `build_profile` (`src/build_monitor/session.rs:455-489`), `OperativeCargoCommand::subcommand` (`:327`) and `selectors` (`:335`) for the command/selectors header, and `SessionRootObservation::root_pid` and `first_observed_at` (`:409-414`) for the root-PID and elapsed-time columns. Do not add a parallel production accessor beside a gated one — un-gate the existing item. `BuildSession::session_scope` (`:467`) is **not** on that list: Phase 8 already un-gated it and it has a production reader in `src/build_monitor/poll.rs`. The two `root_identity` accessors (`:405`, `:470`) stay gated — their first production caller is Phase 11's pre-signal revalidation.

### Phase 9 Retrospective

**What worked:**

- The single-presentation-value goal held. `OutputPresentation::{Hidden, OwnedOnly, Monitor, MonitorWithOwned}` (`src/tui/panes/output/presentation.rs`) is the one value layout, visibility, tabbability, focus reconciliation, bottom labels, copy availability, hit testing, and rendering all derive from; nothing reads `MonitorSnapshot` or `CompileVisibilityState` a second time at the render boundary.
- Pairing `CompileVisibilityState` with `MonitorSnapshot` at that boundary, as the Work Order required, is what made the four non-actionable scope-resolution messages distinguishable — the snapshot alone still cannot tell them apart.

**What deviated from the plan:**

- **The unattributed set could not be narrowed from what the snapshot carried.** The Work Order assumed `record_classification`'s existing scope filter had enough evidence to narrow the unattributed set the way it narrows session rows. It did not: `UnattributedCompileActivity` held only id, kind, crate identity, and attribution. The classifier does resolve a canonical working directory and output directory per compiler process (`CanonicalProcessPathSet`, `src/build_monitor/classify.rs:235-241`) and already reads the output directory for *attributed* compilers, but discarded both for unattributed ones. Phase 9 added `UnattributedScopeEvidence::{WorkingDirectory(AbsolutePath), Unplaceable}` (`src/build_monitor/activity.rs:296-308`) and narrows on working directory — output directory proves membership only through a session's resolved target directory, which an unattributed activity by definition lacks, and under `--target-dir` can point outside the checkout entirely. The filter remains the single narrowing site (`src/build_monitor/poll.rs:194-222`).
- Four presentation types are `pub` rather than `pub(super)`: they appear as fields of `OutputPresentation`'s public variants, and enum variant fields cannot carry their own visibility.
- The phase's rendering split across new files rather than growing `render.rs`: `monitor_render.rs`, `presentation.rs`, `hit_map.rs`, `constants.rs`, and a sibling `presentation_tests.rs`.
- Several bare `Option`s beyond the two the Work Order named were replaced in the same restructure: `OwnedOutputVisibility::{Absent, OnScreen}`, `MonitorVisibility::{Off, On}`, `VisualSelectionPermission::{Denied, CapturedOutput}`, `HitRegion::{Truncated, Drawn(Rect)}`, `UnattributedSectionLayout { height, hidden }`, and `MonitorEmptyStateMessage { headline, index_note }` (both `&'static str`, so the render path no longer allocates per frame).

**Surprises:**

- **`toggle_compile_visibility` still has no production caller.** Every call site is a test (`src/tui/app/mod.rs:1338` is the definition; `process_refresh.rs:449,495,524` and `async_tasks/poll.rs:315` are all `#[cfg(test)]`). The monitor is therefore unreachable from the running application, so Phase 9's monitor-on rendering has never executed outside tests. Phase 10 owns wiring it.
- `OwnedColumnWitness` was a zero-argument constructor whose doc claimed it carried producer identity — it enforced nothing. It is now `OwnedColumnWitness(OwnedRunId)`.
- Sizing the owned body's scroll extent required duplicating the drawing arithmetic: `owned_captured_output_height` (`monitor_render.rs:189-208`) mirrors, term for term, what the pinned and in-scope draw paths consume before emitting output rows.
- Test scaffolding was a substantial part of the work, not an afterthought — `ClassificationFixture::{cargo_root_with_pid, compiler_under}`, `ClassifiedRoot`, `classified_monitor_snapshot`, and `Inflight::with_retained_output_and_next_run_queued`. Two acceptance-gate tests initially passed vacuously precisely because that scaffolding did not yet exist to make them fail.

**Implications for remaining phases:**

- Phase 10 must wire the compile-visibility toggle to a key; until it does, no monitor rendering path is reachable at runtime.
- Phase 10 changing column layout must update `owned_captured_output_height` alongside the draw paths it mirrors, or the owned body's scroll offset silently ranges over the wrong span.
- Phase 13's dependence on `record_classification` as the single narrowing site is now stronger: the unattributed set is narrowed there too, using evidence the classifier retains only for that purpose.
- Phase 12's observed compiler-child count reads the per-activity data this phase added to `MonitorSessionRow`.

### Phase 9 Review

**Code review (4 passes, 6 findings closed).** The dual review and two fix rounds landed 6 fixes, all verified closed by a closure pass. The load-bearing one: the scope filter was dropping nothing from the unattributed set because the set carried no evidence of where its processes ran, so `UnattributedScopeEvidence::{WorkingDirectory, Unplaceable}` was added and the classifier now retains each activity's working directory specifically for that filter — `output_directory` was rejected because it proves membership only through a session's resolved target directory, which an unattributed activity has none of, and under `--target-dir` it can point outside the checkout entirely. `Unplaceable` survives every scope by design: a compiler process whose working directory could not be read cannot be proven outside. The column-window test was also tightened from "some window" to exact `(first, count)` pairs across four widths, and the session-row test moved to a three-root fixture asserting real counts. Both gates ran green twice — 1244/1244 tests, clippy clean, no suppressions added. A pseudo-terminal smoke test drove the app through navigation plus three degenerate resizes (2×200, 1×60, 50×200) and quit cleanly with no panic.

**Architect review of the remaining phases (13 findings, all applied).** Phase 10 changed the most: three of its spec bullets were already satisfied by what shipped (mouse-click column selection, horizontal paging following the cursor, drag/Ctrl-A refusal outside captured output) and became assertions rather than work; it gained the three input sites it actually has to change (`dispatch_output_selection_gesture`, `classify_output_cancel_preflight`, and that function's `CloseVisibleOutput` branch — the last two decide Esc behavior with no column awareness at all), the split-borrow accessor its navigation needs to compile, the fact that `OutputCursor` has no motion API yet, and the `owned_captured_output_height` two-places warning. Phase 12 gained the `StaleWithRetained` variant in its non-actionable list, ownership of the un-gate of `actionability()`/`MonitorDataActionability`/`MonitorObservation`, a requirement to resolve the target session by identity at action time (the cursor is only reconciled during render), and direction to widen the existing `OutputCursorColumn` rather than invent an `Option<&BuildSessionId>`. Phase 13's kill-set justification was invalidated by what shipped and now states that the set is `MonitorData::session_rows()` alone, with unattributed rows counting toward neither the live-root set nor the all-or-refuse check. Phase 11 needed nothing beyond line-reference fixes. Stale references across Phases 10–13 were corrected against real code.

**Two decisions deferred, one since resolved.** Phase 10's was that its navigation spec let the cursor land on activity rows that are neither drawn nor hittable, because no per-column scroll state existed. **Resolved: add the scroll offset.** It is now in Phase 10's Spec, Files, and acceptance gate as a single offset belonging to the selected column, reset to zero when the selection moves to another column — not a map keyed by session, which would need pruning against a snapshot that is replaced wholesale every poll cycle. Phase 12 carries the other: "may this be acted on" now has two independent derivations that partition the seven `MonitorSnapshot` variants identically, one production and one test-gated, and the phases that authorize destruction read the test-gated one while the renderer draws off the other — the recommendation is to derive `actionability()` from `monitor_display()`. Both are recorded as `**Pending decision:**` blocks in those Work Orders and stop the loop before each phase dispatches.

### Phase 10 — Monitor navigation, toggle, and owned-output coexistence · status: done (8a3a1af)

#### Work Order

**Goal:** Users can toggle and navigate compile visibility through the framework keymap while existing Output copy, visual selection, pane snaking, and target stopping remain correct.

**Spec:**

- Add a global framework-keymap action for `Shift-C` that toggles monitoring. The state starts off each launch and is not persisted. Toggling off immediately drops external snapshots/tombstones/deadlines, stops polling, and returns to current owned-output behavior — **asserted here, not built here.** Phase 8 shipped the whole toggle body: `App::toggle_compile_visibility` cancels the in-flight generation, advances it, disables, calls `BuildMonitor::switch_off()`, and pushes the schedule (`src/tui/app/mod.rs:1334-1338`). Every one of its call sites is still `#[cfg(test)]`, so no monitor rendering path is reachable at runtime until this phase wires the key. This phase makes it reachable and proves the behavior, in the same style Phase 8 asserted rather than rebuilt `due_demand` and `compile_classification_demand`.
- Up/Down traverse the selected column's complete body; an owned column crosses activity rows into captured output while skipping the separator. Home/End/Page Up/Page Down/half-page operate vertically in that column. Left/Right and normalized Vim `h`/`l` select adjacent columns without leaving Output.
- **Traversal reaches rows the column is too short to draw, so this phase adds vertical scroll state to the selected column.** Today `render_column` lays activity rows out from the top with no offset and `row_rect` returns `HitRegion::Truncated` for anything past the bottom (`src/tui/panes/output/hit_map.rs:108`, `monitor_render.rs:498-545`), while the pane's single `Viewport` is sized to the owned captured output alone (`render.rs:49-58`) — so without this, Up/Down lands the cursor on rows that are neither drawn nor hittable. Hold **one** offset, belonging to the cursor's current column identity, and reset it to zero whenever the selected column changes. Do not key a map by `BuildSessionId`: the snapshot is replaced wholesale every poll cycle, so such a map needs pruning against a row set that has already changed underneath it, and a retained offset for a column the user is not looking at describes rows that may no longer exist. Reconcile the offset against the cursor the way the owned viewport already is, and draw the column's rows from it. The scope-level unattributed section keeps its existing behavior — it is not cursor-selectable and already reports its remainder through `UnattributedSectionLayout { height, hidden }` (`monitor_render.rs:445-483`).
- Tab/Shift-Tab run an action-aware Output preflight before framework-global pane navigation. Traverse the complete ordered session list, including off-screen columns; with zero/one session or at the first/last boundary, fall through to normal pane-snaking order.
- Mouse clicks select column and typed row through Phase 9 hit rectangles, and horizontal paging follows the selected column — **asserted here, not built here.** Both shipped in Phase 9: `src/tui/interaction.rs:61-64` routes a click through `OutputPane::focus_hit`, and `monitor_render.rs:498` calls `window(area.width, cursor.column_index())`.
- Copying an activity row copies that row. Visual selection, drag selection, and Ctrl-A may start only within owned captured output and retain frozen-snapshot semantics — the drag and Ctrl-A halves are **asserted here, not built here** (`src/tui/input/dispatch.rs:637-640` and `:837-845` already gate both on `visual_selection_permission()`). What this phase adds is the third selection path: `dispatch_output_selection_gesture` (`dispatch.rs:270-297`, handling `V` and Shift/Ctrl-Shift arrows) never consults that permission. Hidden visual selections stay stored but intercept copy/Esc only when their owned output region is selected.
- `Esc` exits active owned visual selection first. While monitoring it stops an owned target only when that owned column is selected; it never stops an unselected external build. Two input sites decide this today and neither is column-aware: `classify_output_cancel_preflight` derives `running_example` from `owned_run().is_running()` (`dispatch.rs:208`), and its `CloseVisibleOutput` branch keys off `output_copy_availability()` (`:210-214`), which reports `CapturedOutput` whenever owned output is on screen at all. Both must require the owned column to be the selected one — Esc-close follows the same rule as Esc-stop, so with an external column selected `Esc` neither stops nor clears the owned run.
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
- `src/tui/panes/output/pane.rs` — vertical/horizontal/tab navigation, and the selected column's vertical scroll offset (one value, reset on column change).
- `src/tui/panes/output/monitor_render.rs` — draw a column's activity rows from that offset instead of always from the top, and report the drawn span so the hit map stays consistent with what was drawn.
- `src/tui/panes/output/selection.rs` — activity copy and owned-only visual selection. `OutputCursor` today has only `focus_*` constructors and `reconcile` (`:239-278`) — there is no motion API yet; this phase writes one.
- `src/tui/app/mod.rs` — reconcile focus, remove the enablement-lifecycle suppression, and add a split-borrow accessor for navigation. Phase 8 already shipped `App::toggle_compile_visibility` (advance generation → disable, or resolve scope → advance → enable) and `CompileVisibilityState::Off` at construction; this phase makes the toggle reachable, it does not write its body. Navigation needs the presentation at input time, and `App::output_presentation()` borrows `&self` across `compile_visibility_state`, `build_monitor`, and `inflight` (`:1152-1159`), so `app.panes.output.<motion>(&presentation)` cannot compile. Add a split-borrow accessor in the style of `split_for_render` (`:660-710`), which the render path already uses to solve exactly this.
- `tests/assets/default-keymap.toml` — pin `shift-c`, `alt-k`, and `alt-shift-k` defaults.

**Constraints from prior phases:** Phase 9's `OutputPresentation` is the sole source of pane/layout/action state. **Changing column layout means changing two places.** `owned_captured_output_height` (`src/tui/panes/output/monitor_render.rs:186-206`) duplicates the draw paths' arithmetic term for term — the pinned path subtracts caption plus separator, the in-scope path subtracts indicator, unattributed section, column header, activity rows, and separator — and its result sizes the owned body's scroll extent. Any change to what a column draws above the captured output must be made there too, or the scroll offset silently ranges over the wrong span and the last rows become unreachable. Phase 5 owns toggle and named scope lifecycle, Phase 8 owns conditional polling, and Phase 9 owns typed hit rectangles plus retained-output producer identity. **Keep `workspace_index_readiness` on the input-event path exactly as it is.** Making `Shift-C` reachable puts `refresh_compile_monitor_scope_if_on` behind `sync_selected_project` (`src/tui/input/dispatch.rs:73`), which reaches `App::workspace_index_readiness` (`src/tui/workspace_index.rs:34`) and takes the metadata-store mutex on every key and mouse event while monitoring is on. That cost was measured and accepted: no lock site anywhere holds the mutex across a slow operation — the seven sites are `construct.rs:261`, `workspace_index.rs:36`, `state/scan.rs:175`, `state/scan.rs:200`, `async_tasks/metadata_handlers.rs:138`, `scan/cargo_metadata.rs:214`, and `scan/cargo_metadata.rs:342`, and the last releases its guard inside the `map_or` before the blocking `cargo metadata` exec begins (`run_cargo_metadata_for_root`, `:338-344`). The steady-state per-event cost is therefore one uncontended lock/unlock plus the two revision compares in `rebuild_if_changed` (`src/project/cargo/workspace_index.rs:348`), which returns `Unchanged` unless the accepted-metadata or project-list revision actually differs — neither of which a keystroke can change. Do **not** add a non-rebuilding readiness accessor or a third "not revalidated on this call" readiness state: either makes `MonitorScopeResolutionRevision` differ between the metadata-handler path and the keystroke path, and that field participates in `requires_scope_replacement` equality, so every keystroke would advance the monitor generation with nothing changed and defeat Phase 8's `PendingWithRetained` retention rule. Do not track index currency separately on `App` either; it is a second source of truth about index freshness. Phase 4's owned stop/copy/clear behavior must remain unchanged when monitoring is off: closing Output during `Stopping` clears captured lines without freeing the active slot; matching late output/progress remains accepted until `Finished`; and final reconciliation places exactly one gone-after-signal marker last.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-port`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-port`, and `bash ~/.claude/scripts/delegate/verify.sh lint cargo-port` are green; interaction tests cover toggle lifecycle, indicator/focus, row and column traversal, tab fallthrough, Vim movement, mouse hits, output separator skipping, traversal of a column holding more activity rows than the pane can draw (every row reachable, the offset following the cursor, and the offset resetting to zero when the selected column changes), activity-row copy, owned-only visual selection, Esc precedence, run N retained output beside queued/starting run N+1, owned run alongside external updates, closing Output while stopping without freeing the run slot, late correlated output/progress, one final gone-after-signal marker after flush, portable defaults, macOS Option labels, other-platform Alt labels, and keymap conflict validation. This phase also deletes the one remaining compile-visibility `dead_code` suppression in `src/` — on `App::toggle_compile_visibility` (`src/tui/app/mod.rs:1334-1338`) — which the keybinding makes reachable. The three the plan previously named here, on the `CompileVisibilityState::On` variant, `enable`, and `disable`, are already gone: Phase 8 gave all three production callers. After this phase no `dead_code` suppression added by the compile-visibility work may remain anywhere in `src/`.

### Retrospective

**What worked:**

- The single-presentation-value design held through navigation. `OutputPresentation` is still the only value layout, focus, copy, hit testing, and now motion derive from; the motion API reads it rather than re-reading `MonitorSnapshot` or `CompileVisibilityState`.
- `App::split_output_for_navigation`, modeled on `split_for_render`, solved the borrow problem the Work Order predicted, with no restructuring of `output_presentation()`.

**What deviated from the plan:**

- The phase's first implementation shipped five blocker-level gaps against its own Work Order, all closed across two fix passes: keyboard traversal (no motion API was written at all — every motion still routed into the owned `Viewport`), Tab/Shift-Tab preflight, activity-row copy, Esc gating for hidden visual selections, and most of the interaction-test matrix.
- `owned_column_selection` initially derived column identity from `VisualSelectionPermission`, which answers a row-kind question — so Esc did nothing with the cursor on the owned column's header. It now compares the cursor's column against the retaining producer.
- `OutputCursorColumn::Detached` was split into `Absent` / `UnattributedSection` / `OwnedCapturedOutput` so the per-column scroll offset resets when moving between the unattributed section and captured output.
- `KeyBind::display()` now spells ALT + uppercase as `alt-shift-k`, and `platform_label` uppercases only alphabetic `KeyCode::Char` — a user-bound `alt-up` rendered `Option-UP` before.
- Nine acceptance-gate matrix cases were satisfied by existing tests rather than re-implemented. The closure review verified each named test genuinely covers its case, and found two items covered by neither list — the monitor indicator row and Vim `h`/`j`/`k`/`l` — which fix pass 2 added.

**Surprises:**

- `refresh_compile_monitor_scope_if_on` replaces any scope the app did not resolve itself and resets the snapshot to `Pending`. A caller that assigns a scope key directly loses the monitor on the very next keystroke; enabling must go through `toggle_compile_visibility`.
- The three Esc branches in `classify_output_cancel_preflight` do not share a guard: `output_visual` and `visible_output` test `is_output_cancel`, but `running_example` tests `code == KeyCode::Esc`. They diverge whenever Esc is unbound from `OutputAction::Cancel`, which is why the per-keystroke `owned_column_selected` derive could not simply be hoisted behind the cancel check.
- Only two Alt binds exist in the whole workspace, so the `display()` change had a two-bind blast radius; `display_short` is untouched and the status bar still renders `⌥K`.
- `script -q /dev/null` cannot smoke a TUI: its pty carries no window size, so the app starts, paints nothing, and exits 0 — which reads as a pass. The smoke test needs a pty with `TIOCSWINSZ` set explicitly.

**Implications for remaining phases:**

- Phase 12's Work Order is factually wrong about `OutputCursorColumn`: it names `{Detached, Session(BuildSessionId)}` at `selection.rs:199-205` and directs the phase to widen that type's visibility. The type now has four variants at `selection.rs:220-232` and is already `pub(super)`.
- Phase 12 inherits a motion API rather than writing one: `ColumnBodyRow`, `SelectedColumn`, and `OutputCursor::{selected_column, body_position, body_row_at, place_body_row, select_column}`.
- Phase 12's requirement to resolve the target session by identity at action time is *partly* served: the cursor stores the session and resolves through `column_index_of`, and `reconcile_cursor` runs before any action reads it. But `OutputCursor::selected_column` is **not** the resolver for a destructive action — it maps `Absent`/`UnattributedSection` to `columns.first()` so motion always has a body to walk. `copy_payload_for_cursor` establishes the dispatch-on-`OutputCursorTarget` pattern its action routing should follow.
- Phase 11's termination authority must not read `owned_column_selection` as a row-kind signal — it answers a column-identity question only.

### Phase 10 Review

Architect review of the three remaining phases against the code Phase 10 actually shipped. Seventeen findings; all applied. **Phase numbering changed here:** the old Phase 11 split into Phase 11 (owned-run actor, platform adapters, terminator worker) and Phase 12 (authorization, bounded transaction, lifecycle overlay), moving the two UI phases to 13 and 14. References below use the new numbers; archival text in earlier phases uses the old ones.

**Cross-cutting answer to the question that triggered the review** — Phase 10's largest failure was that its Work Order assumed a motion API on `OutputCursor` that did not exist. No remaining Work Order repeats it: every type and method they name as already present does exist, including `ProcessObserver` (`src/process_observation/mod.rs:1538`), `OwnedProcessGroupTerminationCapability` (`src/tui/state/inflight.rs`), and both `#[cfg(test)]` root-identity accessors (`src/build_monitor/session.rs:404`, `:465`). Four line references had drifted and are corrected in place.

**Applied without a gate** — Phase 13's cursor-exposure directive is replaced: the pane computes and returns a named answer type rather than exposing any cursor enum, matching the shipped `owned_column_selection` pattern. Its target resolver is now stated as a separate derivation from the motion resolver, its availability rule is restated as column identity rather than row kind, and its terminating/tombstone markers move from `render.rs` to `monitor_render.rs` — Phase 14's identical error corrected the same way. Phase 14 gains `async_tasks/poll.rs` (where `CoveredScopeRoots` invalidation has to run) and loses the keymap fixture from its work list (Phase 10 already pinned it). Both phases now name the two lines that keep Alt-K inert. Phase 13's modal-precedence tests are redirected to the `dispatch.rs` harness, the only one that can reach the modal layer.

**Resolved at the pre-dispatch gate** — the old Phase 11's two open decisions. Cargo Port-owned termination authority gets a single owner: an `OwnedRunProcessActor` holding both the child wait and Phase 4's non-cloneable group capability, issuing opaque run-bound tokens, which also orders signaling against reaping by construction. That, plus the six-safety-boundary size of the original Work Order, drove the split into Phases 11 and 12. Per-session lifecycle state gets a `BuildMonitor`-held overlay outside `monitor_snapshot` — the poll cycle replaces the snapshot every ~500 ms, so `Terminating` and both tombstone kinds have nowhere else to survive — and Phase 12 also changes `OutputPresentation::derive` to carry it, since Phase 9's single-presentation constraint otherwise makes the overlay invisible to every renderer. Proving lifecycle state reaches the screen therefore lands in the phase that creates the first transition rather than a phase later.

**Deferred as a pending decision** — Phase 13 gained one: its modal precedence and `y`/`n`/Esc rules are not new wiring but changes to the confirm modal shared by five existing actions, and shipped dispatch currently runs the Output cancel preflight *before* the confirm handler. Whether to scope those changes to the new action or change the shared modal — and whether that becomes its own phase ahead of Phase 13 — is unresolved.

### Phase 11 — Owned-run termination actor and platform capability foundation · status: done

#### Work Order

**Goal:** One owner of Cargo Port-owned termination authority, identity-bound external platform capabilities, and a nonblocking `ProcessTerminator` worker boundary — with no `BuildMonitor` authorization and no bounded transaction.

**Spec:**

- Introduce an `OwnedRunProcessActor` that is the sole owner of the owned run's child wait and Phase 4's non-cloneable `OwnedProcessGroupTerminationCapability`. It exposes opaque, run-bound termination authorization through one serialized endpoint; monitor state, retained modal confirmations, and UI code hold a token the actor honors or refuses, and never clone, decompose, or inspect the capability. Routing both operations through the one actor is what orders signaling against child waiting by construction, so a reaped group leader can never be confused with a reused group ID.
- Keep observation and termination separate. Add a separate `ProcessTerminator`. `ProcessObserver` produces immutable evidence and safe platform capabilities only; it never accepts termination plans or signals. `ProcessTerminator` runs on a dedicated worker/channel path so revalidation, signaling, and deadline waits never block the TUI event loop.
- Implement platform adapters that bind signaling to the observed process object strongly enough to reject PID reuse. Use an identity-bound handle or another demonstrated safe adapter where the platform supplies one; a platform without a proven safe adapter exposes external sessions as `ObservedOnly`. Do not assume macOS is observed-only without checking available host APIs, and never fall back to a bare/racy PID action or an external ambient process group.
- Define and transport the correlated request/result boundary — `TerminationRequestId`, opaque immutable execution plans, `TerminationOutcomeSummary`, and `TerminationError`. This phase defines these types and carries them across the worker channel; Phase 12 is what constructs a plan from authorized sessions and reconciles its result.
- Do not escalate automatically to `SIGKILL` at any layer this phase introduces.

**Files:**

- `src/process_observation/mod.rs` — expose immutable observation evidence and safe platform capability construction without a signal API.
- `src/process_observation/identity.rs` — expose identity revalidation evidence without weakening encapsulation.
- `src/process_termination/mod.rs` — public `ProcessTerminator` worker/channel boundary and correlated request/result API.
- `src/process_termination/platform.rs` — safe identity-bound capability adapters or explicit observed-only fallback.
- `src/main.rs` — declare the process-termination module.
- `src/tui/state/inflight.rs` — move the Phase 4 owned-run authority and child-wait boundary behind the actor, without exposing group IDs or process identity to UI code.
- `src/tui/terminal/processes.rs` — adapt the existing owned process-group stop to the actor's serialized endpoint.
- `src/tui/background.rs` — host the dedicated nonblocking termination worker.
- `src/tui/messages.rs` — carry correlated termination plan/results.
- `Cargo.toml` — add only platform dependencies needed for identity-bound external signaling.

**Constraints from prior phases:** Phase 2 defines strong/insufficient identity, exec-sensitive incarnation, chronological identity evidence, and validated ancestry. Phase 4 owns isolated process groups and `OwnedProcessGroupTerminationCapability`; this phase takes over that ownership rather than duplicating it. Phase 4's failed-launch cleanup may use `TERM` then `KILL` only because strong identity could not be established and Cargo Port still owns the freshly spawned isolated group; that cleanup helper is never user-requested termination policy and cannot be reused by `ProcessTerminator`. Phase 3's `RunningTargetTerminationCapability` remains a separate legacy consumer and can never be reused as build-monitor authority. `ProcessObserver` remains observation-only and all new build signaling runs through `ProcessTerminator` off the event loop.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-port`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-port`, and `bash ~/.claude/scripts/delegate/verify.sh lint cargo-port` are green; deterministic process-fixture tests prove PID reuse and same-PID exec rejection at the adapter layer, safe-adapter or observed-only fallback per platform, that the actor is the only holder of the owned group capability and no clone/decompose/inspect path exists for callers, owned group signaling serialized with child waiting, termination work off the event loop, exact request/result correlation, and no automatic `SIGKILL`.

#### Retrospective

**What worked:**

- `OwnedRunProcessActor` now owns the child wait and non-cloneable group capability, while the TUI retains only `OwnedRunTerminationToken`.
- Linux external termination uses `pidfd`; platforms without a proven identity-bound adapter remain observed-only, and `ProcessTerminator` keeps host work off the event loop.

**What deviated from the plan:**

- Owned-run output, progress, termination outcomes, and completion moved onto one ordered `OwnedRunEvent` channel, which also required changes in `src/channel.rs`, `src/tui/app/async_tasks/poll.rs`, `src/tui/app/construct.rs`, `src/tui/app/mod.rs`, and `src/tui/state/owned_run_process_actor.rs`.
- No `Cargo.toml` change was needed because the existing platform dependencies cover the Linux adapter.
- Review repairs tightened capability minting, process-gone precedence, child reaping, and the accepted-command/completion race before closure converged.

**Surprises:**

- Correct actor ordering requires closing command admission, draining every already-accepted termination request, and only then publishing `OwnedRunEvent::Finished` on the same FIFO channel.
- A same-lifetime `exec` transition needs executable-image continuity in addition to `ProcessIdentity`; identity continuity alone is insufficient termination authority.

**Implications for remaining phases:**

- Phase 12 must carry `OwnedRunTerminationToken` for owned sessions and `ExternalProcessTerminationCapability` for external sessions without exposing their process-control data to UI code.
- Phase 12 receives external results through `ProcessTerminator` and owned results through `OwnedRunEvent`; `App::poll_example_msgs` already reconciles the actor event stream in FIFO order.
- External signaling is available through Linux `pidfd` only; macOS and other hosts without a demonstrated identity-bound adapter must preserve observed-only actionability.

#### Phase 11 Review

- Phase 12 now owns one transaction identity and bounded fan-in across `ProcessTerminator` results and FIFO `OwnedRunEvent` outcomes, plus semantic per-target correlation.
- Phase 12 now transports move-only external capabilities from the existing `ProcessObserver`, represents observed-only support before constructing authority, and extends rather than duplicates Phase 11's execution-plan/result foundation.
- Phase 12 now names one `BuildTerminationLifecycleRegistry`, states its eviction rules, joins it into presentation values instead of snapshot rows, and includes the missing observer, worker, event-loop, and owned-token join files.
- Phase 13 now uses termination-specific type names and current dispatch references; Phase 14 invalidates retained scope authorization at the shared scope-replacement owner and treats `aggregate.rs` as a projection.
- Two decisions were deferred to Phase 12: derive actionability from the visible monitor classification, and permit only one active build-termination transaction or support concurrent transactions.

### Phase 12 — Termination authorization, bounded transaction, and lifecycle registry · status: todo

#### Work Order

**Goal:** `BuildMonitor` can mint opaque termination authorization from current actionable sessions, execute one frozen identity-revalidated bounded transaction across both Phase 11 termination backends, and hold per-session lifecycle state that survives snapshot replacement and reaches the renderer.

**Pending decision: One visible classification must decide actionability**

Actual problem:
`MonitorDisplay::Rows { monitor_staleness }` drives the pane, while test-gated `MonitorSnapshot::actionability()` independently partitions the same seven snapshot variants. Phase 12 must decide authority retention during `replace_scope`, so leaving the two answers independent can show live-looking data beside a disabled action or stale-looking data beside an enabled destructive action.

What exists now:
- `monitor_display()` is the production renderer input; `actionability()` has no production reader.
- `Fresh` and live `PendingWithRetained` are actionable; stale, stale-retained, pending-without-data, off, and unavailable states are not.

What should change:
- Derive `actionability()` from `monitor_display()`, or derive both from one private exhaustive classification of all seven variants, and add one agreement test covering every variant.

Recommendation:
Derive `actionability()` from `monitor_display()` in `src/build_monitor/snapshot.rs`; the user-visible staleness classification should be the source of truth Phase 12 uses to mint authority.

**Pending decision: Build-termination transaction concurrency**

Actual problem:
`ProcessTerminator` accepts an unbounded request queue, while the owned-run actor admits one request and the plan defines no behavior for a second selected or scope-wide submission before the first transaction completes.

What exists now:
- External requests can queue without a product-level concurrency policy.
- Each owned run withholds retry authority while one actor request is pending.

What should change:
- Either admit one active `BuildTerminationTransaction` and refuse new selected/scope submissions until it reaches a terminal state, or define multiple concurrent transaction storage, modal behavior, lifecycle joins, cancellation, and rendering.

Recommendation:
Permit one active build-termination transaction. Reject later submissions with a visible busy result until the active transaction completes; this matches the owned-run actor and keeps one lifecycle owner.

**Spec:**

- Define authority-bearing `BuildTerminationAuthority::{Owned(OwnedBuildTerminationAuthority), External(ExternalBuildTerminationAuthority)}` variants. The owned variant retains `OwnedRunId` plus Phase 11's actor-issued run-bound authorization; the external variant bundles confirmed scope attribution, strong root identity, lifecycle eligibility, and Phase 11's identity-bound platform capability. An identifier by itself is never action authority.
- Define `ExternalTerminationSupport::{Actionable(ExternalProcessTerminationCapability), ObservedOnly}` at the observation/build-monitor boundary. `PlatformTerminationCapabilityObservation::Available` is not proof of signal support because its private adapter may be observed-only; only the semantic `Actionable` variant can construct `BuildTerminationAuthority::External`.
- A `Fresh` snapshot or live `PendingWithRetained` snapshot may construct a termination request when its session carries `BuildTerminationAuthority`. Pending without retention, stale, stale-retained, inferred, ambiguous, unattributed, weak-identity, completed, tombstoned, or already-terminating sessions carry no action-bearing handles.
- Extend the existing `ProcessRefreshExecutor` result path so its single `ProcessObserver` transports move-only root capabilities into classification and performs each later descendant-observation pass required by an active transaction. Do not instantiate a second observer, clone capabilities, or let `ProcessTerminator` fall back to PID signaling; newly admitted descendants receive safe capabilities from that same observer before they can enter an execution plan.
- Define `SelectedBuildTerminationAuthorization` and `ScopeTerminationAuthorization`. `BuildMonitor` alone creates the selected authorization from one current authority-bearing session and the scope authorization from one exact, all-actionable frozen set; UI code can retain and submit either aggregate but cannot inspect, synthesize, decompose, combine, or subset its authority.
- `BuildMonitor` owns the complete `BuildTerminationTransaction`: freeze authorization and scope into the appropriate aggregate, allocate one transaction identity, transition sessions to `Terminating`, and fan out owned targets to `OwnedRunProcessActor` and external targets to `ProcessTerminator`. It maps each backend request identity/token to the transaction, fans both result streams into one bounded completion policy, and completes only after every semantic target is terminal or the transaction deadline expires. Frozen evidence remains owned after observer-cache eviction.
- Extend Phase 11's `TerminationExecutionPlan`, `TerminationOutcomeSummary`, and `TerminationTargetOutcome` rather than defining parallel plan/result concepts. Each planned target and returned target outcome carries a private semantic target identity suitable for `BuildSessionId` reconciliation; vector order and PID are never correlation keys.
- At execution, re-read the process table and require every frozen identity/scope condition to remain valid. Refresh descendants between bounded passes; admit a newly spawned descendant only while its complete validated parent chain reaches a still-live frozen root. Never first admit a process after its root is gone.
- Exclude Cargo Port, shell/LLM ancestors, persistent `sccache`/`rust-cache`, separate nested sessions, scope-divergent descendants, and compiler units known only by target-directory heuristics. Signal admitted leaves before roots and keep tracking admitted descendants after root exit.
- Finish only after all frozen roots/admitted descendants are gone or a deadline returns partial failure. Distinguish already gone from gone after signaling, without claiming the signal caused an exit when observation cannot prove causation; report permission/signal errors, deadline, and survivors truthfully. Do not escalate automatically to `SIGKILL`. A failed surviving identity becomes retryable only after a new fresh observation and later confirmation.
- **Own one `BuildTerminationLifecycleRegistry` on `BuildMonitor`, keyed by `BuildSessionId` and stored outside `monitor_snapshot`.** `record_classification` (`src/build_monitor/poll.rs:42-56`) builds a fresh `MonitorData` every poll cycle and drops the prior one, so a lifecycle transition written into the snapshot would be erased within ~500 ms and again on `replace_scope`. The registry preserves active and failed transactions across classification replacement; evicts a terminal entry only when its replacement build appears, its covered roots change, or monitoring turns off; and keeps retry unavailable until a new actionable observation appears. Phases 13 and 14 store gone-after-signal tombstones only here; `aggregate.rs` may project registry state but never store a second copy.
- **Fold `live_session_ids` into the registry's key set, or delete it.** It is a `BTreeSet<BuildSessionId>` derived from the snapshot's rows and cleared alongside them (`src/build_monitor/poll.rs:50-56, :68, :90`), reachable only through a `#[cfg(test)]` accessor, with no production reader and no other phase claiming it — a second answer to "which sessions are live" that this phase either adopts as the one answer or removes.
- **Carry the registry to the renderer in this phase, not in a later one.** `OutputPresentation::derive` takes only `&MonitorSnapshot` plus owned-run state (`src/tui/app/mod.rs:1207-1213, 1221-1229`), and Phase 9's single-presentation constraint forbids the pane reading a second source. Change `derive` to take the registry and join lifecycle into a named presentation-layer value on `MonitorColumn` or an equivalent borrowed type in `presentation.rs`; do not attach persistent lifecycle to replaceable `MonitorSessionRow`. This phase draws no markers; it makes `Terminating` reachable and provable end to end, and Phases 13 and 14 add the marker text.

**Files:**

- `src/process_termination/transaction.rs` — descendant admission, leaf-first signaling, deadline, and truthful result model.
- `src/process_termination/mod.rs` — extend Phase 11 plans/outcomes with semantic target correlation and transaction integration.
- `src/process_observation/mod.rs` — expose semantic actionable-versus-observed-only external termination support without exposing adapter internals.
- `src/process_observation/executor.rs` — return move-only root capabilities and run descendant observation passes through the existing sole observer.
- `src/build_monitor/session.rs` — actionable/observed-only session types and termination lifecycle states.
- `src/build_monitor/termination.rs` — frozen authorization, transaction ownership, and reconciliation.
- `src/build_monitor/mod.rs` — hold `BuildTerminationLifecycleRegistry` and expose request construction only from current actionable sessions.
- `src/build_monitor/snapshot.rs` — read the overlay where the snapshot is rebuilt.
- `src/build_monitor/poll.rs` — preserve the overlay across `record_classification` and `replace_scope`; retire or adopt `live_session_ids`.
- `src/tui/app/async_tasks/poll.rs` — reconcile correlated termination results without event-loop blocking.
- `src/tui/background.rs` — expose authority-bearing worker submission/results after the startup handshake.
- `src/tui/terminal/event_loop.rs` — wake for termination results, or route them through an existing selectable App channel.
- `src/tui/process_refresh.rs` — join observer-minted external support and actor-issued owned tokens into classification reconciliation.
- `src/tui/app/mod.rs` — change `OutputPresentation::derive` to take `BuildTerminationLifecycleRegistry`.
- `src/tui/panes/output/presentation.rs` — expose per-session lifecycle on `MonitorSessionRow`.

**Constraints from prior phases:** Phase 6 keys confirmed exact scope/session/activity association by `ProcessIncarnation`; Phase 8 owns fresh/stale lifecycle; Phase 9 owns the single-presentation constraint this phase's `derive` change must not break. A same-PID exec transition invalidates classification and actionability, while opaque authorization retains the current strong `ProcessIdentity` needed for immediate revalidation. Phase 11 owns the actor, the platform adapters, and the terminator worker; this phase constructs plans and reconciles results but never signals directly and never holds a raw capability. Owned sessions carry only `OwnedRunTerminationToken` outside `OwnedRunProcessActor`; external sessions carry `ExternalProcessTerminationCapability`, whose process-control data remains private. `ProcessTerminator` returns external outcomes on its correlated result channel, while owned outcomes and child completion arrive as FIFO `OwnedRunEvent` values already reconciled by `App::poll_example_msgs`. Linux may expose identity-bound `pidfd` signaling; macOS and any host without a demonstrated identity-bound adapter remain observed-only. No UI may synthesize or decompose opaque action authority.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-port`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-port`, and `bash ~/.claude/scripts/delegate/verify.sh lint cargo-port` are green; deterministic tests prove owned/external actionability always carries authority rather than an ID, observed-only adapters never become external authority, capabilities reach roots and newly admitted descendants through the existing sole observer, and only `BuildMonitor` constructs opaque selected/scope authorization aggregates. Mixed owned/external transactions prove one transaction identity, cross-channel fan-in, semantic per-target correlation independent of vector order/PID, the resolved concurrency policy, and one bounded terminal result. Tests also prove immediate strong-identity revalidation before signaling, validated descendant admission with no post-root first admission, leaf-before-root order, continued tracking after root exit, the exclusion list, no automatic `SIGKILL`, and truthful already-gone/gone-after-signal/partial-failure outcomes. The registry gets its own cases: a `Terminating` transition survives a full `record_classification` cycle and a `replace_scope`, each active/failed/terminal eviction rule is explicit, and lifecycle is **observable through `OutputPresentation`** without being stored on `MonitorSessionRow`. Actionability and visible staleness agree across all seven snapshot variants. This phase also removes the `#[cfg(test)]` gates from the two root-identity accessors it is the first production reader of — `SessionRootObservation::root_identity` (`src/build_monitor/session.rs:404`) and `BuildSession::root_identity` (`:465`) — which pre-signal revalidation needs and no earlier phase reads.

### Phase 13 — Selected-build termination interaction · status: todo

#### Work Order

**Goal:** From an actionable selected Output column, `Alt-k` (`Option-K` on macOS labels) opens a modal confirmation and safely terminates that entire root build.

**Pending decision: Modal precedence and confirm-key semantics are changes to the shared confirm modal, not new wiring**

Actual problem:
This phase's Spec states two properties of the confirmation modal as if they were behavior to add. Both are already implemented differently in shipped code, and changing either changes Esc and key handling for the five `ConfirmAction` variants that exist today — `Clean`, `CleanGroup`, `KillTarget`, `PauseLintProject`, `PauseAllLints`.

What exists now:
- **Precedence is inverted.** `handle_app_surface_key` runs `classify_output_cancel_preflight` / `dispatch_output_cancel_preflight` at `src/tui/input/dispatch.rs:148-152`, **before** `handle_confirm_key` at `:153`. With Output focused, the owned column selected, and the owned run running, Esc stops the run and leaves the confirm open. "Modal consumes input before Output cancellation, globals, copy, or navigation" is a reordering of shipped dispatch, not a property to wire up.
- **`n`/Esc is not how the shared handler works.** `handle_confirm_key` (`dispatch.rs:611-659`) calls `take_confirm()` unconditionally, so *any* key that is not `y` cancels. "All other keys do nothing" is a semantics change to that shared modal.
- A third shared-subsystem change this phase had grown is now gone: extending `OutputPresentation` to carry lifecycle state moved to Phase 12, which changes `derive` and proves the channel works before any marker is drawn.

What should change:
- Either scope both changes to this phase's own `ConfirmAction` variant, leaving the other five untouched, or change the shared modal for all of them and state the blast radius in the Spec.
- If the shared change is chosen, consider splitting a "modal-precedence and confirm semantics" phase ahead of this one so each safety boundary is reviewable on its own — the same split already taken for termination authority, which became Phases 11 and 12.

Recommendation:
Make the shared change and split it into its own phase before this one. Two Esc behaviors in one modal layer is the condition that produced the inversion in the first place, and a destructive action is the wrong place to discover that the modal underneath it behaves differently than the other five.

**Spec:**

- The selected compiler/activity row identifies cursor location only; selected-build termination always targets the owning root Cargo invocation.
- Expose the action only when the selected column's session comes from a snapshot whose `MonitorSnapshot::actionability()` is `MonitorDataActionability::Actionable` **and** that session carries authority-bearing `BuildTerminationAuthority::Owned` or `BuildTerminationAuthority::External`. Phase 12 resolves the single-derivation pending decision before this phase. **Availability is column identity, not row kind.** Phase 10 shipped `owned_column_selection` (`src/tui/panes/output/selection.rs:710-732`) deliberately treating the whole column as the unit, so Esc stops the owned run from any row in it — headers, activity rows, and captured-output rows alike. State this action's rule the same way: any cursor position inside an eligible session's column may invoke it, so a captured-output cursor does not get Esc-stop without Alt-K. Unattributed, observed-only, completed, killed, failed-unrefreshed, and terminating sessions cannot.
- **Resolve the target session by identity, not by cursor index.** Phase 10 already shipped most of this: the cursor stores `OutputCursorColumn::Session(BuildSessionId)` and resolves through `column_index_of` (`src/tui/panes/output/selection.rs:772-779`), and `reconcile_cursor` runs from the render body (`src/tui/panes/output/render.rs:51`) before any action reads the cursor (`src/tui/panes/output/pane.rs:239-241`). What remains for this phase is the refusal rule: between a poll result landing and the next frame the retained `BuildSessionId` still describes the previous snapshot, so look the session up against the current snapshot at action time and treat a mismatch as a refusal, never a different process.
- **The target resolver is a separate derivation from the motion resolver.** Do not reach the kill target through `OutputCursor::selected_column` — it deliberately maps `Absent` and `UnattributedSection` to `columns.first()` (`selection.rs:604-610`) so vertical motion always has a body to walk. Reusing it would make Alt-K on an unattributed row kill the first column's build.
- Ask `BuildMonitor` to construct one opaque `SelectedBuildTerminationAuthorization` for the selected root. Confirmation shows operative command, checkout, PID, start age, and current observed compiler-child count as separate display data while retaining that aggregate; UI code must not rebuild authority from `BuildSessionId`, scope, root identity, PID, or the display data. The child count comes from `MonitorSessionRow::compile_activities()`, which Phase 9 added to the stored snapshot.
- Confirmation is modal and consumes input before Output cancellation, globals, copy, or navigation: `y` submits the frozen request; `n` or `Esc` cancels; all other keys do nothing. **Both halves of that sentence contradict shipped code** — see the modal-precedence pending decision above for the exact call sites and the blast radius across the five existing `ConfirmAction` variants. Do not implement this bullet until that decision is resolved.
- Before signaling, Phase 12's submitted selected authorization requires the frozen session identity and scope still match a fresh observation. Exit becomes an already-gone toast, scope/identity mismatch rejects the request, and no replacement process at the PID is touched.
- Render `Terminating` until the correlated Phase 12 transaction completes. Retain a selected-build gone-after-signal tombstone until a new build replaces it, scope changes, or monitoring toggles off; do not label an external process “killed” when only disappearance after a signal is observed. On errors/deadline/survivors render a visible partial failure; enable retry only after a new fresh actionable snapshot and confirmation.
- Preserve existing `Esc` owned-run stop behavior outside the modal and when monitoring is off.

**Files:**

- `src/tui/app/confirm_action.rs` — selected-build confirmation payload retaining `SelectedBuildTerminationAuthorization` plus separate display data.
- `src/tui/app/mod.rs` — construct/submit requests and reconcile toasts/state.
- `src/tui/input/dispatch.rs` — modal priority and `y`/`n`/Esc handling.
- `src/tui/integration/framework_keymap/output_pane.rs` — availability and dispatch for `alt-k`.
- `src/tui/panes/output/pane.rs` — selected session/row lookup, exposed as a **named answer type computed by the pane**, not as the cursor enum. Phase 10's F008 split the cursor's column into four variants — `OutputCursorColumn::{Absent, UnattributedSection, OwnedCapturedOutput, Session(BuildSessionId)}` (`src/tui/panes/output/selection.rs:220-232`) — and that type cannot answer the termination question: two variants are not targets at all, and `OwnedCapturedOutput` names the owned column without carrying its session (recovered through `OwnedPinPresence` plus `column_produced_by`, `selection.rs:611-628, 764-769`). Exposing it would force every caller to redo that resolution. It is also invisible from here: `src/tui/panes/output/mod.rs:22-34` re-exports `OutputPane`, `OwnedColumnSelection`, `VisualSelectionPermission`, `ColumnSelection`, and `CapturedOutputRow` — every cursor type is `pub(super)` and unreachable from `src/tui/app/mod.rs` and `src/tui/input/dispatch.rs`. Add a pane method returning `SelectedBuildTerminationSelection::{NoBuildSelected, Build(BuildSessionId)}`, mirroring the shipped `OutputPane::owned_column_selection` (`pane.rs:244-262`); this keeps the "no `Option<&BuildSessionId>`" intent while stating the domain meaning of absence. Do not widen any cursor type's visibility.
- `src/tui/panes/output/monitor_render.rs` — terminating, gone-after-signal, already-gone, and partial-failure markers. All column, header, and indicator drawing lives here (`render_column` at `:689`, the monitor indicator at `:444-494`); `render.rs:51-60` only reconciles the cursor and delegates, so these markers do not belong there.
- `src/tui/render.rs` — modal confirmation and status/toast presentation.

**Constraints from prior phases:** The selectable set is Phase 8's scope-filtered presentation snapshot, which `BuildMonitor` narrowed once; never re-filter it and never widen back to `BuildClassification::build_sessions()`. Use Phase 10's framework action and platform label; retain and submit only Phase 12's `SelectedBuildTerminationAuthorization` without reconstructing or substituting a scope-wide aggregate. **Phase 10 left both kill actions deliberately inert in exactly two places, and this phase must split each:** `dispatch_output_action` no-ops both kill arms at `src/tui/input/dispatch.rs:900-902`, and `Shortcuts::state` returns `Disabled` for both in a single match arm at `src/tui/integration/framework_keymap/output_pane.rs:68-75`. Wire `KillSelectedBuild` here and leave `KillScopedBuilds` disabled for Phase 14. Phase 5 scope changes make an open request invalid, Phase 9 owns exec-sensitive selection identity/fallback, and the legacy Running Targets termination capability remains unrelated.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-port`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-port`, and `bash ~/.claude/scripts/delegate/verify.sh lint cargo-port` are green; tests prove modal precedence, selected-authorization retention without UI authority reconstruction, exact frozen scope/session, root-not-row semantics, stale/inferred/ambiguous/weak-state unavailability, PID exit/reuse safety, truthful terminating/already-gone/gone-after-signal/partial-failure states, fresh-observation retry, and no effect on unrelated builds/cache daemons. **Modal-precedence cases go in the dispatch harness, not the pane's interaction tests.** `src/tui/input/dispatch.rs`'s test module already builds an app with `crate::tui::test_support::make_app` plus a `staged_output()` fixture staging one owned and one external column over a resolved scope — that is the only harness that can reach the modal layer. Phase 10's `src/tui/panes/output/interaction_tests.rs` constructs `OutputPresentation` directly and cannot see dispatch ordering at all.

### Phase 14 — Scope-wide termination and end-to-end verification · status: todo

#### Work Order

**Goal:** `Alt-Shift-k` (`Option-Shift-K` on macOS labels) safely terminates exactly the live actionable roots in the selected scope, with final end-to-end proof of the complete feature.

**Spec:**

- Scope-wide termination requires a nonempty live root set and refuses to open if any represented live root is observed-only; “all” never means a silent actionable subset.
- **The kill set is not built by a second filtering pass.** It is exactly the rows the stored snapshot already holds: take `MonitorSnapshot::actionability()`, and proceed only when it is `MonitorDataActionability::Actionable`; that `MonitorData`'s `MonitorSessionRow` set — `MonitorData::session_rows()` alone — *is* the kill set. Phase 8 narrowed to the monitor scope once, in `src/build_monitor/poll.rs`, and nothing outside that scope is representable in the snapshot — completed runs and duplicate/nested references to the same root are already absent, so re-excluding them is dead specification. Unattributed activities are the exception and must be excluded explicitly: Phase 9 put the scope-narrowed unattributed set inside the same `MonitorData` (`src/build_monitor/snapshot.rs:179`, filled at `src/build_monitor/poll.rs:140-151`), and `UnattributedScopeEvidence::Unplaceable` deliberately survives into *every* scope (`poll.rs:220`) because a compiler process whose working directory could not be read cannot be proven outside the checkout. Those rows have no root PID to signal and no `BuildSessionId` to authorize against, so they count toward neither the "nonempty live root set" that makes scope-wide termination available nor the observed-only all-or-refuse check — a scope showing only unattributed rows offers no scope-wide kill. A *live* owned run outside the selected scope must stay in the set (see Constraints). The one exclusion that survives is the completed-producer pin, and that is a Phase 9 presentation concept, not a snapshot member — apply it at the display layer, not to the kill set.
- Refuse the action outright when `ActiveMonitorState::build_scope_actionability()` is `BuildScopeActionability::NotActionable`. This runs `tui`-side, so the full `MonitorScopeKey` is available; the snapshot it authorizes is keyed by `BuildScopeKey`, so join through `build_scope_actionability()` rather than comparing the two keys directly or calling `BuildScopeKey::from(&monitor_scope_key)`. Phase 7 established that method (`src/tui/compile_visibility/mod.rs:129`, with the free function it delegates to at `:316`) as the one entry point through which a monitor scope reaches build classification, precisely so the five resolution states are not restated downstream; a direct `From` call bypasses it and would let this phase build a destructive set from a scope that never passed the actionability check.
- Ask `BuildMonitor` to create one opaque `ScopeTerminationAuthorization` from the current exact all-actionable root set. Confirmation names the selected scope and deduplicated `BuildSessionId` set as separate display data while retaining that aggregate; UI code never reassembles, combines, or subsets authority from displayed scope/session IDs. Invalidate the authorization on `CoveredScopeRoots` inequality — not on any revision change. A project-list or metadata revision bump that leaves the covered roots identical is exactly the case Phase 8 made common (it republishes the snapshot as `PendingWithRetained`, which stays `Actionable`); voiding authority on it would make scope-wide termination unusable during ordinary background refreshes.
- A build starting after confirmation is never added to destructive authority; leave it running and report that a newer build was not included. A root that already exited is `gone`, never replaced by a new process at the PID.
- Submit the opaque exact frozen-set authorization through Phase 12's one bounded transaction. Render per-root and aggregate terminating, gone-after-signal, already-gone, survivor, and error outcomes truthfully. Retain gone-after-signal tombstones until scope change, replacement build, or monitor off.
- Complete focused automated coverage for simultaneous debug/release, linked/group worktree scope, unique versus ambiguous cache-wrapper attribution, owned target plus external build/Cargo-lock wait, selected versus scope kill, and disabled polling.
- Perform live verification on macOS where available: debug and release in one checkout; builds in two linked worktrees with group versus checkout scope; `RUSTC_WRAPPER=rust-cache`/`sccache`; owned target launch beside an external build including Cargo-lock wait; selected kill preserving unrelated builds/cache daemon; scope kill affecting only deduplicated scoped roots; toggle off ceasing compile work. If an external platform adapter is intentionally unavailable, verify observed-only rendering/action unavailability rather than using unsafe fallback.

**Files:**

- `src/tui/app/confirm_action.rs` — scoped confirmation payload retaining `ScopeTerminationAuthorization` plus exact-set display data.
- `src/tui/app/mod.rs` — create, submit, and reconcile scope-wide requests.
- `src/tui/input/dispatch.rs` — modal input for the scope action.
- `src/tui/integration/framework_keymap/output_pane.rs` — availability/dispatch for `alt-shift-k`.
- `src/build_monitor/aggregate.rs` — project per-root and aggregate results from `BuildTerminationLifecycleRegistry`; do not store tombstones here.
- `src/build_monitor/mod.rs` — invalidate retained authorization at the same owner that applies `replace_scope`, based on `CoveredScopeRoots` inequality.
- `src/tui/app/mod.rs` and `src/tui/process_refresh.rs` — route every scope replacement and classification landing through that shared invalidation owner.
- `src/tui/panes/output/monitor_render.rs` — scoped transaction outcome rendering. Same reason as Phase 12: `render.rs:51-60` only reconciles the cursor and delegates, and every column/header/indicator draw is in `monitor_render.rs`.
- `src/tui/render.rs` — scope confirmation and completion toast.

**Constraints from prior phases:** **The scope-wide kill set is exactly Phase 8's scope-filtered presentation snapshot — the same value the Output pane renders.** Do not derive an independent set by re-applying `MonitorScopeKey` or `BuildScopeKey` to `BuildClassification::build_sessions()`: `BuildMonitor` is the single filtering site precisely so the set the user is looking at and the set this phase terminates cannot disagree, and a second derivation reintroduces that disagreement in the one place where it destroys work. One consequence follows mechanically from that equality and must be asserted rather than left implicit: an out-of-scope owned run survives Phase 8's filter, so it is inside this kill set. Add an acceptance-gate case proving scope-wide termination signals a live owned run whose checkout root is outside the current `BuildScopeKey` — it is in the set because the Output pane is showing it, and "stop everything shown" that silently spares one column would be the worse surprise. Phase 13 establishes modal selected termination; reuse its input path while retaining Phase 12's distinct `ScopeTerminationAuthorization` for set-wide authority — including whatever Phase 13's modal-precedence pending decision settles, since this phase inherits that modal rather than defining its own. **Phase 10 left `KillScopedBuilds` deliberately inert in two places and Phase 13 leaves it that way; this phase is what enables it:** the no-op arm in `dispatch_output_action` (`src/tui/input/dispatch.rs:900-902`) and the `Disabled` arm in `Shortcuts::state` (`src/tui/integration/framework_keymap/output_pane.rs:68-75`). The keymap fixture needs no edit — `tests/assets/default-keymap.toml:70-71` already pins `kill_scoped_builds = "alt-shift-k"` and `kill_selected_build = "alt-k"` from Phase 10; the acceptance gate re-asserts it rather than producing it. Phase 12's aggregate/transaction semantics plus Phase 5 exact scope generations bind the frozen set; UI code never reconstructs, combines, or subsets it. Phase 4 pinned owned output remains outside unrelated scope-wide authority.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-port`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-port`, and `bash ~/.claude/scripts/delegate/verify.sh lint cargo-port` are green; automated tests prove all-or-refuse actionability, opaque scope-authorization retention without UI reconstruction/combination/subsetting, root deduplication, new-build exclusion, gone-versus-reused identity, pinned-owned exclusion, modal priority, truthful exact scoped outcomes, compile-monitor-off quiescence, and the generated keymap fixture matches Phase 10 defaults; the live verification matrix is completed with any platform-observed-only limitation recorded without weakening safety.
