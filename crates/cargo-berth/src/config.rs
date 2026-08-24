//! Repository-local configuration for `cargo-berth`.

use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::fs;
use std::fs::OpenOptions;
use std::io::ErrorKind;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

const CLAUDE_DIRECTORY: &str = ".claude";
const CONFIGURATION_DIRECTORY: &str = "config";
const CONFIGURATION_FILE: &str = "berth.toml";
const DEFAULT_MAXIMUM_ORDERING_EDGES: u32 = 512;
const DEFAULT_MAXIMUM_RESERVATIONS: u32 = 128;
const DEFAULT_TRUNK: &str = "main";
const GATE_MODE_KEY: &str = "gate_mode";
const MAXIMUM_ORDERING_EDGES_KEY: &str = "maximum_ordering_edges";
const MAXIMUM_RESERVATIONS_KEY: &str = "maximum_reservations";
const TRUNK_KEY: &str = "trunk";

/// Per-repository policy read by future reservation and gate verbs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BerthConfig {
    /// The local branch considered trunk.
    pub(crate) trunk:                  String,
    /// The maximum number of live reservations the graph may contain.
    pub(crate) maximum_reservations:   u32,
    /// The maximum number of declared ordering edges the graph may contain.
    pub(crate) maximum_ordering_edges: u32,
    /// Whether the trunk gate reports or rejects an invalid integration.
    pub(crate) gate_mode:              GateMode,
}

/// The repository's selected trunk-gate policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GateMode {
    /// Evaluate the gate and report its decision without rejecting the update.
    Observe,
    /// Reject updates that violate the gate's ordering decision.
    Enforce,
}

/// Whether initialization created a resource or retained an existing one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InitializationState {
    /// The initialization call created the resource.
    Created,
    /// The initialization call left the existing resource unchanged.
    Existing,
}

impl Default for BerthConfig {
    fn default() -> Self {
        Self {
            trunk:                  DEFAULT_TRUNK.to_owned(),
            maximum_reservations:   DEFAULT_MAXIMUM_RESERVATIONS,
            maximum_ordering_edges: DEFAULT_MAXIMUM_ORDERING_EDGES,
            gate_mode:              GateMode::Observe,
        }
    }
}

impl BerthConfig {
    /// Return this repository's configuration location.
    fn path(repository_root: &Path) -> PathBuf {
        repository_root
            .join(CLAUDE_DIRECTORY)
            .join(CONFIGURATION_DIRECTORY)
            .join(CONFIGURATION_FILE)
    }

    /// Create the default configuration without replacing an existing file.
    pub(crate) fn initialize(repository_root: &Path) -> Result<InitializationState, ConfigError> {
        let configuration_path = Self::path(repository_root);
        let configuration_parent = configuration_path
            .parent()
            .ok_or_else(|| ConfigError::InvalidPath(configuration_path.clone()))?;
        fs::create_dir_all(configuration_parent)?;

        let mut configuration_file = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&configuration_path)
        {
            Ok(configuration_file) => configuration_file,
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                return Ok(InitializationState::Existing);
            },
            Err(error) => return Err(ConfigError::Io(error)),
        };
        configuration_file.write_all(Self::default().to_toml().as_bytes())?;
        configuration_file.sync_all()?;
        Ok(InitializationState::Created)
    }

    /// Read and validate this repository's configuration.
    pub(crate) fn read(repository_root: &Path) -> Result<Self, ConfigError> {
        let contents = fs::read_to_string(Self::path(repository_root))?;
        Self::from_toml(&contents)
    }

    fn to_toml(&self) -> String {
        let gate_mode = match self.gate_mode {
            GateMode::Observe => "observe",
            GateMode::Enforce => "enforce",
        };
        format!(
            "{TRUNK_KEY} = \"{}\"\n{MAXIMUM_RESERVATIONS_KEY} = {}\n{MAXIMUM_ORDERING_EDGES_KEY} = {}\n{GATE_MODE_KEY} = \"{gate_mode}\"\n",
            self.trunk, self.maximum_reservations, self.maximum_ordering_edges
        )
    }

    fn from_toml(contents: &str) -> Result<Self, ConfigError> {
        let mut parsed_values = ParsedConfigValues::default();
        for line in contents.lines() {
            let stripped_line = strip_comment(line)?.trim();
            if stripped_line.is_empty() {
                continue;
            }
            let (key, value) = stripped_line
                .split_once('=')
                .ok_or_else(|| ConfigError::InvalidSyntax(stripped_line.to_owned()))?;
            parsed_values.set(key.trim(), value.trim())?;
        }
        Ok(parsed_values.finish())
    }
}

#[derive(Default)]
struct ParsedConfigValues {
    trunk:                  ConfigValue<String>,
    maximum_reservations:   ConfigValue<u32>,
    maximum_ordering_edges: ConfigValue<u32>,
    gate_mode:              ConfigValue<GateMode>,
}

impl ParsedConfigValues {
    fn set(&mut self, key: &str, value: &str) -> Result<(), ConfigError> {
        match key {
            TRUNK_KEY => self.trunk.set(key, parse_toml_string(value)?),
            MAXIMUM_RESERVATIONS_KEY => self
                .maximum_reservations
                .set(key, parse_unsigned_integer(key, value)?),
            MAXIMUM_ORDERING_EDGES_KEY => self
                .maximum_ordering_edges
                .set(key, parse_unsigned_integer(key, value)?),
            GATE_MODE_KEY => self.gate_mode.set(key, GateMode::parse(value)?),
            _ => Err(ConfigError::UnknownKey(key.to_owned())),
        }
    }

