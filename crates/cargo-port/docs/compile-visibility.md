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

#### As-built

The App-owned `CargoWorkspaceIndex` indexes canonical checkout roots separately from Cargo workspace roots, workspace members, package and target-source identities, and live target directories against accepted metadata and visible-project content revisions. It rebuilds only when those revisions change and exposes `WorkspaceIndexRefreshState::{Current, RetainedLastAccepted, Uninitialized}` without implying that `cargo metadata --no-deps` contains complete dependency records. Running Targets consumes the shared index at its existing one-second cadence, evaluates readiness before filesystem attribution, uses retained accepted metadata after refresh failure, and permits visible-target fallback only while uninitialized. Exact source and owner collisions retain every candidate, and cross-workspace ambiguity remains omitted rather than fabricated as unique ownership.

**Files:**
- `src/project/cargo/workspace_index.rs` — revision-keyed canonical workspace, ownership, target-source, and target-directory index.
- `src/project/cargo/metadata_store.rs` — accepted metadata inputs and revisions.
- `src/tui/app/mod.rs` — App ownership of the shared index.
- `src/tui/running_targets/state.rs` — Running Targets queries over shared indexed identities.

**Gotchas:** Selection-only navigation does not advance `ProjectListRevision`; it tracks visible ownership content, and canonical source/owner collisions can remain ambiguous across workspaces or packages.

### Phase 2 — Strong process observation foundation · status: done

#### As-built

`ProcessObserver` owns one private `sysinfo::System` and returns immutable full or targeted snapshots with `ObservedProcessIdentity::{Strong(ProcessIdentity), Insufficient(InsufficientProcessIdentity)}`, exec-sensitive `ProcessIncarnation`, named `ProcessFieldObservation<T>` states, and validated depth-capped ancestry. Executable, argv, and cwd are coherently sampled; parent evidence is bracketed and reconciled separately, and a fingerprint change invalidates executable, command, cwd, classification, scope, ancestry, and termination evidence at the same PID. Its incarnation cache retains new, changed, or unclassified Cargo/compiler/wrapper candidates and evicts entries absent from a successfully reconciled full snapshot. Observation exposes no signaling capability.

**Files:**
- `src/process_observation/mod.rs` — host-only observer and immutable snapshot API.
- `src/process_observation/identity.rs` — lifetime identities, platform creation tokens, and exec incarnations.
- `src/process_observation/snapshot.rs` — named field evidence, validated ancestry, refresh inputs, and caches.

**Binds later work:** `ProcessObserver` supplies immutable strong/insufficient identity evidence and validated ancestry but never signals; termination authorization retains strong `ProcessIdentity`, while weak identity is non-actionable; execution re-reads identities and never trusts a replacement at the same PID (Phases 12 and 14–15).

**Gotchas:** macOS creation identity uses native `proc_pid_rusage` monotonic start data bracketed by processkit lifetime anchors; later insufficient chronological evidence blocks strong admission, and full-refresh eviction requires direct sampling plus a final direct lookup for omitted cached PIDs.

### Phase 3 — Shared refresh scheduling and Running Targets migration · status: done

#### As-built

The App-owned `ProcessRefreshExecutor` uses one dedicated worker containing the sole `ProcessObserver`, coalesces typed consumer demand, and returns correlated `CompletedProcessRefreshExecution` values that keep elapsed observer time beside immutable snapshots while failures remain distinct from completed-empty refreshes. Running Targets consumes those snapshots at its existing one-second cadence and preserves current, retained-last-accepted, uninitialized, ambiguity-omission, startup-suppression, and cadence-before-attribution behavior. A private `RunningMetricsProcessTable` preserves identity-bound CPU and memory continuity, while `RunningTargetTerminationCapability` performs the legacy stop only after strong-identity revalidation and remains separate from observation. The terminal event loop waits on the minimum animation and process deadlines, and process refresh is not modeled as animation.

**Files:**
- `src/process_observation/executor.rs` — dedicated worker execution over exactly one observer.
- `src/tui/process_refresh.rs` — correlated App-side demand, result, and deadline integration.
- `src/tui/running_targets/app_tick.rs` — one-second Running refresh requests and readiness ordering.
- `src/tui/running_targets/termination.rs` — identity-revalidated legacy Running Targets signaling.
- `src/tui/terminal/frame_metrics.rs` — observer timing separate from rendering.

**Binds later work:** the existing App-owned `ProcessRefreshExecutor` has one dedicated worker and exactly one `ProcessObserver`; extend that result path for move-only root capabilities and descendant passes—no second observer or cloned capabilities (`src/process_observation/executor.rs`, `src/tui/process_refresh.rs`) (Phase 12).

