# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.18.4] - 2026-08-19

### Fixed
- `--fix` no longer fails and rolls everything back when a file already imports a type, but only
  for tests (`#[cfg(test)] use ...`). Mend thought the import was always there and skipped adding
  the one the normal build needed.
- Fixed a case where mend said "applied N import fix(es)" but changed nothing. If a type was used
  both at the top of a file and inside a nested module, mend could pick two different import paths
  for it, refuse to apply either, and still count them as applied. It now picks one path for both,
  and anything it can't fix is no longer counted as applied.

## [0.18.3] - 2026-08-17

### Added
- `internal_parent_pub_use_facade` findings can now be fixed automatically, with
  `cargo mend --fix-pub-use` or `--fix-all`. Mend deletes the parent `pub use`, points everything
  below it at the child module that actually declares the item, and narrows that declaration to
  `pub(super)`. Both ways of naming the item are rewritten: `use super::Widget;` becomes
  `use super::widget::Widget;`, and a `super::Widget` written inline in the code becomes
  `super::widget::Widget`. A facade that does not resolve to one declaring module — a glob
  re-export, or a chain that leaves the crate — is still reported, but not fixed.

### Fixed
- `suspicious_pub` no longer asks to narrow a type used by a trait impl whose trait and self type
  are both `pub`. rustc rejects that (E0446), so `--fix` made the edit, failed its re-check, and
  rolled the whole batch back.
- `--fix` no longer narrows a type below the visibility of the function signature, const, static,
  or struct field that names it. Mend measured those types at their call sites, but rustc's
  `private_interfaces` lint measures at the declaration, so a fix could leave a project with fresh
  warnings it did not have before.
- `internal_parent_pub_use_facade` no longer fires on a re-export that a proc macro in another
  crate depends on. Mend's facade check reads source text, so a path that exists only inside
  another crate's `quote!` was invisible to it and the re-export looked unused outside its own
  subtree. It is now cross-checked against the uses the compiler records after macros expand.

## [0.18.2] - 2026-08-14

### Fixed
- A type re-exported through a `pub(super) use` facade is no longer narrowed below the visibility of
  a function whose signature names it. The facade boundary is now joined with the caller boundary,
  so `--fix` stops producing `private_interfaces` errors and rolling the batch back.

## [0.18.1] - 2026-08-08

### Fixed
- `--fix-all` now preserves `#[cfg(...)]` attributes when rewriting function imports, preventing
  required non-test bindings from being deleted.
- Compiler fixes now validate their resulting build and roll back source edits when validation
  fails.

## [0.18.0] - 2026-08-07

### Changed
- Visibility tightening is now caller-aware across all selected targets. Mend computes the deepest
  module required by callers, signatures, facades, trait bounds, imports, and field use, then chooses
  private, `pub(super)`, exact `pub(in crate::path)`, `pub(crate)`, or `pub` as appropriate.
- `pub(crate)` is now accepted when crate-root access is genuinely required. The
  `forbidden_pub_crate` diagnostic is renamed to `overbroad_pub_crate` and is now a warning for
  declarations and fields that can be tighter, so it fails CI only under `--fail-on-warn`. Existing
  configs may still use the old key; global configs migrate it automatically.
- Exact crate-rooted `pub(in crate::path)` is accepted when it matches the boundary required by
  callers, signatures, or a parent facade and `pub_in_path` is `"permitted"` or `"required"`.
  `"required"` is now the default and also recommends exact boundaries for eligible bare `pub`
  declarations.
- `cargo mend --fix` applies related visibility changes together and repeats successful passes until
  no further fixes are exposed. This allows dependent declarations and named or positional fields
  to narrow in one invocation.
- Mend now uses `RUSTC_WORKSPACE_WRAPPER` for its compiler driver and preserves an existing
  `RUSTC_WRAPPER`, so compiler caches continue to serve dependencies and new Mend builds trigger the
  required workspace analysis.
