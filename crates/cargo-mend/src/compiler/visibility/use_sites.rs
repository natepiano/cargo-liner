//! HIR-level use-site collector.
//!
//! Walks every body in the local crate and emits one `UseSite` per
//! resolved expression-level path reference. The output is persisted with
//! the per-compilation findings so that, after every cargo target
//! compilation has run, `load_report` can compute the union of callers
//! for each item and suppress narrowing-style findings whose proposed
//! tighter visibility would block any actual caller.
//!
//! This catches every reference rustc itself sees, including paths inside
//! macro invocations and paths produced by proc-macro expansion — both
//! of which the source-level scanner cannot.

use rustc_hir::AmbigArg;
use rustc_hir::Expr;
use rustc_hir::ExprKind;
use rustc_hir::HirId;
use rustc_hir::ImplItem;
use rustc_hir::Item;
use rustc_hir::ItemKind;
use rustc_hir::Pat;
use rustc_hir::PatExprKind;
use rustc_hir::PatKind;
use rustc_hir::QPath;
use rustc_hir::TraitItem;
use rustc_hir::Ty;
use rustc_hir::TyKind;
use rustc_hir::def::Res;
use rustc_hir::def_id::CRATE_DEF_ID;
use rustc_hir::def_id::DefId;
use rustc_hir::def_id::LocalDefId;
use rustc_hir::intravisit::Visitor;
use rustc_hir::intravisit::walk_expr;
use rustc_hir::intravisit::walk_impl_item;
use rustc_hir::intravisit::walk_item;
use rustc_hir::intravisit::walk_trait_item;
use rustc_middle::hir::nested_filter::All;
use rustc_middle::ty::TyCtxt;

use crate::compiler::persistence::UseSite;

/// Walk the entire crate's HIR and append every resolved
/// expression/type/pattern path reference to `out`. The caller module is
/// the nearest enclosing module def (defaults to the crate root).
pub(super) fn collect_use_sites(tcx: TyCtxt<'_>, out: &mut Vec<UseSite>) {
    let mut collector = UseSiteCollector {
        tcx,
        current_module: CRATE_DEF_ID.to_def_id(),
        out,
    };
    let crate_items = tcx.hir_crate_items(());
    for item_id in crate_items.free_items() {
        let item = tcx.hir_item(item_id);
        collector.visit_item(item);
    }
    for impl_item_id in crate_items.impl_items() {
        let impl_item = tcx.hir_impl_item(impl_item_id);
        collector.visit_impl_item(impl_item);
    }
    for trait_item_id in crate_items.trait_items() {
        let trait_item = tcx.hir_trait_item(trait_item_id);
        collector.visit_trait_item(trait_item);
    }
}

struct UseSiteCollector<'a, 'tcx> {
    tcx:            TyCtxt<'tcx>,
    /// Def-id of the nearest enclosing module. Updated as the visitor
    /// descends into `mod` items so each call site is tagged with the
    /// module path it lives in (not the function or impl that contains
    /// it).
    current_module: DefId,
    out:            &'a mut Vec<UseSite>,
}

impl UseSiteCollector<'_, '_> {
    fn record_target(&mut self, target: DefId) {
        // Skip references to items in other crates — narrowing decisions
        // only apply to local items.
        if !target.is_local() {
            return;
        }
        let target_def_path = self.tcx.def_path_str(target);
        let caller_module_def_path = self.tcx.def_path_str(self.current_module);
        self.out.push(UseSite {
            target_def_path,
            caller_module_def_path,
        });
    }

    fn record_qpath(&mut self, qpath: &QPath<'_>, hir_id: HirId) {
        let res = match qpath {
            QPath::Resolved(_, path) => path.res,
            QPath::TypeRelative(..) => {
                // Type-relative paths (e.g. `Foo::method`) need typeck to
                // resolve. Best-effort lookup via typeck_results.
                let owner = hir_id.owner.def_id;
                if !self.tcx.has_typeck_results(owner) {
                    return;
                }
                let typeck = self.tcx.typeck(owner);
                typeck.qpath_res(qpath, hir_id)
            },
        };
        if let Res::Def(_, def_id) = res {
            self.record_target(def_id);
        }
    }
}

