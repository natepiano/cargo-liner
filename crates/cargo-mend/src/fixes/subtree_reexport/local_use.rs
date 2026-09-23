use std::fs;
use std::path::Path;

use anyhow::Context;
use anyhow::Result;
use syn::Item;
use syn::UseTree;
use syn::ext::IdentExt;
use syn::parse_file;

use super::crate_files::CrateFiles;
use crate::fixes::facade_redirect;
use crate::fixes::facade_redirect::SourceLines;
use crate::rust_syntax;

/// How the owner module still needs a name its re-export no longer provides,
/// ordered from no need to a need in every build.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum LocalUse {
    /// Nothing in the owner module or below it names the item.
    Unused,
    /// Only modules under `#[cfg(test)]` name it, through a glob of the owner
    /// module; the private `use` carries `#[cfg(test)]`.
    TestOnly,
    /// Code compiled in every build names it.
    Always,
}

/// Finds a module below the owner module that reaches the exported name
/// through a glob `use` of the owner module (`use super::*;`). Callers are
/// rewritten path by path, but a glob import names no path to rewrite, so the
/// owner module keeps a private `use` of the name for it.
pub(super) struct GlobReach<'a> {
    pub(super) owner: &'a [String],
    pub(super) name:  &'a str,
}

impl GlobReach<'_> {
    /// The strongest use by a module under the owner module, declared inline
    /// among `items` (which sit in `module`) or anywhere below them, that globs
    /// the owner module and mentions the name.
    pub(super) fn items_reach(
        &self,
        source: &str,
        lines: &SourceLines<'_>,
        items: &[Item],
        module: &[String],
    ) -> LocalUse {
        self.items_reach_gated(source, lines, items, module, LocalUse::Always)
    }

    /// [`Self::items_reach`] with `gate` the use a mention inside `items`
    /// stands for: `TestOnly` once a `#[cfg(test)]` module encloses them.
    fn items_reach_gated(
        &self,
        source: &str,
        lines: &SourceLines<'_>,
        items: &[Item],
        module: &[String],
        gate: LocalUse,
    ) -> LocalUse {
        let mut reach = LocalUse::Unused;
        for item in items {
            let Item::Mod(item_mod) = item else {
                continue;
            };
            let Some((brace, content)) = &item_mod.content else {
                continue;
            };
            let gate = if rust_syntax::is_cfg_test(&item_mod.attrs) {
                LocalUse::TestOnly
            } else {
                gate
            };
            let mut child = module.to_vec();
            child.push(item_mod.ident.unraw().to_string());
            let start = lines.offset(brace.span.open().end());
            let end = lines.offset(brace.span.close().start());
            if self.module_reaches(&source[start..end], content, &child) {
                reach = reach.max(gate);
            }
            reach = reach.max(self.items_reach_gated(source, lines, content, &child, gate));
            if reach == LocalUse::Always {
                break;
            }
        }
        reach
    }

    /// The strongest use by a crate file other than `owner_file` that sits
    /// under the owner module and reaches the name through a glob of the owner
    /// module. A file module compiled only under test (`#[cfg(test)]` on its
    /// declaration or an ancestor's) counts `TestOnly`, unless `owner_file`
    /// is itself test-only and the gate adds nothing.
    pub(super) fn files_reach(&self, files: &CrateFiles, owner_file: &Path) -> Result<LocalUse> {
        let owner_test_only = files.is_test_only(owner_file);
        let mut reach = LocalUse::Unused;
        for (file, module) in files.scannable() {
            if file == owner_file || !module.starts_with(self.owner) {
                continue;
            }
            let gate = if files.is_test_only(file) && !owner_test_only {
                LocalUse::TestOnly
            } else {
                LocalUse::Always
            };
            let source = fs::read_to_string(file)
                .with_context(|| format!("failed to read {}", file.display()))?;
            let syntax = parse_file(&source)
                .with_context(|| format!("failed to parse {}", file.display()))?;
            let lines = SourceLines::new(&source);
            if self.module_reaches(&source, &syntax.items, module) {
                reach = reach.max(gate);
            }
            reach = reach.max(self.items_reach_gated(&source, &lines, &syntax.items, module, gate));
            if reach == LocalUse::Always {
                break;
            }
        }
        Ok(reach)
    }

    /// Whether `module`, strictly below the owner module, holds a glob `use`
    /// of the owner module among `items` and mentions the name in `text`.
    fn module_reaches(&self, text: &str, items: &[Item], module: &[String]) -> bool {
        if module.len() <= self.owner.len() || !module.starts_with(self.owner) {
            return false;
        }
        let globs_owner = items.iter().any(|item| {
            let Item::Use(item_use) = item else {
                return false;
            };
            let mut prefixes = Vec::new();
            glob_prefixes(Vec::new(), &item_use.tree, &mut prefixes);
            item_use.leading_colon.is_none()
                && prefixes.iter().any(|prefix| {
                    facade_redirect::absolute_use_path(module, prefix).as_deref()
                        == Some(self.owner)
                })
        });
        globs_owner && facade_redirect::word_occurs_outside(text, [], self.name)
    }
}

