use std::path::Path;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::RwLock;

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
    fn from(b: bool) -> Self {
        if b { Self::Include } else { Self::Exclude }
    }
}

impl From<NonRustInclusion> for bool {
    fn from(val: NonRustInclusion) -> Self {
        matches!(val, NonRustInclusion::Include)
    }
}

impl NonRustInclusion {
    pub const fn includes_non_rust(self) -> bool {
        matches!(self, Self::Include)
    }

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
    fn from(b: bool) -> Self {
        if b { Self::Inverted } else { Self::Normal }
    }
}

impl From<ScrollDirection> for bool {
    fn from(val: ScrollDirection) -> Self {
        matches!(val, ScrollDirection::Inverted)
    }
}

impl ScrollDirection {
    pub const fn is_inverted(self) -> bool {
        matches!(self, Self::Inverted)
    }

    pub const fn toggle(&mut self) {
        *self = match *self {
            Self::Normal => Self::Inverted,
            Self::Inverted => Self::Normal,
        };
    }
}

/// Cache storage settings shared by CI and port-report data.
#[derive(Clone, Debug, Default, PartialEq, Eq, confique::Config, Serialize)]
pub struct CacheConfig {
    /// Override the app cache root. Empty uses the system cache directory.
    #[config(default = "")]
    pub root: String,
}

/// Lint status indicator settings.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LintCommandConfig {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub command: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, confique::Config, Serialize)]
pub struct LintConfig {
    /// Show a lint status indicator per project by reading cache-rooted
    /// Port Report JSON artifacts.
    #[config(default = false)]
    pub enabled: bool,

    /// Allow-list lint execution to projects whose display or absolute path
    /// starts with one of these prefixes. Empty means no projects are eligible.
    #[config(default = [])]
    pub include: Vec<String>,

    /// Skip lint execution for projects whose display or absolute path starts
    /// with one of these prefixes.
    #[config(default = [])]
    pub exclude: Vec<String>,

    /// Commands to run when a watched project changes. Empty falls back to the
    /// built-in clippy command.
    #[config(default = [])]
    pub commands: Vec<LintCommandConfig>,
}

impl LintConfig {
    pub fn resolved_commands(&self) -> Vec<LintCommandConfig> {
        let commands = normalize_lint_commands(&self.commands);
        if commands.is_empty() {
            return vec![default_clippy_lint_command()];
        }
        commands
    }
}

pub fn default_clippy_lint_command() -> LintCommandConfig {
    LintCommandConfig {
        name:    "clippy".to_string(),
        command:
            "cargo clippy --workspace --all-targets --all-features --manifest-path \"$MANIFEST_PATH\" -- -D warnings"
                .to_string(),
    }
}

pub fn builtin_lint_command(name: &str) -> Option<LintCommandConfig> {
    match name.trim().to_ascii_lowercase().as_str() {
        "mend" => Some(LintCommandConfig {
            name: "mend".to_string(),
            command: "cargo mend --manifest-path \"$MANIFEST_PATH\"".to_string(),
        }),
        "clippy" => Some(default_clippy_lint_command()),
        _ => None,
    }
}

pub fn infer_lint_command_name(command: &str) -> String {
    let mut parts = command.split_whitespace();
    let Some(first) = parts.next() else {
        return String::new();
    };

    if first == "cargo" {
        if let Some(second) = parts.next()
            && !second.starts_with('-')
        {
            return second.to_string();
        }
        return "cargo".to_string();
    }

    Path::new(first)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(first)
        .to_string()
}

fn normalize_lint_command(command: &LintCommandConfig) -> Option<LintCommandConfig> {
    let name = command.name.trim();
    let command_str = command.command.trim();

    if command_str.is_empty() {
        return builtin_lint_command(name);
    }

    Some(LintCommandConfig {
        name: if name.is_empty() {
            infer_lint_command_name(command_str)
        } else {
            name.to_string()
        },
        command: command_str.to_string(),
    })
}

