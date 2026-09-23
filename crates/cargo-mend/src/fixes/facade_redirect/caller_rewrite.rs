use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs;
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
use syn::parse_file;
use syn::spanned::Spanned;
use syn::visit;
use syn::visit::Visit;

use super::module_aliases::ModuleAliases;
use super::module_path::absolute_use_path;
use super::module_path::import_path;
use super::redirect::FacadeRedirect;
use super::redirect::TargetSide;
use super::source_text::SourceLines;
use super::source_text::item_use_byte_range;
use super::source_text::line_deletion;
use super::use_tree::render_use_tree;
use crate::fixes::imports::UseBinding;
use crate::fixes::imports::UseFix;
use crate::fixes::imports::collect_use_bindings;
use crate::rust_syntax;
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
    module_path: Vec<String>,
    redirects: &[FacadeRedirect],
    module_aliases: &ModuleAliases,
) -> Result<FileRedirects> {
    let source =
        fs::read_to_string(file).with_context(|| format!("failed to read {}", file.display()))?;
    redirect_callers_in_source(file, &source, module_path, redirects, module_aliases)
}

fn redirect_callers_in_source(
    file: &Path,
    source: &str,
    module_path: Vec<String>,
    redirects: &[FacadeRedirect],
    module_aliases: &ModuleAliases,
) -> Result<FileRedirects> {
    let syntax =
        parse_file(source).with_context(|| format!("failed to parse {}", file.display()))?;
    let mut rewriter = CallerRewriter {
        file,
        source,
        lines: SourceLines::new(source),
        module_path,
        scopes: Vec::new(),
        redirects,
        module_aliases,
        fixes: Vec::new(),
        outside_used: BTreeMap::new(),
        named_owner_modules: BTreeSet::new(),
        cfg_test_depth: 0,
    };
    rewriter.visit_file(&syntax);
    Ok(FileRedirects {
        fixes:               rewriter.fixes,
        outside_used:        rewriter.outside_used,
        named_owner_modules: rewriter.named_owner_modules,
    })
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
struct BindingScope {
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
    leaf:       BoundLeaf,
    module:     Vec<String>,
    public:     bool,
    redirected: bool,
    kept:       bool,
}

impl OwnerImport {
    /// A private import whose every path was redirected is left unused, so
    /// its leaf is removed. A public one is a re-export and always stays.
    const fn is_unused(&self) -> bool { !self.public && self.redirected && !self.kept }
}

/// A `use` leaf by its written path (a `self` leaf's is its prefix) and the
/// name it binds.
#[derive(Clone, PartialEq, Eq)]
struct BoundLeaf {
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
    removed:    bool,
}

struct CallerRewriter<'a> {
    file:                &'a Path,
    source:              &'a str,
    lines:               SourceLines<'a>,
    module_path:         Vec<String>,
    scopes:              Vec<BindingScope>,
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
        let scope = self.binding_scope(node.items.iter(), ScopeKind::Module);
        self.scopes.push(scope);
        visit::visit_file(self, node);
        self.close_scope();
    }

    fn visit_item_mod(&mut self, node: &ItemMod) {
        if let Some((_, items)) = &node.content {
            let cfg_test = rust_syntax::is_cfg_test(&node.attrs);
            self.cfg_test_depth += usize::from(cfg_test);
            self.module_path.push(node.ident.unraw().to_string());
            let scope = self.binding_scope(items.iter(), ScopeKind::Module);
            self.scopes.push(scope);
            for item in items {
                self.visit_item(item);
            }
            self.close_scope();
            self.module_path.pop();
            self.cfg_test_depth -= usize::from(cfg_test);
        }
    }

    fn visit_block(&mut self, node: &Block) {
        let items = node.stmts.iter().filter_map(|stmt| match stmt {
            Stmt::Item(item) => Some(item),
            _ => None,
        });
        let scope = self.binding_scope(items, ScopeKind::Block);
        self.scopes.push(scope);
        visit::visit_block(self, node);
        self.close_scope();
    }

    fn visit_item_use(&mut self, node: &ItemUse) {
        // A leading `::` anchors the path at an extern crate, never at a
        // module of the crate under analysis.
        if node.leading_colon.is_some() {
            return;
        }
        self.record_use_tree_paths(&node.tree);
        let (start, _) = item_use_byte_range(&self.lines, node);
        if let Some(scope) = self.scopes.last_mut()
            && scope
                .owner_imports
                .values()
                .any(|import| import.item_start == start)
        {
            scope.deferred.push(node.clone());
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
        // Keep walking: a match replaces only the leading segments, so a facade
        // name nested in the generic arguments (`Vec<super::Widget>`) is still
        // ahead of the cursor.
        visit::visit_path(self, node);
    }

    fn visit_item_macro(&mut self, node: &ItemMacro) {
        // A `macro_rules!` body is a pattern language, not caller code.
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
        &self,
        items: impl Iterator<Item = &'item Item>,
        kind: ScopeKind,
    ) -> BindingScope {
        let mut bindings = FxHashMap::default();
        let mut owner_imports = FxHashMap::default();
        for item in items {
            let Item::Use(item_use) = item else {
                continue;
            };
            if item_use.leading_colon.is_some() {
                continue;
            }
            let (item_start, _) = item_use_byte_range(&self.lines, item_use);
            for binding in collect_use_bindings(&item_use.tree) {
                let UseBinding::Named { name, path } = binding else {
                    continue;
                };
                let segments = split_path(&path);
                let Some(absolute) = absolute_use_path(&self.module_path, &segments) else {
                    continue;
                };
                let module = self.module_aliases.resolve(absolute.clone());
                if self.is_unchanged_module(&module) {
                    let import = OwnerImport {
                        item_start,
                        leaf: BoundLeaf {
                            path: segments,
                            name: name.clone(),
                        },
                        module,
                        public: !matches!(item_use.vis, Visibility::Inherited),
                        redirected: false,
                        kept: false,
                    };
                    owner_imports.insert(name.clone(), import);
                }
                bindings.insert(name, absolute);
            }
        }
        BindingScope {
            kind,
            bindings,
            owner_imports,
            deferred: Vec::new(),
        }
    }

    fn is_unchanged_module(&self, path: &[String]) -> bool {
        self.redirects
            .iter()
            .any(|redirect| redirect.unchanged_module.as_deref() == Some(path))
    }

    /// Rewrites the scope's deferred `use` items without their unused owner
    /// imports, records the unchanged modules its other owner imports still
    /// name, and leaves the scope.
    fn close_scope(&mut self) {
        let Some(scope) = self.scopes.last_mut() else {
            return;
        };
        let deferred = mem::take(&mut scope.deferred);
        let mut unused = Vec::new();
        for import in scope.owner_imports.values() {
            if import.is_unused() {
                unused.push((import.item_start, import.leaf.clone()));
            } else {
                self.named_owner_modules.insert(import.module.clone());
            }
        }
        for item_use in &deferred {
            let (start, _) = item_use_byte_range(&self.lines, item_use);
            let removed = unused
                .iter()
                .filter(|(item_start, _)| *item_start == start)
                .map(|(_, leaf)| leaf.clone())
                .collect::<Vec<_>>();
            self.rewrite_use_item(item_use, &removed);
        }
        self.scopes.pop();
    }

    /// Rewrites one `use` item: redirected leaves repointed, `removed` leaves
    /// dropped, and the whole item deleted when nothing is left.
    fn rewrite_use_item(&mut self, node: &ItemUse, removed: &[BoundLeaf]) {
        let mut lines = UseItemLines::default();
        self.plan_use_tree(&[], &node.tree, removed, &mut lines);
        if lines.redirected.is_empty() && !lines.removed {
            return;
        }
        let (start, end) = item_use_byte_range(&self.lines, node);
        if lines.kept.is_empty() && lines.redirected.is_empty() {
            self.fixes
                .push(line_deletion(self.file, self.source, start..end));
            return;
        }
        let tree_start = self.lines.offset(node.tree.span().start());
        let header = &self.source[start..tree_start];
        let separator = format!("\n{}", self.lines.indent_before(start));
        let replacement = lines
            .kept
            .into_iter()
            .chain(lines.redirected.into_values().flatten())
            .map(|path| format!("{header}{path};"))
            .collect::<Vec<_>>()
            .join(&separator);
        self.push_fix(start, end, replacement);
    }

    /// Records, for each leaf of `tree` written through an owner import,
    /// whether a redirect repoints it. A glob of an unchanged module names
    /// that module.
    fn record_use_tree_paths(&mut self, tree: &UseTree) {
        for binding in collect_use_bindings(tree) {
            let (path, glob) = match binding {
                UseBinding::Named { path, .. } => (path, false),
                UseBinding::Glob { path } => (path, true),
            };
            let segments = split_path(&path);
            if glob
                && let Some(module) = self.resolved(&segments)
                && self.is_unchanged_module(&module)
            {
                self.named_owner_modules.insert(module);
            }
            let redirected = !glob && self.redirect_lookup(&segments).is_some();
            self.record_owner_import_path(&segments, redirected);
        }
    }

    /// Marks the owner import `written` starts with, if any, as having a
    /// redirected path or a kept one.
    fn record_owner_import_path(&mut self, written: &[String], redirected: bool) {
        let Some(first) = written.first() else {
            return;
        };
        let Some(index) = self.binding_scope_index(first) else {
            return;
        };
        if let Some(import) = self.scopes[index].owner_imports.get_mut(first) {
            if redirected {
                import.redirected = true;
            } else {
                import.kept = true;
            }
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
        absolute_use_path(&self.module_path, written)
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
        self.scopes[index].bindings.get(name)
    }

    /// The innermost scope binding `name`; lookup stops at the enclosing
    /// module scope.
    fn binding_scope_index(&self, name: &str) -> Option<usize> {
        for (index, scope) in self.scopes.iter().enumerate().rev() {
            if scope.bindings.contains_key(name) {
                return Some(index);
            }
            if matches!(scope.kind, ScopeKind::Module) {
                break;
            }
        }
        None
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
        removed: &[BoundLeaf],
        lines: &mut UseItemLines,
    ) {
        match tree {
            UseTree::Path(path) => {
                let mut next = written.to_vec();
                next.push(path.ident.to_string());
                if self.tree_changes(&next, &path.tree, removed) {
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
        removed: &[BoundLeaf],
        lines: &mut UseItemLines,
    ) {
        let leaf = leaf_path(written, ident);
        if removed.contains(&bound_leaf(leaf.clone(), alias)) {
            lines.removed = true;
        } else if let Some(target) = self.redirect_target(&leaf) {
            lines
                .redirected
                .entry(target[..target.len().saturating_sub(1)].to_vec())
                .or_default()
                .push(import_path(&self.module_path, target, alias));
        } else {
            let rendered = self.absolute(&leaf).map_or_else(
                || render_written(&leaf, alias),
                |absolute| import_path(&self.module_path, &absolute, alias),
            );
            lines.kept.push(rendered);
        }
    }

    /// Whether a leaf of `tree` is redirected or in `removed`.
    fn tree_changes(&self, written: &[String], tree: &UseTree, removed: &[BoundLeaf]) -> bool {
        let leaf_changes = |ident: &Ident, alias: Option<&str>| {
            let leaf = leaf_path(written, ident);
            self.redirect_lookup(&leaf).is_some() || removed.contains(&bound_leaf(leaf, alias))
        };
        match tree {
            UseTree::Path(path) => {
                let mut next = written.to_vec();
                next.push(path.ident.to_string());
                self.tree_changes(&next, &path.tree, removed)
            },
            UseTree::Name(name) => leaf_changes(&name.ident, None),
            UseTree::Rename(rename) => {
                leaf_changes(&rename.ident, Some(&rename.rename.to_string()))
            },
            UseTree::Glob(_) => false,
            UseTree::Group(group) => group
                .items
                .iter()
                .any(|item| self.tree_changes(written, item, removed)),
        }
    }

    /// A subtree no redirect touches, written on its own line under the
    /// current spelling of its prefix.
    fn render_kept_subtree(&self, written: &[String], tree: &UseTree) -> String {
        let prefix = self.absolute(written).map_or_else(
            || render_written(written, None),
            |absolute| import_path(&self.module_path, &absolute, None),
        );
        format!("{prefix}::{}", render_use_tree(tree))
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
        // A module import is only ever a path's first segment of several; a
        // path still written through it keeps it used.
        if segments.len() > 1 {
            self.record_owner_import_path(&segments, found.is_some() && anchored.is_none());
        }
        let Some((length, target)) = found else {
            return;
        };
        let start = self.lines.offset(idents[0].span().start());
        let end = self.lines.offset(idents[length - 1].span().end());
        let replacement = anchored.unwrap_or_else(|| import_path(&self.module_path, target, None));
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
                TokenTree::Ident(first) if !follows_path_separator(&trees, index) => {
                    let mut idents = vec![first.clone()];
                    let mut cursor = index + 1;
                    while let Some(next) = ident_after_path_separator(&trees, cursor) {
                        idents.push(next);
                        cursor += 3;
                    }
                    self.rewrite_path_prefix(&idents);
                    index = cursor;
                },
                _ => index += 1,
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
fn bound_leaf(path: Vec<String>, alias: Option<&str>) -> BoundLeaf {
    let name = alias
        .map(str::to_string)
        .or_else(|| path.last().cloned())
        .unwrap_or_default();
    BoundLeaf { path, name }
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

fn is_colon(tree: Option<&TokenTree>, spacing: Spacing) -> bool {
    matches!(tree, Some(TokenTree::Punct(punct))
        if punct.as_char() == ':' && punct.spacing() == spacing)
}

/// Whether `trees[index]` is preceded by `::`, so it continues a path (or
/// starts one anchored at the extern prelude) rather than starting one.
fn follows_path_separator(trees: &[TokenTree], index: usize) -> bool {
    index >= 2
        && is_colon(trees.get(index - 2), Spacing::Joint)
        && is_colon(trees.get(index - 1), Spacing::Alone)
}

/// The identifier after a `::` starting at `trees[index]`.
fn ident_after_path_separator(trees: &[TokenTree], index: usize) -> Option<Ident> {
    if !is_colon(trees.get(index), Spacing::Joint)
        || !is_colon(trees.get(index + 1), Spacing::Alone)
    {
        return None;
    }
    match trees.get(index + 2) {
        Some(TokenTree::Ident(ident)) => Some(ident.clone()),
        _ => None,
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::cmp::Reverse;
    use std::path::Path;

    use super::CallerBuilds;
    use super::redirect_callers_in_source;
    use crate::fixes::facade_redirect::FacadeRedirect;
    use crate::fixes::facade_redirect::ModuleAliases;
    use crate::fixes::facade_redirect::RedirectTarget;

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
        assert!(named_owner_modules(source).is_empty());
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
}
