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
- **Invariants:** Compile visibility starts disabled, is not persisted, and while off owns no compile-specific polling, deadline, command parsing, classification, snapshot, tombstone, generation, or late-result acceptance; existing Running Targets retains its one-second behavior and owned-target Output behavior remains unchanged. One App-owned `ProcessObserver` performs at most one combined due refresh and one App-owned revision-keyed `CargoWorkspaceIndex` serves Running Targets and Build Monitor without launching Cargo when monitoring starts. The shared index explicitly reports `Current`, `RetainedLastAccepted`, or `Uninitialized`; consumers preserve the last accepted index on refresh failure, and only an uninitialized index may use a named fallback. `ProjectListRevision` changes only when visible ownership content changes; selected-row identity is separate monitor-scope input. Scope is a typed row-kind-aware `MonitorScopeKey` over sorted canonical checkout/workspace roots plus metadata/project-list revision; workspace members resolve to their owning workspace, groups differ from primary checkout rows, non-Rust scopes are empty, and a changed key makes old data immediately non-actionable. Build/session and row stability use strong `ProcessIdentity` plus exec-sensitive `ProcessIncarnation`, never a bare PID; weak, stale, inferred, ambiguous, or unattributed evidence is observed-only, and system-wide cache-daemon ambiguity is rendered once without guessing. Process observation and termination are separate: `ProcessObserver` produces immutable evidence and capabilities, while `ProcessTerminator` performs identity-revalidated signaling off the TUI event loop. External termination requires an identity-bound platform capability and opaque frozen scope/identity authorization; never signal an ambient process group, Cargo Port, shell/LLM ancestors, cache daemons, divergent nested sessions, or target-directory-only compiler matches. Selected-scope kill refuses partial actionability, never absorbs builds started after confirmation, and bounded leaf-before-root termination reports already gone, gone after signaling, survivors, and errors truthfully without claiming causation it cannot prove or automatically using `SIGKILL`. `OwnedRun` solely owns lifecycle/output; every message carries `OwnedRunId`; its observed activity is joined, not copied, and pinned owned output can coexist with external columns while remaining outside unrelated scope-wide kills. A single `OutputPresentation` controls rendering, layout, focus, tabbability, labels, copy, and hit testing; typed cursors permit visual selection/Ctrl-A only in owned captured output, while columns/navigation preserve stable identities and Tab/Shift-Tab preflight falls through at session boundaries. Defaults are framework-keymap actions only: global `Shift-C`, Output `alt-k` for selected build, and `alt-shift-k` for all scoped builds; render `Option-K`/`Option-Shift-K` on macOS and `Alt-K`/`Alt-Shift-K` elsewhere, with no raw `KeyCode` dispatch outside the keymap. Open termination confirmation is modal above Output/global input. Preserve one Cargo Port-owned run at a time, strict workspace lints/missing docs, `RUSTC_WRAPPER`, nightly formatting for this `natepiano` origin, and inline focused tests plus 1,000/5,000-process refresh benchmarks proving no persistent monitor-off CPU work.

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

### Phase 2 — Strong process observation foundation · status: todo

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

### Phase 3 — Shared refresh scheduling and Running Targets migration · status: todo

#### Work Order

**Goal:** Running Targets uses the App-owned `ProcessObserver`, preserving its one-second behavior while establishing one combined process-refresh scheduler.

**Spec:**

- Move the host-facing process-table work out of `RunningTargetsPoller` and onto the App-owned `ProcessObserver`; retain a Running-target facade for view-specific state.
- Define typed refresh demand for Running, compile monitoring, or both. `ProcessObserver::next_deadline()` and `refresh_due(now)` combine required fields and perform at most one process refresh per due instant.
- Running CPU/history sampling keeps its current one-second cadence even when a later compile-monitor consumer requests identity refreshes more often.
- The terminal event loop waits on the minimum animation/process deadline. Process refresh is not represented as an animation.
- Preserve startup/test suppression semantics in `startup_services.rs` and all existing Running-target visibility, CPU/history, and kill behavior.
- Preserve `WorkspaceIndexRefreshState::{Current, RetainedLastAccepted, Uninitialized}` exactly. Evaluate the one-second cadence/readiness gate before filesystem attribution, use the last accepted index after refresh failure, allow visible-target fallback only while uninitialized, and continue omitting cross-workspace ambiguous exact owners.
- Move the existing Running Targets kill path through a typed `RunningTargetTerminationCapability` that revalidates PID plus creation identity. This capability preserves existing behavior only and remains separate from later build-monitor termination authority.
- Instrument refresh duration through existing frame metrics so Phase 7 can compare the synchronous path against its frame budget.