- Global configs created before this release migrate `pub_in_path` from `"permitted"` to
  `"required"` once. Project `mend.toml` files are never rewritten. To retain the old global
  behavior, run the upgraded binary once and then set the global value back to `"permitted"`.

#### Visibility-policy upgrade contract

- Detection now includes restricted field annotations, relative `pub(in ...)` spellings, impl items
  exposed through their self type, and caller relationships from fields and trait bounds. Finding
  counts can change after upgrading.
- Disabling `forbidden_pub_in_crate` does not suppress other visibility diagnostics. For example,
  `pub(in crate)` can still reach `overbroad_pub_crate`, while an accepted exact boundary can still
  receive `suspicious_pub` advice.
- Struct and union fields now follow the same caller-aware `pub(crate)` policy as declarations.
- Unreachable source files no longer influence findings, and child imports of a parent re-export no
  longer count as outside callers.
- Project `allow_prelude_pub_mod` settings now correctly override the global value.
- JSON diagnostics no longer duplicate the headline as a `note`, and emit at most one `help` child.
  Fix summaries now name the kind of edit applied instead of grouping visibility edits as imports.
- Findings schema 27 replaces older cached reports. The per-build wrapper path forces fresh analysis
  when the installed Mend build changes.

### Performance
- Memoized signature-exposure analysis reduced a representative `hana_lagrange` run from about 160
  seconds to 32 seconds with unchanged findings.
- Indexed facade references and module contexts reduced a 29,000-path, 900-re-export check from
  1,019 seconds to 254 seconds with unchanged findings.
- Compiler and source-path maps now use `rustc-hash`, improving lookup speed and making iteration
  order stable across runs.

### Fixed
- Types mentioned only in attributes such as `#[require(...)]`, `#[reflect(...)]`, or documentation
  are no longer treated as part of a public signature.
- `--fix` no longer narrows declarations pinned to `pub` by a sibling-module `pub use`; those findings
  remain visible but are not offered as automatic fixes.
- A run that produced no compiler analysis now fails with recovery instructions instead of reporting
  a clean crate.
- New installations reliably trigger analysis, and concurrent Mend runs can safely share one target
  directory.

## [0.17.6] - 2026-07-27

### Fixed
- `prefer_module_import` now leaves function imports unchanged when attributes refer to the function
  by string or token, preventing unresolved attribute references after `--fix`.

## [0.17.5] - 2026-07-25

### Fixed
- Fix `cargo mend --fix` narrowing types to `pub(crate)` when they must remain public, such as types exposed by `pub use` or `Iterator::Item`

## [0.17.4] - 2026-07-23

### Fixed
- `prefer_module_import` no longer mistakes inline modules for imported functions.
- Import deduplication now respects inline-module scopes, preventing an import in `mod tests` from
  suppressing or deleting a required file-level import.

## [0.17.3] - 2026-07-20

### Fixed
- `inline_path_qualified_type --fix` now resolves partial module paths inherited from enclosing modules and preserves their `#[cfg(...)]` and `#[cfg_attr(...)]` attributes when adding imports inside nested modules. This prevents generated imports such as `use monitor_probe::Type;` from failing with E0432 and rolling back the fix.

## [0.17.2] - 2026-07-15

### Fixed
- Fix inline path type rewrites introducing imports that conflict with existing private bindings
- Source discovery no longer traverses Cargo's target directory, so transient generated `.rs` files under `target/` are no longer read into the source cache.

## [0.17.1] - 2026-07-10

### Fixed
- `cargo mend --fix` now resolves aliased source paths before merging and applying fixes, and visibility fixes must agree across every target that compiles a shared source file. A module included through paths such as `src/../fixtures.rs` and `examples/../fixtures.rs` is therefore edited once, while items used by any target keep their required visibility.

## [0.17.0] - 2026-07-10

### Changed
- Update to the rustc 1.97 `rustc_private` API. The compiler removed `Visibility::is_at_least`, restructured `ItemKind::Trait`, and stopped normalizing `instantiate_identity()` results, so `cargo-mend` 0.17 must be built with rustc 1.97 and will no longer compile against 1.96 (which 0.16.x requires).

