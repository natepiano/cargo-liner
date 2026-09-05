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

/// A resource available only to a repository enrolled in berth coordination.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Enrollment<T> {
    /// The repository is configured and participating.
    Enrolled(T),
    /// The repository has no configuration file, so it is not participating.
    Unconfigured {
        /// Where `cargo-berth init` would write the configuration.
        expected_configuration_path: PathBuf,
    },
}

/// The files one worktree consults for its configuration.
///
/// The configuration file is untracked and per-worktree, so `git worktree add` never
/// carries it along. Trunk and gate policy are facts about the repository rather than
/// about one checkout of it, so a linked worktree without a file of its own reads the
/// main worktree's.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ConfigurationLookup<'a> {
    /// Only the worktree's own file counts: a main worktree, or a linked worktree of a
    /// bare repository, which has no main worktree to read from.
    Own {
        /// The worktree whose file is read.
        repository_root: &'a Path,
    },
    /// The worktree's own file first, then the main worktree's.
    OwnThenMain {
        /// The linked worktree whose file is read first.
        repository_root:      &'a Path,
        /// The main worktree whose file answers when the linked worktree has none.
        main_repository_root: &'a Path,
    },
}

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

    /// Create the default configuration or validate an existing file without replacing it.
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
                let contents = fs::read_to_string(&configuration_path)?;
                Self::from_toml(&contents)?;
                return Ok(InitializationState::Existing);
            },
            Err(error) => return Err(ConfigError::Io(error)),
        };
        configuration_file.write_all(Self::default().to_toml().as_bytes())?;
        configuration_file.sync_all()?;
        Ok(InitializationState::Created)
    }

    /// Read and validate this worktree's configuration.
    ///
    /// When no file answers, the reported path is the one `cargo-berth init` should
    /// create: a linked worktree names the main worktree's file, because a file written
    /// there serves every worktree while one written in the linked worktree serves only
    /// itself.
    pub(crate) fn read(lookup: &ConfigurationLookup<'_>) -> Result<Enrollment<Self>, ConfigError> {
        let (repository_root, main_repository_root) = match *lookup {
            ConfigurationLookup::Own { repository_root } => (repository_root, None),
            ConfigurationLookup::OwnThenMain {
                repository_root,
                main_repository_root,
            } => (repository_root, Some(main_repository_root)),
        };
        let own_path = Self::path(repository_root);
        if let Some(configuration) = Self::read_file(&own_path)? {
            return Ok(Enrollment::Enrolled(configuration));
        }
        let Some(main_repository_root) = main_repository_root else {
            return Ok(Enrollment::Unconfigured {
                expected_configuration_path: own_path,
            });
        };
        let main_path = Self::path(main_repository_root);
        if let Some(configuration) = Self::read_file(&main_path)? {
            return Ok(Enrollment::Enrolled(configuration));
        }
        Ok(Enrollment::Unconfigured {
            expected_configuration_path: main_path,
        })
    }

    /// Read and validate one configuration file, or report that it does not exist.
    fn read_file(configuration_path: &Path) -> Result<Option<Self>, ConfigError> {
        match fs::read_to_string(configuration_path) {
            Ok(contents) => Self::from_toml(&contents).map(Some),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
            Err(error) => Err(ConfigError::Io(error)),
        }
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
    use std::fs;
    use std::io::Error;
    use std::io::ErrorKind;
    use std::path::Path;

    use tempfile::tempdir;

    use super::BerthConfig;
    use super::ConfigError;
    use super::ConfigurationLookup;
    use super::Enrollment;
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

    #[test]
    fn unconfigured_enrollment_carries_the_expected_path() -> Result<(), Box<dyn std::error::Error>>
    {
        let repository = tempdir()?;
        let expected_path = BerthConfig::path(repository.path());

        match BerthConfig::read(&ConfigurationLookup::Own {
            repository_root: repository.path(),
        }) {
            Ok(Enrollment::Unconfigured {
                expected_configuration_path,
            }) => assert_eq!(expected_configuration_path, expected_path),
            result => {
                return Err(
                    format!("expected unconfigured enrollment, received {result:?}").into(),
                );
            },
        }

        Ok(())
    }

    fn write_configuration(repository_root: &Path, trunk: &str) -> Result<(), ConfigError> {
        let configuration_path = BerthConfig::path(repository_root);
        let parent = configuration_path
            .parent()
            .ok_or_else(|| ConfigError::InvalidPath(configuration_path.clone()))?;
        fs::create_dir_all(parent)?;
        fs::write(configuration_path, format!("trunk = \"{trunk}\"\n"))?;
        Ok(())
    }

    #[test]
    fn a_linked_worktree_without_its_own_file_reads_the_main_worktree()
    -> Result<(), Box<dyn std::error::Error>> {
        let main = tempdir()?;
        let linked = tempdir()?;
        write_configuration(main.path(), "release")?;

        let enrollment = BerthConfig::read(&ConfigurationLookup::OwnThenMain {
            repository_root:      linked.path(),
            main_repository_root: main.path(),
        })?;

        assert!(matches!(
            enrollment,
            Enrollment::Enrolled(configuration) if configuration.trunk == "release"
        ));
        Ok(())
    }

    #[test]
    fn a_linked_worktree_with_its_own_file_ignores_the_main_worktree()
    -> Result<(), Box<dyn std::error::Error>> {
        let main = tempdir()?;
        let linked = tempdir()?;
        write_configuration(main.path(), "release")?;
        write_configuration(linked.path(), "develop")?;

        let enrollment = BerthConfig::read(&ConfigurationLookup::OwnThenMain {
            repository_root:      linked.path(),
            main_repository_root: main.path(),
        })?;

        assert!(matches!(
            enrollment,
            Enrollment::Enrolled(configuration) if configuration.trunk == "develop"
        ));
        Ok(())
    }

    #[test]
    fn a_linked_worktree_with_no_file_anywhere_names_the_main_worktree_path()
    -> Result<(), Box<dyn std::error::Error>> {
        let main = tempdir()?;
        let linked = tempdir()?;

        let enrollment = BerthConfig::read(&ConfigurationLookup::OwnThenMain {
            repository_root:      linked.path(),
            main_repository_root: main.path(),
        })?;

        assert_eq!(
            enrollment,
            Enrollment::Unconfigured {
                expected_configuration_path: BerthConfig::path(main.path()),
            }
        );
        Ok(())
    }

    #[test]
    fn io_error_conversion_preserves_the_error_kind() {
        let config_error = ConfigError::from(Error::new(
            ErrorKind::PermissionDenied,
            "configuration access denied",
        ));

        assert!(matches!(
            config_error,
            ConfigError::Io(error) if error.kind() == ErrorKind::PermissionDenied
        ));
    }
}
