# Production — cargo-port-cleanup

> **Status: PRODUCTION — running.** Wrapped 2026-10-06, reopened 2026-10-07 for the tile-mem unit, a memory column and total in cargo-tile, wrapped 2026-10-08, and reopened the same day for the tile-fixes unit, which removes a drawing artifact around a resizing cell in cargo-tile. Removes the Running section of cargo-port's Targets pane and the process scan behind it. cargo-tile is the tool for watching running cargo processes. A second unit, cargo-tile, joined on 2026-10-06: the summary takes as many cells as it needs and pushes the other cells on.

## Production Context

- **Formerly:** `remove-running-section`
- **Source plans:** one plan, which the showrunner wrote on 2026-10-06 from the user's request, with no producer split. It shipped, and the as-built doc `docs/cargo-port/as-built/targets-pane.md` replaced it. The cargo-tile unit ran a plan it wrote in its worktree. All three phases shipped, and the as-built doc `docs/cargo-tile/as-built/summary-depth.md` replaced it.
- **Repository:** `/home/natepiano/rust/cargo-liner`
- **Merge branch:** `main`. The unit merges here, and only the showrunner pushes it.
- **Showrunner checkout:** `/home/natepiano/rust/cargo-liner`
- **Showrunner session:** cargo-liner
- **Log:** `docs/cargo-port/cargo-liner-log.md`. It is git-excluded and holds one line per event.
- **Change review:** `~/rust/hanadocs/change-review/change-review - cargo-liner.md`
- **Proposals:** `~/rust/hanadocs/issues`
- **Review pool:** code
- **User zone:** `America/New_York`. Every time the showrunner reports is in this zone only.
- **Updates:** on demand. Each update reports every unit in full.
- **Merge tests:** `bash /home/natepiano/.claude/scripts/direct/verify.sh test cargo-tile`, `bash /home/natepiano/.claude/scripts/direct/verify.sh test cargo-handler`, `bash /home/natepiano/.claude/scripts/direct/verify.sh test cargo-port` (they build on tui_pane)
- **Capacity:** 32 cores, 60 GB (17 GB free on 2026-10-06), three units.

## Units

| Unit | Plan | Worktree | Branch | Port | Owns |
| --- | --- | --- | --- | --- | --- |
| tile-unit | docs/cargo-port/remove-running-section-tile-fixes.md | /home/natepiano/rust/cargo-liner-tile | remove-running-section-tile | — | crates/tui_pane, crates/cargo-tile, crates/cargo-handler, docs/cargo-tile |
| handler-unit | docs/cargo-port/remove-running-section-handler-footer-colors.md | /home/natepiano/rust/cargo-liner-handler | remove-running-section-handler | — | crates/cargo-handler/, crates/tui_pane/ |
| port-unit | docs/cargo-port/cargo-liner-port.md | /home/natepiano/rust/cargo-liner-port | cargo-liner-port | — | crates/cargo-port/, crates/cargo-mend/, docs/cargo-port/cargo-liner-port.md, docs/cargo-port/as-built/ |

## Hub files

| File | Owner unit | Other units that touch it |
| --- | --- | --- |

## Gates

| Gate | Waiting | Waits on | Clears when |
| --- | --- | --- | --- |
| tui_pane after the release | cargo-tile phases that change `crates/tui_pane` | the cargo-port 0.8.0 release | cleared 2026-10-06: tui_pane 0.8.0, then cargo-port 0.8.0 pinned to it, both published |

## Close-out

1. A final CI point: `validate_and_push.sh --to main` and its CI run green.
2. `/release cargo-port 0.8.0`. The user asked for this on 2026-10-06 because the removal is a breaking change, and the request pre-approves the release. The release config pins the path-only `tui_pane` dependency to its crates.io version and stops when `crates/tui_pane` has drifted from it. If it stops there, bring that to the user. Done 2026-10-06: it stopped on tui_pane drift, and the user chose to release tui_pane 0.8.0 first.
3. Install the released cargo-port on the Mac. The release's install check covers natedev. Done 2026-10-06: 0.8.0 on both machines.
4. After cargo-tile's last phase merges and CI is green: install cargo-tile and cargo-handler from main on natedev and on the Mac. Done 2026-10-06: installed from 54cb9b11 on both machines.

## Production rules

- **The merge branch is `main`** (showrunner, 2026-10-06). Its units touch separate crates, so a separate merge branch would only add a promotion step. <PromoteMain/> does not apply. Each merge goes to main through `validate_and_push.sh --quick --to main`, and CI points through `validate_and_push.sh --to main`.
- **No UX guide.** cargo-port names none, so the design check is skipped. The unit's smoke run proves the Targets pane shows only its table, filling the pane.
- **Branch and session names** are the user's (2026-10-06): session `cargo-port-cleanup`, worktree `../cargo-port-cleanup`, branch `cleanup/running`.

## Returned changes

Changes the user sent back from change review. Each row stays until its fix merges and the user lands the new change. The showrunner checks this list when a unit resumes and at each merge from that unit.

| Change | What the user said | Unit | Fix phase (plan commit) | State |
| --- | --- | --- | --- | --- |
| C201 — a waiting Codex worker shows in its unit's tree | screenshot 20:44 EST 2026-11-05: herdr workers with live detached runs showed `idle`; "yes send it back" | handler | Phase 62h in `remove-running-section-handler-footer-colors.md` (d7771e0c3) | merged 63277286f (2026-11-06), installed on both machines; waits for the user's check of C218 |
| C205 + C207 — project-list columns drop before names truncate | "i don't see columns for Type, Git, SYnc, Main - you're getting rid of columns before you make project names have an ellipipsis - columns should always be preserved - the project names should get ellipsis first"; C207: "i provided feedback about this - the choices in the project list aren't acceptable"; screenshot 2026-11-06: only Projects and Disk at about 95 columns | port | Phase 4c Spec 24 in `cargo-liner-port.md` (48db7e7be) | merged 295393049 (2026-11-07); live check at 95 columns shows every column with names cut; installed on both machines; waits for the user's check of C222 |
| C217 — tiles keep their places when the accounts table wraps | the user asked the showrunner to validate it (2026-11-07); the showrunner's live 80x45 check: the tiles switch between two columns and one column when a summary status gets wider | handler | Phase 68 in `remove-running-section-handler-footer-colors.md` (d8b51889a) | fix merged 2026-11-07 (ee2c5c839); new change C257 waits for the user's check; showrunner live 80x45 capture, 150 s: two columns in all 75 frames |
