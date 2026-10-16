# handler-footer-colors

> **Status: IMPLEMENTATION PLAN — phased, delegate-ready.** cargo-handler colors roles and accounts with the colors the Claude Code footer uses, read from the footer's own color file, and the summary's machine headings count "top-level agents".

> **Production: remove-running-section** — unit `handler-enh-unit`; production doc `docs/cargo-port/remove-running-section-production.md`

## Source

2026-10-16, the user (via the showrunner): "we now use role colors in our clauide code footers - we should use the same role colors for the role columns in handler - and in the summary it should say "8 top level 8agents" not "8 agents"", then "similarly we have colored accounts in the claude code footer - we should use the same colors for the accounts taht we show in handler".

Showrunner rulings: read `~/.claude/config/statusline.conf` so there is one source, never copy its values into cargo-handler; map `showrunner`→`role.showrunner`, `unit`→`role.unit-director`, `impl`/`test`/`review`/`fix`→`role.worker`; `shell` and `subagent` keep plain role text; a missing file means plain text. The count reads "8 top-level agents". Color the account column and the summary's account text from any `account.<label>` line matching the shown label; a label with no line stays plain, and adding a line later colors it with no code change. Use the footer's color names exactly as `~/.claude/scripts/statusline/statusline.jq` renders them. One phase, with a Change-Review line; the checkpoint notice carries a shot of the hana tile and the summary.

Unit director's calls: `session` (an agent with no role) takes `role.unassigned`, the footer's color for a session with no role; `thread` stays plain like the other work lines. The tile header line's agent label takes the account color too, so an account reads the same color everywhere it is shown. The file is read again when its modification time changes, so an edit shows without a restart.

## Delegation Context

- **Project:** `cargo-handler` — the terminal agent watcher; one tile per top-level agent, plus a summary tile.
- **Project started:** 2026-10-16T13:40:00+00:00
- **Worktree:** `/home/natepiano/rust/cargo-liner-handler-enh`, branch `remove-running-section-handler-enh`.
- **Footer color file:** `~/.claude/config/statusline.conf`, lines `key=value`, `#` starts a comment, blank lines ignored. Today: `role.showrunner=bright-magenta`, `role.unit-director=bright-blue`, `role.worker=bright-cyan`, `role.unassigned=purple`, `account.claude1=cyan`, `account.claude2=magenta`, `account.claude3=blue`, `account.claude4=orange`, plus `level.*`/`tokens.*`/`left.*` keys handler ignores. The footer's name→ANSI table (`statusline.jq`, `def color`): black 30, red 31, green 32, yellow 33, blue 34, magenta 35, cyan 36, white 37, bright-black 90, bright-red 91, bright-green 92, bright-yellow 93, bright-blue 94, bright-magenta 95, bright-cyan 96, bright-white 97, orange `38;5;208`, purple `38;5;141`, pink `38;5;211`, teal `38;5;30`, brown `38;5;137`; any other name renders plain.
- **Key files** (line refs are main 7eacc2e3):
  - `crates/cargo-handler/src/agent_cell.rs` — `RoleLabel::role_text` ≈179; `TreeLine::account` ≈333, `TreeLine::value` ≈350 (`TreeColumn::Role` ≈353, `TreeColumn::Account` ≈364); `draw_tree` ≈857 (non-status columns draw in `text`); `header_line` ≈972.
  - `crates/cargo-handler/src/census/placement.rs` — `RoleLabel` ≈71.
  - `crates/cargo-handler/src/census/mod.rs` — `AgentRow::agent_label` ≈328.
  - `crates/cargo-handler/src/summary/mod.rs` — `heading` ≈220, `count_note` ≈250, `agent_row` ≈259.
  - `crates/cargo-handler/src/constants.rs` — `AGENT_SINGULAR`/`AGENT_PLURAL` ≈188–190.
  - `crates/cargo-handler/src/theme.rs` — `Role` ≈58 (theme roles; footer colors are not theme roles).
  - `crates/cargo-handler/src/app.rs`, `render.rs` — where per-frame state reaches `agent_cell::draw` and `summary::draw`.