## [0.16.2] - 2026-07-08

### Fixed
- `cargo mend` now preserves an existing `RUSTC_WRAPPER` (e.g. `kache`, `sccache`) instead of dropping it. When a wrapper is set, dependency compilations are passed through to it (chained ahead of `rustc`) and only the primary package is intercepted for analysis, so cached artifacts are reused instead of every dependency being recompiled under the bare compiler.
- `imports_at_top` now preserves an imported item's own or enclosing `#[cfg]` gate when moving it,
  preventing platform-specific imports from becoming unconditional.

## [0.16.1] - 2026-07-01

### Fixed
- `prefer_module_import` now skips rewrites when another imported module already uses the target
  module's bare name.

## [0.16.0] - 2026-06-20

### Added
- A crate-root `pub mod prelude;` is now exempt from `review_pub_mod` by default, so a prelude module no longer needs an `allow_pub_mod` override. Nested `pub mod prelude;` and other crate-root `pub mod` declarations are still reviewed. Set `allow_prelude_pub_mod = false` under `[visibility]` in the global config to review crate-root preludes too.
- The global config is now reconciled on every run: any missing diagnostic or visibility keys are added (comments and explicit values preserved), so existing configs gain new options automatically.

## [0.15.5] - 2026-06-10

### Fixed
- `forbidden_pub_crate` now suggests `pub` instead of `pub(super)` for a `pub(crate)` item that is exposed only structurally through a reachable public signature (e.g. a type returned by a `pub` method on a re-exported type); narrowing such an item to `pub(super)` would have introduced a `private_interfaces` error.
- The structural-exposure walk no longer overflows the stack when two public items mention each other in their signatures; visited items are now tracked so the walk terminates.
- `unused_pub` no longer flags a type that a trait impl's interface requires, including impls generated by derives (e.g. bevy's `AsBindGroup`); removing `pub` from such a type produced E0446 and made `cargo mend --fix` roll back.
- Fix `--fix` rolling back all changes when a call inside `mod tests` was rewritten with one `super::` too few (`prefer_module_import`). An inline test module sits one level deeper than the file, so reaching the file's parent requires `super::super::` — the rewrite now adds one `super` per nesting level.

## [0.15.4] - 2026-05-30

### Fixed
- Disable ANSI color in captured `cargo mend` output unless color is explicitly forced.
- `prefer_module_import` no longer rewrites a function import to `use module;` when the file already imports that module, which produced a duplicate import (rustc error E0252: the same name imported twice) and forced `cargo mend --fix` to roll back; the redundant function import is now deleted instead.

## [0.15.3] - 2026-05-26

### Fixed
- Fix the install failing to link on Linux with `cannot find -lLLVM-*`. `build.rs` now points the linker at the toolchain library directory that holds the bundled LLVM.

## [0.15.2] - 2026-05-26

### Fixed
- `unused_pub` no longer fires on a type reachable only through the return or parameter type of a `pub(crate)` function whose callers live in another module, which made `cargo mend --fix` remove a `pub` that the function signature still needs (E0446).

## [0.15.1] - 2026-05-25

### Fixed
- `unused_pub` no longer fires on a type reachable only through a type alias or another type's public field graph, which made `cargo mend --fix` remove a `pub` that a `pub(crate)` alias still needs (E0446).

## [0.15.0] - 2026-05-25

### Added
- Add `unused_pub` to remove `pub` from items used only inside their defining module subtree.

### Changed
- Show an interactive progress indicator while fix validation output is suppressed.

## [0.14.0] - 2026-05-17

### Added
- Add `imports_at_top` diagnostic and fix that lifts in-body `use` statements to the top of their enclosing file or inline module

## [0.13.2] - 2026-05-15

