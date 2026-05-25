mod formatting;

use std::cmp::Ordering;
use std::path::Component;
use std::path::Path;

use cargo_metadata::TargetKind;
pub use formatting::format_ahead_behind_against;
pub use formatting::format_date;
pub use formatting::format_duration;
use formatting::format_rate_limit_bucket;
pub use formatting::format_remote_status;
pub use formatting::format_time;
pub use formatting::format_timestamp;
use ratatui::style::Color;
use tui_pane::CopyLabel;
use tui_pane::CopyPayload;
use tui_pane::CopySelectionResult;

use crate::ci;
use crate::ci::CiRun;
use crate::ci::CiStatus;
use crate::constants::GIT_CLONE;
use crate::constants::GIT_DIR;
use crate::constants::GIT_FORK;
use crate::constants::NO_REMOTE_SYNC;
use crate::http::RateLimitQuota;
use crate::http::ServiceKind;
use crate::lint;
use crate::lint::LintRun;
use crate::project;
use crate::project::AbsolutePath;
use crate::project::Cargo;
use crate::project::GitOrigin;
use crate::project::GitStatus;
use crate::project::HeadState;
use crate::project::NonRustProject;
use crate::project::Package;
use crate::project::PackageRecord;
use crate::project::ProjectFields;
use crate::project::ProjectType;
use crate::project::PushDisabledReason;
use crate::project::PushState;
use crate::project::RemoteKind;
use crate::project::RepoInfo;
use crate::project::RootItem;
use crate::project::RustInfo;
use crate::project::RustProject;
use crate::project::Submodule;
use crate::project::VendoredPackage;
use crate::project::Visibility;
use crate::project::Workspace;
use crate::project::WorkspaceMetadata;
use crate::tui::app::App;
use crate::tui::app::AvailabilityStatus;
use crate::tui::project_list::ProjectList;
use crate::tui::render;
use crate::tui::state::ServiceStatus;

#[derive(Default)]
struct ProjectCounts {
    workspaces:  usize,
    libs:        usize,
    bins:        usize,
    proc_macros: usize,
    examples:    usize,
    benches:     usize,
    tests:       usize,
}

impl ProjectCounts {
    fn add_package(&mut self, project: &Package) { self.add_cargo(&project.cargo); }

    fn add_workspace(&mut self, ws: &Workspace) {
        self.workspaces += 1;
        self.add_cargo(&ws.cargo);
    }

    fn add_cargo(&mut self, cargo: &Cargo) {
        for t in cargo.types() {
            match t {
                ProjectType::Workspace => {},
                ProjectType::Library => self.libs += 1,
                ProjectType::Binary => self.bins += 1,
                ProjectType::ProcMacro => self.proc_macros += 1,
            }
        }
        self.examples += cargo.example_count();
        self.benches += cargo.benches().len();
        self.tests += cargo.test_count();
    }

    /// Returns non-zero stats as (label, count) pairs for column display.
    fn to_rows(&self) -> Vec<(&'static str, usize)> {
        let mut rows = Vec::new();
        if self.workspaces > 0 {
            rows.push(("ws", self.workspaces));
        }
        if self.libs > 0 {
            rows.push(("lib", self.libs));
        }
        if self.bins > 0 {
            rows.push(("bin", self.bins));
        }
        if self.proc_macros > 0 {
            rows.push(("proc-macro", self.proc_macros));
        }
        if self.examples > 0 {
            rows.push(("example", self.examples));
        }
        if self.benches > 0 {
            rows.push(("bench", self.benches));
        }
        if self.tests > 0 {
            rows.push(("test", self.tests));
        }
        rows
    }
}

#[derive(Clone, Copy, Debug, Hash, Eq, PartialEq)]
pub enum RunTargetKind {
    Binary,
    Example,
    Bench,
}

impl RunTargetKind {
    pub fn color(self) -> Color {
        match self {
            Self::Binary => tui_pane::success_color(),
            Self::Example => tui_pane::accent_color(),
            Self::Bench => tui_pane::target_bench_color(),
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Binary => "bin",
            Self::Example => "example",
            Self::Bench => "bench",
        }
    }

    /// Longest label width across all variants, plus 1 for trailing pad.
    pub const fn padded_label_width() -> usize {
        let mut max = 0;
        let labels: [&str; 3] = ["bin", "example", "bench"];
        let mut i = 0;
        while i < labels.len() {
            if labels[i].len() > max {
                max = labels[i].len();
            }
            i += 1;
        }
        max + 1
    }
}

/// Where a target lives within a workspace. `Workspace` is the root
/// package (its manifest sits at `workspace_root/Cargo.toml`); `Member`
/// is any other workspace member, tagged with its cargo `[package].name`
/// so the UI can show it and downstream `cargo` invocations can pass
/// `--package <name>`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TargetSource {
    Workspace,
    Member(String),
}

impl TargetSource {
    pub const fn label(&self) -> &str {
        match self {
            Self::Workspace => "workspace",
            Self::Member(name) => name.as_str(),
        }
    }

    /// Sort key: `Workspace` first, then members alphabetical by name.
    const fn sort_key(&self) -> (u8, &str) {
        match self {
            Self::Workspace => (0, ""),
            Self::Member(name) => (1, name.as_str()),
        }
    }
}

#[derive(Clone, Debug)]
pub struct TargetEntry {
    pub name:         String,
    pub display_name: String,
    pub kind:         RunTargetKind,
    pub source:       TargetSource,
    pub src_path:     AbsolutePath,
}

#[derive(Clone, Copy)]
pub enum BuildMode {
    Debug,
    Release,
}

impl BuildMode {
    pub const fn is_release(self) -> bool { matches!(self, Self::Release) }

    pub const fn label(self) -> &'static str {
        if self.is_release() {
            " (release)"
        } else {
            " (dev)"
        }
    }
}

/// Flatten `TargetsData` into a single render order: binaries first,
/// then examples, then benches. Each kind section is pre-sorted by
/// [`TargetsData::from_workspace_metadata`]; this fn applies a stable
/// running-first pre-pass per section, so running rows float to the top
/// of their kind without disturbing alphabetical order otherwise.
pub fn build_target_list_from_data(
    data: &TargetsData,
    running_for: &dyn Fn(&TargetEntry) -> bool,
) -> Vec<TargetEntry> {
    let mut binaries = data.binaries.clone();
    let mut examples = data.examples.clone();
    let mut benches = data.benches.clone();
    let stable_running_first = |xs: &mut Vec<TargetEntry>| {
        xs.sort_by_key(|entry| !running_for(entry));
    };
    stable_running_first(&mut binaries);
    stable_running_first(&mut examples);
    stable_running_first(&mut benches);
    let mut entries = Vec::with_capacity(binaries.len() + examples.len() + benches.len());
    entries.extend(binaries);
    entries.extend(examples);
    entries.extend(benches);
    entries
}

pub struct PendingExampleRun {
    pub abs_path:     String,
    pub target_name:  String,
    pub package_name: Option<String>,
    pub kind:         RunTargetKind,
    pub build_mode:   BuildMode,
}

/// Whether a CI fetch should sync recent runs or discover older history.
#[derive(Clone, Copy)]
pub enum CiFetchKind {
    /// Fetch runs older than the oldest cached run.
    FetchOlder,
    /// Re-sync the most recent N runs, refreshing stale failures.
    Sync,
}

