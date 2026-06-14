use super::AbsolutePath;
use super::App;
use super::CRATES_IO_UNREACHABLE;
use super::Cargo;
use super::CiStatus;
use super::EmptyDescriptionBehavior;
use super::NonRustProject;
use super::Package;
use super::PackageRecord;
use super::Path;
use super::ProjectList;
use super::ProjectType;
use super::Rect;
use super::RootItem;
use super::RustInfo;
use super::RustProject;
use super::ServiceStatus;
use super::Submodule;
use super::VendoredPackage;
use super::Visibility;
use super::Workspace;
use super::git;
use super::git_data;
use super::git_data::GitData;
use super::git_data::GitDetailFields;
use super::git_data::WorktreeInfo;
use super::package;
use super::package_data;
use super::package_data::PackageData;
use super::package_data::PackagePresence;
use super::package_data::PackageSection;
use super::package_data::PublishStatus;
use super::package_data::StructureCounts;
use super::package_data::WorktreeGroupSummary;
use super::project;
use super::targets::TargetsData;
use crate::project::ProjectFields;
use crate::tui::panes::DescriptionBlock;
use crate::tui::state::CiDisplay;
use crate::tui::state::Lint;
use crate::tui::state::LintDisplay;

pub struct DetailPaneData {
    pub package: PackageData,
    pub git:     GitData,
    pub targets: TargetsData,
}

/// Resolve the title shown in the `Package` column header.
fn resolve_package_title(app: &App, item: &RootItem) -> String {
    if !item.is_rust() {
        return "Project".to_string();
    }
    if app.project_list.is_vendored_path(item.path()) {
        return "Vendored Crate".to_string();
    }
    if let RootItem::Worktrees(group) = item {
        if group.renders_as_group() {
            return "Worktree Group".to_string();
        }
        return match &group.primary {
            RustProject::Workspace(_) => "Workspace".to_string(),
            RustProject::Package(pkg) => resolve_package_title_for_package(app, pkg),
        };
    }
    if matches!(item, RootItem::Rust(RustProject::Workspace(_))) {
        return "Workspace".to_string();
    }
    if app.project_list.is_workspace_member_path(item.path()) {
        "Workspace Member".to_string()
    } else {
        "Package".to_string()
    }
}

/// Resolve the package title for a non-root package (member or vendored).
fn resolve_package_title_for_package(app: &App, pkg: &Package) -> String {
    if app.project_list.is_vendored_path(pkg.path()) {
        "Vendored Crate".to_string()
    } else if app.project_list.is_workspace_member_path(pkg.path()) {
        "Workspace Member".to_string()
    } else {
        "Package".to_string()
    }
}

/// Format a download count with comma-separated thousands (e.g. `1,234,567`).
fn format_downloads(count: u64) -> String {
    let digits = count.to_string();
    let mut result = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            result.push(',');
        }
        result.push(ch);
    }
    result
}

/// Inner height (excluding the pane border) the Details and Git top row must
/// reserve to show every project's content without clipping, measured across
/// all projects. Sizing to the tallest project keeps the row height stable as
/// the selection moves. Shorter projects render with blank space below their
/// content; only the tallest fills the row exactly.
///
/// `package_width` / `git_width` are the outer widths of the two top-row panes
/// (descriptions wrap to those widths), so the result is valid only for the
/// layout that produced them. The caller caches this against the scan
/// generation and the widths, since it rebuilds every project's pane data.
pub fn max_top_pane_inner_height(app: &App, package_width: u16, git_width: u16) -> u16 {
    app.project_list
        .iter()
        .map(|entry| project_top_inner_height(app, &entry.root_item, package_width, git_width))
        .max()
        .unwrap_or(0)
}

/// Inner height the Details and Git panes need to render `item`'s content
/// without clipping: the taller of the two panes. Each pane is its
/// About-section description (wrapped to the pane width and raised to the
/// shared sync height) plus the separator plus its lower content block.
fn project_top_inner_height(app: &App, item: &RootItem, package_width: u16, git_width: u16) -> u16 {
    let data = build_pane_data(app, item);

    let package_lower = package::package_lower_metadata_height(
        &data.package,
        &package_data::package_rows_from_data(&data.package),
    );
    let git_lower =
        git::git_lower_content_height(&data.git, git_data::git_fields_from_data(&data.git).len());

    let package_desc = description_natural_rows(
        data.package.description.as_deref(),
        package_width,
        EmptyDescriptionBehavior::ShowPlaceholder,
    );
    let git_desc = description_natural_rows(
        data.git.description.as_deref(),
        git_width,
        EmptyDescriptionBehavior::RenderEmpty,
    );
    // Both panes sync their description blocks to the taller of the two, so a
    // pane with the shorter description still reserves the shared height.
    let synced_desc = package_desc.max(git_desc);

    // Both panes compute their rendered height via the same helpers the
    // render path feeds to each viewport, so the measured row height and the
    // per-pane scroll pager agree on what renders. The Package About section
    // always renders (a placeholder when the crate has no description); the Git
    // About section renders only when the repo has a description of its own.
    let package_total = package::package_content_height(synced_desc, package_lower);
    let git_total = git::git_content_height(synced_desc, git_desc > 0, git_lower);

    u16::try_from(package_total.max(git_total)).unwrap_or(u16::MAX)
}

