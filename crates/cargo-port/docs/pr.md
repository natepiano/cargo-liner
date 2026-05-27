# Open Pull Requests Plan

## Goal

Show open, unmerged pull requests authored by the current GitHub user for the
selected repository.

The first UI surface is the existing Git pane. The project list gets only a
compact signal after the Git-pane model is working. The universal finder can
search PR titles and jump to the matching PR row.

## Non-goals

- Do not show closed or merged PRs.
- Do not show PRs from other authors.
- Do not create a new pane for the first implementation.
- Do not show PR URLs as visible table columns.
- Do not add timestamps to the Git pane PR rows.
- Do not widen the project list for PR-specific columns.

## User Behavior

- Selecting a project with GitHub remotes fetches the user's open PRs for that
  repo through the existing GitHub enrichment path.
- The Git pane shows a `Pull Requests` section when the repo has matching PRs.
- The selected PR row opens the PR in GitHub on Enter.
- Finder `/` searches PR number, title, branch, and state.
- Choosing a PR finder result selects the owning project, focuses the Git pane,
  and puts the Git pane cursor on that PR.

## Display

Wide Git pane:

```text
┌─ Git Details ───────────────────────────────────────────────┐
│ Head        main ↑2                                         │
│ Status      clean                                           │
│                                                              │
├─ Pull Requests (2) ─────────────────────────────────────────┤
│ #     Status   Branch                   Title                │
│ #128  draft    feature/member-vendored  Show vendored wor... │
│ #124  changes  refactor/ci-cache        Keep failed CI ru... │
│                                                              │
├─ Remotes (2) ────────────────────────────────────────────────┤
│ origin   natepiano/cargo-port        main       ☑           │
└──────────────────────────────────────────────────────────────┘
```

Narrow Git pane:

```text
┌─ Git Details ───────────────────────┐
│ Head   main ↑2                      │
│ Status clean                        │
│                                     │
├─ Pull Requests (2) ─────────────────┤
│ #     Status   Branch     Title     │
│ #128  draft    featur...  Show v... │
│ #124  changes  refact...  Keep f... │
└─────────────────────────────────────┘
```

Show `head -> base` only when the base branch is not the repo default branch:

```text
│ #131  review  fix/0.5-release -> release/0.5  Fix release...│
```

## Stored Model

Add repo-level PR data next to existing GitHub and CI repo data.

```rust
pub(crate) struct GitRepo {
    pub repo_info:   Option<RepoInfo>,
    pub github_info: Option<GitHubInfo>,
    pub ci_data:     ProjectCiData,
    pub pr_data:     ProjectPrData,
}

#[derive(Clone, Default)]
pub(crate) enum ProjectPrData {
    #[default]
    Unfetched,
    Loading,
    Loaded(ProjectPrInfo),
    Unavailable(ProjectPrUnavailable),
}

#[derive(Clone)]
pub(crate) struct ProjectPrInfo {
    pub open:           Vec<PullRequestInfo>,
    pub default_branch: String,
    pub fetched_at:     String,
    pub stale:          bool,
    pub completeness:   PullRequestCompleteness,
    pub viewer_login:   String,
}

#[derive(Clone)]
pub(crate) struct ProjectPrUnavailable {
    pub reason:     PullRequestUnavailableReason,
    pub stale:      Option<ProjectPrInfo>,
    pub fetched_at: Option<String>,
}

#[derive(Clone)]
pub(crate) struct PullRequestInfo {
    pub number:     u32,
    pub title:      String,
    pub url:        String,
    pub state:      PullRequestState,
    pub head:       String,
    pub head_owner: Option<String>,
    pub head_repo:  Option<String>,
    pub base:       String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PullRequestState {
    Draft,
    ChangesRequested,
    ReviewRequired,
    Approved,
    Blocked,
    Behind,
    ChecksFailing,
    Ready,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PullRequestUnavailableReason {
    Unauthenticated,
    RateLimited,
    Network,
    Forbidden,
    RepositoryMissing,
    GraphQlError,
    IncompletePagination,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PullRequestCompleteness {
    Complete,
    Truncated { shown: usize },
}
```

