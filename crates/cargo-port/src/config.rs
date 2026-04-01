use std::path::Path;
use std::path::PathBuf;

use confique::Config as _;
use serde::Deserialize;
use serde::Serialize;

use super::constants::APP_NAME;
use super::constants::CONFIG_FILE;

/// Whether non-Rust projects (git repos without `Cargo.toml`) are included in scans.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "bool", into = "bool")]
pub enum NonRustInclusion {
    Include,
    #[default]
    Exclude,
}

impl From<bool> for NonRustInclusion {
    fn from(b: bool) -> Self { if b { Self::Include } else { Self::Exclude } }
}

impl From<NonRustInclusion> for bool {
    fn from(val: NonRustInclusion) -> Self { matches!(val, NonRustInclusion::Include) }
}

impl NonRustInclusion {
    pub const fn includes_non_rust(self) -> bool { matches!(self, Self::Include) }

    pub const fn toggle(&mut self) {
        *self = match *self {
            Self::Include => Self::Exclude,
            Self::Exclude => Self::Include,
        };
    }
}

/// Scroll direction for mouse wheel events.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "bool", into = "bool")]
pub enum ScrollDirection {
    #[default]
    Normal,
    Inverted,
}

impl From<bool> for ScrollDirection {
    fn from(b: bool) -> Self { if b { Self::Inverted } else { Self::Normal } }
}

impl From<ScrollDirection> for bool {
    fn from(val: ScrollDirection) -> Self { matches!(val, ScrollDirection::Inverted) }
}

impl ScrollDirection {
    pub const fn is_inverted(self) -> bool { matches!(self, Self::Inverted) }

    pub const fn toggle(&mut self) {
        *self = match *self {
            Self::Normal => Self::Inverted,
            Self::Inverted => Self::Normal,
        };
    }
}

/// Lint status indicator settings.
#[derive(confique::Config, Default, Serialize)]
pub struct LintConfig {
    /// Show a lint status indicator per project by reading a cache-rooted
    /// `port-report.log` protocol file. Any external tool can produce it.
    #[config(default = false)]
    pub enabled: bool,
}

/// Top-level application configuration.
#[derive(confique::Config, Default, Serialize)]
pub struct Config {
    #[config(nested)]
    pub mouse: MouseConfig,
    #[config(nested)]
    pub tui:   TuiConfig,
    #[config(nested)]
    pub lint:  LintConfig,
}

/// TUI display and behaviour settings.
#[derive(confique::Config, Serialize)]
pub struct TuiConfig {
    /// Directory names whose members are shown inline (pulled up to the
    /// workspace level). For example, `["crates"]` means projects under
    /// `workspace/crates/` appear directly under the workspace rather than
    /// in a "crates" folder.
    #[config(default = ["crates"])]
    pub inline_dirs: Vec<String>,

    /// Number of recent CI runs to fetch per project.
    #[config(default = 5)]
    pub ci_run_count: u32,

    /// Directories to scan for projects (relative to the scan root, or
    /// absolute paths). When empty, the entire scan root is walked.
    #[config(default = [])]
    pub include_dirs: Vec<String>,

    /// Whether to include non-Rust projects (git repos without Cargo.toml).
    #[config(default = false)]
    pub include_non_rust: NonRustInclusion,

    /// Editor application name, opened via `open -a <editor> <path>`.
    #[config(default = "zed")]
    pub editor: String,

    /// How long (in seconds) the status bar flash is shown (e.g. "no new
    /// runs found").
    #[config(default = 3.0)]
    pub status_flash_secs: f64,
}

impl Default for TuiConfig {
    fn default() -> Self {
        Self {
            inline_dirs: vec!["crates".to_string()],
            ci_run_count: 5,
            include_dirs: Vec::new(),
            include_non_rust: NonRustInclusion::Exclude,
            editor: "zed".to_string(),
            status_flash_secs: 3.0,
        }
    }
}

/// Mouse input settings.
#[derive(confique::Config, Serialize)]
pub struct MouseConfig {
    /// Whether to invert mouse scroll direction.
    #[config(default = true)]
    pub invert_scroll: ScrollDirection,
}

impl Default for MouseConfig {
    fn default() -> Self {
        Self {
            invert_scroll: ScrollDirection::Inverted,
        }
    }
}

pub fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join(APP_NAME).join(CONFIG_FILE))
}

