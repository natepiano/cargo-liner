use super::HashMap;
use super::OnceLock;
use super::Rect;
use super::TileContent;
use super::draw;
use super::frame_inner;
/// One answer from a cell at a content-area size considered this frame.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct FrameContentRowUsage {
    /// Width of the content area inspected.
    width:             u16,
    /// Height of the content area inspected.
    pub(super) height: u16,
    /// Rows the content draws there.
    pub(super) drawn:  u16,
    /// Whether the allocation is filled or contains the whole value.
    pub(super) fit:    FrameHeightFit,
    /// Rows retained before a column mate may use the rest.
    retained:          OnceLock<u16>,
}

impl FrameContentRowUsage {
    fn retained_rows(&self) -> RetainedRows {
        self.retained
            .get()
            .map_or(RetainedRows::Unmeasured, |rows| {
                RetainedRows::Measured(*rows)
            })
    }
}

/// Whether an allocation has a filled span within its retained height bounds.
pub(super) enum FilledSpan {
    /// The content fills this span or is whole there.
    Fits(u16),
    /// No filled span fits the permitted interval.
    NoFit,
}

/// Largest filled span no greater than the requested one.
pub(super) fn measured_filled_span<Id: Clone + Eq>(
    row_usage: &mut Vec<FrameCellRowUsage<Id>>,
    content: &TileContent<Id>,
    rect: Rect,
    mut span: u16,
    minimum: u16,
    rows_drawn: &mut impl FnMut(&TileContent<Id>, Rect) -> u16,
    rows_kept: &mut impl FnMut(&TileContent<Id>, Rect) -> u16,
) -> FilledSpan {
    while span >= minimum {
        let area = draw::content_area(frame_inner(Rect {
            height: span.saturating_add(1),
            ..rect
        }));
        measure_frame_row_use(row_usage, content, area, rows_drawn, rows_kept);
        if frame_height_fit(row_usage, content, area) != FrameHeightFit::LeavesGap {
            return FilledSpan::Fits(span);
        }
        let next = frame_painted_rows(row_usage, content, area)
            .saturating_add(span.saturating_sub(area.height));
        if next >= span {
            break;
        }
        span = next;
    }
    FilledSpan::NoFit
}

/// Whether this allocation has supplied its retained-row answer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RetainedRows {
    /// A motion-only probe has not needed the answer.
    Unmeasured,
    /// The settled layout gathered the answer with its painted rows.
    Measured(u16),
}

/// What the current frame's content does with one candidate height.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FrameHeightFit {
    /// Content reaches the allocation's last row.
    Filled,
    /// Content ends sooner because the whole value is visible.
    Whole,
    /// Content leaves a gap while later rows remain hidden.
    LeavesGap,
}

/// Row-usage probes for one cell during the current frame.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct FrameCellRowUsage<Id> {
    /// The content whose allocations were inspected.
    pub(super) content: TileContent<Id>,
    /// Answers at only the sizes this frame considered.
    pub(super) rows:    FrameCellRowAnswers,
}

/// Frame answers indexed by width and height, retaining only requested sizes.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct FrameCellRowAnswers {
    first_width:        FrameWidthRowAnswers,
    other_widths:       HashMap<u16, FrameHeightRowAnswers>,
    pub(super) answers: Vec<FrameContentRowUsage>,
}

/// The first measured width avoids a map lookup on the common frame path.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
enum FrameWidthRowAnswers {
    #[default]
    Unmeasured,
    Measured {
        width: u16,
        rows:  FrameHeightRowAnswers,
    },
}

/// Index of a fully measured allocation in the frame's answer sequence.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum FrameRowIndex {
    #[default]
    Unmeasured,
    Measured(usize),
}

/// Finite heights are indexed directly; the whole-content probe stands apart.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct FrameHeightRowAnswers {
    finite: Vec<FrameRowIndex>,
    whole:  FrameRowIndex,
}

