//! Assembling what stands underneath this terminal's window.
//!
//! macOS asks the window server for everything below one window and
//! gets it back as a single picture. `KWin` has no such call -- every
//! capture it offers is a whole screen, a whole workspace, or one named
//! window -- so the same picture is assembled here: read the stacking
//! order, take the windows below this one on this display, and draw
//! them over the desktop in the order `KWin` stacks them.
//!
//! Two `KWin` facilities do the work, and they are permitted very
//! differently. `org.kde.kwin.Scripting` is open to anyone on the
//! session bus and reports the stacking order. `org.kde.KWin.ScreenShot2`
//! takes the pictures, and `KWin` answers it only for a process whose
//! desktop entry names the interface in
//! `X-KDE-DBUS-Restricted-Interfaces`; without that entry every capture
//! comes back `org.kde.KWin.ScreenShot2.Error.NoAuthorized` and the
//! caller reconstructs Plasma's wallpaper instead.

use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::Read;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::sync::mpsc::Receiver;
use std::sync::mpsc::Sender;
use std::sync::mpsc::channel;

use image::RgbaImage;
use image::imageops;
use zbus::blocking::Connection;
use zbus::blocking::Proxy;
use zbus::blocking::connection::Builder;
use zbus::interface;
use zbus::zvariant::Fd;
use zbus::zvariant::OwnedValue;
use zbus::zvariant::Value;

use super::constants::COMPOSITE_ANSWER_DEADLINE;
use super::constants::COMPOSITE_COVERAGE_TOLERANCE;
use super::constants::COMPOSITE_RATIO_TOLERANCE;
use super::constants::COMPOSITE_SCRIPT_PREFIX;
use super::constants::KWIN_LOAD_SCRIPT_METHOD;
use super::constants::KWIN_RUN_SCRIPT_METHOD;
use super::constants::KWIN_SCRIPT_INTERFACE;
use super::constants::KWIN_SCRIPT_PATH_PREFIX;
use super::constants::KWIN_SCRIPTING_INTERFACE;
use super::constants::KWIN_SCRIPTING_PATH;
use super::constants::KWIN_SERVICE;
use super::constants::KWIN_UNLOAD_SCRIPT_METHOD;
use super::constants::SCREENSHOT_DECORATION_OPTION;
use super::constants::SCREENSHOT_HEIGHT_RESULT;
use super::constants::SCREENSHOT_INTERFACE;
use super::constants::SCREENSHOT_PATH;
use super::constants::SCREENSHOT_STRIDE_RESULT;
use super::constants::SCREENSHOT_WIDTH_RESULT;
use super::constants::SCREENSHOT_WINDOW_METHOD;
use super::constants::SINK_PATH;
use super::constants::SINK_PLACEHOLDER;
use super::constants::STACKING_SCRIPT;
use super::display::Output;

/// One display's pixels, holding everything that stands under this
/// terminal's window.
pub(super) struct Composite {
    /// The display's pixels, at the density `KWin` composites them.
    pub(super) image: RgbaImage,
    /// Picture pixels to one `KWin` logical coordinate.
    pub(super) ratio: f64,
}

/// One window in `KWin`'s stacking order.
#[derive(Clone, Eq, PartialEq)]
struct StackedWindow {
    /// `KWin`'s stable identifier, in the braced form its capture calls
    /// expect.
    uuid:      String,
    /// The name of the output the window stands on.
    output:    String,
    /// Whether the window is minimized, and so has nothing on screen.
    minimized: bool,
    /// Whether the window shows on every virtual desktop.
    on_all:    bool,
    /// The virtual desktops the window shows on.
    desktops:  String,
    /// The window's frame, in `KWin`'s logical coordinates.
    frame:     Rectangle,
    /// The window's buffer -- the frame grown by its shadow, and what a
    /// capture of this window actually covers.
    buffer:    Rectangle,
}

/// A rectangle in `KWin`'s logical coordinates.
///
/// Whole coordinates, so every conversion out of them is lossless.
/// `KWin` itself does not always report them that way -- see
/// [`whole_coordinate`] for the fraction a dragged window carries.
#[derive(Clone, Copy, Eq, PartialEq)]
struct Rectangle {
    /// The rectangle's top-left corner.
    origin: (i32, i32),
    /// The rectangle's width and height.
    size:   (i32, i32),
}

/// Receives the stacking order the `KWin` script reads.
struct Sink {
    /// Carries the script's one answer back to the capture.
    sender: Sender<String>,
}

