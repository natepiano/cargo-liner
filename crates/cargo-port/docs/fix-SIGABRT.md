# Fix SIGABRT Failures in Tests

> **Status: IMPLEMENTATION PLAN — phased, delegate-ready.** Build deterministic unit-test startup and replace test-side process aborts with diagnosable failures.

## Delegation Context

- **Project:** cargo-port workspace — `cargo-port` TUI for inspecting/managing Rust projects plus `tui_pane` reusable ratatui pane framework.
- **Stack:** Rust 2024; ratatui 0.30.1, crossterm 0.29.0, tokio 1, reqwest 0.13.4, notify 9.0.0-rc.2, tui_pane 0.2.0-dev, cargo-nextest.
- **Layout:** `src/test_support.rs`, `src/tui/test_support.rs` shared test helpers; `src/tui/app/` App construction, async tasks, embedded framework-keymap tests; `src/tui/keymap/` test keymap override lifetime; `src/tui/state/`, `src/tui/panes/`, `src/themes/` startup side effects; `src/scan/`, `src/project/git/`, `tui_pane/src/toasts/` abort cleanup buckets; `scripts/check-no-test-abort.sh` new abort inventory gate.
- **Key files:** `docs/fix-SIGABRT.md` — this phased implementation plan.
- **Key files:** `Cargo.toml` — workspace/package metadata, Rust edition, dependencies, strict lint settings.
- **Key files:** `tui_pane/Cargo.toml` — workspace member purpose/version/deps.
- **Key files:** `.github/workflows/ci.yml` — CI command source: fmt, taplo, clippy, build, nextest, mend.
- **Key files:** `src/test_support.rs` — shared test runtime/header helpers with abort setup paths.
- **Key files:** `src/tui/test_support.rs` — `test_http_client`, `make_app`, `make_app_with_config`, default keymap override, `App::new`, retry spawn mode, and `test_keymap_path`.
- **Key files:** `src/tui/app/construct.rs` — App builder startup, theme load, watcher spawn, panes, `ThemeRuntime`, `build`, `finish_new`, GitHub rate-limit prime.
- **Key files:** `src/tui/state/net.rs` — `HttpClient` state and `spawn_rate_limit_prime()` detached startup effect.
- **Key files:** `src/tui/app/async_tasks/service_handlers.rs` — startup auth warning and HTTP/client retry context.
- **Key files:** `src/tui/app/async_tasks/lint_runtime.rs` — watcher respawn/registration and lint-runtime side-effect owner.
- **Key files:** `src/tui/app/async_tasks/config.rs` — theme reload plus config reload paths that respawn watcher/lint runtime.
- **Key files:** `src/tui/panes/cpu/pane.rs` — CPU monitor pane construction/reset target.
- **Key files:** `src/themes/paths.rs` — developer-local theme directory and test override hooks.
- **Key files:** `src/tui/keymap/load.rs` — `override_keymap_path_for_test` and absent-override layering for keymap fixtures.
- **Key files:** `src/tui/app/mod.rs` — `framework_keymap` state/reload, embedded framework-keymap tests, `make_app_with_keymap_toml`, originally observed test, shared test wrappers, largest abort bucket.
- **Key files:** `tui_pane/src/toasts/mod.rs` — toast test abort cleanup bucket.
- **Key files:** `tui_pane/src/toasts/render/mod.rs` — toast render test abort cleanup bucket.
- **Key files:** `src/scan/tree/mod.rs` — scan tree test abort cleanup bucket.
- **Key files:** `src/scan/disk_usage.rs` — disk usage test abort cleanup bucket.
- **Key files:** `src/project/git/discovery.rs` — git discovery test abort cleanup bucket.
- **Key files:** `src/tui/panes/ci/render.rs` — CI render test abort cleanup bucket.
- **Key files:** `src/tui/running_targets/app_tick.rs` — running-target test abort cleanup bucket.
- **Key files:** `src/tui/app/async_tasks/running_toasts.rs` — remaining production-looking abort in `ReactivateOutcome::Revived` branch.
- **Key files:** `scripts/check-no-test-abort.sh` — executable abort inventory allowlist gate created by Phase 2.
- **Build:** `cargo build --release --all-features --workspace --examples`
- **Test:** `cargo nextest run --workspace --all-features --tests`
- **Lint:** `cargo +nightly fmt --all`; `taplo fmt --check`; `cargo clippy --workspace --all-targets --all-features -- -D warnings`; `cargo mend --workspace --all-targets --fail-on-warn`; `./scripts/check-no-test-abort.sh`.
- **Style:** `zsh ~/.claude/scripts/rust_style/load-rust-style.sh --project-root /Users/natemccoy/rust/cargo-port`
- **Invariants:** Do not change product behavior. Production invariants stay strict unless the remaining production abort is deliberately reviewed and documented/restructured. Quiet unit-test startup must be selected before startup side effects run and must persist through reload/reset paths. Shared unit-test app constructors default to deterministic quiet state and do not spawn watcher, lint runtime, retry probe, rate-limit, lint-history/cache, host `gh`, CPU monitor, theme-directory, or process-global work. Full-startup tests must opt in explicitly, isolate fixture-owned config/keymap/theme/HTTP/watcher/runtime state, and drain or cancel work. Keymap helpers must not return an `App` pointing at deleted temp paths or dropped override guards. Test/helper aborts become contextual panics or unreachable paths with observed spawned-work failures joined/drained. Abort inventory is enforced by an executable exact allowlist.

