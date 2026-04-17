use ratatui::layout::Constraint;
use ratatui::layout::Rect;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in super::super) enum PaneAxisSize {
    Fixed(u16),
    Fill(u16),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in super::super) struct PaneSizeSpec {
    pub(in super::super) width:  PaneAxisSize,
    pub(in super::super) height: PaneAxisSize,
}

impl PaneSizeSpec {
    pub(in super::super) const fn fill() -> Self {
        Self {
            width:  PaneAxisSize::Fill(1),
            height: PaneAxisSize::Fill(1),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in super::super) struct PanePlacement<Id> {
    pub(in super::super) pane:     Id,
    pub(in super::super) row:      usize,
    pub(in super::super) col:      usize,
    pub(in super::super) row_span: usize,
    pub(in super::super) col_span: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in super::super) struct PaneGridLayout<Id> {
    pub(in super::super) placements: Vec<PanePlacement<Id>>,
}

impl<Id: Copy> PaneGridLayout<Id> {
    pub(in super::super) fn tab_order(self) -> Vec<Id> {
        let mut placements = self.placements;
        placements.sort_by_key(|placement| (placement.row, placement.col));
        placements
            .into_iter()
            .map(|placement| placement.pane)
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in super::super) struct ResolvedPane<Id> {
    pub(in super::super) pane: Id,
    pub(in super::super) area: Rect,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in super::super) struct ResolvedPaneLayout<Id> {
    panes: Vec<ResolvedPane<Id>>,
}

impl<Id> ResolvedPaneLayout<Id> {
    pub(in super::super) const fn new(panes: Vec<ResolvedPane<Id>>) -> Self { Self { panes } }

    pub(in super::super) fn panes(&self) -> &[ResolvedPane<Id>] { &self.panes }
}

#[cfg(test)]
impl<Id: Copy + Eq> ResolvedPaneLayout<Id> {
    pub(in super::super) fn area(&self, pane: Id) -> Rect {
        self.panes
            .iter()
            .find(|resolved| resolved.pane == pane)
            .map_or(Rect::ZERO, |resolved| resolved.area)
    }
}

pub(in super::super) fn constraints_for_sizes(sizes: &[PaneAxisSize]) -> Vec<Constraint> {
    sizes
        .iter()
        .map(|size| match size {
            PaneAxisSize::Fixed(length) => Constraint::Length(*length),
            PaneAxisSize::Fill(weight) => Constraint::Fill(*weight),
        })
        .collect()
}
