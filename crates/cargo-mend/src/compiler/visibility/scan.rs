use std::ffi::OsStr;
use std::fs;
use std::path::Path;

use anyhow::Context;
use anyhow::Result;
use rustc_hir::ForeignItem;
use rustc_hir::ImplItem;
use rustc_hir::Item;
use rustc_hir::ItemKind;
use rustc_middle::middle::privacy::EffectiveVisibilities;
use rustc_middle::middle::privacy::Level;
use rustc_middle::ty::TyCtxt;
use rustc_span::Span;
use rustc_span::def_id::CRATE_DEF_ID;
use rustc_span::def_id::LocalDefId;

use super::field_visibility;
use super::policy;
use super::source;
use super::use_sites;
use crate::compiler::facade;
use crate::compiler::facade::ParentFacadeExportStatus;
use crate::compiler::facade::ParentFacadeVisibility;
use crate::compiler::persistence;
use crate::compiler::persistence::CacheBuildKind;
use crate::compiler::persistence::FindingsSink;
use crate::compiler::persistence::StoredPubUseFixFact;
use crate::compiler::persistence::StoredReport;
use crate::compiler::settings;
use crate::compiler::settings::DriverSettings;
use crate::compiler::source_cache;
use crate::compiler::source_cache::SourceCache;
use crate::config::DiagnosticCode;
use crate::constants::FINDINGS_SCHEMA_VERSION;
use crate::constants::PUB_CRATE_VISIBILITY;
use crate::constants::PUB_IN_CRATE_VISIBILITY_PREFIX;
use crate::constants::PUB_VISIBILITY_TOKEN;
use crate::diagnostics::CompilerWarningFacts;
use crate::diagnostics::Severity;
use crate::fix_support::FixSupport;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CrateKind {
    Binary,
    Library,
    IntegrationTest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ModuleLocation {
    CrateRoot,
    ShallowPrivate,
    Nested,
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

pub(super) struct VisibilityContext<'a, 'tcx> {
    pub(super) tcx:                    TyCtxt<'tcx>,
    pub(super) settings:               &'a DriverSettings,
    pub(super) source_root:            &'a Path,
    pub(super) root_module:            &'a Path,
    pub(super) effective_visibilities: &'a EffectiveVisibilities,
    pub(super) source_cache:           &'a SourceCache,
}

struct ItemInfo<'a> {
    def_id:         LocalDefId,
    file_path:      &'a Path,
    vis_text:       &'a str,
    kind_label:     Option<&'static str>,
    name:           Option<&'a str>,
    highlight_span: Span,
    category:       ItemCategory,
    impl_self_name: Option<String>,
}

pub(super) struct SuspiciousPubInput<'a> {
    pub(super) def_id:            LocalDefId,
    pub(super) file_path:         &'a Path,
    pub(super) config_rel_path:   Option<&'a str>,
    pub(super) parent_visibility: ParentVisibility,
    pub(super) module_location:   ModuleLocation,
    pub(super) crate_kind:        CrateKind,
    pub(super) kind_label:        Option<&'static str>,
    pub(super) name:              Option<&'a str>,
    pub(super) highlight_span:    Span,
}

pub(super) struct FindingParams {
    pub(super) severity:                Severity,
    pub(super) code:                    DiagnosticCode,
    pub(super) item:                    Option<String>,
    pub(super) message:                 String,
    pub(super) suggestion:              Option<String>,
    pub(super) fixability:              FixSupport,
    pub(super) related:                 Option<String>,
    pub(super) item_def_path:           Option<String>,
    pub(super) narrower_scope_def_path: Option<String>,
}

