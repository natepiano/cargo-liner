/// Where a window stands, in the window server's global point space.
///
/// A rectangle of four numbers rather than the platform's own type, so
/// that the thread asking the window server where the terminal is can
/// hand the answer back without any platform handle crossing with it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::backdrop) struct Frame {
    /// The frame's top-left corner.
    pub(in crate::backdrop) origin: (f64, f64),
    /// The frame's width and height.
    pub(in crate::backdrop) size:   (f64, f64),
}
