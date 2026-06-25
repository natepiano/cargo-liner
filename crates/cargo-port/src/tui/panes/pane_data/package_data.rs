use super::AbsolutePath;
use super::App;
use super::Cargo;
use super::DetailField;
use super::GitData;
use super::NonRustProject;
use super::PROJECT_LIBS_LABEL;
use super::PROJECT_MEMBERS_LABEL;
use super::PROJECT_PROC_MACROS_LABEL;
use super::PROJECT_SUBMODULES_LABEL;
use super::PROJECT_VENDORED_LABEL;
use super::Package;
use super::PackageRecord;
use super::ProjectType;
use super::TARGET_KIND_BENCH_LABEL;
use super::TARGET_KIND_BIN_LABEL;
use super::TARGET_KIND_EXAMPLE_LABEL;
use super::TESTS_DOC_LABEL;
use super::TESTS_IGNORED_LABEL;
use super::TESTS_INTEGRATION_LABEL;
use super::TESTS_TOTAL_LABEL;
use super::TESTS_UNIT_LABEL;
use super::TestCounts;
use super::Workspace;
use crate::project::ProjectFields;
use crate::tui::state::CiDisplay;
use crate::tui::state::LintDisplay;

#[derive(Default)]
pub(super) struct StructureCounts {
    members:     usize,
    vendored:    usize,
    submodules:  usize,
    libs:        usize,
    bins:        usize,
    proc_macros: usize,
    examples:    usize,
    benches:     usize,
}

impl StructureCounts {
    pub(super) fn add_package(&mut self, project: &Package) {
        self.vendored += project.vendored().len();
        self.submodules += project.info().submodules.len();
        self.add_cargo(&project.cargo);
    }

    pub(super) fn add_workspace(&mut self, ws: &Workspace) {
        self.members += ws
            .groups()
            .iter()
            .map(|group| group.members().len())
            .sum::<usize>();
        self.vendored += ws.vendored().len();
        self.submodules += ws.info().submodules.len();
        self.add_cargo(&ws.cargo);
    }

    pub(super) fn add_non_rust(&mut self, project: &NonRustProject) {
        self.submodules += project.info().submodules.len();
    }

    pub(super) fn add_cargo(&mut self, cargo: &Cargo) {
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
    }

