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

### Phase 14 — Selected-build termination interaction · status: todo

#### Work Order

**Goal:** From an actionable selected Output column, `Alt-k` (`Option-K` on macOS labels) opens a modal confirmation and safely terminates that entire root build.

**Pending decision: Termination transaction timeout policy**

Actual problem:
Phase 12 deliberately accepts a caller-supplied deadline, but no production interaction chooses how long a selected or scope-wide transaction may wait for descendant refresh, external outcome, or owned child reap.

What exists now:
- `BuildMonitor::submit_selected_termination` and `submit_scope_termination` take a raw `Instant`; focused Phase 12 tests use five seconds, and deadline expiry becomes a persistent retry-unavailable terminal result.

What should change:
- Phase 14 should introduce semantic `BuildTerminationDeadline` construction backed by one named timeout policy, and Phase 15 should reuse it unchanged.

Recommendation:
Use a five-second timeout. It matches the proven Phase 12 transaction tests, bounds slow descendant/reap waits without immediate false failure, and gives both actions one policy.

**Spec:**

- The selected compiler/activity row identifies cursor location only; selected-build termination always targets the owning root Cargo invocation.
- Add read-only `BuildMonitor::selected_termination_availability(&BuildSessionId) -> SelectedBuildTerminationAvailability`, with `SelectedBuildTerminationAvailability::{Available, SnapshotNotActionable, SessionNotActionable, Busy}`. It checks the same actionable snapshot and authority map as `selected_termination_authorization` without removing the move-only authority; `Shortcuts::state(&App)` uses this query, and confirmation later consumes authority through the existing constructor. UI code never inspects `BuildTerminationAuthority`. **Availability is column identity, not row kind.** Phase 10 shipped `owned_column_selection` (`src/tui/panes/output/selection.rs:710-732`) deliberately treating the whole column as the unit, so Esc stops the owned run from any row in it — headers, activity rows, and captured-output rows alike. Any cursor position inside an eligible session's column may invoke Alt-K. Unattributed, observed-only, completed, killed, failed-unrefreshed, and terminating sessions return an unavailable semantic variant.
- **Resolve the target session by identity, not by cursor index.** Phase 10 already shipped most of this: the cursor stores `OutputCursorColumn::Session(BuildSessionId)` and resolves through `column_index_of` (`src/tui/panes/output/selection.rs:772-779`), and `reconcile_cursor` runs from the render body (`src/tui/panes/output/render.rs:51`) before any action reads the cursor (`src/tui/panes/output/pane.rs:239-241`). What remains for this phase is the refusal rule: between a poll result landing and the next frame the retained `BuildSessionId` still describes the previous snapshot, so look the session up against the current snapshot at action time and treat a mismatch as a refusal, never a different process.
- **The target resolver is a separate derivation from the motion resolver.** Do not reach the kill target through `OutputCursor::selected_column` — it deliberately maps `Absent` and `UnattributedSection` to `columns.first()` (`selection.rs:604-610`) so vertical motion always has a body to walk. Reusing it would make Alt-K on an unattributed row kill the first column's build.
- Derive `SelectedBuildTerminationSelection::{NoBuildSelected, SelectedBuild(SelectedBuildTerminationDisplayTarget)}` from the reconciled cursor and the same `MonitorColumn` used by rendering. `SelectedBuildTerminationDisplayTarget` identifies the selected session for display without promising actionability; it owns the `BuildSessionId` plus `SelectedBuildTerminationConfirmationDisplay` for operative command, checkout, PID, start age, and current compiler-child count. Move the authoritative header/display derivation into `presentation.rs` so the modal and column header reuse one answer; do not expose cursor enums or return a bare `Option<T>`. Authorization remains represented only by `SelectedBuildTerminationAuthorization`.
- After resolving a `SelectedBuildTerminationDisplayTarget` and checking read-only availability, ask `BuildMonitor` to construct one opaque `SelectedBuildTerminationAuthorization` for the selected root before opening the modal. Move that authorization into the `ConfirmAction`; cancellation or replacement drops it, so a later attempt waits for fresh classification to republish authority. Confirmation shows operative command, checkout, PID, start age, and current observed compiler-child count as separate display data while retaining that aggregate; UI code must not rebuild authority from `BuildSessionId`, scope, root identity, PID, or the display data. The child count comes from `MonitorSessionRow::compile_activities()`, which Phase 9 added to the stored snapshot.
- Add selected-build termination as a `Ready` `ConfirmAction` inside Phase 13's shared `confirmation_modal_state`. Extend only the exhaustive matches in `execute_confirmed_action`, `finish_clean_metadata_confirmation`, `confirm_action_body`, and `render_confirm_popup`; the existing modal-first dispatch supplies the semantics unchanged. `y` moves the frozen request out through `accept_confirmation`, `n` or `Esc` cancels, and every other key leaves it open. Do not add a termination-specific input path or readiness slot. If `Background::available_process_terminator()` is `Starting` or `Unavailable`, restore the same authority-bearing action to a `Ready` modal rather than dropping or reconstructing it.
- Before signaling, Phase 12's submitted selected authorization requires the frozen session identity and scope still match a fresh observation. Exit becomes an already-gone toast, scope/identity mismatch rejects the request, and no replacement process at the PID is touched.
- Start the transaction in one ordered App operation. Borrow the worker through `Background::available_process_terminator()` and surface `Starting`/`Unavailable` without consuming authorization; construct the semantic `BuildTerminationDeadline` chosen above; handle `BuildTerminationSubmission::{Busy, Refused(SnapshotNotActionable | SelectedScopeChanged | SelectedSessionChanged), IdentityExhausted}` explicitly. After `Submitted(transaction_id)`, immediately call `BuildMonitor::submit_owned_termination_targets(transaction_id, |token| Inflight::submit_owned_run_termination(token))`; never leave owned targets at `ReadyToSubmit` across an event-loop turn.
- Add `BuildTerminationCompletionTransition::{NoCompletion, Completed(BuildTerminationTransactionCompletion)}` as the return from owned-target submission and every external-outcome, owned-outcome, owned-`Finished`, observation, and expiry reconciliation path. `submit_owned_termination_targets` can complete synchronously when actor submission is refused, so its App caller must consume that transition before returning to the event loop. The completion value carries the semantic transaction/session/target details needed for a one-shot toast while `BuildTerminationLifecycleRegistry` remains the sole persistent row-independent store. Presentation names projections of `BuildTerminationTerminalRecord`; App must not poll persistent records or maintain a second "already toasted" ledger.
- Render `Terminating` until the correlated Phase 12 transaction completes. Retain a selected-build gone-after-signal tombstone until a new build replaces it, scope changes, or monitoring toggles off; do not label an external process “killed” when only disappearance after a signal is observed. On errors/deadline/survivors render a visible partial failure; enable retry only after a new fresh actionable snapshot and confirmation.
- Preserve existing `Esc` owned-run stop behavior outside the modal and when monitoring is off.

