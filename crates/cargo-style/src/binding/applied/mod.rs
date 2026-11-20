//! Prove each ticket rename against the base and target at one path.

mod admission;
mod edits;
mod location;
mod visibility;

use std::collections::BTreeMap;

use self::edits::EditProof;
use super::pairs::RenamePairs;
use super::presence::Presence;
use super::presence::PresenceRefusal;
use super::scan::BindingScan;
use crate::enclosing_items::UnreadableItems;
use crate::rename_admission::BindingRename;
use crate::rename_admission::OuterBinding;
use crate::rename_admission::OwnedRename;
use crate::rename_admission::PolicyInput;
use crate::rename_admission::RenameRefusal;

pub(crate) struct BindingProof {
    checks: BTreeMap<OwnedRename, BindingOutcome>,
}

#[derive(Clone)]
pub(crate) enum BindingOutcome {
    Applied,
    Unchanged,
    Unchecked,
    Blocked(OuterBinding),
    Unproved(RenameRefusal),
}

pub(crate) struct MissingRenamePolicy;

pub(crate) enum ComparisonSource {
    Text(String),
    Absent,
}

enum ComparedSource {
    Present(Result<Box<BindingScan>, UnreadableItems>),
    Absent,
}

struct EditRefusal;

impl From<ComparisonSource> for ComparedSource {
    fn from(source: ComparisonSource) -> Self {
        match source {
            ComparisonSource::Text(text) => Self::Present(BindingScan::read(text).map(Box::new)),
            ComparisonSource::Absent => Self::Absent,
        }
    }
}

impl BindingProof {
    pub(crate) fn read(
        before: ComparisonSource,
        after: ComparisonSource,
        renames: &[BindingRename<'_>],
        policy: &PolicyInput,
    ) -> Result<Self, MissingRenamePolicy> {
        let before = ComparedSource::from(before);
        let after = ComparedSource::from(after);
        let pairs = RenamePairs::from(renames);
        let edits = edits::compare(&before, &after, renames, &pairs);
        let empty_target;
        let admission_target = match &after {
            ComparedSource::Present(scan) => scan.as_deref(),
            ComparedSource::Absent => {
                empty_target = BindingScan::read(String::new());
                empty_target.as_ref()
            },
        }
        .map_err(|&error| error);
        let mut checks = BTreeMap::new();
        for rename in renames {
            let key = OwnedRename::from(rename);
            let outcome =
                match admission::compare(&before, admission_target, rename, policy, &pairs) {
                    Presence::Refused(PresenceRefusal::Blocked(binding)) => {
                        BindingOutcome::Blocked(binding)
                    },
                    Presence::Refused(PresenceRefusal::Unproved(refusal)) => {
                        BindingOutcome::Unproved(refusal)
                    },
                    Presence::Refused(PresenceRefusal::MissingPolicyFlag) => {
                        return Err(MissingRenamePolicy);
                    },
                    Presence::Clear => match edits.get(&key) {
                        Some(Ok(EditProof::Applied(_))) => BindingOutcome::Applied,
                        Some(Ok(EditProof::Unchanged)) => BindingOutcome::Unchanged,
                        _ => BindingOutcome::Unchecked,
                    },
                };
            checks.insert(key, outcome);
        }
        Ok(Self { checks })
    }

    pub(crate) fn check(&self, rename: &BindingRename<'_>) -> BindingOutcome {
        self.checks
            .get(&OwnedRename::from(rename))
            .cloned()
            .unwrap_or(BindingOutcome::Unchecked)
    }
}
