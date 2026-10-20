use tui_pane::PERF_LOG_TARGET;

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
use super::TargetsData;
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
use super::package_data::PackageVersion;
use super::package_data::StructureCounts;
use super::package_data::WorktreeGroupSummary;
use super::project;
use crate::project::CratesIoRelease;
use crate::project::CratesIoReleaseState;
use crate::project::ProjectFields;
use crate::tui::panes::DescriptionBlock;
use crate::tui::panes::targets;
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WorktreeRootDisplay {
    Grouped,
    Single,
}

impl From<&RootItem> for WorktreeRootDisplay {
    fn from(item: &RootItem) -> Self {
        if git_data::is_worktree_group(item) {
            Self::Grouped
        } else {
            Self::Single
        }
    }
}

impl WorktreeRootDisplay {
    const fn worktree_item(self, item: Option<&RootItem>) -> Option<&RootItem> {
        match self {
            Self::Grouped => item,
            Self::Single => None,
        }
    }

    const fn primary_section(self, package_section: PackageSection) -> Option<PackageSection> {
        match self {
            Self::Grouped => Some(package_section),
            Self::Single => None,
        }
    }
}

/// Build pane data for a root `RootItem`.
pub fn build_pane_data(app: &App, item: &RootItem) -> DetailPaneData {
    let display_path = item.display_path().into_string();
    let worktree_root_display = WorktreeRootDisplay::from(item);

    match item {
        RootItem::Rust(RustProject::Workspace(ws)) => {
            build_pane_data_for_workspace(app, ws, &display_path, worktree_root_display, Some(item))
        },
        RootItem::Rust(RustProject::Package(pkg)) => {
            build_pane_data_for_package(app, pkg, &display_path, worktree_root_display, Some(item))
        },
        RootItem::NonRust(nr) => {
            build_pane_data_non_rust(app, nr, &display_path, worktree_root_display, Some(item))
        },
        RootItem::Worktrees(group) => match &group.primary {
            RustProject::Workspace(ws) => build_pane_data_for_workspace(
                app,
                ws,
                &display_path,
                worktree_root_display,
                Some(item),
            ),
            RustProject::Package(pkg) => build_pane_data_for_package(
                app,
                pkg,
                &display_path,
                worktree_root_display,
                Some(item),
            ),
        },
    }
}

/// Build pane data for a workspace member.
pub fn build_pane_data_for_member(app: &App, pkg: &Package) -> DetailPaneData {
    let display_path = pkg.display_path().into_string();
    build_pane_data_for_package(app, pkg, &display_path, WorktreeRootDisplay::Single, None)
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
            name: vendored.package_name().into_string(),
            package_presence: PackagePresence::Present,
            cargo: Some(cargo),
            worktree_item: None,
            stats_rows,
            test_rows,
            primary_section: None,
            fallback_type: None,
            title: "Vendored Crate".to_string(),
        },
    )
}

/// Build pane data for a linked `Project<Workspace>` worktree entry.
pub fn build_pane_data_for_workspace_ref(
    app: &App,
    ws: &Workspace,
    display_path: &str,
) -> DetailPaneData {
    build_pane_data_for_workspace(app, ws, display_path, WorktreeRootDisplay::Single, None)
}