**Files:**

- `src/tui/app/confirm_action.rs` — selected-build confirmation payload retaining `SelectedBuildTerminationAuthorization` plus separate display data, and the new exhaustive action arm.
- `src/tui/app/mod.rs` — construct/submit selected requests in one ordered operation through Phase 13's modal owner, restore an authority-bearing action when the worker is not ready, extend `finish_clean_metadata_confirmation`, and consume one-shot completion transitions for toasts.
- `src/tui/background.rs` — deterministic test controls for process-termination worker `Starting`, `Available`, and `Unavailable` readiness.
- `src/tui/app/async_tasks/poll.rs` and `src/tui/process_refresh.rs` — consume synchronous and asynchronous completion transitions at the existing reconciliation call sites.
- `src/build_monitor/mod.rs` — read-only selected availability, semantic deadline submission, owned-token fan-out, and completion-transition returns.
- `src/build_monitor/termination/authority.rs` — `SelectedBuildTerminationAvailability` and non-consuming authority checks.
- `src/build_monitor/termination/transaction.rs` — `BuildTerminationDeadline`, explicit submission results, and one-shot `BuildTerminationCompletionTransition`.
- `src/build_monitor/termination/lifecycle.rs` and `src/build_monitor/termination/mod.rs` — expose semantic completion/detail projections without a second result store.
- `src/tui/input/dispatch.rs` — extend `execute_confirmed_action` for the selected-build action through the shared modal-first handler; keep generic modal input unchanged.
- `src/tui/integration/framework_keymap/output_pane.rs` — availability and dispatch for `alt-k`.
- `src/tui/panes/output/pane.rs` — selected display-target lookup as `SelectedBuildTerminationSelection::{NoBuildSelected, SelectedBuild(SelectedBuildTerminationDisplayTarget)}`, computed from the reconciled cursor plus presentation column. Phase 10's cursor remains private and is never exposed or reused as the action answer.
- `src/tui/panes/output/presentation.rs` — `SelectedBuildTerminationDisplayTarget`, shared confirmation/header display derivation, and nameable per-row/terminal completion projections.
- `src/tui/panes/output/monitor_render.rs` — terminating, gone-after-signal, already-gone, and partial-failure markers. All column, header, and indicator drawing lives here (`render_column` at `:694-702`, the monitor indicator at `:444-494`); `render.rs:51-60` only reconciles the cursor and delegates, so these markers do not belong there.
- `src/tui/render.rs` — extend `confirm_action_body` and `render_confirm_popup` for selected-build confirmation, plus status/toast presentation.

