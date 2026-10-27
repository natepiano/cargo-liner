//! Exclusive invocation sites for one checked frame.

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::time::Duration;

use ratatui::layout::Rect;
use sysinfo::Pid;
use tui_pane::EndedCellDeparture;
use tui_pane::FramePlacement;
use tui_pane::PaintedRowSpan;

use crate::census::InvocationId;
use crate::census::scan::ResolvedInvocationId;
use crate::roster::TrackedGroup;
use crate::roster::TrackedRow;
use crate::roster::VisibleRoster;

/// A command cell keeps the invocation identity across process source changes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct InvocationCellId(pub(super) InvocationId);

/// An ended row keeps exactly one site until its receipt completes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum DepartureSite {
    Summary,
    Cell(InvocationCellId),
}

/// A row has one place in the checked frame.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum InvocationPlacement {
    Listed,
    Celled(InvocationCellId),
    Departing(DepartureSite),
    UnshownAfterDeparture,
}

/// The checked grid outcome is the only source of cell assignments.
#[derive(Clone, Eq, PartialEq)]
pub(super) struct FrameInvocationPlacement {
    sites: HashMap<ResolvedInvocationId, InvocationPlacement>,
}

impl FrameInvocationPlacement {
    pub(super) fn listed(roster: &VisibleRoster) -> Self {
        Self {
            sites: roster
                .groups()
                .iter()
                .flat_map(TrackedGroup::rows)
                .filter_map(|row| {
                    let invocation = row.resolved().ok()?.clone();
                    let site = if row.is_ended() {
                        InvocationPlacement::Departing(DepartureSite::Summary)
                    } else {
                        InvocationPlacement::Listed
                    };
                    Some((invocation, site))
                })
                .collect(),
        }
    }

    pub(super) fn checked(roster: &VisibleRoster, grid: &FramePlacement<InvocationId>) -> Self {
        let mut sites = HashMap::new();
        for group in roster.groups() {
            for row in group.rows() {
                let Ok(invocation) = row.resolved() else {
                    continue;
                };
                let cell = InvocationCellId(group.id.clone());
                let site = if grid.celled().contains(&group.id) {
                    if row.is_ended() {
                        InvocationPlacement::Departing(DepartureSite::Cell(cell))
                    } else {
                        InvocationPlacement::Celled(cell)
                    }
                } else if row.faded() == u8::MAX {
                    InvocationPlacement::UnshownAfterDeparture
                } else if row.is_ended() {
                    InvocationPlacement::Departing(DepartureSite::Summary)
                } else {
                    InvocationPlacement::Listed
                };
                sites.insert(invocation.clone(), site);
            }
        }
        Self { sites }
    }

    /// A paint plan borrows the one placement committed for this draw.
    pub(super) fn assignment<'a>(&'a self, cell: &InvocationId) -> CellAssignment<'a> {
        CellAssignment {
            cell:      InvocationCellId(cell.clone()),
            placement: self,
        }
    }

    /// Low-level painter fixtures sit below the checked grid proof.
    #[cfg(test)]
    pub(super) fn painter_fixture(roster: &VisibleRoster, cell: Option<&InvocationId>) -> Self {
        let mut placement = Self::listed(roster);
        if let Some(cell) = cell {
            for group in roster.groups().iter().filter(|group| &group.id == cell) {
                for row in group.rows() {
                    let Ok(invocation) = row.resolved() else {
                        continue;
                    };
                    placement.sites.insert(
                        invocation.clone(),
                        InvocationPlacement::Celled(InvocationCellId(cell.clone())),
                    );
                }
            }
        }
        placement
    }

    pub(super) fn summary(&self, row: &TrackedRow) -> bool {
        row.resolved().is_ok_and(|invocation| {
            matches!(
                self.sites.get(invocation),
                Some(
                    InvocationPlacement::Listed
                        | InvocationPlacement::Departing(DepartureSite::Summary)
                )
            )
        })
    }
}

/// Only a committed placement can supply a command's rows to a painter.
pub(super) struct CellAssignment<'a> {
    cell:      InvocationCellId,
    placement: &'a FrameInvocationPlacement,
}

impl CellAssignment<'_> {
    fn holds(&self, invocation: &ResolvedInvocationId) -> bool {
        matches!(self.placement.sites.get(invocation),
            Some(InvocationPlacement::Celled(owner)
                | InvocationPlacement::Departing(DepartureSite::Cell(owner))) if owner == &self.cell)
    }
}

/// A previous frame can seed measurement but cannot supply a paint plan.
pub(super) struct PlacementTrial(pub(super) FrameInvocationPlacement);

/// One process row belongs to one resolved execution, even across source changes.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) struct InvocationProcessRowKey {
    invocation: ResolvedInvocationId,
    pid:        Pid,
}

/// Publication replaces the values at a key. It cannot append another copy.
pub(super) struct InvocationProcessRows {
    values:   HashMap<InvocationProcessRowKey, (InvocationCellId, TrackedRow)>,
    order:    Vec<InvocationProcessRowKey>,
    resolved: HashMap<InvocationId, ResolvedInvocationId>,
}

