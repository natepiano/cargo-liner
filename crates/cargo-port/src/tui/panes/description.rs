//! `DescriptionBlock` — the description section shared by the Package and
//! Git detail panes.
//!
//! Both panes render a header description (repo/package "about" text) at
//! the top of their pane, and the two block heights must match so the
//! separator rules sit at the same y-coordinate. The previous design split
//! that contract across two free functions — one for the sync floor's
//! natural-height computation and one for the actual render — that read
//! different inputs and could silently diverge if a renderer added content
//! the sync path didn't see. This module bundles both into one value:
//!
//! - [`DescriptionBlock::for_pane`] is the only producer of `rows`.
//! - [`DescriptionBlock::natural_sync_height`] reads `rows` to report the block's contribution to
//!   the inter-pane sync.
//! - [`DescriptionBlock::render`] reads the *same* `rows` to draw.
//!
//! Adding content therefore has to flow through the constructor, which is
//! the single place that updates the rendered rows and the sync height at
//! the same time. Without that, the height-sync invariant can't break.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use tui_pane::PaneSelectionState;
use tui_pane::label_color;

use super::constants::NO_DESCRIPTION_AVAILABLE;
use super::support;
use crate::tui::render;

/// What the block renders when the source description is empty.
#[derive(Clone, Copy)]
pub enum EmptyDescriptionBehavior {
    /// Render the `NO_DESCRIPTION_AVAILABLE` placeholder in a muted style
    /// (the Package pane's behavior). Placeholder rows do *not* count for
    /// inter-pane sync — the sync floor stays `0` so the Git pane (which
    /// renders nothing when empty) doesn't get padded to match.
    ShowPlaceholder,
    /// Render nothing (the Git pane's behavior).
    RenderEmpty,
}

/// Synced description height carried in `PaneRenderCtx`. Constructed only
/// via [`sync_floor`] so neither pane can fabricate a value that wouldn't
/// be backed by a real [`DescriptionBlock`].
#[derive(Clone, Copy, Default)]
pub struct SyncedDescriptionHeight(u16);

impl SyncedDescriptionHeight {
    pub const fn rows(self) -> u16 { self.0 }
}

/// Pre-wrapped description block for one detail pane. Owns the wrapped
/// row strings and the layout dimensions needed to render them; both
/// [`DescriptionBlock::natural_sync_height`] and [`DescriptionBlock::render`] read from the same
/// private `rows` field.
#[derive(Clone)]
enum DescriptionContentState {
    Real,
    PlaceholderOrEmpty,
}

impl DescriptionContentState {
    const fn from_trimmed(trimmed: Option<&str>) -> Self {
        if trimmed.is_some() {
            Self::Real
        } else {
            Self::PlaceholderOrEmpty
        }
    }

    const fn has_real_content(&self) -> bool { matches!(self, Self::Real) }
}

#[derive(Clone)]
pub struct DescriptionBlock {
    rows:             Vec<String>,
    style:            Style,
    column_width:     u16,
    padding:          u16,
    content_state:    DescriptionContentState,
    inner_height_cap: u16,
}

impl DescriptionBlock {
    /// Build the description block for a pane sitting in `outer_area`.
    /// `empty_behavior` decides what renders when the source text is
    /// empty (Package shows a placeholder; Git renders nothing).
    pub fn for_pane(
        text: Option<&str>,
        outer_area: Rect,
        empty_behavior: EmptyDescriptionBehavior,
    ) -> Self {
        let inner_width = outer_area.width.saturating_sub(2);
        let inner_height = outer_area.height.saturating_sub(2);
        let padding = u16::from(inner_width > 2);
        let column_width = inner_width.saturating_sub(padding.saturating_mul(2));

        let trimmed = text.map(str::trim).filter(|s| !s.is_empty());
        let content_state = DescriptionContentState::from_trimmed(trimmed);

        let (body, style) = match (trimmed, empty_behavior) {
            (Some(real), _) => (Some(real), Style::default()),
            (None, EmptyDescriptionBehavior::ShowPlaceholder) => (
                Some(NO_DESCRIPTION_AVAILABLE),
                Style::default().fg(label_color()),
            ),
            (None, EmptyDescriptionBehavior::RenderEmpty) => (None, Style::default()),
        };

        let rows = match body {
            Some(text) if column_width > 0 => support::word_wrap(text, usize::from(column_width)),
            _ => Vec::new(),
        };

        Self {
            rows,
            style,
            column_width,
            padding,
            content_state,
            inner_height_cap: inner_height,
        }
    }

    /// Rows the block contributes to the inter-pane height sync.
    ///
    /// Returns `0` when the source text was empty — placeholders don't
    /// trigger sync, matching the previous free-function behavior: only
    /// when *both* panes have real content do they align their bottoms.
    pub fn natural_sync_height(&self) -> u16 {
        if self.content_state.has_real_content() {
            u16::try_from(self.rows.len())
                .unwrap_or(u16::MAX)
                .min(self.inner_height_cap)
        } else {
            0
        }
    }