## Phases

### Phase 1 — Convert shared test helpers from abort to panic  · status: done (`uncommitted`)

#### Work Order

**Goal:** Shared test setup failures report normal panics with context instead of terminating the process with `SIGABRT`.

**Spec:**
Replace abort-based setup in the shared helpers many tests transitively use:

```rust
let tmp = tempfile::tempdir().expect("create test tempdir");
let value = maybe_value.expect("expected test value");
let Ok(value) = result else {
    panic!("expected test setup to succeed: {result:?}");
};
```

Apply the conversion only to shared helper setup paths in this phase. Keep messages short but specific. Each message should name the expected test condition, not restate the type operation. Do not change application behavior.

Mechanical replacements:

- `unwrap_or_else(|_| std::process::abort())` becomes `expect("specific context")`.
- `unwrap_or_else(|| std::process::abort())` becomes `expect("specific context")`.
- `let Some(x) = maybe else { std::process::abort() };` becomes a `panic!` with context.
- Impossible enum variants in tests become `unreachable!("specific context")`.

**Files:**
- `src/test_support.rs` — convert shared runtime/header helper abort setup paths to contextual panics.
- `src/tui/test_support.rs` — convert TUI app-construction helper abort setup paths to contextual panics without changing startup behavior yet.

**Constraints from prior phases:** Empty.

**Acceptance gate:** `cargo +nightly fmt --all`; `cargo nextest run --workspace --all-features --tests src::test_support` or the closest available focused helper tests; the original focused keymap test still may expose later flake work, but any failure from these helpers should now report a panic, not `SIGABRT`.

#### Retrospective

**What worked:**
- `src/test_support.rs` and `src/tui/test_support.rs` now use contextual `expect(...)` panics instead of `std::process::abort()`.
- The delegate kept the phase scope narrow and did not change startup behavior.

**What deviated from the plan:**
- The requested focused nextest filter matched zero tests, so the delegate used nearby focused helper-heavy tests plus clippy.

**Surprises:**
- Both helper modules needed a local `clippy::expect_used` allow because the repo denies `expect_used`.

**Implications for remaining phases:**
- Later abort sweeps should not revisit `src/test_support.rs` or `src/tui/test_support.rs` unless the abort inventory shows new drift.
- Future helper conversions that intentionally use `expect(...)` may need the same test-only `clippy::expect_used` allowance pattern.

#### Phase 1 review adjustments for remaining phases

- Focused `cargo nextest` gates in later phases must match at least one test. If a named filter matches zero tests, replace it with a confirmed relevant test or document the fallback command used for that phase.
- Phase 3 owns the durable quiet-startup service plumbing and quiet defaults. Phase 4 owns full-startup opt-in fixtures, teardown, serialization checks, and any handle refinements revealed by those tests.
- New test setup code that uses `expect(...)` must either use a scoped test-only `clippy::expect_used` allowance, or use a contextual `panic!`/`unreachable!` form that satisfies the repo lint policy.
- `src/test_support.rs` and `src/tui/test_support.rs` are Phase 1 zero-abort outputs. Later inventory or conversion phases must fail on any new `std::process::abort()` in those files rather than treating them as remaining work.
- Custom keymap helpers must use an owning fixture or a real kept temp file. Do not copy the leaked `TempDir` pattern from `test_keymap_path()` for new custom keymap setup.
- User-approved sequencing change: the abort inventory gate moved from the original Phase 5 to new Phase 2. Repetition gates remain later, after quiet-startup and keymap stabilization.

### Phase 2 — Add executable abort inventory gate  · status: done (`uncommitted`)

#### Work Order

**Goal:** Install the abort inventory gate before additional startup/keymap work so later phases cannot reintroduce shared-helper process aborts.

**Spec:**
Create `scripts/check-no-test-abort.sh` as an executable, deterministic, exact allowlist gate. It should scan Rust source for `std::process::abort()`, `process::abort()`, and obvious direct `abort()` call patterns used by tests. The gate must fail on any abort in `src/test_support.rs` or `src/tui/test_support.rs`, because Phase 1 converted those helpers to zero-abort outputs.

The initial allowlist may include the remaining known test-module abort buckets documented in this plan and the one production-looking abort in `src/tui/app/async_tasks/running_toasts.rs`. Keep the allowlist explicit by file/path and expected count or exact matched line text so drift is visible.

Use stable shell tooling already available in the repo. Do not depend on nextest or cargo for this gate. The script should print a short inventory on failure and exit nonzero. Add executable bit.

**Files:**
- `scripts/check-no-test-abort.sh` — new inventory allowlist gate.
- `docs/fix-SIGABRT.md` — keep the remaining known abort buckets in sync if implementation discovers drift.

**Constraints from prior phases:** Phase 1 converted shared helper setup in `src/test_support.rs` and `src/tui/test_support.rs` from `std::process::abort()` to contextual panics and added local `clippy::expect_used` allowances. Do not reintroduce `std::process::abort()` in those files.

**Acceptance gate:** `./scripts/check-no-test-abort.sh`; `cargo +nightly fmt --all` if any Rust files changed. If the script reports additional abort sites, classify them as either documented remaining buckets or new drift before continuing.

#### Phase 2 implementation notes

`scripts/check-no-test-abort.sh` scans Rust files under the repository root with `find`, excluding `.git` and `target`, and compares matched abort call lines against this explicit count allowlist:

- `src/tui/app/mod.rs` — 250
- `tui_pane/src/toasts/mod.rs` — 18
- `src/scan/tree/mod.rs` — 18
- `src/scan/disk_usage.rs` — 15
- `src/project/git/discovery.rs` — 13
- `tui_pane/src/toasts/render/mod.rs` — 8
- `src/tui/panes/ci/render.rs` — 5
- `src/tui/app/async_tasks/running_toasts.rs` — 1
- `src/tui/running_targets/app_tick.rs` — 1

The gate separately enforces zero matched abort call lines in `src/test_support.rs` and `src/tui/test_support.rs`. The Phase 2 implementation found no additional abort-bearing Rust files beyond the documented remaining buckets.

#### Retrospective

**What worked:**
- `scripts/check-no-test-abort.sh` now provides the early executable abort inventory gate.
- The gate enforces zero abort call lines in the Phase 1 helper files and allowlists the remaining known buckets by explicit path/count.

**What deviated from the plan:**
- No Rust files changed, so the delegate skipped `cargo +nightly fmt --all`.

**Surprises:**
- The current inventory has 329 matched abort call lines across 9 allowlisted files.

**Implications for remaining phases:**
- Remaining phases should run `./scripts/check-no-test-abort.sh` after touching Rust abort sites or helper setup.
- Later phases must tighten the allowlist whenever they remove an abort bucket, rather than leaving stale counts.
- `src/test_support.rs` and `src/tui/test_support.rs` are now protected by both the Phase 1 conversion and the Phase 2 gate.

#### Phase 2 Review

- Phase 3, Phase 4, Phase 5, Phase 6, Phase 7, and Phase 8 now carry the Phase 2 constraint that matched abort count changes must update `scripts/check-no-test-abort.sh` in the same change set.
- Phase 6 was clarified as tightening the existing Phase 2 abort inventory rather than creating a new gate.
- Phase 7 now names Phase 2 as the source of the executable inventory gate and Phase 6 as the later repetition/tightening phase.
- Phase 8 now requires an exact reviewed production-site allowance if the remaining production abort stays intentional.
- CI integration for `scripts/check-no-test-abort.sh` was considered and intentionally not added; the script remains a local migration/final-validation gate for this plan.

### Phase 3 — Add quiet startup services and make shared App helpers quiet by default  · status: done (`uncommitted`)

#### Work Order

**Goal:** Unit tests that call `make_app()` or `make_app_with_config()` construct deterministic `App` state without starting detached startup work.

**Spec:**
Audit `App` construction used by unit tests and introduce a startup profile that is selected before the builder starts any side effects. The quiet profile must be an input to `App::new()` or the builder before `AppBuilder::run_startup()` runs, and the same policy must remain reachable for the entire `App` lifetime. Construction-time gating alone is not enough, because reload and reset paths can respawn watcher, lint, network, or CPU work after the test has already built the app.

Represent the startup choice with one typed capability set instead of scattered booleans. The exact type names may change during implementation, but the ownership rule must hold: startup side-effect policy is decided once, before construction starts, consumed by the builder, and kept by the resulting app or subsystem handles for later reload/reset paths.

Use this shape as the starting point:

```rust
enum StartupProfile {
    Production,
    QuietUnitTest(StartupEffects),
}

struct StartupEffects {
    watcher: bool,
    lint_runtime: bool,
    lint_history_hydration: bool,
    lint_cache_scan: bool,
    github_rate_limit_prime: bool,
    service_retry_probes: bool,
    cpu_monitor: bool,
}
```

Split startup policy from startup execution. Add one startup side-effect owner, such as `StartupServices` or `StartupSideEffects`, that is carried through `AppBuilder<Inputs>`:

- Production services call real functions.
- Quiet test services return no-op handles and record spawn counters.
- The startup profile decides what is allowed.
- The service owner performs, fakes, drains, or counts the work.

Quiet mode must skip or stub:

- GitHub rate-limit priming.
- Service retry probes unless a test explicitly enables them.
- Watcher startup.
- Lint runtime startup.
- Lint history hydration and lint cache usage scans.
- Host-dependent `gh auth token` subprocesses in test HTTP clients.
- CPU monitor startup unless a test is specifically about that behavior.
- Theme directory reads from developer-local config unless a test opts in with fixture-owned theme files.
- Any spawned work on the shared process-wide test runtime unless the fixture owns and drains it.

Do not gate all of `finish_new()` behind one quiet-mode check. Split deterministic setup from external startup effects. Quiet tests should still load keymaps, force settings when appropriate, prune/register in-memory state, and sync selected projects, but should skip or stub watcher registration, lint runtime registration, disk lint refresh, GitHub rate-limit priming, auth/network warnings, and monitor startup.

Use quiet startup as the default contract for shared unit-test constructors. `make_app()` and `make_app_with_config()` remain the common helpers and become quiet by default. Full startup coverage must live behind explicit opt-in helpers, not casual use of the default helper. Direct `App::new()` in unit tests should be reserved for constructor/startup tests.

The profile must reach `AppBuilder<Channeled>::run_startup()`, `AppBuilder<Started>::build()`, and subconstructors. Route it through constructors or facades for `Panes`, `CpuPane`, `RunningTargetsPoller`, and `ThemeRuntime` so quiet mode can use inert state where host-observing work would otherwise start.