**Files:**

- `src/process_observation/mod.rs` — add typed consumers, deadlines, and combined refresh plans.
- `src/process_observation/identity.rs` — expose the strong identity revalidation used by the preserved Running Targets termination capability.
- `src/process_observation/snapshot.rs` — support field-union refreshes without duplicate process walks.
- `src/tui/app/mod.rs` — own and expose the observer.
- `src/tui/app/construct.rs` — initialize it.
- `src/tui/running_targets/state.rs` — replace host snapshot ownership with view state over observer records.
- `src/tui/running_targets/app_tick.rs` — request one-second Running refreshes.
- `src/tui/running_targets/constants.rs` — retain the current cadence.
- `src/tui/running_targets/mod.rs` — adapt exports and tests.
- `src/tui/panes/system.rs` — preserve Running Targets kill/action routing while process ownership moves.
- `src/tui/startup_services.rs` — preserve suppression/test effects.
- `src/tui/terminal/event_loop.rs` — wait on the minimum animation/process deadline.
- `src/tui/terminal/frame_metrics.rs` — record observer work separately from rendering.

**Constraints from prior phases:** Use Phase 1's exact `CargoWorkspaceIndex` identities and named refresh states for project data and Phase 2's immutable strong/insufficient observations. Preserve the Phase 1 cadence-before-attribution gate, retained accepted index, uninitialized-only fallback, and conservative cross-workspace ambiguity omission. `ProcessObserver` remains host-only; Running Targets must not recover its own `sysinfo::System`, and its termination capability is not build-monitor authority.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-port`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-port`, and `bash ~/.claude/scripts/delegate/verify.sh lint cargo-port` are green; existing Running Targets tests pass; focused tests prove simultaneous demands cause one refresh, one-second CPU/history cadence and readiness ordering are preserved, retained/uninitialized index paths remain distinct, cross-workspace ambiguous targets stay omitted, PID reuse is rejected before Running Targets termination, and no compile deadline exists yet.

### Phase 4 — Correlated Cargo Port-owned runs · status: todo

#### Work Order

**Goal:** The existing single Cargo Port-owned target run has a stable ID and owns all lifecycle/output state, with late asynchronous messages unable to mutate a later run.

**Spec:**

- Replace anonymous single-run fields in `Inflight` with one `OwnedRun` aggregate carrying monotonic `OwnedRunId`, verified root/process-group identity, launch directory, command, title, captured output, and semantic `OwnedRunLifecycle` variants for queued, starting, running, stopping, retained success, retained gone-after-signal, and retained failure. Do not model these states as a cluster of bare `Option<T>` fields.
- `OwnedRun` is the sole owner of owned lifecycle, outcome, and output. Other state may retain only `OwnedRunId` plus observations.
- Tag every output, progress, started, and finished message with `OwnedRunId`; reconciliation ignores messages whose ID is not the current run.
- Preserve the current one-owned-run concurrency limit, isolated process group, clear/close lifecycle, stopping behavior, output retention, visual-selection frozen snapshots, and monitor-off rendering byte-for-byte.
- Expose owned run identity, lifecycle, and output by reference for later Output presentation; do not copy output into process snapshots.

**Files:**

- `src/tui/state/inflight.rs` — define `OwnedRunId`/`OwnedRun` and own the aggregate.
- `src/tui/state/mod.rs` — export the owned-run API.
- `src/tui/background.rs` — carry the run ID through background work.
- `src/tui/messages.rs` — add run ID to all owned-run messages.
- `src/tui/terminal/processes.rs` — return verified root/process-group identity and tag captured output.
- `src/tui/app/async_tasks/poll.rs` — reconcile only matching run messages.
- `src/tui/panes/output/mod.rs` — consume owned output through the new aggregate without changing presentation.

