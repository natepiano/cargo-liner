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
    `DirectoryIdentity::exclusive_owner` (`:333`)
  - Why needed: the check is the whole basis for deleting anything in a capture
    root, and it reads mode bits alone — owner is the effective user, and neither
    group nor other may write. On macOS a POSIX ACL can grant another account
    write access without changing those bits, so a root the check calls
    exclusively owned can be writable by someone else, and the scanner will sweep
    in it. The comment on that function records the exposure rather than closing
    it. Linux ACL write masks do appear in the group bits, so this affects macOS
    alone; the runner boxes are Linux, which is why phase 3 shipped it this way.
  - Completion condition: on macOS, a capture root whose mode bits pass the check
    but which carries an ACL granting write access to another account is
    classified the same as a root that fails the mode check, and is not swept.
  - Revealed by: Phase 3
