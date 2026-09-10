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
    `DirectoryIdentity::refusals` (`:458`) and
    `RootScan::cleanup_refusals` (`:279`), which gate private
    `RootScan::access` (`:328`); preserve `RootOwner` and add a
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
  - Target: `crates/cargo-tile/src/render.rs` — `draw_settings` (`:1992`),
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
