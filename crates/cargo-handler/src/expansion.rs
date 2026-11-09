//! Expansion of the handler forest.

use std::collections::HashMap;
use std::collections::HashSet;

use tui_pane::AttractGridPresentation;
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
use crate::tiles::TileKey;

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

/// A digit depth that cannot exceed nine.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DepthDigit {
    Zero,
    One,
    Two,
    Three,
    Four,
    Five,
    Six,
    Seven,
    Eight,
    Nine,
}

impl DepthDigit {
    const fn depth(self) -> usize {
        match self {
            Self::Zero => 0,
            Self::One => 1,
            Self::Two => 2,
            Self::Three => 3,
            Self::Four => 4,
            Self::Five => 5,
            Self::Six => 6,
            Self::Seven => 7,
            Self::Eight => 8,
            Self::Nine => 9,
        }
    }
}

/// The tile's default expansion before individual row choices.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DepthLevel {
    Through(DepthDigit),
    All,
}

impl Default for DepthLevel {
    fn default() -> Self { Self::Through(DepthDigit::One) }
}

impl DepthLevel {
    /// Return 0 (open) or 1 (closed) for `HandlerExpansion::choices`, an `ExpansionModel`.
    /// `HandlerExpansion::default` builds that model with the rule `depth < 1`.
    const fn model_depth(self, depth: usize) -> usize {
        match self {
            Self::All => 0,
            Self::Through(digit) if depth < digit.depth() => 0,
            Self::Through(_) => 1,
        }
    }
}

/// Row choices follow their identities; depth levels belong to tiles.
#[derive(Clone, Debug)]
pub(crate) struct HandlerExpansion {
    choices: ExpansionModel<ExpansionKey>,
    levels:  HashMap<TileKey, DepthLevel>,
}

impl Default for HandlerExpansion {
    fn default() -> Self {
        Self {
            choices: ExpansionModel::new(|depth| depth < 1),
            levels:  HashMap::new(),
        }
    }
}

impl HandlerExpansion {
    fn is_open(&self, key: &ExpansionKey, depth: usize) -> bool { self.choices.is_open(key, depth) }

    pub(crate) fn level(&self, tile: &TileKey) -> DepthLevel {
        self.levels.get(tile).copied().unwrap_or_default()
    }

    fn set_level(&mut self, tile: TileKey, tree: &HandlerTree, level: DepthLevel) {
        let keys: HashSet<_> = tree.rows.iter().map(|row| &row.key).collect();
        self.choices.retain(|key| !keys.contains(key));
        self.levels.insert(tile, level);
    }

    pub(super) fn retain(&mut self, placement: &AgentPlacement<'_>) {
        let tiles = agent_cell::tile_order_for_expansion(placement);
        let keys: HashSet<_> = tiles
            .iter()
            .flat_map(|tile| tile.expansion_tree().rows.into_iter())
            .map(|row| row.key)
            .collect();
        self.choices.retain(|key| keys.contains(key));
        self.levels
            .retain(|key, _| tiles.iter().any(|tile| &tile.key() == key));
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
    rows:  Vec<HandlerTreeRow>,
    level: DepthLevel,
}

impl HandlerTree {
    pub(crate) fn new(rows: Vec<HandlerTreeRow>) -> Self {
        Self {
            rows,
            level: DepthLevel::default(),
        }
    }

    pub(crate) const fn with_level(mut self, level: DepthLevel) -> Self {
        self.level = level;
        self
    }

