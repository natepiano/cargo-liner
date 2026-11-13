//! `cargo-handler` configuration: the sections of
//! `<os config dir>/cargo-handler/config.toml`, and the identity
//! `tui_pane` finds that file, the keymap file, the favorites file and
//! the themes directory by.

use std::collections::BTreeMap;
use std::fmt::Formatter;
#[cfg(test)]
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;
use std::time::Duration;

use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use serde::Serializer;
use serde::de::Error;
use serde::de::Visitor;
use tui_pane::AppConfig;
use tui_pane::AppIdentity;
use tui_pane::AppearanceConfig;
use tui_pane::InitialRows;
use tui_pane::TileFill;
use tui_pane::TileGrowth;
use tui_pane::TileView;

use crate::constants::BINARY_NAME;
use crate::constants::CONFIG_DIRNAME;
use crate::constants::DEFAULT_DARK_THEME;
use crate::constants::DEFAULT_LIGHT_THEME;
use crate::constants::DEFAULT_WIDEN_SUMMARY;
use crate::constants::HERDR_PROBE_WAIT_LIMIT;
use crate::constants::PROBE_TIMEOUT;
#[cfg(test)]
use crate::constants::TEST_CONFIG_ROOT;

/// cargo-handler's identity for the framework's config and runner paths.
pub(crate) enum CargoHandler {}

/// In a test build, `AppIdentity` resolves `config.toml`, `themes/` and
/// `keymap.toml` under `TEST_CONFIG_ROOT` instead of the OS config directory.
/// `settings::rows` includes these paths, and their widths determine the
/// settings overlay's width. A fixed root keeps that width the same on every
/// machine and prevents tests from writing the user's config or keymap.
impl AppIdentity for CargoHandler {
    const BINARY_NAME: &'static str = BINARY_NAME;
    const CONFIG_DIRNAME: &'static str = CONFIG_DIRNAME;
    const DEFAULT_LIGHT_THEME: &'static str = DEFAULT_LIGHT_THEME;
    const DEFAULT_DARK_THEME: &'static str = DEFAULT_DARK_THEME;

    #[cfg(test)]
    fn config_path() -> Option<PathBuf> { Some(test_config_path("config.toml")) }

    #[cfg(test)]
    fn keymap_path() -> Option<PathBuf> { Some(test_config_path("keymap.toml")) }

    #[cfg(test)]
    fn themes_dir() -> Option<PathBuf> { Some(test_config_path("themes")) }
}

/// Where a test build finds `name`: in cargo-handler's directory under
/// [`TEST_CONFIG_ROOT`].
#[cfg(test)]
fn test_config_path(name: &str) -> PathBuf {
    Path::new(TEST_CONFIG_ROOT).join(CONFIG_DIRNAME).join(name)
}

/// Tile grid arrangement and view settings.
#[derive(Debug, Deserialize, Serialize)]
#[serde(default)]
pub(crate) struct TilesConfig {
    /// Rows the grid grows to in a single column before it starts
    /// arranging itself into a square. Read through
    /// [`TilesConfig::growth`], which enforces the floor.
    pub(crate) initial_rows:  InitialRows,
    /// How the cells spread over the columns once there is more than
    /// one: `add_new` or `redistribute`.
    pub(crate) fill:          TileFill,
    /// Whether the grid shows the summary alone or uses cells whenever they fit.
    pub(crate) view:          TileView,
    /// Whether the summary widens over the next columns when its widest
    /// line does not fit its own; see [`TileGrowth::widen_summary`].
    pub(crate) widen_summary: bool,
}

impl Default for TilesConfig {
    fn default() -> Self {
        Self {
            initial_rows:  InitialRows::default(),
            fill:          TileFill::default(),
            view:          TileView::default(),
            widen_summary: DEFAULT_WIDEN_SUMMARY,
        }
    }
}

