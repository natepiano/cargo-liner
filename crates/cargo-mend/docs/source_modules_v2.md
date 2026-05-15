# cargo-mend src/ restructure plan

Target: replace the 18-singleton flat root with a small set of directory submodules grouped by responsibility (configuration surface, workspace selection, reporting, fix passes), dissolve `constants.rs` (a three-axis junk drawer) into the directories whose code consumes each subset, rename the existing `module_paths.rs` to `rust_syntax.rs` so its name matches its absorbed contents, fold `outcome.rs` into `reporting/` next to its peer `diagnostics.rs`, and split the four over-large analysis files. Final root drops to 3 singletons; every layer is within the 6-singleton budget (with one defended singleton in `compiler/`).

## Phase overview

| Phase | What | Risk | Rough size |
|-------|------|------|------------|
| 1 | Absolutization sweep — rewrite every `use super::…` to `use crate::…` across 35 files. Zero file moves, zero behavior change. | Low | ~80 import rewrites; one commit |
| 2 | Restructure — group root singletons into directories, dissolve `constants.rs` into its consumers, move `outcome.rs` into `reporting/` and fold `fix_support.rs` into `reporting/diagnostics.rs`, rename `module_paths.rs` → `rust_syntax.rs` and `field_visibility_fix.rs` → `fixes/field_visibility.rs`. | Low | ~22 file moves + `mod.rs` shims; one commit |
| 3 | Split `fixes/inline_path_qualified_type.rs` (912 prod) into `mod.rs + scope + visitor + process`. | Medium | 3 new files; one commit |
| 4 | Split `compiler/facade.rs` (734 prod) into `mod.rs + boundary + exports + reference`. | Medium | 3 new files; one commit |
| 5 | Split `compiler/visibility/scan.rs` (764 prod) into `mod.rs + analyze + classify + record`. | Medium | 3 new files; one commit |
| 6 | Split `compiler/exposure.rs` (616 prod) into `mod.rs + detect + visitor`. | Low | 2 new files; one commit |

Phases are listed top-to-bottom but only 1→2 is strictly sequenced. After Phase 2 lands, phases 3–6 are independent and may land in any order.

---

## Phase 1 — Absolutization sweep

Rewrite every `use super::…` to `use crate::…` in every file that is about to move (and in every file whose neighbours are about to move). Today 35 files use `use super::` to reach root-level siblings (e.g. `src/diagnostics.rs` has `use super::config::DiagnosticCode;`, `src/cargo_json.rs` has `use super::diagnostics::BuildOutcome;`). After a file moves into a subdirectory in Phase 2, `super::` no longer points to the crate root, so those imports break silently or load the wrong item.

### Files in scope

`diagnostics.rs`, `render.rs`, `cargo_json.rs`, `fix_support.rs`, `outcome.rs`, `config.rs`, `cli.rs`, `run_mode.rs`, `constants.rs`, `selection.rs`, `display_filter.rs`, `imports.rs`, `inline_path_qualified_type.rs`, `field_visibility_fix.rs`, `narrow_pub_crate.rs`, `compiler/settings.rs`, `compiler/source_cache.rs`, `compiler/persistence.rs`, `compiler/build.rs`, `compiler/driver.rs`, and every file under `prefer_module_import/` and `pub_use_fixes/`.

### Sequencing

1. For every file in scope, `rg "use super::" <file>` to enumerate every relative import.
2. Replace each `use super::<x>::…` with `use crate::<x>::…`.
3. For inline `#[cfg(test)]` blocks, `use super::<item>` referring to the file's own top-level items stays as-is.
4. `cargo build && cargo nextest run`. Nothing has moved; behavior is unchanged. **Commit.**

---

## Phase 2 — Restructure

### Proposed layout