/// Build pane data for a git submodule nested under a project.
pub fn build_pane_data_for_submodule(app: &App, submodule: &Submodule) -> DetailPaneData {
    let abs_path = &submodule.path;
    let display_path = project::home_relative_path(abs_path);
    let git_detail = git_data::build_git_detail_fields(app, abs_path);

    let submodule_ctx = git_data::build_submodule_context(submodule);

    DetailPaneData {
        package: PackageData {
            title:                    "Submodule".to_string(),
            name:                     submodule.name.clone(),
            worktree_group_summary:   None,
            primary_section:          None,
            path:                     display_path,
            version:                  PackageVersion::Submodule(submodule.commit.clone()),
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
            checkout: git_detail.checkout,
            failed_reads: git_detail.failed_reads,
            bisect: git_detail.bisect,
            status: git_detail.path,
            vs_local: git_detail.vs_local,
            local_main_branch: git_detail.local_main_branch,
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
    worktree_root_display: WorktreeRootDisplay,
    worktree_item: Option<&RootItem>,
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

    let worktree_item_ref = worktree_root_display.worktree_item(worktree_item);
    let title = worktree_item_ref.map_or_else(
        || "Workspace".to_string(),
        |item| resolve_package_title(app, item),
    );
    build_pane_data_common(
        app,
        PaneDataSource {
            abs_path,
            display_path,
            name: ws.package_name().into_string(),
            package_presence: PackagePresence::from(ws.name().is_some()),
            cargo: Some(cargo),
            worktree_item: worktree_item_ref,
            stats_rows,
            test_rows,
            primary_section: worktree_root_display
                .primary_section(PackageSection::PrimaryWorkspace),
            fallback_type: Some(ProjectType::Workspace),
            title,
        },
    )
}

fn build_pane_data_for_package(
    app: &App,
    pkg: &Package,
    display_path: &str,
    worktree_root_display: WorktreeRootDisplay,
    worktree_item: Option<&RootItem>,
) -> DetailPaneData {
    let abs_path = pkg.path();
    let cargo = &pkg.cargo;

    let mut counts = StructureCounts::default();
    counts.add_package(pkg);
    let stats_rows = counts.to_rows();
    let test_rows = package_data::test_rows_from_counts(pkg.info().test_counts.unwrap_or_default());

    let worktree_item_ref = worktree_root_display.worktree_item(worktree_item);
    let title = worktree_item.map_or_else(
        || resolve_package_title_for_package(app, pkg),
        |item| resolve_package_title(app, item),
    );

    build_pane_data_common(
        app,
        PaneDataSource {
            abs_path,
            display_path,
            name: pkg.package_name().into_string(),
            package_presence: PackagePresence::Present,
            cargo: Some(cargo),
            worktree_item: worktree_item_ref,
            stats_rows,
            test_rows,
            primary_section: worktree_root_display.primary_section(PackageSection::PrimaryPackage),
            fallback_type: None,
            title,
        },
    )
}

fn build_pane_data_non_rust(
    app: &App,
    nr: &NonRustProject,
    display_path: &str,
    worktree_root_display: WorktreeRootDisplay,
    worktree_item: Option<&RootItem>,
) -> DetailPaneData {
    let abs_path = nr.path();
    let worktree_item_ref = worktree_root_display.worktree_item(worktree_item);
    let mut counts = StructureCounts::default();
    counts.add_non_rust(nr);

    build_pane_data_common(
        app,
        PaneDataSource {
            abs_path,
            display_path,
            name: nr.root_directory_name().into_string(),
            package_presence: PackagePresence::Missing,
            cargo: None,
            worktree_item: worktree_item_ref,
            stats_rows: counts.to_rows(),
            test_rows: Vec::new(),
            primary_section: None,
            fallback_type: None,
            title: "Project".to_string(),
        },
    )
}

pub(super) struct PaneDataSource<'a> {
    abs_path:         &'a Path,
    display_path:     &'a str,
    name:             String,
    package_presence: PackagePresence,
    cargo:            Option<&'a Cargo>,
    worktree_item:    Option<&'a RootItem>,
    stats_rows:       Vec<(&'static str, usize)>,
    test_rows:        Vec<(&'static str, usize)>,
    primary_section:  Option<PackageSection>,
    fallback_type:    Option<ProjectType>,
    title:            String,
}

/// The crates.io section has either a complete release or a reason none is shown.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum CratesIoSection {
    NotPublishable,
    AwaitingRelease,
    Unreachable,
    Released(CratesIoRelease),
}

fn resolve_crates_io_section(app: &App, abs_path: &Path) -> CratesIoSection {
    let service = if app.net.crates_io.availability.toast_id().is_some() {
        ServiceStatus::Unreachable
    } else {
        ServiceStatus::Available
    };
    crates_io_section(
        app.project_list.rust_info_at_path(abs_path),
        app.project_list.vendored_at_path(abs_path),
        service,
    )
}

fn crates_io_section(
    rust_info: Option<&RustInfo>,
    vendored: Option<&VendoredPackage>,
    service: ServiceStatus,
) -> CratesIoSection {
    if !rust_info.is_some_and(|r| r.cargo.publishable())
        && !vendored.is_some_and(|v| v.cargo.publishable() && v.name.is_some())
    {
        return CratesIoSection::NotPublishable;
    }
    // Resolve the entire release from one source. In particular, a stable-only
    // RustInfo release must never acquire the vendored source's prerelease.
    match (
        rust_info.map(|r| &r.crates_io),
        vendored.map(|v| &v.crates_io),
        service,
    ) {
        (Some(CratesIoReleaseState::Released(release)), _, _)
        | (_, Some(CratesIoReleaseState::Released(release)), _) => {
            CratesIoSection::Released(release.clone())
        },
        (_, _, ServiceStatus::Unreachable) => CratesIoSection::Unreachable,
        (_, _, ServiceStatus::Available) => CratesIoSection::AwaitingRelease,
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
    title:                    String,
    name:                     String,
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
        title: args.title,
        name: args.name,
        worktree_group_summary: args.worktree_group_summary,
        primary_section: args.primary_section,
        path: args.display_path,
        version: version.map_or(PackageVersion::AwaitingMetadata, PackageVersion::Manifest),
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

fn compute_ci_status(
    app: &App,
    abs_path: &Path,
    worktree_item: Option<&RootItem>,
) -> Option<CiStatus> {
    let lookup = app.ci.status_lookup();
    worktree_item.map_or_else(
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
    title: &str,
) -> (LintDisplay, CiDisplay) {
    let pl = &app.project_list;
    let is_worktree_group = title == "Worktree Group";
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
/// every pane builder shares. Runtime state includes the resolved crates.io
/// section phase; metadata lookups follow, then the package /
/// targets pieces are constructed and the final result is assembled.
fn build_pane_data_common(app: &App, src: PaneDataSource<'_>) -> DetailPaneData {
    let abs_path = src.abs_path;
    let abs_path_owned = AbsolutePath::from(abs_path);

    let runtime = collect_runtime_fields(app, abs_path, src.worktree_item);
    let metadata =
        collect_metadata_fields(app, abs_path, &abs_path_owned, src.cargo, src.fallback_type);
    log_pane_common_breakdown(abs_path, &runtime, &metadata);

    let (lint_display, ci_display) =
        compute_package_displays(app, &abs_path_owned, runtime.ci, &src.title);

    let package = build_package_data(BuildPackageDataArgs {
        title: src.title,
        name: src.name,
        display_path: src.display_path.to_owned(),
        stats_rows: src.stats_rows,
        test_rows: src.test_rows,
        worktree_group_summary: runtime.worktree_group_summary,
        primary_section: src.primary_section,
        package_presence: src.package_presence,
        manifest: metadata.manifest,
        crates_io_rows: build_crates_io_rows(&runtime.crates_io),
        types: metadata.types,
        disk: runtime.disk,
        in_project_target: metadata.in_project_target,
        in_project_non_target: metadata.in_project_non_target,
        out_of_tree_target_bytes: metadata.out_of_tree_target_bytes,
        lint_display,
        ci_display,
    });

    let targets = targets::lookup_targets_data(app, &abs_path_owned, src.worktree_item);
    assemble_detail_pane_data(package, runtime.git_detail, runtime.worktrees, targets)
}

/// Phase 1 output — every field that depends on runtime state
/// (filesystem walks, network probes, in-memory caches). Captures
/// per-piece millisecond timings so the orchestrator can emit a
/// single combined log line without each helper logging
/// independently.
pub(super) struct RuntimeFields {
    git_detail:             GitDetailFields,
    crates_io:              CratesIoSection,
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
    out_of_tree_ms:           u64,
}

/// Phase 1: collect every runtime-derived field (git state, disk
/// usage, CI status, worktree list, crates.io version cache) along
/// with the elapsed time for each block.
pub(super) fn collect_runtime_fields(
    app: &App,
    abs_path: &Path,
    worktree_item: Option<&RootItem>,
) -> RuntimeFields {
    let t_git = std::time::Instant::now();
    let git_detail = git_data::build_git_detail_fields(app, abs_path);
    let git_detail_ms = tui_pane::perf_log_ms(t_git.elapsed().as_millis());

    let crates_io = resolve_crates_io_section(app, abs_path);

    let t_disk = std::time::Instant::now();
    let disk = app
        .project_list
        .at_path(abs_path)
        .and_then(|project| project.disk_usage_bytes);
    let worktree_group_summary = worktree_item.and_then(worktree_group_summary_for);
    let ci = compute_ci_status(app, abs_path, worktree_item);
    let disk_ms = tui_pane::perf_log_ms(t_disk.elapsed().as_millis());

    let t_worktrees = std::time::Instant::now();
    let worktrees = git_data::resolve_worktrees(app, worktree_item);
    let worktrees_ms = tui_pane::perf_log_ms(t_worktrees.elapsed().as_millis());

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
    let t_out_of_tree = std::time::Instant::now();
    let out_of_tree_target_bytes =
        package_data::lookup_out_of_tree_target_bytes(app, abs_path_owned);
    let out_of_tree_ms = tui_pane::perf_log_ms(t_out_of_tree.elapsed().as_millis());

    MetadataFields {
        types,
        manifest,
        in_project_target,
        in_project_non_target,
        out_of_tree_target_bytes,
        metadata_ms,
        out_of_tree_ms,
    }
}

/// Render each crates.io phase deliberately; only a complete release owns metrics.
pub(super) fn build_crates_io_rows(section: &CratesIoSection) -> Vec<(&'static str, String)> {
    match section {
        CratesIoSection::NotPublishable | CratesIoSection::AwaitingRelease => Vec::new(),
        CratesIoSection::Unreachable => vec![("version", CRATES_IO_UNREACHABLE.to_string())],
        CratesIoSection::Released(release) => {
            let mut rows = vec![("version", release.version.clone())];
            if let Some(prerelease) = &release.prerelease {
                rows.push((prerelease_label(prerelease), prerelease.clone()));
            }
            rows.push(("downloads", format_downloads(release.downloads)));
            rows
        },
    }
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
        target: PERF_LOG_TARGET,
        git_detail_ms = runtime.git_detail_ms,
        disk_ms = runtime.disk_ms,
        worktrees_ms = runtime.worktrees_ms,
        metadata_ms = metadata.metadata_ms,
        out_of_tree_ms = metadata.out_of_tree_ms,
        path = %abs_path.display(),
        "pane_common_breakdown"
    );
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
            checkout: git_detail.checkout,
            failed_reads: git_detail.failed_reads,
            bisect: git_detail.bisect,
            status: git_detail.path,
            vs_local: git_detail.vs_local,
            local_main_branch: git_detail.local_main_branch,
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
    use super::CratesIoSection;
    use super::ServiceStatus;
    use super::build_crates_io_rows;
    use super::crates_io_section;
    use super::prerelease_label;
    use crate::project::CratesIoRelease;
    use crate::project::CratesIoReleaseState;
    use crate::project::PublishStatus;
    use crate::project::RustInfo;
    use crate::project::VendoredPackage;

    fn release(version: &str, prerelease: Option<&str>, downloads: u64) -> CratesIoRelease {
        CratesIoRelease {
            version: version.to_string(),
            prerelease: prerelease.map(str::to_string),
            downloads,
        }
    }

    #[test]
    fn crates_io_rows_show_stable_and_prerelease_when_both_present() {
        let section = CratesIoSection::Released(release("0.20.2", Some("0.21.0-rc.2"), 663));
        assert_eq!(
            build_crates_io_rows(&section),
            vec![
                ("version", "0.20.2".to_string()),
                ("rc", "0.21.0-rc.2".to_string()),
                ("downloads", "663".to_string()),
            ],
        );
    }

    #[test]
    fn crates_io_rows_omit_prerelease_row_when_only_stable() {
        let section = CratesIoSection::Released(release("1.0.0", None, 10));
        assert_eq!(
            build_crates_io_rows(&section),
            vec![
                ("version", "1.0.0".to_string()),
                ("downloads", "10".to_string())
            ],
        );
    }

    #[test]
    fn crates_io_without_release_has_explicit_section_phases() {
        let mut rust = RustInfo::default();
        assert_eq!(
            crates_io_section(Some(&rust), None, ServiceStatus::Available),
            CratesIoSection::AwaitingRelease
        );
        assert_eq!(
            crates_io_section(Some(&rust), None, ServiceStatus::Unreachable),
            CratesIoSection::Unreachable
        );
        rust.cargo.publish_status = PublishStatus::NotPublishable;
        assert_eq!(
            crates_io_section(Some(&rust), None, ServiceStatus::Unreachable),
            CratesIoSection::NotPublishable
        );
        for section in [
            CratesIoSection::AwaitingRelease,
            CratesIoSection::NotPublishable,
        ] {
            assert_eq!(build_crates_io_rows(&section), [] as [(&str, String); 0]);
        }
        assert_eq!(
            build_crates_io_rows(&CratesIoSection::Unreachable),
            vec![("version", CRATES_IO_UNREACHABLE.to_string())]
        );
    }

    #[test]
    fn crates_io_release_never_mixes_metadata_from_different_sources() {
        // The preferred source has no newer prerelease. Field-by-field fallback
        // would incorrectly attach the other source's prerelease to version 1.0.0.
        let rust_release = release("1.0.0", None, 10);
        let vendored_release = release("2.0.0", Some("3.0.0-beta.1"), 900);
        let mut rust = RustInfo {
            crates_io: CratesIoReleaseState::Released(rust_release.clone()),
            ..RustInfo::default()
        };
        let vendored = VendoredPackage {
            name: Some("demo".to_string()),
            crates_io: CratesIoReleaseState::Released(vendored_release.clone()),
            ..VendoredPackage::default()
        };
        for service in [ServiceStatus::Available, ServiceStatus::Unreachable] {
            let section = crates_io_section(Some(&rust), Some(&vendored), service);
            assert_eq!(section, CratesIoSection::Released(rust_release.clone()));
            assert_eq!(
                build_crates_io_rows(&section),
                vec![
                    ("version", "1.0.0".to_string()),
                    ("downloads", "10".to_string())
                ]
            );
        }
        rust.crates_io = CratesIoReleaseState::AwaitingRelease;
        assert_eq!(
            crates_io_section(Some(&rust), Some(&vendored), ServiceStatus::Available),
            CratesIoSection::Released(vendored_release)
        );
        assert_eq!(
            crates_io_section(None, Some(&vendored), ServiceStatus::Available),
            CratesIoSection::Released(release("2.0.0", Some("3.0.0-beta.1"), 900))
        );
        rust.crates_io = CratesIoReleaseState::Released(rust_release);
        rust.cargo.publish_status = PublishStatus::NotPublishable;
        assert_eq!(
            crates_io_section(Some(&rust), None, ServiceStatus::Available),
            CratesIoSection::NotPublishable
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
