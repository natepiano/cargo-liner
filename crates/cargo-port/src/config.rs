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
pub(crate) enum NonRustInclusion {
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
    pub(crate) const fn includes_non_rust(self) -> bool { matches!(self, Self::Include) }

    pub(crate) const fn toggle(&mut self) {
        *self = match *self {
            Self::Include => Self::Exclude,
            Self::Exclude => Self::Include,
        };
    }
}

/// Scroll direction for mouse wheel events.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "bool", into = "bool")]
pub(crate) enum ScrollDirection {
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
    pub(crate) const fn is_inverted(self) -> bool { matches!(self, Self::Inverted) }

    pub(crate) const fn toggle(&mut self) {
        *self = match *self {
            Self::Normal => Self::Inverted,
            Self::Inverted => Self::Normal,
        };
    }
}

/// Whether newly discovered projects trigger an immediate lint run or wait
/// for a real file-system change event.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "bool", into = "bool")]
pub(crate) enum DiscoveryLint {
    /// Run lints immediately when a new project appears after the initial scan.
    Immediate,
    /// Wait for an actual disk event before running lints on new projects.
    #[default]
    Deferred,
}

impl From<bool> for DiscoveryLint {
    fn from(b: bool) -> Self { if b { Self::Immediate } else { Self::Deferred } }
}

impl From<DiscoveryLint> for bool {
    fn from(val: DiscoveryLint) -> Self { matches!(val, DiscoveryLint::Immediate) }
}

impl DiscoveryLint {
    pub(crate) const fn is_immediate(self) -> bool { matches!(self, Self::Immediate) }

    pub(crate) const fn toggle(&mut self) {
        *self = match *self {
            Self::Immediate => Self::Deferred,
            Self::Deferred => Self::Immediate,
        };
    }
}

/// Whether `hjkl` should mirror arrow-key navigation in non-text panes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "bool", into = "bool")]
pub(crate) enum NavigationKeys {
    #[default]
    ArrowsOnly,
    ArrowsAndVim,
}

impl From<bool> for NavigationKeys {
    fn from(enabled: bool) -> Self {
        if enabled {
            Self::ArrowsAndVim
        } else {
            Self::ArrowsOnly
        }
    }
}

impl From<NavigationKeys> for bool {
    fn from(value: NavigationKeys) -> Self { matches!(value, NavigationKeys::ArrowsAndVim) }
}

impl NavigationKeys {
    pub(crate) const fn uses_vim(self) -> bool { matches!(self, Self::ArrowsAndVim) }

    pub(crate) const fn toggle(&mut self) {
        *self = match *self {
            Self::ArrowsOnly => Self::ArrowsAndVim,
            Self::ArrowsAndVim => Self::ArrowsOnly,
        };
    }
}

/// Cache storage settings shared by CI and lint-history data.
#[derive(Clone, Debug, Default, PartialEq, Eq, confique::Config, Serialize)]
pub(crate) struct CacheConfig {
    /// Override the app cache root. Empty uses the system cache directory.
    #[config(default = "")]
    pub root: String,
}

/// Lint status indicator settings.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct LintCommandConfig {
    #[serde(default)]
    pub name:    String,
    #[serde(default)]
    pub command: String,
}

#[derive(Clone, Debug, PartialEq, Eq, confique::Config, Serialize)]
pub(crate) struct LintConfig {
    /// Show a lint status indicator per project by reading cache-rooted
    /// lint JSON artifacts.
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

    /// Maximum retained size for lint run artifacts. `0` and `unlimited`
    /// disable pruning.
    #[config(default = "512 MiB")]
    pub cache_size: String,

    /// Run lints immediately when a new project appears (`true`), or wait
    /// for an actual file change before linting (`false`). When `false`,
    /// startup and new-project discovery only set up file watchers — lints
    /// run only after you edit code.
    #[config(default = false)]
    pub on_discovery: DiscoveryLint,
}