/// A pending request to fetch more CI runs for a project.
pub struct PendingCiFetch {
    pub project_path:      String,
    pub ci_run_count:      u32,
    pub oldest_created_at: Option<String>,
    pub kind:              CiFetchKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DetailField {
    Worktrees,
    DeletedWorktrees,
    Path,
    Targets,
    DiskTotal,
    Disk,
    /// Submodule overlay: `.gitmodules` tracking branch (the `branch =` line).
    Tracks,
    /// Submodule overlay: parent repo's pinned commit (`git ls-tree HEAD`).
    Pinned,
    /// Bytes consumed by the `target/` subtree rooted at the project.
    /// Shown alongside Disk when the walker has reported a breakdown.
    DiskTarget,
    /// Bytes under the project root that are *not* inside a `target/`
    /// subtree (source, docs, .git, etc.).
    DiskNonTarget,
    /// Sharer target: the workspace's `target_directory` lives outside
    /// `workspace_root` (e.g. redirected by `CARGO_TARGET_DIR` or a
    /// `.cargo/config.toml`). Byte total is filled by the cached
    /// out-of-tree walk (`BackgroundMsg::OutOfTreeTargetSize`) since the
    /// per-project walker never reaches there.
    DiskOutOfTreeTarget,
    Lint,
    Ci,
    Head,
    GitStatus,
    VsLocal,
    Stars,
    Inception,
    LastCommit,
    LastFetched,
    RateLimitCore,
    RateLimitGraphQl,
    WorktreeError,
    CratesIo,
    Downloads,
    Version,
    Edition,
    License,
    Homepage,
    Repository,
}

impl DetailField {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Worktrees => "Worktrees",
            Self::DeletedWorktrees => "Deleted",
            Self::Path => "Path",
            Self::Targets => "Type",
            Self::DiskTotal => "Disk total",
            Self::Disk => "Disk",
            Self::DiskTarget => "  target/",
            Self::DiskNonTarget => "  other",
            Self::DiskOutOfTreeTarget => "  target/ (out of tree)",
            Self::Lint => "Lint",
            Self::Ci => "CI",
            Self::Head => "Head",
            Self::Tracks => "Tracks",
            Self::Pinned => "Pinned",
            Self::GitStatus => "Status",
            Self::VsLocal => "Ahead/Behind",
            Self::Stars => "Stars",
            Self::Inception => "Incept",
            Self::LastCommit => "Latest",
            Self::LastFetched => "Fetched",
            Self::RateLimitCore => "Rate limit core",
            Self::RateLimitGraphQl => "Rate limit GraphQL",
            Self::WorktreeError => "Error",
            Self::CratesIo => "crates.io",
            Self::Downloads => "Downloads",
            Self::Version => "Version",
            Self::Edition => "Edition",
            Self::License => "License",
            Self::Homepage => "Homepage",
            Self::Repository => "Repository",
        }
    }

    /// Get the display value for a package field from `PackageData`.
    /// All values are pure-on-data. The Lint and Ci rows are *not*
    /// handled here — the package renderer matches on
    /// `data.lint_display` / `data.ci_display` (typed enums)
    /// directly and frames the icon at render time. Calling this
    /// with `Self::Lint` or `Self::Ci` returns an empty string.
    pub fn package_value(self, data: &PackageData) -> String {
        match self {
            Self::Worktrees => data
                .worktree_group_summary
                .as_ref()
                .map_or_else(String::new, |summary| summary.worktrees.to_string()),
            Self::DeletedWorktrees => data
                .worktree_group_summary
                .as_ref()
                .map_or_else(String::new, |summary| summary.deleted.to_string()),
            Self::DiskTotal => data
                .worktree_group_summary
                .as_ref()
                .map_or_else(String::new, |summary| summary.disk.clone()),
            Self::Path => data.path.clone(),
            Self::Disk => data.disk.clone(),
            Self::Targets => data.types.clone(),
            Self::CratesIo => data.crates_version.as_deref().map_or_else(
                || crates_io_placeholder(data).to_string(),
                ToString::to_string,
            ),
            Self::Downloads => data
                .crates_downloads
                .map_or_else(|| crates_io_placeholder(data).to_string(), format_downloads),
            Self::Version => data.version.clone(),
            Self::DiskTarget => data
                .in_project_target
                .map_or_else(String::new, render::format_bytes),
            Self::DiskNonTarget => data
                .in_project_non_target
                .map_or_else(String::new, render::format_bytes),
            Self::DiskOutOfTreeTarget => data
                .out_of_tree_target_bytes
                .map_or_else(String::new, render::format_bytes),
            Self::Edition => or_dash(data.edition.as_deref()),
            Self::License => or_dash(data.license.as_deref()),
            Self::Homepage => or_dash(data.homepage.as_deref()),
            Self::Repository => or_dash(data.repository.as_deref()),
            Self::WorktreeError => "broken .git — gitdir target missing".to_string(),
            // Git fields, Lint, and Ci — should not be called with
            // package_value. Lint and Ci are rendered directly from
            // their typed-enum fields (`PackageData.lint_display` /
            // `ci_display`) at render time.
            Self::Head
            | Self::Tracks
            | Self::Pinned
            | Self::GitStatus
            | Self::VsLocal
            | Self::Stars
            | Self::Inception
            | Self::LastCommit
            | Self::LastFetched
            | Self::RateLimitCore
            | Self::RateLimitGraphQl
            | Self::Lint
            | Self::Ci => String::new(),
        }
    }

    /// Get the display value for a git field from `GitData`.
    pub fn git_value(self, data: &GitData) -> String {
        match self {
            Self::Head => match data.head.as_ref() {
                None | Some(HeadState::Unborn) => "unborn".to_string(),
                Some(HeadState::Detached { short_sha }) => format!("detached @ {short_sha}"),
                Some(HeadState::Branch(name)) => {
                    let is_default = data
                        .local_main_branch
                        .as_deref()
                        .is_some_and(|db| db == name);
                    if is_default {
                        format!("{name} (HEAD)")
                    } else {
                        name.clone()
                    }
                },
            },
            Self::GitStatus => data
                .status
                .map_or_else(String::new, GitStatus::label_with_icon),
            Self::VsLocal => data.vs_local.as_deref().unwrap_or("").to_string(),
            Self::Stars => data
                .stars
                .map_or_else(String::new, |count| format!("⭐ {count}")),
            Self::Inception => data.inception.as_deref().unwrap_or("").to_string(),
            Self::LastCommit => data.last_commit.as_deref().unwrap_or("").to_string(),
            Self::LastFetched => data.last_fetched.as_deref().unwrap_or("").to_string(),
            Self::RateLimitCore => format_rate_limit_bucket(data.rate_limit_core),
            Self::RateLimitGraphQl => format_rate_limit_bucket(data.rate_limit_graphql),
            Self::Tracks => data
                .submodule_ctx
                .as_ref()
                .and_then(|context| context.tracks.as_deref())
                .map_or_else(String::new, |t| format!("{t}  (from .gitmodules)")),
            Self::Pinned => data
                .submodule_ctx
                .as_ref()
                .map(|ctx| format!("{}  (parent HEAD)", ctx.pinned_commit))
                .unwrap_or_default(),
            // Package fields — should not be called with git_value.
            Self::Worktrees
            | Self::DeletedWorktrees
            | Self::Path
            | Self::DiskTotal
            | Self::Disk
            | Self::DiskTarget
            | Self::DiskNonTarget
            | Self::DiskOutOfTreeTarget
            | Self::Targets
            | Self::Lint
            | Self::Ci
            | Self::CratesIo
            | Self::Downloads
            | Self::Version
            | Self::Edition
            | Self::License
            | Self::Homepage
            | Self::Repository
            | Self::WorktreeError => String::new(),
        }
    }
}

/// Per-service "unreachable" placeholder string. Shared across the
/// crates.io placeholder rows on the Package pane and the GitHub
/// stars placeholder row on the Git pane, so the wording stays
/// consistent across surfaces.
pub(super) const fn service_unreachable_placeholder(service: ServiceKind) -> &'static str {
    match service {
        ServiceKind::CratesIo => "crates.io unreachable",
        ServiceKind::GitHub => "github unreachable",
    }
}

/// Placeholder text shown in the `CratesIo` / Downloads value cells
/// when no data has landed and the service is confirmed unreachable.
/// Empty string in every other case — `package_fields_from_data`
/// already gates the row's visibility, so an empty string here means
/// "we have data but it's None" (e.g. workspace member with no
/// version), not "service is down."
pub(super) const fn crates_io_placeholder(data: &PackageData) -> &'static str {
    if data.publish_status.is_publishable()
        && matches!(data.crates_io_service, ServiceStatus::Unreachable)
    {
        service_unreachable_placeholder(ServiceKind::CratesIo)
    } else {
        ""
    }
}

/// True iff the value cell should be rendered in warning color —
/// publishable project with no version landed yet during a confirmed
/// crates.io outage.
pub const fn crates_io_value_is_unreachable_placeholder(data: &PackageData) -> bool {
    data.crates_version.is_none()
        && data.publish_status.is_publishable()
        && matches!(data.crates_io_service, ServiceStatus::Unreachable)
}

