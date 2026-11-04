//! Expansion of the handler forest.

use std::collections::HashSet;

use tui_pane::ExpansionAction;
use tui_pane::ExpansionModel;
use tui_pane::ExpansionTree;
use tui_pane::RowExpansion;
use tui_pane::RowParent;

use super::App;
use crate::agent_cell;
use crate::census;
use crate::census::AgentPlacement;
use crate::interaction::RowIdentity as SelectedRow;
use crate::interaction::SelectableSurface;
use crate::interaction::SelectionSurface;
use crate::interaction::SurfaceLookup;
use crate::interaction::TileSelection;
use crate::tiles::TileContent;

/// Owned process identity for expansion choices.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct RowIdentity {
    machine: String,
    pid:     u32,
}

/// Stable identity for a process row or work without a process.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) enum ExpansionKey {
    Process(RowIdentity),
    Work(SelectedRow),
}

impl ExpansionKey {
    pub(crate) fn process(machine: &str, pid: u32) -> Self {
        Self::Process(RowIdentity {
            machine: machine.to_owned(),
            pid,
        })
    }
}

/// The marker for a visible row's children.
#[derive(Clone, Copy, Debug)]
pub(crate) enum TreeMarker {
    Leaf,
    Open,
    Closed,
}

/// Visibility and marker after applying all ancestor choices.
pub(crate) enum RowVisibility {
    Hidden,
    Visible(TreeMarker),
}

impl RowVisibility {
    const fn is_visible(&self) -> bool { matches!(self, Self::Visible(_)) }
}

/// Whether an ancestor permits its descendants to appear.
#[derive(Clone, Copy)]
enum DescendantVisibility {
    Shown,
    Hidden,
}

/// Expansion choices for the current census.
#[derive(Clone, Debug)]
pub(crate) struct HandlerExpansion(ExpansionModel<ExpansionKey>);

impl Default for HandlerExpansion {
    fn default() -> Self { Self(ExpansionModel::new(|depth| depth < 1)) }
}

impl HandlerExpansion {
    fn is_open(&self, key: &ExpansionKey, depth: usize) -> bool { self.0.is_open(key, depth) }

    pub(super) fn retain(&mut self, placement: &AgentPlacement<'_>) {
        let tiles = agent_cell::tile_order_for_expansion(placement);
        let keys: HashSet<_> = tiles
            .iter()
            .flat_map(|tile| tile.expansion_tree().rows.into_iter())
            .map(|row| row.key)
            .collect();
        self.0.retain(|key| keys.contains(key));
    }
}

/// A complete row before closed ancestors hide it.
pub(crate) struct HandlerTreeRow {
    pub(crate) identity: SelectedRow,
    pub(crate) key:      ExpansionKey,
    pub(crate) depth:    usize,
    pub(crate) parent:   RowParent<usize>,
}

/// A complete depth-first forest with explicit parent relationships.
pub(crate) struct HandlerTree {
    rows: Vec<HandlerTreeRow>,
}

impl HandlerTree {
    pub(crate) const fn new(rows: Vec<HandlerTreeRow>) -> Self { Self { rows } }

    fn row_expansion_ref(&self, index: usize) -> RowExpansion<&ExpansionKey> {
        let row = &self.rows[index];
        if self
            .rows
            .get(index + 1)
            .is_some_and(|next| next.depth > row.depth)
        {
            RowExpansion::Branch {
                key:   &row.key,
                depth: row.depth,
            }
        } else {
            RowExpansion::Leaf
        }
    }

    fn row_expansion_at(&self, index: usize) -> RowExpansion<ExpansionKey> {
        match self.row_expansion_ref(index) {
            RowExpansion::Leaf => RowExpansion::Leaf,
            RowExpansion::Branch { key, depth } => RowExpansion::Branch {
                key: key.clone(),
                depth,
            },
        }
    }

    /// Visit each row once and borrow its expansion key.
    pub(crate) fn visibility<'a>(
        &'a self,
        expansion: &'a HandlerExpansion,
    ) -> impl Iterator<Item = RowVisibility> + 'a {
        let mut ancestors = Vec::new();
        self.rows.iter().enumerate().map(move |(index, row)| {
            ancestors.truncate(row.depth);
            if matches!(ancestors.last(), Some(DescendantVisibility::Hidden)) {
                ancestors.push(DescendantVisibility::Hidden);
                return RowVisibility::Hidden;
            }
            let marker = match self.row_expansion_ref(index) {
                RowExpansion::Leaf => TreeMarker::Leaf,
                RowExpansion::Branch { key, depth } if expansion.is_open(key, depth) => {
                    TreeMarker::Open
                },
                RowExpansion::Branch { .. } => TreeMarker::Closed,
            };
            ancestors.push(match marker {
                TreeMarker::Closed => DescendantVisibility::Hidden,
                TreeMarker::Leaf | TreeMarker::Open => DescendantVisibility::Shown,
            });
            RowVisibility::Visible(marker)
        })
    }
}

