# `pub(in path)` at an exact facade boundary

> **Status: IMPLEMENTATION PLAN — phased, delegate-ready.** Accept `pub(in <path>)` on a declaration when `<path>` names the exact module a parent facade already exposes the item to, and keep rejecting every other use of the form.

> **As-built disposition: create**

## Delegation Context

- **Project:** `cargo-mend` (single-crate repo, not a workspace; package `cargo-mend` v0.18.0-dev, edition 2024) — "Opinionated visibility auditing for Rust crates and workspaces"; a `rustc_driver`-based cargo plugin that reads the compiler's resolved item graph and reports/fixes visibility findings.
- **Project started:** 2026-07-30T15:27:25-04:00
- **Stack:** Rust, edition 2024, links `rustc_driver`/`rustc_middle` private compiler APIs. **No `rust-toolchain.toml`** — build/install on **stable** with `RUSTC_BOOTSTRAP=1`; `.cargo/config.toml` sets `[env] RUSTC_BOOTSTRAP = "1"` repo-wide, and `docs/style/stable-toolchain-install.md` mandates `RUSTC_BOOTSTRAP=1 cargo +stable install --path .` (nightly-built binaries hit E0514 on stable projects). Key deps: `syn` 2.0 (full, visit) + `proc-macro2` (span-locations), `toml` 1.1 / `toml_edit` 0.25, `serde`/`serde_json`, `cargo_metadata` 0.23, `clap` 4.6, `regex`, `walkdir`, `tempfile`.
- **Layout:**
  - `src/compiler/visibility/` — `mod.rs`, `field.rs`, `policy.rs`, `source.rs`, `use_sites.rs`, `scan/{mod,classify,finding_params,record,visibility_context,visit}.rs` (**new `annotation.rs` lands here**)
  - `src/compiler/facade/` — `mod.rs`, `exports.rs`, `boundary.rs`, `reference.rs`
  - `src/compiler/exposure/` — `mod.rs`, `detect.rs`, `visitor.rs`
  - `src/compiler/persistence/` — `load.rs`, `schema.rs`, `visibility_priority.rs`, `caller_aware.rs`
  - `src/compiler/{constants.rs, settings.rs, source_cache.rs, build/execute.rs}`
  - `src/config/` — `mod.rs`, `loaded.rs`, `global.rs`, `constants.rs`, `diagnostics_config.rs`, `diagnostic_code.rs`, `prelude_pub_mod.rs` (**new `pub_in_path.rs` lands here**)
  - `src/reporting/` — `diagnostics.rs`, `cargo_json.rs`, `render/`
  - `src/fixes/` — `unused_pub.rs`, `narrow_pub_crate.rs`, `field_visibility.rs`, `restricted_annotation.rs`, `visibility_annotation_site.rs`, `pub_use_fixes/scan.rs`, `runner/execute.rs`, `runner/notices.rs` (the last two **new in Phase 11**)
  - `src/rust_syntax.rs` (crate root, **not** under `visibility/`)
  - `tests/diagnostics/` (fixtures + `mod.rs` test target), `tests/support/`, `tests/cli_smoke.rs`
  - `README.md`, `CHANGELOG.md`, `docs/style/`, `~/rust/nate_style/rust/use-narrowest-visibility.md`
