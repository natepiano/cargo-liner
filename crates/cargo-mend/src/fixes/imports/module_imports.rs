use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::io::ErrorKind;
use std::ops::Range;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use proc_macro2::LineColumn;
use proc_macro2::Span;
use quote::ToTokens;
use syn::Block;
use syn::File;
use syn::Item;
use syn::ItemMod;
use syn::ItemUse;
use syn::UseTree;
use syn::Visibility;
use syn::spanned::Spanned;
use syn::visit::Visit;
use walkdir::WalkDir;

use super::ConditionalAttributes;
use super::ImportSource;
use super::TaggedFix;
use super::UseBinding;
use super::UseFix;
use super::collect_use_bindings;
use super::import_condition::ImportCondition;
use super::import_scan;
use crate::compiler::SOURCE_DIR_SRC;
use crate::reporting::FixKind;
use crate::rust_syntax::FileModulePath;
use crate::rust_syntax::ParsedSources;

/// A resolved target and the identifier it binds in one lexical scope.
#[derive(Clone, Eq, Ord, PartialEq, PartialOrd)]
struct ImportedName {
    name:   String,
    target: String,
}

impl ImportedName {
    fn resolve(binding: &UseBinding, absolute: bool, scope: &ScopeImports) -> Self {
        let module = &scope.module;
        let path = binding.path();
        let segments = path.split("::").collect::<Vec<_>>();
        let target = if absolute {
            format!("::{path}")
        } else {
            match segments.first().copied() {
                Some("crate") => format!("crate::{}", segments[1..].join("::")),
                Some("self") => {
                    let mut target = module.clone();
                    target.extend(segments[1..].iter().map(|part| (*part).to_string()));
                    format!("crate::{}", target.join("::"))
                },
                Some("super") => {
                    let parents = segments.iter().take_while(|part| **part == "super").count();
                    module.len().checked_sub(parents).map_or_else(
                        || format!("unresolved::{path}"),
                        |retained| {
                            let mut target = module[..retained].to_vec();
                            target
                                .extend(segments[parents..].iter().map(|part| (*part).to_string()));
                            format!("crate::{}", target.join("::"))
                        },
                    )
                },
                Some(first) if matches!(scope.items.get(first), Some(ItemBinding::Module)) => {
                    let mut target = module.clone();
                    target.extend(segments.iter().map(|part| (*part).to_string()));
                    format!("crate::{}", target.join("::"))
                },
                _ => format!("relative::{}::{path}", module.join("::")),
            }
        };
        Self {
            name: binding.name().unwrap_or("*").to_string(),
            target,
        }
    }
}

struct ImportClaim {
    candidate: usize,
    leaf:      usize,
    condition: ImportCondition,
}

enum ItemBinding {
    Module,
    Other,
}

struct ScopeImports {
    range:  Range<usize>,
    module: Vec<String>,
    names:  BTreeMap<ImportedName, Vec<ImportClaim>>,
    items:  BTreeMap<String, ItemBinding>,
}

/// One owner for retained imports, scan-time reservations and final proposals.
/// Scanners ask this owner whether a rewrite is safe; they never maintain a
/// second set of imports or decide whether two written paths are duplicates.
pub(in crate::fixes) struct ModuleImports {
    scopes:     Vec<ScopeImports>,
    candidates: Vec<ImportCandidate>,
}

enum ImportOrigin {
    Retained(Range<usize>),
    Proposal { edit: usize, range: Range<usize> },
    Reservation,
}

enum ImportLeaf {
    Original,
    Under(ImportCondition),
    Removed,
    ConflictingVisibility,
}

enum ModuleEdits {
    Ready(Vec<TaggedFix>),
    Deferred,
}

struct ImportCandidate {
    scope:  usize,
    item:   ItemUse,
    gates:  ImportCondition,
    origin: ImportOrigin,
    leaves: Vec<ImportLeaf>,
}

struct SelectedImport {
    item:  ItemUse,
    edit:  usize,
    range: Range<usize>,
}

/// A file must have one module path across all Cargo target mounts before
/// relative import edits can be planned for it.
pub(in crate::fixes) enum ModuleContext {
    Known(Vec<String>),
    Unresolved,
}

/// Whether a module proposal needs an import in addition to retained or planned bindings.
pub(in crate::fixes) enum ImportReservation {
    Existing,
    Planned,
    Added,
}

/// A rewrite releases its original binding; an addition releases none.
pub(in crate::fixes) enum ImportProposal {
    Addition,
    RewriteAt(LineColumn),
}

/// Hoisting retains the scanner's conservative collision behavior: a gated
/// binding already at module scope can prevent a move from its nested scope.
pub(in crate::fixes) enum ImportMove {
    Blocked,
    Delete,
    Insert,
}

impl ModuleImports {
    pub(in crate::fixes) fn from_syntax(
        syntax: &File,
        source: &ImportSource<'_>,
        module: &[String],
    ) -> Result<Self> {
        Self::collect(syntax, source, module, &[])
    }

