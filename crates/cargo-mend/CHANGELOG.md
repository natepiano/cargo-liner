# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed
- An exact, crate-rooted `pub(in crate::path)` declaration whose boundary is demanded only by a
  signature that crosses it — a type named in another item's return type, parameter, or public
  field, and never written by any caller — is no longer a `forbidden_pub_in_crate` error, and no
  facade is required for it. A caller that writes the item's own path across the boundary still is
  one: only the exposing item's path reaches a signature-exposed type, so a facade re-exporting it
  would have no user and would fail `unused_imports`.
- Exact, crate-rooted `pub(in crate::path)` declaration boundaries that match a parent facade are
  now accepted when `[visibility] pub_in_path` is `"permitted"` or `"required"` (the default).
  `required` also revises `suspicious_pub` advice for a bare `pub` behind such a facade. There is no
  warning-first grace mode: `[diagnostics]` supports enable/disable, not severity levels, and the
  required annotation change is a one-line edit.
- The global configuration is migrated once to the new default. A global `config.toml` written before
  this release carries no `config_version`, so its `[visibility] pub_in_path = "permitted"` is
  rewritten to `"required"` on the first run and then stamped with the version; every later run
  reads the stamp and leaves the value alone. To keep the previous behavior, set
  `pub_in_path = "permitted"` in the global configuration *after the first run of the upgraded
  binary* — that first run is what writes the stamp, and a config that carries the stamp is never
  revisited, so the value sticks from then on. Setting it before that first run does not work: the
  file still has no stamp, so the migration rewrites the value. A project `mend.toml` is never
  rewritten.

#### Visibility-policy upgrade contract

The following two-code matrix applies to `forbidden_pub_in_crate` and `forbidden_pub_crate`:

- **Previously green with `forbidden_pub_in_crate` enabled:** exact `pub(in crate::...)`
  declarations covered by the scanner were previously absent, while `pub(in crate)`, relative
  spellings, and restricted field annotations can now become new errors. Output is unchanged only
  when none of those newly detected forms exist.
- **`forbidden_pub_in_crate` disabled:** existing annotations may be present. Exact boundaries
  become permitted, while `suspicious_pub` or another canonical diagnostic can newly appear;
  disabling `forbidden_pub_in_crate` does not disable those codes.
- **`forbidden_pub_in_crate` disabled while `forbidden_pub_crate` remains enabled:** new
  `pub(in crate)` detection routes to the enabled code, so a project that believed it had opted out
  can still receive new errors.
- **Already failing:** an exact-boundary error can disappear under `permitted`; retained failures
  receive new headlines and help, and secondary findings can change count and order.
- **Machine-readable output:** `rustc_diagnostic` no longer emits the `note` child that duplicated
  the headline, and `render_diagnostic` no longer repeats it in `rendered`. Consumers of mend's
  cargo JSON therefore see one fewer child on every forbidden-visibility finding, including when
  the finding itself is otherwise unchanged.
- **The `--fix` summary now names the kind of edit it applied:** every fixer except `pub use` was
  previously counted into one "import fix(es)" total, so a run that removed a `pub` or rewrote an
  annotation announced an import fix it had not made. Removing a `pub`, narrowing one to
  `pub(crate)`, rewriting a `pub` to an exact `pub(in crate::...)` boundary, and rewriting a field
  annotation now each report under their own noun — "`pub` removal(s)", "visibility narrowing(s)",
  "annotation rewrite(s)", and "field visibility rewrite(s)" — with one clause per kind when a run
  applies more than one. Only the kinds that applied something are named — a kind with nothing to
  do is left out rather than reporting a zero — and a run that applied nothing at all still reports
  a single "nothing available" line. "import fix(es)" now counts only the fixers that move or
  rewrite a `use` item. Consumers matching the old text will see different output.
- **Struct and union fields now follow the `pub(crate)` location policy:** a `pub(crate)` field now
  reaches the same rejection rule as a `pub(crate)` item wherever that policy does not permit it.
  This does not make every `pub(crate)` field an error: permitted locations remain green, and
  `pub(super)` and `pub(self)` fields remain allowed. Struct and union fields were previously
  exempt, so this is the broadest behavior change for existing codebases.
- **Reject-once changes co-occurring codes:** a forbidden `pub(crate) mod` that previously emitted
  both `forbidden_pub_crate` and `review_pub_mod` now emits only the rejection. Finding counts and
  code sets can therefore change even when the source is otherwise unaffected.
- **Facade `use` spellings are quoted only when mend can establish them:**
  `forbidden_pub_crate` has both a spelling-specific and a neutral facade-help variant, and
  `internal_parent_pub_use_facade` uses neutral re-export wording when the written modifier is not
  recoverable. Consumers matching those strings will see different text.
- **Impl items reached through their self type now count as exposed:** items in an `impl` can inherit
  exposure from the implemented type, so items that were previously flagged can now be allowed.
- **Files unreachable from the crate root no longer influence findings:** source files that no `mod`
  declaration reaches are no longer scanned for usage or facades, so findings that depended on them
  disappear.