**Gotchas:** Identity discovery and repeated executable/argv/cwd sampling use short-lived systems because refreshing them on the long-lived metrics table disturbs CPU timing; raw process-table refreshes are counted at the deepest private boundary.

### Phase 4 — Correlated Cargo Port-owned runs · status: done

#### As-built

`OwnedRun` solely owns semantic queued, starting, running, stopping, retained, gone-after-signal, and failed lifecycle state, with monotonic `OwnedRunId`, state-specific verified root identity and opaque `OwnedProcessGroupTerminationCapability`, outcomes, and captured output. `CargoPackageInvocation::{WorkspaceDefault, Package(String)}` represents launch arguments without optional-package semantics, and failure to establish strong root identity terminates and reaps the isolated process group without exposing PID-only authority. `OwnedRunOutputIdentity` keeps retained output tied to its producer while a newer run is queued or starting. Owned output, progress, start, and completion are correlated so late messages cannot mutate another run.

**Files:**
- `src/tui/state/inflight.rs` — owned-run aggregate, IDs, lifecycle, output identity, and authority-bearing states.
- `src/tui/messages.rs` — correlated owned-run events.
- `src/tui/app/async_tasks/poll.rs` — matching-run reconciliation.
- `src/tui/terminal/processes.rs` — identity-verified isolated-group launch and tagged output capture.
- `src/tui/panes/output/mod.rs` — live and retained output views over the aggregate.

**Binds later work:** owned authority is correlated by `OwnedRunId`; owned outcomes and child completion arrive as FIFO `OwnedRunEvent`s reconciled by `App::poll_example_msgs`; existing owned-run Esc stop survives outside confirmations/off state (Phases 12–14). Completed/pinned owned output remains display-only and outside unrelated scope-wide authority, while a live owned column actually present in the snapshot remains in “stop everything shown” (Phase 15).

**Gotchas:** Closing Output during `Stopping` clears captured lines but preserves the active slot until its matching completion; late pipe flushes require final reconciliation to place exactly one killed marker after output and progress.

**Ruled out:** PID-only owned-run stop authority; deriving retained-output ownership from the current lifecycle ID.

### Phase 5 — Typed monitor scope and state shell · status: done

#### As-built

`CompileVisibilityState::{Off, On(ActiveMonitorState)}` makes disabled state own no monitor data; enabled state carries selected-row identity, exhaustive scope resolution, and monotonic `CompileMonitorGeneration`. `MonitorScopeKey` combines `MonitorSelectedRowIdentity`, row kind, sorted canonical checkout/workspace roots, metadata revision, and project-list revision, while `MonitorScopeResolution::{Ready, EmptyNonRust, PendingIndex, AmbiguousOwnership, UnresolvedPath}` permits only `Ready` to become `MonitorScopeActionability::Actionable`. Every `VisibleRow` kind resolves exhaustively, with linked checkouts isolated, worktree groups expanded across represented visible roots, metadata-proven nested workspaces honored, and non-Rust rows empty. `MonitorScopeResolutionRevision` keeps non-ready selections distinct, and selection, project-list, or accepted-metadata changes replace generation through their separate refresh triggers. The shared `WorkspaceIndexReadiness::{Current, RetainedLastAccepted, Uninitialized}` adapter serves both compile visibility and Running Targets.

**Files:**
- `src/tui/compile_visibility/mod.rs` — off/on state shell, generation lifecycle, and scope actionability boundary.
- `src/tui/compile_visibility/scope.rs` — row-aware keys, resolution revisions, and exhaustive row mapping.
- `src/tui/workspace_index.rs` — shared readiness and index-query adapter.
- `src/tui/app/async_tasks/metadata_handlers.rs` — accepted-metadata publication followed by scope refresh.
- `src/tui/app/async_tasks/poll.rs` — project-list revision-triggered scope refresh.

**Binds later work:** `MonitorScopeKey` is the TUI-side, row-kind-aware key; `BuildScopeKey` is its roots/revisions projection, and open requests invalidate on scope change (Phases 14–15). Frozen scope authority is bound by exact scope generations/`CoveredScopeRoots`; invalidate on covered-root inequality, not a revision-only bump, and do not compare the two scope-key types directly (Phase 15).

**Gotchas:** Scope refresh has three triggers—selection, background project-list revision, and accepted metadata—and macOS scope fixtures must canonicalize `/tmp`/`/var` through `/private`; `represented_visible_checkout_roots` intentionally excludes deleted entries even though group “live” counts include them.

**Ruled out:** widening `VisibleRow` outside `tui`; copying the workspace index into every worker request; treating generation or revision changes alone as covered-root inequality.

### Phase 6 — Cargo build and compiler classification · status: done

#### As-built