**Constraints from prior phases:** Use Phase 2 strong identities for the owned root. The isolated process-group termination boundary already exists in `src/tui/terminal/processes.rs`; preserve it while Phase 3 changes shared observation, and do not attribute its ownership to Phase 3.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-port`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-port`, and `bash ~/.claude/scripts/delegate/verify.sh lint cargo-port` are green; focused tests prove late messages from run N cannot mutate run N+1 and all existing launch, stop, clear, copy, visual-selection, and retained-output behavior remains unchanged.

### Phase 5 — Typed monitor scope and state shell · status: todo

#### Work Order

**Goal:** The selected project-list row resolves to a stable, row-kind-aware compile-monitor scope, and monitor state can be on or off without polling or rendering builds yet.

**Spec:**

- Add `src/tui/compile_visibility/` with `CompileVisibilityState::{Off, On(ActiveMonitorState)}`. Only `On` may own a scope key, external snapshot, tombstone, classifier generation, monitor deadline, or late-result acceptance; toggling off drops the entire aggregate.
- Define `MonitorScopeKey` from selected row kind, sorted canonical checkout/workspace roots, metadata revision, and project-list revision. A worktree-group row and its primary checkout row remain different scopes even when they share a path.
- Add a shared App-facing workspace-index adapter with `WorkspaceIndexReadiness::{Current, RetainedLastAccepted, Uninitialized}` so Running Targets and compile visibility consume the same readiness decision instead of duplicating private logic.
- Resolve the selected row as `MonitorScopeResolution::{Ready(MonitorScopeKey), EmptyNonRust, PendingIndex, AmbiguousOwnership, UnresolvedPath}` or an equally semantic exhaustive type. A bare `Option<MonitorScopeKey>` is not permitted; only `Ready` can become actionable.
- Resolve package/workspace rows to the owning workspace checkout; linked-worktree checkout rows include only that checkout; worktree-group rows include the primary and every represented live linked checkout; vendored packages/submodules use their own Cargo workspace when metadata proves one and otherwise their containing checkout; non-Rust rows produce an empty scope.
- A selection, membership, metadata, or project-list revision change immediately replaces the scope key, makes the prior snapshot non-actionable, and leaves the new state pending until its first matching snapshot. Late results carry a monitor generation and are ignored after toggle/scope replacement.
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

**Constraints from prior phases:** Read exact canonical workspace/member/package/target data only from Phase 1's revision-keyed index and preserve its current/retained/uninitialized semantics. `ProjectListRevision` changes only for visible ownership content; selected-row identity is a separate scope input. Process identities and snapshots from Phases 2–3 must not leak into stale scope state. Phase 4 owned output remains independent.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-port`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-port`, and `bash ~/.claude/scripts/delegate/verify.sh lint cargo-port` are green; focused tests cover primary, linked, grouped, workspace-member, vendored/submodule, and non-Rust selections; distinguish current, retained, uninitialized, empty, pending, ambiguous, and unresolved states; prove group and primary keys differ; prove selection changes scope without revising visible content; prove scope/toggle changes invalidate prior generations immediately; prove `Off` owns no monitor deadline or snapshot.

### Phase 6 — Cargo build and compiler classification · status: todo

#### Work Order

**Goal:** Pure build-monitor classification converts an immutable system snapshot plus workspace index into stable build sessions and active compile units without guessing ambiguous attribution.

**Spec:**

