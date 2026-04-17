use std::marker::PhantomData;

use super::Pane;

pub(in super::super) trait PaneKey: Copy {
    fn index(self) -> usize;
    fn key_count() -> usize;
}

pub(in super::super) struct PaneManager<Id: PaneKey> {
    panes:   Vec<Pane>,
    _marker: PhantomData<Id>,
}

impl<Id: PaneKey> PaneManager<Id> {
    pub(in super::super) fn pane(&self, id: Id) -> &Pane { &self.panes[id.index()] }

    pub(in super::super) fn pane_mut(&mut self, id: Id) -> &mut Pane { &mut self.panes[id.index()] }

    pub(in super::super) fn new() -> Self {
        Self {
            panes:   vec![Pane::new(); Id::key_count()],
            _marker: PhantomData,
        }
    }

    pub(in super::super) fn clear_hover(&mut self) {
        for pane in &mut self.panes {
            pane.set_hovered(None);
        }
    }
}
