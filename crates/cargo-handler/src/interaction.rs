//! Where a click lands.
//!
//! The framework owns the order a click is offered around — toasts,
//! then any open framework overlay, then whatever the app tiles
//! underneath — so nothing here re-derives it. [`App`] supplies the two
//! app-side pieces that ladder asks for, and
//! [`tui_pane::handle_tile_click`] walks it and acts on the
//! [`TilePick`] that comes back.

use std::collections::HashMap;
use std::ops::Range;

use ratatui::layout::Position;
use ratatui::layout::Rect;
use tui_pane::FrameworkHit;
use tui_pane::HitTestRegistry;
use tui_pane::Hittable;
use tui_pane::InputContext;
use tui_pane::ModalHit;
use tui_pane::NavAction;
use tui_pane::ScrollDirection;
use tui_pane::TilePick;
use tui_pane::Viewport;

use crate::app::App;
use crate::app::AppPaneId;
use crate::census::Agent;
use crate::census::AgentRow;
use crate::census::ChildKind;
use crate::census::SessionIdentity;
use crate::census::transcript::SubagentRole;
use crate::census::worker_dispatch::WorkerIdentity;
use crate::tiles::TileKey;

/// A selectable surface includes the summary as well as census tiles.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) enum SelectionSurface {
    Summary,
    Tile(TileKey),
}

/// Owning process generation of a work row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RowOwner {
    pub(crate) pid:     u32,
    pub(crate) started: u64,
}

/// Legacy work identity excludes changing observations such as a countdown.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum LegacyWorkKind {
    Shell,
    Subagent(SubagentRole),
    Session(Agent),
    UnderShell(Agent),
    Direct(Agent),
    Detached(Agent),
    Thread,
}

impl From<&ChildKind> for LegacyWorkKind {
    fn from(kind: &ChildKind) -> Self {
        match kind {
            ChildKind::Shell(_) => Self::Shell,
            ChildKind::Subagent(role) => Self::Subagent(role.clone()),
            ChildKind::Session(agent) => Self::Session(*agent),
            ChildKind::UnderShell(agent) => Self::UnderShell(*agent),
            ChildKind::Direct(agent) => Self::Direct(*agent),
            ChildKind::Detached(agent) => Self::Detached(*agent),
            ChildKind::Thread => Self::Thread,
        }
    }
}

/// Owned identity of a row, independent of its label, index and current tile.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RowIdentity {
    Session {
        machine: String,
        session: SessionIdentity,
        pid:     u32,
        started: u64,
    },
    Seat {
        machine: String,
        seat:    String,
        worker:  WorkerIdentity,
    },
    Thread {
        machine: String,
        id:      String,
    },
    Subagent {
        machine: String,
        owner:   RowOwner,
        id:      String,
    },
    Shell {
        machine: String,
        pid:     u32,
        started: u64,
    },
    LegacyWork {
        machine: String,
        owner:   RowOwner,
        kind:    LegacyWorkKind,
        started: u64,
        ordinal: usize,
    },
    SummaryAccount {
        account: String,
    },
    SummaryGroup {
        machine: String,
    },
}

impl RowIdentity {
    /// A seat may replace its physical worker while remaining the same selectable seat.
    fn continues(&self, previous: &Self) -> bool {
        match (self, previous) {
            (
                Self::Seat {
                    machine,
                    seat,
                    worker,
                },
                Self::Seat {
                    machine: old_machine,
                    seat: old_seat,
                    worker: old_worker,
                },
            ) => {
                machine == old_machine
                    && seat == old_seat
                    && std::mem::discriminant(worker) == std::mem::discriminant(old_worker)
            },
            _ => self == previous,
        }
    }

    pub(crate) fn session(machine: &str, row: &AgentRow) -> Self {
        Self::Session {
            machine: machine.to_owned(),
            session: row.session_id.clone(),
            pid:     row.pid,
            started: row.process_started,
        }
    }
}

/// Selection is exposed only from a surface resolved against its current rows.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum TileSelection {
    NoRows,
    Row(RowIdentity),
}

/// Whether a pointer selected a resolved row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RowClick {
    Selected,
    Missed,
}

/// A selectable item always occupies its first line, with zero or more continuations.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RowSpan {
    first:              usize,
    continuation_lines: usize,
}

impl RowSpan {
    pub(crate) const fn single(first: usize) -> Self {
        Self {
            first,
            continuation_lines: 0,
        }
    }

    pub(crate) const fn with_continuations(self, continuation_lines: usize) -> Self {
        Self {
            continuation_lines,
            ..self
        }
    }

    const fn start(self) -> usize { self.first }

