use ratatui::layout::Position;
use tui_pane::FrameworkHit;
use tui_pane::FrameworkOverlayId;
use tui_pane::InputContext;
use tui_pane::ToastHit;
use tui_pane::Viewport;

use super::app::App;
use super::app::HoveredPaneRow;
use super::pane::DismissTarget;
use super::pane::HITTABLE_Z_ORDER;
use super::pane::HitTestRegistry;
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

/// Run the framework's full hit-test ladder for `pos`. Delegates
/// to [`tui_pane::dispatch_hit_test`]; the framework orchestrates
/// toast → framework overlay → app-modal overlay (finder) → tiled
/// z-order through this app's [`InputContext`] impl below.
pub(super) fn hit_test_at(app: &App, pos: Position) -> Option<HoverTarget> {
    tui_pane::dispatch_hit_test(app, pos)
}

impl HitTestRegistry for App {
    type PaneId = HittableId;
    type Target = HoverTarget;

    fn z_order() -> &'static [HittableId] { &HITTABLE_Z_ORDER }

    fn pane(&self, id: HittableId) -> Option<&dyn Hittable<HoverTarget>> {
        Some(match id {
            HittableId::ProjectList => &self.panes.project_list,
            HittableId::Package => &self.panes.package,
            HittableId::Lang => &self.panes.lang,
            HittableId::Cpu => &self.panes.cpu,
            HittableId::Git => &self.panes.git,
            HittableId::Targets => &self.panes.targets,
            HittableId::Lints => &self.lint,
            HittableId::CiRuns => &self.ci,
        })
    }

    fn viewport_mut(&mut self, id: HittableId) -> Option<&mut Viewport> {
        Some(match id {
            HittableId::ProjectList => &mut self.panes.project_list.viewport,
            HittableId::Package => &mut self.panes.package.viewport,
            HittableId::Lang => &mut self.panes.lang.viewport,
            HittableId::Cpu => &mut self.panes.cpu.viewport,
            HittableId::Git => &mut self.panes.git.viewport,
            HittableId::Targets => &mut self.panes.targets.viewport,
            HittableId::Lints => &mut self.lint.viewport,
            HittableId::CiRuns => &mut self.ci.viewport,
        })
    }
}

impl InputContext for App {
    fn framework_hit(&self, pos: Position) -> Option<FrameworkHit> {
        self.framework.hit_test_at(pos)
    }

    fn app_modal_overlay_hit(&self, pos: Position) -> Option<Option<HoverTarget>> {
        if self.overlays.is_finder_open() {
            Some(self.overlays.finder_pane.hit_test_at(pos))
        } else {
            None
        }
    }

    fn map_framework_hit(&self, hit: FrameworkHit) -> Option<HoverTarget> {
        match hit {
            FrameworkHit::Toast(ToastHit::Close(id)) => {
                Some(HoverTarget::Dismiss(DismissTarget::Toast(id)))
            },
            FrameworkHit::Toast(ToastHit::Card(id)) => Some(HoverTarget::ToastCard(id)),
            FrameworkHit::Overlay {
                id: FrameworkOverlayId::Keymap,
                row,
            } => Some(HoverTarget::PaneRow {
                pane: PaneId::Keymap,
                row,
            }),
            FrameworkHit::Overlay {
                id: FrameworkOverlayId::Settings,
                row,
            } => Some(HoverTarget::PaneRow {
                pane: PaneId::Settings,
                row,
            }),
            FrameworkHit::ModalMissed => None,
        }
    }
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
pub(super) fn apply_hovered_pane_row(app: &mut App) {
    clear_all_hover(app);
    if let Some(hovered) = app.panes.hovered_row() {
        set_hovered(app, hovered.pane, Some(hovered.row));
    }
}

fn clear_all_hover(app: &mut App) {
    app.framework.clear_hover();
    app.overlays.finder_pane.viewport.set_hovered(None);
    app.panes.output.viewport.set_hovered(None);
    tui_pane::clear_all_hover(app);
}
