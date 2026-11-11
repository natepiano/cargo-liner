use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::mem;
use std::ops::Range;
use std::path::Path;

use anyhow::Result;
use syn::Block;
use syn::File;
use syn::GenericParam;
use syn::Item;
use syn::ItemMod;
use syn::ItemUse;
use syn::Macro;
use syn::PatIdent;
use syn::Path as SynPath;
use syn::Stmt;
use syn::Visibility;
use syn::ext::IdentExt;
use syn::spanned::Spanned;
use syn::visit;
use syn::visit::Visit;

use crate::fixes::facade_redirect;
use crate::fixes::facade_redirect::CallerBuilds;
use crate::fixes::facade_redirect::CallerRedirect;
use crate::fixes::facade_redirect::FacadeRedirect;
use crate::fixes::facade_redirect::FileRedirects;
use crate::fixes::facade_redirect::IdentifierUse;
use crate::fixes::facade_redirect::ModuleAliases;
use crate::fixes::facade_redirect::SourceLines;
use crate::fixes::facade_redirect::TargetSide;
use crate::fixes::imports;
use crate::fixes::imports::OwnedFix;
use crate::fixes::imports::UseBinding;
use crate::fixes::imports::UseFix;
use crate::rust_syntax;
use crate::rust_syntax::AttributeTestGate;
use crate::rust_syntax::ParsedSources;
use crate::rust_syntax::PathAnchor;

/// Caller imports and refusals retain their indexes in the redirect slice.
#[derive(Default)]
struct GlobCallerEdits {
    fixes:        Vec<OwnedFix>,
    declined:     BTreeSet<usize>,
    outside_used: BTreeMap<usize, CallerBuilds>,
}

pub(super) struct FileCallerPlan {
    pub(super) callers:  FileRedirects,
    pub(super) declined: BTreeSet<usize>,
}

struct GlobCallerRewriter<'a> {
    file:           &'a Path,
    source:         &'a str,
    syntax:         &'a File,
    lines:          SourceLines<'a>,
    module:         Vec<String>,
    redirects:      &'a [FacadeRedirect],
    aliases:        &'a ModuleAliases,
    cfg_test_depth: usize,
    block_aliases:  Vec<BTreeSet<String>>,
    edits:          GlobCallerEdits,
}

impl Visit<'_> for GlobCallerRewriter<'_> {
    fn visit_file(&mut self, node: &File) {
        self.inspect_scope(&node.items.iter().collect::<Vec<_>>(), 0..self.source.len());
        visit::visit_file(self, node);
    }

    fn visit_item_mod(&mut self, node: &ItemMod) {
        let Some((brace, items)) = &node.content else {
            return;
        };
        let test_only =
            rust_syntax::attribute_test_gate(&node.attrs) == AttributeTestGate::DirectCfgTest;
        let block_aliases = mem::take(&mut self.block_aliases);
        self.cfg_test_depth += usize::from(test_only);
        self.module.push(node.ident.unraw().to_string());
        if let Ok((start, end)) = self
            .lines
            .span_range(brace.span.open().end(), brace.span.close().start())
        {
            self.inspect_scope(&items.iter().collect::<Vec<_>>(), start..end);
        } else {
            self.edits.declined.extend(0..self.redirects.len());
        }
        visit::visit_item_mod(self, node);
        self.module.pop();
        self.cfg_test_depth -= usize::from(test_only);
        self.block_aliases = block_aliases;
    }

    fn visit_block(&mut self, node: &Block) {
        let items = node
            .stmts
            .iter()
            .filter_map(|statement| match statement {
                Stmt::Item(item) => Some(item),
                _ => None,
            })
            .collect::<Vec<_>>();
        let aliases = items
            .iter()
            .filter_map(|item| match item {
                Item::Use(item) => Some(imports::collect_use_bindings(&item.tree)),
                _ => None,
            })
            .flatten()
            .filter_map(|binding| match binding {
                UseBinding::Named { name, .. } => Some(name),
                UseBinding::Glob { .. } => None,
            })
            .collect();
        self.block_aliases.push(aliases);
        if let Ok((start, end)) = self.lines.span_range(
            node.brace_token.span.open().end(),
            node.brace_token.span.close().start(),
        ) {
            self.inspect_scope(&items, start..end);
        } else {
            self.edits.declined.extend(0..self.redirects.len());
        }
        visit::visit_block(self, node);
        self.block_aliases.pop();
    }
}

