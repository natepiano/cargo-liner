use std::path::Path;

use ratatui::Frame;
use ratatui::layout::Position;
use ratatui::layout::Rect;
use tui_pane::Hittable;
use tui_pane::PaneFrameChrome;
use tui_pane::RenderFocus;
use tui_pane::Renderable;
use tui_pane::Viewport;

use super::data;
use super::data::TargetEntry;
use crate::project::Visibility;
use crate::tui::hit_test::HoverTarget;
use crate::tui::panes::PaneId;
use crate::tui::panes::ProjectPresentation;
use crate::tui::panes::TargetsData;
use crate::tui::project_list::ProjectPathLookup;
use crate::tui::project_list_state::ProjectList;
use crate::tui::render_context::PaneRenderCtx;

/// A target can launch only while its owning package remains visible.
pub enum TargetAvailability {
    Available,
    HeldOwnerDeleted,
}

impl TargetAvailability {
    pub fn resolve(package_path: &Path, project_path: &Path, projects: &ProjectList) -> Self {
        let owner_is_live = matches!(
            projects.at_path(package_path),
            ProjectPathLookup::Found(owner) if owner.visibility == Visibility::Visible
        );
        let selection_is_live = matches!(
            projects.at_path(project_path),
            ProjectPathLookup::Found(project) if project.visibility == Visibility::Visible
        );
        if owner_is_live && selection_is_live {
            Self::Available
        } else {
            Self::HeldOwnerDeleted
        }
    }
}

// `TargetsPane` owns target content, viewport, focus and rendered-row hit-test geometry.
pub struct TargetsPane {
    pub viewport:  Viewport,
    pub focus:     RenderFocus,
    content:       Option<TargetsData>,
    /// Per-rendered-row hit-test geometry.
    row_rects:     Vec<(Rect, usize)>,
    drawn_targets: Vec<TargetEntry>,
}

impl TargetsPane {
    pub const fn new() -> Self {
        Self {
            viewport:      Viewport::new(),
            focus:         RenderFocus::inactive(),
            content:       None,
            row_rects:     Vec::new(),
            drawn_targets: Vec::new(),
        }
    }

    pub const fn content(&self) -> Option<&TargetsData> { self.content.as_ref() }

    pub fn available_content(&self, projects: &ProjectList) -> Option<TargetsData> {
        let data = self.content()?;
        let available = |entries: &[TargetEntry]| {
            entries
                .iter()
                .filter_map(|entry| {
                    match TargetAvailability::resolve(
                        entry.package_path.as_path(),
                        entry.project_path.as_path(),
                        projects,
                    ) {
                        TargetAvailability::Available => Some(entry.clone()),
                        TargetAvailability::HeldOwnerDeleted => None,
                    }
                })
                .collect()
        };
        Some(TargetsData {
            binaries: available(&data.binaries),
            examples: available(&data.examples),
            benches:  available(&data.benches),
        })
    }

    pub fn has_available_targets(&self, projects: &ProjectList) -> bool {
        self.content().is_some_and(|data| {
            data.binaries
                .iter()
                .chain(&data.examples)
                .chain(&data.benches)
                .any(|entry| {
                    matches!(
                        TargetAvailability::resolve(
                            entry.package_path.as_path(),
                            entry.project_path.as_path(),
                            projects,
                        ),
                        TargetAvailability::Available
                    )
                })
        })
    }

    pub fn selected_available_target(&self, projects: &ProjectList) -> Option<TargetEntry> {
        let candidate = self.drawn_targets.get(self.viewport.pos())?;
        match TargetAvailability::resolve(
            candidate.package_path.as_path(),
            candidate.project_path.as_path(),
            projects,
        ) {
            TargetAvailability::Available => Some(candidate.clone()),
            TargetAvailability::HeldOwnerDeleted => None,
        }
    }

    pub fn set_content(&mut self, data: TargetsData) {
        self.content = Some(data);
        self.drawn_targets.clear();
    }

    pub fn clear_content(&mut self) {
        self.content = None;
        self.drawn_targets.clear();
        self.clear_row_rects();
    }

    pub fn set_row_rects(&mut self, rects: Vec<(Rect, usize)>) { self.row_rects = rects; }

    pub fn clear_row_rects(&mut self) { self.row_rects.clear(); }
}

impl Hittable<HoverTarget> for TargetsPane {
    fn hit_test_at(&self, pos: Position) -> Option<HoverTarget> {
        let (_rect, row) = self
            .row_rects
            .iter()
            .find(|(rect, _)| rect.contains(pos))
            .copied()?;
        Some(HoverTarget::PaneRow {
            pane: PaneId::Targets,
            row,
        })
    }
}

impl Renderable<PaneRenderCtx<'_>> for TargetsPane {
    fn render(
        &mut self,
        frame: &mut Frame<'_>,
        area: Rect,
        ctx: &PaneRenderCtx<'_>,
    ) -> Option<PaneFrameChrome> {
        let data = self.available_content(ctx.project_list).unwrap_or_default();
        self.drawn_targets = data::build_target_list_from_data(&data);
        let mut chrome = super::render_targets_pane_body(frame, area, self, &data, ctx.color_mode);
        if ctx.project_presentation() == ProjectPresentation::Deleted {
            chrome.title = " Targets — deleted ".to_string();
        }
        Some(chrome)
    }
}
