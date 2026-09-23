use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use anyhow::Context;
use anyhow::Result;
use syn::Item;
use syn::ext::IdentExt;
use syn::parse_file;

use super::module_path::absolute_use_path;
use crate::fixes::imports::UseBinding;
use crate::fixes::imports::collect_use_bindings;

/// The module-level `use` items of a crate that bind one of its modules under
/// another path, such as `pub(super) use crate::platform::stream as capture;`
/// in `screen`, which lets any file write `crate::screen::capture::Frame` for
/// `crate::platform::stream::Frame`.
///
/// A caller file only sees its own `use` items, so a path through a binding
/// declared in another file, or reached through a glob such as `use super::*;`
/// in a `mod tests`, is resolved here before it is compared with a redirect's
/// facade path.
#[derive(Default)]
pub(in crate::fixes) struct ModuleAliases {
    /// The crate-relative path of each binding, e.g. `screen::capture`, mapped
    /// to the crate-relative path of the module it binds.
    targets: BTreeMap<Vec<String>, Vec<String>>,
    /// Each module's glob imports of other modules of the crate, e.g.
    /// `screen::tests` to `[screen]` for a `use super::*;` there.
    globs:   BTreeMap<Vec<String>, Vec<Vec<String>>>,
    modules: BTreeSet<Vec<String>>,
}

impl ModuleAliases {
    /// The module bindings of `files`, each paired with the module path it
    /// occupies. A binding counts only when its target is a module of these
    /// files: an inline `mod` or a file's own module.
    pub(in crate::fixes) fn collect<'file>(
        files: impl Iterator<Item = (&'file Path, &'file [String])>,
    ) -> Result<Self> {
        let mut collector = AliasCollector::default();
        for (file, module_path) in files {
            let source = fs::read_to_string(file)
                .with_context(|| format!("failed to read {}", file.display()))?;
            let syntax = parse_file(&source)
                .with_context(|| format!("failed to parse {}", file.display()))?;
            collector.modules.insert(module_path.to_vec());
            collector.collect_items(module_path, &syntax.items);
        }
        let AliasCollector {
            modules,
            bindings,
            globs,
        } = collector;
        let targets = bindings
            .into_iter()
            .filter(|(_, target)| modules.contains(target))
            .collect();
        let globs = globs
            .into_iter()
            .map(|(module, sources)| {
                let sources = sources
                    .into_iter()
                    .filter(|source| modules.contains(source))
                    .collect::<Vec<_>>();
                (module, sources)
            })
            .filter(|(_, sources)| !sources.is_empty())
            .collect();
        Ok(Self {
            targets,
            globs,
            modules,
        })
    }

    /// `path` with every leading module binding replaced by the module it
    /// binds, so `screen::capture::Frame` becomes `platform::stream::Frame`.
    /// The last segment is never replaced: a `use` of an item is itself a
    /// caller, rewritten on its own line.
    pub(in crate::fixes) fn resolve(&self, path: Vec<String>) -> Vec<String> {
        self.resolve_prefixes(path, LastSegment::Kept)
    }

    /// The module `path` names, with the last segment replaced too when it is
    /// a binding: `screen::capture` becomes `platform::stream`.
    pub(in crate::fixes) fn resolve_module(&self, path: Vec<String>) -> Vec<String> {
        self.resolve_prefixes(path, LastSegment::Resolved)
    }

    fn resolve_prefixes(&self, path: Vec<String>, last_segment: LastSegment) -> Vec<String> {
        let mut resolved = path;
        // A binding whose target runs back through itself would loop, so the
        // substitutions stop once each binding could have been used once.
        for _ in 0..=self.targets.len() {
            let longest = match last_segment {
                LastSegment::Kept => resolved.len().saturating_sub(1),
                LastSegment::Resolved => resolved.len(),
            };
            let Some((length, target)) = (1..=longest).find_map(|length| {
                self.target(&resolved[..length], &mut BTreeSet::new())
                    .map(|target| (length, target))
            }) else {
                break;
            };
            resolved = target.iter().chain(&resolved[length..]).cloned().collect();
        }
        resolved
    }

    /// The module `alias` binds: a `use` binding in its parent module, or one
    /// a glob import in that parent brings in. A module of the crate at
    /// `alias` shadows every glob, and `visited` stops a cycle of globs.
    fn target<'aliases>(
        &'aliases self,
        alias: &[String],
        visited: &mut BTreeSet<Vec<String>>,
    ) -> Option<&'aliases Vec<String>> {
        if let Some(target) = self.targets.get(alias) {
            return Some(target);
        }
        let (name, module) = alias.split_last()?;
        if self.modules.contains(alias) || !visited.insert(module.to_vec()) {
            return None;
        }
        self.globs.get(module)?.iter().find_map(|source| {
            let mut through_glob = source.clone();
            through_glob.push(name.clone());
            self.target(&through_glob, visited)
        })
    }
}