- **Key files:** (all verified; where the design's own line refs were stale the corrected value is given)
  - `src/compiler/visibility/annotation.rs` — shipped in Phase 1, 512 lines. `VisibilityAnnotation<'source>` (nine variants) built by `from_item(source, target, tcx) -> Option<Self>` (`:50`); `VisibilitySyntax` (`:17-28`), the nine-variant dispatch enum returned by `syntax()`; `PathSpelling` (`CrateRooted` | `Relative`); `VisibilityReach(Visibility<DefId>)` (`:30-31`, derives **only** `Clone, Copy` — no `Debug`, no `PartialEq`, deliberately no `Ord`) with `compare(self, other, tcx) -> Option<Ordering>`, `join`, `is_at_least`, `is_strictly_wider`, and `to_source(tcx) -> String` (the reach → `pub` / `pub(crate)` / `pub(in crate::…)` rendering); the free fn `anchored(reach, target, tcx)` (`:217`); and a private generic `enum ScopeReach<Scope>` (`:236`) whose `compare`/`join`/`anchored` take the accessibility relation and the parent-module walk as `FnMut` parameters — that seam is what makes the reach algebra unit-testable without a `TyCtxt`, and later phases with the same problem should reuse it. **Everything is `pub(super)`**, reachable only inside `src/compiler/visibility/`; `mod annotation;` in `mod.rs` still carries a scoped `#[expect(dead_code)]` until the last item has a consumer
  - `src/compiler/visibility/mod.rs` — declares only `field, policy, scan, source, use_sites`; needs `mod annotation;`
  - `src/compiler/visibility/scan/record.rs` — **2080 lines after Phase 11 (671 after Phase 5). Phase 5 landed 39 files and shifted this file by up to 80 lines, so the `:NNN` refs in the paragraph below (and in Phases 7/9/11) were last verified against Phase 4 output. Current anchors, re-verified after Phase 6:** `record_forbidden_visibility_annotation` `:200`, `record_forbidden_pub_crate` `:227`, `record_forbidden_pub_in_crate` `:286`, `parent_facade_exports_item` `:477`, `maybe_record_suspicious_pub` `:566`, the `StoredPubUseFixFact` write `:659`. **The `suggestion: None` that Phase 7's pending decision cites as `:553` is now *two* sites — `:608` and `:634`.** `:354` still carries a comment reading "exempt by default (global `allow_prelude_pub_mod`)", which Phase 6 made false; `:356` is the reference call site for reading a resolved config value off `ctx.settings.visibility_config`. Prefer the symbol names over the numbers below. `:34` `record_visibility_findings`, `:47` the `finding_context` computation (already passed as `&finding_context` into `record_forbidden_visibility_annotation` at `:55-62`), `:48-53` the `match annotation.syntax()` — now yielding `Option<ParentFacadeReach>`, not `Option<ParentFacadeVisibility>` — that resolves the parent facade **only for `Crate | InCrate`** (every `InPath`/`InParent`/`InCurrent` gets `None`), the four secondary-check guards using `matches!(annotation.syntax(), VisibilitySyntax::Public)`, `:143` `parent_facade_exports_item` call (unused_pub facade/glob suppression; defined `:472-475`), `:152` + `:244` the two `has_signature_exposure_allowance` calls, `:194-201` `record_forbidden_visibility_annotation` signature (`:211-213` = the `InParent | InCurrent | InPath(_)` dispatcher arm, `:212` = the `record_forbidden_pub_in_crate` call site), `:221` `record_forbidden_pub_crate` (handles `Crate` **and** `InCrate`; `:246-248` = the permitted-`InCrate` branch emitting ``consider using: `pub(crate)` ``), `:279-317` `record_forbidden_pub_in_crate` (**the one recorder still lacking `VisibilityFindingContext`** — one parameter, one call site; `:285-299` = the `suggestion` match, `:288` = the `CrateRooted` arm still yielding `suggestion: None`, `:289-292` = the `InPath(Relative)` arm that already suggests the crate-rooted spelling), the two `narrow_to_pub_crate` recorders, `:459-470` `resolve_parent_facade_reach`, `:486` `maybe_record_suspicious_pub` (`:508-527` = the spelling-gated facade wording — **the reference pattern for the reach≠spelling invariant**; `:553` = its `suggestion: None`), `:578` `StoredPubUseFixFact` write
  - `src/compiler/visibility/scan/visibility_context.rs` — `ItemCategory { Module, Declaration, Use }` at `:40-44` (Phase 3 split the old `NonModule`; `Use` is populated from `ItemKind::Use` in `visit.rs` but **matched nowhere yet** — Phase 7 is its first consumer), `ItemInfo` at `:46` with `impl_self_name: Option<String>`, `FINDINGS_SCHEMA_VERSION` stamping at `:143`
  - `src/compiler/visibility/scan/visit.rs` — `visit_item` at `:19`; `impl_self_name` populated at `:76/:101/:113/:146`
  - `src/compiler/visibility/scan/classify.rs` — `SignatureExposure` at `:27` (retire)
  - `src/compiler/visibility/scan/finding_params.rs` — `FindingParams` construction boundary (where `Visibility<DefId>` converts to text)
  - `src/compiler/visibility/field.rs` — 217 lines after Phase 3 (196 after Phase 1, 214 before; every pre-Phase-3 line reference in this plan is stale). `use super::annotation::VisibilityReach;` at `:21`; `check_item` at `:30`; **Phase 3's shared-classifier call in `check_field` at `:85-104`** — it builds a synthetic `ItemInfo` literal at `:93-102` (`kind_label: Some("field")`, `category: ItemCategory::Declaration`, `impl_self_name: None`) and calls `scan::record_forbidden_visibility_annotation(ctx, &field_info, &annotation, None, sink)` at `:102`, returning early when that reports `true`. **That hard-coded `None` for `parent_facade_visibility` is load-bearing: it is what keeps fields out of facade-based acceptance** (a field is not re-exportable). The wider-than-type comparison `field_declared.is_strictly_wider(type_declared, ctx.tcx)` is now at `:124`; `effective_type_visibility` (returns `VisibilityReach`) at `:162`. `DefIdVisibility`, the local `visibility_strictly_wider`, and the local `is_at_least` are **gone** — all three now live on `VisibilityReach`
  - `src/compiler/visibility/policy.rs` — **1164 lines after Phase 11 (753 after Phase 5); Phase 11 did not touch this file, so every `policy.rs` anchor Phase 12 cites is exact. the `:NNN` refs in the paragraph below are post-Phase-4 and drifted. Current anchors, re-verified after Phase 6:** `classify_suspicious_pub` `:38`, `forbidden_pub_crate_suggestion` `:218`, `assess_parent_facade_usage` `:293`, `assess_signature_exposure_allowance` `:323`, `has_signature_exposure_allowance` `:514`, unit tests start `:524`. Prefer the symbol names over the numbers below. Original post-Phase-4 notes: `:35` `classify_suspicious_pub`, `:66-80` the stale-facade notes (already `use_syntax()`-gated), `:113-122` the doc comment for the depth rule, `:123-139` `resolve_module_location` (**depth 1 and depth 2 both `ShallowPrivate`**), `:150-163` `allow_pub_crate_by_policy`, `:178` `forbidden_pub_crate_help` (`const fn -> &'static str`, stays), `:215` `forbidden_pub_crate_suggestion` — now **three** params, `const fn(ModuleLocation, SignatureExposure, Option<ParentFacadeReach>) -> &'static str`, with **two** `Super`-facade arms: `:224-234` fires only when `spelling == Super && !spelling_conflict` and quotes `` `pub(super) use` `` plus the E0364 compile claim, `:235-244` is the neutral fallback naming neither; `:290` `assess_parent_facade_usage`, `:320` `assess_signature_exposure_allowance`, `:371-422` `parent_facade_export_status` (**loops over every occurrence in `occurrences.matching`**, one `facade::parent_facade_export_status` call each), `:480` `has_signature_exposure_allowance` (four params: `ctx, item_def_id, file_path, item_name`); unit tests start `:490`
  - `src/compiler/visibility/use_sites.rs` — **Phase 4 made this the facade-identity home; every pre-Phase-4 line reference here is stale.** `ReexportIndex` + `reexport_index` builder at `:892`; `ReexportOccurrence` carrying `visibility: Visibility<DefId>` from `tcx.local_visibility(use_def_id).map_id(...)` at `:907-909`/`:924`; `ParentFacadeOccurrences { selected, matching, spelling_conflict }` at `:137-141`; `ReexportIndex::parent_facade_occurrences` at `:171-247` (the ancestor walk; glob handling is inside it at `:214-241`); `FacadeVisibility::widest` at `:106-113` (**an ordinal, not a reach comparison — see the invariant below**); `widest_applicable_occurrence` at `:307-333`; production facade parsing at `:1021-1099`, which calls `super::annotation::VisibilityAnnotation::from_item` at `:1040`; `def_path_string`, `parent_module_path_segments` (dead `PathAnchor::Crate` strip). **`public_reexport_targets` no longer exists.**
  - `src/compiler/facade/exports.rs` — **post-Phase-4.** `ParentFacadeVisibility` at `:44`, `ParentFacadeSpelling` + `spelling_conflict` at `:51-64`, `ParentFacadeReach { visibility, spelling, spelling_conflict }` at `:59-64`, `ParentFacadeExportStatus` at `:74`, `ParentFacadeExportStatus::use_syntax() -> Option<&'static str>` at `:86-96`, `parent_facade_export_status` at `:115`, `scan_facade_usage` call at `:158-166`, `pub_use_is_fix_supported_with_prefix` at `:319`. **`parent_facade_has_glob_export`, `parent_boundary_has_matching_pub_use_glob`, `exported_names_from_parent_boundary`, and `collect_matching_pub_use_exports` are gone from production** — `parent_facade_visibility` (`:239`), `exported_names_from_parent_boundary` (`:266`), `collect_matching_pub_use_exports` (`:300`), and `widest_visibility` (`:342`) are all now `#[cfg(test)]`. Glob suppression for `unused_pub` rides `parent_facade_exports_item` (`record.rs:143`, defined `:472-475`).
  - `src/compiler/facade/mod.rs` — re-export surface for the facade API; changes with `ParentFacadeAnalysis`
  - `src/compiler/facade/boundary.rs` — `parent_boundary_for_child`
  - `src/compiler/facade/reference.rs` — `scan_facade_usage` at `:34`, `workspace_source_mentions_parent_export_literal` at `:117`
  - `src/compiler/exposure/mod.rs` — `:4-8` re-exports the four exposure predicates that used to live in `policy.rs`: `child_item_is_exposed_by_other_crate_visible_signature`, `impl_item_is_exposed_by_exported_self_type`, `child_item_is_exposed_by_sibling_boundary_signature`, `parent_boundary_public_signature_exposes_child_used_outside_parent`
  - `src/compiler/exposure/detect.rs` — `type_is_exposed_outside_parent` at `:315-322`, now taking `item_def_id: LocalDefId`; `module_signature_exposes_item` resolves `exposing_item_def_id` at `:112` and `self_type_def_id` at `:140`. **Phase 4 already threaded `LocalDefId` through this recursion** — the exposing item's identity is no longer discarded
  - `src/compiler/exposure/visitor.rs` — `public_item_name` at `:62-87`, the remaining source/name-based step: it matches only `Visibility::Public(_)` on unexpanded `syn` items, so a `pub(crate)`/`pub(in path)` item never counts as exposing
  - `src/compiler/visibility/scan/mod.rs` — **new in Phase 4.** `:26-42` the `record_forbidden_visibility_annotation` wrapper, whose parameter is already `parent_facade_reach: Option<ParentFacadeReach>`; it computes its own `VisibilityFindingContext`
  - `src/compiler/source_cache.rs` — `SourceCache` at `:45`
  - `src/rust_syntax.rs` — `trim_leading_self` at `:51`, `module_name_for_child_boundary_file` at `:74` (the `#[path]`/filename identity issue)
  - `src/compiler/constants.rs` — `FINDINGS_SCHEMA_VERSION` at `:65` (currently **`22`**; Phase 7 bumped it `18` → `19`, Phase 9 took it to `21` for the typed visibility-constraint facts, Phase 11 took it to `22` when findings gained the typed `ItemVisibility` / `NarrowerScope` shape)
  - `src/compiler/settings.rs` — `current_analysis_fingerprint()` at `:67`
  - `src/compiler/persistence/load.rs` — `stored_report_matches_selection` at `:159`, three-way check at `:166-168`
  - `src/compiler/persistence/schema.rs` — `StoredReport`/`StoredFinding`/`StoredPubUseFixFact` (`:73`)
  - `src/compiler/persistence/visibility_priority.rs` — `apply_visibility_narrowing_priority` at `:7`
  - `src/compiler/persistence/caller_aware.rs` — `apply_caller_aware_suppression` at `:6` (the use-site map the no-facade classifier reads)
  - `src/compiler/build/execute.rs` — `run_selection` at `:74`, `CargoCheck` bail at `:103-107`
  - `src/config/loaded.rs` — **post-Phase-6; every pre-Phase-6 line reference in this plan is stale.** `ProjectVisibilityConfig` (raw parse, `Option<_>` fields) at `:25`, its `resolve(&GlobalConfig) -> VisibilityConfig` at `:42-51` — **the single precedence resolution point**; `VisibilityConfig` (resolved) at `:54`, `load_config` at `:73`, `fingerprint_for` at `:126`. **The global-stamps-over-project block that used to sit at `:81-85` no longer exists** — both `load_config` paths (found `mend.toml` and no `mend.toml`) route through `resolve`
  - `src/config/pub_in_path.rs` — **shipped in Phase 6.** `enum PubInPath { Forbidden, Permitted, Required }`, `#[serde(rename_all = "lowercase")]`, `#[default]` on `Permitted`
  - `src/config/mod.rs` — `mod pub_in_path;` at `:9`. **Still needs `pub(crate) use pub_in_path::PubInPath;`** — the precedent `PreludePubMod` carries one at `:27`, which is how `record.rs:36` imports it; without it `crate::config::PubInPath` is E0603. Phase 7 owns that line
  - `src/config/prelude_pub_mod.rs` — the `PreludePubMod` precedent. **Unchanged by Phase 6** — its project>global move lives entirely in `loaded.rs`
  - `src/config/global.rs` — `GlobalConfig` at `:32`, `GlobalConfigFile` at `:39`, `GlobalVisibility` at `:47` (keeps `#[serde(default)]`, correctly: absence at the *global* layer should fall to the compiled-in default), `reconcile_global_config` at `:86` with the `pub_in_path` insertion block at `:118`, `default_global_config_toml` at `:161`
  - `src/config/constants.rs` — `PRELUDE_KEY` at `:10`, `PUB_IN_PATH_COMMENT` at `:11`, `PUB_IN_PATH_KEY` at `:13`, `DEFAULT_GLOBAL_CONFIG_TOML`
  - `src/config/diagnostics_config.rs` — `DiagnosticsConfig` at `:10`, `is_enabled` at `:16`
  - `src/config/diagnostic_code.rs` — `DiagnosticCode::ForbiddenPubInCrate` at `:8`, `ALL` at `:26`, `as_str()` at `:44`
  - `src/reporting/diagnostics.rs` — `FixSupport` enum at `:14-30` (**no `Standard` variant**; `FixSummaryBucket::Standard` at `:33` is a different type), `DiagnosticSpec` at `:88` (its `headline` is now a private `HeadlineSource`, not `&'static str`), `FORBIDDEN_PUB_IN_CRATE` spec literal at `:105-113`, its `diagnostic_spec` dispatch arm at `:350`, `SUSPICIOUS_PUB.inline_help` at `:123`, `finding_headline` at `:374`, `finding_message_not_in_headline` at `:387`, the mechanism-pinning unit test `forbidden_headline_uses_message_with_static_fallback` at `:445-479`. These are post-Phase-2 lines; anything citing `:82`/`:334`/`:358` predates it. **The production `HeadlineSource` variants are `Static` / `FindingMessage` (`:82-85`) — `Literal` is the *test mirror*'s name for `Static` (`tests/support/diagnostics.rs:124-127`); never write `Literal` when describing `src/`.** Phase 4 moved `INTERNAL_PARENT_PUB_USE_FACADE` (`:172-181`) from `Static` to `FindingMessage`, so **three** specs now use `FindingMessage` while the mechanism-pinning test still loops over only the two forbidden codes.
  - `src/reporting/cargo_json.rs` — `rustc_diagnostic` at `:146`, `render_diagnostic` at `:202`
  - `src/reporting/render/diagnostic.rs`, `src/reporting/render/human.rs` — human renderer consuming `finding_headline` + `suggestion`
  - `src/fixes/visibility_annotation_site.rs` — **new in Phase 11; the single annotation-span parser, and the only one.** `VisibilityAnnotationSite::locate` at `:43` returns the byte range of the written annotation plus a typed `VisibilityAnnotationForm` (`Bare` | restricted). `byte_offset_of_display_column` converts rustc's `col_display + 1` to a byte offset using `TAB_DISPLAY_WIDTH` (`src/fixes/constants.rs`). **The three predecessors are gone:** `field_visibility.rs`'s `visibility_annotation_byte_len`, `unused_pub.rs`'s `bare_pub_annotation_byte_len`, and `narrow_pub_crate.rs`'s raw `line_text.find("pub ")`
  - `src/fixes/unused_pub.rs` — `scan_from_report` at `:15`. **The bare-only refusal is still behavior, not an oversight**, but it now lives at the call site as an explicit `if site.form != VisibilityAnnotationForm::Bare` gate at `:34`, not inside the parser. Removing that gate would make `--fix` start stripping restricted annotations
  - `src/fixes/narrow_pub_crate.rs` — `scan_from_report` at `:15`; the same explicit `site.form != VisibilityAnnotationForm::Bare` gate at `:34`
  - `src/fixes/field_visibility.rs` — also calls `locate`, and correctly carries **no** bare-only gate (a field's restricted annotation is exactly what it narrows)
  - `src/fixes/restricted_annotation.rs` — **new in Phase 11.** The Required-mode fixer that rewrites a bare `pub` to the exact `pub(in crate::…)` boundary. Keys each already-rewritten site by `path:byte-offset` in a `rewritten_sites` set at `:63` — that set, not `load.rs`'s dedup, is what actually prevents a double edit when the same file is compiled as both lib and bin
  - `src/fixes/runner/notices.rs` — `import_fix_notice_count` at `:11-25` sums **every** non-`pub-use` fixer (including `restricted_annotation`) into one counter rendered as "applied N import fix(es)". The noun is wrong for annotation rewrites; it predates Phase 11 and is pinned by the test at `:102`
  - `src/fixes/pub_use_fixes/scan.rs` — `:120` child-module resolution, `screen_candidate` at `:183` (`AlreadyNarrowed` on non-bare-`pub`), reaching `line_contains_plain_pub` at `:237`
  - `src/fixes/pub_use_fixes/parent_boundary.rs` — `trim_leading_self` consumers at `:133/:198`; `:95` selects a parent `use` occurrence by line and picks the wrong one when two `use` declarations share a line
  - `src/fixes/runner/execute.rs` — disabled-diagnostic filtering at `:125-130` (runs *after* analysis)
  - `tests/diagnostics/mod.rs` — the `diagnostics` test target root; new fixture modules declared here
  - `tests/diagnostics/{forbidden_pub_crate,allowances,field_visibility_wider_than_type,pub_use_fixes,rendering,narrow_pub_crate,unused_pub,prelude_pub_mod}.rs` — existing suites the fixtures extend. **`tests/diagnostics/forbidden_pub_in_crate.rs` now exists** — Phase 3 created it (172 lines, registered in `tests/diagnostics/mod.rs`). Its single test `restricted_visibility_annotations_are_rejected_once` asserts complete per-file diagnostic-code vectors (so a duplicate finding fails by construction) plus nine exact headline/help pairs at `:77-130`. Extend it; do not recreate it. **`tests/diagnostics/rendering.rs` is a hard gate — 1543 lines after Phase 5; re-verified after Phase 6.** `assert_rendered_diagnostics` is at `:230` and panics at `:237` with "fixture is missing finding for {code:?}" if any `DiagnosticCode` fails to fire. **There are *two* count assertions, not one: `assert_eq!(…, 16)` at `:409` and at `:633`** (post-Phase-7; they were `:404`/`:522`) — any phase that changes the finding count must adjust both. Older refs of `:220`, `:223-227`, and `:399` in Phases 7/9/12 predate Phase 5 and are stale. The fixture source for `pub(in crate::private_parent) fn subtree_only()` lives at `rendering.rs:144` — it is fixture text inside this test file, not a `src/` path. Any phase that changes which findings fire must adjust the fixture, never the assertion.
  - `tests/support/` — fixture-crate harness
  - `README.md` — `forbidden-pub-crate` anchor at `:194`, `forbidden-pub-in-crate` at `:258`, `suspicious-pub` at `:287`; config section ~`:70-99`
  - `CHANGELOG.md` — release / upgrade-contract bullets
  - `docs/style/diagnostic-lifecycle.md` — checklist for diagnostic changes (its own path list is stale: `src/config.rs` → `src/config/diagnostic_code.rs` + `src/config/constants.rs`; `src/diagnostics.rs` → `src/reporting/diagnostics.rs`; `src/runner.rs` → `src/fixes/runner/`)
  - `docs/style/readme-diagnostic-section.md` — README section format (no change needed)
  - `docs/plans/prelude-pub-mod-exemption.md` — the config-mechanics precedent
  - `~/rust/nate_style/rust/use-narrowest-visibility.md` — external style guide to edit
  - `~/rust/nate_style/rust/name-submodules-after-anchor-types.md` — dictates the `annotation.rs` filename
  - `build.rs` — `rerun-if-changed` on `.git/HEAD`, `.git/refs/heads`, `build.rs` only
- **Build:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-mend`
- **Test:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-mend` — the only declared `[[test]]` is `diagnostics` (`tests/diagnostics/mod.rs`), reachable as `bash ~/.claude/scripts/delegate/verify.sh test cargo-mend diagnostics`; `cli_smoke` is auto-discovered.
- **Lint:** `bash ~/.claude/scripts/delegate/verify.sh lint cargo-mend`
- **Style:** `bash ~/.claude/scripts/rust_style/load-rust-style.sh --scope edit --project-root /Users/natemccoy/rust/cargo-mend`
- **Invariants:**
  - **`Visibility<DefId>` is session-local and must never be persisted.** It needs `TyCtxt` to compare and is only meaningful inside the compiler pass. `VisibilityReach` stays inside `src/compiler/`; convert to text immediately before constructing `FindingParams` — `Public` → `pub`; `Restricted(CRATE_DEF_ID)` → `pub(crate)`; any other restriction → `pub(in crate::{tcx.def_path_str(scope)})`. No `Serialize`, nothing reaching `persistence/schema.rs`.
  - **`tcx.def_path_str` on a local def returns a root-relative path with no `crate::` and no crate-name segment** — verified empirically. Prepending `crate::` is correct. The `PathAnchor::Crate` strip in `use_sites.rs:492` is dead for local defs.
  - **No `Ord`/`PartialOrd` on `VisibilityReach`.** Comparison needs `TyCtxt` and sibling restricted scopes are genuinely incomparable. Ranking `ParentFacadeVisibility` produces `Super → Super` suggestions that fail E0364 — that enum's `Super` is relative to the module holding the `use`.
  - **Every computed reach must be anchored to the declaration** via `join(reach, Restricted(parent_module(target)))`. A `pub(in <path>)` naming a sibling is E0742.
  - **`FINDINGS_SCHEMA_VERSION` is `22`** (`src/compiler/constants.rs:65`; Phase 7 bumped it `18` → `19` when findings gained `visibility_annotation`, Phase 9 took it to `21` when the typed visibility-constraint facts got their own report collection, Phase 11 took it to `22` when the stored finding gained the typed `ItemVisibility` / `NarrowerScope` shape). **A bump silently blanks every stale report** — `load.rs` rejects on version mismatch, so a repo whose `cargo check` is fully cached emits nothing fresh, has its old reports discarded, and prints `No findings.` indistinguishably from a real pass. Any self-policy run or manual scan must `rm -rf target/mend-findings` and force a recompile first, or it proves nothing. Any change to the persisted shape *or to the meaning of stored values* requires a bump; `load.rs:166-168` rejects on version, analysis fingerprint, or config fingerprint mismatch. Prior-schema reports are rejected, never partially loaded.
  - **Accepted known limitation:** fingerprints decide whether a report is trusted; they cannot make Cargo compile. A Cargo-fresh crate whose report is rejected simply vanishes from the run — a green result with missing findings, not stale ones. `cargo clean` clears it. No work is scheduled for this.
  - **Diagnostic codes and README anchors are load-bearing.** `DiagnosticCode::as_str()` supplies `mend.toml` `[diagnostics]` keys and `DiagnosticSpec.help_anchor` must match a live `<a id="...">`. Keep `ForbiddenPubInCrate` / `forbidden_pub_in_crate` / `#forbidden-pub-in-crate` exactly as-is. This plan adds no new diagnostic code, no persisted field, no README anchor, and exactly one config key.
  - **`[diagnostics]` is enable/disable only — there is no severity downgrade,** and filtering happens *after* analysis (`fixes/runner/execute.rs:125`). Do not invent a warning tier. The rejection forms split across two codes: `pub(in crate)` → `forbidden_pub_crate`, everything else → `forbidden_pub_in_crate`. Document the split; do not soften it.
  - **Never advertise a fix that produces no edit.** `FixSupport::None` alone is insufficient — the pub-use fixer routes from *stored facts*, so a restricted-annotation finding must both carry `FixSupport::None` and write no `StoredPubUseFixFact`. `unused_pub.rs:15` and `narrow_pub_crate.rs:15` only match a bare `"pub "`.
  - **Reject once, then stop.** After an annotation is rejected, `suspicious_pub`, both `narrow_to_pub_crate` recorders, `unused_pub`, and the field-specific check must not also run. `visibility_priority.rs:7` only suppresses on `unused_pub`, so post-persistence priority cannot clean up the overlap.
  - **Never suggest code that does not compile.** Whole-chain resolution, E0742 anchoring, and the ordered no-facade classifier all exist for this.
  - **Facade identity comes from active HIR, not source text.** `SourceCache` parses unexpanded source with `syn` and evaluates no attributes — valid for line reporting, usage analysis, and auto-fix eligibility only, never for deciding whether a re-export exists.
  - **Keep the written facade spelling alongside the resolved reach.** `pub(super) use` and `pub(crate) use` in a crate-root child resolve identically but grant `InternalParentFacadeBoundary` differently.
  - **Resolved reach never establishes written syntax — never quote a modifier the tool did not read.** `pub(super)` ≡ `pub(in super)`; at the crate root a private `use` and a `pub(crate) use` both resolve to `CRATE_DEF_ID`; a macro-expanded span has no recoverable spelling. Any diagnostic that quotes a `use` modifier must be gated on `ParentFacadeExportStatus::use_syntax() -> Option<&'static str>` (`exports.rs:86-96`, which already folds in `spelling_conflict`) and must render the neutral word "re-export" on `None`. Reference pattern: `maybe_record_suspicious_pub`, `record.rs:508-527`. Phase 4 spent three fix-pass rounds on violations of this rule alone.
  - **Policy forbidding a modifier is not the same as the compiler rejecting it.** `pub(in crate::<grandparent>)` is narrower than `pub` and compiles fine; this tool refuses it by policy. Never write "a narrower modifier would not compile" as a general claim. The one place a compile claim is correct is a *known, unconflicted* `pub(super) use` facade, where the re-export would exceed the item and fail E0364 (`policy.rs:224-234`).
  - **Fixture depth three or deeper for any `ForbiddenPubCrate`-absence assertion.** `resolve_module_location` (`policy.rs:123-139`) returns `ShallowPrivate` for logical depth **one and two alike**, and `allow_pub_crate_by_policy` (`policy.rs:150-163`) then permits `pub(crate)` independently of anything else — documented at `policy.rs:113-122`. A depth-two fixture therefore passes an absence assertion no matter what the code under test does, proving nothing. Phases 7, 9, and 12 all add fixtures of this shape.
  - **Nearest-facade metadata and chain result stay separate.** Overwriting `ParentFacadeExportStatus::visibility` with the chain-widest value mis-pairs `parent_path`/`child_module`/usage and silently drops fixes.
  - **Cross-crate `crate::` literals must not count as usage** — keep crate identity in any use-site key.
  - **Resolved decision (user, Phase 5): a textual `crate::<module>::<Item>` match inside the crate under analysis counts as usage, including inside a macro body.** The literal scan in `facade/reference.rs` exists precisely because the HIR walk cannot see into unexpanded macro bodies, and a macro can expand into a genuine use. A mention that is *not* a real use — `stringify!(crate::a::Thing)` — therefore counts too; that false "used" is accepted deliberately, because a false "unused" leads to deleting code a macro depends on. This governs only same-crate matches: the cross-crate rule above is unchanged, and `same_package_binary_literal_does_not_count_as_library_facade_usage` (`tests/diagnostics/rendering.rs`) stays correct because a binary target is a different crate from the library. Do not "fix" the macro-body case by making the scan ignore macro bodies or by requiring a syntactic use.
  - **Config precedence: project `mend.toml` > global > compiled-in default.** The project value deserializes as `Option<_>` so absence stays distinguishable. Fingerprint and serialize only the *resolved* config. One `LoadedConfig` serves the entire Cargo selection — per-member visibility policies cannot coexist in one run. Resolution happens in exactly one place, `ProjectVisibilityConfig::resolve` (`config/loaded.rs:42-51`); adding a `[visibility]` key means `Option<_>` on `ProjectVisibilityConfig`, a plain value on `VisibilityConfig`, a line in `resolve`, a `reconcile_global_config` block, and an entry in `default_global_config_toml`.
  - **Every diagnostics fixture that depends on a `[visibility]` setting must write its own project `mend.toml`.** Phase 6 moved `allow_prelude_pub_mod` — and shipped `pub_in_path` — onto project>global precedence, so a fixture that writes no `mend.toml` now inherits **the developer's real machine-global config** and its result depends on who runs it. This is not hypothetical: no fixture in `tests/diagnostics/forbidden_pub_in_crate.rs` writes one, and neither does the all-diagnostics fixture in `rendering.rs`, so once Phase 7 makes `pub_in_path` load-bearing a developer whose global config says `pub_in_path = "required"` gets extra `suspicious_pub` findings and breaks **both** `assert_eq!(…, 16)` gates on unchanged source. Phase 6 already fixed its own two prelude fixtures this way; Phase 7 owns pinning the pre-existing setting-dependent ones, and Phases 9–12 must pin every fixture they add.
  - **Clippy is deny-by-default** at `all`/`cargo`/`nursery`/`pedantic`, plus `unwrap_used`, `expect_used`, `panic`, `unreachable`, `allow_attributes_without_reason`. `self_named_module_files = "deny"` — a module with submodules uses `module/mod.rs`, never `module.rs` beside `module/`. `redundant_pub_crate` is allowed on purpose.
  - **cargo-mend must pass on itself:** `RUSTC_BOOTSTRAP=1 cargo +stable run -- --workspace --all-targets --fail-on-warn` reports "No findings". New code is subject to the policy it implements.

## Phases

### Phase 1 — Annotation type and reach algebra · status: done (`67a7321`)

#### Work Order

**Goal:** `src/compiler/visibility/annotation.rs` exists with a typed visibility annotation and a context-aware reach comparison, unit-tested, with no behavior change anywhere else.

**Spec:**

Create `src/compiler/visibility/annotation.rs` and declare `mod annotation;` in `src/compiler/visibility/mod.rs`. The anchor type is `VisibilityAnnotation`, and `name-submodules-after-anchor-types.md` takes the filename from the anchor with the parent module's prefix dropped — hence `annotation.rs`, not `modifier.rs` and not `visibility_annotation.rs`.

Three parallel fields (`source`, `syntax`, `reach`) would admit combinations rustc can never produce — `source = "pub(in crate::a)"` beside `syntax = Public`. Because the advice matrix dispatches on syntax and compares reach separately, one construction slip accepts the wrong annotation or emits advice contradicting its own headline. Make the illegal states unrepresentable:

```rust
enum VisibilityAnnotation<'source> {
    Private,
    Public,     // `pub`
    Crate,      // `pub(crate)`
    Parent,     // `pub(super)`
    Current,    // `pub(self)`
    InCrate,    // `pub(in crate)`
    InParent,   // `pub(in super)`
    InCurrent,  // `pub(in self)`
    InPath {
        source:   &'source str,   // exactly what the author wrote
        spelling: PathSpelling,   // `CrateRooted` | `Relative`
        reach:    VisibilityReach,
    },
}

struct VisibilityReach(Visibility<DefId>);
```

The fixed spellings need no stored source or reach — both are implied by the variant. Only path spellings carry them. Expose `source()`, `syntax()`, and `reach()` accessors where a consumer wants a uniform interface.

Take `reach` from `tcx.visibility` rather than resolving the written path again — the driver already has the item's `DefId` and the compiler's answer, and `field.rs:100-166` is the existing precedent for using it.

`VisibilityReach` exposes `compare(self, other, tcx) -> Option<Ordering>` and `join(self, other, tcx) -> Self`, built on the relation already implemented at `field.rs:162-166`:

```rust
at_least(lhs, rhs) = match rhs {
    Visibility::Public        => lhs.is_public(),
    Visibility::Restricted(b) => lhs.is_accessible_from(b, tcx),
}
```

`join` returns the wider operand when one reaches the other; for sibling restricted scopes it returns their nearest common ancestor module. `Public` dominates everything. Move `is_at_least` and `visibility_strictly_wider` (`field.rs:154`, `:162-166`) onto `VisibilityReach` so the field scan and this module share one implementation, and retire the local `type DefIdVisibility = Visibility<DefId>;` (`field.rs:24`).

Do **not** derive `Ord`/`PartialOrd`: comparison needs `TyCtxt` and sibling scopes are genuinely incomparable, which an ordering would paper over.

Also provide the anchoring helper, because `pub(in <path>)` is only legal when `<path>` names an *ancestor* of the module the item is declared in — naming a sibling is E0742, verified against rustc (`pub(in crate::a::right)` on an item in `crate::a::left` is E0742; `pub(in crate::a)` compiles):

```rust
fn anchored(reach: VisibilityReach, target: LocalDefId, tcx: TyCtxt<'_>) -> VisibilityReach {
    reach.join(VisibilityReach(Visibility::Restricted(parent_module(target))), tcx)
}
```

And the rendering conversion, used from Phase 5 onward but defined here:

- `Public` → `pub`
- `Restricted(CRATE_DEF_ID)` → `pub(crate)`
- `Restricted(boundary)` → `pub(in crate::{tcx.def_path_str(boundary)})`

**Files:**
- `src/compiler/visibility/annotation.rs` — new module: `VisibilityAnnotation`, `PathSpelling`, `VisibilityReach`, `anchored`, rendering conversion, unit tests
- `src/compiler/visibility/mod.rs` — add `mod annotation;`
- `src/compiler/visibility/field.rs` — move `is_at_least` (`:162-166`) and `visibility_strictly_wider` (`:154`) onto `VisibilityReach`; call through; delete `DefIdVisibility` (`:24`); `:100-166` now uses the shared type

**Constraints from prior phases:** none — this is Phase 1.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-mend` and `bash ~/.claude/scripts/delegate/verify.sh test cargo-mend` green, `bash ~/.claude/scripts/delegate/verify.sh lint cargo-mend` clean. New unit tests in `annotation.rs`: variant classification (`pub(in crate)` → `InCrate`, `pub(in super)` → `InParent`, `pub(in self)` → `InCurrent`, `pub(in crate::a::b)` → `InPath { spelling: CrateRooted, .. }`, `pub(in super::super)` → `InPath { spelling: Relative, .. }`, `""` → `Private`, `"pub"` → `Public`, `"pub(crate)"` → `Crate`); `compare` returns `Equal` for mutual reach, `Greater`/`Less` for ancestry, `None` for sibling restricted scopes; `join` returns the wider operand and the nearest common ancestor for siblings; `anchored` normalizes a lone sibling-scope reach to a common ancestor of the declaration, never to the sibling itself. No existing test changes behavior.

#### Retrospective

**What worked:**
- The illegal-states-unrepresentable enum landed exactly as specified — nine variants, `source`/`syntax`/`reach` carried only where a path spelling needs them.
- Routing `field.rs` through the shared `VisibilityReach` removed `DefIdVisibility`, `is_at_least`, and `visibility_strictly_wider` with no behavior change; the field scan's tests were untouched.

**What deviated from the plan:**
- The acceptance gate demanded unit tests for `compare`/`join`/`anchored`, all of which need a `TyCtxt` a unit test cannot construct. Resolved by factoring the decision logic into a generic `enum ScopeReach<Scope>` whose `compare`/`join`/`anchored` take the accessibility relation and the parent-module walk as `FnMut` parameters. `VisibilityReach` supplies rustc's real functions; tests supply a `TestModule` fixture tree. **Later phases with the same problem should reuse this seam rather than rediscover it.**
- `mod annotation;` carries a scoped `#[expect(dead_code)]` because Phase 1 ships no consumer. The expectation is self-healing: once a consumer exists the unfulfilled `expect` warns and forces its own removal.

**Surprises:**
- `ScopeReach::join`'s common-ancestor walk was unbounded as first written. `tcx.parent_module_from_def_id(CRATE_DEF_ID)` is `CRATE_DEF_ID` itself, and the parent-module closure returns a non-local `DefId` unchanged — either is a fixed point, so the walk could spin forever. It now terminates at a fixed point by returning `ScopeReach::Public`. A local walk resolves at the crate root first (`Restricted(CRATE_DEF_ID)` is accessible from any local boundary), so the `Public` fallback fires only for a boundary the walk cannot climb — a non-local `DefId`.
- Both blind reviews approved a violation of cargo-mend's own house rule (`&syn::VisRestricted`, an inline path-qualified type the tool flags). Reviewers read the diff; only running the tool on its own source catches this class. **The self-run is not optional at any phase gate.**

**Implications for remaining phases:**
- Phase 4 maps `extern crate` subjects into the same reach path, which is where the `Public` fixed-point fallback becomes reachable. Phase 4 must decide deliberately whether `pub` is the right suggestion for a foreign boundary rather than inheriting the fallback silently.
- Phase 5 is the first cross-module consumer of `VisibilityReach`, which ships `pub(super)` (visible only inside `src/compiler/visibility/`). Phase 5's Work Order must name `annotation.rs` in its **Files** and widen the type there — using the house re-export pattern, not `pub(in crate::compiler)`, which the tool still rejects until Phase 7.

### Phase 1 Review

- Phase 3 shrank: `VisibilityAnnotation::from_item` already does the `syn` parse and the eight-way classification, so what remains is rewiring the `record_*` arms, `ItemCategory`, the field path, reject-once ordering, and the schema bump. Its **Files** now point at the `is_strictly_wider` call in `check_field` (`field.rs:100`) instead of a stale `:34`, and its **Constraints** name `VisibilitySyntax` as the dispatch enum, define a `None` from `from_item` as skip-silently, and warn that `reach()` takes `(target, tcx)` — for a field that is `field.def_id`, not the containing type's.
- Phase 4 gained the fixed-point fact: an unmapped `extern crate` subject does not error, it silently yields `ScopeReach::Public` and therefore a too-wide `pub` suggestion. Its gate now asserts the `extern crate` fixture's computed reach is the local boundary and not `pub`, and one fixture must exercise the fixed-point branch against a real `TyCtxt` — it has only ever run against a hand-written test fixture.
- Phase 4 carried a `**Pending decision:**` on how a foreign boundary should be reported. **Resolved by the user in favor of `FacadeChainBlocker::ForeignBoundary`**: an unresolvable chain naming the blocking facade, no replacement annotation, and no public-boundary row in Phase 7's matrix. Recorded in Phase 4's Work Order as a **Resolved decision**; the blocker variant and its detection rule are in Phase 5's **Unresolvable hops**.
- Phases 5, 7, and 9 gained `annotation.rs` **Files** rows; 5 and 9 also gained the `visibility/mod.rs` re-export row.
- **~~Rejected direction, do not relitigate~~ — SUPERSEDED by Phase 7.** Boundary acceptance shipped, and Phase 7 itself declares five items `pub(in crate::compiler)` (`policy.rs:309, 328, 335, 355, 361`) with the self-policy gate green. The original bullet, kept for history: widening `VisibilityReach` to `pub(in crate::compiler)` for the cross-module consumers. `record_forbidden_pub_in_crate` (`scan/record.rs:270-308`) flags every `InPath` spelling unconditionally until Phase 7 lands boundary acceptance, so that spelling breaks the self-policy gate for phases 5, 6, and 9. The plan now specifies this repo's existing pattern instead — bare `pub` on the item inside the private module plus a `pub(super) use` in that module's `mod.rs`, as `facade/exports.rs:31` + `facade/mod.rs:8` already do.
- Phases 5, 7, and 9 gained the derive facts: `VisibilityReach` is `Clone, Copy` only, so a struct holding one cannot derive `Debug`/`PartialEq`, and an equality test must be spelled `compare(other, tcx) == Some(Ordering::Equal)`.
- Phases 7 and 9 own the removal of the `#[expect(dead_code)]` on `mod annotation;` — whichever leaves no item unused must delete it, since clippy is deny-by-default here and a fulfilled expectation is itself an error. **Resolved: Phase 7 removed it.**
- Every phase's acceptance gate now runs cargo-mend on its own source. Phase 1 shipped `check`/`test`/`lint` green while the tool rejected its own new file, and both blind reviews missed it.
- Phase 5's `nearest` field changed from `Vec1<ParentFacadeOccurrence>` to `Vec` — `vec1` is not a dependency and appears nowhere in `src/`; non-emptiness is already guaranteed by the `Option<ParentFacadeAnalysis>` wrapper.
- Phase 12 now states the unnamed cost of making `Required` the default: ~51 sites in this repo use the bare-`pub`-behind-a-facade shape and would all need converting to keep the self-run green.
- Phase 11 now says in one line why it does not reuse the Phase 1 parser: `visibility_annotation_byte_len` measures bytes of raw source, `annotation.rs` classifies syntax with `syn` — different questions, and `fixes/` cannot see `pub(super)` items in `compiler/visibility/` anyway.
- Phases 2, 6, and 8 came through clean; they touch nothing Phase 1 changed.

---

### Phase 2 — Dynamic diagnostic headlines · status: done (`5590deb`)

#### Work Order

**Goal:** a diagnostic can take its headline from `finding.message` instead of a static string, in both the human and cargo-JSON renderers, with no duplicate note.

**Spec:**

Both forbidden-visibility diagnostics run at `DetailMode::None`, which suppresses the stored message and renders the diagnostic's static headline (`reporting/diagnostics.rs:82`). Phase 7's advice matrix needs per-outcome headlines, so replace `DiagnosticSpec::headline` with:

```rust
enum HeadlineSource {
    Static(&'static str),
    FindingMessage,
}
```

`finding_headline` (`reporting/diagnostics.rs:358`) returns the static string for `Static`, and clones `finding.message` for `FindingMessage`. The persisted schema already carries `message`, so **no** new field and **no** schema bump. Help continues to live in `suggestion`, which both renderers already display.

Set both forbidden-visibility specs (`ForbiddenPubCrate`, `ForbiddenPubInCrate` — the latter's spec arm is at `:334`) to `FindingMessage`. Every other spec keeps `Static` with its current text, so their output is byte-identical.

**Cargo JSON must not repeat the headline.** `rustc_diagnostic` (`reporting/cargo_json.rs:146`) copies `finding.message` into a note child and `render_diagnostic` (`:202`) repeats it in `rendered`, so a `FindingMessage` diagnostic would print its headline twice and the two output modes would disagree. Add a helper returning the message only when it is not the headline, and use it in both places.

Until Phase 7 populates the matrix, the two forbidden diagnostics keep emitting the message they emit today — so this phase changes the *mechanism*, not the text. Where a current `message` differs from the current static headline, preserve today's terminal text by setting the message to that headline.

**Files:**
- `src/reporting/diagnostics.rs` — `HeadlineSource` enum, `DiagnosticSpec::headline` field type, `finding_headline` (`:358`), the two forbidden spec arms (one at `:334`)
- `src/reporting/cargo_json.rs` — `rustc_diagnostic` (`:146`) and `render_diagnostic` (`:202`) message-vs-headline helper
- `src/reporting/render/diagnostic.rs`, `src/reporting/render/human.rs` — call sites if the `finding_headline` signature changes
- `tests/diagnostics/rendering.rs` — assertions that both forbidden codes render the expected headline and help with **no duplicate note**, in human and cargo-JSON output

**Constraints from prior phases:** Phase 1 added `src/compiler/visibility/annotation.rs`; nothing in this phase depends on it.

**Acceptance gate:** `verify.sh check`, `verify.sh test`, `verify.sh lint` green, **plus `bash ~/.claude/scripts/delegate/verify.sh test cargo-mend diagnostics` — the bare `verify.sh test` line runs only `--lib`/`--bins`, so every fixture under `tests/diagnostics/` is invisible to it and a phase whose only new tests live there would gate green having run none of them,** **and the self-policy gate: `RUSTC_BOOTSTRAP=1 cargo +stable run --release -- --workspace --all-targets --fail-on-warn` reports "No findings" on cargo-mend's own source.** That last check is not redundant with `lint` — Phase 1 shipped `check`/`test`/`lint` all green while the tool rejected its own new file, and two blind reviews missed it, because the only thing that knows cargo-mend's house rules is cargo-mend. The two rules that bite here are `inline_path_qualified_type` (write `use some::path::Foo;` and then `Foo`, never an inline `some::path::Foo` in a type position) and `imports_at_top`. Existing rendering tests pass unchanged for every `Static` diagnostic. New assertions in `tests/diagnostics/rendering.rs` prove `FindingMessage` reaches the terminal and cargo JSON emits no note duplicating the headline.

### Retrospective

**What worked:** The mechanism swap landed as specced — `HeadlineSource` on `DiagnosticSpec`, `finding_headline` matching on it, both forbidden specs switched, every other spec still `Static` with byte-identical text. The `finding_message_not_in_headline` helper covers all three consumers (`detail_reasons`, `rustc_diagnostic`, `render_diagnostic`) from one place.

**What deviated from the plan:** `FindingMessage` is not the bare unit variant the Spec drew — it carries `fallback: &'static str`, and each forbidden spec's fallback is its exact pre-phase static text. `finding_headline` uses it whenever `finding.message` is empty. This was added in fix pass 1: `StoredFinding` (`persistence/schema.rs:43`) accepts any `String` and `load.rs:208` copies it unvalidated, so a cached finding with an empty message would have rendered a blank `error:` line where the static text used to be.

**Surprises:**
- `verify.sh test <pkg>` with no target derives only `--lib`/`--bins` from cargo metadata (`~/.claude/scripts/delegate/verify.sh:95`), so it never runs `tests/diagnostics/`. This phase's 88 new fixture assertions were invisible to the gate as written. Every phase's Acceptance gate now carries the explicit `verify.sh test cargo-mend diagnostics` line.
- The blind reviewer called the removed duplicate `note` child a cargo-JSON regression. Refuted — suppressing that note is the phase's stated purpose; the note was an exact copy of the headline.
- Nothing in the original test additions would have failed if `FindingMessage` were reverted to `Static`; they asserted the old literal text, which the fallback also produces. The lib unit test at `diagnostics.rs:443` is what actually pins the mechanism, and it covers both forbidden codes.

**Implications for remaining phases:**
- Phase 7 must set a non-empty `message` on every forbidden finding it emits — that string is now the headline the user reads. An empty one silently degrades to the generic pre-phase text instead of the matrix's per-outcome advice.
- Any later phase adding a `FindingMessage` spec must supply a fallback literal and extend the loop in `diagnostics.rs:443` to cover it; that test is the only thing preventing a silent revert to a static headline.
- Phases that add fixtures under `tests/diagnostics/` must run the `diagnostics` target explicitly — the bare test line will report green having run none of them.

### Phase 2 Review

- **No remaining phase is redundant.** Phase 2 shipped a mechanism only; phases 3–12 each keep their full job. Phases 4, 6, 9, and 10 came through clean with respect to it.
- **Phase 3** gained three constraints: every forbidden finding it emits must set a non-empty `message` (it, not Phase 7, is the first phase whose gate asserts a per-outcome headline, and an empty message silently falls back to the generic text); the blocker location must go in `suggestion` because `related` is dropped for `DetailMode::None` specs; and per-outcome messages perturb `build_selection`'s sort/dedup key (`src/fixes/runner/execute.rs:92-121`), so fixture ordering churn is expected.
- **Phase 3 Files** gained three test-support rows: the test-side `Finding` (`tests/support/report.rs:7-19`) carries no headline field, `finding_from_compiler_message` (`tests/support/mend_json.rs:226-262`) never reads `message`, and `tests/support/diagnostics.rs:117-133` asserts a static headline for every code — all three block a headline assertion as written, and Phase 2 only avoided them by hand-parsing raw cargo JSON.
- **Phase 5** gained the fact that a `related` string on a forbidden finding never renders, so its consumer table's blocker path/line belongs in `suggestion`.
- **Phase 7 Files** now records that `HeadlineSource::FindingMessage` carries a `fallback: &'static str` (its Spec still draws the bare unit variant) and that the unit test at `src/reporting/diagnostics.rs:442-477` must grow a row per matrix outcome — that test is the only guard against a silent revert to a static headline. It also inherits Phase 3's three test-support rows.
- **Phase 8** gained a fifth CHANGELOG upgrade case: Phase 2 removed the `note` child duplicating the headline from cargo JSON, so consumers see one fewer child on every forbidden finding even when no finding changed.
- **Delegation Context** line refs for `src/reporting/diagnostics.rs` were corrected to post-Phase-2 positions and now record that `FixSupport` has **no** `Standard` variant. The `cargo_json.rs` refs were verified still exact. Phase 2's own Work Order keeps its pre-phase refs deliberately, as the archive record.
- **Deferred to Phase 7, and since resolved (2026-08-01):** the dynamic `suspicious_pub` suggestion collided with that spec's static `pub(super)` help, and the two renderers picked opposite winners while `rustc_diagnostic` emitted both. **Resolved in favor of deleting the static string** — `SUSPICIOUS_PUB.inline_help` goes to `None`, every suggestion becomes dynamic, both renderers resolve identically, and `rustc_diagnostic` emits one `help` child. Recorded in Phase 7's Work Order under **Resolved decisions**.
- **Deferred to Phase 11, since resolved by the user:** its Spec named `FixSupport::Standard`, which does not exist; adding a real variant touches the persisted `fixability` string and therefore the schema version. Resolved in favor of `FixSupport::RestrictedAnnotation`, a `21` → `22` schema bump, the three plumbed `Finding` fields kept internal with serde skip, and a named three-state enum replacing `Option<String>` for the annotation at the `StoredFinding → Finding` boundary. Folded into Phase 11's Work Order; the block is gone.
- **Partly closed since the Phase 1 review:** the `ForeignBoundary` decision is resolved (user, Option A — unresolvable chain, never advise `pub`). Phase 4's `extern crate` fixed-point fixture obligation remains open and is now load-bearing in both directions, since a mis-mapped subject either suppresses a real finding or lets a synthetic `Public` through.

---

### Phase 3 — Close the three detection holes · status: done (`aa87d94`)

#### Work Order

**Goal:** `pub(in crate)`, relative spellings, and restricted annotations on struct/union fields are detected and rejected; the item scan dispatches off the typed annotation instead of string prefixes.

**Spec:**

Current detection is `item.visibility_text.starts_with("pub(in crate::")` (`record.rs:220`) and `forbidden_pub_crate` matches the exact string `"pub(crate)"` (`record.rs:170`); fields reach neither. Three things leak:

- `pub(in super::super)` — legal Rust, compiles, flagged by nothing today (verified against rustc)
- `pub(in crate)` — equivalent to `pub(crate)`, flagged by nothing today
- any restricted annotation on a **struct or union field** — fields go through `visibility/field.rs:34`, which only checks whether a field is wider than its own type

Build `VisibilityAnnotation` once in `record_visibility_findings` (`record.rs:31`) and dispatch every `record_*` arm off it instead of raw string prefixes. `field.rs` builds the same value for fields and runs the shared rejection classifier **before** the field-specific check. Fields get the rejection half only — there is no acceptance path for them, because a field is not re-exportable, so no facade can justify one.

**Redundant spellings do not fold.** Folding `pub(in crate)` onto `PubCrate` before policy runs would make it *permitted* in the several locations where `record_forbidden_pub_crate` (`record.rs:164`) permits canonical `pub(crate)` — the reverse of the rule; and `pub(in super)`/`pub(in self)` fold onto forms with no diagnostic at all. Dispatch on the **written form**:

| Written | Behavior |
|---|---|
| `pub(crate)` | existing `pub(crate)` policy, unchanged |
| `pub(in crate)` | always `ForbiddenPubCrate`. Where `pub(crate)` is permitted, suggest canonical `pub(crate)`; where crate reach is forbidden, use the existing reach-based suggestion. |
| `pub(in self)` / `pub(in super)` | `ForbiddenPubInCrate`, suggesting `pub(self)` / `pub(super)` |
| `pub(in crate::a::b)` | rejected in this phase (acceptance arrives in Phase 7) |
| `pub(in super::super)` | rejected; suggest the `crate::`-rooted spelling when its resolved reach otherwise matches |

Crate-wide reach belongs to one diagnostic regardless of spelling — that is why `pub(in crate)` routes to `ForbiddenPubCrate` rather than to `ForbiddenPubInCrate`, which carries no suggestion and would send the author round twice.

**`pub(in ...)` is legal on declarations only.** A `use` line picks its own reach directly, and `pub(super)`/`pub(crate)`/`pub` span everything it can need; the only thing `pub(in ...)` adds there is a multi-level hop, which is the "item lives too deep" smell the original ban targets. `use` declarations **do** reach the item scan — `visit_item` (`visit.rs:19`) calls `record_visibility_findings` for every non-expanded HIR item — but `ItemInfo` (`visibility_context.rs:45`) distinguishes only module from non-module, so there is no discriminator. Replace `ItemCategory::NonModule` with `Declaration` and add `Use`, classified directly from `ItemKind::Use`.

**Reject once, then stop.** Once an annotation is rejected, do not also run `suspicious_pub`, the narrowing checks, `unused_pub`, or the field-specific check for it. `persistence/visibility_priority.rs:7` removes suspicious/narrow findings only when `unused_pub` exists, so post-persistence priority cannot clean up the overlap.

**All three start erroring at the default setting**, not gated behind any config and with no warning-only grace release: the ban was always the intent and the string-matching detection was the bug. Bump `FINDINGS_SCHEMA_VERSION` (`compiler/constants.rs:62`, currently `17`) in this phase — it is the first phase that changes emitted findings.

**Files:**
- `src/compiler/visibility/scan/record.rs` — build the annotation at `:31`; replace the `"pub(crate)"` match (`:170`) and the `starts_with("pub(in crate::")` hole (`:220`); reject-once ordering ahead of `:294`/`:345`/`:418` and the `unused_pub` path
- `src/compiler/visibility/scan/visibility_context.rs` — `ItemCategory::{Module, Declaration, Use}` (`:40`)
- `src/compiler/visibility/scan/visit.rs` — classify `ItemKind::Use` (`:19`)
- `src/compiler/visibility/field.rs` — build the annotation for fields and run the rejection classifier before the `field_declared.is_strictly_wider(type_declared, ctx.tcx)` check in `check_field` (`:100`). Phase 1 shrank this file 214 → 196 lines; the old `:34` reference now lands on a closing `) -> Result<()> {`. Current landmarks: `check_item` at `:30`, `effective_type_visibility` at `:140-148`, `use super::annotation::VisibilityReach;` at `:21`
- `src/compiler/visibility/annotation.rs` — read-only for this phase; the source of `VisibilityAnnotation::from_item`, `VisibilitySyntax`, and `PathSpelling`
- `src/compiler/constants.rs` — bump `FINDINGS_SCHEMA_VERSION` (`:62`)
- `tests/diagnostics/forbidden_pub_in_crate.rs` — **new suite**, declared in `tests/diagnostics/mod.rs`
- `tests/diagnostics/mod.rs` — declare the new module
- `tests/diagnostics/forbidden_pub_crate.rs` — `pub(in crate)` routes here
- `tests/diagnostics/field_visibility_wider_than_type.rs` — field rejection precedes the wider-than-type check
- `tests/support/report.rs` — the test-side `Finding` (`:7-19`) deserializes `code`/`path`/`item`/`fixability`/`help` and **carries no headline**, so this phase's headline assertions cannot be written against it as-is. Add `pub headline: String`.
- `tests/support/mend_json.rs` — `finding_from_compiler_message` (`:226-262`) never reads the diagnostic's `message`; populate the new field from `/message`. (Phase 2 sidestepped this by hand-parsing raw cargo JSON in `tests/diagnostics/rendering.rs:262-322` — do not repeat that workaround here.)
- `tests/support/diagnostics.rs` — mirrors `DiagnosticSpec` with a plain `&'static str` headline (`:75-113`), and `assert_rendered_diagnostics` (`:117-133`) loops `DiagnosticCode::ALL` asserting `rendered.contains(spec.headline)`. The moment a fixture makes either forbidden code emit a non-fallback message, that assertion and Phase 2's two helpers at `tests/diagnostics/rendering.rs:262` and `:324` (which hard-code the pre-phase literals) all break together.

**Constraints from prior phases:** Phase 1 provides `VisibilityAnnotation`, `PathSpelling`, and `VisibilityReach` in `src/compiler/visibility/annotation.rs`, with `is_at_least`/`visibility_strictly_wider` moved off `field.rs`. Phase 2 made both forbidden diagnostics take their headline from `finding.message`, so a per-outcome message set here reaches the terminal.

This phase — not Phase 7 — is the first to emit per-outcome headlines, so Phase 2's obligations land here first:

- **Every forbidden finding this phase emits must set a non-empty `message`.** `finding_headline` (`src/reporting/diagnostics.rs:374-385`) falls back to the generic pre-phase text when the message is empty, so an unset message makes this phase's own gate ("`pub(in crate)` errors with the headline quoting `pub(in crate)`") fail for a reason that looks nothing like the cause.
- **The blocker location has no rendering channel but `suggestion`.** Both forbidden specs are `DetailMode::None` (`diagnostics.rs:102`, `:111`), so `detail_reasons` returns empty and a `related` string on a forbidden finding is silently dropped; `finding_message_not_in_headline` (`:387-398`) additionally suppresses the message for any `FindingMessage` spec. Anything the author must see besides the headline goes in `suggestion`.
- **Per-outcome messages change fix selection.** `build_selection` (`src/fixes/runner/execute.rs:92-121`) sorts and dedups on a key that includes `message`, so distinct messages change intra-location ordering and stop two same-location findings from collapsing into one. Expect fixture ordering churn and do not read it as a regression.

**This phase is smaller than the Spec above implies — the parse-and-classify half already shipped.** `VisibilityAnnotation::from_item(source, target, tcx)` (`annotation.rs:50-56`) already does the whole `syn` parse and the eight-way written-form classification this phase's dispatch table needs. Do not rebuild it. What actually remains here: rewiring the `record_*` arms, adding `ItemCategory::{Module, Declaration, Use}`, the field path, reject-once ordering, and the schema bump.

Three shipped-API facts the Spec does not name:

- **The dispatch enum is `VisibilitySyntax`** (`annotation.rs:17-28`), a separate type returned by `syntax()` — nine variants mirroring the nine annotation forms, with `InPath(PathSpelling)` carrying the spelling. Match on it; do not re-derive the written form from the source string.
- **`from_item` returns `Option<Self>`** and yields `None` whenever `syn::parse_str::<syn::Visibility>` fails (`annotation.rs:108`, `:146`). Treat `None` as *skip this item silently* — that matches today's `starts_with` bail, which also just returns without a finding. It is not an internal error and must not panic or report.
- **`reach()` is not a free accessor.** It is `fn reach(&self, target: LocalDefId, tcx: TyCtxt<'_>) -> VisibilityReach` (`annotation.rs:86`), recomputing from the module tree for every non-`InPath` variant. Every dispatch site must therefore carry the item's `LocalDefId` and the `TyCtxt`. **For a field that `LocalDefId` is `field.def_id`, not the containing type's** — passing the type's silently anchors the field's reach to the wrong module.

**Acceptance gate:** `verify.sh check`, `verify.sh test`, `verify.sh lint` green, **plus `bash ~/.claude/scripts/delegate/verify.sh test cargo-mend diagnostics` — the bare `verify.sh test` line runs only `--lib`/`--bins`, so every fixture under `tests/diagnostics/` is invisible to it and a phase whose only new tests live there would gate green having run none of them,** **and the self-policy gate: `RUSTC_BOOTSTRAP=1 cargo +stable run --release -- --workspace --all-targets --fail-on-warn` reports "No findings" on cargo-mend's own source.** That last check is not redundant with `lint` — Phase 1 shipped `check`/`test`/`lint` all green while the tool rejected its own new file, and two blind reviews missed it, because the only thing that knows cargo-mend's house rules is cargo-mend. The two rules that bite new rustc-facing code are `inline_path_qualified_type` (write `use rustc_middle::ty::Foo;` and then `Foo`, never an inline `rustc_middle::ty::Foo` in a type position) and `imports_at_top`. New fixtures: `pub(in crate)` errors with the headline quoting `pub(in crate)` (not `pub(crate)`) and routes to `forbidden_pub_crate`; `pub(in super)` and `pub(in self)` error with `pub(super)`/`pub(self)` suggestions; `pub(in super::super)` errors; each of those four forms on a **struct field** errors with no facade-based acceptance; `pub(in ...)` on a `use` line errors. A rejected annotation asserts the **complete** diagnostic-code set, so a duplicate `suspicious_pub` or `narrow_to_pub_crate` alongside the error fails the test. A prior-schema-version report containing a pub-use fact is rejected rather than loaded.

### Retrospective

**What worked:** `VisibilityAnnotation::from_item` already carried the `syn` parse and the eight-way classification, so the phase was pure rewiring of the `record_*` arms — no parser work. The fixture asserts the **complete** per-file diagnostic-code vector, so a duplicate `suspicious_pub` or `narrow_to_pub_crate` alongside a rejection fails by construction rather than by a hand-written negative assertion.

**What deviated from the plan:** Four struct fields in `src/config/cli/fix.rs` and `src/config/cli/target.rs` changed from `pub(crate)` to bare `pub` — files outside the Work Order's **Files** list. The new field rule fired on cargo-mend's own source and failed the self-policy gate. Both reviews judged the edit justified and behavior-preserving: the containing structs remain `pub(crate)`, so effective reach is unchanged, and `target.rs` already used bare `pub` for its other fields — the two `pub(crate)` ones were the outliers.

**Surprises:**

- **The field path subjects canonical `pub(crate)` fields to the full existing `pub(crate)` location policy**, not only to the four `pub(in ...)` forms the Spec named. `check_field` (`field.rs:85-104`) passes `None` as `parent_facade_visibility`, and `VisibilitySyntax::Crate` routes into `record_forbidden_pub_crate` (`record.rs:216`), which returns `Ok(false)` only when `policy::allow_pub_crate_by_policy` already permits `pub(crate)` there. So the rule is narrower than "every `pub(crate)` field is now an error" but wider than the Spec's three holes. `pub(super)` and `pub(self)` fields remain allowed.
- **Reject-once made `forbidden_pub_crate` and `review_pub_mod` mutually exclusive.** A forbidden `pub(crate) mod` previously emitted both; it now emits only the rejection. Intended by the Spec, but a user-visible change in output shape.
- **`ItemCategory::Use` ships unconsumed.** It is constructed in `visit.rs` but matched nowhere — only `Module` is tested (`record.rs:115`, `:317`). Phase 7 is its first consumer.
- **`from_item` returning `None` is unreachable for source that compiled.** `from_source_and_reach` (`annotation.rs:103-116`) yields `None` only on a `syn` parse failure or on `(false, _, _)` in `from_restricted_source` (`:146`) — a multi-segment restricted path with no `in` token, e.g. `pub(crate::foo)`, which rustc rejects outright. The skip-silently bail is safe.

**Implications for remaining phases:**

- **Phase 7** consumes `ItemCategory::Use` as its declaration-vs-`use` discriminator; the variant exists and is populated.
- **Phase 8** needs two CHANGELOG cases beyond its current five: canonical `pub(crate)` struct fields now follow the existing location policy, and the reject-once dedup changes which codes co-occur.
- **Test gap carried forward:** no fixture covers a plain `pub(crate)` field — every field in `tests/diagnostics/forbidden_pub_in_crate.rs:35` is a `pub(in ...)` form. That path is exercised only by the self-policy run on cargo-mend's own source.

### Phase 3 Review

- **Delegation Context rewritten against shipped code.** The `scan/record.rs` line map was stale in every entry (the file is now 589 lines); `ItemCategory` is `{Module, Declaration, Use}`; `field.rs` is 217 lines; `FINDINGS_SCHEMA_VERSION` is `18` at both the key-files and invariant sites; and the "there is no `forbidden_pub_in_crate.rs`" note now records that Phase 3 created it. Phases 4, 5, 7, and 11 all dereference these.
- **Phase 4** — `src/compiler/visibility/field.rs` added to **Files**: `check_field` now builds a second `ItemInfo` literal carrying `impl_self_name`, so replacing that struct field breaks this file's compile. Set `facade_subject: field.def_id`.
- **Phase 5** — three additions. `scan/mod.rs` and `field.rs` added to **Files** as uncovered `ParentFacadeVisibility` call sites (`field.rs`'s hard-coded `None` is load-bearing and must be preserved). The `record.rs:45-50` facade-resolution match must widen to `InPath(_)`, or Phase 7's acceptance rule can never fire because it will always see "no facade". Stale `record.rs:216-222` citations corrected to `:270-308` at all four sites across Phases 1, 5, and 9.
- **Phase 7** — the largest correction set. Its Spec described the `visibility_text != "pub"` bails Phase 3 deleted (now `matches!(annotation.syntax(), VisibilitySyntax::Public)` guards at `:65/:77/:85/:114`) and restated `record_forbidden_pub_in_crate` with its pre-Phase-3 signature. Recorded: the phase *adds two parameters* rather than rewriting the function; the two `pub(in crate)` matrix rows land in `record_forbidden_pub_crate`, not `_pub_in_crate`; the `InPath(Relative)` suggestion already ships and only the `CrateRooted` arm remains; the three test-support rows are all done; and half the "Fix guards" paragraph is already satisfied.
- **Phase 7 — fields are not excluded by the `Declaration`-vs-`Use` test.** `check_field` sets `category: ItemCategory::Declaration`, so a field satisfies that predicate. Resolved in the Work Order rather than deferred: rely on `field.rs:102` passing `None` for the facade argument, and do **not** add an `ItemCategory::Field` variant — the behavior is already correct and a new variant would churn three files for no behavioral gain.
- **Phase 7 gate** gained three inherited obligations: update the nine `assert_headline_and_help` pairs Phase 3 pinned (leaving the order-sensitive `assert_codes` vectors alone); keep `tests/diagnostics/rendering.rs`'s all-16-findings fixture green by editing the fixture, not the assertion; and close Phase 3's carried-forward gap with a canonical `pub(crate)` field fixture.
- **Phase 8** — five CHANGELOG cases became seven: struct/union fields now follow the `pub(crate)` location policy, and reject-once changed which codes co-occur. A sixth README edit was added for the `forbidden-pub-crate` section, which still documents an item-only rule.
- **Phase 9** — **Constraints** now record that fields reach `has_signature_exposure_allowance` with a field name, so the `Option<VisibilityReach>` conversion must not silently change field behavior.
- **Phases 9 and 12** acceptance gates gained the `rendering.rs` all-16-findings rule.
- **Phase 12** — the ~51-site estimate is now flagged as a lower bound to re-derive at dispatch; Phase 3's four CLI field conversions added to it.
- **Phase 11** — corrected the `tests/support/diagnostics.rs` range: the `FixSupport` mirror is `:60-85`, and `:115-140` is the `HeadlineSource` block Phase 3 added.
- **Deferred to Phase 5, then resolved by the user pre-dispatch** — what an unresolvable-chain finding shows the user. The blocker string would have replaced the written-form repair advice rather than accompanying it, since `suggestion` is the only surviving channel and Phase 3 already occupies it. **Resolved: combine both into one `suggestion`, blocker first**; the composition rule and its wording constraints are in Phase 5's Spec.
- **Deferred to Phase 7, and since resolved (2026-08-01) — the phase is NOT split.** Phase 3 absorbed its dispatch skeleton, leaving five separable jobs under one number, and the recorded recommendation was to split into 7a (acceptance) and 7b (matrix). The user kept it whole: the evidence for splitting was Phases 4 and 5's fix-pass counts, and Phase 6 showed those traced to whole-diff blind review rather than phase size. Phases 8-11 keep their numbers; the Work Order now names the required order for the five jobs.

---

### Phase 4 — HIR re-export index and facade subjects · status: done

#### Work Order

**Goal:** facade identity resolves from active HIR rather than source text and filenames, covering `#[cfg]`, macro-generated re-exports, `#[path]` modules, raw identifiers, variants, and inherent impl items.

**Spec:**

`SourceCache` parses unexpanded source with `syn` and evaluates no attributes, so facade identity currently comes from raw text and filenames. Three failures:

**An inactive re-export still counts.** With `a/b/mod.rs` holding `pub(super) use c::helper;` and `a/mod.rs` holding `#[cfg(feature = "promote")] pub(crate) use b::helper;`, the correct annotation with that feature off is `pub(in crate::a)`. The source walk sees the outer line anyway, computes crate reach, and rejects it. Mirror case: an active macro-generated re-export is invisible, so a correct annotation looks unjustified.

**`#[path]` modules are searched under the filename.** `#[path = "odd.rs"] mod camera_panel;` is looked up as module `odd` (`rust_syntax.rs:74`).

**Inherent impl items can never be accepted.** `resolve_parent_facade_visibility` (`record.rs:384`) searches for a re-export named after the item — `bind`. A method is reached through its self type, and the facade re-exports `Widget`.

Generalize `public_reexport_targets` (`use_sites.rs:455`) into an active-HIR index keyed by **visibility subject** → re-export occurrence, retaining the use item's owner-module `DefId`, `tcx.local_visibility(use_def_id)`, use kind, alias, and span:

| Target | Subject |
|---|---|
| ordinary item | its own `DefId` |
| enum variant or variant constructor | containing enum `DefId` |
| inherent impl item | resolved self-type `DefId` |
| `extern crate <c> as <alias>` | the local `ExternCrate` declaration's `DefId` |

`extern crate` is visibility-bearing and can legally sit behind the facade, but its HIR re-export resolves to the *external* crate root rather than to the local declaration — map it back explicitly or the item reads as "no facade".

**A self-type hop does not carry every inherent item.** Each associated item has its own visibility, independent of the type's. This compiles:

```rust
mod a {
    mod b {
        mod c {
            pub(in crate::a) struct Widget;
            impl Widget {
                pub(in crate::a::b) fn inner_only() {}
            }
        }
        pub(super) use c::Widget;
    }
}
```

A naive subject mapping gives `inner_only` the `Widget` subject, computes `crate::a`, and demands the method widen — even though the facade exposes only the type that far. Count a self-type hop only when the associated item's own reach already reaches at least that hop's reach; stop before wider type-only hops.

**Globs need their own index.** A named import resolves to the imported item, so it populates `subject → occurrence` directly. A glob resolves to its *container* module or enum, not to each name passing through it — which is why the existing collector accepts only `UseKind::Single`. A subject-keyed index therefore cannot answer whether an active `child::*` blocks that subject, and falling back to `SourceCache` would reintroduce the inactive-`cfg` and macro-generated errors the index exists to remove. Build two indexes:

- named occurrences keyed by normalized visibility subject
- glob occurrences keyed by resolved container module or enum

At each ancestor, query named occurrences first; only when none match, query the direct child container for an active glob.

Replace `ItemInfo::impl_self_name: Option<String>` (`visibility_context.rs:53`, populated at `visit.rs:76/101/113/146`, consumed at `record.rs:314`) with `facade_subject: LocalDefId` — equal to `def_id` for ordinary declarations — which removes name-based lookup entirely. Keep `SourceCache` for line reporting, usage analysis, and auto-fix eligibility only.

Cost: one linear index pass per crate, memory proportional to re-export count, plus a memoized resolved self-type query per inherent impl item — replacing repeated name-based ancestor searches and a `String` allocation.

Also fix the wrong doc comment on `def_path_string` (`use_sites.rs:479`) if it has not already been corrected, and leave or delete the dead `PathAnchor::Crate` strip (`use_sites.rs:492`) — it is not evidence of a `crate::` prefix.

**Files:**
- `src/compiler/visibility/use_sites.rs` — generalize `public_reexport_targets` (`:455`) into the two indexes; subject normalization
- `src/compiler/visibility/scan/visibility_context.rs` — `ItemInfo::facade_subject: LocalDefId` replacing `impl_self_name` (`:53`)
- `src/compiler/visibility/scan/visit.rs` — populate `facade_subject` (`:76/:101/:113/:146`)
- `src/compiler/visibility/scan/record.rs` — consume `facade_subject` at `:384`; `resolve_parent_facade_visibility` (`:454`) stops searching by name
- `src/compiler/visibility/field.rs` — **Phase 3 added a second `ItemInfo` construction site here.** `check_field` builds a synthetic `ItemInfo` literal at `:93-102` that also carries `impl_self_name`, so replacing that struct field breaks this file's compile. Set `facade_subject: field.def_id` — a field is not re-exportable, so a self-subject lookup correctly finds no facade
- `src/rust_syntax.rs` — `module_name_for_child_boundary_file` (`:74`) no longer decides identity
- `tests/diagnostics/` — fixtures listed in the gate

**Constraints from prior phases:** Phases 1–3 landed `annotation.rs`, `HeadlineSource`, `ItemCategory::{Module, Declaration, Use}`, the three-hole rejections, reject-once ordering, and the schema bump to `18`. This phase changes facade *identity* only; chain resolution is Phase 5.

**A failed `extern crate` subject mapping does not error — it silently yields `pub`.** Phase 1's `ScopeReach::join` walks toward a common ancestor via a `parent_module` closure that returns any non-local `DefId` unchanged (`annotation.rs:172-176`, `:226-230`). A foreign boundary is therefore its own parent, the walk hits that fixed point immediately, and `join` returns `ScopeReach::Public`, which `to_source` renders as `"pub"`. That is sound (nothing is wider) but it is the *widest possible* suggestion, and it arrives with no error, no blocker, and no diagnostic. This phase is the first to route a non-local `DefId` into that path, which is exactly why the subject table maps `extern crate <c> as <alias>` to the local `ExternCrate` declaration's `DefId` rather than to its HIR re-export target (the external crate root). Get that mapping wrong and the tool confidently recommends `pub`.

Add to this phase's fixtures: the `extern crate` case asserts the computed reach is the **local** boundary and specifically **not** `pub`. Add one fixture that drives a genuinely non-local `DefId` through `join` under a real `TyCtxt` — Phase 1's fixed-point branch has only ever run against a hand-written test fixture (`TestModule::Isolated`, `annotation.rs:466-475`), so it is unverified that `tcx.parent_module_from_def_id(CRATE_DEF_ID)` returns `CRATE_DEF_ID` rather than panicking or walking off the local crate.

**Resolved decision — foreign boundaries are unresolvable, never advised (user, Option A).**

- **Problem.** Phase 7's advice matrix has **no public-boundary row**, on the stated argument that a real `pub use` facade makes `run_selection` return a `CargoCheck` failure on nonzero `cargo check` status *before* a report is loaded, so mend never renders a finding for it. That argument does not cover a *synthetic* `Public` produced by the fixed point above: it comes from code that compiles cleanly. A restricted annotation on such an item reaches the matrix with a required reach nothing matches, and no row fires.
- **What exists now.** `join` returns `ScopeReach::Public` at the fixed point (`annotation.rs:273-277`). Phase 5 already carries an `Unresolvable { blocker }` shape for chains it cannot resolve.
- **Decision.** Phase 5 adds `FacadeChainBlocker::ForeignBoundary` and the chain is reported unresolvable, naming the facade that left the crate. The tool emits **no** replacement annotation. Phase 7's matrix gains **no** public-boundary row — a synthetic `Public` never reaches the matrix, because Phase 5 stops it at the chain layer. Rejected: advising `pub`, on the grounds that a wrong widening recommendation is indistinguishable from a correct finding at the point the user reads it.
- **Scope note.** `extern crate <c> as <alias>` is the phase's *test case*, not the motivating case. Any non-local subject reaches the same fixed point, and the common spelling is `pub use <dependency>::Item;` — a re-export of a foreign item. This phase's subject mapping must therefore be correct for ordinary foreign `use` re-exports, not just for `extern crate`.

**Constraint this places on Phase 4:** the subject table's `extern crate` row (local `ExternCrate` declaration `DefId`, not the external crate root) is now load-bearing in both directions — mapping it to the crate root would route a *correctly local* item into the ForeignBoundary blocker and suppress a real finding, and failing to detect a genuinely foreign subject would let the synthetic `Public` through to the matrix. Fixtures must cover both directions.

**Acceptance gate:** `verify.sh check`, `verify.sh test`, `verify.sh lint` green, **plus `bash ~/.claude/scripts/delegate/verify.sh test cargo-mend diagnostics` — the bare `verify.sh test` line runs only `--lib`/`--bins`, so every fixture under `tests/diagnostics/` is invisible to it and a phase whose only new tests live there would gate green having run none of them,** **and the self-policy gate: `RUSTC_BOOTSTRAP=1 cargo +stable run --release -- --workspace --all-targets --fail-on-warn` reports "No findings" on cargo-mend's own source.** That last check is not redundant with `lint` — Phase 1 shipped `check`/`test`/`lint` all green while the tool rejected its own new file, and two blind reviews missed it, because the only thing that knows cargo-mend's house rules is cargo-mend. The two rules that bite new rustc-facing code are `inline_path_qualified_type` (write `use rustc_middle::ty::Foo;` and then `Foo`, never an inline `rustc_middle::ty::Foo` in a type position) and `imports_at_top`. Fixtures: inactive `#[cfg]` wider facade — the active inner boundary is used; active macro-generated facade — found; `#[path]` module; raw module identifier; `pub use self::...` (via `trim_leading_self`, `rust_syntax.rs:51`); variant re-export — subject maps to the containing enum; inherent method and inherent associated const behind a facade; an inherent method and associated const whose **type travels farther than they do** — no widening demanded; `extern crate <c> as <alias>` behind a facade — the declaration is matched; inactive-`cfg` glob and macro-generated glob alongside a named-beside-glob case. Existing `pub_use_fixes` and `allowances` suites still pass.

### Retrospective

**What worked:** the subject-keyed HIR index landed as specified — named occurrences keyed by normalized visibility subject, glob occurrences keyed by resolved container, named queried first at each ancestor. `ItemInfo::facade_subject: LocalDefId` removed name-based lookup entirely, including at the `field.rs` construction site Phase 3 added. The `extern crate` → local declaration mapping held in both directions.

**What deviated from the plan:** the phase needed **eleven fix passes and twelve blind reviews** (findings per round: 6, 6, 7, 3, 3, 3, 4, 2, 3, 1, 2, 0). The Work Order scoped identity correctly; what it did not anticipate is that moving identity to HIR **widened what the diagnostics can truthfully say**, and every rendered string that had been written against the old source-text model had to be re-audited. Rounds 9–11 were entirely about wording, not mechanics.

**Surprises:**

- **The fixture-depth trap.** `policy.rs:107` independently permits `pub(crate)` for items at logical depth two under private parents. Any diagnostics fixture at depth two therefore passes a `ForbiddenPubCrate`-absence assertion *regardless of the code under test*. Several fixtures across the run proved nothing until they were moved to depth three.
- **Resolved reach does not establish written syntax.** `pub(super)` and `pub(in super)` resolve identically; at the crate root a private `use` and an explicit `pub(crate) use` both resolve to `CRATE_DEF_ID`; expansion spans have no recoverable spelling at all. Any diagnostic that quotes a modifier back to the user must be gated on a *known, unconflicted* spelling, never on reach. This produced `ParentFacadeSpelling` + `spelling_conflict` and `ParentFacadeExportStatus::use_syntax() -> Option<&'static str>`, which yields the exact spelling when justified and `None` otherwise; consumers render the neutral word "re-export" on `None`.
- **Policy forbidding a modifier is not the compiler rejecting it.** The `ForbiddenPubCrate` help claimed "a narrower modifier would not compile", which is false — `pub(in crate::<grandparent>)` is narrower than `pub` and compiles. It is *this tool's policy* that forbids `pub(crate)` and `pub(in path)`. The same phrase was removed outright from the signature-exposure arm, where nothing established it.

**Implications for remaining phases:**

- Phase 5's chain resolution consumes facade identity from the HIR index, not from `SourceCache`. `SourceCache` now answers exactly three questions and no others: which line to report, whether an item is used at all, and whether a rewrite is mechanically applicable.
- Phase 7's advice matrix must not assert a source spelling it has not established. The gating primitive already exists (`use_syntax()` / `spelling_conflict`) and should be reused rather than re-derived.
- Three pre-existing defects were found and ruled out of scope (see **Phase 4 Review**); they touch the auto-fix path that Phase 11 extends.

### Phase 4 Review

Every remaining phase was re-read against what Phase 4 actually shipped. The findings below changed the plan; nothing was left as a review note.

**Delegation Context**

- Three new standing invariants: resolved reach never establishes written syntax (gate every modifier-quoting string on `ParentFacadeExportStatus::use_syntax()`, render "re-export" on `None`); policy forbidding a modifier is not the compiler rejecting it; and any fixture asserting the *absence* of `ForbiddenPubCrate` must sit at logical depth three or deeper, because `resolve_module_location` returns `ShallowPrivate` for depth one **and** two alike and `allow_pub_crate_by_policy` then permits `pub(crate)` independently — a depth-two fixture passes such an assertion regardless of the code under test. Phases 7, 9, and 12 all add fixtures of that shape.
- Four functions the plan cited by line no longer exist in `src/` (`parent_facade_has_glob_export`, `parent_boundary_has_matching_pub_use_glob`, `public_reexport_targets`, `resolve_parent_facade_visibility`); their replacements are named. Key-file rows for `record.rs`, `policy.rs`, `use_sites.rs`, `exports.rs`, `exposure/*`, the new `scan/mod.rs`, `facade/reference.rs`, `config/global.rs`, and the three `fixes/` files were refreshed.
- The production `HeadlineSource` variants are `Static` / `FindingMessage`; `Literal` is the test mirror's name and must never be written when describing `src/`.

**Phase 5** — shrinks to "keep climbing and join". `ReexportOccurrence.visibility` already *is* the per-hop reach table and `ReexportIndex::parent_facade_occurrences` already walks ancestors; the residual work is to stop returning on the first hit, accumulate, and rewire consumers. The type sketch now maps each role onto a type Phase 4 shipped instead of introducing a parallel shape. **`FacadeVisibility::widest` must not be the chain join** — it is an ordinal that drops `Unrecognized`, so at a boundary holding both `pub(super) use X;` and `pub(in crate::a) use X;` it selects the narrower one; replace it with `VisibilityReach::{compare, join}`. The per-item usage-scan count is worse than estimated and the caching site is `policy.rs:371`, not the two files previously named. The cross-module-visibility paragraph moved to Phase 9, which is the first genuine consumer outside `visibility/`. The unresolvable-chain rendering decision is **not** moot and stays open.

**Phase 7** — the `forbidden_pub_crate_suggestion` instruction was stale in three ways, one load-bearing: the function has three parameters now and **two** `Super` arms, and the spelling-gated split between them must survive the conversion to `String`. The "Restricted `use` blocks resolution" matrix row quoted a modifier the tool may not have read; it was deleted when Phase 5's pending decision resolved (the blocker it described no longer exists). The `VisibilityFindingContext` work is one parameter on one function, not a plumbing chain. The headline-guard test must extend to all three `FindingMessage` specs, not two. The split-the-phase decision gained evidence: Phase 4 was a comparable single-mechanism phase that took eleven fix passes, with the last three spent entirely on rendered wording — 7b's content.

**Phase 9** — its central premise is now false and it shrinks. The exposing item's `DefId` is no longer discarded; Phase 4 threaded `LocalDefId` through the whole exposure recursion. What remains source- and name-based is only *which* items count as exposing (`public_item_name` matches bare `pub` only). The wording fix this phase scheduled is already shipped, and the one surviving "would not compile" string is factually correct and deliberately spelling-gated — the instruction was deleted and replaced with an explicit do-not-touch note, since following it would have removed a true statement.

**Phase 11** — the Spec mis-described `unused_pub.rs` and following it as written was a behavior regression: that fixer uses a purpose-built parser that **deliberately** refuses `pub(`, and swapping in the permissive one would make `--fix` start stripping restricted annotations. Corrected to share the parser but keep an explicit bare-only gate. Two of the three carried-forward defects were recorded against paths that do not exist; their real sites are named and both now fold into this phase's blast radius. The third (two parent `use` declarations on one line selecting the wrong occurrence, `pub_use_fixes/parent_boundary.rs:95`) stays a standalone follow-up.

**Phases 6, 8, 12** — Phase 6 is fully self-contained and needed no body edit; only two Delegation Context line refs were off. Phase 8 gained a seventh README edit and four more CHANGELOG cases (seven → eleven), covering the dynamic facade wording plus three behavior changes from earlier phases. Phase 12's "~51 sites" lower bound was raised again — Phase 4 added roughly a dozen more of the exact shape `Required` converts.

**Deferred to the phase that owns them** (auto mode — surfaced at that phase's pre-dispatch check):

- *Phase 5* — does `FacadeChainBlocker::UnsupportedVisibility` survive at all? Its technical basis is gone: production parsing already resolves `pub(in crate::a) use` to a real restricted visibility, so such a hop is now joinable. Recommendation: delete it, leaving `Glob` and `ForeignBoundary`.
- *Phase 7* — where does the ordered no-facade caller classifier run? Not where the plan says: that use-site map lives in the persistence layer, which runs after finding text is built and can only retain or drop whole findings. Recommendation: run it in-pass off `sink.use_sites`, stating the lib-plus-bins limitation explicitly.

---

### Phase 5 — Semantic chain resolution · status: done

#### Work Order

**Goal:** the required boundary is the joined resolved visibility of the whole facade chain, computed once per item, with nearest-occurrence metadata kept separate and usage scanned at most once.

**Spec:**

**Phase 4 built most of the machinery this phase was written to build. Read this section as a statement of what already exists, then implement only the residual.** `ReexportIndex::parent_facade_occurrences` (`visibility/use_sites.rs:171-247`) already walks ancestors, already matches by `DefId` subject, and already handles globs inline (`:214-241`). Each `ReexportOccurrence` already carries `visibility: Visibility<DefId>` taken straight from `tcx.local_visibility(use_def_id).map_id(...)` (`:907-909`, `:924`) — **that is this phase's per-hop reach table, already resolved.** The residual work is: stop `return`ing on the first matching boundary, accumulate `VisibilityReach::join` across hops, add `FacadeChainResolution`, and rewire the consumers.

Re-exports stack. The walk climbs ancestors until a boundary re-exports the item, then stops — so it reports the *innermost* facade's visibility, which need not be the widest. With both of these present:

```
video_plane/plane/mod.rs   pub(super) use camera_panel::bind;
video_plane/mod.rs         pub(crate) use plane::bind;
```

a single-hop computation yields `pub(in crate::video_plane)`, which fails E0364 at the outer `pub(crate) use`. So the walk keeps climbing past its first hit.

**"Widest" cannot be decided by ranking `ParentFacadeVisibility`.** That enum's `Super` is relative to the module holding the `use`: a `Super` in `crate::a::b` reaches `crate::a`, a `Super` in `crate::a` reaches `crate`, and both are the same enum value. Ranking them emits a `Super → Super` suggestion that fails E0364. Convert every hop to a resolved visibility:

| Facade | Resolved reach |
|---|---|
| `pub use` | `Visibility::Public` |
| `pub(crate) use` | `Visibility::Restricted(CRATE_DEF_ID)` |
| `pub(super) use` in module `M` | `Visibility::Restricted(parent(M))` |

`M` is the use item's HIR owner module, so `mod.rs` and named-sibling layouts of the same module produce the same answer. Join the hops with `VisibilityReach::join`. Because `Restricted` already carries the boundary, there is no separate "widest-carrying facade module" to track and **no `parent_of` call at rendering time** — the resolved value *is* the required boundary. Phase 4 already does this conversion: `ReexportOccurrence.visibility` holds exactly the right-hand column.

**`FacadeVisibility::widest` (`visibility/use_sites.rs:106-113`) must NOT be the chain join.** It is an ordinal over an enum, not a reach comparison: it drops `Unrecognized` in favour of whatever the other operand is. At a boundary holding both `pub(super) use X;` and `pub(in crate::a) use X;`, `widest_applicable_occurrence` (`:307-333`) therefore selects the `Super` occurrence even though `crate::a` is strictly wider — and the legacy `#[cfg(test)]` `exports.rs:301-313` propagates `Unrecognized` instead, so the two disagree with each other. This is latent today only because the chain does not yet join across hops; the moment it does, leaving `widest` in place makes the chain under-report the required boundary and emit a suggestion that fails E0364. **Replace it with `VisibilityReach::{compare, join}` over `occurrence.visibility`, which already carries the resolved `Visibility<DefId>`.**

**Reuse the types Phase 4 shipped; do not introduce parallel shapes in `facade/exports.rs`.** The sketch below predates Phase 4 and is kept only to name the *roles*. The mapping:

| Sketch below | What already exists |
|---|---|
| `nearest: Vec<ParentFacadeOccurrence>` | `ParentFacadeOccurrences { selected, matching, spelling_conflict }` (`visibility/use_sites.rs:137-141`) |
| `ParentFacadeOccurrence.syntax: FacadeSyntax` | `ParentFacadeSpelling` + the `spelling_conflict` flag (`facade/exports.rs:51-64`) |
| the `{visibility, spelling, conflict}` triple | `ParentFacadeReach` (`facade/exports.rs:59-64`) |
| `ParentFacadeOccurrence.reach` | `ReexportOccurrence.visibility` (`use_sites.rs:907-909`) |

Only `FacadeChainResolution` and `FacadeChainBlocker` are genuinely new. Return `Option<ParentFacadeAnalysis>` — absence means no facade:

```rust
struct ParentFacadeAnalysis {
    nearest: Vec<ParentFacadeOccurrence>,   // non-empty; one per matching `use`
    chain:   FacadeChainResolution,
}

struct ParentFacadeOccurrence {
    syntax:      FacadeSyntax,   // Public | Crate | Parent — as written
    reach:       VisibilityReach,
    alias:       Option<Symbol>,
    fix_support: FixSupport,
    path:        PathBuf,
    line:        usize,
    usage:       OnceCell<FacadeUsage>,
}

enum FacadeChainResolution {
    Resolved { required: VisibilityReach },
    Unresolvable { blocker: FacadeChainBlocker },   // path, line, reason
}

enum FacadeChainBlocker { Glob, ForeignBoundary }
```

`nearest` is a collection because one boundary can hold several matching `use` declarations with different aliases, visibilities, and usage states — `duplicate_re_exports_take_widest_visibility` (`exports.rs:350`) already covers that. Collapsing them loses metadata each consumer needs, and picking one can miss the widest reach.

**Reach cannot replace written facade syntax.** In a crate-root child module, `pub(super) use child::Thing;` and `pub(crate) use child::Thing;` both resolve to `Restricted(CRATE_DEF_ID)`, but `assess_parent_facade_usage` (`policy.rs:289`) grants `InternalParentFacadeBoundary` only to the first. Retiring `ParentFacadeVisibility` is fine; dropping the spelling is not — hence `FacadeSyntax` on the occurrence.

Overwriting a nearest occurrence's `visibility` with the chain-widest value breaks two things concretely: `assess_parent_facade_usage` loses a used inner-`Super` allowance whenever an outer wider facade is unused; and `StoredPubUseFixFact` (`record.rs:578`) stores `parent_path = video_plane/plane/mod.rs` with `child_module = "camera_panel"` — substitute the outer `video_plane/mod.rs` and `child_module` stays `"camera_panel"` while that line reads `plane::bind`, so `fixes/pub_use_fixes/scan.rs:120` resolves `None` and silently skips the fix.

Per consumer:

| Consumer | Needs | On `Unresolvable` |
|---|---|---|
| `record_forbidden_pub_crate` | chain | error, no replacement annotation |
| `record_forbidden_pub_in_crate` | chain | reject with blocker path/line/reason |
| `maybe_record_narrow_to_pub_crate_nested` | chain | silent |
| `maybe_record_narrow_to_pub_crate` | neither — bare `pub` only | n/a |
| `assess_parent_facade_usage` | nearest; chain also under `Required` | preserve nearest used-`Super` behavior |
| stale-facade reporting | nearest usage/path/line/fix support | still valid |
| pub-use fixer | nearest + eligibility | emit no `StoredPubUseFixFact` |
| `parent_facade_exports_item` / `unused_pub` | active occurrence or matching glob | keep existing suppression |
| `type_is_exposed_outside_parent` (`detect.rs:315`) | nearest usage | unaffected |
| `ReviewInternalParentFacade` | nearest path/line | unaffected |

**Unresolvable hops.** A hop the walk cannot follow makes the chain unresolvable: no boundary is computed, no `pub(in ...)` suggestion is emitted, and the blocking facade is reported. Three cases:

*Glob-only re-exports* — `pub(super) use camera_panel::*;`. A glob does not force the declaration wider: verified against rustc, that line over a `pub(super) fn bind` compiles clean and the failure surfaces at the call site as E0603 rather than at the facade as E0364. A glob also states intent about a module rather than an item. A glob is a barrier **only when nothing else matches** — a named `pub(super) use c::helper;` stays resolvable even when the same `mod.rs` also contains `pub(super) use c::*;`. `parent_facade_has_glob_export` and `parent_boundary_has_matching_pub_use_glob` no longer exist — Phase 4 moved glob handling inside `ReexportIndex::parent_facade_occurrences` (`use_sites.rs:214-241`), where the item name *is* in scope. Existing `unused_pub` glob suppression rides `parent_facade_exports_item` (`record.rs:143`, defined `:472-475`) and is unchanged.

*(Resolved — there is no third blocker.)* An earlier draft made a facade line spelled `pub(in crate::a) use` a third blocker, `UnsupportedVisibility`, on the grounds that it could not be parsed. **Phase 4 removed that limitation and the variant is deleted; the chain follows such hops like any other.** Production facade parsing is `visibility/use_sites.rs:905-930`, which resolves `pub(in crate::a) use` to a real `Visibility::Restricted(scope)` on `ReexportOccurrence.visibility` — directly joinable by `VisibilityReach::join` with no new code. The `facade/exports.rs` functions the old analysis named are all `#[cfg(test)]` and read by nothing in production: `parent_facade_visibility` (`:343`), `exported_names_from_parent_boundary` (`:240`), `collect_matching_pub_use_exports` (`:267`), `widest_visibility` (`:301`). Do not reintroduce a policy-grounded refusal here: refusing to compute a boundary the tool can compute correctly produces a worse diagnostic than the one it replaces.

*Foreign boundary* — the chain reaches a subject that is not local to the crate under
analysis. Ordinary spelling is `pub use <dependency>::Item;`; `extern crate <c> as <alias>`
is the same case with an explicit declaration. Phase 1's `join` does **not** error here: its
`parent_module` closure returns a non-local `DefId` unchanged (`annotation.rs:172-176`,
`:226-230`), so a foreign subject is its own parent, the ancestor walk hits that fixed point
on its first step, `join` returns `ScopeReach::Public`, and `to_source` renders `"pub"`. That
is sound but it is the *widest possible* suggestion, arrived at by running out of crate rather
than by analysis, and it is indistinguishable from a real finding once rendered. Detect the
non-local subject **before** feeding it to `join` and return
`Unresolvable { blocker: ForeignBoundary }` naming the facade line that left the crate. Emit no
replacement annotation. This is what keeps a synthetic `Public` from reaching Phase 7's matrix,
which deliberately has no public-boundary row — see Phase 4's **Resolved decision**.

**Renames are resolvable.** `pub(super) use camera_panel::bind as attach;` is named, so the boundary is computable, and Phase 4's index resolves the alias and its visibility (`use_sites.rs:171-247`). Only the *auto-fix* is unavailable (`pub_use_is_fix_supported_with_prefix`, `exports.rs:319`). Classify a rename as resolvable, manual-fix-only.

**Resolve once, scan usage at most once. Phase 4 made this worse, and the caching site is not where this plan said it was.** `policy::parent_facade_export_status` (`visibility/policy.rs:371-422`) now loops over *every* occurrence in `occurrences.matching`, calling `facade::parent_facade_export_status` once per occurrence, and each of those calls `scan_facade_usage` (`facade/reference.rs:34`, called at `exports.rs:158-166`). The per-item scan count is therefore **O(matching occurrences) × O(resolution sites)**, not the three-to-five this plan estimated. `policy.rs:371` is the caching site — the counter-based performance gate must be expressed per-occurrence, not per-item. Every scan allocates a source-file vector and traverses package then workspace sources. Split structure from usage:

```rust
fn resolve_parent_facade(
    source_cache: &SourceCache,
    source_root:  &Path,
    child_file:   &Path,
    subject:      LocalDefId,
) -> Result<ParentFacadeAnalysis>;
```

`record_visibility_findings` resolves once and passes `&ParentFacadeAnalysis` to boundary-only checks; pass `&mut` only where usage may be requested, and cache the scan result in the occurrence's `OnceCell`. Signature-exposure work stays lazy behind facade and basic allowances.

**Usage scanning must carry crate identity.** `workspace_source_mentions_parent_export_literal` (`facade/reference.rs:115`) text-scans everything under `settings.config_root`. While analyzing member A, a line `crate::video_plane::bind` in member B matches A's module prefix even though `crate::` there means B — which can mark A's facade used and suppress a stale-facade warning. A restricted facade has no legitimate cross-crate callers, so do not count another crate's `crate::` literals. Prefer current-crate HIR use sites keyed by resolved target; if results are combined across targets, keep crate identity in the key.

Retire `ParentFacadeVisibility` (`exports.rs:31`) and update the `facade/mod.rs` re-export surface.

**Files:**
- `src/compiler/visibility/use_sites.rs` — **the primary file.** `ReexportIndex::parent_facade_occurrences` (`:171-247`) stops `return`ing on the first matching boundary and instead accumulates across hops; **delete `FacadeVisibility::widest` (`:106-113`) and rebuild `widest_applicable_occurrence` (`:307-333`) on `VisibilityReach::{compare, join}` over `occurrence.visibility`**; `FacadeChainResolution` / `FacadeChainBlocker` land here alongside `ParentFacadeOccurrences` (`:137-141`). The module already calls `super::annotation::VisibilityAnnotation::from_item` (`:1040`), so the reach algebra is in scope with no re-export work
- `src/compiler/facade/exports.rs` — keep `ParentFacadeReach` (`:59-64`), `ParentFacadeSpelling` (`:51-64`), `use_syntax()` (`:86-96`), and fix-support (`:319`). **Do not add parallel `ParentFacadeOccurrence`/`FacadeSyntax` shapes** — the mapping table in the Spec names the existing type for each role. `ParentFacadeVisibility` (`:44`) is retired, per the Spec
- `src/compiler/facade/mod.rs` — re-export surface (14 lines today)
- `src/compiler/facade/reference.rs` — `scan_facade_usage` (`:34`) called once per occurrence; crate identity in `workspace_source_mentions_parent_export_literal` (`:117`)
- `src/compiler/visibility/policy.rs` — **`parent_facade_export_status` (`:371-422`) is the caching site**: it currently calls `facade::parent_facade_export_status` once per entry in `occurrences.matching`, and each of those scans usage. `assess_parent_facade_usage` (`:290`) reads nearest occurrences, not the chain
- `src/compiler/visibility/scan/record.rs` — `resolve_parent_facade` once at `:34`; rewire every consumer in the table; `resolve_parent_facade_reach` (`:459-470`) becomes the new API. **Also widen the `match annotation.syntax()` at `:48-53`:** it resolves the facade only for `Crate | InCrate` and passes `None` for every `InPath`/`InParent`/`InCurrent`. Phase 7's acceptance rule compares an `InPath` reach against the chain reach, so unless `InPath(_)` also resolves the facade here it will always see "no facade" and acceptance can never fire
- `src/compiler/visibility/scan/mod.rs` — the `record_forbidden_visibility_annotation` wrapper at `:26-42`, whose parameter is already `parent_facade_reach: Option<ParentFacadeReach>` and becomes `Option<&ParentFacadeAnalysis>`
- `src/compiler/visibility/field.rs` — `check_field` calls that wrapper with a hard-coded `None` at `:103` (`facade_subject: field_def_id` at `:101`). **Preserve the `None`** — it is what keeps fields out of facade-based acceptance, since a field is not re-exportable
- `tests/diagnostics/` — chain fixtures listed in the gate. **Any fixture asserting the *absence* of `ForbiddenPubCrate` must place its subject at logical depth three or deeper** — see the fixture-depth invariant in the Delegation Context

**Constraints from prior phases:** Phase 1 supplies `VisibilityReach::{compare, join}` and the free fn `anchored`, all in `src/compiler/visibility/annotation.rs` and all `pub(super)`. **This phase needs no cross-module plumbing to reach them:** Phase 4 put the chain machinery in `visibility/use_sites.rs`, *inside* `visibility/`, already calling `super::annotation::VisibilityAnnotation::from_item` at `:1040`. If the join lands on `ReexportIndex` — which is where the ancestor walk already is — nothing needs re-exporting. (Phase 9 is the first consumer genuinely outside the subtree, since `src/compiler/exposure/` is a sibling; the `pub`-inside-private-module + `pub(super) use` pattern is documented there.) `VisibilityReach` derives only `Clone, Copy` — no `Debug`, no `PartialEq` — so `ParentFacadeOccurrence` cannot carry one and still `#[derive(Debug, Clone, PartialEq, Eq)]` the way its neighbor `ParentFacadeExportStatus` does (`compiler/facade/exports.rs:44`); either hand-write the impls, or keep the reach out of that struct and compute it at the comparison site. `VisibilityReach` deliberately has no `Ord`/`PartialOrd`: `compare` returns `Option<Ordering>` and `None` means two restricted scopes are genuinely incomparable siblings, which is not an error — feed such pairs to `join`, which returns their nearest common ancestor. Phase 4 supplies the two HIR indexes and `ItemInfo::facade_subject: LocalDefId`; facade lookup is by subject, never by item name. Phase 3 established reject-once ordering in `record_visibility_findings`. Phase 2 left the two forbidden diagnostics with **no rendering channel for a blocker location except `suggestion`**: both specs are `DetailMode::None` (`src/reporting/diagnostics.rs:102`, `:111`), so `detail_reasons` returns empty and a `related` string on a forbidden finding is silently dropped, and `finding_message_not_in_headline` (`:387-398`) suppresses the message for any `FindingMessage` spec regardless of whether it differs from the headline. The consumer table's "reject with blocker path/line/reason" therefore puts the path and line in `suggestion`, not in `related`. Phase 4 added two standing invariants this phase must honor — **resolved reach never establishes written syntax** (gate any modifier-quoting string on `ParentFacadeExportStatus::use_syntax()`, render "re-export" on `None`) and **policy forbidding a modifier is not the compiler rejecting it** — both stated in full in the Delegation Context. Phase 4 spent three of its eleven fix passes on violations of the first.

**Resolved decision (user, pre-dispatch):** `FacadeChainBlocker::UnsupportedVisibility` is **deleted**. A `pub(in crate::a) use` hop is joinable from its resolved scope, so the chain follows it; `Glob` and `ForeignBoundary` are the only blockers. Do not reintroduce a policy-grounded refusal for restricted `use` spellings.

**Resolved decision (user, pre-dispatch): an unresolvable-chain finding combines blocker and repair into one `suggestion`, blocker first.** `suggestion` is the only channel that survives on these two `DetailMode::None` specs (`src/reporting/diagnostics.rs:96-115`) — `related` is silently dropped — and `record_forbidden_pub_in_crate` (`scan/record.rs:279`, match at `:285-299`) already fills it with written-form repair advice. Do **not** move these specs off `DetailMode::None`; that is Phase 7's reporting work, not Phase 5's.

Compose the two parts at the single point that builds the string:

```rust
let suggestion = match (blocker_text, repair_text) {
    (Some(blocker), Some(repair)) => Some(format!("{blocker} — {repair}")),
    (Some(blocker), None)         => Some(blocker),
    (None,          repair)       => repair,
};
```

`repair_text` is exactly today's `match annotation.syntax()` arms, unchanged: `InParent` -> ``consider using: `pub(super)` ``, `InCurrent` -> ``consider using: `pub(self)` ``, `InPath(Relative)` -> the crate-rooted spelling from `annotation.reach(...)` (`:289-292`), `InPath(CrateRooted)` -> `None` (`:288`). `blocker_text` names the blocking re-export's path and line and its reason — for `Glob`, ``facade at <path>:<line> uses `*`; replace it with an explicit re-export``; for `ForeignBoundary`, that the chain leaves the crate at that line. **The glob wording may quote `*` because `FacadeUseKind::Glob` is a HIR use-kind, not a spelling; the foreign-boundary wording must not quote the facade's visibility modifier** — see the Delegation Context invariant on written syntax.

Because a finding with no blocker keeps its repair string byte-for-byte, **the ten existing `assert_headline_and_help` pairs in `tests/diagnostics/forbidden_pub_in_crate.rs` do not change.** Only new unresolvable-chain fixtures assert the combined form.

**Acceptance gate:** `verify.sh check`, `verify.sh test`, `verify.sh lint` green, **plus `bash ~/.claude/scripts/delegate/verify.sh test cargo-mend diagnostics` — the bare `verify.sh test` line runs only `--lib`/`--bins`, so every fixture under `tests/diagnostics/` is invisible to it and a phase whose only new tests live there would gate green having run none of them,** **and the self-policy gate: `RUSTC_BOOTSTRAP=1 cargo +stable run --release -- --workspace --all-targets --fail-on-warn` reports "No findings" on cargo-mend's own source.** That last check is not redundant with `lint` — Phase 1 shipped `check`/`test`/`lint` all green while the tool rejected its own new file, and two blind reviews missed it, because the only thing that knows cargo-mend's house rules is cargo-mend. The two rules that bite new rustc-facing code are `inline_path_qualified_type` (write `use rustc_middle::ty::Foo;` and then `Foo`, never an inline `rustc_middle::ty::Foo` in a type position) and `imports_at_top`. Fixtures: `Super → Super` chain resolving to a non-root restricted module — the computed boundary compiles; `Super → Crate` chain — the boundary is `pub(crate)`, not `pub(in crate::<inner parent>)`; direct-child facade whose canonical result is `pub(crate)`; the same owner module expressed as `mod.rs` and as a named sibling file; glob-only facade — chain unresolvable, blocker reported, existing `unused_pub` suppression still holds, **and the finding's `suggestion` asserts the combined blocker-first form from the Spec (blocker, ` — `, then the written-form repair) — plus one fixture on the `InPath(CrateRooted)` arm, where there is no repair text and the blocker stands alone**; named export beside a glob — still resolvable; rename facade — resolvable, boundary computed, auto-fix unavailable; facade line spelled `pub(in crate::a) use` — chain **resolvable**, the hop is joined from its resolved scope and the facade is **not** treated as `Public`; two same-subject re-exports at one boundary with different aliases, visibilities, and usage states; a crate-root child holding both `pub(super) use` and `pub(crate) use` of the same item — the `InternalParentFacadeBoundary` allowance still tracks spelling; a two-member workspace where both members contain the same module and item path but only one uses its own facade — no cross-crate usage match. Counter-based performance test keyed by `LocalDefId`, behind a non-default test-only Cargo feature: one structural resolution and one usage scan for a fixture hitting every check; one structural resolution and **zero** usage scans when boundary validation resolves the finding without usage; the `ReviewInternalParentFacade` branch increments neither again.

### Retrospective

**What worked:** The semantic chain walk, the blocker/repair composition, and the
counter-based performance test all landed as specified. The self-policy gate
(cargo-mend reporting `No findings.` against its own source) caught issues the
build and lint gates did not, as the Work Order predicted it would.

**What deviated from the plan:** The phase needed fourteen fix passes rather than
the one or two a phase normally takes. Passes 1–9 were spec work; 10–14 were
corrections to the textual usage matcher, where pass 12 introduced a regression
(a `//` inside a string read as a comment start) that pass 14 fixed with a proper
forward lexical state machine over code / string / raw string / char literal /
line comment / nesting block comment.

**Surprises:**
- **The fix-pass loop had no reachable exit.** Each pass ended with a fresh blind
  review over the *entire* 8137-line phase diff, prompted to find defects. A
  competent reviewer always returns something, so "review comes back clean" could
  never happen. The ten-item standing-decisions list handed to each reviewer is the
  accumulated record of refutations that kept being re-reported.
- **The textual matcher is the risky part of this phase, not the chain walk.** It
  decides usage for references the compiler cannot see (macro bodies), and a false
  "unused" invites a rewrite of a facade something depends on. Every pass from 10 on
  was about that asymmetry.
- **A finding being real does not make its obvious fix right.** Deferred defect 4
  below was confirmed, attempted, and reverted: the reviewer's proposed rule failed
  an existing fixture for a correct reason.

**Implications for remaining phases:**
- **Scope blind reviews to the incremental diff, not the whole phase.** Re-reviewing
  the full accumulated diff every pass is what produced the loop.
- Phase 7 owns moving the two unresolvable-chain specs off `DetailMode::None`; until
  then `suggestion` stays the only surviving channel and the combined blocker-first
  string stands.
- Deferred defects 5 and 6 both live in `src/compiler/facade/reference.rs`. Fix them
  together, and take a timing baseline first — none exists from before the lexer.

#### Deferred defects — carried out of Phase 5

Six findings from the pass-14 blind review. None produces wrong output on real
input, so none was fixed in Phase 5. Each is a standalone follow-up; none blocks a
later phase. Both `assert_eq!(…, 16)` gates in `tests/diagnostics/rendering.rs`
(`:404`, `:522`) constrain any fixture work here.

1. **Double negation in feature predicates** — `src/compiler/source_cache.rs:501`.
   Polarity is *set* to `Negated` rather than toggled, so `not(not(feature = "x"))`
   reads as negated. The trigger is source nobody writes, and it errs toward
   `AllFeaturesCoverage::NotGuaranteed`, the conservative direction.
2. **Mixed `cfg_attr` drops non-gating payload when an existing import is moved** —
   `src/fixes/imports/conditional_attributes.rs:44`, with the move site at
   `in_body_use_finder.rs:80`. Synthesis correctly keeps only gating attributes,
   but a *moved* import loses a payload such as `allow(unused_imports)`, which can
   break a `-D warnings` build. Needs a mixed payload plus a move plus denied
   warnings to bite.
3. **Expression-level `cfg` is not inherited** —
   `src/fixes/imports/inline_path_qualified_type/visitor.rs:235`. `ExprPath` is
   recorded without its attributes, so a gated expression yields an unconditional
   import. Same family as the Phase 5 fix that covered items and struct fields but
   not expressions.
4. **Ancestor globs over-claim** — `src/compiler/visibility/use_sites.rs`,
   `glob_containers` (`:456`) feeding `matching_glob_occurrences` (`:415`). Every
   module between the subject and `child_module` is registered as a glob container,
   but a glob re-exports only what is in its target module's namespace. The obvious
   correction — keep only the subject's own module — was tried and is **wrong**: in
   `pub use b::*` over a `b` that itself writes `pub use child::Thing;`, `Thing` is
   in `b`'s namespace and the glob does reach it. The correct predicate is whether
   the intermediate module re-exports the subject, answerable only from the
   re-export chain. Any fix must keep both
   `facade_subjects::ancestor_glob_targeting_descendant_module_is_a_blocker` and
   `facade_subjects::unused_named_facade_with_outer_glob_has_no_pub_use_fix`
   passing — they pin the two ends. Current behavior over-suppresses findings; it
   never produces a bad edit.
5. **Decomposed Unicode identifiers are missed** —
   `src/compiler/facade/reference.rs:466`. `char::is_alphanumeric` does not cover
   XID_Continue combining marks, so NFD `cafe\u{301}` fails the boundary check and
   the path is not matched. This fails in the dangerous direction — a false
   "unused" — but it is a remaining gap rather than a regression: Phase 5 is what
   added Unicode support at all.
6. **Literal matching is quadratic** — `src/compiler/facade/reference.rs:201` and
   `:286`. The lexical scan restarts at byte 0 for every `::` separator tested, so
   cost grows with (file length × separators). Fix is a per-file code/trivia mask
   computed once. **Record a timing baseline before changing the scanner** — none
   exists from before the lexer landed, so there is nothing to measure a regression
   against.

---

### Phase 6 — `pub_in_path` configuration · status: done

#### Work Order

**Goal:** a three-state `pub_in_path` setting resolves as project `mend.toml` > global > `Permitted`, is reconciled into the global config file, and participates in the config fingerprint.

**Spec:**

```rust
// src/config/pub_in_path.rs
pub(crate) enum PubInPath {
    Forbidden,      // pre-0.18 behavior: `pub(in ...)` errors everywhere
    Permitted,      // exact-boundary `pub(in ...)` accepted; `pub` also accepted
    Required,       // exact-boundary `pub(in ...)` accepted; `pub` fires suspicious_pub
}
```

```toml
[visibility]
pub_in_path = "permitted"   # default
```

`allow_prelude_pub_mod` (`docs/plans/prelude-pub-mod-exemption.md`, `src/config/prelude_pub_mod.rs`) supplies the mechanics — a field on `VisibilityConfig` (`config/loaded.rs:30`), a global-config key reconciled by the `toml_edit` pass, and inclusion in the fingerprint. It does **not** supply the ownership model: it is global-only *by design*, since `load_config` deliberately stamps the global value over the project-deserialized one (`config/loaded.rs:81-85`). A per-machine preference cannot pin a repo to `Required`, which Phase 12 requires — CI would fall back to the default and two developers on one commit could enforce different policies.

**Precedence: project `mend.toml` > global > `Permitted`.** Deserialize the project value as `Option<PubInPath>` so an absent key stays distinguishable from an explicit `"permitted"`; otherwise absence silently overrides a global `Forbidden`.

**`allow_prelude_pub_mod` moves to the same precedence.** Two keys in one `[visibility]` table where one honors the project file and the other silently discards it is not a distinction worth keeping — visibility policy belongs to the repo, not the machine. No compatibility handling is needed: no project `mend.toml` is deployed, so nothing depends on the discard behavior.

That needs two types, because one `VisibilityConfig` cannot be both the raw parse and the resolved answer:

- `ProjectVisibilityConfig { pub_in_path: Option<PubInPath>, prelude_pub_mod: Option<PreludePubMod>, .. }` — what `mend.toml` deserializes into
- `VisibilityConfig { pub_in_path: PubInPath, prelude_pub_mod: PreludePubMod, .. }` — what the driver receives

Fingerprint (`loaded.rs:107`) and serialize only the resolved config.

The precedent serializes its enum as a bool; this one needs three states, so it serializes as a string — a deliberate departure.

`reconcile_global_config` (`config/global.rs:79`) inserts the key with its default and a comment into the global file on the first run after upgrade, following the `PRELUDE_KEY` pattern (`config/constants.rs:10`):

```rust
if let Some(visibility) = ensure_table(doc.as_table_mut(), VISIBILITY_TABLE_KEY)
    && !visibility.contains_key(PUB_IN_PATH_KEY)
{
    visibility.insert(PUB_IN_PATH_KEY, value("permitted"));
    if let Some(mut key) = visibility.key_mut(PUB_IN_PATH_KEY) {
        key.leaf_decor_mut().set_prefix(PUB_IN_PATH_COMMENT);
    }
    inserted = true;
}
```

The project `mend.toml` is never written — absence there means inherit. The compiled-in `Permitted` is reachable only when `config_dir()` returns `None` or the global file is unparseable.

**Why `Permitted` is the default.** `Forbidden` and `Permitted` differ in exactly one cell:

| written behind a `pub(super) use` facade | `forbidden` | `permitted` | `required` |
|---|---|---|---|
| `pub(in crate::video_plane) fn bind()` | error | accepted | accepted |
| `pub fn bind()` | silent | silent | `suspicious_pub` |

Defaulting to `Forbidden` would mean an author who follows the README and writes the recommended annotation gets an error, then has to discover a config key by name to make the documented advice work. `Permitted` does not push anyone to convert existing `pub` annotations — that is `Required`, which stays opt-in. `Forbidden` remains available for a team that wants the flat ban.

**Files:**
- `src/config/pub_in_path.rs` — **new**: `PubInPath`, string serde
- `src/config/mod.rs` — `mod pub_in_path;`
- `src/config/loaded.rs` — `ProjectVisibilityConfig` vs `VisibilityConfig` (`:30`), resolution in `load_config` (`:47`, replacing the stamp at `:81-85`), `fingerprint_for` (`:107`)
- `src/config/prelude_pub_mod.rs` — move onto project>global
- `src/config/global.rs` — `reconcile_global_config` (`:79`)
- `src/config/constants.rs` — `PUB_IN_PATH_KEY`, its comment, `DEFAULT_GLOBAL_CONFIG_TOML`
- `tests/diagnostics/prelude_pub_mod.rs` — prelude precedence now project-first

**Constraints from prior phases:** Phases 1–5 landed the annotation type, dynamic headlines, the three-hole rejections (schema `18`), the HIR indexes, and `ParentFacadeAnalysis`. Nothing consumes `PubInPath` yet — Phase 7 is the first reader.

**Acceptance gate:** `verify.sh check`, `verify.sh test`, `verify.sh lint` green, **plus `bash ~/.claude/scripts/delegate/verify.sh test cargo-mend diagnostics` — the bare `verify.sh test` line runs only `--lib`/`--bins`, so every fixture under `tests/diagnostics/` is invisible to it and a phase whose only new tests live there would gate green having run none of them,** **and the self-policy gate: `RUSTC_BOOTSTRAP=1 cargo +stable run --release -- --workspace --all-targets --fail-on-warn` reports "No findings" on cargo-mend's own source.** That last check is not redundant with `lint` — Phase 1 shipped `check`/`test`/`lint` all green while the tool rejected its own new file, and two blind reviews missed it, because the only thing that knows cargo-mend's house rules is cargo-mend. The two rules that bite new rustc-facing code are `inline_path_qualified_type` (write `use rustc_middle::ty::Foo;` and then `Foo`, never an inline `rustc_middle::ty::Foo` in a type position) and `imports_at_top`. Config tests: the default is `Permitted`; a project `Required` overrides a global `Forbidden`; an absent project value inherits global; both absent yield `Permitted`; the three string values round-trip; the global reconcile inserts the key without disturbing existing comments or ordering; the resolved value changes the fingerprint. Existing prelude tests updated for the new precedence and passing.

### Retrospective

**What worked:**
- The `ProjectVisibilityConfig` / `VisibilityConfig` split landed exactly as specified. `ProjectVisibilityConfig::resolve(&GlobalConfig) -> VisibilityConfig` is the single resolution point; `load_config` calls it on both the found-`mend.toml` path and the no-`mend.toml` path (`ProjectVisibilityConfig::default().resolve(global)`), so there is no second place precedence can drift.
- Scoping the blind review to the incremental phase diff (~221 lines, six files) instead of the accumulated plan diff produced the run's first `No findings.` review. Phase 5 took 14 fix passes under whole-diff review.

**What deviated from the plan:**
- `src/config/prelude_pub_mod.rs` was **not** modified, though the Files list named it. Correct: the prelude precedence change lives entirely in `loaded.rs` (`Option<PreludePubMod>` + `unwrap_or(global.prelude_pub_mod)`); the enum's own definition needed nothing.
- `FINDINGS_SCHEMA_VERSION` stays `18`. This phase changes the config fingerprint, not the persisted findings shape, and `load.rs:166-168` already rejects a stored report on config-fingerprint mismatch — so the fingerprint change alone invalidates stale reports.

**Surprises:**
- Moving `allow_prelude_pub_mod` onto project-first precedence broke a diagnostics fixture in a way the plan did not predict: `crate_root_prelude_is_not_flagged_without_override` wrote no `mend.toml`, so under the new rule it inherits **the developer's real machine-global config** and its result depends on who runs it. Both prelude fixtures now write an explicit project `mend.toml` (`= true` and `= false`), which also gained free coverage of the new precedence in an end-to-end test. **Any future diagnostics fixture that depends on a `[visibility]` setting must write its own `mend.toml`** — inheriting global is no longer deterministic under test.
- `GlobalVisibility.pub_in_path` correctly keeps `#[serde(default)]` while the project layer uses `Option<_>`. The asymmetry is the design: absence at the global layer *should* fall to the compiled-in `Permitted`, and only absence at the project layer needs to stay distinguishable.

**Implications for remaining phases:**
- Phase 7 reads `LoadedConfig.visibility_config.pub_in_path` — already resolved, always a concrete `PubInPath`, never an `Option`. No consumer ever sees the project/global layering.
- Phase 12 (`required` mode) needs no further config work; the three-state key and its precedence are complete and a repo can now pin `required` in its own `mend.toml`.
- Any later phase adding a `[visibility]` key follows this shape: `Option<_>` on `ProjectVisibilityConfig`, plain value on `VisibilityConfig`, a line in `resolve`, a `reconcile_global_config` block, and an entry in `default_global_config_toml`.

### Phase 6 Review

- **New Delegation Context invariant — fixtures must pin their own `[visibility]` settings.** Once Phase 7 reads `pub_in_path`, any diagnostics fixture without its own `mend.toml` inherits the developer's real machine-global config. Neither `forbidden_pub_in_crate.rs` nor the all-diagnostics fixture in `rendering.rs` writes one today, so a developer whose global config says `"required"` would break both `assert_eq!(…, 16)` gates on unchanged source. Phase 7 owns pinning the pre-existing fixtures; Phases 9–12 must pin every fixture they add.
- **Phase 7 gained the `PubInPath` re-export.** Phase 6 added only `mod pub_in_path;`, so `crate::config::PubInPath` is still E0603. `src/config/mod.rs` is now in Phase 7's Files with the exact line needed.
- **Phase 7's config access path corrected.** `LoadedConfig` never crosses into the driver — the value arrives via the `CONFIG_JSON_ENV` round-trip as `ctx.settings.visibility_config.pub_in_path`.
- **Phase 7 picked up a one-line production comment fix** at `record.rs:354`, which still describes the prelude switch as global-only. Phase 8 is documentation-only and touches no `src/` file, so it could not carry it.
- **Phase 8 gained a sixth README item and a twelfth upgrade case** for the `allow_prelude_pub_mod` precedence change, including README `:102-103`, which sits outside its previous edit range.
- **Phase 12's default-flip branch gained its real Files list** — five config sites, plus the recorded fact that reconcile writes `"permitted"` onto every upgraded user's disk and would override a flipped default.
- **Stale line refs corrected across the doc.** Phase 5 landed 39 files and never got a review pass, so `record.rs` (now 671 lines), `policy.rs` (753), and `rendering.rs` (1543) had drifted by up to 80 lines in Phases 7, 9, 11, and 12. Two specifics that would have caused real errors: the `suggestion: None` Phase 7 cites as one site is now **two** (`record.rs:608` and `:634`), and `rendering.rs` has **two** 16-findings assertions (`:404`, `:522`) where three gates said one (`:399`).
- **Phase 7's split-the-phase pending decision gained a changed input** (the decision itself stays open for the user): the fix-pass explosions it cites as evidence trace to whole-diff blind review, not phase size — Phase 6 scoped review to the incremental diff and came back clean on the first pass.
- No remaining phase is redundant or already satisfied by Phase 6; Phase 6 was config-only and each of Phases 7–12 keeps its full job.

---

### Phase 7 — Acceptance and the advice matrix · status: done (`c9b0f62`)

#### Work Order

**Goal:** an exact-boundary `pub(in ...)` on a declaration is accepted at `permitted`/`required`, every rejection carries outcome-specific advice, and `suspicious_pub` is the only secondary check that admits an accepted annotation.

**Spec:**

`pub(in <path>)` on a declaration is accepted when `<path>` resolves to the **required boundary** — the chain's joined reach:

| Chain reach | Required boundary | Accepted declaration |
|---|---|---|
| `Restricted(M)`, `M` not the crate root | `M` | `pub(in <path of M>)` |
| `Restricted(CRATE_DEF_ID)` | crate | `pub(crate)` (existing policy; `pub(in crate)` stays rejected as a redundant spelling) |
| `Public` | crate-external | `pub` (unchanged) |
| unresolvable | not computed | no `pub(in ...)` suggestion; blocker reported |
| no facade | — | `pub(super)` (unchanged) |

Only the first row is new. Still rejected: any `pub(in <path>)` where `<path>` is not the required boundary (wider, narrower, or unrelated); `pub(in ...)` on a `use` line at any setting; `pub(in crate)`/`pub(in self)`/`pub(in super)`; relative spellings of an otherwise-correct boundary — `pub(in super::super)` when `pub(in crate::video_plane)` names the same module, since one canonical `crate::`-rooted spelling means a module move becomes a compile error instead of a quiet change in reach; `pub(in <path>)` with no parent facade; `pub(in <path>)` on a struct or union field.

`record_forbidden_pub_in_crate` — **now at `record.rs:279-317`, signature `fn record_forbidden_pub_in_crate(ctx, item, annotation: &VisibilityAnnotation<'_>, sink) -> Result<bool>` (`:279-284`).** It already matches on `VisibilitySyntax::{InParent, InCurrent, InPath(_)}`, returning `Ok(false)` for every other form. This phase *adds two parameters* — the `ParentFacadeAnalysis` and the `VisibilityFindingContext` the no-facade classifier needs. **The `VisibilityFindingContext` half is one parameter on one function and one argument at one call site — not a plumbing chain.** `record_visibility_findings` already computes `finding_context` at `record.rs:47` and already passes `&finding_context` into `record_forbidden_visibility_annotation` (`:55-62`, signature `:194-201`); `scan/mod.rs:26-42` computes its own. Only `record_forbidden_pub_in_crate` lacks it; the call site to update is `record.rs:212`. The recorder then becomes: fire unless `pub_in_path` permits the form **and** the annotation's reach equals the chain's required reach **and** the item is a declaration rather than a `use` line. Keep `DiagnosticCode::ForbiddenPubInCrate` and its `forbidden-pub-in-crate` anchor.

**Fields are not excluded by the `Declaration`-vs-`Use` test — do not rely on it for them.** `check_field` sets `category: ItemCategory::Declaration`, so a field satisfies "is a `Declaration`". What keeps fields out of acceptance is that `check_field` passes `None` for the facade argument (`field.rs:103`, with `facade_subject: field_def_id` at `:101`) and a field is not re-exportable, so no chain can ever justify one. **Decision for this phase: rely on that `None`, do not add an `ItemCategory::Field` variant** — the behavior is already correct and a new variant would churn `visibility_context.rs`, `visit.rs`, and `field.rs` for no behavioral gain. State the reliance in a comment at the acceptance site so it is not silently broken later.

**Two matrix rows land in the other recorder.** The `pub(in crate)` rows ("redundant spelling", "too-wide path") are **not** handled by `record_forbidden_pub_in_crate`: Phase 3 routes `VisibilitySyntax::InCrate` to `DiagnosticCode::ForbiddenPubCrate` via `record_forbidden_pub_crate` (`record.rs:221`), which already ships a permitted-`InCrate` branch emitting ``consider using: `pub(crate)` `` (`record.rs:246-248`). Those two rows are edits to `record_forbidden_pub_crate`, and that existing branch must survive.

**Advice matrix.** Boundary mismatch takes priority over redundant-spelling advice, so one run yields the final valid replacement.

| Outcome | Headline | Help |
|---|---|---|
| Exact restricted boundary | ``use of `pub(crate)` does not match the parent facade boundary`` | ``consider using: `pub(in crate::video_plane)` `` |
| Exact annotation already written, `Permitted`/`Required` | *(no finding)* | — |
| Exact annotation under `Forbidden` | ``use of `pub(in crate::video_plane)` is disabled by project visibility policy`` | ``consider using: `pub`; or set `pub_in_path = "permitted"` `` |
| Crate boundary | ``parent facade caps reach at `pub(crate)` `` | ``consider using: `pub(crate)` `` |
| No facade, all callers in the defining module | ``use of `pub(in crate::a)` outside an exact facade boundary is forbidden by policy`` | `consider removing the visibility` |
| No facade, all callers within the parent scope | *(same)* | ``consider using: `pub(super)` `` |
| Glob blocks resolution | `parent facade does not provide a resolvable visibility boundary` | ``facade at <path>:<line> uses `*`; replace it with an explicit re-export before using `pub(in ...)` `` |
| Redundant spelling, canonical reach valid | ``` `pub(in crate)` is a redundant spelling of `pub(crate)` ``` | ``consider using: `pub(crate)` `` (analogous for `self`/`super`) |
| Too-wide path | ``` `pub(in crate)` is wider than the exact parent facade boundary ``` | ``consider using: `pub(in crate::video_plane)` `` |
| No visibility-only rewrite compiles | `no visibility annotation allowed by policy preserves this item's current callers` | ``move the item into `crate::a`, or add an explicit facade at `crate::a` and rerun `cargo mend` `` |

**There is no "Restricted `use` blocks resolution" row.** It was deleted when Phase 5's pending decision resolved: `FacadeChainBlocker::UnsupportedVisibility` is gone, a `pub(in crate::a) use` hop is joinable, and no advice row is needed for it. Do not add one back — resolved reach alone never establishes that a facade was *written* `pub(in ...)`, so any such row would have to be gated on `ParentFacadeExportStatus::use_syntax()` (`facade/exports.rs:86-96`) returning `Some`, and quoting a modifier the tool did not read is exactly the class of defect that consumed three of Phase 4's eleven fix passes. The glob row above is safe as written: `FacadeUseKind::Glob` is a HIR use-kind, not a spelling.

There is **no public-boundary row**. A declaration narrower than a bare `pub use` facade fails rustc's re-export check, and `run_selection` (`compiler/build/execute.rs:103-107`) returns `CargoCheck` failure on any nonzero `cargo check` status *before* a report is loaded — so mend never renders a finding for it. An already-`pub` declaration behind that facade needs no forbidden-visibility diagnostic either. `Public` remains a legal chain state that produces no advice row.

The last row is real. Verified against rustc:

```rust
mod a {
    mod b {
        pub(super) mod c {
            pub(in crate::a) fn helper() {}
        }
    }
    pub fn caller() { b::c::helper(); }
}
```

`pub(super)` on `helper` gives E0603 at `caller`; `pub(crate)` passes privacy checking but is broader and stays forbidden at this nested location; bare `pub` removes the ceiling. The exact annotation is the only one that both compiles and preserves the ceiling — and there is no facade, so it is rejected. For that outcome, emit **no** annotation replacement; name the two structural migrations instead.

**Selecting among the three no-facade rows.** They are distinguished by caller location, which the matrix cannot read off the annotation. Run an ordered classifier after the cross-target caller union is built:

1. every caller inside the item's own module → suggest removing the annotation
2. every caller within the parent scope → suggest `pub(super)`
3. otherwise → the structural-migration row, with no annotation replacement

Order matters: without it, an item whose callers sit above its parent matches the ordinary row and gets advice producing E0603 — worse than saying nothing.

**Resolved decision — the classifier runs in both places, off one shared function.** The persistence layer cannot build the suggestion (`apply_caller_aware_suppression` only `retain`s findings; it never rewrites one) and the compiler pass cannot see the cross-target caller union (`sink.use_sites` covers the current crate target only, so a caller in a sibling bin is invisible to a lib's pass). Resolve it by writing the branch selection **once**, as a pure function over data that is already persisted as plain strings:

```rust
fn classify_no_facade_callers(
    item_module: &str,      // the item's own module def path
    parent_scope: &str,     // its parent module def path
    callers: &BTreeSet<String>,   // caller module def paths
) -> NoFacadeAdvice          // { RemoveAnnotation, SuggestPubSuper, StructuralMigration }
```

Call it twice:

1. **In-pass**, from `record.rs`, with the current target's callers from `sink.use_sites` — this is what composes the suggestion text and covers every single-target package correctly.
2. **At load**, as a fourth pass in the existing chain at `persistence/load.rs:81-83` (beside `apply_cross_compilation_intersection`, `apply_caller_aware_suppression`, and `apply_visibility_narrowing_priority`), with the cross-target union `apply_caller_aware_suppression` already builds. Recompute the branch; overwrite the finding's `suggestion` **only when the union changes the answer**. That layer already exists to reconcile facts that are only true once every target's report is present — this is one more entry in it, not a new boundary.

**The refinement is monotone and therefore safe.** The union can only *add* callers, and adding callers only moves the branch toward wider reach: `RemoveAnnotation` → `SuggestPubSuper` → `StructuralMigration`, never the reverse. So the load pass can only ever retract a suggestion that would have produced E0603 in a sibling target; it can never introduce one. A single-target run gets the same answer twice and the pass is a no-op.

The load pass must be code-gated (it touches only this diagnostic's findings). The original claim that it could leave `message` alone was false: `NoFacadeAdvice::StructuralMigration` has a distinct headline because no visibility-only repair works. Phase 8 owns the shipped follow-up: both the in-pass recorder and the cross-target refinement use one shared headline selector, and the refinement updates `message` alongside `suggestion` whenever the combined caller set reaches that branch.

**`forbidden_pub_crate_suggestion`** (`policy.rs:215`) — **stale in three ways, one load-bearing:**

1. It is a **three-parameter** function today: `const fn(ModuleLocation, SignatureExposure, Option<ParentFacadeReach>) -> &'static str`.
2. There are **two** `Super`-facade arms, not one. `:224-234` fires only when `spelling == ParentFacadeSpelling::Super && !spelling_conflict`, quotes `` `pub(super) use` ``, and carries the E0364 compile claim — that claim is correct precisely *because* the spelling is known and unconflicted. `:235-244` is the neutral fallback, which names no modifier at all.
3. The `SignatureExposure::Present` arm (`:221-223`) now reads ``"this item is exposed through a public signature; consider using `pub`"`` — Phase 4 removed the compile claim it used to carry.

So the instruction is: **both** `Super` arms gain the boundary path, and **the spelling-gated split must survive the conversion to `String`** — collapsing the two arms would either drop a true compile claim or attach it to a facade whose spelling is unknown. Return type changes from `const fn -> &'static str` to `-> String`; the caller already does `.to_string()`, so it allocates unconditionally today and simply drops that call. The unit tests (starting `policy.rs:490`) compile unchanged, since `assert_eq!` between `String` and `&str` works. `const fn` is lost either way — the function gains the boundary path as a parameter. `forbidden_pub_crate_help` (`policy.rs:178`) stays `const fn -> &'static str` and is converted in the fallback arm.

**`suspicious_pub` is the only secondary check that accepts `pub(in ...)`.** Phase 3 deleted the `item.visibility_text != "pub"` string comparisons; each secondary check is now gated on `matches!(annotation.syntax(), VisibilitySyntax::Public)`. Admitting `suspicious_pub` for an accepted `pub(in ...)` therefore means **widening that check's guard**, not removing a string comparison. Note also that these checks are only reached at all when the written-form dispatcher returns `false` (reject-once, `record.rs:55-62`), so an accepted annotation must fall through the dispatcher for any of them to run.

- `narrow_to_pub_crate` must not: both implementations suggest and auto-fix to `pub(crate)`, which is *broader* than an exact boundary — a "narrow" fix that widens and drops the ceiling. An accepted `pub(in ...)` behind a `Super` facade can never satisfy the nested check's `Crate` condition anyway.
- `unused_pub` cannot reach it: acceptance requires a facade, and the check already bails whenever a facade exports the item (`parent_facade_exports_item`, called `record.rs:143`) — correctly, since removing the annotation would break the re-export with E0364.

That leaves `suspicious_pub` (`maybe_record_suspicious_pub` at `record.rs:486`, `classify_suspicious_pub` at `policy.rs:35`) with a real job — the facade exists but nobody uses it, so both the facade and the annotation are dead:

| Case | Result |
|---|---|
| `Permitted` + bare `pub` + used exact facade | allowed |
| `Required` + bare `pub` + exact restricted facade | `suspicious_pub`, dynamic suggestion |
| exact `pub(in ...)` + used facade | allowed at both accepting settings |
| exact `pub(in ...)` + unused facade | stale-facade warning: remove the facade and the now-unneeded annotation |
| rejected `pub(in ...)` | no suspicious check |

The earlier parent-public and effective-public allowances were written for bare `pub` and must not return first for a restricted annotation, or the stale-facade branch never runs. `assess_parent_facade_usage` reads **nearest** occurrence data, not the chain — that is what preserves the used-inner-`Super` allowance.

**Fix guards.** `fixes/unused_pub.rs` and `fixes/narrow_pub_crate.rs` both reject anything that is not a bare `pub` (by different mechanisms — see Phase 11), so any restricted-annotation finding marked auto-fixable produces no edit while `cargo mend --fix` reports success. `FixSupport::None` alone is **not** a sufficient guard: standard fix scans route by diagnostic code, but the pub-use fixer routes from *stored facts*. Today an unused facade gets `FixSupport::PubUse` and `record.rs:578` writes a `StoredPubUseFixFact`; `screen_candidate` (`fixes/pub_use_fixes/scan.rs:183`) then rejects the child as `AlreadyNarrowed`. So a stale-facade finding on an accepted restricted annotation must both carry `FixSupport::None` **and** write no `StoredPubUseFixFact`. **Phase 3 already satisfies the rest of this paragraph — do not re-audit it:** both rejection recorders ship `fix_support: FixSupport::None` with no `StoredPubUseFixFact` write, and field rejection already returns before `field_visibility_wider_than_type` can record (`field.rs:103`). The remaining guard work here is only the stale-facade / accepted-annotation case.

Signature exposure keeps its current arm ordering (`(Present, _)` first) until Phase 9: it suggests `pub`, which is wider than necessary but always compiles.

**Files:**
- `src/compiler/visibility/scan/record.rs` — narrowed `record_forbidden_pub_in_crate` (`:279-317`, gaining two parameters), the `pub(in crate)` matrix rows in `record_forbidden_pub_crate` (`:221`, permitted-`InCrate` branch at `:246-248`), the dispatcher arm at `:211-213` (call site `:212`), advice-matrix dispatch, no-facade classifier, `suspicious_pub` admission (recorder at `:486`), `StoredPubUseFixFact` guard (`:578`). **The `suggestion` match at `:285-299` is partly done:** the `InPath(PathSpelling::Relative)` arm already emits ``consider using: `{annotation.reach(item.def_id, ctx.tcx).to_source(ctx.tcx)}` `` (`:289-292`), which is exactly this phase's "relative spelling of an otherwise-correct boundary -> suggest the `crate::`-rooted spelling" row. **Only the `InPath(PathSpelling::CrateRooted)` arm still yields `None`** (`:288`) — fill that one, and do not regress the `Relative` arm (pinned by the fixture at `tests/diagnostics/forbidden_pub_in_crate.rs:95-100`)
- `src/compiler/visibility/scan/mod.rs` — the `pub(super)` wrapper at `:26-42` forwards the new parameters
- `src/compiler/visibility/policy.rs` — `forbidden_pub_crate_suggestion` → `String` (`:215`, **both** `Super` arms at `:224-234` and `:235-244`), `classify_suspicious_pub` (`:35`), `assess_parent_facade_usage` (`:290`)
- `src/compiler/persistence/caller_aware.rs` — expose the cross-target caller union it already builds (`:6-15`) so the load-side refinement pass can read it. **The `retain` necessarily changed** (this Files line previously claimed it did not): before this phase `ForbiddenPubInCrate` findings carried no `narrower_scope_def_path`, so caller-aware suppression never applied to them; now that they do, an early `if finding.diagnostic_code == DiagnosticCode::ForbiddenPubInCrate { return true; }` is what keeps them visible. **The caller-union key must carry package identity** — it is `CallerKey { package_root, target_def_path }`, not a bare def path. `package_root` comes from `PACKAGE_ROOT_ENV` (`src/compiler/settings.rs:50`), so a package's lib and its `src/bin/*` share it and still join, while separate workspace members that happen to share a module path stay isolated. Later phases that read this union must use the same two-part key.
- `src/compiler/persistence/load.rs` — add the refinement pass to the existing chain at `:81-83`, after `apply_caller_aware_suppression` (which is what builds the union) and before or after `apply_visibility_narrowing_priority` — order against the latter does not matter, since it drops findings rather than editing text
- `src/compiler/visibility/scan/visibility_context.rs` — `sink.use_sites` (`:76`), the in-pass caller source for the first of the classifier's two calls
- `src/reporting/diagnostics.rs` — matrix headlines flow through `HeadlineSource::FindingMessage`, which is **not** the bare unit variant this Spec draws: it carries `fallback: &'static str` (Phase 2). **Phase 4 moved `INTERNAL_PARENT_PUB_USE_FACADE` (`:172-181`) onto `FindingMessage`, so three specs now use it while `forbidden_headline_uses_message_with_static_fallback` (`:445-479`) still loops over only the two forbidden codes — extend that loop to all three, plus every new matrix outcome.** Per Phase 2's retrospective that test is the only thing preventing a silent revert to a static headline. `SUSPICIOUS_PUB.inline_help` is at `:123`. Note the production variant is `Static`, never `Literal` — `Literal` is the test mirror's name for it.
- **Resolved decision — `SUSPICIOUS_PUB.inline_help` (`:123`) is cleared to `None` and every `suspicious_pub` suggestion becomes dynamic.** Today that spec carries the fixed string ``consider using: `pub(super)` ``, which is wrong for an item behind a facade, where the correct annotation names the facade's boundary. Compute the suggestion at the site that knows whether a facade is involved: the boundary-aware string in the facade case, the existing `pub(super)` text otherwise. This removes the precedence question rather than answering it, and it is the only version where the two output modes cannot drift apart again.
- `src/reporting/render/diagnostic.rs` — human output resolves help as `custom_inline_help_text(..).or_else(inline_help_text)` (`:52-53`). With the static string gone this arm always takes the dynamic one; verify nothing else depended on the fallback.
- `src/reporting/cargo_json.rs` — machine output resolves in the **opposite** order, `inline_help_text(..).or_else(custom_inline_help_text)` (`:209-211`), so today `cargo mend --message-format=json` would print `consider using: pub(super)` while the terminal printed the boundary annotation for the same finding. `rustc_diagnostic` (`:154-167`) sidesteps the choice by emitting **both** as separate `help` children, so a consumer sees two contradictory suggestions on one finding. Make both renderers resolve identically and make `rustc_diagnostic` emit exactly one `help` child per finding. Neither this phase nor Phase 12 listed these two files before this decision, so as previously written neither could fix it.
- **The three test-support rows below all shipped in Phase 3 — no plumbing work remains here.** `tests/support/report.rs:10` already carries `pub headline: String`; `tests/support/mend_json.rs:249-253` already populates it from the diagnostic's `/message`; and `tests/support/diagnostics.rs` no longer holds a plain `&'static str` headline — it mirrors `HeadlineSource { Literal(&'static str), FindingMessage { fallback: &'static str } }` at `:122-140`, resolved via `HeadlineSource::resolve(&finding.headline)`. `assert_rendered_diagnostics` lives in `tests/diagnostics/rendering.rs:220`, not in `tests/support/diagnostics.rs`, and already resolves headlines from the report rather than from a static string. The matrix's "does not match the parent facade boundary" row makes a forbidden code emit a non-fallback message, which breaks that assertion and Phase 2's two helpers at `tests/diagnostics/rendering.rs:262` and `:324` together.
- `src/compiler/visibility/annotation.rs` — read-only: `VisibilitySyntax` (`:17-28`) is the enum the matrix dispatches on; `VisibilityAnnotation::syntax()`, `reach(target, tcx)` (`:86`), `VisibilityReach::{compare, to_source}` (`:164`, `:197`) are the comparison and rendering surface. If this phase is the one that leaves no item unused, delete the `#[expect(dead_code)]` on `mod annotation;` in `src/compiler/visibility/mod.rs:1-5` — clippy is deny-by-default here and a fulfilled expectation is itself an error.
- `src/config/mod.rs` — **add `pub(crate) use pub_in_path::PubInPath;`** beside the `PreludePubMod` re-export at `:27`. Phase 6 added only `mod pub_in_path;` (`:9`), so the type is not nameable outside `src/config/` yet
- `tests/diagnostics/forbidden_pub_in_crate.rs`, `tests/diagnostics/allowances.rs`, `tests/diagnostics/pub_use_fixes.rs` — fixtures in the gate
- `tests/diagnostics/rendering.rs` — **the pre-existing all-diagnostics fixture must now pin `pub_in_path` in its own `mend.toml`.** It writes none today, so once this phase reads the setting the fixture inherits the developer's machine-global config and both `assert_eq!(…, 16)` gates (`:404`, `:522`) fail on unchanged source for anyone whose global config says `"required"`

**Note on the `:NNN` refs above:** they are post-Phase-4 and Phase 5 shifted `record.rs` and `policy.rs` by up to 80 lines. The corrected anchors are in the Delegation Context **Key files** rows for those two files; prefer the symbol names over the numbers here.

**Constraints from prior phases:** Phase 1 supplies `VisibilityAnnotation`, `VisibilityReach`, `anchored`, and the reach→text rendering. `VisibilityReach` has no `PartialEq` and deliberately no `Ord`/`PartialOrd`, so "the annotation's reach **equals** the chain's required reach" must be spelled `lhs.compare(rhs, tcx) == Some(Ordering::Equal)`, never `==` on the reaches themselves; `compare` returning `None` means two restricted scopes are incomparable siblings, which for this matrix is a mismatch, not an error. `reach()` is a method taking `(target: LocalDefId, tcx: TyCtxt<'_>)`, so every matrix dispatch site must carry the item's own `LocalDefId`. Phase 2 supplies `HeadlineSource::FindingMessage` on both forbidden specs plus the cargo-JSON no-duplicate helper. Phase 3 supplies written-form dispatch, `ItemCategory::{Declaration, Use}`, field rejection, and reject-once ordering. Phase 5 supplies `ParentFacadeAnalysis` with `nearest: Vec<ParentFacadeOccurrence>` — non-empty by construction, guaranteed by the `Option<ParentFacadeAnalysis>` wrapper rather than by the type — each carrying `FacadeSyntax` and a `OnceCell` usage cache, plus `FacadeChainResolution::{Resolved, Unresolvable}`, resolved once per item. Phase 6 supplies `PubInPath` on the resolved `VisibilityConfig`.

Three Phase 6 facts this phase depends on directly:

1. **`PubInPath` is not re-exported yet.** Phase 6 added only `mod pub_in_path;` (`src/config/mod.rs:9`). The precedent `PreludePubMod` carries `pub(crate) use prelude_pub_mod::PreludePubMod;` at `:27`, which is how `record.rs:36` imports it. **This phase must add the matching `pub(crate) use pub_in_path::PubInPath;`** — without it `crate::config::PubInPath` is E0603.
2. **Read the value as `ctx.settings.visibility_config.pub_in_path`** — a plain `PubInPath`, never an `Option`; precedence is already resolved. `LoadedConfig` never crosses into the driver: `src/compiler/build/execute.rs:168` serializes `VisibilityConfig` to JSON into `CONFIG_JSON_ENV`, and `src/compiler/settings.rs:37` deserializes it into `DriverSettings.visibility_config`. `#[serde(default)]` on the field means an older env payload still deserializes to `Permitted`. Reference call site for reading a resolved config value off `ctx.settings`: `record.rs:356`.
3. **The comment at `record.rs:354` is now false** — it reads "exempt by default (global `allow_prelude_pub_mod`)", but Phase 6 moved that key onto project>global. Correct it here; Phase 8 is documentation-only and touches no `src/` file, and this phase rewrites the surrounding recorder anyway.

**Fixtures must pin their own `[visibility]` settings.** This phase is what makes `pub_in_path` load-bearing, which is what turns the Delegation Context's fixture invariant from latent into live: any diagnostics fixture without its own `mend.toml` inherits the developer's real machine-global config from here on.

**Resolved decisions (2026-08-01).** All three of this phase's pending decisions were answered by the user and their outcomes are folded into the Spec and Files above. Recorded here so later passes do not relitigate them:

1. **Where the no-facade caller classifier runs — both places, off one shared function.** The persistence layer cannot build a suggestion (`apply_caller_aware_suppression` only `retain`s findings) and the compiler pass cannot see the cross-target caller union (`sink.use_sites` is current-target only), so neither site alone is sufficient. The branch selection is written once as a pure function over already-persisted strings, called in-pass from `record.rs` to compose the text and again as a fourth pass at `persistence/load.rs:81-83` to refine it against the union. Safe because the refinement is monotone: added callers can only widen the branch, so the load pass can only retract a suggestion that would have produced E0603 in a sibling target, never introduce one. Full design in the Spec above.

2. **`suspicious_pub` help — the static string is deleted and every suggestion becomes dynamic.** `SUSPICIOUS_PUB.inline_help` goes to `None`; the two renderers are made to resolve identically and `rustc_diagnostic` emits one `help` child instead of two. This removes the precedence question rather than answering it, and it is the only version where the two output modes cannot drift apart again. `src/reporting/cargo_json.rs` and `src/reporting/render/diagnostic.rs` joined this phase's Files as part of the resolution.

3. **This phase is NOT split.** The recommendation to split into 7a/7b rested on Phase 4's eleven fix passes and Phase 5's fourteen as evidence that a wording-heavy phase cannot converge in one dispatch. Phase 6 identified a different cause: every one of those passes handed a fresh blind reviewer the *entire accumulated phase diff*, so "review comes back clean" was unreachable regardless of phase size. Phase 6 scoped its review to the incremental diff and got the run's first clean review on the first pass, with zero fix passes. Phase 7 therefore stays whole and is reviewed incrementally, and Phases 8-11 keep their numbers. **The implementer must sequence the five jobs deliberately, in this order:** (1) boundary acceptance — narrow `record_forbidden_pub_in_crate`, wire the two new parameters, keep today's headlines verbatim, and confirm `tests/diagnostics/forbidden_pub_in_crate.rs` still passes before going further; (2) the advice matrix; (3) the no-facade classifier and its load-side refinement; (4) `suspicious_pub` admission; (5) the renderer help unification. Step 1 is independently green — get it green first rather than landing all five at once.
**Acceptance gate:** `verify.sh check`, `verify.sh test`, `verify.sh lint` green, **plus `bash ~/.claude/scripts/delegate/verify.sh test cargo-mend diagnostics` — the bare `verify.sh test` line runs only `--lib`/`--bins`, so every fixture under `tests/diagnostics/` is invisible to it and a phase whose only new tests live there would gate green having run none of them,** **and the self-policy gate: `RUSTC_BOOTSTRAP=1 cargo +stable run --release -- --workspace --all-targets --fail-on-warn` reports "No findings" on cargo-mend's own source.** That last check is not redundant with `lint` — Phase 1 shipped `check`/`test`/`lint` all green while the tool rejected its own new file, and two blind reviews missed it, because the only thing that knows cargo-mend's house rules is cargo-mend. The two rules that bite new rustc-facing code are `inline_path_qualified_type` (write `use rustc_middle::ty::Foo;` and then `Foo`, never an inline `rustc_middle::ty::Foo` in a type position) and `imports_at_top`. Fixtures: exact-boundary `pub(in ...)` behind a `pub(super) use` facade — no finding at `"permitted"`, error at `"forbidden"`; one level too wide (`pub(in crate)` where `crate::a` is required) — error at every setting with the headline quoting `pub(in crate)`; one level too narrow — a **rustc compile-fail control** asserting E0364/E0365, not a mend fixture; no facade at all + `pub(in ...)` — one fixture per classifier branch (callers all in the defining module → removal; callers within the parent scope → `pub(super)`; a caller above the parent → no annotation replacement, the two structural migrations); bare `pub` behind a `pub(super) use` facade — allowed at `"permitted"`, `suspicious_pub` at `"required"`; `pub(in super::super)` naming the correct boundary — error with the `crate::`-rooted suggestion; external-crate re-export control — no exception granted; stale accepted annotation whose facade is unused — `suspicious_pub` fires; `--fix`, `--fix-pub-use`, and `--fix-all` on that finding — no fixability note, no `StoredPubUseFixFact`, no skipped restricted candidate. Table-driven coverage across all three settings × each written syntax category, asserting diagnostic code, headline, help, and the complete code set — including acceptance under `Required`, canonical-reach-valid `pub(in crate)`, and ordinary `pub(in self)`/`pub(in super)` declarations. **Every fixture in this phase's gate must pin `pub_in_path` in its own `mend.toml`, and so must the pre-existing all-diagnostics fixture in `tests/diagnostics/rendering.rs`** — this phase is what makes the setting load-bearing, and an unpinned fixture silently inherits the developer's machine-global config (Delegation Context invariant). `policy.rs` unit tests (start `:524`, not `:490`): `forbidden_pub_crate_suggestion` names the boundary path in **both** `Super` facade arms, and the spelling-gated split between them still holds — the known-`pub(super)`-spelling arm keeps its E0364 claim, the neutral fallback still names no modifier. **Any fixture asserting the absence of `ForbiddenPubCrate` must sit at logical depth three or deeper** (Delegation Context invariant); a depth-two fixture passes that assertion no matter what this phase does.

**Two gate items from the resolved decisions:**

- **Cross-target refinement (decision 1).** A library-plus-binary fixture in one package where the item's only caller above its parent lives in the sibling binary: the in-pass classifier picks the narrower branch from the library target alone, and the load-side pass must downgrade it to the structural-migration row (no annotation replacement). Assert the *final* suggestion, and assert that the same fixture without the binary keeps the narrower suggestion — otherwise the test passes on a refinement that never ran. The harness already supports this shape: copy `tests/diagnostics/rendering.rs:1490-1541`, which writes `app/src/lib.rs` alongside `app/src/bin/probe.rs` and asserts on cross-target reach. Add a unit test for the shared branch function directly, since it is a pure function over strings.
- **Renderer parity (decision 2).** One `suspicious_pub` finding behind a facade, rendered three ways: the terminal help text and the `--message-format=json` help text must be **byte-identical**, and the `rustc_diagnostic` output must carry exactly **one** `help` child. Assert the count, not just the presence — the defect this replaces was two contradictory children, which every presence-only assertion passes. No `suspicious_pub` finding anywhere may still render `consider using: pub(super)` from a static source; that string now exists only as one branch of the dynamic computation.

Three test obligations this phase inherits from Phase 3:

- **Update the nine `assert_headline_and_help` pairs** in `tests/diagnostics/forbidden_pub_in_crate.rs:77-130`. They pin exact strings of the form ``use of `X` is forbidden by policy``, and the advice matrix replaces at least the `pub(in crate)` headline with the redundant-spelling wording. The same file's `assert_codes` vectors are **order-sensitive and must not change** — if they do, reject-once or the dispatch order has regressed.
- **`tests/diagnostics/rendering.rs` is a hard gate, not a soft assertion.** `:223-227` panics with "fixture is missing finding for {code:?}" if any `DiagnosticCode` fails to fire, and `:399` asserts `report.findings.len() == 16`. This phase changes which findings fire, so **adjust that fixture, never the assertion**. Note `src/private_parent/child.rs:139`'s `pub(in crate::private_parent) fn subtree_only()` has no facade and so still survives acceptance — but confirm rather than assume.
- **Close Phase 3's carried-forward field gap:** add one fixture with a canonical `pub(crate)` field in a location where `policy::allow_pub_crate_by_policy` denies, asserting `forbidden_pub_crate`, plus a `pub(super)` field asserting no finding. Phase 3 shipped that behavior but every field in its fixture is a `pub(in ...)` form, so the canonical path is currently proven only by the self-policy run on cargo-mend's own source.

### Retrospective

**What worked:**
- The mandated job ordering (acceptance → matrix → classifier → `suspicious_pub` → renderer parity) held; step 1 was independently green before the rest landed, exactly as the Work Order required.
- Reviewing each fix pass against its **incremental** diff rather than the accumulated phase diff again produced fast convergence: two fix passes, then a clean review from both the main agent and a fresh blind reviewer.

**What deviated from the plan:**
- **The caller union needed a two-part key.** A bare target def path collided across workspace members. It is now `CallerKey { package_root, target_def_path }`, keyed off `PACKAGE_ROOT_ENV` (`settings.rs:50`) so a package's lib and bins join while separate members stay isolated. The Files line for `caller_aware.rs` has been corrected.
- **`caller_aware.rs`'s `retain` had to change**, contrary to the original Files line. `ForbiddenPubInCrate` findings now carry a `narrower_scope_def_path`, which newly exposed them to caller-aware suppression; an early `return true` for that code preserves prior visibility.
- **Findings now persist the whole annotation, not a source line.** A new `Finding::visibility_annotation` field (`persistence/schema.rs:58`) carries the exact annotation text, because `source_line` holds one physical line and therefore cannot represent a multiline `pub(\n in crate::a\n)`. `FINDINGS_SCHEMA_VERSION` went 18 → 19 (`constants.rs:65`).
- **Caller recording gained two kinds** it was missing: ancestor modules (`record_target_modules`) and field accesses (`record_field_target`, via `expr_ty_adjusted` + `opt_field_index`) in `use_sites.rs`. Without them the classifier saw an incomplete caller set and picked a too-narrow branch.

**Surprises:**
- **Display text and stored text must diverge.** Interpolating the raw annotation slice into a headline put literal newlines into a diagnostic message. `VisibilityAnnotation::display_source()` (`annotation.rs:72`) now normalizes whitespace for the 12 message/suggestion sites, while `source()` stays exact at the only two sites that reparse or persist it (`record.rs:400`, `record.rs:635`).
- **A `use` item's empty caller set proves nothing.** Resolved paths name the imported target, not the local alias, so caller-derived advice is withheld for `use` items — but that guard must sit *below* the redundant-spelling branches, since `pub(in super)` ≡ `pub(super)` is a pure syntactic identity needing no caller evidence at all.
- **A schema bump silently blanks every stale report.** After the 18 → 19 bump, a repo whose `cargo check` was fully cached emitted no fresh findings and had its old reports discarded, printing `No findings.` — indistinguishable from a real pass. Any smoke run or manual scan must clear `target/mend-findings` and force a recompile, or it proves nothing.

**Implications for remaining phases:**
- Phase 9, 10, and 11 all read the caller union; they must use the two-part `CallerKey`, not a bare def path.
- Anything that renders annotation text must use `display_source()`; anything that stores or reparses it must use `source()`.
- Phase 11's auto-fix must read `visibility_annotation`, not `source_line`, or it will mis-handle multiline annotations.

### Phase 7 Review

- **Phase 12** — its `src/` work already shipped here. `classify_suspicious_pub` (`policy.rs:50-66`) gates `basic_suspicious_pub_allowance`, `assess_parent_facade_usage`, and the `ShallowPrivatePolicy` allowance on `required_path.is_none()`, and `required_setting_reviews_bare_pub_behind_restricted_facade` already pins the behavior. Its Files row now says no `src/` edit is expected unless the default flips; remaining work is the hana conversion, the default decision, and the CHANGELOG.
- **Phase 12** — the Phase 9 dependency edge is dropped. `assess_signature_exposure_allowance` is deliberately not gated on `required_path.is_none()`, so signature-exposed items already stay allowed at `Required`. Phase order was left unchanged at that time; it was corrected later — see the Phase 9 Review.
- **Phase 9** — the "do not write `pub(in crate::compiler)`" prohibition is void and was replaced with the shape that now works. Phase 7 uses that spelling at five sites with the self-policy gate green. The Phase 1 Review's matching "rejected direction" bullet is marked superseded.
- **Phase 9** — the instruction to delete `#[expect(dead_code)]` from `mod annotation;` is now a no-op; Phase 7 removed it.
- **Phases 8–12** — every acceptance gate now requires clearing `target/mend-findings` and forcing a recompile before the self-policy run. After a schema bump a fully cached `cargo check` prints `No findings.` indistinguishably from a real pass; this happened for real during Phase 7's smoke test.
- **Phase 11** — gained the undeclared plumbing it needs: `visibility_annotation` lives only on `StoredFinding` and `FindingParams`, not on the `Finding` the fixers consume, so `reporting::Finding`, the `StoredFinding → Finding` conversion, and `tests/support/report.rs` all have to carry it.
- **Phase 11** — now names `canonical_pub_in_boundary` as a third annotation parser that must stay separate from the shared span parser, and flags the divergent duplicate `def_path_is_descendant` in `caller_aware.rs` vs `policy.rs:412`.
- **Phase 9** — "Do not edit `:224-234`" replaced: `forbidden_pub_crate_suggestion` now takes a fourth `boundary_path` parameter and returns `String`, so retiring `SignatureExposure` reaches it; the spelling-gated `Super` split and its E0364 claim must survive.
- **Phase 8** — gained four Phase 7 upgrade cases (schema 18 → 19, the deleted static `suspicious_pub` help, one `help` child in cargo JSON instead of two, and the flipped custom-before-static resolution order) plus a correction naming the real `suspicious_pub` admission guard.
- **Doc-wide** — stale refs corrected: schema version `18` → `19` at `constants.rs:65`; `rendering.rs` count assertions `:404`/`:522` → `:409`/`:633`; `assert_rendered_diagnostics` → `:230` (panic `:237`); `assess_signature_exposure_allowance` → `policy.rs:503`; `has_signature_exposure_allowance` → `:694`; `assess_parent_facade_usage` → `:465`; `maybe_record_suspicious_pub` → `record.rs:899`; the `StoredPubUseFixFact` write → `record.rs:1079`. Phase 12's re-export baseline 162 → 175.
- **Deferred to Phase 8 as a pending decision** — after the cross-target refinement, a no-facade finding's headline and help can contradict each other, because the load pass rewrites only `suggestion` while the recorder also branches `message`. A current test pins the mismatch. Recommendation is to extend the load pass to rewrite `message` too.

---

### Phase 8 — README and style-guide updates · status: done

#### Work Order

**Goal:** `README.md` and `~/rust/nate_style/rust/use-narrowest-visibility.md` document the new rung, the config key, and the upgrade contract.

**Spec:**

First, make the cross-target refinement keep each diagnostic's headline and help in agreement. `no_facade_pub_in_advice` and `refine_no_facade_advice` must use one shared headline selector. When the combined caller set reaches `NoFacadeAdvice::StructuralMigration`, the load pass rewrites `finding.message` to `no visibility annotation allowed by policy preserves this item's current callers` alongside the structural-migration suggestion. For the other two branches, preserve the existing generic headline; do not reconstruct it from the persisted raw annotation text. Update the sibling-binary regression test to assert the structural headline and help together, while the library-only case keeps its current generic headline and removal suggestion.

`README.md`:

1. **`<a id="forbidden-pub-in-crate">` (`:258`)** — rewrite. Keep the smell argument as the lead, then add the exception with the worked `video_plane` example and the E0364 proof of what `pub` gives up. Retitle to "Forbidden `pub(in ...)`" since the diagnostic now covers relative paths too. State that the exception is declaration-only and does not apply to fields.
2. **`<a id="forbidden-pub-crate">` (`:194`)** — the "Otherwise, prefer:" list at `:209-213` gains a bullet between `pub(super)` and the relocate advice:
   > - `pub(in crate::path)` when a parent facade re-exports the item with `pub(super) use`, so `pub(super)` at the declaration would not compile
3. **`<a id="suspicious-pub">` (`:287`)** — document the facade-boundary suggestion, that it only appears when `pub_in_path = "required"`, and that `suspicious_pub` now also inspects accepted `pub(in ...)` items.
4. **Config section (~`:70-99`)** — document `pub_in_path`, its three values, the `permitted` default, and the project-overrides-global precedence, alongside `allow_pub_mod` / `allow_prelude_pub_mod`.
5. **The visibility ladder (~`:184-190`)** — the numbered "prefer" list gains `pub(in crate::path)` as the rung between `pub(super)` and `pub(crate)`, with the one-line condition that gates it.

`docs/style/readme-diagnostic-section.md` needs no change — the section format is unchanged.

`~/rust/nate_style/rust/use-narrowest-visibility.md`:

1. **The ladder** — insert `pub(in crate::path)` as a rung, conditioned on a `pub(super) use` facade, declarations only.
2. **"When bare `pub` is required at depth 3+"** — currently documents the facade case as one of two reasons `pub` is unavoidable. It is no longer unavoidable; the section keeps only the signature-exposure reason and points the facade case at the new rung.
3. **The decision table:**

   | Parent's re-export | Correct declaration | If you write `pub(crate)` |
   |---|---|---|
   | none | `pub(super)` | `forbidden_pub_crate` fires |
   | `pub(super) use` | `pub(in crate::<facade parent>)` | fires — and `pub(super)` will not compile |
   | `pub(crate) use` | `pub(crate)` | accepted |
   | `pub use` to crate root | `pub` | does not compile — a bare `pub use` requires the declaration to be `pub` |

   The `**Tooling:**` line under it gains the `pub_in_path` setting.

**A seventh README edit, also user-visible text Phase 4 changed.** `forbidden_pub_crate`'s help now has **two** facade variants — one that quotes the facade's `use` spelling, one that names no modifier at all (`policy.rs:224-244`) — and `internal_parent_pub_use_facade`'s headline and item text are now dynamic, degrading to the neutral word "re-export" when the spelling cannot be established (`record.rs:508-527`). Document the rule behind that: **mend quotes a facade's `use` spelling only when it can establish it, and resolved reach alone never licenses quoting one.**

**A sixth README edit, in the `forbidden-pub-crate` section (`README.md:194`), not the `forbidden-pub-in-crate` one.** Phase 3 made that diagnostic fire on struct and union fields, which the section does not mention at all — it still reads as an item-only rule. Add: the rule now applies to fields as well as items, a `pub(crate)` field is rejected wherever a `pub(crate)` item would be, and `pub(super)`/`pub(self)` fields are unaffected. This is a user-visible scope change that shipped in Phase 3; without this edit the README documents behavior the tool no longer has.

**Guidance text to land in both READMEs, in the `forbidden-pub-in-crate` section:**

**Reach for `pub(in crate::path)` in exactly one situation:** a parent module re-exports the item with `pub(super) use`, which puts the item's required reach above the module it lives in. `pub(super)` at the declaration is too narrow to compile; `pub` is wider than the truth.

**The path is the parent of the module holding the facade — not one level above the item.** In the `video_plane` example the item lives in `video_plane::plane::camera_panel`, the facade lives in `video_plane::plane`, and the annotation reads `pub(in crate::video_plane)` — two levels above the item. With chained facades the distance grows. The rule is always: find the widest facade, take the parent of the module holding it.

**The path names who can see the item, not who owns it.** Reading `pub(in crate::video_plane)` as "this belongs to `video_plane`" gets the meaning backwards.

**Always spell it `crate::`-rooted.** `pub(in super::super)` compiles and names the same module, but it forces every reader to count levels, and it silently changes meaning if the file moves.

**Declarations only.** A `use` line picks its own reach, so `pub(super)`, `pub(crate)`, and `pub` already span what it can need. Fields are excluded for a different reason: a field is not re-exportable, so no facade can ever justify one.

**Do not use it to avoid moving an item.** If the path you need is long, or names a module that has nothing to do with the item, the item is in the wrong place.

**It is not a way to widen anything.** `pub(in crate::a)` is narrower than `pub(crate)` and narrower than `pub`. If reaching for it feels like unlocking access, check whether a `pub(crate) use` facade is what is actually wanted — and say so on the `use` line, which is where the decision belongs.

**Upgrade contract for `CHANGELOG.md`** — eleven cases:

- *Previously green, `forbidden_pub_in_crate` enabled* — exact `pub(in crate::...)` declarations covered by the scanner were absent, but `pub(in crate)`, relative spellings, and restricted field annotations may become new errors. Output is unchanged only when none of the newly detected forms exist.
- *`forbidden_pub_in_crate` disabled* — existing annotations may be present. Exact boundaries become permitted, while `suspicious_pub` or another canonical diagnostic may newly appear; disabling `forbidden_pub_in_crate` does not suppress those codes.
- *`forbidden_pub_in_crate` disabled but `forbidden_pub_crate` enabled* — the new `pub(in crate)` detection routes to the *enabled* code, so a project that believed it had opted out still gets new errors.
- *Already failing* — an exact-boundary error may disappear under `Permitted`; retained failures get new headlines and help; secondary findings may change count and order.
- *Machine-readable output shape changed* — the other four cases are about which findings fire; this one is about their JSON. Phase 2 stopped `rustc_diagnostic` emitting the `note` child that duplicated the headline (`src/reporting/cargo_json.rs:148-150`) and stopped `render_diagnostic` repeating it in `rendered` (`:220-224`), so any consumer parsing mend's cargo JSON sees one fewer child on **every** forbidden-visibility finding, including projects whose findings are otherwise unchanged.

- *Struct and union fields now follow the `pub(crate)` location policy* — shipped in Phase 3, not Phase 7. `check_field` runs the shared rejection classifier and passes `None` for the facade argument (`src/compiler/visibility/field.rs:103`), so a canonical `pub(crate)` field reaches `record_forbidden_pub_crate` and errors wherever `policy::allow_pub_crate_by_policy` does not permit it — the same rule that already governed `pub(crate)` items. This is narrower than "every `pub(crate)` field is now an error": `pub(super)` and `pub(self)` fields remain allowed, and permitted locations stay green. It is still the widest-reaching behavior change in this feature for existing codebases, because struct fields were previously exempt from the rule entirely.
- *Reject-once changed which codes co-occur* — a forbidden `pub(crate) mod` previously emitted both `forbidden_pub_crate` and `review_pub_mod`; it now emits only the rejection. Finding counts drop and code sets change for projects whose output is otherwise unaffected, which matters to anyone asserting on totals.

Four more from Phase 4:

- *Facade `use` spellings are quoted only when mend can establish them* — `forbidden_pub_crate`'s help now has two facade variants, one naming the facade's actual `use` spelling and one naming no modifier at all (`policy.rs:224-244`), and `internal_parent_pub_use_facade`'s headline and item text are now dynamic, degrading to the neutral word "re-export" when the spelling is unrecoverable (`record.rs:508-527`). Previously these strings always said `pub use`, which was wrong for every restricted facade. Any consumer matching on those exact strings will see different text.
- *Impl items reached through their self type now count as exposed* — an `impl` block's items pick up exposure from the type they are implemented on, which they did not before. Items that were previously flagged may now be allowed.
- *Files unreachable from the crate root no longer influence findings* — a source file that no `mod` declaration reaches is no longer scanned for usage or facades, so findings that depended on it disappear.
- *A child module's self-referential import of a parent re-export no longer counts as outside use* — a child importing an item back through its own parent's facade previously read as an external caller and suppressed narrowing advice; it no longer does, so `narrow_to_pub_crate` and stale-facade findings may newly appear.

The first three need release-note compatibility bullets, stated as the two-code matrix and using the word *disable* — `[diagnostics]` cannot downgrade. The fourth belongs in the feature note describing newly accepted exact boundaries and revised advice. No warning-first grace mode is built: adding one would mean giving `[diagnostics]` a severity state it does not have, in the middle of this feature, to defer a handful of one-line annotation edits.

**Files:**
- `src/compiler/visibility/policy.rs` — shared no-facade headline selection beside `NoFacadeAdvice` / `no_facade_suggestion`
- `src/compiler/visibility/scan/record.rs` — use the shared selector for the in-pass headline
- `src/compiler/persistence/load.rs` — update `message` alongside `suggestion` after cross-target caller refinement
- `tests/diagnostics/forbidden_pub_in_crate.rs` — replace the pinned contradictory sibling-binary pair with the structural headline and matching help; keep the library-only pair unchanged
- `README.md` — `:194`, `:209-213`, `:258`, `:287`, `~:70-99`, `~:184-190`, **and `:83-84` + `:102-103`** (see the sixth item below — `:102-103` sits outside the `~:70-99` range and is easy to miss)
- `CHANGELOG.md` — the eleven upgrade cases above, **plus a twelfth, plus four more from Phase 7 (below)**

**Four more upgrade cases and one README correction — from Phase 7.** None of the twelve cases covers these, and all four are user-visible:

1. **`FINDINGS_SCHEMA_VERSION` 18 → 19** (`src/compiler/constants.rs:65`), because findings gained a `visibility_annotation` field. Every cached report written by an older build is invalidated. Note the failure mode in the changelog entry: a fully cached `cargo check` after an upgrade emits nothing fresh, has its stale reports discarded, and prints `No findings.` — which looks exactly like a clean run. Tell users to `rm -rf target/mend-findings` and force a recompile after upgrading.
2. **`SUSPICIOUS_PUB.inline_help` is deleted** (`src/reporting/diagnostics.rs:123`). The string ``consider using: `pub(super)` `` no longer comes from a static source; every `suspicious_pub` suggestion is now computed, so items behind a facade get the facade's boundary instead.
3. **`cargo mend --message-format=json` now emits exactly one `help` child per finding**, via `resolved_inline_help_text` (`src/reporting/cargo_json.rs:154`). It previously emitted both the static and the dynamic text, so a consumer saw two contradictory suggestions on one finding.
4. **Help resolution order flipped to custom-before-static in both renderers.** For any spec carrying both, `--message-format=json` now prints the same text the terminal prints — previously the two disagreed. Machine consumers pinned to the old string will see a change.

**README item 3 must name the real guard.** Its claim that `suspicious_pub` "now also inspects accepted `pub(in ...)` items" is right but vague; the guard is at `src/compiler/visibility/scan/record.rs:96-99`, which admits `VisibilitySyntax::Public | VisibilitySyntax::InPath(PathSpelling::CrateRooted)` at `logical_module_depth > 1`.

**Sixth README item and twelfth upgrade case — from Phase 6.** Phase 6 moved `allow_prelude_pub_mod` onto project > global precedence, which falsified two README statements that predate it: `:83-84` still says the prelude switch "lives in the global config", and `:102-103` still says only `[diagnostics]` entries override globals. Both must be rewritten to describe project `mend.toml` > global > compiled-in default for the whole `[visibility]` table, not just for the new key. The twelfth CHANGELOG upgrade case: *a project `mend.toml` `[visibility] allow_prelude_pub_mod` is no longer discarded — a repo that set it and relied on it being ignored changes behavior.*
- `~/rust/nate_style/rust/use-narrowest-visibility.md` — ladder, depth-3 section, decision table, Tooling line

**Constraints from prior phases:** Phases 3, 6, and 7 define the behavior being documented: the two-code split for rejections, `pub_in_path` with project>global precedence and a `permitted` default, and acceptance limited to exact-boundary declarations. `FINDINGS_SCHEMA_VERSION` was bumped to `19` in Phase 7 (`constants.rs:65`).

**Acceptance gate:** `verify.sh check`, `verify.sh test`, `verify.sh lint` green, **plus `bash ~/.claude/scripts/delegate/verify.sh test cargo-mend diagnostics` — the bare `verify.sh test` line runs only `--lib`/`--bins`, so every fixture under `tests/diagnostics/` is invisible to it and a phase whose only new tests live there would gate green having run none of them,** **and the self-policy gate: `RUSTC_BOOTSTRAP=1 cargo +stable run --release -- --workspace --all-targets --fail-on-warn` reports "No findings" on cargo-mend's own source. **Before that run, `rm -rf target/mend-findings` and force a recompile (touch every non-`target` `.rs`, or `cargo clean -p cargo-mend`).** A stale stored report is rejected on version or fingerprint mismatch, so a fully cached `cargo check` emits nothing fresh and prints `No findings.` indistinguishably from a real pass — the gate would prove nothing. This is not hypothetical: it happened during Phase 7's smoke test after the 18 → 19 bump.** That last check is not redundant with `lint` — Phase 1 shipped `check`/`test`/`lint` green while the tool rejected its own new file, and two blind reviews missed it, because the only thing that knows cargo-mend's house rules is cargo-mend. The two rules that bite new rustc-facing code are `inline_path_qualified_type` (write `use rustc_middle::ty::Foo;` and then `Foo`, never an inline `rustc_middle::ty::Foo` in a type position) and `imports_at_top`. The sibling-binary no-facade fixture emits the structural-migration headline and matching help after cross-target refinement; the library-only fixture keeps the generic headline and removal suggestion. Every `<a id="...">` anchor referenced by a `DiagnosticSpec.help_anchor` still resolves in `README.md`. `docs/style/diagnostic-lifecycle.md`'s README checklist items are satisfied for both forbidden diagnostics.

### Retrospective

**What worked:**
- One `no_facade_headline` selector now keeps the compiler-pass and cross-target headline branches aligned; the sibling-binary regression pins the structural headline and help together.
- The scoped package tests, diagnostics suite, lint gate, and forced-rebuild self-policy run all passed after the documentation corrections.

**What deviated from the plan:**
- The approved Phase 8 decision expanded this documentation phase into `policy.rs`, `visibility/mod.rs`, `record.rs`, `load.rs`, and the cross-target regression test.
- Dual review found three documentation-only gaps outside the primary edited sections: two flat-ban statements, false global precedence for project-only allowlists, and the omitted widest-facade rule. The orchestrator corrected them directly.

**Surprises:**
- `allow_pub_mod` and `allow_pub_items` are project-only lists; only `allow_prelude_pub_mod` and `pub_in_path` participate in project > global > compiled-in precedence.
- Saying that chained facades increase the path length is insufficient guidance unless the reader is told to select the widest facade and then use its parent.

**Implications for remaining phases:**
- Any phase that recomputes `NoFacadeAdvice` must update headline and suggestion through the shared selectors; changing only one recreates contradictory output.
- Phase 12's default decision must preserve the README's documented `permitted` default unless it also updates the config and README sites already named in that Work Order.

### Phase 8 Review

- **Phase 9:** corrected the partially retained exposing-item identity, added the `record.rs` enforcement surface, explicit field/rendering ownership, live symbol anchors, and equal/narrower/sibling/public reach regressions.
- **Phase 12:** restored the Phase 9 dependency and deferred the rollout structure: the fixer should precede migration, Hana needs a separate repository-local plan, no `mend.toml` may be added, and the default choice waits for measured evidence.
- **Phase 11:** added the missing `FixKind`/runner/module routing surfaces, deterministic branch eligibility, the same-line facade defect (since moved to Phase 10 by the split), and explicit internal-JSON/schema and phase-splitting decisions.
- **All remaining phases:** separated delegate `verify.sh` gates from the orchestrator-owned forced-rebuild self-policy smoke run.

---

### Phase 9 — Signature exposure returns a level · status: done (`f10d1f9`)

#### Work Order

**Goal:** signature-exposure analysis returns the reach an exposure requires instead of a boolean, so an exact `pub(in ...)` is accepted whenever the exposure is contained below the facade boundary.

**Spec:**

The `SignatureExposure::Present` arm currently mandates `pub` on the grounds that a narrower modifier would not compile. That premise is false. Verified against rustc — this compiles clean:

```rust
mod video_plane {
    mod plane {
        mod camera_panel {
            pub(in crate::video_plane) struct Widget;
            pub(in crate::video_plane) fn make() -> Widget { Widget }
        }
        pub(super) use camera_panel::make;
    }
    pub(crate) fn caller() { let _w = plane::make(); }
}
```

The real constraint is "at least as visible as the widest signature carrying it", and an exact `pub(in ...)` clears it whenever the exposure is contained below that boundary.

**Phase 4 retained only part of the identity this phase needs.** Ordinary module-signature paths resolve an exposing `LocalDefId`, but impl exposure still collapses associated-item surfaces to `bool` through `outward_impl_surface_mentions_name`, and the parent-boundary path discards its resolved ID after `is_some()`. Retain the actual exposing declaration through every branch before computing and joining reaches; do not assume the current boolean paths can be lifted mechanically.

The source visitor also gates more than `public_item_name`: both `public_item_surface_mentions_name` and `outward_impl_surface_mentions_name` still admit only bare `pub`. Widen the complete exposing-item selection path so `pub(crate)` and `pub(in path)` declarations can contribute their real reach, while preserving the resolved exposing item rather than returning a name/boolean alone.

The four predicates behind `assess_signature_exposure_allowance` (`policy.rs:503` after Phase 7, not `:320`) **are no longer in `policy.rs`** — Phase 4 moved them into `src/compiler/exposure/`, re-exported from `exposure/mod.rs:4-8` as `child_item_is_exposed_by_other_crate_visible_signature`, `impl_item_is_exposed_by_exported_self_type`, `child_item_is_exposed_by_sibling_boundary_signature`, and `parent_boundary_public_signature_exposes_child_used_outside_parent`. Each still returns on first match and collapses recursion to `bool`.

Each path must retain the resolved exposing item, compute its effective reach, and `join` it with the facade-required reach. The result type is `Option<VisibilityReach>`; `None` means no exposure, which retires `SignatureExposure::{Present, Absent}` (`scan/classify.rs:27`).

The computed exposure floor must reach the decision point in `visibility/scan/record.rs`, not stop inside `exposure/`. Join it with the resolved facade reach before exact-boundary acceptance and before `forbidden_pub_crate`, `suspicious_pub`, and `unused_pub` decide whether an annotation is wide enough. Pin four relationships: exposure equal to the facade boundary, exposure narrower than it, an incomparable sibling exposure whose join is their common ancestor, and public exposure that still requires bare `pub`.

**Anchor every exposure reach before accumulating it.** Sibling scopes are reachable here — an exposing signature can be a sibling or parent boundary reaching past the facade, which is why there are four distinct predicates rather than one. With a single exposure there is no second operand to trigger `join`'s common-ancestor branch, so a lone sibling reach would render as a sibling path and hit E0742. Apply `anchored(reach, target, tcx)` from Phase 1 to each exposure reach, then join the normalized reaches.

**The wording fix this Work Order used to schedule here is already shipped — do not redo it, and do not touch `policy.rs:224-234`.** Phase 4 removed the overstated claim from the signature-exposure arm, which now reads ``"this item is exposed through a public signature; consider using `pub`"`` (`policy.rs:221-223`). The only surviving instance of "would not compile" is at `policy.rs:232-233`, inside the *facade* `Super` arm, where it is factually correct (the re-export would exceed the item and fail E0364) and is deliberately gated on a **known, unconflicted** `pub(super)` spelling. Softening that one would remove a true statement and break Phase 4's spelling gate.

**Files:**
- `src/compiler/exposure/mod.rs` — **the four predicates now live behind this module** (`:4-8`); they and `assess_signature_exposure_allowance` (`policy.rs:503` after Phase 7) return `Option<VisibilityReach>`
- `src/compiler/exposure/visitor.rs` — `public_item_name` (`:62-87`) must widen beyond `Visibility::Public(_)`; retain the resolved exposing item
- `src/compiler/exposure/detect.rs` — recursion carries a reach, not a `bool`: `type_is_exposed_outside_parent` (`:315`, already taking `item_def_id: LocalDefId`), `module_signature_exposes_item`'s `exposing_item_def_id` (`:112`) and `self_type_def_id` (`:140`)
- `src/compiler/visibility/policy.rs` — `assess_signature_exposure_allowance` (`:503` after Phase 7); `has_signature_exposure_allowance` (`:694` after Phase 7) now takes four params `(ctx, item_def_id, file_path, item_name)`. **`forbidden_pub_crate_suggestion` is now `fn(module_location, signature_exposure, parent_facade_reach, boundary_path: Option<&str>) -> String` at `:251`, still taking `SignatureExposure`, so retiring that type reaches it. The spelling-gated `Super` split and its E0364 compile claim must survive the retirement — the arms are now `:258-300` and their unit tests `:868-925`.** (This replaces the old "Do not edit `:224-234`" instruction, whose line numbers no longer exist.)
- `src/compiler/visibility/scan/classify.rs` — retire `SignatureExposure` (`:27`)
- `src/compiler/visibility/scan/record.rs` — consume the exposure reach at the shared forbidden-visibility and exact-boundary decision points; current call sites are `record.rs:165` and `:306`, and exact-boundary acceptance is in the `record.rs:414-435` region
- `src/compiler/visibility/annotation.rs` — source of `VisibilityReach`, `join`, and the free `anchored(reach, target, tcx)`. **This phase — not Phase 5 — is the first genuine consumer outside `src/compiler/visibility/`**, because `src/compiler/exposure/` is a sibling module while Phase 4 put the chain machinery inside `visibility/use_sites.rs`. Use the house pattern: bare `pub` on the items inside this private module, plus `pub(super) use annotation::{VisibilityReach, anchored};` in `src/compiler/visibility/mod.rs`. Precedent: `src/compiler/facade/exports.rs` pairs bare-`pub` items with `pub(super) use` lines in `facade/mod.rs`. **Phase 7 landed boundary acceptance, so `pub(in crate::compiler)` is now a legal spelling in this crate and the former prohibition on it is void.** Phase 7 itself uses it at `policy.rs:309, 328, 335, 355, 361` (`NoFacadeAdvice`, `no_facade_suggestion`, `classify_no_facade_callers`, `parent_scope_def_path`, `canonical_pub_in_boundary`), paired with `pub(super) use` re-exports in `visibility/mod.rs:9-13`, and the self-policy gate is green. Either shape passes: an exact `pub(in ...)` annotation behind a `pub(super) use` facade in `visibility/mod.rs`, or the bare-`pub` + `pub(super) use` house pattern above. What is still rejected is a `pub(in ...)` whose path is not the exact facade boundary
- `src/compiler/visibility/mod.rs` — the `pub(super) use` re-exports above. **The `#[expect(dead_code)]` on `mod annotation;` is already gone — Phase 7 removed it and `src/compiler/visibility/mod.rs:1` is now a bare `mod annotation;`. No action here.**
- `src/compiler/visibility/field.rs` — awareness surface for the shared classifier; runtime edits are expected only if retaining the field's existing behavior requires plumbing the new reach
- `tests/diagnostics/allowances.rs` — exposure fixtures, including explicit struct and union field regressions
- `tests/diagnostics/rendering.rs` — preserve both 16-finding hard gates if the changed classifier moves which fixture emits a diagnostic

**Constraints from prior phases:** Phase 1 supplies `VisibilityReach::join` and the free fn `anchored(reach, target, tcx)`, both in `src/compiler/visibility/annotation.rs` and both shipped `pub(super)` — see **Files** for how this phase reaches them from `src/compiler/exposure/`. `VisibilityReach` derives only `Clone, Copy`: it has no `Debug`, no `PartialEq`, and deliberately no `Ord`/`PartialOrd`, so a struct holding one cannot `#[derive(Debug, PartialEq)]` and an equality test must be spelled `lhs.compare(rhs, tcx) == Some(Ordering::Equal)`. `compare` returning `None` means two restricted scopes are incomparable siblings — not an error; feed such pairs to `join`, which returns their nearest common ancestor. Phase 5 supplies the chain-required reach to join against. Phase 7 shipped with the conservative `(Present, _)`-first arm ordering, which this phase replaces; its acceptance behavior for non-exposed items must not regress. Phase 8 added `no_facade_headline`; if this phase changes `NoFacadeAdvice`, it must update headline and suggestion through the shared selectors. **Phase 3 added a field entry point into this analysis.** Current anchors are `assess_signature_exposure_allowance` at `policy.rs:515`, `has_signature_exposure_allowance` at `policy.rs:706`, and its shared call sites at `record.rs:165` and `:306`. Struct/union fields reach that classifier with `item.name = Some(<field name>)`; preserve that behavior explicitly.

**Redesign checkpoint (2026-08-02):** Cross-target reconciliation is a separate typed-fact stage, not an extension of `StoredFinding`. Each analyzed declaration stores its source identity, declaration identity, declared reach, signature requirement, facade state, exact-boundary eligibility, caller-reconciliation mode, and accepted/finding outcome. Accepted outcomes never enter `findings`. Target facts combine through one associative, commutative, idempotent union; restricted reaches combine through their nearest-common-ancestor join. The final diagnostic is rendered only after that merge. Schema 21 stores these facts in a dedicated report collection and removes all marker encodings from `narrower_scope_def_path`; persisted headlines and suggestions are presentation inputs only and are never parsed back into decisions. Algebra tests cover union and reach laws, while end-to-end fixtures retain the exposure, field, facade, blocker, and caller behaviors.

**Transfer checkpoint (2026-08-02; safe Ultra → High handoff):** The architecture above is complete and must not be redesigned during closeout. Production code contains no accepted/forbidden marker encoding and no parser that reconstructs decisions from rendered advice; the two marker strings remaining in `tests/diagnostics/allowances.rs` are negative assertions. `cargo check --tests`, `cargo clippy --all-targets`, and the full 540-test `cargo nextest` run are green. The four named delegate gates are also green: `verify.sh check cargo-mend`, `verify.sh test cargo-mend`, `verify.sh lint cargo-mend`, and `verify.sh test cargo-mend diagnostics` (195 unit tests and 334 diagnostics tests in the named runs).

The High-tier closeout is mechanical and bounded:

1. Run the Orchestrator smoke exactly as specified below: inspect `target/mend-findings`, remove only that generated directory, force a cargo-mend recompile, then run `RUSTC_BOOTSTRAP=1 cargo +stable run --release -- --workspace --all-targets --fail-on-warn` and require `No findings`.
2. If the smoke exposes a local style finding, repair only that finding and rerun the complete named delegate gate set plus the smoke. Do not add persisted marker fields, parse a headline or suggestion, broaden `StoredFinding`, or rebuild cross-target logic in `load.rs`.
3. Confirm the legacy-marker search still finds only the two negative test assertions; inspect the final diff for unrelated changes and accidental test weakening.
4. Append the Phase 9 retrospective and change the phase status to `done` only after every gate above is green. Then pause for the next plan-phase decision.

Return to Ultra only if a failing regression disproves the typed constraint model or the associative/commutative/idempotent merge law. Ordinary compile, lint, fixture, documentation, and smoke-test corrections stay on High. After the smoke and rerun gates are green, Medium is sufficient for the retrospective, diff accounting, and phase closeout.

**High → Medium checkpoint (2026-08-02):** The bounded self-policy repairs are complete. A forced-recompile orchestrator smoke reports `No findings`; `verify.sh check cargo-mend`, `verify.sh test cargo-mend` (195/195), `verify.sh lint cargo-mend`, and `verify.sh test cargo-mend diagnostics` (334/334) all pass on the repaired source. The legacy-marker search finds only the two negative assertions in `tests/diagnostics/allowances.rs`, and `git diff --check` passes. Medium may now own the remaining closeout: run `plan-phase_review`, append the Phase 9 retrospective and any resulting remaining-phase corrections, perform final diff accounting, then change this phase status to `done` and pause. Do not reopen the typed constraint architecture unless new evidence breaks its merge law or a behavioral regression appears.

**Delegate acceptance gate:** `verify.sh check`, `verify.sh test`, `verify.sh lint`, and `bash ~/.claude/scripts/delegate/verify.sh test cargo-mend diagnostics` green. Fixtures must distinguish equal, narrower, sibling, and public exposure reaches; explicit struct and union field cases preserve current behavior. The `video_plane` snippet already passes after Phase 7, so it is regression coverage rather than proof of this phase's new behavior. `tests/diagnostics/rendering.rs` keeps both 16-finding gates at the current `:409` and `:633`, with the missing-code panic at `:237`.

**Orchestrator smoke:** clear `target/mend-findings`, force a cargo-mend recompile, then run `RUSTC_BOOTSTRAP=1 cargo +stable run --release -- --workspace --all-targets --fail-on-warn`; require `No findings`.

### Retrospective

**What worked:**
- Retiring `SignatureExposure::{Present, Absent}` for a reach that joins with the facade requirement removed the false "a narrower modifier would not compile" premise with no regression in acceptance for non-exposed items.
- The typed-fact model held: each analyzed declaration stores its own facts, the diagnostic renders only after the cross-target merge, and no marker encoding or advice parser survives in production code. The only two legacy marker strings left are negative assertions in `tests/diagnostics/allowances.rs`.

**What deviated from the plan:**
- Cross-target reconciliation became a fourth, separate stage (`src/compiler/persistence/visibility_constraint.rs`, new) rather than an extension of `StoredFinding`. The Work Order's "Redesign checkpoint" records the turn.
- A consolidation pass ran after the acceptance gates were green, since the phase's growth was disproportionate to its scope. It removed five `Option`-clone wrapper enums from `exposure/detect.rs`, deduplicated `def_path_is_descendant` down to one copy, collapsed the triplicated repair classifier, and folded the repeated suggestion phrasing into `policy::consider_using`. Phase `src/` diff went from roughly +3,251 lines to +2,476 net.

**Surprises:**
- **The four reconciliation stages partition cleanly by diagnostic code; none is redundant.** `intersection` owns `SuspiciousPub`, `UnusedPub`, `InternalParentPubUseFacade`, `NarrowToPubCrate`, `FieldVisibilityWiderThanType`; `caller_aware` owns everything *except* the two forbidden codes (it early-returns on them); `visibility_priority` owns `UnusedPub` suppressing narrowing at one location; `visibility_constraint` owns *only* `ForbiddenPubCrate` and `ForbiddenPubInCrate`. `caller_aware` additionally **builds the `CallerMap` that `reconcile_visibility_constraints` consumes** (`persistence/load.rs:117`) — a dependency, not a leftover. Retiring the three older stages yields ~0 lines.
- **`VisibilityConstraintGroup::render` overwrites `message`/`suggestion` only on the caller-reconciled branch.** Its `public_candidate()` and `blocker_candidate()` branches return the in-pass finding *unchanged*, so removing the in-pass advice composition would blank those two diagnostics. Any future "stop composing advice in-pass" cleanup must restructure `render` first.
- **Build + test + clippy green is not sufficient.** With all four `verify.sh` gates passing, the forced-recompile self-policy smoke caught three violations of this crate's own visibility policy: a `pub(in crate::compiler)` outside an exact facade boundary, and two bare-function imports where the crate requires module-qualified form. No unit or integration gate can see these.
- **`real_file_path` (`exposure/detect.rs:1290`) already canonicalizes**, so two resolver loops were calling `fs::canonicalize` a second time per iteration on an already-canonical path.

**Implications for remaining phases:**
- Phase 10's defect-1 instruction to unify `def_path_is_descendant` is **already satisfied** — one copy now lives at `visibility/policy.rs`, re-exported through `visibility/mod.rs`; the extra `scope.is_empty()` arm moved up into `classify_no_facade_callers` as two explicit checks.
- Phase 10's defect 1 (fix facts copied after suppression) now has **four** suppression stages to prune after, not three.
- **Four reconciliation mechanisms are the accepted shape — decided, not defaulted into.** See "Why cross-target reconciliation stays four stages" below. No phase is scheduled to converge them.
- Every remaining phase's acceptance gate must keep the orchestrator smoke as a *separate* requirement from the `verify.sh` set, for the reason recorded under Surprises.

### Why cross-target reconciliation stays four stages

**Decided 2026-08-02. Do not open this as a consolidation phase without new evidence.**

`reconcile_cross_target_reports` (`persistence/load.rs:114-119`) runs four stages in order: `intersection`, `caller_aware`, `visibility_constraint`, `visibility_priority`. Phase 9 added the third and it is the better design — it stores typed facts per declaration, merges across targets, and renders once — but it covers only `ForbiddenPubCrate` and `ForbiddenPubInCrate`. The other five codes still run through the older stages.

The two kinds differ in what verdict they can reach. The older stages **filter finished findings**: each target has already rendered message and suggestion text, and the stage `retain`s a subset. `intersection` is the clearest case —

```rust
reports[idx].findings.retain(|finding| {
    let key = finding_intersection_key(finding);   // (code, path, line, column)
    emission_count.get(&key).copied().unwrap_or(0) == group_size
});
```

— keep or drop, nothing else. The new stage **combines facts**, which is why it had to exist: reconciling a `pub(in ...)` boundary across targets can require the nearest common ancestor of two targets' boundaries, a value neither target produced. A filter cannot express that.

Converging the remaining five codes onto the fact model was considered and **rejected for now**, on three obstacles:

1. **`intersection`'s rule is the opposite operation.** It accepts a finding only when *every* target emitted it. The fact model merges by idempotent union — present in *any*. Converging it means the merge algebra grows a second combining rule, and Phase 9's merge-law tests cover only the first.
2. **`caller_aware` cannot simply be deleted.** It builds the `CallerMap` that `reconcile_visibility_constraints` consumes (`load.rs:116-117`). Its map-building has to move somewhere before the stage can go.
3. **`visibility_priority` is not per-declaration facts at all.** It is a precedence rule *between different diagnostic codes at one source location* — an `UnusedPub` finding suppresses `NarrowToPubCrate` / `SuspiciousPub` at the same line. It likely survives convergence untouched.

So the realistic ceiling is four stages → two, and only `intersection`'s five codes actually move. That payoff is not established, and retiring the older stages yields ~0 lines on its own. Four stages is the accepted shape until something forces the question.

### Phase 9 Review

- **Phase 12** — corrected every stale anchor (`classify_suspicious_pub` `:50-66` → `:80-177`, `assess_parent_facade_usage` `:465` → `:566`, the Required-mode regression test `:137` → `:310`); refreshed the conversion-cost baseline from 175 re-export lines to 206 re-export lines plus 46 already-conforming `pub(in crate::` declarations; recorded that Phase 9's prerequisite is now satisfied and by what mechanism; added a constraint that Phase 9's `ExposedByOtherCrateVisibleSignature` allowance is *not* gated on `required_path`, an implicit invariant Required mode would break if widened past `VisibilitySyntax::Public`.
- **Phase 12** — *(user decision, approved)* added a typed-signature repair to the Work Order: `required_pub_in_path` returns `Option<VisibilityReach>` whose payload no caller reads, and it sits among three other same-typed reach values. The phase is no longer "no `src/` edit expected"; the gate requires every existing test to pass unmodified.
- **Phase 11** — removed the instruction to derive the fix replacement from the rendered suggestion, which would have reintroduced the advice parser Phase 9's Redesign checkpoint bans; the replacement now comes from the typed `narrower_scope_def_path`. Corrected the plumbing paragraph, which named one field where three are needed. Corrected `FINDINGS_SCHEMA_VERSION` 19 → 21 (a bump here is 22), the `canonical_pub_in_boundary` caller list (one caller, not two), `record.rs` anchors (the file grew 671 → 2032 lines), `pub_use_fixes/scan.rs` anchors, and the `src/compiler/fixes/` → `src/fixes/` path error. Dropped the `def_path_is_descendant` unification item — already done.
- **Phase 11** — sharpened its first pending decision: the fixer's `visibility_annotation` is a three-state decision (bare `pub` / already-restricted / not captured), so a bare `Option<String>` conflates two of them; the `StoredFinding → Finding` conversion is the boundary to convert at.
- **Phase 10** — its splitting decision gained a concrete insertion point that did not exist when it was written: all four suppression stages run inside `reconcile_cross_target_reports`, which executes before the fix-fact copies, so "prune once after suppression" has one home. That decision is now resolved — see the reorder-and-split entry below.
- *(user decision, approved)* Converging the four reconciliation stages was considered and **rejected** — recorded above with the three obstacles, so future passes do not relitigate it. No new reconciliation phase was added.
- *(user decision, approved 2026-08-02)* **Reordered and split — the plan now has 12 phases.** The auto-fix work moved ahead of `required` mode (both Work Orders already stated that dependency in prose while the numbering contradicted it) and was then split in two, because the pub-use defects are correctness fixes in code that writes to the user's source files and deserve their own review diff, while the restricted-annotation fixer is additive. Old Phase 11 → **Phase 10** (pub-use fix integrity: prune stored fix facts once at the `reconcile_cross_target_reports` boundary, plus the two pub-use scan defects) and **Phase 11** (shared annotation spans, reporting plumbing, runner routing, and the branch-limited restricted fixer). Old Phase 10 (`required` mode) → **Phase 12**. No `done` phase was renumbered, so every checkpoint commit message still matches its phase. Phase 10 carries no pending decisions. Phase 11 keeps the `FixSupport` variant / schema-bump / typed-field decision. Phase 12 keeps the Hana-split and default-flip halves of its original decision.

---

### Phase 10 — Pub-use fix integrity · status: done

#### Work Order

**Goal:** no stored fix fact survives the suppression of its finding, and the pub-use fixer never advertises an edit it will then skip.

**Spec:**

Three defects in the auto-fix path. All three sit in code that writes to the user's source files, they share one review surface, and none of them depends on the restricted-annotation fixer in Phase 11.

*Defect 1 — a suppressed finding still leaves its stored fix fact behind,* so `--fix pub-use` can apply a narrowing the cross-target analysis already rejected. **Four** stages can remove findings before the facts are copied: `intersection`, `caller_aware`, `visibility_priority`, and Phase 9's `visibility_constraint`. All four run inside `reconcile_cross_target_reports` (`load.rs:113-119`), which executes at `:82` — *before* the unconditional fact copies at `:89-98` and `:243-244`. **Prune once at that single boundary; do not add a pruning step to each of the four stages.** The surviving finding set is established the moment `reconcile_cross_target_reports` returns, so one pass over the stored facts keyed against surviving findings is the entire fix. (This defect was previously recorded against `src/fixes/pub_use_fixes/` paths that do not exist — that directory holds only `mod.rs`, `parent_boundary.rs`, `scan.rs`, `validated_plan.rs`.)

*Defect 2 — `pub_use_fixes/scan.rs` requires the literal `"pub "`,* so a declaration written `pub\nstruct Thing` is advertised as fixable and then silently skipped. The literal is `line_contains_plain_pub` at `scan.rs:234`, reached from `screen_candidate` at `:173`. Detect the bare-`pub` declaration without depending on a trailing space on the same physical line. **This stays a bare-only check.** It must not begin accepting `pub(` — that acceptance question belongs to Phase 11's shared span parser, and widening it here would make the pub-use fixer start touching restricted annotations, which is the exact regression Phase 11's Spec is written to prevent.

*Defect 3 — `pub_use_fixes/parent_boundary.rs:95` selects the wrong occurrence when two parent `use` declarations share a source line.* Select the intended facade occurrence rather than the first textual match.

**Files:**
- `src/compiler/persistence/load.rs` — the single pruning boundary: `reconcile_cross_target_reports` (`:113-119`) is called at `:82`; the unconditional fact copies are at `:70-71`, `:89-98`, and `:232-244`
- `src/compiler/persistence/{caller_aware.rs, intersection.rs, visibility_priority.rs, visibility_constraint.rs}` — read-only for this phase unless the surviving-finding key needs a helper. The pruning belongs in `load.rs`, not distributed across the four stages
- `src/fixes/pub_use_fixes/scan.rs` — `line_contains_plain_pub` (`:234`), reached from `screen_candidate` (`:173`) — defect 2
- `src/fixes/pub_use_fixes/parent_boundary.rs` — same-line occurrence selection (`:95`) — defect 3
- `tests/diagnostics/pub_use_fixes.rs` — regressions for all three: a suppressed finding leaves no applicable stored fix fact; `pub` alone on a line either fixes correctly or reports no fix, never a silent skip; two parent `use` declarations on one line edit the intended one

**Constraints from prior phases:** Phase 7 set the stale-facade finding to `FixSupport::None` **and** suppressed its `StoredPubUseFixFact` — that pairing is the existing guard and must stay intact. Phase 9 added the fourth suppression stage and unified `def_path_is_descendant` at `policy.rs:514`, re-exported through `visibility/mod.rs`; **there is exactly one copy — do not go looking for a second.** `FINDINGS_SCHEMA_VERSION` is `21` and **this phase does not bump it**: pruning removes facts that should never have been written and changes no emitted finding shape. Phase 8's shared headline selector is orthogonal and must remain intact.

**Delegate acceptance gate:** `verify.sh check`, `verify.sh test`, `verify.sh lint`, and `bash ~/.claude/scripts/delegate/verify.sh test cargo-mend diagnostics` green. Suppressed findings leave no applicable stored fix fact. A bare `pub` on its own line is never advertised as fixable and then skipped. Both same-line facade occurrences are pinned. Both 16-finding rendering gates unchanged, and `FINDINGS_SCHEMA_VERSION` stays `21`.

**Orchestrator smoke:** clear `target/mend-findings`, force a cargo-mend recompile, then run `RUSTC_BOOTSTRAP=1 cargo +stable run --release -- --workspace --all-targets --fail-on-warn`; require `No findings`.

### Retrospective

**What worked:** The plan's insistence on pruning once at the `reconcile_cross_target_reports` boundary held — one pass in `load.rs` covers all four suppression stages, and none of the four needed editing. Defects 2 and 3 were independent and landed without interacting.

**What deviated from the plan:** Pruning has to join a surviving finding to its `StoredPubUseFixFact`, and the only link between them is `StoredFinding::item` — a rendered `"{kind_label} {name}"` display string built ad-hoc at five call sites. The join was made safe by centralizing that format into `StoredFinding::render_item` and its exact inverse `StoredFinding::item_name`, which pulled in `visibility/field.rs` and `visibility/scan/record.rs`, two files the Work Order's **Files** list did not name.

**Surprises:**
- `StoredFinding::item` is presentation text doing structural duty. Nothing declared it a join key, so a change to any one of the five formatting sites would have silently broken pruning. The `render_item`/`item_name` pairing is now the only sanctioned way to cross that format.
- `reconcile_visibility_constraints` in `visibility_constraint.rs` carried two superlinear scans: constraints × findings when matching, and findings × reconciled-keys when retaining. Both existed because `VisibilityConstraintKey::matches_finding` compared exactly the five components that constitute the key, so a scan was doing by comparison what a lookup could do by equality. Replaced with `VisibilityConstraintKey::for_finding` plus a `BTreeMap` index and a `BTreeSet::contains`; `or_insert` preserves the old `.find()` first-wins semantics. Test and diagnostic counts were identical before and after, which is what an equivalence-preserving change should produce.
- That fix is correct but small in production terms. cargo-mend's own timing line separates the two halves, and on a full Bevy workspace it reads `check: 8479.75s, mend: 0.82s` — the reconciliation the fix targets is under a second, while the `cargo check` cargo-mend shells out to is hours. Analysis cost is not where this tool's wall-clock lives.

**Implications for remaining phases:**
- Phase 11 plumbs `visibility_annotation` through the same persistence boundary. `render_item`/`item_name` is the precedent to follow: when a stringly format has to be crossed in both directions, define the pair together so the inverse cannot drift.
- `FINDINGS_SCHEMA_VERSION` stayed `21` as planned. Phase 11 still owns the bump to `22`.
- Nothing in this phase changed the fixer's public surface, so Phase 11's and Phase 12's pending decisions are unaffected.

---

### Phase 11 — Auto-fix for restricted annotations · status: done

#### Work Order

**Goal:** `cargo mend --fix` rewrites a bare `pub` to the exact annotation, and no fixer silently no-ops on a restricted annotation.

**Spec:**

**This paragraph used to mis-describe `unused_pub.rs`, and following it as written would be a behavior regression. Corrected:** only `fixes/narrow_pub_crate.rs:32` does the raw string search (`line_text.find("pub ")`). `fixes/unused_pub.rs` already uses a purpose-built `bare_pub_annotation_byte_len` (`:36`, defined `:56-62`) that **deliberately refuses `pub(`** — that refusal is intended behavior, not an oversight. Swapping it for `fixes/field_visibility.rs`'s `visibility_annotation_byte_len` (`:89-101`), which accepts `pub(crate)` and `pub(in ...)`, would make `cargo mend --fix` start **stripping restricted visibility annotations** from `unused_pub` findings — annotations it currently, correctly, leaves alone.

So: share the annotation-span parser at `fixes/field_visibility.rs:89-101` so a restricted annotation's span is *computed* rather than string-matched, but **keep an explicit bare-only gate at the `unused_pub` call site.** Sharing the parser is the goal; sharing the acceptance policy is the bug.

**There is a third annotation parser and it must stay separate.** Phase 7 added `canonical_pub_in_boundary`, a `proc_macro2::TokenStream` walk that resolves a written `pub(in ...)` path to a `crate::`-rooted boundary. **After Phase 9 it has exactly one caller, `record.rs:1332`; `load.rs` no longer calls it at all.** It answers a different question from the other two — *which module does this path name*, versus `fixes/field_visibility.rs:89` (*how many bytes is the annotation*) and `annotation.rs` (*which `VisibilitySyntax` is this*). Do not fold it into the shared span parser.

This deliberately does not reuse Phase 1's `annotation.rs` parser, and that is not an oversight: `visibility_annotation_byte_len` answers "how many bytes of this raw source line does the visibility annotation occupy", while `annotation.rs:103-148` uses `syn` to answer "which of the eight visibility forms is this". Different questions, and the `fixes/` tree cannot see `pub(super)` items inside `compiler/visibility/` anyway.

Then wire `suspicious_pub`'s facade arm from `FixSupport::None` to `FixSupport::RestrictedAnnotation` so the fix rewrites `pub` to the exact annotation. The fix must also confirm the facade line itself needs no edit before applying — that is why this is last.

**The new variant is `FixSupport::RestrictedAnnotation`** (`src/reporting/diagnostics.rs:15-31`), named for its fixer like every other variant. An earlier draft of this Work Order said `FixSupport::Standard`; that variant does not exist on `FixSupport` — `Standard` is a `FixSummaryBucket` variant (`:33-36`) serving end-of-run summary grouping — so following the old wording would not compile. The variant needs a serde name, arms in `note()` (`:39`) and `summary_bucket()` (`:53`), the mirrored variant in `tests/support/diagnostics.rs:60-85`, and an applier registered in the `DiagnosticCode` routing table.

**Bump `FINDINGS_SCHEMA_VERSION` (`src/compiler/constants.rs:65`) from `21` to `22`.** The new variant is persisted as the `fixability` field on every stored finding, and the plan's invariant is that a change to emitted findings bumps the version. The bump invalidates caches, which are then recomputed; nothing else depends on it.

Fix eligibility is branch-specific. Only a Required-mode finding produced from bare `pub` with a resolved exact restricted boundary is eligible for automatic replacement. A finding whose input is already `pub(in ...)`, including a stale boundary whose repair also removes or changes a facade, remains non-fixable. The span must cover the complete annotation even when it is multiline; otherwise report no fix.

**The replacement text must come from a typed field, never from the rendered suggestion.** An earlier draft of this Work Order said "the replacement comes from the finding's exact computed suggestion". That is forbidden: the suggestion is free text built by `policy::consider_using` (`policy.rs:413`, `format!("consider using: \`{visibility}\`")`), and reading a replacement out of it means parsing rendered advice back into a decision — precisely what Phase 9's Redesign checkpoint bans ("persisted headlines and suggestions are presentation inputs only and are never parsed back into decisions") and what its Retrospective records as removed from production code. The string now has **two** producers, since `persistence/visibility_constraint.rs:330` composes it inline (`policy::consider_using` is `pub(super)` and unreachable from `src/compiler/persistence/`), so a parser would have to track both.

The typed replacement target already exists and only needs plumbing: `SuspiciousPubAdvice::Narrowing { narrower_scope_def_path }` (`record.rs:1726-1730`) is set to the stripped exact boundary at `record.rs:1953-1955` and persists as `StoredFinding.narrower_scope_def_path` (`schema.rs:78`). The fixer reads that.

Add `--fix` assertions to the stale-annotation tests: the fixer either edits correctly or reports no fix, never a silent no-op.

**Files:**
- `src/fixes/field_visibility.rs` — extract `visibility_annotation_byte_len` (`:89-101`) into a shared helper
- `src/fixes/unused_pub.rs` — use the shared parser at `:36` but **keep the bare-only gate** that `bare_pub_annotation_byte_len` (`:56-62`) provides today
- `src/fixes/narrow_pub_crate.rs` — replace the raw `line_text.find("pub ")` at `:32`
- `src/fixes/restricted_annotation.rs` (new) and `src/fixes/mod.rs` — restricted-annotation scan and module wiring
- `src/config/run_mode.rs` — add the purpose-specific `FixKind` and include it in the relevant CLI fix selections
- `src/fixes/runner/{plan,mend_runner,combine,notices}.rs` — plan, combine, apply, and user-notice plumbing for the new fix kind
- `src/reporting/diagnostics.rs` — the new `FixSupport` variant (`:14-30`) and the three fields on `Finding` (`:210-225`)
- `src/compiler/persistence/{load.rs, schema.rs}` — the `StoredFinding → Finding` conversion carries the three fields; `constants.rs:65` holds `FINDINGS_SCHEMA_VERSION`
- `src/compiler/visibility/scan/record.rs` — `suspicious_pub` fix support. **`record.rs` grew 671 → 2032 lines in Phase 9; the anchors moved ~1200 lines.** Current: `maybe_record_suspicious_pub` at `:1663`, the `StoredPubUseFixFact` write at `:1884`, `SuspiciousPubAdvice::Narrowing` at `:1726-1730` set at `:1953-1955`. Prefer the symbol names over the numbers
- `tests/diagnostics/unused_pub.rs`, `tests/diagnostics/narrow_pub_crate.rs` — `--fix` assertions, including a regression test that `--fix` does **not** strip a restricted annotation from an `unused_pub` finding
- `tests/support/{diagnostics,report}.rs` — mirror the new fixability and finding fields used by diagnostics fixtures

**Three fields do not reach the fixers yet — this phase must plumb all three.** Phase 7's Retrospective requires the auto-fix to read `Finding::visibility_annotation` rather than `source_line`, because `source_line` holds one physical line and cannot represent a multiline `pub(\n    in crate::a\n)`. The paragraph that used to sit here named only that one field, which is half the plumbing: the fixer also needs `narrower_scope_def_path` for the replacement text (see the typed-field rule above), and `item_def_path` to key the finding.

All three exist on `StoredFinding` (`src/compiler/persistence/schema.rs` — `visibility_annotation` `:68`, `narrower_scope_def_path` `:78`) and on `FindingParams` (`src/compiler/visibility/scan/finding_params.rs`). **None of the three exists on the `Finding` the fixers actually consume** (`src/reporting/diagnostics.rs:210-225`, reached via `scan_from_report(report: &Report)` at `src/fixes/unused_pub.rs:15` and `src/fixes/narrow_pub_crate.rs:15` — note `src/fixes/`, not `src/compiler/fixes/`). Add them to `reporting::Finding`, to the `StoredFinding → Finding` conversion in `persistence/load.rs` (the `findings.push(Finding { … })` at `:282-297`, which today lists twelve fields and none of these three), and to the test mirror at `tests/support/report.rs:7-21`.

**All three stay internal.** `reporting::Finding` is serializable, so each added field carries explicit serde skip attributes. The fixer needs them in-process; nothing outside cargo-mend reads them, and publishing them would create a JSON contract with no consumer.

**`visibility_annotation` becomes a named three-state enum on `reporting::Finding`, not an `Option<String>`.** The fixer's decision is three-way — bare `pub` (rewritable), already-restricted (never fixable, per the branch-specific eligibility rule above), and nothing recorded (report no fix). An `Option<String>` holds two states, so `None` conflates "no annotation" with "not captured" and `Some(s)` forces the fixer to re-parse `s` to recover a distinction the compiler already had. That re-parse is the live hazard this phase exists to remove: `fixes/unused_pub.rs:56-62` deliberately refuses `pub(` while `fixes/field_visibility.rs:89-101` deliberately accepts it, so a fixer that picks the wrong parser starts stripping restricted annotations off source. Convert at the `StoredFinding → Finding` boundary in `persistence/load.rs`, where the producing side still knows which state it recorded, and let the fixer match on variants. `StoredFinding.visibility_annotation` keeps its `Option<String>` wire shape (`schema.rs:73`) — this is a conversion, not a schema change.

**Constraints from prior phases:** Phase 7 set the stale-facade finding to `FixSupport::None` **and** suppressed its `StoredPubUseFixFact`; this phase re-enables only the bare-`pub` exact-boundary branch while keeping restricted-input findings and the pub-use fixer out of that path. Phase 7 already makes Required mode produce these findings, so this phase has no dependency on Phase 12 — that is why both fix phases now run first, so the Required-mode migration can be applied by `--fix` instead of by hand. Phase 8's shared headline selector is orthogonal and must remain intact.

**From Phase 9:** persisted headlines and suggestions are presentation inputs only and are **never** parsed back into decisions — this phase must not reintroduce an advice parser (see the typed-field rule in the Spec). `def_path_is_descendant` is already unified at `policy.rs:514`. `record.rs` is now 2032 lines and its anchors moved ~1200 lines — prefer symbol names over `:NNN`. `policy::consider_using` (`policy.rs:413`) is `pub(super)` and therefore unreachable from `src/compiler/persistence/`, which is why `visibility_constraint.rs:330` composes the same `consider using:` literal inline; if this phase needs one producer of that string, widening `consider_using`'s visibility is in scope, but the fixer must not depend on the string either way.

**From Phase 10:** the three pub-use defects are already fixed — stored fix facts are pruned once at the `reconcile_cross_target_reports` boundary in `load.rs`, `line_contains_plain_pub` no longer requires a trailing space, and `parent_boundary.rs` selects the intended same-line occurrence. **Do not re-open any of them.** Phase 10 deliberately left `pub_use_fixes/scan.rs` accepting bare `pub` only; the shared span parser this phase extracts is for `field_visibility`/`unused_pub`/`narrow_pub_crate` and must not be wired into the pub-use scan. `FINDINGS_SCHEMA_VERSION` is still `21` — Phase 10 did not bump it — so a bump here is `22`.

**Delegate acceptance gate:** `verify.sh check`, `verify.sh test`, `verify.sh lint`, and `bash ~/.claude/scripts/delegate/verify.sh test cargo-mend diagnostics` green. A Required-mode bare-`pub` fixture rewrites only the full annotation to the exact boundary and leaves the facade untouched; every restricted-input finding reports no fix; multiline annotations are pinned. `--fix` does not strip a restricted annotation from an `unused_pub` finding. Both 16-finding rendering gates unchanged, and `FINDINGS_SCHEMA_VERSION` is `22`.

**Orchestrator smoke:** clear `target/mend-findings`, force a cargo-mend recompile, then run `RUSTC_BOOTSTRAP=1 cargo +stable run --release -- --workspace --all-targets --fail-on-warn`; require `No findings`.

### Retrospective

**What worked:**
- The shared annotation-span parser landed as `visibility_annotation_site::locate` and all three fixers adopted it, while `unused_pub.rs` and `narrow_pub_crate.rs` each kept their own explicit bare-only gate at their own call site — the split the Spec insisted on ("sharing the parser is the goal; sharing the acceptance policy is the bug") held under review.
- `NarrowerScope` as a typed three-state enum on `reporting::Finding` removed the re-parse hazard outright: the fixer destructures `NarrowerScope::ExactBoundary(..)` instead of re-checking `fix_support`, and `StoredFinding` kept its `Option<String>` wire shape, so `FINDINGS_SCHEMA_VERSION` bumped 21 → 22 exactly once.
- `--fix` was proven end-to-end with the real release binary on a Required-mode fixture, not only in tests: `pub fn exact() {}` → `pub(in crate::a) fn exact() {}` with the `pub(super) use` facade line byte-identical.

**What deviated from the plan:**
- The Work Order specified three flat fields on `Finding`; **two** shipped, grouped into one `ItemVisibility { written, narrower_scope }` value (`src/reporting/diagnostics.rs:268-272`). The third, `item_def_path`, never reached `reporting::Finding` at all — the fixer keys each site by `path:byte-offset` instead (`src/fixes/restricted_annotation.rs:63`), which also collapses the lib/bin duplicate that a def-path key would have kept apart. Declared with a reason and accepted; the semantic requirement (typed, internal, `#[serde(skip)]`) is met.

**Surprises:**
- The `PubInPath::Required` clause added to `rewrites_annotation_only` (`scan/record.rs:1836-1842`) is defense-in-depth, not a live gate. The elevation that produces the finding at all — `policy::required_pub_in_path` (`policy.rs:60-78`) — is itself mode-gated, so outside `Required` nothing reaches the fixer. A closure reviewer tasked with refuting this found two further escapes (`item_name == None`, empty `occurrences.matching`) and closed both; only a non-real span survives, which no fixture can produce.
- The duplicate lib/bin fix-advertisement repair (`persistence/load.rs:221-229`) has no fixture coverage — no diagnostics fixture in this phase has both `src/lib.rs` and `src/main.rs`.
- `--fix` announces an annotation rewrite as "applied 1 import fix(es)". `runner/notices.rs:11-25` funnels every non-`pub-use` fixer into one import-fix counter and `notices.rs:102` pins that wording for `field_visibility`; the phase followed the established pattern, so the misleading noun predates it and is unresolved.
- The workspace self-scan is **not** clean and was not clean before this phase: `pub struct UseSite` (`compiler/persistence/schema.rs:39`) reports `suspicious_pub` at HEAD too, verified by building and running the pre-phase tree in a detached worktree. The two remaining `**Orchestrator smoke:**` gates demand `No findings`, which no tree in this plan has satisfied since Phase 9.

**Implications for remaining phases:**
- Phase 12 plans to change `required_pub_in_path`'s return type from `Option<VisibilityReach>` to `bool`. That is compatible, but the function must stay dependent on `PubInPath::Required`: `the_exact_boundary_rewrite_is_offered_only_under_required` (`tests/diagnostics/forbidden_pub_in_crate.rs`) proves the `forbidden` and `permitted` arms emit nothing precisely because of that check, and a repair that drops it turns three test arms red.
- `FINDINGS_SCHEMA_VERSION` is now `22`; Phase 12's constraints still carry Phase 10's "still `21`".
- Phase 12's smoke gate needs the same correction as this one: the pre-existing `UseSite` finding means `No findings` is unreachable without either fixing that declaration or restating the gate as no *new* findings.

### Phase 11 Review

An architect pass over the one remaining phase produced sixteen findings; fifteen were applied directly and one was deferred as a decision.

- **Phase 12 re-scoped to four items** — typed repair, re-derived count, default decision, CHANGELOG (plus the `UseSite` one-liner and the new fixture below). Its three behavioral gate clauses were already proven by tests shipped in Phases 7 and 11 and `policy.rs` was untouched by Phase 11, so they are now stated as a regression guard rather than as work.
- **The typed repair grew a body change.** `required_pub_in_path` decides the policy question by string-prefixing rendered output (`policy.rs:74-77`), which the return-type change alone would have left in place. Phase 12 now owns replacing that with a typed query, plus a rename that names the question instead of the setting, plus two more `Option<VisibilityReach>` values crossing the module API (`signature_exposure_reach`, `joined_visibility_requirement`).
- **The `bool` repair is pinned against regression.** Both the `PubInPath::Required` and `VisibilitySyntax::Public` checks at `policy.rs:66-72` must survive it; three test arms rest on them. Added to Phase 12's constraints.
- **The `UseSite` finding is real and Phase 12 fixes it.** The declaration is genuinely over-wide — the only `pub` type in `schema.rs` with no matching re-export, one non-test consumer. The `No findings` smoke gate stays as written rather than being weakened to "no new findings"; the resolution recorded above in Implications is superseded by this.
- **The static ~51 conversion estimate is retired** in favor of measuring `summary.fixable_with_fix` from a `Required` run.
- **A lib+bin Required-mode fixture is now Phase 12's** — the duplicate-fix-advertisement path has two independent mechanisms and zero coverage; the phase resolves the redundancy.
- **The default flip gained a sixth blast-radius item:** eight diagnostics fixture files pin no `pub_in_path` and would silently inherit `Required`.
- **Required-mode fixtures live in `forbidden_pub_in_crate.rs`**, not `allowances.rs` as this phase's Files claimed.
- **Delegation Context corrected** — schema version is `22`; the three removed annotation parsers now point at `visibility_annotation_site::locate`; `record.rs` and `policy.rs` line counts were stale by more than 1400 lines.
- **The "applied N import fix(es)" wording is now owned** by Phase 12 rather than deferred a third time.
- **Deferred to Phase 12 as a pending decision:** the `--fix` migration has no configuration path — the fixer requires a resolved `Required` config and this repo has no `mend.toml`, so neither ordering of convert-then-flip works without a third mechanism.
- **Corrected in this phase's own retrospective:** the shipped `ItemVisibility` has two fields, not three; `item_def_path` never reached `reporting::Finding`.


---

### Phase 12 — `required` mode · status: done

#### Work Order

**Goal:** `required` becomes cargo-mend's shipped default, this crate converts itself to exact `pub(in crate::path)` annotations with its own `--fix`, and the remaining visibility cleanup lands.

**Spec:**

Drop `AllowanceReason::InternalParentFacadeBoundary` for bare `pub` when the setting is `Required`. "Drop the allowance at `Required`" is too broad as stated: a conforming exact annotation still needs that allowance when its facade is used — **only bare `pub` loses it**.

**Resolved 2026-08-03 — hana is out of this plan entirely.** Fixing cargo-mend is independent of its consumers. No hana experiment, no hana conversion, no hana-local plan is owed by this phase or by this document; nothing in this plan may block on another repository. Every earlier reference to a hana migration is dead text.

**Resolved 2026-08-03 — `required` is the project's policy and this phase flips the default.** The measure-then-decide ordering the earlier drafts proposed is gone: the decision is made. What remains is mechanical — convert this crate, flip the default, and report the count that resulted.

**Resolved 2026-08-03 — cargo-mend gets no `mend.toml`. A default that its own repository must override is not a default.** Earlier drafts of this Work Order added a permanent repo-root `mend.toml` pinning `pub_in_path = "required"`. That is retracted: this crate must resolve `Required` from the shipped default like every other repository, and a config file at its root would make cargo-mend the one project that cannot rely on the policy it ships. Do **not** create `mend.toml` at the repository root, temporarily or permanently, and do **not** build a `--pub-in-path` CLI or environment override to stand in for it; if that override is wanted for its own sake it is separate work whose design must not be forced by this migration. Diagnostics **fixtures** keep their own `mend.toml` files — that requirement is unchanged and is a different thing entirely.

**Resolved 2026-08-03 — the flip migrates the value on disk, once, and `reconcile_global_config` carries the migration.** Precedence is project > global > compiled-in default (`ProjectVisibilityConfig::resolve`, `src/config/loaded.rs:42-51`), and `reconcile_global_config` (`src/config/global.rs:118-126`) has been *inserting* `pub_in_path = "permitted"` into every user's global config since 0.18. That on-disk value beats the flipped default, so a flip that refuses to touch it reaches only installs that have never run cargo-mend — which is nobody, including this repository's own developer, whose `[visibility] pub_in_path = "permitted"` sits in the global config today and would otherwise silently mask the flip and block the self-conversion. **Migrate it:** on upgrade, rewrite an on-disk `pub_in_path = "permitted"` to `"required"` and update `PUB_IN_PATH_COMMENT` above it in the same write.

**The migration runs exactly once, and a version marker is what guarantees that.** A bare "rewrite `permitted` to `required`" would re-apply on every run and make the setting unsettable — a user who deliberately chooses `permitted` would find it reverted the next time cargo-mend ran. Add a top-level `config_version` key to the global config (`src/config/constants.rs` gains its key constant alongside `PUB_IN_PATH_KEY`). A config with no `config_version` predates the flip: migrate its `pub_in_path` and stamp the current version. A config that already carries the current version is left alone no matter what its `pub_in_path` says. `create_default_global_config` writes the marker from the start, so a fresh install is never migrated. **An explicit `permitted` set after the migration survives every subsequent run** — that is the property the marker buys, and a test must pin it.

**Convert this crate with `--fix`. Never hand-convert.** Once the default is `Required` and no config overrides it, `cargo mend --fix` resolves `PubInPath::Required`, `rewrites_annotation_only` (`src/compiler/visibility/scan/record.rs:1836-1842`) grants `FixSupport::RestrictedAnnotation`, and the Phase 11 fixer rewrites every bare `pub` behind a resolved facade to its exact `pub(in crate::path)` boundary. A site the fixer declines to convert is a **finding about the fixer**, not a licence to edit the declaration by hand.

**The conversion procedure, in this order — the flip comes first.** (1) Land the five config sites and the migration, so the compiled-in default is `Required` and nothing on disk overrides it. The tree is now red against its own policy; that is expected and lasts the length of this phase. (2) Run `RUSTC_BOOTSTRAP=1 cargo +stable run --release -- --workspace --all-targets --fix` from the repository root and record the `summary.fixable_with_fix` count it reports. Confirm from the run's own output that the resolved setting was `required` before trusting a low count — a zero here means the config resolved wrong far more often than it means the tree was already converted. (3) **Re-run until the reported count reaches zero** — narrowing one declaration can change what the next pass resolves, so one pass is not assumed sufficient; if the count stops decreasing without reaching zero, stop and report it as a fixer defect. (4) `verify.sh check`, `test`, and `lint` must be green on the converted tree; **a compile error after `--fix` means the fixer emitted a wrong boundary — report it, do not repair the declaration by hand.** **This `cargo run` is an explicitly authorized exception to the "run only the listed verification commands" rule: it is implementation, not verification.**

**Do not carry any static number into dispatch. Measure it, then report what it was.** Every count that follows is context, not a target. Re-counted after Phase 11 the tree holds 209 `pub(super) use` / `pub(crate) use` re-export lines, 46 `pub(in crate::` declarations (already conforming — never part of the conversion set), and 105 bare-`pub` item declarations. **105 is an upper bound, not the conversion set:** a bare `pub` whose signature exposure genuinely requires `pub` is excluded by `joined_visibility_requirement`, and 31 exact annotations already exist in `src/` (27 `pub(in crate::compiler)`, 4 `pub(in crate::fixes)`), so the conversion is partly done by hand already. **The conversion set is exactly `summary.fixable_with_fix` from the `pub_in_path = "required"` run** — the same quantity the Phase 11 fixture asserts at `tests/diagnostics/forbidden_pub_in_crate.rs:365`. Record the measured number in the phase retrospective and in the CHANGELOG entry.

**The canonical shape, already half-converted in the tree.** `src/compiler/facade/mod.rs` re-exports two siblings with `pub(super) use`, so both resolve to exactly `crate::compiler`: `boundary.rs:22` states it (`pub(in crate::compiler) struct LogicalParentBoundary`) while `exports.rs:130` does not (`pub enum ParentFacadeSpelling`). The second is what `--fix` rewrites, to the annotation the first already carries.

**Repair the four same-typed reach values in `classify_suspicious_pub`.** Phase 9 left four `Option<VisibilityReach>` values live in this one function — `required_path`, `resolved_facade_reach`, `required_reach`, and `signature_exposure` — with different meanings and no type-level distinction between them, so a reader must trace callers to tell them apart.

`required_pub_in_path` (`policy.rs:60-78`) is the clearest case: it returns `Option<VisibilityReach>` but **its payload is never read** — all three uses are `required_path.is_none()` (`:95`, `:111`, `:151`). A predicate must not wear an `Option` to answer a yes/no question. Change its return type to `bool` (or a named two-state type if a name reads better at the call sites), and give the three surviving reach values names or wrapper types that state which reach each one is.

**The return type is the smaller half of this repair. The body contains a rendered-text sniff, and that is the real defect.** `policy.rs:74-77` decides the policy question with `required_reach?.to_source(ctx.tcx).starts_with("pub(in ")`. `to_source` (`annotation.rs:209-219`) renders `Public` → `"pub"`, `Restricted(CRATE_DEF_ID)` → `"pub(crate)"`, and every other restriction → `"pub(in crate::…)"`, so that prefix test is a stringly-typed spelling of "restricted to a non-crate boundary" — parsing rendered output back into a decision, which Phase 9 banned and Phase 11's Spec restates. Changing the signature to `bool` leaves it untouched. **Replace the sniff with a typed query on the reach itself** (the reach knows whether it is `Public`, crate-wide, or a narrower restriction without being rendered); this deliberately enlarges the repair from "signature and naming" into the function body.

**Rename it in the same edit.** `required_pub_in_path` names the setting it consults, not the question it answers. The question is "does this bare `pub` have a resolved exact boundary it must be narrowed to?" — name it for that. At the three `!…` call sites a two-state named type reads better than a bare `bool` and satisfies the Type Design Contract more directly.

**Two more `Option<VisibilityReach>` values sit on this module's API surface and belong in the same sweep:** `signature_exposure_reach` (`policy.rs:862`) returns one *out of* `policy.rs` into `record.rs`, and `joined_visibility_requirement` (`policy.rs:48-58`) takes two more as parameters. The four-value list above is exact for `classify_suspicious_pub`'s locals, but these three crossings are the domain-owned API the contract actually targets.

This is a signature-and-naming repair plus one body change, with no behavior change: the acceptance gate is that every existing test still passes unchanged.

**Files:**
- `src/compiler/visibility/policy.rs` — **the Required-mode *behavior* already shipped in Phase 7 and needs no change. This phase does carry one typed-signature repair here** (see "Repair the four same-typed reach values" below). `classify_suspicious_pub` (`policy.rs:80-177` after Phase 9) computes a `required_path` from `PubInPath::Required` + `VisibilitySyntax::Public` + a resolved chain reach, then gates `basic_suspicious_pub_allowance` (`:95`), `assess_parent_facade_usage` (now `:566`, gated at `:111`), **and** the `ShallowPrivatePolicy` allowance (`:151`) on `required_path.is_none()`. **Phase 9's new `ExposedByOtherCrateVisibleSignature` allowance (`:140-149`) is *not* gated on `required_path`** — see **Constraints from prior phases**. `required_setting_reviews_bare_pub_behind_restricted_facade` (`tests/diagnostics/forbidden_pub_in_crate.rs:310`) already asserts `SuspiciousPub` at `"required"` and silence at `"permitted"`. Beyond the typed-signature repair this file needs no behavior change; the default flip is carried by the five config sites listed below, not here
- `tests/diagnostics/forbidden_pub_in_crate.rs` — **this is where every Required-mode fixture in the tree already lives** (1276 lines, 23 `required` mentions, 40 `pub_in_path` mentions). Earlier drafts of this Work Order pointed at `tests/diagnostics/allowances.rs`, which owns none of them; following that literally would build a parallel fixture set. Extend this file. **Each fixture must pin `pub_in_path` in its own `mend.toml`** (Delegation Context invariant); an unpinned fixture inherits the developer's machine-global config
- `tests/diagnostics/forbidden_pub_in_crate.rs` — **one new lib+bin Required-mode fixture** (this phase owns it). Ten fixture files declare both `src/lib.rs` and `src/main.rs`, but none is a Required-mode fixture, so the duplicate-fix-advertisement path has no coverage at all. The fixture asserts both the advertised fix count and that exactly one edit is applied. **Two independent mechanisms currently cover this and the phase should resolve the redundancy rather than leave both unexercised:** `retain_one_restricted_annotation_fix_per_site` (`src/compiler/persistence/load.rs:221-229`) demotes the duplicate's `fix_support` to `None` *after* `extend_report_from_stored` built `item_visibility` (`:320-326`), leaving the demoted finding carrying `NarrowerScope::ExactBoundary` — a pair `NarrowerScope::resolve` can never produce — while the thing that actually prevents the double edit is the fixer's own `rewritten_sites` set (`src/fixes/restricted_annotation.rs:63`)
- `src/compiler/persistence/schema.rs:39` — **one-line fix: narrow `pub struct UseSite`.** It is the only `pub` type in `schema.rs` with no corresponding `pub(super) use` line in `src/compiler/persistence/mod.rs:10-27`, and its sole non-test consumer is `sink.rs:12` via `super::schema::UseSite`. `suspicious_pub` is correct to fire on it; this is a genuine over-wide declaration, not an analysis defect, and it is the only thing standing between this repo and the `No findings` smoke gate. Fix the declaration — do **not** weaken the gate
- `src/config/pub_in_path.rs` — move `#[default]` from `Permitted` to `Required`
- `src/config/global.rs:118-126` — `reconcile_global_config`: the inserted literal becomes `value("required")`, **and this function gains the one-time migration** — when the config carries no `config_version`, rewrite an existing `pub_in_path = "permitted"` to `"required"`, refresh `PUB_IN_PATH_COMMENT` above it, and stamp the current `config_version`. A config already at the current version is never touched. Both paths reuse the existing `inserted` write flag (rename it for what it now means) so the file is still written once
- `src/config/global.rs:147-164` — `default_global_config_toml`: the emitted literal becomes `"required"` and the emitted config carries `config_version` from the start, so a fresh install is never a migration candidate
- `src/config/constants.rs` — `PUB_IN_PATH_COMMENT` (`:11`) describes the default and must match after the flip; add the `config_version` key constant beside `PUB_IN_PATH_KEY` and the current version value
- `README.md` — the `pub_in_path` text Phase 8 wrote states `permitted` is the default
- `src/fixes/runner/notices.rs` — `import_fix_notice_count` (`:11-25`) folds every non-`pub-use` fixer into one "import fix(es)" total, so annotation rewrites are announced with the wrong noun. The pinning test at `:102` asserts the current wording and changes with it
- `tests/diagnostics/facade_subjects.rs`, `import_fixes.rs`, `imports_at_top.rs`, `inline_path_fixes.rs`, `narrow_pub_crate.rs`, `prefer_module_import.rs`, `prelude_pub_mod.rs`, `unused_pub.rs` — **the eight fixture files with zero `pub_in_path` mentions.** Pin the setting explicitly in each one's `mend.toml`, or the flip silently re-scopes them
- `src/compiler/**` — the `--fix`-driven self-conversion touches an unknown-in-advance set of declarations across the crate. **Every edit in this set is produced by `cargo mend --fix`, never typed by hand**; review the diff, do not author it
- `CHANGELOG.md` — the default flip, **the one-time global-config migration and how to opt back out** (set `pub_in_path = "permitted"` after upgrading and it sticks), the measured conversion count, and the notice-wording change

**The flip lands at five config sites, all of them owned by this phase:** `#[default]` in `src/config/pub_in_path.rs`; the literal `value("permitted")` in `reconcile_global_config` (`src/config/global.rs:118`); the literal `"permitted"` in `default_global_config_toml` (`:161`); `PUB_IN_PATH_COMMENT` (`src/config/constants.rs:11`); and the README text Phase 8 wrote. All five are **Files** entries for this phase. **The version-marked migration is a sixth change in the same two files** and is what makes the other five reach an existing install at all.

**The flip has a sixth blast-radius item the five config sites hide: every unpinned diagnostics fixture.** `facade_subjects.rs`, `import_fixes.rs`, `imports_at_top.rs`, `inline_path_fixes.rs`, `narrow_pub_crate.rs`, `prefer_module_import.rs`, `prelude_pub_mod.rs`, and `unused_pub.rs` contain zero `pub_in_path` mentions, so under a flipped compiled-in default each silently inherits `Required` on any machine whose global config lacks the key — the Delegation Context invariant about machine-global inheritance, running in reverse. **Pin `pub_in_path` explicitly in every one of those eight files as part of the flip**, or the flip lands as a fixture-wide behavior change disguised as a one-line default.

**The migration's own tests are part of this phase.** `src/config/global.rs` already carries `reconcile_global_config` unit tests, two of which assert `PubInPath::Permitted` (`:210`, `:234`) and change with the flip. Add coverage for each migration branch: an unversioned config with `permitted` migrates to `required` and gains the version stamp; an unversioned config with an explicit `forbidden` or `required` keeps its value and gains the stamp; **a versioned config with `permitted` is left exactly as found** — that last one is what proves the setting is settable and must not be omitted; and a config with no `[visibility]` table at all still ends up correct.

**Constraints from prior phases:** Phase 6 supplies `PubInPath::Required` on the resolved config. Phase 7 already supplies the Required-mode finding and its local regression. **Phase 9's prerequisite is now satisfied — the conversion set is measurable.** `required_pub_in_path` (`policy.rs:60-78`) consumes `required_reach = joined_visibility_requirement(facade_reach, signature_exposure)` (`policy.rs:48-58`, `:92-94`), so a bare `pub` whose signature exposure genuinely requires `pub` no longer produces a Required-mode finding; only declarations whose exposure is contained below the facade boundary do. Re-derive the count against this behavior, not the old boolean allowance. **Phase 9 also left one allowance ungated:** `classify_suspicious_pub` gates three allowances on `required_path.is_none()` (`:95`, `:111`, `:151`) but not `ExposedByOtherCrateVisibleSignature` (`:140-149`). The two are mutually exclusive today only because bare `pub` declares `Public` while `required_path` is `Some` only for a restricted boundary — an implicit invariant. Any widening of Required mode past `VisibilitySyntax::Public` breaks it, so gate that fourth allowance explicitly if this phase touches the classifier at all. Phase 8 documents `permitted` as the default, so flipping it requires updating every named config, test, README, changelog, and shared-style statement together. **The two auto-fix phases now ship ahead of this phase — Phase 10 (pub-use fix integrity) and Phase 11 (the restricted-annotation fixer).** Any bulk Required-mode migration must therefore be driven by `cargo mend --fix`, not by hand-converting the same workload; if the fixer cannot convert a site, that is a finding about the fixer, not a reason to edit the site manually.

**From Phase 11 — the `bool` repair is safe, but two checks may not be dropped.** `required_pub_in_path` is read only via `required_path.is_none()` at `:95`, `:111`, and `:151`, so the payload is genuinely dead and the return-type change is behavior-preserving. What carries the behavior is the early return at `policy.rs:66-72`: **both** the `PubInPath::Required` check and the `VisibilitySyntax::Public` check must survive the repair. `the_exact_boundary_rewrite_is_offered_only_under_required` (`tests/diagnostics/forbidden_pub_in_crate.rs:456`) asserts empty diagnostic-code vectors and byte-identical declarations for its `forbidden` and `permitted` arms solely on that gate; a repair that drops either check turns those arms red.

**From Phase 11 — `FINDINGS_SCHEMA_VERSION` is `22`,** not the `21` earlier drafts of this line carried. Phase 11 bumped it when the stored finding gained the typed `ItemVisibility` / `NarrowerScope` shape.

**From Phase 11 — the "applied N import fix(es)" wording.** `src/fixes/runner/notices.rs:11-25` counts the `restricted_annotation` scan into the same import-fix total every other non-`pub-use` fixer feeds, so the Required-mode migration this phase schedules will announce N annotation rewrites as N "import fixes". The wording predates Phase 11, but this is the phase that makes it user-visible at scale. **Resolved 2026-08-03 — fix the notice; this phase owns it.** The self-conversion this phase runs is exactly the moment the wrong noun becomes visible at scale, so the deferral ends here. Name the fix kind in the notice rather than folding every non-`pub-use` fixer into one "import fix" total. The pinning test at `notices.rs:102` asserts the current wording for `field_visibility` and must change with it.

**Resolved 2026-08-02 — the fixer runs first.** The auto-fix work was moved ahead of this phase and split in two: Phase 10 (pub-use fix integrity) and Phase 11 (the restricted-annotation fixer). This phase was renumbered 10 → 12. The migration it schedules is driven by `--fix`, not by hand.

**Delegate acceptance gate:** `verify.sh check`, `verify.sh test`, `verify.sh lint`, and `bash ~/.claude/scripts/delegate/verify.sh test cargo-mend diagnostics` green. Keep both 16-finding rendering gates unchanged. **The typed-signature repair changes no test expectations — every existing test must pass unmodified.** The new lib+bin Required-mode fixture asserts both the advertised fix count and that exactly one edit is applied. **The self-conversion is a separate proof: after `--fix` has run, `src/` contains no bare `pub` that the fixer still reports as convertible, and the measured `summary.fixable_with_fix` count is recorded.** The eight unpinned fixture files pin `pub_in_path` explicitly, and every diagnostics test passes with the flipped default — a fixture that changes behavior because of the flip is a missing pin, not an expectation to update. **The global-config migration is proved by its own unit tests, including the one asserting a versioned config with an explicit `permitted` is left untouched.** **No `mend.toml` exists at the repository root** when the phase ends — its presence is a failed gate, not a detail.

**Regression-only — these three behaviors already shipped and this phase must not re-implement them.** Bare `pub` behind a used exact restricted facade staying silent at `"permitted"`, yielding the exact-boundary suggestion at `"required"`, and an already-exact annotation staying silent are each asserted today by `required_setting_reviews_bare_pub_behind_restricted_facade` (`tests/diagnostics/forbidden_pub_in_crate.rs:310`), `fix_leaves_an_accepted_restricted_annotation_alone` (`:407`), and `the_exact_boundary_rewrite_is_offered_only_under_required` (`:456`). `policy.rs` was not in Phase 11's diff, so no Required-mode behavior changed. They appear here as a regression guard on the typed repair, not as work. **This phase's scope is seven items:** the typed repair in `policy.rs` (with the rendered-text sniff and the rename); the default flip to `Required` plus the eight fixture pins; the version-marked one-time global-config migration and its tests; this crate's `--fix`-driven self-conversion; the one-line `UseSite` narrowing; the lib+bin Required-mode fixture; the "import fix(es)" notice wording and the CHANGELOG entry.

**Orchestrator smoke:** clear `target/mend-findings`, force a cargo-mend recompile, then run `RUSTC_BOOTSTRAP=1 cargo +stable run --release -- --workspace --all-targets --fail-on-warn`; require `No findings`. This is the phase's strongest proof: with no repo-local config of any kind, the shipped default resolving to `required`, and the self-conversion applied, cargo-mend passes its own strictest policy on itself.

**`No findings` is reachable in this phase and the gate stays as written.** Exactly one declaration blocks it — `pub struct UseSite` (`src/compiler/persistence/schema.rs:39`), verified present at HEAD before Phase 11 and therefore not a Phase 11 regression. It is a genuine over-wide declaration, not a false positive (see **Files**), so the fix is the one-line narrowing this phase already owns. Do not restate this gate as "no *new* findings"; weakening it would retire the only check that proves cargo-mend passes the policy it implements.

### Retrospective

**What worked:**

- The typed-signature repair landed exactly as specified and cost nothing elsewhere. `SignatureExposure` (`Contained` / `ExposedAt(VisibilityReach)`) replaced `Option<VisibilityReach>` through `policy.rs` → `scan/mod.rs` → `record.rs`, and every existing test passed unmodified. The rendered-text sniff (`to_source(..).starts_with("pub(in ")`) is gone, replaced by `required.boundary()` matching `ReachBoundary::Module` against `CrateRoot`/`Everywhere`.
- The version-marked global-config migration is one-time by construction and provably so. `config_version` renders at the root table, `migrate_unversioned_config` returns early the moment the stamp is present, and running the real binary twice left the on-disk config byte-identical.
- Pinning `pub_in_path` explicitly in the eight unpinned diagnostics fixtures absorbed the entire blast radius of the default flip: 344 diagnostics tests passed with `Required` compiled in, with no expectation edited.

**What deviated from the plan:**

- **The self-conversion ran and converted 76 declarations across 16 files** — 45 to `pub(in crate::compiler)`, 19 to `pub(in crate::fixes)`, 12 to `pub(in crate::compiler::visibility)` — in three `--fix` passes (75 fixable, then 1 that only surfaced once `ValidatedFixSet` narrowed, then 0). Afterwards `--workspace --all-targets --fail-on-warn` reports `summary: no issues found` for warnings. The phase keeps its two-commit shape: reviewed code first, conversion second.
- **The conversion was first reported as empty, and that report was void.** A run that prints `No findings.` in 0.55s with `check: 0.19s` analyzed nothing: `cargo mend` needs cargo to re-invoke the compiler, and clearing `target/mend-findings` without also forcing a recompile leaves cargo fresh, so the driver never sees any code and the loaded report is empty. A real analysis of this crate takes 5-9s of `check:`. Any `No findings.` whose `check:` time is under a second is not a result.
- **One error survives the conversion and is not mechanically fixable.** `pub(in crate::compiler) struct UseSite` (`persistence/schema.rs:39`) draws its reach entirely from signatures — `StoredReport.use_sites: Vec<UseSite>` and `UseSiteIndex::into_use_sites()` (`sink.rs:52`, called at `visibility_context.rs:202`) — and no caller ever names the type. Policy therefore demands a facade at `crate::compiler`, but a facade for a signature-only type is dead by construction: adding `pub(super) use schema::UseSite;` to `persistence/mod.rs` clears the mend error and makes rustc fail `check` and `lint` with `unused_imports`. Moving the type out of `persistence/schema.rs` would break the serde schema module's cohesion. Resolved by changing the rule rather than the code — see the next bullet.
- **The policy itself was wrong, and this phase fixed it.** A facade is required only for reach a caller *naming* the item demands; reach demanded solely by a signature requires none, because a signature-exposed type is reached through the exposing item's path and a facade re-exporting it would have no user. The root cause ran deeper than the rule site: `use_sites.rs` recorded a written path reference and a signature-only reach as the same `(target, caller_module)` pair, so nothing downstream could tell them apart. `UseSite` gained `reference: UseSiteReference { Named | ThroughSignature }`, `CallerMap`'s value became `ItemCallers { naming, reaching }` (`naming ⊆ reaching`), and `FINDINGS_SCHEMA_VERSION` went 22 → 23. The acceptance lives in `VisibilityConstraintGroup::boundary_demand` (`persistence/visibility_constraint.rs`), not in the scan: `record.rs::current_pass_callers` sees only the target being walked, so a lib-only view cannot see a sibling binary that names the item, and `render` can suppress or reword a finding but never create one — a scan-time acceptance would be unrecoverable. `no_facade_repair` still reads `reaching`, so existing advice is unchanged.
- **The fix-notice summary changed shape by user decision.** Splitting the per-fixer counts out of the single "import fix(es)" total made a default `--fix` run print one clause per enabled kind, four of them reporting zero. The chosen rule: name only the kinds that applied something; when every enabled kind is at zero, emit exactly one clause for the first kind in `notice_counts()` order. That fallback is what keeps a clean run's single `mend: no import fixes available` line, and it kept `tests/diagnostics/import_fixes.rs:293,334` green unmodified.

**Surprises:**

- The CHANGELOG's opt-out instruction was mechanically wrong on the first attempt and only the closure review caught it. `migrate_unversioned_config` keys on the `config_version` stamp, and that stamp is written by *the first run of the upgraded binary*, not by the upgrade. Setting `pub_in_path = "permitted"` "after upgrading" is silently rewritten on the next run; the value only sticks when set after that first run.
- The migration's idempotency depends on a `toml_edit` rendering detail — a root-table key emitted after existing table headers would be re-parsed as a member of the last table and the migration would re-fire forever. It renders at the root, and the pre-existing idempotency assertion at `global.rs:350-353` is what proves it.
- Two diagnostics fixtures destroyed the pin added to them: `pin_pub_in_path` writes the whole `mend.toml`, and a later fixture-local write of the same file removed it. The durable form is putting `pub_in_path` in the fixture's own literal.
- **The self-conversion surfaced a defect outside this plan's scope: a run that analyzed nothing reports clean.** `load_report` (`persistence/load.rs:114`) yields an empty report when no cached report matches the selection, and `run_selection` (`build/execute.rs:119`) accepts it and prints `No findings.` with exit 0. Nothing verifies that the analysis ran. The findings cache normally covers a fresh rerun by replaying stored findings, but cache-absent-and-cargo-fresh has no guard — and this release reaches that state by design, since findings schema 21 rejects schema 19 and the CHANGELOG's own upgrade note tells users to clear the cache. The proposed guard: after cargo exits, require a compatible report for every selected package root and fail when one is missing.

**Implications for remaining phases:** none — Phase 12 is the last phase. The plan is exhausted.

### Phase 12 Review

- No remaining phases were re-reviewed: Phase 12 is the last phase, so there are none. The architect review would ordinarily have been dispatched here, but with zero remaining phases it has nothing to examine.
- The `UseSite` signature-only reach was resolved inside this phase by changing the policy, not the code: the facade requirement now applies only to reach a naming caller demands. Two existing tests encoded the old rule — `allowances::signature_exposure_does_not_admit_pub_in_without_a_facade` was deleted (its fixture is the accepted case and its name states the inverted rule), and `allowances::restricted_sibling_reexports_add_common_ancestor_signature_reach` plus `allowances::used_parent_facade_reach_joins_independent_public_globs` had their signature-only assertions rewritten to assert absence. Both retain their discriminating power: a neighbouring `pub(self)` re-export in the same fixture still asserts `consider removing the visibility`, so a re-export contributing no reach would fail them.
- One finding outlives the plan and is recorded in the Retrospective rather than folded into a later phase, since Phase 12 is the last: the clean-report-without-analysis defect in `load_report`. It remains unfixed and unapproved.
