use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::Path;

use proc_macro2::LineColumn;
use syn::Attribute;
use syn::Item;
use syn::ItemUse;
use syn::ext::IdentExt as _;

use super::offsets;
use super::scope;
use super::scope::EnclosingModule;
use super::scope::ScopeInfo;
use crate::fixes::imports;
use crate::fixes::imports::ImportSource;
use crate::fixes::imports::UseBinding;
use crate::rust_syntax;
use crate::rust_syntax::ModuleMap;
use crate::rust_syntax::ParsedSources;
use crate::rust_syntax::PathAnchor;

#[derive(Clone, Copy)]
pub(super) enum MacroNameUse {
    InModule(LineColumn),
    Opaque,
}

pub(super) struct MacroTargets<'a, 'text> {
    pub(super) names:          &'a BTreeMap<String, Vec<MacroNameUse>>,
    pub(super) module_map:     &'a ModuleMap,
    pub(super) source_file:    &'a Path,
    pub(super) parsed_sources: &'a ParsedSources,
    pub(super) source:         &'a ImportSource<'text>,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
enum ImportTarget {
    Crate(Vec<String>),
    External(String),
}

enum NameTarget {
    Found(ImportTarget),
    Absent,
    Unknown,
}

enum NameSource {
    Declaration,
    Explicit(String),
    Globs(Vec<String>),
    Unknown,
}

type ActiveBindings = BTreeSet<(Vec<String>, String)>;

impl MacroTargets<'_, '_> {
    pub(super) fn has_same_import(&self, module: &[String], name: &str, import: &str) -> bool {
        let Some(target) = self.import_target(module, import) else {
            return false;
        };
        self.named_import(module, name).is_some_and(|binding| {
            !self
                .import_chain(binding)
                .is_disjoint(&self.import_chain(target))
        })
    }

    pub(super) fn keeps_bindings(
        &self,
        scopes: &[ScopeInfo],
        insertion_scope: &ScopeInfo,
        name: &str,
        import: &str,
    ) -> bool {
        let Some(uses) = self.names.get(name) else {
            return true;
        };
        let Some(target) = self.import_target(&insertion_scope.module_path, import) else {
            return false;
        };
        let targets = self.import_chain(target);
        uses.iter().all(|name_use| {
            let MacroNameUse::InModule(position) = name_use else {
                return false;
            };
            let Ok(offset) = offsets::offset(self.source, *position) else {
                return false;
            };
            let EnclosingModule::Found(id) = scope::find_innermost_scope(scopes, offset) else {
                return false;
            };
            match self.name_target(&scopes[id].module_path, name, &mut BTreeSet::new()) {
                NameTarget::Found(binding) => !self.import_chain(binding).is_disjoint(&targets),
                NameTarget::Absent | NameTarget::Unknown => false,
            }
        })
    }

    fn import_chain(&self, start: ImportTarget) -> BTreeSet<ImportTarget> {
        let mut targets = BTreeSet::new();
        let mut current = start;
        while targets.insert(current.clone()) {
            let ImportTarget::Crate(path) = current else {
                break;
            };
            let Some((name, module)) = path.split_last() else {
                break;
            };
            let Some(target) = self.named_import(module, name) else {
                break;
            };
            current = target;
        }
        targets
    }

    /// An equivalent explicit import can replace the path without adding another import.
    fn named_import(&self, module: &[String], name: &str) -> Option<ImportTarget> {
        let NameSource::Explicit(spelling) =
            self.in_module(module, |items| name_source(items, name))?
        else {
            return None;
        };
        self.import_target(module, &spelling)
    }

    fn name_target(
        &self,
        module: &[String],
        name: &str,
        active: &mut ActiveBindings,
    ) -> NameTarget {
        let key = (module.to_vec(), name.to_owned());
        if !active.insert(key.clone()) {
            return NameTarget::Unknown;
        }
        let target = match self.in_module(module, |items| name_source(items, name)) {
            Some(NameSource::Declaration) => {
                let mut path = module.to_vec();
                path.push(name.to_owned());
                NameTarget::Found(ImportTarget::Crate(path))
            },
            Some(NameSource::Explicit(spelling)) => self
                .resolve_import(module, &spelling, active)
                .map_or(NameTarget::Unknown, NameTarget::Found),
            Some(NameSource::Globs(globs)) => self.glob_target(module, name, &globs, active),
            Some(NameSource::Unknown) | None => NameTarget::Unknown,
        };
        active.remove(&key);
        target
    }

    fn glob_target(
        &self,
        module: &[String],
        name: &str,
        globs: &[String],
        active: &mut ActiveBindings,
    ) -> NameTarget {
        let mut target = None;
        for glob in globs {
            let Some(ImportTarget::Crate(source)) = self.resolve_import(module, glob, active)
            else {
                return NameTarget::Unknown;
            };
            // A descendant can access the private names that an ancestor glob imports.
            if source.len() >= module.len() || !module.starts_with(&source) {
                return NameTarget::Unknown;
            }
            match self.name_target(&source, name, active) {
                NameTarget::Found(binding) => {
                    if target.as_ref().is_some_and(|other| other != &binding) {
                        return NameTarget::Unknown;
                    }
                    target = Some(binding);
                },
                NameTarget::Absent => {},
                NameTarget::Unknown => return NameTarget::Unknown,
            }
        }
        target.map_or(NameTarget::Absent, NameTarget::Found)
    }

