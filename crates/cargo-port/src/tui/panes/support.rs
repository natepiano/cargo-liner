use std::cmp::Ordering;
use std::path::Component;
use std::path::Path;
use std::process::Command;
use std::sync::OnceLock;

use ratatui::style::Color;

use crate::ci;
use crate::ci::CiRun;
use crate::ci::CiStatus;
use crate::constants::IN_SYNC;
use crate::constants::NO_REMOTE_SYNC;
use crate::constants::SYNC_DOWN;
use crate::constants::SYNC_UP;
use crate::http::RateLimitQuota;
use crate::lint::LintRun;
use crate::perf_log;
use crate::project;
use crate::project::AbsolutePath;
use crate::project::Cargo;
use crate::project::ExampleGroup;
use crate::project::GitOrigin;
use crate::project::GitStatus;
use crate::project::NonRustProject;
use crate::project::Package;
use crate::project::PackageRecord;
use crate::project::ProjectFields;
use crate::project::ProjectType;
use crate::project::RemoteKind;
use crate::project::RepoInfo;
use crate::project::RootItem;
use crate::project::RustInfo;
use crate::project::RustProject;
use crate::project::Submodule;
use crate::project::VendoredPackage;
use crate::project::Workspace;
use crate::tui::app::App;
use crate::tui::app::AvailabilityStatus;
use crate::tui::project_list::ProjectList;
use crate::tui::render;

/// Get the local UTC offset in seconds (e.g., -28800 for PST).
fn local_utc_offset_secs() -> i64 {
    static OFFSET: OnceLock<i64> = OnceLock::new();
    *OFFSET.get_or_init(|| {
        Command::new("date")
            .arg("+%z")
            .output()
            .ok()
            .and_then(|output| {
                let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if value.len() >= 5 {
                    let sign: i64 = if value.starts_with('-') { -1 } else { 1 };
                    let hours: i64 = value[1..3].parse().ok()?;
                    let mins: i64 = value[3..5].parse().ok()?;
                    Some(sign * (hours * 3600 + mins * 60))
                } else {
                    None
                }
            })
            .unwrap_or(0)
    })
}

const fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        2 => {
            if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) {
                29
            } else {
                28
            }
        },
        _ => 30,
    }
}

/// Extract the local date from an ISO 8601 timestamp as `yyyy-mm-dd`.
///
/// If the timestamp has an embedded timezone offset, the date portion is
/// already local and is returned directly. For UTC timestamps, the local
/// offset is applied via `format_timestamp`.
pub fn format_date(iso: &str) -> String {
    let stripped = iso.trim_end_matches('Z');
    if let Some((date, after_t)) = stripped.split_once('T') {
        let has_offset = after_t.rfind(['+', '-']).is_some_and(|p| p > 0);
        if has_offset {
            return date.to_string();
        }
    }
    let full = format_timestamp(iso);
    full.split(' ').next().unwrap_or(&full).to_string()
}

/// Extract the local time portion from an ISO 8601 timestamp as `hh:mm:ss`.
///
/// If the timestamp has an embedded timezone offset (e.g., `-04:00`), the
/// time is already local and no offset is applied. If it ends in `Z` or has
/// no offset, the local UTC offset is applied.
pub fn format_time(iso: &str) -> String {
    let is_utc = iso.ends_with('Z');
    let stripped = iso.trim_end_matches('Z');
    let Some((_, time_and_offset)) = stripped.split_once('T') else {
        return "—".to_string();
    };

    let (time_str, has_embedded_offset) =
        time_and_offset
            .rfind(['+', '-'])
            .map_or((time_and_offset, false), |pos| {
                if pos > 0 {
                    (&time_and_offset[..pos], true)
                } else {
                    (time_and_offset, false)
                }
            });

    let time_parts: Vec<&str> = time_str.split(':').collect();
    if time_parts.len() < 3 {
        return time_str.to_string();
    }
    let (Ok(hour), Ok(minute), Ok(second)) = (
        time_parts[0].parse::<i64>(),
        time_parts[1].parse::<i64>(),
        time_parts[2]
            .split('.')
            .next()
            .unwrap_or("0")
            .parse::<i64>(),
    ) else {
        return time_str.to_string();
    };

    let offset = if has_embedded_offset || !is_utc {
        0
    } else {
        local_utc_offset_secs()
    };

    let total_secs = hour * 3600 + minute * 60 + second + offset;
    let mut adj = total_secs % (24 * 3600);
    if adj < 0 {
        adj += 24 * 3600;
    }
    let h = adj / 3600;
    let m = (adj % 3600) / 60;
    let s = adj % 60;
    format!("{h:02}:{m:02}:{s:02}")
}

