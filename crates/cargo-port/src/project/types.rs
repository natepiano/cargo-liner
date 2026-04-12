use std::path::Path;
use std::path::PathBuf;

use super::cargo::ExampleGroup;
use super::cargo::ProjectType;
use super::git;
use super::git::GitInfo;
use super::paths;
use super::paths::AbsolutePath;
use super::paths::DisplayPath;
use super::paths::PackageName;
use super::paths::RootDirectoryName;

// ── Shared enums ─────────────────────────────────────────────────────

/// Visibility state for projects and worktree groups.
/// Progression: `Visible -> Deleted -> Dismissed`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Visibility {
    #[default]
    Visible,
    Deleted,
    Dismissed,
}

/// Whether a worktree's `.git` file points to a valid gitdir.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum WorktreeHealth {
    /// Not a worktree, or health not yet checked.
    #[default]
    Normal,
    /// The `.git` file's gitdir target does not exist on disk.
    Broken,
}

// ── Traits ───────────────────────────────────────────────────────────

/// Marker trait — only `Workspace` and `Package` implement this.
pub(crate) trait CargoKind: Clone + 'static {}

/// Per-node info access for hierarchy leaf types.
pub(crate) trait InfoProvider {
    fn info(&self) -> &ProjectInfo;
    fn info_mut(&mut self) -> &mut ProjectInfo;
}

// ── Kind markers ─────────────────────────────────────────────────────

/// Workspace carries member groups.
#[derive(Clone)]
pub(crate) struct Workspace {
    groups: Vec<MemberGroup>,
}

impl Workspace {
    pub(crate) const fn new(groups: Vec<MemberGroup>) -> Self { Self { groups } }
}

impl CargoKind for Workspace {}

#[derive(Clone)]
pub(crate) struct Package;

impl CargoKind for Package {}

// ── Cargo struct ─────────────────────────────────────────────────────

/// Shared Cargo fields extracted from `Cargo.toml`.
#[derive(Clone, Debug)]
pub(crate) struct Cargo {
    version:     Option<String>,
    description: Option<String>,
    types:       Vec<ProjectType>,
    examples:    Vec<ExampleGroup>,
    benches:     Vec<String>,
    test_count:  usize,
}

impl Cargo {
    pub(crate) const fn new(
        version: Option<String>,
        description: Option<String>,
        types: Vec<ProjectType>,
        examples: Vec<ExampleGroup>,
        benches: Vec<String>,
        test_count: usize,
    ) -> Self {
        Self {
            version,
            description,
            types,
            examples,
            benches,
            test_count,
        }
    }

    pub(crate) fn types(&self) -> &[ProjectType] { &self.types }

    pub(crate) fn examples(&self) -> &[ExampleGroup] { &self.examples }

    pub(crate) fn benches(&self) -> &[String] { &self.benches }

    pub(crate) fn version(&self) -> Option<&str> { self.version.as_deref() }

    pub(crate) fn description(&self) -> Option<&str> { self.description.as_deref() }

    pub(crate) const fn test_count(&self) -> usize { self.test_count }

    pub(crate) fn example_count(&self) -> usize {
        self.examples.iter().map(|g| g.names.len()).sum()
    }

    pub(crate) fn is_binary(&self) -> bool {
        self.types.iter().any(|t| matches!(t, ProjectType::Binary))
    }
}

// ── ProjectInfo ──────────────────────────────────────────────────────

#[derive(Clone, Default)]
pub(crate) struct ProjectInfo {
    pub disk_usage_bytes: Option<u64>,
    pub git_info:         Option<GitInfo>,
    pub visibility:       Visibility,
    pub worktree_health:  WorktreeHealth,
}

// ── RustProject ──────────────────────────────────────────────────────

