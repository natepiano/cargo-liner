use std::path::Path;
use std::path::PathBuf;

use crate::compiler::source_cache;

#[derive(Debug, Clone)]
pub struct ParentBoundary {
    pub boundary_file: PathBuf,
    pub subtree_root:  PathBuf,
    pub module_path:   Vec<String>,
}

pub fn parent_boundary_for_child(source_root: &Path, child_file: &Path) -> Option<ParentBoundary> {
    let parent_dir = child_file.parent()?;
    let parent_module_rs = parent_dir.join("mod.rs");
    if parent_module_rs.is_file() {
        return Some(ParentBoundary {
            boundary_file: parent_module_rs,
            subtree_root:  parent_dir.to_path_buf(),
            module_path:   source_cache::module_path_from_dir(source_root, parent_dir)?,
        });
    }

    let parent_file = parent_dir.with_extension("rs");
    if parent_file.is_file() {
        return Some(ParentBoundary {
            boundary_file: parent_file.clone(),
            subtree_root:  parent_dir.to_path_buf(),
            module_path:   source_cache::module_path_from_boundary_file(source_root, &parent_file)?,
        });
    }

    None
}

/// Find the parent boundary of an existing boundary file itself.
///
/// `parent_boundary_for_child` cannot be called on a `mod.rs` file because it
/// would find itself.  This helper handles both `mod.rs` and named boundary
/// files (e.g. `tools.rs`).
pub(super) fn parent_of_boundary(
    source_root: &Path,
    boundary_file: &Path,
) -> Option<ParentBoundary> {
    if boundary_file.file_name()?.to_str() != Some("mod.rs") {
        return parent_boundary_for_child(source_root, boundary_file);
    }

    // For mod.rs the enclosing directory IS the module, so go up one more
    // level to reach the parent module's directory.
    let container_dir = boundary_file.parent()?.parent()?;

    let module_rs = container_dir.join("mod.rs");
    if module_rs.is_file() {
        return Some(ParentBoundary {
            boundary_file: module_rs,
            subtree_root:  container_dir.to_path_buf(),
            module_path:   source_cache::module_path_from_dir(source_root, container_dir)?,
        });
    }

    let named_file = container_dir.with_extension("rs");
    if named_file.is_file() {
        return Some(ParentBoundary {
            boundary_file: named_file.clone(),
            subtree_root:  container_dir.to_path_buf(),
            module_path:   source_cache::module_path_from_boundary_file(source_root, &named_file)?,
        });
    }

    for name in ["lib.rs", "main.rs"] {
        let root = container_dir.join(name);
        if root.is_file() {
            return Some(ParentBoundary {
                boundary_file: root,
                subtree_root:  container_dir.to_path_buf(),
                module_path:   Vec::new(),
            });
        }
    }

    None
}