impl Default for LintConfig {
    fn default() -> Self {
        Self {
            enabled:      false,
            include:      Vec::new(),
            exclude:      Vec::new(),
            commands:     Vec::new(),
            cache_size:   DEFAULT_CACHE_SIZE.to_string(),
            on_discovery: DiscoveryLint::Deferred,
        }
    }
}

impl LintConfig {
    pub(crate) fn resolved_commands(&self) -> Vec<LintCommandConfig> {
        let commands = normalize_lint_commands(&self.commands);
        if commands.is_empty() {
            return vec![default_clippy_lint_command()];
        }
        commands
    }

    pub(crate) fn cache_size_bytes(&self) -> Result<Option<u64>, String> {
        parse_cache_size(&self.cache_size).map(|parsed| parsed.bytes)
    }

    pub(crate) fn normalized_cache_size(&self) -> Result<String, String> {
        parse_cache_size(&self.cache_size).map(|parsed| parsed.normalized)
    }
}

pub(crate) fn default_clippy_lint_command() -> LintCommandConfig {
    LintCommandConfig {
        name:    "clippy".to_string(),
        command:
            "cargo clippy --workspace --all-targets --all-features --manifest-path \"$MANIFEST_PATH\" -- -D warnings"
                .to_string(),
    }
}

pub(crate) fn builtin_lint_command(name: &str) -> Option<LintCommandConfig> {
    match name.trim().to_ascii_lowercase().as_str() {
        "mend" => Some(LintCommandConfig {
            name:    "mend".to_string(),
            command: "cargo mend --manifest-path \"$MANIFEST_PATH\"".to_string(),
        }),
        "clippy" => Some(default_clippy_lint_command()),
        _ => None,
    }
}

pub(crate) fn infer_lint_command_name(command: &str) -> String {
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
        name:    if name.is_empty() {
            infer_lint_command_name(command_str)
        } else {
            name.to_string()
        },
        command: command_str.to_string(),
    })
}

pub(crate) fn normalize_lint_commands(commands: &[LintCommandConfig]) -> Vec<LintCommandConfig> {
    commands.iter().filter_map(normalize_lint_command).collect()
}

const BYTES_PER_KIB: u64 = 1024;
const BYTES_PER_MIB: u64 = BYTES_PER_KIB * 1024;
const BYTES_PER_GIB: u64 = BYTES_PER_MIB * 1024;
const DEFAULT_CACHE_SIZE: &str = "512 MiB";

pub(crate) struct ParsedCacheSize {
    pub bytes:      Option<u64>,
    pub normalized: String,
}

fn normalize_cache_size_number(number: &str) -> Result<String, String> {
    let (whole_raw, fraction_raw) = number
        .split_once('.')
        .map_or((number, None), |(whole, fraction)| (whole, Some(fraction)));

    if whole_raw.is_empty() && fraction_raw.is_none() {
        return Err(format!("Invalid cache size quantity `{number}`"));
    }
    if !whole_raw.is_empty() && !whole_raw.chars().all(|ch| ch.is_ascii_digit()) {
        return Err(format!("Invalid cache size quantity `{number}`"));
    }

    let whole = if whole_raw.is_empty() {
        "0"
    } else {
        whole_raw.trim_start_matches('0')
    };
    let whole = if whole.is_empty() { "0" } else { whole };

    let Some(fraction_raw) = fraction_raw else {
        return Ok(whole.to_string());
    };
    if !fraction_raw.chars().all(|ch| ch.is_ascii_digit()) {
        return Err(format!("Invalid cache size quantity `{number}`"));
    }

    let fraction = fraction_raw.trim_end_matches('0');
    if fraction.is_empty() {
        Ok(whole.to_string())
    } else {
        Ok(format!("{whole}.{fraction}"))
    }
}

