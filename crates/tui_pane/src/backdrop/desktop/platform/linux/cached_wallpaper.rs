use std::sync::Mutex;

use ratatui::style::Color;

use super::display::Output;
use super::wallpaper;
use super::wallpaper::WallpaperSnapshot;
use crate::backdrop::desktop::CaptureFailure;
use crate::backdrop::desktop::Metrics;
use crate::backdrop::desktop::reduction;

/// The last reduced wallpaper grid, reused while its inputs remain unchanged.
static WALLPAPER_CACHE: Mutex<Option<CachedWallpaper>> = Mutex::new(None);

/// Inputs that determine the reduced wallpaper grid.
#[derive(Clone, Eq, PartialEq)]
struct WallpaperCacheKey {
    /// Terminal geometry used to size each color cell.
    metrics:    Metrics,
    /// Physical dimensions of the output.
    output:     (u32, u32),
    /// The output scale encoded without floating-point equality.
    scale_bits: u64,
    /// Plasma wallpaper settings and source-file timestamp.
    wallpaper:  WallpaperSnapshot,
}

/// A wallpaper already reduced to terminal-sized color cells.
struct CachedWallpaper {
    /// Inputs that produced this grid.
    key:    WallpaperCacheKey,
    /// Cells across and down.
    grid:   (u16, u16),
    /// Row-major colors for the grid.
    colors: Vec<Color>,
}

/// Render Plasma's configured wallpaper for one output and reduce it.
pub(super) fn reconstructed_wallpaper(
    metrics: Metrics,
    output: &Output,
) -> Result<(u16, u16, Vec<Color>), CaptureFailure> {
    let wallpaper = wallpaper::snapshot(output.screen_index, output.size)
        .ok_or(CaptureFailure::DisplayCaptureFailed)?;
    reduced_wallpaper(metrics, output, wallpaper, metrics.cell_points(1.0))
}

/// Return a cached color grid or render and reduce a new one.
fn reduced_wallpaper(
    metrics: Metrics,
    output: &Output,
    wallpaper: WallpaperSnapshot,
    cell: (f64, f64),
) -> Result<(u16, u16, Vec<Color>), CaptureFailure> {
    let key = WallpaperCacheKey {
        metrics,
        output: output.size,
        scale_bits: output.scale.to_bits(),
        wallpaper,
    };
    if let Ok(cache) = WALLPAPER_CACHE.lock()
        && let Some(cached) = cache.as_ref()
        && cached.key == key
    {
        return Ok((cached.grid.0, cached.grid.1, cached.colors.clone()));
    }
    let image = key
        .wallpaper
        .render(key.output)
        .ok_or(CaptureFailure::DisplayCaptureFailed)?;
    let reduced = reduction::reduce_capture(image.as_raw(), key.output, cell)?;
    if let Ok(mut cache) = WALLPAPER_CACHE.lock() {
        *cache = Some(CachedWallpaper {
            key,
            grid: (reduced.0, reduced.1),
            colors: reduced.2.clone(),
        });
    }
    Ok(reduced)
}
