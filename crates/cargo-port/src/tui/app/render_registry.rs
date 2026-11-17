use tui_pane::PaneRegistry;

use crate::tui::app_render_state::RenderRegistry;
use crate::tui::panes::PaneId;
use crate::tui::render_context::PaneRenderCtx;
use crate::tui::settings::SettingsRenderInputs;
use crate::tui::settings::SettingsRenderPreparation;

/// The prepared data for framework overlays.
#[derive(Clone, Copy)]
pub(in crate::tui) struct OverlayRenderInputs<'a> {
    pub(super) settings: SettingsRenderPreparation<'a>,
}

impl<'a> OverlayRenderInputs<'a> {
    pub(in crate::tui) const fn none() -> Self {
        Self {
            settings: SettingsRenderPreparation::NotPrepared,
        }
    }

    pub(in crate::tui) const fn settings(inputs: &'a SettingsRenderInputs) -> Self {
        Self {
            settings: SettingsRenderPreparation::Prepared(inputs),
        }
    }
}

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
