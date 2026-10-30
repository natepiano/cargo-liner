use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::iter;
use std::mem;
use std::path::Path;

use anyhow::Context;
use anyhow::Result;
use proc_macro2::Spacing;
use proc_macro2::TokenStream;
use proc_macro2::TokenTree;
use rustc_hash::FxHashMap;
use syn::Block;
use syn::File;
use syn::Ident;
use syn::Item;
use syn::ItemMacro;
use syn::ItemMod;
use syn::ItemUse;
use syn::Macro;
use syn::Stmt;
use syn::UseTree;
use syn::Visibility;
use syn::ext::IdentExt;
use syn::spanned::Spanned;
use syn::visit;
use syn::visit::Visit;

use super::module_aliases::ModuleAliases;
use super::module_path;
use super::redirect::FacadeRedirect;
use super::redirect::TargetSide;
use super::source_text;
use super::source_text::SourceLines;
use super::use_tree;
use crate::fixes::imports;
use crate::fixes::imports::UseBinding;
use crate::fixes::imports::UseFix;
use crate::rust_syntax;
use crate::rust_syntax::ParsedSources;
use crate::rust_syntax::PathAnchor;

/// Which builds compile a caller, ordered from fewest to most.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::fixes) enum CallerBuilds {
    /// A `#[cfg(test)]` module encloses the caller.
    TestOnly,
    /// Every build compiles the caller.
    Every,
}

/// The caller edits in one file, which redirects sent a caller to their
/// `outside` path, and which unchanged modules the file still names.
pub(in crate::fixes) struct FileRedirects {
    pub(in crate::fixes) fixes:               Vec<UseFix>,
    /// Indexes into the `redirects` slice the file was rewritten with, each
    /// with the widest builds among the callers that received its `outside`
    /// path.
    pub(in crate::fixes) outside_used:        BTreeMap<usize, CallerBuilds>,
    /// Redirects' `unchanged_module` paths a `use` in the file still names
    /// after the rewrite, so the module must stay.
    pub(in crate::fixes) named_owner_modules: BTreeSet<Vec<String>>,
}

/// Rewrites every caller in `file` that reaches an item through a redirect's
/// facade path, given the module path the file occupies and the crate's
/// module bindings a caller path may go through.
pub(in crate::fixes) fn redirect_callers_in_file(
    file: &Path,
    sources: &ParsedSources,
    module_path: Vec<String>,
    redirects: &[FacadeRedirect],
    module_aliases: &ModuleAliases,
) -> Result<FileRedirects> {
    let source = sources
        .source(file)
        .with_context(|| format!("failed to read {}", file.display()))?;
    let syntax = source
        .syntax()
        .with_context(|| format!("failed to parse {}", file.display()))?;
    Ok(redirect_callers_in_syntax(
        file,
        source.text(),
        syntax,
        module_path,
        redirects,
        module_aliases,
    ))
}

fn redirect_callers_in_syntax(
    file: &Path,
    source: &str,
    syntax: &File,
    module_path: Vec<String>,
    redirects: &[FacadeRedirect],
    module_aliases: &ModuleAliases,
) -> FileRedirects {
    let lines = SourceLines::new(source);
    let scope = CallerRewriter::binding_scope(
        syntax.items.iter(),
        ScopeKind::Module,
        &module_path,
        redirects,
        module_aliases,
        &lines,
    );
    let mut rewriter = CallerRewriter {
        file,
        source,
        lines,
        module_path,
        scope,
        outer_scopes: Vec::new(),
        redirects,
        module_aliases,
        fixes: Vec::new(),
        outside_used: BTreeMap::new(),
        named_owner_modules: BTreeSet::new(),
        cfg_test_depth: 0,
    };
    rewriter.visit_file(syntax);
    FileRedirects {
        fixes:               rewriter.fixes,
        outside_used:        rewriter.outside_used,
        named_owner_modules: rewriter.named_owner_modules,
    }
}

/// Whether a binding scope is a module, where name lookup stops, or a block,
/// which sees the names of the module around it.
enum ScopeKind {
    Module,
    Block,
}

/// The names the `use` items of one scope bind, each mapped to the
/// crate-relative path it names, so `tool_staging::f()` written under
/// `use crate::tool::tool_staging;` resolves to `tool::tool_staging::f`.
/// Traversal always owns this active scope, so finishing deferred imports
/// cannot silently return because no scope exists.
struct ActiveBindingScope {
    kind:          ScopeKind,
    bindings:      FxHashMap<String, Vec<String>>,
    /// The bindings of a redirect's `unchanged_module`, by bound name.
    owner_imports: FxHashMap<String, OwnerImport>,
    /// `use` items holding an owner import, rewritten when the scope closes
    /// and every path through the import has been seen.
    deferred:      Vec<ItemUse>,
}

/// A `use` binding of a redirect's `unchanged_module`, such as
/// `use crate::tool::tool_staging;`, and what became of the paths written
/// through it.
struct OwnerImport {
    item_start: usize,
    leaf:       NamedUseLeaf,
    module:     Vec<String>,
    visibility: OwnerImportVisibility,
    redirected: RedirectedPathPresence,
    kept:       RetainedPathPresence,
}

/// Every non-inherited visibility is a re-export and keeps its owner import.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OwnerImportVisibility {
    Private,
    Reexport,
}

impl From<&Visibility> for OwnerImportVisibility {
    fn from(visibility: &Visibility) -> Self {
        match visibility {
            Visibility::Inherited => Self::Private,
            Visibility::Public(_) | Visibility::Restricted(_) => Self::Reexport,
        }
    }
}

/// Whether any path through an owner binding has been redirected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RedirectedPathPresence {
    Absent,
    Present,
}

/// A retained path keeps an owner import, even when other paths were redirected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RetainedPathPresence {
    Absent,
    Present,
}

/// A private import whose every path was redirected is removed; re-exports stay.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ImportRetention {
    Remove,
    Keep,
}

/// Which owner-import history a caller path contributes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CallerPathDisposition {
    Redirected,
    Retained,
}

impl OwnerImport {
    const fn retention(&self) -> ImportRetention {
        match (self.visibility, self.redirected, self.kept) {
            (
                OwnerImportVisibility::Private,
                RedirectedPathPresence::Present,
                RetainedPathPresence::Absent,
            ) => ImportRetention::Remove,
            (
                OwnerImportVisibility::Reexport,
                RedirectedPathPresence::Absent | RedirectedPathPresence::Present,
                RetainedPathPresence::Absent | RetainedPathPresence::Present,
            )
            | (
                OwnerImportVisibility::Private,
                RedirectedPathPresence::Absent,
                RetainedPathPresence::Absent | RetainedPathPresence::Present,
            )
            | (
                OwnerImportVisibility::Private,
                RedirectedPathPresence::Present,
                RetainedPathPresence::Present,
            ) => ImportRetention::Keep,
        }
    }