/// Format a duration in milliseconds as a compact string.
pub fn format_duration(duration_ms: Option<u64>) -> String {
    let Some(ms) = duration_ms else {
        return "—".to_string();
    };
    let total_secs = ms / 1000;
    let minutes = total_secs / 60;
    let seconds = total_secs % 60;
    if minutes > 0 {
        format!("{minutes}m {seconds:02}s")
    } else {
        format!("{seconds}s")
    }
}

/// Convert a UTC ISO 8601 timestamp to local time, formatted as `yyyy-mm-dd hh:mm`.
pub fn format_timestamp(iso: &str) -> String {
    let utc_offset_secs = local_utc_offset_secs();
    let stripped = iso.trim_end_matches('Z');
    match stripped.split_once('T') {
        Some((date, time)) => {
            let date_parts: Vec<&str> = date.split('-').collect();
            let time_parts: Vec<&str> = time.split(':').collect();
            if date_parts.len() >= 3
                && time_parts.len() >= 2
                && let (Ok(y), Ok(month), Ok(day), Ok(hour), Ok(minute)) = (
                    date_parts[0].parse::<i64>(),
                    date_parts[1].parse::<i64>(),
                    date_parts[2].parse::<i64>(),
                    time_parts[0].parse::<i64>(),
                    time_parts[1].parse::<i64>(),
                )
            {
                let total_mins = hour * 60 + minute + utc_offset_secs / 60;
                let mut day = day;
                let mut month = month;
                let mut year = y;
                let mut adj_mins = total_mins % (24 * 60);
                if adj_mins < 0 {
                    adj_mins += 24 * 60;
                    day -= 1;
                    if day < 1 {
                        month -= 1;
                        if month < 1 {
                            month = 12;
                            year -= 1;
                        }
                        day = days_in_month(year, month);
                    }
                } else if adj_mins >= 24 * 60 {
                    adj_mins -= 24 * 60;
                    day += 1;
                    if day > days_in_month(year, month) {
                        day = 1;
                        month += 1;
                        if month > 12 {
                            month = 1;
                            year += 1;
                        }
                    }
                }
                let local_h = adj_mins / 60;
                let local_m = adj_mins % 60;
                return format!("{year:04}-{month:02}-{day:02} {local_h:02}:{local_m:02}");
            }
            let short_time = if time.len() >= 5 { &time[..5] } else { time };
            format!("{date} {short_time}")
        },
        None => stripped.to_string(),
    }
}

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

#[derive(Clone, Copy)]
pub enum RunTargetKind {
    Binary,
    Example,
    Bench,
}

impl RunTargetKind {
    pub const BINARY_COLOR: Color = crate::tui::constants::SUCCESS_COLOR;
    pub const EXAMPLE_COLOR: Color = crate::tui::constants::ACCENT_COLOR;
    pub const BENCH_COLOR: Color = crate::tui::constants::TARGET_BENCH_COLOR;

