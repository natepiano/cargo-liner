use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use serde::Serializer;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum PreludePubMod {
    /// Crate-root `pub mod prelude;` is exempt from `ReviewPubMod`.
    #[default]
    Allowed,
    /// Crate-root `pub mod prelude;` is reviewed like any other `pub mod`.
    Reviewed,
}

impl From<bool> for PreludePubMod {
    fn from(value: bool) -> Self { if value { Self::Allowed } else { Self::Reviewed } }
}

impl From<PreludePubMod> for bool {
    fn from(value: PreludePubMod) -> Self { matches!(value, PreludePubMod::Allowed) }
}

impl Serialize for PreludePubMod {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        bool::from(*self).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for PreludePubMod {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        bool::deserialize(deserializer).map(Self::from)
    }
}

#[cfg(test)]
mod tests {
    use super::PreludePubMod;

    #[test]
    fn bool_round_trips() {
        assert_eq!(PreludePubMod::from(true), PreludePubMod::Allowed);
        assert_eq!(PreludePubMod::from(false), PreludePubMod::Reviewed);
        assert!(bool::from(PreludePubMod::Allowed));
        assert!(!bool::from(PreludePubMod::Reviewed));
    }

    #[test]
    fn default_is_allowed() {
        assert_eq!(PreludePubMod::default(), PreludePubMod::Allowed);
    }
}