    /// Exclusive end of the drawn item.
    const fn end(self) -> usize {
        self.first
            .saturating_add(self.continuation_lines)
            .saturating_add(1)
    }

    const fn contains(self, line: usize) -> bool { self.first <= line && line < self.end() }

    /// The span's occupied lines clipped and translated into viewport coordinates.
    pub(crate) fn visible_lines(self, offset: usize, height: usize) -> Range<usize> {
        let start = self.start().saturating_sub(offset).min(height);
        let end = self.end().saturating_sub(offset).min(height);
        start..end
    }

    pub(crate) const fn shifted(self, offset: usize) -> Self {
        Self {
            first: self.first.saturating_add(offset),
            ..self
        }
    }
}

/// Current row membership owns the cursor; callers cannot install an absent row.
#[derive(Clone, Debug, Default)]
pub(crate) struct SurfaceSelection {
    rows:     Vec<(RowIdentity, RowSpan)>,
    viewport: Viewport,
}

impl SurfaceSelection {
    pub(crate) fn selection(&self) -> TileSelection {
        self.rows
            .get(self.viewport.pos())
            .map(|(identity, _)| identity.clone())
            .map_or(TileSelection::NoRows, TileSelection::Row)
    }

    pub(crate) const fn viewport(&self) -> &Viewport { &self.viewport }

    /// Keep an exact identity, otherwise the successor at the old index, then the tail.
    fn resolve(&mut self, rows: Vec<(RowIdentity, RowSpan)>) {
        let old = self.selection();
        let index = match old {
            TileSelection::Row(ref identity) => rows
                .iter()
                .position(|(row, _)| row.continues(identity))
                .unwrap_or_else(|| self.viewport.pos()),
            TileSelection::NoRows => 0,
        };
        self.rows = rows;
        self.viewport.set_len(self.rows.len());
        self.viewport
            .set_pos(index.min(self.rows.len().saturating_sub(1)));
        self.viewport
            .set_content_height(self.rows.last().map_or(0, |(_, span)| span.end()));
        self.keep_visible();
    }

    pub(crate) fn selected_span(&self) -> Option<RowSpan> {
        self.rows.get(self.viewport.pos()).map(|(_, span)| *span)
    }

    fn select(&mut self, identity: &RowIdentity) {
        if let Some(index) = self
            .rows
            .iter()
            .position(|(row, _)| row.continues(identity))
        {
            self.viewport.set_pos(index);
            self.keep_visible();
        }
    }

    pub(crate) fn navigate(&mut self, action: NavAction) {
        if self.scroll_continuation(action) {
            return;
        }
        self.viewport.navigate(action);
        self.keep_visible();
        if let Some((_, span)) = self.rows.get(self.viewport.pos())
            && span.continuation_lines > 0
        {
            match action {
                NavAction::Home
                    if span.end().saturating_sub(span.start()) > self.viewport.visible_rows() =>
                {
                    self.viewport.set_scroll_offset(span.start());
                },
                NavAction::End => self
                    .viewport
                    .set_scroll_offset(span.end().saturating_sub(self.viewport.visible_rows())),
                _ => {},
            }
        }
    }

    /// Traverse an item taller than the viewport before leaving it for the next item.
    fn scroll_continuation(&mut self, action: NavAction) -> bool {
        let visible = self.viewport.visible_rows();
        let Some((_, span)) = self.rows.get(self.viewport.pos()) else {
            return false;
        };
        if visible == 0 || span.end().saturating_sub(span.start()) <= visible {
            return false;
        }
        let offset = self.viewport.scroll_offset();
        let (forward, step) = match action {
            NavAction::Down | NavAction::Right => (true, 1),
            NavAction::Up | NavAction::Left => (false, 1),
            NavAction::PageDown => (true, visible.saturating_sub(1).max(1)),
            NavAction::PageUp => (false, visible.saturating_sub(1).max(1)),
            NavAction::HalfPageDown => (true, (visible / 2).max(1)),
            NavAction::HalfPageUp => (false, (visible / 2).max(1)),
            NavAction::Home | NavAction::End => return false,
        };
        let next = if forward && offset.saturating_add(visible) < span.end() {
            offset
                .saturating_add(step)
                .min(span.end().saturating_sub(visible))
        } else if !forward && offset > span.start() {
            offset.saturating_sub(step).max(span.start())
        } else {
            return false;
        };
        self.viewport.set_scroll_offset(next);
        true
    }

    pub(crate) fn set_area(&mut self, area: Rect) {
        self.viewport.set_content_area(area);
        self.viewport.set_viewport_rows(usize::from(area.height));
        self.keep_visible();
    }

