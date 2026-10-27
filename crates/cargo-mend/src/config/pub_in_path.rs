use serde::Deserialize;
use serde::Serialize;

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum PubInPath {
    /// `pub(in ...)` produces an error everywhere.
    Forbidden,
    /// Exact-boundary `pub(in ...)` and `pub` are both accepted.
    Permitted,
    /// Exact-boundary `pub(in ...)` is accepted while `pub` is reviewed.
    #[default]
    Required,
}

impl PubInPath {
    /// The spelling this variant takes in a config file, matching the
    /// `rename_all = "lowercase"` serde representation.
    pub(crate) const fn config_value(self) -> &'static str {
        match self {
            Self::Forbidden => "forbidden",
            Self::Permitted => "permitted",
            Self::Required => "required",
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use serde::Deserialize;
    use serde::Serialize;

    use super::PubInPath;
    use crate::config::constants::PUB_IN_PATH_KEY;

    #[derive(Deserialize, Serialize)]
    struct Config {
        pub_in_path: PubInPath,
    }

    #[test]
    fn default_is_required() {
        assert_eq!(PubInPath::default(), PubInPath::Required);
    }

    #[test]
    fn string_values_round_trip_through_serde() {
        for (value, pub_in_path) in [
            (PubInPath::Forbidden.config_value(), PubInPath::Forbidden),
            (PubInPath::Permitted.config_value(), PubInPath::Permitted),
            (PubInPath::Required.config_value(), PubInPath::Required),
        ] {
            let parsed: Config =
                toml::from_str(&format!("{PUB_IN_PATH_KEY} = \"{value}\"\n")).unwrap();

            assert_eq!(parsed.pub_in_path, pub_in_path);
            assert_eq!(
                toml::to_string(&parsed).unwrap(),
                format!("{PUB_IN_PATH_KEY} = \"{value}\"\n")
            );
        }
    }
}