`BuildScopeKey` is the sorted canonical roots plus accepted-metadata/project-list revisions projection of `MonitorScopeKey`, produced only by `impl From<&MonitorScopeKey>` through the TUI actionability boundary; row identity stays TUI-local. Opaque `BuildSessionId(ProcessIncarnation)` and `CompileActivityId(ProcessIncarnation)` key immutable session/activity records, while named scope, compiler, target-directory, profile, package, and ambiguity states keep unresolved evidence non-actionable. `pub(super) fn classify(input: BuildClassificationInput<'_>) -> BuildClassification` is a free pure function; `BuildClassifier` owns the dependency-manifest cache, first-seen ledger, and LRU-capped `BuildDirectoryLedger` and mutates them only outside that call. Two-pass Cargo recognition and compiler association normalize proxies/plugins/nested Cargo, promote compiler-bearing non-build or unrecognized roots for the incarnation lifetime, resolve candidates system-wide before scope filtering, and retain ambiguous units without fabricated uniqueness. Dependency manifests resolve by request/response across cycles, preserving distinct not-yet-looked-up and absent states, and the immutable snapshot remains the sole source of cached candidate incarnations. `SessionTargetDirectory::{Argument, Indexed, Unobservable}` records derivation; unobservable sessions do not participate in output matching, and ordering uses first-seen cycle then total-ordered incarnation.

**Files:**
- `src/build_monitor/classify.rs` — pure recognition, scope resolution, compiler association, and unit derivation.
- `src/build_monitor/build_classifier.rs` — dependency, first-seen, promotion/profile, and build-directory support state.
- `src/build_monitor/session.rs` — opaque session identity, scope attribution, target-directory, profile, and owned-root evidence.
- `src/build_monitor/activity.rs` — opaque activity identity, compiler attribution, and crate/package presentation records.
- `src/build_monitor/scope.rs` — `BuildScopeKey` and build-side actionability.
- `src/process_observation/snapshot.rs` — immutable candidate evidence and shared `LinkerRecognition`.
- `src/project/cargo/workspace_index.rs` — ambiguity-preserving canonical package and target ownership queries.

**Binds later work:** exact scope/session/activity association is keyed by exec-sensitive `ProcessIncarnation`, not PID; same-PID exec invalidates classification/actionability while authorization retains strong `ProcessIdentity` for revalidation (Phase 12). Never widen a selected/scoped action back to `BuildClassification::build_sessions()`; compiler units known only through target-directory heuristics are not signalable (Phases 12 and 14–15).

**Gotchas:** `BuildDirectoryLedger` prevents profile reversion between compiler children; unreadable cwd degrades scope to `Unresolved` instead of dropping the candidate; `LinkerRecognition` preserves dotted linker names such as `ld64.lld`; Windows linker discovery handles rustc `@response-file` arguments; a newly learned build directory applies one cycle later.

**Ruled out:** widening `VisibleRow` into `build_monitor`; resolving configured aliases inside pure classification; scope-filtered output uniqueness; a second candidate-cache owner; an inherent mutable classifier method; row identity in `BuildScopeKey`.

### Phase 7 — Worker-side classification integration · status: done

#### As-built

- The App-owned `ProcessRefreshExecutor` runs one dedicated worker that solely owns `ProcessObserver`, `BuildClassifier`, and the classifier support ledgers; `src/tui/process_refresh.rs` owns shared deadline, dispatch, correlation, and independent consumer reconciliation.
- Requests carry immutable `Arc<CargoWorkspaceIndex>`, scope, generation, owned-root evidence, and `CompileClassificationDemand`; `WorkspaceIndexReadiness` remains `Clone, Copy` by borrowing `&Arc<CargoWorkspaceIndex>`, and cancellation after observation skips classification while preserving any due Running result.
- `CompileClassificationExecution` carries semantic completed, failed, cancelled, and not-requested outcomes inside the still-boxed completed refresh payload. `SessionScope::{Resolved { method, root }, Unresolved}` makes a resolved attribution without a root unrepresentable, while `BuildSession` carries its operative command, root observation, profile, and first-sighting instant.
- Owned-run admission uses internal `OwnedRunIdAllocation::{Allocated, Exhausted}` plus public-call-site `OwnedRunLaunchAdmission::{Queued, AlreadyActive, IdentitiesExhausted}`; nonzero IDs are process-lifetime unique and launch rejection is explicit. The 5,000-process classification fixture records about 74–89 ms per cycle, keeping classification off the 15 ms event-loop budget.

**Files:**
- `src/tui/background.rs` — dedicated observation/classification worker and its sole mutable owners.
- `src/tui/process_refresh.rs` — neutral shared refresh adapter and consumer reconciliation.
- `src/build_monitor/{build_classifier,execution,session}.rs` — classifier state, semantic outcomes, exhaustive scope, and presentation/termination session evidence.
- `src/tui/state/inflight.rs` — unique owned-run allocation and immutable owned-root evidence.
- `src/tui/workspace_index.rs` — borrowed `Arc<CargoWorkspaceIndex>` readiness views.

