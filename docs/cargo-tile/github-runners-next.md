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
