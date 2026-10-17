//! Configuration homes and the labels that identify accounts in the census.

use std::collections::BTreeMap;
use std::collections::HashSet;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;

use super::Agent;
use super::classify;
use super::classify::ProcessEntry;
use crate::config::AccountsConfig;
use crate::constants::CLAUDE_CONFIG_DIR_VARIABLE;
use crate::constants::CLAUDE_DIRNAME;
use crate::constants::CODEX_DIRNAME;
use crate::constants::CODEX_HOME_VARIABLE;

/// A home and its label, resolved together by [`AccountLabels`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct AccountHome {
    /// The home directory, with the machine's home written as `~`.
    home:  String,
    /// The configured label, else the last component without its leading dot.
    label: String,
}

impl AccountHome {
    /// The account label shown in tiles and the summary.
    pub(crate) fn label(&self) -> &str { &self.label }
}

/// The configuration home an agent process runs under, with the label it shows as.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(from = "Option<AccountHome>", into = "Option<AccountHome>")]
pub(crate) enum Account {
    /// No home is known for this row: a probe printed before accounts existed,
    /// a process whose environment could not be read, a default home on a machine
    /// whose home directory is unknown, or a row that is not an agent process
    /// (a shell, a subagent).
    #[default]
    Unrecorded,
    /// The home the process runs under.
    Home(AccountHome),
}

impl Account {
    /// Whether the wire can omit this account.
    pub(crate) const fn is_unrecorded(&self) -> bool { matches!(self, Self::Unrecorded) }
}

impl From<Option<AccountHome>> for Account {
    fn from(home: Option<AccountHome>) -> Self { home.map_or(Self::Unrecorded, Self::Home) }
}

impl From<Account> for Option<AccountHome> {
    fn from(account: Account) -> Self {
        match account {
            Account::Unrecorded => None,
            Account::Home(home) => Some(home),
        }
    }
}

/// The configuration home a process's environment names.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum ConfigHome {
    /// The environment was not read.
    Unread,
    /// The environment names no home, so this program uses its default.
    Default(Program),
    /// The value of `CLAUDE_CONFIG_DIR` or `CODEX_HOME`.
    Named(PathBuf),
}

/// The program whose environment names a configuration home.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum Program {
    /// Claude Code reads `CLAUDE_CONFIG_DIR`.
    Claude,
    /// Codex reads `CODEX_HOME`.
    Codex,
}

impl Program {
    /// The environment variable naming this program's configuration home.
    const fn variable(self) -> &'static str {
        match self {
            Self::Claude => CLAUDE_CONFIG_DIR_VARIABLE,
            Self::Codex => CODEX_HOME_VARIABLE,
        }
    }

    /// The configuration directory under the machine's home when no home is named.
    const fn default_directory(self) -> &'static str {
        match self {
            Self::Claude => CLAUDE_DIRNAME,
            Self::Codex => CODEX_DIRNAME,
        }
    }
}

impl From<Agent> for Program {
    fn from(agent: Agent) -> Self {
        match agent {
            Agent::Claude => Self::Claude,
            Agent::Codex | Agent::CodexThreadHost => Self::Codex,
        }
    }
}

/// `AccountColumn::of` selects account visibility for `AgentRow` labels and
/// tree account columns. `agent_cell::account_column` applies that decision
/// across the frame's agent entries and their nested lines.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AccountColumn {
    /// `AccountColumn::of` found at most one account label per program.
    Hidden,
    /// `AccountColumn::of` found two or more account labels for one program.
    Shown,
}

impl AccountColumn {
    /// Select `Shown` when one program has two distinct account labels;
    /// otherwise select `Hidden`.
    pub(crate) fn of<'a>(accounts: impl IntoIterator<Item = (Agent, &'a Account)>) -> Self {
        let mut labels = HashSet::new();
        for (agent, account) in accounts {
            if let Account::Home(home) = account {
                labels.insert((Program::from(agent), home.label()));
            }
        }
        for program in [Program::Claude, Program::Codex] {
            if labels
                .iter()
                .filter(|(candidate, _)| *candidate == program)
                .count()
                > 1
            {
                return Self::Shown;
            }
        }
        Self::Hidden
    }
}

/// The account labels configured on the machine performing the scan.
#[derive(Debug, Default)]
pub(crate) struct AccountLabels(BTreeMap<String, String>);

impl AccountLabels {
    /// Normalize the configured homes once against the scanned machine's home.
    pub(crate) fn new(config: &AccountsConfig, machine_home: Option<&Path>) -> Self {
        Self(
            config
                .0
                .iter()
                .map(|(home, label)| {
                    (
                        classify::directory_label(Some(Path::new(home)), machine_home),
                        label.clone(),
                    )
                })
                .collect(),
        )
    }

    /// Resolve a home and its label against the scanned machine's home directory.
    pub(super) fn resolve(&self, home: &Path, machine_home: Option<&Path>) -> AccountHome {
        let abbreviated = classify::directory_label(Some(home), machine_home);
        let label = self.0.get(&abbreviated).cloned().unwrap_or_else(|| {
            let component = home
                .file_name()
                .unwrap_or(home.as_os_str())
                .to_string_lossy();
            component
                .strip_prefix('.')
                .unwrap_or(&component)
                .to_string()
        });
        AccountHome {
            home: abbreviated,
            label,
        }
    }
}

/// The home a read environment names for an agent; other processes stay unread.
pub(super) fn config_home_of(process: &ProcessEntry, environment: &[OsString]) -> ConfigHome {
    if process.is_claude() {
        config_home(environment, Program::Claude)
    } else if process.is_codex() {
        config_home(environment, Program::Codex)
    } else {
        ConfigHome::Unread
    }
}

/// The home named by a read environment; absent or empty means the tool's default.
pub(super) fn config_home(environment: &[OsString], program: Program) -> ConfigHome {
    environment
        .iter()
        .find_map(|variable| {
            variable
                .as_bytes()
                .strip_prefix(program.variable().as_bytes())?
                .strip_prefix(b"=")
        })
        .filter(|value| !value.is_empty())
        .map_or(ConfigHome::Default(program), |value| {
            ConfigHome::Named(PathBuf::from(OsStr::from_bytes(value)))
        })
}

/// The account recorded for an agent process's configuration home.
pub(super) fn account_of(
    process: &ProcessEntry,
    machine_home: Option<&Path>,
    account_labels: &AccountLabels,
) -> Account {
    match &process.config_home {
        ConfigHome::Unread => Account::Unrecorded,
        ConfigHome::Default(program) => machine_home.map_or(Account::Unrecorded, |home| {
            Account::Home(
                account_labels.resolve(&home.join(program.default_directory()), machine_home),
            )
        }),
        ConfigHome::Named(home) => Account::Home(account_labels.resolve(home, machine_home)),
    }
}

#[cfg(test)]
#[path = "account/tests.rs"]
mod tests;
