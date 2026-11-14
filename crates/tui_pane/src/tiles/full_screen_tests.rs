//! The full-screen view keeps its tile identity, its motion clock and its clipping.

use std::error::Error;
use std::time::Duration;
use std::time::Instant;

use ratatui::layout::Rect;

use super::*;
use crate::PaneFrame;

type View = FullScreenView<u8, u16>;

const AREA: Rect = Rect::new(0, 0, 100, 40);
const TILE: Rect = Rect::new(50, 20, 50, 20);

fn placements(focused: &TileContent<u8>) -> Vec<TilePlacement<u8>> {
    [
        (TileContent::Summary, Rect::new(0, 0, 50, 20)),
        (TileContent::Group(7), Rect::new(50, 0, 50, 20)),
        (TileContent::Group(9), TILE),
    ]
    .into_iter()
    .map(|(content, rect)| TilePlacement {
        frame: PaneFrame::new(rect).with_focus(&content == focused),
        content,
    })
    .collect()
}

fn opened(now: Instant) -> View {
    let mut view = View::default();
    view.expand(now);
    view.resolve_opening(&placements(&TileContent::Group(9)), || 3);
    view
}

fn travel(view: &View, area: Rect) -> Result<(TileContent<u8>, CellTravel), Box<dyn Error>> {
    match view.travel(area, &placements(&TileContent::Group(9))) {
        FullScreenStep::Travel { content, travel } => Ok((content, travel)),
        step => Err(format!("the view must travel, got {step:?}").into()),
    }
}

#[test]
fn opening_resolves_the_focused_tile_and_its_policy() -> Result<(), Box<dyn Error>> {
    let now = Instant::now();
    let view = opened(now);
    let View::Expanding { cell, started } = &view else {
        return Err(format!("the view must expand, got {view:?}").into());
    };
    assert_eq!(cell.content(), &TileContent::Group(9));
    assert_eq!(cell.displayed(), TILE);
    assert_eq!(cell.policy(), &3);
    assert_eq!(*started, now);
    Ok(())
}

#[test]
fn opening_with_no_focused_tile_keeps_the_grid() {
    let mut view = View::default();
    view.expand(Instant::now());
    let mut unfocused = placements(&TileContent::Empty(0));
    unfocused.retain(|placement| !placement.frame.is_focused());
    view.resolve_opening(&unfocused, || 0);
    assert_eq!(view, View::Grid);
    assert!(!view.is_open());
}

#[test]
fn motion_reads_the_poll_clock_until_the_tile_settles() -> Result<(), Box<dyn Error>> {
    let now = Instant::now();
    let mut view = opened(now);
    let half = Duration::from_millis(100);
    assert_eq!(view.advance(now + half), Repaint::Needed);
    let (content, motion) = travel(&view, AREA)?;
    assert_eq!(content, TileContent::Group(9));
    assert_eq!(
        motion,
        CellTravel {
            from:    TILE,
            to:      AREA,
            elapsed: half,
        }
    );
    assert_eq!(
        view.settle(AREA, Repaint::NotNeeded),
        FullScreenSettle::Shown
    );
    assert!(matches!(view, View::Expanded { .. }));
    assert_eq!(view.advance(now + half * 2), Repaint::NotNeeded);
    assert_eq!(travel(&view, AREA)?.1.elapsed, Duration::MAX);
    Ok(())
}

#[test]
fn travel_clips_both_ends_to_the_pane() -> Result<(), Box<dyn Error>> {
    let view = opened(Instant::now());
    let smaller = Rect::new(0, 0, 80, 30);
    let (_, motion) = travel(&view, smaller)?;
    assert_eq!(motion.from, Rect::new(50, 20, 30, 10));
    assert_eq!(motion.to, smaller);
    Ok(())
}

#[test]
fn collapse_travels_back_to_the_grid_allocation_and_closes() -> Result<(), Box<dyn Error>> {
    let now = Instant::now();
    let mut view = opened(now);
    view.settle(AREA, Repaint::NotNeeded);
    let later = now + Duration::from_secs(1);
    view.collapse(later);
    let View::Collapsing { started, .. } = &view else {
        return Err(format!("the view must collapse, got {view:?}").into());
    };
    assert_eq!(*started, later);
    let (_, motion) = travel(&view, AREA)?;
    assert_eq!((motion.from, motion.to), (AREA, TILE));
    assert_eq!(view.settle(TILE, Repaint::Needed), FullScreenSettle::Shown);
    assert_eq!(
        view.settle(TILE, Repaint::NotNeeded),
        FullScreenSettle::Closed
    );
    assert_eq!(view, View::Grid);
    Ok(())
}

#[test]
fn expand_during_a_collapse_reverses_from_the_drawn_rectangle() -> Result<(), Box<dyn Error>> {
    let now = Instant::now();
    let mut view = opened(now);
    view.settle(AREA, Repaint::NotNeeded);
    view.collapse(now);
    let midway = Rect::new(25, 10, 75, 30);
    view.settle(midway, Repaint::Needed);
    let reverse = now + Duration::from_millis(50);
    view.expand(reverse);
    let View::Expanding { cell, started } = &view else {
        return Err(format!("the view must expand again, got {view:?}").into());
    };
    assert_eq!(*started, reverse);
    assert_eq!(cell.policy(), &3);
    assert_eq!(travel(&view, AREA)?.1.from, midway);
    Ok(())
}

#[test]
fn a_collapsing_tile_with_no_placement_leaves_the_view() {
    let now = Instant::now();
    let mut view = opened(now);
    view.collapse(now);
    let others = placements(&TileContent::Summary)
        .into_iter()
        .filter(|placement| placement.content != TileContent::Group(9))
        .collect::<Vec<_>>();
    assert_eq!(view.travel(AREA, &others), FullScreenStep::ContentLeft);
}

#[test]
fn collapse_of_an_unresolved_opening_returns_to_the_grid() {
    let mut view = View::default();
    view.expand(Instant::now());
    assert!(view.is_open());
    assert_eq!(view.travel(AREA, &[]), FullScreenStep::Inactive);
    view.collapse(Instant::now());
    assert_eq!(view, View::Grid);
}

#[test]
fn policy_state_belongs_to_the_resolved_tile() {
    let mut view = opened(Instant::now());
    if let Some(policy) = view.policy_mut() {
        *policy = 11;
    }
    assert_eq!(view.cell().map(FullScreenCell::policy), Some(&11));
    view.close();
    assert_eq!(view.policy_mut(), None);
}
