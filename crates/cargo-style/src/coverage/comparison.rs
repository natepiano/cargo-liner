//! Compare inputs at the judged commit and the covered tip.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::hash_map::Entry;

use super::CoverageError;
use super::ledger::Judgment;
use crate::facts::CodeIdentity;
use crate::facts::FactReading;
use crate::finding_source::FindingSource;
use crate::finding_source::SourceText;
use crate::finding_source::TreeEntry;
use crate::rule_snapshot::Family;
use crate::rule_snapshot::RuleFamily;
use crate::site_ledger::InputScope;
use crate::site_ledger::ReadScope;
use crate::tip_view;
use crate::tip_view::FingerprintSource;
use crate::tip_view::RepositoryTip;

type Inputs<'a> = BTreeMap<&'a str, &'a TreeEntry>;

#[derive(PartialEq, Eq, Hash)]
enum InputMember {
    File(String),
    Module(String),
    Crate(String),
    Manifest,
    Explicit(String, Vec<String>),
}

enum CodeReading {
    Read(String),
    Unreadable,
}

/// The fingerprint of a cell's inputs at one commit.
enum InputFingerprint {
    Read(String),
    /// A file that the inputs need is not in the tree at that commit.
    MissingInput,
}

#[derive(Default)]
pub(super) struct InputComparison {
    fingerprints: HashMap<String, HashMap<InputMember, String>>,
    code:         HashMap<String, CodeReading>,
}

impl InputComparison {
    pub(super) fn matches(
        &mut self,
        source: &mut FindingSource<'_>,
        path: &str,
        comparison: JudgmentComparison<'_>,
    ) -> Result<bool, CoverageError> {
        let JudgmentComparison {
            judgment,
            historical_commit,
            covered_commit,
            tip,
            scope,
            family,
        } = comparison;
        let effective_scope = judgment.declared.comparison(scope);
        let (member, fingerprint_source) = input_key(tip, effective_scope, path, &judgment.reads);
        let InputFingerprint::Read(covered) = self.fingerprint(
            covered_commit,
            member,
            fingerprint_source,
            ScopedInputs {
                tip,
                path,
                scope: effective_scope,
                reads: &judgment.reads,
            },
        )?
        else {
            return Ok(false);
        };
        let matches = if matches!(judgment.reads, ReadScope::Explicit(_))
            || judgment.scope == effective_scope
        {
            covered == judgment.fingerprint
        } else {
            let before = source.tip(historical_commit)?;
            let (member, fingerprint_source) =
                input_key(&before, effective_scope, path, &judgment.reads);
            let original = self.fingerprint(
                historical_commit,
                member,
                fingerprint_source,
                ScopedInputs {
                    tip: &before,
                    path,
                    scope: effective_scope,
                    reads: &judgment.reads,
                },
            )?;
            matches!(original, InputFingerprint::Read(original) if original == covered)
        };
        if matches || !matches!(family, RuleFamily::Known(Family::Modules)) {
            return Ok(matches);
        }
        let before = source.tip(historical_commit)?;
        let (Ok(before), Ok(after)) = (
            before.inputs(effective_scope, path, &judgment.reads),
            tip.inputs(effective_scope, path, &judgment.reads),
        ) else {
            return Ok(false);
        };
        self.only_comments_changed(source, &before, &after)
    }

    fn fingerprint(
        &mut self,
        commit: &str,
        member: InputMember,
        fingerprint_source: FingerprintSource<'_>,
        input: ScopedInputs<'_>,
    ) -> Result<InputFingerprint, CoverageError> {
        let by_member = self.fingerprints.entry(commit.to_owned()).or_default();
        let fingerprint = match by_member.entry(member) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let Ok(inputs) = input.tip.inputs(input.scope, input.path, input.reads) else {
                    return Ok(InputFingerprint::MissingInput);
                };
                entry.insert(input.tip.fingerprint(fingerprint_source, &inputs)?)
            },
        };
        Ok(InputFingerprint::Read(fingerprint.clone()))
    }

    fn only_comments_changed(
        &mut self,
        source: &mut FindingSource<'_>,
        before: &Inputs<'_>,
        after: &Inputs<'_>,
    ) -> Result<bool, CoverageError> {
        if !before.keys().eq(after.keys()) {
            return Ok(false);
        }
        let mut changed = before
            .iter()
            .zip(after.values())
            .filter(|((_, before), after)| *before != *after)
            .peekable();
        if changed.peek().is_none() {
            return Ok(false);
        }
        for ((path, before), after) in changed {
            if !tip_view::is_rust_path(path)
                || before.mode != after.mode
                || !matches!(before.mode.as_str(), "100644" | "100755")
            {
                return Ok(false);
            }
            let CodeReading::Read(old) = self.code(source, &before.object)? else {
                return Ok(false);
            };
            let old = old.clone();
            if !matches!(self.code(source, &after.object)?, CodeReading::Read(new) if *new == old) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn code(
        &mut self,
        source: &mut FindingSource<'_>,
        blob: &str,
    ) -> Result<&CodeReading, CoverageError> {
        Ok(match self.code.entry(blob.to_owned()) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let source = match source.source(blob)? {
                    SourceText::Utf8(source) => {
                        match FactReading::parse(source.lines.text().as_bytes())? {
                            FactReading::Read {
                                code_identity: CodeIdentity::Digest(digest),
                                ..
                            } => CodeReading::Read(digest),
                            _ => CodeReading::Unreadable,
                        }
                    },
                    SourceText::NotUtf8(_) => CodeReading::Unreadable,
                };
                entry.insert(source)
            },
        })
    }
}

#[derive(Clone, Copy)]
struct ScopedInputs<'a> {
    tip:   &'a RepositoryTip,
    path:  &'a str,
    scope: InputScope,
    reads: &'a ReadScope,
}

#[derive(Clone, Copy)]
pub(super) struct JudgmentComparison<'a> {
    pub(super) judgment:          &'a Judgment,
    pub(super) historical_commit: &'a str,
    pub(super) covered_commit:    &'a str,
    pub(super) tip:               &'a RepositoryTip,
    pub(super) scope:             InputScope,
    pub(super) family:            RuleFamily,
}

fn input_key<'a>(
    tip: &'a RepositoryTip,
    scope: InputScope,
    path: &'a str,
    reads: &ReadScope,
) -> (InputMember, FingerprintSource<'a>) {
    if let ReadScope::Explicit(reads) = reads {
        return (
            InputMember::Explicit(path.to_owned(), reads.clone()),
            FingerprintSource::Entries,
        );
    }
    match scope {
        InputScope::File => (
            InputMember::File(path.to_owned()),
            FingerprintSource::FileBlob(path),
        ),
        InputScope::Module => (
            InputMember::Module(path.to_owned()),
            FingerprintSource::Entries,
        ),
        InputScope::Crate => {
            let member = tip.crate_of(path);
            (
                InputMember::Crate(member.to_owned()),
                FingerprintSource::CrateTree(member),
            )
        },
        InputScope::Manifest => (InputMember::Manifest, FingerprintSource::Entries),
    }
}
