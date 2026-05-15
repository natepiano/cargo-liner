use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use syn::File;
use syn::ItemUse;
use syn::UseTree;
use syn::visit::Visit;

use crate::rust_syntax::PATH_KEYWORD_CRATE;
use crate::selection::CARGO_TARGET_KIND_LIB;
use crate::selection::CARGO_TARGET_KIND_MAIN;

// rust source files
pub(crate) const RUST_LIB_FILE: &str = "lib.rs";
pub(crate) const RUST_MAIN_FILE: &str = "main.rs";
pub(crate) const RUST_MODULE_FILE: &str = "mod.rs";
pub(crate) const RUST_SOURCE_FILE_EXTENSION: &str = "rs";
pub(crate) const RUST_SOURCE_FILE_SUFFIX: &str = ".rs";

// source-tree directories
pub(crate) const SOURCE_DIR_BENCHES: &str = "benches";
pub(crate) const SOURCE_DIR_EXAMPLES: &str = "examples";
pub(crate) const SOURCE_DIR_SRC: &str = "src";
pub(crate) const SOURCE_DIR_TESTS: &str = "tests";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PathOrigin {
    Relative,
    Crate,
}

pub(super) struct ExtractedPaths {
    /// Flattened use-tree paths with their origin (`Relative`/`Crate`).
    pub use_paths:   Vec<(Vec<String>, PathOrigin)>,
    /// All `syn::Path` nodes found via AST visit, as raw segment strings with origin.
    pub expr_paths:  Vec<(Vec<String>, PathOrigin)>,
    /// Module-level renames (`use path::to::module as alias`): maps alias → original path.
    pub use_renames: Vec<UseRename>,
}

pub(super) struct UseRename {
    pub alias:         String,
    pub original_path: Vec<String>,
}

pub(super) struct SourceCache {
    contents:        HashMap<PathBuf, String>,
    files_by_dir:    HashMap<PathBuf, Vec<PathBuf>>,
    parsed:          HashMap<PathBuf, File>,
    extracted_paths: HashMap<PathBuf, ExtractedPaths>,
}

impl SourceCache {
    pub fn build(roots: &[&Path]) -> Result<Self> {
        let mut contents = HashMap::new();
        for root in roots {
            for file in rust_source_files(root)? {
                contents
                    .entry(file.clone())
                    .or_insert(fs::read_to_string(&file).with_context(|| {
                        format!("failed to pre-read source file {}", file.display())
                    })?);
            }
        }
        let mut files_by_dir: HashMap<PathBuf, Vec<PathBuf>> = HashMap::new();
        for path in contents.keys() {
            if let Some(parent) = path.parent() {
                files_by_dir
                    .entry(parent.to_path_buf())
                    .or_default()
                    .push(path.clone());
            }
        }
        let mut parsed = HashMap::new();
        for (path, source) in &contents {
            if let Ok(ast) = syn::parse_file(source) {
                parsed.insert(path.clone(), ast);
            }
        }
        let mut extracted_paths = HashMap::new();
        for (path, ast) in &parsed {
            extracted_paths.insert(path.clone(), extract_paths(ast));
        }
        Ok(Self {
            contents,
            files_by_dir,
            parsed,
            extracted_paths,
        })
    }

    pub fn source_files_under(&self, dir: &Path) -> Vec<&Path> {
        self.files_by_dir
            .iter()
            .filter(|(d, _)| d.starts_with(dir))
            .flat_map(|(_, files)| files.iter().map(PathBuf::as_path))
            .collect()
    }

    pub fn read_source(&self, path: &Path) -> Result<&str> {
        self.contents
            .get(path)
            .map(String::as_str)
            .with_context(|| format!("source file not in cache: {}", path.display()))
    }

    pub fn parsed_file(&self, path: &Path) -> Option<&File> { self.parsed.get(path) }

    pub fn extracted_paths(&self, path: &Path) -> Option<&ExtractedPaths> {
        self.extracted_paths.get(path)
    }
}