The stored model keeps only fields used by display, navigation, freshness, and
open/copy actions. Raw GitHub fields are reduced during fetch. The
`default_branch` value comes from the fetched GitHub repository, not local
`origin/HEAD`, so fork and upstream remotes compare PR bases against the repo
that was actually queried.

`Loaded(open: [])` is the only confirmed empty state. `Unfetched`, `Loading`,
and `Unavailable` must not render as "no PRs".

PR results are scoped to a GitHub user. Cache lookup must require both
`OwnerRepo` and matching `viewer_login`; a login mismatch makes cached PR data
stale/unusable.

## GitHub Query

Use GraphQL from the existing GitHub HTTP client.

Use an author-scoped query so the first page is already filtered to the current
GitHub user. Do not fetch `repository.pullRequests(first: 50)` and then filter
client-side; that misses matching PRs on busy repositories.

Resolve the current GitHub user before PR search:

- Add a small viewer-identity step: `viewer { login }`.
- Cache `viewer_login` with the GitHub client/token.
- Build the PR search query from that concrete login.
- Do not write `<viewer.login>` or any placeholder into the `search.query`
  argument.
- Prefer GraphQL variables for the final query string once variable support
  exists in the local HTTP helper.

Build `search_query` as
`repo:{owner}/{name} is:pr is:open author:{viewer_login}` after resolving
`viewer_login`, then pass it as a GraphQL variable:

```graphql
query PullRequests($owner: String!, $name: String!, $searchQuery: String!, $cursor: String) {
  repository(owner: $owner, name: $name) {
    defaultBranchRef { name }
  }
  search(
    type: ISSUE,
    first: 50,
    after: $cursor,
    query: $searchQuery
  ) {
    pageInfo { hasNextPage endCursor }
    nodes {
      ... on PullRequest {
        number
        title
        url
        isDraft
        reviewDecision
        mergeStateStatus
        headRefName
        baseRefName
        headRepository { name owner { login } }
      }
    }
  }
}
```

Pagination:

- Follow `pageInfo.hasNextPage` until all authored open PRs are fetched, or
  until a documented cap is reached.
- If a cap is reached, store a truncated marker and render a compact
  "more PRs not shown" note in the Git pane.
- Add an acceptance case where a matching PR appears after the first page.

Filtering:

- `is:open` and `is:pr` are enforced by query.
- `author:{viewer_login}` is enforced by query.
- Do not store `mergedAt` unless GitHub produces an observed edge case where an
  open search result is already merged.
- Treat GraphQL `errors`, `repository: null`, missing `search`, and missing PR
  fields as failed PR fetches, not successful empty results.
- Classify required versus optional PR fields. Require number, title, URL,
  state inputs, `headRefName`, and `baseRefName`. Tolerate null
  `headRepository` and fall back to the head ref with an unknown-fork marker.

State reduction:

- `isDraft` -> `Draft`
- `reviewDecision == CHANGES_REQUESTED` -> `ChangesRequested`
- selected `mergeStateStatus` values -> `Blocked`, `Behind`, `ChecksFailing`,
  or `Ready`
- `reviewDecision == REVIEW_REQUIRED` -> `ReviewRequired`
- `reviewDecision == APPROVED` -> `Approved`
- unknown values -> `Unknown`

Display precedence favors action blockers:

1. `Draft`
2. `ChangesRequested`
3. `ChecksFailing`
4. `Blocked`
5. `Behind`
6. `ReviewRequired`
7. `Approved`
8. `Ready`
9. `Unknown`

Head branch display:

- Same repository: display `headRefName`.
- Fork PR: display `owner:headRefName`, using `headRepository.owner.login`.
- Include `headRepository.name` in finder tokens when it differs from the base
  repo name.

