//! Reading and rendering Plasma's static-image wallpaper settings.

use std::collections::HashMap;
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::time::SystemTime;

use image::DynamicImage;
use image::Rgba;
use image::RgbaImage;
use image::imageops;
use image::imageops::FilterType;
use url::Url;
use zbus::blocking::Proxy;
use zbus::zvariant::OwnedValue;
use zbus::zvariant::Structure;

use super::constants::ASPECT_RATIO_DISTANCE_WEIGHT;
use super::constants::DARK_PALETTE_THRESHOLD;
use super::constants::DEFAULT_LOOK_AND_FEEL_PACKAGE;
use super::constants::DEFAULT_WALLPAPER_PACKAGE;
use super::constants::IMAGE_EXTENSIONS;
use super::constants::IMAGE_PLUGIN;
use super::constants::KDE_GLOBALS_FILE;
use super::constants::LOOK_AND_FEEL_DEFAULTS_PATH;
use super::constants::PLASMA_INTERFACE;
use super::constants::PLASMA_PATH;
use super::constants::PLASMA_SERVICE;
use super::constants::QGRAY_BLUE_WEIGHT;
use super::constants::QGRAY_DIVISOR;
use super::constants::QGRAY_GREEN_WEIGHT;
use super::constants::QGRAY_RED_WEIGHT;
use super::constants::UPSCALE_DISTANCE_MULTIPLIER;
use super::constants::WALLPAPERS_PATH;
use super::constants::XDG_CONFIG_DEFAULT;
use super::constants::XDG_DATA_DEFAULT;
use super::constants::XDG_DATA_DIRS_DEFAULT;
use super::session_connection;

/// Wallpaper state that affects pixels rendered for one output.
#[derive(Clone, Eq, PartialEq)]
pub(super) struct WallpaperSnapshot {
    /// Color visible where the image does not cover the output.
    color:       Rgba<u8>,
    /// How Plasma sizes and repeats the image.
    fill_mode:   FillMode,
    /// Configured wallpaper image, absent for a solid-color desktop.
    image:       Option<PathBuf>,
    /// Source timestamp used to invalidate the reduced-grid cache.
    modified_at: Option<SystemTime>,
}

impl WallpaperSnapshot {
    /// Render the wallpaper at an output's physical pixel dimensions.
    pub(super) fn render(&self, output: (u32, u32)) -> Option<RgbaImage> {
        let mut canvas = RgbaImage::from_pixel(output.0, output.1, self.color);
        let Some(path) = self.image.as_ref() else {
            return Some(canvas);
        };
        let source = image::open(path).ok()?.to_rgba8();
        if source.width() == 0 || source.height() == 0 {
            return None;
        }
        match self.fill_mode {
            FillMode::Pad => paint_centered(&mut canvas, &source),
            FillMode::PreserveAspectCrop => {
                let image = DynamicImage::ImageRgba8(source)
                    .resize_to_fill(output.0, output.1, FilterType::Lanczos3)
                    .to_rgba8();
                paint_centered(&mut canvas, &image);
            },
            FillMode::PreserveAspectFit => {
                let image = DynamicImage::ImageRgba8(source)
                    .resize(output.0, output.1, FilterType::Lanczos3)
                    .to_rgba8();
                paint_centered(&mut canvas, &image);
            },
            FillMode::Stretch => {
                canvas = imageops::resize(&source, output.0, output.1, FilterType::Lanczos3);
            },
            FillMode::Tile => paint_tiled(&mut canvas, &source, true, true),
            FillMode::TileHorizontally => {
                let image = resize_to_height(&source, output.1)?;
                paint_tiled(&mut canvas, &image, true, false);
            },
            FillMode::TileVertically => {
                let image = resize_to_width(&source, output.0)?;
                paint_tiled(&mut canvas, &image, false, true);
            },
        }
        Some(canvas)
    }
}