- Add domain identifiers and snapshots: `BuildSessionId(ProcessIdentity)`, Phase 5's `MonitorScopeKey`, `ScopeAttribution::{Confirmed, Inferred}`, `CompilerAssociation::{Confirmed, UniqueHeuristic, Ambiguous { candidates }, Unmatched}`, `MonitorSnapshot::{Pending, Fresh, Stale, Unavailable}`, and presentation-only session/activity records. Reuse Phase 1's `cargo_metadata::PackageId`; do not introduce duplicate `BuildScopeId` or `PackageId` wrappers. Raw PIDs never stand alone in an actionable type.
- Discover the outermost recognized root build in a validated Cargo process chain. Normalize rustup proxies, built-in/configured aliases, `cargo-*` plugins, and nested Cargo. Immediately recognize `build`, `check`, `clippy`, `fix`, `run`, `rustc`, `rustdoc`, `test`, `nextest`, `bench`, and `doc`; deny known metadata/fetch/management commands unless a live compiler descendant proves a build.
- Resolve scope for every Cargo node before normalizing. A nested Cargo belongs to the outer root only when confirmed scope and termination boundary match; a plugin/alias entering another checkout becomes a separate session. Discover compatible roots system-wide before filtering the selected scope.
- Resolve root scope in order: Cargo Port-owned PID/launch directory; `--manifest-path` or absolute manifest argument; cwd plus nearest containing manifest; uniquely matching compiler output directory. Canonicalize both sides and never use string-prefix matching alone.
- Associate `rustc`, `clippy-driver`, `rustdoc`, build-script, and linker descendants by validated parent chain. For cache-daemon parentage, use `(target directory, profile/build directory, target triple)` only when it selects one compatible live session across the entire system. Render ambiguous units once in a scope-level, non-actionable attribution-unavailable section with candidate sessions.
- Derive compile units primarily from `--crate-name`, primary input, `--out-dir`, target triple, flags, and strong compiler identity. Resolve workspace packages from the shared index; for dependencies absent from `no_deps`, parse the nearest package manifest once and cache package identity by canonical source root plus manifest stamp. Reparse after change/removal; otherwise use `CompilerCrateIdentity::{WorkspacePackage(cargo_metadata::PackageId), DependencyPackage(DependencyPackageIdentity), CrateNameFallback(CompilerCrateName)}` or an equally semantic fallback type.
- Keep classification pure. A stateful adapter prepares immutable `BuildClassificationInput` containing the process snapshot, workspace-index view, dependency-manifest snapshot, and first-seen snapshot; caches and the first-seen ledger are updated outside the pure classify call.
- Use session key `BuildSessionId(ProcessIdentity)`; target directory and profile are attributes. Resolve profile from explicit `--profile`/`--release`, then output directories, then metadata defaults, preserving custom/unknown labels. Order sessions and units by first-seen then process identity.

**Files:**

- `src/build_monitor/mod.rs` — domain exports and classification entry point.
- `src/build_monitor/model.rs` — typed IDs, attribution, activity, snapshots, and non-actionable presentation records.
- `src/build_monitor/classify.rs` — Cargo root normalization, scope resolution, compiler association, unit/profile/package derivation, and caches.
- `src/main.rs` — declare the build-monitor domain.
- `src/process_observation/snapshot.rs` — expose immutable executable, argv, cwd, ancestry, and creation evidence required by pure classification.
- `src/project/cargo/metadata_store.rs` — expose manifest/source stamps without adding dependency metadata commands.
- `src/project/cargo/mod.rs` — supply index queries used by classification.
- `src/project/cargo/workspace_index.rs` — supply exact package, target, workspace, and ambiguity queries used by classification.
- `src/project/cargo/workspace_index_api_tests.rs` — prove classification-facing queries retain all exact candidates.
- `src/tui/workspace_index.rs` — supply named readiness and immutable index views to classification callers.
- `Cargo.toml` — add only parsing/platform dependencies required by the classifier.

**Constraints from prior phases:** Consume Phase 1's exact `CargoWorkspaceIndex` identities and existing `cargo_metadata::PackageId`, Phase 2's named strong/insufficient immutable observations, Phase 4's owned identity, and Phase 5's canonical `MonitorScopeKey` plus named scope/index readiness. Cross-workspace exact ambiguity remains non-actionable. Classification creates no signal authority, owns no mutable observer/cache state, and launches no Cargo command.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-port`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-port`, and `bash ~/.claude/scripts/delegate/verify.sh lint cargo-port` are green; pure tests cover root commands, non-build commands, proxies/aliases/plugins/nested and divergent scopes, sibling roots, direct compiler/build-script/linker children, cache-daemon and cross-workspace ambiguity, debug/release/custom profiles, PID reuse and exec transitions, every named scope/index readiness state, dependency manifest caching/invalidation, exact workspace package IDs, semantic dependency/crate-name fallback, and no-deps fallback.

### Phase 7 — Process refresh execution budget · status: todo

#### Work Order

**Goal:** Compile monitoring has one measured `ProcessRefreshExecutor` boundary whose synchronous or worker-backed implementation is chosen before lifecycle polling is added.

**Spec:**

