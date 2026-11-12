use std::cell::OnceCell;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::rc::Rc;

use rustc_hash::FxHashMap;
use syn::File;
use syn::parse_file;

use super::SourcePathKey;
use super::module_map::ModuleMap;
use super::source_path_key::SourcePathKeys;
use crate::compiler::SOURCE_DIR_SRC;
use crate::selection::Selection;

/// One source file's text, parsed on the first request for its syntax.
pub(crate) struct ParsedSource {
    text:   String,
    syntax: OnceCell<syn::Result<File>>,
}

impl ParsedSource {
    pub(crate) fn text(&self) -> &str { &self.text }

    /// The file's syntax tree, or the error that kept it from parsing. The
    /// result is kept, so a second call does not parse again.
    pub(crate) fn syntax(&self) -> syn::Result<&File> {
        self.syntax
            .get_or_init(|| parse_file(&self.text))
            .as_ref()
            .map_err(Clone::clone)
    }
}

/// The source files and module maps one check reads, each read and parsed at
/// most once. `SourcePathKey` gives module walks and directory walks the same identity.
///
/// No entry is ever invalidated, so a cache must not outlive a write to the
/// tree. That holds by its lifetime: the runner builds one per
/// `build_selection`, after `cargo fix` may have rewritten files during the
/// check, and every fixer reads through it before any fix is written. The
/// validation check after a write builds a fresh one.
///
/// `Rc` and `RefCell` rather than `Arc` and `Mutex`: `proc_macro2` types are
/// `!Send`, so a syntax tree never leaves the thread that parsed it.
#[derive(Default)]
pub(crate) struct ParsedSources {
    pub(super) path_keys: Rc<SourcePathKeys>,
    sources:              RefCell<FxHashMap<SourcePathKey, Rc<ParsedSource>>>,
    module_maps:          RefCell<FxHashMap<SourcePathKey, Rc<ModuleMap>>>,
}

impl From<&BTreeMap<PathBuf, String>> for ParsedSources {
    /// Read proposed source text before a batch writes any file.
    fn from(files: &BTreeMap<PathBuf, String>) -> Self {
        let sources = Self::default();
        for (path, text) in files {
            sources.sources.borrow_mut().insert(
                sources.path_key(path),
                Rc::new(ParsedSource {
                    text:   text.clone(),
                    syntax: OnceCell::new(),
                }),
            );
        }
        sources
    }
}

impl ParsedSources {
    /// Reuse the identity of a file or root throughout this source snapshot.
    pub(crate) fn path_key(&self, path: &Path) -> SourcePathKey { self.path_keys.key(path) }

    /// Compare physical identities before rendering a relative diagnostic path.
    pub(crate) fn display_path(&self, root: &Path, file: &Path) -> String {
        let root = self.path_key(root);
        let file = self.path_key(file);
        file.as_path()
            .strip_prefix(root.as_path())
            .unwrap_or(file.as_path())
            .to_string_lossy()
            .replace('\\', "/")
    }

    /// Seed module maps from Cargo's target roots, including integration tests
    /// and targets whose manifest paths do not follow the directory defaults.
    pub(crate) fn for_selection(selection: &Selection) -> Self {
        let parsed_sources = Self::default();
        for package_metadata in &selection.packages {
            let source_root = parsed_sources.path_key(&package_metadata.root.join(SOURCE_DIR_SRC));
            let crate_roots = package_metadata
                .targets
                .iter()
                .map(|target_metadata| target_metadata.src_path.clone())
                .collect::<Vec<_>>();
            let map = ModuleMap::for_crate_roots(&crate_roots, &parsed_sources);
            parsed_sources
                .module_maps
                .borrow_mut()
                .insert(source_root, Rc::new(map));
        }
        parsed_sources
    }

    /// The text of `path`, read on the first request. A read failure is
    /// returned and not kept, so a later request reads again.
    pub(crate) fn source(&self, path: &Path) -> io::Result<Rc<ParsedSource>> {
        let key = self.path_key(path);
        if let Some(source) = self.sources.borrow().get(&key) {
            return Ok(Rc::clone(source));
        }
        let source = Rc::new(ParsedSource {
            text:   fs::read_to_string(path)?,
            syntax: OnceCell::new(),
        });
        self.sources.borrow_mut().insert(key, Rc::clone(&source));
        Ok(source)
    }