fn parse_cache_size_bytes(number: &str, multiplier: u64) -> Result<Option<u64>, String> {
    let (whole_raw, fraction_raw) = number
        .split_once('.')
        .map_or((number, None), |(whole, fraction)| (whole, Some(fraction)));
    if whole_raw.is_empty() && fraction_raw.is_none() {
        return Err(format!("Invalid cache size quantity `{number}`"));
    }

    let whole = if whole_raw.is_empty() {
        0_u128
    } else {
        whole_raw
            .parse::<u128>()
            .map_err(|_| format!("Invalid cache size quantity `{number}`"))?
    };
    let multiplier = u128::from(multiplier);
    let whole_bytes = whole
        .checked_mul(multiplier)
        .ok_or_else(|| "Cache size is too large".to_string())?;

    let Some(fraction_raw) = fraction_raw else {
        return u64::try_from(whole_bytes)
            .map(Some)
            .map_err(|_| "Cache size is too large".to_string());
    };
    if !fraction_raw.chars().all(|ch| ch.is_ascii_digit()) {
        return Err(format!("Invalid cache size quantity `{number}`"));
    }
    if fraction_raw.is_empty() {
        return u64::try_from(whole_bytes)
            .map(Some)
            .map_err(|_| "Cache size is too large".to_string());
    }

    let fraction = fraction_raw
        .parse::<u128>()
        .map_err(|_| format!("Invalid cache size quantity `{number}`"))?;
    let scale = 10_u128
        .checked_pow(u32::try_from(fraction_raw.len()).unwrap_or(u32::MAX))
        .ok_or_else(|| "Cache size is too large".to_string())?;
    let fraction_bytes = fraction
        .checked_mul(multiplier)
        .ok_or_else(|| "Cache size is too large".to_string())?
        .div_ceil(scale);
    let total = whole_bytes
        .checked_add(fraction_bytes)
        .ok_or_else(|| "Cache size is too large".to_string())?;

    if total == 0 {
        Ok(None)
    } else {
        u64::try_from(total)
            .map(Some)
            .map_err(|_| "Cache size is too large".to_string())
    }
}

fn canonical_cache_size_unit(unit: &str) -> Option<(&'static str, u64)> {
    match unit.trim().to_ascii_lowercase().as_str() {
        "b" | "byte" | "bytes" => Some(("B", 1)),
        "kib" | "kb" => Some(("KiB", BYTES_PER_KIB)),
        "mib" | "mb" => Some(("MiB", BYTES_PER_MIB)),
        "gib" | "gb" => Some(("GiB", BYTES_PER_GIB)),
        _ => None,
    }
}

pub(crate) fn parse_cache_size(value: &str) -> Result<ParsedCacheSize, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("Cache size cannot be empty".to_string());
    }
    if trimmed.eq_ignore_ascii_case("unlimited") {
        return Ok(ParsedCacheSize {
            bytes:      None,
            normalized: "unlimited".to_string(),
        });
    }
    if trimmed == "0" {
        return Ok(ParsedCacheSize {
            bytes:      None,
            normalized: "0".to_string(),
        });
    }

    let split_at = trimmed
        .find(|ch: char| !(ch.is_ascii_digit() || ch == '.'))
        .unwrap_or(trimmed.len());
    let number = trimmed[..split_at].trim();
    let unit = trimmed[split_at..].trim();

    if number.is_empty() || unit.is_empty() {
        return Err(
            "Cache size must include a number and unit like `512 MiB` or `1.5 GiB`".to_string(),
        );
    }

    let Some((canonical_unit, multiplier)) = canonical_cache_size_unit(unit) else {
        return Err(format!("Unsupported cache size unit `{unit}`"));
    };
    let normalized = format!(
        "{} {}",
        normalize_cache_size_number(number)?,
        canonical_unit
    );
    Ok(ParsedCacheSize {
        bytes: parse_cache_size_bytes(number, multiplier)?,
        normalized,
    })
}

pub(crate) fn normalize_config(mut config: CargoPortConfig) -> Result<CargoPortConfig, String> {
    config.lint.commands = normalize_lint_commands(&config.lint.commands);
    config.lint.cache_size = config.lint.normalized_cache_size()?;
    Ok(config)
}