impl GlobCallerRewriter<'_> {
    fn inspect_scope(&mut self, items: &[&Item], range: Range<usize>) {
        let uses = items.iter().filter_map(|item| match item {
            Item::Use(item_use) => Some(item_use),
            _ => None,
        });
        let uses = uses.collect::<Vec<_>>();
        let mut globs = Vec::new();
        let mut bindings = BTreeMap::new();
        for item in &uses {
            for binding in imports::collect_use_bindings(&item.tree) {
                match binding {
                    UseBinding::Glob { path } => globs.push((*item, path)),
                    UseBinding::Named { name, path } => {
                        bindings.insert(name, path);
                    },
                }
            }
        }
        if globs.is_empty() {
            return;
        }
        let text = &self.source[range];
        let mut hidden = HiddenNames::default();
        if let Ok(block) = syn::parse_str::<Block>(&format!("{{{text}}}")) {
            hidden.visit_block(&block);
        } else {
            hidden = HiddenNames::Possible;
        }
        for (item, path) in &globs {
            if item.leading_colon.is_some() {
                continue;
            }
            let segments = path.split("::").map(str::to_string).collect::<Vec<_>>();
            let written_target = facade_redirect::absolute_use_path(&self.module, &segments);
            let target = self.glob_source(&segments);
            if matches!(target, GlobSource::External) {
                continue;
            }
            let mut paths = Vec::new();
            let mut facts = Vec::new();
            for (index, redirect) in self.redirects.iter().enumerate() {
                let Some((name, owner)) = redirect.facade_path.split_last() else {
                    continue;
                };
                let known = matches!(&target, GlobSource::Module(path) if path == owner);
                // GlobReach covers direct descendant globs in module scopes.
                if known
                    && self.block_aliases.is_empty()
                    && self.module.len() > owner.len()
                    && self.module.starts_with(owner)
                    && written_target.as_deref() == Some(owner)
                {
                    continue;
                }
                if matches!(&target, GlobSource::Module(path) if path != owner) {
                    continue;
                }
                let occurrence = facade_redirect::word_occurs_outside(text, [], name);
                if occurrence == IdentifierUse::Absent && hidden == HiddenNames::Absent {
                    continue;
                }
                let CallerRedirect::Redirect { path: target, side } =
                    redirect.target_for(&self.module)
                else {
                    continue;
                };
                if !known
                    || hidden == HiddenNames::Possible
                    || occurrence != IdentifierUse::Present
                    || globs.len() != 1
                    || bindings.contains_key(name)
                    || items.iter().any(|item| declares_name(item, name))
                    || !matches!(item.vis, Visibility::Inherited)
                    || !BareNameUse::proven(text, name)
                    || !BareNameUse::unshadowed(self.syntax, name)
                {
                    self.edits.declined.insert(index);
                    continue;
                }
                paths.push(format!("crate::{}", target.join("::")));
                facts.push(index);
                self.record_outside_use(index, side);
            }
            self.insert_imports(item, &paths, facts);
        }
    }

    fn record_outside_use(&mut self, index: usize, side: TargetSide) {
        if side == TargetSide::Outside {
            let builds = if self.cfg_test_depth == 0 {
                CallerBuilds::Every
            } else {
                CallerBuilds::TestOnly
            };
            let widest = self.edits.outside_used.entry(index).or_insert(builds);
            *widest = (*widest).max(builds);
        }
    }

    fn glob_source(&self, segments: &[String]) -> GlobSource {
        if let Some(name) = segments.first()
            && PathAnchor::from(name.as_str()) == PathAnchor::Name
        {
            if self
                .block_aliases
                .iter()
                .rev()
                .any(|aliases| aliases.contains(name))
            {
                return GlobSource::Unresolved;
            }
            let first = facade_redirect::absolute_use_path(&self.module, &segments[..1]);
            if !first.is_some_and(|path| {
                self.aliases
                    .contains_module(&self.aliases.resolve_module(path))
            }) {
                return GlobSource::External;
            }
        }
        facade_redirect::absolute_use_path(&self.module, segments).map_or(
            GlobSource::Unresolved,
            |path| {
                let resolved = self.aliases.resolve_module(path);
                if self.aliases.contains_module(&resolved) {
                    GlobSource::Module(resolved)
                } else {
                    GlobSource::Unresolved
                }
            },
        )
    }

    fn insert_imports(&mut self, item: &ItemUse, paths: &[String], facts: Vec<usize>) {
        if paths.is_empty() {
            return;
        }
        let Ok((start, end)) = facade_redirect::item_use_byte_range(&self.lines, item) else {
            self.edits.declined.extend(facts);
            return;
        };
        let Ok(position) = self.lines.position(item.tree.span().start()) else {
            self.edits.declined.extend(facts);
            return;
        };
        let header = &self.source[start..SourceLines::offset(position)];
        let indent = self.lines.indent_before(start);
        let mut replacement = String::new();
        for path in paths {
            replacement.push('\n');
            replacement.push_str(indent);
            replacement.push_str(header);
            replacement.push_str(path);
            replacement.push(';');
        }
        self.edits.fixes.push(OwnedFix {
            fix: UseFix {
                path: self.file.to_path_buf(),
                start: end,
                end,
                replacement,
                import_group: None,
            },
            facts,
        });
    }
}

