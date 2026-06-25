use std::cmp::Ordering;
use std::cmp::Reverse;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;

use itertools::Itertools;
use toml::Table;
use toml::Value;
use walkdir::WalkDir;

use crate::constants::CARGO_TOML;
use crate::project::AbsolutePath;
use crate::project::CargoParseResult;
use crate::project::MemberGroup;
use crate::project::Package;
use crate::project::ProjectFields;
use crate::project::RootItem;
use crate::project::RustInfo;
use crate::project::RustProject;
use crate::project::VendoredPackage;
use crate::project::WorktreeGroup;

mod build;
mod dependencies;
mod vendored;
mod workspace;
mod worktrees;

pub(crate) use build::build_tree;
pub(crate) use build::cargo_project_to_item;
pub(crate) use build::dir_size;
use dependencies::package_path_dependencies;
use dependencies::workspace_path_dependencies;
use vendored::extract_vendored_new;
pub(crate) use workspace::normalize_workspace_path;
use workspace::workspace_member_paths_new;
pub(super) fn merge_worktrees_new(items: &mut Vec<RootItem>) {
    worktrees::merge_worktrees_new(items);
}
