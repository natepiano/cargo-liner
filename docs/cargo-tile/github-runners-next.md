# cargo-tile and the local GitHub Actions runners — Next

## Items to consider

- [ ] **`cargo tile uninstall` reports each toolchain and continues**
  - Target: `crates/cargo-tile/src/cli.rs` — `uninstall()` (`:127`)
  - Why needed: `install` now reports a failing toolchain and continues to the
    next, while `uninstall` still abandons the whole loop on the first error. On
    a runner box with several toolchains, an uninstall that hits one broken
    toolchain leaves every later toolchain shimmed with nothing said, so cargo
    stays intercepted after an uninstall the operator believes succeeded. The
    asymmetry is pre-existing and unchanged at `79c8bb0c`.
  - Completion condition: with one toolchain that cannot be uninstalled and one
    that can, the command reports the failing toolchain by name and still
    restores `cargo` in the other.
  - Revealed by: Phase 1

- [ ] **The ownership check closes the macOS ACL hole it currently documents**
  - Target: `crates/cargo-tile/src/capture_root.rs` —
    `InspectedDirectoryMetadata::refusals` (`:518`) and
    `RootScan::cleanup_refusals` (`:303`), which gate private
    `RootScan::access` (`:352`); preserve `RootOwner` and add a
    path-qualified `CleanupRefusal` for ACL write access.
  - Why needed: the ownership prerequisite still checks uid and mode bits
    alone. A macOS ACL granting another account write access can pass that
    check; phase 4's registration verification does not close this ownership
    gap.
  - Completion condition: on macOS, non-owner ACL write access on the root,
    state, or pids directory prevents sweeping a verified ended registration
    and its log. The retained status identifies the directory and ACL reason
    cleanup is disabled while preserving the separately observed owner.
  - Revealed by: Phase 3

- [ ] **Every configured-root status remains reachable in Settings**
  - Target: `crates/cargo-tile/src/render.rs` — `draw_settings` (`:2128`),
    including viewport-to-rendered-line positioning.
  - Why needed: configured-root rows and their wrapped diagnostics can exceed
    the popup height, but the paragraph does not follow the settings viewport,
    so rows past the bottom edge cannot be reached.
  - Completion condition: with enough configured roots to overflow a small
    terminal, navigation reveals every root and the settings rows below them,
    keeps the selected row visible after resizing or status updates, and
    leaves root controls inert.
  - Revealed by: Phase 6

- [ ] **A promoted summary row accounts for the descendants it hides**
  - Target: `crates/cargo-tile/src/render.rs` — the summary promotion that draws
    a managed row in place of its hidden driver, with the per-row aggregation in
    `crates/cargo-tile/src/processes.rs`.
  - Why needed: a promoted row reports only its own process's share, so a
    descendant whose CPU is unavailable leaves no mark on the total the summary
    shows — the row can read as a measured value while part of the tree beneath
    it is unknown.
  - Completion condition: a summary row promoted over a hidden driver reports its
    whole subtree, and an unavailable descendant makes that total unavailable
    rather than a number.
  - Revealed by: Phase 7

- [ ] **The reader harness waits for the fixture's command pane instead of asserting it on the first frame**
  - Target: `crates/cargo-tile/tests/shim_registration.rs` — the `reader_regression`
    harness and its `fixture must occupy one command pane` assertion, on the
    exec-excluded scenario `reader_keeps_application_spawned_cargo_when_run_is_excluded`.
  - Why needed: the assertion fires on the first captured frame, so a slow first
    scan fails a scenario whose source is unchanged; observed once in three
    consecutive package runs on the same tree, passing on rerun. A gate that
    fails without a code change blocks checkpoints for nothing.
  - Completion condition: the harness polls for the expected pane count up to
    the scenario's existing deadline before asserting, and the exec-excluded
    scenario passes ten consecutive runs on the Linux machine.
  - Revealed by: Phase 9

- [ ] **Every reader of a shared ledger is upgraded before the first merge-extent reconciliation**
  - Target: `cargo-berth` installation and rollout — every CLI and hook
    entrypoint that reads a shared ledger, including explicit
    `CARGO_BERTH_EXECUTABLE` overrides; no in-repository owner.
  - Why needed: reconciliation appends a `merge_extent_observed` journal record
    on the first `board`, `check`, or `drift` against a ledger, and every older
    `cargo-berth` binary then refuses the whole ledger (`journal record N is
    corrupt: unknown variant merge_extent_observed`). Forwarding the executable
    to nested hooks does not upgrade independently invoked readers. Observed at
    record 82 of the ledger shared by every cargo-liner worktree, after which
    the installed binary failed every hook read.
  - Completion condition: every installed entrypoint reads a fixture ledger
    containing that operation, and the SessionStart and PostToolUse hooks return
    their expected protocol against it; where the record already exists, the
    remaining readers are upgraded rather than the ledger edited.
  - Revealed by: Phase 10

- [ ] **A waiting or deferred reservation exposes its extents on request**
  - Target: `crates/cargo-berth/src/board/report.rs`,
    `crates/cargo-berth/src/verb/board.rs`, `crates/cargo-berth/src/output.rs`,
    `docs/cargo-berth/generated/output-contract.json`,
    `crates/cargo-berth/tests/board.rs`
  - Why needed: board placement (`crates/cargo-berth/src/board/rows.rs`) keeps
    waiting successors and unresolved-overlap endpoints out of every
    reservation-snapshot section, and `board --reservation <id> --json` reports
    lifecycle only, so an operator holding one of those reservations cannot see
    its protected paths or retained evidence.
  - Completion condition: `board --reservation <id> --json` carries
    `race_extent` and `merge_extent`, including `unavailable` with retained
    evidence and `not_derived` with declared protection, and `tests/board.rs`
    verifies that waiting successors and both unresolved-overlap endpoints
    expose protected, empty, and unavailable extents alongside their lifecycle.
  - Revealed by: Phase 10

- [ ] **The durable evidence record carries the effective edit-blocking decision beside the checkpoint evidence**
  - Target: `crates/cargo-berth/src/reservation/lifecycle.rs` —
    `IntegrationEvidenceStatus::edit_blocking_status`;
    `crates/cargo-berth/src/reconcile.rs` and
    `crates/cargo-berth/src/verb/release.rs`, which persist its answer in
    `evidence_revalidated`; `crates/cargo-berth/tests/lifecycle.rs`.
  - Why needed: `edit_blocking_status()` answers `Clear` from checkpoint
    integration alone, and that answer is what the `evidence_revalidated`
    journal record persists. With an integrated checkpoint followed by later
    unmerged work, the persisted field reads `clear` while the reservation
    correctly keeps blocking through its merge extent, so a reader of the
    journal learns the wrong blocking state.
  - Completion condition: the persisted record states the checkpoint evidence
    and the effective protection decision as two facts, and the later-work
    regression in `tests/lifecycle.rs` asserts that the persisted decision
    agrees with the live blocking answer.
  - Revealed by: Phase 10
