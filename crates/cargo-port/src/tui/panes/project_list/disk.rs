use std::collections::HashMap;
use std::path::Path;

use ratatui::style::Color;
use ratatui::style::Style;
use tui_pane::label_color;

use crate::project::MemberGroup;
use crate::project::ProjectFields;
use crate::project::RootItem;
use crate::project::RustProject;
use crate::project::VendoredPackage;
use crate::tui::project_list::ProjectList;
use crate::tui::render;

/// Compute the percentile rank of `bytes` within `sorted_values` (0.0 to 1.0).
#[allow(
    clippy::cast_precision_loss,
    reason = "display-only — index-to-float ratio for color interpolation"
)]
pub(super) fn disk_percentile(bytes: Option<u64>, sorted_values: &[u64]) -> Option<f64> {
    let bytes = bytes?;
    if sorted_values.len() <= 1 {
        return None;
    }
    let rank = sorted_values
        .iter()
        .position(|&v| v >= bytes)
        .unwrap_or(sorted_values.len() - 1);
    Some(rank as f64 / (sorted_values.len() - 1) as f64)
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
// Builds sorted disk-usage values for the Disk column. The scale includes
// top-level rows and collapsed child rows so equal byte counts receive the
// same color anywhere in the project list.

pub(super) fn compute_disk_cache(entries: &ProjectList) -> (Vec<u64>, HashMap<usize, Vec<u64>>) {
    let mut all_sorted = Vec::new();
    let mut child_presence = Vec::new();
    for entry in entries {
        if let Some(bytes) = entry.root_item.disk_usage_bytes() {
            all_sorted.push(bytes);
        }

        let mut values = Vec::new();
        collect_child_disk_values(&entry.root_item, &mut values);
        all_sorted.extend(values.iter().copied());
        child_presence.push(!values.is_empty());
    }
    all_sorted.sort_unstable();

    let mut child_sorted = HashMap::new();
    for (ni, has_children) in child_presence.into_iter().enumerate() {
        if has_children {
            child_sorted.insert(ni, all_sorted.clone());
        }
    }

    (all_sorted, child_sorted)
}

fn collect_child_disk_values(item: &RootItem, values: &mut Vec<u64>) {
    match item {
        RootItem::Rust(RustProject::Workspace(ws)) => {
            collect_member_group_disk(ws.groups(), values);
            collect_vendored_disk(ws.vendored(), values);
        },
        RootItem::Rust(RustProject::Package(pkg)) => {
            collect_vendored_disk(pkg.vendored(), values);
        },
        RootItem::NonRust(_) => {},
        RootItem::Worktrees(group) => {
            for entry in group.iter_entries() {
                if let Some(bytes) = entry.disk_usage_bytes() {
                    values.push(bytes);
                }
                if let RustProject::Workspace(ws) = entry {
                    collect_member_group_disk(ws.groups(), values);
                }
                collect_vendored_disk(entry.rust_info().vendored(), values);
            }
        },
    }
    collect_project_list_entry_disk(item.submodules(), values);
}

fn collect_member_group_disk(groups: &[MemberGroup], values: &mut Vec<u64>) {
    for group in groups {
        for member in group.members() {
            if let Some(bytes) = member.disk_usage_bytes() {
                values.push(bytes);
            }
            collect_vendored_disk(member.vendored(), values);
        }
    }
}

fn collect_vendored_disk(vendored: &[VendoredPackage], values: &mut Vec<u64>) {
    for project in vendored {
        if let Some(bytes) = project.disk_usage_bytes() {
            values.push(bytes);
        }
    }
}

fn collect_project_list_entry_disk(
    entries: &[impl crate::project::ProjectFields],
    values: &mut Vec<u64>,
) {
    for entry in entries {
        if let Some(bytes) = entry.info().disk_usage_bytes {
            values.push(bytes);
        }
    }
}