pub fn load() -> Config {
    let Some(path) = config_path() else {
        return Config::default();
    };

    if !path.exists() {
        let _ = create_default_config(&path);
    }

    Config::builder().file(&path).load().unwrap_or_default()
}

fn create_default_config(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create config directory: {e}"))?;
    }

    let template = confique::toml::template::<Config>(confique::toml::FormatOptions::default());

    std::fs::write(path, template).map_err(|e| format!("Failed to write config: {e}"))?;
    Ok(())
}

pub fn save(config: &Config) -> Result<(), String> {
    let Some(path) = config_path() else {
        return Err("Could not determine config directory".to_string());
    };

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create config directory: {e}"))?;
    }

    let contents =
        toml::to_string_pretty(config).map_err(|e| format!("Failed to serialize config: {e}"))?;

    std::fs::write(&path, contents).map_err(|e| format!("Failed to write config: {e}"))?;

    Ok(())
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::*;

    /// `Config::default()` returns correct values for every field.
    #[test]
    fn defaults_are_correct() {
        let cfg = Config::default();
        assert_eq!(cfg.tui.inline_dirs, vec!["crates".to_string()]);
        assert_eq!(cfg.tui.ci_run_count, 5);
        assert!(cfg.tui.include_dirs.is_empty());
        assert_eq!(cfg.tui.include_non_rust, NonRustInclusion::Exclude);
        assert_eq!(cfg.tui.editor, "zed");
        assert!((cfg.tui.status_flash_secs - 3.0).abs() < f64::EPSILON);
        assert_eq!(cfg.mouse.invert_scroll, ScrollDirection::Inverted);
    }

    /// Generated template parses back into a valid `Config` via confique.
    #[test]
    fn template_round_trips() {
        let template = confique::toml::template::<Config>(confique::toml::FormatOptions::default());

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, &template).expect("write template");

        // Template has all fields commented out, so loading it should
        // succeed with defaults filling every field.
        let cfg = Config::builder()
            .file(&path)
            .load()
            .expect("template should parse");
        assert_eq!(cfg.tui.ci_run_count, 5);
    }

    /// A partial config file gets defaults for missing fields.
    #[test]
    fn partial_config_fills_defaults() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[tui]\nci_run_count = 10\n").expect("write");

        let cfg = Config::builder()
            .file(&path)
            .load()
            .expect("partial config should load");
        assert_eq!(cfg.tui.ci_run_count, 10);
        assert_eq!(cfg.tui.editor, "zed");
        assert_eq!(cfg.mouse.invert_scroll, ScrollDirection::Inverted);
    }

    /// An empty config file gets all defaults.
    #[test]
    fn empty_config_gets_defaults() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "").expect("write");

        let cfg = Config::builder()
            .file(&path)
            .load()
            .expect("empty config should load");
        assert_eq!(cfg.tui.ci_run_count, 5);
        assert_eq!(cfg.tui.editor, "zed");
    }

    /// Saving and reloading preserves all values.
    #[test]
    fn save_and_reload_round_trip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");

        let mut cfg = Config::default();
        cfg.tui.ci_run_count = 42;
        cfg.tui.editor = "vim".to_string();
        cfg.tui.status_flash_secs = 5.0;
        cfg.mouse.invert_scroll = ScrollDirection::Normal;

        let contents = toml::to_string_pretty(&cfg).expect("serialize");
        std::fs::write(&path, &contents).expect("write");

        let reloaded = Config::builder()
            .file(&path)
            .load()
            .expect("reloaded config");
        assert_eq!(reloaded.tui.ci_run_count, 42);
        assert_eq!(reloaded.tui.editor, "vim");
        assert!((reloaded.tui.status_flash_secs - 5.0).abs() < f64::EPSILON);
        assert_eq!(reloaded.mouse.invert_scroll, ScrollDirection::Normal);
    }

    /// Bool-based enums deserialize correctly from TOML booleans.
    #[test]
    fn bool_enums_from_toml() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "[mouse]\ninvert_scroll = false\n\n[tui]\ninclude_non_rust = true\n",
        )
        .expect("write");

        let cfg = Config::builder()
            .file(&path)
            .load()
            .expect("bool enums should parse");
        assert_eq!(cfg.mouse.invert_scroll, ScrollDirection::Normal);
        assert_eq!(cfg.tui.include_non_rust, NonRustInclusion::Include);
    }
}
