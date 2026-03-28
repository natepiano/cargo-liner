# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed
- `inline_path_qualified_type` autofix no longer drops generic parameters (e.g., `crate::error::Result<T>` was incorrectly replaced with `Result` instead of `Result<T>`)
- `inline_path_qualified_type` autofix no longer adds `use` imports that shadow prelude types (e.g., adding `use crate::error::Result;` would break existing bare `Result<T, E>` usage in the same file)
- `prefer_module_import` no longer flags `use super::super::module_name;` where the leaf is a module, not a function
- `prefer_module_import` no longer flags function imports when the target module has a `mod` declaration in the same file (e.g., `mod input;` + `use crate::input::function;`)

## [0.2.0] - 2026-03-25

### Added
- `prefer_module_import` diagnostic: detects direct function imports and rewrites to module-qualified form (`use module` + `module::function()`)
- `inline_path_qualified_type` diagnostic: detects inline path-qualified types (`crate::module::MyType`) and adds `use` imports with bare type names
- Global configuration file at `~/.config/cargo-mend/config.toml` with per-diagnostic enable/disable, auto-created on first run
- Per-project `[diagnostics]` section in `mend.toml` that overrides global settings
- `--help` now shows diagnostic enable/disable status and config file path
- `--dry-run` alone now previews all fixes (no longer requires `--fix` or `--fix-pub-use`)
- `DiagnosticCode` enum for compile-time safe diagnostic code references
- Pre-1.0 warning in README about semver instability and destructive `--fix` behavior

### Changed
- `--fix` now activates all import-related fixes (`ShortenImport`, `PreferModuleImport`, `InlinePathQualifiedType`)
- Fix notice reports finding count instead of raw edit count, matching the summary line
- `OperationMode::from_cli` no longer returns `Result` (cannot fail)

### Fixed
- Overlapping fixes between `ShortenImport` and `PreferModuleImport` on the same `use` statement are resolved automatically
- Two-segment `super::` imports (`use super::module`) no longer falsely flagged as function imports
- Idempotency: running `--fix` twice produces zero findings on the second run
- Exempt depth-2 modules from `suspicious_pub`

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