/// The core project type, parameterized by kind.
/// Private fields with accessors enforce what's available per kind.
pub(crate) struct RustProject<Kind: CargoKind> {
    /// Absolute filesystem path to this project leaf's root directory.
    path:                      AbsolutePath,
    /// Package or workspace name from Cargo metadata when available.
    name:                      Option<String>,
    /// Shared leaf-attached project metadata available across project kinds.
    info:                      ProjectInfo,
    /// Rust-specific Cargo metadata and derived target information.
    cargo:                     Cargo,
    /// Vendored Rust dependencies nested under this project leaf.
    vendored:                  Vec<RustProject<Package>>,
    /// Marker for whether this leaf is a workspace or a package.
    kind:                      Kind,
    /// Worktree label shown in the UI for linked worktree checkouts.
    worktree_name:             Option<String>,
    /// Absolute path to the primary checkout root for this worktree family.
    ///
    /// This is leaf metadata used to identify and regroup related worktrees.
    worktree_primary_abs_path: Option<AbsolutePath>,
}

impl Clone for RustProject<Workspace> {
    fn clone(&self) -> Self {
        Self {
            path:                      self.path.clone(),
            name:                      self.name.clone(),
            info:                      self.info.clone(),
            cargo:                     self.cargo.clone(),
            vendored:                  self.vendored.clone(),
            kind:                      self.kind.clone(),
            worktree_name:             self.worktree_name.clone(),
            worktree_primary_abs_path: self.worktree_primary_abs_path.clone(),
        }
    }
}

impl Clone for RustProject<Package> {
    fn clone(&self) -> Self {
        Self {
            path:                      self.path.clone(),
            name:                      self.name.clone(),
            info:                      self.info.clone(),
            cargo:                     self.cargo.clone(),
            vendored:                  self.vendored.clone(),
            kind:                      Package,
            worktree_name:             self.worktree_name.clone(),
            worktree_primary_abs_path: self.worktree_primary_abs_path.clone(),
        }
    }
}

impl<Kind: CargoKind> InfoProvider for RustProject<Kind> {
    fn info(&self) -> &ProjectInfo { &self.info }

    fn info_mut(&mut self) -> &mut ProjectInfo { &mut self.info }
}

// Shared accessors for all `CargoKind` projects.
impl<Kind: CargoKind> RustProject<Kind> {
    pub(crate) fn path(&self) -> &Path { self.path.as_path() }

    pub(crate) fn name(&self) -> Option<&str> { self.name.as_deref() }

    pub(crate) const fn visibility(&self) -> Visibility { self.info.visibility }

    pub(crate) const fn worktree_health(&self) -> WorktreeHealth { self.info.worktree_health }

    pub(crate) const fn disk_usage_bytes(&self) -> Option<u64> { self.info.disk_usage_bytes }

    pub(crate) const fn git_info(&self) -> Option<&GitInfo> { self.info.git_info.as_ref() }

    /// Display path: `~/`-prefixed for home-relative, otherwise absolute.
    pub(crate) fn display_path(&self) -> DisplayPath { self.path.display_path() }

    pub(crate) const fn cargo(&self) -> &Cargo { &self.cargo }

    pub(crate) fn vendored(&self) -> &[RustProject<Package>] { &self.vendored }

    pub(crate) const fn vendored_mut(&mut self) -> &mut Vec<RustProject<Package>> {
        &mut self.vendored
    }

    pub(crate) fn worktree_name(&self) -> Option<&str> { self.worktree_name.as_deref() }

    pub(crate) fn worktree_primary_abs_path(&self) -> Option<&Path> {
        self.worktree_primary_abs_path
            .as_ref()
            .map(AbsolutePath::as_path)
    }

    /// Directory leaf name for top-level root labels and disambiguation.
    pub(crate) fn root_directory_name(&self) -> RootDirectoryName {
        RootDirectoryName(paths::directory_leaf(self.path.as_path()))
    }

    /// Cargo package name when present, otherwise directory leaf.
    /// Used for member rows, vendored rows, detail title bars, and finder parent labels.
    pub(crate) fn package_name(&self) -> PackageName {
        PackageName(self.name.as_deref().map_or_else(
            || paths::directory_leaf(self.path.as_path()),
            str::to_string,
        ))
    }
}