- **Test:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-handler`
- **Lint:** `bash ~/.claude/scripts/delegate/verify.sh lint cargo-handler`
- **Invariants:**
  - Workers run only the Test/Lint lines above; each `verify.sh` run detaches and its result arrives as the worker's next message. Formatting only with `cargo +nightly fmt`.
  - No color value from the footer file is written into cargo-handler; tests use fixture text.
  - Invalid states are unrepresentable: a footer color is an enum parsed from its name, never a raw string carried to draw time.
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

### Phase 1 — Footer colors and top-level count  · status: done

#### Work Order

**Goal:** role and account text in cargo-handler takes the Claude Code footer's colors, read from the footer's color file, and each summary machine heading counts top-level agents.

**Spec:**
1. New module `crates/cargo-handler/src/footer_colors/mod.rs`:
   - `enum FooterColor` with one variant per name in the footer's table (Delegation Context), `FromStr` from exactly those names, and `From<FooterColor> for ratatui::style::Color` matching the footer's ANSI codes: the eight basic names → `Black, Red, Green, Yellow, Blue, Magenta, Cyan, Gray` (SGR 30–37), the eight `bright-*` names → `DarkGray, LightRed, LightGreen, LightYellow, LightBlue, LightMagenta, LightCyan, White` (SGR 90–97), `orange` → `Indexed(208)`, `purple` → `Indexed(141)`, `pink` → `Indexed(211)`, `teal` → `Indexed(30)`, `brown` → `Indexed(137)`.
   - `enum FooterRole { Showrunner, UnitDirector, Worker, Unassigned }`, keyed `role.showrunner`, `role.unit-director`, `role.worker`, `role.unassigned`.
   - `struct FooterColors { roles, accounts: BTreeMap<String, FooterColor> }` with `parse(&str) -> Self` (statusline.jq's rules: strip `#…`, trim, split at the first `=`, trim both sides; an unknown color name or an unknown key leaves that entry uncolored), `role(FooterRole)` and `account(&str)`. Both answer a domain enum, not `Option` (Type Design Contract): `enum FooterTint { Footer(FooterColor), Plain }`, where `Plain` means the file gives no color and the cell keeps its theme style. `FooterColors::default()` colors nothing.
   - The file is `$HOME/.claude/config/statusline.conf`. A missing or unreadable file is `FooterColors::default()`. The app holds the parsed colors with the file's modification time and reads the file again when that time changes, checked once per census scan, never per frame.
2. Role column (`TreeColumn::Role` in `draw_tree`): `showrunner` → `FooterRole::Showrunner`, `unit` → `UnitDirector`, `impl`/`test`/`review`/`fix` → `Worker`, `session` → `Unassigned`; `subagent` and `thread` stay in `text`. A shell line's role cell shows `—`, drawn like the other missing values, instead of `shell` (user, 2026-10-16: "i think we should just pub a emdash for the shell dispatch - it seems unnecessary"). A role with no color in the file draws in `text`. The mapping is one exhaustive `match` on the role type, not string compares.
3. Account text: wherever an account label is drawn — the tree's `Account` column, the label in `header_line`, and the summary's `agent` column (`agent_row`) — the label part takes `FooterColors::account(label)` when it has one (e.g. `claude1`, and `codex1` once the file gains a line for it). The tier mark (` fast`) and a program name shown in place of an account (`AccountColumn::Hidden`, `Account::Unrecorded`) keep today's styles.
4. `count_note`: `1 top-level agent`, `<n> top-level agents`; `no agents` unchanged. Update `AGENT_SINGULAR`/`AGENT_PLURAL` (or their replacements) and every test that reads the heading.
6. A worker dispatch folds into its worker's row (user, via the showrunner, 2026-10-16). A shell whose command is an `implement.sh` dispatch (`bash ~/.claude/scripts/delegate/implement.sh [--to <seat>] [--phase <id>] <session dir> <worktree> <prompt> <task> "<job text>" …`) names a seat. The seat's thread id is read from `<session dir>/mesh_roster.json`, where the seat's entry carries `thread_id` (and `launcher_pid`, the dispatch's own pid). With no `--to`, the seat is the roster entry whose `launcher_pid` is the dispatch's pid. When that thread is in the same tile, the shell draws no row of its own, and the thread's row is named `<seat short name> · <job text>`, where the short name is the seat with its `<worktree directory name>-` prefix removed (e.g. `impl7 · fix the slow saved-scene handover…`), instead of the thread's own label. A dispatch whose seat is not in the roster, or whose thread is not in the tile, keeps today's shell row. Model it in the types: the census parses the shell command once, at scan time, into a dispatch that names a seat and its thread id (a new module `crates/cargo-handler/src/census/worker_dispatch/mod.rs`), and the tree joins on that type; no string checks in the render. Reading the roster is part of the scan, never per frame.
7. A Codex thread's status column shows its real state (user, via the showrunner, 2026-10-16: "for the codex worker rows ... we should show whether the codex session is busy or idle - which certainly can be the case for long running workers"). Applies to every Codex thread row: managed worker rows (`fix`/`impl`/`test`/`review`) and `thread` work lines. The signal is the thread's rollout, which the scan already reads incrementally (`census/tier.rs` `Rollouts::read` → `RolloutRead`): its last turn event among `event_msg` payload types `task_started`, `task_complete`, `turn_aborted`. `task_started` last means busy; `task_complete` or `turn_aborted` last means idle; a rollout with none yet is idle. A thread is listed only while an app-server holds its rollout open, so a dead server cannot leave a stale busy row. The state is an enum on the thread's row, set at scan time. It draws with the words and colors Claude rows use for busy and idle (`summary::status_text`/`status_role`). The tier mark (` fast`) leaves the status column and follows the account label in the tree's `Account` column, as it does in the header line and the summary.
8. Tests: `FooterColors::parse` over fixture text (comments, blanks, spaces round `=`, unknown names and keys, `level.*` ignored); every `FooterColor` name maps to the footer's code; a drawn tile's role and account cells carry the parsed colors (read back from the buffer); no file colors nothing; heading counts read `1 top-level agent` / `3 top-level agents`. Width tests (`tree_tests.rs` `layout_widths`) still pass, since colors change no widths — the summary heading's text does, so its width tests move with it.

