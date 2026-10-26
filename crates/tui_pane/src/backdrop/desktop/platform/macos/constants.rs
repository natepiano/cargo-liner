/// How many bytes one pixel of the captured image occupies.
pub(super) const BYTES_PER_PIXEL: usize = 4;
/// Where the alpha channel sits in the captured RGBA pixel.
pub(super) const ALPHA: usize = 3;
/// Where the red channel sits in the captured RGBA pixel.
pub(super) const RED: usize = 0;
/// Where the green channel sits in the captured RGBA pixel.
pub(super) const GREEN: usize = 1;
/// Where the blue channel sits in the captured RGBA pixel.
pub(super) const BLUE: usize = 2;