impl FrameHeightRowAnswers {
    pub(super) fn at(&self, height: u16) -> FrameRowIndex {
        if height == u16::MAX {
            self.whole
        } else {
            self.finite
                .get(usize::from(height))
                .copied()
                .unwrap_or_default()
        }
    }

    pub(super) fn record(&mut self, height: u16, index: usize) {
        if height == u16::MAX {
            self.whole = FrameRowIndex::Measured(index);
        } else {
            let height = usize::from(height);
            if self.finite.len() <= height {
                self.finite.resize(height + 1, FrameRowIndex::Unmeasured);
            }
            self.finite[height] = FrameRowIndex::Measured(index);
        }
    }
}

/// Whether this frame has measured a content allocation.
pub(super) enum FrameRowAnswer<'a> {
    Unmeasured,
    Measured(&'a FrameContentRowUsage),
}

impl FrameCellRowAnswers {
    fn index(&self, area: Rect) -> FrameRowIndex {
        match &self.first_width {
            FrameWidthRowAnswers::Measured { width, rows } if *width == area.width => {
                rows.at(area.height)
            },
            _ => self
                .other_widths
                .get(&area.width)
                .map_or(FrameRowIndex::Unmeasured, |rows| rows.at(area.height)),
        }
    }

    pub(super) fn answer(&self, area: Rect) -> FrameRowAnswer<'_> {
        match self.index(area) {
            FrameRowIndex::Unmeasured => FrameRowAnswer::Unmeasured,
            FrameRowIndex::Measured(index) => FrameRowAnswer::Measured(&self.answers[index]),
        }
    }

    pub(super) fn record(&mut self, answer: FrameContentRowUsage) -> usize {
        let index = self.answers.len();
        if self.first_width == FrameWidthRowAnswers::Unmeasured {
            self.first_width = FrameWidthRowAnswers::Measured {
                width: answer.width,
                rows:  FrameHeightRowAnswers::default(),
            };
        }
        let rows = match &mut self.first_width {
            FrameWidthRowAnswers::Measured { width, rows } if *width == answer.width => rows,
            _ => self.other_widths.entry(answer.width).or_default(),
        };
        rows.record(answer.height, index);
        self.answers.push(answer);
        index
    }

    pub(super) fn values(&self) -> impl Iterator<Item = &FrameContentRowUsage> {
        self.answers.iter()
    }
}

/// How one measured content allocation fits, with an unmeasured size rejected.
pub(super) fn frame_height_fit<Id: Eq>(
    usage: &[FrameCellRowUsage<Id>],
    content: &TileContent<Id>,
    area: Rect,
) -> FrameHeightFit {
    match usage
        .iter()
        .find(|cell| &cell.content == content)
        .map(|cell| cell.rows.answer(area))
    {
        Some(FrameRowAnswer::Measured(rows)) => rows.fit,
        _ => FrameHeightFit::LeavesGap,
    }
}

/// The retained-row answer for one measured content allocation.
pub(super) fn frame_retained_rows<Id: Eq>(
    usage: &[FrameCellRowUsage<Id>],
    content: &TileContent<Id>,
    area: Rect,
) -> RetainedRows {
    match usage
        .iter()
        .find(|cell| &cell.content == content)
        .map(|cell| cell.rows.answer(area))
    {
        Some(FrameRowAnswer::Measured(rows)) => rows.retained_rows(),
        _ => RetainedRows::Unmeasured,
    }
}

/// Painted rows at one recorded allocation, keeping unmeasured rows in place.
pub(super) fn frame_painted_rows<Id: Eq>(
    usage: &[FrameCellRowUsage<Id>],
    content: &TileContent<Id>,
    area: Rect,
) -> u16 {
    match usage
        .iter()
        .find(|cell| &cell.content == content)
        .map(|cell| cell.rows.answer(area))
    {
        Some(FrameRowAnswer::Measured(rows)) => rows.drawn,
        _ => area.height,
    }
}