Track startup effects in a table during implementation. Each entry should name the exact call site and effect class: thread spawn, tokio task, blocking task, subprocess, filesystem watcher, disk read, process-global mutation, theme directory read, or render-time/refresh polling. Gate the actual effect call sites, not harmless state objects.

#### Phase 3 startup-effect table

| Call site | Effect class | Phase 3 gate |
|---|---|---|
| `src/http/client.rs::HttpClient::new` | subprocess: `gh auth token` | Quiet helpers call `StartupServices::test_http_client`, which uses `HttpClient::new_without_github_auth_for_test`. |
| `src/tui/app/construct.rs::AppBuilder<Channeled>::run_startup` -> `config::set_active_config` | process-global mutation | `StartupServices::install_active_config`. |
| `src/tui/app/construct.rs::AppBuilder<Channeled>::run_startup` -> `themes::themes_dir` / `ThemeRegistry::from_dir_with_builtins` | theme directory read | `StartupServices::themes_dir`; quiet startup passes `None` and uses built-ins. |
| `src/tui/app/construct.rs::AppBuilder<Channeled>::run_startup` -> `tui_pane::install_theme_state` / `set_focused_pane_tint` | process-global mutation | `StartupServices::install_theme_state`. |
| `src/tui/app/construct.rs::AppBuilder<Channeled>::run_startup` -> `lint::spawn` | thread spawn | `StartupServices::spawn_lint_runtime`. |
| `src/tui/app/construct.rs::AppBuilder<Channeled>::run_startup` -> `watcher::spawn_watcher` | filesystem watcher, thread spawn | `StartupServices::spawn_watcher`. |
| `src/tui/app/async_tasks/lint_runtime.rs::respawn_watcher` | filesystem watcher, thread spawn | `StartupServices::spawn_watcher`. |
| `src/tui/app/async_tasks/lint_runtime.rs::refresh_lint_runs_from_disk` | tokio task, blocking task, disk read | `StartupServices::lint_history_hydration_effect`. |
| `src/tui/app/async_tasks/lint_runtime.rs::refresh_lint_cache_usage_from_disk` | tokio task, blocking task, disk read | `StartupServices::lint_cache_scan_effect`. |
| `src/tui/state/net.rs::Net::spawn_rate_limit_prime` | thread spawn, network request | `StartupServices::spawn_github_rate_limit_prime`. |
| `src/tui/app/async_tasks/service_handlers.rs::spawn_service_retry` | thread spawn, network retry probe | `StartupServices::spawn_service_retry_probe`. |
| `src/tui/panes/cpu/pane.rs::CpuPane::new` / `CpuPane::reset` | CPU monitor thread spawn, host polling | `StartupServices::cpu_monitor_effect`; quiet startup uses `CpuMonitorSlot::Inert`. |
| `src/tui/running_targets/mod.rs::RunningTargetsPoller::tick` | render-time refresh polling, process table read | `StartupServices::running_targets_polling_effect`. |
| `src/tui/app/async_tasks/config.rs::maybe_reload_themes_from_disk` | theme directory read, process-global mutation | `StartupServices::theme_directory_effect` plus `replace_theme_registry`. |
| `src/tui/app/async_tasks/config.rs::apply_config` / `resolve_and_apply_active_theme` | process-global mutation, lint runtime restart, watcher restart | `StartupServices::install_active_config`, `publish_active_theme`, `spawn_lint_runtime`, and `spawn_watcher`. |
| `src/tui/app/async_tasks/priority_fetch.rs::maybe_priority_fetch` | thread spawn, disk read, network request | `StartupServices::priority_detail_fetch_effect`. |
| `src/tui/app/async_tasks/tree.rs::rescan` -> `scan::spawn_streaming_scan` | thread spawn, disk read, cargo metadata, network-capable scan work | `StartupServices::spawn_streaming_scan`; quiet rescan applies an empty in-memory scan result. |
| `src/tui/app/async_tasks/background_services.rs::schedule_startup_project_details` | rayon worker, disk read, network request | `StartupServices::startup_project_details_effect`. |
| `src/tui/app/async_tasks/background_services.rs::schedule_git_first_commit_refreshes` | thread/rayon worker, local git work | `StartupServices::startup_git_first_commit_effect`. |

**Files:**
- `src/tui/app/construct.rs` — thread the startup profile/services through builder inputs, `run_startup()`, `build()`, and `finish_new()`.
- `src/tui/test_support.rs` — make `make_app()` and `make_app_with_config()` request quiet startup by default; add explicit full-startup or opt-in helper names.
- `src/tui/state/net.rs` — route rate-limit priming and test HTTP construction through startup services; test app construction must not shell out to `gh auth token`.
- `src/tui/app/async_tasks/service_handlers.rs` — ensure retry probes consult the persisted quiet policy.
- `src/tui/app/async_tasks/lint_runtime.rs` — route lint runtime registration, watcher respawn, and lint history/cache work through startup services.
- `src/tui/app/async_tasks/config.rs` — ensure config reload paths consult the persisted policy before restarting watcher or lint runtime work.
- `src/tui/panes/cpu/pane.rs` — make CPU monitor construction/reset use quiet-capable services or inert state.
- `src/themes/paths.rs` — ensure quiet tests use built-ins or fixture-owned theme paths instead of developer-local themes.

