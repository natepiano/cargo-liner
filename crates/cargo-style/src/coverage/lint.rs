//! Read the saved workspace lint that matches the committed configuration.

use std::collections::BTreeMap;
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::Path;

use gix::ObjectId;
use rusqlite::Connection;
use rusqlite::OptionalExtension as _;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Serializer;
use serde_json::ser::Formatter;
use sha2::Digest as _;
use sha2::Sha256;
use toml::Table;

use super::CoverageError;
use super::constants::MEND_GLOBAL_CONFIG;
use super::constants::MEND_REPO_CONFIG;
use crate::error::ReaderError;
use crate::facts;
use crate::finding_source::EntryKind;
use crate::finding_source::FindingSource;
use crate::finding_source::SourceText;
use crate::finding_source::TreeEntries;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum LintTool {
    Clippy,
    Rustfmt,
    Mend,
}

impl LintTool {
    pub(super) const fn name(self) -> &'static str {
        match self {
            Self::Clippy => "clippy",
            Self::Rustfmt => "rustfmt",
            Self::Mend => "mend",
        }
    }

    fn covers_config(self, path: &str, name: &str) -> bool {
        let names = match self {
            Self::Clippy => ["clippy.toml", ".clippy.toml"],
            Self::Rustfmt => ["rustfmt.toml", ".rustfmt.toml"],
            Self::Mend => ["mend.toml", ".mend.toml"],
        };
        names.contains(&name) || self == Self::Mend && path == MEND_REPO_CONFIG
    }
}

pub(super) enum LintCoverage {
    NeverRun,
    Stale,
    Passed {
        diagnostics: BTreeMap<String, usize>,
        pins:        BTreeMap<String, String>,
    },
}

impl LintCoverage {
    pub(super) fn read(
        connection: &Connection,
        tool: LintTool,
        tree: &str,
        config: &str,
    ) -> Result<Self, CoverageError> {
        let matching = connection.query_row(
            "SELECT passed,diagnostics_json,rule_blobs_json FROM lint_runs
             WHERE tool=?1 AND scope='workspace' AND tree=?2 AND config_hash=?3 ORDER BY id DESC LIMIT 1",
            [tool.name(), tree, config],
            |row| Ok((row.get::<_, bool>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)),
        ).optional()?;
        if let Some((passed, diagnostics, pins)) = matching {
            let diagnostics =
                serde_json::from_str(&diagnostics).map_err(|source| CoverageError::LintJson {
                    column: "diagnostics_json",
                    source,
                })?;
            let pins = serde_json::from_str(&pins).map_err(|source| CoverageError::LintJson {
                column: "rule_blobs_json",
                source,
            })?;
            return Ok(if passed {
                Self::Passed { diagnostics, pins }
            } else {
                Self::Stale
            });
        }
        let has_runs: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM lint_runs WHERE tool=?1 AND scope='workspace')",
            [tool.name()],
            |row| row.get(0),
        )?;
        Ok(if has_runs {
            Self::Stale
        } else {
            Self::NeverRun
        })
    }
}

pub(super) fn tree_at(repo: &Path, commit: &str) -> Result<String, ReaderError> {
    let repository = gix::open(repo).map_err(|source| ReaderError::Open {
        path: repo.to_owned(),
        source,
    })?;
    let object_id =
        ObjectId::from_hex(commit.as_bytes()).map_err(|source| ReaderError::CommitId {
            commit: commit.to_owned(),
            source,
        })?;
    let tree = repository
        .find_object(object_id)
        .map_err(|source| ReaderError::Tree {
            commit: commit.to_owned(),
            source: Box::new(source),
        })?
        .peel_to_tree()
        .map_err(|source| ReaderError::Tree {
            commit: commit.to_owned(),
            source: Box::new(source),
        })?;
    Ok(tree.id.to_string())
}

pub(super) fn config_digest(
    source: &mut FindingSource<'_>,
    tree: &TreeEntries,
    tool: LintTool,
) -> Result<String, CoverageError> {
    let mut inputs = Vec::new();
    for (path, entry) in tree {
        if entry.kind != EntryKind::Blob {
            continue;
        }
        let name = path.rsplit('/').next().unwrap_or(path);
        if tool.covers_config(path, name) {
            inputs.push((path.as_str(), entry.object.clone()));
        }
        if name == "Cargo.toml" && tool != LintTool::Rustfmt {
            let SourceText::Utf8(text) = source.source(&entry.object)? else {
                return Err(CoverageError::UnreadableLintManifest(path.clone()));
            };
            let manifest: Table = toml::from_str(text.lines.text()).map_err(|source| {
                CoverageError::LintManifest {
                    path:   path.clone(),
                    source: Box::new(source),
                }
            })?;
            let lints = manifest.get("workspace").and_then(|item| item.get("lints"));
            let mend = if tool == LintTool::Mend {
                manifest
                    .get("package")
                    .and_then(|item| item.get("metadata"))
                    .and_then(|item| item.get("mend"))
            } else {
                None
            };
            let mut configuration = serde_json::json!({ "lints": lints, "mend": mend });
            configuration.sort_all_objects();
            let mut json = Vec::new();
            configuration.serialize(&mut Serializer::with_formatter(&mut json, PythonJson))?;
            let json = String::from_utf8(json).map_err(CoverageError::LintEncoding)?;
            inputs.push((path.as_str(), json));
        }
    }
    if tool == LintTool::Mend
        && let Some(root) = env::var_os("XDG_CONFIG_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| env::home_dir().map(|home| home.join(".config")))
    {
        let path = root.join(MEND_GLOBAL_CONFIG);
        if path.is_file() {
            let bytes = fs::read(&path)
                .map_err(|source| CoverageError::GlobalLintConfig { path, source })?;
            let mut digest = String::new();
            for byte in Sha256::digest(bytes) {
                write!(digest, "{byte:02x}").map_err(CoverageError::LintDigestEncoding)?;
            }
            inputs.push(("global:cargo-mend/config.toml", digest));
        }
    }
    Ok(facts::python_json_digest(&inputs)?)
}

/// Match the spaces and ASCII escapes in the Python JSON inside each manifest row.
struct PythonJson;

impl Formatter for PythonJson {
    fn begin_array_value<W: io::Write + ?Sized>(
        &mut self,
        writer: &mut W,
        first: bool,
    ) -> io::Result<()> {
        if first {
            Ok(())
        } else {
            writer.write_all(b", ")
        }
    }

    fn begin_object_key<W: io::Write + ?Sized>(
        &mut self,
        writer: &mut W,
        first: bool,
    ) -> io::Result<()> {
        self.begin_array_value(writer, first)
    }

    fn begin_object_value<W: io::Write + ?Sized>(&mut self, writer: &mut W) -> io::Result<()> {
        writer.write_all(b": ")
    }

    fn write_string_fragment<W: io::Write + ?Sized>(
        &mut self,
        writer: &mut W,
        text: &str,
    ) -> io::Result<()> {
        for character in text.chars() {
            if character <= '~' {
                write!(writer, "{character}")?;
            } else {
                for unit in character.encode_utf16(&mut [0; 2]) {
                    write!(writer, "\\u{unit:04x}")?;
                }
            }
        }
        Ok(())
    }
}