/// Natural (uncapped) wrapped row count for an About-section description at
/// `pane_width`. Builds the same `DescriptionBlock` the render path builds, so
/// the measured height matches what renders; a tall synthetic area lifts the
/// height cap so the full wrapped text is counted.
fn description_natural_rows(
    text: Option<&str>,
    pane_width: u16,
    behavior: super::EmptyDescriptionBehavior,
) -> usize {
    let area = Rect {
        x:      0,
        y:      0,
        width:  pane_width,
        height: u16::MAX,
    };
    usize::from(DescriptionBlock::for_pane(text, area, behavior).natural_sync_height())
}

/// Build pane data for a root `RootItem`.
pub fn build_pane_data(app: &App, item: &RootItem) -> DetailPaneData {
    let display_path = item.display_path().into_string();
    let is_wt_group = git_data::is_worktree_group(item);

    match item {
        RootItem::Rust(RustProject::Workspace(ws)) => {
            build_pane_data_for_workspace(app, ws, &display_path, is_wt_group, Some(item))
        },
        RootItem::Rust(RustProject::Package(pkg)) => {
            build_pane_data_for_package(app, pkg, &display_path, is_wt_group, Some(item))
        },
        RootItem::NonRust(nr) => {
            build_pane_data_non_rust(app, nr, &display_path, is_wt_group, Some(item))
        },
        RootItem::Worktrees(group) => match &group.primary {
            RustProject::Workspace(ws) => {
                build_pane_data_for_workspace(app, ws, &display_path, is_wt_group, Some(item))
            },
            RustProject::Package(pkg) => {
                build_pane_data_for_package(app, pkg, &display_path, is_wt_group, Some(item))
            },
        },
    }
}

/// Build pane data for a workspace member.
pub fn build_pane_data_for_member(app: &App, pkg: &Package) -> DetailPaneData {
    let display_path = pkg.display_path().into_string();
    build_pane_data_for_package(app, pkg, &display_path, false, None)
}

/// Build pane data for a vendored crate row.
pub fn build_pane_data_for_vendored(app: &App, vendored: &VendoredPackage) -> DetailPaneData {
    let display_path = vendored.display_path().into_string();
    let abs_path = vendored.path();
    let cargo = &vendored.cargo;

    let mut counts = StructureCounts::default();
    counts.add_cargo(cargo);
    let stats_rows = counts.to_rows();
    let test_rows =
        package_data::test_rows_from_counts(vendored.info().test_counts.unwrap_or_default());

    build_pane_data_common(
        app,
        PaneDataSource {
            abs_path,
            display_path: &display_path,
            title_name: vendored.package_name().into_string(),
            package_presence: PackagePresence::Present,
            cargo: Some(cargo),
            wt_item: None,
            stats_rows,
            test_rows,
            primary_section: None,
            fallback_type: None,
            package_title: "Vendored Crate".to_string(),
        },
    )
}

/// Build pane data for a linked `Project<Workspace>` worktree entry.
pub fn build_pane_data_for_workspace_ref(
    app: &App,
    ws: &Workspace,
    display_path: &str,
) -> DetailPaneData {
    build_pane_data_for_workspace(app, ws, display_path, false, None)
}

