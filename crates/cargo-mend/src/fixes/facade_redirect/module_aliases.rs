use std::borrow::Cow;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::Path;

use anyhow::Context;
use anyhow::Result;
use syn::Ident;
use syn::Item;
use syn::ItemUse;
use syn::UseTree;
use syn::ext::IdentExt;

use super::module_path;
use super::self_crate_alias::SelfCrateAlias;
use crate::fixes::imports;
use crate::fixes::imports::UseBinding;
use crate::rust_syntax::ParsedSources;

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
    targets:      BTreeMap<Vec<String>, Vec<String>>,
    /// Each module's glob imports of other modules of the crate, e.g.
    /// `screen::tests` to `[screen]` for a `use super::*;` there.
    globs:        BTreeMap<Vec<String>, Vec<Vec<String>>>,
    modules:      BTreeSet<Vec<String>>,
    root_aliases: BTreeMap<String, Vec<String>>,
}

/// Whether a use item names this crate or an external crate.
pub(in crate::fixes) enum CrateUse<'a> {
    Local(Cow<'a, ItemUse>),
    External,
}

/// Module bindings precede the extern prelude, including bindings supplied by globs.
enum ModuleAliasResolution<'a> {
    Bound(&'a Vec<String>),
    Declared,
    Unbound,
    Blocked,
}

impl ModuleAliases {
    pub(in crate::fixes) fn is_root_self_alias(&self, name: &str) -> bool {
        self.root_aliases.contains_key(name)
    }

    pub(in crate::fixes) fn classify_use<'a>(&self, item: &'a ItemUse) -> CrateUse<'a> {
        if item.leading_colon.is_none() {
            return CrateUse::Local(Cow::Borrowed(item));
        }
        let UseTree::Path(prefix) = &item.tree else {
            return CrateUse::External;
        };
        if !self.is_root_self_alias(&prefix.ident.unraw().to_string()) {
            return CrateUse::External;
        }
        let mut local = item.clone();
        if let UseTree::Path(prefix) = &mut local.tree {
            prefix.ident = Ident::new("crate", prefix.ident.span());
        }
        CrateUse::Local(Cow::Owned(local))
    }

    pub(in crate::fixes) fn contains_module(&self, path: &[String]) -> bool {
        self.modules.contains(path)
    }

    /// The module bindings of `files`, each paired with the module path it
    /// occupies and read through `sources`. A binding counts only when its
    /// target is a module of these files: an inline `mod` or a file's own
    /// module.
    pub(in crate::fixes) fn collect<'file>(
        files: impl Iterator<Item = (&'file Path, &'file [String])>,
        sources: &ParsedSources,
    ) -> Result<Self> {
        let mut alias_collector = AliasCollector::default();
        for (file, module_path) in files {
            let parsed_source = sources
                .source(file)
                .with_context(|| format!("failed to read {}", file.display()))?;
            let syntax_file = parsed_source
                .syntax()
                .with_context(|| format!("failed to parse {}", file.display()))?;
            alias_collector.modules.insert(module_path.to_vec());
            alias_collector.collect_items(module_path, &syntax_file.items);
        }
        let AliasCollector {
            modules,
            bindings,
            globs,
            root_aliases,
        } = alias_collector;
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
            root_aliases: root_aliases
                .into_iter()
                .map(|name| (name, Vec::new()))
                .collect(),
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
        // `ModuleAliases::resolve_prefixes` bounds substitutions by `self.targets.len()` because a
        // binding whose target runs back through itself would loop, so the substitutions
        // stop once each binding could have been used once.
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
        match self.module_target(alias, visited) {
            ModuleAliasResolution::Bound(target) => Some(target),
            ModuleAliasResolution::Unbound => {
                alias.last().and_then(|name| self.root_aliases.get(name))
            },
            ModuleAliasResolution::Declared | ModuleAliasResolution::Blocked => None,
        }
    }

    fn module_target<'aliases>(
        &'aliases self,
        alias: &[String],
        visited: &mut BTreeSet<Vec<String>>,
    ) -> ModuleAliasResolution<'aliases> {
        if let Some(target) = self.targets.get(alias) {
            return ModuleAliasResolution::Bound(target);
        }
        let Some((name, module)) = alias.split_last() else {
            return ModuleAliasResolution::Unbound;
        };
        if self.modules.contains(alias) {
            return ModuleAliasResolution::Declared;
        }
        if !visited.insert(module.to_vec()) {
            // A glob cycle supplies no new binding. Search the other glob sources.
            return ModuleAliasResolution::Unbound;
        }
        let mut resolution = ModuleAliasResolution::Unbound;
        for source in self.globs.get(module).into_iter().flatten() {
            let mut through_glob = source.clone();
            through_glob.push(name.clone());
            match self.module_target(&through_glob, &mut visited.clone()) {
                ModuleAliasResolution::Bound(target) => {
                    if let ModuleAliasResolution::Bound(previous) = resolution
                        && previous != target
                    {
                        return ModuleAliasResolution::Blocked;
                    }
                    resolution = ModuleAliasResolution::Bound(target);
                },
                ModuleAliasResolution::Blocked => return ModuleAliasResolution::Blocked,
                // A glob cannot import a private module. A visible module conflicts with
                // another glob binding, so the compiler rejects that original crate.
                ModuleAliasResolution::Declared => {
                    if matches!(resolution, ModuleAliasResolution::Unbound) {
                        resolution = ModuleAliasResolution::Declared;
                    }
                },
                ModuleAliasResolution::Unbound => {},
            }
        }
        resolution
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
    modules:      BTreeSet<Vec<String>>,
    bindings:     BTreeMap<Vec<String>, Vec<String>>,
    globs:        BTreeMap<Vec<String>, Vec<Vec<String>>>,
    root_aliases: BTreeSet<String>,
}