enum GlobSource {
    External,
    Module(Vec<String>),
    Unresolved,
}

/// Macros and nested modules can use names that the current scope does not show.
#[derive(Default, PartialEq, Eq)]
enum HiddenNames {
    #[default]
    Absent,
    Possible,
}

impl Visit<'_> for HiddenNames {
    fn visit_macro(&mut self, _: &Macro) { *self = Self::Possible; }

    fn visit_item_mod(&mut self, _: &ItemMod) { *self = Self::Possible; }
}

#[derive(PartialEq, Eq)]
enum NameReference {
    Absent,
    Present,
}

#[derive(PartialEq, Eq)]
enum NameBinding {
    Unshadowed,
    Shadowed,
}

struct BareNameUse<'a> {
    name:      &'a str,
    reference: NameReference,
    binding:   NameBinding,
}

impl BareNameUse<'_> {
    fn unshadowed(syntax: &File, name: &str) -> bool {
        let mut usage = BareNameUse {
            name,
            reference: NameReference::Absent,
            binding: NameBinding::Unshadowed,
        };
        usage.visit_file(syntax);
        usage.binding == NameBinding::Unshadowed
    }

    fn proven(text: &str, name: &str) -> bool {
        let Ok(block) = syn::parse_str::<Block>(&format!("{{{text}}}")) else {
            return false;
        };
        let mut usage = BareNameUse {
            name,
            reference: NameReference::Absent,
            binding: NameBinding::Unshadowed,
        };
        usage.visit_block(&block);
        usage.reference == NameReference::Present && usage.binding == NameBinding::Unshadowed
    }
}

impl Visit<'_> for BareNameUse<'_> {
    fn visit_generic_param(&mut self, node: &GenericParam) {
        if match node {
            GenericParam::Type(parameter) => parameter.ident.unraw() == self.name,
            GenericParam::Const(parameter) => parameter.ident.unraw() == self.name,
            GenericParam::Lifetime(_) => false,
        } {
            self.binding = NameBinding::Shadowed;
        }
        visit::visit_generic_param(self, node);
    }

    fn visit_path(&mut self, node: &SynPath) {
        if node.leading_colon.is_none()
            && node
                .segments
                .first()
                .is_some_and(|segment| segment.ident.unraw() == self.name)
        {
            self.reference = NameReference::Present;
        }
        visit::visit_path(self, node);
    }

