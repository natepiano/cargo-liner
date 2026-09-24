//! [`LoadedConfig`]: an app's `config.toml` read into its own serde
//! struct, written back when the file does not spell out every setting,
//! and saved after edits.

use std::fs;
use std::path::PathBuf;

use serde::Serialize;
use serde::de::DeserializeOwned;

use super::constants::CONFIG_FILENAME;
use super::identity::AppIdentity;

/// A load attempt: the config that will be used, plus the text of
/// whatever went wrong reading it or writing it back. Surfaced in the
/// settings overlay's Notices.
#[derive(Debug)]
pub struct LoadedConfig<C> {
    /// The config the app runs with.
    pub config: C,
    /// Parse, restate or save failure text.
    pub error:  Option<String>,
}

impl<C: Default + DeserializeOwned + Serialize> LoadedConfig<C> {
    /// Read [`AppIdentity::config_path`], falling back to defaults when
    /// it is absent; a file that parses but does not already spell out
    /// every setting is written back (restated).
    ///
    /// A parse error is not fatal: the app runs on defaults and reports
    /// the error through [`Self::error`], because the terminal is not
    /// yet in raw mode here and a panic would leave nothing on screen.
    #[must_use]
    pub fn load<I: AppIdentity>() -> Self { Self::load_from(I::config_path()) }

    /// [`Self::load`] against an explicit path.
    fn load_from(path: Option<PathBuf>) -> Self {
        let Some(path) = path else {
            return Self::defaults(None);
        };
        let Ok(text) = fs::read_to_string(&path) else {
            return Self::defaults(None);
        };
        toml::from_str(&text).map_or_else(
            |error| Self::defaults(Some(error.to_string())),
            |config| {
                let mut loaded = Self {
                    config,
                    error: None,
                };
                loaded.restate(path, &text);
                loaded
            },
        )
    }

    /// The default config, carrying `error`.
    fn defaults(error: Option<String>) -> Self {
        Self {
            config: C::default(),
            error,
        }
    }
}

impl<C: Serialize> LoadedConfig<C> {
    /// Write [`Self::config`] to [`AppIdentity::config_path`], creating
    /// the config directory when it is missing, and keep any failure in
    /// [`Self::error`] (cleared on success).
    ///
    /// The failure is kept as text for the settings overlay rather than
    /// returned as an error type: every caller renders it, none of them
    /// recover.
    pub fn save<I: AppIdentity>(&mut self) { self.save_to(I::config_path()); }

    /// [`Self::save`] against an explicit path.
    fn save_to(&mut self, path: Option<PathBuf>) { self.error = write_config(&self.config, path); }

    /// Write the parsed config back over the file it came from when the
    /// file does not already say the same thing.
    ///
    /// A file written before a setting existed does not mention it,
    /// which leaves that setting editable only by someone who already
    /// knows its name. Writing the parsed config back spells out every
    /// section at its default, so the file lists the whole of what can
    /// be set. It is a no-op once the file holds everything, and it is
    /// only reached on a file that parsed -- one with a typo in it is
    /// left alone for its author to fix rather than overwritten.
    fn restate(&mut self, path: PathBuf, text: &str) {
        match toml::to_string_pretty(&self.config) {
            Ok(restated) if restated == text => {},
            Ok(_) => self.save_to(Some(path)),
            Err(error) => self.error = Some(error.to_string()),
        }
    }
}

