use std::path::Path;
use std::path::PathBuf;

use ParentFacadeVisibility::Crate;
use ParentFacadeVisibility::Public;
use ParentFacadeVisibility::Super;
use anyhow::Result;
use syn::File;
use syn::Item;
use syn::ItemUse;
use syn::UseTree;
use syn::Visibility;

use super::boundary;
use super::reference;
use super::reference::ParentFacadeUsage;
use crate::compiler::settings::DriverSettings;
use crate::compiler::source_cache;
use crate::compiler::source_cache::SourceCache;
use crate::rust_syntax;
use crate::rust_syntax::PATH_KEYWORD_CRATE;
use crate::rust_syntax::PATH_KEYWORD_SELF;
use crate::rust_syntax::PATH_KEYWORD_SUPER;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ParentFacadeFixSupport {
    #[default]
    Unsupported,
    Supported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParentFacadeVisibility {
    Public,
    Crate,
    Super,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct ParentFacadeExports {
    pub explicit:      Vec<String>,
    pub fix_supported: ParentFacadeFixSupport,
    pub visibility:    Option<ParentFacadeVisibility>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParentFacadeExportStatus {
    pub usage:           ParentFacadeUsage,
    pub fix_supported:   ParentFacadeFixSupport,
    pub visibility:      ParentFacadeVisibility,
    pub parent_path:     PathBuf,
    pub parent_rel_path: String,
    pub parent_line:     usize,
}

/// Check whether `root_module` (lib.rs / main.rs) re-exports `item_name`
/// from the child module that `child_file` belongs to.
pub fn root_module_exports_item(
    source_cache: &SourceCache,
    root_module: &Path,
    child_file: &Path,
    item_name: &str,
) -> bool {
    let Some(child_module_name) = rust_syntax::module_name_for_child_boundary_file(child_file)
    else {
        return false;
    };
    let Some(file) = source_cache.parsed_file(root_module) else {
        return false;
    };
    let exports = exported_names_from_parent_boundary(file, child_module_name, item_name);
    !exports.explicit.is_empty()
}

pub fn parent_facade_export_status(
    source_cache: &SourceCache,
    settings: &DriverSettings,
    source_root: &Path,
    child_file: &Path,
    item_name: &str,
) -> Result<Option<ParentFacadeExportStatus>> {
    let Some(initial_boundary) = boundary::parent_boundary_for_child(source_root, child_file)
    else {
        return Ok(None);
    };

    // Walk up from the immediate parent through ancestors until we find a
    // boundary that re-exports `item_name`, or run out of ancestors.
    let mut current_child: PathBuf = child_file.to_path_buf();
    let mut parent_boundary = initial_boundary;

    let exported_names = loop {
        let Some(child_module_name) =
            rust_syntax::module_name_for_child_boundary_file(&current_child)
        else {
            return Ok(None);
        };

        let Some(file) = source_cache.parsed_file(&parent_boundary.boundary_file) else {
            return Ok(None);
        };
        let exports = exported_names_from_parent_boundary(file, child_module_name, item_name);

        if !exports.explicit.is_empty() {
            break exports;
        }

        // Not found at this level — walk up to the next ancestor.
        current_child.clone_from(&parent_boundary.boundary_file);
        let Some(next_boundary) = boundary::parent_of_boundary(source_root, &current_child) else {
            return Ok(None);
        };
        parent_boundary = next_boundary;
    };

    let parent_rel_path = parent_boundary
        .boundary_file
        .strip_prefix(source_root)
        .unwrap_or(&parent_boundary.boundary_file)
        .to_string_lossy()
        .replace('\\', "/");
    let parent_source = source_cache.read_source(&parent_boundary.boundary_file)?;
    let parent_line = source_cache::first_line_matching(parent_source, item_name).unwrap_or(1);

    let usage = reference::scan_facade_usage(
        source_cache,
        settings,
        source_root,
        &parent_boundary,
        &exported_names,
    )?;

    Ok(Some(ParentFacadeExportStatus {
        usage,
        fix_supported: exported_names.fix_supported,
        visibility: exported_names
            .visibility
            .unwrap_or(ParentFacadeVisibility::Public),
        parent_path: parent_boundary.boundary_file,
        parent_rel_path,
        parent_line,
    }))
}

pub(super) fn exported_names_from_parent_boundary(
    file: &File,
    child_module_name: &str,
    item_name: &str,
) -> ParentFacadeExports {
    let mut exported = ParentFacadeExports::default();
    for item in &file.items {
        let Item::Use(item_use) = item else {
            continue;
        };
        let Some(visibility) = parent_facade_visibility(&item_use.vis) else {
            continue;
        };
        collect_matching_pub_use_exports(
            item_use,
            visibility,
            child_module_name,
            item_name,
            &mut exported,
        );
    }
    exported.explicit.sort();
    exported.explicit.dedup();
    exported
}

fn collect_matching_pub_use_exports(
    item_use: &ItemUse,
    use_visibility: ParentFacadeVisibility,
    child_module_name: &str,
    item_name: &str,
    exported: &mut ParentFacadeExports,
) {
    let mut paths = Vec::new();
    source_cache::flatten_use_tree(Vec::new(), &item_use.tree, &mut paths);
    let mut matched = false;
    for path in paths {
        let normalized = if path
            .first()
            .is_some_and(|segment| segment == PATH_KEYWORD_SELF)
        {
            &path[1..]
        } else {
            &path[..]
        };
        if normalized.len() >= 2
            && normalized[0] == child_module_name
            && normalized[1..].iter().any(|segment| segment == item_name)
            && let Some(export_name) = normalized.last()
        {
            exported.explicit.push(export_name.clone());
            matched = true;
        }
    }
    if matched {
        if pub_use_is_fix_supported(&item_use.tree, child_module_name, item_name) {
            exported.fix_supported = ParentFacadeFixSupport::Supported;
        }
        exported.visibility = Some(exported.visibility.map_or(use_visibility, |existing| {
            widest_visibility(existing, use_visibility)
        }));
    }
}

const fn widest_visibility(
    a: ParentFacadeVisibility,
    b: ParentFacadeVisibility,
) -> ParentFacadeVisibility {
    match (a, b) {
        (Public, _) | (_, Public) => Public,
        (Crate, _) | (_, Crate) => Crate,
        (Super, Super) => Super,
    }
}

fn pub_use_is_fix_supported(tree: &UseTree, child_module_name: &str, item_name: &str) -> bool {
    pub_use_is_fix_supported_with_prefix(Vec::new(), tree, child_module_name, item_name)
}

fn pub_use_is_fix_supported_with_prefix(
    prefix: Vec<String>,
    tree: &UseTree,
    child_module_name: &str,
    item_name: &str,
) -> bool {
    match tree {
        UseTree::Path(path) => {
            let mut next = prefix;
            next.push(path.ident.to_string());
            pub_use_is_fix_supported_with_prefix(next, &path.tree, child_module_name, item_name)
        },
        UseTree::Name(name) => {
            let normalized = if prefix
                .first()
                .is_some_and(|segment| segment == PATH_KEYWORD_SELF)
            {
                &prefix[1..]
            } else {
                &prefix[..]
            };
            normalized.len() == 1 && normalized[0] == child_module_name && name.ident == item_name
        },
        UseTree::Group(group) => group.items.iter().any(|item| {
            pub_use_is_fix_supported_with_prefix(prefix.clone(), item, child_module_name, item_name)
        }),
        UseTree::Rename(_) | UseTree::Glob(_) => false,
    }
}

pub(super) fn parent_facade_visibility(vis: &Visibility) -> Option<ParentFacadeVisibility> {
    match vis {
        Visibility::Public(_) => Some(ParentFacadeVisibility::Public),
        Visibility::Restricted(restricted)
            if restricted.path.segments.len() == 1
                && restricted.path.segments[0].ident == PATH_KEYWORD_SUPER =>
        {
            Some(ParentFacadeVisibility::Super)
        },
        Visibility::Restricted(restricted)
            if restricted.path.segments.len() == 1
                && restricted.path.segments[0].ident == PATH_KEYWORD_CRATE =>
        {
            Some(ParentFacadeVisibility::Crate)
        },
        _ => None,
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use syn::parse_file;

    use super::ParentFacadeExports;
    use super::ParentFacadeFixSupport;
    use super::ParentFacadeVisibility;
    use super::exported_names_from_parent_boundary;

    #[test]
    fn grouped_parent_pub_use_is_fix_supported() {
        let source = "pub use report_writer::{ReportDefinition, ReportWriter};\n";
        let file = parse_file(source).unwrap();
        let exports =
            exported_names_from_parent_boundary(&file, "report_writer", "ReportDefinition");
        assert_eq!(exports.explicit, vec!["ReportDefinition".to_string()]);
        assert_eq!(exports.fix_supported, ParentFacadeFixSupport::Supported);
    }

    #[test]
    fn mixed_pub_uses_pick_visibility_from_matching_re_export() {
        // Parent file has both `pub(crate) use` and `pub use` lines pointing at
        // different children. The visibility on `ParentFacadeExports` must come
        // from the line that actually re-exports the queried item, not from
        // whichever pub-ish `use` appears first in the file.
        let source = "\
pub(crate) use first_child::Alpha;
pub use second_child::Beta;
";
        let file = parse_file(source).unwrap();

        let exports = exported_names_from_parent_boundary(&file, "first_child", "Alpha");
        assert_eq!(exports.explicit, vec!["Alpha".to_string()]);
        assert_eq!(exports.visibility, Some(ParentFacadeVisibility::Crate));

        let exports = exported_names_from_parent_boundary(&file, "second_child", "Beta");
        assert_eq!(exports.explicit, vec!["Beta".to_string()]);
        assert_eq!(exports.visibility, Some(ParentFacadeVisibility::Public));
    }

    #[test]
    fn duplicate_re_exports_take_widest_visibility() {
        // Same item re-exported with both `pub(crate) use` and `pub use` —
        // widest reach wins so `narrow-pub-crate` doesn't fire on an item
        // that's already public.
        let source = "\
pub(crate) use child::Thing;
pub use child::Thing;
";
        let file = parse_file(source).unwrap();
        let exports = exported_names_from_parent_boundary(&file, "child", "Thing");
        assert_eq!(exports.visibility, Some(ParentFacadeVisibility::Public));
    }

    #[test]
    fn multiline_grouped_parent_pub_use_is_fix_supported() {
        let source = "pub use child::{\n    Thing,\n    Other,\n};\n";
        let file = parse_file(source).unwrap();
        let exports = exported_names_from_parent_boundary(&file, "child", "Thing");
        assert_eq!(exports.explicit, vec!["Thing".to_string()]);
        assert_eq!(exports.fix_supported, ParentFacadeFixSupport::Supported);
    }

    #[test]
    fn grouped_parent_pub_use_with_rename_is_manual_only() {
        let source = "pub use child::{Thing as RenamedThing, Other};\n";
        let file = parse_file(source).unwrap();
        let exports = exported_names_from_parent_boundary(&file, "child", "Thing");
        assert_eq!(
            exports,
            ParentFacadeExports {
                explicit:      vec!["RenamedThing".to_string()],
                fix_supported: ParentFacadeFixSupport::Unsupported,
                visibility:    Some(ParentFacadeVisibility::Public),
            }
        );

        let exports = exported_names_from_parent_boundary(&file, "child", "Other");
        assert_eq!(exports.explicit, vec!["Other".to_string()]);
        assert_eq!(exports.fix_supported, ParentFacadeFixSupport::Supported);
    }
}