impl InvocationProcessRows {
    pub(super) fn of(roster: &VisibleRoster) -> Self {
        let mut values = HashMap::new();
        let mut order = Vec::new();
        let mut resolved = HashMap::new();
        for group in roster.groups() {
            for row in group.rows() {
                let Ok(invocation) = row.resolved().cloned() else {
                    continue;
                };
                resolved.insert(row.process.invocation_id.clone(), invocation.clone());
                let key = InvocationProcessRowKey {
                    invocation,
                    pid: Pid::from_u32(row.process.pid),
                };
                let value = (InvocationCellId(group.id.clone()), row.clone());
                match values.entry(key.clone()) {
                    Entry::Occupied(mut entry) => {
                        entry.insert(value);
                    },
                    Entry::Vacant(entry) => {
                        order.push(key);
                        entry.insert(value);
                    },
                }
            }
        }
        Self {
            values,
            order,
            resolved,
        }
    }

    pub(super) fn is_empty(&self) -> bool { self.values.is_empty() }

    pub(super) fn assigned<'a>(&'a self, assignment: &CellAssignment<'_>) -> CellProcessRows<'a> {
        CellProcessRows {
            rows: self
                .order
                .iter()
                .filter_map(|key| {
                    self.values
                        .get(key)
                        .filter(|(owner, _)| {
                            owner == &assignment.cell && assignment.holds(&key.invocation)
                        })
                        .map(|(_, row)| (key, row))
                })
                .collect(),
        }
    }

    /// Capacity trials can measure membership but cannot make a paint assignment.
    pub(super) fn measured(&self, cell: &InvocationId) -> Vec<&TrackedRow> {
        self.order
            .iter()
            .filter_map(|key| {
                self.values
                    .get(key)
                    .filter(|(owner, _)| &owner.0 == cell)
                    .map(|(_, row)| row)
            })
            .collect()
    }

    pub(super) fn listed<'a>(
        &'a self,
        placement: &FrameInvocationPlacement,
    ) -> Vec<&'a TrackedRow> {
        self.order
            .iter()
            .filter_map(|key| {
                self.values
                    .get(key)
                    .map(|(_, row)| row)
                    .filter(|row| placement.summary(row))
            })
            .collect()
    }

    pub(super) fn presented(
        &self,
        source: &PaintedProcessRowSpan,
        visible: PaintedRowSpan<InvocationId>,
    ) {
        if let Some((_, row)) = self.values.get(&source.key) {
            row.presented_process(source.key.pid.as_u32(), visible);
        }
    }

    pub(super) fn span(
        &self,
        span: &ProcessRowPaint,
        site: ProcessRowSite,
    ) -> Option<PaintedProcessRowSpan> {
        let key = InvocationProcessRowKey {
            invocation: self.resolved.get(span.span.id())?.clone(),
            pid:        Pid::from_u32(span.pid),
        };
        let (key, _) = self.values.get_key_value(&key)?;
        Some(PaintedProcessRowSpan {
            key: key.clone(),
            site,
            area: span.span.area(),
        })
    }
}

/// A cell receives rows from its assignment, never from a shared paint vector.
pub(super) struct CellProcessRows<'a> {
    rows: Vec<(&'a InvocationProcessRowKey, &'a TrackedRow)>,
}

impl<'a> CellProcessRows<'a> {
    pub(super) fn values(&self) -> Vec<&'a TrackedRow> {
        self.rows.iter().map(|(_, row)| *row).collect()
    }
}

/// Ended row values stay independent of the header and column widths.
pub(super) struct EndedCellContent(Vec<TrackedRow>);

impl EndedCellContent {
    /// The framework owns the clock; retained values keep the ink at its start.
    pub(super) fn new(rows: CellProcessRows<'_>, end: &EndedCellDeparture, fade: Duration) -> Self {
        Self(
            rows.rows
                .into_iter()
                .map(|(_, row)| row.at_cell_departure(end, fade))
                .collect(),
        )
    }
    pub(super) fn values(&self) -> Vec<&TrackedRow> { self.0.iter().collect() }
}

/// The caller supplies the explicit cell site, including for a summary row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum ProcessRowSite {
    Summary,
    Cell(InvocationCellId),
}

/// The table painter reports the process key with the actual rectangle it wrote.
#[derive(Clone)]
pub(super) struct ProcessRowPaint {
    pub(super) pid:  u32,
    pub(super) span: PaintedRowSpan<InvocationId>,
}

/// A receipt names a row and its exclusive cell allocation.
#[derive(Clone)]
pub(super) struct PaintedProcessRowSpan {
    key:  InvocationProcessRowKey,
    site: ProcessRowSite,
    area: Rect,
}

impl PaintedProcessRowSpan {
    pub(super) fn matches_source(&self, source: &PaintedRowSpan<InvocationId>) -> bool {
        self.key.invocation.id() == source.id() && self.area == source.area()
    }

    pub(super) fn adapter(&self, site: &ProcessRowSite) -> Option<PaintedRowSpan<InvocationId>> {
        if &self.site != site {
            return None;
        }
        PaintedRowSpan::new(self.key.invocation.id().clone(), self.area).ok()
    }
}
