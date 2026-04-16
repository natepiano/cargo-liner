# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.7.0] - 2026-04-16

### Added
- `cargo mend --version` now reports the installed CLI version
- `cargo mend --build-info` now prints build metadata including git hash, build id, and the sysroot used to compile the binary

### Changed
- `cargo-mend` now uses stable Rust for development and installation; install with `rustc-dev` and `RUSTC_BOOTSTRAP=1`

## [0.6.1] - 2026-04-13

### Fixed
- `cargo mend --fix` no longer fails to qualify function references inside macro invocations (e.g., `matches!`, `assert!`), preventing rollback-on-compile-error when `prefer_module_import` rewrites imports for functions called within macros
- `--json` mode no longer leaks cargo `Building` progress lines to stderr

## [0.6.0] - 2026-04-11

### Added
- `--json` output now emits cargo-compatible newline-delimited JSON with `compiler-message` and `build-finished` messages, including `package_id`, `manifest_path`, `target`, and full rustc diagnostic structure
- Positional manifest path argument: `cargo mend /path/to/project` as an alias for `--manifest-path`, accepting both `Cargo.toml` paths and directories

### Fixed
- `--fix-pub-use` no longer breaks compilation when items from a facade module are accessed through a renamed import (e.g., `use crate::module as alias`)

## [0.5.1] - 2026-04-09

### Fixed
- Installation instructions now show the working `rustc-dev` + `RUSTC_BOOTSTRAP=1 cargo install cargo-mend` path, with the nightly install flow as an alternative

## [0.5.0] - 2026-04-09

### Added
- New `--fix-compiler` mode runs `cargo fix` for compiler-fixable warnings; `--fix-all` now applies mend fixes, `pub use` fixes, and compiler fixes together
- `cargo mend` now prints a timing footer with total, check, and mend-analysis durations
- Added CLI smoke tests covering default package runs, workspace selection, `--all-targets`, `--lib`, and named `--example` selection

### Changed
- Removed the toolchain override that forced nightly compilation into an isolated `target/mend/` directory — the wrapper now shares the project's normal `target/` directory, eliminating the multi-gigabyte duplicate build artifacts and 17-20s rebuild penalty on every file change
- The `--cfg=mend_refresh_{pid}` cache-buster now uses a stable `--cfg=mend_refresh` flag, producing one reusable set of artifacts instead of unique unreusable ones per invocation that caused unbounded target directory growth
- `cargo mend` now follows a single-pass `cargo check` flow with cleaner target selection and reporting
- Compiler warning summaries and human-readable output were refined to better separate compiler warnings from mend findings

### Fixed
- `cargo mend --fix` no longer inserts invalid file-scope imports for nested-module `inline_path_qualified_type` rewrites, preventing rollback-on-compile-error failures during autofix

### Performance
- Analysis now caches source file contents instead of re-reading files repeatedly during compiler-driven checks
- Source files are parsed to ASTs once and reused, avoiding repeated `syn::parse_file` work
- AST paths are pre-extracted up front, removing repeated visitor walks during per-query analysis

## [0.4.0] - 2026-04-06

### Added
- New `narrow_to_pub_crate` diagnostic: warns when `pub` items in top-level private modules are not re-exported by the crate root, and auto-fixes them to `pub(crate)`

## [0.3.2] - 2026-04-05

### Fixed
- `suspicious-pub` no longer flags `pub(crate)` in top-level private modules of binary crates

## [0.3.1] - 2026-04-04

### Fixed
- Fix `suspicious-pub` false positives for methods on types whose definition and `impl` blocks live in separate child modules

## [0.3.0] - 2026-04-03

### Added
- New `replace_deep_super_import` diagnostic (warning, auto-fixable with `--fix`) — detects `super::super::` and deeper import chains and suggests the named `crate::` path instead, at any depth

## [0.2.7] - 2026-04-03

### Fixed
- `cargo mend` no longer re-refreshes example-only and `src/bin/*` packages on every run; it now writes and reuses findings caches for those targets, preventing repeated growth in `target/mend/`

## [0.2.6] - 2026-03-30

### Fixed
- `suspicious_pub` and `internal_parent_pub_use_facade` now walk ancestor module boundaries when checking for re-exports — previously only checked the immediate parent, causing false positives when the re-export was at a grandparent or higher

## [0.2.5] - 2026-03-29

### Fixed
- Toolchain override now uses `CARGO_TARGET_DIR` env var instead of `--target-dir` arg — the arg was placed after the `--` separator in the fallback compilation path, causing rustc to receive it instead of cargo

## [0.2.4] - 2026-03-28

### Fixed
- Auto-detect toolchain mismatch between the mend binary and target project — when the binary was compiled with a different rustc than the project's default, mend now forces the matching toolchain and uses an isolated target directory (`target/mend/`) to avoid corrupting the project's build cache

## [0.2.3] - 2026-03-28

### Fixed
- Compiler driver no longer forces `RUSTUP_TOOLCHAIN=nightly`, using the caller's toolchain instead — prevents `E0514` errors when the mend binary was compiled with a different rustc version than nightly

## [0.2.2] - 2026-03-28

### Fixed
- Compiler driver now uses an isolated target directory (`target/mend/`) to prevent `E0514` errors when the main `target/` contains artifacts compiled by a different rustc version (e.g., CI caching stable and nightly builds together)

## [0.2.1] - 2026-03-28

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
