use std::cmp::Ordering;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::iter;
use std::path::Path;
use std::path::PathBuf;

use rustc_hir::Attribute;
use rustc_hir::HirId;
use rustc_hir::Item;
use rustc_hir::ItemKind;
use rustc_hir::UseKind;
use rustc_hir::UsePath;
use rustc_hir::attrs::AttributeKind;
use rustc_hir::def::DefKind;
use rustc_middle::metadata::ModChild;
use rustc_middle::metadata::Reexport;
use rustc_middle::ty::TyCtxt;
use rustc_middle::ty::Visibility;
use rustc_span::BytePos;
use rustc_span::Ident;
use rustc_span::Pos;
use rustc_span::Span;
use rustc_span::Symbol;
use rustc_span::def_id::CRATE_DEF_ID;
use rustc_span::def_id::DefId;
use rustc_span::def_id::LocalDefId;
use rustc_span::def_id::LocalModDefId;
use rustc_span::source_map::SourceMap;

use super::pub_use_outside_subtree;
use crate::compiler::persistence::StoredAncestorReexport;
use crate::compiler::persistence::StoredAncestorReexportInsertion;
use crate::compiler::persistence::StoredFinding;
use crate::compiler::persistence::StoredSubtreeReexportFixFact;
use crate::compiler::visibility::source;
use crate::reporting::ModuleForm;
use crate::reporting::ReexportVisibility;

/// Whether the walk in `pub_use_outside_subtree::entered_module` followed
/// every module segment, or stopped at an enum, a trait, or a name the module
/// does not list.
#[derive(Clone, Copy)]
pub(super) enum PathWalk {
    Complete,
    Interrupted,
}

/// A `pub_use_outside_subtree` re-export, as the check resolved it.
pub(super) struct SubtreeReexport<'a, 'hir> {
    pub item:          &'a Item<'hir>,
    pub path:          &'a UsePath<'hir>,
    pub use_kind:      UseKind,
    pub owner_module:  LocalDefId,
    pub source_module: LocalDefId,
    pub path_walk:     PathWalk,
}

/// Why `--fix` leaves a `pub_use_outside_subtree` finding for the author.
pub(super) enum NoFixReason {
    Glob,
    Rename,
    PathThroughEnumOrTrait,
    PublicOwner { owner: String },
    Unresolved,
    NeedsPathBoundary,
    ConflictingItem { module: String, name: String },
    NarrowerImport { module: String, name: String },
}

impl Display for NoFixReason {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Glob => write!(formatter, "the re-export is a glob"),
            Self::Rename => write!(formatter, "the re-export renames the item with `as`"),
            Self::PathThroughEnumOrTrait => write!(
                formatter,
                "the path passes through an enum or trait before its last segment"
            ),
            Self::PublicOwner { owner } => write!(
                formatter,
                "`{owner}` is reachable from outside the crate, so removing the re-export would \
                 change a public path"
            ),
            Self::Unresolved => write!(
                formatter,
                "the re-exported item does not resolve to one target"
            ),
            Self::NeedsPathBoundary => {
                write!(formatter, "the re-export would need a `pub(in …)` boundary")
            },
            Self::ConflictingItem { module, name } => write!(
                formatter,
                "`{module}` already has a different item named `{name}`"
            ),
            Self::NarrowerImport { module, name } => write!(
                formatter,
                "`{module}` already imports `{name}` with narrower visibility"
            ),
        }
    }
}

/// Everything the fixer needs about one fixable re-export except the position
/// of its finding, which `SubtreeReexportFix::into_fact` copies from the
/// finding itself.
pub(super) struct SubtreeReexportFix {
    exported_name:     String,
    written_path:      String,
    owner_module:      Vec<String>,
    owner_module_form: ModuleForm,
    source_module:     Vec<String>,
    target_scope:      Vec<String>,
    common_ancestor:   Vec<String>,
    ancestor_reexport: StoredAncestorReexport,
}

impl SubtreeReexportFix {
    pub(super) fn into_fact(
        self,
        finding: &StoredFinding,
        crate_root_file: &Path,
    ) -> StoredSubtreeReexportFixFact {
        StoredSubtreeReexportFixFact {
            use_path:          finding.path.clone(),
            use_line:          finding.line,
            use_column:        finding.column,
            exported_name:     self.exported_name,
            written_path:      self.written_path,
            owner_module:      self.owner_module,
            owner_module_form: self.owner_module_form,
            source_module:     self.source_module,
            target_scope:      self.target_scope,
            common_ancestor:   self.common_ancestor,
            crate_root_file:   crate_root_file.to_string_lossy().into_owned(),
            ancestor_reexport: self.ancestor_reexport,
        }
    }
}

