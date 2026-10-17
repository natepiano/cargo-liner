//! Footer file keys and readable-ink policy.

// contrast
pub(super) const CONTRAST_LUMINANCE_OFFSET: f64 = 0.05;
pub(super) const LIGHTNESS_SEARCH_ITERATIONS: usize = 32;
pub(crate) const MINIMUM_CONTRAST: f64 = 4.5;

// footer configuration
pub(super) const ACCOUNT_KEY_PREFIX: &str = "account.";
pub(super) const FOOTER_CONFIG_PATH: &str = ".claude/config/statusline.conf";

// sRGB transfer and luminance
pub(super) const BLUE_LUMINANCE_WEIGHT: f64 = 0.0722;
pub(super) const GREEN_LUMINANCE_WEIGHT: f64 = 0.7152;
pub(super) const RED_LUMINANCE_WEIGHT: f64 = 0.2126;
pub(super) const SRGB_ENCODED_OFFSET: f64 = 0.055;
pub(super) const SRGB_ENCODED_SCALE: f64 = 1.055;
pub(super) const SRGB_LINEAR_DIVISOR: f64 = 12.92;
pub(super) const SRGB_LINEAR_THRESHOLD: f64 = 0.04045;
pub(super) const SRGB_POWER: f64 = 2.4;