/// The written prefix of every glob leaf in `tree`.
fn glob_prefixes(prefix: Vec<String>, tree: &UseTree, out: &mut Vec<Vec<String>>) {
    match tree {
        UseTree::Path(path) => {
            let mut next = prefix;
            next.push(path.ident.to_string());
            glob_prefixes(next, &path.tree, out);
        },
        UseTree::Glob(_) => out.push(prefix),
        UseTree::Group(group) => {
            for item in &group.items {
                glob_prefixes(prefix.clone(), item, out);
            }
        },
        UseTree::Name(_) | UseTree::Rename(_) => {},
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::fs;
    use std::path::Path;

    use syn::parse_file;
    use tempfile::tempdir;

    use super::GlobReach;
    use super::LocalUse;
    use crate::fixes::facade_redirect::SourceLines;
    use crate::fixes::subtree_reexport::crate_files;
    use crate::fixes::subtree_reexport::crate_files::CrateFiles;

    fn reaches(source: &str, name: &str) -> LocalUse {
        let syntax = parse_file(source).expect("parse");
        let owner = vec!["owner".to_string()];
        let reach = GlobReach {
            owner: &owner,
            name,
        };
        reach.items_reach(source, &SourceLines::new(source), &syntax.items, &owner)
    }

    #[test]
    fn a_nested_module_globbing_the_owner_and_naming_the_item_reaches_it() {
        let source = "mod inner {\n    use super::*;\n    fn f() { Widget::new(); }\n}\n";
        assert_eq!(reaches(source, "Widget"), LocalUse::Always);
        assert_eq!(reaches(source, "Other"), LocalUse::Unused);
    }

    #[test]
    fn a_nested_module_without_a_glob_does_not_reach_it() {
        let source = "mod inner {\n    use super::Widget;\n    fn f() { Widget::new(); }\n}\n";
        assert_eq!(reaches(source, "Widget"), LocalUse::Unused);
    }

    #[test]
    fn a_cfg_test_module_and_the_modules_under_it_reach_it_for_tests_only() {
        let direct =
            "#[cfg(test)]\nmod tests {\n    use super::*;\n    fn f() { Widget::new(); }\n}\n";
        assert_eq!(reaches(direct, "Widget"), LocalUse::TestOnly);
        let nested = "#[cfg(test)]\nmod tests {\n    mod deep {\n        use super::super::*;\n        \
                      fn f() { Widget::new(); }\n    }\n}\n";
        assert_eq!(reaches(nested, "Widget"), LocalUse::TestOnly);
    }

    #[test]
    fn an_ungated_module_outranks_a_cfg_test_module() {
        let source = "#[cfg(test)]\nmod tests {\n    use super::*;\n    fn f() { Widget::new(); }\n}\n\
                      mod inner {\n    use super::*;\n    fn g() { Widget::new(); }\n}\n";
        assert_eq!(reaches(source, "Widget"), LocalUse::Always);
    }

    /// A crate whose `owner` module declares `#[cfg(test)] mod tests;` and
    /// `mod plain;`, with `tests_source` and `plain_source` as their files.
    fn files_reach(tests_source: &str, plain_source: &str) -> LocalUse {
        let crate_dir = tempdir().expect("create temp crate");
        let source_root = crate_dir.path().join("src");
        let owner_dir = source_root.join("owner");
        fs::create_dir_all(&owner_dir).expect("create owner dir");
        let write = |path: &Path, text: &str| fs::write(path, text).expect("write crate file");
        write(&source_root.join("lib.rs"), "mod owner;\n");
        write(
            &owner_dir.join("mod.rs"),
            "pub use crate::source::Widget;\nmod plain;\n#[cfg(test)]\nmod tests;\n",
        );
        write(&owner_dir.join("tests.rs"), tests_source);
        write(&owner_dir.join("plain.rs"), plain_source);
        let files = CrateFiles::resolve(&source_root.join("lib.rs"), []);
        let owner = vec!["owner".to_string()];
        GlobReach {
            owner: &owner,
            name:  "Widget",
        }
        .files_reach(&files, &crate_files::canonical(&owner_dir.join("mod.rs")))
        .expect("read crate files")
    }

    #[test]
    fn a_cfg_test_file_module_reaches_it_for_tests_only() {
        let glob = "use super::*;\nfn f() { Widget::new(); }\n";
        assert_eq!(files_reach(glob, ""), LocalUse::TestOnly);
        assert_eq!(files_reach(glob, glob), LocalUse::Always);
        assert_eq!(files_reach("", ""), LocalUse::Unused);
    }

    #[test]
    fn a_doc_comment_mention_does_not_reach_it() {
        let source =
            "#[cfg(test)]\nmod tests {\n    //! Checks [`Widget`].\n    use super::*;\n}\n";
        assert_eq!(reaches(source, "Widget"), LocalUse::Unused);
    }
}
