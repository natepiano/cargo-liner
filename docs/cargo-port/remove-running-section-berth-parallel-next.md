# berth-parallel — Next

## Items to consider

- [ ] **Initializing berth leaves no untracked `.claude/` in trunk**
  - Target: `crates/cargo-berth` (`init`)
  - Why needed: `init` leaves an untracked `.claude/` directory in the trunk worktree.
  - Completion condition: after `cargo-berth init`, `git status --short` in trunk shows no `.claude/` entry.
  - Revealed by: showrunner ruling (5), before phase 9

- [ ] **The board shows reservations created after it opens, and the Overview holds no raw diagnostics**
  - Target: `crates/cargo-berth/src/board/tui.rs` (`event_loop`)
  - Why needed: the TUI board draws one model and never rebuilds it, so reservations created after it opens never appear. The raw `git_cost` and `journal_position` JSON below the Overview's Reservations table are diagnostics, not Overview content.
  - Completion condition: a reservation created while the board is open appears in the Overview and Reservations panes without a restart, and the Overview no longer shows the raw `git_cost` and `journal_position` JSON.
  - Revealed by: Phase 10 (showrunner, from the phase 10 shot)

- [ ] **A single-reservation board answer includes its ordering edges and overlap answers**
  - Target: `crates/cargo-berth` (`board --reservation <id> --json`)
  - Why needed: `board --reservation <id> --json` carries no ordering edges or overlap answers for that reservation.
  - Completion condition: `board --reservation <id> --json` includes the reservation's ordering edges and overlap answers.
  - Revealed by: the showrunner's C5 check