/// Build pane data for a git submodule nested under a project.
pub fn build_pane_data_for_submodule(app: &App, submodule: &Submodule) -> DetailPaneData {
    let abs_path = &submodule.path;
    let display_path = project::home_relative_path(abs_path);
    let git_detail = git_data::build_git_detail_fields(app, abs_path);

    let submodule_ctx = git_data::build_submodule_context(submodule);

    DetailPaneData {
        package: PackageData {
            package_title:            "Submodule".to_string(),
            title_name:               submodule.name.clone(),
            worktree_group_summary:   None,
            primary_section:          None,
            path:                     display_path,
            version:                  submodule.commit.clone(),
            description:              submodule.url.clone(),
            crates_io_rows:           Vec::new(),
            types:                    None,
            disk:                     submodule.project_info.disk_usage_bytes,
            stats_rows:               Vec::new(),
            test_rows:                Vec::new(),
            package_presence:         PackagePresence::Missing,
            edition:                  None,
            license:                  None,
            homepage:                 None,
            repository:               None,
            in_project_target:        None,
            in_project_non_target:    None,
            out_of_tree_target_bytes: None,
            // Submodules don't render the Lint/Ci fields; the
            // `package_fields_from_data` filter excludes them when
            // there's no Cargo manifest. Default values are safe.
            lint_display:             LintDisplay::default(),
            ci_display:               CiDisplay::default(),
        },
        git:     GitData {
            head: git_detail.head,
            head_relation: git_detail.head_relation,
            bisect: git_detail.bisect,
            status: git_detail.path,
            vs_local: git_detail.vs_local,
            stars: git_detail.stars,
            description: git_detail.description,
            inception: git_detail.inception,
            last_commit: git_detail.last_commit,
            last_fetched: git_detail.last_fetched,
            rate_limit_core: git_detail.rate_limit_core,
            rate_limit_graphql: git_detail.rate_limit_graphql,
            github_status: git_detail.github_status,
            pull_requests: git_detail.pull_requests,
            remotes: git_detail.remotes,
            worktrees: Vec::new(),
            submodule_ctx,
        },
        targets: TargetsData::default(),
    }
}

fn build_pane_data_for_workspace(
    app: &App,
    ws: &Workspace,
    display_path: &str,
    is_wt_group: bool,
    wt_item: Option<&RootItem>,
) -> DetailPaneData {
    let abs_path = ws.path();
    let cargo = &ws.cargo;

    let mut counts = StructureCounts::default();
    let mut test_counts = ws.info().test_counts.unwrap_or_default();
    counts.add_workspace(ws);
    if ws.has_members() {
        for group in ws.groups() {
            for member in group.members() {
                counts.add_package(member);
                test_counts = test_counts.merged(member.info().test_counts.unwrap_or_default());
            }
        }
    }
    let stats_rows = counts.to_rows();
    let test_rows = package_data::test_rows_from_counts(test_counts);

    let wt_item_ref = wt_item.filter(|_| is_wt_group);
    let package_title = wt_item_ref.map_or_else(
        || "Workspace".to_string(),
        |item| resolve_package_title(app, item),
    );
    build_pane_data_common(
        app,
        PaneDataSource {
            abs_path,
            display_path,
            title_name: ws.package_name().into_string(),
            package_presence: PackagePresence::from_has_package(ws.name().is_some()),
            cargo: Some(cargo),
            wt_item: wt_item_ref,
            stats_rows,
            test_rows,
            primary_section: Some(PackageSection::PrimaryWorkspace).filter(|_| is_wt_group),
            fallback_type: Some(ProjectType::Workspace),
            package_title,
        },
    )
}

fn build_pane_data_for_package(
    app: &App,
    pkg: &Package,
    display_path: &str,
    is_wt_group: bool,
    wt_item: Option<&RootItem>,
) -> DetailPaneData {
    let abs_path = pkg.path();
    let cargo = &pkg.cargo;

    let mut counts = StructureCounts::default();
    counts.add_package(pkg);
    let stats_rows = counts.to_rows();
    let test_rows = package_data::test_rows_from_counts(pkg.info().test_counts.unwrap_or_default());

    let wt_item_ref = wt_item.filter(|_| is_wt_group);
    let package_title = wt_item.map_or_else(
        || resolve_package_title_for_package(app, pkg),
        |item| resolve_package_title(app, item),
    );

    build_pane_data_common(
        app,
        PaneDataSource {
            abs_path,
            display_path,
            title_name: pkg.package_name().into_string(),
            package_presence: PackagePresence::Present,
            cargo: Some(cargo),
            wt_item: wt_item_ref,
            stats_rows,
            test_rows,
            primary_section: Some(PackageSection::PrimaryPackage).filter(|_| is_wt_group),
            fallback_type: None,
            package_title,
        },
    )
}