**Constraints from prior phases:** Phase 1 converted shared helper setup in `src/test_support.rs` and `src/tui/test_support.rs` from `std::process::abort()` to contextual `expect(...)` panics and added local `clippy::expect_used` allowances. Phase 2 added `scripts/check-no-test-abort.sh`, which must continue to enforce zero abort call lines in those helper files. Any helper failure during this phase should be diagnosable. Do not reintroduce `std::process::abort()` in helpers or rework those helpers except to preserve quiet-startup behavior. If this phase adds or removes any matched abort call line, update `scripts/check-no-test-abort.sh` in the same change set.

**Acceptance gate:** Add tests or assertions proving quiet `make_app()` starts zero watcher, lint runtime, retry probe, rate-limit, lint-history/cache, host `gh`, CPU monitor, theme-directory, and process-global work by default. Run `./scripts/check-no-test-abort.sh` if any abort-bearing or helper files changed. Run `cargo +nightly fmt --all`; run focused App-construction tests including `tui::app::tests::framework_keymap::output_cancel_bindings_clear_output_and_handle_focus`; run `cargo nextest run --workspace --all-features --tests` if the change set is stable enough for full validation.

#### Retrospective

**What worked:**
- `StartupServices` now carries the startup policy through `App` construction and later reload/reset paths.
- Shared `make_app()` and `make_app_with_config()` are quiet by default and assert zero real startup effects.

**What deviated from the plan:**
- The first implementation pass gated initial construction but missed `rescan()` and scan-result startup scheduling; fix pass 1 added `StreamingScan`, `StartupProjectDetails`, and `StartupGitFirstCommit` effects.
- `src/themes/paths.rs` did not need direct changes because quiet startup gates `themes::themes_dir()` at call sites.

**Surprises:**
- Quiet scan-result handling also needed startup-plan changes so suppressed lint-history, disk, metadata, git, crates.io, and detail obligations are not declared.
- Full validation stayed green after adding quiet rescan and scan-result regression tests.

**Implications for remaining phases:**
- Phase 4 should build on the existing `StartupServices`, `StartupEnvironment`, `make_app_with_startup_services`, and `make_app_with_lint_runtime` helpers instead of introducing a parallel profile API.
- Full-startup fixtures still need ownership, serialization, and drain/cancel semantics; Phase 3 only added quiet defaults and one narrow lint-runtime opt-in.
- Later keymap and abort-cleanup phases should keep `make_app()` quiet by default and should not weaken the zero-real-effect assertions.

#### Phase 3 Review

- Phase 4 was narrowed to the remaining work after Phase 3: full-startup opt-in fixtures, disabled watcher semantics, coherent named effect bundles, drain/cancel ownership, and stale builder-stage wording.
- Phase 4 now includes the concrete owners of real startup work left by Phase 3: `src/tui/startup_services.rs`, startup project-detail and git workers, retry probes, priority fetch, streaming scan, and related reload paths.
- Phase 4 explicitly excludes GitHub auth-warning unit coverage unless it adds an injectable auth-gap policy, because `warn_if_github_unauthenticated()` returns early under `cfg(test)`.
- User-approved boundary: Phase 4 creates the shared `TestApp` shell; Phase 5 extends it for custom keymap TOML lifetime rather than creating a separate fixture family.
- Phase 6 now names the Phase 3 quiet scan/rescan regression filters that should stay in its repetition set.
- Phase 7 now preserves quiet App fixtures by default and only relies on spawned-work panics when Phase 4's fixture/drain mechanism observes them.

### Phase 4 — Isolate full-startup opt-in tests and persisted side-effect handles  · status: todo

#### Work Order

**Goal:** Tests that intentionally exercise production startup do so through explicit, isolated fixtures that own and drain their side effects.

**Spec:**
Provide explicit helpers by intent:

- Use the existing `make_app_with_startup_services(...)` helper with `StartupServices::production()` or an owning fixture wrapper for tests that intentionally cover production startup wiring.
- Reuse the existing narrow `make_app_with_lint_runtime(...)` helper where it is enough, and add similarly narrow opt-ins such as `with_watcher` or `with_lint_history` only if broad full startup is unnecessary.
- Direct `App::new()` in unit tests remains reserved for constructor/startup tests.

Full-startup opt-in tests need isolation, not only opt-in names. Put them in a small test group that uses fixture-owned config, keymap, theme, HTTP, watcher, and runtime state; runs serially or behind a repo-local startup test lock when global state cannot be isolated; and drains or cancels all started tasks before teardown.

Do not re-audit quiet startup as open design work. Phase 3 already carries `StartupServices` through `App`, `Panes`, `Net`, `rescan()`, config reload, lint runtime, theme reload, scan-result startup planning, and CPU reset. This phase should focus on the remaining gaps: full-startup opt-in fixtures, disabled watcher semantics beyond a raw no-op `Sender<WatcherMsg>`, and any stale builder-stage wording that implies startup I/O has completed when quiet services intentionally suppressed it.

Quiet handles must remain effective after construction. Keep the side-effect policy or no-op service handles reachable from `App`, `Background`, `Net`, and `Panes` where later spawn paths live. For the watcher specifically, do not treat a raw `Sender<WatcherMsg>` as proof that a real watcher exists; use a `WatcherHandle`/disabled wrapper or service facade so disabled watcher sends and respawns are explicit no-ops.

Define coherent named effect bundles instead of arbitrary partial profiles. `StartupEffects` exposes independent bits, but startup-panel obligations are only coherent when related effects are enabled together. Each opt-in helper should name its intent and include acceptance checks that the startup plan matches the enabled work. For example, a lint-runtime opt-in should not imply project-detail or streaming-scan obligations unless that helper explicitly enables those producers too.

