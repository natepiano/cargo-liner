use ratatui::layout::Position;
use ratatui::layout::Rect;

/// A logical row's vertical extent in rendered content coordinates.
#[derive(Clone, Copy, Debug, Default)]
pub struct VisualRowSpan {
    /// First content line occupied by the row.
    pub start_y: usize,
    /// Number of rendered lines occupied by the row.
    pub height:  usize,
}

/// The logical row under a screen position, if any.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VisualRowHit {
    /// Index of the first matching span.
    Row(usize),
    /// The position does not hit a visible span.
    Miss,
}

/// Projection of rendered row spans into a scrolled content rectangle.
pub struct VisualRowHitLayout<'a> {
    /// Visible content rectangle in screen coordinates.
    pub content_area:  Rect,
    /// First visible content line.
    pub scroll_offset: usize,
    /// Spans in logical row order.
    pub spans:         &'a [VisualRowSpan],
}

impl VisualRowHitLayout<'_> {
    /// Find the first visible row span containing `pos`.
    #[must_use]
    pub fn hit_test(&self, pos: Position) -> VisualRowHit {
        let inner = self.content_area;
        if !inner.contains(pos) {
            return VisualRowHit::Miss;
        }
        let visible_top = inner.y;
        let visible_bottom = inner.y.saturating_add(inner.height);
        for (row_index, span) in self.spans.iter().enumerate() {
            if span.start_y.saturating_add(span.height) <= self.scroll_offset {
                continue;
            }
            let offset = span.start_y.saturating_sub(self.scroll_offset);
            let screen_y = inner
                .y
                .saturating_add(u16::try_from(offset).unwrap_or(u16::MAX));
            let screen_bottom = screen_y.saturating_add(u16::try_from(span.height).unwrap_or(1));
            if screen_bottom <= visible_top || screen_y >= visible_bottom {
                continue;
            }
            if pos.y >= screen_y && pos.y < screen_bottom {
                return VisualRowHit::Row(row_index);
            }
        }
        VisualRowHit::Miss
    }
}

#[cfg(test)]
mod tests {
    use ratatui::layout::Position;
    use ratatui::layout::Rect;

    use super::VisualRowHit;
    use super::VisualRowHitLayout;
    use super::VisualRowSpan;

    #[test]
    fn projection_skips_scrolled_rows_and_keeps_first_hit_and_exclusive_bottom() {
        let spans = [
            VisualRowSpan {
                start_y: 0,
                height:  2,
            },
            VisualRowSpan {
                start_y: 2,
                height:  2,
            },
            VisualRowSpan {
                start_y: 2,
                height:  3,
            },
            VisualRowSpan {
                start_y: 4,
                height:  1,
            },
        ];
        let layout = VisualRowHitLayout {
            content_area:  Rect::new(4, 5, 10, 3),
            scroll_offset: 2,
            spans:         &spans,
        };
        assert_eq!(layout.hit_test(Position::new(4, 5)), VisualRowHit::Row(1));
        assert_eq!(layout.hit_test(Position::new(13, 6)), VisualRowHit::Row(1));
        assert_eq!(layout.hit_test(Position::new(4, 7)), VisualRowHit::Row(2));
        assert_eq!(layout.hit_test(Position::new(3, 5)), VisualRowHit::Miss);
        assert_eq!(layout.hit_test(Position::new(14, 5)), VisualRowHit::Miss);
        assert_eq!(layout.hit_test(Position::new(4, 8)), VisualRowHit::Miss);
    }

    #[test]
    fn oversized_offsets_and_heights_keep_the_original_saturating_fallbacks() {
        let spans = [
            VisualRowSpan {
                start_y: usize::MAX,
                height:  usize::MAX,
            },
            VisualRowSpan {
                start_y: 0,
                height:  usize::MAX,
            },
        ];
        let layout = VisualRowHitLayout {
            content_area:  Rect::new(1, 1, 5, 3),
            scroll_offset: 0,
            spans:         &spans,
        };
        assert_eq!(layout.hit_test(Position::new(1, 1)), VisualRowHit::Row(1));
        assert_eq!(layout.hit_test(Position::new(1, 2)), VisualRowHit::Miss);
        let scrolled = VisualRowHitLayout {
            scroll_offset: usize::MAX,
            ..layout
        };
        assert_eq!(scrolled.hit_test(Position::new(1, 1)), VisualRowHit::Miss);
    }
}