### Fixed
- `narrow_to_pub_crate` no longer fires on a `pub` item that is publicly reachable through another `pub` item's signature (e.g. a struct named in a re-exported enum variant's field type, or in a re-exported `pub const`'s type). Suggesting `pub(crate)` for such items would have introduced a `private_interfaces` error. The check now mirrors rustc's effective-visibility analysis at `Level::Reachable`.

## [0.13.1] - 2026-05-14

### Fixed
- Fix narrow-pub-crate firing on items reached through `pub use` when the parent file also contained an earlier `pub(crate) use` for a sibling

## [0.13.0] - 2026-05-14

### Changed
- `narrow_to_pub_crate` now also fires in nested modules: when an item is
  bare `pub` and its parent re-exports it as `pub(crate) use`, `cargo mend`
  suggests `pub(crate)` and auto-fixes it. The matching definition-site
  `pub(crate)` is now permitted at any depth.

## [0.12.3] - 2026-05-12

### Fixed
- `prefer_module_import --fix` no longer produces a wrong-parent `use super::module;` when rewriting a function import that lives inside an inline `mod` (e.g. `mod tests`). The detector now pushes inline module idents onto its tracked module path so shortened paths are computed against the actual scope, not the file scope.

## [0.12.2] - 2026-05-11

### Fixed
- `inline_path_qualified_type --fix` no longer inserts duplicate imports when an existing `pub use` already binds the target name.

## [0.12.1] - 2026-05-05

### Fixed
- `inline_path_qualified_type` no longer flags generic type parameters used inside fn bodies (closure parameter types, locals). The generic scope now spans the entire fn/method, not just the signature.

## [0.12.0] - 2026-05-05

### Changed
- `inline_path_qualified_type` also flags inline paths from other crates (e.g. `ratatui::Frame`, `std::collections::BTreeMap`) and the trait in `impl SomeTrait for Type`. Previously only `crate::` and `super::` paths.

### Fixed
- `inline_path_qualified_type` skips associated items on a generic type parameter (`S::Ok`, `B::Item`). Active generics are now tracked from `syn::Generics::params` on the enclosing fn/impl/trait/struct/enum/type.
- `inline_path_qualified_type` skips imports that would shadow a prelude name (`Result`, `Option`, `Vec`, `Box`, etc.).
- `inline_path_qualified_type` emits absolute import paths: `use std::fmt::Display;`, not `use fmt::Display;` resolved through an in-scope `use std::fmt;`.
- `--fix` no longer garbles files when an insertion and a replacement target the same offset; the wider replacement runs first.
- `--fix` no longer corrupts lines containing multi-byte UTF-8 characters; replacement windows now use byte offsets, not character columns.
- Shadow detection now considers multi-segment uses like `Result::ok`, not just bare `Result`.

## [0.11.0] - 2026-05-04

### Added
- Add `field_visibility_wider_than_type` lint that flags `pub` field annotations on fully-private types (auto-fixable with `cargo mend --fix`)

### Fixed
- Fix incorrect `pub(super)` suggestions for `pub` items reached only via method calls or struct literals

## [0.10.0] - 2026-05-04

### Changed
- Suspicious-pub suppression now uses HIR-level use sites instead of source-level path matching, catching macro-expanded and proc-macro-generated callers. Replaces the source-level macro walker from 0.9.2.

### Fixed
- Fix `--fix` emitting invalid `use super;` when `prefer_module_import` rewrote a call whose target module was the file's own parent — calls are now rewritten to `super::fn(...)` with no import, and parent-module function imports are dropped

## [0.9.2] - 2026-05-03

### Fixed
- Path extractor now walks macro token streams. Items called only from inside `assert_eq!`, `format!`, etc. were previously invisible to the facade scanner and got incorrectly flagged for narrowing/re-export removal.

## [0.9.1] - 2026-05-03

