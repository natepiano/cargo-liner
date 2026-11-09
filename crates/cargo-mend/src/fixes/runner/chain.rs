use std::borrow::Cow;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;
use std::time::Instant;

use anyhow::Result;

use super::FixScans;
use super::MendRunner;
use super::RunPlan;
use super::function_length::FunctionLengthPlan;
use super::session_snapshot::SessionSnapshot;
use crate::compiler::BuildOutputMode;
use crate::compiler::SelectionResult;
use crate::config::DiagnosticCode;
use crate::config::FixKind;
use crate::fixes::FIX_CONVERGENCE_MAX_PASSES;
use crate::fixes::imports;
use crate::fixes::imports::UseFix;
use crate::fixes::imports::ValidatedFixSet;
use crate::fixes::subtree_reexport;
use crate::fixes::subtree_reexport::ForwardingChains;
use crate::reporting::AppliedFixCounts;
use crate::reporting::CompilerFailureCause;
use crate::reporting::MendFailure;
use crate::reporting::Report;
use crate::reporting::SubtreeReexportFixFact;
use crate::rust_syntax::ParsedSources;

/// A chain proposal keeps its edit count separate from its composed file writes.
enum ChainBatch {
    Unchanged,
    Planned {
        fixes:  ValidatedFixSet,
        counts: AppliedFixCounts,
        steps:  Vec<ChainStep>,
    },
}

/// Compiler error files select a reduced batch or individual validation.
pub(super) enum ChainValidation {
    Batch,
    PrunedBatch {
        declined: BTreeSet<ChainLinkIdentity>,
    },
    IndividualLinks {
        declined:   BTreeSet<ChainLinkIdentity>,
        pass_limit: usize,
    },
}

enum BatchRetry {
    RemainingLinks,
    IndividualLinks,
}

enum ChainExtension {
    OriginalPass,
    AdditionalLinks,
}

enum StepAttribution {
    SingleBinding(ChainLinkIdentity),
    SharedBindings,
}

/// A planning pass retains its changed files and its binding attribution.
/// A pass with multiple bindings cannot attribute a file error to one binding.
struct ChainStep {
    paths:       BTreeSet<PathBuf>,
    attribution: StepAttribution,
}

impl ChainStep {
    fn record(fixes: &ValidatedFixSet, report: &Report, root: &Path) -> Self {
        let paths = fixes
            .iter()
            .map(|fix| fix.path.clone())
            .collect::<BTreeSet<_>>();
        let links = report
            .facts
            .subtree_reexport_fix_facts
            .iter()
            .filter(|fact| {
                let file = root.join(&fact.use_path);
                paths.contains(&std::fs::canonicalize(&file).unwrap_or(file))
            })
            .map(ChainLinkIdentity::from)
            .collect::<Vec<_>>();
        let attribution = match links.as_slice() {
            [link] => StepAttribution::SingleBinding(link.clone()),
            _ => StepAttribution::SharedBindings,
        };
        Self { paths, attribution }
    }
}

fn links_in_error_files(steps: &[ChainStep], failure: &MendFailure) -> BTreeSet<ChainLinkIdentity> {
    let MendFailure::Analysis(failure) = failure else {
        return BTreeSet::new();
    };
    let CompilerFailureCause::CargoCheck { error_files } = &failure.cause else {
        return BTreeSet::new();
    };
    let planned = steps
        .iter()
        .flat_map(|step| &step.paths)
        .collect::<BTreeSet<_>>();
    let mut located = BTreeSet::new();
    for file in error_files {
        let canonical = std::fs::canonicalize(file).unwrap_or_else(|_| file.clone());
        let matches = planned
            .iter()
            .filter(|path| {
                if file.is_absolute() {
                    **path == &canonical
                } else {
                    path.ends_with(file)
                }
            })
            .collect::<Vec<_>>();
        if let [path] = matches.as_slice() {
            located.insert((*(*path)).clone());
        }
    }
    let mut rejected = BTreeSet::new();
    for step in steps
        .iter()
        .filter(|step| !step.paths.is_disjoint(&located))
    {
        match &step.attribution {
            StepAttribution::SingleBinding(link) => {
                rejected.insert(link.clone());
            },
            StepAttribution::SharedBindings => return BTreeSet::new(),
        }
    }
    rejected
}

/// A refused binding keeps its identity when preceding edits change line numbers.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct ChainLinkIdentity {
    file:       String,
    owner:      Vec<String>,
    name:       String,
    crate_root: String,
    source:     Vec<String>,
}

impl From<&SubtreeReexportFixFact> for ChainLinkIdentity {
    fn from(fact: &SubtreeReexportFixFact) -> Self {
        Self {
            file:       fact.use_path.clone(),
            owner:      fact.owner_module.clone(),
            name:       fact.exported_name.clone(),
            crate_root: fact.crate_root_file.clone(),
            source:     fact.source_module.clone(),
        }
    }
}

