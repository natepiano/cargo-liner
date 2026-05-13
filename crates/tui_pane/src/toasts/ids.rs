/// Stable identifier for a toast entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct ToastId(pub u64);

impl ToastId {
    /// Return the raw numeric identifier.
    #[must_use]
    pub const fn get(self) -> u64 { self.0 }
}

/// Stable identifier for a task-backed toast entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct ToastTaskId(pub u64);

impl ToastTaskId {
    /// Return the raw numeric identifier.
    #[must_use]
    pub const fn get(self) -> u64 { self.0 }
}
