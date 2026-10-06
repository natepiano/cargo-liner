# Production — cargo-port-cleanup

> **Status: PRODUCTION — running.** Removes the Running section of cargo-port's Targets pane and the process scan behind it. cargo-tile is the tool for watching running cargo processes.

## Production Context

- **Source plans:** one plan, which the showrunner wrote on 2026-10-06 from the user's request, with no producer split. It shipped, and the as-built doc `docs/cargo-port/as-built/targets-pane.md` replaced it.
- **Repository:** `/home/natepiano/rust/cargo-liner`
- **Merge branch:** `main`. The unit merges here, and only the showrunner pushes it.
- **Showrunner checkout:** `/home/natepiano/rust/cargo-liner`
- **Showrunner session:** cargo-liner
- **Log:** `docs/cargo-port/remove-running-section-log.md`. It is git-excluded and holds one line per event.
- **User zone:** America/Los_Angeles. Every time the showrunner reports is in this zone only.
- **Updates:** every 15 minutes. Each update reports every unit in full.
- **Merge tests:** none beyond the changed packages.
- **Capacity:** 32 cores, 60 GB (17 GB free on 2026-10-06), one unit.

## Units

| Unit | Plan | Worktree | Branch | Session | Port | Owns |
| --- | --- | --- | --- | --- | --- | --- |
| cleanup-unit | `docs/cargo-port/as-built/targets-pane.md` (as-built; plan done) | `/home/natepiano/rust/cargo-port-cleanup` | `cleanup/running` | `cargo-port-cleanup` | — | `crates/cargo-port`, `docs/cargo-port` |

## Hub files

| File | Owner unit | Other units that touch it |
| --- | --- | --- |

## Gates

| Gate | Waiting | Waits on | Clears when |
| --- | --- | --- | --- |

## Close-out

1. A final CI point: `validate_and_push.sh --to main` and its CI run green.
2. `/release cargo-port 0.8.0`. The user asked for this on 2026-10-06 because the removal is a breaking change, and the request pre-approves the release. The release config pins the path-only `tui_pane` dependency to its crates.io version and stops when `crates/tui_pane` has drifted from it. If it stops there, bring that to the user.
3. Install the released cargo-port on the Mac. The release's install check covers natedev.

## Production rules

- **The merge branch is `main`** (showrunner, 2026-10-06). With one unit, a separate merge branch would only add a promotion step. <PromoteMain/> does not apply. Each merge goes to main through `validate_and_push.sh --quick --to main`, and CI points through `validate_and_push.sh --to main`.
- **No UX guide.** cargo-port names none, so the design check is skipped. The unit's smoke run proves the Targets pane shows only its table, filling the pane.
- **Branch and session names** are the user's (2026-10-06): session `cargo-port-cleanup`, worktree `../cargo-port-cleanup`, branch `cleanup/running`.
