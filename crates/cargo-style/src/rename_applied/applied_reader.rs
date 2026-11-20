//! Select and cache rename proofs for one finding reply.

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;

use super::AppliedError;
use super::RenameApplied;
use super::file;
use super::file::FilePair;
use super::qualified;
use super::read_source;
use crate::applied_tickets::BuildComparison;
use crate::applied_tickets::TicketComparison;
use crate::binding;
use crate::binding::BindingOutcome;
use crate::binding::BindingProof;
use crate::binding::ComparisonSource;
use crate::binding::Pair;
use crate::finding_source::FindingSource;
use crate::finding_source::TreeFile;
use crate::ledger_read::HeldRename;
use crate::rename_admission::BindingRename;
use crate::rename_admission::PolicyInput;

/// One cache per reply. The target is fixed by the ledger snapshot.
#[derive(Default)]
pub(crate) struct AppliedReader {
    proofs: BTreeMap<(String, String), BindingProof>,
}

impl AppliedReader {
    pub(crate) fn check(
        &mut self,
        finding: &HeldRename,
        target: &str,
        source: &mut FindingSource<'_>,
        policy: &PolicyInput,
    ) -> Result<RenameApplied, AppliedError> {
        let anchor = &finding.anchor;
        match &finding.comparison {
            BuildComparison::Ticket(ticket) => {
                if let Pair::Local(rename) = binding::read_pair(
                    &finding.kind,
                    &finding.before,
                    &finding.after,
                    anchor.line,
                    &anchor.text,
                ) {
                    return self.binding_check(
                        ticket,
                        &anchor.path,
                        target,
                        &rename,
                        source,
                        policy,
                    );
                }
            },
            BuildComparison::MissingTicket(id) => {
                return Err(AppliedError::MissingTicket(id.clone()));
            },
            BuildComparison::NotBinding | BuildComparison::NoBase => {},
        }
        if let FilePair::Rename(rename) = file::read(&finding.kind, &anchor.path, &finding.after) {
            return rename.check(&finding.kind, source, target);
        }
        let text = read_source(source, target, &anchor.path)?;
        Ok(qualified::check(finding, text))
    }

    fn binding_check(
        &mut self,
        ticket: &TicketComparison,
        path: &str,
        target: &str,
        rename: &BindingRename<'_>,
        source: &mut FindingSource<'_>,
        policy: &PolicyInput,
    ) -> Result<RenameApplied, AppliedError> {
        let key = (ticket.id.clone(), path.to_owned());
        let proof = match self.proofs.entry(key) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let before = ComparisonSource::from(read_source(source, &ticket.base, path)?);
                let after = ComparisonSource::from(read_source(source, target, path)?);
                let renames = ticket
                    .members
                    .iter()
                    .filter_map(|member| {
                        match binding::read_pair(
                            &member.kind,
                            &member.before,
                            &member.after,
                            member.line,
                            &member.text,
                        ) {
                            Pair::Local(rename) => Some(rename),
                            Pair::NonLocal => None,
                        }
                    })
                    .collect::<Vec<_>>();
                let proof = BindingProof::read(before, after, &renames, policy)
                    .map_err(|_| AppliedError::MissingPolicy)?;
                entry.insert(proof)
            },
        };
        Ok(match proof.check(rename) {
            BindingOutcome::Applied => RenameApplied::AppliedInCode,
            BindingOutcome::Unchanged => RenameApplied::NotApplied {
                path: path.to_owned(),
            },
            BindingOutcome::Unchecked => RenameApplied::NoAppliedCheck {
                kind: "rename".to_owned(),
            },
            BindingOutcome::Blocked(binding) => RenameApplied::Blocked(binding),
            BindingOutcome::Unproved(refusal) => RenameApplied::Unproved(refusal),
        })
    }
}

impl From<TreeFile<'_>> for ComparisonSource {
    fn from(file: TreeFile<'_>) -> Self {
        match file {
            TreeFile::Bytes(bytes) => Self::Text(String::from_utf8_lossy(bytes).into_owned()),
            TreeFile::Folder | TreeFile::Missing => Self::Absent,
        }
    }
}
