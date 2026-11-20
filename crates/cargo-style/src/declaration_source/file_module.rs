//! Read paths from a crate root through plain module links.

use std::collections::HashMap;
use std::collections::hash_map::Entry;

use toml::Table;
use toml::Value;

use super::DeclarationFile;
use super::DeclarationSource;
use crate::finding_source::FindingSource;
use crate::finding_source::FindingSourceFailure;
use crate::finding_source::SourceText;
use crate::tip_view;
use crate::tip_view::RepositoryTip;
use crate::word_syntax;

pub(crate) enum FileModule {
    Rooted(Vec<String>),
    Unreadable,
}

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub(crate) struct SourceLocation<'a> {
    pub(crate) commit: &'a str,
    pub(crate) path:   &'a str,
}

pub(super) type FileModules = HashMap<String, Vec<String>>;

pub(super) enum PlainLinks {
    Names(Vec<String>),
    Unproved,
}

impl DeclarationSource {
    pub(crate) fn file_module(
        &mut self,
        source: &mut FindingSource<'_>,
        location: SourceLocation<'_>,
    ) -> Result<FileModule, FindingSourceFailure> {
        let tip = source.tip(location.commit)?;
        let crate_name = tip.crate_of(location.path);
        let key = (location.commit.to_owned(), crate_name.to_owned());
        if !self.modules.contains_key(&key) {
            let modules = self.read_modules(source, &tip, location.commit, crate_name)?;
            self.modules.insert(key.clone(), modules);
        }
        Ok(self.modules[&key]
            .get(location.path)
            .map_or(FileModule::Unreadable, |chain| {
                FileModule::Rooted(chain.clone())
            }))
    }

    fn read_modules(
        &mut self,
        source: &mut FindingSource<'_>,
        tip: &RepositoryTip,
        commit: &str,
        crate_name: &str,
    ) -> Result<FileModules, FindingSourceFailure> {
        let manifest_path = join_repo(crate_name, "Cargo.toml");
        let Some(manifest) = tip.blob(&manifest_path) else {
            return Ok(HashMap::new());
        };
        let text = source
            .source(manifest)
            .map_err(|source| FindingSourceFailure::AtPath {
                path:   manifest_path.clone(),
                source: Box::new(source),
            })?;
        let content = match text {
            SourceText::Utf8(source) => source.lines.text(),
            SourceText::NotUtf8(text) => text,
        };
        let table =
            content
                .parse::<Table>()
                .map_err(|error| FindingSourceFailure::InvalidManifest {
                    path:   manifest_path,
                    source: Box::new(error),
                })?;
        if has_custom_targets(&table) {
            return Ok(HashMap::new());
        }

        let mut files = HashMap::new();
        for path in tip.crate_rust_paths(crate_name) {
            let file = self.file(source, SourceLocation { commit, path })?;
            if matches!(file.plain_links(), PlainLinks::Unproved) {
                return Ok(HashMap::new());
            }
            files.insert(path, file);
        }

        let src = join_repo(crate_name, "src");
        let bin = join_repo(&src, "bin");
        let roots = [join_repo(&src, "lib.rs"), join_repo(&src, "main.rs")];
        let mut pending: Vec<_> = roots
            .iter()
            .filter(|root| files.contains_key(root.as_str()))
            .map(|root| (root.clone(), vec!["crate".to_owned()]))
            .collect();
        let mut modules = HashMap::new();
        while let Some((path, chain)) = pending.pop() {
            match modules.entry(path.clone()) {
                Entry::Occupied(entry) if entry.get() != &chain => return Ok(HashMap::new()),
                Entry::Occupied(_) => continue,
                Entry::Vacant(entry) => {
                    entry.insert(chain.clone());
                },
            }
            let directory = if roots.contains(&path) || path.ends_with("/mod.rs") {
                path.rsplit_once('/').map_or("", |(directory, _)| directory)
            } else {
                path.strip_suffix(".rs").unwrap_or(&path)
            };
            let PlainLinks::Names(names) = files[path.as_str()].plain_links() else {
                return Ok(HashMap::new());
            };
            for name in names {
                let children = [
                    join_repo(directory, &format!("{name}.rs")),
                    join_repo(directory, &format!("{name}/mod.rs")),
                ];
                let mut existing = children.into_iter().filter(|child| {
                    files.contains_key(child.as_str()) && !tip_view::under(child, &bin)
                });
                if let Some(child) = existing.next()
                    && existing.next().is_none()
                {
                    let mut child_chain = chain.clone();
                    child_chain.push(name.clone());
                    pending.push((child, child_chain));
                }
            }
        }
        Ok(modules)
    }
}

impl DeclarationFile {
    fn plain_links(&self) -> &PlainLinks {
        self.links.get_or_init(|| plain_module_links(&self.words))
    }
}

fn join_repo(directory: &str, name: &str) -> String {
    if matches!(directory, "." | "") {
        name.to_owned()
    } else {
        format!("{directory}/{name}")
    }
}

fn has_custom_targets(table: &Table) -> bool {
    let has_path = |target: &Value| {
        target
            .as_table()
            .is_some_and(|target| target.contains_key("path"))
    };
    table.get("lib").is_some_and(has_path)
        || table
            .get("bin")
            .and_then(Value::as_array)
            .is_some_and(|targets| targets.iter().any(has_path))
}

fn plain_module_links(words: &[String]) -> PlainLinks {
    let attributes = word_syntax::decorated_items(words);
    if attributes.iter().any(|(&index, words_in_attribute)| {
        words[index] == "mod" && words_in_attribute.iter().any(|word| word == "path")
    }) {
        return PlainLinks::Unproved;
    }
    let mut names = Vec::new();
    let mut depth = 0;
    for (index, word) in words.iter().enumerate() {
        if word == "mod"
            && depth == 0
            && !attributes.contains_key(&index)
            && let Some(name) = words
                .get(index + 1)
                .filter(|name| word_syntax::identifier(name))
            && word_syntax::word(words, index + 2) == ";"
        {
            names.push(name.clone());
        }
        depth += i32::from(matches!(word.as_str(), "{" | "(" | "["))
            - i32::from(matches!(word.as_str(), "}" | ")" | "]"));
    }
    PlainLinks::Names(names)
}
