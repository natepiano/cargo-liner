# CI runs, titles, and narrow tables

The CI pane lists cached and fetched runs with whole job headers and count-aware titles.
It uses drawn row geometry for selection, pagination, and mouse input.

Paths below are relative to `crates/cargo-port/src/`.

## Data and columns

`tui/state/ci.rs` owns the CI state and its display selection.
`scan/ci_cache.rs` combines cached and fetched run data.
`project/info.rs` defines `GithubRunCount::{Loading, Known, Unavailable}`.
A failed request retains a known count.

`tui/panes/ci/data.rs` defines pane data and empty states.
`tui/panes/ci/render.rs` measures columns and draws the table.
Job headers keep their whole text by taking cells from Commit when necessary.
Multi-word job headers keep the ordinary one-cell column gap.
Branch mode can hide Branch when the complete value does not fit.
Outside branch mode, Total remains available when its column fits.
The Total label aligns with its duration and status values.

The pane keeps one blank cell at each border.
Commit values use `render::truncate_with_ellipsis`.
That helper removes trailing spaces and a trailing middle-dot separator before it adds `…`.
A short date omits its time before it uses the short year form.

## Titles and empty messages

`CiTitleCursor::{Hidden, Visible(usize)}` names whether the title includes a selected run position.
`ci_panel_title(data, cursor)` builds the requested label and count.
`fitted_ci_panel_title(data, cursor, width)` fits it to the pane.
It converts the cursor to the framework's optional cursor only when it constructs `PaneTitleCount`.

When a title cannot fit, it removes the branch suffix and cursor-page part before shortening the count.
For example, a narrow title can retain `CI runs (48)`.
The pane does not use an on-branch-rows empty state.

If a fixed empty title cannot fit, the title becomes CI runs.
The body then carries the complete empty message on wrapped rows.
A branch-dependent message cuts the branch name rather than a fixed label.
This preserves the meaning without clipping the title.

## Invariants and gotchas

1. Keep job headers whole.
2. Give the Commit column the remaining text budget.
3. Remove optional title suffixes before the count loses cells.
4. Keep whole empty messages available in the body.
5. Match hit areas and page counts to drawn rows.
6. Preserve both blank margins.

Use full-window drawn tests at 60, 80, and 160 columns.
An explicit pane-width test alone does not check the window's actual split.
Title counters use run-list length; they do not substitute the repository's total fetched-page count.
