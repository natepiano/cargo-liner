use std::collections::BTreeMap;
use std::collections::BTreeSet;

use syn::Attribute;
use syn::File;
use syn::Item;
use syn::ItemExternCrate;
use syn::ItemMacro;
use syn::ItemMod;
use syn::ItemUse;
use syn::Path;
use syn::ext::IdentExt;
use syn::visit;
use syn::visit::Visit;

use crate::fixes::imports;

const STANDARD_MACROS: &[(&str, MacroCrates)] = &[
    ("assert", MacroCrates::StdAndCore),
    ("assert_eq", MacroCrates::StdAndCore),
    ("assert_ne", MacroCrates::StdAndCore),
    ("debug", MacroCrates::LogAndTracing),
    ("debug_assert", MacroCrates::StdAndCore),
    ("debug_assert_eq", MacroCrates::StdAndCore),
    ("debug_assert_ne", MacroCrates::StdAndCore),
    ("debug_span", MacroCrates::Tracing),
    ("enabled", MacroCrates::Tracing),
    ("eprint", MacroCrates::Std),
    ("eprintln", MacroCrates::Std),
    ("error", MacroCrates::LogAndTracing),
    ("error_span", MacroCrates::Tracing),
    ("event", MacroCrates::Tracing),
    ("event_enabled", MacroCrates::Tracing),
    ("format", MacroCrates::StdAndAlloc),
    ("info", MacroCrates::LogAndTracing),
    ("info_span", MacroCrates::Tracing),
    ("level_enabled", MacroCrates::Tracing),
    ("log", MacroCrates::Log),
    ("log_enabled", MacroCrates::Log),
    ("matches", MacroCrates::StdAndCore),
    ("panic", MacroCrates::StdAndCore),
    ("print", MacroCrates::Std),
    ("println", MacroCrates::Std),
    ("span", MacroCrates::Tracing),
    ("span_enabled", MacroCrates::Tracing),
    ("todo", MacroCrates::StdAndCore),
    ("trace", MacroCrates::LogAndTracing),
    ("trace_span", MacroCrates::Tracing),
    ("unreachable", MacroCrates::StdAndCore),
    ("vec", MacroCrates::StdAndAlloc),
    ("warn", MacroCrates::LogAndTracing),
    ("warn_span", MacroCrates::Tracing),
    ("write", MacroCrates::StdAndCore),
    ("writeln", MacroCrates::StdAndCore),
];

#[derive(Clone, Copy)]
enum MacroCrates {
    Std,
    StdAndCore,
    StdAndAlloc,
    Log,
    LogAndTracing,
    Tracing,
}

impl MacroCrates {
    fn includes(self, root: &str) -> bool {
        match self {
            Self::Std => root == "std",
            Self::StdAndCore => matches!(root, "std" | "core"),
            Self::StdAndAlloc => matches!(root, "std" | "alloc"),
            Self::Log => root == "log",
            Self::LogAndTracing => matches!(root, "log" | "tracing"),
            Self::Tracing => root == "tracing",
        }
    }
}

#[derive(Default)]
pub(super) struct MacroBindings {
    shadowing_names: BTreeSet<String>,
    prelude:         PreludeMacros,
}

#[derive(Default)]
enum PreludeMacros {
    #[default]
    Standard,
    Unknown,
}

impl MacroBindings {
    pub(super) fn collect(file: &File) -> Self {
        let mut bindings = Self::default();
        bindings.record_attributes(&file.attrs);
        bindings.visit_file(file);
        bindings
    }

    pub(super) fn binds_no_name(&self, path: &Path, module_imports: &MacroImports) -> bool {
        let mut segments = path.segments.iter();
        let Some(first) = segments.next() else {
            return false;
        };
        let first = first.ident.unraw().to_string();
        let Some(second) = segments.next() else {
            if path.leading_colon.is_some() || self.shadowing_names.contains(&first) {
                return false;
            }
            if let Some(target) = module_imports.targets.get(&first) {
                let Some((root, name)) = target.split_once("::") else {
                    return false;
                };
                return !self.shadowing_names.contains(root) && standard_path(root, name);
            }
            return matches!(self.prelude, PreludeMacros::Standard) && standard_path("std", &first);
        };
        segments.next().is_none()
            && (path.leading_colon.is_some() || !self.shadowing_names.contains(&first))
            && standard_path(&first, &second.ident.unraw().to_string())
    }

    fn record_attributes(&mut self, attributes: &[Attribute]) {
        if attributes.iter().any(|attribute| {
            attribute.path().is_ident("macro_use")
                || attribute.path().is_ident("no_implicit_prelude")
        }) {
            self.prelude = PreludeMacros::Unknown;
        }
    }
}

impl Visit<'_> for MacroBindings {
    fn visit_item_macro(&mut self, item: &ItemMacro) {
        if let Some(name) = &item.ident {
            self.shadowing_names.insert(name.unraw().to_string());
        }
    }

    fn visit_item_mod(&mut self, item: &ItemMod) {
        self.shadowing_names.insert(item.ident.unraw().to_string());
        self.record_attributes(&item.attrs);
        visit::visit_item_mod(self, item);
    }

    fn visit_item_extern_crate(&mut self, item: &ItemExternCrate) {
        if let Some((_, name)) = &item.rename {
            self.shadowing_names.insert(name.unraw().to_string());
        }
        self.record_attributes(&item.attrs);
    }

    fn visit_item_use(&mut self, item: &ItemUse) {
        for binding in imports::collect_use_bindings(&item.tree) {
            if let Some(name) = binding.name()
                && !binding
                    .path()
                    .split_once("::")
                    .is_some_and(|(root, leaf)| item.attrs.is_empty() && standard_path(root, leaf))
            {
                self.shadowing_names.insert(name.to_owned());
            }
        }
    }
}

#[derive(Default)]
pub(super) struct MacroImports {
    targets: BTreeMap<String, String>,
}

impl MacroImports {
    pub(super) fn collect(items: &[Item]) -> Self {
        let mut targets = BTreeMap::new();
        for item in items {
            if let Item::Use(item) = item
                && item.attrs.is_empty()
            {
                for binding in imports::collect_use_bindings(&item.tree) {
                    if let Some(name) = binding.name()
                        && binding
                            .path()
                            .split_once("::")
                            .is_some_and(|(root, name)| standard_path(root, name))
                    {
                        targets.insert(name.to_owned(), binding.path().to_owned());
                    }
                }
            }
        }
        Self { targets }
    }
}

fn standard_path(root: &str, name: &str) -> bool {
    STANDARD_MACROS
        .binary_search_by_key(&name, |entry| entry.0)
        .is_ok_and(|index| STANDARD_MACROS[index].1.includes(root))
}
