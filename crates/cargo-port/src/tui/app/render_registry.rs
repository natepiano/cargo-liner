use tui_pane::PaneRegistry;

use super::RenderRegistry;
use crate::tui::panes::PaneId;
use crate::tui::render_context::PaneRenderCtx;

impl PaneRegistry for RenderRegistry<'_> {
    type Ctx<'ctx> = PaneRenderCtx<'ctx>;
    type PaneId = PaneId;

    fn pane_mut(
        &mut self,
        id: Self::PaneId,
    ) -> Option<&mut dyn for<'ctx> tui_pane::Renderable<Self::Ctx<'ctx>>> {
        let pane: &mut dyn for<'ctx> tui_pane::Renderable<Self::Ctx<'ctx>> = match id {
            PaneId::Package => self.package,
            PaneId::Lang => self.lang,
            PaneId::Cpu => self.cpu,
            PaneId::Git => self.git,
            PaneId::Targets => self.targets,
            PaneId::ProjectList => self.project_list,
            PaneId::Output => self.output,
            PaneId::Lints => self.lint,
            PaneId::CiRuns => self.ci,
            PaneId::Settings => self.settings_pane,
            PaneId::Keymap | PaneId::Toasts | PaneId::Finder | PaneId::Sccache => return None,
        };
        Some(pane)
    }
}