// Workspace-specific accessors.
impl RustProject<Workspace> {
    pub(crate) fn groups(&self) -> &[MemberGroup] { &self.kind.groups }

    pub(crate) const fn groups_mut(&mut self) -> &mut Vec<MemberGroup> { &mut self.kind.groups }

    pub(crate) fn new(
        path: PathBuf,
        name: Option<String>,
        cargo: Cargo,
        groups: Vec<MemberGroup>,
        vendored: Vec<RustProject<Package>>,
        worktree_name: Option<String>,
        worktree_primary_abs_path: Option<PathBuf>,
    ) -> Self {
        Self {
            path: path.into(),
            name,
            info: ProjectInfo::default(),
            cargo,
            vendored,
            kind: Workspace::new(groups),
            worktree_name,
            worktree_primary_abs_path: worktree_primary_abs_path.map(AbsolutePath::from),
        }
    }

    pub(crate) fn has_members(&self) -> bool {
        self.kind.groups.iter().any(|g| !g.members().is_empty())
    }

    /// Language icon for the project list.
    pub(crate) const fn lang_icon() -> &'static str { "\u{1f980}" }
}

// Package-specific accessors.
impl RustProject<Package> {
    pub(crate) fn new(
        path: PathBuf,
        name: Option<String>,
        cargo: Cargo,
        vendored: Vec<Self>,
        worktree_name: Option<String>,
        worktree_primary_abs_path: Option<PathBuf>,
    ) -> Self {
        Self {
            path: path.into(),
            name,
            info: ProjectInfo::default(),
            cargo,
            vendored,
            kind: Package,
            worktree_name,
            worktree_primary_abs_path: worktree_primary_abs_path.map(AbsolutePath::from),
        }
    }

    /// Language icon for the project list.
    pub(crate) const fn lang_icon() -> &'static str { "\u{1f980}" }
}

// ── NonRustProject ───────────────────────────────────────────────────

/// A non-Rust project. Separate struct, no generic parameter.
pub(crate) struct NonRustProject {
    /// Absolute filesystem path to this project's root directory.
    path: AbsolutePath,
    /// Directory-derived project name used for display and search.
    name: Option<String>,
    /// Shared leaf-attached project metadata available across project kinds.
    info: ProjectInfo,
}

impl Clone for NonRustProject {
    fn clone(&self) -> Self {
        Self {
            path: self.path.clone(),
            name: self.name.clone(),
            info: self.info.clone(),
        }
    }
}

impl InfoProvider for NonRustProject {
    fn info(&self) -> &ProjectInfo { &self.info }
    fn info_mut(&mut self) -> &mut ProjectInfo { &mut self.info }
}

impl NonRustProject {
    pub(crate) fn new(path: PathBuf, name: Option<String>) -> Self {
        Self {
            path: path.into(),
            name,
            info: ProjectInfo::default(),
        }
    }

    pub(crate) fn path(&self) -> &Path { self.path.as_path() }

    pub(crate) fn name(&self) -> Option<&str> { self.name.as_deref() }

    pub(crate) const fn visibility(&self) -> Visibility { self.info.visibility }

    pub(crate) const fn worktree_health(&self) -> WorktreeHealth { self.info.worktree_health }

    pub(crate) const fn disk_usage_bytes(&self) -> Option<u64> { self.info.disk_usage_bytes }

    pub(crate) const fn git_info(&self) -> Option<&GitInfo> { self.info.git_info.as_ref() }

    /// Display path: `~/`-prefixed for home-relative, otherwise absolute.
    pub(crate) fn display_path(&self) -> DisplayPath { self.path.display_path() }

    /// Directory leaf name for top-level root labels and disambiguation.
    pub(crate) fn root_directory_name(&self) -> RootDirectoryName {
        RootDirectoryName(paths::directory_leaf(self.path.as_path()))
    }