    const fn observe(&mut self, disposition: CallerPathDisposition) {
        match disposition {
            CallerPathDisposition::Redirected => self.redirected = RedirectedPathPresence::Present,
            CallerPathDisposition::Retained => self.kept = RetainedPathPresence::Present,
        }
    }
}

/// Whether an unused owner import leaf was left out of a use item.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum OwnerLeafRemoval {
    #[default]
    Unchanged,
    Removed,
}

/// Whether a resolved module is an owner a redirect leaves unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OwnerModuleMembership {
    UnchangedOwner,
    Unrelated,
}

/// A glob keeps its unchanged owner module and is never redirected as a named leaf.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ImportLeafKind {
    Named,
    Glob,
}

/// Whether a use subtree has a leaf to redirect or remove.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TreeRewrite {
    Unchanged,
    Required,
}

/// A `use` leaf by its written path (a `self` leaf's is its prefix) and the
/// name it binds. Unbound root `self` leaves have no named leaf to remove.
#[derive(Clone, PartialEq, Eq)]
struct NamedUseLeaf {
    path: Vec<String>,
    name: String,
}

/// The lines one `use` item becomes: leaves no redirect touches, each on its
/// own line in written order, then redirected leaves grouped by the module
/// they now come from.
#[derive(Default)]
struct UseItemLines {
    kept:       Vec<String>,
    redirected: BTreeMap<Vec<String>, Vec<String>>,
    /// Whether an unused owner import leaf was left out.
    removed:    OwnerLeafRemoval,
}

/// The completed edit for a use item, before source text is emitted.
enum UseRewritePlan {
    Unchanged,
    Delete,
    Replace { lines: Vec<String> },
}

impl From<UseItemLines> for UseRewritePlan {
    fn from(lines: UseItemLines) -> Self {
        if lines.redirected.is_empty() && lines.removed == OwnerLeafRemoval::Unchanged {
            return Self::Unchanged;
        }
        if lines.kept.is_empty() && lines.redirected.is_empty() {
            return Self::Delete;
        }
        Self::Replace {
            lines: lines
                .kept
                .into_iter()
                .chain(lines.redirected.into_values().flatten())
                .collect(),
        }
    }
}

/// What happens to a leaf after removal and redirect lookup, in that order.
enum UseLeafDisposition<'a> {
    Removed,
    Redirected { target: &'a [String] },
    Kept { path: Vec<String> },
}

struct CallerRewriter<'a> {
    file:                &'a Path,
    source:              &'a str,
    lines:               SourceLines<'a>,
    module_path:         Vec<String>,
    scope:               ActiveBindingScope,
    outer_scopes:        Vec<ActiveBindingScope>,
    redirects:           &'a [FacadeRedirect],
    module_aliases:      &'a ModuleAliases,
    fixes:               Vec<UseFix>,
    outside_used:        BTreeMap<usize, CallerBuilds>,
    named_owner_modules: BTreeSet<Vec<String>>,
    /// How many enclosing inline modules carry `#[cfg(test)]`.
    cfg_test_depth:      usize,
}

impl Visit<'_> for CallerRewriter<'_> {
    fn visit_file(&mut self, node: &File) {
        visit::visit_file(self, node);
        self.close_scope();
    }

    fn visit_item_mod(&mut self, node: &ItemMod) {
        if let Some((_, items)) = &node.content {
            let cfg_test = rust_syntax::is_cfg_test(&node.attrs);
            self.cfg_test_depth += usize::from(cfg_test);
            self.module_path.push(node.ident.unraw().to_string());
            let scope = Self::binding_scope(
                items.iter(),
                ScopeKind::Module,
                &self.module_path,
                self.redirects,
                self.module_aliases,
                &self.lines,
            );
            self.in_scope(scope, |rewriter| {
                for item in items {
                    rewriter.visit_item(item);
                }
            });
            self.module_path.pop();
            self.cfg_test_depth -= usize::from(cfg_test);
        }
    }

    fn visit_block(&mut self, node: &Block) {
        let items = node.stmts.iter().filter_map(|stmt| match stmt {
            Stmt::Item(item) => Some(item),
            _ => None,
        });
        let scope = Self::binding_scope(
            items,
            ScopeKind::Block,
            &self.module_path,
            self.redirects,
            self.module_aliases,
            &self.lines,
        );
        self.in_scope(scope, |rewriter| visit::visit_block(rewriter, node));
    }

    fn visit_item_use(&mut self, node: &ItemUse) {
        // `CallerRewriter::visit_item_use` skips `node.leading_colon` because a leading `::`
        // anchors the path at an extern crate, never at a module of the crate under
        // analysis.
        if node.leading_colon.is_some() {
            return;
        }
        self.record_use_tree_paths(&node.tree);
        let Ok((start, _)) = source_text::item_use_byte_range(&self.lines, node) else {
            return;
        };
        if self
            .scope
            .owner_imports
            .values()
            .any(|import| import.item_start == start)
        {
            self.scope.deferred.push(node.clone());
            return;
        }
        self.rewrite_use_item(node, &[]);
    }

    fn visit_path(&mut self, node: &syn::Path) {
        if node.leading_colon.is_none() {
            let idents = node
                .segments
                .iter()
                .map(|segment| segment.ident.clone())
                .collect::<Vec<_>>();
            self.rewrite_path_prefix(&idents);
        }
        // `CallerRewriter::visit_path` keeps walking after `rewrite_path_prefix`: a match replaces
        // only the leading segments, so a facade name nested in the generic arguments
        // (`Vec<super::Widget>`) is still ahead of the cursor.
        visit::visit_path(self, node);
    }

    fn visit_item_macro(&mut self, node: &ItemMacro) {
        // `CallerRewriter::visit_item_macro` skips `macro_rules!` bodies because they are a pattern
        // language, not caller code.
        if !node.mac.path.is_ident("macro_rules") {
            visit::visit_item_macro(self, node);
        }
    }

    fn visit_macro(&mut self, node: &Macro) {
        self.rewrite_token_paths(node.tokens.clone());
        visit::visit_macro(self, node);
    }
}