```
src/
├─ main.rs                          # entry point
├─ runner.rs                        # top-level orchestration; absorbs FIX_ALL_MAX_PASSES
├─ rust_syntax.rs                   # (renamed from module_paths.rs) Rust-language
│                                   #   syntax helpers and string tokens: module-path
│                                   #   utilities (file_module_path etc.), path
│                                   #   keywords (PATH_KEYWORD_*), separators
│                                   #   (MODULE_PATH_SEPARATOR, MODULE_GLOB_SUFFIX),
│                                   #   visibility tokens (PUB_*_VISIBILITY*)
├─ config/                          # configuration surface (argv + file)
│  ├─ mod.rs                        # (was config.rs body) LoadedConfig, VisibilityConfig,
│  │                                #   DiagnosticCode, DiagnosticStatus, DiagnosticsConfig,
│  │                                #   load_config(); absorbs APP_NAME,
│  │                                #   DEFAULT_GLOBAL_CONFIG_TOML, GLOBAL_CONFIG_FILE
│  ├─ cli.rs                        # Cli, BuildInfoMode, argv parsing
│  └─ run_mode.rs                   # OperationMode, FixSelection, OperationIntent
├─ selection/                       # which packages/targets/findings are in scope
│  ├─ mod.rs
│  ├─ selection.rs                  # Selection, CargoCheckPlan, package/target metadata;
│  │                                #   absorbs cargo target-kind tokens
│  │                                #   (CARGO_TARGET_KIND_BENCH/_BIN/_EXAMPLE/_LIB/_MAIN/_TEST)
│  └─ display_filter.rs             # DisplayFilter (CLI scope → finding subset)
├─ reporting/                       # findings + result domain types and every output format
│  ├─ mod.rs
│  ├─ diagnostics.rs                # Finding, Report, Severity, DiagnosticSpec,
│  │                                #   FixSupport (absorbs fix_support.rs);
│  │                                #   absorbs DIAGNOSTICS_HELP_NAME_COLUMN_WIDTH
│  ├─ outcome.rs                    # (moved from root) ExecutionOutcome, MendFailure,
│  │                                #   ExecutionNotice; absorbs EXIT_CODE_ERROR/_WARNING
│  ├─ render.rs                     # human-readable + ANSI rendering
│  ├─ cargo_json.rs                 # cargo-message JSON output
│  └─ colors.rs                     # ANSI_BOLD*, ANSI_DIM, CARGO_TERM_COLOR_*,
│                                   #   CLICOLOR_*, TERM_DUMB_VALUE, TERM_ENV
├─ fixes/                           # every scan-and-fix pass
│  ├─ mod.rs
│  ├─ imports.rs                    # ImportScan, UseFix, ValidatedFixSet, ImportGroup
│  ├─ inline_path_qualified_type.rs # InlinePathScan (split in Phase 3)
│  ├─ field_visibility.rs           # (was field_visibility_fix.rs) FieldVisibilityFixScan
│  ├─ narrow_pub_crate.rs           # NarrowPubCrateScan
│  ├─ prefer_module_import/         # (existing — moved as a whole)
│  └─ pub_use_fixes/                # (existing — moved as a whole)
└─ compiler/                        # rustc integration
   ├─ mod.rs
   ├─ build.rs                      # cargo check/fix invocation; absorbs cargo-CLI
   │                                #   invocation strings (CARGO_BIN, CARGO_FLAG_*,
   │                                #   CARGO_SUBCOMMAND_*, CARGO_MANIFEST_FILE)
   ├─ driver.rs                     # rustc driver entry, AnalysisCallbacks
   ├─ settings.rs                   # DriverSettings, config fingerprint;
   │                                #   absorbs driver-IPC env-var constants
   │                                #   (CONFIG_*_ENV, FINDINGS_DIR_ENV,
   │                                #   PACKAGE_ROOT_ENV, SCOPE_FINGERPRINT_ENV,
   │                                #   DRIVER_ENV*, RUSTC_WORKSPACE_WRAPPER_ENV)
   ├─ source_cache.rs               # SourceCache, path extraction, use-tree flattening;
   │                                #   absorbs Rust-source-tree vocabulary
   │                                #   (RUST_LIB_FILE, RUST_MAIN_FILE, RUST_MODULE_FILE,
   │                                #   RUST_SOURCE_FILE_EXTENSION, RUST_SOURCE_FILE_SUFFIX,
   │                                #   SOURCE_DIR_BENCHES, SOURCE_DIR_EXAMPLES,
   │                                #   SOURCE_DIR_SRC, SOURCE_DIR_TESTS)
   ├─ persistence.rs                # disk-format I/O for analysis output (defended
   │                                #   singleton; absorbs FINDINGS_SCHEMA_VERSION)
   ├─ exposure.rs                   # (split in Phase 6)
   ├─ facade.rs                     # (split in Phase 4)
   └─ visibility/                   # existing subtree — untouched by Phase 2
      ├─ mod.rs
      ├─ field_visibility.rs        # detection (sibling of fixes/field_visibility.rs)
      ├─ policy.rs
      ├─ scan.rs                    # (split in Phase 5)
      ├─ source.rs
      └─ use_sites.rs
```

### Moves, with rationale

**`config/`** — `cli.rs`, the loader body now in `config/mod.rs`, and `run_mode.rs` together answer the question *"what was the user asking for?"*: argv → parsed Cli → loaded config file → operational mode. `runner.rs` consumes the merged result. The loader body lives in `config/mod.rs` directly (no `loader.rs` rename) — there is only one loader, it can be the `mod.rs`, and the resulting path `crate::config::DiagnosticCode` reads cleaner than `crate::config::loader::DiagnosticCode`. The app-level loader constants (`APP_NAME`, `DEFAULT_GLOBAL_CONFIG_TOML`, `GLOBAL_CONFIG_FILE`) move into `config/mod.rs` alongside the loader code that uses them — they have no consumer elsewhere.

**`selection/`** — `selection.rs` resolves cargo-metadata into Selection/CargoCheckPlan; `display_filter.rs` narrows the report's findings to the same Selection's display subset. Both depend on cargo workspace structure; both are consumed once by `runner.rs`.

**`reporting/`** — `diagnostics.rs` defines the finding domain types and per-code metadata; `render.rs` formats human output; `cargo_json.rs` formats the JSON variant; `colors.rs` holds the ANSI/terminal-color vocabulary; `outcome.rs` defines the execution-result domain types (`ExecutionOutcome`, `MendFailure`, `ExecutionNotice`) which are built by `runner.rs` and consumed by every renderer in this directory. `outcome.rs` and `diagnostics.rs` are peers along the same axis — both are *result domain types rendered by the same code*. `fix_support.rs` (51 lines) folds into `diagnostics.rs` because `FixSupport` is per-DiagnosticCode metadata. `EXIT_CODE_ERROR`/`EXIT_CODE_WARNING` fold into `outcome.rs` next to the failure types that produce them. `DIAGNOSTICS_HELP_NAME_COLUMN_WIDTH` folds into `render.rs` next to its sole caller. Note: `CLICOLOR_ENV`, `CLICOLOR_FORCE_ENV`, and `CARGO_TERM_COLOR_ENV` are also read by `main.rs` to propagate color-mode hints to the cargo subprocess — `main.rs` reaches into `crate::reporting::colors::*` for those, which is acceptable: they're constants, not behavior, and the upward import does not couple `main.rs` to render state.