    /// Language icon for the project list.
    pub(crate) const fn lang_icon() -> &'static str { "  " }
}

// ── WorktreeGroup ────────────────────────────────────────────────────

/// A generic worktree group: primary + linked checkouts.
pub(crate) struct WorktreeGroup<Kind: CargoKind> {
    primary:    RustProject<Kind>,
    linked:     Vec<RustProject<Kind>>,
    visibility: Visibility,
}

impl<Kind: CargoKind> WorktreeGroup<Kind> {
    pub(crate) fn new(primary: RustProject<Kind>, linked: Vec<RustProject<Kind>>) -> Self {
        Self {
            primary,
            linked,
            visibility: Visibility::default(),
        }
    }

    pub(crate) const fn primary(&self) -> &RustProject<Kind> { &self.primary }

    pub(crate) const fn primary_mut(&mut self) -> &mut RustProject<Kind> { &mut self.primary }

    pub(crate) fn linked(&self) -> &[RustProject<Kind>] { &self.linked }

    pub(crate) const fn linked_mut(&mut self) -> &mut Vec<RustProject<Kind>> { &mut self.linked }

    pub(crate) const fn visibility(&self) -> Visibility { self.visibility }

    pub(crate) fn live_entry_count(&self) -> usize {
        std::iter::once(self.primary.visibility())
            .chain(self.linked.iter().map(RustProject::visibility))
            .filter(|visibility| !matches!(visibility, Visibility::Dismissed))
            .count()
    }

    pub(crate) fn renders_as_group(&self) -> bool { self.live_entry_count() > 1 }

    pub(crate) fn single_live(&self) -> Option<&RustProject<Kind>> {
        if self.live_entry_count() != 1 {
            return None;
        }

        std::iter::once(&self.primary)
            .chain(self.linked.iter())
            .find(|project| !matches!(project.visibility(), Visibility::Dismissed))
    }
}

impl Clone for WorktreeGroup<Workspace> {
    fn clone(&self) -> Self {
        Self {
            primary:    self.primary.clone(),
            linked:     self.linked.clone(),
            visibility: self.visibility,
        }
    }
}

impl Clone for WorktreeGroup<Package> {
    fn clone(&self) -> Self {
        Self {
            primary:    self.primary.clone(),
            linked:     self.linked.clone(),
            visibility: self.visibility,
        }
    }
}

// ── RootItem ─────────────────────────────────────────────────────────

/// The top-level enum for the project list.
pub(crate) enum RootItem {
    Workspace(RustProject<Workspace>),
    Package(RustProject<Package>),
    NonRust(NonRustProject),
    WorkspaceWorktrees(WorktreeGroup<Workspace>),
    PackageWorktrees(WorktreeGroup<Package>),
}

impl Clone for RootItem {
    fn clone(&self) -> Self {
        match self {
            Self::Workspace(p) => Self::Workspace(p.clone()),
            Self::Package(p) => Self::Package(p.clone()),
            Self::NonRust(p) => Self::NonRust(p.clone()),
            Self::WorkspaceWorktrees(g) => Self::WorkspaceWorktrees(g.clone()),
            Self::PackageWorktrees(g) => Self::PackageWorktrees(g.clone()),
        }
    }
}

impl RootItem {
    pub(crate) const fn visibility(&self) -> Visibility {
        match self {
            Self::Workspace(p) => p.visibility(),
            Self::Package(p) => p.visibility(),
            Self::NonRust(p) => p.visibility(),
            Self::WorkspaceWorktrees(g) => g.visibility(),
            Self::PackageWorktrees(g) => g.visibility(),
        }
    }

    pub(crate) const fn worktree_health(&self) -> WorktreeHealth {
        match self {
            Self::Workspace(p) => p.worktree_health(),
            Self::Package(p) => p.worktree_health(),
            Self::NonRust(p) => p.worktree_health(),
            Self::WorkspaceWorktrees(g) => g.primary().worktree_health(),
            Self::PackageWorktrees(g) => g.primary().worktree_health(),
        }
    }

