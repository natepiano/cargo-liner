# `src/` reorganization plan

`src/` is organized by where code landed historically, not by what each
module is. `tui/` holds ~65% of the codebase and mixes application
state with terminal rendering. `http/` knows about GitHub-specific
response types. `ci.rs` merges domain identifiers, GitHub wire types,
and fetch lifecycle. Top-level utility files compete visually with
real layers.

This plan introduces four organizing principles, derives a target
top-level structure from them, and stages the moves into phases that
each ship as a single commit.

## Prerequisite

**This plan does not start until `docs/tui-pane-extraction.md` is
complete.** That extraction moves generic TUI primitives out of
`src/tui/` and into the `tui_pane/` workspace crate over five phases.
Per the extraction plan's target layout, the following items land at
these destinations (everything goes to `tui_pane/` crate root — there
is no `support/`, `chrome/`, or `widgets/` subtree inside `tui_pane/`):

| Item | Destination |
|------|-------------|
| `format_progressive` (from `tui/support/duration.rs`) | `tui_pane/src/util.rs` |
| `tui/support/watched_file.rs` | `tui_pane/src/watched_file.rs` |
| `tui/support/running_tracker.rs` | `tui_pane/src/running_tracker.rs` |
| `tui/pane/chrome.rs` | `tui_pane/src/pane_chrome.rs` |
| `tui/pane/title.rs` | `tui_pane/src/pane_title.rs` |
| `tui/pane/state.rs` | `tui_pane/src/pane_state.rs` |
| `tui/pane/layout.rs` | `tui_pane/src/layout.rs` |
| `tui/pane/rules.rs` | `tui_pane/src/rules.rs` |
| `tui/columns/widths.rs` | `tui_pane/src/table/widths.rs` |
| `tui/overlays/popup.rs` | `tui_pane/src/popup.rs` |
| generic palette + dimensions from `tui/constants.rs` | `tui_pane/src/constants.rs` |
| `Hittable` trait (from `tui/pane/dispatch.rs`) | `tui_pane/src/dispatch/mod.rs` |
| `Pane` trait (renamed `Renderable`) | `tui_pane/src/dispatch/mod.rs` |
| `HitTestRegistry` + `hit_test_at` (new) | `tui_pane/src/dispatch/hit_test.rs` |
| `PaneRegistry` + `render_panes` (new) | `tui_pane/src/dispatch/render.rs` |

Two structural changes the extraction also performs (not file moves,
but they affect what cargo-port code looks like when this reorg
starts):

- `LayoutCache` (in `src/tui/panes/layout.rs`) is deleted.
  `tiled_layout: Option<ResolvedPaneLayout<AppPaneId>>` moves onto
  `tui_pane::Framework` with `tiled_layout()` / `set_tiled_layout()`
  accessors. `project_list_body: Rect` moves onto `ProjectListPane`
  as `body_rect`. The `App.layout_cache` field is gone.
- Every cargo-port pane that previously had a stub `impl Pane` (or
  bypassed the trait via a free render function) has a real
  `impl tui_pane::Renderable<PaneRenderCtx<'_>>` body. Cargo-port's
  render orchestration is one call to `tui_pane::render_panes`
  followed by `tui_pane::render_toasts`.

Files that **stay in cargo-port** after the extraction:

- `tui/cpu.rs` — CPU sampler stays in the app crate. Phase 9 below
  moves it to `app/cpu.rs`.
- `tui/overlays/render_state.rs` — the `FinderPane` viewport-wrapper
  struct is app-owned.
- `tui/overlays/pane_impls.rs` — `KeymapPane` / `SettingsPane` /
  `FinderPane` impl blocks live here.
- `tui/interaction.rs` — `hit_test_toasts` stays as the pre-pass that
  runs before the generic `tui_pane::hit_test_at` dispatch.
  `HittableId` (Toasts variant removed) and `HITTABLE_Z_ORDER` also
  stay.
- `tui/pane/dismiss.rs` — `DismissTarget` enum.
- `tui/pane/mod.rs` — declares `dismiss` plus re-exports of `tui_pane`
  items for call-site convenience. `tui/pane/dispatch.rs` is deleted
  by extraction Phase 5 (every type it held either moved or was
  deleted as dead).
- App-specific entries in `tui/constants.rs` (popup dims, startup
  phase labels).

Running this reorg against an incomplete extraction would either
duplicate the moves or leave files orphaned in two trees. All file
paths and phase scopes below assume the post-extraction state of
`src/tui/`.

## Principles

### 1. Single-direction dependency paths