- Add repeatable 1,000- and 5,000-process benchmarks covering the combined Phase 2 observer refresh and Phase 6 classification input/classification path, including representative Cargo, compiler, wrapper, and unrelated processes.
- Measure against `tui_pane::SLOW_FRAME_MS`, currently 30 ms. The synchronous implementation is eligible only when repeated 5,000-process samples remain at or below a 15 ms event-loop allocation and therefore below the 30 ms slow-frame boundary; otherwise implement immutable request/result work behind a dedicated worker and `crossbeam-channel`.
- Define one semantic `ProcessRefreshExecutor` API used by later phases regardless of the selected implementation. Requests carry refresh correlation plus immutable observation/classification inputs; results carry the matching correlation and immutable output.
- Record the selected architecture in the implementation and deterministic tests. Benchmark timing is reported evidence, not a flaky CI assertion; tests prove scheduling, correlation, result ordering, and absence of mutable App state in worker inputs.
- This phase adds no compile-monitor deadline or lifecycle. Phase 8 consumes the executor without reopening the architecture choice.

**Files:**

- `src/process_observation/mod.rs` — expose combined observation work through the executor boundary.
- `src/process_observation/snapshot.rs` — build immutable refresh inputs/results for measurement or worker transfer.
- `src/build_monitor/classify.rs` — accept the immutable classification input measured by the executor.
- `src/build_monitor/benchmarks.rs` — repeatable 1,000/5,000-process fixtures and timing report harness.
- `src/build_monitor/model.rs` — carry semantic refresh correlation and immutable results.
- `src/build_monitor/mod.rs` — export `ProcessRefreshExecutor`-facing classification APIs.
- `src/tui/terminal/frame_metrics.rs` — use the existing 30 ms slow-frame boundary and record refresh cost separately.
- `src/tui/terminal/event_loop.rs` — host only the selected executor integration, without adding a compile deadline.
- `src/tui/background.rs` — add a dedicated refresh worker only if the 15 ms synchronous allocation is exceeded.
- `src/tui/messages.rs` — add immutable correlated refresh results only if the worker path is selected.

**Constraints from prior phases:** Phase 2 supplies named observation evidence and immutable snapshots; Phase 3 supplies one combined refresh plan and separate duration instrumentation; Phase 5 supplies named index/scope readiness; Phase 6 supplies the pure classifier and immutable `BuildClassificationInput`. Preserve Running Targets cadence and add no compile polling yet.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-port`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-port`, and `bash ~/.claude/scripts/delegate/verify.sh lint cargo-port` are green; 1,000/5,000-process benchmarks are run and their repeated timing is recorded; the synchronous path is used only at or below the 15 ms allocation, otherwise a worker/channel path is implemented; deterministic tests prove executor correlation, scheduling, and immutable transfer; no compile deadline exists yet.

### Phase 8 — Conditional monitor polling and lifecycle · status: todo

#### Work Order

**Goal:** Enabling compile visibility produces fresh scoped monitor snapshots on a bounded cadence, while disabling it removes all compile-specific polling and idle work.

**Spec:**

- Add `BuildMonitor` state over the pure classifier. It retains only live session/unit identities, explicit owned association, termination tombstones added in later phases, and the latest presentation snapshot; it does not accumulate external history.
- While enabled, request command-line/process fields through the combined Phase 3 refresh plan and execute through Phase 7's chosen `ProcessRefreshExecutor`; perform one bounded process walk per interval, never one scan per workspace or column.
- Track live target-directory resolution as a typed state and revision, rechecked on each due poll. A previously missing target directory appearing, or a symlink being created/retargeted, invalidates affected classification and actionability even when metadata, project-list content, and selected scope are unchanged.
- `ProcessObserver::next_deadline()` contributes no compile deadline while monitor state is `Off`. On a due instant shared with Running Targets, union fields and perform one refresh while preserving the Running one-second CPU/history sample.
- A successful result must carry monitor generation and exact `MonitorScopeKey`. Ignore mismatches. On scope change show `Pending`; on one refresh failure retain the last good snapshot as visibly `Stale` and non-actionable for one interval, then `Unavailable`.
- A root Cargo process anchors a session through gaps with no live compiler. Report evidence-backed compiling, build-script, linking, owned Cargo-lock wait, and running-target states; report external no-child gaps only as active.
- Associate an owned run with exactly one observed session by identity and retain only `OwnedRunId`; never copy owned output into snapshots. External completed sessions disappear.
- Prove no persistent idle CPU work while off and no compile-specific request/result acceptance after a toggle or scope generation change.

