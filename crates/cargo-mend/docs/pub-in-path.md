# `pub(in path)` at an exact facade boundary

> **Status: IMPLEMENTATION PLAN — phased, delegate-ready.** Accept `pub(in <path>)` on a declaration when `<path>` names the exact module a parent facade already exposes the item to, and keep rejecting every other use of the form.

> **As-built disposition: create**

## Delegation Context

- **Project:** `cargo-mend` (single-crate repo, not a workspace; package `cargo-mend` v0.18.0-dev, edition 2024) — "Opinionated visibility auditing for Rust crates and workspaces"; a `rustc_driver`-based cargo plugin that reads the compiler's resolved item graph and reports/fixes visibility findings.
- **Stack:** Rust, edition 2024, links `rustc_driver`/`rustc_middle` private compiler APIs. **No `rust-toolchain.toml`** — build/install on **stable** with `RUSTC_BOOTSTRAP=1`; `.cargo/config.toml` sets `[env] RUSTC_BOOTSTRAP = "1"` repo-wide, and `docs/style/stable-toolchain-install.md` mandates `RUSTC_BOOTSTRAP=1 cargo +stable install --path .` (nightly-built binaries hit E0514 on stable projects). Key deps: `syn` 2.0 (full, visit) + `proc-macro2` (span-locations), `toml` 1.1 / `toml_edit` 0.25, `serde`/`serde_json`, `cargo_metadata` 0.23, `clap` 4.6, `regex`, `walkdir`, `tempfile`.
- **Layout:**
  - `src/compiler/visibility/` — `mod.rs`, `field.rs`, `policy.rs`, `source.rs`, `use_sites.rs`, `scan/{mod,classify,finding_params,record,visibility_context,visit}.rs` (**new `annotation.rs` lands here**)
  - `src/compiler/facade/` — `mod.rs`, `exports.rs`, `boundary.rs`, `reference.rs`
  - `src/compiler/exposure/` — `mod.rs`, `detect.rs`, `visitor.rs`
  - `src/compiler/persistence/` — `load.rs`, `schema.rs`, `visibility_priority.rs`, `caller_aware.rs`
  - `src/compiler/{constants.rs, settings.rs, source_cache.rs, build/execute.rs}`
  - `src/config/` — `mod.rs`, `loaded.rs`, `global.rs`, `constants.rs`, `diagnostics_config.rs`, `diagnostic_code.rs`, `prelude_pub_mod.rs` (**new `pub_in_path.rs` lands here**)
  - `src/reporting/` — `diagnostics.rs`, `cargo_json.rs`, `render/`
  - `src/fixes/` — `unused_pub.rs`, `narrow_pub_crate.rs`, `field_visibility.rs`, `pub_use_fixes/scan.rs`, `runner/execute.rs`
  - `src/rust_syntax.rs` (crate root, **not** under `visibility/`)
  - `tests/diagnostics/` (fixtures + `mod.rs` test target), `tests/support/`, `tests/cli_smoke.rs`
  - `README.md`, `CHANGELOG.md`, `docs/style/`, `~/rust/nate_style/rust/use-narrowest-visibility.md`