Every module sits on a tier. A module on tier *N* may import from tiers
*N+1*, *N+2*, … (the layers below it). It must not import sideways at
its own tier, and it must never import upward. When two modules at the
same tier need to share a type, that type moves *down* a tier until
both callers can see it.

Tier order, top to bottom:

```
main
  └─ app   tui
       └─ project   lint   watcher   ci   scan
            └─ service::{github, crates_io}
                 ├─ http
                 └─ support
```

The `service/` subtree holds the service tier as a single unit: the
`Service` identity enum at `service/mod.rs`, and each service client
as a submodule (`service::github`, `service::crates_io`). Submodules
import their identity from `super::Service`, their transport from
`crate::http::*`, and constants from `crate::support::*`.

### 2. Domain vs. service vs. transport

Three concepts have been conflated and need three homes:

- **Transport** — generic HTTP: requests, retries, rate-limit header
  parsing. Does not know which upstream a request is for. Lives in
  `http/` (split internally into `client.rs`, `retry.rs`,
  `rate_limit.rs`).
- **Service** — the identity enum that names all upstreams, plus the
  API clients that implement each one. The `Service` enum lives in
  `service/mod.rs` (variants: `GitHub`, `CratesIo`). Each client is a
  submodule (`service::github`, `service::crates_io`) that knows
  endpoints, wire types, and upstream-specific rate-limit semantics.
  Clients depend on `http/` for transport and `super::Service` for
  identity.
- **Domain** — what the application represents to the user: a CI status,
  a project identifier, a fetch lifecycle. Does not know HTTP exists;
  receives data via `BackgroundMsg`. Lives in `ci/` (and the existing
  `project/`, `lint/`, etc.).

Reader-facing test: someone reading `ci/` should not be able to tell
which API the data came from. Someone reading `service::github`
should not be able to tell which UI shows it.

### 3. Support heuristic

A module belongs in `support/` if it represents no concept the
architecture has a name for — it is ambient plumbing the rest of the
program reaches for without thinking about it as a "layer."

Primary test: when code uses this module, does the reader need to know
what *concept* it represents, or just what *utility* it provides?

Secondary test: a support module is **purely depended upon**. It has no
peer relationships and no horizontal coupling. The moment a module
starts being part of a tier with siblings, it is a layer, not support.

Concrete constraint to keep `support/` from drifting into a junk drawer:
each support submodule must export **at most ~2 public items** and must be
**unit-testable in isolation** (no shared application state, no setup
beyond construction). A utility that needs more API surface or that pulls
in app state to test is a layer, not support.

By this rule:

- `cache_paths`, `constants`, `perf_log`, `test_support` → `support/`.
- `keymap`, `http`, `ci`, `project`, `scan`, `config` → named layer,
  stays top-level.
- `config.rs` is a named layer but at 1144 lines exceeds the threshold
  we just used to split `git/state.rs`. It becomes `config/`.

### 4. App domain vs. terminal domain

This is a **code organization** principle. The codebase has two
domains that have grown tangled, and this plan separates them:

- **App domain.** Program state, lifecycle, async tasks, navigation,
  background work, target index, project list state, watcher
  integration, action vocabulary. This is what the program *is*.
- **Terminal domain.** Render, panes, pane chrome, overlays, columns,
  terminal setup, input capture, mouse hit-testing, scroll viewports.
  This is how the program is *presented and operated*.

The criterion for assigning a file is which domain it serves, not
which crate it currently imports. A field that holds a
`ratatui::Position` to track mouse hit-tests belongs in the terminal
domain even if it lives on `App` today. A file that holds project
state but happens to live next to render code belongs in the app
domain.

The boundary becomes **compiler-enforceable** in Phase 8 via a set
of composed `*View` traits — one per concern (`ProjectListView`,
`FocusView`, `CiStatusView`, `LintView`, `OverlayView`, etc.). Each
trait holds both `&self` (read) and `&mut self` (write) methods for
its concern. Render code takes `&impl ProjectListView` and can only
read; event handlers take `&mut impl ProjectListView` and can read
or write. Rust's borrow checker enforces the read/write split — no
need for separate read traits and write traits.

After Phase 8, `tui/` never reads or writes `App` fields directly —
everything goes through the traits. The directory move in Phase 9
is then mechanical, not a refactor.

The interface is one-directional from `tui`'s side: `tui` reads and
mutates `app/` state through the trait family; `app/` never reaches
into `tui/`. `main.rs` calls `app::run()`; `app::run()` constructs
state, then hands off to `tui::run(...)` to drive the terminal loop.

## Target top-level structure

