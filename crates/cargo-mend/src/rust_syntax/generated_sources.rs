//! Source ownership for one checked source snapshot.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::io;
use std::io::ErrorKind;
use std::path::Path;
use std::path::PathBuf;
use std::rc::Rc;

use super::ParsedSources;
use super::SourcePathKey;
use crate::config::GeneratedSourcesConfig;
use crate::selection::Selection;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SourceOwnership {
    Generated,
    Authored,
}

pub(crate) struct SourceOwnershipClassifier {
    sources:          Rc<ParsedSources>,
    config:           GeneratedSourcesConfig,
    crate_roots:      BTreeSet<SourcePathKey>,
    declared_aliases: BTreeMap<SourcePathKey, BTreeSet<PathBuf>>,
    classified:       RefCell<BTreeMap<SourcePathKey, SourceOwnership>>,
}

impl SourceOwnershipClassifier {
    pub(crate) fn for_selection(
        sources: Rc<ParsedSources>,
        config: &GeneratedSourcesConfig,
        selection: &Selection,
    ) -> Self {
        let crate_roots = selection.packages.iter().flat_map(|package| {
            package
                .targets
                .iter()
                .map(|target| target.src_path.as_path())
        });
        Self::new(sources, config, crate_roots)
    }

    fn new<'a>(
        sources: Rc<ParsedSources>,
        config: &GeneratedSourcesConfig,
        crate_roots: impl IntoIterator<Item = &'a Path>,
    ) -> Self {
        Self {
            crate_roots: crate_roots
                .into_iter()
                .map(|path| sources.path_key(path))
                .collect(),
            declared_aliases: sources.declared_source_aliases(),
            sources,
            config: config.clone(),
            classified: RefCell::default(),
        }
    }

    pub(crate) fn classify(&self, path: &Path) -> io::Result<SourceOwnership> {
        let key = self.sources.path_key(path);
        let mut visited = BTreeSet::new();
        let mut authored = Vec::new();
        let mut pending = vec![path.to_path_buf()];
        let mut classified = self.classified.borrow_mut();
        while let Some(path) = pending.pop() {
            let candidate = self.sources.path_key(&path);
            let declared = self.declared_aliases.get(&candidate);
            let known_mount =
                path == candidate.as_path() || declared.is_some_and(|paths| paths.contains(&path));
            match classified.get(&candidate) {
                Some(SourceOwnership::Generated) => {
                    classified.insert(key, SourceOwnership::Generated);
                    return Ok(SourceOwnership::Generated);
                },
                Some(SourceOwnership::Authored) if known_mount => continue,
                _ => {},
            }
            if !visited.insert(candidate.clone()) {
                continue;
            }
            let source = match self.sources.source(&path) {
                Ok(source) => source,
                Err(error) if error.kind() == ErrorKind::NotFound => {
                    authored.extend(known_mount.then_some(candidate));
                    continue;
                },
                Err(error) => return Err(error),
            };
            if source
                .text()
                .lines()
                .take(self.config.header_lines.get())
                .any(|line| {
                    self.config
                        .markers
                        .iter()
                        .any(|marker| line.contains(marker))
                })
            {
                classified.insert(candidate, SourceOwnership::Generated);
                classified.insert(key, SourceOwnership::Generated);
                return Ok(SourceOwnership::Generated);
            }
            if !self.crate_roots.contains(&candidate) {
                if let Some(paths) = declared {
                    for mount in paths {
                        extend_module_ancestors(mount, &mut pending);
                    }
                }
                if declared.is_none() || !known_mount {
                    extend_module_ancestors(&path, &mut pending);
                }
            }
            authored.extend(known_mount.then_some(candidate));
        }
        classified.extend(
            authored
                .into_iter()
                .map(|key| (key, SourceOwnership::Authored)),
        );
        Ok(SourceOwnership::Authored)
    }
}