#[interface(name = "dev.tui_pane.Backdrop")]
impl Sink {
    /// Called once by the script, with one window to the line.
    ///
    /// The name is spelled out because `KWin`'s `callDBus` names the
    /// method exactly as written and the macro would otherwise publish
    /// it capitalized.
    #[zbus(name = "result")]
    fn result(&self, rows: String) { let _ = self.sender.send(rows); }
}

/// Draw everything below `uuid` on `output` over that display's desktop.
///
/// [`None`] wherever the stack cannot be read, the window is no longer
/// in it, or the display's own desktop cannot be captured -- each of
/// which leaves the caller reconstructing Plasma's wallpaper instead.
pub(super) fn below_window(uuid: &str, output: &Output) -> Option<Composite> {
    let stack = stacking_order()?;
    let ours = stack.iter().position(|window| window.uuid == uuid)?;
    let below = &stack[..ours];
    let (desktop, over) = desktop_and_windows(below, &stack[ours], output)?;

    let connection = Connection::session().ok()?;
    let screenshot = Proxy::new(
        &connection,
        KWIN_SERVICE,
        SCREENSHOT_PATH,
        SCREENSHOT_INTERFACE,
    )
    .ok()?;

    let mut image = capture_window(&screenshot, &desktop.uuid)?;
    let ratio = pixels_per_coordinate(&image, desktop.buffer)?;
    let covered = desktop.buffer;
    let across = image.width();
    let down = image.height();
    for window in over {
        let Some(captured) = capture_window(&screenshot, &window.uuid) else {
            continue;
        };
        imageops::overlay(
            &mut image,
            &captured,
            scaled(
                window.buffer.origin.0,
                covered.origin.0,
                across,
                covered.size.0,
            ),
            scaled(
                window.buffer.origin.1,
                covered.origin.1,
                down,
                covered.size.1,
            ),
        );
    }
    Some(Composite { image, ratio })
}

/// Split the windows below ours into the display's desktop and the
/// windows standing over it.
///
/// The desktop is the lowest window covering the whole of `output`,
/// which is the one Plasma paints the wallpaper and its icons onto.
/// [`None`] where nothing below ours covers the display, which is what
/// a window on another display or a stack read mid-layout leaves.
fn desktop_and_windows<'a>(
    below: &'a [StackedWindow],
    ours: &StackedWindow,
    output: &Output,
) -> Option<(
    &'a StackedWindow,
    impl Iterator<Item = &'a StackedWindow> + use<'a>,
)> {
    let at = below
        .iter()
        .position(|window| window.output == ours.output && window.covers(output))?;
    let ours = ours.clone();
    let desktop = &below[at];
    let over = below[at.saturating_add(1)..]
        .iter()
        .filter(move |window| window.shows_beside(&ours));
    Some((desktop, over))
}

impl StackedWindow {
    /// One tab-separated row from the script, or [`None`] for a row it
    /// did not write.
    fn parse(row: &str) -> Option<Self> {
        let mut fields = row.split('\t');
        let mut next = || fields.next().map(str::to_owned);
        let uuid = next()?;
        let output = next()?;
        let minimized = next()? == "true";
        let on_all = next()? == "true";
        let desktops = next()?;
        let mut rectangle = || {
            let mut edge = || whole_coordinate(fields.next()?);
            Some(Rectangle {
                origin: (edge()?, edge()?),
                size:   (edge()?, edge()?),
            })
        };
        let frame = rectangle()?;
        let buffer = rectangle()?;
        Some(Self {
            uuid,
            output,
            minimized,
            on_all,
            desktops,
            frame,
            buffer,
        })
    }

    /// Whether this window's frame covers the whole of `output`.
    fn covers(&self, output: &Output) -> bool {
        let size = output.logical_size();
        let covered =
            |edge: i32, want: f64| (f64::from(edge) - want).abs() <= COMPOSITE_COVERAGE_TOLERANCE;
        covered(self.frame.origin.0, output.origin.0)
            && covered(self.frame.origin.1, output.origin.1)
            && covered(self.frame.size.0, size.0)
            && covered(self.frame.size.1, size.1)
    }

    /// Whether this window is on screen wherever `ours` is.
    fn shows_beside(&self, ours: &Self) -> bool {
        !self.minimized
            && self.output == ours.output
            && (self.on_all || ours.on_all || self.desktops == ours.desktops)
    }
}

