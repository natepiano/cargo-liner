# cargo-tile and the local GitHub Actions runners — Next items

## Items to consider

- [ ] **The macOS `shim_registration` failures**
  - Target: `cargo-tile` — `crates/cargo-tile/tests/shim_registration.rs` and the shim it drives
  - Why needed: `fifo_removal_failure_preserves_original_cargo_and_unowned_directory`, `publication_links_a_complete_tmp_file_into_place`, and `stale_invocation_fifo_still_publishes_registration_and_captures_stderr` fail natively on macOS on the current tree and equally on a `git archive` of the Phase 2 checkpoint (1449 passed / 3 failed on each), so they predate the current work and no phase owns them. Two later phases name a green native `cargo-tile` suite in their gates, and neither can meet it until these are resolved. Check the raw-versus-canonical temporary-root mismatch first: `InstalledShim::new` (`crates/cargo-tile/tests/shim_registration.rs:1197`) embeds the raw temporary path while `InstalledShim::path` canonicalizes it and the FIFO observer matches the canonical spelling literally, so an aliased temporary root can defeat fault injection and invalidate publication-path comparisons. The forwarded argv missing a `--quiet` the expectation carries is consistent with the shim's existing JSON capture rewrite (`crates/cargo-tile/src/cargo-capture-shim.sh:427`). Developer impact is measured; operator impact is not yet established and the work should establish it rather than assume it.
  - Completion condition: all three cases pass natively on macOS with the argv, umask, stderr-capture, publication, and cleanup assertions preserved, the complete `cargo-tile` package suite green on both Linux and macOS, and each of the three recorded as a regression.
  - Revealed by: Phase 3
