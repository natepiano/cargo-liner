use crate::tiles::draw::tests::*;
use crate::tiles::draw::*;

#[test]
fn idle_visual_appearance_change_requests_a_settled_frame() {
    let area = Rect::new(0, 0, 80, 24);
    let (mut grid, _) = settled_grid(area, &StubCells::new([1]));
    assert!(!grid.tick(), "settled paint is current");
    let tint = crate::focused_pane_tint_enabled();
    crate::set_focused_pane_tint(!tint);
    let changed = grid.tick();
    crate::set_focused_pane_tint(tint);
    assert!(changed, "tint is part of the painted visual state");
}

#[test]
fn idle_visual_theme_and_transparency_changes_need_a_body_paint() {
    let area = Rect::new(0, 0, 80, 24);
    let (mut grid, _) = settled_grid(area, &StubCells::new([1]));
    let transparent = crate::transparent_background();
    crate::set_transparent_background(!transparent);
    let transparency_changed = grid.tick();
    crate::set_transparent_background(transparent);
    assert!(transparency_changed, "transparency changes the drawn frame");
    let theme = crate::theme();
    let mut changed = (*theme).clone();
    changed.pane_chrome.active_title.color = Color::Rgb(17, 29, 43);
    crate::set_active_theme(std::sync::Arc::new(changed));
    let theme_changed = grid.tick();
    crate::set_active_theme(theme);
    assert!(theme_changed, "theme changes the drawn frame");
    assert!(!grid.tick(), "polls did not replace the original paint");
}
