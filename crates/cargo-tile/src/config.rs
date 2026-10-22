//! `cargo-tile` configuration: the sections of
//! `<os config dir>/cargo-tile/config.toml`, and the identity
//! `tui_pane` finds that file, the keymap file and the themes directory
//! by.

use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use serde::Serialize;
use tui_pane::AppConfig;
use tui_pane::AppIdentity;
use tui_pane::AppearanceConfig;
use tui_pane::InitialRows;
use tui_pane::TileFill;
use tui_pane::TileGrowth;
use tui_pane::TileView;

use crate::constants::ARGUMENT_ASSIGNMENT;
use crate::constants::BINARY_NAME;
use crate::constants::CONFIG_DIRNAME;
use crate::constants::DEFAULT_ARGUMENTS;
use crate::constants::DEFAULT_CAPTURE_AUTO_INSTALL;
use crate::constants::DEFAULT_DARK_THEME;
use crate::constants::DEFAULT_EXCLUDED;
use crate::constants::DEFAULT_FADE_SECONDS;
use crate::constants::DEFAULT_HIDDEN_COMMANDS;
use crate::constants::DEFAULT_LIGHT_THEME;
use crate::constants::DEFAULT_WIDEN_SUMMARY;
use crate::constants::FLAG_MARK;
use crate::constants::MAX_FADE_SECONDS;

/// cargo-tile's identity for the framework's config and runner paths.
pub(crate) enum CargoTile {}

impl AppIdentity for CargoTile {
    const BINARY_NAME: &'static str = BINARY_NAME;
    const CONFIG_DIRNAME: &'static str = CONFIG_DIRNAME;
    const DEFAULT_LIGHT_THEME: &'static str = DEFAULT_LIGHT_THEME;
    const DEFAULT_DARK_THEME: &'static str = DEFAULT_DARK_THEME;
    const RETIRED_CONFIG_KEYS: &'static [&'static [&'static str]] = Config::RETIRED_KEYS;
}

/// Argument names kept in lexical order without duplicates at every config boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DefaultArguments(Arc<[String]>);

impl DefaultArguments {
    /// Establish ordering once when entries enter the configuration.
    pub(crate) fn new(mut entries: Vec<String>) -> Self {
        entries.sort_unstable();
        entries.dedup();
        Self(entries.into())
    }

    /// Sorted names for the settings editor and display selection.
    pub(crate) fn entries(&self) -> &[String] { &self.0 }

    /// Match a flag's name independently of its leading dashes and assigned value.
    pub(crate) fn contains(&self, argument: &str) -> bool {
        let name = argument
            .trim_start_matches(FLAG_MARK)
            .split(ARGUMENT_ASSIGNMENT)
            .next()
            .unwrap_or_default();
        self.0
            .binary_search_by(|entry| entry.as_str().cmp(name))
            .is_ok()
    }
}

impl Default for DefaultArguments {
    fn default() -> Self {
        Self::new(
            DEFAULT_ARGUMENTS
                .iter()
                .map(|name| (*name).to_string())
                .collect(),
        )
    }
}

impl Serialize for DefaultArguments {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.entries().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for DefaultArguments {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Vec::<String>::deserialize(deserializer).map(Self::new)
    }
}

/// Cargo subcommands omitted by the scanner or hidden with their whole subtree.
#[derive(Debug, Serialize)]
pub(crate) struct CommandsConfig {
    /// Commands omitted by the scanner while descendants remain eligible.
    pub(crate) excluded:          Vec<String>,
    /// Commands whose whole subtree stays out of every table and total.
    pub(crate) hidden:            Vec<String>,
    /// Argument names shown without enabling full argument detail.
    pub(crate) default_arguments: DefaultArguments,
}

impl<'de> Deserialize<'de> for CommandsConfig {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        /// Optional keys exist only at the saved-file boundary.
        #[derive(Deserialize)]
        struct SavedCommands {
            excluded:          Option<Vec<String>>,
            hidden:            Option<Vec<String>>,
            hidden_when_idle:  Option<Vec<String>>,
            default_arguments: Option<DefaultArguments>,
        }
        let saved = SavedCommands::deserialize(deserializer)?;
        let defaults = Self::default();
        Ok(Self {
            default_arguments: saved
                .default_arguments
                .unwrap_or(defaults.default_arguments),
            excluded:          saved.excluded.unwrap_or(defaults.excluded),
            hidden:            saved
                .hidden
                .or(saved.hidden_when_idle)
                .unwrap_or(defaults.hidden),
        })
    }
}