    fn finish(self) -> BerthConfig {
        let BerthConfig {
            trunk,
            maximum_reservations,
            maximum_ordering_edges,
            gate_mode,
        } = BerthConfig::default();
        BerthConfig {
            trunk:                  self.trunk.into_or(trunk),
            maximum_reservations:   self.maximum_reservations.into_or(maximum_reservations),
            maximum_ordering_edges: self.maximum_ordering_edges.into_or(maximum_ordering_edges),
            gate_mode:              self.gate_mode.into_or(gate_mode),
        }
    }
}

#[derive(Default)]
enum ConfigValue<Value> {
    #[default]
    Missing,
    Present(Value),
}

impl<Value> ConfigValue<Value> {
    fn set(&mut self, key: &str, value: Value) -> Result<(), ConfigError> {
        match self {
            Self::Missing => {
                *self = Self::Present(value);
                Ok(())
            },
            Self::Present(_) => Err(ConfigError::DuplicateKey(key.to_owned())),
        }
    }

    fn into_or(self, default: Value) -> Value {
        match self {
            Self::Missing => default,
            Self::Present(value) => value,
        }
    }
}

impl GateMode {
    /// Return whether invalid trunk updates must be rejected.
    pub(crate) const fn enforces(self) -> bool { matches!(self, Self::Enforce) }

    fn parse(value: &str) -> Result<Self, ConfigError> {
        match parse_toml_string(value)?.as_str() {
            "observe" => Ok(Self::Observe),
            "enforce" => Ok(Self::Enforce),
            _ => Err(ConfigError::InvalidValue {
                key:   GATE_MODE_KEY.to_owned(),
                value: value.to_owned(),
            }),
        }
    }
}

fn parse_toml_string(value: &str) -> Result<String, ConfigError> {
    let Some(without_opening_quote) = value.strip_prefix('"') else {
        return Err(ConfigError::InvalidSyntax(value.to_owned()));
    };
    let Some(string_contents) = without_opening_quote.strip_suffix('"') else {
        return Err(ConfigError::UnterminatedString(value.to_owned()));
    };
    if string_contents.contains('"') {
        return Err(ConfigError::InvalidSyntax(value.to_owned()));
    }
    Ok(string_contents.to_owned())
}

fn strip_comment(line: &str) -> Result<&str, ConfigError> {
    let mut quoted_string = false;
    for (index, character) in line.char_indices() {
        match character {
            '"' => quoted_string = !quoted_string,
            '#' if !quoted_string => return Ok(&line[..index]),
            _ => {},
        }
    }
    if quoted_string {
        return Err(ConfigError::UnterminatedString(line.to_owned()));
    }
    Ok(line)
}

fn parse_unsigned_integer(key: &str, value: &str) -> Result<u32, ConfigError> {
    value.parse().map_err(|_| ConfigError::InvalidValue {
        key:   key.to_owned(),
        value: value.to_owned(),
    })
}

/// A failure while reading or creating repository policy.
#[derive(Debug)]
pub(crate) enum ConfigError {
    /// The configuration path had no parent directory.
    InvalidPath(PathBuf),
    /// Filesystem access failed.
    Io(std::io::Error),
    /// A line was not in the supported key/value form.
    InvalidSyntax(String),
    /// A known key contained an invalid value.
    InvalidValue {
        /// The invalid key.
        key:   String,
        /// The malformed value.
        value: String,
    },
    /// A quoted configuration value had no closing quote.
    UnterminatedString(String),
    /// A field appeared more than once.
    DuplicateKey(String),
    /// A field is not part of this configuration format.
    UnknownKey(String),
}

impl Display for ConfigError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPath(path) => {
                write!(
                    formatter,
                    "configuration path has no parent: {}",
                    path.display()
                )
            },
            Self::Io(error) => write!(formatter, "configuration I/O failed: {error}"),
            Self::InvalidSyntax(line) => {
                write!(formatter, "invalid berth configuration syntax: {line}")
            },
            Self::InvalidValue { key, value } => {
                write!(formatter, "invalid value for {key}: {value}")
            },
            Self::UnterminatedString(value) => {
                write!(
                    formatter,
                    "unterminated berth configuration string: {value}"
                )
            },
            Self::DuplicateKey(key) => {
                write!(formatter, "duplicate berth configuration key: {key}")
            },
            Self::UnknownKey(key) => write!(formatter, "unknown berth configuration key: {key}"),
        }
    }
}

impl std::error::Error for ConfigError {}

impl From<std::io::Error> for ConfigError {
    fn from(error: std::io::Error) -> Self { Self::Io(error) }
}

#[cfg(test)]
mod tests {
    use super::BerthConfig;
    use super::GateMode;

    #[test]
    fn default_configuration_round_trips() {
        let configuration = BerthConfig::default();

        assert!(
            BerthConfig::from_toml(&configuration.to_toml())
                .is_ok_and(|parsed_configuration| parsed_configuration == configuration)
        );
        assert_eq!(BerthConfig::default().gate_mode, GateMode::Observe);
    }

    #[test]
    fn empty_configuration_uses_every_default() {
        assert!(
            BerthConfig::from_toml("")
                .is_ok_and(|configuration| configuration == BerthConfig::default())
        );
    }

    #[test]
    fn partial_configuration_uses_defaults_for_omitted_values() {
        let expected_configuration = BerthConfig {
            trunk: "release".to_owned(),
            ..BerthConfig::default()
        };

        assert!(
            BerthConfig::from_toml("trunk = \"release\"")
                .is_ok_and(|configuration| configuration == expected_configuration)
        );
    }

    #[test]
    fn quoted_values_preserve_hash_characters() {
        assert!(
            BerthConfig::from_toml("trunk = \"release#1\"")
                .is_ok_and(|configuration| configuration.trunk == "release#1")
        );
    }
}