```
src/
  main.rs                  entry — calls app::run()
  app/                     program (no terminal deps)
  tui/                     terminal rendering + input
  project/                 domain: projects, members, paths, git, cargo
  scan/                    per-tree and per-entry probes (absorbs enrichment)
  lint/                    domain: lint runs and history
  watcher/                 filesystem watching
  ci/                      domain: CI status per project
  service/                 service tier: identity enum + per-upstream clients
    mod.rs                   pub(crate) enum Service { GitHub, CratesIo }
    github/                  GitHub REST + GraphQL client, wire types, OwnerRepo
    crates_io/               crates.io client, wire types
  http/                    transport: generic HTTP, split into 4 files
    mod.rs
    client.rs
    retry.rs
    rate_limit.rs
  keymap/                  domain: key bindings
  config/                  configuration (split from config.rs)
  support/                 ambient utilities
```

## Boundary enforcement

The tier rule (Principle 1) is upheld by three mechanisms. Be
clear on what each enforces — only one of them is compile-time:

1. **Directory structure.** Each tier lives in its own subtree. A
   reader can scan the imports of any module and see whether it
   reaches the right direction. This is *organizational*, not
   enforced.

2. **Visibility.** `pub(crate)` is the default. `pub` escapes the
   crate (today, none). `pub(super)` keeps types from leaking
   sideways inside nested modules. Note: `pub(crate)` does **not**
   enforce the tier rule — it only limits the surface to "any
   module in this crate." A `pub(crate)` item in `service/` can be
   imported from `http/` and the compiler will accept it. Visibility
   is about *minimizing surface*, not directing it.

3. **CI grep check.** This is the only mechanism that actually
   enforces the tier rule. A permanent script verifies that no
   module reaches into a tier above it. The first check guards
   `http/`:

   ```bash
   rg "use crate::(ci|service|project|lint|watcher|scan|app|tui|config|keymap)" src/http/
   ```

   If the command returns any match, the build fails. The script
   lives at `scripts/check-tier-boundaries.sh` and runs alongside
   `cargo mend` (or as a dedicated `make tier-check` target).

The same `rg` pattern can extend to any tier boundary as the reorg
progresses. After Phase 4, a second check guards `service/`:

```bash
rg "use crate::(ci|project|lint|watcher|scan|app|tui|config|keymap)" src/service/
```

A workspace crate (e.g. `cargo-port-http`) was considered for
stronger enforcement but rejected: the conversion is reversible —
Phase 4 leaves `http/` with a clean API, so lifting it to a crate
later is a small follow-up — and the grep check gives
compile-time-equivalent enforcement at zero coordination cost.

## Phase overview

| Phase | What | Risk | Rough size |
|-------|------|------|------------|
| 1 | Move `enrichment.rs` into `scan/` | Low | 1 file moved, ~5 call sites |
| 2 | Collect `support/` (cache_paths, perf_log, constants, test_support) | Low | 4 files moved, import sweep |
| 3 | Split `ci.rs`; create `service/github/` skeleton; `OwnerRepo` moves with validation | Medium | 1 file split, ~13 call sites handle new `Result` return |
| 4 | Create `service/mod.rs` (Service enum); move clients into `service/github/` and `service/crates_io/`; split `http/` into 4 files | Medium | 2 service clients populated, `http/` directory restructured |
| 5 | Split `BackgroundMsg` into per-service nested enums (`GitHub(GitHubMsg)`, `CratesIo(CratesIoMsg)`) | Medium | ~24 match arms rewritten |
| 6 | Split `config.rs` → `config/` | Medium | 1 file split into domain submodules |
| 7 | Probe `app ↔ tui` coupling; record findings | None (read-only) | One commit-less investigation |
| 8 | Introduce composed `*View` traits at the app↔tui boundary | Medium | 5–8 traits in `view/`, render call-sites updated |
| 9 | Promote `tui/app/` → top-level `app/`, move non-terminal modules out of `tui/` | Medium | ~20+ files moved (mechanical after Phase 8) |

Each implementation phase lands as a single commit after `cargo build
&& cargo nextest run && cargo clippy && cargo mend && cargo +nightly
fmt` all pass green.

## Phase 1 — Move `enrichment.rs` into `scan/`

`enrichment.rs` is a single 71-line function that calls
`scan::emit_git_info`, `scan::dir_size`, `scan::collect_language_stats_single`,
and `ctx.client.fetch_crates_io_info`. It is the per-entry counterpart
to `scan`'s per-tree bulk passes — a `scan` concept, not a peer.

### What moves

- `src/enrichment.rs` → `src/scan/enrichment.rs`
- Re-export `enrich` and `spawn_language_scan` from `src/scan/mod.rs`.

### Call-site updates

