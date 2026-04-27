use ratatui::layout::Constraint;
use ratatui::layout::Rect;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaneAxisSize {
    Fixed(u16),
    Fill(u16),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaneSizeSpec {
    pub width:  PaneAxisSize,
    pub height: PaneAxisSize,
}

impl PaneSizeSpec {
    pub const fn fill() -> Self {
        Self {
            width:  PaneAxisSize::Fill(1),
            height: PaneAxisSize::Fill(1),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PanePlacement<Id> {
    pub pane:     Id,
    pub row:      usize,
    pub col:      usize,
    pub row_span: usize,
    pub col_span: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaneGridLayout<Id> {
    pub placements: Vec<PanePlacement<Id>>,
}

impl<Id: Copy> PaneGridLayout<Id> {
    pub fn tab_order(self) -> Vec<Id> {
        let mut placements = self.placements;
        placements.sort_by_key(|placement| (placement.row, placement.col));
        placements
            .into_iter()
            .map(|placement| placement.pane)
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolvedPane<Id> {
    pub pane: Id,
    pub area: Rect,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResolvedPaneLayout<Id> {
    panes: Vec<ResolvedPane<Id>>,
}

impl<Id> ResolvedPaneLayout<Id> {
    pub const fn new(panes: Vec<ResolvedPane<Id>>) -> Self { Self { panes } }

    pub fn panes(&self) -> &[ResolvedPane<Id>] { &self.panes }
}

#[cfg(test)]
impl<Id: Copy + Eq> ResolvedPaneLayout<Id> {
    pub fn area(&self, pane: Id) -> Rect {
        self.panes
            .iter()
            .find(|resolved| resolved.pane == pane)
            .map_or(Rect::ZERO, |resolved| resolved.area)
    }
}

pub fn constraints_for_sizes(sizes: &[PaneAxisSize]) -> Vec<Constraint> {
    sizes
        .iter()
        .map(|size| match size {
            PaneAxisSize::Fixed(length) => Constraint::Length(*length),
            PaneAxisSize::Fill(weight) => Constraint::Fill(*weight),
        })
        .collect()
}
