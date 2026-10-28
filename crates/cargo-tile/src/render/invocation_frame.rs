//! Exclusive invocation sites for one checked frame.

use std::collections::HashMap;
use std::time::Duration;

use ratatui::layout::Rect;
use sysinfo::Pid;
use tui_pane::EndedCellDeparture;
use tui_pane::FramePlacement;
use tui_pane::PaintedRowSpan;

use crate::census::InvocationId;
use crate::census::scan::EndedPresentation;
use crate::census::scan::InvocationPaintSite;
use crate::census::scan::PublishedInvocation;
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
    pub(super) fn listed(roster: &VisibleRoster) -> Self { Self::from_rows(roster, |_| false) }

    pub(super) fn checked(roster: &VisibleRoster, grid: &FramePlacement<InvocationId>) -> Self {
        Self::from_rows(roster, |group| grid.celled().contains(&group.id))
    }

    fn from_rows(roster: &VisibleRoster, celled: impl Fn(&TrackedGroup) -> bool) -> Self {
        let mut sites = HashMap::new();
        for group in roster.groups() {
            for row in group.rows() {
                let Ok(invocation) = row.resolved() else {
                    continue;
                };
                let site = match row.publication() {
                    PublishedInvocation::Ended(end) => match end.presentation() {
                        EndedPresentation::Presented(InvocationPaintSite::Summary) => {
                            InvocationPlacement::Departing(DepartureSite::Summary)
                        },
                        EndedPresentation::Presented(InvocationPaintSite::Cell(cell)) => {
                            InvocationPlacement::Departing(DepartureSite::Cell(InvocationCellId(
                                cell,
                            )))
                        },
                        EndedPresentation::Departed => InvocationPlacement::UnshownAfterDeparture,
                        EndedPresentation::Unpresented | EndedPresentation::Withdrawn => continue,
                    },
                    PublishedInvocation::Live(_)
                    | PublishedInvocation::AwaitingCargo(_)
                    | PublishedInvocation::Missing(_) => {
                        if celled(group) {
                            InvocationPlacement::Celled(InvocationCellId(group.id.clone()))
                        } else {
                            InvocationPlacement::Listed
                        }
                    },
                    PublishedInvocation::Joined { .. } => continue,
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
    pub(super) fn painter_fixture(roster: &VisibleRoster, cell: &InvocationId) -> Self {
        let mut placement = Self::listed(roster);
        {
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

    fn summary_site(&self, row: &TrackedRow) -> SummaryRowSite {
        if row.resolved().is_ok_and(|invocation| {
            matches!(
                self.sites.get(invocation),
                Some(
                    InvocationPlacement::Listed
                        | InvocationPlacement::Departing(DepartureSite::Summary)
                )
            )
        }) {
            SummaryRowSite::Included
        } else {
            SummaryRowSite::Elsewhere
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum SummaryRowSite {
    Included,
    Elsewhere,
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
pub(super) struct PlacementTrial(FrameInvocationPlacement);

impl From<FrameInvocationPlacement> for PlacementTrial {
    fn from(placement: FrameInvocationPlacement) -> Self { Self(placement) }
}

impl PlacementTrial {
    pub(super) fn agrees_with(&self, placement: &FrameInvocationPlacement) -> bool {
        &self.0 == placement
    }

    pub(super) fn summary_rows<'a>(
        &self,
        rows: &'a InvocationProcessRows,
    ) -> super::SummaryRows<'a> {
        super::SummaryRows::of(rows, &self.0)
    }

    pub(super) fn listed<'a>(&self, rows: &'a InvocationProcessRows) -> Vec<&'a TrackedRow> {
        rows.listed(&self.0)
    }
}

/// One process row belongs to one resolved execution, even across source changes.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) struct InvocationProcessRowKey {
    invocation: ResolvedInvocationId,
    pid:        Pid,
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
    site: DepartureSite,
    area: Rect,
}

impl PaintedProcessRowSpan {
    pub(super) fn matches_source(&self, source: &PaintedRowSpan<InvocationId>) -> bool {
        self.key.invocation.id() == source.id() && self.area == source.area()
    }

    pub(super) fn adapter(&self, site: &DepartureSite) -> Option<PaintedRowSpan<InvocationId>> {
        if &self.site != site {
            return None;
        }
        PaintedRowSpan::new(self.key.invocation.id().clone(), self.area).ok()
    }
}

/// Publication replaces the values at a key. It cannot append another copy.
pub(super) struct InvocationProcessRows {
    values:   HashMap<InvocationProcessRowKey, (InvocationCellId, TrackedRow)>,
    order:    Vec<InvocationProcessRowKey>,
    resolved: HashMap<InvocationId, ResolvedInvocationId>,
}

impl From<&VisibleRoster> for InvocationProcessRows {
    fn from(roster: &VisibleRoster) -> Self {
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
                if values.insert(key.clone(), value).is_none() {
                    order.push(key);
                }
            }
        }
        Self {
            values,
            order,
            resolved,
        }
    }
}

impl InvocationProcessRows {
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
                                        .filter(|(owner, row)| {
                        &owner.0 == cell && match row.publication() {
                            PublishedInvocation::Ended(end) => matches!(end.presentation(),
                                EndedPresentation::Presented(InvocationPaintSite::Cell(owner)) if &owner == cell),
                            PublishedInvocation::Live(_)
                            | PublishedInvocation::AwaitingCargo(_)
                            | PublishedInvocation::Missing(_) => true,
                            PublishedInvocation::Joined { .. } => false,
                        }
                    })
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
                    .filter(|row| placement.summary_site(row) == SummaryRowSite::Included)
            })
            .collect()
    }

    pub(super) fn listed_with_owners<'a>(
        &'a self,
        placement: &FrameInvocationPlacement,
    ) -> Vec<(&'a TrackedRow, InvocationId)> {
        self.order
            .iter()
            .filter_map(|key| self.values.get(key))
            .filter(|(_, row)| placement.summary_site(row) == SummaryRowSite::Included)
            .map(|(owner, row)| (row, owner.0.clone()))
            .collect()
    }

    pub(super) fn presented(
        &self,
        source: &PaintedProcessRowSpan,
        visible: PaintedRowSpan<InvocationId>,
    ) {
        if let Some((_, row)) = self.values.get(&source.key) {
            row.presented_process(
                source.key.pid.as_u32(),
                InvocationPaintSite::from(&source.site),
                visible,
            );
        }
    }

    pub(super) fn span(
        &self,
        paint: &ProcessRowPaint,
        site: DepartureSite,
    ) -> Option<PaintedProcessRowSpan> {
        let key = InvocationProcessRowKey {
            invocation: self.resolved.get(paint.span.id())?.clone(),
            pid:        Pid::from_u32(paint.pid),
        };
        let (key, _) = self.values.get_key_value(&key)?;
        Some(PaintedProcessRowSpan {
            key: key.clone(),
            site,
            area: paint.span.area(),
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

impl From<&DepartureSite> for InvocationPaintSite {
    fn from(site: &DepartureSite) -> Self {
        match site {
            DepartureSite::Summary => Self::Summary,
            DepartureSite::Cell(cell) => Self::Cell(cell.0.clone()),
        }
    }
}