    pub const fn color(self) -> Color {
        match self {
            Self::Binary => Self::BINARY_COLOR,
            Self::Example => Self::EXAMPLE_COLOR,
            Self::Bench => Self::BENCH_COLOR,
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

pub struct TargetEntry {
    pub name:         String,
    pub display_name: String,
    pub kind:         RunTargetKind,
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

/// Build a flat list of all runnable targets: binaries first, then examples alphabetically,
/// then benches alphabetically.
pub fn build_target_list_from_data(data: &TargetsData) -> Vec<TargetEntry> {
    let mut entries = Vec::new();

    if let Some(name) = &data.primary_binary {
        entries.push(TargetEntry {
            display_name: name.clone(),
            name:         name.clone(),
            kind:         RunTargetKind::Binary,
        });
    }

    // Collect examples with category prefix for display, sorted with
    // categorized (containing '/') before uncategorized, then alphabetically.
    let mut examples: Vec<(String, String)> = data
        .examples
        .iter()
        .flat_map(|g| {
            g.names.iter().map(|n| {
                let display = if g.category.is_empty() {
                    n.clone()
                } else {
                    format!("{}/{}", g.category, n)
                };
                (n.clone(), display)
            })
        })
        .collect();
    examples.sort_by(|a, b| {
        let a_has_cat = a.1.contains('/');
        let b_has_cat = b.1.contains('/');
        match (a_has_cat, b_has_cat) {
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            _ => a.1.cmp(&b.1),
        }
    });
    for (name, display_name) in examples {
        entries.push(TargetEntry {
            name,
            display_name,
            kind: RunTargetKind::Example,
        });
    }

    let mut bench_names = data.benches.clone();
    bench_names.sort();
    for name in bench_names {
        entries.push(TargetEntry {
            display_name: name.clone(),
            name,
            kind: RunTargetKind::Bench,
        });
    }

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
    Path,
    Targets,
    Disk,
    /// Step 5b: bytes consumed by the `target/` subtree rooted at the
    /// project. Shown alongside Disk when the walker has reported a
    /// breakdown.
    DiskTarget,
    /// Step 5b: bytes under the project root that are *not* inside a
    /// `target/` subtree (source, docs, .git, etc.).
    DiskNonTarget,
    /// Sharer target: the workspace's `target_directory` lives outside
    /// `workspace_root` (e.g. redirected by `CARGO_TARGET_DIR` or a
    /// `.cargo/config.toml`). Byte total is filled by the cached
    /// out-of-tree walk (`BackgroundMsg::OutOfTreeTargetSize`) since the
    /// per-project walker never reaches there.
    DiskOutOfTreeTarget,
    Lint,
    Ci,
    Branch,
    GitPath,
    VsLocal,
    Stars,
    RepoDesc,
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
            Self::Path => "Path",
            Self::Targets => "Type",
            Self::Disk => "Disk",
            Self::DiskTarget => "  target/",
            Self::DiskNonTarget => "  other",
            Self::DiskOutOfTreeTarget => "  target/ (out of tree)",
            Self::Lint => "Lint",
            Self::Ci => "CI",
            Self::Branch => "Branch",
            Self::GitPath => "Git Path",
            Self::VsLocal => "vs local main",
            Self::Stars => "Stars",
            Self::RepoDesc => "About",
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
            Self::Path => data.path.clone(),
            Self::Disk => data.disk.clone(),
            Self::Targets => data.types.clone(),
            Self::CratesIo => data.crates_version.as_deref().unwrap_or("").to_string(),
            Self::Downloads => data
                .crates_downloads
                .map_or_else(String::new, format_downloads),
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
            Self::Branch
            | Self::GitPath
            | Self::VsLocal
            | Self::Stars
            | Self::RepoDesc
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
            Self::Branch => {
                let branch = data.branch.as_deref().unwrap_or("");
                let is_default = data
                    .local_main_branch
                    .as_deref()
                    .is_some_and(|db| db == branch);
                if is_default {
                    format!("{branch} (HEAD)")
                } else {
                    branch.to_string()
                }
            },
            Self::GitPath => data
                .status
                .map_or_else(String::new, GitStatus::label_with_icon),
            Self::VsLocal => data.vs_local.as_deref().unwrap_or("").to_string(),
            Self::Stars => data
                .stars
                .map_or_else(String::new, |count| format!("⭐ {count}")),
            Self::RepoDesc => data.description.as_deref().unwrap_or("").to_string(),
            Self::Inception => data.inception.as_deref().unwrap_or("").to_string(),
            Self::LastCommit => data.last_commit.as_deref().unwrap_or("").to_string(),
            Self::LastFetched => data.last_fetched.as_deref().unwrap_or("").to_string(),
            Self::RateLimitCore => format_rate_limit_bucket(data.rate_limit_core),
            Self::RateLimitGraphQl => format_rate_limit_bucket(data.rate_limit_graphql),
            // Package fields — should not be called with git_value.
            Self::Path
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

/// Render a rate-limit bucket as `"remaining/limit resets HH:MM:SS"`.
/// Returns the empty string when the bucket has not been observed yet;
/// drops the `resets …` suffix when no reset timestamp is available
/// **or** when the bucket is fully unused (`used == 0`). GitHub
/// re-bases the reset window on every `/rate_limit` poll for an
/// unused bucket, so including the countdown for those rows makes
/// the value oscillate between `HH:00:00` and `(HH-1):59:59` every
/// second. Nothing has been consumed there — no countdown to show.
pub(super) fn format_rate_limit_bucket(quota: Option<RateLimitQuota>) -> String {
    let Some(quota) = quota else {
        return String::new();
    };
    let base = format!("{}/{}", quota.remaining, quota.limit);
    if quota.used == 0 {
        return base;
    }
    let Some(reset_at) = quota.reset_at else {
        return base;
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let secs = reset_at.saturating_sub(now);
    format!(
        "{base} resets {}",
        crate::tui::duration_fmt::format_progressive(secs)
    )
}

/// All fields for the `Package` column.
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
    // Step 5b: insert the target / non-target breakdown immediately
    // below the aggregate Disk row when the walker has reported one,
    // so the user sees which half of the bytes is build artifact vs
    // source (the two always sum to Disk for owners; for sharers the
    // target line reads 0 — target is redirected out of tree).
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
    if data.crates_version.is_some() {
        fields.push(DetailField::CratesIo);
    }
    if data.crates_downloads.is_some() {
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

pub fn git_fields_from_data(data: &GitData) -> Vec<DetailField> {
    let mut fields = Vec::new();
    if data.branch.is_some() {
        fields.push(DetailField::Branch);
    }
    if data.status.is_some() {
        fields.push(DetailField::GitPath);
    }
    if data.vs_local.is_some() {
        fields.push(DetailField::VsLocal);
    }
    if data.stars.is_some() {
        fields.push(DetailField::Stars);
    }
    // RepoDesc is rendered separately in the About section by
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

/// Per-pane data for the Package detail panel.
#[derive(Clone, Default)]
/// Per-project pane data the Package pane renders. The "value"
/// fields are pre-resolved display strings — callers can render
/// without `&App` access.
///
/// The `lint_display` and `ci_display` fields are the App-derived
/// strings that used to be computed at render time via
/// `DetailField::package_value(data, app)`. Pre-resolving them
/// lets `PackagePane::render` operate on `&PackageData` alone.
pub struct PackageData {
    pub package_title:            String,
    pub title_name:               String,
    pub path:                     String,
    pub version:                  String,
    pub description:              Option<String>,
    pub crates_version:           Option<String>,
    pub crates_downloads:         Option<u64>,
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
    /// Step 5b: bytes under the project root inside any `target/`
    /// subtree. `None` until the walker has reported a breakdown.
    pub in_project_target:        Option<u64>,
    /// Step 5b: everything else under the project root (source,
    /// docs, .git, vendored crates outside target, etc.).
    pub in_project_non_target:    Option<u64>,
    /// Typed display value for the Lint field; populated at
    /// assembly time so render can read it without `&App`. The
    /// renderer matches on variants and applies
    /// `animation_elapsed` to `status.icon()` at render time.
    pub lint_display:             super::LintDisplay,
    /// Typed display value for the Ci row in the Package detail
    /// pane. Renderer matches on variants directly. Domain
    /// authority lives on [`crate::tui::ci_state::Ci`]; produced
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

/// Render "—" when a `PackageRecord` field is absent from the manifest
/// (design plan → step 4: "When any is missing from the manifest,
/// render `—`.").
fn or_dash(value: Option<&str>) -> String {
    value
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map_or_else(|| "—".to_string(), String::from)
}

/// Per-pane data for the Git detail panel.
#[derive(Clone, Default)]
pub struct GitData {
    pub branch:             Option<String>,
    pub status:             Option<GitStatus>,
    pub vs_local:           Option<String>,
    pub local_main_branch:  Option<String>,
    pub main_branch_label:  String,
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
    pub name:        String,
    pub icon:        &'static str,
    pub display_url: String,
    pub tracked_ref: String,
    pub status:      String,
    pub full_url:    Option<String>,
}

/// Per-worktree info rendered in the Git pane's Worktrees table.
///
/// `ahead_behind` is relative to the primary worktree's HEAD commit.
#[derive(Clone)]
pub struct WorktreeInfo {
    pub name:         String,
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
    Field(DetailField),
    Remote(&'a RemoteRow),
    Worktree(&'a WorktreeInfo),
}

pub fn git_row_at(data: &GitData, pos: usize) -> Option<GitRow<'_>> {
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

/// Per-pane data for the Targets panel.
#[derive(Clone, Default)]
pub struct TargetsData {
    pub primary_binary: Option<String>,
    pub examples:       Vec<ExampleGroup>,
    pub benches:        Vec<String>,
}

impl TargetsData {
    pub const fn has_targets(&self) -> bool {
        self.primary_binary.is_some() || !self.examples.is_empty() || !self.benches.is_empty()
    }

    /// Build from a [`PackageRecord`]. Examples grouped by
    /// subdirectory derived from `TargetRecord.src_path` relative to
    /// the package's manifest directory (design plan → Step 3, Targets
    /// pane); benches listed flat. The primary-binary name is the bin
    /// target whose name matches `title_name` (cargo's "default run"
    /// target); falls back to `None` if no such bin exists.
    pub fn from_package_record(record: &PackageRecord, title_name: &str) -> Self {
        use std::collections::HashMap;

        use cargo_metadata::TargetKind;

        let manifest_dir = record.manifest_path.as_path().parent();

        let mut example_groups: HashMap<String, Vec<String>> = HashMap::new();
        let mut benches: Vec<String> = Vec::new();
        let mut has_bin_with_title = false;

        for target in &record.targets {
            if target.kinds.contains(&TargetKind::Example) {
                let category = manifest_dir
                    .and_then(|dir| target.src_path.as_path().strip_prefix(dir).ok())
                    .and_then(|rel| {
                        let parts: Vec<_> = rel
                            .components()
                            .filter_map(|c| match c {
                                Component::Normal(seg) => Some(seg.to_string_lossy().into_owned()),
                                _ => None,
                            })
                            .collect();
                        // `examples/<category>/<file>.rs` → category.
                        // `examples/<file>.rs` → root-level (empty).
                        if parts.len() >= 3 {
                            Some(parts[1].clone())
                        } else {
                            None
                        }
                    })
                    .unwrap_or_default();
                example_groups
                    .entry(category)
                    .or_default()
                    .push(target.name.clone());
            }
            if target.kinds.contains(&TargetKind::Bench) {
                benches.push(target.name.clone());
            }
            if target.kinds.contains(&TargetKind::Bin) && target.name == title_name {
                has_bin_with_title = true;
            }
        }

        let mut examples: Vec<ExampleGroup> = example_groups
            .into_iter()
            .map(|(category, mut names)| {
                names.sort();
                ExampleGroup { category, names }
            })
            .collect();
        // Root-level first, then alphabetical by category — matches
        // the hand-parsed `build_sorted_groups` convention so the UI
        // ordering doesn't shift across the migration.
        examples.sort_by(|a, b| {
            let a_root = a.category.is_empty();
            let b_root = b.category.is_empty();
            match (a_root, b_root) {
                (true, false) => Ordering::Less,
                (false, true) => Ordering::Greater,
                _ => a.category.cmp(&b.category),
            }
        });
        benches.sort();

        Self {
            primary_binary: has_bin_with_title.then(|| title_name.to_string()),
            examples,
            benches,
        }
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
    if matches!(item, RootItem::Worktrees(_)) {
        return "Worktree Group".to_string();
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

fn format_ahead_behind((ahead, behind): (usize, usize)) -> String {
    match (ahead, behind) {
        (0, 0) => IN_SYNC.to_string(),
        (ahead, 0) => format!("{SYNC_UP}{ahead} ahead"),
        (0, behind) => format!("{SYNC_DOWN}{behind} behind"),
        (ahead, behind) => format!("{SYNC_UP}{ahead} {SYNC_DOWN}{behind}"),
    }
}

pub(super) fn format_remote_status(ahead_behind: Option<(usize, usize)>) -> String {
    match ahead_behind {
        Some((0, 0)) => IN_SYNC.to_string(),
        Some((ahead, 0)) => format!("{SYNC_UP}{ahead} ahead"),
        Some((0, behind)) => format!("{SYNC_DOWN}{behind} behind"),
        Some((ahead, behind)) => format!("{SYNC_UP}{ahead} {SYNC_DOWN}{behind}"),
        None => NO_REMOTE_SYNC.to_string(),
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
    branch:             Option<String>,
    path:               Option<GitStatus>,
    vs_local:           Option<String>,
    local_main_branch:  Option<String>,
    main_branch_label:  String,
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
    let entry = app.project_list.entry_containing(abs_path);
    let git_repo = entry.and_then(|entry| entry.git_repo.as_ref());
    let repo_info = git_repo.and_then(|repo| repo.repo_info.as_ref());
    let checkout = app.project_list.git_info_for(abs_path);

    let branch = checkout.and_then(|info| info.branch.clone());
    let vs_local = checkout
        .and_then(|info| info.ahead_behind_local)
        .map(format_ahead_behind);
    let local_main_branch = repo_info.and_then(|repo| repo.local_main_branch.clone());
    let main_branch_label = app.config.current().tui.main_branch.clone();
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
        branch,
        path: app.project_list.git_status_for(abs_path),
        vs_local,
        local_main_branch,
        main_branch_label,
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
                RemoteKind::Fork => crate::constants::GIT_FORK,
                RemoteKind::Clone => crate::constants::GIT_CLONE,
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
            RemoteRow {
                name: remote.name.clone(),
                icon,
                display_url,
                tracked_ref,
                status,
                full_url: remote.url.clone(),
            }
        })
        .collect()
}

/// If `url` starts with `default_host`, return `owner/repo` (stripping
/// `.git` suffix); otherwise return the full URL.
fn shorten_remote_url(url: &str, default_host: &str) -> String {
    let stripped = url.strip_prefix(default_host).unwrap_or(url);
    stripped
        .strip_suffix(".git")
        .unwrap_or(stripped)
        .to_string()
}

/// Check whether a `RootItem` is a worktree group.
const fn is_worktree_group(item: &RootItem) -> bool { matches!(item, RootItem::Worktrees(_)) }

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
                .and_then(|info| info.branch.clone());
            let ahead_behind = if path.as_path() == primary_path.as_path() {
                Some((0, 0))
            } else {
                project::worktree_ahead_behind_primary(path.as_path(), primary_path.as_path())
            };
            WorktreeInfo {
                name,
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
                build_pane_data_for_workspace(app, ws, &display_path, true, Some(item))
            },
            RustProject::Package(pkg) => {
                build_pane_data_for_package(app, pkg, &display_path, true, Some(item))
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

    DetailPaneData {
        package: PackageData {
            package_title: "Submodule".to_string(),
            title_name: submodule.name.clone(),
            path: display_path,
            version,
            description: submodule.url.clone(),
            crates_version: None,
            crates_downloads: None,
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
            branch:             git_detail.branch,
            status:             git_detail.path,
            vs_local:           git_detail.vs_local,
            local_main_branch:  git_detail.local_main_branch,
            main_branch_label:  git_detail.main_branch_label,
            stars:              git_detail.stars,
            description:        git_detail.description,
            inception:          git_detail.inception,
            last_commit:        git_detail.last_commit,
            last_fetched:       git_detail.last_fetched,
            rate_limit_core:    git_detail.rate_limit_core,
            rate_limit_graphql: git_detail.rate_limit_graphql,
            github_status:      git_detail.github_status,
            remotes:            git_detail.remotes,
            worktrees:          Vec::new(),
        },
        targets: TargetsData {
            primary_binary: None,
            examples:       Vec::new(),
            benches:        Vec::new(),
        },
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
            package_title: "Workspace".to_string(),
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
            package_title: "Project".to_string(),
        },
    )
}

struct PaneDataSource<'a> {
    abs_path:      &'a Path,
    display_path:  &'a str,
    title_name:    String,
    has_cargo:     bool,
    cargo:         Option<&'a Cargo>,
    wt_item:       Option<&'a RootItem>,
    stats_rows:    Vec<(&'static str, usize)>,
    package_title: String,
}

/// Crates-io fields pulled from either a Rust info or vendored entry.
struct CratesIoFields {
    version:   Option<String>,
    downloads: Option<u64>,
}

fn resolve_crates_io_fields(app: &App, abs_path: &Path) -> CratesIoFields {
    let rust_info = app.project_list.rust_info_at_path(abs_path);
    let vendored = app.project_list.vendored_at_path(abs_path);
    CratesIoFields {
        version:   rust_info
            .and_then(|r| r.crates_version().map(String::from))
            .or_else(|| vendored.and_then(|v| v.crates_version().map(String::from))),
        downloads: rust_info
            .and_then(RustInfo::crates_downloads)
            .or_else(|| vendored.and_then(VendoredPackage::crates_downloads)),
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
    display_path:             String,
    stats_rows:               Vec<(&'static str, usize)>,
    has_cargo:                bool,
    manifest:                 ManifestFields,
    crates_version:           Option<String>,
    crates_downloads:         Option<u64>,
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
        path: args.display_path,
        version,
        description,
        crates_version: args.crates_version,
        crates_downloads: args.crates_downloads,
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

fn compute_in_project_bytes(pl: &ProjectList, abs_path: &Path) -> (Option<u64>, Option<u64>) {
    pl.at_path(abs_path).map_or((None, None), |pi| {
        (pi.in_project_target, pi.in_project_non_target)
    })
}

fn compute_ci_status(app: &App, abs_path: &Path, wt_item: Option<&RootItem>) -> Option<CiStatus> {
    wt_item.map_or_else(
        || app.project_list.ci_status_for(abs_path, &app.ci),
        |item| app.project_list.ci_status_for_root_item(item, &app.ci),
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

fn build_pane_data_common(app: &App, src: PaneDataSource<'_>) -> DetailPaneData {
    let PaneDataSource {
        abs_path,
        display_path,
        title_name,
        has_cargo,
        cargo,
        wt_item,
        stats_rows,
        package_title,
    } = src;
    let display_path = display_path.to_owned();
    let t_git = std::time::Instant::now();
    let git_detail = build_git_detail_fields(app, abs_path);
    let git_detail_ms = perf_log::ms(t_git.elapsed().as_millis());

    let crates_io = resolve_crates_io_fields(app, abs_path);

    let t_disk = std::time::Instant::now();
    let disk = wt_item.map_or_else(
        || super::formatted_disk(app, abs_path),
        super::formatted_disk_for_item,
    );
    let ci = compute_ci_status(app, abs_path, wt_item);
    let disk_ms = perf_log::ms(t_disk.elapsed().as_millis());

    let t_wt = std::time::Instant::now();
    let worktrees = resolve_worktrees(app, wt_item);
    let worktrees_ms = perf_log::ms(t_wt.elapsed().as_millis());

    let types_str = cargo.map_or_else(String::new, |c| {
        c.types()
            .iter()
            .map(std::string::ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    });

    let abs_path_owned = AbsolutePath::from(abs_path);
    let t_meta = std::time::Instant::now();
    let package_record = lookup_package_record(app, &abs_path_owned);
    let metadata_ms = perf_log::ms(t_meta.elapsed().as_millis());
    let manifest = manifest_fields_from(package_record.as_ref());

    let (in_project_target, in_project_non_target) =
        compute_in_project_bytes(&app.project_list, abs_path);
    let t_oot = std::time::Instant::now();
    let out_of_tree_target_bytes = lookup_out_of_tree_target_bytes(app, &abs_path_owned);
    let oot_ms = perf_log::ms(t_oot.elapsed().as_millis());

    tracing::info!(
        git_detail_ms,
        disk_ms,
        worktrees_ms,
        metadata_ms,
        oot_ms,
        path = %abs_path.display(),
        "pane_common_breakdown"
    );

    let ManifestFields {
        edition,
        license,
        homepage,
        repository,
        version,
        description,
    } = manifest;
    let crates_version = crates_io.version;
    let crates_downloads = crates_io.downloads;

    let (lint_display, ci_display) =
        compute_package_displays(app, &abs_path_owned, ci, &package_title);
    let package = build_package_data(BuildPackageDataArgs {
        package_title,
        title_name,
        display_path,
        stats_rows,
        has_cargo,
        manifest: ManifestFields {
            edition,
            license,
            homepage,
            repository,
            version,
            description,
        },
        crates_version,
        crates_downloads,
        types_str,
        disk,
        in_project_target,
        in_project_non_target,
        out_of_tree_target_bytes,
        lint_display,
        ci_display,
    });

    assemble_detail_pane_data(package, git_detail, worktrees, package_record.as_ref())
}

/// Assemble `DetailPaneData` from already-resolved inputs.
/// Step 3a/3b: derive Targets-pane data from workspace metadata when it
/// covers this project. Without metadata the pane stays empty
/// (design plan → "Workspace members + Targets pane migrate to
/// metadata atomically; targets show 'Loading…' without
/// metadata"). The old hand-parsed fallback could disagree with
/// cargo's real discovery rules (autoexamples, required-features,
/// excluded targets), so we'd rather render nothing pre-metadata
/// than render something misleading.
fn assemble_detail_pane_data(
    package: PackageData,
    git_detail: GitDetailFields,
    worktrees: Vec<WorktreeInfo>,
    package_record: Option<&PackageRecord>,
) -> DetailPaneData {
    DetailPaneData {
        package,
        git: GitData {
            branch: git_detail.branch,
            status: git_detail.path,
            vs_local: git_detail.vs_local,
            local_main_branch: git_detail.local_main_branch,
            main_branch_label: git_detail.main_branch_label,
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
        },
        targets: package_record.map_or_else(TargetsData::default, |record| {
            TargetsData::from_package_record(record, record.name.as_str())
        }),
    }
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
            .and_then(|git| git.branch.clone())
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
