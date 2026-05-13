use ratatui::layout::Position;
use tui_pane::FrameworkOverlayId;
use tui_pane::Viewport;

use super::app::App;
use super::app::DismissTarget;
use super::app::HoveredPaneRow;
use super::pane::HITTABLE_Z_ORDER;
use super::pane::Hittable;
use super::pane::HittableId;
use super::pane::HoverTarget;
use super::panes::PaneId;

pub(super) fn handle_click(app: &mut App, pos: Position) -> bool {
    let Some(hit) = hit_test_at(app, pos) else {
        return false;
    };
    match hit {
        HoverTarget::PaneRow { pane, row } => {
            app.set_focus_to_pane(pane);
            if pane == PaneId::ProjectList {
                app.project_list.set_cursor(row);
            } else {
                set_pane_pos(app, pane, row);
            }
            true
        },
        HoverTarget::Dismiss(target) => {
            app.dismiss(target);
            true
        },
        HoverTarget::ToastCard(id) => {
            let active = app.framework.toasts.active_now();
            if let Some(index) = active.iter().position(|toast| toast.id() == id) {
                app.framework.toasts.viewport.set_pos(index);
                app.set_focus_to_pane(PaneId::Toasts);
            }
            true
        },
    }
}

pub(super) fn hovered_pane_row_at(app: &App, pos: Position) -> Option<HoveredPaneRow> {
    match hit_test_at(app, pos)? {
        HoverTarget::PaneRow { pane, row } => Some(HoveredPaneRow { pane, row }),
        HoverTarget::Dismiss(_) | HoverTarget::ToastCard(_) => None,
    }
}

/// Walk `HITTABLE_Z_ORDER` top-to-bottom and return the first pane's
/// `hit_test_at` answer. Lives at App-level so per-arm reach can
/// resolve to whichever owner holds the pane.
pub(super) fn hit_test_at(app: &App, pos: Position) -> Option<HoverTarget> {
    if app.overlays.is_finder_open() {
        return app.overlays.finder_pane.hit_test_at(pos);
    }
    match app.framework.overlay() {
        Some(FrameworkOverlayId::Settings) => return app.framework.settings_pane.hit_test_at(pos),
        Some(FrameworkOverlayId::Keymap) => return app.framework.keymap_pane.hit_test_at(pos),
        None => {},
    }

    for id in HITTABLE_Z_ORDER {
        if id == HittableId::Toasts {
            if let Some(target) = hit_test_toasts(app, pos) {
                return Some(target);
            }
            continue;
        }
        let pane: &dyn Hittable = match id {
            HittableId::Toasts => continue,
            HittableId::Finder => &app.overlays.finder_pane,
            HittableId::Settings => &app.framework.settings_pane,
            HittableId::Keymap => &app.framework.keymap_pane,
            HittableId::ProjectList => &app.panes.project_list,
            HittableId::Package => &app.panes.package,
            HittableId::Lang => &app.panes.lang,
            HittableId::Cpu => &app.panes.cpu,
            HittableId::Git => &app.panes.git,
            HittableId::Targets => &app.panes.targets,
            HittableId::Lints => &app.lint,
            HittableId::CiRuns => &app.ci,
        };
        if let Some(hit) = pane.hit_test_at(pos) {
            return Some(hit);
        }
    }
    None
}

fn hit_test_toasts(app: &App, pos: Position) -> Option<HoverTarget> {
    for hit in app.framework.toasts.hits().iter().rev() {
        if hit.close_rect.contains(pos) {
            return Some(HoverTarget::Dismiss(DismissTarget::Toast(hit.id)));
        }
        if hit.card_rect.contains(pos) {
            return Some(HoverTarget::ToastCard(hit.id));
        }
    }
    None
}

/// Set the cursor position for `id`'s viewport. Matches by `PaneId`
/// to whichever owner holds the target viewport. `ProjectList`'s
/// cursor lives on `Selection.cursor`; callers route through
/// `app.project_list.set_cursor(row)`, not this fn.
pub(super) const fn set_pane_pos(app: &mut App, id: PaneId, row: usize) {
    match id {
        PaneId::ProjectList => {},
        PaneId::Toasts => app.framework.toasts.viewport.set_pos(row),
        PaneId::Keymap => app.framework.keymap_pane.viewport_mut().set_pos(row),
        PaneId::Settings => app.framework.settings_pane.viewport_mut().set_pos(row),
        _ => {
            if let Some(viewport) = viewport_mut_for(app, id) {
                viewport.set_pos(row);
            }
        },
    }
}

/// Mutable viewport accessor by `PaneId`.
pub(super) const fn viewport_mut_for(app: &mut App, id: PaneId) -> Option<&mut Viewport> {
    let viewport = match id {
        PaneId::Cpu => &mut app.panes.cpu.viewport,
        PaneId::Lang => &mut app.panes.lang.viewport,
        PaneId::Lints => &mut app.lint.viewport,
        PaneId::CiRuns => &mut app.ci.viewport,
        PaneId::Package => &mut app.panes.package.viewport,
        PaneId::Git => &mut app.panes.git.viewport,
        PaneId::Finder => &mut app.overlays.finder_pane.viewport,
        PaneId::Output => &mut app.panes.output.viewport,
        PaneId::Targets => &mut app.panes.targets.viewport,
        PaneId::ProjectList => &mut app.panes.project_list.viewport,
        PaneId::Keymap | PaneId::Settings | PaneId::Toasts => return None,
    };
    Some(viewport)
}

const fn set_hovered(app: &mut App, pane: PaneId, row: Option<usize>) {
    match pane {
        PaneId::Toasts => app.framework.toasts.viewport.set_hovered(row),
        PaneId::Keymap => app.framework.keymap_pane.viewport_mut().set_hovered(row),
        PaneId::Settings => app.framework.settings_pane.viewport_mut().set_hovered(row),
        _ => {
            if let Some(viewport) = viewport_mut_for(app, pane) {
                viewport.set_hovered(row);
            }
        },
    }
}

/// Push the current `hovered_pane_row` into the per-pane viewports.
/// Clears any prior hover across every pane first, then sets the row
/// on the pane indicated by `hovered_pane_row` (if any).
pub(super) const fn apply_hovered_pane_row(app: &mut App) {
    clear_all_hover(app);
    if let Some(hovered) = app.panes.hovered_row() {
        set_hovered(app, hovered.pane, Some(hovered.row));
    }
}

const fn clear_all_hover(app: &mut App) {
    app.framework.toasts.viewport.set_hovered(None);
    app.ci.viewport.set_hovered(None);
    app.lint.viewport.set_hovered(None);
    app.framework.keymap_pane.viewport_mut().set_hovered(None);
    app.framework.settings_pane.viewport_mut().set_hovered(None);
    app.overlays.finder_pane.viewport.set_hovered(None);
    let panes = &mut app.panes;
    panes.package.viewport.set_hovered(None);
    panes.lang.viewport.set_hovered(None);
    panes.cpu.viewport.set_hovered(None);
    panes.git.viewport.set_hovered(None);
    panes.output.viewport.set_hovered(None);
    panes.targets.viewport.set_hovered(None);
    panes.project_list.viewport.set_hovered(None);
}