/// Whether [`ModuleAliases::resolve_prefixes`] may replace a path's last
/// segment.
#[derive(Clone, Copy)]
enum LastSegment {
    Kept,
    Resolved,
}

/// The module paths and the `use` bindings `ModuleAliases::collect` has seen.
#[derive(Default)]
struct AliasCollector {
    modules:  BTreeSet<Vec<String>>,
    bindings: BTreeMap<Vec<String>, Vec<String>>,
    globs:    BTreeMap<Vec<String>, Vec<Vec<String>>>,
}

impl AliasCollector {
    /// Records the `use` bindings among `items`, declared in `module_path`, and
    /// walks every inline `mod` among them.
    fn collect_items(&mut self, module_path: &[String], items: &[Item]) {
        for item in items {
            match item {
                Item::Use(item_use) if item_use.leading_colon.is_none() => {
                    for binding in collect_use_bindings(&item_use.tree) {
                        let segments = binding
                            .path()
                            .split("::")
                            .map(str::to_string)
                            .collect::<Vec<_>>();
                        let Some(target) = absolute_use_path(module_path, &segments) else {
                            continue;
                        };
                        match binding {
                            UseBinding::Named { name, .. } => {
                                let mut alias = module_path.to_vec();
                                alias.push(name);
                                if alias != target {
                                    self.bindings.insert(alias, target);
                                }
                            },
                            UseBinding::Glob { .. } => {
                                self.globs
                                    .entry(module_path.to_vec())
                                    .or_default()
                                    .push(target);
                            },
                        }
                    }
                },
                Item::Mod(item_mod) => {
                    if let Some((_, inline_items)) = &item_mod.content {
                        let mut inline_module = module_path.to_vec();
                        inline_module.push(item_mod.ident.unraw().to_string());
                        self.modules.insert(inline_module.clone());
                        self.collect_items(&inline_module, inline_items);
                    }
                },
                _ => {},
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ModuleAliases;

    fn path(text: &str) -> Vec<String> { text.split("::").map(str::to_string).collect() }

    fn aliases(entries: &[(&str, &str)]) -> ModuleAliases {
        ModuleAliases {
            targets: entries
                .iter()
                .map(|(alias, target)| (path(alias), path(target)))
                .collect(),
            ..ModuleAliases::default()
        }
    }

    #[test]
    fn a_path_through_a_module_binding_resolves_to_the_bound_module() {
        let aliases = aliases(&[("screen::capture", "platform::stream")]);
        assert_eq!(
            aliases.resolve(path("screen::capture::Frame")),
            path("platform::stream::Frame")
        );
    }

    #[test]
    fn a_chain_of_bindings_resolves_to_the_last_module() {
        let aliases = aliases(&[
            ("app::screen", "video::screen"),
            ("video::screen::capture", "platform::stream"),
        ]);
        assert_eq!(
            aliases.resolve(path("app::screen::capture::Frame")),
            path("platform::stream::Frame")
        );
    }

    #[test]
    fn the_last_segment_is_never_replaced() {
        let aliases = aliases(&[("screen::capture", "platform::stream")]);
        assert_eq!(
            aliases.resolve(path("screen::capture")),
            path("screen::capture")
        );
    }

    #[test]
    fn a_module_path_resolves_its_last_segment() {
        let aliases = aliases(&[("screen::capture", "platform::stream")]);
        assert_eq!(
            aliases.resolve_module(path("screen::capture")),
            path("platform::stream")
        );
    }

    #[test]
    fn a_binding_reached_through_a_glob_of_the_parent_resolves() {
        let mut aliases = aliases(&[("readiness::tool", "tool")]);
        aliases
            .globs
            .insert(path("readiness::tests"), vec![path("readiness")]);
        aliases.globs.insert(
            path("readiness::tests::recipe"),
            vec![path("readiness::tests")],
        );
        assert_eq!(
            aliases.resolve(path("readiness::tests::recipe::tool::staging::build")),
            path("tool::staging::build")
        );
    }

    #[test]
    fn a_module_of_the_crate_shadows_a_glob() {
        let mut aliases = aliases(&[("readiness::tool", "tool")]);
        aliases
            .globs
            .insert(path("readiness::tests"), vec![path("readiness")]);
        aliases.modules.insert(path("readiness::tests::tool"));
        assert_eq!(
            aliases.resolve(path("readiness::tests::tool::Item")),
            path("readiness::tests::tool::Item")
        );
    }

    #[test]
    fn a_cycle_of_bindings_stops() {
        let aliases = aliases(&[("a::b", "c::d"), ("c::d", "a::b")]);
        let resolved = aliases.resolve(path("a::b::Item"));
        assert_eq!(resolved.last(), Some(&"Item".to_string()));
    }
}