    fn row_expansion_ref(&self, index: usize) -> RowExpansion<&ExpansionKey> {
        let row = &self.rows[index];
        if self
            .rows
            .get(index + 1)
            .is_some_and(|next| next.depth > row.depth)
        {
            RowExpansion::Branch {
                key:   &row.key,
                depth: self.level.model_depth(row.depth),
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
#[derive(Clone)]
pub(crate) enum HandlerExpansionAction {
    Shared(ExpansionAction),
    Toggle,
    Press(SelectedRow),
    Level(DepthDigit),
}

/// Change the forest and retain the nearest visible selected ancestor.
pub(crate) fn dispatch(app: &mut App, action: HandlerExpansionAction) {
    let surface = focused_surface(app);
    let (action, selected) = match action {
        HandlerExpansionAction::Press(identity) => {
            (HandlerExpansionAction::Toggle, TileSelection::Row(identity))
        },
        action => (
            action,
            match &surface {
                SelectableSurface::Surface(surface) => app.selections.surface(surface).map_or(
                    TileSelection::NoRows,
                    super::super::interaction::SurfaceSelection::selection,
                ),
                SelectableSurface::EmptyCell => TileSelection::NoRows,
            },
        ),
    };
    let machines = app
        .census
        .machines(&app.loaded_config.config.machines.remote);
    let placement = AgentPlacement::from_machines(&machines);
    let agents = agent_cell::cell_order_placed(&placement);
    let tiles = agent_cell::group_tiles(&placement, &agents, &app.tree_sorts, census::unix_now());
    for tile in tiles {
        let key = tile.key();
        let tree = tile.expansion_tree();
        match action {
            HandlerExpansionAction::Shared(ExpansionAction::ExpandAll) => {
                app.expansion.set_level(key.clone(), &tree, DepthLevel::All);
            },
            HandlerExpansionAction::Shared(ExpansionAction::CollapseAll) => {
                app.expansion
                    .set_level(key.clone(), &tree, DepthLevel::default());
            },
            HandlerExpansionAction::Level(digit) if matches!(&surface, SelectableSurface::Surface(SelectionSurface::Tile(focused)) if focused == &key) =>
            {
                app.expansion
                    .set_level(key.clone(), &tree, DepthLevel::Through(digit));
            },
            _ => {},
        }
        let tree = tree.with_level(app.expansion.level(&key));
        if let TileSelection::Row(identity) = &selected
            && let Some(index) = tree.rows.iter().position(|row| &row.identity == identity)
        {
            let index = change_selected_row(&mut app.expansion, &tree, index, &action);
            if let SelectableSurface::Surface(surface) = &surface
                && let SurfaceLookup::Current(state) = app.selections.surface_mut(surface)
            {
                state.select(&tree.rows[index].identity);
            }
        }
    }
    app.invalidate_body();
}

fn focused_surface(app: &App) -> SelectableSurface {
    if matches!(
        app.attract.configuration().grid_presentation(),
        AttractGridPresentation::ReplacesGrid
    ) {
        return SelectableSurface::EmptyCell;
    }
    let growth = app.loaded_config.config.tiles.growth();
    let focused = app
        .tiles
        .placements(app.selection_area, growth)
        .into_iter()
        .find(|placement| placement.frame.is_focused());
    match focused.map(|placement| placement.content) {
        Some(TileContent::Group(key)) => SelectableSurface::Surface(SelectionSurface::Tile(key)),
        Some(TileContent::Summary) => SelectableSurface::Surface(SelectionSurface::Summary),
        _ => SelectableSurface::EmptyCell,
    }
}

fn change_selected_row(
    expansion: &mut HandlerExpansion,
    tree: &HandlerTree,
    mut index: usize,
    action: &HandlerExpansionAction,
) -> usize {
    match action {
        HandlerExpansionAction::Shared(ExpansionAction::CollapseRow) => {
            index = expansion.choices.collapse_row(tree, index).selected;
        },
        HandlerExpansionAction::Shared(ExpansionAction::ExpandRow) => {
            if let RowExpansion::Branch { key, depth } = tree.row_expansion_at(index) {
                expansion.choices.expand(key, depth);
            }
        },
        HandlerExpansionAction::Toggle | HandlerExpansionAction::Press(_) => {
            expansion.choices.press_row(tree, &index);
        },
        _ => {},
    }
    let visibility: Vec<_> = tree.visibility(expansion).collect();
    while !visibility[index].is_visible() {
        match tree.parent(&index) {
            RowParent::Parent(parent) => index = parent,
            RowParent::Root => break,
        }
    }
    index
}

#[cfg(test)]
#[path = "expansion_tests.rs"]
mod tests;