impl TilesConfig {
    /// The grid's growth: the single column's rows, never below one
    /// (see [`InitialRows::get`]), the fill, and whether the summary
    /// widens.
    pub(crate) fn growth(&self) -> TileGrowth {
        TileGrowth {
            initial_rows:  self.initial_rows.get(),
            fill:          self.fill,
            widen_summary: self.widen_summary,
        }
    }
}

/// The machines the summary lists besides this one.
#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub(crate) struct MachinesConfig {
    /// ssh host names, each probed for its agents, listed in this
    /// order after this machine.
    pub(crate) remote: Vec<String>,
}

/// How this machine discovers relationships between its agents.
#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub(crate) struct CensusConfig {
    /// The staff file path. An empty path disables staff discovery.
    pub(crate) staff_file:                StaffFilePath,
    /// Optional command printing stored account roles and usage as JSON.
    pub(crate) accounts_command:          String,
    /// Seconds after process startup in which a new Codex thread can match.
    pub(crate) codex_thread_start_window: CodexStartWindow,
    /// Milliseconds from scan start allowed for the probe to obtain herdr names.
    pub(crate) herdr_probe_wait:          HerdrProbeWait,
    /// Milliseconds allowed for each herdr socket read or write.
    pub(crate) herdr_read_timeout:        HerdrReadTimeout,
}

/// The configured staff file path, including its default location.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub(crate) struct StaffFilePath(pub(crate) String);

impl Default for StaffFilePath {
    fn default() -> Self { Self("~/.local/state/staff/staff.json".to_owned()) }
}

/// The configured span for the Codex startup fallback, in seconds.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(transparent)]
pub(crate) struct CodexStartWindow(u64);

impl Default for CodexStartWindow {
    fn default() -> Self { Self(60) }
}

impl CodexStartWindow {
    pub(crate) const fn milliseconds(self) -> u64 { self.0.saturating_mul(1000) }
}

/// A bound in milliseconds from scan start that reserves time for ssh and output.
/// The 12-second default covers the reported 11-second probe at load 47.
/// Five current probes took 1.60–5.88 seconds at load 82–84.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(try_from = "u64", into = "u64")]
pub(crate) struct HerdrProbeWait(u64);

impl Default for HerdrProbeWait {
    fn default() -> Self { Self(12_000) }
}

impl HerdrProbeWait {
    pub(crate) const fn duration(self) -> Duration { Duration::from_millis(self.0) }
}

impl TryFrom<u64> for HerdrProbeWait {
    type Error = String;

    fn try_from(milliseconds: u64) -> Result<Self, Self::Error> {
        if Duration::from_millis(milliseconds) > HERDR_PROBE_WAIT_LIMIT {
            return Err(format!(
                "herdr_probe_wait must not exceed {} ms",
                HERDR_PROBE_WAIT_LIMIT.as_millis()
            ));
        }
        Ok(Self(milliseconds))
    }
}

impl From<HerdrProbeWait> for u64 {
    fn from(wait: HerdrProbeWait) -> Self { wait.0 }
}

/// A positive herdr socket timeout in milliseconds, within the remote probe deadline.
/// Three of 20 snapshot reads exceeded 100 ms at load 79; the maximum was 155 ms.
/// The 1,000 ms default allows more time for a loaded service.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(try_from = "u64", into = "u64")]
pub(crate) struct HerdrReadTimeout(u64);

impl Default for HerdrReadTimeout {
    fn default() -> Self { Self(1_000) }
}

impl HerdrReadTimeout {
    pub(crate) const fn duration(self) -> Duration { Duration::from_millis(self.0) }
}

impl TryFrom<u64> for HerdrReadTimeout {
    type Error = String;

    fn try_from(milliseconds: u64) -> Result<Self, Self::Error> {
        if milliseconds == 0 || Duration::from_millis(milliseconds) > PROBE_TIMEOUT {
            return Err(format!(
                "herdr_read_timeout must be between 1 and {} ms",
                PROBE_TIMEOUT.as_millis()
            ));
        }
        Ok(Self(milliseconds))
    }
}