/// How the common ancestor module already binds the re-exported name.
enum AncestorBinding {
    /// No explicit binding, or only glob bindings an explicit `use` shadows.
    Absent,
    /// Every namespace is already bound to the same item, wide enough.
    Sufficient,
}

/// Where the fixer inserts the ancestor re-export.
struct InsertionAnchor {
    file:   PathBuf,
    offset: usize,
    indent: String,
}

/// Decides whether `--fix` can rewrite `reexport`, and computes the rewrite.
///
/// The target scope is the widest scope every caller of the item can sit in.
/// Callers inside it name the item through its canonical path once the
/// re-export is gone; callers outside it need a re-export at the common
/// ancestor, which is where the item's own subtree hands it outward.
pub(super) fn classify(
    tcx: TyCtxt<'_>,
    reexport: &SubtreeReexport<'_, '_>,
    crate_root_file: &Path,
) -> Result<SubtreeReexportFix, NoFixReason> {
    let exported_name = match reexport.use_kind {
        UseKind::Glob => return Err(NoFixReason::Glob),
        UseKind::ListStem => return Err(NoFixReason::Unresolved),
        UseKind::Single(ident) => ident,
    };
    let last_segment = reexport
        .path
        .segments
        .last()
        .ok_or(NoFixReason::Unresolved)?;
    if last_segment.ident.name != exported_name.name {
        return Err(NoFixReason::Rename);
    }
    if matches!(reexport.path_walk, PathWalk::Interrupted) {
        return Err(NoFixReason::PathThroughEnumOrTrait);
    }
    // A `use` in a function body is scoped to that body, not to the module.
    if tcx.def_kind(tcx.local_parent(reexport.item.owner_id.def_id)) != DefKind::Mod {
        return Err(NoFixReason::Unresolved);
    }
    if tcx
        .effective_visibilities(())
        .is_exported(reexport.owner_module)
    {
        return Err(NoFixReason::PublicOwner {
            owner: pub_use_outside_subtree::module_display_path(tcx, reexport.owner_module),
        });
    }
    let bindings = source_bindings(tcx, reexport, exported_name.name)?;
    let common_ancestor = common_ancestor(tcx, reexport.owner_module, reexport.source_module);
    let target_scope = target_scope(tcx, reexport.source_module, &bindings)?;
    if !tcx.is_descendant_of(common_ancestor.to_def_id(), target_scope.to_def_id()) {
        return Err(NoFixReason::Unresolved);
    }
    let ancestor_reexport = ancestor_reexport(
        tcx,
        reexport,
        exported_name,
        &bindings,
        common_ancestor,
        target_scope,
        crate_root_file,
    )?;
    Ok(SubtreeReexportFix {
        exported_name: exported_name.to_string(),
        written_path: reexport
            .path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect::<Vec<_>>()
            .join("::"),
        owner_module: module_path(tcx, reexport.owner_module)?,
        owner_module_form: module_form(tcx, reexport.owner_module),
        source_module: module_path(tcx, reexport.source_module)?,
        target_scope: module_path(tcx, target_scope)?,
        common_ancestor: module_path(tcx, common_ancestor)?,
        ancestor_reexport,
    })
}

/// Crate-relative module path, empty for the crate root. Fails with
/// `NoFixReason::Unresolved` when a module on the chain sits inside a function
/// body, where the path has no spelling.
pub(super) fn module_path(tcx: TyCtxt<'_>, module: LocalDefId) -> Result<Vec<String>, NoFixReason> {
    let mut segments = Vec::new();
    let mut current = module;
    while current != CRATE_DEF_ID {
        let parent = tcx.local_parent(current);
        if tcx.def_kind(parent) != DefKind::Mod {
            return Err(NoFixReason::Unresolved);
        }
        segments.push(Ident::with_dummy_span(tcx.item_name(current.to_def_id())).to_string());
        current = parent;
    }
    segments.reverse();
    Ok(segments)
}