/// Top-level application configuration.
#[derive(Clone, Debug, Default, PartialEq, confique::Config, Serialize)]
pub(crate) struct CargoPortConfig {
    #[config(nested)]
    pub cache: CacheConfig,
    #[config(nested)]
    pub mouse: MouseConfig,
    #[config(nested)]
    pub tui:   TuiConfig,
    #[config(nested)]
    pub lint:  LintConfig,
}

/// TUI display and behaviour settings.
#[derive(Clone, Debug, PartialEq, confique::Config, Serialize)]
pub(crate) struct TuiConfig {
    /// Directory names whose members are shown inline (pulled up to the
    /// workspace level). For example, `["crates"]` means projects under
    /// `workspace/crates/` appear directly under the workspace rather than
    /// in a "crates" folder.
    #[config(default = ["crates"])]
    pub inline_dirs: Vec<String>,

    /// Number of recent CI runs to fetch per project.
    #[config(default = 5)]
    pub ci_run_count: u32,

    /// Whether `hjkl` mirrors arrow navigation in non-text panes.
    #[config(default = false)]
    pub navigation_keys: NavigationKeys,

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
    #[config(default = 5.0)]
    pub status_flash_secs: f64,

    /// How long (in seconds) a completed task toast lingers before
    /// starting its exit animation.
    #[config(default = 1.0)]
    pub task_linger_secs: f64,
}

impl Default for TuiConfig {
    fn default() -> Self {
        Self {
            inline_dirs:       vec!["crates".to_string()],
            ci_run_count:      5,
            navigation_keys:   NavigationKeys::ArrowsOnly,
            include_dirs:      Vec::new(),
            include_non_rust:  NonRustInclusion::Exclude,
            editor:            "zed".to_string(),
            status_flash_secs: 5.0,
            task_linger_secs:  1.0,
        }
    }
}

/// Mouse input settings.
#[derive(Clone, Debug, PartialEq, Eq, confique::Config, Serialize)]
pub(crate) struct MouseConfig {
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

pub(crate) fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join(APP_NAME).join(CONFIG_FILE))
}

fn active_config_cell() -> &'static RwLock<CargoPortConfig> {
    static ACTIVE_CONFIG: OnceLock<RwLock<CargoPortConfig>> = OnceLock::new();
    ACTIVE_CONFIG.get_or_init(|| RwLock::new(CargoPortConfig::default()))
}

pub(crate) fn active_config() -> CargoPortConfig {
    active_config_cell()
        .read()
        .map_or_else(|_| CargoPortConfig::default(), |cfg| cfg.clone())
}

pub(crate) fn set_active_config(config: &CargoPortConfig) {
    if let Ok(mut active) = active_config_cell().write() {
        *active = normalize_config(config.clone()).unwrap_or_else(|_| config.clone());
    }
}

fn load_from_path(path: &Path) -> Result<CargoPortConfig, String> {
    if !path.exists() {
        create_default_config(path)?;
    }

    CargoPortConfig::builder()
        .file(path)
        .load()
        .map_err(|err| format!("Failed to load config '{}': {err}", path.display()))
        .and_then(normalize_config)
        .map_err(|err| format!("Failed to load config '{}': {err}", path.display()))
}

pub(crate) fn try_load_from_path(path: &Path) -> Result<CargoPortConfig, String> {
    load_from_path(path)
}

pub(crate) fn try_load() -> Result<CargoPortConfig, String> {
    let Some(path) = config_path() else {
        return Ok(CargoPortConfig::default());
    };
    try_load_from_path(&path)
}

fn create_default_config(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create config directory: {e}"))?;
    }

    let template =
        confique::toml::template::<CargoPortConfig>(confique::toml::FormatOptions::default());

    std::fs::write(path, template).map_err(|e| format!("Failed to write config: {e}"))?;
    Ok(())
}