fn extend_module_ancestors(path: &Path, pending: &mut Vec<PathBuf>) {
    let directory = if path.file_name().is_some_and(|name| name == "mod.rs") {
        path.parent().and_then(Path::parent)
    } else {
        path.parent()
    };
    if let Some(directory) = directory {
        let mut named_module = directory.as_os_str().to_os_string();
        named_module.push(".rs");
        pending.push(named_module.into());
        pending.push(directory.join("mod.rs"));
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    #[cfg(unix)]
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::symlink;
    use std::path::PathBuf;
    use std::rc::Rc;

    use anyhow::Result;

    use super::ParsedSources;
    use super::SourceOwnership;
    use super::SourceOwnershipClassifier;
    use crate::config::GeneratedSourcesConfig;

    #[test]
    fn generated_headers_and_parent_modules_protect_their_sources() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let text = [
            ("lib.rs", "// generated marker\n"),
            ("plain.rs", "fn plain() {}\n"),
            ("bindings/mod.rs", "// generated marker\n"),
            ("bindings/child.rs", "fn child() {}\n"),
            ("bindings/lib.rs", "fn local_module() {}\n"),
            ("bindings/custom.rs", "fn custom_target() {}\n"),
            ("bindings/nested/mod.rs", "fn nested() {}\n"),
            ("bindings/nested/leaf.rs", "fn leaf() {}\n"),
            ("other.rs", "\n// generated marker\n"),
            ("other/child.rs", "fn child() {}\n"),
            ("late.rs", "\n\n// generated marker\n"),
        ];
        let files: BTreeMap<PathBuf, String> = text
            .into_iter()
            .map(|(path, text)| (directory.path().join(path), text.to_string()))
            .collect();
        let sources = Rc::new(ParsedSources::from(&files));
        let config = GeneratedSourcesConfig {
            markers:      vec!["generated marker".to_string()],
            header_lines: 2.try_into()?,
        };
        let custom_target = directory.path().join("bindings/custom.rs");
        let classifier =
            SourceOwnershipClassifier::new(sources, &config, [custom_target.as_path()]);
        for (path, expected) in [
            ("lib.rs", SourceOwnership::Generated),
            ("plain.rs", SourceOwnership::Authored),
            ("bindings/mod.rs", SourceOwnership::Generated),
            ("bindings/child.rs", SourceOwnership::Generated),
            ("bindings/lib.rs", SourceOwnership::Generated),
            ("bindings/custom.rs", SourceOwnership::Authored),
            ("bindings/nested/mod.rs", SourceOwnership::Generated),
            ("bindings/nested/leaf.rs", SourceOwnership::Generated),
            ("other/child.rs", SourceOwnership::Generated),
            ("late.rs", SourceOwnership::Authored),
        ] {
            let path = directory.path().join(path);
            assert_eq!(classifier.classify(&path)?, expected, "{}", path.display());
            assert_eq!(classifier.classify(&path)?, expected);
        }
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn symlink_mounts_inherit_generation_and_stop_at_repeated_source_ids() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        fs::create_dir(root.join("bindings"))?;
        fs::create_dir(root.join("cycle"))?;
        fs::write(root.join("lib.rs"), "mod bindings;\nmod cycle;\n")?;
        fs::write(root.join("bindings/mod.rs"), "// @generated\nmod linked;\n")?;
        fs::write(root.join("shared.rs"), "fn shared() {}\n")?;
        fs::write(root.join("cycle/child.rs"), "mod child;\n")?;
        symlink("../shared.rs", root.join("bindings/linked.rs"))?;
        symlink("child.rs", root.join("cycle/mod.rs"))?;

        let sources = Rc::new(ParsedSources::default());
        sources.module_map(root);
        let crate_root = root.join("lib.rs");
        let classifier = SourceOwnershipClassifier::new(
            sources,
            &GeneratedSourcesConfig::default(),
            [crate_root.as_path()],
        );
        for (relative, ownership) in [
            ("shared.rs", SourceOwnership::Generated),
            ("bindings/linked.rs", SourceOwnership::Generated),
            ("cycle/child.rs", SourceOwnership::Authored),
            ("cycle/mod.rs", SourceOwnership::Authored),
        ] {
            assert_eq!(
                classifier.classify(&root.join(relative))?,
                ownership,
                "{relative}"
            );
        }
        Ok(())
    }
}