/// The source module's children the re-export resolved to, one per namespace
/// the `use` bound.
fn source_bindings<'tcx>(
    tcx: TyCtxt<'tcx>,
    reexport: &SubtreeReexport<'_, '_>,
    name: Symbol,
) -> Result<Vec<&'tcx ModChild>, NoFixReason> {
    let children = tcx.module_children_local(reexport.source_module);
    let bindings = reexport
        .path
        .res
        .present_items()
        .map(|res| {
            let def_id = res.opt_def_id().ok_or(NoFixReason::Unresolved)?;
            children
                .iter()
                .find(|child| {
                    child.ident.name == name
                        && child.res.ns() == res.ns()
                        && child.res.opt_def_id() == Some(def_id)
                })
                .ok_or(NoFixReason::Unresolved)
        })
        .collect::<Result<Vec<_>, _>>()?;
    if bindings.is_empty() {
        return Err(NoFixReason::Unresolved);
    }
    Ok(bindings)
}

/// The nearest ancestor of `owner_module` whose subtree holds `source_module`.
fn common_ancestor(
    tcx: TyCtxt<'_>,
    owner_module: LocalDefId,
    source_module: LocalDefId,
) -> LocalDefId {
    let mut module = owner_module;
    while !tcx.is_descendant_of(source_module.to_def_id(), module.to_def_id()) {
        module = tcx.parent_module_from_def_id(module).into();
    }
    module
}

/// The deepest scope among the visibilities of the source module's chain and
/// the item's bindings in it: every caller of the item sits inside it. All
/// those visibilities lie on the source module's ancestor chain, so each must
/// be accessible from the deepest one.
fn target_scope(
    tcx: TyCtxt<'_>,
    source_module: LocalDefId,
    bindings: &[&ModChild],
) -> Result<LocalDefId, NoFixReason> {
    let scopes: Vec<Visibility<DefId>> = iter::successors(Some(source_module), |&module| {
        (module != CRATE_DEF_ID).then(|| tcx.parent_module_from_def_id(module).into())
    })
    .take_while(|&module| module != CRATE_DEF_ID)
    .map(|module| tcx.local_visibility(module).to_def_id())
    .chain(bindings.iter().map(|binding| binding.vis))
    .collect();
    let deepest =
        scopes.iter().fold(
            CRATE_DEF_ID.to_def_id(),
            |deepest, visibility| match visibility {
                Visibility::Restricted(scope) if tcx.is_descendant_of(*scope, deepest) => *scope,
                Visibility::Public | Visibility::Restricted(_) => deepest,
            },
        );
    if scopes
        .iter()
        .all(|visibility| visibility.is_accessible_from(deepest, tcx))
    {
        deepest.as_local().ok_or(NoFixReason::Unresolved)
    } else {
        Err(NoFixReason::Unresolved)
    }
}

fn ancestor_reexport(
    tcx: TyCtxt<'_>,
    reexport: &SubtreeReexport<'_, '_>,
    exported_name: Ident,
    bindings: &[&ModChild],
    common_ancestor: LocalDefId,
    target_scope: LocalDefId,
    crate_root_file: &Path,
) -> Result<StoredAncestorReexport, NoFixReason> {
    let use_visibility = tcx.local_visibility(reexport.item.owner_id.def_id);
    // The owner module is not exported, so a crate-wide target scope already
    // holds every caller.
    let callers_inside_target_scope = target_scope == CRATE_DEF_ID
        || matches!(use_visibility, Visibility::Restricted(scope)
            if tcx.is_descendant_of(scope.to_def_id(), target_scope.to_def_id()));
    if callers_inside_target_scope {
        return Ok(StoredAncestorReexport::NotRequired);
    }
    let reexport_visibility = reexport_visibility(tcx, use_visibility, common_ancestor, bindings)?;
    let intended = visibility_at(tcx, reexport_visibility, common_ancestor);
    match ancestor_binding(tcx, common_ancestor, exported_name.name, bindings, intended)? {
        AncestorBinding::Sufficient => Ok(StoredAncestorReexport::NotRequired),
        AncestorBinding::Absent => {
            if reexport.source_module == common_ancestor {
                return Err(NoFixReason::Unresolved);
            }
            let source_path = module_path(tcx, reexport.source_module)?;
            let ancestor_path = module_path(tcx, common_ancestor)?;
            let relative_path = source_path
                .get(ancestor_path.len()..)
                .ok_or(NoFixReason::Unresolved)?
                .iter()
                .cloned()
                .chain(iter::once(exported_name.to_string()))
                .collect::<Vec<_>>()
                .join("::");
            let anchor = insertion_anchor(tcx, common_ancestor, crate_root_file)?;
            Ok(StoredAncestorReexport::Insert(
                StoredAncestorReexportInsertion {
                    reexport_visibility,
                    relative_path,
                    cfg_attributes: cfg_attributes(tcx, reexport.item)?,
                    file: anchor.file.to_string_lossy().into_owned(),
                    offset: anchor.offset,
                    indent: anchor.indent,
                },
            ))
        },
    }
}

