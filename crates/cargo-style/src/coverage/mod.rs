//! Answer judgment coverage from one read-only ledger snapshot.

mod catalog;
mod comparison;
mod constants;
mod judgment;
mod ledger;
mod lint;
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
use catalog::RuleReach;
use catalog::Tier;
use constants::CELLS_FLAG;
use constants::COVERAGE_REPLY_SCHEMA;
use constants::CRATE_FLAG;
use constants::LEDGER_FLAG;
use constants::REPO_FLAG;
use constants::RULE_FLAG;
use judgment::CoveredTip;
use judgment::JudgmentReader;
use ledger::CoverageLedger;
use lint::LintCoverage;
use lint::LintTool;
use reply::CellProjection;
use reply::CellState;
use reply::CoverageAnswer;
use reply::CoverageReply;
use reply::CoverageReport;
use reply::CoverageTotals;
use reply::HeldCoverage;
use reply::Observation;
use reply::Prerequisite;
use reply::RuleTally;

use crate::constants::FINDING_RULES_FLAG;
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
use crate::tip_view::RepositoryTip;
use crate::tip_view::under;

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

/// A Rust file at the synced tip, with the crate that holds it.
struct RustFile<'a> {
    path:   &'a str,
    member: &'a str,
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
    rule_filter:  RuleFilter,
    crate_filter: CrateFilter,
    detail:       ReplyDetail,
}

#[derive(Debug, thiserror::Error)]
enum CoverageError {
    #[error(
        "usage: cargo style coverage --repo <path> --ledger <ledger.sqlite> --rules <rules.json> [--rule <id>] [--crate <path>] [--cells]"
    )]
    Usage,
    #[error("unknown rule {0}")]
    UnknownRule(String),
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
            rule_filter,
            crate_filter,
            detail,
        })
    }

    fn answer(self) -> Result<(), CoverageError> {
        let (catalog, pins) = CoverageCatalog::read(&self.rules)?;
        let rules = self.selected_rules(&catalog)?;
        let json = ledger::read_snapshot(&self.ledger, |ledger| self.reply(ledger, &rules, &pins))?;
        let mut stdout = io::stdout().lock();
        stdout
            .write_all(&json)
            .and_then(|()| stdout.write_all(b"\n"))
            .map_err(CoverageError::Output)
    }

    fn selected_rules<'a>(
        &self,
        catalog: &'a CoverageCatalog,
    ) -> Result<Vec<(&'a str, &'a CoverageRule)>, CoverageError> {
        match &self.rule_filter {
            RuleFilter::All => Ok(catalog
                .rules
                .iter()
                .filter(|(_, rule)| rule.tier != Tier::Unclassified)
                .map(|(identity, rule)| (identity.as_str(), rule))
                .collect()),
            RuleFilter::Named(identity) => {
                let (identity, rule) = catalog
                    .rules
                    .get_key_value(identity)
                    .ok_or_else(|| CoverageError::UnknownRule(identity.clone()))?;
                Ok(if rule.tier == Tier::Unclassified {
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
        rules: &[(&str, &CoverageRule)],
        pins: &RuleSnapshot,
    ) -> Result<Vec<u8>, CoverageError> {
        let synced_tip = ledger.sync.commit.clone();
        let unavailable: Vec<_> = rules
            .iter()
            .filter_map(|&(identity, rule)| {
                matches!(rule.tier, Tier::B | Tier::D).then_some(identity)
            })
            .collect();
        let coverage = if unavailable.is_empty() {
            CoverageAnswer::Available(self.report(ledger, &synced_tip, rules, pins)?)
        } else {
            CoverageAnswer::Unavailable {
                prerequisite: Prerequisite::CandidateSites,
                rules:        unavailable,
            }
        };
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
        rules: &[(&'a str, &'a CoverageRule)],
        pins: &RuleSnapshot,
    ) -> Result<CoverageReport<'a>, CoverageError> {
        let mut source = FindingSource::open(&self.repo)?;
        let tip = source.tip(synced_tip)?;
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
        let tree = if rules.iter().any(|(_, rule)| rule.tier == Tier::A) {
            lint::tree_at(&self.repo, synced_tip)?
        } else {
            String::new()
        };
        let rust_files = self.rust_files(&tip, &entries);
        for &(identity, rule) in rules {
            let mut tally = RuleTally::new(identity, rule);
            let cell_evidence = read_cell_evidence(
                &mut lints,
                &mut source,
                ledger.connection,
                &entries,
                &tree,
                (identity, rule),
            )?;
            for file in rule_files(rule, &rust_files) {
                let (state, held) = match cell_evidence {
                    CellEvidence::Lint(lint) => (
                        lint_cell(lint, file.path, identity, pins)?,
                        HeldCoverage::NoCurrentHold,
                    ),
                    CellEvidence::Judgment => {
                        judgments.cell(&mut ledger, &mut source, (identity, rule), file.path)?
                    },
                };
                totals.record(&mut tally, file.path, file.member, state, held);
            }
            totals.finish_rule(tally);
        }
        Ok(totals.finish())
    }

    /// The Rust files at the synced tip in the requested crate scope, in tree order.
    fn rust_files<'a>(
        &self,
        tip: &'a RepositoryTip,
        entries: &'a TreeEntries,
    ) -> Vec<RustFile<'a>> {
        entries
            .keys()
            .filter(|path| tip.has_rust_path(path))
            .filter_map(|path| {
                let member = tip.crate_of(path);
                self.crate_filter.includes(member).then_some(RustFile {
                    path: path.as_str(),
                    member,
                })
            })
            .collect()
    }
}

/// The files in `rust_files` that are cells of the rule. Tier A and C rules start from the Rust
/// files in the rule's reach; tiers B, D and E have no Rust file cells.
fn rule_files<'f, 'a>(
    rule: &'f CoverageRule,
    rust_files: &'f [RustFile<'a>],
) -> impl Iterator<Item = &'f RustFile<'a>> {
    let rust_files = if matches!(rule.tier, Tier::A | Tier::C) {
        rust_files
    } else {
        &[]
    };
    rust_files.iter().filter(move |file| match &rule.reach {
        RuleReach::Repository => true,
        RuleReach::Member(directory) => under(file.path, directory),
    })
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
    connection: &rusqlite::Connection,
    entries: &TreeEntries,
    tree: &str,
    (identity, rule): (&str, &CoverageRule),
) -> Result<CellEvidence<'a>, CoverageError> {
    if rule.tier != Tier::A {
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