**`fixes/`** — every fix pass produces a `*Scan { findings, fixes }` value consumed in parallel by `runner.rs`: `imports.rs`, `inline_path_qualified_type.rs`, `field_visibility_fix.rs` (renamed to `field_visibility.rs` — the `_fix` suffix becomes redundant under `fixes/`), `narrow_pub_crate.rs`, plus the two existing directory submodules `prefer_module_import/` and `pub_use_fixes/`. Grouping them makes the pattern visible: each is a scan-and-fix instance feeding the same merge step.

**`compiler/` — no `state/` grouping.** A previous draft proposed grouping `settings.rs` (driver config + fingerprint), `source_cache.rs` (parsed-source caching), and `persistence.rs` (serde-to-disk for analysis output) under `compiler/state/`. They share the label "state that survives invocations" but they're three different doings: configuration, caching, serialization. Group-by-label is a smell — keep them as top-level files in `compiler/` and **defend `persistence.rs` explicitly** as the sole singleton above the 6-file budget (see the gate below). `settings.rs` absorbs the driver-IPC env-var constants (`CONFIG_*_ENV`, `FINDINGS_DIR_ENV`, `PACKAGE_ROOT_ENV`, `SCOPE_FINGERPRINT_ENV`, `DRIVER_ENV`, `RUSTC_WORKSPACE_WRAPPER_ENV`) — those were duplicated in `constants.rs` despite having `settings.rs` as their sole consumer. `persistence.rs` absorbs `FINDINGS_SCHEMA_VERSION` for the same reason.

**No `cargo_vocab.rs`.** A previous draft proposed a single `compiler/cargo_vocab.rs` to hold cargo-CLI strings, cargo target-kind tokens, and Rust-source-tree filenames. That bundles three different doings under one label — cargo invocation, target taxonomy, source-tree navigation — and is the same vibe-grouping that earned `compiler/state/` its dissolution. Instead, each subset moves into its actual consumer: cargo-CLI invocation strings into `compiler/build.rs` (the sole cargo subprocess invoker); Rust-source-tree filenames into `compiler/source_cache.rs` (the source-tree walker); cargo target-kind tokens into `selection/selection.rs` (target metadata domain, consumed by `cargo_json.rs`, `source_cache.rs`, and `selection.rs` itself — cross-directory reads of a `pub(crate) const` are fine). Dissolving the proposed file also drops `compiler/` back to 6 singletons inside budget + 1 defended.

**`rust_syntax.rs` (renamed from `module_paths.rs`) absorbs Rust-language string tokens.** The file currently holds module-path helpers (`file_module_path`, `module_name_for_child_boundary_file`, `module_name_for_boundary_file`); after rename it also holds `PATH_KEYWORD_CRATE/SELF/SUPER`, `MODULE_PATH_SEPARATOR`, `MODULE_GLOB_SUFFIX`, and the visibility-syntax tokens `PUB_CRATE_VISIBILITY` / `PUB_IN_CRATE_VISIBILITY_PREFIX` / `PUB_VISIBILITY_PREFIX` / `PUB_VISIBILITY_TOKEN`. The new name covers both the existing helpers (manipulating Rust path syntax) and the absorbed visibility tokens (Rust visibility syntax); keeping the name `module_paths.rs` while broadening the scope would be the same junk-drawer growth the constants dissolution was supposed to prevent.

### What stays where

- **`main.rs`** — binary entry point, referenced from `[[bin]]`. No peer at root.
- **`runner.rs`** — single orchestrator (`MendRunner::run`). Imported by `main.rs`; imports every directory in the layout. Promoting it into a subdirectory would only add an indirection. Absorbs `FIX_ALL_MAX_PASSES` (sole consumer).
- **`rust_syntax.rs`** (renamed from `module_paths.rs`) — Rust-language path/visibility syntax helpers used from at least 5 unrelated callers across `fixes/`, `compiler/visibility/`, and `compiler/source_cache.rs`. No grouping partner — `source_cache.rs` has overlapping helpers but they are coupled to its data structures; merging would couple unrelated callers.
- **`compiler/visibility/`** — five tightly cohesive files (policy, scan, source, use_sites, field_visibility). Pass 1 cohesion review found this subtree well-organized; only `scan.rs` is over-large (handled in Phase 5).

### Singleton-budget gate

| Layer | Singletons after Phase 2 | Budget |
|-------|--------------------------|--------|
| `src/` | 3 (main, runner, rust_syntax) | ≤6 ✓ |
| `src/config/` | 2 (cli, run_mode) — loader body lives in `mod.rs` | ≤6 ✓ |
| `src/selection/` | 2 | ≤6 ✓ |
| `src/reporting/` | 5 (diagnostics, outcome, render, cargo_json, colors) | ≤6 ✓ |
| `src/fixes/` | 4 (imports, inline_path_qualified_type, field_visibility, narrow_pub_crate) | ≤6 ✓ |
| `src/compiler/` | 6 (build, driver, settings, source_cache, exposure, facade) + **1 defended** (persistence) | ≤6 ✓ |
| `src/compiler/visibility/` | 5 | ≤6 ✓ |