    /// Absolute path to the primary project root.
    pub(crate) fn path(&self) -> &Path {
        match self {
            Self::Workspace(p) => p.path(),
            Self::Package(p) => p.path(),
            Self::NonRust(p) => p.path(),
            Self::WorkspaceWorktrees(g) => g.primary().path(),
            Self::PackageWorktrees(g) => g.primary().path(),
        }
    }

    pub(crate) fn name(&self) -> Option<&str> {
        match self {
            Self::Workspace(p) => p.name(),
            Self::Package(p) => p.name(),
            Self::NonRust(p) => p.name(),
            Self::WorkspaceWorktrees(g) => g.primary().name(),
            Self::PackageWorktrees(g) => g.primary().name(),
        }
    }

    pub(crate) fn display_path(&self) -> DisplayPath {
        match self {
            Self::Workspace(p) => p.display_path(),
            Self::Package(p) => p.display_path(),
            Self::NonRust(p) => p.display_path(),
            Self::WorkspaceWorktrees(g) => g.primary().display_path(),
            Self::PackageWorktrees(g) => g.primary().display_path(),
        }
    }

    pub(crate) fn git_directory(&self) -> Option<AbsolutePath> {
        let project_path = match self {
            Self::Workspace(project) => project.path(),
            Self::Package(project) => project.path(),
            Self::NonRust(project) => project.path(),
            Self::WorkspaceWorktrees(group) => group.primary().path(),
            Self::PackageWorktrees(group) => group.primary().path(),
        };
        git::resolve_git_dir(project_path).map(AbsolutePath::from)
    }

    /// Directory leaf name for top-level root labels and disambiguation.
    pub(crate) fn root_directory_name(&self) -> RootDirectoryName {
        match self {
            Self::Workspace(p) => p.root_directory_name(),
            Self::Package(p) => p.root_directory_name(),
            Self::NonRust(p) => p.root_directory_name(),
            Self::WorkspaceWorktrees(g) => g.primary().root_directory_name(),
            Self::PackageWorktrees(g) => g.primary().root_directory_name(),
        }
    }

    pub(crate) fn worktree_badge_suffix(&self) -> Option<String> {
        let live_worktrees = match self {
            Self::WorkspaceWorktrees(group) if group.renders_as_group() => group.live_entry_count(),
            Self::PackageWorktrees(group) if group.renders_as_group() => group.live_entry_count(),
            Self::Workspace(_)
            | Self::Package(_)
            | Self::NonRust(_)
            | Self::WorkspaceWorktrees(_)
            | Self::PackageWorktrees(_) => 0,
        };
        (live_worktrees > 0).then(|| format!(" {}:{live_worktrees}", crate::constants::WORKTREE))
    }

    /// Whether this item has expandable children.
    pub(crate) fn has_children(&self) -> bool {
        match self {
            Self::Workspace(ws) => {
                ws.groups().iter().any(|g| !g.members().is_empty()) || !ws.vendored().is_empty()
            },
            Self::Package(pkg) => !pkg.vendored().is_empty(),
            Self::NonRust(_) => false,
            Self::WorkspaceWorktrees(g) => {
                if g.renders_as_group() {
                    true
                } else {
                    g.single_live().is_some_and(|workspace| {
                        workspace.has_members() || !workspace.vendored().is_empty()
                    })
                }
            },
            Self::PackageWorktrees(g) => {
                if g.renders_as_group() {
                    true
                } else {
                    g.single_live()
                        .is_some_and(|package| !package.vendored().is_empty())
                }
            },
        }
    }

