use crate::ci::CiRun;
use crate::project::GitInfo;
use crate::project::ProjectType;
use crate::project::RustProject;

#[derive(Default, PartialEq, Eq, Clone, Copy)]
pub enum FocusTarget {
    #[default]
    ProjectList,
    DetailFields,
    CiRuns,
    ScanLog,
}

/// An expand key: either a workspace node or a group within a node.
#[derive(Hash, Eq, PartialEq, Clone)]
pub enum ExpandKey {
    Node(usize),
    Group(usize, usize),
}

/// What a visible row represents.
#[derive(Clone, Copy)]
pub enum VisibleRow {
    /// A top-level project/workspace root.
    Root { node_index: usize },
    /// A group header (e.g., "examples").
    GroupHeader {
        node_index:  usize,
        group_index: usize,
    },
    /// An actual project member.
    Member {
        node_index:   usize,
        group_index:  usize,
        member_index: usize,
    },
    /// A worktree entry shown directly under the parent node.
    WorktreeEntry {
        node_index:     usize,
        worktree_index: usize,
    },
}

/// Members within a workspace are organized into groups by their first subdirectory.
/// The "inline" group (empty name) contains members directly under the workspace root
/// or under the primary `crates/` directory — these are shown without a folder header.
pub struct MemberGroup {
    pub name:    String,
    pub members: Vec<RustProject>,
}

pub struct ProjectNode {
    pub project:   RustProject,
    pub groups:    Vec<MemberGroup>,
    pub worktrees: Vec<Self>,
    pub vendored:  Vec<RustProject>,
}

impl ProjectNode {
    pub fn has_members(&self) -> bool { self.groups.iter().any(|g| !g.members.is_empty()) }

    pub fn has_children(&self) -> bool {
        self.has_members() || !self.worktrees.is_empty() || !self.vendored.is_empty()
    }
}

/// A flattened entry for fuzzy search.
pub struct FlatEntry {
    pub node_index:   usize,
    pub group_index:  usize,
    pub member_index: usize,
    pub name:         String,
}

pub enum ExampleMsg {
    Output(String),
    Finished,
}

pub enum BackgroundMsg {
    DiskUsage { path: String, bytes: u64 },
    CiRuns { path: String, runs: Vec<CiRun> },
    GitInfo { path: String, info: GitInfo },
    CratesIoVersion { path: String, version: String },
    Stars { path: String, count: u64 },
    ProjectDiscovered { project: RustProject },
    ScanActivity { path: String },
    ScanComplete,
}

impl BackgroundMsg {
    /// Returns the project path this message relates to, if any.
    pub(super) fn path(&self) -> Option<&str> {
        match self {
            Self::DiskUsage { path, .. }
            | Self::CiRuns { path, .. }
            | Self::GitInfo { path, .. }
            | Self::CratesIoVersion { path, .. }
            | Self::Stars { path, .. } => Some(path),
            Self::ProjectDiscovered { project } => Some(&project.path),
            Self::ScanActivity { .. } | Self::ScanComplete => None,
        }
    }
}

/// Message sent when a background CI fetch completes.
pub enum CiFetchMsg {
    /// The fetch completed with updated runs for the given project path.
    Complete { path: String, runs: Vec<CiRun> },
}

#[derive(Default)]
pub struct ProjectCounts {
    pub workspaces:  usize,
    pub libs:        usize,
    pub bins:        usize,
    pub proc_macros: usize,
    pub examples:    usize,
    pub benches:     usize,
    pub tests:       usize,
}

impl ProjectCounts {
    pub fn add_project(&mut self, project: &RustProject) {
        if project.is_workspace() {
            self.workspaces += 1;
        }
        for t in &project.types {
            match t {
                ProjectType::Library => self.libs += 1,
                ProjectType::Binary => self.bins += 1,
                ProjectType::ProcMacro => self.proc_macros += 1,
                ProjectType::BuildScript => {},
            }
        }
        self.examples += project.example_count();
        self.benches += project.benches.len();
        self.tests += project.test_count;
    }

    pub fn summary(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if self.workspaces > 0 {
            parts.push(format!("{} ws", self.workspaces));
        }
        if self.libs > 0 {
            parts.push(format!("{} lib", self.libs));
        }
        if self.bins > 0 {
            parts.push(format!("{} bin", self.bins));
        }
        if self.proc_macros > 0 {
            parts.push(format!("{} proc", self.proc_macros));
        }
        if self.examples > 0 {
            parts.push(format!("{} ex", self.examples));
        }
        if self.benches > 0 {
            parts.push(format!("{} bench", self.benches));
        }
        if self.tests > 0 {
            parts.push(format!("{} test", self.tests));
        }
        parts.join("  ")
    }

    /// Returns non-zero stats as (label, count) pairs for column display.
    pub fn to_rows(&self) -> Vec<(&'static str, usize)> {
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