**Files:**

- `src/build_monitor/mod.rs` — `BuildMonitor` lifecycle and snapshot API.
- `src/build_monitor/poll.rs` — conditional refresh requests, classification, failure aging, and generation correlation.
- `src/build_monitor/model.rs` — fresh/stale/unavailable presentation states and stable first-seen ordering.
- `src/process_observation/mod.rs` — add optional compile consumer/deadline.
- `src/tui/compile_visibility/mod.rs` — connect enabled scope/generation to monitor polling.
- `src/tui/app/mod.rs` — own `BuildMonitor`.
- `src/tui/app/construct.rs` — initialize it without enabling it.
- `src/tui/app/async_tasks/poll.rs` — reconcile generation-tagged results if a worker is required.
- `src/tui/terminal/event_loop.rs` — include the optional monitor deadline.
- `src/tui/terminal/frame_metrics.rs` — record and assert bounded refresh work.
- `src/tui/startup_services.rs` — ensure disabled/test startup creates no monitor work.

**Constraints from prior phases:** Phase 1 supplies exact index identities and current/retained/uninitialized readiness; Phase 3 supplies the single combined refresh scheduler and cadence-before-filesystem ordering; Phase 5 owns named scope resolution and enablement/scope generation; Phase 6 supplies pure classification and ambiguity omission; Phase 7 owns the measured executor architecture; Phase 4 owns run output and semantic lifecycle. Do not add rendering or termination authority.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-port`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-port`, and `bash ~/.claude/scripts/delegate/verify.sh lint cargo-port` are green; tests prove disabled means no compile deadline/refresh/parsing/result acceptance, combined due work performs one process walk through Phase 7's executor, stale data is non-actionable then unavailable, scope/generation changes reject late results, target-directory appearance and symlink retargeting revise live resolution without metadata/list changes, owned activity joins once, and selection changes never affect external processes.

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
- Introduce typed cursor targets `Empty`, `Header`, `Activity(ProcessIdentity)`, `Unattributed(ProcessIdentity)`, and `CapturedOutput(OutputSelection)`. The captured-output variant is constructible only for an owned column. Output hit results carry session identity plus header/activity/output-row identity; empty monitor has a full-pane focusable hit.
- Retain per-column selected row by process identity. On unit exit choose the row now at prior index, then previous, then header. On session exit choose the session now at prior ordered index, then preceding, then pinned owned, then empty. Scope invalidation retains only a still-present pinned owned selection.
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

**Constraints from prior phases:** Join Phase 8 immutable monitor snapshots with Phase 4 `OwnedRun` by ID. Phase 6 first-seen ordering and Phase 5 named scope/index readiness plus immediate invalidation bind cursor fallback and empty-state rendering. Do not add key handling or termination.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-port`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-port`, and `bash ~/.claude/scripts/delegate/verify.sh lint cargo-port` are green; render/state tests prove distinct non-actionable pending-index, empty-non-Rust, ambiguous-ownership, and unresolved-path states; empty enabled focus; full-width single session; readable/windowed multiple columns; selected-column visibility; stable row/session fallback; unattributed section non-actionability; owned output joined once and pinned across scope changes; and monitor-off output matches prior behavior.

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

- Keep observation and termination separate. Define `ActionableBuild::{OwnedProcessGroup(OwnedRunId), External(ActionableExternalBuild)}`; the external aggregate bundles confirmed scope attribution, strong root identity, lifecycle eligibility, and an opaque `TerminationAuthorizationToken` so those facts cannot drift independently or be reconstructed by UI code.
- Add a separate `ProcessTerminator`. `ProcessObserver` produces immutable evidence and safe platform capabilities only; it never accepts termination plans or signals. `ProcessTerminator` executes every bounded transaction on a dedicated worker/channel path so revalidation, signaling, and deadline waits never block the TUI event loop.
- Implement platform adapters that bind signaling to the observed process object strongly enough to reject PID reuse. Use an identity-bound handle or another demonstrated safe adapter where the platform supplies one; a platform without a proven safe adapter exposes external sessions as `ObservedOnly`. Do not assume macOS is observed-only without checking available host APIs, and never fall back to a bare/racy PID action or external ambient process group.
- Only a fresh snapshot with `ActionableBuild` may construct a termination request. Pending, stale, inferred, ambiguous, unattributed, weak-identity, completed, tombstoned, or already-terminating sessions carry no action-bearing handles.
- Define `TerminationRequestId`, opaque immutable authorization/plan, `TerminationOutcomeSummary`, and `TerminationError`. Frozen evidence remains owned by the token/request even after observer-cache eviction; UI code can retain and submit the token but cannot inspect, synthesize, or decompose its authority.
- `BuildMonitor` owns the complete transaction: freeze authorization and scope, transition sessions to `Terminating`, send an immutable plan to `ProcessTerminator`, and reconcile exactly one matching result.
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