**Binds later work:** `ActiveMonitorState::build_scope_actionability()` is the sole TUI-to-classification entry point (`src/tui/compile_visibility/mod.rs:129`, delegated free function `:316`) and exhaustively preserves the five resolution states; `BuildScopeActionability::NotActionable` refuses scope-wide kill, and direct `BuildScopeKey::from(&monitor_scope_key)` bypass is forbidden (Phase 15).

**Gotchas:** Root `ProcessIdentity` is reachable through `BuildSession`, never bare `BuildSessionId`; `first_seen` remains a cycle counter while `FirstSighting { first_seen, first_observed_at }` supplies elapsed-time data.

**Ruled out:** A second observer/classifier or synchronous production path; placing `BuildClassifier` in neutral `process_observation`; owning `Arc` inside `WorkspaceIndexReadiness`; resetting or saturating owned-run IDs.

### Phase 8 — Conditional monitor polling and lifecycle · status: done

#### As-built

- `BuildMonitor` stores lifecycle and presentation results while the dedicated worker retains the sole classifier. Scope filtering occurs once in `src/build_monitor/poll.rs`: resolved external sessions must share a covered checkout root, while an explicitly owned session survives outside the selected scope.
- `ActiveMonitorState` owns a 500 ms `MonitorRefreshSchedule`; the executor receives only its derived dispatch deadline, coalesces simultaneous Running and compile demand, and accepts results only for the matching generation and `BuildScopeKey`. `CompileVisibilityState::Off` owns no schedule, while boxed `On(ActiveMonitorState)` carries snapshot and cadence state.
- `MonitorSnapshot::{Off, Pending, PendingWithRetained, Fresh, Stale, Unavailable}` distinguishes disabled, awaiting, retained, current, aged, and unavailable data; unchanged covered roots retain the display across generation changes, while failure ages data to non-actionable and then unavailable. `LiveTargetDirectoryRevision` participates directly in `BuildScopeKey` equality.
- `MonitorSessionRow` stores owned association and evidence-backed compiling, build-script, linking, or active state; owned lock-wait evidence comes only from semantic owned lifecycle state, and session association matches the verified exec-sensitive root once without copying captured output.

**Files:**
- `src/build_monitor/poll.rs` — the single scope filter, generation correlation, snapshot replacement, and failure aging.
- `src/build_monitor/snapshot.rs` — monitor/data/session presentation states and observation instants.
- `src/tui/compile_visibility/{mod,constants}.rs` — enabled lifecycle, generation, and 500 ms scheduling intent.
- `src/process_observation/executor.rs` and `src/tui/process_refresh.rs` — derived deadlines, coalesced demand, cancellation, and independent reconciliation.

**Binds later work:** `BuildMonitor` performs scope filtering once in `src/build_monitor/poll.rs`; the stored scope-filtered presentation snapshot is the selectable/kill set and must never be re-filtered (Phases 14–15). `Fresh` and live `PendingWithRetained` are actionable; stale, stale-retained, pending-without-data, off, and unavailable are not (Phase 12). `MonitorData::session_rows()` is the exact root kill set; completed and duplicate/nested roots are absent, but an out-of-scope live owned run retained in the displayed snapshot remains included (Phase 15). A metadata/project-list revision bump with unchanged covered roots republishes actionable `PendingWithRetained` data (Phase 15).

**Gotchas:** Scope replacement retention compares `covered_scope_roots()` directly; revision or selected-row changes alone do not justify blanking the display. Host-wide classification remains unfiltered so the single `BuildMonitor` narrowing pass can preserve owned runs.

**Ruled out:** Filtering in both the worker and `BuildMonitor`; an optional payload on `Pending`; a parallel disabled schedule state; inferring lock wait or running-target state without evidence.

### Phase 9 — Output monitor presentation and columns · status: done

#### As-built

- `OutputPresentation::{Hidden, OwnedOnly, Monitor, MonitorWithOwned}` is the single value used by layout, visibility, focus, copy, hit testing, action labels, and rendering. It derives distinct monitor-off, pending, retained, stale-retained, stale, unavailable, and named non-actionable scope displays.
- `OwnedRunOutputStateRef::{Absent, Retained { producer, title, lines }}` and `OwnedOutputPresentation` make visible output without a producer unrepresentable; completed output pins by producer identity independently of the current owned-run lifecycle.
- Each stable exec-sensitive session is a column with command/selectors, path, profile, root PID, elapsed time, state, and selectable activity rows. `MonitorSessionRow` retains attributed activities and `MonitorData` retains the scope-narrowed unattributed set created by the single filter in `record_classification`.
- Typed cursor targets and identity-based reconciliation preserve column/activity selection across refreshes with deterministic fallback; external rows remain live samples, and owned captured output follows its activity rows behind a non-selectable separator.

