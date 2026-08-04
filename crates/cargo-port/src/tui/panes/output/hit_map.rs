//! Which model row each drawn row of the Output pane's monitor stands for.
//!
//! The map is rebuilt every frame by the renderer, so a hit can only ever name
//! something that was actually drawn. The identities it carries are the
//! exec-sensitive ones: a same-PID exec produces a new [`BuildSessionId`] and
//! [`CompileActivityId`], so a hit recorded before the exec matches nothing
//! after it.

use ratatui::layout::Position;
use ratatui::layout::Rect;

use crate::build_monitor::BuildSessionId;
use crate::build_monitor::CompileActivityId;
use crate::tui::OwnedRunId;

/// What the user clicked in the Output pane.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OutputMonitorHit {
    /// A session's header row.
    Header {
        build_session_id: BuildSessionId,
        column_index:     usize,
    },
    /// One activity row under a session's column.
    Activity {
        compile_activity_id: CompileActivityId,
        build_session_id:    BuildSessionId,
        column_index:        usize,
        row_index:           usize,
    },
    /// One row of the scope-level unattributed section.
    Unattributed {
        compile_activity_id: CompileActivityId,
        row_index:           usize,
    },
    /// One row of Cargo Port's own captured output, named together with the run
    /// that produced it so a click can only aim the cursor at an owned column.
    CapturedOutput {
        producer: OwnedRunId,
        row:      usize,
    },
    /// The enabled monitor has no rows; the whole pane is still focusable so
    /// the user can tab to it and see why it is empty.
    EmptyMonitor,
}

/// Every hittable row drawn this frame, in draw order.
#[derive(Debug, Default)]
pub(super) struct MonitorHitMap {
    rows: Vec<MonitorHitRow>,
}

/// One drawn row and the model row it stands for.
#[derive(Debug)]
struct MonitorHitRow {
    area: Rect,
    hit:  OutputMonitorHit,
}

impl MonitorHitMap {
    /// A map with nothing drawn yet.
    pub(super) const fn new() -> Self { Self { rows: Vec::new() } }

    /// Record one drawn region. Rows that carry no identity — the separator,
    /// the section captions — are simply never recorded, so no cursor motion or
    /// click can land on them; nor is a row that fell past the bottom of the
    /// area it was laid out in and was therefore never painted.
    pub(super) fn record(&mut self, hit_region: HitRegion, hit: OutputMonitorHit) {
        let HitRegion::Drawn(area) = hit_region else {
            return;
        };
        self.rows.push(MonitorHitRow { area, hit });
    }

    /// Forget every recorded row, for a frame in which the pane was not drawn
    /// at all. Without this a rect recorded before the pane left the bottom row
    /// would keep answering hits for a pane the user cannot see.
    pub(super) fn clear(&mut self) { self.rows.clear(); }

    /// The model row drawn at `position`. Later rows win, matching the draw
    /// order the user sees.
    pub(super) fn hit_at(&self, position: Position) -> Option<&OutputMonitorHit> {
        self.rows
            .iter()
            .rev()
            .find(|row| row.area.contains(position))
            .map(|row| &row.hit)
    }

    /// Whether anything hittable was drawn.
    pub(super) const fn is_empty(&self) -> bool { self.rows.is_empty() }
}

/// Whether a laid-out row landed inside the area it was drawn in.
///
/// A `Vec<Line>` longer than its `Rect` is truncated by ratatui rather than
/// rejected, so the rows past the bottom are laid out but never painted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum HitRegion {
    /// The row is past the bottom of its area and was not painted.
    Truncated,
    /// The rectangle the row occupies on screen.
    Drawn(Rect),
}

/// The one-row rectangle `line_index` rows down from the top of `area`, or
/// [`HitRegion::Truncated`] when that row is past the area's last row.
pub(super) fn row_rect(area: Rect, line_index: usize) -> HitRegion {
    let offset = u16::try_from(line_index).unwrap_or(u16::MAX);
    if offset >= area.height {
        return HitRegion::Truncated;
    }
    HitRegion::Drawn(Rect::new(
        area.x,
        area.y.saturating_add(offset),
        area.width,
        1,
    ))
}
