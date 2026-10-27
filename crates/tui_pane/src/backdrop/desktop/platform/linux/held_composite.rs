use std::sync::Mutex;
use std::time::Instant;

use ratatui::style::Color;

use super::compose;
use super::compose::Layout;
use super::constants::COMPOSITE_HOLD;
use super::display::Output;
use super::window;
use crate::backdrop::desktop::Metrics;
use crate::backdrop::desktop::reduction;

/// The composite the animation is being drawn from, held until a frame that draws nothing over
/// the grid may replace it.
static HELD_COMPOSITE: Mutex<Option<HeldComposite>> = Mutex::new(None);

/// Inputs that decide whether a held composite still describes the display.
#[derive(Clone, Eq, PartialEq)]
struct CompositeKey {
    /// Terminal geometry used to size each colour cell.
    metrics:     Metrics,
    /// The output's top-left logical coordinate, encoded without floating-point equality.
    origin_bits: (u64, u64),
    /// Physical dimensions of the output.
    output:      (u32, u32),
    /// The output scale encoded without floating-point equality.
    scale_bits:  u64,
    /// The windows the picture was assembled from, and where they stood.
    layout:      Layout,
}

impl CompositeKey {
    /// The key one output and one arrangement of windows answer to.
    const fn of(metrics: Metrics, output: &Output, layout: Layout) -> Self {
        Self {
            metrics,
            origin_bits: (output.origin.0.to_bits(), output.origin.1.to_bits()),
            output: output.size,
            scale_bits: output.scale.to_bits(),
            layout,
        }
    }
}

/// One composite of the display, already reduced to terminal-sized colour cells.
struct HeldComposite {
    /// Inputs that produced this grid.
    key:      CompositeKey,
    /// Cells across and down.
    grid:     (u16, u16),
    /// Row-major colours for the grid.
    colors:   Vec<Color>,
    /// When the picture was assembled.
    taken_at: Instant,
}

/// Return the composite already held, or assemble and reduce a new one.
///
/// The stacking order is read every time, because it is what says whether the picture already
/// held still describes the display and it costs a few milliseconds against the round trip per
/// window a picture costs.
///
/// [`None`] wherever nothing can be captured, which leaves the caller reconstructing the
/// wallpaper.
pub(super) fn composited_display(
    metrics: Metrics,
    handle: u32,
    output: &Output,
) -> Option<(u16, u16, Vec<Color>)> {
    let uuid = window::uuid_of(handle)?;
    let layout = compose::layout_below(&uuid, output)?;
    let key = CompositeKey::of(metrics, output, layout.clone());
    let now = Instant::now();
    if let Ok(held) = HELD_COMPOSITE.lock()
        && let Some(held) = held.as_ref()
        && !composite_due(held, &key, now)
    {
        return Some((held.grid.0, held.grid.1, held.colors.clone()));
    }
    let composite = layout.capture()?;
    let cell = metrics.cell_points(output.scale / composite.ratio);
    let reduced =
        reduction::reduce_capture(composite.image.as_raw(), composite.image.dimensions(), cell)
            .ok()?;
    if let Ok(mut held) = HELD_COMPOSITE.lock() {
        *held = Some(HeldComposite {
            key,
            grid: (reduced.0, reduced.1),
            colors: reduced.2.clone(),
            taken_at: now,
        });
    }
    Some(reduced)
}

/// Whether a new composite is due.
///
/// Assembling one costs a capture of every window standing under this one, so it is not done per
/// frame: a composite stands for [`COMPOSITE_HOLD`] unless the display, the terminal's own
/// geometry, or the arrangement of the windows it was drawn from has changed under it, all of
/// which the key carries.
fn composite_due(held: &HeldComposite, key: &CompositeKey, now: Instant) -> bool {
    held.key != *key || now.duration_since(held.taken_at) >= COMPOSITE_HOLD
}