**Constraints from prior phases:** Phase 13 owns `confirmation_modal_state: ConfirmationModalState`; `open_confirmation` atomically installs an action and readiness, `accept_confirmation` atomically closes a `Ready` modal and returns its move-only action, and modal-first keyboard/mouse semantics already cover every action. This phase adds one `Ready` action and extends the four exhaustive matches without reopening generic modal behavior. A selected authorization is consumed into the action when the modal opens—not after `y`—and must never be cloned or reconstructed. The selectable set is Phase 8's scope-filtered presentation snapshot, which `BuildMonitor` narrowed once; never re-filter it and never widen back to `BuildClassification::build_sessions()`. Use Phase 10's framework action and platform label; retain and submit only Phase 12's `SelectedBuildTerminationAuthorization` without reconstructing or substituting a scope-wide aggregate. Phase 12 submission revalidates the exact selected scope and session and returns semantic busy/refusal/identity-exhaustion results before lifecycle mutation. A successful owned `Sent` outcome proves only signal delivery and leaves the target `Terminating`; only the correlated `OwnedRunEvent::Finished` proves reap and permits gone-after-signal presentation, while a missing `Finished` ends at the deadline as retry-unavailable. Terminal records remain in `BuildTerminationLifecycleRegistry` independently of rows and are already projected through `OutputPresentation`; do not create a second store or infer a one-shot toast by polling them. **Phase 10 left both kill actions deliberately inert in exactly two places, and this phase must split each:** `dispatch_output_action` no-ops both kill arms at `src/tui/input/dispatch.rs:906-908`, and `Shortcuts::state` returns `Disabled` for both in a single match arm at `src/tui/integration/framework_keymap/output_pane.rs:68-75`. Wire `KillSelectedBuild` here and leave `KillScopedBuilds` disabled for Phase 15. Phase 5 scope changes make an open request invalid, Phase 9 owns exec-sensitive selection identity/fallback, and the legacy Running Targets termination capability remains unrelated.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-port`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-port`, and `bash ~/.claude/scripts/delegate/verify.sh lint cargo-port` are green; tests prove read-only shortcut availability never consumes authority, modal precedence/readiness, selected-authorization retention without UI authority reconstruction, exact frozen scope/session, root-not-row semantics, stale/inferred/ambiguous/weak-state unavailability, explicit worker-starting/unavailable and every submission-result path, immediate owned-token fan-out after submission, PID exit/reuse safety, truthful terminating/already-gone/gone-after-signal/partial-failure states, one completion transition/toast per transaction including synchronous owned-submission refusal, fresh-observation retry, and no effect on unrelated builds/cache daemons. Signal acceptance alone draws no gone-after-signal marker; missing owned `Finished` reaches the deadline/retry-unavailable state. **Modal-precedence cases go in the dispatch harness, not the pane's interaction tests.** `src/tui/input/dispatch.rs`'s test module already builds an app with `crate::tui::test_support::make_app` plus `staged_output()`, but that helper supplies presentation data only; extend the harness with matching live `Inflight` state, actionable owned/external monitor authority, deterministic `Background` worker readiness, and teardown that drops cloned runtime handles before the owning App fixture. Phase 10's `src/tui/panes/output/interaction_tests.rs` constructs `OutputPresentation` directly and cannot see dispatch ordering at all.