pub fn normalize_lint_commands(commands: &[LintCommandConfig]) -> Vec<LintCommandConfig> {
    commands.iter().filter_map(normalize_lint_command).collect()
}

pub fn normalize_config(mut config: Config) -> Config {
    config.lint.commands = normalize_lint_commands(&config.lint.commands);
    config
}

/// Top-level application configuration.
#[derive(Clone, Debug, Default, PartialEq, confique::Config, Serialize)]
pub struct Config {
    #[config(nested)]
    pub cache: CacheConfig,
    #[config(nested)]
    pub mouse: MouseConfig,
    #[config(nested)]
    pub tui: TuiConfig,
    #[config(nested)]
    pub lint: LintConfig,
}

/// TUI display and behaviour settings.
#[derive(Clone, Debug, PartialEq, confique::Config, Serialize)]
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
#[derive(Clone, Debug, PartialEq, Eq, confique::Config, Serialize)]
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

fn active_config_cell() -> &'static RwLock<Config> {
    static ACTIVE_CONFIG: OnceLock<RwLock<Config>> = OnceLock::new();
    ACTIVE_CONFIG.get_or_init(|| RwLock::new(Config::default()))
}

pub fn active_config() -> Config {
    active_config_cell()
        .read()
        .map_or_else(|_| Config::default(), |cfg| cfg.clone())
}

pub fn set_active_config(config: &Config) {
    if let Ok(mut active) = active_config_cell().write() {
        *active = normalize_config(config.clone());
    }
}

fn load_from_path(path: &Path) -> Result<Config, String> {
    if !path.exists() {
        create_default_config(path)?;
    }

    Config::builder()
        .file(path)
        .load()
        .map(normalize_config)
        .map_err(|err| format!("Failed to load config '{}': {err}", path.display()))
}

pub fn try_load_from_path(path: &Path) -> Result<Config, String> {
    load_from_path(path)
}