    /// Language icon for the project list.
    pub(crate) const fn lang_icon(&self) -> &'static str {
        match self {
            Self::Workspace(_) | Self::WorkspaceWorktrees(_) => {
                RustProject::<Workspace>::lang_icon()
            },
            Self::Package(_) | Self::PackageWorktrees(_) => RustProject::<Package>::lang_icon(),
            Self::NonRust(_) => NonRustProject::lang_icon(),
        }
    }

    /// Whether this is a Rust project (has Cargo.toml).
    pub(crate) const fn is_rust(&self) -> bool {
        matches!(
            self,
            Self::Workspace(_)
                | Self::Package(_)
                | Self::WorkspaceWorktrees(_)
                | Self::PackageWorktrees(_)
        )
    }

    /// Disk usage for this item. Worktree groups sum primary + linked.
    pub(crate) fn disk_usage_bytes(&self) -> Option<u64> {
        match self {
            Self::Workspace(p) => p.disk_usage_bytes(),
            Self::Package(p) => p.disk_usage_bytes(),
            Self::NonRust(p) => p.disk_usage_bytes(),
            Self::WorkspaceWorktrees(g) => sum_worktree_disk(g.primary(), g.linked()),
            Self::PackageWorktrees(g) => sum_worktree_disk(g.primary(), g.linked()),
        }
    }

    pub(crate) const fn git_info(&self) -> Option<&GitInfo> {
        match self {
            Self::Workspace(p) => p.git_info(),
            Self::Package(p) => p.git_info(),
            Self::NonRust(p) => p.git_info(),
            Self::WorkspaceWorktrees(g) => g.primary().git_info(),
            Self::PackageWorktrees(g) => g.primary().git_info(),
        }
    }

    pub(crate) fn at_path(&self, path: &Path) -> Option<&ProjectInfo> {
        match self {
            Self::Workspace(p) => info_in_workspace(p, path),
            Self::Package(p) => info_in_package(p, path),
            Self::NonRust(p) => (p.path() == path).then(|| p.info()),
            Self::WorkspaceWorktrees(g) => info_in_workspace(g.primary(), path).or_else(|| {
                g.linked()
                    .iter()
                    .find_map(|linked| info_in_workspace(linked, path))
            }),
            Self::PackageWorktrees(g) => info_in_package(g.primary(), path).or_else(|| {
                g.linked()
                    .iter()
                    .find_map(|linked| info_in_package(linked, path))
            }),
        }
    }

    pub(crate) fn at_path_mut(&mut self, path: &Path) -> Option<&mut ProjectInfo> {
        match self {
            Self::Workspace(p) => info_in_workspace_mut(p, path),
            Self::Package(p) => info_in_package_mut(p, path),
            Self::NonRust(p) => (p.path() == path).then(|| p.info_mut()),
            Self::WorkspaceWorktrees(g) => {
                if info_in_workspace(g.primary(), path).is_some() {
                    return info_in_workspace_mut(g.primary_mut(), path);
                }
                let linked_index = g
                    .linked()
                    .iter()
                    .position(|linked| info_in_workspace(linked, path).is_some())?;
                info_in_workspace_mut(&mut g.linked_mut()[linked_index], path)
            },
            Self::PackageWorktrees(g) => {
                if info_in_package(g.primary(), path).is_some() {
                    return info_in_package_mut(g.primary_mut(), path);
                }
                let linked_index = g
                    .linked()
                    .iter()
                    .position(|linked| info_in_package(linked, path).is_some())?;
                info_in_package_mut(&mut g.linked_mut()[linked_index], path)
            },
        }
    }

    pub(crate) fn collect_project_info(&self) -> Vec<(PathBuf, ProjectInfo)> {
        let mut out = Vec::new();
        match self {
            Self::Workspace(p) => collect_project_info_from_workspace(p, &mut out),
            Self::Package(p) => collect_project_info_from_package(p, &mut out),
            Self::NonRust(p) => collect_project_info_from_non_rust(p, &mut out),
            Self::WorkspaceWorktrees(g) => {
                collect_project_info_from_workspace(g.primary(), &mut out);
                for linked in g.linked() {
                    collect_project_info_from_workspace(linked, &mut out);
                }
            },
            Self::PackageWorktrees(g) => {
                collect_project_info_from_package(g.primary(), &mut out);
                for linked in g.linked() {
                    collect_project_info_from_package(linked, &mut out);
                }
            },
        }
        out
    }
}