impl ChainValidation {
    pub(super) fn next_links(&self, report: &Report) -> Vec<ChainLinkIdentity> {
        let Self::IndividualLinks { declined, .. } = self else {
            return Vec::new();
        };
        let facts = &report.facts.subtree_reexport_fix_facts;
        let Some(first) = facts
            .iter()
            .find(|fact| !declined.contains(&ChainLinkIdentity::from(*fact)))
        else {
            return Vec::new();
        };
        facts
            .iter()
            .filter(|fact| {
                fact.use_path == first.use_path
                    && fact.use_line == first.use_line
                    && fact.use_column == first.use_column
            })
            .map(ChainLinkIdentity::from)
            .collect()
    }

    pub(super) fn planning_report<'a>(&self, report: &'a Report) -> Cow<'a, Report> {
        if let Self::PrunedBatch { declined } = self {
            let mut selected = report.clone();
            selected.facts.subtree_reexport_fix_facts = report
                .facts
                .subtree_reexport_fix_facts
                .iter()
                .filter(|fact| !declined.contains(&ChainLinkIdentity::from(*fact)))
                .cloned()
                .collect::<Vec<_>>()
                .into();
            return Cow::Owned(selected);
        }
        let Self::IndividualLinks { declined, .. } = self else {
            return Cow::Borrowed(report);
        };
        let next = self.next_links(report);
        let mut selected = report.clone();
        selected.findings.retain(|finding| {
            finding.diagnostic_code != DiagnosticCode::PubUseOutsideSubtree
                || !report.facts.subtree_reexport_fix_facts.iter().any(|fact| {
                    fact.use_path == finding.path
                        && fact.use_line == finding.line
                        && fact.use_column == finding.column
                        && !next.contains(&ChainLinkIdentity::from(fact))
                        && !declined.contains(&ChainLinkIdentity::from(fact))
                })
        });
        selected.facts.subtree_reexport_fix_facts = report
            .facts
            .subtree_reexport_fix_facts
            .iter()
            .filter(|fact| next.contains(&ChainLinkIdentity::from(*fact)))
            .cloned()
            .collect::<Vec<_>>()
            .into();
        Cow::Owned(selected)
    }
}

/// Written edits keep their validation result until the caller accepts or restores them.
pub(super) struct WrittenPass {
    pub(super) counts:     AppliedFixCounts,
    pub(super) validation: Result<RunPlan, MendFailure>,
}