**Files:**
- `crates/cargo-handler/src/footer_colors/mod.rs`, `crates/cargo-handler/src/main.rs`
- `crates/cargo-handler/src/footer_colors/tests.rs`
- `crates/cargo-handler/src/agent_cell.rs`, `crates/cargo-handler/src/agent_cell/tests/`
- `crates/cargo-handler/src/summary/mod.rs`, `crates/cargo-handler/src/summary/footer_tests.rs`
- `crates/cargo-handler/src/constants.rs`
- `crates/cargo-handler/src/census/worker_dispatch/mod.rs`, `crates/cargo-handler/src/census/worker_dispatch/tests.rs`, `crates/cargo-handler/src/census/mod.rs`, `crates/cargo-handler/src/census/scan.rs`, `crates/cargo-handler/src/census/tree.rs`, `crates/cargo-handler/src/census/placement.rs`, `crates/cargo-handler/src/census/tier.rs`, `crates/cargo-handler/src/census/thread_state_tests.rs`
- `crates/cargo-handler/src/app.rs`, `crates/cargo-handler/src/render.rs`, `crates/cargo-handler/src/terminal.rs`
- `crates/cargo-handler/CHANGELOG.md`

**Workers:** 1 writer + 1 tester.
- `impl` — every production file above, including `main.rs` and the `mod` lines that wire the test files (`#[cfg(test)] mod tests;` in `footer_colors/mod.rs`, a `#[path = "footer_color_tests.rs"] mod footer_color_tests;` beside `tree_tests` in `agent_cell.rs`'s test module, a `#[path = "footer_tests.rs"] mod footer_tests;` in `summary/mod.rs`'s test module), and the existing heading assertions inside `summary/mod.rs` and `agent_cell/tests/tree_tests.rs` that read `agents`.
- `test` — `crates/cargo-handler/src/footer_colors/tests.rs`, `crates/cargo-handler/src/agent_cell/tests/footer_color_tests.rs`, `crates/cargo-handler/src/summary/footer_tests.rs`.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-handler` and `bash ~/.claude/scripts/delegate/verify.sh lint cargo-handler` clean; a live shot of the hana tile and the summary shows role and account colors matching the footer and the `top-level agents` count. The checkpoint carries `Change-Review: role and account colors match the Claude Code footer; summary headings count top-level agents | open cargo-handler, compare a tile's role and account cells with that session's footer, and read a machine heading`.