struct VisibilityFindingContext {
    crate_kind:        CrateKind,
    config_rel_path:   Option<String>,
    module_location:   ModuleLocation,
    parent_visibility: ParentVisibility,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AllowanceReason {
    Allowlist,
    ParentIsPublic,
    ShallowPrivatePolicy,
    ReachablePublicApi,
    ParentFacadeUsedOutsideParent,
    InternalParentFacadeBoundary,
    ExposedByOtherCrateVisibleSignature,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SuspiciousPubAssessment {
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
    let crate_root_file = source::real_file_path(tcx, tcx.def_span(CRATE_DEF_ID))
        .context("failed to determine local crate root file")?;
    let Some(source_root) =
        source_cache::analysis_source_root_for(&crate_root_file, &settings.package_root)
    else {
        return Ok(false);
    };

    let mut sink = FindingsSink::default();
    let crate_items = tcx.hir_crate_items(());
    let cache_roots: Vec<&Path> = if settings.config_root == settings.package_root {
        vec![&source_root]
    } else {
        vec![&source_root, &settings.config_root]
    };
    let source_cache = SourceCache::build(&cache_roots)?;
    let ctx = VisibilityContext {
        tcx,
        settings,
        source_root: &source_root,
        root_module: &crate_root_file,
        effective_visibilities: tcx.effective_visibilities(()),
        source_cache: &source_cache,
    };

    for item_id in crate_items.free_items() {
        let item = tcx.hir_item(item_id);
        analyze_item(&ctx, item, &mut sink)?;
        field_visibility::check_item(&ctx, item, &mut sink)?;
    }

    for item_id in crate_items.impl_items() {
        analyze_impl_item(&ctx, tcx.hir_impl_item(item_id), &mut sink)?;
    }

    for item_id in crate_items.foreign_items() {
        analyze_foreign_item(&ctx, tcx.hir_foreign_item(item_id), &mut sink)?;
    }

    use_sites::collect_use_sites(tcx, &mut sink.use_sites);

    let build_kind = if tcx.sess.opts.test {
        CacheBuildKind::Test
    } else {
        CacheBuildKind::Library
    };
    let output_path = settings.findings_dir.join(persistence::cache_filename_for(
        &settings.package_root,
        &crate_root_file,
        build_kind,
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
        use_sites:            sink.use_sites,
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
    let Some(file_path) = source::real_file_path(ctx.tcx, item.vis_span) else {
        return Ok(());
    };
    let Some(vis_text) = source::visibility_text(ctx.tcx, item.vis_span)? else {
        return Ok(());
    };

    let name = item.kind.ident().as_ref().map(ToString::to_string);

    if vis_text == PUB_VISIBILITY_TOKEN
        && policy::is_boundary_file(ctx.source_root, ctx.root_module, &file_path)
        && matches!(item.kind, ItemKind::Use(..))
        && source::use_item_contains_glob(ctx.tcx, item.span)?
    {
        sink.findings.push(source::build_finding(
            ctx.tcx,
            &file_path,
            item.span,
            FindingParams {
                severity:                Severity::Warning,
                code:                    DiagnosticCode::WildcardParentPubUse,
                item:                    None,
                message:                 String::new(),
                suggestion:              None,
                fixability:              FixSupport::None,
                related:                 None,
                item_def_path:           None,
                narrower_scope_def_path: None,
            },
        )?);
    }

    record_visibility_findings(
        ctx,
        &ItemInfo {
            def_id:         item.owner_id.def_id,
            file_path:      &file_path,
            vis_text:       &vis_text,
            kind_label:     source::item_kind_label(item.kind),
            name:           name.as_deref(),
            highlight_span: source::highlight_span(
                item.vis_span,
                item.kind.ident().map(|ident| ident.span),
            ),
            category:       if matches!(item.kind, ItemKind::Mod(..)) {
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
    let Some(file_path) = source::real_file_path(ctx.tcx, vis_span) else {
        return Ok(());
    };
    let Some(vis_text) = source::visibility_text(ctx.tcx, vis_span)? else {
        return Ok(());
    };

    let name = item.ident.to_string();
    let impl_self_name = source::impl_self_type_name_from_tcx(ctx.tcx, item.owner_id.def_id);

    record_visibility_findings(
        ctx,
        &ItemInfo {
            def_id: item.owner_id.def_id,
            file_path: &file_path,
            vis_text: &vis_text,
            kind_label: Some(source::impl_item_kind_label(item.kind)),
            name: Some(name.as_str()),
            highlight_span: source::highlight_span(vis_span, Some(item.ident.span)),
            category: ItemCategory::NonModule,
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
    let Some(file_path) = source::real_file_path(ctx.tcx, item.vis_span) else {
        return Ok(());
    };
    let Some(vis_text) = source::visibility_text(ctx.tcx, item.vis_span)? else {
        return Ok(());
    };

    let name = item.ident.to_string();

    record_visibility_findings(
        ctx,
        &ItemInfo {
            def_id:         item.owner_id.def_id,
            file_path:      &file_path,
            vis_text:       &vis_text,
            kind_label:     Some(source::foreign_item_kind_label(item.kind)),
            name:           Some(name.as_str()),
            highlight_span: source::highlight_span(item.vis_span, Some(item.ident.span)),
            category:       ItemCategory::NonModule,
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

    record_forbidden_pub_crate(ctx, item, &finding_context, sink)?;
    record_forbidden_pub_in_crate(ctx, item, sink)?;
    record_review_pub_mod(ctx, item, &finding_context, sink)?;

    if item.vis_text == PUB_VISIBILITY_TOKEN
        && finding_context.parent_visibility == ParentVisibility::Private
        && policy::is_top_level_module_file(ctx.source_root, ctx.root_module, item.file_path)
        && policy::allow_pub_crate_by_policy(
            finding_context.crate_kind,
            finding_context.module_location,
            finding_context.parent_visibility,
        )
    {
        maybe_record_narrow_to_pub_crate(ctx, item, sink)?;
    }

    if item.vis_text == PUB_VISIBILITY_TOKEN
        && finding_context.parent_visibility == ParentVisibility::Private
        && !policy::is_top_level_module_file(ctx.source_root, ctx.root_module, item.file_path)
        && finding_context.crate_kind != CrateKind::IntegrationTest
    {
        maybe_record_narrow_to_pub_crate_nested(ctx, item, sink)?;
    }

    if item.vis_text == PUB_VISIBILITY_TOKEN
        && !policy::is_boundary_file(ctx.source_root, ctx.root_module, item.file_path)
    {
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
                name:              item.name,
                highlight_span:    item.highlight_span,
            },
            sink,
        )?;
    }
    Ok(())
}

fn record_forbidden_pub_crate(
    ctx: &VisibilityContext<'_, '_>,
    item: &ItemInfo<'_>,
    finding_context: &VisibilityFindingContext,
    sink: &mut FindingsSink,
) -> Result<()> {
    if !matches!(item.vis_text, PUB_CRATE_VISIBILITY) {
        return Ok(());
    }
    if policy::allow_pub_crate_by_policy(
        finding_context.crate_kind,
        finding_context.module_location,
        finding_context.parent_visibility,
    ) {
        return Ok(());
    }
    if parent_facade_caps_at_pub_crate(ctx, item)? {
        return Ok(());
    }
    sink.findings.push(source::build_finding(
        ctx.tcx,
        item.file_path,
        item.highlight_span,
        FindingParams {
            severity:                Severity::Error,
            code:                    DiagnosticCode::ForbiddenPubCrate,
            item:                    None,
            message:                 "use of `pub(crate)` is forbidden by policy".to_string(),
            suggestion:              Some(
                policy::forbidden_pub_crate_help(finding_context.module_location).to_string(),
            ),
            fixability:              FixSupport::None,
            related:                 None,
            item_def_path:           None,
            narrower_scope_def_path: None,
        },
    )?);
    Ok(())
}

fn record_forbidden_pub_in_crate(
    ctx: &VisibilityContext<'_, '_>,
    item: &ItemInfo<'_>,
    sink: &mut FindingsSink,
) -> Result<()> {
    if !item.vis_text.starts_with(PUB_IN_CRATE_VISIBILITY_PREFIX) {
        return Ok(());
    }
    sink.findings.push(source::build_finding(
        ctx.tcx,
        item.file_path,
        item.highlight_span,
        FindingParams {
            severity:                Severity::Error,
            code:                    DiagnosticCode::ForbiddenPubInCrate,
            item:                    None,
            message:                 "use of `pub(in crate::...)` is forbidden by policy"
                .to_string(),
            suggestion:              None,
            fixability:              FixSupport::None,
            related:                 None,
            item_def_path:           None,
            narrower_scope_def_path: None,
        },
    )?);
    Ok(())
}

fn record_review_pub_mod(
    ctx: &VisibilityContext<'_, '_>,
    item: &ItemInfo<'_>,
    finding_context: &VisibilityFindingContext,
    sink: &mut FindingsSink,
) -> Result<()> {
    if item.category != ItemCategory::Module || !item.vis_text.starts_with(PUB_VISIBILITY_TOKEN) {
        return Ok(());
    }
    let allowlisted = finding_context
        .config_rel_path
        .as_ref()
        .is_some_and(|path| {
            ctx.settings
                .visibility_config
                .allow_pub_mod
                .iter()
                .any(|allowed| allowed == path)
        });
    if allowlisted {
        return Ok(());
    }
    sink.findings.push(source::build_finding(
        ctx.tcx,
        item.file_path,
        item.highlight_span,
        FindingParams {
            severity:                Severity::Error,
            code:                    DiagnosticCode::ReviewPubMod,
            item:                    item.name.map(str::to_owned),
            message:                 "`pub mod` requires explicit review or allowlisting"
                .to_string(),
            suggestion:              None,
            fixability:              FixSupport::None,
            related:                 None,
            item_def_path:           None,
            narrower_scope_def_path: None,
        },
    )?);
    Ok(())
}

fn visibility_finding_context(
    ctx: &VisibilityContext<'_, '_>,
    item: &ItemInfo<'_>,
) -> VisibilityFindingContext {
    let crate_kind = policy::crate_kind_for_root(ctx.root_module, &ctx.settings.package_root);
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
    let module_location = policy::resolve_module_location(ctx.tcx, parent_module.to_local_def_id());

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
    let (Some(name), Some(kind_label)) = (item.name, item.kind_label) else {
        return Ok(());
    };
    if ctx
        .effective_visibilities
        .is_public_at_level(item.def_id, Level::Reachable)
    {
        return Ok(());
    }
    if facade::root_module_exports_item(ctx.source_cache, ctx.root_module, item.file_path, name) {
        return Ok(());
    }
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
    sink.findings.push(source::build_finding(
        ctx.tcx,
        item.file_path,
        item.highlight_span,
        FindingParams {
            severity:                Severity::Warning,
            code:                    DiagnosticCode::NarrowToPubCrate,
            item:                    Some(format!("{kind_label} {name}")),
            message:                 String::from(
                "item is not re-exported by the crate root — use `pub(crate)`",
            ),
            suggestion:              Some(String::from("consider using: `pub(crate)`")),
            fixability:              FixSupport::NarrowToPubCrate,
            related:                 None,
            item_def_path:           None,
            narrower_scope_def_path: None,
        },
    )?);
    Ok(())
}

fn maybe_record_narrow_to_pub_crate_nested(
    ctx: &VisibilityContext<'_, '_>,
    item: &ItemInfo<'_>,
    sink: &mut FindingsSink,
) -> Result<()> {
    let (Some(name), Some(kind_label)) = (item.name, item.kind_label) else {
        return Ok(());
    };
    if !parent_facade_caps_at_pub_crate(ctx, item)? {
        return Ok(());
    }
    sink.findings.push(source::build_finding(
        ctx.tcx,
        item.file_path,
        item.highlight_span,
        FindingParams {
            severity:                Severity::Warning,
            code:                    DiagnosticCode::NarrowToPubCrate,
            item:                    Some(format!("{kind_label} {name}")),
            message:                 String::from(
                "parent facade caps reach at `pub(crate)` — narrow source to match",
            ),
            suggestion:              Some(String::from("consider using: `pub(crate)`")),
            fixability:              FixSupport::NarrowToPubCrate,
            related:                 None,
            item_def_path:           None,
            narrower_scope_def_path: None,
        },
    )?);
    Ok(())
}

fn parent_facade_caps_at_pub_crate(
    ctx: &VisibilityContext<'_, '_>,
    item: &ItemInfo<'_>,
) -> Result<bool> {
    let Some(name) = item.name else {
        return Ok(false);
    };
    let status = facade::parent_facade_export_status(
        ctx.source_cache,
        ctx.settings,
        ctx.source_root,
        item.file_path,
        name,
    )?;
    Ok(matches!(
        status.as_ref().map(|s| s.visibility),
        Some(ParentFacadeVisibility::Crate)
    ))
}

fn maybe_record_suspicious_pub(
    ctx: &VisibilityContext<'_, '_>,
    input: &SuspiciousPubInput<'_>,
    sink: &mut FindingsSink,
) -> Result<()> {
    let Some(kind_label) = input.kind_label else {
        return Ok(());
    };

    match policy::classify_suspicious_pub(ctx, input)? {
        SuspiciousPubAssessment::Allowed(_) => {},
        SuspiciousPubAssessment::ReviewInternalParentFacade { related } => {
            let Some(status) = input
                .name
                .map(|name| {
                    facade::parent_facade_export_status(
                        ctx.source_cache,
                        ctx.settings,
                        ctx.source_root,
                        input.file_path,
                        name,
                    )
                })
                .transpose()?
                .flatten()
            else {
                return Ok(());
            };
            sink.findings.push(source::build_line_finding(
                ctx.source_cache,
                &status.parent_path,
                status.parent_line,
                FindingParams {
                    severity: Severity::Warning,
                    code: DiagnosticCode::InternalParentPubUseFacade,
                    item: input.name.map(|name| format!("pub use {name}")),
                    message: String::from(
                        "this `pub use` is used inside its parent module subtree",
                    ),
                    suggestion: None,
                    fixability: FixSupport::InternalParentFacade,
                    related,
                    item_def_path: None,
                    narrower_scope_def_path: None,
                },
            )?);
        },
        SuspiciousPubAssessment::Warn {
            fixability,
            related,
            stale_parent_pub_use,
        } => {
            // For suspicious_pub, expose the item's canonical def-path and
            // the parent module's def-path. Cross-compilation merge in
            // load_report uses these to suppress the finding when any
            // caller (across all compilations) lives outside the proposed
            // narrower scope.
            let item_def_path = Some(use_sites::def_path_string(ctx.tcx, input.def_id));
            let narrower_scope_def_path =
                Some(use_sites::parent_module_def_path(ctx.tcx, input.def_id));
            sink.findings.push(source::build_finding(
                ctx.tcx,
                input.file_path,
                input.highlight_span,
                FindingParams {
                    severity: Severity::Warning,
                    code: DiagnosticCode::SuspiciousPub,
                    item: input.name.map(|name| format!("{kind_label} {name}")),
                    message: policy::suspicious_pub_note(input.crate_kind, kind_label),
                    suggestion: None,
                    fixability,
                    related,
                    item_def_path,
                    narrower_scope_def_path,
                },
            )?);
            if let (Some(status), Some(item_name)) = (stale_parent_pub_use, input.name)
                && fixability == FixSupport::PubUse
            {
                let child_line = ctx
                    .tcx
                    .sess
                    .source_map()
                    .lookup_char_pos(input.highlight_span.lo())
                    .line;
                let Some(child_module) = input
                    .file_path
                    .file_stem()
                    .and_then(OsStr::to_str)
                    .filter(|stem| *stem != "mod")
                    .map(String::from)
                else {
                    return Ok(());
                };
                sink.pub_use_fix_facts.push(StoredPubUseFixFact {
                    child_path: input.file_path.to_string_lossy().into_owned(),
                    child_line,
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