/// Frame-cache index for a content identity, retaining stable cell order.
fn frame_cell_index<Id: Clone + Eq>(
    usage: &mut Vec<FrameCellRowUsage<Id>>,
    content: &TileContent<Id>,
) -> usize {
    usage
        .iter()
        .position(|cell| &cell.content == content)
        .unwrap_or_else(|| {
            usage.push(FrameCellRowUsage {
                content: content.clone(),
                rows:    FrameCellRowAnswers::default(),
            });
            usage.len() - 1
        })
}

/// Record one content-area answer, reusing an answer already gathered this frame.
#[cfg(test)]
pub(super) fn measure_frame_row_usage<Id: Clone + Eq>(
    usage: &mut Vec<FrameCellRowUsage<Id>>,
    content: &TileContent<Id>,
    area: Rect,
    rows_drawn: &mut impl FnMut(&TileContent<Id>, Rect) -> u16,
) -> u16 {
    let index = frame_cell_index(usage, content);
    measured_content_rows(&mut usage[index].rows, content, area, rows_drawn).drawn
}

/// Supply one stored answer, including a whole-content answer when needed.
fn measured_content_rows<'a, Id>(
    answers: &'a mut FrameCellRowAnswers,
    content: &TileContent<Id>,
    area: Rect,
    rows_drawn: &mut impl FnMut(&TileContent<Id>, Rect) -> u16,
) -> &'a FrameContentRowUsage {
    let index = match answers.index(area) {
        FrameRowIndex::Measured(index) => index,
        FrameRowIndex::Unmeasured => {
            let drawn = rows_drawn(content, area).min(area.height);
            let fit = if drawn == area.height {
                FrameHeightFit::Filled
            } else if area.height == u16::MAX {
                FrameHeightFit::Whole
            } else {
                let whole_area = Rect {
                    height: u16::MAX,
                    ..area
                };
                let whole = match answers.answer(whole_area) {
                    FrameRowAnswer::Measured(rows) => rows.drawn,
                    FrameRowAnswer::Unmeasured => {
                        let whole = rows_drawn(content, whole_area);
                        answers.record(FrameContentRowUsage {
                            width:    area.width,
                            height:   u16::MAX,
                            drawn:    whole,
                            fit:      FrameHeightFit::Whole,
                            retained: OnceLock::new(),
                        });
                        whole
                    },
                };
                if drawn >= whole {
                    FrameHeightFit::Whole
                } else {
                    FrameHeightFit::LeavesGap
                }
            };
            answers.record(FrameContentRowUsage {
                width: area.width,
                height: area.height,
                drawn,
                fit,
                retained: OnceLock::new(),
            })
        },
    };
    &answers.answers[index]
}

/// Record painted and retained rows together, reusing this frame's answer.
pub(super) fn measure_frame_row_use<Id: Clone + Eq>(
    usage: &mut Vec<FrameCellRowUsage<Id>>,
    content: &TileContent<Id>,
    area: Rect,
    rows_drawn: &mut impl FnMut(&TileContent<Id>, Rect) -> u16,
    rows_kept: &mut impl FnMut(&TileContent<Id>, Rect) -> u16,
) -> u16 {
    let index = frame_cell_index(usage, content);
    measure_cell_retained_rows(&mut usage[index], area, rows_drawn, rows_kept)
}

/// Record both purposes directly through a known frame cell index.
pub(super) fn measure_cell_retained_rows<Id>(
    cell: &mut FrameCellRowUsage<Id>,
    area: Rect,
    rows_drawn: &mut impl FnMut(&TileContent<Id>, Rect) -> u16,
    rows_kept: &mut impl FnMut(&TileContent<Id>, Rect) -> u16,
) -> u16 {
    let rows = measured_content_rows(&mut cell.rows, &cell.content, area, rows_drawn);
    *rows
        .retained
        .get_or_init(|| rows_kept(&cell.content, area).min(area.height))
}