- `crate::enrichment::enrich` → `crate::scan::enrich`
- `crate::enrichment::spawn_language_scan` → `crate::scan::spawn_language_scan`
- Remove `mod enrichment;` from `src/main.rs`.

### Risks

None beyond the import sweep. No logic changes.

## Phase 2 — Collect `support/`

Move ambient utilities into a single `support/` directory so the
top-level listing reflects layers, not file sizes.

### What moves

- `src/cache_paths.rs` → `src/support/cache_paths.rs`
- `src/perf_log.rs` → `src/support/perf_log.rs`
- `src/constants.rs` → `src/support/constants.rs`
- `src/test_support.rs` → `src/support/test_support.rs`

### Call-site updates

- `crate::cache_paths::` → `crate::support::cache_paths::`
- `crate::perf_log::` → `crate::support::perf_log::`
- `crate::constants::` → `crate::support::constants::`
- `crate::test_support::` → `crate::support::test_support::`

`src/main.rs` replaces four `mod` lines with `mod support;` and an
internal `support/mod.rs` declares the four submodules.

### Risks

Wide import sweep — `constants` and `perf_log` are imported across
nearly every module. A find-and-replace pass plus a clean build will
catch every site.

## Phase 3 — Split `ci.rs`; create `service/github/` skeleton

`ci.rs` currently merges five concerns. They sort into two
destinations:

- **Move down to `service/github/`** (service tier):
  - `GhRun`, `GqlCheckRun` — GitHub Actions API wire types.
  - `OwnerRepo` — the `(owner, repo)` identifier. It matches the
    GitHub URL convention `github.com/<owner>/<repo>` and is used by
    every consumer above the service tier (`ci`, `scan`, `app`,
    `tui`), so it must live at or below the lowest tier that all
    consumers can reach. `service/github/` is that tier.

- **Stay in `ci/`** (domain tier):
  - `FetchStatus` — fetch lifecycle flag.
  - `CiStatus`, `CiRun`, `CiJob` — domain types the UI displays.
  - `build_ci_run()` — wire→domain builder (imports
    `crate::service::github`, which is allowed: `ci → service` is a
    downward import).

### `OwnerRepo` gains validation in this phase

The current `OwnerRepo::new(owner: impl Into<String>, repo: impl Into<String>) -> Self`
constructor accepts empty strings. The move to `service/github/` is
paired with a signature change to `Result` with a typed error,
matching the house pattern (`CargoMetadataError` and similar —
hand-written enums, no `thiserror`):

```rust
#[derive(Clone, Debug)]
pub(crate) enum OwnerRepoError {
    EmptyOwner,
    EmptyRepo,
    WhitespaceOwner,
    WhitespaceRepo,
}

impl OwnerRepo {
    pub(crate) fn new(
        owner: impl Into<String>,
        repo:  impl Into<String>,
    ) -> Result<Self, OwnerRepoError> {
        // validate, return Err(...) on failure
    }
}
```

All ~13 call sites update to handle the `Result`. Call-site patterns:

- **Parsing upstream data** (most sites): propagate with `?`,
  surfacing the rejection reason in logs.
- **Test fixtures**: use `.unwrap_or_else(|_| std::process::abort())`
  — `.expect()` is a style violation. Hardcoded literals should not
  appear in non-test code.

### What else moves

- `GhRun`, `GqlCheckRun`, their `Deserialize` impls, and the
  `OwnerRepo` newtype move to a new `src/service/github/wire.rs`
  (re-exported from `src/service/github/mod.rs`; the client code
  follows in Phase 4).
- `src/service/mod.rs` is created as an empty namespace file with
  `pub(crate) mod github;`. The `Service` enum is *not* added until
  Phase 4 — leaving it out here keeps Phase 3 focused on the ci
  split.
- `FetchStatus`, `CiStatus`, `CiRun`, `CiJob` stay in `src/ci/`
  (formerly `ci.rs`, now a directory). `ci/mod.rs` re-exports them.
- `build_ci_run()` moves to `src/ci/builder.rs` and imports
  `crate::service::github::{GhRun, GqlCheckRun}`.
- `http/mod.rs` stops importing `super::ci::GhRun` /
  `super::ci::GqlCheckRun`; it imports from
  `crate::service::github::` instead for the duration of Phase 3
  only — Phase 4 will move the importer out of `http/` entirely.

### Risks

The temporary `http → service` import in Phase 3 violates the tier
rule for one phase. Phase 4 closes the violation by moving the
consuming code into `service/github/`.

### Phase 3 gating

To prevent the temporary violation from outliving Phase 3:

- The Phase 3 commit message must include the line
  `TIER VIOLATION: http→service temporary until Phase 4`.