### Phase 15 — Scope-wide termination and end-to-end verification · status: todo

#### Work Order

**Goal:** `Alt-Shift-k` (`Option-Shift-K` on macOS labels) safely terminates exactly the live actionable roots in the selected scope, with final end-to-end proof of the complete feature.

**Pending decision: Toggle-off behavior for an already-confirmed termination transaction**

Actual problem:
Phase 15 says toggle-off stops termination observation, but `BuildMonitor::switch_off` preserves an active transaction and Phase 12 tests require it to continue through completion or deadline.

What exists now:
- Toggle-off clears monitor presentation/authority while the submitted termination transaction remains active and keeps its deadline/observation demand.

What should change:
- Choose whether a confirmed transaction continues independently of compile-monitor visibility or toggle-off becomes a new cancellation operation with explicit terminal semantics.

Recommendation:
Let an already-confirmed transaction continue to completion or deadline. Toggle-off should stop new compile-monitor polling and actions, not abandon an authorized destructive transaction mid-flight.

**Pending decision: Safe macOS runtime configuration isolation for the live gate**

Actual problem:
Production `config_path()` uses `dirs::config_dir()`, so XDG variables do not isolate config on macOS; the Phase 13 smoke attempt therefore reached the real user configuration and was stopped without exercising destructive behavior.

What exists now:
- The only configuration-root override is test-only, while the live gate needs isolated config, keymap, and theme paths.

What should change:
- Either add one supported runtime configuration-root override covering config, keymap, and themes, or require the live matrix to run under a disposable macOS user environment.

Recommendation:
Add a supported `CARGO_PORT_CONFIG_DIR` override for the whole Cargo Port configuration root, then run the live matrix with a temporary directory.

**Spec:**