impl From<HerdrReadTimeout> for u64 {
    fn from(timeout: HerdrReadTimeout) -> Self { timeout.0 }
}

/// `[accounts]`: each agent home this machine runs, mapped to the label its tiles show.
#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(transparent)]
pub(crate) struct AccountsConfig(pub(crate) BTreeMap<String, String>);

/// `WordHighlightEmphasis` sets the configured strength of a matched word.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum WordHighlightEmphasis {
    Subtle,
    #[default]
    Clear,
    Strong,
}

impl WordHighlightEmphasis {
    const VALUES: [Self; 3] = [Self::Subtle, Self::Clear, Self::Strong];
    const NAMES: [&'static str; 3] = [Self::Subtle.name(), Self::Clear.name(), Self::Strong.name()];

    const fn name(self) -> &'static str {
        match self {
            Self::Subtle => "subtle",
            Self::Clear => "clear",
            Self::Strong => "strong",
        }
    }
}

impl Serialize for WordHighlightEmphasis {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.name())
    }
}

/// `WordHighlightEmphasisVisitor` names all allowed values for every rejected TOML type.
struct WordHighlightEmphasisVisitor;

impl Visitor<'_> for WordHighlightEmphasisVisitor {
    type Value = WordHighlightEmphasis;

    fn expecting(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("one of ")?;
        formatter
            .debug_list()
            .entries(WordHighlightEmphasis::NAMES)
            .finish()
    }

    fn visit_str<E: Error>(self, value: &str) -> Result<Self::Value, E> {
        WordHighlightEmphasis::VALUES
            .into_iter()
            .find(|emphasis| emphasis.name() == value)
            .ok_or_else(|| E::unknown_variant(value, &WordHighlightEmphasis::NAMES))
    }
}

impl<'de> Deserialize<'de> for WordHighlightEmphasis {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_str(WordHighlightEmphasisVisitor)
    }
}

/// `WordHighlightConfig` controls `[word_highlight]` without changing the theme.
#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub(crate) struct WordHighlightConfig {
    pub(crate) emphasis: WordHighlightEmphasis,
}

/// Parsed `config.toml`. Every section defaults, so a missing file and
/// an empty file behave the same.
#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub(crate) struct Config {
    /// `[appearance]` — theme selection.
    pub(crate) appearance:     AppearanceConfig<CargoHandler>,
    /// `Config::word_highlight` sets the strength of matching words.
    pub(crate) word_highlight: WordHighlightConfig,
    /// `[tiles]` — how the tile grid is arranged and viewed.
    pub(crate) tiles:          TilesConfig,
    /// `[machines]` — the remote machines the summary lists.
    pub(crate) machines:       MachinesConfig,
    /// `[census]` — discovery of this machine's agent hierarchy.
    pub(crate) census:         CensusConfig,
    /// `[accounts]` — labels for this machine's agent configuration homes.
    pub(crate) accounts:       AccountsConfig,
}

impl AppConfig for Config {
    type Identity = CargoHandler;

    fn appearance(&self) -> &AppearanceConfig<CargoHandler> { &self.appearance }

    fn appearance_mut(&mut self) -> &mut AppearanceConfig<CargoHandler> { &mut self.appearance }

    fn initial_rows_mut(&mut self) -> &mut InitialRows { &mut self.tiles.initial_rows }

    fn tile_fill_mut(&mut self) -> &mut TileFill { &mut self.tiles.fill }

    fn tile_view_mut(&mut self) -> &mut TileView { &mut self.tiles.view }

    fn widen_summary_mut(&mut self) -> &mut bool { &mut self.tiles.widen_summary }
}