Process-global state is part of the flake surface. `TestApp` or startup fixtures should own config, keymap, and theme override guards, or App-construction tests should run behind a shared serial lock. Quiet startup acceptance should prove deterministic config/theme/keymap state as well as zero spawned tasks.

Create the reusable `TestApp` shell in this phase. It should wrap `App` plus owned config, keymap, theme, startup-service, and task/runtime resources, and expose `Deref`/`DerefMut<Target = App>` so existing tests stay readable. Phase 5 extends this shell for custom keymap TOML lifetime; do not create a separate incompatible keymap fixture family.

Shared test runtime work is part of the flake surface. Startup tasks created during tests must be owned by a fixture with cancellation and drain semantics. Quiet App construction should not schedule onto the global runtime. Full-startup tests should either use an owned runtime/task tracker or explicitly await/cancel all startup tasks before teardown.

Full-startup unit fixtures cannot currently prove the production GitHub auth-warning/status path because `warn_if_github_unauthenticated()` returns early under `cfg(test)`. Exclude that path from Phase 4 unit acceptance unless this phase deliberately adds an injectable auth-gap policy.

If the current `AppBuilder<Started>` typestate name or docs imply "startup I/O complete", update the docs or type names so the stage means "startup policy applied" when disabled/no-op handles are present. Prefer explicit fields such as `watcher: WatcherHandle` over `watch_tx` when disabled startup is representable.

**Files:**
- `src/tui/app/construct.rs` — refine builder state and handle types so disabled/no-op startup is explicit.
- `src/tui/startup_services.rs` — define coherent opt-in effect bundles and any tracked handles needed to drain/cancel real startup work.
- `src/tui/test_support.rs` — add the shared `TestApp` shell plus full-startup/opt-in test fixtures with owned resources, drain/cancel behavior, and any required serialization.
- `src/tui/app/async_tasks/background_services.rs` — include startup project-detail, crates.io, and git first-commit workers in the full-startup ownership/drain decision.
- `src/tui/app/async_tasks/service_handlers.rs` — include retry-probe threads in the full-startup ownership/drain decision.
- `src/tui/app/async_tasks/lint_runtime.rs` — ensure watcher/lint runtime restart paths use persisted handles and are no-op under quiet policy.
- `src/tui/app/async_tasks/config.rs` — ensure reload-triggered restarts use persisted handles and can be drained/disabled in tests.
- `src/tui/app/async_tasks/priority_fetch.rs` — include priority detail fetches in the tracked/suppressed/out-of-scope decision for full-startup fixtures.
- `src/tui/state/net.rs` — keep network/test HTTP side effects owned by fixture or no-op service handles.
- `src/tui/panes/cpu/pane.rs` — ensure CPU monitor reset remains disabled under quiet policy and explicit under full startup.
- `src/themes/paths.rs` — ensure theme test override lifetime is fixture-owned for full-startup tests.

**Constraints from prior phases:** Phase 2 added `scripts/check-no-test-abort.sh` and protects `src/test_support.rs` plus `src/tui/test_support.rs` as zero-abort helper files. Phase 3 made quiet startup the default for shared unit-test constructors and introduced `StartupServices`, `StartupEnvironment`, `make_app_with_startup_services`, and `make_app_with_lint_runtime`. Phase 3 also gates quiet `rescan()` and scan-result startup scheduling through persisted services. This phase preserves that contract while adding explicit full-startup fixtures and lifetime/teardown discipline.

**Acceptance gate:** Tests prove quiet policy persists after construction: reload/reset paths cannot respawn watcher, lint runtime, retry probes, rate-limit priming, lint hydration, host auth, CPU monitor, streaming scan, project-detail workers, git first-commit refreshes, priority fetch, or theme-directory work unless the test opted in. Full-startup fixture tests prove expected effects happen and are drained/cancelled, or are explicitly documented as outside unit-fixture ownership. Run `./scripts/check-no-test-abort.sh` if any helper setup or abort-bearing Rust files changed. Run `cargo +nightly fmt --all` and targeted startup/reload tests.

### Phase 5 — Fix keymap test lifetime hazards  · status: todo

#### Work Order

**Goal:** Keymap-backed tests cannot return an `App` that points at deleted temp keymap paths or dropped override guards.

**Spec:**
The framework keymap tests use temporary keymap files through helpers such as `make_app_with_keymap_toml`. The helper must not return an `App` that stores a path to a temporary directory that has already been deleted or an override guard that has already been restored.

Preferred fixes, in order:

1. Return an owning fixture such as `TestApp` or `KeymapFixture` that keeps the temp directory or kept temp file alive for at least as long as the `App`.
2. Store custom keymap TOML in a real kept temp file or owning fixture; do not copy the leaked `TempDir` pattern from `test_keymap_path()`.
3. For tests that only need parse-time behavior, load from an explicit TOML string and avoid storing a fake path in `App`.

Audit all direct uses of `override_keymap_path_for_test`, not just helper functions. Any test that exercises reload or migration through `app.keymap.path()` must keep the backing file alive for the full operation.

Add a regression test showing that an app returned by the keymap helper can still reload from `app.keymap.path()` after the helper returns.

Keep the fixture narrow. Generic `make_app()` should still return `App`; only custom keymap-path helpers should return the owning fixture. Give the fixture `Deref` and `DerefMut<Target = App>` so existing test bodies stay readable, and avoid an easy `into_app()` escape hatch for tests that rely on reload or migration paths.