    pub(crate) fn click(&mut self, position: Position) -> RowClick {
        if !self.viewport.content_area().contains(position) {
            return RowClick::Missed;
        }
        let line = usize::from(position.y - self.viewport.content_area().y)
            + self.viewport.scroll_offset();
        if let Some(index) = self.rows.iter().position(|(_, span)| span.contains(line)) {
            self.viewport.set_pos(index);
            self.keep_visible();
            RowClick::Selected
        } else {
            RowClick::Missed
        }
    }

    pub(crate) fn scroll(&mut self, direction: ScrollDirection) {
        self.navigate(match direction {
            ScrollDirection::Up => NavAction::Up,
            ScrollDirection::Down => NavAction::Down,
        });
    }

    fn keep_visible(&mut self) {
        let visible = self.viewport.visible_rows();
        if visible == 0 {
            return;
        }
        let offset = self.viewport.scroll_offset();
        let Some((_, span)) = self.rows.get(self.viewport.pos()) else {
            return;
        };
        let offset = if span.end().saturating_sub(span.start()) > visible {
            // A tall item can remain partially visible while its continuation is scrolled.
            if span.end() <= offset || span.start() >= offset.saturating_add(visible) {
                span.start()
            } else {
                offset
            }
        } else if span.start() < offset {
            span.start()
        } else if span.end() > offset.saturating_add(visible) {
            span.end().saturating_sub(visible)
        } else {
            offset
        };
        self.viewport
            .set_scroll_offset(offset.min(self.viewport.content_height().saturating_sub(visible)));
    }
}

/// Whether this surface keeps its previous selected identity.
enum RowRetention {
    Surviving,
    Vacated,
}

/// All resolved surface selections for the current census.
#[derive(Clone, Debug, Default)]
pub(crate) struct Selections {
    surfaces: HashMap<SelectionSurface, SurfaceSelection>,
}

impl Selections {
    pub(crate) fn containing(&self, identity: &RowIdentity) -> Option<SelectionSurface> {
        self.surfaces
            .iter()
            .find(|(_, state)| state.rows.iter().any(|(row, _)| row.continues(identity)))
            .map(|(surface, _)| surface.clone())
    }

    /// Move the active cursor with its exact row across surfaces.
    /// Background transfers are reconciled separately and cannot displace a survivor.
    pub(crate) fn follow_focused(&mut self, identity: &RowIdentity) -> Option<SelectionSurface> {
        let destination = self.containing(identity)?;
        self.surfaces.get_mut(&destination)?.select(identity);
        Some(destination)
    }

    pub(crate) fn surface(&self, surface: &SelectionSurface) -> Option<&SurfaceSelection> {
        self.surfaces.get(surface)
    }
    pub(crate) fn surface_mut(
        &mut self,
        surface: &SelectionSurface,
    ) -> Option<&mut SurfaceSelection> {
        self.surfaces.get_mut(surface)
    }

    /// Reconcile together so exact identities follow a session between surfaces.
    pub(crate) fn resolve(&mut self, rows: Vec<(SelectionSurface, Vec<(RowIdentity, RowSpan)>)>) {
        let selected: Vec<_> = self
            .surfaces
            .iter()
            .filter_map(|(surface, state)| match state.selection() {
                TileSelection::Row(row) => Some((surface.clone(), row)),
                TileSelection::NoRows => None,
            })
            .collect();
        self.surfaces
            .retain(|surface, _| rows.iter().any(|(current, _)| current == surface));
        for (surface, identities) in rows {
            let incoming = selected.iter().find(|(origin, row)| {
                origin != &surface
                    && identities
                        .iter()
                        .any(|(identity, _)| identity.continues(row))
            });
            let state = self.surfaces.entry(surface).or_default();
            let previous = state.selection();
            let retained = match previous {
                TileSelection::Row(ref row)
                    if identities
                        .iter()
                        .any(|(identity, _)| identity.continues(row)) =>
                {
                    RowRetention::Surviving
                },
                TileSelection::Row(_) | TileSelection::NoRows => RowRetention::Vacated,
            };
            state.resolve(identities);
            if matches!(retained, RowRetention::Vacated)
                && let Some((_, row)) = incoming
            {
                state.select(row);
            }
        }
    }
}

/// Every pane a click can land on, top of the stack first. The tile
/// grid fills the whole body, so there is only the one.
const HIT_TEST_Z_ORDER: [AppPaneId; 1] = [AppPaneId::Main];

impl HitTestRegistry for App {
    type PaneId = AppPaneId;
    type Target = TilePick;

    fn z_order() -> &'static [AppPaneId] { &HIT_TEST_Z_ORDER }

