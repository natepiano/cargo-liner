//! Find the judgment that covers a cell at the synced tip.

use super::CoverageError;
use super::comparison::InputComparison;
use super::comparison::JudgmentComparison;
use super::ledger::CoverageLedger;
use super::ledger::DeclaredScope;
use super::ledger::Judgment;
use super::ledger::PriorGeneration;
use super::ledger::StyleBranch;
use super::ledger::Verdict;
use super::reply::CellState;
use super::reply::HeldCoverage;
use crate::finding_source::CommitResolution;
use crate::finding_source::FindingSource;
use crate::rule_snapshot::PinComparison;
use crate::rule_snapshot::RuleFamily;
use crate::rule_snapshot::RuleSnapshot;
use crate::site_ledger::InputScope;
use crate::tip_view::RepositoryTip;

enum NonCurrentCoverage {
    Open,
    Stale,
    AwaitingMerge,
}

enum HistoricalCommit {
    Present(String),
    Absent,
}

pub(super) struct CoveredTip<'a> {
    pub(super) commit: &'a str,
    pub(super) tip:    &'a RepositoryTip,
    pub(super) branch: &'a StyleBranch,
}

pub(super) struct JudgmentReader<'a> {
    covered:    CoveredTip<'a>,
    pins:       &'a RuleSnapshot,
    comparison: InputComparison,
}

#[derive(Clone, Copy)]
pub(super) struct CellJudgmentPolicy<'a> {
    pub(super) identity: &'a str,
    pub(super) scope:    InputScope,
    pub(super) family:   RuleFamily,
}

impl<'a> JudgmentReader<'a> {
    pub(super) fn new(covered: CoveredTip<'a>, pins: &'a RuleSnapshot) -> Self {
        Self {
            covered,
            pins,
            comparison: InputComparison::default(),
        }
    }

    pub(super) fn cell(
        &mut self,
        ledger: &mut CoverageLedger<'_>,
        source: &mut FindingSource<'_>,
        policy: CellJudgmentPolicy<'_>,
        path: &str,
    ) -> Result<(CellState, HeldCoverage), CoverageError> {
        let CellJudgmentPolicy {
            identity,
            scope,
            family,
        } = policy;
        let rows = ledger.judgments(identity, path)?;
        let removals = self.removals(source, &rows)?;
        let rows = surviving_rows(source, &rows, &removals)?;
        let mut state = if rows.is_empty()
            && matches!(
                ledger.prior_generation(identity, path)?,
                PriorGeneration::InactiveJudgment
            ) {
            NonCurrentCoverage::Stale
        } else {
            NonCurrentCoverage::Open
        };
        let declared = rows
            .iter()
            .find(|row| row.verdict != Verdict::Removed)
            .map_or(DeclaredScope::Rule, |row| row.declared);
        for row in rows {
            if row.verdict == Verdict::Removed {
                if !matches!(state, NonCurrentCoverage::AwaitingMerge)
                    && branch_removes(source, self.covered.branch, row, path)?
                {
                    state = NonCurrentCoverage::AwaitingMerge;
                }
                continue;
            }
            if matches!(state, NonCurrentCoverage::Open) {
                state = NonCurrentCoverage::Stale;
            }
            if row.declared != declared
                || !matches!(
                    self.pins.compares_pins(identity, "", &row.pin)?,
                    PinComparison::Matches
                )
            {
                continue;
            }
            let HistoricalCommit::Present(judged) = historical_commit(source, &row.commit)? else {
                continue;
            };
            if source.is_ancestor(&judged, self.covered.commit)? {
                let comparison = JudgmentComparison {
                    judgment: row,
                    historical_commit: &judged,
                    covered_commit: self.covered.commit,
                    tip: self.covered.tip,
                    scope,
                    family,
                };
                if self.comparison.matches(source, path, comparison)? {
                    let held = if row.verdict == Verdict::Held {
                        HeldCoverage::Current
                    } else {
                        HeldCoverage::NoCurrentHold
                    };
                    return Ok((CellState::Current, held));
                }
            } else if let StyleBranch::Saved(branch) = self.covered.branch {
                let branch = branch_commit(source, branch)?;
                let tip = source.tip(&branch)?;
                let comparison = JudgmentComparison {
                    judgment: row,
                    historical_commit: &judged,
                    covered_commit: &branch,
                    tip: &tip,
                    scope,
                    family,
                };
                if source.is_ancestor(&judged, &branch)?
                    && self.comparison.matches(source, path, comparison)?
                {
                    state = NonCurrentCoverage::AwaitingMerge;
                }
            }
        }
        let cell = match state {
            NonCurrentCoverage::Open => CellState::Open,
            NonCurrentCoverage::Stale => CellState::Stale,
            NonCurrentCoverage::AwaitingMerge => CellState::AwaitingMerge,
        };
        Ok((cell, HeldCoverage::NoCurrentHold))
    }

    fn removals(
        &self,
        source: &mut FindingSource<'_>,
        rows: &[Judgment],
    ) -> Result<Vec<String>, CoverageError> {
        let mut commits = Vec::new();
        for row in rows.iter().filter(|row| row.verdict == Verdict::Removed) {
            let HistoricalCommit::Present(commit) = historical_commit(source, &row.commit)? else {
                return Err(CoverageError::MissingRemovalCommit(row.commit.clone()));
            };
            if source.is_ancestor(&commit, self.covered.commit)? && !commits.contains(&commit) {
                commits.push(commit);
            }
        }
        Ok(commits)
    }
}

fn surviving_rows<'a>(
    source: &mut FindingSource<'_>,
    rows: &'a [Judgment],
    removals: &[String],
) -> Result<Vec<&'a Judgment>, CoverageError> {
    if removals.is_empty() {
        return Ok(rows.iter().collect());
    }
    let mut retained = Vec::new();
    'row: for row in rows {
        let HistoricalCommit::Present(commit) = historical_commit(source, &row.commit)? else {
            return Err(CoverageError::MissingHistoryAfterRemoval(
                row.commit.clone(),
            ));
        };
        if row.verdict == Verdict::Removed && removals.contains(&commit) {
            continue;
        }
        for removal in removals {
            if !source.is_ancestor(removal, &commit)? {
                continue 'row;
            }
        }
        retained.push(row);
    }
    Ok(retained)
}

fn branch_removes(
    source: &mut FindingSource<'_>,
    branch: &StyleBranch,
    row: &Judgment,
    path: &str,
) -> Result<bool, CoverageError> {
    let StyleBranch::Saved(branch) = branch else {
        return Ok(false);
    };
    let HistoricalCommit::Present(judged) = historical_commit(source, &row.commit)? else {
        return Ok(false);
    };
    let branch = branch_commit(source, branch)?;
    if !source.is_ancestor(&judged, &branch)? {
        return Ok(false);
    }
    Ok(source.tip(&branch)?.entry(path).is_none())
}

fn branch_commit(source: &mut FindingSource<'_>, branch: &str) -> Result<String, CoverageError> {
    let CommitResolution::Resolved(commit) = source.resolve_commit(branch)? else {
        return Err(CoverageError::MissingBranch(branch.to_owned()));
    };
    Ok(commit.clone())
}

fn historical_commit(
    source: &mut FindingSource<'_>,
    commit: &str,
) -> Result<HistoricalCommit, CoverageError> {
    Ok(match source.resolve_commit(commit)? {
        CommitResolution::Resolved(commit) => HistoricalCommit::Present(commit.clone()),
        CommitResolution::Unresolved => HistoricalCommit::Absent,
    })
}