/// Serialize `config` to `path`, creating its directory. Returns the
/// failure text, if any.
fn write_config<C: Serialize>(config: &C, path: Option<PathBuf>) -> Option<String> {
    let Some(path) = path else {
        return Some(format!(
            "no OS config directory: cannot write {CONFIG_FILENAME}"
        ));
    };
    let text = match toml::to_string_pretty(config) {
        Ok(text) => text,
        Err(error) => return Some(error.to_string()),
    };
    if let Some(parent) = path.parent()
        && let Err(error) = fs::create_dir_all(parent)
    {
        return Some(format!("{}: {error}", parent.display()));
    }
    fs::write(&path, text)
        .err()
        .map(|error| format!("{}: {error}", path.display()))
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::path::Path;

    use serde::Deserialize;

    use super::*;

    /// A config with two keys, both defaulted to something other than
    /// their type's zero so a fallback to defaults is visible.
    #[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
    #[serde(default)]
    struct Sample {
        name:  String,
        count: u32,
    }

    impl Default for Sample {
        fn default() -> Self {
            Self {
                name:  "sample".to_string(),
                count: 3,
            }
        }
    }

    /// A directory of the test's own under the system temp directory,
    /// removed when dropped. Named by process and test so tests running
    /// side by side never share one.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(test: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("tui_pane-{test}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).expect("the scratch directory should be created");
            Self(dir)
        }

        fn config_path(&self) -> PathBuf { self.0.join(CONFIG_FILENAME) }
    }

    impl Drop for Scratch {
        fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
    }

    fn read(path: &Path) -> String {
        fs::read_to_string(path).expect("the config file should be readable")
    }

    /// A file that already spells out every setting must not be written
    /// again, or every startup rewrites the config for good. The file
    /// is made read-only, so a write would come back as an error.
    #[test]
    fn restate_is_a_no_op_for_a_complete_file() {
        let scratch = Scratch::new("restate_no_op");
        let path = scratch.config_path();
        let complete =
            toml::to_string_pretty(&Sample::default()).expect("a config should serialize");
        fs::write(&path, &complete).expect("the config file should be written");
        let mut permissions = fs::metadata(&path)
            .expect("the config file should exist")
            .permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&path, permissions).expect("the config file should be made read-only");

        let loaded = LoadedConfig::<Sample>::load_from(Some(path.clone()));

        assert_eq!(loaded.error, None);
        assert_eq!(loaded.config, Sample::default());
        assert_eq!(read(&path), complete);
    }

    /// A file missing a key comes back carrying it, and keeps what it
    /// did say.
    #[test]
    fn a_file_missing_a_key_is_restated() {
        let scratch = Scratch::new("restate_missing_key");
        let path = scratch.config_path();
        fs::write(&path, "name = \"kept\"\n").expect("the config file should be written");

        let loaded = LoadedConfig::<Sample>::load_from(Some(path.clone()));

        assert_eq!(loaded.error, None);
        assert_eq!(read(&path), "name = \"kept\"\ncount = 3\n");
    }

    /// A file that does not parse leaves the app on defaults, keeps the
    /// parse error's text for the settings overlay, and is left alone
    /// for its author to fix.
    #[test]
    fn a_parse_error_falls_back_to_defaults_and_keeps_the_text() {
        let scratch = Scratch::new("parse_error");
        let path = scratch.config_path();
        let broken = "count = \"three\"\n";
        fs::write(&path, broken).expect("the config file should be written");

        let loaded = LoadedConfig::<Sample>::load_from(Some(path.clone()));

        assert_eq!(loaded.config, Sample::default());
        let error = loaded.error.expect("a parse error should be kept");
        assert!(error.contains("invalid type"), "{error}");
        assert_eq!(read(&path), broken);
    }

    /// Saving creates the config directory and clears an earlier error.
    #[test]
    fn save_creates_the_directory_and_clears_the_error() {
        let scratch = Scratch::new("save");
        let path = scratch.0.join("nested").join(CONFIG_FILENAME);
        let mut loaded = LoadedConfig {
            config: Sample::default(),
            error:  Some("stale".to_string()),
        };

        loaded.save_to(Some(path.clone()));

        assert_eq!(loaded.error, None);
        assert_eq!(read(&path), "name = \"sample\"\ncount = 3\n");
    }

    /// With no OS config directory there is nowhere to write, and the
    /// error says which file was lost.
    #[test]
    fn saving_without_a_config_directory_names_the_file() {
        let mut loaded = LoadedConfig {
            config: Sample::default(),
            error:  None,
        };

        loaded.save_to(None);

        assert_eq!(
            loaded.error.as_deref(),
            Some("no OS config directory: cannot write config.toml")
        );
    }
}
