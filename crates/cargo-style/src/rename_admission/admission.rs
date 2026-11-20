//! Prove the anchor and bind the current result to the finding history.

use serde::Serialize;
use serde_json::Error as JsonError;
use serde_json::ser::Formatter;

use super::outcome::AdmittedLocalRename;
use super::outcome::BindingRename;
use super::outcome::RefusalCause;
use super::outcome::RenameAdmission;
use super::outcome::RenameRefusal;
use super::policy::MissingPolicyFlag;
use super::policy::PolicyInput;
use crate::binding;
use crate::binding::BindingScan;
use crate::binding::CurrentAdmission;
use crate::binding::Declaration;
use crate::binding::PresenceRefusal;
use crate::facts;
use crate::finding_anchor::FindingAnchor;
use crate::finding_source::FindingSource;
use crate::finding_source::FindingSourceFailure;
use crate::finding_source::TreeFile;
use crate::ledger_read::HeldRename;
use crate::tokenizer;
use crate::word_syntax;

#[derive(Debug, thiserror::Error)]
pub(crate) enum AdmissionError {
    #[error("cannot read rename admission source {path} at {commit}")]
    Source {
        path:   String,
        commit: String,
        source: FindingSourceFailure,
    },
    #[error(transparent)]
    MissingPolicy(#[from] MissingPolicyFlag),
    #[error("cannot encode the rename finding history")]
    History(#[from] JsonError),
}

pub(crate) fn admit(
    id: &str,
    finding: &HeldRename,
    anchor: &FindingAnchor,
    synced_tip: &str,
    source: &mut FindingSource<'_>,
    policy: &PolicyInput,
) -> Result<RenameAdmission, AdmissionError> {
    let PairReading::Local { old, new } = name_pair(finding) else {
        return Ok(RenameAdmission::NonLocalReferenceCompanion);
    };
    let rename = BindingRename {
        line: finding.anchor.line,
        text: &finding.anchor.text,
        old:  &old,
        new:  &new,
    };
    let refuse = |cause| RenameAdmission::Unproved(RenameRefusal::at_finding(cause, &rename));
    let anchor = match anchor {
        FindingAnchor::Anchored(anchor) => anchor,
        FindingAnchor::NeedsJudgment { reason } => return Ok(refuse((*reason).into())),
    };
    if anchor.path != finding.anchor.path {
        return Ok(refuse(RefusalCause::CurrentPathUnproved));
    }
    let original = match read_source(source, &anchor.commit, &anchor.path)? {
        TreeFile::Bytes(bytes) => {
            let Ok(text) = std::str::from_utf8(bytes) else {
                return Ok(refuse(RefusalCause::AnchorFileNotUtf8));
            };
            text.to_owned()
        },
        TreeFile::Folder | TreeFile::Missing => return Ok(refuse(RefusalCause::AnchorFileMissing)),
    };
    let Ok(previous) = BindingScan::read(original) else {
        return Ok(refuse(RefusalCause::AnchorTokensUnread));
    };
    let anchored = BindingRename {
        line: anchor.line,
        ..rename
    };
    let scope = match previous.declare(&anchored) {
        Ok(Declaration::Local(scope)) => scope,
        Ok(Declaration::NonLocal) => return Ok(RenameAdmission::NonLocalReferenceCompanion),
        Err(_) if previous.proves_companion(&anchored) => {
            return Ok(RenameAdmission::NonLocalReferenceCompanion);
        },
        Err(cause) => return Ok(refuse(cause)),
    };
    let current = match read_source(source, synced_tip, &finding.anchor.path)? {
        TreeFile::Bytes(bytes) => String::from_utf8_lossy(bytes),
        TreeFile::Folder | TreeFile::Missing => {
            return Ok(refuse(RefusalCause::CurrentFileMissing));
        },
    };
    let outcome = match binding::map_current_rename(&current, &anchored, &previous, &scope, policy)
    {
        CurrentAdmission::Mapped(mapped) => RenameAdmission::Admitted(AdmittedLocalRename {
            finding_version:      finding_version(id, finding)?,
            rename:               (&rename).into(),
            anchor:               anchor.clone(),
            item_identity:        mapped.item_identity,
            declaration_position: mapped.declaration_position,
            current_path:         finding.anchor.path.clone(),
            source_fingerprint:   facts::source_digest(&current)?,
            edit_mapping:         mapped.edit_mapping,
        }),
        CurrentAdmission::Refused(PresenceRefusal::Blocked(binding)) => {
            RenameAdmission::Blocked(binding)
        },
        CurrentAdmission::Refused(PresenceRefusal::Unproved(refusal)) => {
            RenameAdmission::Unproved(refusal)
        },
        CurrentAdmission::Refused(PresenceRefusal::MissingPolicyFlag) => {
            return Err(MissingPolicyFlag.into());
        },
    };
    Ok(outcome)
}

fn read_source<'a>(
    source: &'a mut FindingSource<'_>,
    commit: &str,
    path: &str,
) -> Result<TreeFile<'a>, AdmissionError> {
    source
        .raw_file(commit, path)
        .map_err(|source| AdmissionError::Source {
            path: path.to_owned(),
            commit: commit.to_owned(),
            source,
        })
}