    /// Returns non-zero stats as (label, count) pairs for column display.
    pub(super) fn to_rows(&self) -> Vec<(&'static str, usize)> {
        let mut rows = Vec::new();
        if self.members > 0 {
            rows.push((PROJECT_MEMBERS_LABEL, self.members));
        }
        if self.vendored > 0 {
            rows.push((PROJECT_VENDORED_LABEL, self.vendored));
        }
        if self.submodules > 0 {
            rows.push((PROJECT_SUBMODULES_LABEL, self.submodules));
        }
        if self.libs > 0 {
            rows.push((PROJECT_LIBS_LABEL, self.libs));
        }
        if self.bins > 0 {
            rows.push((TARGET_KIND_BIN_LABEL, self.bins));
        }
        if self.proc_macros > 0 {
            rows.push((PROJECT_PROC_MACROS_LABEL, self.proc_macros));
        }
        if self.examples > 0 {
            rows.push((TARGET_KIND_EXAMPLE_LABEL, self.examples));
        }
        if self.benches > 0 {
            rows.push((TARGET_KIND_BENCH_LABEL, self.benches));
        }
        rows
    }
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
    /// Index into `PackageData::stats_rows` (the Structure section).
    Structure(usize),
    /// Index into `PackageData::test_rows` (the Tests section).
    Tests(usize),
    /// Index into `PackageData::crates_io_rows` (the crates.io section).
    CratesIo(usize),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorktreeGroupSummary {
    pub worktrees: usize,
    pub deleted:   usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum PublishStatus {
    Publishable,
    #[default]
    NotPublishable,
}

impl PublishStatus {
    pub(super) const fn is_publishable(self) -> bool { matches!(self, Self::Publishable) }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PackagePresence {
    Present,
    #[default]
    Missing,
}

impl PackagePresence {
    const fn has_package(self) -> bool { matches!(self, Self::Present) }
}

impl From<bool> for PackagePresence {
    fn from(has_package: bool) -> Self {
        if has_package {
            Self::Present
        } else {
            Self::Missing
        }
    }
}

/// Per-pane data for the Package detail panel.
#[derive(Clone, Default)]
/// Per-project pane data the Package pane renders. The "value"
/// fields are pre-resolved display strings — callers can render
/// without `&App` access. Pre-resolving `lint_display` and `ci_display`
/// lets `PackagePane::render` operate on `&PackageData` alone.
pub struct PackageData {
    pub title:                    String,
    pub name:                     String,
    pub worktree_group_summary:   Option<WorktreeGroupSummary>,
    pub primary_section:          Option<PackageSection>,
    pub path:                     String,
    /// Version string. `None` before metadata lands; the renderer
    /// maps absence to a placeholder. For submodules this carries the
    /// pinned commit rather than a semver.
    pub version:                  Option<String>,
    pub description:              Option<String>,
    /// crates.io section of the stats column — `version` (latest stable,
    /// or the newest prerelease when there is no stable), an optional
    /// prerelease row (`rc` / `beta` / `alpha`) when a newer prerelease
    /// exists alongside a stable, and `downloads`. Empty for
    /// non-publishable projects; a publishable project with no data during
    /// a confirmed crates.io outage gets a single `version` →
    /// `unreachable` row so the user knows why it is empty.
    pub crates_io_rows:           Vec<(&'static str, String)>,
    /// Resolved cargo target kinds. `None` before `cargo metadata`
    /// lands and for non-Rust projects; `Some(vec)` once resolved —
    /// the vec is empty only for the rare crate with no lib/bin/
    /// proc-macro target, and workspaces fold their `Workspace`
    /// identity into it. The renderer maps absence to a placeholder
    /// so a missing value can never render as a blank.
    pub types:                    Option<Vec<ProjectType>>,
    /// Bytes under the project root. `None` until the disk walk has
    /// reported; the renderer formats `Some` and leaves `None` blank,
    /// matching the `target/` / `other` sub-rows.
    pub disk:                     Option<u64>,
    /// Structure section of the stats column — project-child counts
    /// (`members` / `vendored` / `submodules`) followed by cargo target-kind
    /// counts (`lib` / `bin` / `proc-macro` / `example` / `bench`).
    pub stats_rows:               Vec<(&'static str, usize)>,
    /// Tests section of the stats column — `unit` / `integration` /
    /// `doc` test counts from the source scan, plus an `(ignored)`
    /// doctest annotation and a runnable `total` when applicable. Empty
    /// until the scan lands (or when the project has no tests); an empty
    /// vec hides the section.
    pub test_rows:                Vec<(&'static str, usize)>,
    pub package_presence:         PackagePresence,
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
    pub lint_display:             LintDisplay,
    /// Typed display value for the Ci row in the Package detail
    /// pane. Renderer matches on variants directly. Domain
    /// authority lives on [`crate::tui::state::Ci`]; produced
    /// by `Ci::package_display`.
    pub ci_display:               CiDisplay,
    /// Byte size of the workspace's out-of-tree `target_directory`
    /// (when the resolved target sits outside `workspace_root`). Flows
    /// from `WorkspaceMetadata::out_of_tree_target_bytes` once the
    /// cached walk reports back; `None` for in-tree targets or before
    /// the walk lands.
    pub out_of_tree_target_bytes: Option<u64>,
}

impl PackageData {
    pub const fn has_package(&self) -> bool { self.package_presence.has_package() }
}

/// Build the Tests-section rows from accumulated test counts. Lists the
/// non-zero `unit` / `integration` / `doc` buckets, an `(ignored)`
/// annotation when doctests are skipped, and a `total` of the runnable
/// buckets once two or more rows are present. Returns an empty vec when
/// nothing runs, which hides the section (matching the pre-scan state
/// where the counts are `None`). `(ignored)` is excluded from the total —
/// rustdoc registers those doctests but never runs them.
pub(super) fn test_rows_from_counts(counts: TestCounts) -> Vec<(&'static str, usize)> {
    let mut rows = Vec::new();
    if counts.unit > 0 {
        rows.push((TESTS_UNIT_LABEL, counts.unit));
    }
    if counts.integration > 0 {
        rows.push((TESTS_INTEGRATION_LABEL, counts.integration));
    }
    if counts.doc > 0 {
        rows.push((TESTS_DOC_LABEL, counts.doc));
    }
    if counts.doc_ignored > 0 {
        rows.push((TESTS_IGNORED_LABEL, counts.doc_ignored));
    }
    if rows.len() >= 2 {
        rows.push((
            TESTS_TOTAL_LABEL,
            counts.unit + counts.integration + counts.doc,
        ));
    }
    rows
}

/// True iff the Git pane's Stars row should render a "github
/// unreachable" placeholder in warning color: GitHub is confirmed
/// down (Unreachable / `RateLimited`) and no stars count has landed
/// yet. Unauthenticated is excluded: it's not an
/// outage, and the rate-limit rows already carry the auth hint.
pub(super) const fn github_stars_is_unreachable_placeholder(data: &GitData) -> bool {
    data.stars.is_none()
        && !data.github_status.is_available()
        && !data.github_status.is_unauthenticated()
}

/// Primary project fields for the `Package` column.
/// Non-Rust projects show only name, path, disk, and CI.
pub(super) fn package_fields_from_data(data: &PackageData) -> Vec<DetailField> {
    if data.title == "Project" {
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
    if data.has_package() {
        fields.push(DetailField::Version);
    }
    // crates.io version / prerelease / downloads now render in their own
    // right-side stats section (`crates_io_rows`), not as left-column fields.
    // Step 4 fields: show unconditionally on Rust packages so that
    // unset values render as `—` and the UI surface matches the
    // manifest faithfully even before metadata arrives.
    if data.has_package() {
        fields.push(DetailField::Edition);
        fields.push(DetailField::License);
        fields.push(DetailField::Homepage);
        fields.push(DetailField::Repository);
    }
    fields
}

pub(super) fn package_rows_from_data(data: &PackageData) -> Vec<PackageRow> {
    let fields = package_fields_from_data(data);
    let mut rows = vec![PackageRow::Description];
    let Some(summary) = data.worktree_group_summary.as_ref() else {
        rows.extend(fields.into_iter().map(PackageRow::Field));
        rows.extend((0..data.stats_rows.len()).map(PackageRow::Structure));
        rows.extend((0..data.test_rows.len()).map(PackageRow::Tests));
        rows.extend((0..data.crates_io_rows.len()).map(PackageRow::CratesIo));
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
    rows.extend((0..data.test_rows.len()).map(PackageRow::Tests));
    rows
}

pub(super) const fn package_row_is_selectable(row: &PackageRow) -> bool {
    matches!(
        row,
        PackageRow::Description
            | PackageRow::Field(_)
            | PackageRow::Structure(_)
            | PackageRow::Tests(_)
            | PackageRow::CratesIo(_)
    )
}

pub(super) fn package_first_selectable_row(rows: &[PackageRow]) -> Option<usize> {
    rows.iter().position(package_row_is_selectable)
}

pub(super) fn package_last_selectable_row(rows: &[PackageRow]) -> Option<usize> {
    rows.iter().rposition(package_row_is_selectable)
}

pub(super) fn package_selectable_row_at_or_after(rows: &[PackageRow], pos: usize) -> Option<usize> {
    rows.iter()
        .enumerate()
        .skip(pos.min(rows.len()))
        .find_map(|(index, row)| package_row_is_selectable(row).then_some(index))
}

pub(super) fn package_selectable_row_at_or_before(
    rows: &[PackageRow],
    pos: usize,
) -> Option<usize> {
    rows.iter()
        .enumerate()
        .take(pos.saturating_add(1).min(rows.len()))
        .rev()
        .find_map(|(index, row)| package_row_is_selectable(row).then_some(index))
}

pub(super) fn package_nearest_selectable_row(rows: &[PackageRow], pos: usize) -> Option<usize> {
    package_selectable_row_at_or_after(rows, pos)
        .or_else(|| package_selectable_row_at_or_before(rows, pos))
}

/// Resolve (version, description) for the detail pane from the
/// authoritative metadata. Returns `(None, None)` pre-metadata; the
/// renderer maps the absent version to its placeholder.
pub(super) fn version_and_description(
    pkg: Option<&PackageRecord>,
) -> (Option<String>, Option<String>) {
    let version = pkg.map(|p| p.version.to_string());
    let description = pkg.and_then(|p| p.description.clone());
    (version, description)
}

/// Resolve the sharer target size for the row at `abs_path` — i.e. the
/// workspace's cached walk of its out-of-tree `target_directory`. Returns
/// `None` for in-tree targets (already reflected in `DiskTarget`) or
/// before the walk has landed.
pub(super) fn lookup_out_of_tree_target_bytes(app: &App, abs_path: &AbsolutePath) -> Option<u64> {
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
pub(super) fn or_dash(value: Option<&str>) -> String {
    value
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map_or_else(|| "—".to_string(), String::from)
}

#[cfg(test)]
mod test_row_tests {
    use super::*;
    use crate::tui::panes::constants::TESTS_IGNORED_LABEL;
    use crate::tui::panes::constants::TESTS_TOTAL_LABEL;

    fn counts(unit: usize, integration: usize, doc: usize, doc_ignored: usize) -> TestCounts {
        TestCounts {
            unit,
            integration,
            doc,
            doc_ignored,
        }
    }

    #[test]
    fn single_bucket_has_no_total_row() {
        assert_eq!(test_rows_from_counts(counts(5, 0, 0, 0)), vec![("unit", 5)]);
    }

    #[test]
    fn multiple_buckets_append_runnable_total() {
        assert_eq!(
            test_rows_from_counts(counts(117, 48, 1185, 0)),
            vec![
                ("unit", 117),
                ("integration", 48),
                ("doc", 1185),
                (TESTS_TOTAL_LABEL, 1350),
            ]
        );
    }

    #[test]
    fn ignored_is_shown_but_excluded_from_total() {
        assert_eq!(
            test_rows_from_counts(counts(0, 0, 1185, 152)),
            vec![
                ("doc", 1185),
                (TESTS_IGNORED_LABEL, 152),
                (TESTS_TOTAL_LABEL, 1185),
            ]
        );
    }

    #[test]
    fn all_zero_counts_hide_the_section() {
        assert!(test_rows_from_counts(counts(0, 0, 0, 0)).is_empty());
    }
}

#[cfg(test)]
mod package_row_tests {
    use super::*;
    use crate::project::ProjectType;
    use crate::tui::state::CiDisplay;
    use crate::tui::state::LintDisplay;

    fn package_data(is_rust_project: bool) -> PackageData {
        PackageData {
            title:                    if is_rust_project {
                "Package".to_string()
            } else {
                "Project".to_string()
            },
            name:                     "demo".to_string(),
            worktree_group_summary:   None,
            primary_section:          None,
            path:                     "~/demo".to_string(),
            version:                  Some("0.1.0".to_string()),
            description:              None,
            crates_io_rows:           Vec::new(),
            types:                    Some(vec![ProjectType::Library]),
            disk:                     Some(38_989_922_304),
            stats_rows:               Vec::new(),
            test_rows:                Vec::new(),
            package_presence:         PackagePresence::Present,
            edition:                  None,
            license:                  None,
            homepage:                 None,
            repository:               None,
            in_project_target:        None,
            in_project_non_target:    None,
            out_of_tree_target_bytes: None,
            lint_display:             LintDisplay::default(),
            ci_display:               CiDisplay::default(),
        }
    }

    #[test]
    fn crates_io_rows_appended_as_selectable_section_rows() {
        // The crates.io stats section contributes one selectable
        // `PackageRow::CratesIo` per row, after the left-column fields.
        let mut data = package_data(true);
        data.crates_io_rows = vec![
            ("version", "0.20.2".to_string()),
            ("rc", "0.21.0-rc.2".to_string()),
            ("downloads", "663".to_string()),
        ];
        let crates_io_rows: Vec<_> = package_rows_from_data(&data)
            .into_iter()
            .filter(|row| matches!(row, PackageRow::CratesIo(_)))
            .collect();
        assert_eq!(
            crates_io_rows,
            vec![
                PackageRow::CratesIo(0),
                PackageRow::CratesIo(1),
                PackageRow::CratesIo(2),
            ],
        );
    }

    #[test]
    fn crates_io_section_absent_without_data() {
        // No crates.io data -> no crates.io rows.
        let data = package_data(true);
        assert!(
            !package_rows_from_data(&data)
                .iter()
                .any(|row| matches!(row, PackageRow::CratesIo(_)))
        );
    }

    #[test]
    fn fields_place_lint_and_ci_before_disk_for_rust_projects() {
        let data = package_data(true);
        // Step 4 adds Edition / License / Homepage / Repository at the
        // end of the Rust-package field list. They show unconditionally
        // (the pane renders `-` for unset values).
        assert_eq!(
            package_fields_from_data(&data)
                .into_iter()
                .map(DetailField::label)
                .collect::<Vec<_>>(),
            vec![
                "Path",
                "Disk",
                "Type",
                "Lint",
                "CI",
                "Version",
                "Edition",
                "License",
                "Homepage",
                "Repository",
            ]
        );
    }

    #[test]
    fn fields_place_lint_and_ci_before_disk_for_non_rust_projects() {
        let data = package_data(false);
        assert_eq!(
            package_fields_from_data(&data)
                .into_iter()
                .map(DetailField::label)
                .collect::<Vec<_>>(),
            vec!["Path", "Disk", "Lint", "CI"]
        );
    }
}