/// Qt Quick Image fill modes used by Plasma's image wallpaper plugin.
#[derive(Clone, Copy, Eq, PartialEq)]
enum FillMode {
    /// Distort the source to the output dimensions.
    Stretch,
    /// Preserve aspect ratio with uncovered background on one axis.
    PreserveAspectFit,
    /// Preserve aspect ratio and crop the overflowing axis.
    PreserveAspectCrop,
    /// Repeat at the source's natural size on both axes.
    Tile,
    /// Fit vertically and repeat horizontally.
    TileHorizontally,
    /// Fit horizontally and repeat vertically.
    TileVertically,
    /// Center at the source's natural size.
    Pad,
}

/// Light or dark image directory selected from a Plasma wallpaper package.
#[derive(Clone, Copy, Eq, PartialEq)]
enum ImageVariant {
    Light,
    Dark,
}

impl ImageVariant {
    /// Read the current KDE window palette classification.
    fn current() -> Self {
        let Some(kde_globals) = kde_globals() else {
            return Self::Light;
        };
        if let Some(background) = ini_value(&kde_globals, "Colors:Window", "BackgroundNormal")
            && let Some((red, green, blue)) = rgb(background)
        {
            return Self::from((red, green, blue));
        }
        ini_value(&kde_globals, "General", "ColorScheme").map_or(Self::Light, |name| {
            if name.to_ascii_lowercase().contains("dark") {
                Self::Dark
            } else {
                Self::Light
            }
        })
    }

    /// Apply an explicit Plasma image URL fragment to the desktop variant.
    fn with_fragment(self, fragment: Option<&str>) -> Self {
        match fragment {
            Some(value) if value.contains("dark") => Self::Dark,
            Some(value) if value.contains("light") => Self::Light,
            _ => self,
        }
    }
}

impl From<(u32, u32, u32)> for ImageVariant {
    fn from((red, green, blue): (u32, u32, u32)) -> Self {
        let gray = red * QGRAY_RED_WEIGHT + green * QGRAY_GREEN_WEIGHT + blue * QGRAY_BLUE_WEIGHT;
        if gray / QGRAY_DIVISOR < DARK_PALETTE_THRESHOLD {
            Self::Dark
        } else {
            Self::Light
        }
    }
}

impl TryFrom<i32> for FillMode {
    type Error = ();

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Stretch),
            1 => Ok(Self::PreserveAspectFit),
            2 => Ok(Self::PreserveAspectCrop),
            3 => Ok(Self::Tile),
            4 => Ok(Self::TileVertically),
            5 => Ok(Self::TileHorizontally),
            6 => Ok(Self::Pad),
            _ => Err(()),
        }
    }
}

/// Read the wallpaper currently assigned to one Plasma screen.
pub(super) fn snapshot(screen_index: u32, output: (u32, u32)) -> Option<WallpaperSnapshot> {
    let proxy = Proxy::new(
        session_connection()?,
        PLASMA_SERVICE,
        PLASMA_PATH,
        PLASMA_INTERFACE,
    )
    .ok()?;
    let settings: HashMap<String, OwnedValue> = proxy.call("wallpaper", &screen_index).ok()?;
    if setting_text(&settings, "wallpaperPlugin")? != IMAGE_PLUGIN {
        return None;
    }
    if setting_bool(&settings, "Blur").unwrap_or(false) {
        return None;
    }
    let configured_image = setting_text(&settings, "PreviewImage")
        .filter(|value| !value.is_empty() && value != "null")
        .or_else(|| setting_text(&settings, "Image"))?;
    let image = resolve_image(&configured_image, output)?;
    let modified_at = fs::metadata(&image)
        .ok()
        .and_then(|metadata| metadata.modified().ok());
    Some(WallpaperSnapshot {
        color: setting_color(&settings, "Color").unwrap_or(Rgba([0, 0, 0, u8::MAX])),
        fill_mode: FillMode::try_from(setting_i32(&settings, "FillMode")?).ok()?,
        image: Some(image),
        modified_at,
    })
}

/// Read a string setting.
fn setting_text(settings: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    <&str>::try_from(settings.get(key)?).ok().map(str::to_owned)
}

/// Read a Boolean setting.
fn setting_bool(settings: &HashMap<String, OwnedValue>, key: &str) -> Option<bool> {
    bool::try_from(settings.get(key)?).ok()
}

