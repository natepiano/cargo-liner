use std::ffi::OsStr;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PathAnchor {
    Crate,
    Super,
    SelfMod,
    SelfType,
    Name,
}

impl PathAnchor {
    pub(crate) fn first(segments: &[String]) -> Option<Self> {
        segments.first().map(|segment| Self::from(segment.as_str()))
    }

    pub(crate) const fn is_crate_relative(self) -> bool {
        matches!(self, Self::Crate | Self::Super)
    }

    pub(crate) const fn is_explicit_self(self) -> bool {
        matches!(self, Self::SelfMod | Self::SelfType)
    }
}

impl From<&str> for PathAnchor {
    fn from(segment: &str) -> Self {
        match segment {
            "crate" => Self::Crate,
            "super" => Self::Super,
            "self" => Self::SelfMod,
            "Self" => Self::SelfType,
            _ => Self::Name,
        }
    }
}

enum BoundaryModuleName<'a> {
    Root,
    Named(&'a str),
}

pub(crate) fn leading_super_count(segments: &[String]) -> usize {
    segments
        .iter()
        .take_while(|segment| PathAnchor::from(segment.as_str()) == PathAnchor::Super)
        .count()
}

pub(crate) fn trim_leading_self(segments: &[String]) -> &[String] {
    match PathAnchor::first(segments) {
        Some(PathAnchor::SelfMod) => &segments[1..],
        _ => segments,
    }
}

pub(crate) fn file_module_path(source_root: &Path, path: &Path) -> Option<Vec<String>> {
    let relative = path.strip_prefix(source_root).ok()?;
    let mut result: Vec<String> = relative
        .parent()
        .into_iter()
        .flat_map(Path::iter)
        .filter_map(|segment| segment.to_str().map(str::to_string))
        .collect();

    if let Some(name) = module_name_for_file_module_path(path) {
        result.push(name.to_string());
    }

    Some(result)
}

pub(crate) fn module_name_for_child_boundary_file(child_file: &Path) -> Option<&str> {
    match module_name_for_boundary_file(child_file)? {
        BoundaryModuleName::Named(name) => Some(name),
        BoundaryModuleName::Root => None,
    }
}

fn module_name_for_boundary_file(path: &Path) -> Option<BoundaryModuleName<'_>> {
    match path.file_name().and_then(OsStr::to_str) {
        Some("mod.rs") => Some(BoundaryModuleName::Named(
            path.parent()?.file_name()?.to_str()?,
        )),
        Some(name) if name == "lib.rs" || name == "main.rs" => Some(BoundaryModuleName::Root),
        _ => Some(BoundaryModuleName::Named(path.file_stem()?.to_str()?)),
    }
}

fn module_name_for_file_module_path(path: &Path) -> Option<&str> {
    match path.file_name().and_then(OsStr::to_str) {
        Some(name) if name == "mod.rs" || name == "lib.rs" || name == "main.rs" => None,
        _ => path.file_stem()?.to_str(),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::file_module_path;
    use super::module_name_for_child_boundary_file;

    #[test]
    fn file_module_path_includes_leaf_file_name() {
        let source_root = Path::new("/repo/src");
        let path = Path::new("/repo/src/outer/child.rs");
        assert_eq!(
            file_module_path(source_root, path),
            Some(vec!["outer".to_string(), "child".to_string()])
        );
    }

    #[test]
    fn file_module_path_uses_parent_dir_for_module_rs() {
        let source_root = Path::new("/repo/src");
        let path = Path::new("/repo/src/outer/child/mod.rs");
        assert_eq!(
            file_module_path(source_root, path),
            Some(vec!["outer".to_string(), "child".to_string()])
        );
    }

    #[test]
    fn file_module_path_treats_lib_rs_as_root() {
        let source_root = Path::new("/repo/src");
        let path = Path::new("/repo/src/lib.rs");
        assert_eq!(file_module_path(source_root, path), Some(Vec::new()));
    }

    #[test]
    fn file_module_path_treats_main_rs_as_root() {
        let source_root = Path::new("/repo/src");
        let path = Path::new("/repo/src/main.rs");
        assert_eq!(file_module_path(source_root, path), Some(Vec::new()));
    }

    #[test]
    fn child_boundary_name_for_module_rs_is_parent_dir() {
        let path = Path::new("/repo/src/outer/child/mod.rs");
        assert_eq!(module_name_for_child_boundary_file(path), Some("child"));
    }

    #[test]
    fn child_boundary_name_for_leaf_file_is_stem() {
        let path = Path::new("/repo/src/outer/child.rs");
        assert_eq!(module_name_for_child_boundary_file(path), Some("child"));
    }

    #[test]
    fn child_boundary_name_for_root_file_is_none() {
        let path = Path::new("/repo/src/lib.rs");
        assert_eq!(module_name_for_child_boundary_file(path), None);
    }
}
