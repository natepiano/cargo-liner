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
  - Target: `crates/cargo-tile/src/capture_root.rs` — cleanup eligibility,
    currently `DirectoryIdentity::exclusive_owner` (`:363`) and
    `RootScan::access` (`:228`), including phase 6's diagnostic classification.
  - Why needed: the ownership prerequisite still checks uid and mode bits
    alone. A macOS ACL granting another account write access can pass that
    check; phase 4's registration verification does not close this ownership
    gap.
  - Completion condition: on macOS, non-owner ACL write access on the root,
    state, or pids directory prevents sweeping a verified ended registration
    and its log. The retained status identifies the directory and ACL reason
    cleanup is disabled while preserving the separately observed owner.
  - Revealed by: Phase 3
