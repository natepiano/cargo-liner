//! Read a finding's declaration at the synced tip.

use std::cell::OnceCell;

use serde::Serialize;

use super::approvals::ApprovalSnapshot;
use super::approvals::ReadAnswer;
use super::approvals::ReadAnswers;
use super::branches::Branches;
use super::branches::Stop;
use super::built;
use super::built::BuiltFailure;
use super::built::BuiltRename;
use super::outcome::DeclarationOutcome;
use super::outcome::FilingEvidence;
use super::outcome::OutsideEvidence;
use crate::finding_anchor::FindingAnchor;
use crate::finding_source::FindingSource;
use crate::finding_source::FindingSourceFailure;
use crate::ledger_read::BuiltRow;
use crate::ledger_read::HeldSite;
use crate::original_site::OriginalSite;
use crate::outside_rename::OutsideReader;
use crate::outside_rename::OutsideRename;
use crate::outside_rename::OutsideSourceFailure;
use crate::rename_applied::AppliedError;
use crate::same_declaration::InPlaceBasis;
use crate::same_declaration::NotReadCause;
use crate::same_declaration::RelocationReader;
use crate::same_declaration::SameDeclaration;

/// One reply's approvals and built findings.
pub(crate) struct DeclarationReader {
    approvals: ApprovalSnapshot,
    rows:      Vec<BuiltRow>,
    /// The built renames, read at the first finding that needs them.
    renames:   OnceCell<Vec<BuiltRename>>,
}

/// The proofs of one finding that its declaration reading starts from.
pub(crate) struct DeclarationInput<'a> {
    pub(crate) held:          &'a HeldSite,
    pub(crate) anchor:        &'a FindingAnchor,
    pub(crate) original_site: &'a OriginalSite,
    pub(crate) same:          &'a SameDeclaration,
}