impl MendRunner<'_> {
    fn plan_chain(&self, planned: &RunPlan) -> Result<ChainBatch> {
        if matches!(
            self.chain_validation,
            ChainValidation::IndividualLinks { .. }
        ) {
            return Ok(ChainBatch::Unchanged);
        }
        if planned
            .subtree_reexport_scan
            .as_ref()
            .is_none_or(|scan| scan.fixes.is_empty())
        {
            return Ok(ChainBatch::Unchanged);
        }
        let chains = ForwardingChains::collect(
            &self.selection.analysis_root,
            &planned.report,
            &planned.sources,
        )?;
        if chains.is_empty() {
            return Ok(ChainBatch::Unchanged);
        }
        let first = if matches!(self.chain_validation, ChainValidation::PrunedBatch { .. }) {
            self.plan_selected_chain_links(planned)
                .map_err(|failure| match failure {
                    MendFailure::Unexpected(error) => error,
                    failure => anyhow::anyhow!("{failure}"),
                })?
                .fixes
        } else {
            planned.function_plan.fixes.clone()
        };
        if first.is_empty() {
            return Ok(ChainBatch::Unchanged);
        }
        let mut steps = vec![ChainStep::record(
            &first,
            &self.chain_validation.planning_report(&planned.report),
            &self.selection.analysis_root,
        )];
        let rewritten = first.rewritten_files()?;
        let mut files = rewritten.files;
        let mut counts = rewritten.counts;
        let mut extension = ChainExtension::OriginalPass;
        loop {
            let sources = ParsedSources::from(&files);
            let generated = chains.next_report(&planned.report, &sources)?;
            let report = self.chain_validation.planning_report(&generated);
            if report.findings.is_empty() {
                break;
            }
            let next = self.chain_pass(&report, &sources)?;
            if next.is_empty() {
                break;
            }
            steps.push(ChainStep::record(
                &next,
                &report,
                &self.selection.analysis_root,
            ));
            let rewritten = next.rewritten_sources(&sources)?;
            if rewritten
                .files
                .iter()
                .all(|(path, text)| files.get(path) == Some(text))
            {
                break;
            }
            files.extend(rewritten.files);
            for tagged in next.tagged() {
                if let Some(kind) = tagged.fix_kind {
                    counts.record(kind);
                }
            }
            extension = ChainExtension::AdditionalLinks;
        }
        if matches!(extension, ChainExtension::OriginalPass)
            && !matches!(self.chain_validation, ChainValidation::PrunedBatch { .. })
        {
            return Ok(ChainBatch::Unchanged);
        }
        self.protect_chain(&files, counts, &planned.sources, steps)
    }

    fn chain_pass(&self, report: &Report, sources: &ParsedSources) -> Result<ValidatedFixSet> {
        let subtree = subtree_reexport::scan_selection(self.selection, report, sources, |scan| {
            Self::combined_import_plan(
                FixScans {
                    subtree_reexports: Some(scan),
                    ..FixScans::default()
                },
                sources,
                &self.selection.package_roots,
            )
            .map(|plan| plan.declined_files)
        })?;
        let next = Self::combined_import_plan(
            FixScans {
                subtree_reexports: Some(&subtree),
                ..FixScans::default()
            },
            sources,
            &self.selection.package_roots,
        )?;
        ValidatedFixSet::try_from(next.fixes)
    }

    fn protect_chain(
        &self,
        proposed_sources: &BTreeMap<PathBuf, String>,
        counts: AppliedFixCounts,
        sources: &ParsedSources,
        steps: Vec<ChainStep>,
    ) -> Result<ChainBatch> {
        let fixes = proposed_sources
            .iter()
            .map(|(path, replacement)| {
                let source = sources.source(path)?;
                let original = source.text();
                let start = original
                    .chars()
                    .zip(replacement.chars())
                    .take_while(|(a, b)| a == b)
                    .map(|(ch, _)| ch.len_utf8())
                    .sum::<usize>();
                let tail = original[start..]
                    .chars()
                    .rev()
                    .zip(replacement[start..].chars().rev())
                    .take_while(|(a, b)| a == b)
                    .map(|(ch, _)| ch.len_utf8())
                    .sum::<usize>();
                Ok(UseFix {
                    path: path.clone(),
                    start,
                    end: original.len() - tail,
                    replacement: replacement[start..replacement.len() - tail].to_string(),
                    import_group: None,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let fixes = ValidatedFixSet::try_from(fixes)?;
        let protected = FunctionLengthPlan::check(
            &fixes.tagged().cloned().collect::<Vec<_>>(),
            &[&fixes],
            sources,
            &self.selection.package_roots,
            &self.selection.workspace_root,
        )?;
        if &protected.fixes.rewritten_files()?.files != proposed_sources {
            return Ok(ChainBatch::Unchanged);
        }
        Ok(ChainBatch::Planned {
            fixes: protected.fixes,
            counts,
            steps,
        })
    }

    /// A rejected batch restores only this proposal before ordinary validation.
    pub(super) fn write_chain_or_single(
        &mut self,
        planned: &RunPlan,
        output_mode: BuildOutputMode,
    ) -> Result<WrittenPass, MendFailure> {
        let mut rejected_duration = Duration::ZERO;
        loop {
            let ChainBatch::Planned {
                fixes,
                counts,
                steps,
            } = self.plan_chain(planned).map_err(MendFailure::Unexpected)?
            else {
                break;
            };
            let mut checkpoint = SessionSnapshot::new();
            checkpoint.record(&fixes).map_err(MendFailure::Unexpected)?;
            self.session_snapshot
                .record(&fixes)
                .map_err(MendFailure::Unexpected)?;
            imports::apply_fixes(&fixes).map_err(MendFailure::Unexpected)?;
            let started = Instant::now();
            let failure = match self.validate_chain(planned, output_mode) {
                Ok(mut validation) => {
                    validation.check_duration += rejected_duration;
                    return Ok(WrittenPass {
                        counts,
                        validation: Ok(validation),
                    });
                },
                Err(failure) => failure,
            };
            rejected_duration += started.elapsed();
            checkpoint.restore().map_err(MendFailure::Unexpected)?;
            if matches!(
                self.reject_batch(&steps, &failure),
                BatchRetry::IndividualLinks
            ) {
                break;
            }
        }
        if matches!(
            self.chain_validation,
            ChainValidation::IndividualLinks { .. }
        ) {
            return self.write_individual_link(planned, output_mode, rejected_duration);
        }
        let fixes = if matches!(self.chain_validation, ChainValidation::PrunedBatch { .. }) {
            self.plan_selected_chain_links(planned)?.fixes
        } else {
            planned.function_plan.fixes.clone()
        };
        if fixes.is_empty() {
            let mut validation = self.rescan_restored_chain(planned)?;
            validation.check_duration += rejected_duration;
            return Ok(WrittenPass {
                counts:     AppliedFixCounts::default(),
                validation: Ok(validation),
            });
        }
        self.session_snapshot
            .record(&fixes)
            .map_err(MendFailure::Unexpected)?;
        let counts = imports::apply_fixes(&fixes).map_err(MendFailure::Unexpected)?;
        let validation = self
            .validate_chain(planned, output_mode)
            .map(|mut validation| {
                validation.check_duration += rejected_duration;
                validation
            });
        Ok(WrittenPass { counts, validation })
    }

    fn reject_batch(&mut self, steps: &[ChainStep], failure: &MendFailure) -> BatchRetry {
        let rejected = links_in_error_files(steps, failure);
        let mut declined =
            match std::mem::replace(&mut self.chain_validation, ChainValidation::Batch) {
                ChainValidation::PrunedBatch { declined }
                | ChainValidation::IndividualLinks { declined, .. } => declined,
                ChainValidation::Batch => BTreeSet::new(),
            };
        let previous = declined.len();
        declined.extend(rejected);
        if declined.len() > previous {
            self.chain_validation = ChainValidation::PrunedBatch { declined };
            eprintln!("mend: chain batch did not validate; retrying remaining links as one batch");
            eprintln!("mend: chain link did not validate; keeping the previous checked sources");
            BatchRetry::RemainingLinks
        } else {
            self.chain_validation = ChainValidation::IndividualLinks {
                declined,
                pass_limit: FIX_CONVERGENCE_MAX_PASSES,
            };
            eprintln!("mend: chain batch did not validate; retrying individual links");
            BatchRetry::IndividualLinks
        }
    }

    fn write_individual_link(
        &mut self,
        planned: &RunPlan,
        output_mode: BuildOutputMode,
        mut rejected_duration: Duration,
    ) -> Result<WrittenPass, MendFailure> {
        let pass = self.plan_selected_chain_links(planned)?;
        let attempts_link = !pass.declined_on_failure.is_empty();
        let mut checkpoint = SessionSnapshot::new();
        checkpoint
            .record(&pass.fixes)
            .map_err(MendFailure::Unexpected)?;
        self.session_snapshot
            .record(&pass.fixes)
            .map_err(MendFailure::Unexpected)?;
        let mut counts = imports::apply_fixes(&pass.fixes).map_err(MendFailure::Unexpected)?;
        let started = Instant::now();
        let validation = match self.validate_chain(planned, output_mode) {
            Err(_) if !pass.declined_on_failure.is_empty() => {
                rejected_duration += started.elapsed();
                checkpoint.restore().map_err(MendFailure::Unexpected)?;
                if let ChainValidation::IndividualLinks { declined, .. } =
                    &mut self.chain_validation
                {
                    declined.extend(pass.declined_on_failure);
                }
                counts = AppliedFixCounts::default();
                eprintln!(
                    "mend: chain link did not validate; keeping the previous checked sources"
                );
                self.rescan_restored_chain(planned)
            },
            validation => validation,
        }
        .map(|mut validation| {
            validation.check_duration += rejected_duration;
            validation
        });
        // Each successful transition removes a forwarding binding or refuses it.
        // Other pending links retain their own validation pass.
        if attempts_link
            && validation.is_ok()
            && let ChainValidation::IndividualLinks { pass_limit, .. } = &mut self.chain_validation
        {
            *pass_limit += 1;
        }
        Ok(WrittenPass { counts, validation })
    }

    fn rescan_restored_chain(&self, planned: &RunPlan) -> Result<RunPlan, MendFailure> {
        let checked = self.scan_checked_sources(SelectionResult {
            report:            planned.report.clone(),
            check_duration:    Duration::ZERO,
            compiler_warnings: planned.compiler_warnings,
            compiler_fixable:  planned.compiler_fixable,
        })?;
        self.scan_fixes(
            planned.operation_mode.clone(),
            checked,
            planned.operation_mode.fixes.selection(FixKind::PubUse),
        )
    }

    pub(crate) const fn fix_pass_limit(&self) -> usize {
        match self.chain_validation {
            ChainValidation::Batch | ChainValidation::PrunedBatch { .. } => {
                FIX_CONVERGENCE_MAX_PASSES
            },
            ChainValidation::IndividualLinks { pass_limit, .. } => pass_limit,
        }
    }

    fn validate_chain(
        &self,
        planned: &RunPlan,
        output_mode: BuildOutputMode,
    ) -> Result<RunPlan, MendFailure> {
        let checked = self.build_selection(output_mode)?;
        self.scan_fixes(
            planned.operation_mode.clone(),
            checked,
            planned.operation_mode.fixes.selection(FixKind::PubUse),
        )
    }
}
