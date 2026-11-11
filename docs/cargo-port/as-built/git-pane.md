# Git details, remote tables, and quota rows

The Git pane draws repository fields, pull requests, Remotes, and Worktrees from typed detail data.
Its height calculation uses the same row shapes as its renderer.

Paths below are relative to `crates/cargo-port/src/`.

## State and height

`tui/panes/pane_data/git_data.rs` defines `GitData`, `RemoteRows`, `QuotaObservation`, and `PullRequestSectionState`.
`QuotaObservation::{Unobserved, Observed(RateLimitQuota)}` keeps unmeasured buckets out of the field list.
A partial or failed quota reply preserves each previously measured bucket.

`DescriptionState` in `tui/panes/description.rs` separates pending text, known text, and known-empty text.
`RepositoryProbe` in `project/entry.rs` supplies a completed no-repository result.
Without that result, a known-empty description cannot resolve.

`tui/panes/git/render.rs` owns `GitAboutLayout` and `git_lower_content_height`.
They account for description separators and stacked quota rows.
`tui/panes/pane_data/detail_data.rs` uses the matching height estimate.
A short body reserves its lower sections and caps the description at two rows when required.
Messages can use additional free rows in larger panes.

The unloaded pane draws `loading…`.
A project without a repository retains the Git title and its no-repository body state.
Quota arrivals do not rename the pane.
Pull Requests shows Loading before a pending fetch completes.
Its title shows a count only when that count is known.
Results also update matching submodule entries.

## Remotes and Worktrees

`SyncColLayout` in `git/render.rs` supplies shared Branch, Tracked, and Sync widths.
`remote_col_layout`, `sync_col_layout`, and `sync_value_widths` measure the rows.
`SyncTableMode::{Compact, Full}` selects the leading columns.
Reduced layouts remove columns before they cut a header.
Every table keeps its blank outer margins.

Name and URL columns give up width before the trailing columns.
Branch, Tracked, and Sync stay aligned between both sections.
Columns use display-cell widths, including wide characters.
`fit_table_sync_text` first uses full status text, then whole counts without the words ahead and behind.
It never cuts through a numeric count.
A further cut removes whole tokens and adds `…`.

Remote URLs use `render::cut_path_from_left` in `tui/render/mod.rs`.
A cut starts with `…` and retains the checkout end.
The Finder Dir column uses the same helper and `FIT_ELLIPSIS` constant.
Other ordinary Git values use the shared right-cut helper.

The full table preserves even two-cell gaps.
A 53-cell Git pane can have only an 11-cell URL column beside Branch 6, Tracked 11, and Sync 4.
Those trailing widths already satisfy their widest header or value.
The URL therefore draws `…argo-liner`; the gaps do not become URL cells.
This rule also applies when no Worktrees section is present.

## Quota reset and fitting

`tui/app/async_tasks/quota_refresh.rs` schedules a quota fetch at the reset.
A reply that still names the old window receives at most three additional retries, ten seconds apart.
`ResetRetrySchedule::{Due, Exhausted}` and `ResetRetries` own that retry state.
After exhaustion, the normal poll supplies the next request.
The reset request and its retries use quota-exempt `/rate_limit` requests.

`fit_rate_limit_value` returns `RateLimitValue::{Inline, Stacked}`.
It keeps remaining and limit counts whole.
It includes reset text only when the available row can hold it.
Precision uses the widest countdown form so the row does not alternate between inline and stacked states each second.
Expired resets show no reset countdown.
A GitHub outage produces one styled outage row.

## Invariants and calibration

1. Keep Remotes and Worktrees columns aligned.
2. Preserve whole headers and whole numeric Sync tokens.
3. Keep even gaps in the full table.
4. Count stacked quota rows in both measurement and rendering.
5. Keep cached quota values after partial or failed requests.
6. Fetch again after an old-window reset reply within the bounded retry schedule.

The Git label margin at a 60-column window remains unchanged.
A wide emoji can supply its own colors independently of terminal text colors.
A narrow date first omits the time, then uses the short year form when needed.
The pane keeps one Remotes divider and no extra rule below its top border.