impl Default for CommandsConfig {
    fn default() -> Self {
        Self {
            default_arguments: DefaultArguments::default(),
            excluded:          DEFAULT_EXCLUDED
                .iter()
                .map(|subcommand| (*subcommand).to_string())
                .collect(),
            hidden:            DEFAULT_HIDDEN_COMMANDS
                .iter()
                .map(|subcommand| (*subcommand).to_string())
                .collect(),
        }
    }
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
    /// Seconds a finished row stays on screen, greyed, before it and any
    /// cell it leaves empty go. Read through
    /// [`TilesConfig::fade`], which enforces the ceiling.
    pub(crate) fade_seconds:  u64,
}

impl Default for TilesConfig {
    fn default() -> Self {
        Self {
            initial_rows:  InitialRows::default(),
            fill:          TileFill::default(),
            view:          TileView::default(),
            widen_summary: DEFAULT_WIDEN_SUMMARY,
            fade_seconds:  DEFAULT_FADE_SECONDS,
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

    /// How long a finished row lingers before the display lets go of it,
    /// clamped on read for the same reason as [`growth`](Self::growth).
    pub(crate) fn fade(&self) -> Duration {
        Duration::from_secs(self.fade_seconds.min(MAX_FADE_SECONDS))
    }
}

/// How the grid installs the capture shim. The former `roots` key no longer
/// exists; old entries are ignored because every account uses the shared parent.
#[derive(Debug, Deserialize, Serialize)]
#[serde(default)]
pub(crate) struct CaptureConfig {
    /// Put the capture shim in front of every toolchain's cargo when
    /// the grid opens, and bring an installed one up to date. On, the
    /// grid reports progress from its first launch and repairs itself
    /// after `rustup update`; off, the shim is only ever touched by
    /// `cargo tile install` and `cargo tile uninstall`. The grid never
    /// takes the shim out on its own either way.
    pub(crate) auto_install: bool,
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            auto_install: DEFAULT_CAPTURE_AUTO_INSTALL,
        }
    }
}

/// Parsed `config.toml`. Every section defaults, so a missing file and
/// an empty file behave the same.
#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub(crate) struct Config {
    /// `[appearance]` — theme selection.
    pub(crate) appearance: AppearanceConfig<CargoTile>,
    /// `[capture]` — automatic shim installation.
    pub(crate) capture:    CaptureConfig,
    /// `[commands]` — which commands the grid excludes or hides.
    pub(crate) commands:   CommandsConfig,
    /// `[tiles]` — how the tile grid is arranged and viewed.
    pub(crate) tiles:      TilesConfig,
}

impl Config {
    /// Saved keys whose values have migrated to the current schema.
    const RETIRED_KEYS: &'static [&'static [&'static str]] = &[&["commands", "hidden_when_idle"]];
}

impl AppConfig for Config {
    type Identity = CargoTile;

    fn appearance(&self) -> &AppearanceConfig<CargoTile> { &self.appearance }

    fn appearance_mut(&mut self) -> &mut AppearanceConfig<CargoTile> { &mut self.appearance }

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
    use std::path::PathBuf;

    use toml::Value;

    use super::*;

    /// What [`LoadedConfig::load`] compares against, for a config that
    /// has been through the file and back.
    fn round_trip(text: &str) -> String {
        let config: Config = toml::from_str(text).expect("a config the test wrote should parse");
        toml::to_string_pretty(&config).expect("a config should serialize")
    }

    #[test]
    fn the_default_arguments_setting_saves_sorted() {
        let written =
            round_trip("[commands]\ndefault_arguments = [\"workspace\", \"fix\", \"workspace\"]\n");
        let value: Value = toml::from_str(&written).expect("saved configuration");
        assert_eq!(
            value["commands"]["default_arguments"],
            Value::Array(vec![
                Value::String("fix".into()),
                Value::String("workspace".into()),
            ])
        );
    }

    /// [`LoadedConfig::load`] writes the file back whenever it does not already hold
    /// every setting, so a file that does hold them must serialize to
    /// itself -- otherwise every startup rewrites the config, for good.
    #[test]
    fn a_complete_config_is_left_alone() {
        let complete =
            toml::to_string_pretty(&Config::default()).expect("a config should serialize");
        assert_eq!(round_trip(&complete), complete);
    }

    #[test]
    fn legacy_hidden_commands_load_and_the_new_key_takes_precedence() {
        for (text, expected) in [
            (
                "[commands]\nhidden_when_idle = [\"check\"]\n",
                vec!["check"],
            ),
            (
                "[commands]\nhidden_when_idle = [\"check\"]\nhidden = [\"test\"]\n",
                vec!["test"],
            ),
            (
                "[commands]\nhidden_when_idle = [\"check\"]\nhidden = []\n",
                vec![],
            ),
        ] {
            let config: Config = toml::from_str(text).expect("saved command policy");
            assert_eq!(config.commands.hidden, expected);
        }
    }

