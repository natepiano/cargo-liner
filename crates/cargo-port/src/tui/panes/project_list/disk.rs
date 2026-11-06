use std::path::Path;

use ratatui::style::Style;
use tui_pane::error_color;
use tui_pane::label_color;
use tui_pane::success_color;
use tui_pane::warning_color;

use crate::project::RootItem;
use crate::scan::StorageHeadroom;
use crate::tui::project_list::ProjectPathLookup::Found;
use crate::tui::project_list::ProjectPathLookup::StaleRoute;
use crate::tui::project_list::ProjectPathLookup::UnknownPath;
use crate::tui::project_list_state::DiskCacheKey;
use crate::tui::project_list_state::ProjectList;
use crate::tui::render;

pub(super) fn disk_style(bytes: Option<u64>, sorted_values: &[u64]) -> Style {
    disk_color(disk_percentile(bytes, sorted_values))
}

/// Compute a logarithmic position for `bytes` within the smallest..largest
/// span of `sorted_values` (0.0 to 1.0). The smallest disk value maps to 0.0,
/// the largest to 1.0, and the geometric midpoint to 0.5, so equal ratios in
/// size cover equal distance on the color gradient. `None` when there is no
/// span to interpolate against (empty, or all values identical).
#[allow(
    clippy::cast_precision_loss,
    reason = "display-only — byte magnitudes to f64 for log interpolation"
)]
pub(super) fn disk_percentile(bytes: Option<u64>, sorted_values: &[u64]) -> Option<f64> {
    let bytes = bytes?;
    let min = *sorted_values.first()?;
    let max = *sorted_values.last()?;
    let lo = (min.max(1) as f64).log2();
    let hi = (max.max(1) as f64).log2();
    if hi <= lo {
        return None;
    }
    let pos = ((bytes.max(1) as f64).log2() - lo) / (hi - lo);
    Some(pos.clamp(0.0, 1.0))
}

/// Compute a color for a disk value by interpolating the active
/// theme's three `disk_usage` stops: low (smallest) → mid → high
/// (largest). Modifiers on the theme stops are ignored.
pub(super) fn disk_color(percentile: Option<f64>) -> Style {
    let Some(pos) = percentile else {
        return Style::default().fg(label_color());
    };

    let theme = tui_pane::theme();
    let stops = &theme.disk_usage;
    let (start, end, t) = if pos < 0.5 {
        (stops.low.color, stops.mid.color, pos * 2.0)
    } else {
        (stops.mid.color, stops.high.color, (pos - 0.5) * 2.0)
    };
    Style::default().fg(tui_pane::interpolate_terminal_color(start, end, t))
}

/// Style for the Available row under the Σ disk total, banded by how much
/// free space the projects' volume has left: the theme's success color for
/// [`StorageHeadroom::Ample`], warning for [`StorageHeadroom::Low`], error
/// for [`StorageHeadroom::Critical`].
pub(super) fn headroom_style(headroom: StorageHeadroom) -> Style {
    let color = match headroom {
        StorageHeadroom::Ample => success_color(),
        StorageHeadroom::Low => warning_color(),
        StorageHeadroom::Critical => error_color(),
    };
    Style::default().fg(color)
}

pub(super) fn formatted_disk(projects: &ProjectList, path: &Path) -> String {
    let bytes = (match projects.at_path(path) {
        Found(project) => project.disk_usage_bytes,
        UnknownPath | StaleRoute => None,
    })
    .unwrap_or(0);
    render::format_bytes(bytes)
}

pub(super) fn formatted_disk_for_item(item: &RootItem) -> String {
    item.disk_usage_bytes()
        .map_or_else(|| render::format_bytes(0), render::format_bytes)
}

// Body of `ProjectListPane::render`. Same pattern as the
// other Phase-4 absorptions: typed parameters through `ctx`.
// ── Disk-cache ───────────────────────────────────────────────────────
//
// Builds sorted disk-usage values for the rows currently rendered in the
// Project pane. Expansion and collapse update the gradient.

pub(super) fn compute_disk_cache(key: &DiskCacheKey) -> Vec<u64> {
    let mut sorted = key.visible_bytes.clone();
    sorted.sort_unstable();
    sorted
}