- Scope-wide termination requires a nonempty live root set and refuses to open if any represented live root is observed-only; “all” never means a silent actionable subset.
- Add read-only `BuildMonitor::scope_termination_availability() -> ScopeTerminationAvailability`, with `ScopeTerminationAvailability::{Available, SnapshotNotActionable, ScopeNotFullyActionable, Busy}`. It verifies the nonempty exact row set and authority completeness without consuming any move-only authority; `Shortcuts::state(&App)` uses it, while confirmation later calls the existing consuming constructor.
- **The kill set is not built by a second filtering pass.** It is exactly the rows the stored snapshot already holds: take `MonitorSnapshot::actionability()`, and proceed only when it is `MonitorDataActionability::Actionable`; that `MonitorData`'s `MonitorSessionRow` set — `MonitorData::session_rows()` alone — *is* the kill set. Phase 8 narrowed to the monitor scope once, in `src/build_monitor/poll.rs`, and nothing outside that scope is representable in the snapshot — completed runs and duplicate/nested references to the same root are already absent, so re-excluding them is dead specification. Unattributed activities are the exception and must be excluded explicitly: Phase 9 put the scope-narrowed unattributed set inside the same `MonitorData` (`src/build_monitor/snapshot.rs:179`, filled at `src/build_monitor/poll.rs:208-225`), and `UnattributedScopeEvidence::Unplaceable` deliberately survives into *every* scope (`poll.rs:288`) because a compiler process whose working directory could not be read cannot be proven outside the checkout. Those rows have no root PID to signal and no `BuildSessionId` to authorize against, so they count toward neither the "nonempty live root set" that makes scope-wide termination available nor the observed-only all-or-refuse check — a scope showing only unattributed rows offers no scope-wide kill. A *live* owned run outside the selected scope must stay in the set (see Constraints). The one exclusion that survives is the completed-producer pin, and that is a Phase 9 presentation concept, not a snapshot member — apply it at the display layer, not to the kill set.
- Refuse the action outright when `ActiveMonitorState::build_scope_actionability()` is `BuildScopeActionability::NotActionable`. This runs `tui`-side, so the full `MonitorScopeKey` is available; the snapshot it authorizes is keyed by `BuildScopeKey`, so join through `build_scope_actionability()` rather than comparing the two keys directly or calling `BuildScopeKey::from(&monitor_scope_key)`. Phase 7 established that method (`src/tui/compile_visibility/mod.rs:129`, with the free function it delegates to at `:316`) as the one entry point through which a monitor scope reaches build classification, precisely so the five resolution states are not restated downstream; a direct `From` call bypasses it and would let this phase build a destructive set from a scope that never passed the actionability check.
- Ask `BuildMonitor` to create one opaque `ScopeTerminationAuthorization` from the current exact all-actionable root set. `ScopeBuildTerminationConfirmationDisplay` freezes the selected-row display identity and exact target summaries separately from that aggregate. Construct it through `ScopeBuildTerminationConfirmationDisplayConstruction::{Display(ScopeBuildTerminationConfirmationDisplay), SelectedRowDisplayUnavailable}`, converting `ProjectList::display_path_for_row`'s bare `Option<DisplayPath>` at the App boundary; do not retain optional display data. UI code never reassembles, combines, or subsets authority from displayed scope/session IDs. Do not add eager invalidation routing: the Phase 12 submission owner already accepts revision churn when `CoveredScopeRoots` remain equal and returns `ScopeRootsChanged` or `SnapshotNotActionable` before mutation when they do not. A project-list or metadata revision bump that leaves the covered roots identical republishes `PendingWithRetained` and remains actionable.
- Add scope-wide termination as a `Ready` `ConfirmAction` by extending only `execute_confirmed_action`, `finish_clean_metadata_confirmation`, `confirm_action_body`, and `render_confirm_popup`; reuse Phase 13's modal-first keyboard and mouse behavior unchanged.
- A build starting after confirmation is never added to destructive authority; leave it running and report that a newer build was not included. A root that already exited is `gone`, never replaced by a new process at the PID.
- Submit the opaque exact frozen-set authorization through Phase 14's ordered transaction-start operation and semantic deadline policy. Reuse its worker-starting/unavailable handling, every `BuildTerminationSubmission` result, immediate owned-token fan-out, and one-shot `BuildTerminationCompletionTransition`; do not introduce a scope-specific transaction path or completion ledger.
- Render per-root and aggregate terminating, gone-after-signal, already-gone, survivor, and error outcomes from `BuildTerminationLifecycleRegistry` and `BuildTerminationTerminalRecord::{session_completion, aggregate_completion, target_results}` projections. Group the row-independent records by transaction identity for one scope result, and retain tombstones until scope change, replacement build, or monitor off without storing another aggregate.
- Complete focused automated coverage for simultaneous debug/release, linked/group worktree scope, unique versus ambiguous cache-wrapper attribution, owned target plus external build/Cargo-lock wait, selected versus scope kill, and disabled polling.
- Perform live verification on macOS where available: debug and release in one checkout; builds in two linked worktrees with group versus checkout scope; `RUSTC_WRAPPER=rust-cache`/`sccache`; owned target launch beside an external build including Cargo-lock wait; selected kill preserving unrelated builds/cache daemon; scope kill affecting only deduplicated scoped roots; toggle off ceasing compile work. If an external platform adapter is intentionally unavailable, verify observed-only rendering/action unavailability rather than using unsafe fallback.

**Files:**

- `src/tui/app/confirm_action.rs` — scoped confirmation payload retaining `ScopeTerminationAuthorization` plus `ScopeBuildTerminationConfirmationDisplay`, and the new exhaustive action arm.
- `src/tui/app/mod.rs` — create, submit, and reconcile scope-wide requests.
- `src/tui/input/dispatch.rs` — extend `execute_confirmed_action` for the scope action; keep generic modal input unchanged.
- `src/tui/integration/framework_keymap/output_pane.rs` — availability/dispatch for `alt-shift-k`.
- `src/build_monitor/mod.rs` — read-only scope availability and reuse of the existing authorization/submission/completion owner.
- `src/build_monitor/termination/authority.rs` and `src/build_monitor/termination/transaction.rs` — `ScopeTerminationAvailability` plus the existing equal-roots submission revalidation; no eager invalidation path.
- `src/build_monitor/termination/lifecycle.rs` and `src/build_monitor/termination/mod.rs` — expose existing transaction/session/target completion projections needed by scoped presentation.
- `src/tui/panes/output/presentation.rs` — `ScopeBuildTerminationConfirmationDisplay`, its semantic construction result, and named per-root/aggregate projections from registry records grouped by transaction identity; never store a second aggregate or optional display data.
- `src/tui/panes/output/monitor_render.rs` — scoped transaction outcome rendering. Same reason as Phase 12: `render.rs:51-60` only reconciles the cursor and delegates, and every column/header/indicator draw is in `monitor_render.rs`.
- `src/tui/render.rs` — extend `confirm_action_body` and `render_confirm_popup` for scope confirmation, plus the completion toast.

