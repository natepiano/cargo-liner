//! The attract screen's share of an app's frame, drawn in the order its
//! layers stack.

use std::time::Instant;

use ratatui::Frame;
use ratatui::layout::Rect;

use super::controller::AttractGrid;
use super::controller::AttractWork;
use super::fade;
use super::host::AttractHost;
use super::updates::Updates;
use crate::FramePhase;
use crate::FrameProbe;
use crate::TileGridContents;

/// Draw the attract screen's layers in order: advance, the app's panes
/// (hidden and faded while the screen arrives or leaves, left out while
/// it has the terminal), the animation, and the backdrop notice.
///
/// `body` is the part of the frame the app's panes fill, above whatever
/// it draws along the bottom; the animation covers the whole frame.
/// `draw_panes` draws those panes with the [`TileGridContents`] this
/// frame calls for. Each stretch is timed through the app's
/// [`FrameProbe`] as its [`FramePhase`].
///
/// Everything the app draws over the attract screen -- its status line,
/// toasts and overlays -- comes after this call.
pub fn draw_attract_layers<A: AttractHost>(
    frame: &mut Frame,
    app: &mut A,
    body: Rect,
    work: AttractWork,
    updates: Updates,
    draw_panes: impl FnOnce(&mut Frame, &mut A, TileGridContents),
) {
    let area = frame.area();
    // Asked for, the attract screen replaces the grid rather than
    // sharing the terminal with it: a strip of characters drawn across
    // a grid of borders and tables reads as neither one thing nor the
    // other. Left to come on by itself, it draws over whatever is
    // there, which with nothing running is a summary cell and little
    // else. Between those two it draws over bare panes, which is what
    // it arrives over and leaves over. See [`Attract::advance`].
    let grid = A::Probe::timed(FramePhase::Advance, || {
        app.attract_mut()
            .advance(area, work, updates, Instant::now())
    });
    A::Probe::timed(FramePhase::Panes, || match grid {
        AttractGrid::Full => draw_panes(frame, app, TileGridContents::Shown),
        AttractGrid::Empty(faded) => {
            draw_panes(frame, app, TileGridContents::Hidden);
            fade::fade_to_background(frame.buffer_mut(), body, faded);
        },
        AttractGrid::Off => (),
    });
    // Over the grid rather than under it: the attract screen owns the
    // whole terminal while nothing is running, and the app's status line
    // goes back on top of it after this.
    A::Probe::timed(FramePhase::Band, || {
        app.attract().render(frame.buffer_mut(), area);
    });
    fade::draw_backdrop_notice::<A::Probe>(
        frame,
        app.attract().backdrop_notice(Instant::now()),
        body,
    );
}
