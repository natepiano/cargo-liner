use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;

use super::parent_export;
use super::rewrite;
use crate::constants::PUB_VISIBILITY_PREFIX;
use crate::diagnostics::Report;
use crate::imports::UseFix;
use crate::imports::ValidatedFixSet;
use crate::module_paths;
use crate::selection::Selection;

pub struct PubUseFixScan {
    pub fixes:   ValidatedFixSet,
    pub applied: usize,
    pub skipped: usize,
}

struct PubUseFixFact {
    child_file:      PathBuf,
    child_line:      usize,
    child_item_name: String,
    parent_module:   PathBuf,
    parent_line:     usize,
    child_module:    String,
}

struct PubUseCandidate {
    child_file:         PathBuf,
    child_line:         usize,
    child_module:       String,
    exported_name:      String,
    parent_module_path: Vec<String>,
    target_item_path:   Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct ParentBoundaryKey {
    pub(super) parent_module: PathBuf,
    pub(super) item_start:    usize,
    pub(super) item_end:      usize,
}

pub(super) struct ValidatedPubUsePlan {
    pub(super) parent_boundary:    ParentBoundaryKey,
    pub(super) child_file:         PathBuf,
    pub(super) child_module:       String,
    pub(super) exported_name:      String,
    pub(super) parent_module_path: Vec<String>,
    pub(super) target_item_path:   Vec<String>,
    pub(super) child_narrowing:    UseFix,
}

struct PubUseAnalysis {
    supported_plans: Vec<ValidatedPubUsePlan>,
    skipped:         usize,
}

enum CandidateScreening {
    Accept(PubUseCandidate),
    Skip,
}

enum CandidateExportMatch {
    Matches,
    Mismatch,
}

enum ChildVisibilityState {
    PlainPub,
    AlreadyNarrowed,
}

pub fn scan_selection(selection: &Selection, report: &Report) -> Result<PubUseFixScan> {
    let mut fixes = Vec::new();
    let facts = collect_pub_use_fix_facts(selection, report);
    let analysis = analyze_pub_use_candidates(&facts)?;
    let parent_fix_groups = group_parent_pub_use_plans(&analysis.supported_plans);

    for plan in &analysis.supported_plans {
        fixes.push(plan.child_narrowing.clone());
    }

    for (parent_boundary, exports) in parent_fix_groups {
        let removal =
            parent_export::build_parent_pub_use_edit_for_exports(&parent_boundary, &exports)?;
        fixes.push(removal);
    }

    fixes.extend(rewrite::rewrite_subtree_imports_for_plans(
        selection,
        &analysis.supported_plans,
    )?);
    let fixes = ValidatedFixSet::from_vec(fixes)?;

    Ok(PubUseFixScan {
        fixes,
        applied: analysis.supported_plans.len(),
        skipped: analysis.skipped,
    })
}

fn collect_pub_use_fix_facts(selection: &Selection, report: &Report) -> Vec<PubUseFixFact> {
    let mut facts = Vec::new();
    for fact in report.facts.pub_use.iter() {
        let child_rel = normalize_rel_path(&fact.child_path);
        let parent_rel = normalize_rel_path(&fact.parent_path);
        facts.push(PubUseFixFact {
            child_file:      selection.analysis_root.join(&child_rel),
            child_line:      fact.child_line,
            child_item_name: fact.child_item_name.clone(),
            parent_module:   selection.analysis_root.join(&parent_rel),
            parent_line:     fact.parent_line,
            child_module:    fact.child_module.clone(),
        });
    }

    facts
}

fn analyze_pub_use_candidates(facts: &[PubUseFixFact]) -> Result<PubUseAnalysis> {
    let mut supported_plans = Vec::new();
    let mut skipped = 0usize;
    for fact in facts {
        let child_source = fs::read_to_string(&fact.child_file)
            .with_context(|| format!("failed to read {}", fact.child_file.display()))?;
        let parent_source = fs::read_to_string(&fact.parent_module)
            .with_context(|| format!("failed to read {}", fact.parent_module.display()))?;
        let Some(parent_export) = parent_export::resolve_parent_pub_use_export(
            &parent_source,
            fact.parent_line,
            &fact.child_module,
            &fact.child_item_name,
        )
        .with_context(|| {
            format!(
                "failed to resolve exported item from {}:{}",
                fact.parent_module.display(),
                fact.parent_line
            )
        })?
        else {
            skipped += 1;
            continue;
        };

        let src_root = find_src_root(&fact.parent_module)
            .context("failed to determine src root for parent module")?;

        let parent_module_path = module_path_from_boundary_file(&src_root, &fact.parent_module)
            .context("failed to determine parent module path")?;
        let mut target_item_path = parent_module_path.clone();
        target_item_path.push(fact.child_module.clone());
        target_item_path.push(fact.child_item_name.clone());

        let parent_boundary = ParentBoundaryKey {
            parent_module: fact.parent_module.clone(),
            ..parent_export.parent_boundary
        };
        let candidate = PubUseCandidate {
            child_file: fact.child_file.clone(),
            child_line: fact.child_line,
            child_module: fact.child_module.clone(),
            exported_name: parent_export.exported_name,
            parent_module_path,
            target_item_path,
        };
        match screen_candidate(candidate, &fact.child_item_name, &child_source)? {
            CandidateScreening::Accept(candidate) => {
                supported_plans.push(build_validated_plan(candidate, parent_boundary)?);
            },
            CandidateScreening::Skip => {},
        }
    }

    Ok(PubUseAnalysis {
        supported_plans,
        skipped,
    })
}

fn screen_candidate(
    candidate: PubUseCandidate,
    child_item: &str,
    child_source: &str,
) -> Result<CandidateScreening> {
    let export_match = if candidate.exported_name == child_item {
        CandidateExportMatch::Matches
    } else {
        CandidateExportMatch::Mismatch
    };
    let child_visibility = if line_contains_plain_pub(child_source, candidate.child_line)? {
        ChildVisibilityState::PlainPub
    } else {
        ChildVisibilityState::AlreadyNarrowed
    };

    Ok(match (export_match, child_visibility) {
        (CandidateExportMatch::Matches, ChildVisibilityState::PlainPub) => {
            CandidateScreening::Accept(candidate)
        },
        _ => CandidateScreening::Skip,
    })
}

fn build_validated_plan(
    candidate: PubUseCandidate,
    parent_boundary: ParentBoundaryKey,
) -> Result<ValidatedPubUsePlan> {
    let child_narrowing = build_child_pub_super_fix(&candidate)?;
    Ok(ValidatedPubUsePlan {
        parent_boundary,
        child_file: candidate.child_file,
        child_module: candidate.child_module,
        exported_name: candidate.exported_name,
        parent_module_path: candidate.parent_module_path,
        target_item_path: candidate.target_item_path,
        child_narrowing,
    })
}

fn build_child_pub_super_fix(candidate: &PubUseCandidate) -> Result<UseFix> {
    let source = fs::read_to_string(&candidate.child_file)
        .with_context(|| format!("failed to read {}", candidate.child_file.display()))?;
    let line_span = rewrite::line_span(&source, candidate.child_line)
        .context("failed to compute child visibility line span")?;
    let line_text = &source[line_span.0..line_span.1];
    let Some(relative_start) = line_text.find(PUB_VISIBILITY_PREFIX) else {
        anyhow::bail!(
            "child item line {} does not contain a plain `pub ` prefix",
            candidate.child_line
        );
    };
    Ok(UseFix {
        path:        candidate.child_file.clone(),
        start:       line_span.0 + relative_start,
        end:         line_span.0 + relative_start + PUB_VISIBILITY_PREFIX.len(),
        replacement: "pub(super) ".to_string(),
    })
}

fn line_contains_plain_pub(source: &str, line: usize) -> Result<bool> {
    let line_span =
        rewrite::line_span(source, line).context("failed to compute child item line span")?;
    Ok(source[line_span.0..line_span.1].contains(PUB_VISIBILITY_PREFIX))
}

fn group_parent_pub_use_plans(
    plans: &[ValidatedPubUsePlan],
) -> BTreeMap<ParentBoundaryKey, Vec<(String, String)>> {
    let mut groups = BTreeMap::new();
    for plan in plans {
        groups
            .entry(plan.parent_boundary.clone())
            .or_insert_with(Vec::new)
            .push((plan.child_module.clone(), plan.exported_name.clone()));
    }
    groups
}

fn normalize_rel_path(path: impl AsRef<Path>) -> String {
    path.as_ref().to_string_lossy().replace('\\', "/")
}

fn module_path_from_dir(src_root: &Path, module_dir: &Path) -> Option<Vec<String>> {
    let relative = module_dir.strip_prefix(src_root).ok()?;
    let components = relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    (!components.is_empty()).then_some(components)
}

fn module_path_from_boundary_file(src_root: &Path, boundary_file: &Path) -> Option<Vec<String>> {
    if boundary_file.file_name().and_then(|name| name.to_str()) == Some("mod.rs") {
        return module_path_from_dir(src_root, boundary_file.parent()?);
    }

    module_paths::file_module_path(src_root, boundary_file)
}

pub(super) fn find_src_root(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .find(|ancestor| ancestor.file_name().and_then(|name| name.to_str()) == Some("src"))
        .map(Path::to_path_buf)
}