## Fetch And Cache

Extend the repo-level GitHub fetch path instead of adding an unrelated worker.

- Add PR results to the repo-keyed cache data.
- Fetch PRs in the same background repo fetch that loads CI runs and repository
  metadata.
- Deduplicate fetches by `OwnerRepo`, matching the existing repo fetch behavior.
- Emit a new background message carrying PR data for `OwnerRepo` plus the source
  path.
- Store PR data on `ProjectEntry::git_repo.pr_data`.
- Apply the PR payload to every visible entry whose fetch URL resolves to that
  `OwnerRepo`, including linked worktrees and worktree-group siblings.
- Bump the scan generation after PR data lands so detail panes rebuild. If the
  currently selected row resolves to the same `OwnerRepo`, its Git detail data
  must be invalidated even when the source path was a different checkout.

Cache merge rules:

- CI refreshes must preserve cached PR data unless they intentionally replace
  PR results.
- PR refreshes must preserve cached CI runs and repository metadata.
- PR fetching is a distinct outcome inside the same background task. CI/meta
  success must not imply PR success, and PR failure must not overwrite valid
  CI/meta cache.
- Cache recovery must track PR fetch success independently from metadata fetch
  success, so a rate-limited PR query can be retried even if CI and stars were
  fetched successfully.
- Replace all-or-nothing repo-cache loading with a component-level fetch plan:
  CI runs, repository metadata, and PRs each decide independently whether cached
  data is fresh enough. A CI/meta cache hit must not suppress a missing, stale,
  or retryable PR fetch.
- Carry `viewer_login` in the PR cache facet and reject confirmed PR cache hits
  when the active login differs.

Freshness:

- Store `fetched_at` for PR data.
- Mark cached PRs stale after a short TTL or when manual sync requests a fresh
  GitHub fetch.
- PR freshness must not depend only on local `FETCH_HEAD`; draft state, review
  state, mergeability, title, and closure can change on GitHub without a local
  fetch.
- Render stale data as stale rather than silently presenting it as current.
- Before PR search, check known GitHub availability and the GraphQL rate-limit
  bucket. If GraphQL is known exhausted, preserve stale PRs, mark PR data
  `RateLimited`, and wait for the existing recovery path before retrying.

Pagination failures:

- Page-2 errors, page-2 rate limits, and cap hits with `hasNextPage` still true
  must not become confirmed-complete `Loaded` results.
- Preserve fetched rows as incomplete/stale data with a visible Git-pane note,
  or keep the previous complete snapshot and record the refresh as unavailable.

## Git Pane Data

Extend `GitData`:

```rust
pub struct GitData {
    // existing fields...
    pub pull_requests: PullRequestSection,
}

#[derive(Clone)]
pub struct PullRequestSection {
    pub state:              PullRequestSectionState,
    pub rows:               Vec<PullRequestRow>,
    pub fetched_at:         Option<String>,
    pub unavailable_reason: Option<PullRequestUnavailableReason>,
    pub completeness:       PullRequestCompleteness,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PullRequestSectionState {
    HiddenConfirmedEmpty,
    Loading,
    Loaded,
    Stale,
    Unavailable,
}

#[derive(Clone)]
pub struct PullRequestRow {
    pub number:        u32,
    pub title:         String,
    pub url:           String,
    pub state_label:   &'static str,
    pub head_label:    String,
    pub base:          String,
    pub show_base:     bool,
}
```

`show_base` is false when `base` equals the fetched repository default branch.

The renderer consumes the typed `PullRequestSection`, not just a row vector, so
loading, unavailable, stale, fetched-at, and truncated states survive the
projection from repo data into pane data.

Update Git-pane row routing:

- Add `PullRequest(usize)` to a shared Git row-routing model.
- Add PR rows to `GitRow`.
- Use one row-routing helper for row count, section lookup, rendering, copy,
  activation, Enter, and finder jumps.
