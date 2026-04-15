use std::fs;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use rustc_hir::ForeignItem;
use rustc_hir::ForeignItemKind;
use rustc_hir::ImplItem;
use rustc_hir::ImplItemKind;
use rustc_hir::Item;
use rustc_hir::ItemKind;
use rustc_middle::middle::privacy::Level;
use rustc_middle::ty::TyCtxt;
use rustc_span::FileName;
use rustc_span::Span;
use rustc_span::def_id::CRATE_DEF_ID;
use rustc_span::def_id::LocalDefId;

use super::exposure;
use super::facade;
use super::facade::ParentFacadeExportStatus;
use super::facade::ParentFacadeUsage;
use super::facade::ParentFacadeVisibility;
use super::persistence;
use super::persistence::FindingsSink;
use super::persistence::StoredFinding;
use super::persistence::StoredPubUseFixFact;
use super::persistence::StoredReport;
use super::settings;
use super::settings::DriverSettings;
use super::source_cache;
use super::source_cache::SourceCache;
use crate::config::DiagnosticCode;
use crate::constants::FINDINGS_SCHEMA_VERSION;
use crate::diagnostics::CompilerWarningFacts;
use crate::diagnostics::Severity;
use crate::fix_support::FixSupport;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CrateKind {
    Binary,
    Library,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ModuleLocation {
    CrateRoot,
    TopLevelPrivateModule,
    NestedModule,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ParentVisibility {
    Public,
    Private,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ItemCategory {
    Module,
    NonModule,
}

struct VisibilityContext<'a, 'tcx> {
    tcx:                    TyCtxt<'tcx>,
    settings:               &'a DriverSettings,
    src_root:               &'a Path,
    root_module:            &'a Path,
    effective_visibilities: &'a rustc_middle::middle::privacy::EffectiveVisibilities,
    source_cache:           &'a SourceCache,
}

struct ItemInfo<'a> {
    def_id:         LocalDefId,
    file_path:      &'a Path,
    vis_text:       &'a str,
    kind_label:     Option<&'static str>,
    item_name:      Option<&'a str>,
    highlight_span: Span,
    item_category:  ItemCategory,
    impl_self_name: Option<String>,
}

struct SuspiciousPubInput<'a> {
    def_id:            LocalDefId,
    file_path:         &'a Path,
    config_rel_path:   Option<&'a str>,
    parent_visibility: ParentVisibility,
    module_location:   ModuleLocation,
    crate_kind:        CrateKind,
    kind_label:        Option<&'static str>,
    item_name:         Option<&'a str>,
    highlight_span:    Span,
}

struct FindingParams {
    severity:   Severity,
    code:       DiagnosticCode,
    item:       Option<String>,
    message:    String,
    suggestion: Option<String>,
    fixability: FixSupport,
    related:    Option<String>,
}

struct VisibilityFindingContext {
    crate_kind:        CrateKind,
    config_rel_path:   Option<String>,
    module_location:   ModuleLocation,
    parent_visibility: ParentVisibility,
}

#[derive(Debug)]
struct LineDisplay {
    line:          usize,
    column:        usize,
    highlight_len: usize,
    source_line:   String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AllowanceReason {
    Allowlist,
    ParentIsPublic,
    TopLevelPrivateModulePolicy,
    ReachablePublicApi,
    ParentFacadeUsedOutsideParent,
    InternalParentFacadeBoundary,
    ExposedByOtherCrateVisibleSignature,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SuspiciousPubAssessment {
    Allowed(AllowanceReason),
    ReviewInternalParentFacade {
        related: Option<String>,
    },
    Warn {
        fixability:           FixSupport,
        related:              Option<String>,
        stale_parent_pub_use: Option<ParentFacadeExportStatus>,
    },
}

pub(super) fn collect_and_store_findings(
    tcx: TyCtxt<'_>,
    settings: &DriverSettings,
) -> Result<bool> {
    let crate_root_file = real_file_path(tcx, tcx.def_span(CRATE_DEF_ID))
        .context("failed to determine local crate root file")?;
    let Some(src_root) =
        source_cache::analysis_source_root_for(&crate_root_file, &settings.package_root)
    else {
        return Ok(false);
    };

    let mut sink = FindingsSink::default();
    let crate_items = tcx.hir_crate_items(());
    let cache_roots: Vec<&Path> = if settings.config_root == settings.package_root {
        vec![&src_root]
    } else {
        vec![&src_root, &settings.config_root]
    };
    let source_cache = SourceCache::build(&cache_roots)?;
    let ctx = VisibilityContext {
        tcx,
        settings,
        src_root: &src_root,
        root_module: &crate_root_file,
        effective_visibilities: tcx.effective_visibilities(()),
        source_cache: &source_cache,
    };

    for item_id in crate_items.free_items() {
        let item = tcx.hir_item(item_id);
        analyze_item(&ctx, item, &mut sink)?;
    }

    for item_id in crate_items.impl_items() {
        let item = tcx.hir_impl_item(item_id);
        analyze_impl_item(&ctx, item, &mut sink)?;
    }

    for item_id in crate_items.foreign_items() {
        let item = tcx.hir_foreign_item(item_id);
        analyze_foreign_item(&ctx, item, &mut sink)?;
    }

    let output_path = settings.findings_dir.join(persistence::cache_filename_for(
        &settings.package_root,
        &crate_root_file,
    ));
    let stored_crate_root = if crate_root_file.is_absolute() {
        crate_root_file.clone()
    } else {
        settings.config_root.join(&crate_root_file)
    };
    if !sink.findings.is_empty() {
        sink.findings.sort_by(|a, b| {
            (&a.path, a.line, a.column, &a.code, &a.item, &a.message)
                .cmp(&(&b.path, b.line, b.column, &b.code, &b.item, &b.message))
        });
        sink.findings.dedup_by(|a, b| {
            a.code == b.code
                && a.path == b.path
                && a.line == b.line
                && a.column == b.column
                && a.message == b.message
                && a.item == b.item
        });
    }

    let report = StoredReport {
        version:              FINDINGS_SCHEMA_VERSION,
        analysis_fingerprint: settings.analysis_fingerprint.clone(),
        scope_fingerprint:    settings.scope_fingerprint.clone(),
        package_root:         settings.package_root.to_string_lossy().into_owned(),
        crate_root_file:      stored_crate_root.to_string_lossy().into_owned(),
        config_fingerprint:   settings.config_fingerprint.clone(),
        findings:             sink.findings,
        pub_use_fix_facts:    sink.pub_use_fix_facts,
        compiler_warnings:    CompilerWarningFacts::None,
    };
    fs::write(&output_path, serde_json::to_vec_pretty(&report)?)
        .with_context(|| format!("failed to write findings file {}", output_path.display()))?;
    Ok(true)
}

fn analyze_item(
    ctx: &VisibilityContext<'_, '_>,
    item: &Item<'_>,
    sink: &mut FindingsSink,
) -> Result<()> {
    if item.span.from_expansion() || item.vis_span.from_expansion() {
        return Ok(());
    }
    let Some(file_path) = real_file_path(ctx.tcx, item.vis_span) else {
        return Ok(());
    };
    let Some(vis_text) = visibility_text(ctx.tcx, item.vis_span)? else {
        return Ok(());
    };

    let item_name = item.kind.ident().map(|ident| ident.to_string());

    if vis_text == "pub"
        && is_boundary_file(ctx.src_root, ctx.root_module, &file_path)
        && matches!(item.kind, ItemKind::Use(..))
        && use_item_contains_glob(ctx.tcx, item.span)?
    {
        sink.findings.push(build_finding(
            ctx.tcx,
            &file_path,
            item.span,
            FindingParams {
                severity:   Severity::Warning,
                code:       DiagnosticCode::WildcardParentPubUse,
                item:       None,
                message:    String::new(),
                suggestion: None,
                fixability: FixSupport::None,
                related:    None,
            },
        )?);
    }

    record_visibility_findings(
        ctx,
        &ItemInfo {
            def_id:         item.owner_id.def_id,
            file_path:      &file_path,
            vis_text:       &vis_text,
            kind_label:     item_kind_label(item.kind),
            item_name:      item_name.as_deref(),
            highlight_span: highlight_span(
                item.vis_span,
                item.kind.ident().map(|ident| ident.span),
            ),
            item_category:  if matches!(item.kind, ItemKind::Mod(..)) {
                ItemCategory::Module
            } else {
                ItemCategory::NonModule
            },
            impl_self_name: None,
        },
        sink,
    )
}

fn analyze_impl_item(
    ctx: &VisibilityContext<'_, '_>,
    item: &ImplItem<'_>,
    sink: &mut FindingsSink,
) -> Result<()> {
    let Some(vis_span) = item.vis_span() else {
        return Ok(());
    };
    if item.span.from_expansion() || vis_span.from_expansion() {
        return Ok(());
    }
    let Some(file_path) = real_file_path(ctx.tcx, vis_span) else {
        return Ok(());
    };
    let Some(vis_text) = visibility_text(ctx.tcx, vis_span)? else {
        return Ok(());
    };

    let item_name = item.ident.to_string();

    let impl_self_name = impl_self_type_name_from_tcx(ctx.tcx, item.owner_id.def_id);

    record_visibility_findings(
        ctx,
        &ItemInfo {
            def_id: item.owner_id.def_id,
            file_path: &file_path,
            vis_text: &vis_text,
            kind_label: Some(impl_item_kind_label(item.kind)),
            item_name: Some(item_name.as_str()),
            highlight_span: highlight_span(vis_span, Some(item.ident.span)),
            item_category: ItemCategory::NonModule,
            impl_self_name,
        },
        sink,
    )
}

fn analyze_foreign_item(
    ctx: &VisibilityContext<'_, '_>,
    item: &ForeignItem<'_>,
    sink: &mut FindingsSink,
) -> Result<()> {
    if item.span.from_expansion() || item.vis_span.from_expansion() {
        return Ok(());
    }
    let Some(file_path) = real_file_path(ctx.tcx, item.vis_span) else {
        return Ok(());
    };
    let Some(vis_text) = visibility_text(ctx.tcx, item.vis_span)? else {
        return Ok(());
    };

    let item_name = item.ident.to_string();

    record_visibility_findings(
        ctx,
        &ItemInfo {
            def_id:         item.owner_id.def_id,
            file_path:      &file_path,
            vis_text:       &vis_text,
            kind_label:     Some(foreign_item_kind_label(item.kind)),
            item_name:      Some(item_name.as_str()),
            highlight_span: highlight_span(item.vis_span, Some(item.ident.span)),
            item_category:  ItemCategory::NonModule,
            impl_self_name: None,
        },
        sink,
    )
}

fn record_visibility_findings(
    ctx: &VisibilityContext<'_, '_>,
    item: &ItemInfo<'_>,
    sink: &mut FindingsSink,
) -> Result<()> {
    let finding_context = visibility_finding_context(ctx, item);

    if matches!(item.vis_text, "pub(crate)")
        && !allow_pub_crate_by_policy(
            finding_context.crate_kind,
            finding_context.module_location,
            finding_context.parent_visibility,
        )
    {
        sink.findings.push(build_finding(
            ctx.tcx,
            item.file_path,
            item.highlight_span,
            FindingParams {
                severity:   Severity::Error,
                code:       DiagnosticCode::ForbiddenPubCrate,
                item:       None,
                message:    "use of `pub(crate)` is forbidden by policy".to_string(),
                suggestion: Some(
                    forbidden_pub_crate_help(finding_context.module_location).to_string(),
                ),
                fixability: FixSupport::None,
                related:    None,
            },
        )?);
    }

    if item.vis_text.starts_with("pub(in crate::") {
        sink.findings.push(build_finding(
            ctx.tcx,
            item.file_path,
            item.highlight_span,
            FindingParams {
                severity:   Severity::Error,
                code:       DiagnosticCode::ForbiddenPubInCrate,
                item:       None,
                message:    "use of `pub(in crate::...)` is forbidden by policy".to_string(),
                suggestion: None,
                fixability: FixSupport::None,
                related:    None,
            },
        )?);
    }

    if item.item_category == ItemCategory::Module && item.vis_text.starts_with("pub") {
        let allowlisted = finding_context
            .config_rel_path
            .as_ref()
            .is_some_and(|path| {
                ctx.settings
                    .config
                    .allow_pub_mod
                    .iter()
                    .any(|allowed| allowed == path)
            });
        if !allowlisted {
            sink.findings.push(build_finding(
                ctx.tcx,
                item.file_path,
                item.highlight_span,
                FindingParams {
                    severity:   Severity::Error,
                    code:       DiagnosticCode::ReviewPubMod,
                    item:       item.item_name.map(str::to_owned),
                    message:    "`pub mod` requires explicit review or allowlisting".to_string(),
                    suggestion: None,
                    fixability: FixSupport::None,
                    related:    None,
                },
            )?);
        }
    }

    if item.vis_text == "pub"
        && finding_context.parent_visibility == ParentVisibility::Private
        && is_top_level_module_file(ctx.src_root, ctx.root_module, item.file_path)
    {
        maybe_record_narrow_to_pub_crate(ctx, item, sink)?;
    }

    if item.vis_text == "pub" && !is_boundary_file(ctx.src_root, ctx.root_module, item.file_path) {
        maybe_record_suspicious_pub(
            ctx,
            &SuspiciousPubInput {
                def_id:            item.def_id,
                file_path:         item.file_path,
                config_rel_path:   finding_context.config_rel_path.as_deref(),
                parent_visibility: finding_context.parent_visibility,
                module_location:   finding_context.module_location,
                crate_kind:        finding_context.crate_kind,
                kind_label:        item.kind_label,
                item_name:         item.item_name,
                highlight_span:    item.highlight_span,
            },
            sink,
        )?;
    }
    Ok(())
}

fn visibility_finding_context(
    ctx: &VisibilityContext<'_, '_>,
    item: &ItemInfo<'_>,
) -> VisibilityFindingContext {
    let crate_kind = if ctx.root_module.file_name().and_then(|name| name.to_str()) == Some("lib.rs")
    {
        CrateKind::Library
    } else {
        CrateKind::Binary
    };
    let config_rel_path = settings::config_relative_path_for_settings(item.file_path, ctx.settings);
    let parent_module = ctx.tcx.parent_module_from_def_id(item.def_id);
    let parent_visibility = if ctx
        .tcx
        .local_visibility(parent_module.to_local_def_id())
        .is_public()
    {
        ParentVisibility::Public
    } else {
        ParentVisibility::Private
    };
    let module_location = resolve_module_location(ctx.tcx, parent_module.to_local_def_id());

    VisibilityFindingContext {
        crate_kind,
        config_rel_path,
        module_location,
        parent_visibility,
    }
}

fn maybe_record_narrow_to_pub_crate(
    ctx: &VisibilityContext<'_, '_>,
    item: &ItemInfo<'_>,
    sink: &mut FindingsSink,
) -> Result<()> {
    let (Some(item_name), Some(kind_label)) = (item.item_name, item.kind_label) else {
        return Ok(());
    };
    // Check if the item itself is re-exported by the crate root.
    if facade::root_module_exports_item(
        ctx.source_cache,
        ctx.root_module,
        item.file_path,
        item_name,
    ) {
        return Ok(());
    }
    // For impl items (methods, consts, types), also check if the self type
    // is re-exported — pub methods on re-exported types must stay pub.
    if let Some(self_name) = &item.impl_self_name
        && facade::root_module_exports_item(
            ctx.source_cache,
            ctx.root_module,
            item.file_path,
            self_name,
        )
    {
        return Ok(());
    }
    sink.findings.push(build_finding(
        ctx.tcx,
        item.file_path,
        item.highlight_span,
        FindingParams {
            severity:   Severity::Warning,
            code:       DiagnosticCode::NarrowToPubCrate,
            item:       Some(format!("{kind_label} {item_name}")),
            message:    String::from(
                "item is not re-exported by the crate root — use `pub(crate)`",
            ),
            suggestion: Some(String::from("consider using: `pub(crate)`")),
            fixability: FixSupport::NarrowToPubCrate,
            related:    None,
        },
    )?);
    Ok(())
}

fn maybe_record_suspicious_pub(
    ctx: &VisibilityContext<'_, '_>,
    input: &SuspiciousPubInput<'_>,
    sink: &mut FindingsSink,
) -> Result<()> {
    let Some(kind_label) = input.kind_label else {
        return Ok(());
    };

    match classify_suspicious_pub(ctx, input)? {
        SuspiciousPubAssessment::Allowed(_) => {},
        SuspiciousPubAssessment::ReviewInternalParentFacade { related } => {
            let Some(status) = input
                .item_name
                .map(|name| {
                    facade::parent_facade_export_status(
                        ctx.source_cache,
                        ctx.settings,
                        ctx.src_root,
                        input.file_path,
                        name,
                    )
                })
                .transpose()?
                .flatten()
            else {
                return Ok(());
            };
            sink.findings.push(build_line_finding(
                ctx.source_cache,
                &status.parent_path,
                status.parent_line,
                FindingParams {
                    severity: Severity::Warning,
                    code: DiagnosticCode::InternalParentPubUseFacade,
                    item: input.item_name.map(|name| format!("pub use {name}")),
                    message: String::from(
                        "this `pub use` is used inside its parent module subtree",
                    ),
                    suggestion: None,
                    fixability: FixSupport::InternalParentFacade,
                    related,
                },
            )?);
        },
        SuspiciousPubAssessment::Warn {
            fixability,
            related,
            stale_parent_pub_use,
        } => {
            sink.findings.push(build_finding(
                ctx.tcx,
                input.file_path,
                input.highlight_span,
                FindingParams {
                    severity: Severity::Warning,
                    code: DiagnosticCode::SuspiciousPub,
                    item: input.item_name.map(|name| format!("{kind_label} {name}")),
                    message: suspicious_pub_note(input.crate_kind, kind_label),
                    suggestion: None,
                    fixability,
                    related,
                },
            )?);
            if let (Some(status), Some(item_name)) = (stale_parent_pub_use, input.item_name)
                && fixability == FixSupport::FixPubUse
            {
                let display = line_display(ctx.tcx, input.file_path, input.highlight_span)?;
                let Some(child_module) = input
                    .file_path
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .filter(|stem| *stem != "mod")
                    .map(str::to_string)
                else {
                    return Ok(());
                };
                sink.pub_use_fix_facts.push(StoredPubUseFixFact {
                    child_path: input.file_path.to_string_lossy().into_owned(),
                    child_line: display.line,
                    child_item_name: item_name.to_string(),
                    parent_path: status.parent_path.to_string_lossy().into_owned(),
                    parent_line: status.parent_line,
                    child_module,
                });
            }
        },
    }
    Ok(())
}

fn classify_suspicious_pub(
    ctx: &VisibilityContext<'_, '_>,
    input: &SuspiciousPubInput<'_>,
) -> Result<SuspiciousPubAssessment> {
    if let Some(allowance) = basic_suspicious_pub_allowance(
        ctx.settings,
        ctx.effective_visibilities,
        input.def_id,
        input.config_rel_path,
        input.parent_visibility,
        input.item_name,
    ) {
        return Ok(SuspiciousPubAssessment::Allowed(allowance));
    }

    let parent_facade_export = input
        .item_name
        .map(|name| {
            facade::parent_facade_export_status(
                ctx.source_cache,
                ctx.settings,
                ctx.src_root,
                input.file_path,
                name,
            )
        })
        .transpose()?
        .flatten();

    if let Some(assessment) = assess_parent_facade_usage(parent_facade_export.as_ref()) {
        return Ok(assessment);
    }

    if let Some(allowance) = assess_signature_exposure_allowance(
        ctx.source_cache,
        ctx.settings,
        ctx.src_root,
        input.file_path,
        input.item_name,
    )? {
        return Ok(SuspiciousPubAssessment::Allowed(allowance));
    }

    let stale_result = parent_facade_export.as_ref().and_then(|status| {
        let message = match status.usage {
            ParentFacadeUsage::Unused => format!(
                "parent module also has an `unused import` warning for this `pub use` at {}:{}",
                status.parent_rel_path, status.parent_line
            ),
            ParentFacadeUsage::UsedInsideParentSubtreeByCratePath
            | ParentFacadeUsage::UsedInsideParentSubtreeByCrateImport => format!(
                "parent `pub use` at {}:{} is only used through crate-relative paths inside its own subtree",
                status.parent_rel_path, status.parent_line
            ),
            ParentFacadeUsage::UsedInsideParentSubtreeByRelativeImport
            | ParentFacadeUsage::UsedInsideParentSubtreeByRelativePath
            | ParentFacadeUsage::UsedOutsideParentSubtree => return None,
        };
        Some((message, status))
    });

    if matches!(input.module_location, ModuleLocation::TopLevelPrivateModule)
        && stale_result.is_none()
    {
        return Ok(SuspiciousPubAssessment::Allowed(
            AllowanceReason::TopLevelPrivateModulePolicy,
        ));
    }

    let (related, fixability, stale_parent_pub_use) = match stale_result {
        Some((message, status)) => {
            let fix = if status.fix_supported {
                FixSupport::FixPubUse
            } else {
                FixSupport::NeedsManualPubUseCleanup
            };
            (Some(message), fix, Some(status.clone()))
        },
        None => (None, FixSupport::None, None),
    };

    Ok(SuspiciousPubAssessment::Warn {
        fixability,
        related,
        stale_parent_pub_use,
    })
}

fn basic_suspicious_pub_allowance(
    settings: &DriverSettings,
    effective_visibilities: &rustc_middle::middle::privacy::EffectiveVisibilities,
    def_id: LocalDefId,
    config_rel_path: Option<&str>,
    parent_visibility: ParentVisibility,
    item_name: Option<&str>,
) -> Option<AllowanceReason> {
    let item_key = config_rel_path.and_then(|path| item_name.map(|name| format!("{path}::{name}")));
    let allowlisted = item_key.as_ref().is_some_and(|key| {
        settings
            .config
            .allow_pub_items
            .iter()
            .any(|allowed| allowed == key)
    });
    if allowlisted {
        return Some(AllowanceReason::Allowlist);
    }
    if parent_visibility == ParentVisibility::Public {
        return Some(AllowanceReason::ParentIsPublic);
    }
    if effective_visibilities.is_public_at_level(def_id, Level::Reachable) {
        return Some(AllowanceReason::ReachablePublicApi);
    }
    None
}

fn assess_parent_facade_usage(
    parent_facade_export: Option<&ParentFacadeExportStatus>,
) -> Option<SuspiciousPubAssessment> {
    let status = parent_facade_export?;
    if status.visibility == ParentFacadeVisibility::Super
        && !matches!(status.usage, ParentFacadeUsage::Unused)
    {
        return Some(SuspiciousPubAssessment::Allowed(
            AllowanceReason::InternalParentFacadeBoundary,
        ));
    }
    match status.usage {
        ParentFacadeUsage::UsedOutsideParentSubtree => Some(SuspiciousPubAssessment::Allowed(
            AllowanceReason::ParentFacadeUsedOutsideParent,
        )),
        ParentFacadeUsage::UsedInsideParentSubtreeByRelativePath
        | ParentFacadeUsage::UsedInsideParentSubtreeByRelativeImport => {
            let related = Some(format!(
                "parent module uses this item as an internal facade at {}:{}",
                status.parent_rel_path, status.parent_line
            ));
            Some(SuspiciousPubAssessment::ReviewInternalParentFacade { related })
        },
        ParentFacadeUsage::UsedInsideParentSubtreeByCratePath
        | ParentFacadeUsage::UsedInsideParentSubtreeByCrateImport
        | ParentFacadeUsage::Unused => None,
    }
}

fn assess_signature_exposure_allowance(
    source_cache: &SourceCache,
    settings: &DriverSettings,
    src_root: &Path,
    file_path: &Path,
    item_name: Option<&str>,
) -> Result<Option<AllowanceReason>> {
    let Some(item_name) = item_name else {
        return Ok(None);
    };
    if exposure::child_item_is_exposed_by_other_crate_visible_signature(
        source_cache,
        settings,
        src_root,
        file_path,
        item_name,
    )? || exposure::impl_item_is_exposed_by_exported_self_type(
        source_cache,
        settings,
        src_root,
        file_path,
        item_name,
    )? || exposure::child_item_is_exposed_by_sibling_boundary_signature(
        source_cache,
        settings,
        src_root,
        file_path,
        item_name,
    )? || exposure::parent_boundary_public_signature_exposes_child_used_outside_parent(
        source_cache,
        settings,
        src_root,
        file_path,
        item_name,
    )? {
        return Ok(Some(AllowanceReason::ExposedByOtherCrateVisibleSignature));
    }
    Ok(None)
}

fn build_finding(
    tcx: TyCtxt<'_>,
    file_path: &Path,
    highlight_span: Span,
    params: FindingParams,
) -> Result<StoredFinding> {
    let display = line_display(tcx, file_path, highlight_span)?;
    Ok(StoredFinding {
        severity:      params.severity,
        code:          params.code,
        path:          file_path.to_string_lossy().into_owned(),
        line:          display.line,
        column:        display.column,
        highlight_len: display.highlight_len,
        source_line:   display.source_line,
        item:          params.item,
        message:       params.message,
        suggestion:    params.suggestion,
        fixability:    params.fixability,
        related:       params.related,
    })
}

fn build_line_finding(
    source_cache: &SourceCache,
    file_path: &Path,
    line: usize,
    params: FindingParams,
) -> Result<StoredFinding> {
    let text = source_cache.read_source(file_path)?;
    let source_line = text
        .lines()
        .nth(line.saturating_sub(1))
        .unwrap_or_default()
        .to_string();
    let trimmed = source_line.trim_start();
    let column = source_line.len().saturating_sub(trimmed.len()) + 1;
    let highlight_len = trimmed
        .find(char::is_whitespace)
        .unwrap_or(trimmed.len())
        .max(1);

    Ok(StoredFinding {
        severity: params.severity,
        code: params.code,
        path: file_path.to_string_lossy().into_owned(),
        line,
        column,
        highlight_len,
        source_line,
        item: params.item,
        message: params.message,
        suggestion: params.suggestion,
        fixability: params.fixability,
        related: params.related,
    })
}

fn resolve_module_location(tcx: TyCtxt<'_>, parent_def: LocalDefId) -> ModuleLocation {
    if parent_def == CRATE_DEF_ID {
        return ModuleLocation::CrateRoot;
    }

    let grandparent = tcx.parent_module_from_def_id(parent_def).to_local_def_id();
    if grandparent == CRATE_DEF_ID {
        return ModuleLocation::TopLevelPrivateModule;
    }

    let great_grandparent = tcx.parent_module_from_def_id(grandparent).to_local_def_id();
    if great_grandparent == CRATE_DEF_ID {
        return ModuleLocation::TopLevelPrivateModule;
    }

    ModuleLocation::NestedModule
}

fn use_item_contains_glob(tcx: TyCtxt<'_>, span: Span) -> Result<bool> {
    let snippet = tcx.sess.source_map().span_to_snippet(span).map_err(|err| {
        anyhow::anyhow!("failed to extract use item snippet for span {span:?}: {err:?}")
    })?;
    Ok(snippet.contains('*'))
}

pub(super) const fn allow_pub_crate_by_policy(
    crate_kind: CrateKind,
    module_location: ModuleLocation,
    parent_visibility: ParentVisibility,
) -> bool {
    match (crate_kind, module_location) {
        (CrateKind::Library, ModuleLocation::CrateRoot) => true,
        (_, ModuleLocation::TopLevelPrivateModule) => {
            matches!(parent_visibility, ParentVisibility::Private)
        },
        _ => false,
    }
}

pub(super) const fn forbidden_pub_crate_help(module_location: ModuleLocation) -> &'static str {
    if matches!(
        module_location,
        ModuleLocation::CrateRoot | ModuleLocation::TopLevelPrivateModule
    ) {
        "consider using just `pub` or removing `pub(crate)` entirely"
    } else {
        "consider using `pub(super)` or removing `pub(crate)` entirely"
    }
}

pub(super) fn suspicious_pub_note(crate_kind: CrateKind, kind_label: &str) -> String {
    match crate_kind {
        CrateKind::Library => {
            format!("{kind_label} is not reachable from the crate's public API")
        },
        CrateKind::Binary => {
            format!("{kind_label} is not used outside its parent module subtree")
        },
    }
}

fn line_display(tcx: TyCtxt<'_>, file_path: &Path, span: Span) -> Result<LineDisplay> {
    let source_map = tcx.sess.source_map();
    let start = source_map.lookup_char_pos(span.lo());
    let end = source_map.lookup_char_pos(span.hi());
    let line = start.line;
    let column = start.col_display + 1;
    let highlight_len = if start.line == end.line {
        (end.col_display.saturating_sub(start.col_display)).max(1)
    } else {
        1
    };
    let text = fs::read_to_string(file_path)
        .with_context(|| format!("failed to read source file {}", file_path.display()))?;
    let source_line = text
        .lines()
        .nth(line.saturating_sub(1))
        .unwrap_or_default()
        .to_string();

    Ok(LineDisplay {
        line,
        column,
        highlight_len,
        source_line,
    })
}

fn visibility_text(tcx: TyCtxt<'_>, vis_span: Span) -> Result<Option<String>> {
    if vis_span.is_dummy() {
        return Ok(None);
    }
    Ok(Some(
        tcx.sess
            .source_map()
            .span_to_snippet(vis_span)
            .map_err(|err| {
                anyhow::anyhow!(
                    "failed to extract visibility snippet for span {vis_span:?}: {err:?}"
                )
            })?
            .trim()
            .to_string(),
    ))
}

fn real_file_path(tcx: TyCtxt<'_>, span: Span) -> Option<PathBuf> {
    let source_map = tcx.sess.source_map();
    let file = source_map.lookup_char_pos(span.lo()).file;
    real_file_path_from_name(file.name.clone())
}

fn real_file_path_from_name(name: FileName) -> Option<PathBuf> {
    match name {
        FileName::Real(real) => real.local_path().map(Path::to_path_buf),
        _ => None,
    }
}

fn highlight_span(vis_span: Span, ident_span: Option<Span>) -> Span {
    ident_span.map_or(vis_span, |ident_span| vis_span.to(ident_span))
}

const fn item_kind_label(kind: ItemKind<'_>) -> Option<&'static str> {
    match kind {
        ItemKind::Const(..) => Some("const"),
        ItemKind::Enum(..) => Some("enum"),
        ItemKind::Fn { .. } => Some("fn"),
        ItemKind::Static(..) => Some("static"),
        ItemKind::Struct(..) => Some("struct"),
        ItemKind::Trait(..) | ItemKind::TraitAlias(..) => Some("trait"),
        ItemKind::TyAlias(..) => Some("type"),
        ItemKind::Union(..) => Some("union"),
        ItemKind::Mod(..) => Some("mod"),
        ItemKind::Use(..)
        | ItemKind::ExternCrate(..)
        | ItemKind::ForeignMod { .. }
        | ItemKind::GlobalAsm { .. }
        | ItemKind::Impl(..)
        | ItemKind::Macro(..) => None,
    }
}

const fn impl_item_kind_label(kind: ImplItemKind<'_>) -> &'static str {
    match kind {
        ImplItemKind::Const(..) => "const",
        ImplItemKind::Fn(..) => "fn",
        ImplItemKind::Type(..) => "type",
    }
}

const fn foreign_item_kind_label(kind: ForeignItemKind<'_>) -> &'static str {
    match kind {
        ForeignItemKind::Fn(..) => "fn",
        ForeignItemKind::Static(..) => "static",
        ForeignItemKind::Type => "type",
    }
}

/// Extract the self type name for an impl item via the compiler.
///
/// Given an impl item's `LocalDefId`, walks up to the parent impl block
/// and returns the last path segment of the self type (e.g., `"MyStruct"`).
fn impl_self_type_name_from_tcx(tcx: TyCtxt<'_>, impl_item_def: LocalDefId) -> Option<String> {
    let hir_id = tcx.local_def_id_to_hir_id(impl_item_def);
    let parent_id = tcx.hir_get_parent_item(hir_id);
    let parent_node = tcx.hir_node_by_def_id(parent_id.def_id);
    let rustc_hir::Node::Item(parent_item) = parent_node else {
        return None;
    };
    let ItemKind::Impl(impl_block) = parent_item.kind else {
        return None;
    };
    let rustc_hir::TyKind::Path(rustc_hir::QPath::Resolved(_, path)) = impl_block.self_ty.kind
    else {
        return None;
    };
    path.segments.last().map(|seg| seg.ident.to_string())
}

/// True when `file` is part of a top-level module — either `src/foo.rs` or
/// `src/foo/mod.rs` — but NOT the root module itself (lib.rs / main.rs).
fn is_top_level_module_file(src_root: &Path, root_module: &Path, file: &Path) -> bool {
    if file == root_module {
        return false;
    }
    let Ok(relative) = file.strip_prefix(src_root) else {
        return false;
    };
    let count = relative.components().count();
    // src/foo.rs → 1 component
    if count == 1 {
        return true;
    }
    // src/foo/mod.rs → 2 components, last is "mod.rs"
    count == 2 && relative.file_name().and_then(|name| name.to_str()) == Some("mod.rs")
}

fn is_boundary_file(src_root: &Path, root_module: &Path, file: &Path) -> bool {
    let is_root_file = file == root_module;
    let is_mod_rs = file.file_name().and_then(|name| name.to_str()) == Some("mod.rs");
    let is_top_level_file = file
        .strip_prefix(src_root)
        .ok()
        .is_some_and(|path| path.components().count() == 1);
    is_root_file || is_mod_rs || is_top_level_file
}