/// True iff the Git pane's Stars row should render a "github
/// unreachable" placeholder in warning color: GitHub is confirmed
/// down (Unreachable / `RateLimited`) and no stars count has landed
/// yet. Mirrors [`crates_io_value_is_unreachable_placeholder`] for
/// the GitHub-derived field — see [`service_unreachable_placeholder`]
/// for the shared string.
pub const fn github_stars_is_unreachable_placeholder(data: &GitData) -> bool {
    data.stars.is_none() && !data.github_status.is_available()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackageSection {
    WorktreeGroupSummary,
    PrimaryWorkspace,
    PrimaryPackage,
}

impl PackageSection {
    pub const fn label(self) -> &'static str {
        match self {
            Self::WorktreeGroupSummary => "Worktree Group Summary",
            Self::PrimaryWorkspace => "Primary Workspace",
            Self::PrimaryPackage => "Primary Package",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackageRow {
    Description,
    Section(PackageSection),
    Field(DetailField),
    Structure(usize),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorktreeGroupSummary {
    pub worktrees: usize,
    pub deleted:   usize,
    pub disk:      String,
}

/// Primary project fields for the `Package` column.
/// Non-Rust projects show only name, path, disk, and CI.
pub fn package_fields_from_data(data: &PackageData) -> Vec<DetailField> {
    if data.package_title == "Project" {
        return vec![
            DetailField::Path,
            DetailField::Disk,
            DetailField::Lint,
            DetailField::Ci,
        ];
    }
    let mut fields = vec![DetailField::Path, DetailField::Disk];
    // Insert the target / non-target breakdown immediately below the
    // aggregate Disk row when the walker has reported one, so the user
    // sees which half of the bytes is build artifact vs source (the
    // two always sum to Disk for owners; for sharers the target line
    // reads 0 — target is redirected out of tree).
    if data.in_project_target.is_some() {
        fields.push(DetailField::DiskTarget);
    }
    if data.in_project_non_target.is_some() {
        fields.push(DetailField::DiskNonTarget);
    }
    if data.out_of_tree_target_bytes.is_some() {
        fields.push(DetailField::DiskOutOfTreeTarget);
    }
    fields.push(DetailField::Targets);
    fields.push(DetailField::Lint);
    fields.push(DetailField::Ci);
    if data.has_package {
        fields.push(DetailField::Version);
    }
    // Show the CratesIo / Downloads rows when:
    //   - data has landed (real value), OR
    //   - the project is publishable AND the crates.io service is confirmed unreachable
    //     (placeholder in warning color, so the user knows why the field is empty). When the
    //     service recovers the placeholder hides again until a fetch lands.
    let show_unreachable_placeholder = data.publish_status.is_publishable()
        && matches!(data.crates_io_service, ServiceStatus::Unreachable);
    if data.crates_version.is_some() || show_unreachable_placeholder {
        fields.push(DetailField::CratesIo);
    }
    if data.crates_downloads.is_some() || show_unreachable_placeholder {
        fields.push(DetailField::Downloads);
    }
    // Step 4 fields: show unconditionally on Rust packages so that
    // unset values render as `—` and the UI surface matches the
    // manifest faithfully even before metadata arrives.
    if data.has_package {
        fields.push(DetailField::Edition);
        fields.push(DetailField::License);
        fields.push(DetailField::Homepage);
        fields.push(DetailField::Repository);
    }
    fields
}

pub fn package_rows_from_data(data: &PackageData) -> Vec<PackageRow> {
    let fields = package_fields_from_data(data);
    let mut rows = vec![PackageRow::Description];
    let Some(summary) = data.worktree_group_summary.as_ref() else {
        rows.extend(fields.into_iter().map(PackageRow::Field));
        rows.extend((0..data.stats_rows.len()).map(PackageRow::Structure));
        return rows;
    };

    rows.extend([
        PackageRow::Section(PackageSection::WorktreeGroupSummary),
        PackageRow::Field(DetailField::Worktrees),
    ]);
    if summary.deleted > 0 {
        rows.push(PackageRow::Field(DetailField::DeletedWorktrees));
    }
    rows.extend([
        PackageRow::Field(DetailField::DiskTotal),
        PackageRow::Field(DetailField::Lint),
        PackageRow::Field(DetailField::Ci),
    ]);
    if let Some(section) = data.primary_section {
        rows.push(PackageRow::Section(section));
    }
    rows.extend(
        fields
            .into_iter()
            .filter(|field| !matches!(field, DetailField::Lint | DetailField::Ci))
            .map(PackageRow::Field),
    );
    rows.extend((0..data.stats_rows.len()).map(PackageRow::Structure));
    rows
}

pub fn package_field_at(data: &PackageData, pos: usize) -> Option<DetailField> {
    package_rows_from_data(data)
        .get(pos)
        .and_then(|row| match row {
            PackageRow::Description | PackageRow::Section(_) | PackageRow::Structure(_) => None,
            PackageRow::Field(field) => Some(*field),
        })
}

pub const fn package_row_is_selectable(row: &PackageRow) -> bool {
    matches!(
        row,
        PackageRow::Description | PackageRow::Field(_) | PackageRow::Structure(_)
    )
}

pub fn package_first_selectable_row(rows: &[PackageRow]) -> Option<usize> {
    rows.iter().position(package_row_is_selectable)
}

pub fn package_last_selectable_row(rows: &[PackageRow]) -> Option<usize> {
    rows.iter().rposition(package_row_is_selectable)
}

pub fn package_selectable_row_at_or_after(rows: &[PackageRow], pos: usize) -> Option<usize> {
    rows.iter()
        .enumerate()
        .skip(pos.min(rows.len()))
        .find_map(|(index, row)| package_row_is_selectable(row).then_some(index))
}

pub fn package_selectable_row_at_or_before(rows: &[PackageRow], pos: usize) -> Option<usize> {
    rows.iter()
        .enumerate()
        .take(pos.saturating_add(1).min(rows.len()))
        .rev()
        .find_map(|(index, row)| package_row_is_selectable(row).then_some(index))
}

pub fn package_nearest_selectable_row(rows: &[PackageRow], pos: usize) -> Option<usize> {
    package_selectable_row_at_or_after(rows, pos)
        .or_else(|| package_selectable_row_at_or_before(rows, pos))
}

pub fn git_fields_from_data(data: &GitData) -> Vec<DetailField> {
    let mut fields = Vec::new();
    if data.head.is_some() {
        fields.push(DetailField::Head);
    }
    if let Some(ctx) = data.submodule_ctx.as_ref() {
        if ctx.tracks.is_some() {
            fields.push(DetailField::Tracks);
        }
        fields.push(DetailField::Pinned);
    }
    if data.status.is_some() {
        fields.push(DetailField::GitStatus);
    }
    if data.vs_local.is_some() {
        fields.push(DetailField::VsLocal);
    }
    // Show the Stars row when:
    //   - the count has landed (real value), OR
    //   - GitHub is confirmed unreachable / rate-limited (placeholder in warning color, mirrors the
    //     crates.io unreachable row behavior on the Package pane).
    if data.stars.is_some() || github_stars_is_unreachable_placeholder(data) {
        fields.push(DetailField::Stars);
    }
    // Repo description is rendered separately in the About section by
    // `render_git_about_section`, so it is intentionally not a flat field.
    if data.inception.is_some() {
        fields.push(DetailField::Inception);
    }
    if data.last_commit.is_some() {
        fields.push(DetailField::LastCommit);
    }
    if data.last_fetched.is_some() {
        fields.push(DetailField::LastFetched);
    }
    // Rate-limit rows are always shown so the section structure stays
    // stable across fetch state; rendering handles the empty-quota
    // case.
    fields.push(DetailField::RateLimitCore);
    fields.push(DetailField::RateLimitGraphQl);
    if !data.worktrees.is_empty() {
        // Worktree count is appended by the render function, not as a field.
    }
    fields
}

/// Whether a project is publishable to crates.io. Drives whether the
/// `CratesIo` / Downloads rows ever appear in the Package pane —
/// non-publishable projects never trigger a fetch, so showing
/// placeholder rows for them during an outage would be misleading.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PublishStatus {
    Publishable,
    #[default]
    NotPublishable,
}

impl PublishStatus {
    pub const fn is_publishable(self) -> bool { matches!(self, Self::Publishable) }
}

/// Per-pane data for the Package detail panel.
#[derive(Clone, Default)]
/// Per-project pane data the Package pane renders. The "value"
/// fields are pre-resolved display strings — callers can render
/// without `&App` access. Pre-resolving `lint_display` and `ci_display`
/// lets `PackagePane::render` operate on `&PackageData` alone.
pub struct PackageData {
    pub package_title:            String,
    pub title_name:               String,
    pub worktree_group_summary:   Option<WorktreeGroupSummary>,
    pub primary_section:          Option<PackageSection>,
    pub path:                     String,
    pub version:                  String,
    pub description:              Option<String>,
    pub crates_version:           Option<String>,
    pub crates_downloads:         Option<u64>,
    /// Whether this project would have fired a crates.io fetch.
    /// Combined with `crates_io_service`, drives the "crates.io
    /// unreachable" placeholder row that keeps the field visible
    /// during an outage. Non-publishable rows never show the row.
    pub publish_status:           PublishStatus,
    /// Snapshot of the crates.io service's render-side availability.
    /// Set at build time from the live availability state. While
    /// `Unreachable`, publishable rows without a version yet render
    /// the warning placeholder; once it flips back to `Available`,
    /// the placeholder disappears (the row hides until a fetch lands).
    pub crates_io_service:        ServiceStatus,
    pub types:                    String,
    pub disk:                     String,
    pub stats_rows:               Vec<(&'static str, usize)>,
    pub has_package:              bool,
    /// Cargo edition ("2021", "2024", …) from the workspace metadata.
    /// `None` until metadata has landed or for non-Rust projects.
    pub edition:                  Option<String>,
    pub license:                  Option<String>,
    pub homepage:                 Option<String>,
    pub repository:               Option<String>,
    /// Bytes under the project root inside any `target/` subtree.
    /// `None` until the walker has reported a breakdown.
    pub in_project_target:        Option<u64>,
    /// Everything else under the project root (source, docs, .git,
    /// vendored crates outside target, etc.).
    pub in_project_non_target:    Option<u64>,
    /// Typed display value for the Lint field; populated at
    /// assembly time so render can read it without `&App`. The
    /// renderer matches on variants and applies
    /// `animation_elapsed` to `status.icon()` at render time.
    pub lint_display:             super::LintDisplay,
    /// Typed display value for the Ci row in the Package detail
    /// pane. Renderer matches on variants directly. Domain
    /// authority lives on [`crate::tui::state::Ci`]; produced
    /// by `Ci::package_display`.
    pub ci_display:               super::CiDisplay,
    /// Byte size of the workspace's out-of-tree `target_directory`
    /// (when the resolved target sits outside `workspace_root`). Flows
    /// from `WorkspaceMetadata::out_of_tree_target_bytes` once the
    /// cached walk reports back; `None` for in-tree targets or before
    /// the walk lands.
    pub out_of_tree_target_bytes: Option<u64>,
}

/// Resolve (version, description) for the detail pane from the
/// authoritative metadata. Returns `("-", None)` pre-metadata — matches
/// the Targets pane's pre-metadata placeholder UX.
fn version_and_description(pkg: Option<&PackageRecord>) -> (String, Option<String>) {
    let version = pkg.map_or_else(|| "-".to_string(), |p| p.version.to_string());
    let description = pkg.and_then(|p| p.description.clone());
    (version, description)
}

/// Resolve the sharer target size for the row at `abs_path` — i.e. the
/// workspace's cached walk of its out-of-tree `target_directory`. Returns
/// `None` for in-tree targets (already reflected in `DiskTarget`) or
/// before the walk has landed.
fn lookup_out_of_tree_target_bytes(app: &App, abs_path: &AbsolutePath) -> Option<u64> {
    let store = app.scan.metadata_store_handle();
    let guard = store.lock().ok()?;
    let snap = guard
        .containing_workspace_root(abs_path)
        .and_then(|root| guard.get(root))?;
    let is_in_tree = snap
        .target_directory
        .as_path()
        .starts_with(snap.workspace_root.as_path());
    let bytes = (!is_in_tree).then_some(snap.out_of_tree_target_bytes)?;
    drop(guard);
    bytes
}

/// Render "—" when a `PackageRecord` field is absent from the manifest.
fn or_dash(value: Option<&str>) -> String {
    value
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map_or_else(|| "—".to_string(), String::from)
}

/// Per-pane data for the Git detail panel.
#[derive(Clone, Default)]
pub struct GitData {
    pub head:               Option<HeadState>,
    pub status:             Option<GitStatus>,
    pub vs_local:           Option<String>,
    pub local_main_branch:  Option<String>,
    pub stars:              Option<u64>,
    pub description:        Option<String>,
    pub inception:          Option<String>,
    pub last_commit:        Option<String>,
    pub last_fetched:       Option<String>,
    pub rate_limit_core:    Option<RateLimitQuota>,
    pub rate_limit_graphql: Option<RateLimitQuota>,
    pub github_status:      AvailabilityStatus,
    pub remotes:            Vec<RemoteRow>,
    pub worktrees:          Vec<WorktreeInfo>,
    /// Submodule-specific overlay. `Some` only when this `GitData` is
    /// built for a submodule pane — the renderer reads this to decide
    /// whether to emit the `Tracks` / `Pinned` rows. Submodule identity
    /// is conveyed by the project-list `(s)` marker and the pane's
    /// "Submodule — <name>" title, not by an About-section line.
    pub submodule_ctx:      Option<SubmoduleContext>,
}

/// Submodule-only render context: facts the parent repo provides about
/// the submodule that no normal repo has.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubmoduleContext {
    /// Tracking branch from `.gitmodules` (the `branch =` line). `None`
    /// when `.gitmodules` doesn't specify one.
    pub tracks:        Option<String>,
    /// Pinned commit SHA from `git ls-tree HEAD` in the parent repo.
    /// Always present when `SubmoduleContext` is built — without it
    /// there's no reason to render the overlay.
    pub pinned_commit: String,
}

impl GitData {
    /// Whether the repo has no remotes — drives the `(📁 local)` branch
    /// annotation in the git pane.
    pub const fn is_local(&self) -> bool { self.remotes.is_empty() }
}

/// Per-remote row rendered in the Git pane's Remotes table. Pre-formatted
/// for display — status and `tracked_ref` already reduce to rendered text.
#[derive(Clone)]
pub struct RemoteRow {
    pub name:            String,
    pub icon:            &'static str,
    pub display_url:     String,
    pub tracked_ref:     String,
    pub status:          String,
    pub full_url:        Option<String>,
    /// Pre-formatted push-disabled annotation (e.g. `"↛ push disabled"`
    /// or `"↛ push disabled (DISABLED)"`). `None` when push is enabled.
    pub push_annotation: Option<String>,
}

/// Per-worktree info rendered in the Git pane's Worktrees table.
///
/// `ahead_behind` is relative to the primary worktree's HEAD commit.
#[derive(Clone)]
pub struct WorktreeInfo {
    pub name:         String,
    pub path:         String,
    pub branch:       Option<String>,
    pub ahead_behind: Option<(usize, usize)>,
}

/// What the Git pane cursor selects at a given `pos()`. Single source of
/// truth shared by the renderer and the Enter-key handler so neither side
/// can drift from the other's row layout.
#[allow(
    dead_code,
    reason = "Field/Worktree payloads exist for exhaustiveness; callers may match only Remote"
)]
pub enum GitRow<'a> {
    Description(&'a str),
    Field(DetailField),
    Remote(&'a RemoteRow),
    Worktree(&'a WorktreeInfo),
}

pub fn git_has_description_row(data: &GitData) -> bool {
    data.description
        .as_deref()
        .map(str::trim)
        .is_some_and(|description| !description.is_empty())
}

pub fn git_row_at(data: &GitData, pos: usize) -> Option<GitRow<'_>> {
    let description_rows = usize::from(git_has_description_row(data));
    if description_rows > 0 && pos == 0 {
        return data.description.as_deref().map(GitRow::Description);
    }
    let pos = pos.checked_sub(description_rows)?;
    let fields = git_fields_from_data(data);
    let flat_len = fields.len();
    if pos < flat_len {
        return fields.get(pos).copied().map(GitRow::Field);
    }
    let pos = pos - flat_len;
    if pos < data.remotes.len() {
        return data.remotes.get(pos).map(GitRow::Remote);
    }
    let pos = pos - data.remotes.len();
    data.worktrees.get(pos).map(GitRow::Worktree)
}

fn copyable_text(text: impl Into<String>) -> Option<String> {
    let text = text.into();
    let trimmed = text.trim();
    if trimmed.is_empty() || matches!(trimmed, "-" | "—") {
        None
    } else {
        Some(text)
    }
}

fn copy_payload(text: impl Into<String>, label: CopyLabel) -> CopySelectionResult {
    copyable_text(text).map_or(CopySelectionResult::Nothing, |text| {
        CopySelectionResult::Payload(CopyPayload::new(text, label))
    })
}

pub fn copy_payload_for_package(data: &PackageData, pos: usize) -> CopySelectionResult {
    let Some(row) = package_rows_from_data(data).get(pos).copied() else {
        return CopySelectionResult::Nothing;
    };
    let PackageRow::Field(field) = row else {
        return match row {
            PackageRow::Description => copy_payload(
                data.description.as_deref().unwrap_or_default(),
                CopyLabel::Value,
            ),
            PackageRow::Structure(index) => {
                let Some((label, count)) = data.stats_rows.get(index) else {
                    return CopySelectionResult::Nothing;
                };
                copy_payload(format!("{count} {label}"), CopyLabel::Value)
            },
            PackageRow::Section(_) | PackageRow::Field(_) => CopySelectionResult::Nothing,
        };
    };
    match field {
        DetailField::Lint | DetailField::Ci => CopySelectionResult::Nothing,
        DetailField::CratesIo => {
            if data.title_name.trim().is_empty() || data.title_name == "-" {
                CopySelectionResult::Nothing
            } else {
                copy_payload(
                    format!("https://crates.io/crates/{}", data.title_name),
                    CopyLabel::Url,
                )
            }
        },
        DetailField::Path | DetailField::GitStatus => {
            copy_payload(field.package_value(data), CopyLabel::Path)
        },
        DetailField::Homepage | DetailField::Repository => {
            copy_payload(field.package_value(data), CopyLabel::Url)
        },
        _ => copy_payload(field.package_value(data), CopyLabel::Value),
    }
}

pub fn copy_payload_for_git(data: &GitData, pos: usize) -> CopySelectionResult {
    match git_row_at(data, pos) {
        Some(GitRow::Description(description)) => copy_payload(description, CopyLabel::Value),
        Some(GitRow::Field(field)) => {
            copy_payload(git_field_copy_value(data, field), CopyLabel::Value)
        },
        Some(GitRow::Remote(remote)) => copy_payload(
            remote
                .full_url
                .as_deref()
                .unwrap_or(remote.display_url.as_str()),
            CopyLabel::Url,
        ),
        Some(GitRow::Worktree(worktree)) => copy_payload(&worktree.path, CopyLabel::Path),
        None => CopySelectionResult::Nothing,
    }
}

fn git_field_copy_value(data: &GitData, field: DetailField) -> String {
    match field {
        DetailField::Head => match data.head.as_ref() {
            Some(HeadState::Branch(name)) => name.clone(),
            Some(HeadState::Detached { short_sha }) => short_sha.clone(),
            Some(HeadState::Unborn) | None => String::new(),
        },
        DetailField::GitStatus => data
            .status
            .map_or_else(String::new, GitStatus::label_with_icon),
        DetailField::Tracks => data
            .submodule_ctx
            .as_ref()
            .and_then(|context| context.tracks.clone())
            .unwrap_or_default(),
        DetailField::Pinned => data
            .submodule_ctx
            .as_ref()
            .map(|context| context.pinned_commit.clone())
            .unwrap_or_default(),
        _ => field.git_value(data),
    }
}

pub fn copy_payload_for_ci(data: &CiData, pos: usize) -> CopySelectionResult {
    let Some(run) = data.runs.get(pos) else {
        return CopySelectionResult::Nothing;
    };
    copy_payload(&run.url, CopyLabel::Url)
}

pub fn copy_payload_for_targets(
    data: &TargetsData,
    pos: usize,
    running_for: &dyn Fn(&TargetEntry) -> bool,
) -> CopySelectionResult {
    let entries = build_target_list_from_data(data, running_for);
    let Some(entry) = entries.get(pos) else {
        return CopySelectionResult::Nothing;
    };
    copy_payload(entry.src_path.display().to_string(), CopyLabel::Path)
}

pub fn copy_payload_for_lints(
    data: &LintsData,
    pos: usize,
    project_root: &Path,
) -> CopySelectionResult {
    let Some(run) = data.runs.get(pos) else {
        return CopySelectionResult::Nothing;
    };
    let Some(command) = run.commands.first() else {
        return CopySelectionResult::Nothing;
    };
    copy_payload(
        lint::project_dir(project_root)
            .join(&command.log_file)
            .display()
            .to_string(),
        CopyLabel::Path,
    )
}

/// Per-pane data for the Targets panel. Each kind list is pre-sorted by
/// (source bucket, then category for examples, then name). Source
/// tagging lets the renderer expose a per-row origin column and lets
/// `cargo` invocations pass `--package <name>` for member-owned
/// targets.
#[derive(Clone, Default)]
pub struct TargetsData {
    pub binaries: Vec<TargetEntry>,
    pub examples: Vec<TargetEntry>,
    pub benches:  Vec<TargetEntry>,
}

impl TargetsData {
    pub const fn has_targets(&self) -> bool {
        !self.binaries.is_empty() || !self.examples.is_empty() || !self.benches.is_empty()
    }

    /// Aggregate runnable targets for the project at `selected_path`.
    ///
    /// When `selected_path` is the workspace root, every package's
    /// targets across the workspace are included. When it's any
    /// other path (a workspace member), only that package's targets
    /// appear — selecting a member narrows the view to that member's
    /// own targets.
    ///
    /// Per included package: lift the bin target whose name matches
    /// the package name (cargo's "default-run" convention) as a
    /// `Binary` entry; every `Example` target becomes an entry with
    /// category derived from `examples/<category>/<file>.rs`; every
    /// `Bench` becomes a flat entry. Each entry's [`TargetSource`]
    /// is `Workspace` only when the metadata describes a real
    /// multi-package workspace AND the owning package's manifest
    /// sits at the workspace root. Standalone packages (cargo's
    /// implicit single-package workspace) always get
    /// `Member(<package name>)` so the Source column shows the
    /// package name, not the misleading word "workspace".
    pub fn from_workspace_metadata(
        metadata: &WorkspaceMetadata,
        selected_path: &AbsolutePath,
    ) -> Self {
        let workspace_root = metadata.workspace_root.as_path();
        let selected_path = selected_path.as_path();
        let include_all_members = selected_path == workspace_root;
        let is_real_workspace = metadata.packages.len() > 1;
        let mut binaries: Vec<TargetEntry> = Vec::new();
        let mut examples: Vec<TargetEntry> = Vec::new();
        let mut benches: Vec<TargetEntry> = Vec::new();

        for record in metadata.packages.values() {
            let manifest_dir = record.manifest_path.as_path().parent();
            if !include_all_members && manifest_dir != Some(selected_path) {
                continue;
            }
            let source = if is_real_workspace && manifest_dir == Some(workspace_root) {
                TargetSource::Workspace
            } else {
                TargetSource::Member(record.name.clone())
            };

            for target in &record.targets {
                if target.kinds.contains(&TargetKind::Bin) && target.name == record.name {
                    binaries.push(TargetEntry {
                        name:         target.name.clone(),
                        display_name: target.name.clone(),
                        kind:         RunTargetKind::Binary,
                        source:       source.clone(),
                        src_path:     target.src_path.clone(),
                    });
                }
                if target.kinds.contains(&TargetKind::Example) {
                    let category = example_category(manifest_dir, target.src_path.as_path());
                    let display_name = if category.is_empty() {
                        target.name.clone()
                    } else {
                        format!("{category}/{}", target.name)
                    };
                    examples.push(TargetEntry {
                        name: target.name.clone(),
                        display_name,
                        kind: RunTargetKind::Example,
                        source: source.clone(),
                        src_path: target.src_path.clone(),
                    });
                }
                if target.kinds.contains(&TargetKind::Bench) {
                    benches.push(TargetEntry {
                        name:         target.name.clone(),
                        display_name: target.name.clone(),
                        kind:         RunTargetKind::Bench,
                        source:       source.clone(),
                        src_path:     target.src_path.clone(),
                    });
                }
            }
        }

        binaries.sort_by(|a, b| {
            a.source
                .sort_key()
                .cmp(&b.source.sort_key())
                .then_with(|| a.name.cmp(&b.name))
        });
        examples.sort_by(|a, b| {
            a.source
                .sort_key()
                .cmp(&b.source.sort_key())
                .then_with(|| example_display_order(&a.display_name, &b.display_name))
        });
        benches.sort_by(|a, b| {
            a.source
                .sort_key()
                .cmp(&b.source.sort_key())
                .then_with(|| a.name.cmp(&b.name))
        });

        Self {
            binaries,
            examples,
            benches,
        }
    }
}

/// Derive the example's category subdirectory from its `src_path`
/// relative to its package's manifest dir. `examples/<file>.rs` is
/// root-level (empty); `examples/<category>/<file>.rs` is categorized.
fn example_category(manifest_dir: Option<&Path>, src_path: &Path) -> String {
    manifest_dir
        .and_then(|dir| src_path.strip_prefix(dir).ok())
        .and_then(|rel| {
            let parts: Vec<_> = rel
                .components()
                .filter_map(|c| match c {
                    Component::Normal(seg) => Some(seg.to_string_lossy().into_owned()),
                    _ => None,
                })
                .collect();
            if parts.len() >= 3 {
                Some(parts[1].clone())
            } else {
                None
            }
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod target_list_tests {
    use super::*;

    fn entry(name: &str, kind: RunTargetKind) -> TargetEntry {
        TargetEntry {
            name: name.into(),
            display_name: name.into(),
            kind,
            source: TargetSource::Workspace,
            src_path: AbsolutePath::from(format!("/tmp/{name}.rs")),
        }
    }

    fn data() -> TargetsData {
        TargetsData {
            binaries: vec![
                entry("a", RunTargetKind::Binary),
                entry("b", RunTargetKind::Binary),
                entry("c", RunTargetKind::Binary),
            ],
            examples: vec![entry("ex1", RunTargetKind::Example)],
            benches:  vec![entry("bn1", RunTargetKind::Bench)],
        }
    }

    #[test]
    fn running_binary_floats_to_top_of_its_section() {
        let data = data();
        let entries = build_target_list_from_data(&data, &|e| e.name == "b");
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["b", "a", "c", "ex1", "bn1"]);
    }

    #[test]
    fn empty_running_set_preserves_input_order() {
        let data = data();
        let entries = build_target_list_from_data(&data, &|_| false);
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["a", "b", "c", "ex1", "bn1"]);
    }

    #[test]
    fn all_running_preserves_input_order() {
        let data = data();
        let entries = build_target_list_from_data(&data, &|_| true);
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["a", "b", "c", "ex1", "bn1"]);
    }
}

/// Within an examples section, sort root-level (no `/`) before
/// categorized, then alphabetically by display name. Matches the
/// Bevy-style listing convention preserved across the workspace
/// aggregation.
fn example_display_order(a: &str, b: &str) -> Ordering {
    let a_root = !a.contains('/');
    let b_root = !b.contains('/');
    match (a_root, b_root) {
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        _ => a.cmp(b),
    }
}

#[derive(Clone)]
pub enum CiEmptyState {
    BranchScopedOnly,
    Fetching,
    Loading,
    NoRuns,
    NoRunsForBranch(String),
    NoRunsForUnpublishedBranch(String),
    NoWorkflowConfigured,
    NotGitRepo,
    RequiresGithubRemote,
}

impl CiEmptyState {
    pub fn title(&self) -> String {
        match self {
            Self::BranchScopedOnly => " CI Runs — shown on branch/worktree rows ".to_string(),
            Self::Fetching | Self::Loading => " CI Runs — loading… ".to_string(),
            Self::NoRuns => " No CI Runs ".to_string(),
            Self::NoRunsForBranch(branch) => format!(" No CI runs for branch {branch} "),
            Self::NoRunsForUnpublishedBranch(branch) => {
                format!(" No CI runs for unpublished branch {branch} ")
            },
            Self::NoWorkflowConfigured => " No CI workflow configured ".to_string(),
            Self::NotGitRepo => " CI Runs — not a git repository ".to_string(),
            Self::RequiresGithubRemote => " CI Runs — requires a GitHub origin remote ".to_string(),
        }
    }
}

#[derive(Clone)]
pub struct CiData {
    pub runs:           Vec<CiRun>,
    pub mode_label:     Option<String>,
    pub current_branch: Option<String>,
    pub empty_state:    CiEmptyState,
}

impl CiData {
    pub const fn has_runs(&self) -> bool { !self.runs.is_empty() }
}

#[derive(Clone, Default)]
pub struct LintsData {
    pub runs:    Vec<LintRun>,
    /// Archive-directory size in bytes for each run, aligned by index with
    /// `runs`.
    /// Per-run archive size aligned with `runs`. `None` means the run has
    /// no archive entry yet; `Some(0)` means the archive exists and is
    /// empty. The renderer renders `None` as "—" and `Some(_)` as a byte
    /// count, distinguishing missing data from known-empty.
    pub sizes:   Vec<Option<u64>>,
    pub is_rust: bool,
}

impl LintsData {
    pub const fn has_runs(&self) -> bool { !self.runs.is_empty() }
}

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

struct GitDetailFields {
    head:               Option<HeadState>,
    path:               Option<GitStatus>,
    vs_local:           Option<String>,
    local_main_branch:  Option<String>,
    stars:              Option<u64>,
    description:        Option<String>,
    inception:          Option<String>,
    last_commit:        Option<String>,
    last_fetched:       Option<String>,
    rate_limit_core:    Option<RateLimitQuota>,
    rate_limit_graphql: Option<RateLimitQuota>,
    github_status:      AvailabilityStatus,
    remotes:            Vec<RemoteRow>,
}

fn build_git_detail_fields(app: &App, abs_path: &Path) -> GitDetailFields {
    let git_repo = app.project_list.git_repo_for(abs_path);
    let repo_info = git_repo.and_then(|repo| repo.repo_info.as_ref());
    let checkout = app.project_list.git_info_for(abs_path);

    let head = checkout.map(|info| info.head.clone());
    let local_main_branch = repo_info.and_then(|repo| repo.local_main_branch.clone());
    let local_main_label = local_main_branch
        .as_deref()
        .unwrap_or_else(|| app.config.current().tui.main_branch.as_str());
    let vs_local = checkout
        .and_then(|info| info.ahead_behind_local)
        .map(|ahead_behind| format_ahead_behind_against(ahead_behind, local_main_label));
    let github = git_repo.and_then(|repo| repo.github_info.as_ref());
    let stars = github.map(|g| g.stars);
    let description = github.and_then(|g| g.description.clone());
    let inception = repo_info
        .and_then(|repo| repo.first_commit.as_deref())
        .map(format_timestamp);
    let last_commit = checkout
        .and_then(|info| info.last_commit.as_deref())
        .map(format_timestamp);
    let last_fetched = repo_info
        .and_then(|repo| repo.last_fetched.as_deref())
        .map(format_timestamp);
    let default_host = app.config.current().tui.default_remote_host_url.clone();
    let remotes = repo_info.map_or_else(Vec::new, |repo| build_remote_rows(repo, &default_host));
    let rate_limit = app.net.rate_limit();
    GitDetailFields {
        head,
        path: app.project_list.git_status_for(abs_path),
        vs_local,
        local_main_branch,
        stars,
        description,
        inception,
        last_commit,
        last_fetched,
        rate_limit_core: rate_limit.core,
        rate_limit_graphql: rate_limit.graphql,
        github_status: app.net.github_status(),
        remotes,
    }
}

/// Convert each `RemoteInfo` into a render-ready `RemoteRow`, shortening
/// the URL when it begins with `default_host` and collapsing missing
/// tracked refs / ahead-behind values to placeholder runes.
fn build_remote_rows(repo: &RepoInfo, default_host: &str) -> Vec<RemoteRow> {
    repo.remotes
        .iter()
        .map(|remote| {
            let icon = match remote.kind {
                RemoteKind::Fork => GIT_FORK,
                RemoteKind::Clone => GIT_CLONE,
            };
            let display_url = remote
                .url
                .as_deref()
                .map_or_else(String::new, |raw| shorten_remote_url(raw, default_host));
            let tracked_ref = remote
                .tracked_ref
                .clone()
                .unwrap_or_else(|| NO_REMOTE_SYNC.to_string());
            let status = format_remote_status(remote.ahead_behind);
            let push_annotation = format_push_annotation(&remote.push);
            RemoteRow {
                name: remote.name.clone(),
                icon,
                display_url,
                tracked_ref,
                status,
                full_url: remote.url.clone(),
                push_annotation,
            }
        })
        .collect()
}

/// Pre-format the `↛ push disabled` annotation rendered after the
/// status column in the Remotes table. Returns `None` for enabled
/// remotes — rendering then leaves the slot empty.
fn format_push_annotation(push: &PushState) -> Option<String> {
    let PushState::Disabled { reason } = push else {
        return None;
    };
    let suffix = match reason {
        PushDisabledReason::KnownSentinel(s) => Some(s.label()),
        PushDisabledReason::NoPushUrl => None,
    };
    Some(suffix.map_or_else(
        || "\u{21A0} push disabled".to_string(),
        |label| format!("\u{21A0} push disabled ({label})"),
    ))
}

/// If `url` starts with `default_host`, return `owner/repo` (stripping
/// `.git` suffix); otherwise return the full URL.
fn shorten_remote_url(url: &str, default_host: &str) -> String {
    let stripped = url.strip_prefix(default_host).unwrap_or(url);
    stripped
        .strip_suffix(GIT_DIR)
        .unwrap_or(stripped)
        .to_string()
}

/// Check whether a `RootItem` currently renders as a worktree group.
fn is_worktree_group(item: &RootItem) -> bool {
    matches!(item, RootItem::Worktrees(group) if group.renders_as_group())
}

/// Collect worktree info from a worktree group item.
///
/// Branch is read from cached `CheckoutInfo` populated by the watcher (no
/// shell-out). Ahead/behind is computed via git shell-out — this is the
/// expensive part — and is the reason the caller wraps this in
/// `App::worktree_summary_or_compute` so each `(group, data_generation)`
/// pair pays at most once.
fn worktrees_from_item(app: &App, item: &RootItem) -> Vec<WorktreeInfo> {
    let (paths_and_names, primary_path) = match item {
        RootItem::Worktrees(group) => {
            let primary_path = group.primary.path().clone();
            let entries: Vec<(AbsolutePath, String)> = group
                .iter_entries()
                .filter(|p| p.visibility() != Visibility::Dismissed)
                .map(|p| (p.path().clone(), p.root_directory_name().into_string()))
                .collect();
            (entries, primary_path)
        },
        _ => return Vec::new(),
    };

    paths_and_names
        .into_iter()
        .map(|(path, name)| {
            let branch = app
                .project_list
                .git_info_for(path.as_path())
                .and_then(|info| info.head.branch_name().map(str::to_string));
            let ahead_behind = if path.as_path() == primary_path.as_path() {
                Some((0, 0))
            } else {
                project::worktree_ahead_behind_primary(path.as_path(), primary_path.as_path())
            };
            WorktreeInfo {
                name,
                path: path.display().to_string(),
                branch,
                ahead_behind,
            }
        })
        .collect()
}

/// Build pane data for a root `RootItem`.
pub fn build_pane_data(app: &App, item: &RootItem) -> DetailPaneData {
    let display_path = item.display_path().into_string();
    let is_wt_group = is_worktree_group(item);

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

    let mut counts = ProjectCounts::default();
    counts.add_cargo(cargo);
    let stats_rows = counts.to_rows();

    build_pane_data_common(
        app,
        PaneDataSource {
            abs_path,
            display_path: &display_path,
            title_name: vendored.package_name().into_string(),
            has_cargo: true,
            cargo: Some(cargo),
            wt_item: None,
            stats_rows,
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
    let git_detail = build_git_detail_fields(app, abs_path);

    let version = submodule.commit.as_deref().unwrap_or("-").to_string();
    let disk = submodule
        .info
        .disk_usage_bytes
        .map_or_else(String::new, render::format_bytes);

    let submodule_ctx = build_submodule_context(submodule);

    DetailPaneData {
        package: PackageData {
            package_title: "Submodule".to_string(),
            title_name: submodule.name.clone(),
            worktree_group_summary: None,
            primary_section: None,
            path: display_path,
            version,
            description: submodule.url.clone(),
            crates_version: None,
            crates_downloads: None,
            publish_status: PublishStatus::NotPublishable,
            crates_io_service: ServiceStatus::Available,
            types: String::new(),
            disk,
            stats_rows: Vec::new(),
            has_package: false,
            edition: None,
            license: None,
            homepage: None,
            repository: None,
            in_project_target: None,
            in_project_non_target: None,
            out_of_tree_target_bytes: None,
            // Submodules don't render the Lint/Ci fields; the
            // `package_fields_from_data` filter excludes them when
            // there's no Cargo manifest. Default values are safe.
            lint_display: super::LintDisplay::default(),
            ci_display: super::CiDisplay::default(),
        },
        git:     GitData {
            head: git_detail.head,
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

    let mut counts = ProjectCounts::default();
    counts.add_workspace(ws);
    if ws.has_members() {
        for group in ws.groups() {
            for member in group.members() {
                counts.add_package(member);
            }
        }
    }
    let stats_rows = counts.to_rows();

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
            has_cargo: ws.name().is_some(),
            cargo: Some(cargo),
            wt_item: wt_item_ref,
            stats_rows,
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

    let mut counts = ProjectCounts::default();
    counts.add_package(pkg);
    let stats_rows = counts.to_rows();

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
            has_cargo: true,
            cargo: Some(cargo),
            wt_item: wt_item_ref,
            stats_rows,
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

    build_pane_data_common(
        app,
        PaneDataSource {
            abs_path,
            display_path,
            title_name: nr.root_directory_name().into_string(),
            has_cargo: false,
            cargo: None,
            wt_item: wt_item_ref,
            stats_rows: Vec::new(),
            primary_section: None,
            fallback_type: None,
            package_title: "Project".to_string(),
        },
    )
}

struct PaneDataSource<'a> {
    abs_path:        &'a Path,
    display_path:    &'a str,
    title_name:      String,
    has_cargo:       bool,
    cargo:           Option<&'a Cargo>,
    wt_item:         Option<&'a RootItem>,
    stats_rows:      Vec<(&'static str, usize)>,
    primary_section: Option<PackageSection>,
    fallback_type:   Option<ProjectType>,
    package_title:   String,
}

/// Crates-io fields pulled from either a Rust info or vendored entry.
struct CratesIoFields {
    version:     Option<String>,
    downloads:   Option<u64>,
    /// True iff this project would have fired a crates.io fetch — i.e.
    /// a publishable package. Used by `package_fields_from_data` to
    /// keep the `CratesIo` placeholder row visible during a crates.io
    /// outage even before any version landed; non-publishable rows
    /// (where no fetch ever runs) never show the row.
    publishable: bool,
}

fn resolve_crates_io_fields(app: &App, abs_path: &Path) -> CratesIoFields {
    let rust_info = app.project_list.rust_info_at_path(abs_path);
    let vendored = app.project_list.vendored_at_path(abs_path);
    let publishable = rust_info.is_some_and(|r| r.cargo.publishable())
        || vendored.is_some_and(|v| v.cargo.publishable() && v.name.is_some());
    CratesIoFields {
        version: rust_info
            .and_then(|r| r.crates_version().map(String::from))
            .or_else(|| vendored.and_then(|v| v.crates_version().map(String::from))),
        downloads: rust_info
            .and_then(RustInfo::crates_downloads)
            .or_else(|| vendored.and_then(VendoredPackage::crates_downloads)),
        publishable,
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
struct ManifestFields {
    edition:     Option<String>,
    license:     Option<String>,
    homepage:    Option<String>,
    repository:  Option<String>,
    version:     String,
    description: Option<String>,
}

fn manifest_fields_from(package_record: Option<&PackageRecord>) -> ManifestFields {
    let (version, description) = version_and_description(package_record);
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
struct BuildPackageDataArgs {
    package_title:            String,
    title_name:               String,
    worktree_group_summary:   Option<WorktreeGroupSummary>,
    primary_section:          Option<PackageSection>,
    display_path:             String,
    stats_rows:               Vec<(&'static str, usize)>,
    has_cargo:                bool,
    manifest:                 ManifestFields,
    crates_version:           Option<String>,
    crates_downloads:         Option<u64>,
    publish_status:           PublishStatus,
    crates_io_service:        ServiceStatus,
    types_str:                String,
    disk:                     String,
    in_project_target:        Option<u64>,
    in_project_non_target:    Option<u64>,
    out_of_tree_target_bytes: Option<u64>,
    lint_display:             super::LintDisplay,
    ci_display:               super::CiDisplay,
}

/// Pure constructor: assemble `PackageData` from already-resolved
/// values. Extracted from `build_pane_data_common` so that
/// orchestrator stays under the line limit.
fn build_package_data(args: BuildPackageDataArgs) -> PackageData {
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
        crates_version: args.crates_version,
        crates_downloads: args.crates_downloads,
        publish_status: args.publish_status,
        crates_io_service: args.crates_io_service,
        types: args.types_str,
        disk: args.disk,
        stats_rows: args.stats_rows,
        has_package: args.has_cargo,
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

fn resolve_worktrees(app: &App, wt_item: Option<&RootItem>) -> Vec<WorktreeInfo> {
    wt_item.map_or_else(Vec::new, |item| {
        app.panes
            .git
            .worktree_summary_or_compute(item.path().as_path(), || worktrees_from_item(app, item))
    })
}

fn worktree_group_summary_for(item: &RootItem, disk: String) -> Option<WorktreeGroupSummary> {
    let RootItem::Worktrees(group) = item else {
        return None;
    };
    Some(WorktreeGroupSummary {
        worktrees: group.visible_entry_count(),
        deleted: group
            .iter_entries()
            .filter(|entry| entry.visibility() == Visibility::Deleted)
            .count(),
        disk,
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
) -> (super::LintDisplay, super::CiDisplay) {
    let pl = &app.project_list;
    let is_worktree_group = package_title == "Worktree Group";
    let is_rust = pl.is_rust_at_path(abs_path.as_path());
    let lint_display = super::Lint::package_display(pl, abs_path, is_worktree_group, is_rust);
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
        worktree_group_summary: runtime.worktree_group_summary,
        primary_section: src.primary_section,
        has_cargo: src.has_cargo,
        manifest: metadata.manifest,
        publish_status: crates_io_status.publish,
        crates_io_service: crates_io_status.service,
        crates_version: runtime.crates_io.version,
        crates_downloads: runtime.crates_io.downloads,
        types_str: metadata.types_str,
        disk: runtime.disk,
        in_project_target: metadata.in_project_target,
        in_project_non_target: metadata.in_project_non_target,
        out_of_tree_target_bytes: metadata.out_of_tree_target_bytes,
        lint_display,
        ci_display,
    });

    let targets = lookup_targets_data(app, &abs_path_owned);
    assemble_detail_pane_data(package, runtime.git_detail, runtime.worktrees, targets)
}

/// Phase 1 output — every field that depends on runtime state
/// (filesystem walks, network probes, in-memory caches). Captures
/// per-piece millisecond timings so the orchestrator can emit a
/// single combined log line without each helper logging
/// independently.
struct RuntimeFields {
    git_detail:             GitDetailFields,
    crates_io:              CratesIoFields,
    disk:                   String,
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
struct MetadataFields {
    types_str:                String,
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
struct CratesIoStatus {
    publish: PublishStatus,
    service: ServiceStatus,
}

/// Phase 1: collect every runtime-derived field (git state, disk
/// usage, CI status, worktree list, crates.io version cache) along
/// with the elapsed time for each block.
fn collect_runtime_fields(app: &App, abs_path: &Path, wt_item: Option<&RootItem>) -> RuntimeFields {
    let t_git = std::time::Instant::now();
    let git_detail = build_git_detail_fields(app, abs_path);
    let git_detail_ms = tui_pane::perf_log_ms(t_git.elapsed().as_millis());

    let crates_io = resolve_crates_io_fields(app, abs_path);

    let t_disk = std::time::Instant::now();
    let disk = super::formatted_disk(&app.project_list, abs_path);
    let worktree_group_summary = wt_item
        .and_then(|item| worktree_group_summary_for(item, super::formatted_disk_for_item(item)));
    let ci = compute_ci_status(app, abs_path, wt_item);
    let disk_ms = tui_pane::perf_log_ms(t_disk.elapsed().as_millis());

    let t_wt = std::time::Instant::now();
    let worktrees = resolve_worktrees(app, wt_item);
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
fn collect_metadata_fields(
    app: &App,
    abs_path: &Path,
    abs_path_owned: &AbsolutePath,
    cargo: Option<&Cargo>,
    fallback_type: Option<ProjectType>,
) -> MetadataFields {
    let types_str = cargo.map_or_else(String::new, |c| {
        c.types()
            .iter()
            .map(std::string::ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    });
    let types_str = if types_str.is_empty() {
        fallback_type.map_or_else(String::new, |project_type| project_type.to_string())
    } else {
        types_str
    };

    let t_meta = std::time::Instant::now();
    let package_record = lookup_package_record(app, abs_path_owned);
    let metadata_ms = tui_pane::perf_log_ms(t_meta.elapsed().as_millis());
    let manifest = manifest_fields_from(package_record.as_ref());

    let (in_project_target, in_project_non_target) =
        compute_in_project_bytes(&app.project_list, abs_path);
    let t_oot = std::time::Instant::now();
    let out_of_tree_target_bytes = lookup_out_of_tree_target_bytes(app, abs_path_owned);
    let oot_ms = tui_pane::perf_log_ms(t_oot.elapsed().as_millis());

    MetadataFields {
        types_str,
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
const fn derive_crates_io_status(crates_io: &CratesIoFields, app: &App) -> CratesIoStatus {
    let publish = if crates_io.publishable {
        PublishStatus::Publishable
    } else {
        PublishStatus::NotPublishable
    };
    let service = if app.net.crates_io.availability.toast_id().is_some() {
        ServiceStatus::Unreachable
    } else {
        ServiceStatus::Available
    };
    CratesIoStatus { publish, service }
}

/// Emit the combined `pane_common_breakdown` perf log line so each
/// helper doesn't log independently — keeping the existing log
/// format intact for downstream tracing consumers.
fn log_pane_common_breakdown(abs_path: &Path, runtime: &RuntimeFields, metadata: &MetadataFields) {
    tracing::info!(
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
fn lookup_targets_data(app: &App, abs_path: &AbsolutePath) -> TargetsData {
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
fn assemble_detail_pane_data(
    package: PackageData,
    git_detail: GitDetailFields,
    worktrees: Vec<WorktreeInfo>,
    targets: TargetsData,
) -> DetailPaneData {
    DetailPaneData {
        package,
        git: GitData {
            head: git_detail.head,
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
            remotes: git_detail.remotes,
            worktrees,
            submodule_ctx: None,
        },
        targets,
    }
}

/// Build the submodule render overlay (`tracks`, `pinned_commit`).
/// Returns `None` when the parent has no pinned commit recorded —
/// without it there's nothing meaningful to render in the overlay.
fn build_submodule_context(submodule: &Submodule) -> Option<SubmoduleContext> {
    let pinned_commit = submodule.commit.clone()?;
    Some(SubmoduleContext {
        tracks: submodule.branch.clone(),
        pinned_commit,
    })
}

pub fn build_ci_data(app: &App) -> CiData {
    let selected_path = app.project_list.selected_project_path();
    let has_ci_owner = app.project_list.selected_ci_path().is_some();
    let git_info = selected_path.and_then(|path| app.project_list.git_info_for(path));
    let repo_info = selected_path.and_then(|path| app.project_list.repo_info_for(path));
    let ci_info = selected_path.and_then(|path| app.project_list.ci_info_for(path));
    let current_branch = selected_path.and_then(|path| {
        app.project_list
            .git_info_for(path)
            .and_then(|git| git.head.branch_name().map(str::to_string))
    });
    let unpublished_branch_name =
        selected_path.and_then(|path| app.project_list.unpublished_ci_branch_name(path));
    let runs = app
        .project_list
        .selected_project_path()
        .map_or_else(Vec::new, |path| {
            app.project_list.ci_runs_for_ci_pane(path, &app.ci)
        });
    let is_fetching = selected_path.is_some_and(|path| app.ci.fetch_tracker.is_fetching(path));
    let branch_filtered_empty = selected_path.is_some_and(|path| {
        app.ci_toggle_available_for(path) && app.ci.display_mode_label_for(path) == "branch"
    }) && ci_info.is_some_and(|info| !info.runs.is_empty())
        && runs.is_empty();
    // "Do we have a GitHub-parseable remote?" is a per-repo question and
    // must not depend on whether the current branch has an upstream — a
    // checkout on a branch without upstream tracking still belongs to
    // the repo.
    let has_github_remote = repo_info.is_some_and(|r| {
        r.remotes
            .iter()
            .filter_map(|r| r.url.as_deref())
            .any(|url| ci::parse_owner_repo(url).is_some())
    });
    let empty_state = if selected_path.is_some() && !has_ci_owner {
        CiEmptyState::BranchScopedOnly
    } else if git_info.is_none() {
        CiEmptyState::NotGitRepo
    } else if has_ci_owner
        && (repo_info.is_none_or(|r| r.origin_kind() == GitOrigin::Local) || !has_github_remote)
    {
        CiEmptyState::RequiresGithubRemote
    } else if repo_info.is_some_and(|r| !r.workflows.is_present()) {
        CiEmptyState::NoWorkflowConfigured
    } else if is_fetching {
        CiEmptyState::Fetching
    } else if ci_info.is_none() || !app.scan.is_complete() {
        CiEmptyState::Loading
    } else if branch_filtered_empty {
        unpublished_branch_name.map_or_else(
            || {
                CiEmptyState::NoRunsForBranch(
                    current_branch
                        .clone()
                        .unwrap_or_else(|| "current".to_string()),
                )
            },
            CiEmptyState::NoRunsForUnpublishedBranch,
        )
    } else {
        CiEmptyState::NoRuns
    };

    CiData {
        runs,
        mode_label: selected_path.and_then(|path| {
            app.ci_toggle_available_for(path)
                .then(|| app.ci.display_mode_label_for(path).to_string())
        }),
        current_branch,
        empty_state,
    }
}

pub fn build_lints_data(app: &App) -> LintsData {
    let selected_path = app.project_list.selected_project_path();
    let lint_runs = selected_path.and_then(|path| {
        app.lint_at_path(path)
            .or_else(|| app.project_list.vendored_owner_lint(path))
    });
    let (runs, sizes) = lint_runs.map_or_else(
        || (Vec::new(), Vec::new()),
        |lr| {
            let sizes: Vec<Option<u64>> = lr
                .runs()
                .iter()
                .map(|run| lr.archive_bytes(&run.run_id))
                .collect();
            (lr.runs().to_vec(), sizes)
        },
    );
    LintsData {
        runs,
        sizes,
        is_rust: selected_path.is_some_and(|path| app.project_list.is_rust_at_path(path)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_limit_bucket_empty_without_quota() {
        assert!(format_rate_limit_bucket(None).is_empty());
    }

    #[test]
    fn rate_limit_bucket_without_reset_omits_countdown() {
        let quota = RateLimitQuota {
            limit:     5000,
            used:      42,
            remaining: 4958,
            reset_at:  None,
        };
        assert_eq!(format_rate_limit_bucket(Some(quota)), "4958/5000");
    }

    #[test]
    fn rate_limit_bucket_fully_unused_omits_countdown() {
        let quota = RateLimitQuota {
            limit:     5000,
            used:      0,
            remaining: 5000,
            reset_at:  Some(u64::MAX),
        };
        assert_eq!(format_rate_limit_bucket(Some(quota)), "5000/5000");
    }

    #[test]
    fn rate_limit_bucket_with_past_reset_renders_zero_countdown() {
        let quota = RateLimitQuota {
            limit:     5000,
            used:      100,
            remaining: 4900,
            reset_at:  Some(0),
        };
        assert_eq!(format_rate_limit_bucket(Some(quota)), "4900/5000 resets 0s");
    }
}
