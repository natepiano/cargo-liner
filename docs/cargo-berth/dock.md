# dock — a tile grid of every worktree's berth status

Parked notes, not a plan. A ratatui app that draws one tile per worktree
carrying berth status, using cargo-tile's grid: a tile appears when the
worktree has something to say and leaves when it has nothing left to say.

## Where the tiling actually lives

`tui_pane` has the drawing; `cargo-tile/src/tiles.rs` has the growth. The
module doc in `tiles.rs` already names the boundary — the framework is handed
a `ResolvedPaneLayout` and "what is left here is only what the framework has
no opinion about: how many columns there are, how tall each one is, and how a
cell travels when that changes." That remainder is the whole algorithm, and it
is what a second app needs.

| `tui_pane` today | `cargo-tile/src/tiles.rs`, every item `pub(crate)` |
|---|---|
| `PaneFrame`, `frame_inner`, `share_borders`, `draw_clipped`, `GridLines` | `columns` — column-at-a-time fill and the `ceil_sqrt` regime change |
| `PaneAxisSize`, `PaneSizeSpec`, `constraints_for_sizes` | `shares` and `apportion` — demand-driven row division, focus served first |
| `PaneGridLayout`, `PanePlacement`, `ResolvedPaneLayout` | `Held` — a cell keeps the rows it grew into until a neighbour is short |
| `Region`, `Viewport`, `ColumnWidths` | `Slot` identity, `Transition`, `Step`, `moving_cell`, `wrapping_cell`, `column_band`, `edge_rect`, `closing_rect`, `lerp_rect`, `eased` |

`PaneGridLayout` is a result type holding placements someone else computed.
Nothing in `tui_pane` computes them, so a second app starts with borders and
clipping and no grid.

### What the extraction costs

`tiles.rs` is nearly content-free already. `columns`, `shares`, `apportion`
and the transition machinery are pure functions of their arguments. Four
couplings to cut:

1. **Eleven constants** from `crate::constants` — `MIN_TILE_HEIGHT`,
   `MIN_TILE_WIDTH`, `MIN_INITIAL_ROWS`, `TILE_DEMAND_STEP`,
   `TILE_BORDER_ROWS`, `TILE_ANIMATION_MILLIS`, `FOCUS_ANIMATION_MILLIS`,
   `MIN_STEP_MILLIS`, `MAX_PENDING_STEPS`, `PROGRESS_SCALE`, `TABLE_CELL`.
   These become one config struct the app hands in.
2. **`TileContent::Summary`** — cell one is hardcoded as the summary table.
   Either generalise to a `Pinned` first cell or make cell one an ordinary
   slot.
3. **`TileDemand::id: u32`**, documented as `crate::roster::TrackedGroup`.
   Becomes generic over `Id: Copy + Eq`, which dock needs anyway because its
   key is a pair (below).
4. **Demand measurement** stays with the app. `crate::render` measures the
   rows a cell would draw at the width it will get, and `TileDemands` is
   already the whole contract between that and the grid. That boundary is
   right as it stands.

Same division as themes: `tui_pane` supplies the machinery, each app owns its
content. See [[per-app-theme-ownership]].

## One tile per worktree, repository as the ordering key

Not one tile per `.git`.

- **The motion is keyed on cells joining and leaving.** `Slot` identity exists
  so a cell vanishing makes every cell after it travel one place forward.
  Worktrees come and go constantly; repositories essentially never do. A
  repository-level tile would sit static and the animation would carry nothing.
- **The lifecycle rule is worktree-scoped**, which is the requirement as
  stated.

Repository still matters as the ordering key: the ledger is per-repository, so
every ordering edge, overlap and incursion is a relation *within* one
repository. Keeping a repository's worktrees contiguous lets the column-at-a-time
fill in `columns` tend to give each repository its own column.

### Correction to the disappearance rule

"Disappear when the worktree disappears" hides the single loudest thing berth
has to report. `ReservationHolder` carries `WorktreeLiveness`, and a dead
worktree holding an unresolved reservation is the orphan case. The rule wants
to be: **leave when the worktree is gone _and_ its reservations are settled**;
until then the tile stays and says the worktree is dead.

### Data

`board --json` is reservation-centric, so a worktree tile is a projection of
it rather than a partition of it.

```rust
/// One repository's ledger and every worktree drawing from it.
struct RepositoryBoard {
    repository_id:    RepositoryId,
    common_git_dir:   PathBuf,            // what the watcher watches
    journal_position: JournalByteOffset,  // the cheap "anything new" check
    worktrees:        Vec<WorktreeStatus>,
}

/// One worktree's berth status — the unit a tile draws.
struct WorktreeStatus {
    worktree_id:  WorktreeId,
    root:         CanonicalWorktreeRoot,
    branch:       HolderBranch,
    liveness:     WorktreeLiveness,
    reservations: Vec<ReservationRow>,
    incursions:   Vec<OutstandingIncursion>,
    blocked_by:   Vec<WorktreeId>,
    blocking:     Vec<WorktreeId>,
    ahead_behind: AheadBehind,
}
```

`ready_now`, `unconstrained_reservations` and `resolved` group cleanly by
`ReservationHolder::worktree_id`. `waiting`, `unresolved_overlaps` and
`outstanding_incursions` do not: each belongs to a *pair* of reservations, not
to one tile. Those resolve into the `blocked_by` and `blocking` lists and get
drawn in both tiles from opposite ends.

Tile identity is `(RepositoryId, WorktreeId)`, which is the second reason the
extracted grid wants to be generic over its id rather than keeping `u32`.

Severity drives sort order and `TileDemand::rows` together: a worktree holding
an outstanding incursion asks for more rows and gets them, because `shares`
moves room off the cells not using theirs.

### Refresh

Do not poll `board --json`. Its own `git_cost` block reports the git calls each
invocation makes — `worktree_list_calls`, `worktree_ahead_behind_computations`,
`trunk_resolution_calls`, `reservation_evidence_revalidations` — so a per-second
poll across every repository is expensive by construction.

The journal is append-only and every board section reports a
`journal_position.journal_byte_offset`, which is an exact change signal that
costs a stat. Watch `.git/cargo-berth/journal.ndjson` per repository;
`tui_pane::WatchedFile` already does this.

## Open input

Where the repository list comes from. A configured list of roots, a scan for
`.git/cargo-berth/` beneath a scan root, or whatever berth has enrolled — these
lead to different code and the choice is the user's. A configured scan root is
the working assumption until then.