    /// The module map of every crate root under `source_root`, resolved on
    /// the first request.
    pub(crate) fn module_map(&self, source_root: &Path) -> Rc<ModuleMap> {
        let key = self.path_key(source_root);
        if let Some(module_map) = self.module_maps.borrow().get(&key) {
            return Rc::clone(module_map);
        }
        let module_map = Rc::new(ModuleMap::resolve(source_root, self));
        self.module_maps
            .borrow_mut()
            .insert(key, Rc::clone(&module_map));
        module_map
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::fs;
    use std::rc::Rc;

    use tempfile::tempdir;

    use super::ParsedSources;

    #[test]
    fn a_repeat_request_returns_the_same_source() {
        let temp_dir = tempdir().expect("create temp dir");
        let path = temp_dir.path().join("lib.rs");
        fs::write(&path, "pub(crate) fn run() {}\n").expect("write source");
        let parsed_sources = ParsedSources::default();

        let first_parsed_source = parsed_sources.source(&path).expect("read source");
        let repeated_parsed_source = parsed_sources
            .source(&temp_dir.path().join("./nested/../lib.rs"))
            .expect("read source again");

        assert!(Rc::ptr_eq(&first_parsed_source, &repeated_parsed_source));
        assert!(first_parsed_source.syntax().is_ok());
        #[cfg(unix)]
        {
            let alias = temp_dir.path().join("alias.rs");
            std::os::unix::fs::symlink(&path, &alias).expect("link source");
            let alias_source = parsed_sources.source(&alias).expect("read linked source");
            assert!(Rc::ptr_eq(&first_parsed_source, &alias_source));
        }
    }

    #[test]
    fn a_repeat_module_map_request_returns_the_same_map() {
        let temp_dir = tempdir().expect("create temp dir");
        let source_root = temp_dir.path().join("src");
        fs::create_dir_all(&source_root).expect("create source root");
        fs::write(source_root.join("lib.rs"), "").expect("write lib root");
        let parsed_sources = ParsedSources::default();

        let first_module_map = parsed_sources.module_map(&source_root);
        let repeated_module_map = parsed_sources.module_map(&source_root);

        assert!(Rc::ptr_eq(&first_module_map, &repeated_module_map));
        #[cfg(unix)]
        {
            let alias = temp_dir.path().join("linked_src");
            std::os::unix::fs::symlink(&source_root, &alias).expect("link source root");
            assert!(Rc::ptr_eq(
                &first_module_map,
                &parsed_sources.module_map(&alias)
            ));
        }
    }

    #[test]
    fn a_parse_error_is_kept_and_reported() {
        let temp_dir = tempdir().expect("create temp dir");
        let path = temp_dir.path().join("broken.rs");
        fs::write(&path, "fn broken( {\n").expect("write source");
        let parsed_sources = ParsedSources::default();

        let source = parsed_sources.source(&path).expect("read source");
        let first = source
            .syntax()
            .map(|_| ())
            .expect_err("parse fails")
            .to_string();
        let again = parsed_sources
            .source(&path)
            .expect("read source again")
            .syntax()
            .map(|_| ())
            .expect_err("parse still fails")
            .to_string();

        assert_eq!(first, again);
        assert_eq!(source.text(), "fn broken( {\n");
    }

    #[test]
    fn a_read_error_is_not_kept() {
        let temp_dir = tempdir().expect("create temp dir");
        let path = temp_dir.path().join("late.rs");
        let parsed_sources = ParsedSources::default();

        assert!(parsed_sources.source(&path).is_err());
        fs::write(&path, "fn late() {}\n").expect("write source");

        let source = parsed_sources
            .source(&path)
            .expect("read source after it exists");
        assert_eq!(source.text(), "fn late() {}\n");
    }

    #[test]
    fn a_fresh_cache_sees_a_rewritten_file() {
        let temp_dir = tempdir().expect("create temp dir");
        let path = temp_dir.path().join("lib.rs");
        fs::write(&path, "fn before() {}\n").expect("write source");
        let before_parsed_sources = ParsedSources::default();
        assert_eq!(
            before_parsed_sources
                .source(&path)
                .expect("read source")
                .text(),
            "fn before() {}\n"
        );

        fs::write(&path, "fn after() {}\n").expect("rewrite source");
        let after_parsed_sources = ParsedSources::default();

        assert_eq!(
            after_parsed_sources
                .source(&path)
                .expect("read rewritten source")
                .text(),
            "fn after() {}\n"
        );
    }
}