pub fn try_load() -> Result<Config, String> {
    let Some(path) = config_path() else {
        return Ok(Config::default());
    };
    try_load_from_path(&path)
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

    let config = normalize_config(config.clone());
    let contents =
        toml::to_string_pretty(&config).map_err(|e| format!("Failed to serialize config: {e}"))?;

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
        assert!(cfg.cache.root.is_empty());
        assert_eq!(cfg.tui.inline_dirs, vec!["crates".to_string()]);
        assert_eq!(cfg.tui.ci_run_count, 5);
        assert!(cfg.tui.include_dirs.is_empty());
        assert_eq!(cfg.tui.include_non_rust, NonRustInclusion::Exclude);
        assert_eq!(cfg.tui.editor, "zed");
        assert!((cfg.tui.status_flash_secs - 3.0).abs() < f64::EPSILON);
        assert_eq!(cfg.mouse.invert_scroll, ScrollDirection::Inverted);
        assert!(!cfg.lint.enabled);
        assert!(cfg.lint.include.is_empty());
        assert!(cfg.lint.exclude.is_empty());
        assert!(cfg.lint.commands.is_empty());
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
        assert!(cfg.cache.root.is_empty());
        assert_eq!(cfg.tui.ci_run_count, 5);
        assert!(cfg.lint.commands.is_empty());
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
        assert!(cfg.cache.root.is_empty());
        assert_eq!(cfg.tui.ci_run_count, 10);
        assert_eq!(cfg.tui.editor, "zed");
        assert_eq!(cfg.mouse.invert_scroll, ScrollDirection::Inverted);
        assert!(cfg.lint.commands.is_empty());
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
        assert!(cfg.cache.root.is_empty());
        assert_eq!(cfg.tui.ci_run_count, 5);
        assert_eq!(cfg.tui.editor, "zed");
        assert!(cfg.lint.commands.is_empty());
    }

    /// Saving and reloading preserves all values.
    #[test]
    fn save_and_reload_round_trip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");

        let mut cfg = Config::default();
        cfg.cache.root = "/tmp/cargo-port-cache".to_string();
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
        assert_eq!(reloaded.cache.root, "/tmp/cargo-port-cache");
        assert_eq!(reloaded.tui.ci_run_count, 42);
        assert_eq!(reloaded.tui.editor, "vim");
        assert!((reloaded.tui.status_flash_secs - 5.0).abs() < f64::EPSILON);
        assert_eq!(reloaded.mouse.invert_scroll, ScrollDirection::Normal);
        assert!(reloaded.lint.commands.is_empty());
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
        assert!(cfg.cache.root.is_empty());
        assert_eq!(cfg.mouse.invert_scroll, ScrollDirection::Normal);
        assert_eq!(cfg.tui.include_non_rust, NonRustInclusion::Include);
    }

    /// Cache root override parses from TOML.
    #[test]
    fn cache_root_override_parses() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[cache]\nroot = \"/tmp/cargo-port\"\n").expect("write");

        let cfg = Config::builder()
            .file(&path)
            .load()
            .expect("cache root should parse");
        assert_eq!(cfg.cache.root, "/tmp/cargo-port");
        assert!(!cfg.lint.enabled);
    }

    /// Lint command arrays parse from TOML and preserve ordering.
    #[test]
    fn lint_commands_parse() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "[lint]\n\
             enabled = true\n\
             include = [\"~/rust/cargo-port_report\"]\n\
             exclude = [\"~/rust/archive\"]\n\
             [[lint.commands]]\n\
             name = \"fmt\"\n\
             command = \"cargo fmt --check\"\n\
             [[lint.commands]]\n\
             name = \"clippy\"\n\
             command = \"cargo clippy -- -D warnings\"\n",
        )
        .expect("write");

        let cfg = Config::builder()
            .file(&path)
            .load()
            .expect("lint commands should parse");
        assert!(cfg.lint.enabled);
        assert_eq!(cfg.lint.include, vec!["~/rust/cargo-port_report"]);
        assert_eq!(cfg.lint.exclude, vec!["~/rust/archive"]);
        assert_eq!(cfg.lint.commands.len(), 2);
        assert_eq!(cfg.lint.commands[0].name, "fmt");
        assert_eq!(cfg.lint.commands[0].command, "cargo fmt --check");
        assert_eq!(cfg.lint.commands[1].name, "clippy");
    }

    #[test]
    fn normalize_config_resolves_builtin_name_only_commands() {
        let cfg = normalize_config(Config {
            lint: LintConfig {
                commands: vec![LintCommandConfig {
                    name: "clippy".to_string(),
                    command: String::new(),
                }],
                ..LintConfig::default()
            },
            ..Config::default()
        });

        assert_eq!(cfg.lint.commands.len(), 1);
        assert_eq!(cfg.lint.commands[0].name, "clippy");
        assert!(cfg.lint.commands[0].command.contains("cargo clippy"));
    }

    #[test]
    fn normalize_config_names_raw_commands() {
        let cfg = normalize_config(Config {
            lint: LintConfig {
                commands: vec![LintCommandConfig {
                    name: String::new(),
                    command: "cargo fmt --check".to_string(),
                }],
                ..LintConfig::default()
            },
            ..Config::default()
        });

        assert_eq!(cfg.lint.commands.len(), 1);
        assert_eq!(cfg.lint.commands[0].name, "fmt");
        assert_eq!(cfg.lint.commands[0].command, "cargo fmt --check");
    }

    /// Empty lint command config falls back to the built-in clippy command.
    #[test]
    fn resolved_lint_commands_default_to_builtins() {
        let cfg = Config::default();
        let commands = cfg.lint.resolved_commands();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].name, "clippy");
        assert!(commands[0].command.contains("cargo clippy"));
    }
}
