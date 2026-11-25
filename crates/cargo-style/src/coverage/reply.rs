//! Count coverage and write the selected cell projection.

use std::collections::BTreeMap;

use serde::Serialize;

use super::catalog::CoverageRule;
use super::catalog::RuleInputScope;
use super::catalog::Tier;
use crate::ledger_read::LedgerRevision;

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum CellState {
    Open,
    Stale,
    Current,
    #[serde(rename = "awaiting merge")]
    AwaitingMerge,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum HeldCoverage {
    Current,
    NoCurrentHold,
}

#[derive(Default, Serialize)]
struct StateCounts {
    current:        usize,
    stale:          usize,
    open:           usize,
    awaiting_merge: usize,
    held:           usize,
}

impl StateCounts {
    fn record(&mut self, state: CellState, held: HeldCoverage) {
        match state {
            CellState::Open => self.open += 1,
            CellState::Stale => self.stale += 1,
            CellState::Current => self.current += 1,
            CellState::AwaitingMerge => self.awaiting_merge += 1,
        }
        if held == HeldCoverage::Current {
            self.held += 1;
        }
    }

    const fn add(&mut self, other: &Self) {
        self.current += other.current;
        self.stale += other.stale;
        self.open += other.open;
        self.awaiting_merge += other.awaiting_merge;
        self.held += other.held;
    }
}

#[derive(Serialize)]
pub(super) struct Cell<'a> {
    pub(super) rule:   &'a str,
    pub(super) path:   String,
    #[serde(rename = "crate")]
    pub(super) member: String,
    pub(super) tier:   Tier,
    pub(super) family: Option<&'static str>,
    pub(super) state:  CellState,
}

#[derive(Serialize)]
struct RuleCounts<'a> {
    rule_id:     &'a str,
    tier:        Tier,
    input_scope: RuleInputScope,
    #[serde(flatten)]
    counts:      StateCounts,
}

#[derive(Serialize)]
struct FamilyCounts {
    family: Option<&'static str>,
    #[serde(flatten)]
    counts: StateCounts,
}

pub(super) enum CellProjection<'a> {
    CountsOnly,
    IncludeCells(Vec<Cell<'a>>),
}

impl CellProjection<'_> {
    const fn is_counts_only(&self) -> bool { matches!(self, Self::CountsOnly) }
}

impl Serialize for CellProjection<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::IncludeCells(cells) => cells.serialize(serializer),
            Self::CountsOnly => serializer.serialize_none(),
        }
    }
}

#[derive(Serialize)]
pub(super) struct CoverageReport<'a> {
    #[serde(skip_serializing_if = "CellProjection::is_counts_only")]
    cells:    CellProjection<'a>,
    rules:    Vec<RuleCounts<'a>>,
    families: Vec<FamilyCounts>,
}

pub(super) struct CoverageTotals<'a> {
    cells:    CellProjection<'a>,
    rules:    Vec<RuleCounts<'a>>,
    families: BTreeMap<Option<&'static str>, StateCounts>,
}

pub(super) struct RuleTally<'a> {
    identity: &'a str,
    rule:     &'a CoverageRule,
    counts:   StateCounts,
}

impl<'a> RuleTally<'a> {
    pub(super) fn new(identity: &'a str, rule: &'a CoverageRule) -> Self {
        Self {
            identity,
            rule,
            counts: StateCounts::default(),
        }
    }
}

impl<'a> CoverageTotals<'a> {
    pub(super) const fn new(cells: CellProjection<'a>) -> Self {
        Self {
            cells,
            rules: Vec::new(),
            families: BTreeMap::new(),
        }
    }

    pub(super) fn record(
        &mut self,
        tally: &mut RuleTally<'a>,
        path: &str,
        member: &str,
        state: CellState,
        held: HeldCoverage,
    ) {
        tally.counts.record(state, held);
        if let CellProjection::IncludeCells(cells) = &mut self.cells {
            cells.push(Cell {
                rule: tally.identity,
                path: path.to_owned(),
                member: member.to_owned(),
                tier: tally.rule.tier,
                family: tally.rule.family_name(),
                state,
            });
        }
    }

    pub(super) fn finish_rule(&mut self, tally: RuleTally<'a>) {
        self.families
            .entry(tally.rule.family_name())
            .or_default()
            .add(&tally.counts);
        self.rules.push(RuleCounts {
            rule_id:     tally.identity,
            tier:        tally.rule.tier,
            input_scope: tally.rule.input_scope,
            counts:      tally.counts,
        });
    }

    pub(super) fn finish(self) -> CoverageReport<'a> {
        CoverageReport {
            cells:    self.cells,
            rules:    self.rules,
            families: self
                .families
                .into_iter()
                .map(|(family, counts)| FamilyCounts { family, counts })
                .collect(),
        }
    }
}

#[derive(Serialize)]
pub(super) struct Observation<'a> {
    pub(super) synced_tip:      &'a str,
    pub(super) ledger_revision: LedgerRevision,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Prerequisite {
    CandidateSites,
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(super) enum CoverageAnswer<'a> {
    Available(CoverageReport<'a>),
    Unavailable {
        prerequisite: Prerequisite,
        rules:        Vec<&'a str>,
    },
}

#[derive(Serialize)]
pub(super) struct CoverageReply<'a> {
    pub(super) schema:   u32,
    pub(super) observed: Observation<'a>,
    pub(super) coverage: CoverageAnswer<'a>,
}