impl<'tcx> Visitor<'tcx> for UseSiteCollector<'_, 'tcx> {
    type NestedFilter = All;

    fn maybe_tcx(&mut self) -> TyCtxt<'tcx> { self.tcx }

    fn visit_item(&mut self, item: &'tcx Item<'tcx>) {
        let prev = self.current_module;
        if matches!(item.kind, ItemKind::Mod(..)) {
            self.current_module = item.owner_id.def_id.to_def_id();
        } else {
            self.current_module = self
                .tcx
                .parent_module_from_def_id(item.owner_id.def_id)
                .to_def_id();
        }
        walk_item(self, item);
        self.current_module = prev;
    }

    fn visit_impl_item(&mut self, item: &'tcx ImplItem<'tcx>) {
        let prev = self.current_module;
        self.current_module = self
            .tcx
            .parent_module_from_def_id(item.owner_id.def_id)
            .to_def_id();
        walk_impl_item(self, item);
        self.current_module = prev;
    }

    fn visit_trait_item(&mut self, item: &'tcx TraitItem<'tcx>) {
        let prev = self.current_module;
        self.current_module = self
            .tcx
            .parent_module_from_def_id(item.owner_id.def_id)
            .to_def_id();
        walk_trait_item(self, item);
        self.current_module = prev;
    }

    fn visit_expr(&mut self, expr: &'tcx Expr<'tcx>) {
        match &expr.kind {
            ExprKind::Path(qpath) => self.record_qpath(qpath, expr.hir_id),
            ExprKind::MethodCall(..) => {
                // Method-call dispatch is type-dependent, not path-based.
                // The callee def-id lives in TypeckResults, not in any
                // QPath the visitor descends into.
                let owner = expr.hir_id.owner.def_id;
                if self.tcx.has_typeck_results(owner)
                    && let Some(def_id) = self.tcx.typeck(owner).type_dependent_def_id(expr.hir_id)
                {
                    self.record_target(def_id);
                }
            },
            ExprKind::Struct(qpath, ..) => self.record_qpath(qpath, expr.hir_id),
            _ => {},
        }
        walk_expr(self, expr);
    }

    fn visit_ty(&mut self, ty: &'tcx Ty<'tcx, AmbigArg>) {
        if let TyKind::Path(qpath) = &ty.kind {
            self.record_qpath(qpath, ty.hir_id);
        }
        rustc_hir::intravisit::walk_ty(self, ty);
    }

    fn visit_pat(&mut self, pat: &'tcx Pat<'tcx>) {
        if let PatKind::Expr(expr) = &pat.kind
            && let PatExprKind::Path(qpath) = &expr.kind
        {
            self.record_qpath(qpath, expr.hir_id);
        }
        rustc_hir::intravisit::walk_pat(self, pat);
    }
}

/// Returns the def-path of `LocalDefId` as a `String`, e.g.
/// `crate::tui::panes::cpu::cpu_required_pane_height`.
pub(super) fn def_path_string(tcx: TyCtxt<'_>, def_id: LocalDefId) -> String {
    tcx.def_path_str(def_id.to_def_id())
}

/// Returns the def-path of the parent module of `def_id`. For a function
/// in `crate::tui::panes::cpu`, returns `crate::tui::panes::cpu`. Used
/// when synthesizing the proposed narrower scope for a `pub(super)`
/// suggestion.
pub(super) fn parent_module_def_path(tcx: TyCtxt<'_>, def_id: LocalDefId) -> String {
    let parent = tcx.parent_module_from_def_id(def_id);
    tcx.def_path_str(parent.to_def_id())
}

pub(super) fn parent_module_path_segments(tcx: TyCtxt<'_>, def_id: LocalDefId) -> Vec<String> {
    let mut segments = parent_module_def_path(tcx, def_id)
        .split("::")
        .filter(|segment| !segment.is_empty())
        .map(String::from)
        .collect::<Vec<_>>();
    if segments.first().is_some_and(|segment| segment == "crate") {
        segments.remove(0);
    }
    segments
}