### Fixed
- Cross-compilation cfg(test) suppression now works on binary crates. The bin and bin-test compilations were sharing a cache file; one overwrote the other and intersection had nothing to compare. Cache filename now distinguishes them, with fix-fact dedup so a real fix isn't applied twice.
- `build.rs` now declares `rerun-if-changed=.git/HEAD`, so post-commit installs link a fresh `MEND_GIT_HASH`/`MEND_BUILD_ID`. Previously cargo cached the build-script env output and new commits shipped binaries that self-identified as the previous commit, causing mend's per-project findings cache to silently replay stale results.

## [0.9.0] - 2026-05-03

### Changed
- Target-selection flags (`--lib`, `--bin`, `--example`, `--test`, `--bench`, `--all-targets`) are now display filters. Mend always analyzes every target so the call graph is complete; the flags only narrow what gets printed. Cold runs on test-heavy crates are ~2× slower; the cache makes warm runs identical.
- `--fix-all` loops the fix passes until the tree stops changing, so cascading fixes converge in one invocation.
- `--fix-pub-use` runs `cargo fix` automatically when its rewrites leave unused imports. The old "consider running cargo fix" hint is gone.
- Summary lists each fixable category on its own line, with an aggregate `--fix-all` line when fixables span ≥2 categories. The combined `--fix --fix-pub-use` suggestion is gone.
- Mend errors render in their own block above the summary, separate from the "X fixable" warning count.

### Fixed
- `--fix-compiler` no longer deletes imports referenced only from `#[cfg(test)]` code. The chained `cargo fix` now runs against the test compilation, so cfg(test) callers are visible. Trade-off: imports unused in both lib and test mode aren't auto-removed.
- `--fix` and `--fix-pub-use` no longer suggest narrowing `pub` to `pub(super)` or removing a parent re-export when the only outside-subtree caller lives in `#[cfg(test)]` code. Mend now intersects findings across the lib and lib-test compilations for narrowing codes; single-compilation false positives are dropped. `narrow_to_pub_crate` is unaffected since `pub(crate)` always reaches cfg(test).
- Caveat: `#[cfg(feature = "x")]` reachability is not handled — pass `--features <set>` explicitly for non-default features.

## [0.8.2] - 2026-05-02

### Fixed
- `--fix-pub-use` now applies the fix when the re-export in the parent module is already narrowed (e.g. `pub(super)`) instead of reporting it as fixable and then skipping it

## [0.8.1] - 2026-04-26

### Fixed
- `prefer_module_import --fix` now preserves references shadowed by local bindings in `let`,
  function, closure, `for`, and match-arm scopes.
- `prefer_module_import --fix` no longer corrupts struct literal field shorthand. `Foo { name }` is now left as shorthand instead of being rewritten to the parse-error `Foo { module::name }`
- Findings caches are now shared across target display filters, so `cargo mend`, `--lib`, and
  `--all-targets` report consistently without redundant compilation.

## [0.8.0] - 2026-04-22

### Added
- `prefer_module_import` now flags inline fully-qualified function calls (e.g. `crate::layout::set_root_grow_height(tree)`) with no matching `use`. `--fix` inserts `use crate::layout;` and rewrites the call site to `layout::set_root_grow_height(tree)`, deduplicating against existing module imports and function imports that pass 1 will rewrite

### Changed
- `inline_path_qualified_type --fix` now also shortens fully-qualified paths that appear as struct construction (`crate::foo::Bar { .. }`) or destructuring (`let crate::foo::Bar { x } = ..`, `Some(crate::foo::Bar(x))`). Previously these spots were left alone, so a single file could end up with a mix of shortened and still-qualified references to the same type

### Fixed
- `cargo mend --fix` no longer breaks the build when a file mixes an enum variant and a same-named struct (e.g. `RustProject::Package` alongside a `Package` struct). The enum variant is now imported via its parent type, so existing bare uses of the struct keep resolving
- `cargo mend --fix` no longer turns a struct associated-function call like `Foo::bar()` into a bogus `use crate::...::Foo;` import
- `narrow_to_pub_crate` no longer suggests narrowing `pub` items to `pub(crate)` in integration test root files under `tests/`, `examples/`, or `benches/`, which are compiled both as their own targets and as modules of sibling targets

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
