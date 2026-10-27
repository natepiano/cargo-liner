//! Shared table navigation retains cargo-port's cursor and hit boundaries.

use ratatui::layout::Position;
use ratatui::layout::Rect;

use super::Viewport;
use super::table_row_at;
use crate::NavAction;
use crate::ScrollDirection;

#[test]
fn table_navigation_moves_by_rows_pages_and_endpoints_without_wrapping() {
    let mut viewport = Viewport::new();
    viewport.set_len(12);
    viewport.set_viewport_rows(5);
    for (action, expected) in [
        (NavAction::Up, 0),
        (NavAction::Down, 1),
        (NavAction::PageDown, 5),
        (NavAction::HalfPageDown, 7),
        (NavAction::End, 11),
        (NavAction::Down, 11),
        (NavAction::PageDown, 11),
        (NavAction::PageUp, 7),
        (NavAction::HalfPageUp, 5),
        (NavAction::Left, 4),
        (NavAction::Right, 5),
        (NavAction::Home, 0),
        (NavAction::PageUp, 0),
    ] {
        viewport.navigate(action);
        assert_eq!(viewport.pos(), expected, "{action:?}");
    }
    viewport.set_len(0);
    for action in [NavAction::Down, NavAction::End, NavAction::PageDown] {
        viewport.navigate(action);
        assert_eq!(viewport.pos(), 0, "empty table: {action:?}");
    }
}

#[test]
fn table_hit_rows_exclude_the_header_and_blank_tail_after_scrolling() {
    let mut viewport = Viewport::new();
    viewport.set_len(7);
    viewport.set_content_area(Rect::new(10, 20, 30, 6));
    viewport.set_scroll_offset(4);
    for (point, expected) in [
        (Position::new(10, 20), None),
        (Position::new(10, 21), Some(4)),
        (Position::new(39, 23), Some(6)),
        (Position::new(10, 24), None),
        (Position::new(9, 21), None),
        (Position::new(40, 21), None),
        (Position::new(10, 26), None),
    ] {
        assert_eq!(table_row_at(&viewport, point), expected, "{point:?}");
    }
}

#[test]
fn wheel_navigation_moves_only_the_viewport_under_the_pointer() {
    let mut left = Viewport::new();
    let mut right = Viewport::new();
    for viewport in [&mut left, &mut right] {
        viewport.set_len(4);
        viewport.set_pos(1);
    }
    left.set_content_area(Rect::new(2, 3, 10, 6));
    right.set_content_area(Rect::new(14, 3, 10, 6));
    let pointer = Position::new(15, 5);
    left.scroll_if_contains(pointer, ScrollDirection::Down);
    right.scroll_if_contains(pointer, ScrollDirection::Down);
    assert_eq!((left.pos(), right.pos()), (1, 2));
    right.scroll_if_contains(pointer, ScrollDirection::Up);
    assert_eq!(right.pos(), 1);
    right.scroll_if_contains(Position::new(24, 5), ScrollDirection::Down);
    assert_eq!(right.pos(), 1, "right edge is outside the viewport");
}
