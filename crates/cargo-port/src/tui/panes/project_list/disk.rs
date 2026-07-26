use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;

use ratatui::style::Color;
use ratatui::style::Style;
use tui_pane::hover_focus_color;
use tui_pane::label_color;

use crate::project::RootItem;
use crate::tui::project_list::ProjectList;
use crate::tui::project_list::VisibleRow;
use crate::tui::render;

/// Number of largest distinct visible disk values highlighted by
/// [`disk_style`].
const TOP_DISK_COUNT: usize = 3;

/// Style for a disk value: the theme's high (red) `disk_usage` stop on a
/// subtle focus tint when `bytes` is among the largest [`TOP_DISK_COUNT`]
/// distinct values in `sorted_values`, otherwise the logarithmic gradient
/// from [`disk_color`].
pub(super) fn disk_style(bytes: Option<u64>, sorted_values: &[u64]) -> Style {
    if let Some(bytes) = bytes
        && is_top_disk_value(bytes, sorted_values)
    {
        return Style::default()
            .fg(tui_pane::theme().disk_usage.high.color)
            .bg(hover_focus_color());
    }
    disk_color(disk_percentile(bytes, sorted_values))
}

/// Whether `bytes` is at or above the [`TOP_DISK_COUNT`]-th largest distinct
/// value in the ascending `sorted_values`. When fewer than [`TOP_DISK_COUNT`]
/// distinct values exist, the threshold is the smallest, so every visible
/// value qualifies.
fn is_top_disk_value(bytes: u64, sorted_values: &[u64]) -> bool {
    let mut distinct = 0usize;
    let mut prev = None;
    let mut threshold = None;
    for &value in sorted_values.iter().rev() {
        if prev != Some(value) {
            distinct += 1;
            prev = Some(value);
            threshold = Some(value);
            if distinct == TOP_DISK_COUNT {
                break;
            }
        }
    }
    threshold.is_some_and(|t| bytes >= t)
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
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "values are clamped to 0.0..=255.0 before cast"
)]
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
    let (sr, sg, sb) = rgb_channels(start);
    let (er, eg, eb) = rgb_channels(end);
    let lerp = |a: u8, b: u8| -> u8 {
        let af = f64::from(a);
        let bf = f64::from(b);
        (bf - af).mul_add(t, af).clamp(0.0, 255.0) as u8
    };

    Style::default().fg(Color::Rgb(lerp(sr, er), lerp(sg, eg), lerp(sb, eb)))
}

/// Extract RGB channels from a [`Color`], converting named/indexed
/// colors into their nearest ANSI 24-bit equivalents. Used by
/// [`disk_color`] so a theme can supply a named color (e.g. `Green`)
/// as a gradient stop and still interpolate smoothly.
const fn rgb_channels(color: Color) -> (u8, u8, u8) {
    match color {
        Color::Rgb(r, g, b) => (r, g, b),
        Color::Reset | Color::Black => (0, 0, 0),
        Color::Red => (170, 0, 0),
        Color::Green => (0, 170, 0),
        Color::Yellow => (170, 85, 0),
        Color::Blue => (0, 0, 170),
        Color::Magenta => (170, 0, 170),
        Color::Cyan => (0, 170, 170),
        Color::Gray | Color::Indexed(_) => (170, 170, 170),
        Color::DarkGray => (85, 85, 85),
        Color::LightRed => (255, 85, 85),
        Color::LightGreen => (85, 255, 85),
        Color::LightYellow => (255, 255, 85),
        Color::LightBlue => (85, 85, 255),
        Color::LightMagenta => (255, 85, 255),
        Color::LightCyan => (85, 255, 255),
        Color::White => (255, 255, 255),
    }
}

pub(super) fn formatted_disk(projects: &ProjectList, path: &Path) -> String {
    let bytes = projects
        .at_path(path)
        .and_then(|project| project.disk_usage_bytes)
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
// Project pane. Expansion and collapse therefore update both the gradient and
// the three emphasized values.

pub(super) fn compute_disk_cache(entries: &ProjectList) -> (Vec<u64>, HashMap<usize, Vec<u64>>) {
    let mut all_sorted: Vec<u64> = entries
        .visible_rows()
        .iter()
        .filter_map(|row| entries.visible_row_disk_usage(*row))
        .collect();
    all_sorted.sort_unstable();

    let child_nodes: HashSet<usize> = entries
        .visible_rows()
        .iter()
        .filter_map(child_row_node_index)
        .collect();
    let mut child_sorted = HashMap::new();
    for node_index in child_nodes {
        child_sorted.insert(node_index, all_sorted.clone());
    }

    (all_sorted, child_sorted)
}

const fn child_row_node_index(row: &VisibleRow) -> Option<usize> {
    match row {
        VisibleRow::Root { .. } | VisibleRow::GroupHeader { .. } => None,
        VisibleRow::Member { node_index, .. }
        | VisibleRow::MemberVendored { node_index, .. }
        | VisibleRow::Vendored { node_index, .. }
        | VisibleRow::WorktreeEntry { node_index, .. }
        | VisibleRow::WorktreeGroupHeader { node_index, .. }
        | VisibleRow::WorktreeMember { node_index, .. }
        | VisibleRow::WorktreeMemberVendored { node_index, .. }
        | VisibleRow::WorktreeVendored { node_index, .. }
        | VisibleRow::Submodule { node_index, .. } => Some(*node_index),
    }
}