impl<'a> CallerRewriter<'a> {
    fn binding_scope<'item>(
        items: impl Iterator<Item = &'item Item>,
        kind: ScopeKind,
        module_path: &[String],
        redirects: &[FacadeRedirect],
        module_aliases: &ModuleAliases,
        lines: &SourceLines<'_>,
    ) -> ActiveBindingScope {
        let mut bindings = FxHashMap::default();
        let mut owner_imports = FxHashMap::default();
        for item in items {
            let Item::Use(item_use) = item else {
                continue;
            };
            if item_use.leading_colon.is_some() {
                continue;
            }
            let Ok((item_start, _)) = source_text::item_use_byte_range(lines, item_use) else {
                continue;
            };
            for binding in imports::collect_use_bindings(&item_use.tree) {
                let UseBinding::Named { name, path } = binding else {
                    continue;
                };
                let segments = split_path(&path);
                let Some(absolute) = module_path::absolute_use_path(module_path, &segments) else {
                    continue;
                };
                let module = module_aliases.resolve(absolute.clone());
                if redirects
                    .iter()
                    .any(|redirect| redirect.unchanged_module.as_ref() == Some(&module))
                {
                    let Some(leaf) = bound_leaf(segments, Some(&name)) else {
                        continue;
                    };
                    let import = OwnerImport {
                        item_start,
                        leaf,
                        module,
                        visibility: OwnerImportVisibility::from(&item_use.vis),
                        redirected: RedirectedPathPresence::Absent,
                        kept: RetainedPathPresence::Absent,
                    };
                    owner_imports.insert(name.clone(), import);
                }
                bindings.insert(name, absolute);
            }
        }
        ActiveBindingScope {
            kind,
            bindings,
            owner_imports,
            deferred: Vec::new(),
        }
    }

    fn owner_module_membership(&self, path: &[String]) -> OwnerModuleMembership {
        if self
            .redirects
            .iter()
            .any(|redirect| redirect.unchanged_module.as_deref() == Some(path))
        {
            OwnerModuleMembership::UnchangedOwner
        } else {
            OwnerModuleMembership::Unrelated
        }
    }

    /// Runs inside a new active scope while retaining every enclosing binding.
    fn in_scope(&mut self, scope: ActiveBindingScope, visit: impl FnOnce(&mut Self)) {
        let parent = mem::replace(&mut self.scope, scope);
        let parent_index = self.outer_scopes.len();
        self.outer_scopes.push(parent);
        visit(self);
        self.close_scope();
        self.scope = self.outer_scopes.remove(parent_index);
    }

    /// Finishes the active scope's deferred imports. An active scope always
    /// exists: closing without one would silently lose deferred caller edits.
    fn close_scope(&mut self) {
        let deferred = mem::take(&mut self.scope.deferred);
        let mut unused = Vec::new();
        for import in self.scope.owner_imports.values() {
            match import.retention() {
                ImportRetention::Remove => unused.push((import.item_start, import.leaf.clone())),
                ImportRetention::Keep => {
                    self.named_owner_modules.insert(import.module.clone());
                },
            }
        }
        for item_use in &deferred {
            let Ok((start, _)) = source_text::item_use_byte_range(&self.lines, item_use) else {
                continue;
            };
            let removed = unused
                .iter()
                .filter(|(item_start, _)| *item_start == start)
                .map(|(_, leaf)| leaf.clone())
                .collect::<Vec<_>>();
            self.rewrite_use_item(item_use, &removed);
        }
    }

    /// Rewrites one `use` item: redirected leaves repointed, `removed` leaves
    /// dropped, and the whole item deleted when nothing is left.
    fn rewrite_use_item(&mut self, node: &ItemUse, removed: &[NamedUseLeaf]) {
        let mut lines = UseItemLines::default();
        self.plan_use_tree(&[], &node.tree, removed, &mut lines);
        match UseRewritePlan::from(lines) {
            UseRewritePlan::Unchanged => {},
            UseRewritePlan::Delete => {
                let Ok((start, end)) = source_text::item_use_byte_range(&self.lines, node) else {
                    return;
                };
                self.fixes.push(source_text::line_deletion(
                    self.file,
                    self.source,
                    start..end,
                ));
            },
            UseRewritePlan::Replace { lines } => {
                let Ok((start, end)) = source_text::item_use_byte_range(&self.lines, node) else {
                    return;
                };
                let Ok(position) = self.lines.position(node.tree.span().start()) else {
                    return;
                };
                let tree_start = SourceLines::offset(position);
                let header = &self.source[start..tree_start];
                let separator = format!("\n{}", self.lines.indent_before(start));
                let replacement = lines
                    .into_iter()
                    .map(|path| format!("{header}{path};"))
                    .collect::<Vec<_>>()
                    .join(&separator);
                self.push_fix(start, end, replacement);
            },
        }
    }

    /// Records, for each leaf of `tree` written through an owner import,
    /// whether a redirect repoints it. A glob of an unchanged module names
    /// that module.
    fn record_use_tree_paths(&mut self, tree: &UseTree) {
        for binding in imports::collect_use_bindings(tree) {
            let (path, kind) = match binding {
                UseBinding::Named { path, .. } => (path, ImportLeafKind::Named),
                UseBinding::Glob { path } => (path, ImportLeafKind::Glob),
            };
            let segments = split_path(&path);
            let disposition = match kind {
                ImportLeafKind::Glob => {
                    if let Some(module) = self.resolved(&segments)
                        && self.owner_module_membership(&module)
                            == OwnerModuleMembership::UnchangedOwner
                    {
                        self.named_owner_modules.insert(module);
                    }
                    CallerPathDisposition::Retained
                },
                ImportLeafKind::Named => {
                    if self.redirect_lookup(&segments).is_some() {
                        CallerPathDisposition::Redirected
                    } else {
                        CallerPathDisposition::Retained
                    }
                },
            };
            self.record_owner_import_path(&segments, disposition);
        }
    }

    /// Marks the owner import `written` starts with, if any, as having a
    /// redirected path or a kept one.
    fn record_owner_import_path(&mut self, written: &[String], disposition: CallerPathDisposition) {
        let Some(first) = written.first() else {
            return;
        };
        let Some(index) = self.binding_scope_index(first) else {
            return;
        };
        if let Some(import) = self.scope_mut(index).owner_imports.get_mut(first) {
            import.observe(disposition);
        }
    }

    /// The crate-relative path `written` names here: through a `use` binding
    /// when its first segment is one, else relative to the current module.
    fn absolute(&self, written: &[String]) -> Option<Vec<String>> {
        let Some(first) = written.first() else {
            return Some(self.module_path.clone());
        };
        if PathAnchor::from(first.as_str()) == PathAnchor::Name
            && let Some(bound) = self.binding(first)
        {
            return Some(bound.iter().chain(&written[1..]).cloned().collect());
        }
        module_path::absolute_use_path(&self.module_path, written)
    }

    /// The crate-relative path `written` names here, with every module binding
    /// it goes through replaced by the module bound, so it compares equal to a
    /// facade path however the caller reached that module.
    fn resolved(&self, written: &[String]) -> Option<Vec<String>> {
        self.absolute(written)
            .map(|absolute| self.module_aliases.resolve(absolute))
    }

    fn binding(&self, name: &str) -> Option<&Vec<String>> {
        let index = self.binding_scope_index(name)?;
        self.scope_at(index).bindings.get(name)
    }

    /// The innermost scope binding `name`; lookup stops at the enclosing
    /// module scope.
    fn binding_scope_index(&self, name: &str) -> Option<usize> {
        for (index, scope) in iter::once((self.outer_scopes.len(), &self.scope))
            .chain(self.outer_scopes.iter().enumerate().rev())
        {
            if scope.bindings.contains_key(name) {
                return Some(index);
            }
            if matches!(scope.kind, ScopeKind::Module) {
                break;
            }
        }
        None
    }

    fn scope_at(&self, index: usize) -> &ActiveBindingScope {
        if index == self.outer_scopes.len() {
            &self.scope
        } else {
            &self.outer_scopes[index]
        }
    }

    fn scope_mut(&mut self, index: usize) -> &mut ActiveBindingScope {
        if index == self.outer_scopes.len() {
            &mut self.scope
        } else {
            &mut self.outer_scopes[index]
        }
    }

    /// The redirect `written` names here, the path a caller here writes
    /// instead, and the side of the redirect that path came from.
    fn redirect_lookup(&self, written: &[String]) -> Option<(usize, &'a [String], TargetSide)> {
        let resolved = self.resolved(written)?;
        let redirects: &'a [FacadeRedirect] = self.redirects;
        let (index, redirect) = redirects
            .iter()
            .enumerate()
            .find(|(_, redirect)| redirect.facade_path == resolved)?;
        let (target, side) = redirect.target_for(&self.module_path)?;
        Some((index, target, side))
    }

    /// The path a caller here writes instead of `written`, when `written`
    /// names a redirected facade path. Records the redirect when the caller
    /// receives its `outside` path.
    fn redirect_target(&mut self, written: &[String]) -> Option<&'a [String]> {
        let (index, target, side) = self.redirect_lookup(written)?;
        if side == TargetSide::Outside {
            let builds = if self.cfg_test_depth > 0 {
                CallerBuilds::TestOnly
            } else {
                CallerBuilds::Every
            };
            let widest = self.outside_used.entry(index).or_insert(builds);
            *widest = (*widest).max(builds);
        }
        Some(target)
    }

    fn plan_use_tree(
        &mut self,
        written: &[String],
        tree: &UseTree,
        removed: &[NamedUseLeaf],
        lines: &mut UseItemLines,
    ) {
        match tree {
            UseTree::Path(path) => {
                let mut next = written.to_vec();
                next.push(path.ident.to_string());
                if self.tree_rewrite(&next, &path.tree, removed) == TreeRewrite::Required {
                    self.plan_use_tree(&next, &path.tree, removed, lines);
                } else {
                    lines.kept.push(self.render_kept_subtree(written, tree));
                }
            },
            UseTree::Name(name) => {
                self.plan_use_leaf(written, &name.ident, None, removed, lines);
            },
            UseTree::Rename(rename) => {
                let alias = rename.rename.to_string();
                self.plan_use_leaf(written, &rename.ident, Some(&alias), removed, lines);
            },
            UseTree::Glob(_) => lines.kept.push(self.render_kept_subtree(written, tree)),
            UseTree::Group(group) => {
                for item in &group.items {
                    self.plan_use_tree(written, item, removed, lines);
                }
            },
        }
    }

    fn plan_use_leaf(
        &mut self,
        written: &[String],
        ident: &Ident,
        alias: Option<&str>,
        removed: &[NamedUseLeaf],
        lines: &mut UseItemLines,
    ) {
        let leaf = leaf_path(written, ident);
        match self.use_leaf_disposition(leaf, alias, removed) {
            UseLeafDisposition::Removed => lines.removed = OwnerLeafRemoval::Removed,
            UseLeafDisposition::Redirected { target } => {
                lines
                    .redirected
                    .entry(target[..target.len().saturating_sub(1)].to_vec())
                    .or_default()
                    .push(module_path::import_path(&self.module_path, target, alias));
            },
            UseLeafDisposition::Kept { path } => {
                let rendered = self.absolute(&path).map_or_else(
                    || render_written(&path, alias),
                    |absolute| module_path::import_path(&self.module_path, &absolute, alias),
                );
                lines.kept.push(rendered);
            },
        }
    }

    fn use_leaf_disposition(
        &mut self,
        leaf: Vec<String>,
        alias: Option<&str>,
        removed: &[NamedUseLeaf],
    ) -> UseLeafDisposition<'a> {
        if bound_leaf(leaf.clone(), alias).is_some_and(|leaf| removed.contains(&leaf)) {
            return UseLeafDisposition::Removed;
        }
        self.redirect_target(&leaf)
            .map_or(UseLeafDisposition::Kept { path: leaf }, |target| {
                UseLeafDisposition::Redirected { target }
            })
    }

    /// Whether a leaf of `tree` is redirected or in `removed`.
    fn tree_rewrite(
        &self,
        written: &[String],
        tree: &UseTree,
        removed: &[NamedUseLeaf],
    ) -> TreeRewrite {
        let leaf_rewrite = |ident: &Ident, alias: Option<&str>| {
            let leaf = leaf_path(written, ident);
            if self.redirect_lookup(&leaf).is_some()
                || bound_leaf(leaf, alias).is_some_and(|leaf| removed.contains(&leaf))
            {
                TreeRewrite::Required
            } else {
                TreeRewrite::Unchanged
            }
        };
        match tree {
            UseTree::Path(path) => {
                let mut next = written.to_vec();
                next.push(path.ident.to_string());
                self.tree_rewrite(&next, &path.tree, removed)
            },
            UseTree::Name(name) => leaf_rewrite(&name.ident, None),
            UseTree::Rename(rename) => {
                leaf_rewrite(&rename.ident, Some(&rename.rename.to_string()))
            },
            UseTree::Glob(_) => TreeRewrite::Unchanged,
            UseTree::Group(group) => {
                for item in &group.items {
                    match self.tree_rewrite(written, item, removed) {
                        TreeRewrite::Required => return TreeRewrite::Required,
                        TreeRewrite::Unchanged => {},
                    }
                }
                TreeRewrite::Unchanged
            },
        }
    }

    /// A subtree no redirect touches, written on its own line under the
    /// current spelling of its prefix.
    fn render_kept_subtree(&self, written: &[String], tree: &UseTree) -> String {
        let prefix = self.absolute(written).map_or_else(
            || render_written(written, None),
            |absolute| module_path::import_path(&self.module_path, &absolute, None),
        );
        format!("{prefix}::{}", use_tree::render_use_tree(tree))
    }

    /// Replaces the leading segments of an inline path when they name a
    /// redirected facade path.
    ///
    /// The written path can be longer than the facade name it goes through —
    /// `super::Widget::new()` reaches `Widget` at its second segment — so
    /// every prefix is resolved and compared, shortest first. A single-segment
    /// path is skipped: that name came from a `use`, and `visit_item_use`
    /// repoints the `use` instead.
    fn rewrite_path_prefix(&mut self, idents: &[Ident]) {
        let segments = idents.iter().map(ToString::to_string).collect::<Vec<_>>();
        let found = (2..=segments.len()).find_map(|length| {
            self.redirect_target(&segments[..length])
                .map(|target| (length, target))
        });
        let anchored = found.and_then(|(_, target)| self.anchored_path(&segments[0], target));
        // `record_owner_import_path` tracks module imports only for paths with several segments: a
        // module import is only ever their first segment; a path still written through it
        // keeps it used.
        if segments.len() > 1 {
            let disposition = if found.is_some() && anchored.is_none() {
                CallerPathDisposition::Redirected
            } else {
                CallerPathDisposition::Retained
            };
            self.record_owner_import_path(&segments, disposition);
        }
        let Some((length, target)) = found else {
            return;
        };
        let Ok((start, end)) = self
            .lines
            .span_range(idents[0].span().start(), idents[length - 1].span().end())
        else {
            return;
        };
        let replacement =
            anchored.unwrap_or_else(|| module_path::import_path(&self.module_path, target, None));
        self.push_fix(start, end, replacement);
    }

    /// `target` written through `anchor`, the caller's first segment, when
    /// `anchor` names a module (a module import, an alias, or a name a glob
    /// such as `use super::*;` brings in) that `target` sits in. The caller
    /// keeps its own spelling and the binding or glob it went through stays
    /// used.
    fn anchored_path(&self, anchor: &str, target: &[String]) -> Option<String> {
        if PathAnchor::from(anchor) != PathAnchor::Name {
            return None;
        }
        let module = self
            .absolute(&[anchor.to_string()])
            .map(|absolute| self.module_aliases.resolve_module(absolute))?;
        let rest = target.strip_prefix(module.as_slice())?;
        Some(
            iter::once(anchor)
                .chain(rest.iter().map(String::as_str))
                .collect::<Vec<_>>()
                .join("::"),
        )
    }

    /// Finds `ident (:: ident)+` runs in a macro invocation's tokens, nested
    /// delimiters included, and rewrites each like an inline path.
    fn rewrite_token_paths(&mut self, tokens: TokenStream) {
        let trees = tokens.into_iter().collect::<Vec<_>>();
        let mut index = 0;
        while index < trees.len() {
            match &trees[index] {
                TokenTree::Group(group) => {
                    self.rewrite_token_paths(group.stream());
                    index += 1;
                },
                TokenTree::Ident(first) => match path_position(&trees, index) {
                    PathPosition::Continuation => index += 1,
                    PathPosition::Start => {
                        let mut idents = vec![first.clone()];
                        let mut cursor = index + 1;
                        while let Some(next) = ident_after_path_separator(&trees, cursor) {
                            idents.push(next);
                            cursor += 3;
                        }
                        self.rewrite_path_prefix(&idents);
                        index = cursor;
                    },
                },
                TokenTree::Literal(_) | TokenTree::Punct(_) => index += 1,
            }
        }
    }

    fn push_fix(&mut self, start: usize, end: usize, replacement: String) {
        self.fixes.push(UseFix {
            path: self.file.to_path_buf(),
            start,
            end,
            replacement,
            import_group: None,
        });
    }
}

