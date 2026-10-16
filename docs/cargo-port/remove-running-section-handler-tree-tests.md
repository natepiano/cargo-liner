# handler-tree-tests

> **Status: IMPLEMENTATION PLAN — phased, delegate-ready.** The tree height tests check `height` against the drawn rows only at the widths where an agent tile's layout changes, plus one width on each side, in place of sweeping every width from 10 to 140.

> **Production: remove-running-section** — unit `handler-enh-unit`; production doc `docs/cargo-port/remove-running-section-production.md`

## Source

2026-10-16, the showrunner (from tile-fixes): six `agent_cell::tests::tree_tests` tests run 1.03–1.28 s on the Mac in a loaded suite run. Alone they took 0.468–0.640 s on the Mac and 0.144–0.376 s on natedev, so none is a defect, but the showrunner ruled: "Cut them anyway: half a second each on the Mac, times six, is time we don't need to spend. Test each width where a column appears or drops, plus the width on either side of it, so an off-by-one at a boundary still fails. Derive the boundaries from the column-width constants, not a hand-written list. Run it as a one-phase plan with a Change-Review: none checkpoint, and report before and after times on the Mac."

Before, alone on the Mac (`cargo nextest run -p cargo-handler -E 'test(/tree_heights_/)' --test-threads 1`, main 6e041a56):

| test | Mac |
| --- | --- |
| `tree_heights_match_drawn_rows_at_every_width` | 0.526 s |
| `tree_heights_match_drawn_rows_at_wide_widths` | 0.640 s |
| `tree_heights_with_nested_work_match_drawn_rows_at_every_width` | 0.491 s |
| `tree_heights_with_nested_work_match_drawn_rows_at_wide_widths` | 0.557 s |
| `tree_heights_without_accounts_match_drawn_rows_at_every_width` | 0.468 s |
| `tree_heights_without_accounts_match_drawn_rows_at_wide_widths` | 0.597 s |

## Delegation Context

- **Project:** `cargo-handler` — the terminal agent watcher; each top-level agent gets a tile drawn by `agent_cell.rs`.
- **Project started:** 2026-10-16T09:43:44.158+00:00
- **Worktree:** `/home/natepiano/rust/cargo-liner-handler-enh`, branch `remove-running-section-handler-enh`.
- **Key files** (line refs are this branch at plan time):
  - `crates/cargo-handler/src/agent_cell.rs` — `TreeColumn` ≈243 (`header_width` ≈280); `TreeLayout::columns` ≈290; `TreeLine::lead` ≈315; `FULL_ACCOUNTS`/`FULL`/`COMPRESSED_ACCOUNTS`/`COMPRESSED` ≈636–669; `CellView::at_width` ≈679 (the only width-dependent view choice: `Full` or `Compressed`); `FlatColumns::fitted` ≈707 (full needs `header_line` and `place_line` to fit whole and `table_width(tree_widths(lines, full columns)) <= header_width(width)`); `height` ≈735, `view_height` ≈751; `draw` ≈778; `tree_widths` ≈833, `table_width` ≈848; `draw_tree` ≈857 (compressed: `spacing = TABLE_COLUMN_SPACING.min((area.width - 1) / gaps)` ≈875, the session column takes `area.width - fixed` ≈881, each name cut to what the column leaves after `lead()` ≈892); `above_tree` ≈933 (one line each, cut by `cut_line`); `header_line` ≈972, `place_line` ≈1013; `header_width` ≈1022 and `header_indented` ≈1027 (the tree area is the tile width less `SECTION_HEADER_INDENT` on each side); `mod tests` ≈1104 (`#[path = "tree_tests.rs"] mod tree_tests;` ≈1111, `lines` ≈1352, `boss_group` ≈1665, `natedev_cells` ≈1733).
  - `crates/cargo-handler/src/agent_cell/tests/tree_tests.rs` — `mockup` ≈24; `assert_heights_at_widths` ≈561; the six tests ≈582–611.
  - `crates/cargo-handler/src/constants.rs` — `TABLE_COLUMN_SPACING` ≈180.
