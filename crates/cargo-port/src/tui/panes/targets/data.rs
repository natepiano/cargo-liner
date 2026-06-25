use std::cmp::Ordering;
use std::path::Component;
use std::path::Path;

use cargo_metadata::TargetKind;
use ratatui::style::Color;

use crate::project::AbsolutePath;
use crate::project::RootItem;
use crate::project::Visibility;
use crate::project::WorkspaceMetadata;
use crate::tui::app::App;
use crate::tui::constants::TARGET_KIND_BENCH_LABEL;
use crate::tui::constants::TARGET_KIND_BIN_LABEL;
use crate::tui::constants::TARGET_KIND_EXAMPLE_LABEL;
use crate::tui::theme_roles;

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

    fn append(&mut self, mut other: Self) {
        self.binaries.append(&mut other.binaries);
        self.examples.append(&mut other.examples);
        self.benches.append(&mut other.benches);
    }

    fn relabel_as_worktree(&mut self, checkout_name: &str) {
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

    fn sort_entries(&mut self) {
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

/// Look up the workspace that covers `abs_path` and aggregate its
/// runnable targets. Returns `TargetsData::default()` when no
/// metadata covers the path yet, so callers render an empty pane
/// instead of a hand-parsed view that disagrees with cargo's discovery
/// rules.
pub fn lookup_targets_data(
    app: &App,
    abs_path: &AbsolutePath,
    worktree_item: Option<&RootItem>,
) -> TargetsData {
    if let Some(data) = lookup_worktree_group_targets(app, worktree_item) {
        return data;
    }
    lookup_targets_data_for_path(app, abs_path)
}

fn lookup_worktree_group_targets(
    app: &App,
    worktree_item: Option<&RootItem>,
) -> Option<TargetsData> {
    let RootItem::Worktrees(group) = worktree_item? else {
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

#[cfg(test)]
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

#[cfg(test)]
mod targets_from_metadata {
    use std::collections::BTreeMap;
    use std::collections::HashMap;
    use std::path::PathBuf;

    use cargo_metadata::PackageId;
    use cargo_metadata::TargetKind;
    use cargo_metadata::semver::Version;

    use super::TargetSource;
    use super::TargetsData;
    use crate::project::AbsolutePath;
    use crate::project::FileStamp;
    use crate::project::ManifestFingerprint;
    use crate::project::PackageRecord;
    use crate::project::PublishPolicy;
    use crate::project::TargetRecord;
    use crate::project::WorkspaceMetadata;

    fn target(name: &str, kinds: Vec<TargetKind>, src_path: &str) -> TargetRecord {
        TargetRecord {
            name: name.into(),
            kinds,
            src_path: AbsolutePath::from(PathBuf::from(src_path)),
            required_features: Vec::new(),
        }
    }

    fn record(name: &str, manifest: &str, targets: Vec<TargetRecord>) -> PackageRecord {
        PackageRecord {
            name: name.into(),
            version: Version::new(0, 1, 0),
            edition: "2021".into(),
            description: None,
            license: None,
            homepage: None,
            repository: None,
            manifest_path: AbsolutePath::from(PathBuf::from(manifest)),
            targets,
            publish: PublishPolicy::Any,
        }
    }

    fn path(s: &str) -> AbsolutePath { AbsolutePath::from(PathBuf::from(s)) }

    fn workspace(workspace_root: &str, packages: Vec<PackageRecord>) -> WorkspaceMetadata {
        let root = AbsolutePath::from(PathBuf::from(workspace_root));
        let mut map: HashMap<PackageId, PackageRecord> = HashMap::new();
        for pkg in packages {
            let id = PackageId {
                repr: format!("{}-test-id", pkg.name),
            };
            map.insert(id, pkg);
        }
        WorkspaceMetadata {
            workspace_root:           root.clone(),
            target_directory:         AbsolutePath::from(root.as_path().join("target")),
            packages:                 map,
            fingerprint:              ManifestFingerprint {
                manifest:       FileStamp {
                    content_hash: [0_u8; 32],
                },
                lockfile:       None,
                rust_toolchain: None,
                configs:        BTreeMap::new(),
            },
            out_of_tree_target_bytes: None,
        }
    }

    #[test]
    fn groups_examples_by_subdirectory_and_sorts_root_first() {
        let pkg = record(
            "demo",
            "/ws/demo/Cargo.toml",
            vec![
                target("top", vec![TargetKind::Example], "/ws/demo/examples/top.rs"),
                target(
                    "draw",
                    vec![TargetKind::Example],
                    "/ws/demo/examples/2d/draw.rs",
                ),
                target(
                    "mesh",
                    vec![TargetKind::Example],
                    "/ws/demo/examples/3d/mesh.rs",
                ),
                target(
                    "cube",
                    vec![TargetKind::Example],
                    "/ws/demo/examples/3d/cube.rs",
                ),
            ],
        );
        let data = TargetsData::from_workspace_metadata(
            &workspace("/ws/demo", vec![pkg]),
            &path("/ws/demo"),
        );

        let display_names: Vec<&str> = data
            .examples
            .iter()
            .map(|e| e.display_name.as_str())
            .collect();
        assert_eq!(display_names, vec!["top", "2d/draw", "3d/cube", "3d/mesh"]);
    }

    #[test]
    fn multi_file_examples_are_not_categorized_by_their_own_directory() {
        let pkg = record(
            "bevy_window_manager",
            "/ws/bwm/Cargo.toml",
            vec![
                target(
                    "restore_window",
                    vec![TargetKind::Example],
                    "/ws/bwm/examples/restore_window/main.rs",
                ),
                target(
                    "custom_app_name",
                    vec![TargetKind::Example],
                    "/ws/bwm/examples/custom_app_name/main.rs",
                ),
            ],
        );
        let data = TargetsData::from_workspace_metadata(
            &workspace("/ws/bwm", vec![pkg]),
            &path("/ws/bwm"),
        );

        let display_names: Vec<&str> = data
            .examples
            .iter()
            .map(|e| e.display_name.as_str())
            .collect();
        assert_eq!(display_names, vec!["custom_app_name", "restore_window"]);
    }

    #[test]
    fn surfaces_benches_flat_and_sorted() {
        let pkg = record(
            "demo",
            "/ws/demo/Cargo.toml",
            vec![
                target(
                    "b_zed",
                    vec![TargetKind::Bench],
                    "/ws/demo/benches/b_zed.rs",
                ),
                target(
                    "a_alpha",
                    vec![TargetKind::Bench],
                    "/ws/demo/benches/a_alpha.rs",
                ),
            ],
        );
        let data = TargetsData::from_workspace_metadata(
            &workspace("/ws/demo", vec![pkg]),
            &path("/ws/demo"),
        );
        let names: Vec<&str> = data.benches.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["a_alpha", "b_zed"]);
    }

    #[test]
    fn standalone_package_uses_package_name_as_source_label() {
        // bevy_liminal etc. - a project with no `[workspace]` table
        // and a single package. Cargo still reports it with a
        // workspace_root pointing at the package dir, but the
        // Source column must say the package name, not "workspace".
        let pkg = record(
            "bevy_liminal",
            "/repo/bevy_liminal/Cargo.toml",
            vec![target(
                "bevy_liminal",
                vec![TargetKind::Bin],
                "/repo/bevy_liminal/src/main.rs",
            )],
        );
        let data = TargetsData::from_workspace_metadata(
            &workspace("/repo/bevy_liminal", vec![pkg]),
            &path("/repo/bevy_liminal"),
        );
        assert_eq!(data.binaries.len(), 1);
        assert_eq!(
            data.binaries[0].source,
            TargetSource::member("bevy_liminal".into())
        );
    }

    #[test]
    fn primary_binary_matches_package_name_only() {
        // A bin target named "demo" is considered the default-run
        // binary; other bin targets are not lifted into the
        // workspace-aggregated binary list.
        let with_match = record(
            "demo",
            "/ws/demo/Cargo.toml",
            vec![target(
                "demo",
                vec![TargetKind::Bin],
                "/ws/demo/src/main.rs",
            )],
        );
        let data = TargetsData::from_workspace_metadata(
            &workspace("/ws/demo", vec![with_match]),
            &path("/ws/demo"),
        );
        assert_eq!(data.binaries.len(), 1);
        assert_eq!(data.binaries[0].name, "demo");

        let without_match = record(
            "demo",
            "/ws/demo/Cargo.toml",
            vec![target(
                "other",
                vec![TargetKind::Bin],
                "/ws/demo/src/bin/other.rs",
            )],
        );
        let data = TargetsData::from_workspace_metadata(
            &workspace("/ws/demo", vec![without_match]),
            &path("/ws/demo"),
        );
        assert!(data.binaries.is_empty());
    }

    #[test]
    fn ignores_non_example_non_bench_non_bin_kinds() {
        let pkg = record(
            "demo",
            "/ws/demo/Cargo.toml",
            vec![
                target("demo", vec![TargetKind::Lib], "/ws/demo/src/lib.rs"),
                target("it", vec![TargetKind::Test], "/ws/demo/tests/it.rs"),
                target(
                    "build-script",
                    vec![TargetKind::CustomBuild],
                    "/ws/demo/build.rs",
                ),
            ],
        );
        let data = TargetsData::from_workspace_metadata(
            &workspace("/ws/demo", vec![pkg]),
            &path("/ws/demo"),
        );
        assert!(data.binaries.is_empty());
        assert!(data.examples.is_empty());
        assert!(data.benches.is_empty());
    }

    /// Three-package workspace: root "ws-root" plus members "core"
    /// and "engine". Used by both the workspace-root and member-filter
    /// tests below.
    fn three_package_workspace() -> WorkspaceMetadata {
        let ws_root = record(
            "ws-root",
            "/ws/Cargo.toml",
            vec![
                target("ws-root", vec![TargetKind::Bin], "/ws/src/main.rs"),
                target(
                    "root-ex",
                    vec![TargetKind::Example],
                    "/ws/examples/root-ex.rs",
                ),
            ],
        );
        let core = record(
            "core",
            "/ws/crates/core/Cargo.toml",
            vec![
                target("core", vec![TargetKind::Bin], "/ws/crates/core/src/main.rs"),
                target(
                    "core-ex",
                    vec![TargetKind::Example],
                    "/ws/crates/core/examples/core-ex.rs",
                ),
            ],
        );
        let engine = record(
            "engine",
            "/ws/crates/engine/Cargo.toml",
            vec![target(
                "engine-ex",
                vec![TargetKind::Example],
                "/ws/crates/engine/examples/engine-ex.rs",
            )],
        );
        workspace("/ws", vec![ws_root, core, engine])
    }

    #[test]
    fn aggregates_targets_across_root_and_members_when_selected_is_workspace_root() {
        let metadata = three_package_workspace();
        let data = TargetsData::from_workspace_metadata(&metadata, &path("/ws"));

        let binary_sources: Vec<&TargetSource> = data.binaries.iter().map(|e| &e.source).collect();
        assert!(binary_sources.contains(&&TargetSource::workspace_root("ws-root".into())));
        assert!(binary_sources.contains(&&TargetSource::member("core".into())));
        assert_eq!(data.binaries.len(), 2);
        assert_eq!(
            data.examples[0].source,
            TargetSource::workspace_root("ws-root".into())
        );
        assert_eq!(data.examples[0].source.label(), "ws-root");
        assert_eq!(data.examples[0].name, "root-ex");
        assert_eq!(data.examples[1].source, TargetSource::member("core".into()));
        assert_eq!(
            data.examples[2].source,
            TargetSource::member("engine".into())
        );
    }

    #[test]
    fn filters_to_member_when_selected_is_a_member_path() {
        // When the selected project is a workspace member, the
        // Targets pane shows only that member's targets - selecting
        // sibling members or the workspace root surfaces a different
        // view. Confirms the user-visible "narrow on member" rule.
        let metadata = three_package_workspace();
        let data = TargetsData::from_workspace_metadata(&metadata, &path("/ws/crates/core"));

        assert_eq!(data.binaries.len(), 1, "only core's bin shows");
        assert_eq!(data.binaries[0].name, "core");
        assert_eq!(data.binaries[0].source, TargetSource::member("core".into()));
        assert_eq!(data.examples.len(), 1);
        assert_eq!(data.examples[0].name, "core-ex");
        assert!(
            data.examples
                .iter()
                .all(|e| e.source.is_member_named("core"))
        );
    }

    #[test]
    fn member_filter_returns_empty_for_unknown_path() {
        // A selected path that doesn't match any member's manifest
        // dir produces an empty pane rather than falling back to the
        // workspace aggregation - selection must be unambiguous.
        let metadata = three_package_workspace();
        let data = TargetsData::from_workspace_metadata(&metadata, &path("/ws/crates/unknown"));

        assert!(data.binaries.is_empty());
        assert!(data.examples.is_empty());
        assert!(data.benches.is_empty());
    }

    #[test]
    fn virtual_workspace_has_no_workspace_source() {
        // No root package - only members. Selecting the workspace
        // root still aggregates both members, but no entry maps to
        // No entry uses the workspace-root source kind.
        let m1 = record(
            "m1",
            "/ws/crates/m1/Cargo.toml",
            vec![target(
                "m1-ex",
                vec![TargetKind::Example],
                "/ws/crates/m1/examples/m1-ex.rs",
            )],
        );
        let m2 = record(
            "m2",
            "/ws/crates/m2/Cargo.toml",
            vec![target(
                "m2-ex",
                vec![TargetKind::Example],
                "/ws/crates/m2/examples/m2-ex.rs",
            )],
        );
        let data =
            TargetsData::from_workspace_metadata(&workspace("/ws", vec![m1, m2]), &path("/ws"));

        assert!(data.examples.iter().all(|e| !e.source.is_workspace_root()));
        assert_eq!(data.examples.len(), 2);
    }
}
