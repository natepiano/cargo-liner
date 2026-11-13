use std::cmp::Reverse;

use super::Slot;

/// The rule that assigns detail tiles to existing column slots.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum TileColumnOrder<Id> {
    /// Keep the order supplied by the application.
    #[default]
    SourceOrder,
    /// Pair long and short trees by their full row counts.
    /// Counts exclude headings and include rows hidden by collapse or scrolling.
    LongestWithShortest(Vec<(Id, usize)>),
}

/// Only a rank change changes the placement input.
#[derive(Default, Eq, PartialEq)]
pub(super) enum ColumnRanks<Id> {
    #[default]
    SourceOrder,
    LongestWithShortest {
        source: Vec<Id>,
        ranked: Vec<Id>,
    },
}

/// Each slot identity occurs once in both orders.
struct CompleteColumnRanks<'a, Id> {
    source: Vec<&'a Id>,
    ranked: Vec<&'a Id>,
}

impl<'a, Id: Eq> CompleteColumnRanks<'a, Id> {
    fn for_slots(source: &'a [Id], ranked: &'a [Id], slots: &'a [Slot<Id>]) -> Self {
        let slot_ids: Vec<_> = slots
            .iter()
            .filter_map(|slot| match slot {
                Slot::Group(id) => Some(id),
                Slot::Empty(_) => None,
            })
            .collect();
        let mut complete_source = Vec::with_capacity(slot_ids.len());
        for id in source.iter().chain(slot_ids.iter().copied()) {
            if slot_ids.contains(&id) && !complete_source.contains(&id) {
                complete_source.push(id);
            }
        }
        let mut complete_ranked = Vec::with_capacity(complete_source.len());
        for id in ranked.iter().chain(complete_source.iter().copied()) {
            if complete_source.contains(&id) && !complete_ranked.contains(&id) {
                complete_ranked.push(id);
            }
        }
        Self {
            source: complete_source,
            ranked: complete_ranked,
        }
    }
}

enum SweepDirection {
    LeftToRight,
    RightToLeft,
}

impl SweepDirection {
    const fn column(&self, index: usize, count: usize) -> usize {
        match self {
            Self::LeftToRight => index,
            Self::RightToLeft => count - 1 - index,
        }
    }

    const fn next(self) -> Self {
        match self {
            Self::LeftToRight => Self::RightToLeft,
            Self::RightToLeft => Self::LeftToRight,
        }
    }
}

impl<Id: Clone + Eq> ColumnRanks<Id> {
    /// Equal counts retain their previous rank, or source order on the first frame.
    pub(super) fn refresh(&self, order: TileColumnOrder<Id>) -> Self {
        match order {
            TileColumnOrder::SourceOrder => Self::SourceOrder,
            TileColumnOrder::LongestWithShortest(mut rows) => {
                let source = rows.iter().map(|(id, _)| id.clone()).collect();
                rows.sort_by_key(|(id, rows)| {
                    (
                        Reverse(*rows),
                        match self {
                            Self::SourceOrder => 0,
                            Self::LongestWithShortest { ranked, .. } => ranked
                                .iter()
                                .position(|held| held == id)
                                .unwrap_or(usize::MAX),
                        },
                    )
                });
                Self::LongestWithShortest {
                    source,
                    ranked: rows.into_iter().map(|(id, _)| id).collect(),
                }
            },
        }
    }

    pub(super) const fn keeps_source_order(&self) -> bool { matches!(self, Self::SourceOrder) }

    /// Keep empty slots fixed and retain source order inside each column.
    pub(super) fn arrange(
        &self,
        slots: &[Slot<Id>],
        column_capacities: &[usize],
        depth: usize,
    ) -> Vec<Slot<Id>> {
        let Self::LongestWithShortest { source, ranked } = self else {
            return slots.to_vec();
        };
        let complete = CompleteColumnRanks::for_slots(source, ranked, slots);
        let mut next = slots.to_vec();
        let mut offset = 0;
        let positions: Vec<Vec<usize>> = column_capacities
            .iter()
            .enumerate()
            .map(|(column, &height)| {
                let count = if column == 0 {
                    height.saturating_sub(depth)
                } else {
                    height
                };
                let positions = (offset..offset + count)
                    .filter(|&i| matches!(slots.get(i), Some(Slot::Group(_))))
                    .collect();
                offset += count;
                positions
            })
            .collect();
        let mut assigned = vec![Vec::new(); positions.len()];
        let mut ranks = complete.ranked.into_iter();
        let mut direction = SweepDirection::LeftToRight;
        for _ in 0..slots.len() {
            for index in 0..positions.len() {
                let column = direction.column(index, positions.len());
                if assigned[column].len() < positions[column].len()
                    && let Some(id) = ranks.next()
                {
                    assigned[column].push(id);
                }
            }
            direction = direction.next();
        }
        for (positions, mut ids) in positions.into_iter().zip(assigned) {
            ids.sort_by_key(|id| complete.source.iter().position(|source| source == id));
            for (position, id) in positions.into_iter().zip(ids) {
                next[position] = Slot::Group(id.clone());
            }
        }
        next
    }
}

#[cfg(test)]
#[path = "column_order_tests.rs"]
mod tests;