/// The spelling of the ancestor re-export that reaches the callers the
/// original re-export reached. A scope strictly between the ancestor's parent
/// and the crate root has no keyword spelling; `pub(crate)` stands in for it
/// only when the item itself is crate-visible.
fn reexport_visibility(
    tcx: TyCtxt<'_>,
    use_visibility: Visibility<LocalDefId>,
    common_ancestor: LocalDefId,
    bindings: &[&ModChild],
) -> Result<ReexportVisibility, NoFixReason> {
    let Visibility::Restricted(scope) = use_visibility else {
        return Ok(ReexportVisibility::Public);
    };
    if tcx.is_descendant_of(scope.to_def_id(), common_ancestor.to_def_id()) {
        return Ok(ReexportVisibility::Private);
    }
    if scope == CRATE_DEF_ID {
        return Ok(ReexportVisibility::Crate);
    }
    if scope == LocalDefId::from(tcx.parent_module_from_def_id(common_ancestor)) {
        return Ok(ReexportVisibility::Parent);
    }
    let crate_visible = bindings.iter().all(|binding| {
        binding.vis.is_public() || binding.vis == Visibility::Restricted(CRATE_DEF_ID.to_def_id())
    });
    if crate_visible {
        Ok(ReexportVisibility::Crate)
    } else {
        Err(NoFixReason::NeedsPathBoundary)
    }
}

fn visibility_at(
    tcx: TyCtxt<'_>,
    reexport_visibility: ReexportVisibility,
    module: LocalDefId,
) -> Visibility<DefId> {
    match reexport_visibility {
        ReexportVisibility::Public => Visibility::Public,
        ReexportVisibility::Crate => Visibility::Restricted(CRATE_DEF_ID.to_def_id()),
        ReexportVisibility::Parent => {
            Visibility::Restricted(tcx.parent_module_from_def_id(module).to_def_id())
        },
        ReexportVisibility::Private => Visibility::Restricted(module.to_def_id()),
    }
}

/// Compares the common ancestor's own bindings of `name` with the item's.
/// A glob binding counts as absent: an explicit `use` shadows it.
fn ancestor_binding(
    tcx: TyCtxt<'_>,
    common_ancestor: LocalDefId,
    name: Symbol,
    bindings: &[&ModChild],
    intended: Visibility<DefId>,
) -> Result<AncestorBinding, NoFixReason> {
    let children = tcx.module_children_local(common_ancestor);
    let mut explicit = Vec::new();
    for binding in bindings {
        let Some(child) = children
            .iter()
            .find(|child| child.ident.name == name && child.res.ns() == binding.res.ns())
        else {
            continue;
        };
        if child.res.opt_def_id() != binding.res.opt_def_id() {
            return Err(NoFixReason::ConflictingItem {
                module: pub_use_outside_subtree::module_display_path(tcx, common_ancestor),
                name:   name.to_string(),
            });
        }
        if !matches!(child.reexport_chain.first(), Some(Reexport::Glob(_))) {
            explicit.push(child);
        }
    }
    if explicit.is_empty() {
        return Ok(AncestorBinding::Absent);
    }
    let module = pub_use_outside_subtree::module_display_path(tcx, common_ancestor);
    if explicit.len() < bindings.len() {
        return Err(NoFixReason::ConflictingItem {
            module,
            name: name.to_string(),
        });
    }
    let wide_enough = explicit.iter().all(|child| {
        matches!(
            child.vis.partial_cmp(intended, tcx),
            Some(Ordering::Greater | Ordering::Equal)
        )
    });
    if wide_enough {
        Ok(AncestorBinding::Sufficient)
    } else {
        Err(NoFixReason::NarrowerImport {
            module,
            name: name.to_string(),
        })
    }
}

/// The source text of each `#[cfg(...)]` that evaluated true on `item`, in
/// source order. A `cfg_attr` leaves no arguments behind, so it is not copied.
fn cfg_attributes(tcx: TyCtxt<'_>, item: &Item<'_>) -> Result<Vec<String>, NoFixReason> {
    let mut spans: Vec<Span> = cfg_trace_spans(tcx, item.hir_id()).collect();
    spans.sort_by_key(|span| span.lo());
    spans.dedup();
    let source_map = tcx.sess.source_map();
    spans
        .into_iter()
        .map(|span| {
            source_map
                .span_to_snippet(span)
                .map_err(|_| NoFixReason::Unresolved)
        })
        .collect()
}