#[derive(Serialize)]
pub(crate) struct DeclarationAnswer {
    declaration:   DeclarationOutcome,
    site_evidence: FilingEvidence,
    approvals:     Vec<ReadAnswer>,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum DeclarationFailure {
    #[error(transparent)]
    Source(FindingSourceFailure),
    #[error(transparent)]
    Applied(AppliedError),
    #[error(transparent)]
    Outside(#[from] OutsideSourceFailure),
    #[error("cannot read built finding {id} at {path} against {tip}")]
    Built {
        id:     String,
        path:   String,
        tip:    String,
        source: Box<FindingSourceFailure>,
    },
}

impl DeclarationReader {
    pub(crate) const fn new(approvals: ApprovalSnapshot, rows: Vec<BuiltRow>) -> Self {
        Self {
            approvals,
            rows,
            renames: OnceCell::new(),
        }
    }

    pub(crate) fn read(
        &mut self,
        source: &mut FindingSource<'_>,
        relocation: &mut RelocationReader,
        outside: &mut OutsideReader,
        tip: &str,
        input: &DeclarationInput<'_>,
    ) -> Result<(OutsideRename, DeclarationAnswer), DeclarationFailure> {
        let mut answers = ReadAnswers::default();
        let (outside, outcome) =
            self.outcome(source, relocation, outside, tip, input, &mut answers)?;
        let declaration = match outcome {
            Ok(outcome) => outcome,
            Err(Stop::Unobserved(path)) => DeclarationOutcome::ApprovalUnobserved { path },
            Err(Stop::Source(failure)) => return Err(DeclarationFailure::Source(failure)),
            Err(Stop::Applied(failure)) => return Err(DeclarationFailure::Applied(failure)),
            Err(Stop::Built(BuiltFailure { id, path, source })) => {
                return Err(DeclarationFailure::Built {
                    id,
                    path,
                    tip: tip.to_owned(),
                    source: Box::new(source),
                });
            },
        };
        let site_evidence = FilingEvidence::from((&declaration, input));
        Ok((
            outside,
            DeclarationAnswer {
                declaration,
                site_evidence,
                approvals: answers.into_answers(),
            },
        ))
    }

    fn outcome(
        &mut self,
        source: &mut FindingSource<'_>,
        relocation: &mut RelocationReader,
        outside: &mut OutsideReader,
        tip: &str,
        input: &DeclarationInput<'_>,
        answers: &mut ReadAnswers,
    ) -> Result<(OutsideRename, Result<DeclarationOutcome, Stop>), OutsideSourceFailure> {
        let not_proven = OutsideEvidence::OutsideRenameNotProven;
        let finding = &input.held.finding;
        let declaration = match input.same {
            SameDeclaration::NotRead {
                cause: NotReadCause::Unread,
            } => DeclarationOutcome::NotRead,
            SameDeclaration::NotRead {
                cause: NotReadCause::UnnamedLine,
            } => after_refusal(DeclarationOutcome::NotRead, input.original_site),
            SameDeclaration::InPlace {
                basis: InPlaceBasis::UnchangedPath,
            } => DeclarationOutcome::InPlace,
            SameDeclaration::InPlace {
                basis: InPlaceBasis::MatchingItem,
            } => after_refusal(DeclarationOutcome::InPlace, input.original_site),
            SameDeclaration::Moved {
                declaration,
                path,
                line,
            } => DeclarationOutcome::Moved {
                declaration: declaration.clone(),
                path:        path.clone(),
                line:        *line,
            },
            SameDeclaration::SeveralDestinations { declaration, paths } => {
                DeclarationOutcome::SeveralFiles {
                    declaration: declaration.clone(),
                    paths:       paths.clone(),
                    evidence:    not_proven,
                }
            },
            SameDeclaration::AnotherCrate { declaration, path } => {
                DeclarationOutcome::AnotherCrate {
                    declaration: declaration.clone(),
                    path:        path.clone(),
                    evidence:    not_proven,
                }
            },
            SameDeclaration::TraitIdentityUncertain { declaration } => {
                DeclarationOutcome::TraitIdentityUncertain {
                    declaration: declaration.clone(),
                }
            },
            SameDeclaration::OpenBuild { ticket } => DeclarationOutcome::KeptForOpenBuild {
                ticket: ticket.clone(),
            },
            SameDeclaration::StatementNotFollowed {
                declaration,
                path,
                reason,
            } => DeclarationOutcome::StatementNotFollowed {
                declaration: declaration.clone(),
                path:        path.clone(),
                reason:      *reason,
            },
            SameDeclaration::NoDestination {
                declaration,
                original,
            } => {
                let proof =
                    outside.prove_relocated(source, relocation, tip, &finding.anchor, original)?;
                let read = original.read();
                let outcome = (|| {
                    let Self {
                        approvals,
                        rows,
                        renames,
                    } = self;
                    let renames = read_renames(renames, rows, source, relocation)?;
                    let branches = Branches {
                        tip: source.tip(tip)?,
                        changed: source.changed_paths(read, tip)?,
                        source,
                        relocation,
                        approvals,
                        answers,
                        renames,
                        rows,
                        finding,
                        state: &input.held.relocation.finding,
                        read,
                        tip_commit: tip,
                    };
                    branches.outcome(original.item(), declaration, &proof)
                })();
                return Ok((OutsideRename::Read(Box::new(proof)), outcome));
            },
        };
        let proof = outside.prove(
            source,
            relocation,
            tip,
            &finding.anchor,
            input.anchor,
            input.same,
        )?;
        Ok((proof, Ok(declaration)))
    }
}

fn read_renames<'r>(
    renames: &'r OnceCell<Vec<BuiltRename>>,
    rows: &[BuiltRow],
    source: &mut FindingSource<'_>,
    relocation: &mut RelocationReader,
) -> Result<&'r [BuiltRename], Stop> {
    if let Some(read) = renames.get() {
        return Ok(read);
    }
    let (declarations, _) = relocation.source_readers();
    let read = built::read_renames(rows, source, declarations).map_err(Stop::Built)?;
    Ok(renames.get_or_init(|| read))
}

/// A refused declaration proof keeps the finding's original site when that site is unchanged.
fn after_refusal(outcome: DeclarationOutcome, site: &OriginalSite) -> DeclarationOutcome {
    if matches!(site, OriginalSite::Unchanged { .. }) {
        return DeclarationOutcome::FindingSite;
    }
    outcome
}