    fn pane(&self, id: AppPaneId) -> Option<&dyn Hittable<TilePick>> {
        match id {
            AppPaneId::Main => Some(&self.tiles),
            // The attract screen has no click behavior, and the
            // favorites modal absorbs clicks before this pane walk.
            AppPaneId::Attract(_) | AppPaneId::Favorites => None,
        }
    }

    /// The grid tracks focus by cell rather than through a viewport, so
    /// there is nothing here for the framework's hover pass to clear.
    fn viewport_mut(&mut self, _: AppPaneId) -> Option<&mut Viewport> { None }
}

impl InputContext for App {
    fn framework_hit(&self, pos: Position) -> Option<FrameworkHit> {
        self.framework.hit_test_at(pos)
    }

    /// The favorites modal has no mouse selection yet, but still
    /// absorbs every click so none reaches the grid underneath it.
    fn app_modal_overlay_hit(&self, _: Position) -> ModalHit<TilePick> {
        if self.favorites_overlay.is_open() {
            ModalHit::MissedRow
        } else {
            ModalHit::Closed
        }
    }

    fn map_framework_hit(&self, hit: FrameworkHit) -> Option<TilePick> {
        TilePick::from_framework_hit(hit)
    }
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::path::PathBuf;
    use std::rc::Rc;

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::layout::Rect;
    use tui_pane::FavoritesFileState;
    use tui_pane::TABLE_CELL;
    use tui_pane::TerminalApp;
    use tui_pane::TileAction;

    use super::*;
    use crate::constants::STATUS_LINE_HEIGHT;
    use crate::render;
    use crate::tiles::TileContent;

    /// Width of the frame the click tests draw.
    const WIDTH: u16 = 80;
    /// Height of the frame the click tests draw.
    const HEIGHT: u16 = 24;

    /// The body the grid is drawn in: every row but the status line.
    const fn body() -> Rect { Rect::new(0, 0, WIDTH, HEIGHT - STATUS_LINE_HEIGHT) }

    /// Draw one frame of `app`, which is what records where the grid's
    /// cells sit for a click to find.
    fn draw_frame(app: &mut App) {
        let keymap = Rc::clone(&app.keymap);
        let mut terminal =
            Terminal::new(TestBackend::new(WIDTH, HEIGHT)).expect("the test terminal opens");
        terminal
            .draw(|frame| render::draw(frame, app, &keymap))
            .expect("the test terminal draws");
    }

    /// An app whose grid holds the summary and one empty cell, settled
    /// and drawn.
    fn two_cell_app() -> App {
        let mut app = App::new_for_test().expect("test app should build");
        draw_frame(&mut app);
        let growth = app.loaded_config.config.tiles.growth();
        app.tiles.apply(TileAction::Add, growth);
        app.tiles.settle_for_test();
        draw_frame(&mut app);
        app
    }

    /// What the cell holding the focus ring shows.
    fn focused(app: &App) -> TileContent {
        let growth = app.loaded_config.config.tiles.growth();
        app.tiles
            .placements(body(), growth)
            .into_iter()
            .find(|placement| placement.frame.is_focused())
            .map(|placement| placement.content)
            .expect("one cell holds the focus ring")
    }

    /// The middle of the cell showing `content`.
    fn middle_of(app: &App, content: &TileContent) -> Position {
        let growth = app.loaded_config.config.tiles.growth();
        let rect = app
            .tiles
            .placements(body(), growth)
            .into_iter()
            .find(|placement| &placement.content == content)
            .map(|placement| placement.frame.rect())
            .expect("the grid draws the cell");
        Position::new(rect.x + rect.width / 2, rect.y + rect.height / 2)
    }

    /// A click on the second cell of a two-cell grid moves the ring
    /// onto it.
    #[test]
    fn a_click_on_the_second_cell_focuses_it() {
        let mut app = two_cell_app();
        assert_eq!(focused(&app), TileContent::Summary);

        let cell = middle_of(&app, &TileContent::Empty(TABLE_CELL + 1));
        app.click(cell);

        assert_eq!(focused(&app), TileContent::Empty(TABLE_CELL + 1));
    }

    /// The open favorites overlay takes a click on the cell under it,
    /// so the ring stays where it was.
    #[test]
    fn an_open_favorites_overlay_absorbs_clicks() {
        let mut app = two_cell_app();
        tui_pane::open_favorites_on_state_for_test(
            &mut app,
            FavoritesFileState::Missing {
                path: PathBuf::from("/tmp/favorites.toml"),
            },
        );
        assert_eq!(
            app.app_modal_overlay_hit(Position::new(0, 0)),
            ModalHit::MissedRow
        );

        let cell = middle_of(&app, &TileContent::Empty(TABLE_CELL + 1));
        app.click(cell);

        assert_eq!(focused(&app), TileContent::Summary);
    }
}