- A CI grep check (`rg "use crate::service" src/http/`) must return
  zero results before Phase 4 is merged.
- Phase 3 must not be merged unless a Phase 4 PR is open. If Phase 4
  is delayed beyond a week, Phase 3 is reverted rather than left in
  place.

## Phase 4 prerequisites

Before Phase 4 starts, one preparatory item must land:

### Rate-limit boundary split

`http/mod.rs` today mixes generic retry/backoff with GitHub-specific
bucket dispatch (`RateLimitBucket::GraphQl` and friends). Today
`http/mod.rs` calls GitHub-specific bucket logic directly from its
generic retry path — Phase 4 cannot move that logic out without first
introducing a seam.

The pre-step splits two functions:

- `parse_rate_limit_headers_generic()` — returns a generic
  `RateLimitQuota` from headers. Stays in `http/`. No knowledge of
  GitHub buckets.
- `parse_rate_limit_response()` — handles GitHub-specific GraphQL
  errors and bucket routing. Moves to `service/github/` in Phase 4.

This refactor ships as a separate commit before Phase 4. It is not
numbered as its own phase because it has no independent goal — it
exists solely to make Phase 4's move clean. The commit is small
(splitting one function into two) and lands immediately before the
Phase 4 commit on the same branch.


## Phase 4 — Populate `service/` clients; split `http/` into 4 files

`http/mod.rs` today contains the generic transport, the GitHub
REST/GraphQL clients, the crates.io client, and rate-limit handling
that mixes generic logic with GitHub-bucket-specific logic. Phase 4
extracts the per-upstream code into `service/{github, crates_io}/`,
adds the `Service` enum, and splits the remaining transport code
into named submodules.

### `service/mod.rs` gets the `Service` enum

```rust
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub(crate) enum Service {
    GitHub,
    CratesIo,
}

impl Service {
    pub(crate) const fn probe_url(self) -> &'static str {
        match self {
            Self::GitHub   => crate::support::constants::GITHUB_API_BASE,
            Self::CratesIo => crate::support::constants::CRATES_IO_API_BASE,
        }
    }
}

pub(crate) mod github;
pub(crate) mod crates_io;
```

The `crate::support::constants::*` paths assume Phase 2 has already
moved `constants.rs` into `support/`. Phase 4 runs after Phase 2,
so this is consistent.

The current `ServiceKind` enum in `http/mod.rs` is deleted; every
call site imports `crate::service::Service` instead. The rename
(`ServiceKind` → `Service`) is part of this phase.

### What moves

- GitHub REST and GraphQL client functions → `src/service/github/client.rs`.
- GitHub-specific rate-limit bucket logic (`RateLimitBucket::GraphQl`
  distinction, the `parse_rate_limit_response()` helper from the
  Phase 4 prerequisite) → `src/service/github/rate_limit.rs`.
- crates.io client functions → `src/service/crates_io/client.rs`.
- Generic transport stays in `http/`, split into:
  - `http/mod.rs` — re-exports
  - `http/client.rs` — `Client` construction (reqwest wrapper)
  - `http/retry.rs` — retry/backoff helpers
  - `http/rate_limit.rs` — generic header parsing + `RateLimitQuota`
    (the `parse_rate_limit_headers_generic()` helper from the
    Phase 4 prerequisite lives here)

### Resulting dependency graph

```
service::github      service::crates_io
        ↓                    ↓
        └─ super::Service ───┘
                ↓
              http
                ↓
            support
```

`http/` after this phase has no upward imports. Service clients
import `super::Service` for identity and `crate::http::*` for
transport.

### `http/` public API after Phase 4

The post-Phase-4 boundary that `service/` consumers may import:

- `http::Client` — reqwest wrapper, retries, generic backoff.
- `http::HttpOutcome<T>` — the outcome type for fallible calls.
- `http::ServiceSignal` — generic up/down/rate-limited signal.
- `http::RateLimitQuota` — parsed generic rate-limit headers.
- `http::parse_rate_limit_headers_generic()` — header parser
  returning `RateLimitQuota`.

Items that **leave** `http/` in Phase 4:

- `GhRun`, `GqlCheckRun`, `GhRunsList`, `GitHubRateLimit`,
  `RateLimitBucket`, `parse_rate_limit_response()`,
  `github_is_rate_limited()`, `graphql_body_is_rate_limited()`
  → `service/github/`.
- `CratesIoInfo`, `RepoMetaInfo` and any crates.io wire types
  → `service/crates_io/`.
- `ServiceKind` enum → `service/mod.rs` (renamed `Service`).

