/// Stable identifier for a toast entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct ToastId(pub u64);

impl ToastId {
    /// Return the raw numeric identifier.
    #[must_use]
    pub const fn get(self) -> u64 { self.0 }
}

/// Stable identifier for a colored toast entry.
///
/// Unlike [`ToastTaskId`], this handle cannot be passed to task-finish APIs.
/// It is returned only by colored-toast constructors, so callers that need a
/// colored countdown do not have to model that UI as a finished task.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct ColoredToastId(ToastId);

impl ColoredToastId {
    pub(super) const fn new(id: ToastId) -> Self { Self(id) }

    pub(super) const fn toast_id(self) -> ToastId { self.0 }
}

/// Stable identifier for a task-backed toast entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct ToastTaskId(pub u64);

impl ToastTaskId {
    /// Return the raw numeric identifier.
    #[must_use]
    pub const fn get(self) -> u64 { self.0 }
}