fn split_path(path: &str) -> Vec<String> { path.split("::").map(str::to_string).collect() }

/// The written path and bound name of a `use` leaf, given its written path
/// and alias.
fn bound_leaf(path: Vec<String>, alias: Option<&str>) -> Option<NamedUseLeaf> {
    let name = alias.map(str::to_string).or_else(|| path.last().cloned())?;
    if name.is_empty() {
        return None;
    }
    Some(NamedUseLeaf { path, name })
}

/// The written path a `use` leaf binds; a `self` leaf binds its prefix.
fn leaf_path(written: &[String], ident: &Ident) -> Vec<String> {
    let mut leaf = written.to_vec();
    if ident != "self" {
        leaf.push(ident.to_string());
    }
    leaf
}

fn render_written(written: &[String], alias: Option<&str>) -> String {
    let path = written.join("::");
    match alias {
        Some(alias) => format!("{path} as {alias}"),
        None => path,
    }
}

/// Whether punctuation is the colon with the spacing needed for a path separator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PathColonMatch {
    Matches,
    Rejected,
}

fn path_colon_match(tree: Option<&TokenTree>, spacing: Spacing) -> PathColonMatch {
    match tree {
        Some(TokenTree::Punct(punct)) => {
            if punct.as_char() == ':' && punct.spacing() == spacing {
                PathColonMatch::Matches
            } else {
                PathColonMatch::Rejected
            }
        },
        Some(TokenTree::Group(_) | TokenTree::Ident(_) | TokenTree::Literal(_)) | None => {
            PathColonMatch::Rejected
        },
    }
}

