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
| handler-unit | — (plan done; as-built in docs/cargo-handler/as-built/) | /home/natepiano/rust/cargo-liner-handler | cargo-liner-handler | — | crates/cargo-handler/, crates/tui_pane/ |
| port-unit | docs/cargo-port/as-built/ | /home/natepiano/rust/cargo-liner-port | cargo-liner-port | — | crates/cargo-port/, crates/cargo-mend/, docs/cargo-port/as-built/ |
| cargo-liner-style | docs/cargo-liner-style-production.md | /home/natepiano/rust/cargo-liner-style | style/main |  |  |

## Style units

| Unit | Families | Open tickets |
| --- | --- | --- |
| cargo-liner-style | names, values, types, modules | 1 |

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
| C201 — a waiting Codex worker shows in its unit's tree | screenshot 20:44 EST 2026-11-05: herdr workers with live detached runs showed `idle`; "yes send it back" | handler | [Detached-run activity](../cargo-handler/as-built/activity.md#detached-records-and-worker-rows) (d7771e0c3) | merged 63277286f (2026-11-06), installed on both machines; C218 returned 2026-11-08, fix in handler Phase 71 |
| C218 — a detached run shows on its pane worker's row | "i don't see it" (screenshot 2026-11-08: tile `impl` shows idle during its detached run; the runs show as separate Dispatch rows; worker `test` has no row while its run is live) | handler | [Pane-worker run joins](../cargo-handler/as-built/activity.md#detached-records-and-worker-rows) (881ce8801) | merged ebc3df967 (2026-11-08), installed on both machines; new change C269 waits for the user's check |
| C205 + C207 — project-list columns drop before names truncate | "i don't see columns for Type, Git, SYnc, Main - you're getting rid of columns before you make project names have an ellipipsis - columns should always be preserved - the project names should get ellipsis first"; C207: "i provided feedback about this - the choices in the project list aren't acceptable"; screenshot 2026-11-06: only Projects and Disk at about 95 columns | port | [Project columns](as-built/project-list.md) | merged 295393049 (2026-11-07) as C222; C222 returned 2026-11-09 by the showrunner check the user asked for: at 60 columns Type, Git, Sync and Main drop while names grow; fix in Phase 4v (29ce1bc2f), after 4s; fix merged 5df05544 (2026-11-10, port 4v), installed on natedev, Mac owed; new change C296 waits for the user's check |
| C224 — Package and Git at 80 columns | user asked the showrunner to validate; check found a cut Package label ('integ… 1294'), a cut pane title ('Worktree Gr…'), and a one-row Git list with no 'Worktrees (N)' title at 13 worktrees | port | Phase 4v (29ce1bc2f) | returned 2026-11-09; fix merged 5df05544 (2026-11-10), installed on natedev, Mac owed; new change C297 waits for the user's check |
| C227 — popups show focus | user asked the showrunner to validate; color mode draws a thin Sccache border where the check says thick in both modes | port | Phase 4v (29ce1bc2f) | returned 2026-11-09; fix merged 5df05544 (2026-11-10), installed on natedev, Mac owed; new change C298 waits for the user's check |
| C217 — tiles keep their places when the accounts table wraps | the user asked the showrunner to validate it (2026-11-07); the showrunner's live 80x45 check: the tiles switch between two columns and one column when a summary status gets wider | handler | [Stable layout](../cargo-handler/as-built/tile-layout.md#grid-admission-and-stable-layout) (d8b51889a) | fix merged 2026-11-07 (ee2c5c839); new change C257 waits for the user's check; showrunner live 80x45 capture, 150 s: two columns in all 75 frames |
