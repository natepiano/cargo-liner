//! Define the wire results of a local rename proof.

use std::fmt;
use std::fmt::Write as _;

use serde::Serialize;
use serde::Serializer;

use super::unprintable::UNPRINTABLE;
use crate::enclosing_items::UnreadableItems;
use crate::finding_anchor::NeedsJudgmentReason;
use crate::finding_anchor::ResolvedAnchor;
use crate::tokenizer;

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum RenameAdmission {
    Admitted(AdmittedLocalRename),
    NonLocalReferenceCompanion,
    Blocked(OuterBinding),
    Unproved(RenameRefusal),
}

#[derive(Serialize)]
pub(crate) struct BindingRename<'a> {
    pub(crate) line: usize,
    pub(crate) text: &'a str,
    pub(crate) old:  &'a str,
    pub(crate) new:  &'a str,
}

#[derive(Serialize)]
pub(crate) struct AdmittedLocalRename {
    pub(super) finding_version:      String,
    pub(super) rename:               OwnedRename,
    pub(super) anchor:               ResolvedAnchor,
    pub(super) item_identity:        Vec<String>,
    pub(super) declaration_position: usize,
    pub(super) current_path:         String,
    pub(super) source_fingerprint:   String,
    pub(super) edit_mapping:         Vec<(usize, usize)>,
}

#[derive(Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub(crate) struct OwnedRename {
    line: usize,
    text: String,
    old:  String,
    new:  String,
}

impl From<&BindingRename<'_>> for OwnedRename {
    fn from(rename: &BindingRename<'_>) -> Self {
        Self {
            line: rename.line,
            text: rename.text.to_owned(),
            old:  rename.old.to_owned(),
            new:  rename.new.to_owned(),
        }
    }
}

#[derive(Clone, Serialize)]
pub(crate) struct OuterBinding {
    #[serde(serialize_with = "serialize_reason")]
    reason: BindingConflict,
    name:   String,
    line:   usize,
    text:   String,
}

#[derive(Clone, thiserror::Error, Debug)]
#[error("rename blocked by outer binding {} at line {line}", python_repr(.name))]
struct BindingConflict {
    name: String,
    line: usize,
}

impl OuterBinding {
    pub(crate) fn new(name: &str, line: usize, text: &str) -> Self {
        Self {
            reason: BindingConflict {
                name: name.to_owned(),
                line,
            },
            name: name.to_owned(),
            line,
            text: text.to_owned(),
        }
    }
}

#[derive(Clone, Serialize)]
pub(crate) struct RenameRefusal {
    #[serde(serialize_with = "serialize_reason")]
    pub(crate) reason:   RefusalCause,
    #[serde(flatten)]
    pub(crate) location: RefusalLocation,
}

impl RenameRefusal {
    pub(crate) fn at_finding(reason: RefusalCause, rename: &BindingRename<'_>) -> Self {
        Self {
            reason,
            location: RefusalLocation::SourceLine {
                line: rename.line,
                text: rename.text.to_owned(),
            },
        }
    }
}

#[derive(Clone)]
pub(crate) enum RefusalLocation {
    Absent,
    Line(usize),
    SourceLine { line: usize, text: String },
}

impl Serialize for RefusalLocation {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Location<'a> {
            line: usize,
            text: &'a str,
        }
        match self {
            Self::Absent => Location { line: 0, text: "" },
            Self::Line(line) => Location {
                line: *line,
                text: "",
            },
            Self::SourceLine { line, text } => Location { line: *line, text },
        }
        .serialize(serializer)
    }
}

