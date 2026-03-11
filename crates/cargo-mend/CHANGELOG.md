# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.1] - 2026-03-10

### Fixed
- Compatibility with nightly 1.96+ where `rustc_driver::catch_with_exit_code` returns `ExitCode` instead of `i32` (rust-lang/rust#150379)

## [0.1.0] - 2026-03-10

### Added
- Visibility auditing via rustc compiler analysis after macro expansion
- `pub(crate)` and `pub(in crate::...)` detection as hard errors
- `pub mod` review-or-allowlist enforcement
- Suspicious `pub` detection for items broader than their module boundary
- Wildcard `pub use *` re-export warnings
- Internal parent `pub use` facade detection
- `crate::`-relative import shortening to `super::` local-relative paths
- Auto-fix with `--fix` for import shortening with automatic rollback on `cargo check` failure
- Auto-fix with `--fix-pub-use` for narrowing child `pub` to `pub(super)` and removing stale parent re-exports
- `--dry-run` mode for previewing fixes without applying
- `--json` output for machine-readable reports
- `--fail-on-warn` flag for CI enforcement
- `mend.toml` configuration with `allow_pub_mod` and `allow_pub_items` allowlists
- Workspace-aware auditing with `--manifest-path` support
- Colored terminal output with `CARGO_TERM_COLOR`, `CLICOLOR`, and `CLICOLOR_FORCE` support