**Defended singleton in `compiler/`:** `persistence.rs` (505 prod lines) is the sole file that performs serde-to-disk for analysis output. No file in `compiler/` shares the *serialization* axis with it — `source_cache.rs` reads parsed Rust source from memory; `build.rs` invokes cargo as a subprocess; `settings.rs` builds an in-memory config struct from env vars; nothing else writes findings to disk. `persistence.rs` is referenced from `mod.rs` (`compiler::driver_main` writes findings via it) and has no responsibility-peer along any axis. With persistence held out, the remaining 6 singletons are within budget.

Gate passes at every layer.

### Module re-exports

> **Re-export blocks below are illustrative — the actual sequencing step requires every `/*, … */` placeholder to be replaced with the enumerated exports before the commit is made.** For every soon-to-move file, run `rg "pub\(crate\)" <file>` and copy every exported item into the corresponding `pub(crate) use` line. **No glob (`use foo::*;`) survives in any new `mod.rs`** — globs silently re-export every future item, including ones that should stay file-private. Enumerate first; if the list is uncomfortably long, that is itself a signal to look at whether the consuming code should narrow what it imports.

`src/config/mod.rs` — note: this is also the loader body, not a re-export-only shim
```rust
mod cli;
mod run_mode;

// Loader body (was src/config.rs body). All types and the load_config()
// function are declared here directly.
pub(crate) enum DiagnosticCode { /* … */ }
pub(crate) enum DiagnosticStatus { /* … */ }
pub(crate) struct DiagnosticsConfig { /* … */ }
pub(crate) struct LoadedConfig { /* … */ }
pub(crate) struct VisibilityConfig { /* … */ }
pub(crate) fn load_config(/* … */) -> Result<LoadedConfig> { /* … */ }
// Absorbs APP_NAME, DEFAULT_GLOBAL_CONFIG_TOML, GLOBAL_CONFIG_FILE.

pub(crate) use cli::{BuildInfoMode, Cli /*, … */};
pub(crate) use run_mode::{FixKind, FixSelection, OperationIntent, OperationMode};
```

`src/selection/mod.rs`
```rust
mod display_filter;
mod selection;

pub(crate) use display_filter::DisplayFilter;
pub(crate) use selection::{CargoCheckPlan, PackageMetadata, Selection, TargetMetadata};
```

`src/reporting/mod.rs`
```rust
mod cargo_json;
mod colors;
mod diagnostics;
mod outcome;
mod render;

pub(crate) use cargo_json::render_report;  // enumerate every pub(crate) item
pub(crate) use colors::{
    ANSI_BOLD, ANSI_BOLD_BLUE, ANSI_BOLD_GREEN, ANSI_BOLD_RED, ANSI_BOLD_YELLOW,
    ANSI_DIM, CARGO_TERM_COLOR_ALWAYS, CARGO_TERM_COLOR_ENV, CARGO_TERM_COLOR_NEVER,
    CLICOLOR_DISABLED_VALUE, CLICOLOR_ENV, CLICOLOR_FORCE_ENV, TERM_DUMB_VALUE, TERM_ENV
};
pub(crate) use diagnostics::{
    BuildOutcome, DiagnosticSpec, Finding, FixSupport, FixSummaryBucket,
    Report, ReportSummary, Severity /* enumerate the rest before commit */
};
pub(crate) use outcome::{
    CompilerFailureCause, ExecutionNotice, ExecutionOutcome, FixNotice,
    FixValidationFailure, MendFailure, NoticeKind, PubUseNotice, RollbackStatus,
    EXIT_CODE_ERROR, EXIT_CODE_WARNING /* enumerate the rest before commit */
};
pub(crate) use render::{
    ColorMode, CompilerStats, OutputFormat, render_human_report, render_timing
    /* enumerate the rest before commit */
};
```

`src/fixes/mod.rs`
```rust
pub(crate) mod field_visibility;
pub(crate) mod imports;
pub(crate) mod inline_path_qualified_type;
pub(crate) mod narrow_pub_crate;
pub(crate) mod prefer_module_import;
pub(crate) mod pub_use_fixes;

pub(crate) use field_visibility::FieldVisibilityFixScan;
pub(crate) use imports::{
    ImportGroup, ImportScan, UseFix, ValidatedFixSet,
    apply_fixes, restore_files, scan_selection as scan_imports, snapshot_files /*, … */
};
pub(crate) use inline_path_qualified_type::InlinePathScan;
pub(crate) use narrow_pub_crate::NarrowPubCrateScan;
pub(crate) use prefer_module_import::PreferModuleImportScan;
pub(crate) use pub_use_fixes::PubUseFixScan;
```

The `prefer_module_import/` and `pub_use_fixes/` subdirectories currently import `UseFix`, `ImportGroup`, and `ValidatedFixSet` via `use crate::imports::…` — those paths must be rewritten to `use crate::fixes::imports::…` (or, equivalently, `use super::imports::…` since their parent is now `fixes/`). Phase 1's absolutization sweep plus the dedicated rewrite step in Phase 2 sequencing cover this.

`src/compiler/mod.rs` keeps its current public surface (`BuildOutputMode`, `SelectionResult`, `run_cargo_fix`, `run_selection`, `driver_main`). No new submodules — `settings.rs`, `source_cache.rs`, and `persistence.rs` remain direct children, with `persistence.rs` defended as the over-budget singleton. No `cargo_vocab.rs` (its proposed contents are distributed into the actual consumers). No changes for callers outside `compiler/`.

### Sequencing

Phase 1 (absolutization) must be in already. Run `cargo build && cargo nextest run` between every step.

1. **Rename `src/module_paths.rs` → `src/rust_syntax.rs`.** Update `mod module_paths;` → `mod rust_syntax;` in `src/main.rs` and every `use crate::module_paths::…` → `use crate::rust_syntax::…`. Build.