    /// Include Cargo-declared files even when a target root or `#[path]`
    /// declaration lies outside the conventional source directory.
    pub(in crate::fixes) fn source_files(
        sources: &ParsedSources,
        package_root: &Path,
    ) -> BTreeSet<PathBuf> {
        let source_root = package_root.join(SOURCE_DIR_SRC);
        let module_map = sources.module_map(&source_root);
        let mut paths = module_map
            .declared_files()
            .map(|(path, _)| path.to_path_buf())
            .collect::<BTreeSet<_>>();
        paths.extend(
            WalkDir::new(&source_root)
                .into_iter()
                .filter_map(Result::ok)
                .filter(|entry| {
                    entry.file_type().is_file()
                        && entry.path().extension().and_then(OsStr::to_str) == Some("rs")
                })
                .map(walkdir::DirEntry::into_path),
        );
        paths
    }

    pub(in crate::fixes) fn file_module(
        sources: &ParsedSources,
        package_roots: &[PathBuf],
        path: &Path,
    ) -> ModuleContext {
        let mut context = ModuleContext::Unresolved;
        for package_root in package_roots {
            let source_root = package_root.join(SOURCE_DIR_SRC);
            let module_map = sources.module_map(&source_root);
            match module_map.file_module_path(&source_root, path) {
                Some(FileModulePath::Known(module)) => match &context {
                    ModuleContext::Known(previous) if previous != &module => {
                        return ModuleContext::Unresolved;
                    },
                    ModuleContext::Known(_) => {},
                    ModuleContext::Unresolved => context = ModuleContext::Known(module),
                },
                Some(FileModulePath::SeveralParents) => return ModuleContext::Unresolved,
                None => {},
            }
        }
        context
    }

    fn collect(
        syntax: &File,
        source: &ImportSource<'_>,
        module: &[String],
        edits: &[&TaggedFix],
    ) -> Result<Self> {
        let mut imports = Self {
            scopes:     vec![ScopeImports {
                range:  0..usize::MAX,
                module: module.to_vec(),
                names:  BTreeMap::new(),
                items:  BTreeMap::new(),
            }],
            candidates: Vec::new(),
        };
        let mut collector = RetainedImports {
            source,
            edits,
            imports: &mut imports,
            scope: 0,
            result: Ok(()),
        };
        collector.visit_file(syntax);
        collector.result?;
        imports.reserve_candidates();
        Ok(imports)
    }

    fn scope_at(&self, offset: usize) -> usize {
        self.scopes
            .iter()
            .rposition(|scope| scope.range.contains(&offset))
            .unwrap_or(0)
    }

    pub(in crate::fixes) fn collides(
        &self,
        offset: usize,
        name: &str,
        absolute_module: &[String],
        proposal: &ImportProposal,
    ) -> bool {
        let scope = &self.scopes[self.scope_at(offset)];
        let target = format!("crate::{}", absolute_module.join("::"));
        scope.names.iter().any(|(binding, claims)| {
            binding.name == name
                && binding.target != target
                && claims.iter().any(|claim| match proposal {
                    ImportProposal::Addition => true,
                    ImportProposal::RewriteAt(start) => {
                        let candidate = &self.candidates[claim.candidate];
                        !matches!(candidate.origin, ImportOrigin::Retained(_))
                            || candidate.item.span().start() != *start
                    },
                })
        })
    }

    pub(in crate::fixes) fn reserve_module(
        &mut self,
        offset: usize,
        path: &str,
        gates: &ConditionalAttributes,
    ) -> ImportReservation {
        let item: ItemUse = match syn::parse_str(&format!("{}use {path};", gates.render(""))) {
            Ok(item) => item,
            Err(_) => return ImportReservation::Existing,
        };
        let condition = ImportCondition::from_attributes(&item.attrs);
        let scope = self.scope_at(offset);
        let bindings = collect_use_bindings(&item.tree);
        if bindings
            .iter()
            .all(|binding| self.covered(scope, binding, item.leading_colon.is_some(), &condition))
        {
            let existing = bindings.iter().any(|binding| {
                let key = ImportedName::resolve(
                    binding,
                    item.leading_colon.is_some(),
                    &self.scopes[scope],
                );
                self.scopes[scope].names.get(&key).is_some_and(|claims| {
                    claims.iter().any(|claim| {
                        matches!(
                            self.candidates[claim.candidate].origin,
                            ImportOrigin::Retained(_)
                        )
                    })
                })
            });
            return if existing {
                ImportReservation::Existing
            } else {
                ImportReservation::Planned
            };
        }
        self.add_candidate(scope, item, ImportOrigin::Reservation);
        self.reserve_candidates();
        ImportReservation::Added
    }

    pub(in crate::fixes) fn offset_for_scope(&self, inline_scope: &[String]) -> usize {
        let mut module = self.scopes[0].module.clone();
        module.extend_from_slice(inline_scope);
        self.scopes
            .iter()
            .find(|scope| scope.module == module)
            .map_or(0, |scope| scope.range.start)
    }

    #[cfg(test)]
    pub(in crate::fixes) fn has_reservations(&self) -> bool {
        self.candidates
            .iter()
            .any(|candidate| matches!(candidate.origin, ImportOrigin::Reservation))
    }

