//! Read the rename policy only when a proof reaches the presence check.

use std::cell::OnceCell;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use toml::Value;

use super::outcome::python_repr;
use super::policy_syntax;
use crate::constants::FINDING_RENAME_POLICY_FLAG;

pub(crate) enum PolicyInput {
    NoFlag,
    File(PolicyFile),
}

pub(crate) struct PolicyFile {
    path:     PathBuf,
    snapshot: OnceCell<PolicySnapshot>,
}

enum PolicySnapshot {
    Read(RenamePolicy),
    Unread(PolicyError),
}

pub(crate) enum PolicyLoad<'a> {
    Read(&'a RenamePolicy),
    Unread(&'a PolicyError),
    MissingFlag,
}

pub(crate) struct RenamePolicy {
    pub(crate) inert_attributes: Vec<String>,
    pub(crate) std_macros:       Vec<String>,
}

#[derive(Clone, Debug, thiserror::Error)]
pub(crate) enum PolicyError {
    #[error("[Errno {number}] {message}: {}", python_repr(.path))]
    File {
        number:  i32,
        message: String,
        path:    String,
    },
    #[error("{0}")]
    Encoding(String),
    #[error("{0}")]
    Syntax(String),
    #[error("inert_attributes must be a list of names")]
    InertAttributes,
    #[error("std_macros must be a list of names")]
    StandardMacros,
}

#[derive(Debug, thiserror::Error)]
#[error("rename admission requires {FINDING_RENAME_POLICY_FLAG} <rename_policy.toml>")]
pub(crate) struct MissingPolicyFlag;

impl From<PathBuf> for PolicyInput {
    fn from(path: PathBuf) -> Self {
        Self::File(PolicyFile {
            path,
            snapshot: OnceCell::new(),
        })
    }
}

impl PolicyInput {
    pub(crate) fn load(&self) -> PolicyLoad<'_> {
        match self {
            Self::NoFlag => PolicyLoad::MissingFlag,
            Self::File(file) => match file.snapshot.get_or_init(|| match read(&file.path) {
                Ok(policy) => PolicySnapshot::Read(policy),
                Err(error) => PolicySnapshot::Unread(error),
            }) {
                PolicySnapshot::Read(policy) => PolicyLoad::Read(policy),
                PolicySnapshot::Unread(error) => PolicyLoad::Unread(error),
            },
        }
    }
}

fn read(path: &Path) -> Result<RenamePolicy, PolicyError> {
    let bytes = fs::read(path).map_err(|error| {
        let number = error.raw_os_error().unwrap_or(0);
        let message = error.to_string();
        let message = message.split(" (os error ").next().unwrap_or(&message);
        PolicyError::File {
            number,
            message: message.to_owned(),
            path: path.to_string_lossy().into_owned(),
        }
    })?;
    let text = std::str::from_utf8(&bytes).map_err(|error| {
        let start = error.valid_up_to();
        let reason = if error.error_len().is_some() {
            if (0xc2..=0xf4).contains(&bytes[start]) {
                "invalid continuation byte"
            } else {
                "invalid start byte"
            }
        } else {
            "unexpected end of data"
        };
        let length = error.error_len().unwrap_or(bytes.len() - start);
        let detail = if length == 1 {
            format!("byte 0x{:02x} in position {start}", bytes[start])
        } else {
            format!("bytes in position {start}-{}", start + length - 1)
        };
        PolicyError::Encoding(format!("'utf-8' codec can't decode {detail}: {reason}"))
    })?;
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let table = policy_syntax::parse(&text)?;
    let names = |key| match table.get(key) {
        Some(Value::Array(values)) => values
            .iter()
            .map(|value| value.as_str().map(str::to_owned))
            .collect::<Option<Vec<_>>>(),
        _ => None,
    };
    Ok(RenamePolicy {
        inert_attributes: names("inert_attributes").ok_or(PolicyError::InertAttributes)?,
        std_macros:       names("std_macros").ok_or(PolicyError::StandardMacros)?,
    })
}