### Module declarations

- `src/main.rs` adds `mod service;` and `mod http;` survives as-is.
- `src/service/mod.rs` declares `pub(crate) mod github;` and
  `pub(crate) mod crates_io;`.
- `src/http/mod.rs` declares `mod client; mod retry; mod rate_limit;`
  and re-exports the items listed above.

### Risks

- `GitHubRateLimit` and the GraphQL bucket distinction depend on
  GitHub internals. Confirmed home: `service/github/rate_limit.rs`.
  `http/rate_limit.rs` retains generic `RateLimitQuota` only.
- The `ServiceKind` → `Service` rename touches every site that
  pattern-matches on the enum. Use a single search-and-replace
  pass at the top of the commit so the diff stays readable.

## Phase 5 — Split `BackgroundMsg` into per-service nested enums

`BackgroundMsg` (in `src/scan/mod.rs`) is a ~24-variant grab-bag.
After Phase 4, both `service::github` and `service::crates_io` send
into it. Today nothing prevents `service::github` from constructing
a crates.io variant or a domain-only variant. This phase introduces
nested enums so the compiler enforces the service boundary.

### What ships

- `src/service/github/mod.rs` defines `pub(crate) enum GitHubMsg`
  with every variant currently sent by GitHub-side code (`CiRuns`,
  `RepoFetchQueued`, `RepoFetchComplete`, `RepoMeta`).
- `src/service/crates_io/mod.rs` defines `pub(crate) enum
  CratesIoMsg` with crates.io-side variants (`Version`).
- `src/scan/mod.rs` collapses those flat variants into two outer
  arms:

  ```rust
  pub(crate) enum BackgroundMsg {
      // domain
      ScanResult { .. },
      ProjectDiscovered { .. },
      CheckoutInfo { .. },
      RepoInfo { .. },
      DiskUsage { .. },
      LanguageStatsBatch { .. },
      // service-driven
      GitHub(GitHubMsg),
      CratesIo(CratesIoMsg),
  }
  ```

- Every match site updates to the nested form:
  `BackgroundMsg::CiRuns { .. }` becomes
  `BackgroundMsg::GitHub(GitHubMsg::CiRuns { .. })`.

### Why a dedicated phase

This isn't part of Phase 4 because Phase 4 is already substantial
(service enum, client moves, http split). Folding the message-bus
restructure into the same commit muddies the diff and blocks
bisection. A separate, focused phase keeps both commits readable.

### Risks

Mostly mechanical. The match arms in
`tui/app/async_tasks/dispatch.rs` (24 of them) are the call sites
that pattern-match on `BackgroundMsg` variants by name. Two arms —
`ProjectDiscovered` and `ProjectRefreshed` — contain early-return
control flow (`if self.handle_*(...) { return true; }`); the
nesting rewrite must preserve those returns, not just the
destructuring.

A clean `cargo build && cargo nextest run` will catch type errors
but not the silent control-flow regression of a missed early return.
Add a regression test for `ProjectDiscovered` / `ProjectRefreshed`
dispatch in the same commit.

## Phase 6 — Split `config.rs` into `config/`

1144-line single file. Same rationale that justified splitting
`git/state.rs` today.

### Approach

Read `config.rs` and group sibling types by what they configure
(TUI settings, scan settings, lint settings, etc.). One submodule per
group. `config/mod.rs` re-exports the top-level `CargoPortConfig` and
its loader.

Exact submodule names are deferred until the read pass — sized to the
content, not predicted ahead of time.

### Risks

Low. `config.rs` already has internal section boundaries; the split
follows them. Import sweep is wide but mechanical.

## Phase 7 — Probe `app ↔ tui` coupling (read-only)

Phase 8 introduces a family of composed `*View` traits that become
the only channel from `tui/` into app state (both reads and writes).
Phase 9 then moves `tui/app/` to top-level `app/`. Both depend on
knowing exactly what `tui/` reads from and writes to `App` today —
this phase measures it.

### Questions to answer

1. How often does code currently in `tui/app/` reach sideways into
   `tui/panes/`, `tui/pane/`, `tui/render`, or `tui/overlays/`?
2. How often does code currently in `tui/render`, `tui/panes/`, or
   `tui/pane/` reach into `tui/app/` types? Which specific types
   straddle the boundary?
3. Does `tui/finder/` separate cleanly? Initial read suggests
   `finder/dispatch.rs` is render-heavy (imports `ratatui::Frame`,
   `Rect`, widgets) while `finder/index.rs` is pure search-index logic.
   If true, `index` moves to `app/finder/` and `dispatch` stays in
   `tui/finder/`.