    #[test]
    fn loading_legacy_hidden_commands_retires_the_saved_key() {
        enum SavedCargoTile {}
        impl AppIdentity for SavedCargoTile {
            const BINARY_NAME: &'static str = CargoTile::BINARY_NAME;
            const CONFIG_DIRNAME: &'static str = CargoTile::CONFIG_DIRNAME;
            const DEFAULT_LIGHT_THEME: &'static str = CargoTile::DEFAULT_LIGHT_THEME;
            const DEFAULT_DARK_THEME: &'static str = CargoTile::DEFAULT_DARK_THEME;
            const RETIRED_CONFIG_KEYS: &'static [&'static [&'static str]] =
                CargoTile::RETIRED_CONFIG_KEYS;

            fn config_path() -> Option<PathBuf> {
                Some(std::env::temp_dir().join(format!(
                    "cargo-tile-retired-key-{}.toml",
                    std::process::id()
                )))
            }
        }
        let path = SavedCargoTile::config_path().expect("scratch config path");
        std::fs::write(&path, "[commands]\nhidden_when_idle = [\"check\"]\n")
            .expect("legacy config writes");
        let loaded = LoadedConfig::load::<SavedCargoTile>();
        assert_eq!(loaded.error, None);
        assert_eq!(loaded.config.commands.hidden, ["check"]);
        let written = std::fs::read_to_string(&path).expect("saved config reads");
        let value: Value = toml::from_str(&written).expect("saved config parses");
        assert_eq!(
            value["commands"]["hidden"].as_array().expect("hidden list"),
            &[Value::String("check".into())]
        );
        assert_eq!(value["commands"].get("hidden_when_idle"), None);
        let loaded_again = LoadedConfig::load::<SavedCargoTile>();
        assert_eq!(loaded_again.error, None);
        assert_eq!(
            std::fs::read_to_string(&path).expect("saved config reads"),
            written
        );
        std::fs::remove_file(path).expect("scratch config removes");
    }

    #[test]
    fn summary_tile_view_round_trips() {
        let restated = round_trip("[tiles]\nview = \"summary\"\n");
        assert!(restated.contains("view = \"summary\""));
        assert_eq!(round_trip(&restated), restated);
    }

    /// The case the write exists for: a file written before a section
    /// existed comes back carrying it.
    #[test]
    fn a_config_missing_a_section_gains_it() {
        let old = "[appearance]\nmode = \"dark\"\n";
        let restated = round_trip(old);
        assert_ne!(restated, old);
        assert!(restated.contains("[capture]"));
        assert!(!restated.contains("roots ="));
        assert!(restated.contains("[commands]"));
        assert!(restated.contains("[tiles]"));
        // What the file did say survives the rewrite; only what it left
        // out is filled in.
        assert!(restated.contains("mode = \"dark\""));
    }

    /// Every key the default config writes, in the order and spelling
    /// `config.toml` has always carried. A change here rewrites every
    /// user's file on their next startup.
    #[test]
    fn the_default_config_serializes_to_the_known_file() {
        let expected = "\
[appearance]
mode = \"auto\"
light_theme = \"Default Light\"
dark_theme = \"Default Dark\"
iterm2_profile = \"cargo-tile\"
transparent = true

[capture]
auto_install = true

[commands]
excluded = [\"berth\"]
hidden = [\n    \"port\",\n    \"handler\",\n]
default_arguments = [\n    \"all-targets\",\n    \"fix\",\n    \"workspace\",\n]

[tiles]
initial_rows = 4
fill = \"redistribute\"
view = \"cells\"
widen_summary = false
fade_seconds = 3
";
        let written =
            toml::to_string_pretty(&Config::default()).expect("a config should serialize");
        assert_eq!(written, expected);
    }

    /// A file that leaves `iterm2_profile` and the theme ids out gets the
    /// app's own defaults for them when it is restated.
    #[test]
    fn a_config_missing_iterm2_profile_restates_the_binary_name() {
        let restated = round_trip("[appearance]\nmode = \"dark\"\n");
        assert!(restated.contains("iterm2_profile = \"cargo-tile\""));
        assert!(restated.contains("light_theme = \"Default Light\""));
    }

    #[test]
    fn obsolete_roots_are_ignored_and_auto_install_is_preserved() {
        for roots in ["[]", "[\"/old/captures\"]", "42"] {
            let text = format!("[capture]\nauto_install = false\nroots = {roots}\n");
            let config: Config = toml::from_str(&text).expect("obsolete keys should be ignored");
            assert!(!config.capture.auto_install);
            let restated = round_trip(&text);
            assert!(!restated.contains("roots"));
            assert_eq!(round_trip(&restated), restated);
        }
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
            parsed_fill("[tiles]\nfade_seconds = 5\n"),
            TileFill::Redistribute
        );
    }
}