    fn visit_pat_ident(&mut self, node: &PatIdent) {
        if node.ident.unraw() == self.name {
            self.binding = NameBinding::Shadowed;
        }
        visit::visit_pat_ident(self, node);
    }

    fn visit_item(&mut self, node: &Item) {
        if declares_name(node, self.name) {
            self.binding = NameBinding::Shadowed;
        }
        visit::visit_item(self, node);
    }

    fn visit_item_use(&mut self, node: &ItemUse) {
        if imports::collect_use_bindings(&node.tree)
            .iter()
            .any(|binding| binding.name() == Some(self.name))
        {
            self.binding = NameBinding::Shadowed;
        }
    }
}

pub(super) fn plan_file_callers(
    file: &Path,
    sources: &ParsedSources,
    module: &[String],
    redirects: &[FacadeRedirect],
    aliases: &ModuleAliases,
) -> Result<FileCallerPlan> {
    let globs = redirect_glob_callers(file, sources, module, redirects, aliases)?;
    let mut callers = facade_redirect::redirect_callers_in_file(
        file,
        sources,
        module.to_vec(),
        redirects,
        aliases,
    )?;
    callers.fixes.extend(globs.fixes);
    for (index, builds) in globs.outside_used {
        let widest = callers.outside_used.entry(index).or_insert(builds);
        *widest = (*widest).max(builds);
    }
    Ok(FileCallerPlan {
        callers,
        declined: globs.declined,
    })
}

fn redirect_glob_callers(
    file: &Path,
    sources: &ParsedSources,
    module: &[String],
    redirects: &[FacadeRedirect],
    aliases: &ModuleAliases,
) -> Result<GlobCallerEdits> {
    let source = sources.source(file)?;
    let syntax = source.syntax()?;
    let mut rewriter = GlobCallerRewriter {
        file,
        source: source.text(),
        syntax,
        lines: SourceLines::new(source.text()),
        module: module.to_vec(),
        redirects,
        aliases,
        cfg_test_depth: 0,
        block_aliases: Vec::new(),
        edits: GlobCallerEdits::default(),
    };
    rewriter.visit_file(syntax);
    Ok(rewriter.edits)
}