- **Test:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-handler`
- **Lint:** `bash ~/.claude/scripts/delegate/verify.sh lint cargo-handler`
- **Invariants:**
  - Workers run only the Test/Lint lines above; each `verify.sh` run detaches and its result arrives as the worker's next message, so a worker never polls, sleeps or waits on one. Formatting only with `cargo +nightly fmt`.
  - Test-only change: no production code changes and no new production API. Private items of `agent_cell.rs` are visible to the test file through `super::*`.
  - No test takes a second.

## Phases

<!-- PHASE NUMBERING — binds every command that edits this doc.
     N is a bare integer, numbered from 1, contiguous, in execution order.
     Never a letter suffix (`4a`, `2b1a`) — those stop sorting and make ranges
     unreadable. Any spine edit (insert, split, merge, reorder, delete)
     resequences from the edit point to the end AND updates every cross-reference:
     substitute highest-first so `Phase 8` does not corrupt `Phase 12`, and
     re-check that each `Phases X–Y` range still spans the same set.
     Insert after the last `done` phase where possible — a done phase's number
     is baked into its checkpoint commit message and cannot be rewritten; if one
     must be renumbered, add an old→new mapping note to the doc.
     Full procedure: /plan:to_phased_plan → <PhaseNumbering/>. -->

### Phase 1 — Boundary widths  · status: done

#### Work Order

**Goal:** the tree height tests check each cell only at the widths where its layout changes, plus the width on either side, with each boundary computed from the layout's own width functions and constants, and each test runs in well under the time it does now.

**Spec:**
1. In `tree_tests.rs`, a test-only `fn layout_widths(cell: &AgentEntry<'_>, account_column: AccountColumn) -> BTreeSet<u16>` returns, for one cell, each boundary `b` below together with `b - 1` and `b + 1`, plus the range ends 10 and 140, clamped to `10..=140`. `indent` is `summary::cell_width(SECTION_HEADER_INDENT)`; a tree-area width `w` is the tile width `w + 2 * indent`. The boundaries, each computed with the module's own functions and constants, never a written-out number:
   - **Full view.** The narrowest tile width the full view fits: the largest of `header_line(…).width()`, `place_line(…).width()` and, when the cell has tree lines, `table_width(&tree_widths(&cell.lines, TreeLayout::Full.columns(account_column), NOW)) + 2 * indent`.
   - **Compressed spacing.** With `gaps` the compressed column count less one (`TreeLayout::Compressed.columns(account_column)`), the tree-area widths `k * gaps + 1` for `k` in `1..=TABLE_COLUMN_SPACING`, where `draw_tree`'s spacing steps.
   - **Session column.** With the compressed `tree_widths` past the session column summed as `fixed` plus `TABLE_COLUMN_SPACING * gaps`: the tree-area width `fixed` (session column empty) and `fixed` plus the widest `lead()` among the cell's lines (every name cut to nothing).
2. The derivation checks itself so it cannot drift from the view choice: when the full boundary `full` lies in range, `CellView::at_width(cell, full, NOW, account_column)` is `Full` and at `full - 1` is `Compressed`.
3. `assert_heights_at_widths` takes the fixture alone and checks each cell at its `layout_widths`, with the same height-equals-drawn-rows assertion and failure message as now.
4. The six tests become three, one per fixture, with the range split gone: `tree_heights_match_drawn_rows_at_layout_widths` (`mockup(["claude2", "claude1", "claude3"])`), `tree_heights_without_accounts_match_drawn_rows_at_layout_widths` (`mockup(["claude2"; 3])`) and `tree_heights_with_nested_work_match_drawn_rows_at_layout_widths` (`boss_group()`). Remove the `RangeInclusive` import if nothing else uses it.

**Files:**
- `crates/cargo-handler/src/agent_cell/tests/tree_tests.rs`

**Workers:** 1 tester — one test file.
- `test` — `crates/cargo-handler/src/agent_cell/tests/tree_tests.rs`

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-handler` and `bash ~/.claude/scripts/delegate/verify.sh lint cargo-handler` clean; the unit director times the three tests alone on the Mac and natedev, each well under the fastest of the six it replaces, and sends the showrunner the before and after times. The checkpoint carries one line, `Change-Review: none — test-only change; nothing visible changes`.