fn build_pane_data_non_rust(
    app: &App,
    nr: &NonRustProject,
    display_path: &str,
    is_wt_group: bool,
    wt_item: Option<&RootItem>,
) -> DetailPaneData {
    let abs_path = nr.path();
    let wt_item_ref = wt_item.filter(|_| is_wt_group);
    let mut counts = StructureCounts::default();
    counts.add_non_rust(nr);

    build_pane_data_common(
        app,
        PaneDataSource {
            abs_path,
            display_path,
            title_name: nr.root_directory_name().into_string(),
            package_presence: PackagePresence::Missing,
            cargo: None,
            wt_item: wt_item_ref,
            stats_rows: counts.to_rows(),
            test_rows: Vec::new(),
            primary_section: None,
            fallback_type: None,
            package_title: "Project".to_string(),
        },
    )
}

pub(super) struct PaneDataSource<'a> {
    abs_path:         &'a Path,
    display_path:     &'a str,
    title_name:       String,
    package_presence: PackagePresence,
    cargo:            Option<&'a Cargo>,
    wt_item:          Option<&'a RootItem>,
    stats_rows:       Vec<(&'static str, usize)>,
    test_rows:        Vec<(&'static str, usize)>,
    primary_section:  Option<PackageSection>,
    fallback_type:    Option<ProjectType>,
    package_title:    String,
}

/// Crates-io fields pulled from either a Rust info or vendored entry.
pub(super) struct CratesIoFields {
    version:    Option<String>,
    prerelease: Option<String>,
    downloads:  Option<u64>,
    /// Whether this project would have fired a crates.io fetch — i.e.
    /// whether it is a publishable package. Used to keep the crates.io section's
    /// placeholder row visible during a crates.io outage even before any
    /// version landed; non-publishable rows (where no fetch ever runs)
    /// never show the section.
    publish:    PublishStatus,
}

pub(super) fn resolve_crates_io_fields(app: &App, abs_path: &Path) -> CratesIoFields {
    let rust_info = app.project_list.rust_info_at_path(abs_path);
    let vendored = app.project_list.vendored_at_path(abs_path);
    let publish = if rust_info.is_some_and(|r| r.cargo.publishable())
        || vendored.is_some_and(|v| v.cargo.publishable() && v.name.is_some())
    {
        PublishStatus::Publishable
    } else {
        PublishStatus::NotPublishable
    };
    CratesIoFields {
        version: rust_info
            .and_then(|r| r.crates_version().map(String::from))
            .or_else(|| vendored.and_then(|v| v.crates_version().map(String::from))),
        prerelease: rust_info
            .and_then(|r| r.crates_prerelease().map(String::from))
            .or_else(|| vendored.and_then(|v| v.crates_prerelease().map(String::from))),
        downloads: rust_info
            .and_then(RustInfo::crates_downloads)
            .or_else(|| vendored.and_then(VendoredPackage::crates_downloads)),
        publish,
    }
}

fn lookup_package_record(app: &App, abs_path: &AbsolutePath) -> Option<PackageRecord> {
    app.scan
        .metadata_store_handle()
        .lock()
        .ok()
        .and_then(|store| store.package_for_path(abs_path).cloned())
}

/// Manifest-derived fields pulled from the workspace metadata.
pub(super) struct ManifestFields {
    edition:     Option<String>,
    license:     Option<String>,
    homepage:    Option<String>,
    repository:  Option<String>,
    version:     Option<String>,
    description: Option<String>,
}

fn manifest_fields_from(package_record: Option<&PackageRecord>) -> ManifestFields {
    let (version, description) = package_data::version_and_description(package_record);
    ManifestFields {
        edition: package_record.map(|pkg| pkg.edition.clone()),
        license: package_record.and_then(|pkg| pkg.license.clone()),
        homepage: package_record.and_then(|pkg| pkg.homepage.clone()),
        repository: package_record.and_then(|pkg| pkg.repository.clone()),
        version,
        description,
    }
}

/// Args for `build_package_data` — the resolved values
/// `build_pane_data_common` computes once it has `&App` access,
/// handed off to the pure constructor below.
pub(super) struct BuildPackageDataArgs {
    package_title:            String,
    title_name:               String,
    worktree_group_summary:   Option<WorktreeGroupSummary>,
    primary_section:          Option<PackageSection>,
    display_path:             String,
    stats_rows:               Vec<(&'static str, usize)>,
    test_rows:                Vec<(&'static str, usize)>,
    package_presence:         PackagePresence,
    manifest:                 ManifestFields,
    crates_io_rows:           Vec<(&'static str, String)>,
    types:                    Option<Vec<ProjectType>>,
    disk:                     Option<u64>,
    in_project_target:        Option<u64>,
    in_project_non_target:    Option<u64>,
    out_of_tree_target_bytes: Option<u64>,
    lint_display:             LintDisplay,
    ci_display:               CiDisplay,
}

/// Pure constructor: assemble `PackageData` from already-resolved
/// values. Extracted from `build_pane_data_common` so that
/// orchestrator stays under the line limit.
pub(super) fn build_package_data(args: BuildPackageDataArgs) -> PackageData {
    let ManifestFields {
        edition,
        license,
        homepage,
        repository,
        version,
        description,
    } = args.manifest;
    PackageData {
        package_title: args.package_title,
        title_name: args.title_name,
        worktree_group_summary: args.worktree_group_summary,
        primary_section: args.primary_section,
        path: args.display_path,
        version,
        description,
        crates_io_rows: args.crates_io_rows,
        types: args.types,
        disk: args.disk,
        stats_rows: args.stats_rows,
        test_rows: args.test_rows,
        package_presence: args.package_presence,
        edition,
        license,
        homepage,
        repository,
        in_project_target: args.in_project_target,
        in_project_non_target: args.in_project_non_target,
        out_of_tree_target_bytes: args.out_of_tree_target_bytes,
        lint_display: args.lint_display,
        ci_display: args.ci_display,
    }
}

pub(super) fn worktree_group_summary_for(item: &RootItem) -> Option<WorktreeGroupSummary> {
    let RootItem::Worktrees(group) = item else {
        return None;
    };
    Some(WorktreeGroupSummary {
        worktrees: group.visible_entry_count(),
        deleted:   group
            .iter_entries()
            .filter(|entry| entry.visibility() == Visibility::Deleted)
            .count(),
    })
}

fn compute_in_project_bytes(pl: &ProjectList, abs_path: &Path) -> (Option<u64>, Option<u64>) {
    pl.at_path(abs_path).map_or((None, None), |pi| {
        (pi.in_project_target, pi.in_project_non_target)
    })
}

fn compute_ci_status(app: &App, abs_path: &Path, wt_item: Option<&RootItem>) -> Option<CiStatus> {
    let lookup = app.ci.status_lookup();
    wt_item.map_or_else(
        || app.project_list.ci_status_using_lookup(abs_path, &lookup),
        |item| {
            app.project_list
                .ci_status_for_root_item_using_lookup(item, &lookup)
        },
    )
}

fn compute_package_displays(
    app: &App,
    abs_path: &AbsolutePath,
    ci_status: Option<CiStatus>,
    package_title: &str,
) -> (LintDisplay, CiDisplay) {
    let pl = &app.project_list;
    let is_worktree_group = package_title == "Worktree Group";
    let is_rust = pl.is_rust_at_path(abs_path.as_path());
    let lint_display = Lint::package_display(pl, abs_path, is_worktree_group, is_rust);
    let ci_display = app.ci.package_display(
        abs_path,
        pl.repo_info_for(abs_path.as_path()),
        pl.git_info_for(abs_path.as_path()),
        pl.ci_info_for(abs_path.as_path()),
        ci_status,
        is_worktree_group,
    );
    (lint_display, ci_display)
}

/// Orchestrator for assembling a `DetailPaneData` from the inputs
/// every pane builder shares. Reads divide into three phases (runtime
/// state, metadata lookups, derived status), then the package /
/// targets pieces are constructed and the final result is assembled.
fn build_pane_data_common(app: &App, src: PaneDataSource<'_>) -> DetailPaneData {
    let abs_path = src.abs_path;
    let abs_path_owned = AbsolutePath::from(abs_path);

    let runtime = collect_runtime_fields(app, abs_path, src.wt_item);
    let metadata =
        collect_metadata_fields(app, abs_path, &abs_path_owned, src.cargo, src.fallback_type);
    log_pane_common_breakdown(abs_path, &runtime, &metadata);

    let crates_io_status = derive_crates_io_status(&runtime.crates_io, app);
    let (lint_display, ci_display) =
        compute_package_displays(app, &abs_path_owned, runtime.ci, &src.package_title);

    let package = build_package_data(BuildPackageDataArgs {
        package_title: src.package_title,
        title_name: src.title_name,
        display_path: src.display_path.to_owned(),
        stats_rows: src.stats_rows,
        test_rows: src.test_rows,
        worktree_group_summary: runtime.worktree_group_summary,
        primary_section: src.primary_section,
        package_presence: src.package_presence,
        manifest: metadata.manifest,
        crates_io_rows: build_crates_io_rows(&runtime.crates_io, &crates_io_status),
        types: metadata.types,
        disk: runtime.disk,
        in_project_target: metadata.in_project_target,
        in_project_non_target: metadata.in_project_non_target,
        out_of_tree_target_bytes: metadata.out_of_tree_target_bytes,
        lint_display,
        ci_display,
    });

    let targets = lookup_targets_data(app, &abs_path_owned, src.wt_item);
    assemble_detail_pane_data(package, runtime.git_detail, runtime.worktrees, targets)
}

/// Phase 1 output — every field that depends on runtime state
/// (filesystem walks, network probes, in-memory caches). Captures
/// per-piece millisecond timings so the orchestrator can emit a
/// single combined log line without each helper logging
/// independently.
pub(super) struct RuntimeFields {
    git_detail:             GitDetailFields,
    crates_io:              CratesIoFields,
    disk:                   Option<u64>,
    worktree_group_summary: Option<WorktreeGroupSummary>,
    ci:                     Option<CiStatus>,
    worktrees:              Vec<WorktreeInfo>,
    git_detail_ms:          u64,
    disk_ms:                u64,
    worktrees_ms:           u64,
}

/// Phase 2 output — every field derived from
/// `WorkspaceMetadata` / `PackageRecord` lookups plus the in-project
/// disk-usage breakdown. Same timing capture as `RuntimeFields`.
pub(super) struct MetadataFields {
    types:                    Option<Vec<ProjectType>>,
    manifest:                 ManifestFields,
    in_project_target:        Option<u64>,
    in_project_non_target:    Option<u64>,
    out_of_tree_target_bytes: Option<u64>,
    metadata_ms:              u64,
    oot_ms:                   u64,
}

/// Phase 3 output — the two enum statuses derived from raw
/// crates.io data plus the live service-availability snapshot. Kept
/// together because both feed the same `PackageData` row gating
/// logic.
pub(super) struct CratesIoStatus {
    publish: PublishStatus,
    service: ServiceStatus,
}

/// Phase 1: collect every runtime-derived field (git state, disk
/// usage, CI status, worktree list, crates.io version cache) along
/// with the elapsed time for each block.
pub(super) fn collect_runtime_fields(
    app: &App,
    abs_path: &Path,
    wt_item: Option<&RootItem>,
) -> RuntimeFields {
    let t_git = std::time::Instant::now();
    let git_detail = git_data::build_git_detail_fields(app, abs_path);
    let git_detail_ms = tui_pane::perf_log_ms(t_git.elapsed().as_millis());

    let crates_io = resolve_crates_io_fields(app, abs_path);

    let t_disk = std::time::Instant::now();
    let disk = app
        .project_list
        .at_path(abs_path)
        .and_then(|project| project.disk_usage_bytes);
    let worktree_group_summary = wt_item.and_then(worktree_group_summary_for);
    let ci = compute_ci_status(app, abs_path, wt_item);
    let disk_ms = tui_pane::perf_log_ms(t_disk.elapsed().as_millis());

    let t_wt = std::time::Instant::now();
    let worktrees = git_data::resolve_worktrees(app, wt_item);
    let worktrees_ms = tui_pane::perf_log_ms(t_wt.elapsed().as_millis());

    RuntimeFields {
        git_detail,
        crates_io,
        disk,
        worktree_group_summary,
        ci,
        worktrees,
        git_detail_ms,
        disk_ms,
        worktrees_ms,
    }
}

/// Phase 2: pull the workspace-metadata-derived fields (manifest,
/// in-project byte breakdown, out-of-tree target bytes) and roll up
/// the cargo target-kind list into a comma-separated label.
pub(super) fn collect_metadata_fields(
    app: &App,
    abs_path: &Path,
    abs_path_owned: &AbsolutePath,
    cargo: Option<&Cargo>,
    fallback_type: Option<ProjectType>,
) -> MetadataFields {
    // `None` for non-Rust projects (no cargo). For Rust projects the
    // resolved target kinds, falling back to the project's own
    // identity (e.g. `Workspace`) when metadata has not yet supplied
    // any — an empty vec then means a resolved package with no
    // lib/bin/proc-macro target.
    let types = cargo.map(|c| {
        let resolved = c.types();
        if resolved.is_empty() {
            fallback_type.into_iter().collect()
        } else {
            resolved.to_vec()
        }
    });

    let t_meta = std::time::Instant::now();
    let package_record = lookup_package_record(app, abs_path_owned);
    let metadata_ms = tui_pane::perf_log_ms(t_meta.elapsed().as_millis());
    let manifest = manifest_fields_from(package_record.as_ref());

    let (in_project_target, in_project_non_target) =
        compute_in_project_bytes(&app.project_list, abs_path);
    let t_oot = std::time::Instant::now();
    let out_of_tree_target_bytes =
        package_data::lookup_out_of_tree_target_bytes(app, abs_path_owned);
    let oot_ms = tui_pane::perf_log_ms(t_oot.elapsed().as_millis());

    MetadataFields {
        types,
        manifest,
        in_project_target,
        in_project_non_target,
        out_of_tree_target_bytes,
        metadata_ms,
        oot_ms,
    }
}

/// Phase 3: collapse the raw `CratesIoFields` plus the live
/// availability state into the two enums that gate the
/// `PackageData` rendering.
pub(super) const fn derive_crates_io_status(
    crates_io: &CratesIoFields,
    app: &App,
) -> CratesIoStatus {
    let service = if app.net.crates_io.availability.toast_id().is_some() {
        ServiceStatus::Unreachable
    } else {
        ServiceStatus::Available
    };
    CratesIoStatus {
        publish: crates_io.publish,
        service,
    }
}

/// Build the crates.io stats-section rows from the resolved fields plus
/// the live availability state. Empty for non-publishable projects.
pub(super) fn build_crates_io_rows(
    crates_io: &CratesIoFields,
    status: &CratesIoStatus,
) -> Vec<(&'static str, String)> {
    let mut rows = Vec::new();
    if let Some(version) = crates_io.version.as_deref() {
        rows.push(("version", version.to_string()));
        if let Some(prerelease) = crates_io.prerelease.as_deref() {
            rows.push((prerelease_label(prerelease), prerelease.to_string()));
        }
        if let Some(downloads) = crates_io.downloads {
            rows.push(("downloads", format_downloads(downloads)));
        }
    } else if status.publish.is_publishable()
        && matches!(status.service, ServiceStatus::Unreachable)
    {
        rows.push(("version", CRATES_IO_UNREACHABLE.to_string()));
    }
    rows
}

/// Label for the prerelease row, derived from the leading alphabetic
/// token of the prerelease identifier: `0.21.0-rc.2` → `rc`,
/// `1.0.0-beta.1` → `beta`, `1.0.0-alpha` → `alpha`; anything else → `pre`.
pub(super) fn prerelease_label(prerelease: &str) -> &'static str {
    let identifier = prerelease.split('-').nth(1).unwrap_or_default();
    let token = identifier
        .split(|c: char| !c.is_ascii_alphabetic())
        .next()
        .unwrap_or_default();
    match token {
        "rc" => "rc",
        "beta" => "beta",
        "alpha" => "alpha",
        _ => "pre",
    }
}

/// Emit the combined `pane_common_breakdown` perf log line so each
/// helper doesn't log independently — keeping the existing log
/// format intact for downstream tracing consumers.
fn log_pane_common_breakdown(abs_path: &Path, runtime: &RuntimeFields, metadata: &MetadataFields) {
    tracing::trace!(
        target: tui_pane::PERF_LOG_TARGET,
        git_detail_ms = runtime.git_detail_ms,
        disk_ms = runtime.disk_ms,
        worktrees_ms = runtime.worktrees_ms,
        metadata_ms = metadata.metadata_ms,
        oot_ms = metadata.oot_ms,
        path = %abs_path.display(),
        "pane_common_breakdown"
    );
}

/// Look up the workspace that covers `abs_path` and aggregate its
/// runnable targets. Returns `TargetsData::default()` when no
/// metadata covers the path yet — callers render an empty pane in
/// that case so we don't surface a hand-parsed view that disagrees
/// with cargo's discovery rules.
pub(super) fn lookup_targets_data(
    app: &App,
    abs_path: &AbsolutePath,
    wt_item: Option<&RootItem>,
) -> TargetsData {
    if let Some(data) = lookup_worktree_group_targets(app, wt_item) {
        return data;
    }
    lookup_targets_data_for_path(app, abs_path)
}

fn lookup_worktree_group_targets(app: &App, wt_item: Option<&RootItem>) -> Option<TargetsData> {
    let RootItem::Worktrees(group) = wt_item? else {
        return None;
    };
    if !group.renders_as_group() {
        return None;
    }

    let mut merged = TargetsData::default();
    for entry in group
        .iter_entries()
        .filter(|entry| entry.visibility() == Visibility::Visible)
    {
        let mut targets = lookup_targets_data_for_path(app, entry.path());
        targets.relabel_as_worktree(&entry.root_directory_name().into_string());
        merged.append(targets);
    }
    merged.sort_entries();
    Some(merged)
}

fn lookup_targets_data_for_path(app: &App, abs_path: &AbsolutePath) -> TargetsData {
    let handle = app.scan.metadata_store_handle();
    let Ok(store) = handle.lock() else {
        return TargetsData::default();
    };
    let Some(root) = store.containing_workspace_root(abs_path) else {
        return TargetsData::default();
    };
    let Some(metadata) = store.get(root) else {
        return TargetsData::default();
    };
    TargetsData::from_workspace_metadata(metadata, abs_path)
}

/// Assemble `DetailPaneData` from already-resolved inputs.
pub(super) fn assemble_detail_pane_data(
    package: PackageData,
    git_detail: GitDetailFields,
    worktrees: Vec<WorktreeInfo>,
    targets: TargetsData,
) -> DetailPaneData {
    DetailPaneData {
        package,
        git: GitData {
            head: git_detail.head,
            head_relation: git_detail.head_relation,
            bisect: git_detail.bisect,
            status: git_detail.path,
            vs_local: git_detail.vs_local,
            stars: git_detail.stars,
            description: git_detail.description,
            inception: git_detail.inception,
            last_commit: git_detail.last_commit,
            last_fetched: git_detail.last_fetched,
            rate_limit_core: git_detail.rate_limit_core,
            rate_limit_graphql: git_detail.rate_limit_graphql,
            github_status: git_detail.github_status,
            pull_requests: git_detail.pull_requests,
            remotes: git_detail.remotes,
            worktrees,
            submodule_ctx: None,
        },
        targets,
    }
}

#[cfg(test)]
mod tests {
    use super::CRATES_IO_UNREACHABLE;
    use super::CratesIoFields;
    use super::CratesIoStatus;
    use super::PublishStatus;
    use super::ServiceStatus;
    use super::build_crates_io_rows;
    use super::prerelease_label;

    fn publishable_available() -> CratesIoStatus {
        CratesIoStatus {
            publish: PublishStatus::Publishable,
            service: ServiceStatus::Available,
        }
    }

    #[test]
    fn crates_io_rows_show_stable_and_prerelease_when_both_present() {
        let fields = CratesIoFields {
            version:    Some("0.20.2".to_string()),
            prerelease: Some("0.21.0-rc.2".to_string()),
            downloads:  Some(663),
            publish:    PublishStatus::Publishable,
        };
        assert_eq!(
            build_crates_io_rows(&fields, &publishable_available()),
            vec![
                ("version", "0.20.2".to_string()),
                ("rc", "0.21.0-rc.2".to_string()),
                ("downloads", "663".to_string()),
            ],
        );
    }

    #[test]
    fn crates_io_rows_omit_prerelease_row_when_only_stable() {
        let fields = CratesIoFields {
            version:    Some("1.0.0".to_string()),
            prerelease: None,
            downloads:  Some(10),
            publish:    PublishStatus::Publishable,
        };
        assert_eq!(
            build_crates_io_rows(&fields, &publishable_available()),
            vec![
                ("version", "1.0.0".to_string()),
                ("downloads", "10".to_string())
            ],
        );
    }

    #[test]
    fn crates_io_rows_empty_for_non_publishable() {
        let fields = CratesIoFields {
            version:    None,
            prerelease: None,
            downloads:  None,
            publish:    PublishStatus::NotPublishable,
        };
        let status = CratesIoStatus {
            publish: PublishStatus::NotPublishable,
            service: ServiceStatus::Unreachable,
        };
        assert!(build_crates_io_rows(&fields, &status).is_empty());
    }

    #[test]
    fn crates_io_rows_show_unreachable_placeholder_during_outage() {
        let fields = CratesIoFields {
            version:    None,
            prerelease: None,
            downloads:  None,
            publish:    PublishStatus::Publishable,
        };
        let status = CratesIoStatus {
            publish: PublishStatus::Publishable,
            service: ServiceStatus::Unreachable,
        };
        assert_eq!(
            build_crates_io_rows(&fields, &status),
            vec![("version", CRATES_IO_UNREACHABLE.to_string())],
        );
    }

    #[test]
    fn prerelease_label_reflects_identifier_kind() {
        assert_eq!(prerelease_label("0.21.0-rc.2"), "rc");
        assert_eq!(prerelease_label("1.0.0-beta.1"), "beta");
        assert_eq!(prerelease_label("1.0.0-alpha"), "alpha");
        assert_eq!(prerelease_label("1.0.0-pre.3"), "pre");
        assert_eq!(prerelease_label("1.0.0"), "pre");
    }
}