    fn import_target(&self, module: &[String], import: &str) -> Option<ImportTarget> {
        self.resolve_import(module, import, &mut BTreeSet::new())
    }

    fn resolve_import(
        &self,
        module: &[String],
        import: &str,
        active: &mut ActiveBindings,
    ) -> Option<ImportTarget> {
        if let Some(external) = import.strip_prefix("::") {
            return Some(ImportTarget::External(external.to_owned()));
        }
        let segments: Vec<String> = import.split("::").map(str::to_owned).collect();
        let (prefix, suffix) = match PathAnchor::first(&segments)? {
            PathAnchor::Crate => (&[][..], &segments[1..]),
            PathAnchor::SelfMod => (module, &segments[1..]),
            PathAnchor::Super => {
                let depth = rust_syntax::leading_super_count(&segments);
                let prefix_length = module.len().checked_sub(depth)?;
                (&module[..prefix_length], &segments[depth..])
            },
            PathAnchor::Name => {
                let (root, suffix) = segments.split_first()?;
                let target = match self.name_target(module, root, active) {
                    NameTarget::Found(target) => target,
                    NameTarget::Absent => return Some(ImportTarget::External(import.to_owned())),
                    NameTarget::Unknown => return None,
                };
                return Some(match target {
                    ImportTarget::Crate(mut path) => {
                        path.extend_from_slice(suffix);
                        ImportTarget::Crate(path)
                    },
                    ImportTarget::External(path) if suffix.is_empty() => {
                        ImportTarget::External(path)
                    },
                    ImportTarget::External(path) => {
                        ImportTarget::External(format!("{path}::{}", suffix.join("::")))
                    },
                });
            },
            PathAnchor::SelfType => return None,
        };
        Some(ImportTarget::Crate(
            prefix.iter().chain(suffix).cloned().collect(),
        ))
    }

    fn in_module<T>(&self, module: &[String], read: impl Fn(&[Item]) -> T) -> Option<T> {
        let mut result = None;
        for (file, mount) in self
            .module_map
            .module_ancestors_in_owning_crate(self.source_file, module)
        {
            let parsed = self.parsed_sources.source(file).ok()?;
            let syntax = parsed.syntax().ok()?;
            let Some(items) = inline_module_items(&syntax.items, &module[mount.len()..]) else {
                continue;
            };
            if result.is_some() {
                return None;
            }
            result = Some(read(items));
        }
        result
    }
}

/// Check explicit names before glob imports. An uncertain source can still bind the name.
fn name_source(items: &[Item], name: &str) -> NameSource {
    let mut explicit = None;
    let mut globs = Vec::new();
    let mut opaque_glob = false;
    for item in items {
        if let Item::Use(item_use) = item {
            for binding in imports::collect_use_bindings(&item_use.tree) {
                match binding {
                    UseBinding::Named { name: bound, path } if bound == name => {
                        if explicit.is_some() || !item_use.attrs.is_empty() {
                            return NameSource::Unknown;
                        }
                        explicit = Some(NameSource::Explicit(import_spelling(item_use, path)));
                    },
                    UseBinding::Glob { path } if item_use.attrs.is_empty() => {
                        globs.push(import_spelling(item_use, path));
                    },
                    UseBinding::Glob { .. } => opaque_glob = true,
                    UseBinding::Named { .. } => {},
                }
            }
            continue;
        }
        if scope::declared_item_name(item).is_some_and(|ident| ident.unraw() == name) {
            if explicit.is_some() {
                return NameSource::Unknown;
            }
            explicit = Some(match item {
                Item::Enum(_)
                | Item::Mod(_)
                | Item::Struct(_)
                | Item::Trait(_)
                | Item::TraitAlias(_)
                | Item::Type(_)
                | Item::Union(_)
                    if imports::item_attributes(item)
                        .iter()
                        .all(preserves_declaration) =>
                {
                    NameSource::Declaration
                },
                _ => NameSource::Unknown,
            });
        }
        if matches!(item, Item::Macro(item_macro) if item_macro.ident.is_none()) {
            opaque_glob = true;
        }
    }
    explicit.unwrap_or(if opaque_glob {
        NameSource::Unknown
    } else {
        NameSource::Globs(globs)
    })
}

fn preserves_declaration(attribute: &Attribute) -> bool {
    const INERT_ATTRIBUTES: &[&str] = &[
        "allow",
        "automatically_derived",
        "cold",
        "deny",
        "deprecated",
        "derive",
        "doc",
        "expect",
        "forbid",
        "inline",
        "must_use",
        "non_exhaustive",
        "repr",
        "track_caller",
        "warn",
    ];

    attribute
        .path()
        .get_ident()
        .is_some_and(|ident| INERT_ATTRIBUTES.iter().any(|name| ident == *name))
}

fn import_spelling(item_use: &ItemUse, path: String) -> String {
    if item_use.leading_colon.is_some() {
        format!("::{path}")
    } else {
        path
    }
}

fn inline_module_items<'a>(items: &'a [Item], path: &[String]) -> Option<&'a [Item]> {
    path.iter().try_fold(items, |items, name| {
        let mut modules = items.iter().filter_map(|item| match item {
            Item::Mod(item_mod) if item_mod.ident == name.as_str() => Some(item_mod),
            _ => None,
        });
        let module = modules.next()?;
        if modules.next().is_some() {
            return None;
        }
        module.content.as_ref().map(|(_, items)| items.as_slice())
    })
}