pub(super) fn analysis_source_root_for(
    crate_root_file: &Path,
    package_root: &Path,
) -> Option<PathBuf> {
    let source_root = crate_root_file.parent()?.to_path_buf();
    let canonical_crate_root =
        fs::canonicalize(crate_root_file).unwrap_or_else(|_| crate_root_file.to_path_buf());
    let canonical_package_root =
        fs::canonicalize(package_root).unwrap_or_else(|_| package_root.to_path_buf());
    let relative = canonical_crate_root
        .strip_prefix(&canonical_package_root)
        .ok()?;
    let first_component = relative.components().next()?.as_os_str().to_str()?;
    [
        SOURCE_DIR_SRC,
        SOURCE_DIR_EXAMPLES,
        SOURCE_DIR_TESTS,
        SOURCE_DIR_BENCHES,
    ]
    .contains(&first_component)
    .then_some(source_root)
}

pub(super) fn module_path_from_boundary_file(
    source_root: &Path,
    boundary_file: &Path,
) -> Option<Vec<String>> {
    let relative = boundary_file.strip_prefix(source_root).ok()?;
    let mut components = relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let last = components.last_mut()?;
    *last = last.strip_suffix(RUST_SOURCE_FILE_SUFFIX)?.to_string();
    if matches!(
        components.as_slice(),
        [name] if name == CARGO_TARGET_KIND_LIB || name == CARGO_TARGET_KIND_MAIN
    ) {
        Some(Vec::new())
    } else {
        Some(components)
    }
}

pub(super) fn module_path_from_source_file(
    source_root: &Path,
    source_file: &Path,
) -> Option<Vec<String>> {
    if source_file.file_name().and_then(OsStr::to_str) == Some(RUST_MODULE_FILE) {
        module_path_from_dir(source_root, source_file.parent()?)
    } else {
        module_path_from_boundary_file(source_root, source_file)
    }
}

pub(super) fn module_path_from_dir(source_root: &Path, module_dir: &Path) -> Option<Vec<String>> {
    let relative = module_dir.strip_prefix(source_root).ok()?;
    let components = relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    (!components.is_empty()).then_some(components)
}

pub(super) fn first_line_matching(source: &str, needle: &str) -> Option<usize> {
    source
        .lines()
        .position(|line| line.contains(needle))
        .map(|index| index + 1)
}

pub(super) fn flatten_use_tree(prefix: Vec<String>, tree: &UseTree, out: &mut Vec<Vec<String>>) {
    match tree {
        UseTree::Path(path) => {
            let mut next = prefix;
            next.push(path.ident.to_string());
            flatten_use_tree(next, &path.tree, out);
        },
        UseTree::Name(name) => {
            let mut next = prefix;
            next.push(name.ident.to_string());
            out.push(next);
        },
        UseTree::Rename(rename) => {
            let mut next = prefix;
            next.push(rename.ident.to_string());
            next.push(rename.rename.to_string());
            out.push(next);
        },
        UseTree::Group(group) => {
            for item in &group.items {
                flatten_use_tree(prefix.clone(), item, out);
            }
        },
        UseTree::Glob(_) => {
            let mut next = prefix;
            next.push("*".to_string());
            out.push(next);
        },
    }
}

fn rust_source_files(source_root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    collect_rust_source_files(source_root, &mut files)?;
    Ok(files)
}

fn collect_rust_source_files(dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir)
        .with_context(|| format!("failed to read source directory {}", dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_rust_source_files(&path, files)?;
        } else if path.extension().and_then(OsStr::to_str) == Some(RUST_SOURCE_FILE_EXTENSION) {
            files.push(path);
        }
    }
    Ok(())
}

pub(super) fn path_origin(raw: &[String]) -> PathOrigin {
    if raw.first().map(String::as_str) == Some(PATH_KEYWORD_CRATE) {
        PathOrigin::Crate
    } else {
        PathOrigin::Relative
    }
}

