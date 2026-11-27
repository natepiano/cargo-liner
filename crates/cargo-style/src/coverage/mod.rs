//! Answer judgment coverage from one read-only ledger snapshot.

mod catalog;
mod comparison;
mod constants;
mod judgment;
mod ledger;
mod lint;
mod membership;
mod reply;

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::ffi::OsString;
use std::io;
use std::io::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;
use std::rc::Rc;
use std::string::FromUtf8Error;

use catalog::CoverageCatalog;
use catalog::CoverageRule;
use catalog::Mechanism;
use constants::CELLS_FLAG;
use constants::COVERAGE_REPLY_SCHEMA;
use constants::CRATE_FLAG;
use constants::LEDGER_FLAG;
use constants::REPO_FLAG;
use constants::RULE_FLAG;
use judgment::CellJudgmentPolicy;
use judgment::CoveredTip;
use judgment::JudgmentReader;
use ledger::CoverageLedger;
use lint::LintCoverage;
use lint::LintTool;
use membership::CellMembership;
use membership::CoverageComponent;
use reply::CellPresentation;
use reply::CellProjection;
use reply::CellState;
use reply::CoverageAnswer;
use reply::CoverageReply;
use reply::CoverageReport;
use reply::CoverageTotals;
use reply::HeldCoverage;
use reply::Observation;
use reply::RuleTally;
use rusqlite::Connection;

use crate::constants::CHECKOUT_FLAG;
use crate::constants::FINDING_RULES_FLAG;
use crate::constants::UNCOVERED;
use crate::error::ReaderError;
use crate::finding_source::FindingSource;
use crate::finding_source::FindingSourceFailure;
use crate::finding_source::TreeBlobs;
use crate::finding_source::TreeEntries;
use crate::ledger_read::LedgerReadFailure;
use crate::ledger_read::LedgerRevision;
use crate::rule_snapshot::PinComparison;
use crate::rule_snapshot::RuleSnapshot;
use crate::rule_snapshot::RuleSnapshotFailure;
use crate::sites::Catalog;
use crate::sites::CatalogFailure;
use crate::sites::CheckoutSource;
use crate::sites::SitesFailure;
use crate::sites::Tier;

enum RuleFilter {
    All,
    Named(String),
}

enum CrateFilter {
    WholeRepository,
    Member(String),
}

impl CrateFilter {
    fn includes(&self, crate_name: &str) -> bool {
        match self {
            Self::WholeRepository => true,
            Self::Member(member) => member == crate_name,
        }
    }
}

/// The evidence that gives the state of each cell of one rule.
enum CellEvidence<'a> {
    /// A tier A rule: the saved workspace lint run for its `LintTool`.
    Lint(&'a LintCoverage),
    /// Every other tier: the judgment rows in the ledger.
    Judgment,
}

#[derive(Clone, Copy)]
enum ReplyDetail {
    CountsOnly,
    WithCells,
}

struct CoverageRequest {
    repo:         PathBuf,
    ledger:       PathBuf,
    rules:        PathBuf,
    checkout:     CheckoutSource,
    rule_filter:  RuleFilter,
    crate_filter: CrateFilter,
    detail:       ReplyDetail,
}