/// Read a signed integer setting.
fn setting_i32(settings: &HashMap<String, OwnedValue>, key: &str) -> Option<i32> {
    i32::try_from(settings.get(key)?).ok()
}

/// Read Plasma's single-field `QColor` D-Bus structure as RGBA.
fn setting_color(settings: &HashMap<String, OwnedValue>, key: &str) -> Option<Rgba<u8>> {
    let structure = <&Structure<'_>>::try_from(settings.get(key)?).ok()?;
    let argb = structure.fields().first()?.downcast_ref::<u32>().ok()?;
    Some(rgba_from_argb(argb))
}

/// Reorder Qt's ARGB bytes into the image crate's RGBA order.
const fn rgba_from_argb(argb: u32) -> Rgba<u8> {
    let [alpha, red, green, blue] = argb.to_be_bytes();
    Rgba([red, green, blue, alpha])
}

/// Resolve Plasma's configured source or its Look-and-Feel default to one image file.
fn resolve_image(value: &str, output: (u32, u32)) -> Option<PathBuf> {
    let data_locations = data_locations();
    let desktop_variant = ImageVariant::current();
    let (source, image_variant) = if value.is_empty() {
        (default_wallpaper_package(&data_locations)?, desktop_variant)
    } else {
        configured_source(value, &data_locations, desktop_variant)?
    };
    select_image(&source, output, image_variant)
}

/// Convert a Plasma image URL, absolute path, or wallpaper package name to a local path.
fn configured_source(
    value: &str,
    data_locations: &[PathBuf],
    desktop_variant: ImageVariant,
) -> Option<(PathBuf, ImageVariant)> {
    if value == "null" {
        return None;
    }
    if let Ok(url) = Url::parse(value) {
        if url.scheme() != "file" {
            return None;
        }
        let image_variant = desktop_variant.with_fragment(url.fragment());
        return url.to_file_path().ok().map(|path| (path, image_variant));
    }
    let (source, fragment) = value
        .split_once('#')
        .map_or((value, None), |(source, fragment)| (source, Some(fragment)));
    let image_variant = desktop_variant.with_fragment(fragment);
    let path = PathBuf::from(source);
    if path.is_absolute() {
        return Some((path, image_variant));
    }
    locate_data_path(data_locations, &Path::new(WALLPAPERS_PATH).join(path))
        .map(|path| (path, image_variant))
}

/// Resolve a file directly or choose the closest-sized image from a wallpaper package.
fn select_image(source: &Path, output: (u32, u32), image_variant: ImageVariant) -> Option<PathBuf> {
    if source.is_file() {
        return Some(source.to_path_buf());
    }
    if !source.is_dir() {
        return None;
    }
    let dark = source.join("contents/images_dark");
    if image_variant == ImageVariant::Dark
        && let Some(image) = preferred_image_in(&dark, output)
    {
        return Some(image);
    }
    preferred_image_in(&source.join("contents/images"), output)
}

/// Choose the package image Plasma ranks closest to the output dimensions.
fn preferred_image_in(directory: &Path, output: (u32, u32)) -> Option<PathBuf> {
    let mut images = fs::read_dir(directory)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && is_supported_image(path))
        .collect::<Vec<_>>();
    images.sort();
    preferred_image(images.iter(), output).cloned()
}

/// Apply Plasma's package-image distance function to a set of file paths.
fn preferred_image<'a>(
    images: impl IntoIterator<Item = &'a PathBuf>,
    output: (u32, u32),
) -> Option<&'a PathBuf> {
    images
        .into_iter()
        .filter(|path| dimensions_from_name(path).is_some())
        .min_by(|left, right| {
            image_distance(left, output)
                .total_cmp(&image_distance(right, output))
                .then_with(|| left.cmp(right))
        })
}