**Files:**
- `src/tui/panes/output/{presentation,monitor_render,selection,pane,hit_map}.rs` — unified presentation, columns, typed navigation state, rendering, and hits.
- `src/build_monitor/snapshot.rs` — per-session activities, unattributed evidence, staleness-preserving retention, and production read API.
- `src/build_monitor/poll.rs` — population and one-time narrowing of sessions and unattributed activities.
- `src/tui/state/inflight.rs` — semantic retained-output producer state and zero-copy lines.

**Binds later work:** The pane reads one `OutputPresentation`; lifecycle is joined through `OutputPresentation::derive` into a named `MonitorColumn` presentation value, never read as a second source or persisted on replaceable `MonitorSessionRow` (`src/tui/app/mod.rs:1207-1213,1221-1229`) (Phase 12). `MonitorSessionRow::compile_activities()` supplies selected-build child count, and selection identity/fallback remains exec-sensitive (Phase 14). The snapshot’s scope-narrowed unattributed set lives at `src/build_monitor/snapshot.rs:179`, populated by `src/build_monitor/poll.rs:140-151`; `UnattributedScopeEvidence::Unplaceable` survives every scope (`poll.rs:220`) but has no root PID/`BuildSessionId`, so it neither enables nor blocks all-or-refuse scope kill. Completed-producer pinning is presentation-only, not a snapshot kill member (Phase 15).

**Gotchas:** `owned_captured_output_height` duplicates the draw-path arithmetic and must change with any rows drawn above captured output. `UnattributedScopeEvidence::WorkingDirectory` is the narrowing evidence; output directory cannot prove scope membership for an unattributed activity.

**Ruled out:** Re-filtering at render time; reading back through `BuildClassification`; flattening semantic producer/title states to `Option`; treating completed producer pins or unattributed rows as termination roots.

### Phase 10 — Monitor navigation, toggle, and owned-output coexistence · status: done (8a3a1af)

#### As-built

- Framework `Shift-C` makes compile visibility reachable and preserves the off-by-default, non-persistent lifecycle. Output navigation traverses typed columns and complete column bodies, skips the output separator, windows horizontally, and keeps one selected-column vertical offset that resets when column identity changes.
- Tab and Shift-Tab preflight Output before pane snaking; mouse hits, Home/End/Page movement, normalized Vim movement, activity-row copy, and owned-output-only visual selection all consume the same `OutputPresentation`.
- Owned stop, close, copy, and Esc behavior is gated by owned column identity even on headers; hidden visual selections intercept copy/Esc only while their owned region is selected. `App::split_output_for_navigation` supplies the presentation without conflicting App borrows.
- Portable key display canonicalizes uppercase Alt characters as shifted bindings, labels them as Option on macOS and Alt elsewhere, and leaves `display_short` unchanged.

**Files:**
- `src/tui/panes/output/{pane,selection,monitor_render}.rs` — motion, cursor identity, selected-column offset, and owned-column selection.
- `src/tui/input/dispatch.rs` — action-aware navigation and Esc/selection preflight.
- `src/tui/integration/framework_keymap/{navigation,output_pane,app_context}.rs` — toggle, Output actions, and pane-boundary behavior.
- `src/tui/keymap/{actions,canonical,constants}.rs` and `tests/assets/default-keymap.toml` — portable actions, labels, and defaults.

**Binds later work:** Column identity, not row kind, governs owned/selected actions; shipped `owned_column_selection` treats the whole owned column as selected (`src/tui/panes/output/selection.rs:710-732`) (Phase 14). Cursor identity is `OutputCursorColumn::{Absent, UnattributedSection, OwnedCapturedOutput, Session(BuildSessionId)}` (`selection.rs:220-232`); `Session` resolves through `column_index_of` (`:772-779`), render reconciles before actions (`render.rs:51`; `pane.rs:239-241`), and `selected_column` is motion-only because it maps absent/unattributed to `columns.first()` (`selection.rs:604-610`). Cursor types remain `pub(super)`; `OwnedCapturedOutput` resolves through `OwnedPinPresence`/`column_produced_by` (`:611-628,764-769`), and the pane exposes a named `SelectedBuildTerminationSelection` answer instead (`pane.rs:244-262`; exports `output/mod.rs:22-34`) (Phase 14). Framework actions/portable labels already exist; both kill actions are intentionally inert at `dispatch_output_action` (`dispatch.rs:900-902`) and `Shortcuts::state` (`output_pane.rs:68-75`): Phase 14 enables only `KillSelectedBuild`, Phase 15 enables `KillScopedBuilds`. The fixture already pins `kill_scoped_builds = "alt-shift-k"` and `kill_selected_build = "alt-k"` (`tests/assets/default-keymap.toml:70-71`) (Phases 14–15).