    fn covered(
        &self,
        scope: usize,
        binding: &UseBinding,
        absolute: bool,
        condition: &ImportCondition,
    ) -> bool {
        let scope = &self.scopes[scope];
        let key = ImportedName::resolve(binding, absolute, scope);
        scope.names.get(&key).is_some_and(|claims| {
            let coverage = claims
                .iter()
                .fold(ImportCondition::Never, |coverage, claim| {
                    coverage.union(&claim.condition)
                });
            condition.implies(&coverage)
        })
    }

    pub(in crate::fixes) fn move_import(
        &mut self,
        offset: usize,
        item: &ItemUse,
        gates: &ConditionalAttributes,
    ) -> ImportMove {
        let scope_index = self.scope_at(offset);
        // The carried gate includes conditions on enclosing constructs.
        let Ok(parsed) =
            syn::parse_str::<ItemUse>(&format!("{}{}", gates.render(""), item.to_token_stream()))
        else {
            return ImportMove::Blocked;
        };
        let condition = ImportCondition::from_attributes(&parsed.attrs);
        // A glob can supply named bindings and unnamed traits that we cannot resolve here.
        if self.candidates.iter().any(|candidate| {
            candidate.scope == scope_index
                && candidate.gates.overlaps(&condition)
                && collect_use_bindings(&candidate.item.tree)
                    .iter()
                    .any(|binding| matches!(binding, UseBinding::Glob { .. }))
        }) {
            return ImportMove::Blocked;
        }
        let scope = &self.scopes[scope_index];
        let conditional = !gates.is_empty();
        let bindings = collect_use_bindings(&item.tree);
        let mut duplicate = true;
        for binding in &bindings {
            let key = ImportedName::resolve(binding, item.leading_colon.is_some(), scope);
            if scope.items.contains_key(&key.name) {
                return ImportMove::Blocked;
            }
            let mut same = false;
            for (existing, claims) in &scope.names {
                if existing.name != key.name {
                    continue;
                }
                if conditional
                    || claims.iter().any(|claim| !claim.condition.is_always())
                    || existing.target != key.target
                {
                    return ImportMove::Blocked;
                }
                same = true;
            }
            duplicate &= same;
        }
        if duplicate {
            return ImportMove::Delete;
        }
        let mut moved = item.clone();
        moved.attrs = parsed.attrs;
        self.add_candidate(scope_index, moved, ImportOrigin::Reservation);
        self.reserve_candidates();
        ImportMove::Insert
    }

    fn add_candidate(&mut self, scope: usize, item: ItemUse, origin: ImportOrigin) {
        let leaves = collect_use_bindings(&item.tree)
            .into_iter()
            .map(|_| ImportLeaf::Original)
            .collect();
        self.candidates.push(ImportCandidate {
            scope,
            gates: ImportCondition::from_attributes(&item.attrs),
            item,
            origin,
            leaves,
        });
    }

    fn reserve_candidates(&mut self) {
        for scope in &mut self.scopes {
            scope.names.clear();
        }
        for candidate in &mut self.candidates {
            for leaf in &mut candidate.leaves {
                *leaf = ImportLeaf::Original;
            }
        }
        // Preserve visibility requirements before merging private spellings.
        // A private import may cover only configurations left by a re-export.
        // Within one visibility, broader conditions still precede gated ones.
        let mut order = (0..self.candidates.len()).collect::<Vec<_>>();
        order.sort_by_key(|index| {
            let candidate = &self.candidates[*index];
            (
                matches!(candidate.item.vis, Visibility::Inherited),
                !candidate.gates.is_always(),
            )
        });
        for index in order {
            let candidate = &self.candidates[index];
            let scope_index = candidate.scope;
            let absolute = candidate.item.leading_colon.is_some();
            let bindings = collect_use_bindings(&candidate.item.tree);
            for (leaf, binding) in bindings.into_iter().enumerate() {
                if matches!(binding.name(), None | Some("_")) {
                    continue;
                }
                let scope = &mut self.scopes[scope_index];
                let key = ImportedName::resolve(&binding, absolute, scope);
                let claims = scope.names.entry(key).or_default();
                Self::reserve_binding(&mut self.candidates, claims, index, leaf);
            }
        }
    }

    /// Keep each visibility requirement in the configurations it can supply.
    fn reserve_binding(
        candidates: &mut [ImportCandidate],
        claims: &mut Vec<ImportClaim>,
        index: usize,
        leaf: usize,
    ) {
        let mut condition = candidates[index].gates.clone();
        let visibility = candidates[index].item.vis.to_token_stream().to_string();
        for claim in claims.iter() {
            let existing = &candidates[claim.candidate];
            if existing.item.vis.to_token_stream().to_string() != visibility {
                if !matches!(candidates[index].item.vis, Visibility::Inherited)
                    && condition.overlaps(&claim.condition)
                {
                    candidates[index].leaves[leaf] = ImportLeaf::ConflictingVisibility;
                    break;
                }
                condition = condition.excluding(&claim.condition);
            }
        }
        if matches!(
            candidates[index].leaves[leaf],
            ImportLeaf::ConflictingVisibility
        ) {
            return;
        }
        if !condition.overlaps(&ImportCondition::Always) {
            candidates[index].leaves[leaf] = ImportLeaf::Removed;
            return;
        }
        if !condition.equivalent(&candidates[index].gates) {
            candidates[index].leaves[leaf] = ImportLeaf::Under(condition.clone());
        }
        Self::merge_overlapping_claims(candidates, claims, index, leaf, condition, &visibility);
    }

