use std::path::Path;
use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;

use super::constants::APP_NAME;
use super::constants::CONFIG_FILE;
use super::constants::DEFAULT_CONFIG_TOML;

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

#[derive(Deserialize, Serialize, Default)]
pub struct Config {
    #[serde(default)]
    pub mouse: MouseConfig,
    #[serde(default)]
    pub tui:   TuiConfig,
}

#[derive(Deserialize, Serialize)]
pub struct TuiConfig {
    /// Directory names whose members are shown inline (pulled up to the workspace level).
    /// For example, `["crates"]` means projects under `workspace/crates/` appear
    /// directly under the workspace rather than in a "crates" folder.
    #[serde(default = "default_inline_dirs")]
    pub inline_dirs: Vec<String>,

    /// Number of recent CI runs to fetch per project.
    #[serde(default = "default_ci_run_count")]
    pub ci_run_count: u32,

    /// Directory names to skip during scanning.
    #[serde(default = "default_exclude_dirs")]
    pub exclude_dirs: Vec<String>,

    /// Whether to include non-Rust projects (git repos without `Cargo.toml`).
    #[serde(default)]
    pub include_non_rust: NonRustInclusion,

    /// Editor application name, opened via `open -a <editor> <path>`.
    #[serde(default = "default_editor")]
    pub editor: String,
}

impl Default for TuiConfig {
    fn default() -> Self {
        Self {
            inline_dirs:      default_inline_dirs(),
            ci_run_count:     default_ci_run_count(),
            exclude_dirs:     default_exclude_dirs(),
            include_non_rust: NonRustInclusion::Exclude,
            editor:           default_editor(),
        }
    }
}

fn default_inline_dirs() -> Vec<String> { vec!["crates".to_string()] }

const fn default_ci_run_count() -> u32 { 5 }

const fn default_exclude_dirs() -> Vec<String> { Vec::new() }

fn default_editor() -> String { "zed".to_string() }

#[derive(Deserialize, Serialize)]
pub struct MouseConfig {
    #[serde(default)]
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
        // Create default config on first run with recommended settings
        let _ = create_default_config(&path);
        // Now read it back so the toml is authoritative
        let Ok(contents) = std::fs::read_to_string(&path) else {
            return Config::default();
        };
        return toml::from_str(&contents).unwrap_or_default();
    }

    let Ok(contents) = std::fs::read_to_string(&path) else {
        return Config::default();
    };

    toml::from_str(&contents).unwrap_or_default()
}

fn create_default_config(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create config directory: {e}"))?;
    }

    std::fs::write(path, DEFAULT_CONFIG_TOML)
        .map_err(|e| format!("Failed to write config: {e}"))?;
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
mod tests {
    use super::*;

    #[test]
    fn default_config_toml_parses_correctly() {
        let result: Result<Config, _> = toml::from_str(DEFAULT_CONFIG_TOML);
        assert!(result.is_ok(), "DEFAULT_CONFIG_TOML should parse");
        let cfg = result.unwrap_or_default();
        assert_eq!(cfg.tui.include_non_rust, NonRustInclusion::Exclude);
        assert_eq!(cfg.tui.ci_run_count, 5);
        assert_eq!(cfg.tui.inline_dirs, vec!["crates".to_string()]);
        assert_eq!(cfg.mouse.invert_scroll, ScrollDirection::Inverted);
    }
}