**Files:**
- `src/tui/app/mod.rs` — update embedded framework-keymap helpers such as `make_app_with_keymap_toml`, `make_app_with_config_and_keymap_toml`, direct keymap override uses, and the originally observed framework-keymap test area.
- `src/tui/keymap/load.rs` — preserve override guard APIs as needed; update only if fixture ownership requires helper API adjustment.
- `src/tui/test_support.rs` — reuse kept temp-file patterns or fixture ownership if shared helper support is needed.

**Constraints from prior phases:** Phase 2 added `scripts/check-no-test-abort.sh`; this phase touches `src/tui/app/mod.rs`, which is an allowlisted abort bucket, so inventory counts must not drift accidentally. If this phase adds or removes any matched abort call line, update `scripts/check-no-test-abort.sh` in the same change set. Shared `make_app()` is quiet by default, startup side effects are fixture-aware, and quiet rescan/scan-result handling must stay zero-real-effect. Phase 4 created the shared `TestApp` shell; extend it for custom keymap TOML lifetime instead of creating a separate incompatible fixture family. Keymap fixtures should build on that quiet default; do not broaden generic `make_app()` return types.

**Acceptance gate:** A regression test proves an app produced by the keymap helper can still reload from `app.keymap.path()` after helper return. Run `./scripts/check-no-test-abort.sh`; run `cargo +nightly fmt --all`; run the framework-keymap test module or the originally observed focused test; run the fixed repetition command if available from Phase 6, otherwise repeat the focused test manually enough to exercise late exits.

### Phase 6 — Add flake repetition gates and tighten existing abort inventory  · status: todo

#### Work Order

**Goal:** Turn the stabilized suspect tests into repeatable flake gates and tighten the Phase 2 abort inventory after quiet-startup and keymap lifetime fixes.

**Spec:**
Use the Phase 2 `scripts/check-no-test-abort.sh` gate as the executable source of truth. If Phases 3 through 5 removed any abort buckets, update the allowlist in this phase so removed sites cannot reappear silently. The gate must continue to fail on any abort in `src/test_support.rs` or `src/tui/test_support.rs`.

Add a documented repetition command for the original flake trigger and any focused tests changed by Phases 3 through 5. The command must be practical for local use and must fail on the first nonzero nextest run. A simple shell loop is acceptable if the repo does not already have a repeat-test helper.

The original observed trigger was:

```text
tui::app::tests::framework_keymap::output_cancel_bindings_clear_output_and_handle_focus
```

Every focused repetition gate must first prove that the filter matches at least one test. If the intended filter matches zero tests, replace it with a confirmed relevant test filter or document the fallback command.

**Files:**
- `scripts/check-no-test-abort.sh` — tighten allowlist after earlier removals.
- `docs/fix-SIGABRT.md` — record the exact repetition command and any inventory drift discovered during implementation.
- Add a small helper script only if that is clearer than documenting a command inline.

**Constraints from prior phases:** Phase 1 converted `src/test_support.rs` and `src/tui/test_support.rs` to contextual panics. Phase 2 installed the abort inventory gate. Phase 3 added quiet startup services plus quiet rescan/scan-result gating. Phases 3 through 5 may have removed startup/keymap abort triggers. Do not reintroduce test-helper aborts. New test setup `expect(...)` calls need scoped test-only lint allowances or contextual panic alternatives. If this phase adds or removes any matched abort call line, update `scripts/check-no-test-abort.sh` in the same change set.

**Acceptance gate:** `./scripts/check-no-test-abort.sh`; the documented repetition command for the original keymap flake with a filter that matches at least one test; `cargo nextest run --workspace --all-features --tests` if the repetition command exposes broader startup coupling.

Phase 3 regression filters to include in the repetition command or documented focused subset:

- `tui::app::tests::quiet_scan_result_does_not_start_startup_workers_or_wait_for_lint_history`
- `tui::app::tests::quiet_rescan_uses_noop_scan_without_real_startup_effects`
- `tui::app::tests::quiet_completed_scan_applies_noop_rescan_when_enabling_non_rust_without_cached_projects`

### Phase 7 — Convert remaining test-module abort buckets  · status: todo

#### Work Order

**Goal:** All test modules and shared test helpers report contextual panics or unreachable states instead of calling `std::process::abort()`.

**Spec:**
After shared-helper cleanup, quiet `App` construction, keymap fixture work, and the executable inventory are in place, convert test modules in focused batches so review and rollback stay easy. This order prevents the largest abort sweep from running while the original "test passed, process later aborted" flake surface is still active.

Recommended order:

1. `src/tui/app/mod.rs`
2. `tui_pane/src/toasts/mod.rs`
3. `tui_pane/src/toasts/render/mod.rs`
4. `src/scan/tree/mod.rs`
5. `src/scan/disk_usage.rs`
6. `src/project/git/discovery.rs`
7. `src/tui/panes/ci/render.rs`
8. `src/tui/running_targets/app_tick.rs`

Mechanical replacements:

- `unwrap_or_else(|_| std::process::abort())` becomes `expect("specific context")`.
- `unwrap_or_else(|| std::process::abort())` becomes `expect("specific context")`.
- `let Some(x) = maybe else { std::process::abort() };` becomes a `panic!` with context.
- Impossible enum variants in tests become `unreachable!("specific context")`.