- **A child module importing a parent re-export no longer counts as an outside caller:** this can
  make `narrow_to_pub_crate` and stale-facade findings newly appear.
- **Project `[visibility] allow_prelude_pub_mod` now takes effect:** a project `mend.toml` value is
  no longer discarded in favor of the global configuration, so repositories that relied on it being
  ignored change behavior.
- **Findings schema 21 invalidates schema 19:** cached reports are rejected because
  forbidden-visibility persistence now stores typed signature, facade, caller, and acceptance
  constraints separately from diagnostics. After upgrading, run
  `rm -rf target/mend-findings` and force a recompile.
- **`suspicious_pub` help is now always computed:** the former static
  ``consider using: `pub(super)` `` text is gone, so items behind a facade receive the facade
  boundary instead.
- **`cargo mend --message-format=json` now emits exactly one `help` child per finding:** it no
  longer emits both static and dynamic suggestions for the same diagnostic.
- **Both renderers resolve custom help before static help:** terminal output and
  `--message-format=json` now agree for diagnostics carrying both values. Consumers pinned to the
  former JSON string will see a change.

## [0.17.6] - 2026-07-27

### Fixed
- `prefer_module_import` now leaves a function import alone when an attribute names that function, instead of rewriting the import and breaking the build. Attributes can name a function as a string — `#[serde(default = "default_monitor_scale")]` — and a string is not a path, so nothing rewrote it when the import changed. Rewriting `use super::window_state::default_monitor_scale;` to `use super::window_state;` left the attribute naming something no longer in scope (E0425), and `cargo mend --fix` rolled the entire run back. The same guard covers functions named as bare idents inside attribute tokens, such as `#[arg(default_value_t = default_scale())]`, which were invisible to the rewrite for the same reason.

## [0.17.5] - 2026-07-25

### Fixed
- Fix `cargo mend --fix` narrowing types to `pub(crate)` when they must remain public, such as types exposed by `pub use` or `Iterator::Item`

## [0.17.4] - 2026-07-23

### Fixed
- `prefer_module_import` no longer rewrites `use crate::parent::child;` to `use crate::parent;` when `child` is an inline `mod` block inside the parent module's file. Module detection only checked the filesystem for `child.rs`/`child/mod.rs`, so the inline module was misclassified as a function import, leaving multi-segment references like `child::CONST` unresolved (E0433) and forcing `cargo mend --fix` to roll back.
- `prefer_module_import` import dedup is now scope-aware: a `use` inside a nested `mod` (e.g. `mod tests`) no longer suppresses inserting the same module import at file top level, no longer causes deletion of an import that only exists in a different scope, and same-module imports in different scopes each rewrite in place. Previously an inline call rewrite could be left without its `use crate::module;` (E0433, rollback) because a `mod tests` import already claimed that module file-globally.

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
- `imports_at_top` no longer strips the `#[cfg]` gate when it moves a conditionally-compiled `use` to the file top. A `use` nested in a `#[cfg]`-gated block (the winit `#[cfg(target_os = "…")] let raw = { use winit::platform::…; … }` pattern) or carrying its own `#[cfg]` was moved unconditionally, so the other targets' imports became active on the current platform, failed to resolve (E0432), and forced `cargo mend --fix` to roll back. The moved import now carries the enclosing block's `#[cfg]` (or its own) with it, staying conditionally compiled; the gated block is left in place minus the `use`.

## [0.16.1] - 2026-07-01

### Fixed
- `prefer_module_import` no longer rewrites a function import to `use module;` when the file already imports a *different* module under that same bare name, which produced a duplicate-name error (E0252) and a misrouted call (E0425) and forced `cargo mend --fix` to roll back. When the target module name is already bound to another module, the function import is now left untouched instead. The prior fix only handled the case where the *same* module was already imported.

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
- `prefer_module_import --fix` no longer rolls back when an imported function is shadowed by a local binding of the same name. The call-site rewriter now tracks `let`, function/closure parameter, `for`, and `match`-arm bindings, and leaves bare references alone when they resolve to a local. Previously code like `let dot_radius = scaling::dot_radius(...);` followed by uses of `dot_radius` got rewritten to `scaling::dot_radius` everywhere, producing fn-item-where-`f32`-expected errors and triggering rollback
- `prefer_module_import --fix` no longer corrupts struct literal field shorthand. `Foo { name }` is now left as shorthand instead of being rewritten to the parse-error `Foo { module::name }`
- The on-disk findings cache is now reused across different cargo target-selection flags. Previously the cache was keyed on the full cargo CLI argument vector, so a `cargo mend --all-targets` run that immediately followed a plain `cargo mend` would silently drop the lib's findings: cargo's own fingerprinting correctly skipped recompiling the lib, the rustc-driver wrapper therefore did not re-emit, and the cache file from the prior run was rejected because its `scope_fingerprint` didn't match. The cache now matches purely on schema version, mend driver build id, and diagnostic config — `cargo mend`, `cargo mend --lib`, `cargo mend --all-targets`, etc. now produce consistent findings in any order with no extra recompilation and no extra target growth

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