4. Does `tui/integration/` wire `app` to `tui`, or does it also do
   rendering? Initial read suggests `integration/framework_keymap.rs`
   (769 lines) has no `ratatui` imports — it is app-side wiring.
5. Where do `tui/state/` types get mutated from? Each mutation site
   gets a corresponding `&mut self` method on the appropriate `*View`
   trait during Phase 8.
6. How many ratatui types (`Position`, `Rect`, etc.) appear in
   `tui/app/` outside the test modules? Each occurrence is either a
   field to lift to `tui/`, or a re-export-as-newtype boundary.

### Required output

A new document at `docs/reorg-coupling.md` containing, at minimum:

- Cross-boundary reference counts (`rg`-derived, with file lists).
- List of `tui/app/` types that render-side code reads.
- List of `tui/` types that `tui/app/` reaches into.
- For finder and integration: explicit verdict — single-unit or
  split. If split, name the files going each way.
- The candidate trait list and method list per trait — every read
  and write `tui/` performs on `App` today, grouped by concern,
  ready to land as `view/*.rs` files in Phase 8.
- A go / no-go statement for Phase 8 (introducing the trait
  family), with the rationale.

### Gate

Phase 8 must not start until `docs/reorg-coupling.md` exists and
its go / no-go statement is "go." No code changes in Phase 7. No
commit other than the new document.

The Phase 8 PR description must include a link to the
`docs/reorg-coupling.md` "go" line. PRs without that link don't
get reviewed.

## Phase 8 — Introduce composed `*View` traits at the app↔tui boundary

This phase is the **code-organization** seam that makes the app
domain / terminal domain split (Principle 4) compiler-enforceable.
After this phase, `tui/` reads and mutates `app/` state through a
set of small composed traits — one per concern — not through `App`
fields. Phase 9 (the directory move) then becomes a directory
rename, not a refactor.

### What ships

A new directory `src/tui/app/view/` holding one trait per concern:

```
src/tui/app/view/
  mod.rs                 pub(crate) use of all sub-traits
  project_list.rs        pub(crate) trait ProjectListView
  focus.rs               pub(crate) trait FocusView
  ci_status.rs           pub(crate) trait CiStatusView
  lint.rs                pub(crate) trait LintView
  overlay.rs             pub(crate) trait OverlayView
  ...                    (more as Phase 7's audit surfaces)
```

Each trait holds both `&self` (read) and `&mut self` (write) methods
for its concern — Rust's borrow checker enforces the read/write split
at call sites (`&` references can only call read methods).
Render code takes `&impl ProjectListView`; event handlers take
`&mut impl ProjectListView`. The exact method list per trait comes
from Phase 7's output.

Implementation:

- Each `impl <ConcernView> for App { … }` lives next to `App`'s
  definition (e.g., `src/tui/app/mod.rs`), not in `view/`. The
  contract goes in `view/`; the behavior stays next to the data.
- Render-side files (`tui/panes/*`, `tui/pane/*`, `tui/overlays/*`,
  `tui/render.rs`, `tui/columns/*`) update their function signatures
  to take `&impl <ConcernView>` or `&mut impl <ConcernView>` —
  whichever they actually need. `App` fields lose any `pub(super)`
  visibility that was only needed for render-side access.
- Names: `*View` is short and recognized convention; the trait
  containing writes is a small misnomer that the user has accepted.
  Renaming is reversible in the IDE later if it bothers anyone.

### Why this is its own phase

Without these traits, Phase 9 would have to both move files *and*
rewrite call sites — large risk, hard to bisect. With the traits in
place, Phase 9 becomes `git mv` plus an import sweep.

### Risks

- Trait family will be 5–8 traits, ~60–80 methods total. That's the
  size of the actual boundary; smaller would mean the boundary still
  leaks.
- Object safety: if any accessor would benefit from a `Self`-returning
  signature, keep the trait generic (`&impl ConcernView`) rather than
  forcing `&dyn ConcernView`.
- Some render functions touch multiple concerns. Signatures grow:
  `fn render(view: &impl ProjectListView, focus: &impl FocusView)`.
  Acceptable; surfaces which concerns each pane actually reads.

## Phase 9 — Promote `tui/app/` to top-level `app/`

The directory move. After this phase:

- `main.rs` calls `app::run()`.
- `app/` owns program state, lifecycle, async tasks, navigation,
  background work, integration wiring, finder logic.
- `tui/` owns the cargo-port-specific terminal layer: pane impls,
  terminal setup, rendering glue, input capture. Generic TUI
  primitives (cpu sampling, mouse hit-test dispatch, pane chrome,
  popup, column widths) already live in `tui_pane/` after the
  prerequisite extraction.