**Gotchas:** `owned_captured_output_height` mirrors drawing arithmetic; the three Esc branches have different guards; monitor enablement must pass through `toggle_compile_visibility`, and `workspace_index_readiness` remains on the input path to preserve scope-revision equality.

**Ruled out:** Raw `KeyCode` dispatch; row-kind-derived termination selection; exposing cursor internals as a destructive-action API; per-session scroll-offset maps.

### Phase 11 — Owned-run termination actor and platform capability foundation · status: done (`da3aa9e`)

#### As-built

- `OwnedRunProcessActor` solely owns the child wait and non-cloneable `OwnedProcessGroupTerminationCapability`, issues opaque run-bound `OwnedRunTerminationToken` authority, and admits one pending request through its serialized endpoint.
- Owned output, progress, termination outcomes, and completion share one ordered `OwnedRunEvent` channel; completion closes command admission, drains every accepted termination request, and publishes `Finished` last.
- `ProcessTerminator` owns external signaling on a dedicated worker and returns request-correlated outcomes. `ProcessObserver` supplies immutable evidence and private identity-bound external capabilities but exposes no signal API.
- Linux external termination uses identity-bound `pidfd`; macOS and hosts without a demonstrated safe adapter remain observed-only. External revalidation requires process identity plus executable-image continuity, so same-lifetime exec and PID reuse are refused.

**Files:**
- `src/tui/state/owned_run_process_actor.rs` and `src/tui/state/inflight.rs` — serialized owned authority, run-bound tokens, and ordered events.
- `src/process_termination/{mod,platform}.rs` — correlated worker API, identity-bound capability adapter, and signaling outcomes.
- `src/process_observation/{mod,identity}.rs` — immutable identity evidence and capability construction without signaling.
- `src/tui/{background,messages}.rs` and `src/tui/app/async_tasks/poll.rs` — off-event-loop execution and ordered result reconciliation.

**Binds later work:** Termination has two backends: `OwnedRunProcessActor` issues run-bound `OwnedRunTerminationToken` authority and admits one pending request; `ProcessTerminator` owns external signaling and returns correlated outcomes (Phase 12). External authority requires identity-bound `ExternalProcessTerminationCapability`; `PlatformTerminationCapabilityObservation::Available` alone may still be observed-only. Capability internals stay private, no raw capability escapes, and no PID fallback/direct UI signaling is allowed. Extend existing `TerminationExecutionPlan`, `TerminationOutcomeSummary`, and `TerminationTargetOutcome` with private semantic target identity rather than parallel models or vector/PID correlation. Linux may use identity-bound `pidfd`; macOS/hosts lacking a demonstrated adapter remain observed-only (Phase 12).

**Gotchas:** Actor ordering requires command-admission closure and accepted-request draining before `OwnedRunEvent::Finished`; `ProcessIdentity` alone does not authorize a same-lifetime exec transition.

**Ruled out:** Bare-PID or ambient-process-group signaling; raw capability escape; direct UI signaling; a parallel termination result model; automatic `SIGKILL` escalation.

### Phase 12 — Termination authorization, bounded transaction, and lifecycle registry · status: done

#### As-built

- `MonitorSnapshot::actionability()` derives destructive-action eligibility from the same seven-state display/staleness answer used by Output.
- `BuildMonitor` alone constructs opaque `SelectedBuildTerminationAuthorization` and exact-scope `ScopeTerminationAuthorization`, owns their move-only authority and the sole active transaction, and revalidates submissions before lifecycle mutation.
- `BuildTerminationTransaction` correlates owned and external targets by semantic identity, admits validated descendants in bounded passes, signals leaves before roots, and terminates through outcomes, correlated owned `Finished`, observation, or deadline expiry.
- Owned signal acceptance leaves a session `Terminating`; only the correlated `OwnedRunEvent::Finished` proves reap, while missing completion produces a retry-unavailable deadline result.
- `BuildTerminationLifecycleRegistry` survives valid classification/scope replacement, retains detailed terminal records outside snapshot rows, and projects lifecycle through `OutputPresentation` without storage on `MonitorSessionRow`.
- External capabilities and descendant refresh remain on the sole `ProcessObserver`; observed-only adapters never yield action authority, frozen evidence survives cache eviction, and `ProcessTerminator` has no PID fallback.