    /// Coalesce claims with the same visibility, preserving the first spelling.
    fn merge_overlapping_claims(
        candidates: &mut [ImportCandidate],
        claims: &mut Vec<ImportClaim>,
        index: usize,
        leaf: usize,
        mut condition: ImportCondition,
        visibility: &str,
    ) {
        let mut overlapping = Vec::new();
        // Unioning can connect conditions that were previously disjoint.
        loop {
            let mut changed = false;
            for (claim_index, claim) in claims.iter().enumerate() {
                if !overlapping.contains(&claim_index)
                    && candidates[claim.candidate]
                        .item
                        .vis
                        .to_token_stream()
                        .to_string()
                        == visibility
                    && condition.overlaps(&claim.condition)
                {
                    condition = condition.union(&claim.condition);
                    overlapping.push(claim_index);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        if overlapping.is_empty() {
            claims.push(ImportClaim {
                candidate: index,
                leaf,
                condition,
            });
            return;
        }
        let winner = overlapping[0];
        let winner_candidate = claims[winner].candidate;
        let winner_leaf = claims[winner].leaf;
        candidates[index].leaves[leaf] = ImportLeaf::Removed;
        for other in overlapping.iter().copied().skip(1) {
            candidates[claims[other].candidate].leaves[claims[other].leaf] = ImportLeaf::Removed;
        }
        if !condition.equivalent(&candidates[winner_candidate].gates) {
            candidates[winner_candidate].leaves[winner_leaf] = ImportLeaf::Under(condition.clone());
        }
        claims[winner].condition = condition;
        let mut claim_index = 0;
        claims.retain(|_| {
            let retain = claim_index == winner || !overlapping.contains(&claim_index);
            claim_index += 1;
            retain
        });
    }

    fn finish(mut self, edits: &mut [TaggedFix], path: &Path) -> ModuleEdits {
        self.reserve_candidates();
        if self.candidates.iter().any(|candidate| {
            candidate
                .leaves
                .iter()
                .any(|leaf| matches!(leaf, ImportLeaf::ConflictingVisibility))
        }) {
            return ModuleEdits::Deferred;
        }
        let mut changes: BTreeMap<usize, Vec<(Range<usize>, String)>> = BTreeMap::new();
        let mut retained_changes = Vec::new();
        for candidate in self.candidates {
            if candidate
                .leaves
                .iter()
                .all(|leaf| matches!(leaf, ImportLeaf::Original))
            {
                continue;
            }
            let replacement = render_candidate(&candidate);
            match candidate.origin {
                ImportOrigin::Retained(range) => retained_changes.push(TaggedFix {
                    fix_kind: Some(FixKind::Import),
                    fix:      UseFix {
                        path: path.to_path_buf(),
                        start: range.start,
                        end: range.end,
                        replacement,
                        import_group: None,
                    },
                }),
                ImportOrigin::Proposal { edit, range } => {
                    changes.entry(edit).or_default().push((range, replacement));
                },
                ImportOrigin::Reservation => {},
            }
        }
        for (index, mut replacements) in changes {
            replacements.sort_by_key(|(range, _)| Reverse(range.start));
            for (range, replacement) in replacements {
                edits[index]
                    .fix
                    .replacement
                    .replace_range(range, &replacement);
            }
        }
        // Retained imports are changed through the same typed proposals, never
        // by scanning the edited file for duplicate text.
        ModuleEdits::Ready(retained_changes)
    }
}

fn render_candidate(candidate: &ImportCandidate) -> String {
    let mut rendered = String::new();
    let original = candidate
        .leaves
        .iter()
        .map(|leaf| matches!(leaf, ImportLeaf::Original))
        .collect::<Vec<_>>();
    if original.iter().any(|keep| *keep) {
        let mut item = candidate.item.clone();
        prune_tree(&mut item.tree, &mut original.into_iter());
        rendered.push_str(&item.to_token_stream().to_string());
    }
    for (index, leaf) in candidate.leaves.iter().enumerate() {
        let ImportLeaf::Under(condition) = leaf else {
            continue;
        };
        let mut item = candidate.item.clone();
        prune_tree(
            &mut item.tree,
            &mut (0..candidate.leaves.len()).map(|leaf| leaf == index),
        );
        item.attrs
            .retain(|attribute| !super::is_conditional(attribute));
        if !rendered.is_empty() {
            rendered.push('\n');
        }
        rendered.push_str(&condition.render());
        rendered.push_str(&item.to_token_stream().to_string());
    }
    rendered
}

/// Import-safe proposals and the files whose proposals require their owners
/// to rebuild dependent rewrites before any edits can be committed.
pub(in crate::fixes) struct ImportPlan {
    pub(in crate::fixes) fixes:          Vec<TaggedFix>,
    pub(in crate::fixes) declined_files: BTreeSet<PathBuf>,
}

/// Combine only selected fix groups, before range validation and writing.
pub(in crate::fixes) fn plan_module_imports(
    fixes: Vec<TaggedFix>,
    sources: &ParsedSources,
    package_roots: &[PathBuf],
) -> Result<ImportPlan> {
    let mut fixes = import_scan::canonical_fixes(fixes);
    let mut by_file = BTreeMap::new();
    for (index, tagged) in fixes.iter().enumerate() {
        by_file
            .entry(tagged.fix.path.clone())
            .or_insert_with(Vec::new)
            .push(index);
    }
    let mut retained_changes = Vec::new();
    let mut deferred_files = BTreeSet::new();
    for (path, indices) in by_file {
        let proposals = selected_imports(&fixes, &indices)?;
        if proposals.is_empty() {
            // Reference-only edits introduce no bindings. In particular,
            // include! sources need no module context to keep caller edits
            // selected together with a re-export's relocation.
            continue;
        }
        let parsed = match sources.source(&path) {
            Ok(parsed) => parsed,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(error).with_context(|| format!("read imports in {}", path.display()));
            },
        };
        let text = parsed.text();
        let syntax = parsed.syntax().context("parse import edit source")?;
        let source = ImportSource::new(text);
        let ModuleContext::Known(module) =
            ModuleImports::file_module(sources, package_roots, &path)
        else {
            // Keep reference edits with their imports when no common module
            // context is valid for every mount of this source file.
            deferred_files.insert(path);
            continue;
        };
        let edits = indices
            .iter()
            .map(|index| &fixes[*index])
            .collect::<Vec<_>>();
        let mut imports = ModuleImports::collect(syntax, &source, &module, &edits)?;
        for proposal in proposals {
            let scope = imports.scope_at(fixes[proposal.edit].fix.start);
            imports.add_candidate(
                scope,
                proposal.item,
                ImportOrigin::Proposal {
                    edit:  proposal.edit,
                    range: proposal.range,
                },
            );
        }
        match imports.finish(&mut fixes, &path) {
            ModuleEdits::Ready(changes) => retained_changes.extend(changes),
            ModuleEdits::Deferred => {
                deferred_files.insert(path);
            },
        }
    }
    fixes.extend(retained_changes);
    fixes.retain(|tagged| {
        !deferred_files.contains(&tagged.fix.path)
            && (tagged.fix.start != tagged.fix.end || !tagged.fix.replacement.trim().is_empty())
    });
    Ok(ImportPlan {
        fixes,
        declined_files: deferred_files,
    })
}

fn selected_imports(fixes: &[TaggedFix], indices: &[usize]) -> Result<Vec<SelectedImport>> {
    let mut imports = Vec::new();
    for &edit in indices {
        let replacement = &fixes[edit].fix.replacement;
        let Ok(proposal) = syn::parse_file(replacement) else {
            continue;
        };
        let source = ImportSource::new(replacement);
        for item in proposal.items {
            if let Item::Use(item) = item {
                imports.push(SelectedImport {
                    range: span_range(&source, item.span())?,
                    item,
                    edit,
                });
            }
        }
    }
    Ok(imports)
}

struct RetainedImports<'a, 'source> {
    source:  &'a ImportSource<'source>,
    edits:   &'a [&'a TaggedFix],
    imports: &'a mut ModuleImports,
    scope:   usize,
    result:  Result<()>,
}

impl RetainedImports<'_, '_> {
    fn in_scope(&mut self, span: Span, module: Vec<String>, visit: impl FnOnce(&mut Self)) {
        match span_range(self.source, span) {
            Ok(range) => {
                let parent = self.scope;
                self.scope = self.imports.scopes.len();
                self.imports.scopes.push(ScopeImports {
                    range,
                    module,
                    names: BTreeMap::new(),
                    items: BTreeMap::new(),
                });
                visit(self);
                self.scope = parent;
            },
            Err(error) => self.result = Err(error),
        }
    }
}

impl<'ast> Visit<'ast> for RetainedImports<'_, '_> {
    fn visit_item(&mut self, item: &'ast Item) {
        let name = match item {
            Item::Struct(item) => Some(item.ident.to_string()),
            Item::Enum(item) => Some(item.ident.to_string()),
            Item::Union(item) => Some(item.ident.to_string()),
            Item::Fn(item) => Some(item.sig.ident.to_string()),
            Item::Const(item) => Some(item.ident.to_string()),
            Item::Static(item) => Some(item.ident.to_string()),
            Item::Mod(item) => Some(item.ident.to_string()),
            Item::Trait(item) => Some(item.ident.to_string()),
            Item::TraitAlias(item) => Some(item.ident.to_string()),
            Item::Type(item) => Some(item.ident.to_string()),
            Item::ExternCrate(item) => Some(
                item.rename
                    .as_ref()
                    .map_or_else(|| item.ident.to_string(), |(_, name)| name.to_string()),
            ),
            _ => None,
        };
        if let Some(name) = name {
            self.imports.scopes[self.scope].items.insert(
                name,
                if matches!(item, Item::Mod(_)) {
                    ItemBinding::Module
                } else {
                    ItemBinding::Other
                },
            );
        }
        syn::visit::visit_item(self, item);
    }

