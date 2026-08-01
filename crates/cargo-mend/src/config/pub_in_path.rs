use serde::Deserialize;
use serde::Serialize;

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum PubInPath {
    /// `pub(in ...)` produces an error everywhere.
    Forbidden,
    /// Exact-boundary `pub(in ...)` and `pub` are both accepted.
    #[default]
    Permitted,
    /// Exact-boundary `pub(in ...)` is accepted while `pub` is reviewed.
    Required,
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

    #[derive(Deserialize, Serialize)]
    struct Config {
        pub_in_path: PubInPath,
    }

    #[test]
    fn default_is_permitted() {
        assert_eq!(PubInPath::default(), PubInPath::Permitted);
    }

    #[test]
    fn string_values_round_trip_through_serde() {
        for (value, pub_in_path) in [
            ("forbidden", PubInPath::Forbidden),
            ("permitted", PubInPath::Permitted),
            ("required", PubInPath::Required),
        ] {
            let parsed: Config = toml::from_str(&format!("pub_in_path = \"{value}\"\n")).unwrap();

            assert_eq!(parsed.pub_in_path, pub_in_path);
            assert_eq!(
                toml::to_string(&parsed).unwrap(),
                format!("pub_in_path = \"{value}\"\n")
            );
        }
    }
}