fn declares_name(item: &Item, name: &str) -> bool {
    match item {
        Item::Const(item) => item.ident.unraw() == name,
        Item::Enum(item) => item.ident.unraw() == name,
        Item::Fn(item) => item.sig.ident.unraw() == name,
        Item::Mod(item) => item.ident.unraw() == name,
        Item::Static(item) => item.ident.unraw() == name,
        Item::Struct(item) => item.ident.unraw() == name,
        Item::Trait(item) => item.ident.unraw() == name,
        Item::TraitAlias(item) => item.ident.unraw() == name,
        Item::Type(item) => item.ident.unraw() == name,
        Item::Union(item) => item.ident.unraw() == name,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::process::Command;

    use anyhow::Result;

    use crate::fixes::imports;
    use crate::fixes::imports::ValidatedFixSet;
    use crate::fixes::subtree_reexport::scan;
    use crate::fixes::subtree_reexport::scan::SubtreeReexportScan;
    use crate::reporting::AncestorReexport;
    use crate::reporting::ModuleForm;
    use crate::reporting::Report;
    use crate::reporting::ReportFacts;
    use crate::reporting::SubtreeReexportFixFact;
    use crate::rust_syntax::ParsedSources;
    use crate::selection::Selection;
    use crate::selection::SelectionScope;

    const REEXPORT: &str = "pub(super) use crate::rig::BOOT_BASE_RADIUS_SCALE;\n";

    fn fixture(root: &Path, caller: &str) -> Result<Selection> {
        for directory in ["src/rig", "src/rig_plugin"] {
            fs::create_dir_all(root.join(directory))?;
        }
        for (file, source) in [
            (
                "Cargo.toml",
                "[package]\nname = \"glob_caller\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
            ),
            (
                "src/lib.rs",
                "mod rig; mod rig_plugin; pub fn radius() -> f32 { rig_plugin::radius() }",
            ),
            (
                "src/rig/mod.rs",
                "mod constants; pub(crate) use constants::BOOT_BASE_RADIUS_SCALE;",
            ),
            (
                "src/rig/constants.rs",
                "pub(crate) const BOOT_BASE_RADIUS_SCALE: f32 = 2.0;",
            ),
            (
                "src/rig_plugin/mod.rs",
                "mod constants; mod look; mod idle; mod other { pub(super) const OTHER: f32 = 1.0; } pub(crate) fn radius() -> f32 { look::radius() }",
            ),
            ("src/rig_plugin/look.rs", caller),
            ("src/rig_plugin/idle.rs", ""),
        ] {
            fs::write(root.join(file), source)?;
        }
        fs::write(
            root.join("src/rig_plugin/constants.rs"),
            format!("pub(super) const LOCAL_SCALE: f32 = 1.0;\n{REEXPORT}"),
        )?;
        Ok(Selection {
            manifest_path:    root.join("Cargo.toml"),
            manifest_dir:     root.to_path_buf(),
            workspace_root:   root.to_path_buf(),
            target_directory: root.join("target"),
            analysis_root:    root.to_path_buf(),
            scope:            SelectionScope::SinglePackage,
            package_roots:    vec![root.to_path_buf()],
            packages:         Vec::new(),
        })
    }

    fn report() -> Result<Report> {
        Ok(Report {
            findings: vec![serde_json::from_value(serde_json::json!({
                "severity": "error", "fixability": "fix_pub_use_outside_subtree",
                "diagnostic_code": "pub_use_outside_subtree",
                "path": "src/rig_plugin/constants.rs", "line": 2, "column": 1,
                "highlight_len": REEXPORT.trim().len(), "source_line": REEXPORT.trim(),
                "message": "The re-export reaches outside its subtree."
            }))?],
            facts: ReportFacts {
                subtree_reexport_fix_facts: vec![SubtreeReexportFixFact {
                    use_path:          "src/rig_plugin/constants.rs".to_string(),
                    use_line:          2,
                    use_column:        1,
                    exported_name:     "BOOT_BASE_RADIUS_SCALE".to_string(),
                    written_path:      "crate::rig::BOOT_BASE_RADIUS_SCALE".to_string(),
                    owner_module:      vec!["rig_plugin".to_string(), "constants".to_string()],
                    owner_module_form: ModuleForm::File,
                    source_module:     vec!["rig".to_string()],
                    target_scope:      Vec::new(),
                    common_ancestor:   Vec::new(),
                    crate_root_file:   "src/lib.rs".to_string(),
                    ancestor_reexport: AncestorReexport::NotRequired,
                }]
                .into(),
                ..ReportFacts::default()
            },
            ..Report::default()
        })
    }

    fn fixes(selection: &Selection, report: &Report) -> Result<ValidatedFixSet> {
        let sources = ParsedSources::default();
        let plan = |scan: &SubtreeReexportScan| {
            imports::plan_module_imports(
                scan.fixes.tagged().cloned().collect(),
                &sources,
                &selection.package_roots,
            )
        };
        let scanned = scan::scan_selection(selection, report, &sources, |scan| {
            Ok(plan(scan)?.declined_files)
        })?;
        ValidatedFixSet::try_from(plan(&scanned)?.fixes)
    }

    fn assert_compiles(root: &Path) -> Result<()> {
        let output = Command::new("rustc")
            .args([
                "--edition=2024",
                "--crate-type=lib",
                "--emit=metadata",
                "-Dunused-imports",
            ])
            .arg(root.join("src/lib.rs"))
            .arg("-o")
            .arg(root.join("fixture.rmeta"))
            .output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(())
    }

    #[test]
    fn glob_callers_keep_removed_reexport_bindings_and_compile() -> Result<()> {
        let body = "pub(super) fn radius() -> f32 { LOCAL_SCALE * BOOT_BASE_RADIUS_SCALE }\n";
        let direct = format!("use super::constants::*;\n{body}");
        let direct_fixed =
            format!("use super::constants::*;\nuse crate::rig::BOOT_BASE_RADIUS_SCALE;\n{body}");
        let gated = "pub(super) fn radius() -> f32 { #[cfg(all())] use super::constants::*; LOCAL_SCALE * BOOT_BASE_RADIUS_SCALE }\n";
        let gated_fixed = "pub(super) fn radius() -> f32 { #[cfg(all())] use super::constants::*;\n#[cfg(all())] use crate::rig::BOOT_BASE_RADIUS_SCALE; LOCAL_SCALE * BOOT_BASE_RADIUS_SCALE }\n";
        let competing = "use super::constants::*;\nuse super::other::*;\npub(super) fn radius() -> f32 { LOCAL_SCALE * BOOT_BASE_RADIUS_SCALE * OTHER }\n";
        let opaque = "use super::constants::*;\nmacro_rules! radius { () => { LOCAL_SCALE * BOOT_BASE_RADIUS_SCALE }; }\npub(super) fn radius() -> f32 { radius!() }\n";
        let shadowed = "pub(super) fn radius() -> f32 { scale(3.0) + super::constants::BOOT_BASE_RADIUS_SCALE }\nfn scale(BOOT_BASE_RADIUS_SCALE: f32) -> f32 { use super::constants::*; LOCAL_SCALE * BOOT_BASE_RADIUS_SCALE }\n";
        let external = "use std::collections::*; fn idle() { let map = HashMap::<u8, u8>::new(); assert!(map.is_empty()); }";
        let absent = "use super::constants::*; fn idle() -> f32 { LOCAL_SCALE }";
        let child_caller = "pub(super) fn radius() -> f32 { super::constants::child::radius() }";
        let descendant = format!(
            "pub(super) const LOCAL_SCALE: f32 = 1.0;\n{REEXPORT}pub(crate) mod child {{ pub(crate) fn radius() -> f32 {{ use super::*; LOCAL_SCALE * BOOT_BASE_RADIUS_SCALE }} }}"
        );
        let block_alias = "use super::other as source; pub(super) fn radius() -> f32 { let scale = source::OTHER; { use super::constants as source; use source::*; scale * LOCAL_SCALE * BOOT_BASE_RADIUS_SCALE } }";
        let none: &[(&str, &str)] = &[];
        for (caller, expected, retained, extra) in [
            (direct.as_str(), direct_fixed.as_str(), false, none),
            (gated, gated_fixed, false, none),
            (competing, competing, true, none),
            (opaque, opaque, true, none),
            (shadowed, shadowed, true, none),
            (
                direct.as_str(),
                direct_fixed.as_str(),
                false,
                &[("src/rig_plugin/idle.rs", external)],
            ),
            (
                direct.as_str(),
                direct_fixed.as_str(),
                false,
                &[("src/rig_plugin/idle.rs", absent)],
            ),
            (
                child_caller,
                child_caller,
                true,
                &[("src/rig_plugin/constants.rs", descendant.as_str())],
            ),
            (block_alias, block_alias, true, none),
        ] {
            let dir = tempfile::tempdir()?;
            let selection = fixture(dir.path(), caller)?;
            for (file, source) in extra {
                fs::write(dir.path().join(file), source)?;
            }
            let report = report()?;
            imports::apply_fixes(&fixes(&selection, &report)?)?;
            let caller_path = dir.path().join("src/rig_plugin/look.rs");
            assert_eq!(fs::read_to_string(&caller_path)?, expected, "{caller}");
            let constants = fs::read_to_string(dir.path().join("src/rig_plugin/constants.rs"))?;
            assert_eq!(constants.contains(REEXPORT), retained, "{caller}");
            assert_compiles(dir.path())?;
            assert!(
                fixes(&selection, &report)?.iter().next().is_none(),
                "The second pass must propose no edit."
            );
            assert_eq!(fs::read_to_string(caller_path)?, expected);
        }
        Ok(())
    }
}
