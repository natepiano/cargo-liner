//! Reading KDE's active output layout.

use std::process::Command;

use serde::Deserialize;

use crate::backdrop::desktop::Frame;

/// One enabled `KScreen` output.
pub(super) struct Output {
    /// Where the output begins in `KWin`'s logical coordinate space.
    pub(super) origin:       (f64, f64),
    /// Plasma's index for the enabled screen.
    pub(super) screen_index: u32,
    /// Physical pixel dimensions used to render the wallpaper.
    pub(super) size:         (u32, u32),
    /// Physical pixels per `KWin` logical coordinate.
    pub(super) scale:        f64,
}

impl Output {
    /// The output's logical width and height.
    fn logical_size(&self) -> (f64, f64) {
        (
            f64::from(self.size.0) / self.scale,
            f64::from(self.size.1) / self.scale,
        )
    }
}

/// The portion of `kscreen-doctor -j` used by the capture backend.
#[derive(Deserialize)]
struct KScreenDocument {
    /// Every connected and disconnected output known to `KScreen`.
    outputs: Vec<KScreenOutput>,
}

/// Geometry and state for one `KScreen` output.
#[derive(Deserialize)]
struct KScreenOutput {
    /// Whether the connector currently participates in the desktop.
    connected: bool,
    /// Whether the output is currently enabled.
    enabled:   bool,
    /// Its top-left logical coordinate.
    pos:       KScreenPoint,
    /// `KScreen`'s rotation flag.
    rotation:  u32,
    /// Physical pixels per logical coordinate.
    scale:     f64,
    /// Current physical pixel dimensions.
    size:      KScreenSize,
}

/// An integer `KScreen` coordinate.
#[derive(Deserialize)]
struct KScreenPoint {
    /// Horizontal coordinate.
    x: i32,
    /// Vertical coordinate.
    y: i32,
}

/// A physical `KScreen` extent.
#[derive(Deserialize)]
struct KScreenSize {
    /// Height in pixels.
    height: u32,
    /// Width in pixels.
    width:  u32,
}

/// Every output currently forming the KDE desktop.
pub(super) fn active_outputs() -> Vec<Output> {
    let Ok(command) = Command::new("kscreen-doctor").arg("-j").output() else {
        return Vec::new();
    };
    if !command.status.success() {
        return Vec::new();
    }
    let Ok(document) = serde_json::from_slice::<KScreenDocument>(&command.stdout) else {
        return Vec::new();
    };
    let mut outputs = Vec::new();
    for raw in document.outputs {
        if !(raw.connected && raw.enabled && raw.scale.is_finite() && raw.scale > 0.0) {
            continue;
        }
        let size = if matches!(raw.rotation, 2 | 8) {
            (raw.size.height, raw.size.width)
        } else {
            (raw.size.width, raw.size.height)
        };
        if size.0 == 0 || size.1 == 0 {
            continue;
        }
        let Ok(screen_index) = u32::try_from(outputs.len()) else {
            break;
        };
        outputs.push(Output {
            origin: (f64::from(raw.pos.x), f64::from(raw.pos.y)),
            screen_index,
            size,
            scale: raw.scale,
        });
    }
    outputs
}

/// The output holding the centre of a window, or the nearest output when the centre is off-screen.
pub(super) fn under(outputs: &[Output], frame: Frame) -> Option<&Output> {
    let center = (
        frame.origin.0 + frame.size.0 / 2.0,
        frame.origin.1 + frame.size.1 / 2.0,
    );
    outputs
        .iter()
        .find(|output| holds(output, center))
        .or_else(|| {
            outputs.iter().min_by(|left, right| {
                distance_squared(left, center).total_cmp(&distance_squared(right, center))
            })
        })
}

/// Whether an output contains a logical coordinate.
fn holds(output: &Output, point: (f64, f64)) -> bool {
    let size = output.logical_size();
    point.0 >= output.origin.0
        && point.0 < output.origin.0 + size.0
        && point.1 >= output.origin.1
        && point.1 < output.origin.1 + size.1
}

/// Squared distance from a point to the centre of an output.
fn distance_squared(output: &Output, point: (f64, f64)) -> f64 {
    let size = output.logical_size();
    let x = output.origin.0 + size.0 / 2.0 - point.0;
    let y = output.origin.1 + size.1 / 2.0 - point.1;
    x.mul_add(x, y * y)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A high-density output beginning at the desktop origin.
    const OUTPUT: Output = Output {
        origin:       (0.0, 0.0),
        screen_index: 0,
        size:         (3840, 2160),
        scale:        2.0,
    };

    #[test]
    fn output_containment_uses_logical_dimensions() {
        assert!(holds(&OUTPUT, (1919.0, 1079.0)));
        assert!(!holds(&OUTPUT, (1920.0, 1080.0)));
    }
}