**Constraints from prior phases:** **The scope-wide kill set is exactly Phase 8's scope-filtered presentation snapshot — the same value the Output pane renders.** Do not derive an independent set by re-applying `MonitorScopeKey` or `BuildScopeKey` to `BuildClassification::build_sessions()`: `BuildMonitor` is the single filtering site precisely so the set the user is looking at and the set this phase terminates cannot disagree, and a second derivation reintroduces that disagreement in the one place where it destroys work. One consequence follows mechanically from that equality and must be asserted rather than left implicit: an out-of-scope owned run survives Phase 8's filter, so it is inside this kill set. Add an acceptance-gate case proving scope-wide termination signals a live owned run whose checkout root is outside the current `BuildScopeKey` — it is in the set because the Output pane is showing it, and "stop everything shown" that silently spares one column would be the worse surprise. Phase 12's `ScopeTerminationAuthorization` freezes the exact all-actionable set and submission revalidates only actionable state plus equal `CoveredScopeRoots`; do not add a second invalidation owner. Phase 12 terminal records already carry per-target, session, and aggregate completion independently of rows. A successful owned `Sent` outcome leaves the target `Terminating`; only the correlated `OwnedRunEvent::Finished` proves reap, and a missing `Finished` reaches deadline/retry-unavailable. Phase 13 owns `confirmation_modal_state` and generic modal-first input; Phase 14 adds the selected `Ready` action, the semantic deadline, worker/submission sequence, immediate owned-token fan-out including synchronous completion, one-shot completion transition, and actionable dispatch fixture. This phase adds only the scope action and reuses those mechanisms unchanged while retaining the distinct scope authorization. **Phase 10 left `KillScopedBuilds` deliberately inert in two places and Phase 14 leaves it that way; this phase is what enables it:** the no-op arm in `dispatch_output_action` (`src/tui/input/dispatch.rs:906-908`) and the `Disabled` arm in `Shortcuts::state` (`src/tui/integration/framework_keymap/output_pane.rs:68-75`). The keymap fixture needs no edit — `tests/assets/default-keymap.toml:70-71` already pins `kill_scoped_builds = "alt-shift-k"` and `kill_selected_build = "alt-k"` from Phase 10; the acceptance gate re-asserts it rather than producing it. Phase 5 exact scope generations bind the frozen set; UI code never reconstructs, combines, or subsets it. Phase 4 completed-producer pinned output remains outside scope-wide authority, while a live owned monitor row remains inside.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-port`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-port`, and `bash ~/.claude/scripts/delegate/verify.sh lint cargo-port` are green; automated tests prove read-only scope availability never consumes authority, all-or-refuse actionability, opaque scope-authorization retention without UI reconstruction/combination/subsetting, equal-root revision churn acceptance and incompatible-root refusal at submission, root deduplication, new-build exclusion, gone-versus-reused identity, modal priority, immediate owned-token fan-out, one completion transition per transaction, and truthful exact scoped outcomes. Signal acceptance alone draws no gone-after-signal marker, while missing owned `Finished` reaches deadline/retry-unavailable. Completed-producer pinned output is excluded while a live out-of-scope owned run shown as a monitor row remains included. Toggle off ceases compile-monitor polling and termination observation without signaling any build. The generated keymap fixture matches Phase 10 defaults; the live verification matrix is completed with any platform-observed-only limitation recorded without weakening safety.