**Files:**
- `src/build_monitor/termination/{authority,lifecycle,observation,transaction}.rs` — opaque authority, registry, sole-observer integration, and mixed-backend transaction ownership.
- `src/build_monitor/{mod,poll,snapshot,execution}.rs` — actionability, registry preservation, submission revalidation, reconciliation, and lifecycle projection.
- `src/process_termination/{mod,platform,transaction}.rs` — semantic correlation, identity-bound execution, bounded descendant admission, exclusions, leaf-first planning, and truthful outcomes.
- `src/process_observation/{mod,executor,snapshot}.rs` — move-only actionable support and transaction refresh through the existing observer.
- `src/tui/` — worker/result routing, nonblocking reconciliation, disconnected-receiver removal, and row-independent lifecycle projection into the single Output presentation path.

**Binds later work:** Phase 14 consumes `SelectedBuildTerminationAuthorization`, semantic refusal/busy/identity-exhaustion results, and registry terminal records; Phase 15 consumes `ScopeTerminationAuthorization` and its `CoveredScopeRoots` equality rule; both use owned `Finished`, not signal acceptance, as gone-after-signal proof and add no second store.

**Gotchas:** Retained actionability requires prior displayed rows plus the current `BuildScopeKey`; same-PID exec invalidates identity; identity exhaustion is distinct from unavailable/busy/refusal; a disconnected result receiver must leave event-loop selection.

**Ruled out:** Bare-PID, ambient process-group, or direct UI signaling; exposed/cloned raw capabilities; automatic `SIGKILL`; PID/vector-position correlation; and any second observer, lifecycle store, or aggregate result ledger.

### Phase 13 — Shared confirmation modal state and input precedence · status: done

#### As-built

- `App::confirmation_modal_state` is the sole modal owner, using `ConfirmationModalState::{Closed, Open { action, readiness }}`, `ConfirmationReadiness::{Ready, VerifyingCleanMetadata(AbsolutePath)}`, and `ConfirmationAcceptance::{Closed, Verifying, Ready(ConfirmAction)}` without cloning the move-only action.
- `open_confirmation` atomically replaces the action and readiness; `accept_confirmation` closes and moves out only a `Ready` action, while a verifying acceptance preserves the open modal.
- Metadata completion changes readiness only when the current clean or clean-group action's primary path and the `VerifyingCleanMetadata` path both match exactly.
- An open modal handles keyboard input before Output cancellation, overlays, globals, copy, or navigation and blocks mouse input; accepted `y` behavior remains intact for `Clean`, `CleanGroup`, `KillTarget`, `PauseLintProject`, and `PauseAllLints`.

**Files:**
- `src/tui/app/confirm_action.rs` — modal lifecycle, readiness, acceptance, and action types.
- `src/tui/app/mod.rs` — App-owned modal operations and exact-path readiness reconciliation.
- `src/tui/app/construct.rs` — `confirmation_modal_state` initialization.
- `src/tui/state/scan.rs` — scan state without an independent confirmation-readiness slot.
- `src/tui/app/async_tasks/metadata_handlers.rs` — metadata-completion routing into App reconciliation.
- `src/tui/input/dispatch.rs` — modal-first keyboard behavior and mouse blocking.
- `src/tui/state/inflight.rs` — test-only live-owned-run fixture for input-precedence coverage.
- `src/tui/render.rs` — popup rendering from the App-owned action and readiness.

**Binds later work:** Termination actions open as `Ready` through the same owner and input path; later phases extend `execute_confirmed_action`, `finish_clean_metadata_confirmation`, `confirm_action_body`, and `render_confirm_popup` without adding another readiness owner.

**Gotchas:** Runtime-handle clones retaining supervisor senders must drop before App test teardown; on macOS, `dirs::config_dir()` ignores XDG isolation.

**Ruled out:** A second readiness owner or termination-specific input path; cloning or reconstructing a move-only action.

### Phase 14 — Selected-build termination interaction · status: done

#### As-built

- `Alt-K` resolves the exact selected root from its header, activity, or matching captured-output rows and moves one opaque `SelectedBuildTerminationAuthorization` into the shared `Ready` confirmation modal.
- `BuildTerminationDeadline::from_submission_time` applies the shared `BUILD_TERMINATION_TIMEOUT` five-second policy to selected submission.
- Submitted transactions immediately fan out owned tokens through `submit_owned_termination_targets`, whose `BuildTerminationCompletionTransition` also reports synchronous completion such as actor-submission refusal.
- Detailed lifecycle terminal records preserve target identity and already-gone, gone-after-signal, or incomplete results beside remaining live columns.
- The App submits selected requests in order, restores the same authority-bearing action when the worker is `Starting` or `Unavailable`, and emits selected-build-specific completion toast wording.