// ── RootItem helpers ─────────────────────────────────────────────────

fn sum_worktree_disk<Kind: CargoKind>(
    primary: &RustProject<Kind>,
    linked: &[RustProject<Kind>],
) -> Option<u64> {
    let mut total = 0u64;
    let mut any = false;
    for p in std::iter::once(primary).chain(linked) {
        if let Some(b) = p.disk_usage_bytes() {
            total += b;
            any = true;
        }
    }
    any.then_some(total)
}

fn info_in_workspace<'a>(ws: &'a RustProject<Workspace>, path: &Path) -> Option<&'a ProjectInfo> {
    if ws.path() == path {
        return Some(ws.info());
    }
    for group in ws.groups() {
        for member in group.members() {
            if member.path() == path {
                return Some(member.info());
            }
        }
    }
    for vendored in ws.vendored() {
        if vendored.path() == path {
            return Some(vendored.info());
        }
    }
    None
}

fn info_in_package<'a>(pkg: &'a RustProject<Package>, path: &Path) -> Option<&'a ProjectInfo> {
    if pkg.path() == path {
        return Some(pkg.info());
    }
    for vendored in pkg.vendored() {
        if vendored.path() == path {
            return Some(vendored.info());
        }
    }
    None
}

fn info_in_workspace_mut<'a>(
    ws: &'a mut RustProject<Workspace>,
    path: &Path,
) -> Option<&'a mut ProjectInfo> {
    if ws.path() == path {
        return Some(ws.info_mut());
    }
    let member_index = ws
        .groups()
        .iter()
        .enumerate()
        .find_map(|(group_index, group)| {
            group
                .members()
                .iter()
                .position(|member| member.path() == path)
                .map(|member_index| (group_index, member_index))
        });
    if let Some((group_index, member_index)) = member_index {
        return Some(ws.groups_mut()[group_index].members_mut()[member_index].info_mut());
    }
    let vendored_index = ws
        .vendored()
        .iter()
        .position(|vendored| vendored.path() == path);
    if let Some(vendored_index) = vendored_index {
        return Some(ws.vendored_mut()[vendored_index].info_mut());
    }
    None
}

fn info_in_package_mut<'a>(
    pkg: &'a mut RustProject<Package>,
    path: &Path,
) -> Option<&'a mut ProjectInfo> {
    if pkg.path() == path {
        return Some(pkg.info_mut());
    }
    for vendored in pkg.vendored_mut() {
        if vendored.path() == path {
            return Some(vendored.info_mut());
        }
    }
    None
}

fn collect_project_info_from_workspace(
    ws: &RustProject<Workspace>,
    out: &mut Vec<(PathBuf, ProjectInfo)>,
) {
    out.push((ws.path().to_path_buf(), ws.info().clone()));
    for group in ws.groups() {
        for member in group.members() {
            out.push((member.path().to_path_buf(), member.info().clone()));
        }
    }
    for vendored in ws.vendored() {
        out.push((vendored.path().to_path_buf(), vendored.info().clone()));
    }
}

fn collect_project_info_from_package(
    pkg: &RustProject<Package>,
    out: &mut Vec<(PathBuf, ProjectInfo)>,
) {
    out.push((pkg.path().to_path_buf(), pkg.info().clone()));
    for vendored in pkg.vendored() {
        out.push((vendored.path().to_path_buf(), vendored.info().clone()));
    }
}

fn collect_project_info_from_non_rust(
    project: &NonRustProject,
    out: &mut Vec<(PathBuf, ProjectInfo)>,
) {
    out.push((project.path().to_path_buf(), project.info().clone()));
}

// ── MemberGroup ──────────────────────────────────────────────────────
// Re-exported from the dedicated submodule.
pub(crate) use super::member_group::MemberGroup;