pub(crate) fn save(config: &CargoPortConfig) -> Result<(), String> {
    let Some(path) = config_path() else {
        return Err("Could not determine config directory".to_string());
    };

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create config directory: {e}"))?;
    }

    let config = normalize_config(config.clone())?;
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

    fn assert_default_config_subset(cfg: &CargoPortConfig, expected_ci_run_count: u32) {
        assert!(cfg.cache.root.is_empty());
        assert_eq!(cfg.tui.inline_dirs, vec!["crates".to_string()]);
        assert_eq!(cfg.tui.ci_run_count, expected_ci_run_count);
        assert!(cfg.tui.include_dirs.is_empty());
        assert_eq!(cfg.tui.include_non_rust, NonRustInclusion::Exclude);
        assert_eq!(cfg.tui.editor, "zed");
        assert!((cfg.tui.status_flash_secs - 5.0).abs() < f64::EPSILON);
        assert_eq!(cfg.tui.navigation_keys, NavigationKeys::ArrowsOnly);
        assert_eq!(cfg.mouse.invert_scroll, ScrollDirection::Inverted);
        assert!(!cfg.lint.enabled);
        assert!(cfg.lint.include.is_empty());
        assert!(cfg.lint.exclude.is_empty());
        assert!(cfg.lint.commands.is_empty());
        assert_eq!(cfg.lint.cache_size, "512 MiB");
    }

    /// `Config::default()` returns correct values for every field.
    #[test]
    fn defaults_are_correct() {
        let cfg = CargoPortConfig::default();
        assert_default_config_subset(&cfg, 5);
    }

    /// Generated template parses back into a valid `CargoPortConfig` via confique.
    #[test]
    fn template_round_trips() {
        let template =
            confique::toml::template::<CargoPortConfig>(confique::toml::FormatOptions::default());

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, &template).expect("write template");

        // Template has all fields commented out, so loading it should
        // succeed with defaults filling every field.
        let cfg = CargoPortConfig::builder()
            .file(&path)
            .load()
            .expect("template should parse");
        assert_default_config_subset(&cfg, 5);
    }

    /// A partial config file gets defaults for missing fields.
    #[test]
    fn partial_config_fills_defaults() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[tui]\nci_run_count = 10\n").expect("write");

        let cfg = CargoPortConfig::builder()
            .file(&path)
            .load()
            .expect("partial config should load");
        assert_default_config_subset(&cfg, 10);
    }

    /// An empty config file gets all defaults.
    #[test]
    fn empty_config_gets_defaults() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "").expect("write");

        let cfg = CargoPortConfig::builder()
            .file(&path)
            .load()
            .expect("empty config should load");
        assert_default_config_subset(&cfg, 5);
    }

    /// Saving and reloading preserves all values.
    #[test]
    fn save_and_reload_round_trip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");

        let mut cfg = CargoPortConfig::default();
        cfg.cache.root = "/tmp/cargo-port-cache".to_string();
        cfg.tui.ci_run_count = 42;
        cfg.tui.editor = "vim".to_string();
        cfg.tui.navigation_keys = NavigationKeys::ArrowsAndVim;
        cfg.tui.status_flash_secs = 5.0;
        cfg.mouse.invert_scroll = ScrollDirection::Normal;

        let contents = toml::to_string_pretty(&cfg).expect("serialize");
        std::fs::write(&path, &contents).expect("write");

        let reloaded = CargoPortConfig::builder()
            .file(&path)
            .load()
            .expect("reloaded config");
        assert_eq!(reloaded.cache.root, "/tmp/cargo-port-cache");
        assert_eq!(reloaded.tui.ci_run_count, 42);
        assert_eq!(reloaded.tui.editor, "vim");
        assert_eq!(reloaded.tui.navigation_keys, NavigationKeys::ArrowsAndVim);
        assert!((reloaded.tui.status_flash_secs - 5.0).abs() < f64::EPSILON);
        assert_eq!(reloaded.mouse.invert_scroll, ScrollDirection::Normal);
        assert!(reloaded.tui.include_dirs.is_empty());
        assert_eq!(reloaded.tui.include_non_rust, NonRustInclusion::Exclude);
        assert!(reloaded.lint.commands.is_empty());
        assert!(reloaded.lint.include.is_empty());
        assert!(reloaded.lint.exclude.is_empty());
        assert!(!reloaded.lint.enabled);
        assert_eq!(reloaded.lint.cache_size, "512 MiB");
    }

    /// Bool-based enums deserialize correctly from TOML booleans.
    #[test]
    fn bool_enums_from_toml() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "[mouse]\ninvert_scroll = false\n\n[tui]\ninclude_non_rust = true\nnavigation_keys = true\n",
        )
        .expect("write");

        let cfg = CargoPortConfig::builder()
            .file(&path)
            .load()
            .expect("bool enums should parse");
        assert!(cfg.cache.root.is_empty());
        assert_eq!(cfg.mouse.invert_scroll, ScrollDirection::Normal);
        assert_eq!(cfg.tui.include_non_rust, NonRustInclusion::Include);
        assert_eq!(cfg.tui.navigation_keys, NavigationKeys::ArrowsAndVim);
    }

    /// Cache root override parses from TOML.
    #[test]
    fn cache_root_override_parses() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[cache]\nroot = \"/tmp/cargo-port\"\n").expect("write");

        let cfg = CargoPortConfig::builder()
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

        let cfg = CargoPortConfig::builder()
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
        let cfg = normalize_config(CargoPortConfig {
            lint: LintConfig {
                commands: vec![LintCommandConfig {
                    name:    "clippy".to_string(),
                    command: String::new(),
                }],
                ..LintConfig::default()
            },
            ..CargoPortConfig::default()
        })
        .expect("normalize config");

        assert_eq!(cfg.lint.commands.len(), 1);
        assert_eq!(cfg.lint.commands[0].name, "clippy");
        assert!(cfg.lint.commands[0].command.contains("cargo clippy"));
    }

    #[test]
    fn normalize_config_names_raw_commands() {
        let cfg = normalize_config(CargoPortConfig {
            lint: LintConfig {
                commands: vec![LintCommandConfig {
                    name:    String::new(),
                    command: "cargo fmt --check".to_string(),
                }],
                ..LintConfig::default()
            },
            ..CargoPortConfig::default()
        })
        .expect("normalize config");

        assert_eq!(cfg.lint.commands.len(), 1);
        assert_eq!(cfg.lint.commands[0].name, "fmt");
        assert_eq!(cfg.lint.commands[0].command, "cargo fmt --check");
    }

    /// Empty lint command config falls back to the built-in clippy command.
    #[test]
    fn resolved_lint_commands_default_to_builtins() {
        let cfg = CargoPortConfig::default();
        let commands = cfg.lint.resolved_commands();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].name, "clippy");
        assert!(commands[0].command.contains("cargo clippy"));
    }

    #[test]
    fn parse_cache_size_accepts_decimal_binary_units() {
        let parsed = parse_cache_size("1.5 GiB").expect("parse cache size");
        assert_eq!(parsed.normalized, "1.5 GiB");
        assert_eq!(parsed.bytes, Some(1_610_612_736));
    }

    #[test]
    fn parse_cache_size_accepts_unlimited_aliases() {
        assert_eq!(
            parse_cache_size("unlimited").expect("unlimited").bytes,
            None
        );
        assert_eq!(parse_cache_size("0").expect("zero").bytes, None);
    }

    #[test]
    fn normalize_config_normalizes_cache_size_units() {
        let cfg = normalize_config(CargoPortConfig {
            lint: LintConfig {
                cache_size: "1.50 gib".to_string(),
                ..LintConfig::default()
            },
            ..CargoPortConfig::default()
        })
        .expect("normalize config");

        assert_eq!(cfg.lint.cache_size, "1.5 GiB");
    }
}