    fn visit_item_use(&mut self, item: &'ast ItemUse) {
        let tree = match span_range(self.source, item.tree.span()) {
            Ok(range) => range,
            Err(error) => {
                self.result = Err(error);
                return;
            },
        };
        if self
            .edits
            .iter()
            .any(|edit| edit.fix.start < tree.end && tree.start < edit.fix.end)
        {
            return;
        }
        match span_range(self.source, item.span()) {
            Ok(range) => {
                self.imports
                    .add_candidate(self.scope, item.clone(), ImportOrigin::Retained(range));
            },
            Err(error) => self.result = Err(error),
        }
    }

    fn visit_item_mod(&mut self, item: &'ast ItemMod) {
        if let Some((brace, _)) = &item.content {
            let mut module = self.imports.scopes[self.scope].module.clone();
            module.push(item.ident.to_string());
            self.in_scope(brace.span.join(), module, |collector| {
                syn::visit::visit_item_mod(collector, item);
            });
        }
    }

    fn visit_block(&mut self, block: &'ast Block) {
        let module = self.imports.scopes[self.scope].module.clone();
        self.in_scope(block.brace_token.span.join(), module, |collector| {
            syn::visit::visit_block(collector, block);
        });
    }
}

fn span_range(source: &ImportSource<'_>, span: Span) -> Result<Range<usize>> {
    let location = source
        .locate(span.start(), span.end())
        .context("import span outside source")?;
    Ok(location.start()..location.end())
}

/// The binding collector supplies one decision per leaf, including globs and
/// renamed self imports. Keeping its traversal order avoids another definition
/// of what name a use tree binds.
fn prune_tree(tree: &mut UseTree, retained: &mut impl Iterator<Item = bool>) -> bool {
    match tree {
        UseTree::Path(path) => prune_tree(&mut path.tree, retained),
        UseTree::Group(group) => {
            let mut items = syn::punctuated::Punctuated::new();
            for mut pair in std::mem::take(&mut group.items).into_pairs() {
                if prune_tree(pair.value_mut(), retained) {
                    items.push_value(pair.value().clone());
                    if let Some(punctuation) = pair.punct() {
                        items.push_punct(*punctuation);
                    }
                }
            }
            group.items = items;
            !group.items.is_empty()
        },
        UseTree::Name(_) | UseTree::Rename(_) | UseTree::Glob(_) => retained.next().unwrap_or(true),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::ops::Range;
    use std::path::Path;

    use anyhow::Result;
    use syn::Item;
    use syn::ItemUse;
    use tempfile::tempdir;

    use super::plan_module_imports;
    use crate::fixes::imports;
    use crate::fixes::imports::TaggedFix;
    use crate::fixes::imports::UseFix;
    use crate::fixes::imports::ValidatedFixSet;
    use crate::rust_syntax::ParsedSources;
    use crate::selection::CARGO_TARGET_KIND_LIB;
    use crate::selection::PackageMetadata;
    use crate::selection::Selection;
    use crate::selection::SelectionScope;
    use crate::selection::TargetMetadata;
    use crate::selection::TargetSupport;

    fn fixture_selection(root: &Path, crate_root: &Path) -> Selection {
        Selection {
            manifest_path:    root.join("Cargo.toml"),
            manifest_dir:     root.to_path_buf(),
            workspace_root:   root.to_path_buf(),
            target_directory: root.join("target"),
            analysis_root:    root.to_path_buf(),
            scope:            SelectionScope::SinglePackage,
            package_roots:    vec![root.to_path_buf()],
            packages:         vec![PackageMetadata {
                id:            "import_fixture".to_string(),
                manifest_path: root.join("Cargo.toml"),
                root:          root.to_path_buf(),
                targets:       vec![TargetMetadata {
                    kind:              vec![CARGO_TARGET_KIND_LIB.to_string()],
                    crate_types:       vec![CARGO_TARGET_KIND_LIB.to_string()],
                    name:              "import_fixture".to_string(),
                    src_path:          crate_root.to_path_buf(),
                    edition:           "2024".to_string(),
                    required_features: Vec::new(),
                    doc:               TargetSupport::Enabled,
                    doctest:           TargetSupport::Enabled,
                    test:              TargetSupport::Enabled,
                }],
            }],
        }
    }

    fn apply(source: &str, edits: &[(Range<usize>, &str)]) -> Result<String> {
        let temp = tempdir()?;
        // Match Cargo's canonical package paths, including macOS temp symlinks.
        let root = fs::canonicalize(temp.path())?;
        let path = root.join("src/lib.rs");
        fs::create_dir(root.join("src"))?;

        fs::write(&path, source)?;
        let fixes = edits
            .iter()
            .map(|(range, replacement)| TaggedFix {
                fix_kind: None,
                fix:      UseFix {
                    path:         path.clone(),
                    start:        range.start,
                    end:          range.end,
                    replacement:  (*replacement).to_string(),
                    import_group: None,
                },
            })
            .collect();
        let selection = fixture_selection(&root, &path);
        let sources = ParsedSources::for_selection(&selection);
        let fixes = ValidatedFixSet::try_from(
            plan_module_imports(fixes, &sources, &selection.package_roots)?.fixes,
        )?;
        imports::apply_fixes(&fixes)?;
        Ok(fs::read_to_string(path)?)
    }

    #[test]
    fn module_imports_report_unresolved_proposals_but_keep_reference_only_edits() -> Result<()> {
        let temp = tempdir()?;
        let root = fs::canonicalize(temp.path())?;
        let imported = root.join("imported.rs");
        let referenced = root.join("referenced.rs");
        fs::write(&imported, "use crate::old::Item;\n")?;
        fs::write(&referenced, "fn run() { old::run(); }\n")?;
        let fixes = [
            (
                imported.clone(),
                0.."use crate::old::Item;".len(),
                "use crate::new::Item;",
            ),
            (referenced.clone(), 11..19, "new::run"),
        ]
        .into_iter()
        .map(|(path, range, replacement)| TaggedFix {
            fix_kind: None,
            fix:      UseFix {
                path,
                start: range.start,
                end: range.end,
                replacement: replacement.to_string(),
                import_group: None,
            },
        })
        .collect();
        // Both files lie outside a declared target: only the import requires
        // a module context. A reference-only edit must remain usable.
        let plan = plan_module_imports(fixes, &ParsedSources::default(), &[root])?;
        assert_eq!(plan.declined_files, std::iter::once(imported).collect());
        assert_eq!(plan.fixes.len(), 1);
        assert_eq!(plan.fixes[0].fix.path, referenced);
        assert_eq!(plan.fixes[0].fix.replacement, "new::run");
        Ok(())
    }

    #[test]
    fn module_imports_rewrite_releases_only_its_original_binding() -> Result<()> {
        let text = "use crate::reconcile::reconcile;\nuse crate::other as worker;\nmod tests { fn reconcile() {} }\n";
        let syntax = syn::parse_file(text)?;
        let source = super::ImportSource::new(text);
        let owner = super::ModuleImports::from_syntax(&syntax, &source, &[])?;
        let Item::Use(original) = &syntax.items[0] else {
            return Err(anyhow::anyhow!("fixture function import"));
        };
        let start = syn::spanned::Spanned::span(original).start();
        let module = ["reconcile".to_string()];
        assert!(!owner.collides(
            0,
            "reconcile",
            &module,
            &super::ImportProposal::RewriteAt(start),
        ));
        assert!(owner.collides(0, "reconcile", &module, &super::ImportProposal::Addition));
        assert!(owner.collides(
            0,
            "worker",
            &["worker".to_string()],
            &super::ImportProposal::RewriteAt(start),
        ));
        Ok(())
    }

    #[test]
    fn module_imports_relative_anchors_share_resolved_targets() -> Result<()> {
        let source = "mod outer { mod m {} mod consumer { use crate::outer::m::{self}; } }";
        let offset = source
            .find("use")
            .ok_or_else(|| anyhow::anyhow!("fixture import"))?;
        let fixed = apply(
            source,
            &[(offset..offset, "use super::m;\nuse crate::outer::m;\n")],
        )?;
        assert_eq!(fixed, source);
        Ok(())
    }

    #[test]
    fn module_imports_ungated_proposal_replaces_kept_test_binding() -> Result<()> {
        let source = "#[cfg(test)] use crate::m::{self};\nuse crate::m::one;\n";
        let offset = source
            .find("use crate::m::one")
            .ok_or_else(|| anyhow::anyhow!("fixture function import"))?;
        let fixed = apply(
            source,
            &[(offset..source.trim_end().len(), "use crate::m;")],
        )?;
        assert_eq!(fixed, "\nuse crate::m;\n");
        Ok(())
    }

    #[test]
    fn module_imports_preserve_public_gates_during_reservation_and_planning() -> Result<()> {
        let text = "#[cfg(test)] pub use crate::m::{self};\nuse crate::m::one;\n";
        let syntax = syn::parse_file(text)?;
        let source = super::ImportSource::new(text);
        let mut owner = super::ModuleImports::from_syntax(&syntax, &source, &[])?;
        assert!(matches!(
            owner.reserve_module(0, "crate::m", &super::ConditionalAttributes::default()),
            super::ImportReservation::Added,
        ));
        let module: ItemUse = syn::parse_str("use crate::m;")?;
        let bindings = super::collect_use_bindings(&module.tree);
        assert!(owner.covered(0, &bindings[0], false, &super::ImportCondition::Always));
        let offset = text
            .find("use crate::m::one")
            .ok_or_else(|| anyhow::anyhow!("function import"))?;
        let fixed = apply(text, &[(offset..text.trim_end().len(), "use crate::m;")])?;
        assert!(
            fixed.starts_with("#[cfg(test)] pub use crate::m::{self};\n"),
            "{fixed}"
        );
        let syntax = syn::parse_file(&fixed)?;
        let Item::Use(private) = &syntax.items[1] else {
            return Err(anyhow::anyhow!("private import missing: {fixed}"));
        };
        let expected: ItemUse = syn::parse_str("#[cfg(not(test))] use crate::m;")?;
        assert!(matches!(private.vis, syn::Visibility::Inherited));
        assert!(
            super::ImportCondition::from_attributes(&private.attrs)
                .equivalent(&super::ImportCondition::from_attributes(&expected.attrs))
        );
        Ok(())
    }

    #[test]
    fn module_imports_merge_overlapping_gates_without_widening_other_leaves() -> Result<()> {
        let source = "#[cfg(any(test, unix))] use crate::m::{self, Item};\n";
        let fixed = apply(
            source,
            &[(
                source.len()..source.len(),
                "#[cfg(any(test, windows))] use crate::m;\n",
            )],
        )?;
        let syntax = syn::parse_file(&fixed)?;
        let expected_module: ItemUse =
            syn::parse_str("#[cfg(any(test, unix, windows))] use crate::m;")?;
        let expected_item: ItemUse = syn::parse_str("#[cfg(any(test, unix))] use crate::m::Item;")?;
        let mut names = Vec::new();
        for item in syntax.items {
            let Item::Use(item) = item else {
                continue;
            };
            for binding in super::collect_use_bindings(&item.tree) {
                let expected = if binding.name() == Some("m") {
                    &expected_module
                } else {
                    &expected_item
                };
                assert!(
                    super::ImportCondition::from_attributes(&item.attrs)
                        .equivalent(&super::ImportCondition::from_attributes(&expected.attrs)),
                    "wrong binding configurations: {fixed}"
                );
                names.push(binding.name().unwrap_or("*").to_string());
            }
        }
        names.sort();
        assert_eq!(names, ["Item", "m"]);
        Ok(())
    }

    #[test]
    fn module_imports_merge_moves_and_rewrites_without_losing_references() -> Result<()> {
        let source = "use crate::m::one;\nfn run() { one(); }\n";
        let reference = source
            .rfind("one")
            .ok_or_else(|| anyhow::anyhow!("fixture reference"))?;
        let fixed = apply(
            source,
            &[
                (0..0, "use crate::m::{self};\n"),
                (0.."use crate::m::one;".len(), "use crate::m;"),
                (reference..reference + "one".len(), "m::one"),
            ],
        )?;
        assert_eq!(fixed, "use crate::m::{self};\n\nfn run() { m::one(); }\n");
        Ok(())
    }

    #[test]
    fn module_imports_prune_only_duplicate_leaves_and_recognize_renamed_self() -> Result<()> {
        let source = "use crate::m::{self as renamed, Item};\n";
        let fixed = apply(
            source,
            &[(
                source.len()..source.len(),
                "use crate::m as renamed;\nuse crate::m::{Item, Other};\n",
            )],
        )?;
        let syntax = syn::parse_file(&fixed)?;
        let bindings = syntax
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Use(item) => Some(super::collect_use_bindings(&item.tree)),
                _ => None,
            })
            .flatten()
            .map(|binding| binding.name().unwrap_or("*").to_string())
            .collect::<Vec<_>>();
        assert_eq!(bindings, ["renamed", "Item", "Other"]);
        assert!(
            fixed.starts_with(source),
            "retained imports keep their spelling: {fixed}"
        );
        Ok(())
    }