/// `config.toml` as loaded, with whatever went wrong reading or
/// writing it.
pub(crate) type LoadedConfig = tui_pane::LoadedConfig<Config>;

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::*;

    /// What [`LoadedConfig::load`] compares against, for a config that
    /// has been through the file and back.
    fn round_trip(text: &str) -> String {
        let config: Config = toml::from_str(text).expect("a config the test wrote should parse");
        toml::to_string_pretty(&config).expect("a config should serialize")
    }

    /// Every key the default config writes, in the order and spelling
    /// `config.toml` carries. A change here rewrites every user's file
    /// on their next startup.
    #[test]
    fn the_default_config_serializes_to_the_known_file() {
        let expected = "\
[appearance]
mode = \"auto\"
light_theme = \"Default Light\"
dark_theme = \"Default Dark\"
iterm2_profile = \"cargo-handler\"
transparent = true

[word_highlight]
emphasis = \"clear\"

[tiles]
initial_rows = 4
fill = \"redistribute\"
view = \"cells\"
widen_summary = true

[machines]
remote = []

[census]
staff_file = \"~/.local/state/staff/staff.json\"
accounts_command = \"\"
codex_thread_start_window = 60
herdr_probe_wait = 12000
herdr_read_timeout = 1000

[accounts]
";
        let written =
            toml::to_string_pretty(&Config::default()).expect("a config should serialize");
        assert_eq!(written, expected);
    }

    #[test]
    fn word_highlight_emphasis_parses_and_reports_allowed_values() {
        for emphasis in ["subtle", "clear", "strong"] {
            let saved = round_trip(&format!("[word_highlight]\nemphasis = \"{emphasis}\"\n"));
            assert!(saved.contains(&format!("emphasis = \"{emphasis}\"")));
            assert_eq!(round_trip(&saved), saved);
        }
        for value in ["'invisible'", "1", "true", "1.5", "[]", "{}", "1979-05-27"] {
            let error =
                toml::from_str::<Config>(&format!("[word_highlight]\nemphasis = {value}\n"))
                    .expect_err("an invalid emphasis must fail");
            for allowed in ["subtle", "clear", "strong"] {
                assert!(error.to_string().contains(allowed), "{value}: {error}");
            }
        }
    }

    #[test]
    fn herdr_probe_wait_defaults_and_configured_limits_survive_serialization() {
        let missing: Config = toml::from_str("[census]\n").expect("missing keys");
        assert_eq!(
            missing.census.herdr_probe_wait.duration(),
            Duration::from_secs(12)
        );
        assert_eq!(
            missing.census.herdr_read_timeout.duration(),
            Duration::from_secs(1)
        );
        let configured: Config =
            toml::from_str("[census]\nherdr_probe_wait = 750\nherdr_read_timeout = 300\n")
                .expect("configured limits");
        let saved = toml::to_string(&configured).expect("saved limits");
        let restored: Config = toml::from_str(&saved).expect("restored limits");
        assert_eq!(
            restored.census.herdr_probe_wait.duration(),
            Duration::from_millis(750)
        );
        assert_eq!(
            restored.census.herdr_read_timeout.duration(),
            Duration::from_millis(300)
        );
    }

    #[test]
    fn herdr_probe_wait_must_fit_the_remote_probe_deadline() {
        let limit = HERDR_PROBE_WAIT_LIMIT.as_millis();
        let at_limit: Config = toml::from_str(&format!("[census]\nherdr_probe_wait = {limit}\n"))
            .expect("wait at the scan limit");
        assert_eq!(
            at_limit.census.herdr_probe_wait.duration(),
            HERDR_PROBE_WAIT_LIMIT
        );
        let rejected =
            toml::from_str::<Config>(&format!("[census]\nherdr_probe_wait = {}\n", limit + 1))
                .expect_err("wait beyond the scan limit");
        assert!(rejected.to_string().contains(&format!("{limit} ms")));
    }

    #[test]
    fn herdr_read_timeout_rejects_zero_and_remote_deadline_overflow() {
        for milliseconds in [0, PROBE_TIMEOUT.as_millis() + 1] {
            let rejected = toml::from_str::<Config>(&format!(
                "[census]\nherdr_read_timeout = {milliseconds}\n"
            ))
            .expect_err("invalid socket timeout");
            assert!(rejected.to_string().contains("herdr_read_timeout"));
        }
    }

    #[test]
    fn configured_accounts_parse_and_survive_config_serialization() {
        let config: Config = toml::from_str("[accounts]\n\"~/.claude\" = \"claude2\"\n")
            .expect("accounts table parses");
        assert_eq!(config.accounts.0.len(), 1);
        assert_eq!(
            config.accounts.0.get("~/.claude").map(String::as_str),
            Some("claude2")
        );

        let saved = toml::to_string(&config).expect("config serializes");
        let restored: Config = toml::from_str(&saved).expect("saved config parses");
        assert_eq!(restored.accounts.0, config.accounts.0);
        assert_eq!(
            toml::to_string(&restored).expect("restored config serializes"),
            saved
        );
    }

    #[test]
    fn config_without_an_accounts_table_has_no_labels() {
        let config: Config =
            toml::from_str("[census]\nstaff_file = \"~/.local/state/staff/staff.json\"\n")
                .expect("config from before accounts parses");
        assert!(config.accounts.0.is_empty());
    }

    /// [`LoadedConfig::load`] writes the file back whenever it does not
    /// already hold every setting, so a file that does hold them must
    /// serialize to itself -- otherwise every startup rewrites the
    /// config, for good.
    #[test]
    fn a_complete_config_is_left_alone() {
        let complete =
            toml::to_string_pretty(&Config::default()).expect("a config should serialize");
        assert_eq!(round_trip(&complete), complete);
    }

    #[test]
    fn summary_tile_view_round_trips() {
        let restated = round_trip("[tiles]\nview = \"summary\"\n");
        assert!(restated.contains("view = \"summary\""));
        assert_eq!(round_trip(&restated), restated);
    }

    /// Spaces in the staff path survive a config save.
    #[test]
    fn the_staff_file_round_trips_as_a_path() {
        let text = "[census]\nstaff_file = '/tmp/staff list.json'\n";
        let restated = round_trip(text);
        let reloaded: Config = toml::from_str(&restated).expect("the staff path should parse");
        assert_eq!(reloaded.census.staff_file.0, "/tmp/staff list.json");
        assert_eq!(round_trip(&restated), restated);
    }

    /// A file that leaves `iterm2_profile`, the theme ids and `[tiles]`
    /// out gets the app's own defaults for them when it is restated,
    /// and keeps what it did say.
    #[test]
    fn a_config_missing_keys_restates_the_binary_name() {
        let restated = round_trip("[appearance]\nmode = \"dark\"\n");
        assert!(restated.contains("mode = \"dark\""));
        assert!(restated.contains("iterm2_profile = \"cargo-handler\""));
        assert!(restated.contains("light_theme = \"Default Light\""));
        assert!(restated.contains("[tiles]"));
    }

    /// The fill a config spelling `text` loads with.
    fn parsed_fill(text: &str) -> TileFill {
        toml::from_str::<Config>(text)
            .expect("a config the test wrote should parse")
            .tiles
            .fill
    }

    /// `tiles.fill` parses in both spellings, and a `[tiles]` table
    /// that leaves it out gets redistribute.
    #[test]
    fn tile_fill_parses_both_values_and_falls_back_to_redistribute() {
        assert_eq!(
            parsed_fill("[tiles]\nfill = \"add_new\"\n"),
            TileFill::AddNew
        );
        assert_eq!(
            parsed_fill("[tiles]\nfill = \"redistribute\"\n"),
            TileFill::Redistribute
        );
        assert_eq!(
            parsed_fill("[tiles]\ninitial_rows = 3\n"),
            TileFill::Redistribute
        );
    }
}