### What moves out of `tui/` and into `app/`

- `tui/app/` (40 files pre-extraction; the count after `tui_pane/`
  extraction will be smaller) → `app/`, with subdirectories preserved
  internally (`app/async_tasks/`, `app/navigation/`, `app/startup.rs`,
  `app/target_index.rs`, `app/phase_state.rs`, `app/tests/`).
- `tui/state/` → `app/state/`.
- `tui/background.rs` → `app/background.rs`.
- `tui/cpu.rs` → `app/cpu.rs` (stays in cargo-port through the
  `tui_pane/` extraction; lands in `app/` here).
- `tui/integration/` → `app/integration/` (unless Phase 7 reveals it
  does rendering work, in which case it splits).
- `tui/finder/` → `app/finder/` (unless Phase 7 reveals it does not
  separate cleanly).

### What stays in `tui/`

After this phase, `tui/` contains only cargo-port-specific terminal
code:

- `tui/panes/` — domain pane impls (Package, Lang, Git, Targets,
  Output, etc.) plus the `Panes` registry that impls
  `tui_pane::HitTestRegistry` and `tui_pane::PaneRegistry`.
- `tui/project_list/` — project-list rendering.
- `tui/keymap_ui/` — keymap popup body (deferred from the extraction).
- `tui/overlays/` — `mod.rs`, `pane_impls.rs` (`KeymapPane`,
  `SettingsPane`, `FinderPane` impl blocks), `render_state.rs` (the
  app-owned `FinderPane` viewport wrapper).
- `tui/pane/` — `mod.rs` + `dismiss.rs` (`DismissTarget` enum).
  `pane/dispatch.rs` is gone by this point (extraction Phase 5).
- `tui/interaction.rs` — `hit_test_toasts` pre-pass, the cargo-port
  `HittableId` enum (Toasts-free), and the `HITTABLE_Z_ORDER` const.
- `tui/render.rs` — main render orchestration (registry + one call
  to `tui_pane::render_panes` + `tui_pane::render_toasts`).
- `tui/terminal.rs`, `tui/input/`, `tui/settings.rs`,
  `tui/test_support.rs`, app-specific entries of `tui/constants.rs`.

Anything generic (pane chrome, popup, column widths, hit-test and
render dispatch traits + loops, color palette, watched-file,
running-tracker, layout primitives) is already in `tui_pane/` after
the extraction completes.

### New entry sequence

```rust
// main.rs
fn main() -> ExitCode { app::run() }

// app/mod.rs
pub fn run() -> ExitCode {
    let state = startup::build_state();
    let handle = async_tasks::spawn(&state);
    tui::run(state, handle)
}
```

### Risks

- Medium, not High. `tui/app/` has the most internal references in
  the codebase, but Phase 8's `*View` trait family already pinned
  the `tui→app` interface. This phase only changes paths.
- Import sweep is wide: every `crate::tui::app::` site updates to
  `crate::app::`. Mechanical.
- `cargo nextest run` is the canary — any test that still reaches a
  no-longer-public `App` field after the move will fail
  compilation, surfacing places where Phase 8 missed a leak.

### Commit packaging

Default: single commit.

Split into two commits (state-and-wiring first, `tui/app/` second)
if **any** of these conditions hold after Phase 8 lands:

- Phase 8 added more than 3 `// boundary escape:` comments across
  the `*View` trait family.
- Phase 8 left any direct `App` field access in `tui/` without a
  documented escape hatch.
- Phase 8 surfaced mutation sites that `docs/reorg-coupling.md`
  did not list (i.e., the probe missed them).

The Phase 8 commit message must include a one-line summary of these
counts (e.g., `Phase 8: view traits (3 escapes, 0 missed)`) so the
Phase 9 author can apply the rule mechanically.

## Followups (post-reorg)

Items not part of the reorg itself but surfaced during planning:

- **`FetchStatus` as a real state machine.** Currently a two-variant
  flag (`Pending` / `Fetched`). The actual lifecycle has more states
  (in-flight, completed, failed, retry-scheduled). A sum type would
  eliminate guard logic in handlers and prevent invalid transitions.
  Defer until after Phase 9 so the type is rewritten in its final
  module rather than dragged across the move.

## Out-of-band questions to resolve before sequencing starts

1. **`keymap/` flat vs. directory.** Two files, no submodules.
   Flattening to `keymap.rs` at top level is on the table. Decision
   deferred until after Phase 2 (so `keymap` is the only loose top-level
   `.rs` candidate left to evaluate).
2. **`tui/finder/` cohesion.** Phase 7 probe answers this directly.
