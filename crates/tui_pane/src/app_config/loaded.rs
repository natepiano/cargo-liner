//! [`LoadedConfig`]: an app's `config.toml` read into its own serde
//! struct, written back when the file does not spell out every setting,
//! and saved after edits.

use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;

use serde::Serialize;
use serde::de::DeserializeOwned;
use toml::Table;
use toml::Value;
use toml_edit::DocumentMut;
use toml_edit::TableLike;

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
    /// every setting, or still holds retired keys, is written back (restated).
    ///
    /// A parse error is not fatal: the app runs on defaults and reports
    /// the error through [`Self::error`], because the terminal is not
    /// yet in raw mode here and a panic would leave nothing on screen.
    #[must_use]
    pub fn load<I: AppIdentity>() -> Self { Self::load_from::<I>(I::config_path()) }

    /// [`Self::load`] against an explicit path.
    fn load_from<I: AppIdentity>(path: Option<PathBuf>) -> Self {
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
                loaded.save_to::<I>(Some(path));
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
    pub fn save<I: AppIdentity>(&mut self) { self.save_to::<I>(I::config_path()); }

    /// [`Self::save`] against an explicit path.
    fn save_to<I: AppIdentity>(&mut self, path: Option<PathBuf>) {
        self.error = write_config::<C, I>(&self.config, path);
    }
}

/// Whether a merged document needs a filesystem write.
enum ConfigRewrite {
    /// Every known setting already has its requested value.
    Unchanged,
    /// The document with known settings updated and user text retained.
    Changed(String),
}

/// Merge known keys and retire declared paths, retaining unknown keys and user text.
fn merge_config<I: AppIdentity>(text: &str, serialized: &str) -> Result<ConfigRewrite, String> {
    let mut document = text
        .parse::<DocumentMut>()
        .map_err(|error| error.to_string())?;
    let desired = serialized
        .parse::<DocumentMut>()
        .map_err(|error| error.to_string())?;
    let current_values = toml::from_str::<toml::Table>(text).map_err(|error| error.to_string())?;
    let desired_values =
        toml::from_str::<toml::Table>(serialized).map_err(|error| error.to_string())?;
    merge_table(
        document.as_table_mut(),
        desired.as_table(),
        &current_values,
        &desired_values,
    );
    for path in I::RETIRED_CONFIG_KEYS {
        remove_retired_key(document.as_table_mut(), path);
    }
    let merged = document.to_string();
    Ok(if merged == text {
        ConfigRewrite::Unchanged
    } else {
        ConfigRewrite::Changed(merged)
    })
}

/// Remove only the exact table path declared by the app.
fn remove_retired_key(current: &mut dyn TableLike, path: &[&str]) {
    let Some((key, parents)) = path.split_first() else {
        return;
    };
    if parents.is_empty() {
        current.remove(key);
    } else if let Some(table) = current
        .get_mut(key)
        .and_then(toml_edit::Item::as_table_like_mut)
    {
        remove_retired_key(table, parents);
    }
}

/// Semantic equality keeps existing quoting and number formatting for unchanged values.
fn merge_table(
    current: &mut dyn TableLike,
    desired: &dyn TableLike,
    current_values: &Table,
    desired_values: &Table,
) {
    for (key, value) in desired.iter() {
        let Some(desired_value) = desired_values.get(key) else {
            continue;
        };
        if current_values.get(key) == Some(desired_value) {
            continue;
        }
        match (current.get_mut(key), current_values.get(key), desired_value) {
            (Some(existing), Some(Value::Table(old)), Value::Table(new)) => {
                if let (Some(existing), Some(desired)) =
                    (existing.as_table_like_mut(), value.as_table_like())
                {
                    merge_table(existing, desired, old, new);
                } else {
                    current.insert(key, value.clone());
                }
            },
            (Some(existing), _, _) => {
                let mut replacement = value.clone();
                if let (Some(old), Some(new)) = (existing.as_value(), replacement.as_value_mut()) {
                    *new.decor_mut() = old.decor().clone();
                }
                *existing = replacement;
            },
            (None, _, _) => {
                current.insert(key, value.clone());
            },
        }
    }
}

