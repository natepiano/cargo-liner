use unicode_width::UnicodeWidthChar;
use unicode_width::UnicodeWidthStr;

const ELLIPSIS: &str = "…";
const ELLIPSIS_WIDTH: usize = 1;
/// All items included in the body — the toast renderer truncates based on
/// allocated space and shows (+N more) as needed.
const MAX_VISIBLE_ITEMS: usize = usize::MAX;

/// Format a list of items for toast display within `max_width` columns.
///
/// - 1 item: returned as-is (the renderer wraps; if it exceeds two lines the second line is
///   truncated with `…` by the renderer)
/// - 2+ items: one per line, each truncated with `…` if too wide
/// - If extras remain beyond the visible items, the last visible line uses [`truncate_with_suffix`]
///   to guarantee `(+ N others)` fits
pub fn format_toast_items(items: &[&str], max_width: usize) -> String {
    match items.len() {
        0 => String::new(),
        1 => format_single(items[0], max_width),
        _ => format_multiple(items, max_width),
    }
}

/// Single item: wrap up to two lines. If the second line still overflows,
/// truncate it with ellipsis.
fn format_single(item: &str, max_width: usize) -> String {
    let width = UnicodeWidthStr::width(item);
    if width <= max_width {
        return item.to_string();
    }
    // Fits on two lines?
    if width <= max_width * 2 {
        return item.to_string();
    }
    // Truncate to two lines worth, with ellipsis on the second.
    let target = max_width * 2 - ELLIPSIS_WIDTH;
    truncate_to_width(item, target, true)
}

/// Show up to `MAX_VISIBLE` items, one per line. If more exist, the
/// last visible line gets a `(+ N others)` suffix.
fn format_multiple(items: &[&str], max_width: usize) -> String {
    let visible = items.len();
    let extra_count = items.len().saturating_sub(MAX_VISIBLE_ITEMS);
    let mut lines = Vec::with_capacity(visible);

    for (i, &item) in items.iter().take(visible).enumerate() {
        let is_last_visible = i == visible - 1;
        if is_last_visible && extra_count > 0 {
            let suffix = format!("(+ {extra_count} others)");
            lines.push(truncate_with_suffix(item, &suffix, max_width));
        } else {
            lines.push(truncate_ellipsis(item, max_width));
        }
    }

    lines.join("\n")
}

/// Truncate `text` to fit within `max_width` Unicode columns.
/// If truncated, appends `…`.
fn truncate_ellipsis(text: &str, max_width: usize) -> String {
    if UnicodeWidthStr::width(text) <= max_width {
        return text.to_string();
    }
    truncate_to_width(text, max_width.saturating_sub(ELLIPSIS_WIDTH), true)
}

/// Fit `text` and `suffix` on one line within `max_width`.
/// Returns `"text suffix"` if it fits, otherwise `"tex… suffix"`.
fn truncate_with_suffix(text: &str, suffix: &str, max_width: usize) -> String {
    let suffix_width = UnicodeWidthStr::width(suffix);
    let gap = 1; // space between text and suffix
    let full_width = UnicodeWidthStr::width(text) + gap + suffix_width;

    if full_width <= max_width {
        return format!("{text} {suffix}");
    }

    // How much room is left for text (+ ellipsis)?
    let text_budget = max_width.saturating_sub(suffix_width + gap + ELLIPSIS_WIDTH);
    if text_budget == 0 {
        // Suffix alone fills the line.
        return suffix.to_string();
    }

    let truncated = truncate_to_width(text, text_budget, true);
    format!("{truncated} {suffix}")
}

