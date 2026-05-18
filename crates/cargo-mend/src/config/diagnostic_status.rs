#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DiagnosticStatus {
    Enabled,
    Disabled,
}

impl DiagnosticStatus {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Enabled => "enabled",
            Self::Disabled => "disabled",
        }
    }
}

impl From<bool> for DiagnosticStatus {
    fn from(value: bool) -> Self { if value { Self::Enabled } else { Self::Disabled } }
}