/// Merge `config` into `path`, creating its directory only when a write is needed.
fn write_config<C: Serialize, I: AppIdentity>(config: &C, path: Option<PathBuf>) -> Option<String> {
    let Some(path) = path else {
        return Some(format!(
            "no OS config directory: cannot write {CONFIG_FILENAME}"
        ));
    };
    let text = match toml::to_string_pretty(config) {
        Ok(text) => text,
        Err(error) => return Some(error.to_string()),
    };
    let rewrite = match fs::read_to_string(&path) {
        Ok(existing) => match merge_config::<I>(&existing, &text) {
            Ok(rewrite) => rewrite,
            Err(error) => return Some(format!("{}: {error}", path.display())),
        },
        Err(error) if error.kind() == ErrorKind::NotFound => ConfigRewrite::Changed(text),
        Err(error) => return Some(format!("{}: {error}", path.display())),
    };
    let ConfigRewrite::Changed(text) = rewrite else {
        return None;
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
    use tempfile::TempDir;

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

    impl AppIdentity for Sample {
        const BINARY_NAME: &'static str = "sample";
        const CONFIG_DIRNAME: &'static str = "sample";
        const DEFAULT_LIGHT_THEME: &'static str = "light";
        const DEFAULT_DARK_THEME: &'static str = "dark";
        const RETIRED_CONFIG_KEYS: &'static [&'static [&'static str]] = &[&["settings", "retired"]];
    }

    /// A test-owned config directory under the runner's temporary directory.
    struct Scratch(TempDir);

    impl Scratch {
        fn new(test: &str) -> Self {
            Self(
                tempfile::Builder::new()
                    .prefix(&format!("tui_pane-{test}-"))
                    .tempdir()
                    .expect("the scratch directory should be created"),
            )
        }

        fn config_path(&self) -> PathBuf { self.0.path().join(CONFIG_FILENAME) }
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

        let loaded = LoadedConfig::<Sample>::load_from::<Sample>(Some(path.clone()));

        assert_eq!(loaded.error, None);
        assert_eq!(loaded.config, Sample::default());
        assert_eq!(read(&path), complete);
    }

    #[test]
    fn loading_a_retired_key_removes_it_and_preserves_the_document() {
        let scratch = Scratch::new("retired_key");
        let path = scratch.config_path();
        let original = "# user's heading\nname='sample' # name note\ncount=3\nunknown='kept' # future note\n\n[settings] # section note\nretired = 7\nfuture = true # nested note\n";
        fs::write(&path, original).expect("fixture writes");
        let loaded = LoadedConfig::<Sample>::load_from::<Sample>(Some(path.clone()));
        assert_eq!(loaded.error, None);
        assert_eq!(loaded.config, Sample::default());
        assert_eq!(read(&path), original.replace("retired = 7\n", ""));
    }

    /// A file missing a key comes back carrying it, and keeps what it
    /// did say.
    #[test]
    fn a_file_missing_a_key_is_restated() {
        let scratch = Scratch::new("restate_missing_key");
        let path = scratch.config_path();
        fs::write(&path, "name = \"kept\"\n").expect("the config file should be written");

        let loaded = LoadedConfig::<Sample>::load_from::<Sample>(Some(path.clone()));

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

        let loaded = LoadedConfig::<Sample>::load_from::<Sample>(Some(path.clone()));

        assert_eq!(loaded.config, Sample::default());
        let error = loaded.error.expect("a parse error should be kept");
        assert!(error.contains("invalid type"), "{error}");
        assert_eq!(read(&path), broken);
    }

    /// Saving creates the config directory and clears an earlier error.
    #[test]
    fn save_creates_the_directory_and_clears_the_error() {
        let scratch = Scratch::new("save");
        let path = scratch.0.path().join("nested").join(CONFIG_FILENAME);
        let mut loaded = LoadedConfig {
            config: Sample::default(),
            error:  Some("stale".to_string()),
        };

        loaded.save_to::<Sample>(Some(path.clone()));

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

        loaded.save_to::<Sample>(None);

        assert_eq!(
            loaded.error.as_deref(),
            Some("no OS config directory: cannot write config.toml")
        );
    }

    #[derive(Default, Deserialize, Serialize)]
    #[serde(default)]
    struct Sectioned {
        name:     String,
        settings: Sample,
    }

    #[test]
    fn config_restate_and_save_preserve_unknown_keys_and_comments() {
        let scratch = Scratch::new("preserve_document");
        let path = scratch.config_path();
        let original = "# user's heading\nfuture = 'kept' # top-level note\nname='old' # name note\n\n[settings] # section note\nextra = 42 # nested note\nname = 'section'\n";
        fs::write(&path, original).expect("fixture writes");

        let mut loaded = LoadedConfig::<Sectioned>::load_from::<Sample>(Some(path.clone()));
        assert_eq!(loaded.error, None);
        let restated = read(&path);
        assert!(restated.starts_with(original), "{restated}");
        assert!(restated.contains("count = 3"), "{restated}");

        loaded.config.name = "new".into();
        loaded.config.settings.count = 9;
        loaded.save_to::<Sample>(Some(path.clone()));
        assert_eq!(loaded.error, None);
        let saved = read(&path);
        assert_eq!(
            saved,
            restated
                .replace("name='old'", "name=\"new\"")
                .replace("count = 3", "count = 9")
        );
        let parsed: Sectioned = toml::from_str(&saved).expect("saved config parses");
        assert_eq!(parsed.name, "new");
        assert_eq!(parsed.settings.count, 9);
    }

    #[test]
    fn complete_config_skips_restate_and_save_writes() {
        let scratch = Scratch::new("preserve_no_write");
        let path = scratch.config_path();
        let original =
            "# formatting is intentional\ncount=0x3 # three\nunknown=true\nname='sample'\n";
        fs::write(&path, original).expect("fixture writes");
        let timestamp = std::time::UNIX_EPOCH + std::time::Duration::from_secs(100);
        fs::File::options()
            .write(true)
            .open(&path)
            .expect("fixture opens")
            .set_modified(timestamp)
            .expect("fixture modification time sets");

        let mut loaded = LoadedConfig::<Sample>::load_from::<Sample>(Some(path.clone()));
        assert_eq!(loaded.error, None);
        assert_eq!(
            fs::metadata(&path)
                .expect("fixture exists")
                .modified()
                .expect("mtime reads"),
            timestamp
        );
        loaded.save_to::<Sample>(Some(path.clone()));
        assert_eq!(loaded.error, None);
        assert_eq!(read(&path), original);
        assert_eq!(
            fs::metadata(&path)
                .expect("fixture exists")
                .modified()
                .expect("mtime reads"),
            timestamp
        );
    }

    #[test]
    fn config_restate_adds_a_missing_section_without_reordering_existing_keys() {
        let scratch = Scratch::new("missing_section");
        let path = scratch.config_path();
        let original = "future = 7\n# retained name\nname='existing'\n";
        fs::write(&path, original).expect("fixture writes");

        let loaded = LoadedConfig::<Sectioned>::load_from::<Sample>(Some(path.clone()));
        assert_eq!(loaded.error, None);
        let saved = read(&path);
        assert!(saved.starts_with(original), "{saved}");
        let parsed: Sectioned = toml::from_str(&saved).expect("restated config parses");
        assert_eq!(parsed.settings, Sample::default());
        assert!(saved.contains("[settings]"), "{saved}");
    }

    #[test]
    fn config_save_preserves_inline_section_unknowns_and_spacing() {
        let scratch = Scratch::new("inline_section");
        let path = scratch.config_path();
        let original =
            "name = ''\nsettings = { name='sample', count=3, future='kept' } # inline note\n";
        fs::write(&path, original).expect("fixture writes");

        let mut loaded = LoadedConfig::<Sectioned>::load_from::<Sample>(Some(path.clone()));
        assert_eq!(loaded.error, None);
        assert_eq!(read(&path), original);
        loaded.config.settings.count = 4;
        loaded.save_to::<Sample>(Some(path.clone()));
        assert_eq!(loaded.error, None);
        assert_eq!(read(&path), original.replace("count=3", "count=4"));
    }

    #[test]
    fn config_save_leaves_a_malformed_existing_document_untouched() {
        let scratch = Scratch::new("save_malformed");
        let path = scratch.config_path();
        let malformed = "# repair me\n[unterminated\n";
        fs::write(&path, malformed).expect("fixture writes");
        let mut loaded = LoadedConfig {
            config: Sample::default(),
            error:  None,
        };

        loaded.save_to::<Sample>(Some(path.clone()));
        assert!(loaded.error.is_some());
        assert_eq!(read(&path), malformed);
    }
}