#[derive(Clone, Debug, thiserror::Error)]
pub(crate) enum RefusalCause {
    #[error("last follow commit is missing")]
    LastFollowCommitMissing,
    #[error("recorded anchor commit is missing")]
    RecordedAnchorCommitMissing,
    #[error("no judging ticket for source anchor")]
    NoJudgingTicket,
    #[error("no source anchor matches the stored coordinate")]
    NoMatchingAnchor,
    #[error("source anchor is ambiguous between base and checkpoint")]
    AmbiguousAnchor,
    #[error("source or input proof unreadable")]
    AnchorProofUnreadable,
    #[error("source anchor file is missing")]
    AnchorFileMissing,
    #[error("source anchor file is not UTF-8")]
    AnchorFileNotUtf8,
    #[error("current path has no proved follow from source anchor")]
    CurrentPathUnproved,
    #[error("source anchor tokens unread")]
    AnchorTokensUnread,
    #[error("current source file is missing")]
    CurrentFileMissing,
    #[error("finding line has no unique declaration of {}", python_repr(.0))]
    DeclarationNotUnique(String),
    #[error("declaration token has no binding scope")]
    BindingScopeMissing,
    #[error("condition let scrutinee contains a block")]
    ConditionBlock,
    #[error("for iterator contains a block before its body")]
    IteratorBlock,
    #[error("finding text is absent")]
    FindingTextAbsent,
    #[error("target tokens unread")]
    TokensUnread,
    #[error("target tokens unread: source token offsets are unavailable")]
    TargetTokenSpansUnread,
    #[error("recorded containing item unread")]
    RecordedItemUnread,
    #[error("containing item unread: {0}")]
    ContainingItemUnread(UnreadableItems),
    #[error("target has no established containing item")]
    ContainingItemMissing,
    #[error("recorded item path is not unique in current source")]
    ItemPathNotUnique,
    #[error("containing item body unread")]
    ItemBodyUnread,
    #[error("containing item header unread")]
    ItemHeaderUnread,
    #[error("recorded declaration position is not proved")]
    DeclarationPositionUnproved,
    #[error("current rename's own edits cannot be identified exactly")]
    EditsUnproved,
    #[error("current rename edits have unread binding visibility")]
    BindingVisibilityUnread,
    #[error("current rename's declaration edit is not proved")]
    DeclarationEditUnproved,
    #[error("rename presence policy unread: {0}")]
    PolicyUnread(super::PolicyError),
    #[error(transparent)]
    MissingPolicyFlag(super::MissingPolicyFlag),
    #[error("unresolved attribute macro {} at line {line}", python_repr(.name))]
    AttributeMacro { name: String, line: usize },
    #[error("unresolved bang macro {} at line {line}", python_repr(.name))]
    BangMacro { name: String, line: usize },
    #[error("wanted name already used in this item: {} at line {line}", python_repr(.name))]
    WantedNameUsed { name: String, line: usize },
}

impl From<NeedsJudgmentReason> for RefusalCause {
    fn from(reason: NeedsJudgmentReason) -> Self {
        match reason {
            NeedsJudgmentReason::LastFollowCommitMissing => Self::LastFollowCommitMissing,
            NeedsJudgmentReason::RecordedAnchorCommitMissing => Self::RecordedAnchorCommitMissing,
            NeedsJudgmentReason::NoJudgingTicket => Self::NoJudgingTicket,
            NeedsJudgmentReason::NoMatch => Self::NoMatchingAnchor,
            NeedsJudgmentReason::Ambiguous => Self::AmbiguousAnchor,
            NeedsJudgmentReason::UnreadableProof => Self::AnchorProofUnreadable,
        }
    }
}

fn serialize_reason<S: Serializer>(
    reason: &impl fmt::Display,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.collect_str(reason)
}

pub(super) fn python_repr(text: &str) -> String {
    let quote = if text.contains('\'') && !text.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut result = String::from(quote);
    for character in text.chars() {
        match character {
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            '\\' => result.push_str("\\\\"),
            character if character == quote => {
                result.push('\\');
                result.push(character);
            },
            character if tokenizer::in_table(UNPRINTABLE, character) => {
                let code = u32::from(character);
                let _ = if code <= 0xff {
                    write!(result, "\\x{code:02x}")
                } else if code <= 0xffff {
                    write!(result, "\\u{code:04x}")
                } else {
                    write!(result, "\\U{code:08x}")
                };
            },
            character => result.push(character),
        }
    }
    result.push(quote);
    result
}
