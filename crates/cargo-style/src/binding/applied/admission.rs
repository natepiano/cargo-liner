//! Apply current rename admission to a proof from a ticket.

use super::ComparedSource;
use crate::binding::alignment;
use crate::binding::alignment::CurrentAdmission;
use crate::binding::declaration::Declaration;
use crate::binding::pairs::RenamePairs;
use crate::binding::presence::Presence;
use crate::binding::presence::PresenceRefusal;
use crate::binding::scan::BindingScan;
use crate::enclosing_items::UnreadableItems;
use crate::rename_admission::BindingRename;
use crate::rename_admission::PolicyInput;
use crate::rename_admission::RefusalCause;

pub(super) fn compare(
    before: &ComparedSource,
    after: Result<&BindingScan, UnreadableItems>,
    rename: &BindingRename<'_>,
    policy: &PolicyInput,
    pairs: &RenamePairs<'_>,
) -> Presence {
    let empty_base = match before {
        ComparedSource::Absent => true,
        ComparedSource::Present(scan) => scan
            .as_ref()
            .is_ok_and(|scan| scan.source.text().is_empty()),
    };
    let Ok(after) = after else {
        let reason = if empty_base {
            RefusalCause::TargetTokenSpansUnread
        } else {
            RefusalCause::TokensUnread
        };
        return alignment::unproved_presence(reason, rename);
    };
    if empty_base {
        return alignment::coordinate_refusal(after, rename, policy);
    }
    if let ComparedSource::Present(Ok(before)) = before
        && let Ok(Declaration::Local(scope)) = before.declare(rename)
    {
        return match alignment::map_approved_renames(after, rename, before, &scope, policy, pairs) {
            CurrentAdmission::Mapped(_) => Presence::Clear,
            CurrentAdmission::Refused(refusal) => Presence::Refused(refusal),
        };
    }
    if matches!(
        alignment::coordinate_refusal(after, rename, policy),
        Presence::Refused(PresenceRefusal::MissingPolicyFlag)
    ) {
        return Presence::Refused(PresenceRefusal::MissingPolicyFlag);
    }
    let ComparedSource::Present(Ok(before)) = before else {
        return alignment::unproved_presence(RefusalCause::TokensUnread, rename);
    };
    match before.declare(rename) {
        Err(cause) if !before.proves_companion(rename) => {
            alignment::unproved_presence(cause, rename)
        },
        _ => Presence::Clear,
    }
}