/// Read `KWin`'s stacking order, bottom window first.
///
/// `KWin` runs scripts in its own process and gives them no way to
/// return a value, so the script is handed the bus name this connection
/// already owns and calls [`Sink`] back with what it read. The script is
/// unloaded and its file removed whether or not the answer arrived, so a
/// capture that failed part way through leaves nothing loaded behind it.
fn stacking_order() -> Option<Vec<StackedWindow>> {
    let (sender, receiver) = channel();
    let connection = Builder::session()
        .ok()?
        .serve_at(SINK_PATH, Sink { sender })
        .ok()?
        .build()
        .ok()?;
    let sink = connection.unique_name()?.as_str().to_owned();
    let path = script_path();
    fs::write(&path, STACKING_SCRIPT.replace(SINK_PLACEHOLDER, &sink)).ok()?;

    let plugin = path.file_stem()?.to_str()?.to_owned();
    let scripting = Proxy::new(
        &connection,
        KWIN_SERVICE,
        KWIN_SCRIPTING_PATH,
        KWIN_SCRIPTING_INTERFACE,
    )
    .ok()?;
    let loaded: Option<i32> = scripting
        .call(KWIN_LOAD_SCRIPT_METHOD, &(path.to_str()?, plugin.as_str()))
        .ok();
    let answer = loaded.and_then(|id| run_script(&connection, id, &receiver));
    let _: Result<bool, _> = scripting.call(KWIN_UNLOAD_SCRIPT_METHOD, &plugin.as_str());
    let _ = fs::remove_file(&path);

    Some(answer?.lines().filter_map(StackedWindow::parse).collect())
}

/// Run one loaded script and wait for the answer it calls back with.
fn run_script(connection: &Connection, id: i32, receiver: &Receiver<String>) -> Option<String> {
    let script = Proxy::new(
        connection,
        KWIN_SERVICE,
        format!("{KWIN_SCRIPT_PATH_PREFIX}{id}"),
        KWIN_SCRIPT_INTERFACE,
    )
    .ok()?;
    let ran: Result<(), _> = script.call(KWIN_RUN_SCRIPT_METHOD, &());
    ran.ok()?;
    receiver.recv_timeout(COMPOSITE_ANSWER_DEADLINE).ok()
}

/// Where the next stacking script is written.
///
/// The session's runtime directory where there is one, because only this
/// user may read it, and the shared temporary directory otherwise. The
/// name carries the process and a count, which keeps two captures from
/// loading each other's script and doubles as the plugin name `KWin`
/// unloads it by.
fn script_path() -> PathBuf {
    /// Tells one script from the next within this process.
    static READS: AtomicU64 = AtomicU64::new(0);

    let directory = dirs::runtime_dir().unwrap_or_else(env::temp_dir);
    let reads = READS.fetch_add(1, Ordering::Relaxed);
    directory.join(format!(
        "{COMPOSITE_SCRIPT_PREFIX}{}-{reads}.js",
        process::id()
    ))
}

/// Capture one window, or [`None`] where `KWin` will not answer for it.
///
/// `KWin` writes the pixels into a pipe as premultiplied ARGB and
/// describes them in its reply, so the bytes are read out of the pipe
/// and turned back into the straight alpha [`RgbaImage`] compositing
/// wants.
fn capture_window(screenshot: &Proxy<'_>, uuid: &str) -> Option<RgbaImage> {
    let (mut reader, writer) = UnixStream::pair().ok()?;
    let options: HashMap<&str, Value<'_>> =
        HashMap::from([(SCREENSHOT_DECORATION_OPTION, Value::Bool(true))]);
    let described: HashMap<String, OwnedValue> = screenshot
        .call(
            SCREENSHOT_WINDOW_METHOD,
            &(uuid, options, Fd::from(writer.as_fd())),
        )
        .ok()?;
    drop(writer);
    let mut pixels = Vec::new();
    reader.read_to_end(&mut pixels).ok()?;

    let measure = |key: &str| -> Option<u32> {
        described
            .get(key)
            .and_then(|value| u32::try_from(value.clone()).ok())
    };
    let width = measure(SCREENSHOT_WIDTH_RESULT)?;
    let height = measure(SCREENSHOT_HEIGHT_RESULT)?;
    let stride = measure(SCREENSHOT_STRIDE_RESULT)?;
    let rows = (0..height).map(|row| usize::try_from(row.checked_mul(stride)?).ok());
    let taken: Option<Vec<u8>> = rows
        .map(|row| {
            let row = row?;
            let wide = usize::try_from(width.checked_mul(4)?).ok()?;
            pixels.get(row..row.checked_add(wide)?).map(<[u8]>::to_vec)
        })
        .collect::<Option<Vec<Vec<u8>>>>()
        .map(|rows| rows.concat());
    RgbaImage::from_vec(width, height, straight_alpha(&taken?))
}