**Files:**
- `src/build_monitor/termination/{authority,lifecycle,transaction}.rs` — selected availability, semantic deadline, completion transition, and retained terminal records.
- `src/build_monitor/mod.rs` — selected submission, immediate owned-token fan-out, and reconciliation entry points.
- `src/tui/app/{mod,confirm_action}.rs`, `src/tui/background.rs`, and `src/tui/input/dispatch.rs` — authority-bearing modal action, worker-readiness restoration, ordered submission, and interaction dispatch.
- `src/tui/panes/output/{selection,pane,presentation,monitor_render,interaction_tests}.rs` — exact destructive selection, shared display derivation, terminal projections, and rendered lifecycle state.
- `src/tui/process_refresh.rs` and `src/tui/app/async_tasks/poll.rs` — synchronous and asynchronous completion-transition consumption.
- `src/tui/integration/framework_keymap/output_pane.rs` and `src/tui/render.rs` — `Alt-K` availability, confirmation body, popup, and selected completion presentation.

**Binds later work:** Phase 15 reuses `BuildTerminationDeadline::from_submission_time`, `BUILD_TERMINATION_TIMEOUT`, `BuildTerminationCompletionTransition`, immediate owned-token fan-out, worker-readiness restoration, and detailed terminal projections.

**Gotchas:**
- Captured-output destructive selection requires retained `OwnedColumnWitness` and current `OwnedPinPresence` to identify the same producer before action resolution.
- On macOS, live smoke cannot isolate production configuration, keymap, and theme paths from the user's Cargo Port state.

**Ruled out:** Destructive targeting through the cursor's motion fallback; reconstructing move-only authority in UI code; a second completion ledger.

### Phase 15 — Scope-wide termination and end-to-end verification · status: done

#### As-built

- `BuildMonitor::output_build_set_termination_availability() -> OutputBuildSetTerminationAvailability` derives a nonempty exact target set from actionable `MonitorData::session_rows()` and returns `Available`, `SnapshotNotActionable`, `BuildSetNotFullyActionable`, or `Busy`; any observed-only root makes the whole set unavailable. `OutputBuildSetTerminationAuthorization` freezes that set as opaque authority, while `OutputBuildSetTerminationConfirmationDisplayResolution::{Ready, SelectedRowUnavailable}` keeps `ProjectListRowDisplayPathResolution` context separate from exact target summaries; UI code cannot reconstruct, combine, or subset authority.
- `ConfirmAction::TerminateOutputBuildSet` and `BuildTerminationTransactionTargetSet::{SelectedBuild, OutputBuildSet}` share submission, immediate owned-token fan-out, `BuildTerminationDeadline`, and one-shot completion; later roots become `AdditionalBuildExclusion::{NoAdditionalBuilds, Excluded { count }}` rather than new targets. `BuildTerminationLifecycleRegistry` retains the confirmed root count and renders one aggregate followed by per-root outcomes even after individual records are evicted. Turning visibility off prevents new requests without signaling a build, while an authorized transaction retains `ProcessRefreshConsumerDemand::TerminationTransaction` through reconciliation or its five-second deadline.
- `CargoPortConfigurationPathResolution` preserves resolved, platform-unavailable, and invalid-empty-override states through startup and Cargo Port-owned constructors, then collapses only at external watcher boundaries. A nonempty `CARGO_PORT_CONFIG_DIR` places `config.toml`, `keymap.toml`, and `themes/` beneath one root; an empty value fails closed.
- **Files:** `src/build_monitor/termination/{authority,transaction,lifecycle}.rs` owns exact-set authority, transaction identity/exclusions, completion projections, and retained counts; `src/tui/app/{confirm_action,mod,construct}.rs` owns confirmation, shared submission, and semantic startup state; `src/tui/input/dispatch.rs` routes the action; `src/tui/panes/output/{presentation,monitor_render}.rs` builds confirmation views and grouped outcomes; `src/config.rs`, `src/tui/keymap/load.rs`, and `src/themes/paths.rs` share configuration-root resolution; `src/tui/state/{config,keymap}.rs` is the Cargo Port-owned constructor boundary where semantic path resolution collapses only inside `WatchedFile::new`.
- **Gotchas:** Unattributed rows never provide a signalable root and do not make the set nonempty, but an unplaceable compiler still remains visible in every scope. Completed-producer pinned output stays outside authority, while every live owned row shown in Output stays inside even when its checkout root is outside the selected scope. Test-only path overrides remain the outermost boundary so ambient environment variables cannot alter fixtures.
- **Ruled out:** Partial signaling of actionable rows from a mixed displayed set; independent scope re-filtering outside `BuildMonitor`; eager invalidation when revisions change but covered roots remain equal; treating an empty configuration-root override as unset; separate output-set transaction or aggregate storage.