- **Key files:** (all verified; where the design's own line refs were stale the corrected value is given)
  - `src/compiler/visibility/annotation.rs` — shipped in Phase 1, 512 lines. `VisibilityAnnotation<'source>` (nine variants) built by `from_item(source, target, tcx) -> Option<Self>` (`:50`); `VisibilitySyntax` (`:17-28`), the nine-variant dispatch enum returned by `syntax()`; `PathSpelling` (`CrateRooted` | `Relative`); `VisibilityReach(Visibility<DefId>)` (`:30-31`, derives **only** `Clone, Copy` — no `Debug`, no `PartialEq`, deliberately no `Ord`) with `compare(self, other, tcx) -> Option<Ordering>`, `join`, `is_at_least`, `is_strictly_wider`, and `to_source(tcx) -> String` (the reach → `pub` / `pub(crate)` / `pub(in crate::…)` rendering); the free fn `anchored(reach, target, tcx)` (`:217`); and a private generic `enum ScopeReach<Scope>` (`:236`) whose `compare`/`join`/`anchored` take the accessibility relation and the parent-module walk as `FnMut` parameters — that seam is what makes the reach algebra unit-testable without a `TyCtxt`, and later phases with the same problem should reuse it. **Everything is `pub(super)`**, reachable only inside `src/compiler/visibility/`; `mod annotation;` in `mod.rs` still carries a scoped `#[expect(dead_code)]` until the last item has a consumer
  - `src/compiler/visibility/mod.rs` — declares only `field, policy, scan, source, use_sites`; needs `mod annotation;`
  - `src/compiler/visibility/scan/record.rs` — **589 lines after Phase 3; every pre-Phase-3 line reference in this plan is stale.** `:34` `record_visibility_findings`, `:45-50` the `match annotation.syntax()` that resolves `parent_facade_visibility` **only for `Crate | InCrate`** (every `InPath`/`InParent`/`InCurrent` gets `None`), `:65/:77/:85/:114` the four secondary-check guards — now `matches!(annotation.syntax(), VisibilitySyntax::Public)`, not string comparison (`:65` `narrow_to_pub_crate`, `:77` `narrow_to_pub_crate_nested`, `:85` `suspicious_pub`, `:114` `maybe_record_unused_pub`), `:139-147` unused_pub facade/glob suppression, `:189-214` `record_forbidden_visibility_annotation` (the written-form dispatcher; `:206-208` = the `InParent | InCurrent | InPath(_)` arm), `:216` `record_forbidden_pub_crate` (handles `Crate` **and** `InCrate`; `:238-239` = the permitted-`InCrate` branch emitting ``consider using: `pub(crate)` ``), `:235-236` `has_signature_exposure_allowance` call, `:270-308` `record_forbidden_pub_in_crate` (`:276-290` = the `suggestion` match, `:280-283` = the `InPath(Relative)` arm that already suggests the crate-rooted spelling, `:302` = `related: None`), `:364`/`:415` the two `narrow_to_pub_crate` recorders, `:384` `impl_self_name` use, `:454` `resolve_parent_facade_visibility`, `:471` `parent_facade_exports_item`, `:488` `maybe_record_suspicious_pub`, `:577` `StoredPubUseFixFact` write
  - `src/compiler/visibility/scan/visibility_context.rs` — `ItemCategory { Module, Declaration, Use }` at `:40-44` (Phase 3 split the old `NonModule`; `Use` is populated from `ItemKind::Use` in `visit.rs` but **matched nowhere yet** — Phase 7 is its first consumer), `ItemInfo` at `:46` with `impl_self_name: Option<String>`, `FINDINGS_SCHEMA_VERSION` stamping at `:143`
  - `src/compiler/visibility/scan/visit.rs` — `visit_item` at `:19`; `impl_self_name` populated at `:76/:101/:113/:146`
  - `src/compiler/visibility/scan/classify.rs` — `SignatureExposure` at `:27` (retire)
  - `src/compiler/visibility/scan/finding_params.rs` — `FindingParams` construction boundary (where `Visibility<DefId>` converts to text)
  - `src/compiler/visibility/field.rs` — 217 lines after Phase 3 (196 after Phase 1, 214 before; every pre-Phase-3 line reference in this plan is stale). `use super::annotation::VisibilityReach;` at `:21`; `check_item` at `:30`; **Phase 3's shared-classifier call in `check_field` at `:85-104`** — it builds a synthetic `ItemInfo` literal at `:93-102` (`kind_label: Some("field")`, `category: ItemCategory::Declaration`, `impl_self_name: None`) and calls `scan::record_forbidden_visibility_annotation(ctx, &field_info, &annotation, None, sink)` at `:102`, returning early when that reports `true`. **That hard-coded `None` for `parent_facade_visibility` is load-bearing: it is what keeps fields out of facade-based acceptance** (a field is not re-exportable). The wider-than-type comparison `field_declared.is_strictly_wider(type_declared, ctx.tcx)` is now at `:124`; `effective_type_visibility` (returns `VisibilityReach`) at `:162`. `DefIdVisibility`, the local `visibility_strictly_wider`, and the local `is_at_least` are **gone** — all three now live on `VisibilityReach`
  - `src/compiler/visibility/policy.rs` — `:178` `forbidden_pub_crate_help` (`const fn -> &'static str`, stays), `:202` `forbidden_pub_crate_suggestion`, `:289` `assess_parent_facade_usage`, `:318` `assess_signature_exposure_allowance`; unit tests `:505-560`
  - `src/compiler/visibility/use_sites.rs` — `public_reexport_targets` at `:455`, `def_path_string` at `:479`, `parent_module_path_segments` at `:492` (dead `PathAnchor::Crate` strip)
  - `src/compiler/facade/exports.rs` — `:31` `ParentFacadeVisibility`, `:45` `ParentFacadeExportStatus`, `:73` `parent_facade_export_status`, `:136` the `unwrap_or(ParentFacadeVisibility::Public)`, `:143` `parent_facade_has_glob_export`, `:177` `parent_boundary_has_matching_pub_use_glob`, `:194` `exported_names_from_parent_boundary` with the discarding `else { continue; }` at `:202-205`, `:220` `collect_matching_pub_use_exports` (alias/visibility resolution), `:251` `widest_visibility`, `:266` `pub_use_is_fix_supported_with_prefix`, `:289` `parent_facade_visibility`, test `duplicate_re_exports_take_widest_visibility` at `:350`
  - `src/compiler/facade/mod.rs` — re-export surface for the facade API; changes with `ParentFacadeAnalysis`
  - `src/compiler/facade/boundary.rs` — `parent_boundary_for_child`
  - `src/compiler/facade/reference.rs` — `scan_facade_usage` at `:35`, `workspace_source_mentions_parent_export_literal` at `:115`
  - `src/compiler/exposure/detect.rs` — `type_is_exposed_outside_parent` at `:376`
  - `src/compiler/exposure/visitor.rs` — bare-`pub`-only exposure visitor at `:62`
  - `src/compiler/source_cache.rs` — `SourceCache` at `:45`
  - `src/rust_syntax.rs` — `trim_leading_self` at `:51`, `module_name_for_child_boundary_file` at `:74` (the `#[path]`/filename identity issue)
  - `src/compiler/constants.rs` — `FINDINGS_SCHEMA_VERSION` at `:62` (currently `18`; Phase 3 bumped it from `17`)
  - `src/compiler/settings.rs` — `current_analysis_fingerprint()` at `:67`
  - `src/compiler/persistence/load.rs` — `stored_report_matches_selection` at `:159`, three-way check at `:166-168`
  - `src/compiler/persistence/schema.rs` — `StoredReport`/`StoredFinding`/`StoredPubUseFixFact` (`:73`)
  - `src/compiler/persistence/visibility_priority.rs` — `apply_visibility_narrowing_priority` at `:7`
  - `src/compiler/persistence/caller_aware.rs` — `apply_caller_aware_suppression` at `:6` (the use-site map the no-facade classifier reads)
  - `src/compiler/build/execute.rs` — `run_selection` at `:74`, `CargoCheck` bail at `:103-107`
  - `src/config/loaded.rs` — `VisibilityConfig` at `:30` (`prelude_pub_mod` at `:35-36`), `load_config` at `:47`, global-stamps-over-project at `:81-85`, `fingerprint_for` at `:107`
  - `src/config/pub_in_path.rs` — **NEW**: `enum PubInPath { Forbidden, Permitted, Required }`, string-serialized
  - `src/config/mod.rs` — needs `mod pub_in_path;`
  - `src/config/prelude_pub_mod.rs` — the `PreludePubMod` precedent (also moves onto project>global)
  - `src/config/global.rs` — `reconcile_global_config` at `:79`, `GlobalConfig`/`GlobalConfigFile` at `:31/:44`
  - `src/config/constants.rs` — `PRELUDE_KEY` at `:10`, `DEFAULT_GLOBAL_CONFIG_TOML`
  - `src/config/diagnostics_config.rs` — `DiagnosticsConfig` at `:10`, `is_enabled` at `:16`
  - `src/config/diagnostic_code.rs` — `DiagnosticCode::ForbiddenPubInCrate` at `:8`, `ALL` at `:26`, `as_str()` at `:44`
  - `src/reporting/diagnostics.rs` — `FixSupport` enum at `:14-30` (**no `Standard` variant**; `FixSummaryBucket::Standard` at `:33` is a different type), `DiagnosticSpec` at `:88` (its `headline` is now a private `HeadlineSource`, not `&'static str`), `FORBIDDEN_PUB_IN_CRATE` spec literal at `:105-113`, its `diagnostic_spec` dispatch arm at `:350`, `SUSPICIOUS_PUB.inline_help` at `:123`, `finding_headline` at `:374`, `finding_message_not_in_headline` at `:387`, the mechanism-pinning unit test at `:442-477`. These are post-Phase-2 lines; anything citing `:82`/`:334`/`:358` predates it.
  - `src/reporting/cargo_json.rs` — `rustc_diagnostic` at `:146`, `render_diagnostic` at `:202`
  - `src/reporting/render/diagnostic.rs`, `src/reporting/render/human.rs` — human renderer consuming `finding_headline` + `suggestion`
  - `src/fixes/field_visibility.rs` — `visibility_annotation_byte_len` at `:89-103` (the annotation-span parser to share)
  - `src/fixes/unused_pub.rs` — `scan_from_report` at `:15` (bare `"pub "` search)
  - `src/fixes/narrow_pub_crate.rs` — `scan_from_report` at `:15` (bare `"pub "` search)
  - `src/fixes/pub_use_fixes/scan.rs` — `:120` child-module resolution, `screen_candidate` at `:173` (`AlreadyNarrowed` on non-bare-`pub`)
  - `src/fixes/pub_use_fixes/parent_boundary.rs` — `trim_leading_self` consumers at `:133/:198`
  - `src/fixes/runner/execute.rs` — disabled-diagnostic filtering at `:125-130` (runs *after* analysis)
  - `tests/diagnostics/mod.rs` — the `diagnostics` test target root; new fixture modules declared here
  - `tests/diagnostics/{forbidden_pub_crate,allowances,field_visibility_wider_than_type,pub_use_fixes,rendering,narrow_pub_crate,unused_pub,prelude_pub_mod}.rs` — existing suites the fixtures extend. **`tests/diagnostics/forbidden_pub_in_crate.rs` now exists** — Phase 3 created it (172 lines, registered in `tests/diagnostics/mod.rs`). Its single test `restricted_visibility_annotations_are_rejected_once` asserts complete per-file diagnostic-code vectors (so a duplicate finding fails by construction) plus nine exact headline/help pairs at `:77-130`. Extend it; do not recreate it. **`tests/diagnostics/rendering.rs` is a hard gate:** `:223-227` panics with "fixture is missing finding for {code:?}" if any `DiagnosticCode` fails to fire, and `:399` asserts `report.findings.len() == 16`. Any phase that changes which findings fire must adjust that fixture, never the assertion.
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
  - **`FINDINGS_SCHEMA_VERSION` is `18`** (`src/compiler/constants.rs:62`; Phase 3 bumped it from `17`). Any change to the persisted shape *or to the meaning of stored values* requires a bump; `load.rs:166-168` rejects on version, analysis fingerprint, or config fingerprint mismatch. Prior-schema reports are rejected, never partially loaded.
  - **Accepted known limitation:** fingerprints decide whether a report is trusted; they cannot make Cargo compile. A Cargo-fresh crate whose report is rejected simply vanishes from the run — a green result with missing findings, not stale ones. `cargo clean` clears it. No work is scheduled for this.
  - **Diagnostic codes and README anchors are load-bearing.** `DiagnosticCode::as_str()` supplies `mend.toml` `[diagnostics]` keys and `DiagnosticSpec.help_anchor` must match a live `<a id="...">`. Keep `ForbiddenPubInCrate` / `forbidden_pub_in_crate` / `#forbidden-pub-in-crate` exactly as-is. This plan adds no new diagnostic code, no persisted field, no README anchor, and exactly one config key.
  - **`[diagnostics]` is enable/disable only — there is no severity downgrade,** and filtering happens *after* analysis (`fixes/runner/execute.rs:125`). Do not invent a warning tier. The rejection forms split across two codes: `pub(in crate)` → `forbidden_pub_crate`, everything else → `forbidden_pub_in_crate`. Document the split; do not soften it.
  - **Never advertise a fix that produces no edit.** `FixSupport::None` alone is insufficient — the pub-use fixer routes from *stored facts*, so a restricted-annotation finding must both carry `FixSupport::None` and write no `StoredPubUseFixFact`. `unused_pub.rs:15` and `narrow_pub_crate.rs:15` only match a bare `"pub "`.
  - **Reject once, then stop.** After an annotation is rejected, `suspicious_pub`, both `narrow_to_pub_crate` recorders, `unused_pub`, and the field-specific check must not also run. `visibility_priority.rs:7` only suppresses on `unused_pub`, so post-persistence priority cannot clean up the overlap.
  - **Never suggest code that does not compile.** Whole-chain resolution, E0742 anchoring, and the ordered no-facade classifier all exist for this.
  - **Facade identity comes from active HIR, not source text.** `SourceCache` parses unexpanded source with `syn` and evaluates no attributes — valid for line reporting, usage analysis, and auto-fix eligibility only, never for deciding whether a re-export exists.
  - **Keep the written facade spelling alongside the resolved reach.** `pub(super) use` and `pub(crate) use` in a crate-root child resolve identically but grant `InternalParentFacadeBoundary` differently.
  - **Nearest-facade metadata and chain result stay separate.** Overwriting `ParentFacadeExportStatus::visibility` with the chain-widest value mis-pairs `parent_path`/`child_module`/usage and silently drops fixes.
  - **Cross-crate `crate::` literals must not count as usage** — keep crate identity in any use-site key.
  - **Config precedence: project `mend.toml` > global > compiled-in default.** The project value deserializes as `Option<_>` so absence stays distinguishable. Fingerprint and serialize only the *resolved* config. One `LoadedConfig` serves the entire Cargo selection — per-member visibility policies cannot coexist in one run.
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
- Phase 4 carries a `**Pending decision:**` on how a foreign boundary should be reported: a new `FacadeChainBlocker::ForeignBoundary` (recommended) versus a public-boundary row in Phase 7's matrix. Deferred rather than decided, since Phase 4 is the first phase that can reach the path.
- Phases 5, 7, and 9 gained `annotation.rs` **Files** rows; 5 and 9 also gained the `visibility/mod.rs` re-export row.
- **Rejected direction, do not relitigate:** widening `VisibilityReach` to `pub(in crate::compiler)` for the cross-module consumers. `record_forbidden_pub_in_crate` (`scan/record.rs:270-308`) flags every `InPath` spelling unconditionally until Phase 7 lands boundary acceptance, so that spelling breaks the self-policy gate for phases 5, 6, and 9. The plan now specifies this repo's existing pattern instead — bare `pub` on the item inside the private module plus a `pub(super) use` in that module's `mod.rs`, as `facade/exports.rs:31` + `facade/mod.rs:8` already do.
- Phases 5, 7, and 9 gained the derive facts: `VisibilityReach` is `Clone, Copy` only, so a struct holding one cannot derive `Debug`/`PartialEq`, and an equality test must be spelled `compare(other, tcx) == Some(Ordering::Equal)`.
- Phases 7 and 9 own the removal of the `#[expect(dead_code)]` on `mod annotation;` — whichever leaves no item unused must delete it, since clippy is deny-by-default here and a fulfilled expectation is itself an error.
- Every phase's acceptance gate now runs cargo-mend on its own source. Phase 1 shipped `check`/`test`/`lint` green while the tool rejected its own new file, and both blind reviews missed it.
- Phase 5's `nearest` field changed from `Vec1<ParentFacadeOccurrence>` to `Vec` — `vec1` is not a dependency and appears nowhere in `src/`; non-emptiness is already guaranteed by the `Option<ParentFacadeAnalysis>` wrapper.
- Phase 10 now states the unnamed cost of making `Required` the default: ~51 sites in this repo use the bare-`pub`-behind-a-facade shape and would all need converting to keep the self-run green.
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

- **No remaining phase is redundant.** Phase 2 shipped a mechanism only; phases 3–11 each keep their full job. Phases 4, 6, 9, and 10 came through clean with respect to it.
- **Phase 3** gained three constraints: every forbidden finding it emits must set a non-empty `message` (it, not Phase 7, is the first phase whose gate asserts a per-outcome headline, and an empty message silently falls back to the generic text); the blocker location must go in `suggestion` because `related` is dropped for `DetailMode::None` specs; and per-outcome messages perturb `build_selection`'s sort/dedup key (`src/fixes/runner/execute.rs:92-121`), so fixture ordering churn is expected.
- **Phase 3 Files** gained three test-support rows: the test-side `Finding` (`tests/support/report.rs:7-19`) carries no headline field, `finding_from_compiler_message` (`tests/support/mend_json.rs:226-262`) never reads `message`, and `tests/support/diagnostics.rs:117-133` asserts a static headline for every code — all three block a headline assertion as written, and Phase 2 only avoided them by hand-parsing raw cargo JSON.
- **Phase 5** gained the fact that a `related` string on a forbidden finding never renders, so its consumer table's blocker path/line belongs in `suggestion`.
- **Phase 7 Files** now records that `HeadlineSource::FindingMessage` carries a `fallback: &'static str` (its Spec still draws the bare unit variant) and that the unit test at `src/reporting/diagnostics.rs:442-477` must grow a row per matrix outcome — that test is the only guard against a silent revert to a static headline. It also inherits Phase 3's three test-support rows.
- **Phase 8** gained a fifth CHANGELOG upgrade case: Phase 2 removed the `note` child duplicating the headline from cargo JSON, so consumers see one fewer child on every forbidden finding even when no finding changed.
- **Delegation Context** line refs for `src/reporting/diagnostics.rs` were corrected to post-Phase-2 positions and now record that `FixSupport` has **no** `Standard` variant. The `cargo_json.rs` refs were verified still exact. Phase 2's own Work Order keeps its pre-phase refs deliberately, as the archive record.
- **Deferred to Phase 7:** the dynamic `suspicious_pub` suggestion collides with that spec's static `pub(super)` help, and the two renderers pick opposite winners while `rustc_diagnostic` emits both. Written as a `**Pending decision:**` block on Phase 7.
- **Deferred to Phase 11:** its Spec names `FixSupport::Standard`, which does not exist; adding a real variant touches the persisted `fixability` string and therefore the schema version. Written as a `**Pending decision:**` block on Phase 11.
- **Still open from the Phase 1 review, unchanged:** Phase 4's `extern crate` fixed point and its `ForeignBoundary` pending decision both still block that phase's dispatch.

---

### Phase 3 — Close the three detection holes · status: done (`b320079`)

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
- **Phases 9 and 10** acceptance gates gained the `rendering.rs` all-16-findings rule.
- **Phase 10** — the ~51-site estimate is now flagged as a lower bound to re-derive at dispatch; Phase 3's four CLI field conversions added to it.
- **Phase 11** — corrected the `tests/support/diagnostics.rs` range: the `FixSupport` mirror is `:60-85`, and `:115-140` is the `HeadlineSource` block Phase 3 added.
- **Deferred to Phase 5** (`**Pending decision:**`) — what an unresolvable-chain finding shows the user. The blocker string would replace the written-form repair advice rather than accompany it, since `suggestion` is the only surviving channel and Phase 3 already occupies it. Recommendation recorded: combine both into one `suggestion`, blocker first.
- **Deferred to Phase 7** (`**Pending decision:**`) — whether to split the phase. Phase 3 absorbed its dispatch skeleton, leaving five separable jobs under one number. Recommendation recorded: split into 7a (acceptance) and 7b (matrix), with the renumbering cost to Phases 8-11 stated as the counterweight.

---

### Phase 4 — HIR re-export index and facade subjects · status: todo

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

**Pending decision:** how a foreign boundary should surface, given that `pub` is what the reach algebra currently produces for one.

- **Problem.** Phase 7's advice matrix has **no public-boundary row**, on the stated argument that a real `pub use` facade makes `run_selection` return a `CargoCheck` failure on nonzero `cargo check` status *before* a report is loaded, so mend never renders a finding for it. That argument does not cover a *synthetic* `Public` produced by the fixed point above: it comes from code that compiles cleanly. A restricted annotation on such an item reaches the matrix with a required reach nothing matches, and no row fires.
- **What exists now.** `join` returns `ScopeReach::Public` at the fixed point (`annotation.rs:273-277`). Phase 5 already carries an `Unresolvable { blocker }` shape for chains it cannot resolve.
- **Option A (recommended).** Add `FacadeChainBlocker::ForeignBoundary` in Phase 5 and report the item as unresolvable rather than advising `pub`. Reuses machinery Phase 5 already builds, and refusing to advise is the honest answer when the boundary is outside this crate.
- **Option B.** Add the missing public-boundary row to Phase 7's matrix and let the tool advise `pub`. Cheaper, but it means recommending the widest possible visibility on the strength of a fallback rather than an analysis.

Resolve this before dispatching Phase 4; it determines whether Phase 4's `extern crate` work feeds a blocker or a matrix row.

**Acceptance gate:** `verify.sh check`, `verify.sh test`, `verify.sh lint` green, **plus `bash ~/.claude/scripts/delegate/verify.sh test cargo-mend diagnostics` — the bare `verify.sh test` line runs only `--lib`/`--bins`, so every fixture under `tests/diagnostics/` is invisible to it and a phase whose only new tests live there would gate green having run none of them,** **and the self-policy gate: `RUSTC_BOOTSTRAP=1 cargo +stable run --release -- --workspace --all-targets --fail-on-warn` reports "No findings" on cargo-mend's own source.** That last check is not redundant with `lint` — Phase 1 shipped `check`/`test`/`lint` all green while the tool rejected its own new file, and two blind reviews missed it, because the only thing that knows cargo-mend's house rules is cargo-mend. The two rules that bite new rustc-facing code are `inline_path_qualified_type` (write `use rustc_middle::ty::Foo;` and then `Foo`, never an inline `rustc_middle::ty::Foo` in a type position) and `imports_at_top`. Fixtures: inactive `#[cfg]` wider facade — the active inner boundary is used; active macro-generated facade — found; `#[path]` module; raw module identifier; `pub use self::...` (via `trim_leading_self`, `rust_syntax.rs:51`); variant re-export — subject maps to the containing enum; inherent method and inherent associated const behind a facade; an inherent method and associated const whose **type travels farther than they do** — no widening demanded; `extern crate <c> as <alias>` behind a facade — the declaration is matched; inactive-`cfg` glob and macro-generated glob alongside a named-beside-glob case. Existing `pub_use_fixes` and `allowances` suites still pass.

---

### Phase 5 — Semantic chain resolution · status: todo

#### Work Order

**Goal:** the required boundary is the joined resolved visibility of the whole facade chain, computed once per item, with nearest-occurrence metadata kept separate and usage scanned at most once.

**Spec:**

Re-exports stack. `parent_facade_export_status` (`facade/exports.rs:73`) climbs ancestors until a boundary re-exports the item, then stops — so it reports the *innermost* facade's visibility, which need not be the widest. With both of these present:

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

`M` is the use item's HIR owner module, so `mod.rs` and named-sibling layouts of the same module produce the same answer. Join the hops with `VisibilityReach::join`. Because `Restricted` already carries the boundary, there is no separate "widest-carrying facade module" to track and **no `parent_of` call at rendering time** — the resolved value *is* the required boundary.

Return `Option<ParentFacadeAnalysis>` — absence means no facade:

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

enum FacadeChainFailure { Glob, UnsupportedVisibility }
```

`nearest` is a collection because one boundary can hold several matching `use` declarations with different aliases, visibilities, and usage states — `duplicate_re_exports_take_widest_visibility` (`exports.rs:350`) already covers that. Collapsing them loses metadata each consumer needs, and picking one can miss the widest reach.

**Reach cannot replace written facade syntax.** In a crate-root child module, `pub(super) use child::Thing;` and `pub(crate) use child::Thing;` both resolve to `Restricted(CRATE_DEF_ID)`, but `assess_parent_facade_usage` (`policy.rs:289`) grants `InternalParentFacadeBoundary` only to the first. Retiring `ParentFacadeVisibility` is fine; dropping the spelling is not — hence `FacadeSyntax` on the occurrence.

Overwriting a nearest occurrence's `visibility` with the chain-widest value breaks two things concretely: `assess_parent_facade_usage` loses a used inner-`Super` allowance whenever an outer wider facade is unused; and `StoredPubUseFixFact` (`record.rs:498`) stores `parent_path = video_plane/plane/mod.rs` with `child_module = "camera_panel"` — substitute the outer `video_plane/mod.rs` and `child_module` stays `"camera_panel"` while that line reads `plane::bind`, so `fixes/pub_use_fixes/scan.rs:120` resolves `None` and silently skips the fix.

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
| `type_is_exposed_outside_parent` (`detect.rs:376`) | nearest usage | unaffected |
| `ReviewInternalParentFacade` | nearest path/line | unaffected |

**Unresolvable hops.** A hop the walk cannot follow makes the chain unresolvable: no boundary is computed, no `pub(in ...)` suggestion is emitted, and the blocking facade is reported. Two cases:

*Glob-only re-exports* — `pub(super) use camera_panel::*;`. A glob does not force the declaration wider: verified against rustc, that line over a `pub(super) fn bind` compiles clean and the failure surfaces at the call site as E0603 rather than at the facade as E0364. A glob also states intent about a module rather than an item. A glob is a barrier **only when nothing else matches** — a named `pub(super) use c::helper;` stays resolvable even when the same `mod.rs` also contains `pub(super) use c::*;`. `parent_facade_has_glob_export` (`exports.rs:143`/`:177`) takes no item name and only recognizes `child::*`, so its boolean cannot serve as the chain result. Existing glob suppressions (`unused_pub` at `record.rs:115`) are unchanged.

*Unrecognized facade visibility* — a facade line spelled `pub(in crate::a) use`. `parent_facade_visibility` (`exports.rs:289`) returns `None` for any multi-segment restricted path, and the caller collapses that to `Public` via `unwrap_or` (`exports.rs:136`). **That `unwrap_or` is unreachable for this case**: `exported_names_from_parent_boundary` discards the visibility at `exports.rs:202-205` —

```rust
let Some(visibility) = parent_facade_visibility(&item_use.vis) else {
    continue;
};
```

— *before* the use tree is matched, so `explicit` can never be non-empty while `visibility == None`. Changing the `unwrap_or` does nothing. The fix is in the parse: return `Private | Recognized(visibility) | RestrictedUnrecognized`, match the use tree even in the last state, and stop the ancestor walk before `exports.rs:106`.

**Renames are resolvable.** `pub(super) use camera_panel::bind as attach;` is named, so the boundary is computable, and `exports.rs:220` already resolves the alias and its visibility. Only the *auto-fix* is unavailable (`pub_use_is_fix_supported_with_prefix`, `exports.rs:266`). Classify a rename as resolvable, manual-fix-only.

**Resolve once, scan usage at most once.** Bare `pub` already performs three facade resolutions today, four when `ReviewInternalParentFacade` re-fetches; boundary validation would make an accepted `pub(in ...)` perform four or five, and recursive exposure lookups mean five is not a ceiling. Every resolution calls `scan_facade_usage` (`facade/reference.rs:35`), which allocates a source-file vector and traverses package then workspace sources. Split structure from usage:

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
- `src/compiler/facade/exports.rs` — three-state facade parse (`:289`), stop the walk (`:106`), `ParentFacadeAnalysis`/`ParentFacadeOccurrence`/`FacadeChainResolution`, retire `ParentFacadeVisibility` (`:31`), keep alias resolution (`:220`) and fix-support (`:266`)
- `src/compiler/facade/mod.rs` — re-export surface
- `src/compiler/facade/reference.rs` — `scan_facade_usage` (`:35`) called once per occurrence; crate identity in `workspace_source_mentions_parent_export_literal` (`:115`)
- `src/compiler/visibility/scan/record.rs` — `resolve_parent_facade` once at `:34`; rewire every consumer in the table; `resolve_parent_facade_visibility` (`:454`) becomes the new API. **Also widen the `match annotation.syntax()` at `:45-50`:** Phase 3 resolves `parent_facade_visibility` only for `Crate | InCrate` and passes `None` for every `InPath`/`InParent`/`InCurrent`. Phase 7's acceptance rule compares an `InPath` reach against the chain reach, so unless `InPath(_)` also resolves the facade here it will always see "no facade" and acceptance can never fire
- `src/compiler/visibility/scan/mod.rs` — **Phase 3 added `pub(super) fn record_forbidden_visibility_annotation` at `:29-42`**, whose `parent_facade_visibility: Option<ParentFacadeVisibility>` parameter becomes `Option<&ParentFacadeAnalysis>`. It builds the finding context via `classify::visibility_finding_context` and delegates to `record::record_forbidden_visibility_annotation`
- `src/compiler/visibility/field.rs` — `check_field` calls that wrapper at `:102` with a hard-coded `None`. **Preserve the `None`** — it is what keeps fields out of facade-based acceptance, since a field is not re-exportable
- `src/compiler/visibility/policy.rs` — `assess_parent_facade_usage` (`:289`) reads nearest occurrences, not the chain
- `src/compiler/visibility/annotation.rs` — reach `VisibilityReach` (and the `compare` / `join` / `is_at_least` / `is_strictly_wider` / `to_source` methods and the free `anchored` fn this phase calls) from `crate::compiler::facade`. Phase 1 shipped them `pub(super)` — reachable only inside `src/compiler/visibility/` — because it had no cross-module consumer and cargo-mend enforces narrowest-visibility on itself. This phase is the first consumer outside that subtree. **Use the house pattern, not `pub(in crate::compiler)`:** bare `pub` on the items inside this private module, plus `pub(super) use annotation::VisibilityReach;` (and the same for `anchored`) in `src/compiler/visibility/mod.rs`. Precedent: `src/compiler/facade/exports.rs:31` is a bare `pub enum ParentFacadeVisibility` paired with `src/compiler/facade/mod.rs:8`'s `pub(super) use exports::ParentFacadeVisibility;`. A literal `pub(in crate::compiler)` would fail cargo-mend's own gate — `record_forbidden_pub_in_crate` (`src/compiler/visibility/scan/record.rs:270-308`) rejects **every** `InPath` spelling — crate-rooted and relative alike — unconditionally, and acceptance of an exact boundary does not arrive until Phase 7, so the annotation would be flagged on this crate's own source for two phases
- `src/compiler/visibility/mod.rs` — add the `pub(super) use` re-exports named above
- `tests/diagnostics/` — chain fixtures listed in the gate

**Constraints from prior phases:** Phase 1 supplies `VisibilityReach::{compare, join}` and the free fn `anchored`, all in `src/compiler/visibility/annotation.rs` and all `pub(super)` — see **Files** for how this phase reaches them from outside that module, and note that it does **not** do so by writing `pub(in crate::compiler)`: `record_forbidden_pub_in_crate` (`scan/record.rs:270-308`) flags every `InPath` spelling unconditionally until Phase 7 lands acceptance, so that spelling would break this phase's own self-policy gate. `VisibilityReach` derives only `Clone, Copy` — no `Debug`, no `PartialEq` — so `ParentFacadeOccurrence` cannot carry one and still `#[derive(Debug, Clone, PartialEq, Eq)]` the way its neighbor `ParentFacadeExportStatus` does (`compiler/facade/exports.rs:44`); either hand-write the impls, or keep the reach out of that struct and compute it at the comparison site. `VisibilityReach` deliberately has no `Ord`/`PartialOrd`: `compare` returns `Option<Ordering>` and `None` means two restricted scopes are genuinely incomparable siblings, which is not an error — feed such pairs to `join`, which returns their nearest common ancestor. Phase 4 supplies the two HIR indexes and `ItemInfo::facade_subject: LocalDefId`; facade lookup is by subject, never by item name. Phase 3 established reject-once ordering in `record_visibility_findings`. Phase 2 left the two forbidden diagnostics with **no rendering channel for a blocker location except `suggestion`**: both specs are `DetailMode::None` (`src/reporting/diagnostics.rs:102`, `:111`), so `detail_reasons` returns empty and a `related` string on a forbidden finding is silently dropped, and `finding_message_not_in_headline` (`:387-398`) suppresses the message for any `FindingMessage` spec regardless of whether it differs from the headline. The consumer table's "reject with blocker path/line/reason" therefore puts the path and line in `suggestion`, not in `related`.

**Pending decision:** what an unresolvable-chain finding actually shows the user.

Actual problem:
Phase 5 must report a blocker (path + line + reason) when a facade chain cannot be resolved, and Phase 2 left `suggestion` as the only rendering channel that survives on the two forbidden diagnostics. But `suggestion` is already occupied: Phase 3's `record_forbidden_pub_in_crate` (`scan/record.rs:276-290`) builds `suggestion: Option<String>` from a `match` on the written form, so a blocker string would **replace** the repair advice rather than accompany it.

What exists now:
- `record_forbidden_pub_in_crate` sets `suggestion` per written form: `InParent` -> ``consider using: `pub(super)` ``, `InCurrent` -> ``consider using: `pub(self)` ``, `InPath(Relative)` -> the crate-rooted spelling computed from `annotation.reach(...)`, and `InPath(CrateRooted)` -> `None` (`scan/record.rs:276-290`).
- It passes `related: None` (`:302`), and `related` is dropped anyway for these `DetailMode::None` specs.
- So only the `InPath(CrateRooted)` arm has a free `suggestion` slot today.

What should change:
- Either (a) an unresolvable-chain finding shows **only** the blocker, discarding the written-form advice for that finding, or (b) the two are combined into one `suggestion` string (blocker first, then the repair), or (c) Phase 5 also gives the forbidden specs a real detail channel by changing them off `DetailMode::None` in `src/reporting/diagnostics.rs`, which is a wider change than Phase 5 currently scopes.

Recommendation:
Take (b) — combine into one `suggestion`, blocker first. It preserves the repair advice the fixture at `tests/diagnostics/forbidden_pub_in_crate.rs:77-130` already pins, and it avoids widening Phase 5 into the reporting layer. Record the chosen format in Phase 5's Spec before dispatch; the fixture's nine `assert_headline_and_help` pairs must be updated to match whichever form is chosen.

**Acceptance gate:** `verify.sh check`, `verify.sh test`, `verify.sh lint` green, **plus `bash ~/.claude/scripts/delegate/verify.sh test cargo-mend diagnostics` — the bare `verify.sh test` line runs only `--lib`/`--bins`, so every fixture under `tests/diagnostics/` is invisible to it and a phase whose only new tests live there would gate green having run none of them,** **and the self-policy gate: `RUSTC_BOOTSTRAP=1 cargo +stable run --release -- --workspace --all-targets --fail-on-warn` reports "No findings" on cargo-mend's own source.** That last check is not redundant with `lint` — Phase 1 shipped `check`/`test`/`lint` all green while the tool rejected its own new file, and two blind reviews missed it, because the only thing that knows cargo-mend's house rules is cargo-mend. The two rules that bite new rustc-facing code are `inline_path_qualified_type` (write `use rustc_middle::ty::Foo;` and then `Foo`, never an inline `rustc_middle::ty::Foo` in a type position) and `imports_at_top`. Fixtures: `Super → Super` chain resolving to a non-root restricted module — the computed boundary compiles; `Super → Crate` chain — the boundary is `pub(crate)`, not `pub(in crate::<inner parent>)`; direct-child facade whose canonical result is `pub(crate)`; the same owner module expressed as `mod.rs` and as a named sibling file; glob-only facade — chain unresolvable, blocker reported, existing `unused_pub` suppression still holds; named export beside a glob — still resolvable; rename facade — resolvable, boundary computed, auto-fix unavailable; facade line spelled `pub(in crate::a) use` — chain unresolvable and the facade is **not** treated as `Public`; two same-subject re-exports at one boundary with different aliases, visibilities, and usage states; a crate-root child holding both `pub(super) use` and `pub(crate) use` of the same item — the `InternalParentFacadeBoundary` allowance still tracks spelling; a two-member workspace where both members contain the same module and item path but only one uses its own facade — no cross-crate usage match. Counter-based performance test keyed by `LocalDefId`, behind a non-default test-only Cargo feature: one structural resolution and one usage scan for a fixture hitting every check; one structural resolution and **zero** usage scans when boundary validation resolves the finding without usage; the `ReviewInternalParentFacade` branch increments neither again.

---

### Phase 6 — `pub_in_path` configuration · status: todo

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

`allow_prelude_pub_mod` (`docs/plans/prelude-pub-mod-exemption.md`, `src/config/prelude_pub_mod.rs`) supplies the mechanics — a field on `VisibilityConfig` (`config/loaded.rs:30`), a global-config key reconciled by the `toml_edit` pass, and inclusion in the fingerprint. It does **not** supply the ownership model: it is global-only *by design*, since `load_config` deliberately stamps the global value over the project-deserialized one (`config/loaded.rs:81-85`). A per-machine preference cannot pin a repo to `Required`, which Phase 10 requires — CI would fall back to the default and two developers on one commit could enforce different policies.

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

---

### Phase 7 — Acceptance and the advice matrix · status: todo

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

`record_forbidden_pub_in_crate` — **shipped by Phase 3 at `record.rs:270-308` with the signature `fn record_forbidden_pub_in_crate(ctx, item, annotation: &VisibilityAnnotation<'_>, sink) -> Result<bool>`.** It takes **no** `finding_context` and **no** facade parameter today, and it already matches on `VisibilitySyntax::{InParent, InCurrent, InPath(_)}`, returning `Ok(false)` for every other form. This phase *adds two parameters* — the `ParentFacadeAnalysis` and the `VisibilityFindingContext` the no-facade classifier needs — and updates the dispatcher arm in `record_forbidden_visibility_annotation` (`record.rs:206-208`) plus the `pub(super)` wrapper in `scan/mod.rs:29-42` to pass them. It then becomes: fire unless `pub_in_path` permits the form **and** the annotation's reach equals the chain's required reach **and** the item is a declaration rather than a `use` line. Keep `DiagnosticCode::ForbiddenPubInCrate` and its `forbidden-pub-in-crate` anchor.

**Fields are not excluded by the `Declaration`-vs-`Use` test — do not rely on it for them.** `check_field` sets `category: ItemCategory::Declaration` (`field.rs:99`), so a field satisfies "is a `Declaration`". What keeps fields out of acceptance is that `check_field` passes `None` for the facade argument (`field.rs:102`) and a field is not re-exportable, so no chain can ever justify one. **Decision for this phase: rely on that `None`, do not add an `ItemCategory::Field` variant** — the behavior is already correct and a new variant would churn `visibility_context.rs`, `visit.rs`, and `field.rs` for no behavioral gain. State the reliance in a comment at the acceptance site so it is not silently broken later.

**Two matrix rows land in the other recorder.** The `pub(in crate)` rows ("redundant spelling", "too-wide path") are **not** handled by `record_forbidden_pub_in_crate`: Phase 3 routes `VisibilitySyntax::InCrate` to `DiagnosticCode::ForbiddenPubCrate` via `record_forbidden_pub_crate` (`record.rs:198`, `:254`), which already ships a permitted-`InCrate` branch emitting ``consider using: `pub(crate)` `` (`record.rs:238-239`). Those two rows are edits to `record_forbidden_pub_crate`, and that existing branch must survive.

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
| Restricted `use` blocks resolution | *(same)* | ``facade at <path>:<line> uses `pub(in ...)`; rewrite it as `pub(super)`, `pub(crate)`, or `pub`, then rerun `cargo mend` `` |
| Redundant spelling, canonical reach valid | ``` `pub(in crate)` is a redundant spelling of `pub(crate)` ``` | ``consider using: `pub(crate)` `` (analogous for `self`/`super`) |
| Too-wide path | ``` `pub(in crate)` is wider than the exact parent facade boundary ``` | ``consider using: `pub(in crate::video_plane)` `` |
| No visibility-only rewrite compiles | `no visibility annotation allowed by policy preserves this item's current callers` | ``move the item into `crate::a`, or add an explicit facade at `crate::a` and rerun `cargo mend` `` |

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

Order matters: without it, an item whose callers sit above its parent matches the ordinary row and gets advice producing E0603 — worse than saying nothing. The inputs are the persisted use-site map that `apply_caller_aware_suppression` (`compiler/persistence/caller_aware.rs:6`) already reads, so this is comparisons over recorded callers, not another HIR or source scan.

**`forbidden_pub_crate_suggestion`** (`policy.rs:202`): the `Super`-facade arm currently says "consider using `pub`" and becomes the matrix row naming the boundary. Return type changes from `const fn -> &'static str` to `-> String`. The sole caller (`record.rs:198`) already does `.to_string()`, so it allocates unconditionally today; the caller drops its `.to_string()` and the unit tests (`policy.rs:505-560`) compile unchanged, since `assert_eq!` between `String` and `&str` works. `const fn` is lost either way — the function gains the boundary path as a parameter. `forbidden_pub_crate_help` (`policy.rs:178`) stays `const fn -> &'static str` and is converted in the fallback arm.

**`suspicious_pub` is the only secondary check that accepts `pub(in ...)`.** Phase 3 deleted the `item.visibility_text != "pub"` string comparisons; each secondary check is now gated on `matches!(annotation.syntax(), VisibilitySyntax::Public)` — `record.rs:65` (`narrow_to_pub_crate`), `:77` (`narrow_to_pub_crate_nested`), `:85` (`suspicious_pub`), `:114` (`maybe_record_unused_pub`). Admitting `suspicious_pub` for an accepted `pub(in ...)` therefore means **widening the guard at `:85`**, not removing a string comparison. Note also that these checks are only reached at all when the written-form dispatcher returns `false` (reject-once, `record.rs:52-62`), so an accepted annotation must fall through the dispatcher for any of them to run.

- `narrow_to_pub_crate` must not: both implementations (`record.rs:294`, `:345`) suggest and auto-fix to `pub(crate)`, which is *broader* than an exact boundary — a "narrow" fix that widens and drops the ceiling. An accepted `pub(in ...)` behind a `Super` facade can never satisfy the nested check's `Crate` condition anyway.
- `unused_pub` cannot reach it: acceptance requires a facade, and the check already bails whenever a facade exports the item (`record.rs:114`) — correctly, since removing the annotation would break the re-export with E0364.

That leaves `suspicious_pub` (`record.rs:418`, `policy.rs:30`) with a real job — the facade exists but nobody uses it, so both the facade and the annotation are dead:

| Case | Result |
|---|---|
| `Permitted` + bare `pub` + used exact facade | allowed |
| `Required` + bare `pub` + exact restricted facade | `suspicious_pub`, dynamic suggestion |
| exact `pub(in ...)` + used facade | allowed at both accepting settings |
| exact `pub(in ...)` + unused facade | stale-facade warning: remove the facade and the now-unneeded annotation |
| rejected `pub(in ...)` | no suspicious check |

The earlier parent-public and effective-public allowances were written for bare `pub` and must not return first for a restricted annotation, or the stale-facade branch never runs. `assess_parent_facade_usage` reads **nearest** occurrence data, not the chain — that is what preserves the used-inner-`Super` allowance.

**Fix guards.** `fixes/unused_pub.rs:15` and `fixes/narrow_pub_crate.rs:15` both search for bare `"pub "`, so any restricted-annotation finding marked auto-fixable produces no edit while `cargo mend --fix` reports success. `FixSupport::None` alone is **not** a sufficient guard: standard fix scans route by diagnostic code, but the pub-use fixer routes from *stored facts*. Today an unused facade gets `FixSupport::PubUse` and `record.rs:498` writes a `StoredPubUseFixFact`; `screen_candidate` (`fixes/pub_use_fixes/scan.rs:173`) then rejects the child as `AlreadyNarrowed`. So a stale-facade finding on an accepted restricted annotation must both carry `FixSupport::None` **and** write no `StoredPubUseFixFact`. **Phase 3 already satisfies the rest of this paragraph — do not re-audit it:** both rejection recorders ship `fix_support: FixSupport::None` with no `StoredPubUseFixFact` write (`record.rs:261`, `:301`), and field rejection already returns before `field_visibility_wider_than_type` can record (`field.rs:102-103`). The remaining guard work here is only the stale-facade / accepted-annotation case.

Signature exposure keeps its current arm ordering (`(Present, _)` first) until Phase 9: it suggests `pub`, which is wider than necessary but always compiles.

**Files:**
- `src/compiler/visibility/scan/record.rs` — narrowed `record_forbidden_pub_in_crate` (`:270-308`, gaining two parameters), the `pub(in crate)` matrix rows in `record_forbidden_pub_crate` (`:216`, permitted-`InCrate` branch at `:238-239`), the dispatcher arm at `:206-208`, advice-matrix dispatch, no-facade classifier, `suspicious_pub` admission (guard at `:85`, recorder at `:488`), `StoredPubUseFixFact` guard (`:577`). **The `suggestion` match at `:276-290` is partly done:** the `InPath(PathSpelling::Relative)` arm already emits ``consider using: `{annotation.reach(item.def_id, ctx.tcx).to_source(ctx.tcx)}` `` (`:280-283`), which is exactly this phase's "relative spelling of an otherwise-correct boundary -> suggest the `crate::`-rooted spelling" row. **Only the `InPath(PathSpelling::CrateRooted)` arm still yields `None`** — fill that one, and do not regress the `Relative` arm (pinned by the fixture at `tests/diagnostics/forbidden_pub_in_crate.rs:95-100`)
- `src/compiler/visibility/scan/mod.rs` — the `pub(super)` wrapper at `:29-42` forwards the new parameters
- `src/compiler/visibility/policy.rs` — `forbidden_pub_crate_suggestion` → `String` (`:202`), `classify_suspicious_pub` (`:30`), `assess_parent_facade_usage` (`:289`)
- `src/compiler/persistence/caller_aware.rs` — expose the caller union for the classifier (`:6`)
- `src/reporting/diagnostics.rs` — matrix headlines flow through `HeadlineSource::FindingMessage`, which is **not** the bare unit variant this Spec draws: it carries `fallback: &'static str` (Phase 2). Extend the loop in `forbidden_headline_uses_message_with_static_fallback` (`:442-477`) to cover every new matrix outcome — per Phase 2's retrospective that test is the only thing preventing a silent revert to a static headline. `SUSPICIOUS_PUB.inline_help` is at `:123`.
- **The three test-support rows below all shipped in Phase 3 — no plumbing work remains here.** `tests/support/report.rs:10` already carries `pub headline: String`; `tests/support/mend_json.rs:249-253` already populates it from the diagnostic's `/message`; and `tests/support/diagnostics.rs` no longer holds a plain `&'static str` headline — it mirrors `HeadlineSource { Literal(&'static str), FindingMessage { fallback: &'static str } }` at `:122-140`, resolved via `HeadlineSource::resolve(&finding.headline)`. `assert_rendered_diagnostics` lives in `tests/diagnostics/rendering.rs:220`, not in `tests/support/diagnostics.rs`, and already resolves headlines from the report rather than from a static string. The matrix's "does not match the parent facade boundary" row makes a forbidden code emit a non-fallback message, which breaks that assertion and Phase 2's two helpers at `tests/diagnostics/rendering.rs:262` and `:324` together.
- `src/compiler/visibility/annotation.rs` — read-only: `VisibilitySyntax` (`:17-28`) is the enum the matrix dispatches on; `VisibilityAnnotation::syntax()`, `reach(target, tcx)` (`:86`), `VisibilityReach::{compare, to_source}` (`:164`, `:197`) are the comparison and rendering surface. If this phase is the one that leaves no item unused, delete the `#[expect(dead_code)]` on `mod annotation;` in `src/compiler/visibility/mod.rs:1-5` — clippy is deny-by-default here and a fulfilled expectation is itself an error.
- `tests/diagnostics/forbidden_pub_in_crate.rs`, `tests/diagnostics/allowances.rs`, `tests/diagnostics/pub_use_fixes.rs` — fixtures in the gate

**Constraints from prior phases:** Phase 1 supplies `VisibilityAnnotation`, `VisibilityReach`, `anchored`, and the reach→text rendering. `VisibilityReach` has no `PartialEq` and deliberately no `Ord`/`PartialOrd`, so "the annotation's reach **equals** the chain's required reach" must be spelled `lhs.compare(rhs, tcx) == Some(Ordering::Equal)`, never `==` on the reaches themselves; `compare` returning `None` means two restricted scopes are incomparable siblings, which for this matrix is a mismatch, not an error. `reach()` is a method taking `(target: LocalDefId, tcx: TyCtxt<'_>)`, so every matrix dispatch site must carry the item's own `LocalDefId`. Phase 2 supplies `HeadlineSource::FindingMessage` on both forbidden specs plus the cargo-JSON no-duplicate helper. Phase 3 supplies written-form dispatch, `ItemCategory::{Declaration, Use}`, field rejection, and reject-once ordering. Phase 5 supplies `ParentFacadeAnalysis` with `nearest: Vec<ParentFacadeOccurrence>` — non-empty by construction, guaranteed by the `Option<ParentFacadeAnalysis>` wrapper rather than by the type — each carrying `FacadeSyntax` and a `OnceCell` usage cache, plus `FacadeChainResolution::{Resolved, Unresolvable}`, resolved once per item. Phase 6 supplies `PubInPath` on the resolved `VisibilityConfig`.

**Pending decision:** the dynamic `suspicious_pub` suggestion collides with that diagnostic's static help, and the two renderers disagree about which one wins.

Actual problem:
This phase's case table (`Required` + bare `pub` + exact restricted facade → `suspicious_pub` with a dynamic suggestion) and Phase 10 both require setting `finding.suggestion` on a `SuspiciousPub` finding. That field is `None` for every `suspicious_pub` finding today (`src/compiler/visibility/scan/record.rs:482`), so no code path has ever had both a custom suggestion and the spec's static help in play at once. `SUSPICIOUS_PUB` carries `inline_help: Some("consider using: `pub(super)`")` (`src/reporting/diagnostics.rs:123`) — advice that is wrong for an item sitting behind a facade, where the correct annotation names the facade's boundary.

What exists now:
- Human output picks the custom suggestion: `src/reporting/render/diagnostic.rs:52-53` is `custom_inline_help_text(..).or_else(inline_help_text)`.
- Cargo JSON picks the static one: `src/reporting/cargo_json.rs:209-211` is `inline_help_text(..).or_else(custom_inline_help_text)` — so `cargo mend --message-format=json` would print `consider using: pub(super)` while the terminal printed the boundary annotation.
- `rustc_diagnostic` (`src/reporting/cargo_json.rs:154-167`) sidesteps the choice by emitting **both** as separate `help` children, so a machine consumer sees two contradictory suggestions on one finding.
- Neither this phase nor Phase 10 lists `src/reporting/cargo_json.rs` in its **Files**, so as written neither one can fix this.

What should change:
- Clear `SUSPICIOUS_PUB.inline_help` to `None` and let every `suspicious_pub` finding carry its own suggestion, computed at the site that knows whether a facade is involved — the boundary-aware string in the facade case, the existing `pub(super)` text otherwise.
- Add `src/reporting/cargo_json.rs` and `src/reporting/render/diagnostic.rs` to this phase's **Files** either way, since the precedence must end up identical in both renderers.

Recommendation:
Take the first option in this phase. Making the suggestion always dynamic removes the precedence question instead of answering it, and it is the only version where the two output modes cannot drift again. Unifying the precedence and keeping the static fallback is the smaller diff, but it leaves `rustc_diagnostic` emitting two `help` children that a consumer has to rank, and leaves the wrong advice reachable whenever the dynamic suggestion is absent.

Approve this direction, or modify it?

**Pending decision:** whether to split this phase in two before dispatching it.

Actual problem:
Phase 3 absorbed the written-form dispatch skeleton this phase was scoped around, and everything Phase 3 did *not* absorb is still here. Phase 7 now carries five separable jobs: boundary acceptance, the eleven-row advice matrix, the ordered no-facade caller classifier (which needs `caller_aware.rs` plumbing), `suspicious_pub` admission, and the renderer-precedence decision spanning `cargo_json.rs` and `render/diagnostic.rs`. That is well past the one-implementer-one-pass sizing every other phase in this plan holds to.

What exists now:
- One `### Phase 7` Work Order covering all five jobs, with an acceptance gate enumerating roughly twenty distinct fixture requirements plus three inherited test obligations.
- Two of the five jobs (acceptance, matrix) touch the same two recorders, so they cannot be done blind to each other.

What should change:
- Split into **7a — acceptance**: narrow `record_forbidden_pub_in_crate` to fire unless the reach matches the chain, wire the two new parameters, keep today's headlines verbatim. Verifiable on its own against the existing `tests/diagnostics/forbidden_pub_in_crate.rs` with no headline churn.
- And **7b — advice matrix + no-facade classifier + `suspicious_pub` admission + renderer precedence**: where every headline, help string, and test-support assertion churns.

Recommendation:
Split. 7a is a small, independently green change and 7b is where the risk concentrates; landing them together means one review pass over both. **Cost to weigh:** splitting renumbers Phases 8-11 to 9-12 and requires rewriting every cross-reference in the doc per the plan's own numbering rule — including the `Constraints from prior phases` lines that name "Phase 7" and the two other `**Pending decision:**` blocks. If that churn is judged worse than a single large phase, keep Phase 7 whole and dispatch it with the five jobs listed explicitly in its Spec so the implementer sequences them deliberately.

**Acceptance gate:** `verify.sh check`, `verify.sh test`, `verify.sh lint` green, **plus `bash ~/.claude/scripts/delegate/verify.sh test cargo-mend diagnostics` — the bare `verify.sh test` line runs only `--lib`/`--bins`, so every fixture under `tests/diagnostics/` is invisible to it and a phase whose only new tests live there would gate green having run none of them,** **and the self-policy gate: `RUSTC_BOOTSTRAP=1 cargo +stable run --release -- --workspace --all-targets --fail-on-warn` reports "No findings" on cargo-mend's own source.** That last check is not redundant with `lint` — Phase 1 shipped `check`/`test`/`lint` all green while the tool rejected its own new file, and two blind reviews missed it, because the only thing that knows cargo-mend's house rules is cargo-mend. The two rules that bite new rustc-facing code are `inline_path_qualified_type` (write `use rustc_middle::ty::Foo;` and then `Foo`, never an inline `rustc_middle::ty::Foo` in a type position) and `imports_at_top`. Fixtures: exact-boundary `pub(in ...)` behind a `pub(super) use` facade — no finding at `"permitted"`, error at `"forbidden"`; one level too wide (`pub(in crate)` where `crate::a` is required) — error at every setting with the headline quoting `pub(in crate)`; one level too narrow — a **rustc compile-fail control** asserting E0364/E0365, not a mend fixture; no facade at all + `pub(in ...)` — one fixture per classifier branch (callers all in the defining module → removal; callers within the parent scope → `pub(super)`; a caller above the parent → no annotation replacement, the two structural migrations); bare `pub` behind a `pub(super) use` facade — allowed at `"permitted"`, `suspicious_pub` at `"required"`; `pub(in super::super)` naming the correct boundary — error with the `crate::`-rooted suggestion; external-crate re-export control — no exception granted; stale accepted annotation whose facade is unused — `suspicious_pub` fires; `--fix`, `--fix-pub-use`, and `--fix-all` on that finding — no fixability note, no `StoredPubUseFixFact`, no skipped restricted candidate. Table-driven coverage across all three settings × each written syntax category, asserting diagnostic code, headline, help, and the complete code set — including acceptance under `Required`, canonical-reach-valid `pub(in crate)`, and ordinary `pub(in self)`/`pub(in super)` declarations. `policy.rs` unit tests: `forbidden_pub_crate_suggestion` names the boundary path in the `Super` facade arm.

Three test obligations this phase inherits from Phase 3:

- **Update the nine `assert_headline_and_help` pairs** in `tests/diagnostics/forbidden_pub_in_crate.rs:77-130`. They pin exact strings of the form ``use of `X` is forbidden by policy``, and the advice matrix replaces at least the `pub(in crate)` headline with the redundant-spelling wording. The same file's `assert_codes` vectors are **order-sensitive and must not change** — if they do, reject-once or the dispatch order has regressed.
- **`tests/diagnostics/rendering.rs` is a hard gate, not a soft assertion.** `:223-227` panics with "fixture is missing finding for {code:?}" if any `DiagnosticCode` fails to fire, and `:399` asserts `report.findings.len() == 16`. This phase changes which findings fire, so **adjust that fixture, never the assertion**. Note `src/private_parent/child.rs:139`'s `pub(in crate::private_parent) fn subtree_only()` has no facade and so still survives acceptance — but confirm rather than assume.
- **Close Phase 3's carried-forward field gap:** add one fixture with a canonical `pub(crate)` field in a location where `policy::allow_pub_crate_by_policy` denies, asserting `forbidden_pub_crate`, plus a `pub(super)` field asserting no finding. Phase 3 shipped that behavior but every field in its fixture is a `pub(in ...)` form, so the canonical path is currently proven only by the self-policy run on cargo-mend's own source.

---

### Phase 8 — README and style-guide updates · status: todo

#### Work Order

**Goal:** `README.md` and `~/rust/nate_style/rust/use-narrowest-visibility.md` document the new rung, the config key, and the upgrade contract.

**Spec:**

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

**A sixth README edit, in the `forbidden-pub-crate` section (`README.md:194`), not the `forbidden-pub-in-crate` one.** Phase 3 made that diagnostic fire on struct and union fields, which the section does not mention at all — it still reads as an item-only rule. Add: the rule now applies to fields as well as items, a `pub(crate)` field is rejected wherever a `pub(crate)` item would be, and `pub(super)`/`pub(self)` fields are unaffected. This is a user-visible scope change that shipped in Phase 3; without this edit the README documents behavior the tool no longer has.

**Guidance text to land in both READMEs, in the `forbidden-pub-in-crate` section:**

**Reach for `pub(in crate::path)` in exactly one situation:** a parent module re-exports the item with `pub(super) use`, which puts the item's required reach above the module it lives in. `pub(super)` at the declaration is too narrow to compile; `pub` is wider than the truth.

**The path is the parent of the module holding the facade — not one level above the item.** In the `video_plane` example the item lives in `video_plane::plane::camera_panel`, the facade lives in `video_plane::plane`, and the annotation reads `pub(in crate::video_plane)` — two levels above the item. With chained facades the distance grows. The rule is always: find the widest facade, take the parent of the module holding it.

**The path names who can see the item, not who owns it.** Reading `pub(in crate::video_plane)` as "this belongs to `video_plane`" gets the meaning backwards.

**Always spell it `crate::`-rooted.** `pub(in super::super)` compiles and names the same module, but it forces every reader to count levels, and it silently changes meaning if the file moves.

**Declarations only.** A `use` line picks its own reach, so `pub(super)`, `pub(crate)`, and `pub` already span what it can need. Fields are excluded for a different reason: a field is not re-exportable, so no facade can ever justify one.

**Do not use it to avoid moving an item.** If the path you need is long, or names a module that has nothing to do with the item, the item is in the wrong place.

**It is not a way to widen anything.** `pub(in crate::a)` is narrower than `pub(crate)` and narrower than `pub`. If reaching for it feels like unlocking access, check whether a `pub(crate) use` facade is what is actually wanted — and say so on the `use` line, which is where the decision belongs.

**Upgrade contract for `CHANGELOG.md`** — seven cases:

- *Previously green, `forbidden_pub_in_crate` enabled* — exact `pub(in crate::...)` declarations covered by the scanner were absent, but `pub(in crate)`, relative spellings, and restricted field annotations may become new errors. Output is unchanged only when none of the newly detected forms exist.
- *`forbidden_pub_in_crate` disabled* — existing annotations may be present. Exact boundaries become permitted, while `suspicious_pub` or another canonical diagnostic may newly appear; disabling `forbidden_pub_in_crate` does not suppress those codes.
- *`forbidden_pub_in_crate` disabled but `forbidden_pub_crate` enabled* — the new `pub(in crate)` detection routes to the *enabled* code, so a project that believed it had opted out still gets new errors.
- *Already failing* — an exact-boundary error may disappear under `Permitted`; retained failures get new headlines and help; secondary findings may change count and order.
- *Machine-readable output shape changed* — the other four cases are about which findings fire; this one is about their JSON. Phase 2 stopped `rustc_diagnostic` emitting the `note` child that duplicated the headline (`src/reporting/cargo_json.rs:148-150`) and stopped `render_diagnostic` repeating it in `rendered` (`:220-224`), so any consumer parsing mend's cargo JSON sees one fewer child on **every** forbidden-visibility finding, including projects whose findings are otherwise unchanged.

- *Struct and union fields now follow the `pub(crate)` location policy* — shipped in Phase 3, not Phase 7. `check_field` runs the shared rejection classifier and passes `None` for the facade argument (`src/compiler/visibility/field.rs:102`), so a canonical `pub(crate)` field reaches `record_forbidden_pub_crate` and errors wherever `policy::allow_pub_crate_by_policy` does not permit it — the same rule that already governed `pub(crate)` items. This is narrower than "every `pub(crate)` field is now an error": `pub(super)` and `pub(self)` fields remain allowed, and permitted locations stay green. It is still the widest-reaching behavior change in this feature for existing codebases, because struct fields were previously exempt from the rule entirely.
- *Reject-once changed which codes co-occur* — a forbidden `pub(crate) mod` previously emitted both `forbidden_pub_crate` and `review_pub_mod`; it now emits only the rejection. Finding counts drop and code sets change for projects whose output is otherwise unaffected, which matters to anyone asserting on totals.

The first three need release-note compatibility bullets, stated as the two-code matrix and using the word *disable* — `[diagnostics]` cannot downgrade. The fourth belongs in the feature note describing newly accepted exact boundaries and revised advice. No warning-first grace mode is built: adding one would mean giving `[diagnostics]` a severity state it does not have, in the middle of this feature, to defer a handful of one-line annotation edits.

**Files:**
- `README.md` — `:194`, `:209-213`, `:258`, `:287`, `~:70-99`, `~:184-190`
- `CHANGELOG.md` — the five upgrade cases
- `~/rust/nate_style/rust/use-narrowest-visibility.md` — ladder, depth-3 section, decision table, Tooling line

**Constraints from prior phases:** Phases 3, 6, and 7 define the behavior being documented: the two-code split for rejections, `pub_in_path` with project>global precedence and a `permitted` default, and acceptance limited to exact-boundary declarations. `FINDINGS_SCHEMA_VERSION` was bumped to `18` in Phase 3.

**Acceptance gate:** `verify.sh check`, `verify.sh test`, `verify.sh lint` green, **plus `bash ~/.claude/scripts/delegate/verify.sh test cargo-mend diagnostics` — the bare `verify.sh test` line runs only `--lib`/`--bins`, so every fixture under `tests/diagnostics/` is invisible to it and a phase whose only new tests live there would gate green having run none of them,** **and the self-policy gate: `RUSTC_BOOTSTRAP=1 cargo +stable run --release -- --workspace --all-targets --fail-on-warn` reports "No findings" on cargo-mend's own source.** That last check is not redundant with `lint` — Phase 1 shipped `check`/`test`/`lint` all green while the tool rejected its own new file, and two blind reviews missed it, because the only thing that knows cargo-mend's house rules is cargo-mend. The two rules that bite new rustc-facing code are `inline_path_qualified_type` (write `use rustc_middle::ty::Foo;` and then `Foo`, never an inline `rustc_middle::ty::Foo` in a type position) and `imports_at_top`. Every `<a id="...">` anchor referenced by a `DiagnosticSpec.help_anchor` still resolves in `README.md`. `docs/style/diagnostic-lifecycle.md`'s README checklist items are satisfied for both forbidden diagnostics.

---

### Phase 9 — Signature exposure returns a level · status: todo

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

Changing the four predicates behind `assess_signature_exposure_allowance` (`policy.rs:318`) from `bool` to a level is **not sufficient**. They return on first match, collapse recursion to `bool`, and their visitor (`exposure/visitor.rs:62`) inspects only bare-`pub` exposing items — the exposing item's `DefId` and scope are already discarded before a level could be recovered.

Each path must retain the resolved exposing item, compute its effective reach, and `join` it with the facade-required reach. The result type is `Option<VisibilityReach>`; `None` means no exposure, which retires `SignatureExposure::{Present, Absent}` (`scan/classify.rs:27`).

**Anchor every exposure reach before accumulating it.** Sibling scopes are reachable here — an exposing signature can be a sibling or parent boundary reaching past the facade, which is why there are four distinct predicates rather than one. With a single exposure there is no second operand to trigger `join`'s common-ancestor branch, so a lone sibling reach would render as a sibling path and hit E0742. Apply `anchored(reach, target, tcx)` from Phase 1 to each exposure reach, then join the normalized reaches.

Wording fix in the same arm: "a narrower modifier would not compile" overstates the rule. It is enforced by the `private_interfaces` lint, which warns rather than errors in current Rust. Confirm and soften.

**Files:**
- `src/compiler/visibility/policy.rs` — `assess_signature_exposure_allowance` (`:318`) and its four predicates return `Option<VisibilityReach>`
- `src/compiler/exposure/visitor.rs` — retain the resolved exposing item (`:62`)
- `src/compiler/exposure/detect.rs` — recursion carries a reach, not a `bool` (`:376`)
- `src/compiler/visibility/scan/classify.rs` — retire `SignatureExposure` (`:27`)
- `src/compiler/visibility/annotation.rs` — source of `VisibilityReach`, `join`, and the free `anchored(reach, target, tcx)`. `src/compiler/exposure/` is outside `crate::compiler::visibility`, so this phase needs the same cross-module reach Phase 5 established: bare `pub` on the items plus a `pub(super) use` in `src/compiler/visibility/mod.rs`. If Phase 5 already did it, this phase adds nothing here — check before editing. Do **not** convert to `pub(in crate::compiler)`; that spelling is rejected by this crate's own `record_forbidden_pub_in_crate` (`scan/record.rs:270-308`) until Phase 7 lands boundary acceptance
- `src/compiler/visibility/mod.rs` — the `pub(super) use` re-exports above, if Phase 5 has not already added them. **Also delete the `#[expect(dead_code)]` wrapping `mod annotation;` (`:1-5`) if it is still present.** Clippy is deny-by-default here, so once the last item in `annotation.rs` has a consumer the unfulfilled expectation is itself a lint failure — `anchored` is the final holdout and this phase is its first caller
- `tests/diagnostics/allowances.rs` — exposure fixtures

**Constraints from prior phases:** Phase 1 supplies `VisibilityReach::join` and the free fn `anchored(reach, target, tcx)`, both in `src/compiler/visibility/annotation.rs` and both shipped `pub(super)` — see **Files** for how this phase reaches them from `src/compiler/exposure/`. `VisibilityReach` derives only `Clone, Copy`: it has no `Debug`, no `PartialEq`, and deliberately no `Ord`/`PartialOrd`, so a struct holding one cannot `#[derive(Debug, PartialEq)]` and an equality test must be spelled `lhs.compare(rhs, tcx) == Some(Ordering::Equal)`. `compare` returning `None` means two restricted scopes are incomparable siblings — not an error; feed such pairs to `join`, which returns their nearest common ancestor. Phase 5 supplies the chain-required reach to join against. Phase 7 shipped with the conservative `(Present, _)`-first arm ordering, which this phase replaces; its acceptance behavior for non-exposed items must not regress. **Phase 3 added a field entry point into this analysis.** `record_forbidden_pub_crate` calls `policy::has_signature_exposure_allowance(ctx, item.file_path, item.name)` at `scan/record.rs:235-236`, and struct/union fields now reach it with `item.name = Some(<field name>)` because `check_field` (`src/compiler/visibility/field.rs:85-104`) routes fields through the shared classifier. Converting the predicates to return `Option<VisibilityReach>` must therefore not silently change field behavior — add `src/compiler/visibility/field.rs` to this phase's awareness list even though it is not edited.

**Acceptance gate:** `verify.sh check`, `verify.sh test`, `verify.sh lint` green, **plus `bash ~/.claude/scripts/delegate/verify.sh test cargo-mend diagnostics` — the bare `verify.sh test` line runs only `--lib`/`--bins`, so every fixture under `tests/diagnostics/` is invisible to it and a phase whose only new tests live there would gate green having run none of them,** **and the self-policy gate: `RUSTC_BOOTSTRAP=1 cargo +stable run --release -- --workspace --all-targets --fail-on-warn` reports "No findings" on cargo-mend's own source.** That last check is not redundant with `lint` — Phase 1 shipped `check`/`test`/`lint` all green while the tool rejected its own new file, and two blind reviews missed it, because the only thing that knows cargo-mend's house rules is cargo-mend. The two rules that bite new rustc-facing code are `inline_path_qualified_type` (write `use rustc_middle::ty::Foo;` and then `Foo`, never an inline `rustc_middle::ty::Foo` in a type position) and `imports_at_top`. Fixtures: the `video_plane` snippet above is accepted at `"permitted"`; a **single sibling signature exposure** yields a common-ancestor boundary, not the sibling scope, and the suggested code compiles; an exposure reaching past the facade still forces the wider annotation. Existing allowance tests pass unchanged. **`tests/diagnostics/rendering.rs` is a hard gate:** `:223-227` panics with "fixture is missing finding for {code:?}" if any `DiagnosticCode` fails to fire and `:399` asserts `report.findings.len() == 16`. This phase changes which findings fire, so the all-diagnostics fixture must still emit all 16, one per code — adjust the fixture, never the assertion.

---

### Phase 10 — `required` mode · status: todo

#### Work Order

**Goal:** at `pub_in_path = "required"`, bare `pub` behind an exact restricted facade fires `suspicious_pub`, and hana is converted.

**Spec:**

Drop `AllowanceReason::InternalParentFacadeBoundary` for bare `pub` when the setting is `Required`. "Drop the allowance at `Required`" is too broad as stated: a conforming exact annotation still needs that allowance when its facade is used — **only bare `pub` loses it**.

Then flip hana to `pub_in_path = "required"` in its committed `mend.toml` and convert its 10 sites by hand. Decide from that experience whether `Required` becomes the default; record the decision in `CHANGELOG.md` either way.

The "yes, make it the default" branch costs more than hana's 10 sites: cargo-mend itself uses the bare-`pub`-behind-a-`pub(super) use`-facade shape at roughly **51** sites (`compiler/facade/exports.rs:31` + `compiler/facade/mod.rs:8` is the canonical one), and that shape is exactly what `Required` converts to `pub(in crate::path)`. Defaulting to `Required` therefore also means converting those ~51 declarations in this repo, or the self-policy gate goes red the moment the default flips. Price that in before choosing; the "no, keep `Permitted` as the default" branch has no such cost. **Treat ~51 as a lower bound and re-derive the count at dispatch time:** Phase 3 converted four struct fields in `src/config/cli/fix.rs` and `src/config/cli/target.rs` from `pub(crate)` to bare `pub` to satisfy the self-policy gate, and bare `pub` behind a facade is precisely the shape `Required` converts, so this phase's own predecessor added to the total.

**Files:**
- `src/compiler/visibility/policy.rs` — `assess_parent_facade_usage` (`:289`), the `Required` branch
- `tests/diagnostics/allowances.rs` — `Required`-mode fixtures
- `CHANGELOG.md` — the default decision
- (external, not in this repo) hana's `mend.toml` and its 10 conversion sites

**Constraints from prior phases:** Phase 6 supplies `PubInPath::Required` on the resolved config. Phase 7 supplies the `suspicious_pub` case table, in which `Required` + bare `pub` + exact restricted facade already yields a dynamic suggestion; this phase is what removes the allowance that currently pre-empts it. Phase 9 supplies exposure levels, so an item whose signature genuinely requires `pub` is not swept into this.

**Acceptance gate:** `verify.sh check`, `verify.sh test`, `verify.sh lint` green, **plus `bash ~/.claude/scripts/delegate/verify.sh test cargo-mend diagnostics` — the bare `verify.sh test` line runs only `--lib`/`--bins`, so every fixture under `tests/diagnostics/` is invisible to it and a phase whose only new tests live there would gate green having run none of them,** **and the self-policy gate: `RUSTC_BOOTSTRAP=1 cargo +stable run --release -- --workspace --all-targets --fail-on-warn` reports "No findings" on cargo-mend's own source.** That last check is not redundant with `lint` — Phase 1 shipped `check`/`test`/`lint` all green while the tool rejected its own new file, and two blind reviews missed it, because the only thing that knows cargo-mend's house rules is cargo-mend. The two rules that bite new rustc-facing code are `inline_path_qualified_type` (write `use rustc_middle::ty::Foo;` and then `Foo`, never an inline `rustc_middle::ty::Foo` in a type position) and `imports_at_top`. Fixtures: bare `pub` behind a used exact restricted facade is silent at `"permitted"` and fires `suspicious_pub` with the exact-boundary suggestion at `"required"`; an accepted exact annotation behind the same used facade stays silent at `"required"`. hana reports zero findings after conversion. **`tests/diagnostics/rendering.rs` is a hard gate:** `:223-227` panics with "fixture is missing finding for {code:?}" if any `DiagnosticCode` fails to fire and `:399` asserts `report.findings.len() == 16`. This phase changes which findings fire, so the all-diagnostics fixture must still emit all 16, one per code — adjust the fixture, never the assertion.

---

### Phase 11 — Auto-fix for restricted annotations · status: todo

#### Work Order

**Goal:** `cargo mend --fix` rewrites a bare `pub` to the exact annotation, and no fixer silently no-ops on a restricted annotation.

**Spec:**

`fixes/unused_pub.rs:15` and `fixes/narrow_pub_crate.rs:15` both search for bare `"pub "`. Share the annotation-span parser at `fixes/field_visibility.rs:89-103` (`visibility_annotation_byte_len`) with both, so a restricted annotation's span is computed rather than string-matched.

This deliberately does not reuse Phase 1's `annotation.rs` parser, and that is not an oversight: `visibility_annotation_byte_len` answers "how many bytes of this raw source line does the visibility annotation occupy", while `annotation.rs:103-148` uses `syn` to answer "which of the eight visibility forms is this". Different questions, and the `fixes/` tree cannot see `pub(super)` items inside `compiler/visibility/` anyway.

Then wire `suspicious_pub`'s facade arm from `FixSupport::None` to `FixSupport::Standard` so the fix rewrites `pub` to the exact annotation. The fix must also confirm the facade line itself needs no edit before applying — that is why this is last.

Add `--fix` assertions to the stale-annotation tests: the fixer either edits correctly or reports no fix, never a silent no-op.

**Files:**
- `src/fixes/field_visibility.rs` — extract `visibility_annotation_byte_len` (`:89-103`) into a shared helper
- `src/fixes/unused_pub.rs` — use it (`:15`)
- `src/fixes/narrow_pub_crate.rs` — use it (`:15`)
- `src/compiler/visibility/scan/record.rs` — `suspicious_pub` fix support (`:418`)
- `tests/diagnostics/unused_pub.rs`, `tests/diagnostics/narrow_pub_crate.rs`, `tests/diagnostics/pub_use_fixes.rs` — `--fix` assertions

**Constraints from prior phases:** Phase 7 set the stale-facade finding to `FixSupport::None` **and** suppressed its `StoredPubUseFixFact`; this phase re-enables the first while keeping the pub-use fixer out of the path, since that fixer routes from stored facts and only accepts a bare `"pub "` child (`fixes/pub_use_fixes/scan.rs:173`). Phase 10 made `Required` produce these findings in volume.

**Pending decision:** this phase's Spec names a `FixSupport` variant that does not exist, and adding one changes the persisted findings schema.

Actual problem:
The Spec says to wire `suspicious_pub`'s facade arm from `FixSupport::None` to `FixSupport::Standard`. There is no `Standard` variant on `FixSupport` (`src/reporting/diagnostics.rs:14-30`). The name exists on `FixSummaryBucket` (`:33`), which is a different type serving a different purpose, so this reads as a plausible instruction that will not compile.

What exists now:
- `FixSupport` variants are all fixer-specific: `ShortenImport`, `PreferModuleImport`, `InlinePathQualifiedType`, `PubUse`, `NeedsManualPubUseCleanup`, `InternalParentFacade`, `UnusedPub`, `NarrowToPubCrate`, `FieldVisibility`, `ImportsAtTop`, plus `None`.
- The enum is `Serialize, Deserialize` with `#[serde(rename_all = "snake_case")]` and several explicit `#[serde(rename = ...)]` overrides, and its value is persisted as the `fixability` field on every stored finding.
- Fix appliers route by `DiagnosticCode`, not by `FixSupport` (`src/fixes/runner/execute.rs:64-90`).
- `tests/support/diagnostics.rs` maintains a parallel copy of the enum — Phase 3 moved it: the `FixSupport` mirror is now `:60-85` with its `impl` at `:86`, and `:115-140` is the `DiagnosticSpec` / `HeadlineSource` block Phase 3 added that must be kept in step.

What should change:
- Add a purpose-named variant — `RestrictedAnnotation` rather than `Standard`, matching how every other variant names its fixer — with a serde name, arms in `note()` (`:39`) and `summary_bucket()` (`:53`), the mirrored variant in the test support copy, and an applier registered in the `DiagnosticCode` routing table.
- Decide whether the new `fixability` string requires a `FINDINGS_SCHEMA_VERSION` bump (`src/compiler/constants.rs:62`; Phase 3 already takes it to `18`). A cache written by an older mend has no such value, so only newly written reports carry it — but the plan's own invariant is that a change to emitted findings bumps the version.

Recommendation:
Add `FixSupport::RestrictedAnnotation` and bump the schema version in this phase. The bump is cheap — it invalidates caches, nothing more — and the alternative is relying on the argument that old caches never contain the new string, which is exactly the kind of reasoning the schema invariant exists to make unnecessary. Confirm the variant name before the phase is dispatched, since it lands in the persisted format and in the test mirror.

Approve this direction, or modify it?

**Acceptance gate:** `verify.sh check`, `verify.sh test`, `verify.sh lint` green, **plus `bash ~/.claude/scripts/delegate/verify.sh test cargo-mend diagnostics` — the bare `verify.sh test` line runs only `--lib`/`--bins`, so every fixture under `tests/diagnostics/` is invisible to it and a phase whose only new tests live there would gate green having run none of them,** **and the self-policy gate: `RUSTC_BOOTSTRAP=1 cargo +stable run --release -- --workspace --all-targets --fail-on-warn` reports "No findings" on cargo-mend's own source.** That last check is not redundant with `lint` — Phase 1 shipped `check`/`test`/`lint` all green while the tool rejected its own new file, and two blind reviews missed it, because the only thing that knows cargo-mend's house rules is cargo-mend. The two rules that bite new rustc-facing code are `inline_path_qualified_type` (write `use rustc_middle::ty::Foo;` and then `Foo`, never an inline `rustc_middle::ty::Foo` in a type position) and `imports_at_top`. `cargo mend --fix` on a `Required`-mode fixture rewrites bare `pub` to the exact annotation and leaves the facade line untouched; `--fix`, `--fix-pub-use`, and `--fix-all` on every restricted-annotation finding either produce a correct edit or report no fix — never success with no edit.