/// Truncate `text` to at most `target_width` display columns.
/// If `add_ellipsis` is true and text was truncated, appends `…`.
fn truncate_to_width(text: &str, target_width: usize, add_ellipsis: bool) -> String {
    let mut out = String::new();
    let mut used = 0usize;
    for ch in text.chars() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + ch_width > target_width {
            if add_ellipsis {
                out.push_str(ELLIPSIS);
            }
            return out;
        }
        out.push(ch);
        used += ch_width;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn width() -> usize { usize::from(crate::tui::constants::TOAST_WIDTH.saturating_sub(2)) }

    #[test]
    fn single_short_item_unchanged() {
        let result = format_toast_items(&["~/rust/foo"], width());
        assert_eq!(result, "~/rust/foo");
    }

    #[test]
    fn single_item_wraps_to_two_lines() {
        // 70 chars, fits in 2 × 58 = 116
        let long = "~/rust/this-is-a-very-long-project-name-that-exceeds-the-toast-width";
        let result = format_toast_items(&[long], width());
        assert_eq!(result, long);
    }

    #[test]
    fn single_item_exceeding_two_lines_truncated() {
        let very_long = "a".repeat(200);
        let result = format_toast_items(&[&very_long], width());
        assert!(result.ends_with('…'));
        assert!(UnicodeWidthStr::width(result.as_str()) <= width() * 2);
    }

    #[test]
    fn two_short_items_no_truncation() {
        let result = format_toast_items(&["~/rust/foo", "~/rust/bar"], width());
        assert_eq!(result, "~/rust/foo\n~/rust/bar");
    }

    #[test]
    fn two_items_first_long_truncated() {
        let long = "a".repeat(70);
        let result = format_toast_items(&[&long, "~/short"], width());
        let lines: Vec<&str> = result.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].ends_with('…'));
        assert!(UnicodeWidthStr::width(lines[0]) <= width());
        assert_eq!(lines[1], "~/short");
    }

    #[test]
    fn three_items_all_visible() {
        let result = format_toast_items(&["~/rust/a", "~/rust/b", "~/rust/c"], width());
        let lines: Vec<&str> = result.lines().collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], "~/rust/a");
        assert_eq!(lines[1], "~/rust/b");
        assert_eq!(lines[2], "~/rust/c");
    }

    #[test]
    fn four_items_shows_all() {
        let result = format_toast_items(&["~/rust/a", "~/rust/b", "~/rust/c", "~/rust/d"], width());
        let lines: Vec<&str> = result.lines().collect();
        assert_eq!(lines.len(), 4);
        for line in &lines {
            assert!(UnicodeWidthStr::width(*line) <= width());
        }
    }

    #[test]
    fn long_paths_are_truncated_with_ellipsis() {
        let long_path = "a".repeat(50);
        let result = format_toast_items(&[&long_path, &long_path, &long_path, "d", "e"], width());
        let lines: Vec<&str> = result.lines().collect();
        assert_eq!(lines.len(), 5);
        assert!(lines[0].contains('…'));
        for line in &lines {
            assert!(UnicodeWidthStr::width(*line) <= width());
        }
    }

    #[test]
    fn large_item_count_shows_all() {
        let items: Vec<&str> = (0..103).map(|_| "~/path").collect();
        let result = format_toast_items(&items, width());
        assert_eq!(result.lines().count(), 103);
    }

    #[test]
    fn exact_width_no_truncation() {
        let exact = "a".repeat(width());
        let result = format_toast_items(&[&exact[..]], width());
        assert_eq!(result, exact);
    }

    #[test]
    fn truncate_ellipsis_short() {
        assert_eq!(truncate_ellipsis("hello", 10), "hello");
    }

    #[test]
    fn truncate_ellipsis_exact() {
        assert_eq!(truncate_ellipsis("hello", 5), "hello");
    }

    #[test]
    fn truncate_ellipsis_long() {
        let result = truncate_ellipsis("hello world", 8);
        assert_eq!(result, "hello w…");
        assert!(UnicodeWidthStr::width(result.as_str()) <= 8);
    }

    #[test]
    fn truncate_with_suffix_fits() {
        let result = truncate_with_suffix("short", "(+ 1 others)", 30);
        assert_eq!(result, "short (+ 1 others)");
    }

    #[test]
    fn truncate_with_suffix_needs_truncation() {
        let result = truncate_with_suffix("very-long-project-name", "(+ 3 others)", 30);
        assert!(result.contains('…'));
        assert!(result.ends_with("(+ 3 others)"));
        assert!(UnicodeWidthStr::width(result.as_str()) <= 30);
    }

    #[test]
    fn truncate_with_suffix_only_fits_suffix() {
        let result = truncate_with_suffix("anything", "(+ 99 others)", 14);
        assert_eq!(result, "(+ 99 others)");
    }
}