2. **Dissolve `constants.rs`.** No new directories are created yet — the `reporting/` directory does not exist until step 4. The ANSI/color/TERM constants therefore land *temporarily* at the bottom of `src/render.rs`; they will travel with `render.rs` into `src/reporting/` in step 4 and be split out into `src/reporting/colors.rs` at the same time. Every other constant goes directly into its final destination file:
   - ANSI/color/TERM constants (`ANSI_BOLD*`, `ANSI_DIM`, `CARGO_TERM_COLOR_*`, `CLICOLOR_*`, `TERM_DUMB_VALUE`, `TERM_ENV`) → temporary home at the bottom of `src/render.rs`; split into `src/reporting/colors.rs` in step 4.
   - Cargo-CLI invocation strings (`CARGO_BIN`, `CARGO_FLAG_*`, `CARGO_SUBCOMMAND_*`, `CARGO_MANIFEST_FILE`) → `src/compiler/build.rs`.
   - Rust-source-tree vocabulary (`RUST_LIB_FILE`, `RUST_MAIN_FILE`, `RUST_MODULE_FILE`, `RUST_SOURCE_FILE_EXTENSION`, `RUST_SOURCE_FILE_SUFFIX`, `SOURCE_DIR_BENCHES`, `SOURCE_DIR_EXAMPLES`, `SOURCE_DIR_SRC`, `SOURCE_DIR_TESTS`) → `src/compiler/source_cache.rs`.
   - Cargo target-kind tokens (`CARGO_TARGET_KIND_BENCH/_BIN/_EXAMPLE/_LIB/_MAIN/_TEST`) → `src/selection.rs`.
   - Driver-IPC env-var constants (`CONFIG_*_ENV`, `FINDINGS_DIR_ENV`, `PACKAGE_ROOT_ENV`, `SCOPE_FINGERPRINT_ENV`, `DRIVER_ENV`, `DRIVER_ENV_ENABLED`, `RUSTC_WORKSPACE_WRAPPER_ENV`) → `src/compiler/settings.rs`.
   - `FINDINGS_SCHEMA_VERSION` → `src/compiler/persistence.rs`.
   - App-level loader constants (`APP_NAME`, `DEFAULT_GLOBAL_CONFIG_TOML`, `GLOBAL_CONFIG_FILE`) → `src/config.rs` (the file moves into `src/config/mod.rs` in step 6).
   - `DIAGNOSTICS_HELP_NAME_COLUMN_WIDTH` → `src/render.rs` (the file moves into `reporting/` in step 4).
   - `EXIT_CODE_ERROR`, `EXIT_CODE_WARNING` → `src/outcome.rs` (the file moves into `reporting/` in step 4).
   - `FIX_ALL_MAX_PASSES` → `src/runner.rs`.
   - Rust syntax tokens (`PATH_KEYWORD_*`, `MODULE_PATH_SEPARATOR`, `MODULE_GLOB_SUFFIX`, `PUB_*_VISIBILITY*`) → `src/rust_syntax.rs` (renamed in step 1).
   Rewrite every `use crate::constants::X` to its new path. Delete `src/constants.rs`. Build.

3. **Verify `compiler/`.** After step 2, every `compiler/*.rs` file that lost a `use crate::constants::…` line should still compile against its absorbed constants. No files move; this is a checkpoint. `cargo build && cargo nextest run`.

4. **Create the four new directories and move `reporting/` files.** Create `src/config/mod.rs`, `src/selection/mod.rs`, `src/reporting/mod.rs`, `src/fixes/mod.rs` (each starts empty). Declare the new modules in `src/main.rs`. Two of the four declarations collide with existing files: `mod config;` collides with `src/config.rs`, and `mod selection;` collides with `src/selection.rs`. The other two declarations (`mod reporting;` and `mod fixes;`) introduce brand-new module names — no preexisting `src/reporting.rs` or `src/fixes.rs` exists — so they don't collide on their own, but until the leaf files move into those directories, callers that already say `use crate::reporting::…` or `use crate::fixes::…` won't resolve. The build is **expected to fail** between this step and the end of step 7. Resolve the collisions and the unresolved imports by completing steps 4–7 in the same working session; do not build in between. Specifically, in this step:
   - Move `diagnostics.rs`, `render.rs`, `cargo_json.rs`, `outcome.rs` into `src/reporting/`.
   - Split the temporary ANSI/color/TERM constants out of `src/render.rs` into a new `src/reporting/colors.rs`.
   - Fold the body of `fix_support.rs` (`FixSupport`, `FixSummaryBucket`) into `src/reporting/diagnostics.rs`; delete `src/fix_support.rs`.
   - Rewrite every `use crate::diagnostics::…`, `use crate::render::…`, `use crate::cargo_json::…`, `use crate::outcome::…`, `use crate::fix_support::…` to `use crate::reporting::…`.

5. **Move `selection/` files.** `selection.rs` and `display_filter.rs` → `src/selection/`. Rewrite `use crate::display_filter::…` → `use crate::selection::…`.

6. **Merge `config/`.** Move `cli.rs` and `run_mode.rs` → `src/config/`. Move the body of `src/config.rs` directly into `src/config/mod.rs` (alongside the `mod cli; mod run_mode;` declarations and the items absorbed in step 2). Delete `src/config.rs`. Rewrite `use crate::cli::…`, `use crate::run_mode::…` → `use crate::config::…`.