Do not do one blind repo-wide replacement. Use an allowlisted inventory by file/module:

1. Generate the current abort inventory by path.
2. Automate only exact test-side shapes such as `unwrap_or_else(|_| std::process::abort())` and `unwrap_or_else(|| std::process::abort())`.
3. Manually handle `let else`, enum variants, and places where the panic message needs domain context.
4. Re-run the inventory after each bucket and record the remaining paths.

Keep messages short but specific. They should name the expected test condition, not restate the type operation.

For every converted abort inside spawned closures, check whether a panic would be observed by the originating test. Test-owned spawned work should return a `JoinHandle`, send errors over a channel, or be drained by the fixture so failures are attributed to the correct test.

**Files:**
- `src/tui/app/mod.rs` — largest abort bucket, including framework-keymap tests and many embedded TUI tests.
- `tui_pane/src/toasts/mod.rs` — toast behavior tests.
- `tui_pane/src/toasts/render/mod.rs` — toast render tests.
- `src/scan/tree/mod.rs` — scan tree tests.
- `src/scan/disk_usage.rs` — disk usage tests.
- `src/project/git/discovery.rs` — git discovery tests.
- `src/tui/panes/ci/render.rs` — CI render tests.
- `src/tui/running_targets/app_tick.rs` — running-target tests.
- `scripts/check-no-test-abort.sh` — tighten temporary allowlist as buckets are converted.

**Constraints from prior phases:** Phase 2 provides the executable inventory gate, and Phase 6 tightens its allowlist after startup/keymap stabilization. Phase 3 and Phase 4 ensure `App` tests use deterministic quiet startup unless explicitly opted in. Phase 5 ensures keymap helpers own their backing paths. The `src/tui/app/mod.rs` abort sweep must preserve quiet fixtures by default and must not rely on panics inside spawned work unless Phase 4's fixture/drain mechanism observes them. Each converted abort bucket must update `scripts/check-no-test-abort.sh` in the same change set so the gate records the new remaining count.

**Acceptance gate:** `./scripts/check-no-test-abort.sh` reports no test-module or shared-test-helper aborts remain; `cargo +nightly fmt --all`; `cargo nextest run --workspace --all-features --tests`; the fixed repetition command passes.

### Phase 8 — Review and remove or document the remaining production abort  · status: todo

#### Work Order

**Goal:** The remaining production-looking abort in running-toasts is either made unrepresentable or explicitly documented as a deliberate production invariant.

**Spec:**
Review the production-looking invariant at `src/tui/app/async_tasks/running_toasts.rs`.

The current code uses `std::process::abort()` for an internal invariant in the `ReactivateOutcome::Revived` branch. Decide whether it should remain a hard abort, become `unreachable!`, or be expressed by restructuring the match so the invariant is encoded directly.

Prefer making the state unrepresentable locally. For example, bind the toast task id in the same branch that can produce `ReactivateOutcome::Revived`, by matching on `toast_slot` first or matching `(toast_slot, outcome)`.

Expected outcomes:

- If it stays hard-fail, the reason is documented and `scripts/check-no-test-abort.sh` allowlists the exact reviewed production site. Do not leave the production allowance as path/count only if the abort remains intentional.
- If it can be made unrepresentable, prefer that over a runtime abort and remove the allowlist entry.

**Files:**
- `src/tui/app/async_tasks/running_toasts.rs` — restructure or document the `ReactivateOutcome::Revived` invariant.
- `scripts/check-no-test-abort.sh` — remove or narrow the production abort allowlist to match the final decision.

**Constraints from prior phases:** Phase 7 removed test/helper aborts. The inventory script now distinguishes reviewed production invariants from forbidden test aborts.

**Acceptance gate:** `./scripts/check-no-test-abort.sh`; `cargo +nightly fmt --all`; targeted running-toasts tests if present; `cargo nextest run --workspace --all-features --tests`.

### Phase 9 — Final validation and workflow handoff  · status: todo

#### Work Order

**Goal:** The full repo validates with deterministic tests and no hidden test-side process aborts.

**Spec:**
Run the complete validation sequence and update this plan only if a remaining Work Order needs forward-propagated facts from validation.

Validation order:

1. `cargo +nightly fmt --all`
2. `./scripts/check-no-test-abort.sh`
3. fixed repetition command for the originally observed keymap test and App-construction-heavy subset
4. `taplo fmt --check`
5. `cargo clippy --workspace --all-targets --all-features -- -D warnings`
6. `cargo mend --workspace --all-targets --fail-on-warn`
7. `cargo build --release --all-features --workspace --examples`
8. `cargo nextest run --workspace --all-features --tests`
9. the existing `validate_and_push` workflow after all phases are complete, if publishing the branch is intended

The goal is not only that the suite passes, but that any future failure reports a normal panic with useful context rather than `SIGABRT`.

**Files:**
- `docs/fix-SIGABRT.md` — update only if validation changes remaining-phase constraints or records final phase outcome.
- `scripts/check-no-test-abort.sh` — final gate should allow no test/helper aborts and only reviewed production aborts, if any remain.

**Constraints from prior phases:** Phase 8 settled the final production abort. Phase 7 removed test/helper aborts. Quiet startup, keymap fixtures, and full-startup isolation are in place.

**Acceptance gate:** Full validation sequence above passes. `cargo nextest run --workspace --all-features --tests` passes, and any induced failure in helper/setup paths reports panic context instead of `SIGABRT`.
