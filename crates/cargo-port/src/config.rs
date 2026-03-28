use std::path::Path;
use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;

const APP_NAME: &str = "cargo-port";
const CONFIG_FILE: &str = "config.toml";

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
    pub include_non_rust: bool,

    /// GitHub owners whose projects you can edit (Version, Description).
    /// Add your username and/or org names here.
    #[serde(default)]
    pub owned_owners: Vec<String>,

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
            include_non_rust: false,
            owned_owners:     Vec::new(),
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
    pub invert_scroll: bool,
}

impl Default for MouseConfig {
    fn default() -> Self {
        Self {
            invert_scroll: true,
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

/// Default configuration TOML written on first run.
const DEFAULT_CONFIG_TOML: &str = r#"[mouse]
invert_scroll = true

[tui]
inline_dirs = ["crates"]
ci_run_count = 5

# Directories to skip when scanning. Edit this list for your setup.
exclude_dirs = [
    "Library",
    "Applications",
    "Downloads",
    "Documents",
    "Movies",
    "Music",
    "Pictures",
    "Public",
    "vendor",
]

# Include non-Rust projects (git repos without Cargo.toml).
include_non_rust = false

# GitHub owners whose projects you can edit (Version, Description).
# Add your username and/or org names here.
owned_owners = []

# Editor application name, opened via `open -a <editor> <path>`.
editor = "zed"
"#;

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
        assert!(!cfg.tui.include_non_rust);
        assert_eq!(cfg.tui.ci_run_count, 5);
        assert_eq!(cfg.tui.inline_dirs, vec!["crates".to_string()]);
        assert!(cfg.tui.owned_owners.is_empty());
        assert!(cfg.mouse.invert_scroll);
    }
}
