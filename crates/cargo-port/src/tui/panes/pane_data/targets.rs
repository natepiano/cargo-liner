use super::AbsolutePath;
use super::Color;
use super::Component;
use super::Ordering;
use super::Path;
use super::TARGET_KIND_BENCH_LABEL;
use super::TARGET_KIND_BIN_LABEL;
use super::TARGET_KIND_EXAMPLE_LABEL;
use super::TargetKind;
use super::WorkspaceMetadata;
use super::theme_roles;

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
            Self::Bench => theme_roles::target_bench_color(),
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Binary => TARGET_KIND_BIN_LABEL,
            Self::Example => TARGET_KIND_EXAMPLE_LABEL,
            Self::Bench => TARGET_KIND_BENCH_LABEL,
        }
    }

    /// Longest label width across all variants, plus 1 for trailing pad.
    pub const fn padded_label_width() -> usize {
        let mut max = 0;
        let labels: [&str; 3] = [
            TARGET_KIND_BIN_LABEL,
            TARGET_KIND_EXAMPLE_LABEL,
            TARGET_KIND_BENCH_LABEL,
        ];
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

/// Where a target appears in the Source column.
///
/// The label is always explicit: root-package targets and member targets
/// both display their cargo `[package].name`. `TargetSourceKind` only
/// controls ordering.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TargetSource {
    label: String,
    kind:  TargetSourceKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TargetSourceKind {
    WorkspaceRoot,
    Member,
    Worktree,
}

impl TargetSource {
    pub const fn workspace_root(package_name: String) -> Self {
        Self {
            label: package_name,
            kind:  TargetSourceKind::WorkspaceRoot,
        }
    }

    pub const fn member(package_name: String) -> Self {
        Self {
            label: package_name,
            kind:  TargetSourceKind::Member,
        }
    }

    pub const fn worktree(label: String) -> Self {
        Self {
            label,
            kind: TargetSourceKind::Worktree,
        }
    }

    pub const fn label(&self) -> &str { self.label.as_str() }

    /// Sort key: workspace root first, then members, then worktree labels.
    const fn sort_key(&self) -> (u8, &str) {
        let order = match self.kind {
            TargetSourceKind::WorkspaceRoot => 0,
            TargetSourceKind::Member => 1,
            TargetSourceKind::Worktree => 2,
        };
        (order, self.label())
    }

    #[cfg(test)]
    pub const fn is_workspace_root(&self) -> bool {
        matches!(self.kind, TargetSourceKind::WorkspaceRoot)
    }

    #[cfg(test)]
    pub fn is_member_named(&self, package_name: &str) -> bool {
        matches!(self.kind, TargetSourceKind::Member) && self.label == package_name
    }
}

#[derive(Clone, Debug)]
pub struct TargetEntry {
    pub name:              String,
    pub display_name:      String,
    pub run_target_kind:   RunTargetKind,
    pub source:            TargetSource,
    pub project_path:      AbsolutePath,
    pub package_name:      String,
    pub src_path:          AbsolutePath,
    /// Cargo `required-features` for this target, passed as `--features`
    /// when running so feature-gated targets launch without manual flags.
    pub required_features: Vec<String>,
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
pub fn build_target_list_from_data(data: &TargetsData) -> Vec<TargetEntry> {
    let mut entries =
        Vec::with_capacity(data.binaries.len() + data.examples.len() + data.benches.len());
    entries.extend(data.binaries.iter().cloned());
    entries.extend(data.examples.iter().cloned());
    entries.extend(data.benches.iter().cloned());
    entries
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

    /// Total runnable targets across the three kind sections — the
    /// Targets table's row count.
    pub const fn target_count(&self) -> usize {
        self.binaries.len() + self.examples.len() + self.benches.len()
    }

    pub(super) fn append(&mut self, mut other: Self) {
        self.binaries.append(&mut other.binaries);
        self.examples.append(&mut other.examples);
        self.benches.append(&mut other.benches);
    }

    pub(super) fn relabel_as_worktree(&mut self, checkout_name: &str) {
        for entry in self
            .binaries
            .iter_mut()
            .chain(self.examples.iter_mut())
            .chain(self.benches.iter_mut())
        {
            entry.source =
                TargetSource::worktree(format!("{checkout_name}/{}", entry.package_name));
        }
    }

    pub(super) fn sort_entries(&mut self) {
        self.binaries.sort_by(compare_target_name);
        self.examples.sort_by(compare_example_name);
        self.benches.sort_by(compare_target_name);
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
        let project_path = AbsolutePath::from(selected_path);
        let mut binaries: Vec<TargetEntry> = Vec::new();
        let mut examples: Vec<TargetEntry> = Vec::new();
        let mut benches: Vec<TargetEntry> = Vec::new();

        for record in metadata.packages.values() {
            let manifest_dir = record.manifest_path.as_path().parent();
            if !include_all_members && manifest_dir != Some(selected_path) {
                continue;
            }
            let source = if is_real_workspace && manifest_dir == Some(workspace_root) {
                TargetSource::workspace_root(record.name.clone())
            } else {
                TargetSource::member(record.name.clone())
            };

            for target in &record.targets {
                if target.kinds.contains(&TargetKind::Bin) && target.name == record.name {
                    binaries.push(TargetEntry {
                        name:              target.name.clone(),
                        display_name:      target.name.clone(),
                        run_target_kind:   RunTargetKind::Binary,
                        source:            source.clone(),
                        project_path:      project_path.clone(),
                        package_name:      record.name.clone(),
                        src_path:          target.src_path.clone(),
                        required_features: target.required_features.clone(),
                    });
                }
                if target.kinds.contains(&TargetKind::Example) {
                    let category =
                        example_category(manifest_dir, target.src_path.as_path(), &target.name);
                    let display_name = if category.is_empty() {
                        target.name.clone()
                    } else {
                        format!("{category}/{}", target.name)
                    };
                    examples.push(TargetEntry {
                        name: target.name.clone(),
                        display_name,
                        run_target_kind: RunTargetKind::Example,
                        source: source.clone(),
                        project_path: project_path.clone(),
                        package_name: record.name.clone(),
                        src_path: target.src_path.clone(),
                        required_features: target.required_features.clone(),
                    });
                }
                if target.kinds.contains(&TargetKind::Bench) {
                    benches.push(TargetEntry {
                        name:              target.name.clone(),
                        display_name:      target.name.clone(),
                        run_target_kind:   RunTargetKind::Bench,
                        source:            source.clone(),
                        project_path:      project_path.clone(),
                        package_name:      record.name.clone(),
                        src_path:          target.src_path.clone(),
                        required_features: target.required_features.clone(),
                    });
                }
            }
        }

        let mut data = Self {
            binaries,
            examples,
            benches,
        };
        data.sort_entries();
        data
    }
}

fn compare_target_name(a: &TargetEntry, b: &TargetEntry) -> Ordering {
    a.source
        .sort_key()
        .cmp(&b.source.sort_key())
        .then_with(|| a.name.cmp(&b.name))
}

fn compare_example_name(a: &TargetEntry, b: &TargetEntry) -> Ordering {
    a.source
        .sort_key()
        .cmp(&b.source.sort_key())
        .then_with(|| example_display_order(&a.display_name, &b.display_name))
}

/// Derive the example's category subdirectory from its `src_path`
/// relative to its package's manifest dir. `examples/<file>.rs` is
/// root-level (empty); `examples/<category>/<file>.rs` is categorized.
///
/// A subdirectory whose name equals `target_name` is the example's own
/// directory, not a grouping category: cargo names a multi-file
/// `examples/<name>/main.rs` example after its directory, so the
/// directory and the target name match. Treat that as root-level to
/// avoid a `<name>/<name>` display.
fn example_category(manifest_dir: Option<&Path>, src_path: &Path, target_name: &str) -> String {
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
            if parts.len() >= 3 && parts[1] != target_name {
                Some(parts[1].clone())
            } else {
                None
            }
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod test_row_tests {
    use crate::tui::panes::pane_data::TESTS_IGNORED_LABEL;
    use crate::tui::panes::pane_data::TESTS_TOTAL_LABEL;
    use crate::tui::panes::pane_data::TestCounts;
    use crate::tui::panes::pane_data::package_data;

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
        assert_eq!(
            package_data::test_rows_from_counts(counts(5, 0, 0, 0)),
            vec![("unit", 5)]
        );
    }

    #[test]
    fn multiple_buckets_append_runnable_total() {
        assert_eq!(
            package_data::test_rows_from_counts(counts(117, 48, 1185, 0)),
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
            package_data::test_rows_from_counts(counts(0, 0, 1185, 152)),
            vec![
                ("doc", 1185),
                (TESTS_IGNORED_LABEL, 152),
                (TESTS_TOTAL_LABEL, 1185),
            ]
        );
    }

    #[test]
    fn all_zero_counts_hide_the_section() {
        assert!(package_data::test_rows_from_counts(counts(0, 0, 0, 0)).is_empty());
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "tests should panic on unexpected values"
)]
mod target_list_tests {
    use super::AbsolutePath;
    use super::RunTargetKind;
    use super::TargetEntry;
    use super::TargetSource;
    use super::TargetsData;
    use super::build_target_list_from_data;

    fn entry(name: &str, run_target_kind: RunTargetKind) -> TargetEntry {
        TargetEntry {
            name: name.into(),
            display_name: name.into(),
            run_target_kind,
            source: TargetSource::workspace_root("demo".into()),
            project_path: AbsolutePath::from("/tmp"),
            package_name: "demo".into(),
            src_path: AbsolutePath::from(format!("/tmp/{name}.rs")),
            required_features: Vec::new(),
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
    fn preserves_input_order_binaries_then_examples_then_benches() {
        let data = data();
        let entries = build_target_list_from_data(&data);
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["a", "b", "c", "ex1", "bn1"]);
    }

    #[test]
    fn target_count_matches_the_flat_entry_list() {
        let data = data();
        assert_eq!(
            data.target_count(),
            build_target_list_from_data(&data).len()
        );
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
