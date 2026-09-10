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

- [ ] **Every account line in Settings remains reachable**
  - Target: `crates/cargo-tile/src/render.rs` — `draw_settings` (`:2128`),
    including viewport-to-rendered-line positioning.
  - Why needed: the capture section lists the shared directory and one line per
    account directory, and those lines with their wrapped diagnostics can exceed
    the popup height; the paragraph does not follow the settings viewport, so
    rows past the bottom edge cannot be reached.
  - Completion condition: with enough account directories to overflow a small
    terminal, navigation reveals every account line and the settings rows below
    them, keeps the selected row visible after resizing or status updates, and
    leaves the capture lines inert.
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

- [ ] **`processes.rs` and `progress.rs` are relocated by behavior, then split**
  - Target: `crates/cargo-tile/src/processes.rs` (2773 non-test lines, 43
    top-level types) and `crates/cargo-tile/src/progress.rs` (1205 non-test
    lines, 23 top-level types).
  - Why needed: both files now meet two of the style guide's split criteria
    (line count and several independent type clusters). `processes.rs` holds
    process identity (`InvocationId`, `RunId`, `ProcessIdentity`), capture
    attribution (`CaptureAssociation`, `DirectCapture`, `NearestRegistration`),
    CPU measurement (`CpuBaseline`, `CpuPublication`, `CpuSmoothing`), and the
    `Census` scan in one file; `progress.rs` holds capture-root lookup
    (`CaptureRoots`, `CaptureRootEnvironment`, `RegisteredRuns`) beside the
    `Progress` counter state.
  - Completion condition: each type first moves to the module that owns its
    sole constructor or single consumer; what remains splits into
    anchor-named submodules, with every file under the guide's line threshold
    or holding one type cluster, and the package tests unchanged.
  - Revealed by: style review

- [ ] **`capture_root.rs` is anchored and split**
  - Target: `crates/cargo-tile/src/capture_root.rs` (924 non-test lines, 23
    top-level types; the anchor type is `RootScan`, not `CaptureRoot`).
  - Why needed: a new file over the line threshold with three clusters —
    directory inspection (`InspectedDirectory`, `Inventory`, `ScanEntry`),
    root identity (`RootHistory`, `TreeIdentity`, `RootIncarnation`), and sweep
    policy (`SweepBudget`, `SweepDisposition`, `OwnedRoot`) — under a module
    name that no type carries.
  - Completion condition: the module directory is named for its anchor type,
    each cluster is an anchor-named submodule, and the package tests are
    unchanged.
  - Revealed by: style review

- [ ] **`birth_stamp/mod.rs` keeps only the module-name type**
  - Target: `crates/cargo-tile/src/birth_stamp/mod.rs`.
  - Why needed: the module root declares `linux` and `macos` and then defines
    `ProcessLifetime`, `LifetimeEvidence`, `IdentityEvidence`, `Observation`,
    `KernelObservation`, and `Verification` beside `BirthStamp`; none of them
    is a field type of `BirthStamp`, so the root is not a table of contents.
  - Completion condition: the root holds the `mod` block, re-exports, and
    `BirthStamp`; the other types live in leaf files named for them, and the
    package tests are unchanged.
  - Revealed by: style review

- [ ] **A `cargo install` run shows its build progress like a `cargo build` run**
  - Target: the shim's command classification and the reader's progress parsing in `crates/cargo-tile/src/cargo-capture-shim.sh` and `crates/cargo-tile/src/progress.rs`.
  - Why needed: `cargo install` compiles the same way `build` does, but its row in cargo tile carries no progress counter while the build runs; the user watched an install with nothing moving.
  - Completion condition: an integration test captures a `cargo install --path <fixture>` run and the reader reports the same compiling counter it reports for `cargo build`; the settings pane needs no change.
  - Revealed by: the user, watching an install during the Mac smoke check (2026-09-10).

- [ ] **`uninstall` and `status` gain `--all-accounts` counterparts to the admin install**
  - Target: `crates/cargo-tile/src/cli.rs` — beside `install_all_accounts()`
    (`:182`), reusing the per-account re-execution in
    `crates/cargo-tile/src/hook.rs` — `install_accounts` (`:423`).
  - Why needed: root can install the shim for every account in one command, but
    removing it or checking it still means acting as each account in turn; an
    operator who installs for the runner accounts has no matching way to see
    the result or undo it.
  - Completion condition: `sudo cargo-tile uninstall --all-accounts` restores
    `cargo` for every account's toolchains, `status --all-accounts` prints one
    line per account and toolchain, and both go through the same per-account
    child protocol as the install.
  - Revealed by: final gate — shared capture directory repair

- [ ] **The per-account install report keeps an orphaned toolchain visible**
  - Target: `crates/cargo-tile/src/hook.rs` — `install_accounts`
    (`:423`), where the child's per-toolchain report lines are
    folded into one outcome per account.
  - Why needed: one installed or refreshed toolchain line marks the whole
    account installed, so a sibling toolchain the child reported as orphaned is
    folded away and the operator reads the account as fully installed.
  - Completion condition: an account whose child reports one installed and one
    orphaned toolchain is reported with the orphaned toolchain named, and a test
    drives that report through the child protocol in
    `crates/cargo-tile/tests/support/shared_capture.rs`.
  - Revealed by: final-gate closure review 2

- [ ] **The per-account install child carries the account's supplementary groups**
  - Target: `crates/cargo-tile/src/hook.rs` — `install_account`
    (`:442`), the command that re-executes cargo-tile with the
    account's uid and gid.
  - Why needed: setting the uid alone leaves the child with root's supplementary
    groups cleared and the account's never set, so a toolchain behind a
    group-only-readable ancestor (mode 0770) is invisible to the child and
    reported as absent.
  - Completion condition: the child starts with the account's group list
    (`getgrouplist` through `CommandExt::groups`), and a test with a toolchain
    under a 0770 group-owned directory reports it installed.
  - Revealed by: final-gate closure review 2

- [ ] **The runner services reach the shared capture directory and drop the per-root environment**
  - Target: the Linux runner units in the NixOS configuration (owned by the
    natedev session) and the Mac runner's launchd plist; no in-repository owner.
  - Why needed: the units still set `CARGO_TILE_ROOT` and pre-create 0750
    capture directories for the removed per-root design, and the Mac plist still
    exports `CARGO_TILE_ROOT`; the shim now writes under `/tmp/cargo-tile/<uid>`,
    which a unit with a private `/tmp` cannot reach without
    `BindPaths=/tmp/cargo-tile`.
  - Completion condition: each Linux runner unit binds `/tmp/cargo-tile` into
    its namespace and no longer sets `CARGO_TILE_ROOT` or creates the 0750
    directories, the Mac plist no longer sets `CARGO_TILE_ROOT`, and a CI job on
    each machine produces a `[hana-ci]` row in cargo tile.
  - Revealed by: final gate — shared capture directory repair

- [ ] **The integration tests stop scrubbing the removed `CARGO_TILE_ROOT` variable**
  - Target: `crates/cargo-tile/tests/shim_modes.rs` (`:147`) and
    `crates/cargo-tile/tests/shim_registration.rs` (`:225`, `:1225`).
  - Why needed: the shim no longer reads `CARGO_TILE_ROOT`, so the `env_remove`
    calls and the scrub list entry guard against a variable nothing consults;
    a reader of the tests is led to look for an override that does not exist.
  - Completion condition: the three references are gone and the package tests
    are unchanged.
  - Revealed by: final gate — shared capture directory repair