    /// Render into `project_inner` with `max_height` as the hard cap
    /// (computed by the pane from its reserved-lower budget) and
    /// `synced_floor` as the inter-pane sync floor. Returns the
    /// rendered height, which the pane uses to position the separator
    /// rule and the rest of its content.
    #[allow(
        dead_code,
        reason = "selectable panes call render_with_selection; keep the unselected wrapper for callers that do not track row focus"
    )]
    pub fn render(
        &self,
        frame: &mut Frame,
        project_inner: Rect,
        synced_floor: SyncedDescriptionHeight,
        max_height: u16,
    ) -> u16 {
        self.render_with_selection(
            frame,
            project_inner,
            synced_floor,
            max_height,
            PaneSelectionState::Unselected,
        )
    }

    pub fn render_with_selection(
        &self,
        frame: &mut Frame,
        project_inner: Rect,
        synced_floor: SyncedDescriptionHeight,
        max_height: u16,
        selection: PaneSelectionState,
    ) -> u16 {
        if self.rows.is_empty() || self.column_width == 0 {
            return 0;
        }
        let synced_cap = project_inner.height.saturating_sub(1);
        let synced_floor = synced_floor.rows().min(synced_cap);
        let row_budget = max_height.max(synced_floor);
        if row_budget == 0 {
            return 0;
        }

        let visible_count = usize::from(row_budget).min(self.rows.len());
        let overflowed = self.rows.len() > visible_count;
        let mut visible: Vec<String> = self.rows.iter().take(visible_count).cloned().collect();
        if overflowed && let Some(last) = visible.last_mut() {
            let with_ellipsis = format!("{last}\u{2026}");
            *last = render::truncate_with_ellipsis(
                &with_ellipsis,
                usize::from(self.column_width),
                "\u{2026}",
            );
        }

        let style = selection.patch(self.style);
        let lines: Vec<Line<'static>> = visible
            .into_iter()
            .map(|row| Line::from(Span::styled(row, style)))
            .collect();
        let natural = u16::try_from(lines.len()).unwrap_or(u16::MAX);
        let description_height = natural.max(synced_floor);
        if description_height == 0 {
            return 0;
        }

        let area = Rect {
            x:      project_inner.x.saturating_add(self.padding),
            y:      project_inner.y,
            width:  self.column_width,
            height: description_height,
        };
        frame.render_widget(Paragraph::new(lines), area);
        description_height
    }
}

/// Inter-pane sync floor: the height both panes' description blocks
/// must clear so their bottom edges line up. Returns `0` if any block
/// is empty (placeholder or missing source), matching the old free-
/// function behavior.
pub fn sync_floor(blocks: &[&DescriptionBlock]) -> SyncedDescriptionHeight {
    let heights: Vec<u16> = blocks.iter().map(|b| b.natural_sync_height()).collect();
    if heights.contains(&0) {
        SyncedDescriptionHeight(0)
    } else {
        SyncedDescriptionHeight(heights.into_iter().max().unwrap_or(0))
    }
}

/// The placeholder string the Package pane renders when its source is
/// empty. Exposed only for tests that check the placeholder content.
#[cfg(test)]
const fn placeholder_text() -> &'static str { NO_DESCRIPTION_AVAILABLE }

#[cfg(test)]
impl DescriptionBlock {
    /// Test-only accessor for the wrapped row strings (pre-truncation).
    pub fn rows(&self) -> &[String] { &self.rows }

    /// Test-only accessor for the row style (placeholder rows are muted).
    pub const fn style(&self) -> Style { self.style }
}

#[cfg(test)]
mod tests {
    use ratatui::layout::Rect;
    use tui_pane::label_color;

    use super::DescriptionBlock;
    use super::EmptyDescriptionBehavior;
    use super::placeholder_text;

    /// Helper: outer pane area sized so `DescriptionBlock::for_pane` yields
    /// the desired inner column width. Outer width = `inner_width` + 2 (borders)
    /// + 2 (padding). Outer height = `inner_height` + 2 (borders).
    fn description_area(column_width: u16, inner_height: u16) -> Rect {
        Rect {
            x:      0,
            y:      0,
            width:  column_width.saturating_add(4),
            height: inner_height.saturating_add(2),
        }
    }

    #[test]
    fn block_uses_muted_placeholder_when_missing() {
        let block = DescriptionBlock::for_pane(
            None,
            description_area(80, 3),
            EmptyDescriptionBehavior::ShowPlaceholder,
        );

        assert_eq!(block.rows(), &[placeholder_text().to_string()]);
        assert_eq!(block.style().fg, Some(label_color()));
    }

    #[test]
    fn empty_behavior_render_empty_produces_no_rows() {
        let block = DescriptionBlock::for_pane(
            None,
            description_area(80, 3),
            EmptyDescriptionBehavior::RenderEmpty,
        );

        assert!(block.rows().is_empty());
        assert_eq!(block.natural_sync_height(), 0);
    }

    #[test]
    fn renders_real_description_with_default_style() {
        let block = DescriptionBlock::for_pane(
            Some("Real package description"),
            description_area(80, 3),
            EmptyDescriptionBehavior::ShowPlaceholder,
        );

        assert_eq!(block.rows(), &["Real package description".to_string()]);
        assert_eq!(block.style().fg, None);
    }

    #[test]
    fn wraps_overflowing_text_into_rows() {
        let block = DescriptionBlock::for_pane(
            Some("one two three four five six seven eight"),
            description_area(13, 5),
            EmptyDescriptionBehavior::ShowPlaceholder,
        );

        // Pre-truncation rows - the render path's ellipsis is applied
        // when `max_height` clamps below `rows.len()`. natural_sync_height
        // reflects what feeds the inter-pane sync.
        assert!(block.rows().len() > 2);
        assert_eq!(block.rows()[0], "one two three");
    }
}