/// Rank one package image using the same aspect-ratio and upscaling penalties as Plasma.
fn image_distance(path: &Path, output: (u32, u32)) -> f64 {
    let Some((width, height)) = dimensions_from_name(path) else {
        return f64::INFINITY;
    };
    let desired_aspect = f64::from(output.0) / f64::from(output.1);
    let candidate_aspect = f64::from(width) / f64::from(height);
    let width_delta = f64::from(width) - f64::from(output.0);
    let scale_penalty = if width_delta >= 0.0 {
        width_delta
    } else {
        -width_delta * UPSCALE_DISTANCE_MULTIPLIER
    };
    (candidate_aspect - desired_aspect)
        .abs()
        .mul_add(ASPECT_RATIO_DISTANCE_WEIGHT, scale_penalty)
}

/// Parse Plasma's `WIDTHxHEIGHT` wallpaper-package filename convention.
fn dimensions_from_name(path: &Path) -> Option<(u32, u32)> {
    let stem = path.file_stem()?.to_str()?;
    let (width, height) = stem.split_once('x')?;
    let width = width.parse().ok()?;
    let height = height.parse().ok()?;
    (width > 0 && height > 0).then_some((width, height))
}

/// Whether the image crate features enabled by `tui_pane` can decode this file.
fn is_supported_image(path: &Path) -> bool {
    path.extension()
        .and_then(OsStr::to_str)
        .is_some_and(|extension| {
            IMAGE_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str())
        })
}

/// Locate the wallpaper package named by the active Look-and-Feel defaults.
fn default_wallpaper_package(data_locations: &[PathBuf]) -> Option<PathBuf> {
    let kde_globals = kde_globals();
    let look_and_feel = kde_globals
        .as_deref()
        .and_then(|config| ini_value(config, "KDE", "LookAndFeelPackage"))
        .filter(|name| !name.is_empty())
        .unwrap_or(DEFAULT_LOOK_AND_FEEL_PACKAGE);
    let defaults = Path::new(LOOK_AND_FEEL_DEFAULTS_PATH)
        .join(look_and_feel)
        .join("contents/defaults");
    let configured = locate_data_path(data_locations, &defaults)
        .and_then(|path| fs::read_to_string(path).ok())
        .and_then(|config| ini_value(&config, "Wallpaper", "Image").map(str::to_owned))
        .filter(|name| !name.is_empty())
        .and_then(|name| locate_data_path(data_locations, &Path::new(WALLPAPERS_PATH).join(name)));
    configured.or_else(|| {
        locate_data_path(
            data_locations,
            &Path::new(WALLPAPERS_PATH).join(DEFAULT_WALLPAPER_PACKAGE),
        )
    })
}

/// KDE's per-user global configuration file contents.
fn kde_globals() -> Option<String> {
    let config_home = env::var_os("XDG_CONFIG_HOME")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os("HOME")
                .filter(|path| !path.is_empty())
                .map(|home| PathBuf::from(home).join(XDG_CONFIG_DEFAULT))
        })?;
    fs::read_to_string(config_home.join(KDE_GLOBALS_FILE)).ok()
}

/// User and system directories searched by `QStandardPaths::GenericDataLocation`.
fn data_locations() -> Vec<PathBuf> {
    let mut locations = Vec::new();
    if let Some(data_home) = env::var_os("XDG_DATA_HOME").filter(|path| !path.is_empty()) {
        locations.push(PathBuf::from(data_home));
    } else if let Some(home) = env::var_os("HOME").filter(|path| !path.is_empty()) {
        locations.push(PathBuf::from(home).join(XDG_DATA_DEFAULT));
    }
    let data_dirs = env::var_os("XDG_DATA_DIRS")
        .filter(|paths| !paths.is_empty())
        .unwrap_or_else(|| XDG_DATA_DIRS_DEFAULT.into());
    locations.extend(env::split_paths(&data_dirs));
    locations
}

/// Return the first existing relative path in KDE's data search order.
fn locate_data_path(data_locations: &[PathBuf], relative: &Path) -> Option<PathBuf> {
    data_locations
        .iter()
        .map(|directory| directory.join(relative))
        .find(|path| path.exists())
}

/// Read one value from a KDE INI-style configuration section.
fn ini_value<'a>(contents: &'a str, section: &str, key: &str) -> Option<&'a str> {
    let mut in_section = false;
    for line in contents.lines().map(str::trim) {
        if let Some(name) = line
            .strip_prefix('[')
            .and_then(|line| line.strip_suffix(']'))
        {
            in_section = name == section;
            continue;
        }
        if in_section
            && let Some((name, value)) = line.split_once('=')
            && name.trim() == key
        {
            return Some(value.trim());
        }
    }
    None
}