7. **Move `fixes/` files.** `imports.rs`, `inline_path_qualified_type.rs`, `narrow_pub_crate.rs` → `src/fixes/`; `field_visibility_fix.rs` → `src/fixes/field_visibility.rs`; entire `prefer_module_import/` and `pub_use_fixes/` directories → `src/fixes/`. Run `rg "use crate::imports::" src/fixes/prefer_module_import/ src/fixes/pub_use_fixes/` — every match must be rewritten to `use crate::fixes::imports::…`. Rewrite `runner.rs`'s imports en masse to `use crate::fixes::…`. **Build.** This is the first successful build after step 4; all directory moves are complete.

8. **Enumerate every `mod.rs` re-export.** For each new directory (`config/`, `selection/`, `reporting/`, `fixes/`), replace any glob or placeholder in the `pub(crate) use` blocks with the explicit list of items the rest of the crate imports. Run `rg "pub\(crate\)\s+(use|fn|struct|enum|const|type|mod)" src/<dir>/` per directory to enumerate the candidate list, then narrow the re-exports to only what is used externally.

9. **Final cleanup.** Verify `src/main.rs` declares exactly: `mod compiler; mod config; mod fixes; mod reporting; mod runner; mod rust_syntax; mod selection;` (binary entry `main()` stays in `main.rs`). Run `cargo build && cargo nextest run --lib && cargo nextest run --test diagnostics && cargo +nightly fmt`. **Commit.**

Step ordering: the rename (step 1) and constants dissolution (step 2) come first because they have the smallest blast radius — they edit existing files without moving them, and every later step depends on `use crate::constants::…` already being rewritten. After that, the directory creation and the leaf directory moves (reporting, selection, config) all happen in one build-failure window (steps 4–6), and `fixes/` plus the `runner.rs` rewrite close it in step 7.

---

Phases 3–6 are file-local splits. Each preserves its file's existing public surface via re-exports in the new `mod.rs`, so no caller outside the affected directory needs to change. They are independent of one another and may land in any order after Phase 2. Each phase has a hard pre-commit step: **every `pub(super) use <submodule>::*;` and `pub(crate) use <submodule>::*;` must be replaced with an enumerated import list before the commit lands** — globs silently re-export every future item added to a submodule, defeating the encapsulation gained by the split.

Borderline files NOT in scope for phases 3–6: `imports.rs` (521 prod — single cohesive domain per the coupling review), `runner.rs` (496 prod — under threshold), `compiler/persistence.rs` (**505 prod, 0 test** — at threshold but single-domain serde-to-disk; defended as the over-budget singleton in `compiler/`), `compiler/visibility/policy.rs` (325 prod / 177 test) — all single-domain.

---

## Phase 3 — Split `fixes/inline_path_qualified_type.rs` (912 prod lines)

Hits three criteria from `when-to-split-a-module.md`: line count, multiple type clusters (`InlinePathOccurrence`, `ScopeInfo`/`ScopeSpan`, `InlinePathVisitor`, `ScopeCollectionContext`, `OccurrenceContext`), and mixed domains (scope tracking, AST visitation, occurrence/collision processing). Only public caller is `runner.rs`, which calls `scan_selection()` and consumes `InlinePathScan`.

### Target layout

```
fixes/inline_path_qualified_type/
├─ mod.rs        # InlinePathScan, scan_selection(), scan_file() orchestrator, byte-offset utils
├─ scope.rs      # ScopeInfo, ScopeSpan, ScopeCollectionContext, collect_scopes(),
│                # find_innermost_scope(), indentation_at(), canonicalize_inserted_use_path()
├─ visitor.rs    # InlinePathVisitor, InlinePathOccurrence, check_path(),
│                # record_bare_name_footprint(), visit_* impls, flatten_use_path(),
│                # is_pascal_case()
└─ process.rs    # process_occurrence(), absolutize_import_path(),
                 # find_collision_names(), shadows_prelude()
```

### What goes where

| File | Lines (old) | Items |
|------|-------------|-------|
| `scope.rs` | 108–132, 389–516 | `ScopeInfo`, `ScopeSpan`, `ScopeCollectionContext`; `collect_scopes`, `find_innermost_scope`, `indentation_at`, `canonicalize_inserted_use_path` |
| `visitor.rs` | 90–106, 556–884 | `InlinePathOccurrence`, `InlinePathVisitor`; `check_path`, `record_bare_name_footprint`, all `Visit` impls; `flatten_use_path`, `is_pascal_case` |
| `process.rs` | 143–308, 518–552 | `process_occurrence`, `absolutize_import_path`, `find_collision_names`, `shadows_prelude` |
| `mod.rs` | 53–88, 310–387, 886–894, 913–941 | `InlinePathScan`, `scan_selection`, `scan_file`, `line_offsets`, `offset`; the existing `#[cfg(test)]` block stays here next to its targets (`is_pascal_case` test, etc.) |

### Sequencing

Steps are checkpoints — run `cargo build && cargo nextest run` between them. The whole phase lands as **one commit**.

1. Create `fixes/inline_path_qualified_type/mod.rs` empty alongside the file, declare `mod scope; mod visitor; mod process;` (each empty), keep all current content in `mod.rs` for now. Build.
2. Extract `scope.rs` (no internal deps on the others). Items become `pub(super)` for use inside `mod.rs`. Build.
3. Extract `visitor.rs` (imports `ScopeInfo` from `scope`). Build.
4. Extract `process.rs` (imports from `scope` and `visitor`). Build.
5. **Enumeration gate.** `rg "use (super|crate)::" fixes/inline_path_qualified_type/` to confirm no `use super::*;` or `use crate::fixes::inline_path_qualified_type::*;` remains. Every cross-submodule import names specific items.
6. Trim `mod.rs` to the orchestrator + utilities + tests. Confirm `runner.rs` continues to compile unchanged. Run full test suite + `cargo +nightly fmt`. **Commit.**