impl ExpansionTree for HandlerTree {
    type Key = ExpansionKey;
    type Row = usize;

    fn row_expansion(&self, row: &usize) -> RowExpansion<ExpansionKey> {
        self.row_expansion_at(*row)
    }
    fn parent(&self, row: &usize) -> RowParent<usize> { self.rows[*row].parent.clone() }
    fn expandable_rows(&self) -> impl Iterator<Item = (ExpansionKey, usize)> {
        (0..self.rows.len()).filter_map(|index| match self.row_expansion_at(index) {
            RowExpansion::Branch { key, depth } => Some((key, depth)),
            RowExpansion::Leaf => None,
        })
    }
}

/// A tree operation requested by the user.
#[derive(Clone, Copy)]
pub(crate) enum HandlerExpansionAction {
    Shared(ExpansionAction),
    Toggle,
    Reset,
}

/// Change the forest and retain the nearest visible selected ancestor.
pub(crate) fn dispatch(app: &mut App, action: HandlerExpansionAction) {
    let growth = app.loaded_config.config.tiles.growth();
    let focused = app
        .tiles
        .placements(app.selection_area, growth)
        .into_iter()
        .find(|placement| placement.frame.is_focused());
    let surface = match focused.map(|placement| placement.content) {
        Some(TileContent::Group(key)) => SelectableSurface::Surface(SelectionSurface::Tile(key)),
        Some(TileContent::Summary) => SelectableSurface::Surface(SelectionSurface::Summary),
        _ => SelectableSurface::EmptyCell,
    };
    let selected = match &surface {
        SelectableSurface::Surface(surface) => app.selections.surface(surface).map_or(
            TileSelection::NoRows,
            super::super::interaction::SurfaceSelection::selection,
        ),
        SelectableSurface::EmptyCell => TileSelection::NoRows,
    };
    let machines = app
        .census
        .machines(&app.loaded_config.config.machines.remote);
    let placement = AgentPlacement::from_machines(&machines);
    let agents = agent_cell::cell_order_placed(&placement);
    let tiles = agent_cell::group_tiles(&placement, &agents, &app.tree_sorts, census::unix_now());
    let mut rows = Vec::new();
    for tile in tiles {
        let offset = rows.len();
        for mut row in tile.expansion_tree().rows {
            if let RowParent::Parent(parent) = &mut row.parent {
                *parent += offset;
            }
            rows.push(row);
        }
    }
    {
        let tree = HandlerTree::new(rows);
        let selected_index = tree.rows.iter().position(
            |row| matches!(&selected, TileSelection::Row(identity) if identity == &row.identity),
        );
        match action {
            HandlerExpansionAction::Shared(ExpansionAction::ExpandAll) => {
                app.expansion.0.expand_all(&tree);
            },
            HandlerExpansionAction::Shared(ExpansionAction::CollapseAll) => {
                app.expansion.0.collapse_all(&tree);
            },
            HandlerExpansionAction::Reset => app.expansion.0.reset(),
            _ => {},
        }
        if let Some(mut index) = selected_index {
            match action {
                HandlerExpansionAction::Shared(ExpansionAction::CollapseRow) => {
                    index = app.expansion.0.collapse_row(&tree, index).selected;
                },
                HandlerExpansionAction::Shared(ExpansionAction::ExpandRow) => {
                    if let RowExpansion::Branch { key, depth } = tree.row_expansion_at(index) {
                        app.expansion.0.expand(key, depth);
                    }
                },
                HandlerExpansionAction::Toggle => {
                    if let RowExpansion::Branch { key, depth } = tree.row_expansion_at(index) {
                        app.expansion.0.toggle(key, depth);
                    }
                },
                _ => {},
            }
            let visibility: Vec<_> = tree.visibility(&app.expansion).collect();
            while !visibility[index].is_visible() {
                match tree.parent(&index) {
                    RowParent::Parent(parent) => index = parent,
                    RowParent::Root => break,
                }
            }
            if let SelectableSurface::Surface(surface) = &surface
                && let SurfaceLookup::Current(state) = app.selections.surface_mut(surface)
            {
                state.select(&tree.rows[index].identity);
            }
        }
    }
    app.invalidate_body();
}

#[cfg(test)]
#[path = "expansion_tests.rs"]
mod tests;
