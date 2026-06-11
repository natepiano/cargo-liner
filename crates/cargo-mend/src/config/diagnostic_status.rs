use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use serde::Serializer;

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

impl From<DiagnosticStatus> for bool {
    fn from(value: DiagnosticStatus) -> Self { matches!(value, DiagnosticStatus::Enabled) }
}

impl Serialize for DiagnosticStatus {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        bool::from(*self).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for DiagnosticStatus {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        bool::deserialize(deserializer).map(Self::from)
    }
}