enum PairReading {
    Local { old: String, new: String },
    NonLocal,
}

fn name_pair(finding: &HeldRename) -> PairReading {
    if finding.kind != "rename" || is_qualified_pair(&finding.before, &finding.after) {
        return PairReading::NonLocal;
    }
    let before = tokenizer::read_with_comments(&finding.before.chars().collect::<Vec<_>>()).tokens;
    let after = tokenizer::read_with_comments(&finding.after.chars().collect::<Vec<_>>()).tokens;
    if before.len() != after.len() {
        return PairReading::NonLocal;
    }
    let mut changes = before.iter().zip(&after).filter_map(|(old, new)| {
        (old.value != new.value || old.kind != new.kind).then_some((&old.value, &new.value))
    });
    let Some((old, new)) = changes.next() else {
        return PairReading::NonLocal;
    };
    if !binding::local_name(old)
        || !binding::local_name(new)
        || changes.any(|(left, right)| left != old || right != new)
    {
        return PairReading::NonLocal;
    }
    PairReading::Local {
        old: old.clone(),
        new: new.clone(),
    }
}

fn is_qualified_pair(before: &str, after: &str) -> bool {
    let left = before.rsplit_once("::");
    let right = after.rsplit_once("::");
    matches!((left, right), (Some((owner, old)), Some((next_owner, new)))
        if owner == next_owner && old != new && word_syntax::identifier(old) && word_syntax::identifier(new))
}

struct PythonSeparators;

impl Formatter for PythonSeparators {
    fn begin_array_value<W: ?Sized + std::io::Write>(
        &mut self,
        output: &mut W,
        first: bool,
    ) -> std::io::Result<()> {
        if first {
            Ok(())
        } else {
            output.write_all(b", ")
        }
    }

    fn begin_object_key<W: ?Sized + std::io::Write>(
        &mut self,
        output: &mut W,
        first: bool,
    ) -> std::io::Result<()> {
        self.begin_array_value(output, first)
    }

    fn begin_object_value<W: ?Sized + std::io::Write>(
        &mut self,
        output: &mut W,
    ) -> std::io::Result<()> {
        output.write_all(b": ")
    }
}

fn finding_version(id: &str, finding: &HeldRename) -> Result<String, AdmissionError> {
    let fields = (
        &finding.rule,
        &finding.rule_blob,
        &finding.anchor.path,
        finding.anchor.line,
        &finding.anchor.text,
        &finding.kind,
        &finding.before,
        &finding.after,
        &finding.ticket,
        &finding.follow,
        &finding.history,
    );
    let mut encoded = Vec::new();
    let mut serializer = serde_json::Serializer::with_formatter(&mut encoded, PythonSeparators);
    fields.serialize(&mut serializer)?;
    let digest = facts::python_serialized_json_digest(&String::from_utf8_lossy(&encoded))?;
    Ok(format!("{id}:{digest}"))
}
