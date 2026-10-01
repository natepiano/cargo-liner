use std::cell::OnceCell;
use std::cell::RefCell;
use std::fs;
use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::rc::Rc;

use rustc_hash::FxHashMap;
use syn::File;
use syn::parse_file;

use super::module_map;
use super::module_map::ModuleMap;

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
/// most once, keyed by lexically normalized path so a module walk and a
/// directory walk share entries.
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
    sources:     RefCell<FxHashMap<PathBuf, Rc<ParsedSource>>>,
    module_maps: RefCell<FxHashMap<PathBuf, Rc<ModuleMap>>>,
}

impl ParsedSources {
    /// The text of `path`, read on the first request. A read failure is
    /// returned and not kept, so a later request reads again.
    pub(crate) fn source(&self, path: &Path) -> io::Result<Rc<ParsedSource>> {
        let key = module_map::lexically_normalized(path);
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
        let key = module_map::lexically_normalized(source_root);
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
        let dir = tempdir().expect("create temp dir");
        let path = dir.path().join("lib.rs");
        fs::write(&path, "pub(crate) fn run() {}\n").expect("write source");
        let sources = ParsedSources::default();

        let first = sources.source(&path).expect("read source");
        let again = sources
            .source(&dir.path().join("./nested/../lib.rs"))
            .expect("read source again");

        assert!(Rc::ptr_eq(&first, &again));
        assert!(first.syntax().is_ok());
    }

    #[test]
    fn a_repeat_module_map_request_returns_the_same_map() {
        let dir = tempdir().expect("create temp dir");
        let source_root = dir.path().join("src");
        fs::create_dir_all(&source_root).expect("create source root");
        fs::write(source_root.join("lib.rs"), "").expect("write lib root");
        let sources = ParsedSources::default();

        let first = sources.module_map(&source_root);
        let again = sources.module_map(&source_root);

        assert!(Rc::ptr_eq(&first, &again));
    }

    #[test]
    fn a_parse_error_is_kept_and_reported() {
        let dir = tempdir().expect("create temp dir");
        let path = dir.path().join("broken.rs");
        fs::write(&path, "fn broken( {\n").expect("write source");
        let sources = ParsedSources::default();

        let source = sources.source(&path).expect("read source");
        let first = source
            .syntax()
            .map(|_| ())
            .expect_err("parse fails")
            .to_string();
        let again = sources
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
        let dir = tempdir().expect("create temp dir");
        let path = dir.path().join("late.rs");
        let sources = ParsedSources::default();

        assert!(sources.source(&path).is_err());
        fs::write(&path, "fn late() {}\n").expect("write source");

        let source = sources.source(&path).expect("read source after it exists");
        assert_eq!(source.text(), "fn late() {}\n");
    }

    #[test]
    fn a_fresh_cache_sees_a_rewritten_file() {
        let dir = tempdir().expect("create temp dir");
        let path = dir.path().join("lib.rs");
        fs::write(&path, "fn before() {}\n").expect("write source");
        let before = ParsedSources::default();
        assert_eq!(
            before.source(&path).expect("read source").text(),
            "fn before() {}\n"
        );

        fs::write(&path, "fn after() {}\n").expect("rewrite source");
        let after = ParsedSources::default();

        assert_eq!(
            after.source(&path).expect("read rewritten source").text(),
            "fn after() {}\n"
        );
    }
}
