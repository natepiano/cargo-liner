//! Read the user's answers on decision files and record each answer a finding reads.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::fmt;
use std::fmt::Formatter;
use std::fs;
use std::io;
use std::path::Path;
use std::path::PathBuf;

use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use serde::de::MapAccess;
use serde::de::Visitor;

use crate::tip_view;

/// The user's answer on one decision file.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Answer {
    Approved,
    Declined,
    Unanswered,
}

/// The answers on decision files, by normalized path.
#[derive(Default)]
pub(crate) struct ApprovalSnapshot {
    decisions: HashMap<String, Answer>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ApprovalFile {
    decisions: DecisionEntries,
}

/// The decision entries in file order, with each repeated key kept, so a repeated key is an error.
struct DecisionEntries(Vec<(String, Answer)>);

struct DecisionEntriesVisitor;

#[derive(Debug, thiserror::Error)]
pub(crate) enum ApprovalFailure {
    #[error("cannot read approvals {path}")]
    Read { path: PathBuf, source: io::Error },
    #[error("cannot parse approvals {path}")]
    Parse {
        path:   PathBuf,
        source: serde_json::Error,
    },
    #[error("approvals {path} name decision {decision} twice")]
    Repeated { path: PathBuf, decision: String },
}

/// A decision path that the approvals file does not answer.
pub(crate) struct Unobserved(pub(super) String);

/// The answers that one finding read, by normalized path.
#[derive(Default)]
pub(crate) struct ReadAnswers {
    answers: BTreeMap<String, Answer>,
}

#[derive(Serialize)]
pub(super) struct ReadAnswer {
    path:   String,
    answer: Answer,
}

impl ApprovalSnapshot {
    pub(crate) fn read(path: &Path) -> Result<Self, ApprovalFailure> {
        let text = fs::read(path).map_err(|source| ApprovalFailure::Read {
            path: path.to_owned(),
            source,
        })?;
        let file: ApprovalFile =
            serde_json::from_slice(&text).map_err(|source| ApprovalFailure::Parse {
                path: path.to_owned(),
                source,
            })?;
        let mut decisions = HashMap::new();
        for (decision, answer) in file.decisions.0 {
            match decisions.entry(normalized(&decision)) {
                Entry::Occupied(entry) => {
                    return Err(ApprovalFailure::Repeated {
                        path:     path.to_owned(),
                        decision: entry.key().clone(),
                    });
                },
                Entry::Vacant(entry) => {
                    entry.insert(answer);
                },
            }
        }
        Ok(Self { decisions })
    }

    /// Whether the user approved the decision at `proposal`; an unanswered path stops the finding.
    pub(crate) fn approved(
        &self,
        proposal: &str,
        read: &mut ReadAnswers,
    ) -> Result<bool, Unobserved> {
        let path = normalized(proposal);
        let Some(&answer) = self.decisions.get(&path) else {
            return Err(Unobserved(path));
        };
        read.answers.insert(path, answer);
        Ok(answer == Answer::Approved)
    }
}

impl<'de> Deserialize<'de> for DecisionEntries {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(DecisionEntriesVisitor)
    }
}

impl<'de> Visitor<'de> for DecisionEntriesVisitor {
    type Value = DecisionEntries;

    fn expecting(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str("a map from decision path to answer")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut entries = Vec::new();
        while let Some(entry) = map.next_entry()? {
            entries.push(entry);
        }
        Ok(DecisionEntries(entries))
    }
}

impl ReadAnswers {
    pub(super) fn into_answers(self) -> Vec<ReadAnswer> {
        self.answers
            .into_iter()
            .map(|(path, answer)| ReadAnswer { path, answer })
            .collect()
    }
}

/// The path as Python's `PurePosixPath` prints it.
fn normalized(path: &str) -> String {
    let root = if path.starts_with("//") && !path.starts_with("///") {
        "//"
    } else if path.starts_with('/') {
        "/"
    } else {
        ""
    };
    let parts: Vec<_> = tip_view::path_parts(path).collect();
    if root.is_empty() && parts.is_empty() {
        return ".".to_owned();
    }
    format!("{root}{}", parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::normalized;

    #[test]
    fn paths_normalize_as_python_prints_them() {
        for (path, expected) in [
            ("", "."),
            (".", "."),
            ("a//b/./c/", "a/b/c"),
            ("/a", "/a"),
            ("//a", "//a"),
            ("///a", "/a"),
            ("./decision.md", "decision.md"),
        ] {
            assert_eq!(normalized(path), expected, "{path}");
        }
    }
}