**Constraints from prior phases:** Phase 2 defines strong/insufficient identity, named evidence, incarnation, and validated ancestry; Phase 6 defines confirmed exact scope/session association; Phase 8 owns fresh/stale lifecycle; Phase 4 owns isolated process groups. `ProcessObserver` remains observation-only, all signaling runs through `ProcessTerminator` off the event loop, and no UI may synthesize or decompose opaque action authority.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-port`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-port`, and `bash ~/.claude/scripts/delegate/verify.sh lint cargo-port` are green; deterministic process-fixture tests prove PID reuse rejection, safe-adapter or observed-only fallback, opaque authority cannot be reconstructed, exact request/result correlation, termination work stays off the event loop, validated descendant admission, no post-root first admission, leaf-before-root order, continued tracking after root exit, exclusions, owned group/wait serialization, no automatic `SIGKILL`, and truthful already-gone/gone-after-signal/partial-failure outcomes.

### Phase 12 — Selected-build termination interaction · status: todo

#### Work Order

**Goal:** From an actionable selected Output column, `Alt-k` (`Option-K` on macOS labels) opens a modal confirmation and safely terminates that entire root build.

**Spec:**

- The selected compiler/activity row identifies cursor location only; selected-build termination always targets the owning root Cargo invocation.
- Expose the action only when the selected column's fresh session has `OwnedProcessGroup` or `ActionableExternalBuild`. Headers and activity rows may invoke it; unattributed, pending, stale, observed-only, completed, killed, failed-unrefreshed, and terminating sessions cannot.
- Confirmation shows operative command, checkout, PID, start age, and current observed compiler-child count. It retains the opaque `TerminationAuthorizationToken` produced by Phase 11 plus separate display data; UI code must not rebuild authority from `BuildSessionId`, scope, root identity, or PID.
- Confirmation is modal and consumes input before Output cancellation, globals, copy, or navigation: `y` submits the frozen request; `n` or `Esc` cancels; all other keys do nothing.
- Before signaling, Phase 11's opaque token requires the frozen session identity and scope still match a fresh observation. Exit becomes an already-gone toast, scope/identity mismatch rejects the request, and no replacement process at the PID is touched.
- Render `Terminating` until the correlated Phase 11 transaction completes. Retain a selected-build gone-after-signal tombstone until a new build replaces it, scope changes, or monitoring toggles off; do not label an external process “killed” when only disappearance after a signal is observed. On errors/deadline/survivors render a visible partial failure; enable retry only after a new fresh actionable snapshot and confirmation.
- Preserve existing `Esc` owned-run stop behavior outside the modal and when monitoring is off.

**Files:**

- `src/tui/app/confirm_action.rs` — selected-build confirmation payload retaining opaque authorization plus separate display data.
- `src/tui/app/mod.rs` — construct/submit requests and reconcile toasts/state.
- `src/tui/input/dispatch.rs` — modal priority and `y`/`n`/Esc handling.
- `src/tui/integration/framework_keymap/output_pane.rs` — availability and dispatch for `alt-k`.
- `src/tui/panes/output/pane.rs` — selected session/row lookup.
- `src/tui/panes/output/render.rs` — terminating, gone-after-signal, already-gone, and partial-failure markers.
- `src/tui/render.rs` — modal confirmation and status/toast presentation.
- `src/tui/messages.rs` — carry selected request/result IDs through the Phase 11 transaction channel.
- `src/build_monitor/termination.rs` — construct a one-session frozen plan.

