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
}

impl Default for TuiConfig {
    fn default() -> Self {
        Self {
            inline_dirs:  default_inline_dirs(),
            ci_run_count: default_ci_run_count(),
            exclude_dirs: default_exclude_dirs(),
        }
    }
}

fn default_inline_dirs() -> Vec<String> { vec!["crates".to_string()] }

const fn default_ci_run_count() -> u32 { 5 }

const fn default_exclude_dirs() -> Vec<String> { Vec::new() }

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

fn create_default_config(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create config directory: {e}"))?;
    }

    let contents = r#"[mouse]
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
"#;

    std::fs::write(path, contents).map_err(|e| format!("Failed to write config: {e}"))?;
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