### Module re-exports

`fixes/inline_path_qualified_type/mod.rs` keeps `InlinePathScan` and `scan_selection` defined locally (or re-exported); nothing else needs to leak out. The current `pub(crate)` surface is preserved.

---

## Phase 4 — Split `compiler/facade.rs` (734 prod lines)

Hits three criteria: line count, multiple type clusters (`ParentBoundary` vs `ParentFacadeExports`/`ParentFacadeExportStatus` vs `ParentFacadeReferenceUsage`/`ParentFacadeUsage`), and independently testable domains (the existing inline tests already split along these lines: 4 tests on export status, 4 tests on re-exports).

### Target layout

```
compiler/facade/
├─ mod.rs                  # ParentFacadeFixSupport, ParentFacadeVisibility (small leaf enums),
│                          # re-exports of the public surface
├─ boundary.rs             # ParentBoundary; parent_boundary_for_child(), parent_of_boundary()
├─ exports.rs              # ParentFacadeExports, ParentFacadeExportStatus;
│                          # root_module_exports_item(), parent_facade_export_status(),
│                          # exported_names_from_parent_boundary(),
│                          # collect_matching_pub_use_exports(), widest_visibility(),
│                          # pub_use_is_fix_supported(), parent_facade_visibility()
└─ reference.rs            # ParentFacadeReferenceUsage, ParentFacadeUsage;
                           # scan_facade_usage(),
                           # workspace_source_mentions_parent_export_literal(),
                           # source_references_parent_export(),
                           # resolve_alias_expr_path(), matching_origin_indexed(),
                           # resolve_module_relative_paths(), merge_reference_usage(),
                           # public_reexport_exists_outside_parent()
```

### What goes where

| File | Lines (old) | Items |
|------|-------------|-------|
| `boundary.rs` | 27–31, 291–364 | `ParentBoundary`; `parent_boundary_for_child`, `parent_of_boundary` |
| `exports.rs` | 34–48, 81–167, 366–495, 747–789 | `ParentFacadeExports`, `ParentFacadeExportStatus`; export-status logic; the 4 export-status tests |
| `reference.rs` | 58–79, 169–289, 497–733, 791–817 | `ParentFacadeReferenceUsage`, `ParentFacadeUsage`; reference-matching logic; the 4 re-export tests |
| `mod.rs` | 50–72 (leaf enums) | `ParentFacadeFixSupport`, `ParentFacadeVisibility`; `pub(super) use {boundary, exports, reference}::*;` to preserve the caller-facing surface |

### Callers (preserved without edits)

`compiler/visibility/scan.rs`, `compiler/visibility/policy.rs`, `compiler/exposure.rs` all reach in via `super::facade::*`. After the split, `compiler/facade/mod.rs` re-exports every symbol they import, so no caller edits.

### Sequencing

Checkpoints between steps; one commit.

1. Create `compiler/facade/mod.rs`, declare `mod boundary; mod exports; mod reference;`. Initially keep all content in the old `facade.rs` → move it intact to `compiler/facade/mod.rs`. Build.
2. Extract `boundary.rs` (no inter-submodule deps). Visibility on `ParentBoundary` and the two functions stays `pub(super)`. Build.
3. Extract `exports.rs` (imports `ParentBoundary` from `boundary`). Move its inline tests with it. Build (tests run).
4. Extract `reference.rs` (imports from `boundary` and `exports`). Move its inline tests with it. Build (tests run).
5. **Enumeration gate.** No glob re-exports anywhere in `compiler/facade/`. Replace any `pub(super) use boundary::*;`-style line in `mod.rs` with an explicit list: `pub(super) use boundary::{ParentBoundary, parent_boundary_for_child, parent_of_boundary};` and similarly for `exports` and `reference`. Run `rg "use \w+::\*" compiler/facade/` to verify it's empty.
6. Confirm `mod.rs` is just the leaf enums plus enumerated re-exports. Format + **commit.**

---

## Phase 5 — Split `compiler/visibility/scan.rs` (764 prod lines)

Hits three criteria: line count, multiple type clusters (the classification vocabulary `CrateKind`/`ModuleLocation`/`ParentVisibility`, the recording params `FindingParams`/`SuspiciousPubInput`/`AllowanceReason`/`SuspiciousPubAssessment`, and the runtime context `VisibilityContext`), and independently testable workflows (item analysis vs. finding-recording vs. suspicious-pub assessment). Zero inline tests today, so this is a pure split.

### Target layout

```
compiler/visibility/scan/
├─ mod.rs        # VisibilityContext, ItemInfo, SuspiciousPubInput, FindingParams,
│                # AllowanceReason, SuspiciousPubAssessment;
│                # collect_and_store_findings() entry point
├─ analyze.rs    # analyze_item(), analyze_impl_item(), analyze_foreign_item()
├─ classify.rs   # CrateKind, ModuleLocation, ParentVisibility, VisibilityFindingContext;
│                # visibility_finding_context()
└─ record.rs     # record_visibility_findings(), record_forbidden_pub_crate(),
                 # record_forbidden_pub_in_crate(), record_review_pub_mod(),
                 # maybe_record_narrow_to_pub_crate(),
                 # maybe_record_narrow_to_pub_crate_nested(),
                 # parent_facade_caps_at_pub_crate(),
                 # maybe_record_suspicious_pub()
```