/// Turn `KWin`'s premultiplied little-endian ARGB pixels into the
/// straight-alpha RGBA the compositing draws with.
fn straight_alpha(pixels: &[u8]) -> Vec<u8> {
    pixels
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|&[blue, green, red, alpha]| {
            let straight = |channel: u8| {
                if alpha == 0 {
                    return 0;
                }
                let raised = u32::from(channel).saturating_mul(u32::from(u8::MAX));
                u8::try_from(raised / u32::from(alpha)).unwrap_or(u8::MAX)
            };
            [straight(red), straight(green), straight(blue), alpha]
        })
        .collect()
}

/// How many picture pixels one logical coordinate covers, or [`None`]
/// where the picture does not cover the rectangle it was taken of.
///
/// `KWin` captures at whatever density it composites the display, which
/// on a scaled output is that scale. Both axes carry the same density,
/// so two that disagree say the display was rearranged between the stack
/// read and the capture, and the caller reconstructs the wallpaper
/// rather than drawing windows at the wrong size.
fn pixels_per_coordinate(desktop: &RgbaImage, covered: Rectangle) -> Option<f64> {
    let across = f64::from(desktop.width()) / f64::from(covered.size.0);
    let down = f64::from(desktop.height()) / f64::from(covered.size.1);
    (across.is_finite() && across > 0.0 && (across - down).abs() <= COMPOSITE_RATIO_TOLERANCE)
        .then_some(across)
}

/// One edge of a `KWin` rectangle, as the whole coordinate a
/// [`Rectangle`] holds.
///
/// `KWin` reports a window's geometry as a fraction whenever one has
/// been dragged rather than snapped -- a window standing at
/// `298.80934941192993` is ordinary, not a fault -- and a fraction
/// parses as no integer at all. Read as one, the window is dropped from
/// the stack, this terminal's own window is not found in it, and the
/// whole composite gives way to the wallpaper: a backdrop that stops
/// showing what is underneath the moment the window is moved by hand.
/// The fraction is under one coordinate of a desktop a few thousand
/// wide, so it is cut rather than carried.
fn whole_coordinate(edge: &str) -> Option<i32> {
    let units = edge.split_once('.').map_or(edge, |(units, _)| units);
    units.parse().ok()
}

