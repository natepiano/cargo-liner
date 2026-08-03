use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;

use super::parent_boundary;
use super::parent_boundary::ParentBoundaryKey;
use super::validated_plan;
use super::validated_plan::ValidatedPubUsePlan;
use crate::fixes::imports::UseFix;
use crate::fixes::imports::ValidatedFixSet;
use crate::reporting::Report;
use crate::rust_syntax;
use crate::selection::Selection;

pub(crate) struct PubUseFixScan {
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

struct PubUseAnalysis {
    supported_plans: Vec<ValidatedPubUsePlan>,
    skipped:         usize,
}

enum CandidateScreening {
    Accept(NarrowablePubUseCandidate),
    Skip,
}

enum CandidateExportMatch {
    Matches,
    Mismatch,
}

/// Byte range of a bare `pub` keyword inside a child source file — exactly the
/// text `--fix-pub-use` replaces with `pub(super)`.
struct BarePubKeyword {
    start: usize,
    end:   usize,
}

enum ChildVisibilityState {
    /// The declaration is annotated bare `pub`, so narrowing it to
    /// `pub(super)` is a single keyword rewrite.
    PlainPub(BarePubKeyword),
    /// The declaration is already reachable from less than bare `pub` — either
    /// a restricted annotation (`pub(crate)`, `pub(super)`, `pub(in …)`) or no
    /// `pub` at all. The pub-use fixer never rewrites those.
    NarrowerThanPub,
}

/// A candidate whose child declaration was confirmed rewritable, carrying the
/// keyword range the narrowing edit will replace.
struct NarrowablePubUseCandidate {
    candidate:        PubUseCandidate,
    bare_pub_keyword: BarePubKeyword,
}

pub(crate) fn scan_selection(selection: &Selection, report: &Report) -> Result<PubUseFixScan> {
    let mut fixes = Vec::new();
    let facts = collect_pub_use_fix_facts(selection, report);
    let analysis = analyze_pub_use_candidates(&facts)?;
    let parent_fix_groups = group_parent_pub_use_plans(&analysis.supported_plans);

    for plan in &analysis.supported_plans {
        fixes.push(plan.child_narrowing.clone());
    }

    for (parent_boundary, exports) in parent_fix_groups {
        let removal =
            parent_boundary::build_parent_pub_use_edit_for_exports(&parent_boundary, &exports)?;
        fixes.push(removal);
    }

    fixes.extend(validated_plan::rewrite_subtree_imports_for_plans(
        selection,
        &analysis.supported_plans,
    )?);
    let fixes = ValidatedFixSet::try_from(fixes)?;

    Ok(PubUseFixScan {
        fixes,
        applied: analysis.supported_plans.len(),
        skipped: analysis.skipped,
    })
}

fn collect_pub_use_fix_facts(selection: &Selection, report: &Report) -> Vec<PubUseFixFact> {
    let mut facts = Vec::new();
    for fact in report.facts.pub_use_fix_facts.iter() {
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
        let Some(parent_export) = parent_boundary::resolve_parent_pub_use_export(
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

        let source_root = validated_plan::find_source_root(&fact.parent_module)
            .context("failed to determine src root for parent module")?;

        let parent_module_path = module_path_from_boundary_file(&source_root, &fact.parent_module)
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
            CandidateScreening::Accept(narrowable) => {
                supported_plans.push(build_validated_plan(narrowable, parent_boundary));
            },
            // A screened-out candidate still reaches the user as a skip: the
            // finding advertised `FixSupport::PubUse`, so silently dropping it
            // would report "applied 0 pub use fix(es)" with no explanation.
            CandidateScreening::Skip => skipped += 1,
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
    let child_visibility = child_visibility_state(child_source, candidate.child_line)?;

    Ok(match (export_match, child_visibility) {
        (CandidateExportMatch::Matches, ChildVisibilityState::PlainPub(bare_pub_keyword)) => {
            CandidateScreening::Accept(NarrowablePubUseCandidate {
                candidate,
                bare_pub_keyword,
            })
        },
        _ => CandidateScreening::Skip,
    })
}

fn build_validated_plan(
    narrowable: NarrowablePubUseCandidate,
    parent_boundary: ParentBoundaryKey,
) -> ValidatedPubUsePlan {
    let NarrowablePubUseCandidate {
        candidate,
        bare_pub_keyword,
    } = narrowable;
    let child_narrowing = UseFix {
        path:         candidate.child_file.clone(),
        start:        bare_pub_keyword.start,
        end:          bare_pub_keyword.end,
        replacement:  "pub(super)".to_string(),
        import_group: None,
    };
    ValidatedPubUsePlan {
        parent_boundary,
        child_file: candidate.child_file,
        child_module: candidate.child_module,
        exported_name: candidate.exported_name,
        parent_module_path: candidate.parent_module_path,
        target_item_path: candidate.target_item_path,
        child_narrowing,
    }
}

fn child_visibility_state(source: &str, line: usize) -> Result<ChildVisibilityState> {
    let line_span = validated_plan::line_span(source, line)
        .context("failed to compute child item line span")?;
    Ok(
        find_bare_pub_keyword(&source[line_span.0..line_span.1], line_span.0)
            .map_or(ChildVisibilityState::NarrowerThanPub, |bare_pub_keyword| {
                ChildVisibilityState::PlainPub(bare_pub_keyword)
            }),
    )
}

/// Locates a `pub` keyword that stands alone as the whole visibility
/// annotation. A following `(` means the declaration is already restricted,
/// whether it is written tight (`pub(crate)`) or spaced (`pub (crate)`), and
/// nothing at all after the keyword means the item body starts on the next
/// line — `pub\nstruct Thing;` is still a bare `pub` declaration.
fn find_bare_pub_keyword(line_text: &str, line_start: usize) -> Option<BarePubKeyword> {
    let mut search_from = 0usize;
    while let Some(offset) = line_text[search_from..].find("pub") {
        let start = search_from + offset;
        let end = start + "pub".len();
        let extends_a_longer_identifier = line_text[..start]
            .chars()
            .next_back()
            .is_some_and(|character| character.is_alphanumeric() || character == '_');
        let keyword_ends_here = line_text[end..]
            .chars()
            .next()
            .is_none_or(char::is_whitespace);
        let restriction_follows = line_text[end..].trim_start().starts_with('(');
        if !extends_a_longer_identifier && keyword_ends_here && !restriction_follows {
            return Some(BarePubKeyword {
                start: line_start + start,
                end:   line_start + end,
            });
        }
        search_from = end;
    }
    None
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

fn module_path_from_dir(source_root: &Path, module_dir: &Path) -> Option<Vec<String>> {
    let relative = module_dir.strip_prefix(source_root).ok()?;
    let components = relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    (!components.is_empty()).then_some(components)
}

fn module_path_from_boundary_file(source_root: &Path, boundary_file: &Path) -> Option<Vec<String>> {
    if boundary_file.file_name().and_then(OsStr::to_str) == Some("mod.rs") {
        return module_path_from_dir(source_root, boundary_file.parent()?);
    }

    rust_syntax::file_module_path(source_root, boundary_file)
}