### What goes where

| File | Lines (old) | Items |
|------|-------------|-------|
| `classify.rs` | 43–73, 113–118, 532–556 | `CrateKind`, `ModuleLocation`, `ParentVisibility`, `VisibilityFindingContext`; `visibility_finding_context` |
| `analyze.rs` | 236–367 | `analyze_item`, `analyze_impl_item`, `analyze_foreign_item` |
| `record.rs` | 369–764 | All `record_*` and `maybe_record_*` functions including `maybe_record_suspicious_pub` |
| `mod.rs` | 69–142, 144–234 | `VisibilityContext`, `ItemInfo`, `SuspiciousPubInput`, `FindingParams`, `AllowanceReason`, `SuspiciousPubAssessment`; `collect_and_store_findings` |

### Callers (preserved without edits)

`compiler/visibility/mod.rs` delegates `collect_and_store_findings`. `compiler/visibility/policy.rs` imports `AllowanceReason`, `CrateKind`, `ModuleLocation`, `ParentVisibility`, `SuspiciousPubAssessment`, `SuspiciousPubInput`, `VisibilityContext`. `compiler/visibility/field_visibility.rs` imports `FindingParams`, `VisibilityContext`. After the split, `compiler/visibility/scan/mod.rs` re-exports every such symbol via `pub(super) use {classify, analyze, record}::*;` so no caller edits.

### Sequencing

Checkpoints between steps; one commit.

1. Create `compiler/visibility/scan/mod.rs`; declare `mod analyze; mod classify; mod record;`. Move full body of `scan.rs` into the new `mod.rs`. Build (no functional change).
2. Extract `classify.rs` (no internal deps). Build.
3. Extract `analyze.rs` (imports `VisibilityContext`, `ItemInfo` from `mod.rs`; classification types from `classify`). Build.
4. Extract `record.rs` (imports from `classify`, `analyze`, `mod.rs`'s context types, and `super::super::facade`). Build.
5. **Enumeration gate.** No glob re-exports. Replace any `pub(super) use <submodule>::*;` with explicit `pub(super) use <submodule>::{…};`. Run `rg "use \w+::\*" compiler/visibility/scan/` to verify it's empty. Confirm `policy.rs` and `field_visibility.rs` resolve their `use super::scan::…` imports through the enumerated re-exports.
6. Format + **commit.**

---

## Phase 6 — Split `compiler/exposure.rs` (616 prod lines)

Hits two criteria: line count and mixed domains (boundary-crossing *detection* functions vs. AST *surface-walking* visitor + name-matching helpers). Anchor type for the visitor side is `ItemSurfaceReferenceVisitor`; the detection side is a function cohort with no anchor type. Sole caller is `compiler/visibility/policy.rs`.

### Target layout

```
compiler/exposure/
├─ mod.rs        # re-exports of the public surface for visibility::policy
├─ detect.rs     # child_item_is_exposed_by_other_crate_visible_signature(),
│                # child_item_is_exposed_by_sibling_boundary_signature(),
│                # impl_item_is_exposed_by_exported_self_type(),
│                # find_type_definition_file(), file_defines_type(),
│                # parent_boundary_public_signature_exposes_child_used_outside_parent(),
│                # type_is_exposed_outside_parent()
└─ visitor.rs    # PublicSurfaceStatus, SurfaceReferenceMatch,
                 # ItemSurfaceReferenceVisitor; public_item_name(),
                 # public_item_surface_mentions_name(), impl_self_type_name(),
                 # outward_impl_surface_mentions_name(),
                 # attributes_mention_name(), attribute_tokens_mention_name()
```

### What goes where

| File | Lines (old) | Items |
|------|-------------|-------|
| `visitor.rs` | 376–616 | `PublicSurfaceStatus`, `SurfaceReferenceMatch`, `ItemSurfaceReferenceVisitor`; surface name-matching helpers |
| `detect.rs` | 24–374 | All `child_item_is_exposed_*`, `impl_item_is_exposed_*`, `type_is_exposed_outside_parent`, `parent_boundary_public_signature_exposes_*`, plus `find_type_definition_file`, `file_defines_type` |
| `mod.rs` | — | `pub(super) use {detect, visitor}::*;` |

### Caller (preserved without edits)

`compiler/visibility/policy.rs` calls four detection functions via `super::exposure::…`. The re-exports preserve those paths.

### Sequencing

Checkpoints between steps; one commit.

1. Create `compiler/exposure/mod.rs`; declare `mod detect; mod visitor;`. Move full body of `exposure.rs` into `mod.rs`. Build.
2. Extract `visitor.rs` (no internal deps on `detect`). Build.
3. Extract `detect.rs` (imports `public_item_name`, `public_item_surface_mentions_name`, `outward_impl_surface_mentions_name` from `visitor`). Build.
4. **Enumeration gate.** Replace any `pub(super) use detect::*;` / `pub(super) use visitor::*;` with the explicit lists: `pub(super) use detect::{child_item_is_exposed_by_other_crate_visible_signature, child_item_is_exposed_by_sibling_boundary_signature, impl_item_is_exposed_by_exported_self_type, parent_boundary_public_signature_exposes_child_used_outside_parent, type_is_exposed_outside_parent};` and similarly for `visitor`. Run `rg "use \w+::\*" compiler/exposure/` to verify it's empty.
5. Format + **commit.**