struct PathExtractor {
    use_paths:       Vec<(Vec<String>, PathOrigin)>,
    expr_paths:      Vec<(Vec<String>, PathOrigin)>,
    use_renames:     Vec<UseRename>,
    inside_use_item: UseItemPosition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UseItemPosition {
    Outside,
    Inside,
}

impl<'ast> Visit<'ast> for PathExtractor {
    fn visit_item_use(&mut self, item_use: &'ast ItemUse) {
        let mut flat = Vec::new();
        flatten_use_tree(Vec::new(), &item_use.tree, &mut flat);
        for raw in flat {
            let origin = path_origin(&raw);
            self.use_paths.push((raw, origin));
        }
        extract_use_renames(Vec::new(), &item_use.tree, &mut self.use_renames);
        self.inside_use_item = UseItemPosition::Inside;
        syn::visit::visit_item_use(self, item_use);
        self.inside_use_item = UseItemPosition::Outside;
    }

    fn visit_path(&mut self, path: &'ast syn::Path) {
        if self.inside_use_item == UseItemPosition::Outside {
            let segments: Vec<String> = path
                .segments
                .iter()
                .map(|segment| segment.ident.to_string())
                .collect();
            let origin = path_origin(&segments);
            self.expr_paths.push((segments, origin));
        }
        syn::visit::visit_path(self, path);
    }
}

pub(super) fn extract_paths(file: &File) -> ExtractedPaths {
    let mut extractor = PathExtractor {
        use_paths:       Vec::new(),
        expr_paths:      Vec::new(),
        use_renames:     Vec::new(),
        inside_use_item: UseItemPosition::Outside,
    };
    extractor.visit_file(file);

    ExtractedPaths {
        use_paths:   extractor.use_paths,
        expr_paths:  extractor.expr_paths,
        use_renames: extractor.use_renames,
    }
}

pub(super) fn extract_use_renames(prefix: Vec<String>, tree: &UseTree, out: &mut Vec<UseRename>) {
    match tree {
        UseTree::Path(path) => {
            let mut next = prefix;
            next.push(path.ident.to_string());
            extract_use_renames(next, &path.tree, out);
        },
        UseTree::Rename(rename) => {
            let mut original_path = prefix;
            original_path.push(rename.ident.to_string());
            out.push(UseRename {
                alias: rename.rename.to_string(),
                original_path,
            });
        },
        UseTree::Group(group) => {
            for item in &group.items {
                extract_use_renames(prefix.clone(), item, out);
            }
        },
        UseTree::Name(_) | UseTree::Glob(_) => {},
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::time::SystemTime;
    use std::time::UNIX_EPOCH;

    use anyhow::Result;

    use super::analysis_source_root_for;
    use super::module_path_from_source_file;

    #[test]
    fn analysis_source_root_ignores_build_scripts() {
        let package_root = Path::new("/tmp/example-crate");

        assert_eq!(
            analysis_source_root_for(&package_root.join("src/lib.rs"), package_root),
            Some(package_root.join("src"))
        );
        assert_eq!(
            analysis_source_root_for(&package_root.join("src/bin/demo.rs"), package_root),
            Some(package_root.join("src/bin"))
        );
        assert_eq!(
            analysis_source_root_for(&package_root.join("examples/demo.rs"), package_root),
            Some(package_root.join("examples"))
        );
        assert_eq!(
            analysis_source_root_for(&package_root.join("build.rs"), package_root),
            None
        );
    }

    #[test]
    fn module_path_from_source_file_treats_main_rs_as_crate_root() -> Result<()> {
        let unique = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("mend-main-root-test-{unique}"));
        let source_dir = temp_dir.join("src");
        fs::create_dir_all(&source_dir)?;
        let main_rs = source_dir.join("main.rs");
        fs::write(&main_rs, "fn main() {}\n")?;

        assert_eq!(
            module_path_from_source_file(&source_dir, &main_rs),
            Some(Vec::new())
        );

        fs::remove_dir_all(&temp_dir)?;
        Ok(())
    }
}