/// Parse the first three channels of a KDE comma-separated RGB value.
fn rgb(value: &str) -> Option<(u32, u32, u32)> {
    let mut channels = value.split(',').map(str::trim);
    Some((
        channels.next()?.parse().ok()?,
        channels.next()?.parse().ok()?,
        channels.next()?.parse().ok()?,
    ))
}

/// Center an image over the output, clipping any excess at its edges.
fn paint_centered(canvas: &mut RgbaImage, image: &RgbaImage) {
    let x = (i64::from(canvas.width()) - i64::from(image.width())) / 2;
    let y = (i64::from(canvas.height()) - i64::from(image.height())) / 2;
    imageops::overlay(canvas, image, x, y);
}

/// Repeat an image over either or both output axes.
fn paint_tiled(canvas: &mut RgbaImage, image: &RgbaImage, horizontal: bool, vertical: bool) {
    let mut y = 0_u32;
    loop {
        let mut x = 0_u32;
        loop {
            imageops::overlay(canvas, image, i64::from(x), i64::from(y));
            if !horizontal {
                break;
            }
            let Some(next) = x.checked_add(image.width()) else {
                break;
            };
            x = next;
            if x >= canvas.width() {
                break;
            }
        }
        if !vertical {
            break;
        }
        let Some(next) = y.checked_add(image.height()) else {
            break;
        };
        y = next;
        if y >= canvas.height() {
            break;
        }
    }
}

/// Resize an image proportionally to an output width.
fn resize_to_width(image: &RgbaImage, width: u32) -> Option<RgbaImage> {
    let numerator = u64::from(image.height()).checked_mul(u64::from(width))?;
    let height = proportional_dimension(numerator, image.width())?;
    Some(imageops::resize(image, width, height, FilterType::Lanczos3))
}

/// Resize an image proportionally to an output height.
fn resize_to_height(image: &RgbaImage, height: u32) -> Option<RgbaImage> {
    let numerator = u64::from(image.width()).checked_mul(u64::from(height))?;
    let width = proportional_dimension(numerator, image.height())?;
    Some(imageops::resize(image, width, height, FilterType::Lanczos3))
}

/// Round a non-negative image ratio to a usable `u32` dimension.
fn proportional_dimension(numerator: u64, denominator: u32) -> Option<u32> {
    let denominator = u64::from(denominator);
    let rounded = numerator
        .checked_add(denominator / 2)?
        .checked_div(denominator)?;
    u32::try_from(rounded.max(1)).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qcolor_argb_channels_become_rgba() {
        assert_eq!(
            rgba_from_argb(0x7f_12_34_56),
            Rgba([0x12, 0x34, 0x56, 0x7f])
        );
    }

    #[test]
    fn solid_color_fills_the_output() {
        let wallpaper = WallpaperSnapshot {
            color:       Rgba([0, 0, 0, u8::MAX]),
            fill_mode:   FillMode::PreserveAspectCrop,
            image:       None,
            modified_at: None,
        };
        assert_eq!(
            wallpaper.render((2, 3)).map(|image| image.dimensions()),
            Some((2, 3))
        );
    }

    #[test]
    fn kde_ini_value_reads_only_the_requested_section() {
        let contents = "[General]\nImage=wrong\n[Wallpaper]\nImage=Next\n";
        assert_eq!(ini_value(contents, "Wallpaper", "Image"), Some("Next"));
    }

    #[test]
    fn package_image_selection_matches_plasma_distance() {
        let images = [
            PathBuf::from("1440x2960.png"),
            PathBuf::from("5120x2880.png"),
            PathBuf::from("7680x2160.png"),
        ];
        assert_eq!(preferred_image(images.iter(), (3440, 1440)), images.get(1));
    }

    #[test]
    fn dark_window_palette_selects_dark_images() {
        assert!(matches!(
            ImageVariant::from((32, 35, 38)),
            ImageVariant::Dark
        ));
    }
}