#[derive(Debug, thiserror::Error)]
enum CoverageError {
    #[error(
        "usage: cargo style coverage --repo <path> --ledger <ledger.sqlite> --rules <rules.json> [--checkout <path>] [--rule <id>] [--crate <path>] [--cells]"
    )]
    Usage,
    #[error("unknown rule {0}")]
    UnknownRule(String),
    #[error("coverage rule snapshot lacks {0}")]
    MissingRulePolicy(String),
    #[error("coverage rule snapshot does not match the native catalog for {0}")]
    InconsistentRule(String),
    #[error(transparent)]
    Catalog(#[from] CatalogFailure),
    #[error(transparent)]
    Sites(#[from] SitesFailure),
    #[error("tier A rule {0} lacks a lint mechanism")]
    MissingMechanism(String),
    #[error("cannot open the coverage rule snapshot")]
    OpenRules(#[source] io::Error),
    #[error("cannot decode the coverage rule snapshot")]
    DecodeRules(#[source] serde_json::Error),
    #[error(transparent)]
    Ledger(#[from] LedgerReadFailure),
    #[error(transparent)]
    Source(#[from] FindingSourceFailure),
    #[error(transparent)]
    Repository(#[from] ReaderError),
    #[error(transparent)]
    Pins(#[from] RuleSnapshotFailure),
    #[error("cannot resolve the stored style branch {0}")]
    MissingBranch(String),
    #[error("cannot resolve the removal commit {0}")]
    MissingRemovalCommit(String),
    #[error("cannot resolve judgment commit {0} while checking removal history")]
    MissingHistoryAfterRemoval(String),
    #[error("invalid lint_runs.{column}")]
    LintJson {
        column: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("the lint manifest at {0} is not UTF-8")]
    UnreadableLintManifest(String),
    #[error("cannot parse the lint manifest at {path}")]
    LintManifest {
        path:   String,
        #[source]
        source: Box<toml::de::Error>,
    },
    #[error("cannot read the global lint configuration at {path:?}")]
    GlobalLintConfig {
        path:   PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("cannot encode the lint configuration as UTF-8")]
    LintEncoding(#[source] FromUtf8Error),
    #[error("cannot encode the lint configuration digest")]
    LintDigestEncoding(#[source] std::fmt::Error),
    #[error("cannot encode coverage data")]
    Encode(#[from] serde_json::Error),
    #[error("cannot write the coverage reply")]
    Output(#[source] io::Error),
}

impl From<rusqlite::Error> for CoverageError {
    fn from(error: rusqlite::Error) -> Self { LedgerReadFailure::from(error).into() }
}

impl CoverageRequest {
    fn parse(mut arguments: impl Iterator<Item = OsString>) -> Result<Self, CoverageError> {
        let mut repo = None;
        let mut ledger = None;
        let mut rules = None;
        let mut checkout = CheckoutSource::Repository;
        let mut rule_filter = RuleFilter::All;
        let mut crate_filter = CrateFilter::WholeRepository;
        let mut detail = ReplyDetail::CountsOnly;
        while let Some(flag) = arguments.next() {
            match flag.to_str() {
                Some(REPO_FLAG) if repo.is_none() => repo = arguments.next().map(PathBuf::from),
                Some(LEDGER_FLAG) if ledger.is_none() => {
                    ledger = arguments.next().map(PathBuf::from);
                },
                Some(FINDING_RULES_FLAG) if rules.is_none() => {
                    rules = arguments.next().map(PathBuf::from);
                },
                Some(CHECKOUT_FLAG) if matches!(checkout, CheckoutSource::Repository) => {
                    checkout = CheckoutSource::Explicit(
                        arguments.next().ok_or(CoverageError::Usage)?.into(),
                    );
                },
                Some(RULE_FLAG) if matches!(rule_filter, RuleFilter::All) => {
                    rule_filter = RuleFilter::Named(read_text_argument(arguments.next())?);
                },
                Some(CRATE_FLAG) if matches!(crate_filter, CrateFilter::WholeRepository) => {
                    crate_filter = CrateFilter::Member(read_text_argument(arguments.next())?);
                },
                Some(CELLS_FLAG) if matches!(detail, ReplyDetail::CountsOnly) => {
                    detail = ReplyDetail::WithCells;
                },
                _ => return Err(CoverageError::Usage),
            }
        }
        let (Some(repo), Some(ledger), Some(rules)) = (repo, ledger, rules) else {
            return Err(CoverageError::Usage);
        };
        Ok(Self {
            repo,
            ledger,
            rules,
            checkout,
            rule_filter,
            crate_filter,
            detail,
        })
    }

    fn answer(self) -> Result<(), CoverageError> {
        let (catalog, pins) = CoverageCatalog::read(&self.rules)?;
        let native = Catalog::read(self.checkout.path(&self.repo))?;
        let json = ledger::read_snapshot(&self.ledger, |ledger| {
            self.reply(ledger, &catalog, &native, &pins)
        })?;
        let mut stdout = io::stdout().lock();
        stdout
            .write_all(&json)
            .and_then(|()| stdout.write_all(b"\n"))
            .map_err(CoverageError::Output)
    }

    fn selected_rules<'a>(
        &self,
        catalog: &'a CoverageCatalog,
        native: &Catalog,
    ) -> Result<Vec<(&'a str, &'a CoverageRule)>, CoverageError> {
        match &self.rule_filter {
            RuleFilter::All => {
                if let Some(rule) = native
                    .classified_rules()
                    .find(|rule| !catalog.rules.contains_key(&rule.id))
                {
                    return Err(CoverageError::MissingRulePolicy(rule.id.clone()));
                }
                let mut selected = Vec::new();
                for (identity, rule) in &catalog.rules {
                    rule.check_native(identity, native.rule(identity)?)?;
                    if rule.policy.tier != Tier::Unclassified {
                        selected.push((identity.as_str(), rule));
                    }
                }
                Ok(selected)
            },
            RuleFilter::Named(requested) => {
                let base = requested.strip_suffix(UNCOVERED).unwrap_or(requested);
                let current = native.rule(base)?;
                let (identity, rule) = catalog
                    .rules
                    .get_key_value(base)
                    .ok_or_else(|| CoverageError::MissingRulePolicy(base.to_owned()))?;
                rule.check_native(identity, current)?;
                if requested.ends_with(UNCOVERED) && !rule.has_uncovered() {
                    return Err(CoverageError::UnknownRule(requested.clone()));
                }
                Ok(if rule.policy.tier == Tier::Unclassified {
                    Vec::new()
                } else {
                    vec![(identity.as_str(), rule)]
                })
            },
        }
    }

    fn reply(
        &self,
        ledger: CoverageLedger<'_>,
        catalog: &CoverageCatalog,
        native: &Catalog,
        pins: &RuleSnapshot,
    ) -> Result<Vec<u8>, CoverageError> {
        let synced_tip = ledger.sync.commit.clone();
        let coverage =
            CoverageAnswer::Available(self.report(ledger, &synced_tip, catalog, native, pins)?);
        Ok(serde_json::to_vec(&CoverageReply {
            schema: COVERAGE_REPLY_SCHEMA,
            observed: Observation {
                synced_tip:      &synced_tip,
                ledger_revision: LedgerRevision::Absent,
            },
            coverage,
        })?)
    }

    fn report<'a>(
        &self,
        mut ledger: CoverageLedger<'_>,
        synced_tip: &str,
        catalog: &'a CoverageCatalog,
        native: &Catalog,
        pins: &RuleSnapshot,
    ) -> Result<CoverageReport<'a>, CoverageError> {
        let mut source = FindingSource::open(&self.repo)?;
        let tip = source.tip(synced_tip)?;
        let sites =
            native.membership(&self.repo, ledger.connection, synced_tip, &mut source, &tip)?;
        let membership = CellMembership {
            tip:   &tip,
            sites: &sites,
        };
        let rules = self.selected_rules(catalog, native)?;
        let TreeBlobs::Listed(entries) = source.tree(synced_tip)? else {
            return Err(FindingSourceFailure::UndecodableSyncedTree(synced_tip.to_owned()).into());
        };
        let entries = Rc::clone(entries);
        let branch = ledger.sync.style_branch.clone();
        let mut judgments = JudgmentReader::new(
            CoveredTip {
                commit: synced_tip,
                tip:    &tip,
                branch: &branch,
            },
            pins,
        );
        let mut totals = CoverageTotals::new(match self.detail {
            ReplyDetail::CountsOnly => CellProjection::CountsOnly,
            ReplyDetail::WithCells => CellProjection::IncludeCells(Vec::new()),
        });
        let mut lints = BTreeMap::new();
        let tree = if rules.iter().any(|(_, rule)| rule.policy.tier == Tier::A) {
            lint::tree_at(&self.repo, synced_tip)?
        } else {
            String::new()
        };
        for (identity, rule) in rules {
            let mut tally = RuleTally::new(identity, rule);
            let cell_evidence = read_cell_evidence(
                &mut lints,
                &mut source,
                ledger.connection,
                &entries,
                &tree,
                (identity, rule),
            )?;
            let discoveries: Vec<_> = native.discoveries(identity).collect();
            let base = CoverageComponent::base(identity, rule);
            let uncovered = rule
                .has_uncovered()
                .then(|| CoverageComponent::uncovered(identity));
            for component in std::iter::once(base).chain(uncovered) {
                let presentation = match &self.rule_filter {
                    RuleFilter::Named(id) if id.ends_with(UNCOVERED) && id != component.id() => {
                        CellPresentation::CountOnly
                    },
                    _ => CellPresentation::Show,
                };
                let paths = discoveries.iter().flat_map(|definition| {
                    membership.paths(&component, &definition.reach, rule.policy.input_scope)
                });
                for path in paths {
                    if !self.crate_filter.includes(path.member) {
                        continue;
                    }
                    let (state, held) = match cell_evidence {
                        CellEvidence::Lint(lint) => (
                            lint_cell(lint, path.path, identity, pins)?,
                            HeldCoverage::NoCurrentHold,
                        ),
                        CellEvidence::Judgment => {
                            let policy = CellJudgmentPolicy {
                                identity: component.id(),
                                scope:    path
                                    .judgment_scope(component.tier(), rule.policy.input_scope),
                                family:   rule.policy.family,
                            };
                            judgments.cell(&mut ledger, &mut source, policy, path.path)?
                        },
                    };
                    totals.record(&mut tally, &component, path, state, held, presentation);
                }
            }
            totals.finish_rule(tally, discoveries.len());
        }
        Ok(totals.finish())
    }
}

fn read_text_argument(argument: Option<OsString>) -> Result<String, CoverageError> {
    argument
        .and_then(|argument| argument.into_string().ok())
        .filter(|argument| !argument.is_empty() && !argument.starts_with('-'))
        .ok_or(CoverageError::Usage)
}

fn read_cell_evidence<'a>(
    lints: &'a mut BTreeMap<LintTool, LintCoverage>,
    source: &mut FindingSource<'_>,
    connection: &Connection,
    entries: &TreeEntries,
    tree: &str,
    (identity, rule): (&str, &CoverageRule),
) -> Result<CellEvidence<'a>, CoverageError> {
    if rule.policy.tier != Tier::A {
        return Ok(CellEvidence::Judgment);
    }
    let Mechanism::Lint(tool) = rule.mechanism else {
        return Err(CoverageError::MissingMechanism(identity.to_owned()));
    };
    let lint = match lints.entry(tool) {
        Entry::Occupied(entry) => entry.into_mut(),
        Entry::Vacant(entry) => {
            let digest = lint::config_digest(source, entries, tool)?;
            entry.insert(LintCoverage::read(connection, tool, tree, &digest)?)
        },
    };
    Ok(CellEvidence::Lint(lint))
}

fn lint_cell(
    coverage: &LintCoverage,
    path: &str,
    identity: &str,
    rules: &RuleSnapshot,
) -> Result<CellState, CoverageError> {
    Ok(match coverage {
        LintCoverage::NeverRun => CellState::Open,
        LintCoverage::Passed { diagnostics, pins }
            if diagnostics.get(path).copied().unwrap_or_default() == 0 =>
        {
            if let Some(pin) = pins.get(identity).filter(|pin| !pin.is_empty())
                && matches!(
                    rules.compares_pins(identity, "", pin)?,
                    PinComparison::Matches
                )
            {
                CellState::Current
            } else {
                CellState::Stale
            }
        },
        LintCoverage::Passed { .. } | LintCoverage::Stale => CellState::Stale,
    })
}

pub(crate) fn run(arguments: impl Iterator<Item = OsString>) -> ExitCode {
    match CoverageRequest::parse(arguments).and_then(CoverageRequest::answer) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!(
                "cargo-style: {}",
                crate::error_chain(&error).replace(['\r', '\n'], " ")
            );
            ExitCode::FAILURE
        },
    }
}
