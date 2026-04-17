use std::path::Path;

use super::package::Package;

/// Members within a workspace organized into groups.
#[derive(Clone)]
pub(crate) enum MemberGroup {
    Named {
        name:    String,
        members: Vec<Package>,
    },
    Inline {
        members: Vec<Package>,
    },
}

impl MemberGroup {
    pub(crate) fn members(&self) -> &[Package] {
        match self {
            Self::Named { members, .. } | Self::Inline { members } => members,
        }
    }

    pub(crate) const fn members_mut(&mut self) -> &mut Vec<Package> {
        match self {
            Self::Named { members, .. } | Self::Inline { members } => members,
        }
    }

    pub(crate) fn group_name(&self) -> &str {
        match self {
            Self::Named { name, .. } => name,
            Self::Inline { .. } => "",
        }
    }

    pub(crate) const fn is_named(&self) -> bool { matches!(self, Self::Named { .. }) }

    pub(crate) const fn is_inline(&self) -> bool { matches!(self, Self::Inline { .. }) }

    pub(crate) fn into_members(self) -> Vec<Package> {
        match self {
            Self::Named { members, .. } | Self::Inline { members } => members,
        }
    }
}

pub(crate) fn count_rs_files_recursive(dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };

    let mut count = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && path.extension().is_some_and(|e| e == "rs") {
            count += 1;
        } else if path.is_dir() {
            // Subdirectories can contain examples too (e.g., `examples/foo/main.rs`)
            // Count the directory as one example if it has a `main.rs`
            if path.join("main.rs").exists() {
                count += 1;
            }
        }
    }
    count
}