/// A preceding `::` continues a path or anchors it at the extern prelude.
/// Only a start position may begin a caller path to rewrite.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PathPosition {
    Continuation,
    Start,
}

fn path_position(trees: &[TokenTree], index: usize) -> PathPosition {
    if index >= 2
        && path_colon_match(trees.get(index - 2), Spacing::Joint) == PathColonMatch::Matches
        && path_colon_match(trees.get(index - 1), Spacing::Alone) == PathColonMatch::Matches
    {
        PathPosition::Continuation
    } else {
        PathPosition::Start
    }
}

/// The identifier after a `::` starting at `trees[index]`.
fn ident_after_path_separator(trees: &[TokenTree], index: usize) -> Option<Ident> {
    if path_colon_match(trees.get(index), Spacing::Joint) == PathColonMatch::Rejected
        || path_colon_match(trees.get(index + 1), Spacing::Alone) == PathColonMatch::Rejected
    {
        return None;
    }
    match trees.get(index + 2) {
        Some(TokenTree::Ident(ident)) => Some(ident.clone()),
        Some(TokenTree::Group(_) | TokenTree::Literal(_) | TokenTree::Punct(_)) | None => None,
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::cmp::Reverse;
    use std::collections::BTreeMap;
    use std::collections::BTreeSet;
    use std::path::Path;

    use proc_macro2::Spacing;
    use proc_macro2::TokenStream;
    use proc_macro2::TokenTree;
    use syn::Ident;
    use syn::ItemUse;

    use super::CallerBuilds;
    use super::CallerPathDisposition;
    use super::FileRedirects;
    use super::ImportRetention;
    use super::OwnerImport;
    use super::OwnerImportVisibility;
    use super::OwnerLeafRemoval;
    use super::OwnerModuleMembership;
    use super::PathColonMatch;
    use super::PathPosition;
    use super::RedirectedPathPresence;
    use super::RetainedPathPresence;
    use super::TreeRewrite;
    use super::UseItemLines;
    use super::UseRewritePlan;
    use super::redirect_callers_in_syntax;
    use crate::fixes::facade_redirect::FacadeRedirect;
    use crate::fixes::facade_redirect::ModuleAliases;
    use crate::fixes::facade_redirect::RedirectTarget;

    fn redirect_callers_in_source(
        file: &Path,
        source: &str,
        module_path: Vec<String>,
        redirects: &[FacadeRedirect],
        module_aliases: &ModuleAliases,
    ) -> syn::Result<FileRedirects> {
        let syntax = syn::parse_file(source)?;
        Ok(redirect_callers_in_syntax(
            file,
            source,
            &syntax,
            module_path,
            redirects,
            module_aliases,
        ))
    }

    fn path(text: &str) -> Vec<String> {
        text.split("::")
            .filter(|segment| !segment.is_empty())
            .map(str::to_string)
            .collect()
    }

    fn redirect(facade: &str, target: &str) -> FacadeRedirect {
        FacadeRedirect {
            facade_path:      path(facade),
            target:           RedirectTarget::Everywhere(path(target)),
            unchanged_module: None,
        }
    }

    /// `source` in module `module` after every fix is applied.
    fn rewritten(source: &str, module: &str, redirects: &[FacadeRedirect]) -> String {
        let mut fixes = redirect_callers_in_source(
            Path::new("caller.rs"),
            source,
            path(module),
            redirects,
            &ModuleAliases::default(),
        )
        .expect("rewrite callers")
        .fixes;
        fixes.sort_by_key(|fix| Reverse(fix.start));
        let mut text = source.to_string();
        for fix in fixes {
            text.replace_range(fix.start..fix.end, &fix.replacement);
        }
        text
    }

    #[test]
    fn a_nested_group_leaf_is_split_out_and_redirected() {
        assert_eq!(
            rewritten(
                "use crate::tool::{staging::X, Other};\n",
                "input",
                &[redirect("tool::staging::X", "tool::staging::inner::X")],
            ),
            "use crate::tool::Other;\nuse crate::tool::staging::inner::X;\n"
        );
    }

    #[test]
    fn a_rename_matches_on_the_imported_name_and_keeps_its_alias() {
        assert_eq!(
            rewritten(
                "use super::{Widget as W, Other};\n",
                "parent::sibling",
                &[redirect("parent::Widget", "parent::child::Widget")],
            ),
            "use super::Other;\nuse super::child::Widget as W;\n"
        );
    }

    #[test]
    fn visibility_attributes_and_indent_repeat_on_every_line() {
        assert_eq!(
            rewritten(
                "mod inner {\n    #[cfg(test)]\n    pub(crate) use crate::parent::{Widget, Other};\n}\n",
                "",
                &[redirect("parent::Widget", "parent::child::Widget")],
            ),
            "mod inner {\n    #[cfg(test)]\n    pub(crate) use crate::parent::Other;\n    #[cfg(test)]\n    pub(crate) use crate::parent::child::Widget;\n}\n"
        );
    }

    #[test]
    fn a_path_through_a_module_import_is_redirected() {
        assert_eq!(
            rewritten(
                "use crate::tool::staging;\n\nfn f() { staging::run(); }\n",
                "input",
                &[redirect("tool::staging::run", "tool::run")],
            ),
            "use crate::tool::staging;\n\nfn f() { crate::tool::run(); }\n"
        );
    }

    #[test]
    fn a_target_under_the_callers_module_import_is_written_through_it() {
        assert_eq!(
            rewritten(
                "use crate::tool;\n\nfn f() { tool::staging::run(); }\n",
                "input",
                &[redirect("tool::staging::run", "tool::run")],
            ),
            "use crate::tool;\n\nfn f() { tool::run(); }\n"
        );
    }

    #[test]
    fn a_target_under_the_callers_alias_is_written_through_it() {
        assert_eq!(
            rewritten(
                "use crate::tool as kit;\n\nfn f() { assert_eq!(kit::staging::run(), 1); }\n",
                "input",
                &[redirect("tool::staging::run", "tool::run")],
            ),
            "use crate::tool as kit;\n\nfn f() { assert_eq!(kit::run(), 1); }\n"
        );
    }

    #[test]
    fn a_path_inside_a_macro_call_is_redirected() {
        assert_eq!(
            rewritten(
                "fn f() { assert_eq!(tool::staging::f(), 1); }\n",
                "",
                &[redirect("tool::staging::f", "tool::f")],
            ),
            "fn f() { assert_eq!(tool::f(), 1); }\n"
        );
    }

    #[test]
    fn a_macro_rules_body_is_left_alone() {
        let source = "macro_rules! m { () => { tool::staging::f() }; }\n";
        assert_eq!(
            rewritten(source, "", &[redirect("tool::staging::f", "tool::f")]),
            source
        );
    }

    #[test]
    fn an_extern_prelude_path_is_left_alone() {
        let source = "fn f() { ::tool::staging::f(); m!(::tool::staging::f()); }\n";
        assert_eq!(
            rewritten(source, "", &[redirect("tool::staging::f", "tool::f")]),
            source
        );
    }

    #[test]
    fn the_unchanged_module_keeps_the_facade_path() {
        let source = "fn f() { super::Widget::new(); }\n";
        let mut kept = redirect("parent::Widget", "parent::child::Widget");
        kept.unchanged_module = Some(path("parent::sibling"));
        assert_eq!(rewritten(source, "parent::sibling", &[kept]), source);
    }

    #[test]
    fn an_inline_path_keeps_its_trailing_segments_and_generics() {
        assert_eq!(
            rewritten(
                "fn f() -> Vec<super::Widget> { super::Widget::new() }\n",
                "parent::sibling",
                &[redirect("parent::Widget", "parent::child::Widget")],
            ),
            "fn f() -> Vec<super::child::Widget> { super::child::Widget::new() }\n"
        );
    }

    fn scoped_redirect() -> FacadeRedirect {
        FacadeRedirect {
            facade_path:      path("tool::tool_staging::stage"),
            target:           RedirectTarget::Scoped {
                scope:   path("tool"),
                inside:  path("tool::stager::stage"),
                outside: path("tool::stage"),
            },
            unchanged_module: Some(path("tool::tool_staging")),
        }
    }

    /// A caller outside the scope that imports the owner module and calls
    /// through it gets the `outside` path, and the redirect is reported.
    #[test]
    fn a_module_import_caller_outside_the_scope_gets_the_outside_path() {
        let source = "use crate::tool::tool_staging;\nfn f() { tool_staging::stage(); }\n";
        let redirects = [scoped_redirect()];
        let result = redirect_callers_in_source(
            Path::new("caller.rs"),
            source,
            path("app"),
            &redirects,
            &ModuleAliases::default(),
        )
        .expect("rewrite callers");
        assert_eq!(
            result.outside_used.into_iter().collect::<Vec<_>>(),
            vec![(0, CallerBuilds::Every)]
        );
        assert_eq!(
            rewritten(source, "app", &redirects),
            "fn f() { crate::tool::stage(); }\n"
        );
    }

    /// A caller outside the scope under `#[cfg(test)]` reports its redirect
    /// as used by test builds only; one ungated caller widens it to every
    /// build.
    #[test]
    fn a_cfg_test_module_caller_reports_test_only_builds() {
        let outside_used = |source: &str| {
            redirect_callers_in_source(
                Path::new("caller.rs"),
                source,
                path("app"),
                &[scoped_redirect()],
                &ModuleAliases::default(),
            )
            .expect("rewrite callers")
            .outside_used
            .into_iter()
            .collect::<Vec<_>>()
        };
        let test_only =
            "#[cfg(test)]\nmod tests {\n    fn f() { crate::tool::tool_staging::stage(); }\n}\n";
        assert_eq!(outside_used(test_only), vec![(0, CallerBuilds::TestOnly)]);
        let both = format!("fn g() {{ crate::tool::tool_staging::stage(); }}\n{test_only}");
        assert_eq!(outside_used(&both), vec![(0, CallerBuilds::Every)]);
    }

    #[test]
    fn a_caller_inside_the_scope_gets_the_inside_path() {
        let source = "use super::tool_staging;\nfn f() { tool_staging::stage(); }\n";
        let redirects = [scoped_redirect()];
        let result = redirect_callers_in_source(
            Path::new("caller.rs"),
            source,
            path("tool::render"),
            &redirects,
            &ModuleAliases::default(),
        )
        .expect("rewrite callers");
        assert!(result.outside_used.is_empty());
        assert_eq!(
            rewritten(source, "tool::render", &redirects),
            "fn f() { super::stager::stage(); }\n"
        );
    }

    /// The unchanged modules a `use` in `source` still names after the rewrite.
    fn named_owner_modules(source: &str) -> Vec<Vec<String>> {
        redirect_callers_in_source(
            Path::new("caller.rs"),
            source,
            path("app"),
            &[scoped_redirect()],
            &ModuleAliases::default(),
        )
        .expect("rewrite callers")
        .named_owner_modules
        .into_iter()
        .collect()
    }

    #[test]
    fn a_fully_redirected_module_import_leaves_its_group() {
        let source = "use crate::tool::{tool_staging, other};\nfn f() { tool_staging::stage(); other::g(); }\n";
        assert_eq!(
            rewritten(source, "app", &[scoped_redirect()]),
            "use crate::tool::other;\nfn f() { crate::tool::stage(); other::g(); }\n"
        );
        assert_eq!(named_owner_modules(source), [] as [Vec<String>; 0]);
    }

    #[test]
    fn a_fully_redirected_module_import_alias_is_removed() {
        assert_eq!(
            rewritten(
                "use crate::tool::tool_staging as staging;\nfn f() { staging::stage(); }\n",
                "app",
                &[scoped_redirect()],
            ),
            "fn f() { crate::tool::stage(); }\n"
        );
    }

    #[test]
    fn a_module_import_with_a_path_left_unredirected_is_kept() {
        let source = "use crate::tool::tool_staging;\nfn f() { tool_staging::stage(); tool_staging::other(); }\n";
        assert_eq!(
            rewritten(source, "app", &[scoped_redirect()]),
            "use crate::tool::tool_staging;\nfn f() { crate::tool::stage(); tool_staging::other(); }\n"
        );
        assert_eq!(
            named_owner_modules(source),
            vec![path("tool::tool_staging")]
        );
    }

    #[test]
    fn a_public_reexport_of_the_module_is_kept() {
        let source = "pub use crate::tool::tool_staging;\nfn f() { tool_staging::stage(); }\n";
        assert_eq!(
            rewritten(source, "app", &[scoped_redirect()]),
            "pub use crate::tool::tool_staging;\nfn f() { crate::tool::stage(); }\n"
        );
        assert_eq!(
            named_owner_modules(source),
            vec![path("tool::tool_staging")]
        );
    }
    fn rewriter<'a>(
        redirects: &'a [FacadeRedirect],
        aliases: &'a ModuleAliases,
    ) -> super::CallerRewriter<'a> {
        super::CallerRewriter {
            file: Path::new("caller.rs"),
            source: "",
            lines: super::SourceLines::new(""),
            module_path: path("app"),
            scope: super::ActiveBindingScope {
                kind:          super::ScopeKind::Module,
                bindings:      super::FxHashMap::default(),
                owner_imports: super::FxHashMap::default(),
                deferred:      Vec::new(),
            },
            outer_scopes: Vec::new(),
            redirects,
            module_aliases: aliases,
            fixes: Vec::new(),
            outside_used: BTreeMap::default(),
            named_owner_modules: BTreeSet::default(),
            cfg_test_depth: 0,
        }
    }

    #[test]
    fn removing_a_leaf_precedes_redirect_and_outside_use_accounting() {
        let redirects = [scoped_redirect()];
        let aliases = ModuleAliases::default();
        let mut rewriter = rewriter(&redirects, &aliases);
        let ident: Ident = syn::parse_str("stage").expect("parse name");
        let removed = [
            super::bound_leaf(path("crate::tool::tool_staging::stage"), Some("alias"))
                .expect("named aliased leaf"),
        ];
        let mut lines = super::UseItemLines::default();
        rewriter.plan_use_leaf(
            &path("crate::tool::tool_staging"),
            &ident,
            Some("alias"),
            &removed,
            &mut lines,
        );
        assert_eq!(lines.removed, super::OwnerLeafRemoval::Removed);
        assert!(lines.redirected.is_empty());
        assert!(rewriter.outside_used.is_empty());
        rewriter.plan_use_leaf(
            &path("crate::tool::tool_staging"),
            &ident,
            Some("other_alias"),
            &removed,
            &mut lines,
        );
        assert_eq!(
            rewriter.outside_used.into_iter().collect::<Vec<_>>(),
            vec![(0, CallerBuilds::Every)]
        );
        assert_eq!(
            lines.redirected.into_values().flatten().collect::<Vec<_>>(),
            vec!["crate::tool::stage as other_alias"]
        );
    }

    #[test]
    fn nested_blocks_restore_the_enclosing_module_binding() {
        let source = concat!(
            "use crate::tool;\nfn f() { { use crate::tool as kit; ",
            "kit::staging::run(); } tool::staging::run(); }\n",
        );
        let expected = concat!(
            "use crate::tool;\nfn f() { { use crate::tool as kit; ",
            "kit::run(); } tool::run(); }\n",
        );
        assert_eq!(
            rewritten(
                source,
                "app",
                &[redirect("tool::staging::run", "tool::run")]
            ),
            expected
        );
    }

    #[test]
    fn unbound_root_leaves_have_no_named_removal_identity() {
        assert!(super::bound_leaf(Vec::new(), None).is_none());
        assert!(super::bound_leaf(Vec::new(), Some("")).is_none());
        let aliased = super::bound_leaf(Vec::new(), Some("root")).expect("aliased root");
        assert_eq!(aliased.name, "root");
    }

    #[test]
    fn an_untouched_group_retains_its_original_spelling() {
        let source = "use crate::other::{Second, First as Alias};\n";
        assert_eq!(rewritten(source, "app", &[scoped_redirect()]), source);
    }
    #[test]
    fn owner_import_retention_preserves_every_history_and_visibility() {
        for (visibility, redirected, kept, expected) in [
            (
                OwnerImportVisibility::Private,
                RedirectedPathPresence::Absent,
                RetainedPathPresence::Absent,
                ImportRetention::Keep,
            ),
            (
                OwnerImportVisibility::Private,
                RedirectedPathPresence::Absent,
                RetainedPathPresence::Present,
                ImportRetention::Keep,
            ),
            (
                OwnerImportVisibility::Private,
                RedirectedPathPresence::Present,
                RetainedPathPresence::Absent,
                ImportRetention::Remove,
            ),
            (
                OwnerImportVisibility::Private,
                RedirectedPathPresence::Present,
                RetainedPathPresence::Present,
                ImportRetention::Keep,
            ),
            (
                OwnerImportVisibility::Reexport,
                RedirectedPathPresence::Absent,
                RetainedPathPresence::Absent,
                ImportRetention::Keep,
            ),
            (
                OwnerImportVisibility::Reexport,
                RedirectedPathPresence::Absent,
                RetainedPathPresence::Present,
                ImportRetention::Keep,
            ),
            (
                OwnerImportVisibility::Reexport,
                RedirectedPathPresence::Present,
                RetainedPathPresence::Absent,
                ImportRetention::Keep,
            ),
            (
                OwnerImportVisibility::Reexport,
                RedirectedPathPresence::Present,
                RetainedPathPresence::Present,
                ImportRetention::Keep,
            ),
        ] {
            let mut import = OwnerImport {
                item_start: 0,
                leaf: super::bound_leaf(path("owner"), None).expect("owner leaf"),
                module: path("owner"),
                visibility,
                redirected,
                kept,
            };
            assert_eq!(import.retention(), expected);
            import.observe(CallerPathDisposition::Redirected);
            assert_eq!(import.redirected, RedirectedPathPresence::Present);
            import.observe(CallerPathDisposition::Retained);
            assert_eq!(import.kept, RetainedPathPresence::Present);
            assert_eq!(import.retention(), ImportRetention::Keep);
        }
        for (source, expected) in [
            ("use owner;", OwnerImportVisibility::Private),
            ("pub use owner;", OwnerImportVisibility::Reexport),
            ("pub(crate) use owner;", OwnerImportVisibility::Reexport),
            ("pub(self) use owner;", OwnerImportVisibility::Reexport),
        ] {
            let item: ItemUse = syn::parse_str(source).expect("use item");
            assert_eq!(OwnerImportVisibility::from(&item.vis), expected);
        }
    }

    #[test]
    fn owner_leaf_removal_distinguishes_untouched_deleted_and_remaining_lines() {
        assert!(matches!(
            UseRewritePlan::from(UseItemLines::default()),
            UseRewritePlan::Unchanged
        ));
        let mut lines = UseItemLines {
            removed: OwnerLeafRemoval::Removed,
            ..UseItemLines::default()
        };
        assert!(matches!(
            UseRewritePlan::from(UseItemLines {
                removed: OwnerLeafRemoval::Removed,
                ..UseItemLines::default()
            }),
            UseRewritePlan::Delete
        ));
        lines.kept.push("owner::Kept".to_string());
        assert!(
            matches!(UseRewritePlan::from(lines), UseRewritePlan::Replace { lines }
            if lines == ["owner::Kept"])
        );
    }

    #[test]
    fn owner_membership_and_recursive_tree_rewrites_preserve_leaf_decisions() {
        let redirects = [scoped_redirect()];
        let aliases = ModuleAliases::default();
        let rewriter = rewriter(&redirects, &aliases);
        assert_eq!(
            rewriter.owner_module_membership(&path("tool::tool_staging")),
            OwnerModuleMembership::UnchangedOwner
        );
        assert_eq!(
            rewriter.owner_module_membership(&path("tool::stager")),
            OwnerModuleMembership::Unrelated
        );
        let removed = [super::bound_leaf(path("gone"), None).expect("leaf")];
        for (source, expected) in [
            (
                "use crate::tool::{tool_staging::{stage, keep}, untouched};",
                TreeRewrite::Required,
            ),
            (
                "use crate::tool::tool_staging::stage as alias;",
                TreeRewrite::Required,
            ),
            ("use gone;", TreeRewrite::Required),
            ("use gone as alias;", TreeRewrite::Unchanged),
            ("use crate::tool::tool_staging::*;", TreeRewrite::Unchanged),
            ("use {untouched, {}};", TreeRewrite::Unchanged),
            (
                "use {{}, crate::tool::tool_staging::stage};",
                TreeRewrite::Required,
            ),
        ] {
            let item: ItemUse = syn::parse_str(source).expect("use tree");
            assert_eq!(
                rewriter.tree_rewrite(&[], &item.tree, &removed),
                expected,
                "{source}"
            );
        }
    }

    #[test]
    fn a_glob_keeps_its_owner_while_a_named_leaf_redirects() {
        let source = concat!(
            "use crate::tool::tool_staging;\n",
            "use tool_staging::{*, stage};\n",
        );
        assert_eq!(
            rewritten(source, "app", &[scoped_redirect()]),
            concat!(
                "use crate::tool::tool_staging;\n",
                "use crate::tool::tool_staging::*;\nuse crate::tool::stage;\n",
            )
        );
        assert_eq!(
            named_owner_modules(source),
            vec![path("tool::tool_staging")]
        );
    }

    #[test]
    fn macro_path_positions_require_both_colons_and_their_spacing() {
        let tokens: TokenStream = "a::b".parse().expect("tokens");
        let trees = tokens.into_iter().collect::<Vec<_>>();
        assert_eq!(super::path_position(&trees, 0), PathPosition::Start);
        assert_eq!(super::path_position(&trees, 3), PathPosition::Continuation);
        let rejected: TokenStream = "() 1 +".parse().expect("non-colon tokens");
        let rejected = rejected.into_iter().collect::<Vec<_>>();
        for (token, spacing, expected) in [
            (rejected.first(), Spacing::Alone, PathColonMatch::Rejected),
            (rejected.get(1), Spacing::Alone, PathColonMatch::Rejected),
            (rejected.get(2), Spacing::Alone, PathColonMatch::Rejected),
            (trees.get(1), Spacing::Joint, PathColonMatch::Matches),
            (trees.get(2), Spacing::Alone, PathColonMatch::Matches),
            (trees.get(1), Spacing::Alone, PathColonMatch::Rejected),
            (trees.first(), Spacing::Joint, PathColonMatch::Rejected),
            (None, Spacing::Joint, PathColonMatch::Rejected),
        ] {
            assert_eq!(super::path_colon_match(token, spacing), expected);
        }
        for source in ["a : b", "a + b", "a : : b"] {
            let tokens: TokenStream = source.parse().expect("tokens");
            let trees = tokens.into_iter().collect::<Vec<TokenTree>>();
            assert_eq!(
                super::path_position(&trees, trees.len() - 1),
                PathPosition::Start
            );
        }
    }

    #[test]
    fn path_separator_without_a_following_identifier_is_rejected() {
        for source in ["::", "::()", "::1", "::+"] {
            let tokens: TokenStream = source.parse().expect("separator tokens");
            let trees = tokens.into_iter().collect::<Vec<_>>();
            assert!(super::ident_after_path_separator(&trees, 0).is_none());
        }
    }
}