/// Where a window's own edge falls inside the display's picture.
///
/// The picture's density is `pixels` over `covered`, both of which `KWin`
/// reports as whole numbers, so this stays in integers rather than
/// rounding a float back to a pixel. `covered` is never zero here:
/// [`pixels_per_coordinate`] divides by it first and refuses a density
/// that is not finite, which is what a zero-width rectangle produces.
fn scaled(edge: i32, origin: i32, pixels: u32, covered: i32) -> i64 {
    (i64::from(edge) - i64::from(origin)) * i64::from(pixels) / i64::from(covered)
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::*;

    /// One display, 3440 by 1440 at the origin, carrying one pixel to
    /// the logical coordinate.
    fn display() -> Output {
        Output {
            origin:       (0.0, 0.0),
            screen_index: 0,
            size:         (3440, 1440),
            scale:        1.0,
        }
    }

    /// One row as the script writes it.
    fn row(uuid: &str, output: &str, desktops: &str, frame: (i64, i64, i64, i64)) -> String {
        format!(
            "{uuid}\t{output}\tfalse\tfalse\t{desktops}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            frame.0,
            frame.1,
            frame.2,
            frame.3,
            frame.0 - 10,
            frame.1 - 10,
            frame.2 + 20,
            frame.3 + 20,
        )
    }

    #[test]
    fn a_row_carries_the_frame_and_the_shadow_around_it() {
        let parsed =
            StackedWindow::parse(&row("{window-one}", "DP-3", "desk", (3440, 0, 1720, 1440)))
                .expect("the script's own row parses");

        assert_eq!(parsed.uuid, "{window-one}");
        assert_eq!(parsed.output, "DP-3");
        assert_eq!(parsed.frame.origin, (3440, 0));
        assert_eq!(parsed.buffer.origin, (3430, -10));
        assert_eq!(parsed.buffer.size, (1740, 1460));
    }

    /// What `KWin` reported for a window the user had dragged rather
    /// than snapped, copied off the running compositor. Read as an
    /// integer the row parses as nothing, the window vanishes from the
    /// stack, and the backdrop falls back to the wallpaper the moment
    /// the window is moved by hand.
    #[test]
    fn a_dragged_window_reported_with_a_fraction_still_parses() {
        let dragged = "{window-dragged}\tHDMI-A-2\tfalse\tfalse\tdesk\t\
                       298.80934941192993\t116.28557054879525\t1637\t941\t\
                       298.80934941192993\t144.28557054879525\t1637\t913";

        let parsed = StackedWindow::parse(dragged).expect("a dragged window's row parses");

        assert_eq!(parsed.frame.origin, (298, 116));
        assert_eq!(parsed.frame.size, (1637, 941));
        assert_eq!(parsed.buffer.origin, (298, 144));
    }

    #[test]
    fn a_row_the_script_did_not_write_is_refused() {
        assert!(StackedWindow::parse("").is_none());
        assert!(StackedWindow::parse("{one}\tDP-3\tfalse").is_none());
        assert_eq!(whole_coordinate("north"), None);
        assert_eq!(whole_coordinate(".5"), None);
    }

    /// The desktop is the lowest window covering the display, and every
    /// window above it that shows beside ours is drawn over it.
    #[test]
    fn the_desktop_is_found_under_the_windows_standing_on_it() {
        let below = [
            StackedWindow::parse(&row("{desktop-window}", "DP-3", "desk", (0, 0, 3440, 1440)))
                .expect("a row parses"),
            StackedWindow::parse(&row("{window-over}", "DP-3", "desk", (100, 100, 800, 600)))
                .expect("a row parses"),
            StackedWindow::parse(&row("{elsewhere}", "HDMI-A-2", "desk", (0, 0, 800, 600)))
                .expect("a row parses"),
        ];
        let ours = StackedWindow::parse(&row("{window-ours}", "DP-3", "desk", (0, 0, 1720, 1440)))
            .expect("a row parses");

        let (desktop, over) =
            desktop_and_windows(&below, &ours, &display()).expect("the desktop is below ours");
        let over: Vec<&str> = over.map(|window| window.uuid.as_str()).collect();

        assert_eq!(desktop.uuid, "{desktop-window}");
        assert_eq!(over, ["{window-over}"]);
    }

    /// Nothing below ours covering the display leaves nothing to draw
    /// over, which is what a stack read while the layout changes leaves.
    #[test]
    fn a_stack_with_no_desktop_under_ours_is_refused() {
        let below =
            [
                StackedWindow::parse(&row("{window-over}", "DP-3", "desk", (100, 100, 800, 600)))
                    .expect("a row parses"),
            ];
        let ours = StackedWindow::parse(&row("{window-ours}", "DP-3", "desk", (0, 0, 1720, 1440)))
            .expect("a row parses");

        assert!(desktop_and_windows(&below, &ours, &display()).is_none());
    }

    #[test]
    fn a_minimized_window_is_not_drawn() {
        let ours = StackedWindow::parse(&row("{window-ours}", "DP-3", "desk", (0, 0, 1720, 1440)))
            .expect("a row parses");
        let mut minimized =
            StackedWindow::parse(&row("{window-over}", "DP-3", "desk", (100, 100, 800, 600)))
                .expect("a row parses");
        minimized.minimized = true;

        assert!(!minimized.shows_beside(&ours));
    }

    #[test]
    fn a_window_on_another_virtual_desktop_is_not_drawn() {
        let ours = StackedWindow::parse(&row("{window-ours}", "DP-3", "desk", (0, 0, 1720, 1440)))
            .expect("a row parses");
        let elsewhere =
            StackedWindow::parse(&row("{window-over}", "DP-3", "other", (100, 100, 800, 600)))
                .expect("a row parses");
        let everywhere =
            StackedWindow::parse(&row("{window-all}", "DP-3", "other", (0, 0, 800, 600)))
                .map(|window| StackedWindow {
                    on_all: true,
                    ..window
                })
                .expect("a row parses");

        assert!(!elsewhere.shows_beside(&ours));
        assert!(everywhere.shows_beside(&ours));
    }

    #[test]
    fn a_picture_of_the_whole_display_carries_one_pixel_to_the_coordinate() {
        let covered = Rectangle {
            origin: (0, 0),
            size:   (3440, 1440),
        };

        assert_eq!(
            pixels_per_coordinate(&RgbaImage::new(3440, 1440), covered),
            Some(1.0)
        );
        assert!(pixels_per_coordinate(&RgbaImage::new(3440, 2880), covered).is_none());
    }

    /// `KWin` hands back premultiplied little-endian ARGB, so the blue
    /// byte comes first and every channel is scaled by the alpha.
    #[test]
    fn premultiplied_argb_comes_back_as_straight_rgba() {
        let opaque = straight_alpha(&[10, 20, 30, 255]);
        let half = straight_alpha(&[64, 64, 64, 128]);
        let clear = straight_alpha(&[0, 0, 0, 0]);

        assert_eq!(opaque, [30, 20, 10, 255]);
        assert_eq!(half, [127, 127, 127, 128]);
        assert_eq!(clear, [0, 0, 0, 0]);
    }
}