**Constraints from prior phases:** Use Phase 10's framework action and platform label; retain and submit only Phase 11's opaque authority token/transaction without reconstructing it. Phase 5 scope changes make an open request invalid, and Phase 9 owns selection identity/fallback.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-port`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-port`, and `bash ~/.claude/scripts/delegate/verify.sh lint cargo-port` are green; interaction tests prove modal precedence, opaque-token retention without UI authority reconstruction, exact frozen scope/session, root-not-row semantics, stale/inferred/ambiguous/weak-state unavailability, PID exit/reuse safety, truthful terminating/already-gone/gone-after-signal/partial-failure states, fresh-observation retry, and no effect on unrelated builds/cache daemons.

### Phase 13 — Scope-wide termination and end-to-end verification · status: todo

#### Work Order

**Goal:** `Alt-Shift-k` (`Option-Shift-K` on macOS labels) safely terminates exactly the live actionable roots in the selected scope, with final end-to-end proof of the complete feature.

**Spec:**

- Scope-wide termination requires a nonempty live root set and refuses to open if any represented live root is observed-only; “all” never means a silent actionable subset.
- Build the set from the current `MonitorScopeKey` only. Exclude pinned owned output outside the selected scope, completed runs, tombstones, unattributed compiler units, and duplicate/nested references to the same root.
- Confirmation names the selected scope and exact deduplicated `BuildSessionId` set for display, while retaining one opaque Phase 11 authorization token for that frozen set. UI code never reassembles authority from the displayed scope/session IDs. Any scope/metadata/project-list revision change invalidates the token.
- A build starting after confirmation is never added to destructive authority; leave it running and report that a newer build was not included. A root that already exited is `gone`, never replaced by a new process at the PID.
- Submit the opaque exact frozen-set token through Phase 11's one bounded transaction. Render per-root and aggregate terminating, gone-after-signal, already-gone, survivor, and error outcomes truthfully. Retain gone-after-signal tombstones until scope change, replacement build, or monitor off.
- Complete focused automated coverage for simultaneous debug/release, linked/group worktree scope, unique versus ambiguous cache-wrapper attribution, owned target plus external build/Cargo-lock wait, selected versus scope kill, and disabled polling.
- Perform live verification on macOS where available: debug and release in one checkout; builds in two linked worktrees with group versus checkout scope; `RUSTC_WRAPPER=rust-cache`/`sccache`; owned target launch beside an external build including Cargo-lock wait; selected kill preserving unrelated builds/cache daemon; scope kill affecting only deduplicated scoped roots; toggle off ceasing compile work. If an external platform adapter is intentionally unavailable, verify observed-only rendering/action unavailability rather than using unsafe fallback.

**Files:**

- `src/tui/app/confirm_action.rs` — scoped confirmation payload retaining the opaque token plus exact-set display data.
- `src/tui/app/mod.rs` — create, submit, and reconcile scope-wide requests.
- `src/tui/input/dispatch.rs` — modal input for the scope action.
- `src/tui/integration/framework_keymap/output_pane.rs` — availability/dispatch for `alt-shift-k`.
- `src/build_monitor/termination.rs` — deduplicate and freeze the all-scope transaction.
- `src/build_monitor/model.rs` — per-root and aggregate results/tombstones.
- `src/tui/panes/output/render.rs` — scoped transaction outcome rendering.
- `src/tui/render.rs` — scope confirmation and completion toast.
- `tests/assets/default-keymap.toml` — verify final generated defaults if Phase 9 fixtures changed during integration.

**Constraints from prior phases:** Phase 12 establishes modal selected termination; reuse it rather than creating a second input path. Phase 11's opaque token and transaction semantics plus Phase 5 exact scope generations bind the frozen set; UI code never reconstructs it. Phase 4 pinned owned output remains outside unrelated scope-wide authority.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-port`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-port`, and `bash ~/.claude/scripts/delegate/verify.sh lint cargo-port` are green; automated tests prove all-or-refuse actionability, opaque frozen-set retention without UI reconstruction, root deduplication, new-build exclusion, gone-versus-reused identity, pinned-owned exclusion, modal priority, truthful exact scoped outcomes, and compile-monitor-off quiescence; the live verification matrix is completed with any platform-observed-only limitation recorded without weakening safety.