- Make Enter on a PR row open the URL.
- Make copy on a PR row copy the PR URL.
- Enable the Git-pane Activate shortcut for PR rows in the keymap visibility
  layer, not just in the Enter handler.
- Keep Remotes and Worktrees behavior unchanged.

Single-line PR rows:

- PR rows render as `#`, `Status`, `Branch`, and `Title` columns.
- The header row mirrors the Remotes and Worktrees table treatment.
- Cursor movement advances by PR row.
- Hit testing maps the rendered PR row back to the same logical PR row.
- Long branch or title text is truncated to keep the table on one row.

## Finder

Add PRs to the universal finder index.

```rust
pub enum FinderKind {
    Project,
    Binary,
    Example,
    Bench,
    PullRequest,
}
```

Add a typed target rather than overloading `target_name`:

```rust
pub enum FinderTarget {
    Project,
    CargoTarget { name: String, kind: RunTargetKind },
    PullRequest { owner_repo: OwnerRepo, number: u32 },
}
```

PR finder item:

```text
display_name: "#128 Show vendored workspace member packages"
tokens:       ["128", title, head branch, state label]
project_path: repo root path
target:       PullRequest { owner_repo, number }
```

Finder columns:

- `Name`: `#number title`
- `Project`: repo/project label
- `Branch`: `head` or `head -> base` when the base is non-default
- `Dir`: repo directory, or blank if the row is already clear without it
- `Type`: `pr`

Finder tokens include the exact branch display plus separate `head`, `base`,
`head_owner`, and differing `head_repo` tokens, so searches for a non-default
base branch can find the PR.

Deduplicate PR finder rows by `(OwnerRepo, number)`. If multiple visible entries
share the same repo, choose the primary/root path as the representative
navigation target.

Finder confirmation for `PullRequest`:

1. Select the owning repo in the project list.
2. Close finder without restoring the prior focus.
3. Rebuild detail data if needed, or store a pending PR cursor request.
4. Focus the Git pane.
5. Set the Git pane cursor to the matching PR row by PR number.
6. Leave Enter on the Git pane to open the PR URL.

The finder should not open the browser directly; it should navigate first, so
users can inspect the row and decide.

Update the finder empty hint and tests so pull requests are included in the
searchable item set.

## Project List Signal

After the Git pane and finder behavior are stable, add a compact project-list
signal:

```text
cargo-port PR2
```

Rules:

- No new project-list column in the first pass.
- Hide the badge when the count is zero.
- Count only open, unmerged PRs from the current user.
- Treat worktree groups as a repo-level count, not per-worktree count.

## Phases

### Phase 1: Data Model

- Add `ProjectPrData`, `ProjectPrInfo`, `PullRequestInfo`, and
  `PullRequestState`.
- Add `pr_data` to `GitRepo`.
- Add project-list accessors for PR data by path.
- Add explicit loading, unavailable, stale, and confirmed-empty states.
- Add unit tests for default/unfetched behavior, unavailable behavior, stale
  preservation, and repo-level sharing.

### Phase 2: GitHub Fetch

- Add viewer-login resolution and caching to the GitHub client/fetch path.
- Add an author-scoped paginated GraphQL PR query to `HttpClient`, built from
  the resolved viewer login.
- Reduce GitHub raw fields into `PullRequestInfo`.
- Extend `CachedRepoData` to store a facet-level PR cache with viewer identity,
  freshness, availability, and completeness.
- Extend repo fetch completion handling to write PR data.
- Define cache merge semantics so CI writes preserve PR data and PR writes
  preserve CI data.
- Replace all-or-nothing repo-cache hits with component-level fetch planning.
- Add tests for current-user open PRs, PRs after the first page, GraphQL
  partial errors, unauthenticated/rate-limited fetches, and fork head labels.
  Include tests where CI/meta cache exists but PR data is absent or stale.

### Phase 3: Git Pane Rendering