impl AliasCollector {
    /// Records the `use` bindings among `items`, declared in `module_path`, and
    /// walks every inline `mod` among them.
    fn collect_items(&mut self, module_path: &[String], items: &[Item]) {
        for item in items {
            if let SelfCrateAlias::Named(name) = SelfCrateAlias::classify(item) {
                let name = name.unraw().to_string();
                let mut alias = module_path.to_vec();
                alias.push(name.clone());
                self.bindings.insert(alias, Vec::new());
                if module_path.is_empty() {
                    self.root_aliases.insert(name);
                }
                continue;
            }
            match item {
                Item::Use(item_use) if item_use.leading_colon.is_none() => {
                    for binding in imports::collect_use_bindings(&item_use.tree) {
                        let segments = binding
                            .path()
                            .split("::")
                            .map(str::to_string)
                            .collect::<Vec<_>>();
                        let Some(target) = module_path::absolute_use_path(module_path, &segments)
                        else {
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

    const APP_CAPTURE_FRAME_PATH: &str = "app::screen::capture::Frame";
    const APP_SCREEN_MODULE_PATH: &str = "app::screen";
    const CYCLE_FIRST_MODULE_PATH: &str = "a::b";
    const CYCLE_ITEM_NAME: &str = "Item";
    const CYCLE_ITEM_PATH: &str = "a::b::Item";
    const CYCLE_SECOND_MODULE_PATH: &str = "c::d";
    const PLATFORM_STREAM_FRAME_PATH: &str = "platform::stream::Frame";
    const PLATFORM_STREAM_MODULE_PATH: &str = "platform::stream";
    const READINESS_MODULE_PATH: &str = "readiness";
    const READINESS_TESTS_MODULE_PATH: &str = "readiness::tests";
    const READINESS_TOOL_ALIAS_PATH: &str = "readiness::tool";
    const RECIPE_MODULE_PATH: &str = "readiness::tests::recipe";
    const RECIPE_TOOL_BUILD_PATH: &str = "readiness::tests::recipe::tool::staging::build";
    const SCREEN_CAPTURE_FRAME_PATH: &str = "screen::capture::Frame";
    const SCREEN_CAPTURE_MODULE_PATH: &str = "screen::capture";
    const SHADOWING_TOOL_ITEM_PATH: &str = "readiness::tests::tool::Item";
    const SHADOWING_TOOL_MODULE_PATH: &str = "readiness::tests::tool";
    const TOOL_BUILD_PATH: &str = "tool::staging::build";
    const TOOL_MODULE_PATH: &str = "tool";
    const VIDEO_CAPTURE_MODULE_PATH: &str = "video::screen::capture";
    const VIDEO_SCREEN_MODULE_PATH: &str = "video::screen";

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
        let aliases = aliases(&[(SCREEN_CAPTURE_MODULE_PATH, PLATFORM_STREAM_MODULE_PATH)]);
        assert_eq!(
            aliases.resolve(path(SCREEN_CAPTURE_FRAME_PATH)),
            path(PLATFORM_STREAM_FRAME_PATH)
        );
    }

    #[test]
    fn a_chain_of_bindings_resolves_to_the_last_module() {
        let aliases = aliases(&[
            (APP_SCREEN_MODULE_PATH, VIDEO_SCREEN_MODULE_PATH),
            (VIDEO_CAPTURE_MODULE_PATH, PLATFORM_STREAM_MODULE_PATH),
        ]);
        assert_eq!(
            aliases.resolve(path(APP_CAPTURE_FRAME_PATH)),
            path(PLATFORM_STREAM_FRAME_PATH)
        );
    }

    #[test]
    fn the_last_segment_is_never_replaced() {
        let aliases = aliases(&[(SCREEN_CAPTURE_MODULE_PATH, PLATFORM_STREAM_MODULE_PATH)]);
        assert_eq!(
            aliases.resolve(path(SCREEN_CAPTURE_MODULE_PATH)),
            path(SCREEN_CAPTURE_MODULE_PATH)
        );
    }

    #[test]
    fn a_module_path_resolves_its_last_segment() {
        let aliases = aliases(&[(SCREEN_CAPTURE_MODULE_PATH, PLATFORM_STREAM_MODULE_PATH)]);
        assert_eq!(
            aliases.resolve_module(path(SCREEN_CAPTURE_MODULE_PATH)),
            path(PLATFORM_STREAM_MODULE_PATH)
        );
    }

    #[test]
    fn a_binding_reached_through_a_glob_of_the_parent_resolves() {
        let mut aliases = aliases(&[(READINESS_TOOL_ALIAS_PATH, TOOL_MODULE_PATH)]);
        aliases.globs.insert(
            path(READINESS_TESTS_MODULE_PATH),
            vec![path(READINESS_MODULE_PATH)],
        );
        aliases.globs.insert(
            path(RECIPE_MODULE_PATH),
            vec![path(READINESS_TESTS_MODULE_PATH)],
        );
        assert_eq!(
            aliases.resolve(path(RECIPE_TOOL_BUILD_PATH)),
            path(TOOL_BUILD_PATH)
        );
    }

    #[test]
    fn a_module_of_the_crate_shadows_a_glob() {
        let mut aliases = aliases(&[(READINESS_TOOL_ALIAS_PATH, TOOL_MODULE_PATH)]);
        aliases.globs.insert(
            path(READINESS_TESTS_MODULE_PATH),
            vec![path(READINESS_MODULE_PATH)],
        );
        aliases.modules.insert(path(SHADOWING_TOOL_MODULE_PATH));
        assert_eq!(
            aliases.resolve(path(SHADOWING_TOOL_ITEM_PATH)),
            path(SHADOWING_TOOL_ITEM_PATH)
        );
    }

    #[test]
    fn a_cycle_of_bindings_stops() {
        let aliases = aliases(&[
            (CYCLE_FIRST_MODULE_PATH, CYCLE_SECOND_MODULE_PATH),
            (CYCLE_SECOND_MODULE_PATH, CYCLE_FIRST_MODULE_PATH),
        ]);
        let resolved = aliases.resolve(path(CYCLE_ITEM_PATH));
        assert_eq!(resolved.last(), Some(&CYCLE_ITEM_NAME.to_string()));
    }
}
