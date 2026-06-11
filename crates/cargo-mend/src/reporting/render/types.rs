#[derive(Debug, Clone, Copy)]
pub(crate) enum ColorMode {
    Enabled,
    Disabled,
}

impl ColorMode {
    pub const fn is_enabled(self) -> bool { matches!(self, Self::Enabled) }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OutputFormat {
    Human,
    Json,
}

pub(crate) struct CompilerStats {
    pub warnings: usize,
    pub fixable:  usize,
}
