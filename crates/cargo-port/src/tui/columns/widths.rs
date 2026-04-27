//! Generic "fit columns to content with min-width-per-column" helper.
//!
//! Phase 2 of the App-API carve (see `docs/app-api.md`). Two consumers
//! today: the project list (via [`super::ProjectListWidths`]) and the
//! CI pane (`tui::panes::ci::build_ci_widths`). New table-style panes
//! that fit columns to content use this primitive rather than
//! re-implementing the seed/observe/total pattern.

use ratatui::layout::Constraint;

/// Per-column width constraint for [`ColumnWidths`].
#[derive(Clone, Copy, Debug)]
pub(in crate::tui) struct ColumnSpec {
    /// Minimum width for this column. The column never shrinks below
    /// this, and is seeded to this width by [`ColumnWidths::new`].
    pub min: u16,
    /// Optional cap. The column never grows past this. Use `Some(min)`
    /// for "Fixed"-style columns whose width is independent of
    /// content.
    pub max: Option<u16>,
}

impl ColumnSpec {
    /// Fit-to-content column with a header-label minimum and no
    /// upper cap.
    pub(in crate::tui) const fn fit(min: u16) -> Self { Self { min, max: None } }

    /// Fixed-width column whose width is independent of content.
    pub(in crate::tui) const fn fixed(width: u16) -> Self {
        Self {
            min: width,
            max: Some(width),
        }
    }
}

/// Resolved widths for a fit-to-content table. Owns one width per
/// column; mutate via [`Self::observe_cell`] as content is iterated.
#[derive(Clone, Debug)]
pub(in crate::tui) struct ColumnWidths {
    specs:  Vec<ColumnSpec>,
    widths: Vec<u16>,
}

impl ColumnWidths {
    /// Seed widths to each spec's minimum.
    pub(in crate::tui) fn new(specs: Vec<ColumnSpec>) -> Self {
        let widths = specs.iter().map(|spec| spec.min).collect();
        Self { specs, widths }
    }

    /// Grow `col` to fit `width`. Capped at the spec's `max` if set.
    /// `Fixed` columns (where `max == Some(min)`) are no-ops.
    pub(in crate::tui) fn observe_cell(&mut self, col: usize, width: u16) {
        let spec = self.specs[col];
        let candidate = self.widths[col].max(width);
        self.widths[col] = spec.max.map_or(candidate, |cap| candidate.min(cap));
    }

    /// Convenience: observe `width` (as `usize`, clamped to `u16`).
    pub(in crate::tui) fn observe_cell_usize(&mut self, col: usize, width: usize) {
        let clamped = u16::try_from(width).unwrap_or(u16::MAX);
        self.observe_cell(col, clamped);
    }

    /// Resolved width for `col`.
    pub(in crate::tui) fn get(&self, col: usize) -> u16 { self.widths[col] }

    /// Resolved widths as an `&[u16]`.
    #[allow(
        dead_code,
        reason = "part of the documented ColumnWidths facade; future panes that \
                  need raw widths use this rather than reaching for `get` per column"
    )]
    pub(in crate::tui) fn widths(&self) -> &[u16] { &self.widths }

    /// Convert to ratatui [`Constraint::Length`] entries — one per
    /// column, in order.
    pub(in crate::tui) fn to_constraints(&self) -> Vec<Constraint> {
        self.widths
            .iter()
            .copied()
            .map(Constraint::Length)
            .collect()
    }

    /// Sum of all resolved column widths.
    #[allow(
        dead_code,
        reason = "part of the documented ColumnWidths facade; future fit-check \
                  paths (analogous to `ci_table_shows_durations`) consume it"
    )]
    pub(in crate::tui) fn total(&self) -> u16 {
        self.widths.iter().copied().fold(0u16, u16::saturating_add)
    }

    /// Number of columns.
    #[allow(dead_code, reason = "part of the documented ColumnWidths facade")]
    pub(in crate::tui) const fn len(&self) -> usize { self.widths.len() }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::*;

    #[test]
    fn fit_grows_to_observed_content() {
        let mut widths = ColumnWidths::new(vec![ColumnSpec::fit(4)]);
        widths.observe_cell(0, 7);
        assert_eq!(widths.get(0), 7);
    }

    #[test]
    fn fit_never_shrinks_below_min() {
        let mut widths = ColumnWidths::new(vec![ColumnSpec::fit(6)]);
        widths.observe_cell(0, 2);
        assert_eq!(widths.get(0), 6);
    }

    #[test]
    fn fixed_column_ignores_observations() {
        let mut widths = ColumnWidths::new(vec![ColumnSpec::fixed(2)]);
        widths.observe_cell(0, 12);
        assert_eq!(widths.get(0), 2);
    }

    #[test]
    fn capped_fit_column_clamps_at_max() {
        let mut widths = ColumnWidths::new(vec![ColumnSpec {
            min: 4,
            max: Some(8),
        }]);
        widths.observe_cell(0, 99);
        assert_eq!(widths.get(0), 8);
    }

    #[test]
    fn to_constraints_emits_length_per_column() {
        let mut widths = ColumnWidths::new(vec![ColumnSpec::fit(2), ColumnSpec::fixed(3)]);
        widths.observe_cell(0, 5);
        let cs = widths.to_constraints();
        assert_eq!(cs.len(), 2);
        assert!(matches!(cs[0], Constraint::Length(5)));
        assert!(matches!(cs[1], Constraint::Length(3)));
    }
}