- Add a typed PR section to `GitData`.
- Add a `Pull Requests` section in the Git pane.
- Add a shared Git row-routing helper before wiring PR rows into rendering,
  copy, activation, and viewport length.
- Add row layout for wide and narrow panes, including logical-row spans for
  multi-line PR blocks.
- Add Enter/copy routing for selected PR rows.
- Add shortcut-visibility tests for PR activation.
- Add render and interaction tests for zero, one, and multiple PRs.

### Phase 4: Finder Integration

- Add `FinderKind::PullRequest`.
- Add a typed finder target carrying `OwnerRepo` and PR number.
- Index PR title, number, branch display, head branch, base branch, state, and
  fork owner/repo tokens.
- Deduplicate PR finder entries by `(OwnerRepo, number)`.
- Route finder confirmation to the owning project and Git pane PR row.
- Add a pending PR selection path if Git detail data is not rebuilt in the same
  step.
- Override prior finder return focus for PR results so the final focus is Git.
- Add finder tests for title search, PR row selection, and narrow-layout jump
  behavior.

### Phase 5: Project List Badge

- Add a compact `PRn` suffix to root/worktree-group project rows.
- Include the suffix in name width calculation.
- Add tests that zero PRs render no badge and worktree groups count once.

## Verification

- `cargo +nightly fmt --all`
- `cargo mend --workspace --all-targets`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo nextest run`
- Manual smoke test:
  - repo with no user PRs
  - repo with one draft PR
  - repo with one changes-requested PR
  - repo with a fork PR using `owner:branch`
  - repo where matching PRs appear after the first search page
  - repo where page 2 fails after page 1 succeeds
  - GitHub unavailable with stale cached PRs
  - existing CI/meta cache with missing PR data still triggers PR fetch
  - finder title search jumps to the PR row
  - finder base-branch search finds a non-default-base PR
  - Enter opens the selected PR URL

## Team Review Log

### Cycle 1 Accepted Refinements

- Author-scoped PR querying must happen before result limiting; client-side
  filtering after `first: 50` is insufficient.
- PR data needs explicit loading, unavailable, stale, and confirmed-empty
  states.
- GraphQL PR errors must not be cached as successful empty results.
- PR cache freshness must not depend only on local `FETCH_HEAD`.
- Repo-level PR fetch completion must fan out to all matching `OwnerRepo`
  entries and invalidate details by repo identity.
- `CachedRepoData` needs merge rules so CI and PR writers preserve each other's
  cached data.
- Fork PRs need head repository identity for display and finder tokens.
- Git pane PR rows need a single row-routing model shared by render, copy,
  activation, Enter, viewport length, and finder jumps.
- Multi-line PR rows are one logical selectable row with multiple visual lines.
- Finder PR items need a typed PR target with `OwnerRepo` and PR number.
- Git-pane Activate shortcut visibility must include PR URL rows.

### Cycle 2 Accepted Refinements

- Viewer login must be resolved before the author-scoped search query is built;
  GraphQL cannot interpolate `viewer.login` inside the same operation's search
  argument.
- PR cache hits are valid only for the current `viewer_login`.
- Repo cache must be facet-aware so CI/meta cache hits do not suppress missing
  or stale PR fetches.
- PR fetch success/failure is independent from CI and repo metadata success.
- Stale unavailable PR data should keep a full stale `ProjectPrInfo` snapshot,
  not only stale rows.
- Pagination completeness must be modeled and rendered.
- Null `headRepository` is tolerated for fork PRs with unavailable source repos.
- Known exhausted GraphQL rate limits should prevent immediate repeat PR
  searches.
- `GitData` carries a typed PR section so rendering can distinguish loading,
  stale, unavailable, truncated, loaded, and confirmed-empty states.
- PR state labels prioritize action blockers over positive review states.
- Multi-line Git rows use explicit visual spans.
- Finder PR indexing includes base branch tokens, deduplicates by repo/number,
  and forces Git focus after confirmation.