    #[test]
    fn module_imports_reserve_each_inline_module_and_block_separately() -> Result<()> {
        let source =
            "use crate::m;\nmod child { use crate::m; fn f() { use crate::m; } }\nmod other {}\n";
        let other = source
            .find("{}")
            .ok_or_else(|| anyhow::anyhow!("fixture module"))?
            + 1;
        let fixed = apply(source, &[(other..other, "use crate::m;")])?;
        assert_eq!(
            fixed,
            "use crate::m;\nmod child { use crate::m; fn f() { use crate::m; } }\nmod other {use crate::m;}\n"
        );
        Ok(())
    }

    #[test]
    fn module_imports_preserve_cfg_exports_globs_and_unnamed_imports() -> Result<()> {
        let source = "#[cfg(test)] use crate::m;\npub use crate::a::PublicItem;\n";
        let addition = "#[cfg(not(test))] use crate::m;\nuse crate::a::Item;\nuse crate::traits::{self as traits, Trait as _};\nuse crate::traits::*;\nuse crate::traits::Trait as _;\n";
        let fixed = apply(source, &[(source.len()..source.len(), addition)])?;
        assert_eq!(fixed, format!("{source}{addition}"));
        Ok(())
    }

    #[test]
    fn module_imports_deduplicate_identical_edits_before_reserving_names() -> Result<()> {
        let source = "use crate::m::one;\n";
        let edit = (0..source.trim_end().len(), "use crate::m;");
        let fixed = apply(source, &[edit.clone(), edit])?;
        assert_eq!(fixed, "use crate::m;\n");
        Ok(())
    }
}