fn cfg_trace_spans(tcx: TyCtxt<'_>, hir_id: HirId) -> impl Iterator<Item = Span> {
    tcx.hir_attrs(hir_id)
        .iter()
        .filter_map(|attribute| match attribute {
            Attribute::Parsed(AttributeKind::CfgTrace(entries)) => Some(entries),
            _ => None,
        })
        .flatten()
        .map(|(_, span)| *span)
}

/// The line after the last `use` directly in `module`'s own file, or the
/// module's first item position when it has none. A `use` under `#[cfg]` is
/// skipped: it exists only in some compilation units, and every unit must pick
/// the same anchor or `load` drops the fact as conflicting.
fn insertion_anchor(
    tcx: TyCtxt<'_>,
    module: LocalDefId,
    crate_root_file: &Path,
) -> Result<InsertionAnchor, NoFixReason> {
    let (hir_module, _, _) = tcx.hir_get_module(LocalModDefId::new_unchecked(module));
    let file = if module == CRATE_DEF_ID {
        crate_root_file.to_path_buf()
    } else {
        source::real_file_path(tcx, hir_module.spans.inner_span).ok_or(NoFixReason::Unresolved)?
    };
    let source_map = tcx.sess.source_map();
    let last_use = hir_module
        .item_ids
        .iter()
        .map(|item_id| tcx.hir_item(*item_id))
        .filter(|item| {
            matches!(item.kind, ItemKind::Use(..))
                && !item.span.from_expansion()
                && cfg_trace_spans(tcx, item.hir_id()).next().is_none()
                && source::real_file_path(tcx, item.span).is_some_and(|path| path == file)
        })
        .max_by_key(|item| item.span.hi());
    let (offset, indent) = last_use.map_or_else(
        || before_first_item(source_map, hir_module.spans.inject_use_span.lo()),
        |item| after_item(source_map, item.span),
    )?;
    Ok(InsertionAnchor {
        file,
        offset,
        indent,
    })
}

/// The start of the line after `span`'s last line, and that item's indent.
fn after_item(source_map: &SourceMap, span: Span) -> Result<(usize, String), NoFixReason> {
    let last_byte = BytePos(span.hi().0.saturating_sub(1).max(span.lo().0));
    let end = source_map
        .lookup_line(last_byte)
        .map_err(|_| NoFixReason::Unresolved)?;
    let offset = (end.sf.line_bounds(end.line).end - end.sf.start_pos).to_usize();
    Ok((offset, line_indent(source_map, span.lo())?))
}

/// `inject_use_span`'s position, moved back to its line start when only
/// whitespace precedes it; in a one-line inline module it stays mid-line.
fn before_first_item(
    source_map: &SourceMap,
    inject: BytePos,
) -> Result<(usize, String), NoFixReason> {
    let start = source_map
        .lookup_line(inject)
        .map_err(|_| NoFixReason::Unresolved)?;
    let line_start = start.sf.line_bounds(start.line).start;
    let line = start
        .sf
        .get_line(start.line)
        .ok_or(NoFixReason::Unresolved)?;
    let before_inject = line
        .get(..(inject - line_start).to_usize())
        .ok_or(NoFixReason::Unresolved)?;
    let position = if before_inject.trim().is_empty() {
        line_start
    } else {
        inject
    };
    let offset = (position - start.sf.start_pos).to_usize();
    Ok((offset, line_indent(source_map, inject)?))
}

fn line_indent(source_map: &SourceMap, position: BytePos) -> Result<String, NoFixReason> {
    let located = source_map
        .lookup_line(position)
        .map_err(|_| NoFixReason::Unresolved)?;
    let line = located
        .sf
        .get_line(located.line)
        .ok_or(NoFixReason::Unresolved)?;
    Ok(line
        .chars()
        .take_while(|character| character.is_whitespace())
        .collect())
}

fn module_form(tcx: TyCtxt<'_>, module: LocalDefId) -> ModuleForm {
    if module == CRATE_DEF_ID {
        return ModuleForm::File;
    }
    let (hir_module, item_span, _) = tcx.hir_get_module(LocalModDefId::new_unchecked(module));
    if item_span.contains(hir_module.spans.inner_span) {
        ModuleForm::Inline
    } else {
        ModuleForm::File
    }
}
