use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::ops::Deref;

use serde::Deserialize;
use serde::Serialize;

use super::caller_aware;
use super::caller_aware::CallerMap;
use super::schema::StoredFinding;
use super::schema::StoredReport;
use crate::compiler::visibility;
use crate::compiler::visibility::NoFacadeVisibilityRepair;
use crate::config::DiagnosticCode;
use crate::reporting::ExactBoundarySpelling;
use crate::reporting::FixSupport;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(in crate::compiler) enum StoredVisibilityReach {
    Public,
    Crate,
    Restricted { boundary: String },
}

/// The inverse of [`StoredVisibilityReach::boundary`]: the vocabulary every
/// producer in this module already speaks, where `crate-external` means `pub`
/// and `crate` means `pub(crate)`.
impl From<&str> for StoredVisibilityReach {
    fn from(boundary: &str) -> Self {
        match boundary {
            "crate-external" => Self::Public,
            "crate" => Self::Crate,
            restricted => Self::Restricted {
                boundary: restricted.to_string(),
            },
        }
    }
}

impl StoredVisibilityReach {
    pub(in crate::compiler) fn join(&self, other: &Self) -> Self {
        match (self, other) {
            (Self::Public, _) | (_, Self::Public) => Self::Public,
            (Self::Crate, _) | (_, Self::Crate) => Self::Crate,
            (Self::Restricted { boundary: left }, Self::Restricted { boundary: right }) => {
                let boundary = common_def_path_ancestor(left, right);
                if boundary == "crate" || boundary.is_empty() {
                    Self::Crate
                } else {
                    Self::Restricted { boundary }
                }
            },
        }
    }

    pub(in crate::compiler) fn to_source(&self) -> String {
        match self {
            Self::Public => String::from("pub"),
            Self::Crate => String::from("pub(crate)"),
            Self::Restricted { boundary } => format!("pub(in {boundary})"),
        }
    }

    /// Whether `self` reaches anywhere `ceiling` does not. Joining takes the
    /// wider of the two, so a join that is still the ceiling means `self` is
    /// inside it.
    pub(in crate::compiler) fn exceeds(&self, ceiling: &Self) -> bool {
        self.join(ceiling) != *ceiling
    }

    pub(in crate::compiler) fn boundary(&self) -> &str {
        match self {
            Self::Public => "crate-external",
            Self::Crate => "crate",
            Self::Restricted { boundary } => boundary,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(in crate::compiler) struct StoredVisibilitySource {
    pub path:   String,
    pub line:   usize,
    pub column: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(in crate::compiler) struct StoredVisibilityDeclaration {
    pub item_def_path:        String,
    pub item_module_def_path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(in crate::compiler) enum StoredVisibilitySpelling {
    Public,
    Crate,
    InCrate,
    ExactPath,
    NonCanonical,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(in crate::compiler) enum StoredFacadeConstraint {
    Absent,
    Impossible,
    Resolved { required: StoredVisibilityReach },
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(in crate::compiler) enum StoredExactBoundaryAcceptance {
    Eligible,
    Ineligible,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(in crate::compiler) enum StoredExactPathPolicy {
    #[default]
    Forbidden,
    Allowed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(in crate::compiler) enum StoredCallerReconciliation {
    Fixed,
    CallerAware,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(in crate::compiler) enum StoredConstraintOutcome {
    Accepted,
    Finding,
}

/// What a widening of this item's visibility would leak, and how far it can be
/// widened without leaking it.
///
/// Widening an item raises the effective visibility of every trait impl it is
/// the self type of, and rustc then requires every type in those impls'
/// interfaces to be at least that visible (E0446). See
/// `visibility::interface_ceiling` for how the boundary is derived.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(in crate::compiler) struct StoredInterfaceCeiling {
    pub reach:       StoredVisibilityReach,
    /// Crate-rooted path of the type left behind: `crate::a::b::c::PrepError`.
    pub leaked_type: String,
    /// The impl that would expose it: `impl TryFrom<u8> for Thing`.
    pub impl_header: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(in crate::compiler) struct StoredVisibilityConstraint {
    pub diagnostic_code: DiagnosticCode,
    pub source:          StoredVisibilitySource,
    pub declaration:     StoredVisibilityDeclaration,
    #[serde(flatten)]
    pub facts:           StoredVisibilityFacts,
}

/// Target-specific requirements, independent of the module mounting the item.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(in crate::compiler) struct StoredVisibilityFacts {
    pub visibility_annotation:      String,
    pub declared_reach:             StoredVisibilityReach,
    pub spelling:                   StoredVisibilitySpelling,
    pub signature_requirement:      Option<StoredVisibilityReach>,
    /// How far this item may be widened before a trait impl of it leaves a
    /// narrower type in its own interface. `None` means nothing caps it.
    #[serde(default)]
    pub interface_ceiling:          Option<StoredInterfaceCeiling>,
    pub facade:                     StoredFacadeConstraint,
    pub exact_boundary_acceptance:  StoredExactBoundaryAcceptance,
    /// Whether the declaration form alone lets mend edit this annotation.
    /// Independent of `pub_in_path`, which governs only what a replacement may
    /// spell.
    pub annotation_edit_acceptance: StoredExactBoundaryAcceptance,
    /// Project policy for an exact `pub(in crate::...)` replacement.
    #[serde(default)]
    pub exact_path_policy:          StoredExactPathPolicy,
    pub caller_reconciliation:      StoredCallerReconciliation,
    pub outcome:                    StoredConstraintOutcome,
}

impl StoredVisibilityFacts {
    pub(in crate::compiler) fn required_reach(&self) -> Option<StoredVisibilityReach> {
        let facade_requirement = match &self.facade {
            StoredFacadeConstraint::Resolved { required } => Some(required.clone()),
            StoredFacadeConstraint::Absent
            | StoredFacadeConstraint::Impossible
            | StoredFacadeConstraint::Blocked => None,
        };
        match (&self.signature_requirement, facade_requirement) {
            (Some(signature), Some(facade)) => Some(signature.join(&facade)),
            (Some(signature), None) => Some(signature.clone()),
            (None, Some(facade)) => Some(facade),
            (None, None) => None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct VisibilityConstraintSet {
    constraints: BTreeMap<StoredVisibilityFacts, Vec<StoredFinding>>,
}

impl VisibilityConstraintSet {
    #[cfg(test)]
    fn from_constraint(constraint: StoredVisibilityConstraint) -> Self {
        Self {
            constraints: BTreeMap::from([(constraint.facts, Vec::new())]),
        }
    }

    #[cfg(test)]
    fn join(mut self, additional: Self) -> Self {
        for (facts, findings) in additional.constraints {
            self.constraints.entry(facts).or_default().extend(findings);
        }
        self
    }

    fn required_reach(&self) -> Option<StoredVisibilityReach> {
        self.constraints
            .keys()
            .filter_map(StoredVisibilityFacts::required_reach)
            .reduce(|current, additional| current.join(&additional))
    }

    fn includes_facade_blocker(&self) -> bool {
        self.constraints
            .keys()
            .any(|constraint| matches!(constraint.facade, StoredFacadeConstraint::Blocked))
    }

    fn includes_absent_facade(&self) -> bool {
        self.constraints.keys().any(|constraint| {
            matches!(
                constraint.facade,
                StoredFacadeConstraint::Absent | StoredFacadeConstraint::Impossible
            )
        })
    }

    fn all_facades_are_impossible(&self) -> bool {
        !self.constraints.is_empty()
            && self
                .constraints
                .keys()
                .all(|constraint| matches!(constraint.facade, StoredFacadeConstraint::Impossible))
    }

    fn all_facades_are_absent_or_impossible(&self) -> bool {
        !self.constraints.is_empty()
            && self.constraints.keys().all(|constraint| {
                matches!(
                    constraint.facade,
                    StoredFacadeConstraint::Absent | StoredFacadeConstraint::Impossible
                )
            })
    }

    fn all_facades_are_resolved(&self) -> bool {
        self.constraints
            .keys()
            .all(|constraint| matches!(constraint.facade, StoredFacadeConstraint::Resolved { .. }))
    }

    fn all_exact_boundaries_are_eligible(&self) -> bool {
        self.constraints.keys().all(|constraint| {
            constraint.exact_boundary_acceptance == StoredExactBoundaryAcceptance::Eligible
        })
    }

    fn all_exact_paths_are_allowed(&self) -> bool {
        self.constraints
            .keys()
            .all(|constraint| constraint.exact_path_policy == StoredExactPathPolicy::Allowed)
    }

    /// Whether every constraint's declaration form lets mend edit its
    /// visibility annotation, with no question of what the replacement spells.
    ///
    /// [`Self::all_exact_boundaries_are_eligible`] is the suppression-side
    /// question and folds `pub_in_path` in; this one deliberately does not, so
    /// [`MountVisibility::supports_rewrite`] can apply the setting
    /// against `ExactBoundarySpelling::CratePath` alone.
    fn all_annotation_edits_are_eligible(&self) -> bool {
        self.constraints.keys().all(|constraint| {
            constraint.annotation_edit_acceptance == StoredExactBoundaryAcceptance::Eligible
        })
    }

    fn uniform_declared_reach(&self) -> Option<StoredVisibilityReach> {
        let mut reaches = self
            .constraints
            .keys()
            .map(|constraint| &constraint.declared_reach);
        let first = reaches.next()?.clone();
        reaches.all(|reach| *reach == first).then_some(first)
    }

    /// Every constraint in the group spells an exact crate-rooted
    /// `pub(in crate::…)` that matches the reach its own signature demands, and
    /// no target in the group has a facade. Nothing but the caller map can
    /// still demand the boundary.
    fn declares_exactly_its_signature_reach(&self) -> bool {
        !self.constraints.is_empty()
            && self.all_exact_boundaries_are_eligible()
            && self.constraints.keys().all(|constraint| {
                constraint.spelling == StoredVisibilitySpelling::ExactPath
                    && matches!(constraint.facade, StoredFacadeConstraint::Absent)
                    && constraint.signature_requirement.as_ref() == Some(&constraint.declared_reach)
            })
    }

    fn uses_caller_reconciliation(&self) -> bool {
        self.constraints.keys().any(|constraint| {
            constraint.caller_reconciliation == StoredCallerReconciliation::CallerAware
        }) || (self.includes_absent_facade() && self.required_reach().is_some())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct VisibilityConstraintKey {
    diagnostic_code: DiagnosticCode,
    source:          StoredVisibilitySource,
}

impl VisibilityConstraintKey {
    fn new(constraint: &StoredVisibilityConstraint) -> Self {
        Self {
            diagnostic_code: constraint.diagnostic_code,
            source:          constraint.source.clone(),
        }
    }

    /// The key a finding would carry if it were a constraint, so a finding can
    /// be matched against constraints by lookup instead of by scanning. A
    /// constraint and a finding correspond when their diagnostic code and
    /// physical source position agree, including across package boundaries.
    fn for_finding(finding: &StoredFinding) -> Self {
        Self {
            diagnostic_code: finding.diagnostic_code,
            source:          StoredVisibilitySource {
                path:   finding.path.clone(),
                line:   finding.line,
                column: finding.column,
            },
        }
    }
}

/// One package-qualified meaning of an item in a shared physical source.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct VisibilityModuleMount {
    package_root: String,
    declaration:  StoredVisibilityDeclaration,
}

/// Per-mount requirements for the annotation at one physical source position.
/// Module boundaries are resolved within each mount before comparing spellings.
#[derive(Default)]
struct SharedSourceVisibility {
    mounts: BTreeMap<VisibilityModuleMount, VisibilityConstraintGroup>,
}

/// A source's package-qualified module identity. Library, binary and test
/// reports can describe the same module; their requirements are joined before
/// comparing distinct mounts. Compilation identity is evidence, not a mount.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct SourceModuleMount {
    package_root: String,
    module_path:  Vec<String>,
}

#[derive(Default)]
struct SourceMountInventory {
    sources: BTreeMap<String, BTreeMap<SourceModuleMount, BTreeSet<usize>>>,
}

impl SourceMountInventory {
    fn collect(reports: &[StoredReport]) -> Self {
        let mut inventory = Self::default();
        for (report_index, report) in reports.iter().enumerate() {
            for mount in &report.module_mount_facts {
                inventory
                    .sources
                    .entry(mount.file.clone())
                    .or_default()
                    .entry(SourceModuleMount {
                        package_root: report.package_root.clone(),
                        module_path:  mount.module_path.clone(),
                    })
                    .or_default()
                    .insert(report_index);
            }
        }
        inventory
    }

    fn locate(&self, report_index: usize, path: &str, module: &[String]) -> FindingMount {
        let Some(mounts) = self.sources.get(path) else {
            return FindingMount::Unknown;
        };
        let mut in_report = mounts
            .iter()
            .filter(|(_, reports)| reports.contains(&report_index))
            .map(|(mount, _)| mount);
        let Some(first) = in_report.next() else {
            return FindingMount::Unknown;
        };
        // Producers without a def-path still identify the mount unambiguously
        // when this target compiled the source in just one place.
        if in_report.next().is_none() {
            return FindingMount::Known(first.clone());
        }
        mounts
            .iter()
            .filter(|(mount, reports)| {
                reports.contains(&report_index) && module.starts_with(&mount.module_path)
            })
            .max_by_key(|(mount, _)| mount.module_path.len())
            .map_or(FindingMount::Unknown, |(mount, _)| {
                FindingMount::Known(mount.clone())
            })
    }
}

fn module_segments(module: &str) -> Vec<String> {
    module
        .split("::")
        .filter(|segment| !segment.is_empty() && *segment != "crate")
        .map(str::to_string)
        .collect()
}

enum FindingMount {
    Known(SourceModuleMount),
    Unknown,
}

struct VisibilityProposal {
    report_index: usize,
    module_path:  Vec<String>,
    finding:      StoredFinding,
}

/// The written replacement, independent of which diagnostic proposed it.
#[derive(PartialEq, Eq)]
enum VisibilityReplacement {
    Private,
    Public,
    Crate,
    Parent,
    CratePath(String),
    FieldSuggestion(String),
    Unproven,
}

impl VisibilityReplacement {
    fn for_finding(finding: &StoredFinding) -> Self {
        match finding.fix_support {
            FixSupport::UnusedPub => Self::Private,
            FixSupport::NarrowToPubCrate => Self::Crate,
            FixSupport::FieldVisibility => finding
                .suggestion
                .as_ref()
                .map_or(Self::Unproven, |suggestion| {
                    Self::FieldSuggestion(suggestion.clone())
                }),
            FixSupport::RestrictedAnnotation => match finding.exact_boundary_spelling {
                ExactBoundarySpelling::Private => Self::Private,
                ExactBoundarySpelling::Public => Self::Public,
                ExactBoundarySpelling::Crate => Self::Crate,
                ExactBoundarySpelling::Parent => Self::Parent,
                ExactBoundarySpelling::CratePath => finding
                    .narrower_scope_def_path
                    .as_ref()
                    .map_or(Self::Unproven, |boundary| Self::CratePath(boundary.clone())),
            },
            // A facade edit changes other declarations too; annotation agreement
            // alone does not prove that the complete edit works in every mount.
            _ => Self::Unproven,
        }
    }
}

// These fixes rewrite declaration annotations. Subtree re-export fixes move
// imports instead; their planner already rejects owners with several parents
// and retains their diagnostics so its skipped-fix notice remains observable.
const fn edits_visibility(finding: &StoredFinding) -> bool {
    matches!(
        finding.fix_support,
        FixSupport::RestrictedAnnotation
            | FixSupport::UnusedPub
            | FixSupport::NarrowToPubCrate
            | FixSupport::FieldVisibility
            | FixSupport::PubUse
    )
}

const fn concerns_visibility(finding: &StoredFinding) -> bool {
    edits_visibility(finding)
        || matches!(
            finding.diagnostic_code,
            DiagnosticCode::ForbiddenPubInCrate
                | DiagnosticCode::OverbroadPubCrate
                | DiagnosticCode::SuspiciousPub
                | DiagnosticCode::UnusedPub
                | DiagnosticCode::NarrowToPubCrate
                | DiagnosticCode::FieldVisibilityWiderThanType
                | DiagnosticCode::InternalParentPubUseFacade
        )
}

/// Agreement owns automatic edit decisions only. Manual diagnostics survive
/// independently, once per distinct diagnostic at the physical source site.
fn resolve_visibility_proposals(
    proposals: Vec<VisibilityProposal>,
    inventory: &SourceMountInventory,
    reports: &[StoredReport],
) -> Vec<(usize, StoredFinding)> {
    // Agreement follows every source an edit changes, not just the diagnostic
    // location advertising it. Fix facts own the additional destinations.
    let expected: BTreeMap<_, _> = proposals
        .iter()
        .flat_map(|proposal| {
            visibility_edit_sources(&reports[proposal.report_index], &proposal.finding)
        })
        .filter_map(|path| {
            inventory
                .sources
                .get(path)
                .filter(|mounts| mounts.len() > 1)
                .map(|mounts| (path.to_string(), mounts))
        })
        .collect();
    if expected.is_empty() {
        return proposals
            .into_iter()
            .map(|proposal| (proposal.report_index, proposal.finding))
            .collect();
    }
    let mut covered: BTreeMap<String, BTreeSet<SourceModuleMount>> = BTreeMap::new();
    let mut replacement = VisibilityReplacement::Unproven;
    let mut agreeing = true;
    let mut automatic = Vec::new();
    let mut manual = Vec::new();
    let mut manual_keys = BTreeSet::new();
    for proposal in proposals {
        if edits_visibility(&proposal.finding) {
            let proposed = VisibilityReplacement::for_finding(&proposal.finding);
            if automatic.is_empty() {
                replacement = proposed;
            } else if replacement != proposed {
                agreeing = false;
            }
            for path in visibility_edit_sources(&reports[proposal.report_index], &proposal.finding)
            {
                if !expected.contains_key(path) {
                    continue;
                }
                match inventory.locate(proposal.report_index, path, &proposal.module_path) {
                    FindingMount::Known(mount) => {
                        covered.entry(path.to_string()).or_default().insert(mount);
                    },
                    FindingMount::Unknown => {
                        agreeing = false;
                    },
                }
            }
            automatic.push((proposal.report_index, proposal.finding));
        } else {
            let finding = &proposal.finding;
            let key = (
                finding.diagnostic_code,
                finding.severity,
                finding.message.clone(),
                finding.item.clone(),
                finding.suggestion.clone(),
                finding.related.clone(),
            );
            if manual_keys.insert(key) {
                manual.push((proposal.report_index, proposal.finding));
            }
        }
    }
    let all_mounts_agree = expected.iter().all(|(path, mounts)| {
        covered
            .get(path)
            .is_some_and(|covered| covered.iter().eq(mounts.keys()))
    });
    if agreeing
        && replacement != VisibilityReplacement::Unproven
        && all_mounts_agree
        && manual.is_empty()
        && let Some(finding) = automatic.into_iter().next()
    {
        manual.push(finding);
    }
    manual
}

/// A finding can authorize an edit at its own site or through persisted fix
/// facts at other sites. Use the same advertising-site identity as fact pruning.
fn visibility_edit_sources<'a>(
    report: &'a StoredReport,
    finding: &'a StoredFinding,
) -> BTreeSet<&'a str> {
    let mut paths = BTreeSet::from([finding.path.as_str()]);
    let site = super::schema::PubUseNarrowingSite::for_finding(finding);
    for fact in &report.pub_use_fix_facts {
        if super::schema::PubUseNarrowingSite::advertising_sites(fact)
            .iter()
            .any(|advertising| site.as_ref() == Some(advertising))
        {
            paths.insert(&fact.child_path);
            paths.insert(&fact.parent_path);
        }
    }
    paths
}

#[derive(Clone)]
struct VisibilityFindingCandidate<'a> {
    constraint: &'a StoredVisibilityFacts,
    finding:    &'a StoredFinding,
}

/// What demands the boundary a declaration spells. A type named only in
/// another item's signature is reached through that item's path, never through
/// its own, so no facade can re-export it and none is required. A caller that
/// writes the item's own path across the boundary does need one.
#[derive(Clone, Copy, PartialEq, Eq)]
enum BoundaryDemand {
    /// Signatures alone cross the boundary — the declaration stands as written.
    SignatureAlone,
    /// A facade, a declaration wider than its signature, or a caller naming the
    /// item demands the boundary.
    BeyondSignature,
}

/// Constraints across compilation targets for one `VisibilityModuleMount`.
/// Every declaration in this group therefore has the same module path.
#[derive(Default)]
struct VisibilityConstraintGroup {
    constraints: VisibilityConstraintSet,
    reports:     BTreeSet<usize>,
}

/// Whether replacing the annotation with `spelling` at `boundary` changes what
/// can name the item, rather than only how the same reach is spelled.
///
/// `pub(in crate::a)` on an item whose parent module is `crate::a` reaches
/// exactly what `pub(super)` reaches, so a rewrite between the two is a spelling
/// change and the policy headline still describes it. A rewrite to a different
/// reach is about what uses the item instead, which is what
/// [`visibility::resolved_boundary_headline`] says.
fn replacement_changes_reach(
    spelling: ExactBoundarySpelling,
    declared_reach: &StoredVisibilityReach,
    boundary: &str,
) -> bool {
    match spelling {
        ExactBoundarySpelling::Public => *declared_reach != StoredVisibilityReach::Public,
        ExactBoundarySpelling::Crate => *declared_reach != StoredVisibilityReach::Crate,
        ExactBoundarySpelling::Private => true,
        ExactBoundarySpelling::CratePath | ExactBoundarySpelling::Parent => !matches!(
            declared_reach,
            StoredVisibilityReach::Restricted { boundary: declared } if declared == boundary
        ),
    }
}

/// How the resolved boundary is written, for a message that names it outside a
/// `consider using:` suggestion.
fn replacement_spelling(spelling: ExactBoundarySpelling, boundary: &str) -> String {
    match spelling {
        ExactBoundarySpelling::CratePath => format!("`pub(in {boundary})`"),
        ExactBoundarySpelling::Parent => String::from("`pub(super)`"),
        ExactBoundarySpelling::Crate => String::from("`pub(crate)`"),
        ExactBoundarySpelling::Private => String::from("private"),
        ExactBoundarySpelling::Public => String::from("`pub`"),
    }
}

/// Whether replacing `annotation` with `spelling` can only widen the item's
/// reach.
///
/// Any restricted spelling is contained in `pub(crate)`, and every spelling is
/// contained in `pub`; a bare `pub` narrowed to `pub(crate)` is the case this
/// must exclude. [`MountVisibility::interface_leak`] is the only
/// caller, because widening is the edit that can raise a trait impl past a type
/// its own interface names.
fn replacement_only_widens(spelling: ExactBoundarySpelling, annotation: &str) -> bool {
    match spelling {
        ExactBoundarySpelling::Public => annotation != "pub",
        ExactBoundarySpelling::Crate => {
            !matches!(annotation, "pub" | "pub(crate)" | "pub(in crate)")
        },
        ExactBoundarySpelling::CratePath
        | ExactBoundarySpelling::Parent
        | ExactBoundarySpelling::Private => false,
    }
}

impl VisibilityConstraintGroup {
    fn include(
        &mut self,
        report_index: usize,
        facts: StoredVisibilityFacts,
        finding: Option<StoredFinding>,
    ) {
        let candidates = self.constraints.constraints.entry(facts).or_default();
        if let Some(finding) = finding {
            self.reports.insert(report_index);
            candidates.push(finding);
        }
    }

    fn candidates(&self) -> impl Iterator<Item = VisibilityFindingCandidate<'_>> {
        self.constraints
            .constraints
            .iter()
            .flat_map(|(constraint, findings)| {
                findings
                    .iter()
                    .map(move |finding| VisibilityFindingCandidate {
                        constraint,
                        finding,
                    })
            })
    }

    fn render(
        &self,
        callers: &CallerMap,
        package_root: &str,
        declaration: &StoredVisibilityDeclaration,
    ) -> Option<StoredFinding> {
        MountVisibility {
            group: self,
            declaration,
        }
        .render(callers, package_root)
    }
}

/// A mount supplies the sole declaration used to interpret its target facts.
struct MountVisibility<'a> {
    group:       &'a VisibilityConstraintGroup,
    declaration: &'a StoredVisibilityDeclaration,
}

impl Deref for MountVisibility<'_> {
    type Target = VisibilityConstraintGroup;

    fn deref(&self) -> &Self::Target { self.group }
}

impl MountVisibility<'_> {
    fn render(&self, callers: &CallerMap, package_root: &str) -> Option<StoredFinding> {
        let required_reach = self.constraints.required_reach();
        if self.declares_effective_required_boundary(callers, package_root, required_reach.as_ref())
        {
            return None;
        }
        if required_reach == Some(StoredVisibilityReach::Public) {
            return self.public_candidate();
        }
        if self.constraints.includes_facade_blocker() {
            if self.constraints.constraints.keys().all(|constraint| {
                constraint.spelling == StoredVisibilitySpelling::Crate
                    && constraint.declared_reach == StoredVisibilityReach::Crate
            }) {
                return None;
            }
            return self.blocker_candidate();
        }
        if let Some(finding) = self.facade_cleanup_candidate() {
            return Some(finding);
        }
        if self.constraints.all_facades_are_resolved() {
            return self.render_resolved_facades(callers, package_root, required_reach.as_ref());
        }
        let candidate = self.preferred_candidate()?;
        let mut finding = candidate.finding.clone();
        let item_def_path = self.declaration.item_def_path.clone();
        let item_module = self.declaration.item_module_def_path.clone();
        if !self.constraints.uses_caller_reconciliation() {
            return Some(finding);
        }
        if self.declares_facade_less_caller_boundary(callers, package_root) {
            return None;
        }
        if self.boundary_demand(callers, package_root, required_reach.as_ref())
            == BoundaryDemand::SignatureAlone
        {
            return None;
        }
        let repair = self.no_facade_repair(callers, package_root, required_reach.as_ref());
        let (boundary, exact_boundary_spelling) = match repair {
            NoFacadeVisibilityRepair::RemoveAnnotation => {
                (String::from("private"), ExactBoundarySpelling::Private)
            },
            NoFacadeVisibilityRepair::UseParentVisibility => {
                let parent = self.parent_boundary_path();
                let spelling = self.exact_boundary_spelling(&parent);
                (parent, spelling)
            },
            NoFacadeVisibilityRepair::StructuralMigrationForCallerLocations
            | NoFacadeVisibilityRepair::StructuralMigrationForSignatureReach { .. } => {
                let boundary =
                    self.caller_repair_boundary(callers, package_root, required_reach.as_ref());
                let spelling = self.exact_boundary_spelling(&boundary);
                (boundary, spelling)
            },
        };
        if exact_boundary_spelling == ExactBoundarySpelling::Crate
            && self.constraints.constraints.keys().all(|constraint| {
                constraint.spelling == StoredVisibilitySpelling::Crate
                    && constraint.declared_reach == StoredVisibilityReach::Crate
            })
        {
            return None;
        }
        if self.supports_rewrite(exact_boundary_spelling) {
            self.apply_rewrite(&mut finding, &boundary, exact_boundary_spelling);
        } else {
            if finding.fix_support == FixSupport::RestrictedAnnotation {
                finding.fix_support = FixSupport::None;
                finding.narrower_scope_def_path = None;
            }
            finding.exact_boundary_spelling = ExactBoundarySpelling::CratePath;
            finding.message = visibility::no_facade_headline(repair, finding.message);
            finding.suggestion = Some(visibility::no_facade_suggestion(
                repair,
                &boundary,
                &item_def_path,
            ));
            finding.related = None;
        }
        // Whichever branch ran replaced `finding.suggestion` and its
        // `finding.related` note: that note used one target's callers,
        // while `repair` reflects every target's. Rebuild the note from
        // `repair` so it cannot argue for a repair no longer offered.
        // `visibility::no_facade_caller_note` declines a whole-crate
        // boundary; `Self::apply_rewrite` has a resolved boundary to
        // name, so preserve its note when this call has nothing to say.
        let resolved_boundary_note = finding.related.take();
        finding.related = visibility::no_facade_caller_note(
            repair,
            &item_module,
            visibility::parent_scope_def_path(&item_module),
        )
        .or(resolved_boundary_note);
        Some(finding)
    }

    fn declares_effective_required_boundary(
        &self,
        callers: &CallerMap,
        package_root: &str,
        required_reach: Option<&StoredVisibilityReach>,
    ) -> bool {
        if self.candidates().any(|candidate| {
            matches!(
                candidate.finding.fix_support,
                FixSupport::PubUse | FixSupport::NeedsManualPubUseCleanup
            )
        }) {
            return false;
        }
        if !self.constraints.all_exact_boundaries_are_eligible() {
            return false;
        }
        let Some(declared_reach) = self.constraints.uniform_declared_reach() else {
            return false;
        };
        let caller_reach = self.caller_required_reach(callers, package_root);
        let signature_or_facade_reach = if self.constraints.all_facades_are_impossible() {
            None
        } else {
            required_reach
        };
        let effective_reach = match (signature_or_facade_reach, caller_reach) {
            (Some(required), Some(caller)) => Some(required.join(&caller)),
            (Some(required), None) => Some(required.clone()),
            (None, Some(caller)) => Some(caller),
            (None, None) => None,
        };
        effective_reach == Some(declared_reach)
    }

    fn caller_required_reach(
        &self,
        callers: &CallerMap,
        package_root: &str,
    ) -> Option<StoredVisibilityReach> {
        let declaration = self.declaration;
        let reaching =
            &caller_aware::callers_for_package(callers, package_root, &declaration.item_def_path)?
                .reaching;
        if reaching.is_empty() {
            return None;
        }
        let boundary =
            visibility::common_ancestor_def_path(&declaration.item_module_def_path, reaching);
        match boundary {
            Some(boundary) if boundary == declaration.item_module_def_path => None,
            Some(boundary) => Some(StoredVisibilityReach::Restricted {
                boundary: visibility::crate_rooted_def_path(&boundary),
            }),
            None => Some(StoredVisibilityReach::Crate),
        }
    }

    fn declares_facade_less_caller_boundary(
        &self,
        callers: &CallerMap,
        package_root: &str,
    ) -> bool {
        self.constraints.all_facades_are_absent_or_impossible()
            && self.callers_require_structural_repair(callers, package_root)
            && self.constraints.all_exact_boundaries_are_eligible()
            && self
                .constraints
                .constraints
                .keys()
                .all(|constraint| constraint.spelling == StoredVisibilitySpelling::ExactPath)
            && self
                .constraints
                .uniform_declared_reach()
                .is_some_and(|declared| {
                    declared.boundary() == self.caller_boundary_path(callers, package_root)
                })
    }

    fn callers_require_structural_repair(&self, callers: &CallerMap, package_root: &str) -> bool {
        let reaching_callers = caller_aware::callers_for_package(
            callers,
            package_root,
            &self.declaration.item_def_path,
        )
        .map(|item_callers| &item_callers.reaching);
        matches!(
            caller_repair(self.declaration, reaching_callers),
            NoFacadeVisibilityRepair::StructuralMigrationForCallerLocations
        )
    }

    fn exact_boundary_spelling(&self, boundary: &str) -> ExactBoundarySpelling {
        if boundary == "crate" {
            return ExactBoundarySpelling::Crate;
        }
        let parent = visibility::parent_scope_def_path(&self.declaration.item_module_def_path);
        if !parent.is_empty() && visibility::crate_rooted_def_path(parent) == boundary {
            ExactBoundarySpelling::Parent
        } else {
            ExactBoundarySpelling::CratePath
        }
    }

    fn parent_boundary_path(&self) -> String {
        visibility::crate_rooted_def_path(visibility::parent_scope_def_path(
            &self.declaration.item_module_def_path,
        ))
    }

    fn supports_rewrite(&self, spelling: ExactBoundarySpelling) -> bool {
        self.constraints.all_annotation_edits_are_eligible()
            && (spelling != ExactBoundarySpelling::CratePath
                || self.constraints.all_exact_paths_are_allowed())
    }

    /// The ceiling a widening would break through, if it would.
    ///
    /// Only a widening can. The item keeps every trait impl it is the self type
    /// of, and raising the item raises those impls with it, so a type their
    /// interfaces name that sits below the new boundary is one rustc will
    /// reject (E0446). A narrowing or a respelling of the same reach moves no
    /// impl above anything.
    fn interface_leak(
        &self,
        boundary: &str,
        spelling: ExactBoundarySpelling,
    ) -> Option<&StoredInterfaceCeiling> {
        let constraint = self
            .constraints
            .constraints
            .first_key_value()
            .map(|(facts, _)| facts)?;
        if !replacement_only_widens(spelling, &constraint.visibility_annotation) {
            return None;
        }
        let ceiling = constraint.interface_ceiling.as_ref()?;
        StoredVisibilityReach::from(boundary)
            .exceeds(&ceiling.reach)
            .then_some(ceiling)
    }

    fn apply_rewrite(
        &self,
        finding: &mut StoredFinding,
        boundary: &str,
        spelling: ExactBoundarySpelling,
    ) {
        let interface_leak = self.interface_leak(boundary, spelling);
        finding.suggestion = Some(match spelling {
            ExactBoundarySpelling::CratePath => {
                format!("consider using: `pub(in {boundary})`")
            },
            ExactBoundarySpelling::Parent => String::from("consider using: `pub(super)`"),
            ExactBoundarySpelling::Crate => String::from("consider using: `pub(crate)`"),
            ExactBoundarySpelling::Private => String::from("consider removing the visibility"),
            ExactBoundarySpelling::Public => String::from("consider using: `pub`"),
        });
        // `visibility::missing_facade_note` explains a re-export suggestion,
        // but `finding.suggestion` now asks for a visibility change: resolving
        // `boundary` disproves the old note's claim that nothing names the item
        // there. `finding.related` will explain why this boundary. The caller
        // prefers `visibility::no_facade_caller_note` when available, otherwise
        // keeping this note. Removing an annotation names no visibility to report,
        // and `NoFacadeVisibilityRepair::RemoveAnnotation` always has a caller note.
        finding.related = interface_leak.map_or_else(
            || {
                (matches!(finding.diagnostic_code, DiagnosticCode::ForbiddenPubInCrate)
                    && spelling != ExactBoundarySpelling::Private)
                    .then(|| {
                        visibility::resolved_boundary_note(&replacement_spelling(
                            spelling, boundary,
                        ))
                    })
            },
            |leak| {
                Some(visibility::interface_leak_note(
                    &replacement_spelling(spelling, boundary),
                    &leak.leaked_type,
                    &leak.impl_header,
                ))
            },
        );
        finding.message = match finding.diagnostic_code {
            DiagnosticCode::OverbroadPubCrate => {
                String::from("`pub(crate)` is broader than required")
            },
            DiagnosticCode::SuspiciousPub => format!(
                "the narrowest proven visibility is {}",
                replacement_spelling(spelling, boundary)
            ),
            // `DiagnosticCode::ForbiddenPubInCrate` has two scan-pass headlines superseded by a
            // resolved boundary. The structural headline says nothing keeps the item reachable,
            // true of the original single-crate view; retaining it printed "`pub(super)` is too
            // narrow" directly above "help: consider using: `pub(super)`" on 13 findings of one
            // workspace. The policy headline says the spelling lies outside an exact facade
            // boundary, relevant only while the repair is a facade; it cannot explain why a
            // suggestion names wider visibility. When `replacement_changes_reach` holds,
            // `visibility::resolved_boundary_headline` names the annotation; respelling the
            // same reach keeps the policy headline, which remains relevant. Restrict this
            // through `visibility::is_annotation_policy_headline`: headlines naming what caps
            // the item convey more and remain valid after boundary resolution.
            DiagnosticCode::ForbiddenPubInCrate => self
                .constraints
                .constraints
                .first_key_value()
                .map(|(facts, _)| facts)
                .map_or_else(
                    || finding.message.clone(),
                    |constraint| {
                        let annotation = constraint.visibility_annotation.as_str();
                        if !visibility::is_annotation_policy_headline(&finding.message, annotation)
                        {
                            finding.message.clone()
                        } else if replacement_changes_reach(
                            spelling,
                            &constraint.declared_reach,
                            boundary,
                        ) {
                            visibility::resolved_boundary_headline(annotation)
                        } else {
                            visibility::forbidden_pub_in_headline(annotation)
                        }
                    },
                ),
            _ => finding.message.clone(),
        };
        // Every call site is already behind `Self::supports_rewrite`, which
        // decides whether the declaration form can be edited at all and whether
        // `pub_in_path` allows the spelling. That leaves the resolved boundary,
        // and the one thing that disqualifies it is a leak through a trait impl.
        //
        // This used to also require the written annotation be `pub`,
        // `pub(crate)`, or `pub(in crate)`, so a `pub(in crate::a)` narrowing to
        // `pub(in crate::a::b)` was suggested and never applied. That list was
        // added after a `--fix` on hana_valence_arrangements rolled back with
        // two E0446s, and it named the wrong culprit: those errors came from
        // narrowing an item a wider item's signature still named, which is what
        // `signature_requirement` now folds into the boundary. The written
        // spelling never bore on whether the scan saw every use — the 395
        // retargets mend already applied from the three listed spellings carry
        // that risk identically. Applying all 17 held-back narrowings at once
        // leaves `cargo check --workspace --all-targets` clean.
        finding.fix_support = if interface_leak.is_none() {
            FixSupport::RestrictedAnnotation
        } else {
            FixSupport::None
        };
        if let Some(constraint) = self
            .constraints
            .constraints
            .first_key_value()
            .map(|(facts, _)| facts)
        {
            finding.visibility_annotation = Some(constraint.visibility_annotation.clone());
            finding.item_def_path = Some(self.declaration.item_def_path.clone());
        }
        finding.narrower_scope_def_path = interface_leak.is_none().then(|| match spelling {
            ExactBoundarySpelling::CratePath | ExactBoundarySpelling::Parent => boundary
                .strip_prefix("crate::")
                .unwrap_or(boundary)
                .to_string(),
            ExactBoundarySpelling::Crate | ExactBoundarySpelling::Public => String::from("crate"),
            ExactBoundarySpelling::Private => String::from("private"),
        });
        finding.exact_boundary_spelling = spelling;
    }

    fn caller_repair_boundary(
        &self,
        callers: &CallerMap,
        package_root: &str,
        required_reach: Option<&StoredVisibilityReach>,
    ) -> String {
        let caller_boundary = self.caller_boundary_path(callers, package_root);
        if self.constraints.all_facades_are_impossible() {
            return caller_boundary;
        }
        let caller_reach = StoredVisibilityReach::from(caller_boundary.as_str());
        required_reach.map_or_else(
            || caller_reach.boundary().to_string(),
            |required| required.join(&caller_reach).boundary().to_string(),
        )
    }

    fn public_candidate(&self) -> Option<StoredFinding> {
        let mut finding = self
            .candidates()
            .filter(|candidate| {
                candidate.constraint.required_reach() == Some(StoredVisibilityReach::Public)
            })
            .min_by(candidate_order)
            .or_else(|| self.candidates().min_by(candidate_order))
            .map(|candidate| candidate.finding.clone())?;
        if self.supports_rewrite(ExactBoundarySpelling::Public) {
            self.apply_rewrite(
                &mut finding,
                "crate-external",
                ExactBoundarySpelling::Public,
            );
        }
        Some(finding)
    }

    fn blocker_candidate(&self) -> Option<StoredFinding> {
        self.candidates()
            .filter(|candidate| {
                matches!(candidate.constraint.facade, StoredFacadeConstraint::Blocked)
            })
            .min_by(candidate_order)
            .or_else(|| self.candidates().min_by(candidate_order))
            .map(|candidate| candidate.finding.clone())
    }

    fn facade_cleanup_candidate(&self) -> Option<StoredFinding> {
        self.candidates()
            .filter(|candidate| {
                matches!(
                    candidate.finding.fix_support,
                    FixSupport::PubUse | FixSupport::NeedsManualPubUseCleanup
                )
            })
            .min_by(candidate_order)
            .map(|candidate| candidate.finding.clone())
    }

    /// A resolved facade fixes the reach the item's *own path* needs, but a
    /// caller can reach the same declaration without writing that path — most
    /// often by naming a function whose signature mentions the type. Joining the
    /// caller map in keeps the two halves of this module agreed:
    /// [`Self::declares_effective_required_boundary`] already accepts an
    /// annotation against the joined reach, so rewriting against the facade
    /// alone would narrow past a `ThroughSignature` caller and leave a private
    /// type in a wider item's signature (`private_interfaces`), rolling `--fix`
    /// back.
    fn render_resolved_facades(
        &self,
        callers: &CallerMap,
        package_root: &str,
        required_reach: Option<&StoredVisibilityReach>,
    ) -> Option<StoredFinding> {
        let facade_reach = required_reach?;
        let required_reach = self
            .caller_required_reach(callers, package_root)
            .map_or_else(|| facade_reach.clone(), |caller| facade_reach.join(&caller));
        let declared_reach = self.constraints.uniform_declared_reach();
        if declared_reach.as_ref() == Some(&required_reach)
            && self.constraints.all_exact_boundaries_are_eligible()
        {
            return None;
        }
        let mut finding = self.preferred_candidate()?.finding.clone();
        if declared_reach.as_ref() != Some(&required_reach) {
            let boundary = required_reach.boundary();
            let spelling = self.exact_boundary_spelling(boundary);
            if self.supports_rewrite(spelling) {
                self.apply_rewrite(&mut finding, boundary, spelling);
            } else {
                finding.suggestion =
                    Some(format!("consider using: `{}`", required_reach.to_source()));
                finding.fix_support = FixSupport::None;
                finding.narrower_scope_def_path = None;
            }
        }
        Some(finding)
    }

    fn preferred_candidate(&self) -> Option<VisibilityFindingCandidate<'_>> {
        self.candidates().min_by(|left, right| {
            candidate_priority(left)
                .cmp(&candidate_priority(right))
                .then_with(|| candidate_order(left, right))
        })
    }

    /// Decided here rather than during the scan because the scan sees only the
    /// callers of the compilation target it is walking, while `callers` is the
    /// union across every target in the package. An acceptance is also
    /// unrecoverable: [`Self::render`] can suppress or reword a finding but
    /// never create one, so a scan that accepted in every target would leave
    /// nothing for the union to correct.
    fn boundary_demand(
        &self,
        callers: &CallerMap,
        package_root: &str,
        required_reach: Option<&StoredVisibilityReach>,
    ) -> BoundaryDemand {
        if !self.constraints.declares_exactly_its_signature_reach()
            || self.constraints.uniform_declared_reach().as_ref() != required_reach
        {
            return BoundaryDemand::BeyondSignature;
        }
        let naming_callers = caller_aware::callers_for_package(
            callers,
            package_root,
            &self.declaration.item_def_path,
        )
        .map(|item_callers| &item_callers.naming);
        let demanded_beyond_signature = !matches!(
            requirement_repair(self.declaration, required_reach),
            NoFacadeVisibilityRepair::StructuralMigrationForCallerLocations
        ) || matches!(
            caller_repair(self.declaration, naming_callers),
            NoFacadeVisibilityRepair::StructuralMigrationForCallerLocations
        );
        if demanded_beyond_signature {
            BoundaryDemand::BeyondSignature
        } else {
            BoundaryDemand::SignatureAlone
        }
    }

    fn no_facade_repair(
        &self,
        callers: &CallerMap,
        package_root: &str,
        required_reach: Option<&StoredVisibilityReach>,
    ) -> NoFacadeVisibilityRepair {
        let repair = if self.constraints.all_facades_are_impossible() {
            NoFacadeVisibilityRepair::RemoveAnnotation
        } else {
            requirement_repair(self.declaration, required_reach)
        };
        let reaching_callers = caller_aware::callers_for_package(
            callers,
            package_root,
            &self.declaration.item_def_path,
        )
        .map(|item_callers| &item_callers.reaching);
        repair.most_invasive(caller_repair(self.declaration, reaching_callers))
    }

    fn caller_boundary_path(&self, callers: &CallerMap, package_root: &str) -> String {
        let declaration = self.declaration;
        let no_callers = BTreeSet::new();
        let reaching_callers =
            caller_aware::callers_for_package(callers, package_root, &declaration.item_def_path)
                .map_or(&no_callers, |item_callers| &item_callers.reaching);
        visibility::common_ancestor_def_path(&declaration.item_module_def_path, reaching_callers)
            .map_or_else(
                || String::from("crate"),
                |boundary| visibility::crate_rooted_def_path(&boundary),
            )
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum CandidatePriority {
    Signature,
    ResolvedFacade,
    Other,
}

const fn candidate_priority(candidate: &VisibilityFindingCandidate) -> CandidatePriority {
    if candidate.constraint.signature_requirement.is_some() {
        CandidatePriority::Signature
    } else if matches!(
        candidate.constraint.facade,
        StoredFacadeConstraint::Resolved { .. }
    ) {
        CandidatePriority::ResolvedFacade
    } else {
        CandidatePriority::Other
    }
}

fn candidate_order(
    left: &VisibilityFindingCandidate,
    right: &VisibilityFindingCandidate,
) -> Ordering {
    (
        &left.constraint,
        &left.finding.message,
        &left.finding.suggestion,
    )
        .cmp(&(
            &right.constraint,
            &right.finding.message,
            &right.finding.suggestion,
        ))
}

fn requirement_repair(
    declaration: &StoredVisibilityDeclaration,
    required_reach: Option<&StoredVisibilityReach>,
) -> NoFacadeVisibilityRepair {
    let Some(required_reach) = required_reach else {
        return NoFacadeVisibilityRepair::RemoveAnnotation;
    };
    let StoredVisibilityReach::Restricted { boundary } = required_reach else {
        return NoFacadeVisibilityRepair::StructuralMigrationForCallerLocations;
    };
    let compiler_boundary = compiler_boundary_path(boundary, &declaration.item_module_def_path);
    if visibility::def_path_is_descendant(compiler_boundary, &declaration.item_module_def_path) {
        return NoFacadeVisibilityRepair::RemoveAnnotation;
    }
    let parent = visibility::parent_scope_def_path(&declaration.item_module_def_path);
    if visibility::def_path_is_descendant(compiler_boundary, parent) {
        NoFacadeVisibilityRepair::UseParentVisibility
    } else {
        NoFacadeVisibilityRepair::StructuralMigrationForCallerLocations
    }
}

fn compiler_boundary_path<'path>(boundary: &'path str, item_module: &str) -> &'path str {
    if item_module.starts_with("crate::") {
        boundary
    } else {
        boundary.strip_prefix("crate::").unwrap_or(boundary)
    }
}

fn caller_repair(
    declaration: &StoredVisibilityDeclaration,
    callers: Option<&BTreeSet<String>>,
) -> NoFacadeVisibilityRepair {
    let no_callers = BTreeSet::new();
    let item_module = &declaration.item_module_def_path;
    visibility::classify_no_facade_callers(
        item_module,
        visibility::parent_scope_def_path(item_module),
        callers.unwrap_or(&no_callers),
    )
}

fn group_visibility_constraints(
    reports: &[StoredReport],
) -> BTreeMap<VisibilityConstraintKey, SharedSourceVisibility> {
    let mut groups: BTreeMap<VisibilityConstraintKey, SharedSourceVisibility> = BTreeMap::new();
    for (report_index, report) in reports.iter().enumerate() {
        let mut findings_by_key: BTreeMap<VisibilityConstraintKey, Vec<&StoredFinding>> =
            BTreeMap::new();
        for finding in &report.findings {
            findings_by_key
                .entry(VisibilityConstraintKey::for_finding(finding))
                .or_default()
                .push(finding);
        }
        for constraint in &report.visibility_constraints {
            let key = VisibilityConstraintKey::new(constraint);
            let mount = VisibilityModuleMount {
                package_root: report.package_root.clone(),
                declaration:  constraint.declaration.clone(),
            };
            let group = groups
                .entry(key.clone())
                .or_default()
                .mounts
                .entry(mount)
                .or_default();
            let findings = findings_by_key.get(&key);
            if constraint.facts.outcome == StoredConstraintOutcome::Finding
                && let Some(findings) = findings
            {
                for finding in findings {
                    group.include(
                        report_index,
                        constraint.facts.clone(),
                        Some((*finding).clone()),
                    );
                }
            } else {
                group.include(report_index, constraint.facts.clone(), None);
            }
        }
    }
    groups
}

pub(super) fn reconcile_visibility_constraints(reports: &mut [StoredReport], callers: &CallerMap) {
    let inventory = SourceMountInventory::collect(reports);
    let groups = group_visibility_constraints(reports);

    let reconciled_keys: BTreeSet<_> = groups
        .iter()
        .filter(|(_, shared)| {
            shared
                .mounts
                .values()
                .any(|group| group.candidates().next().is_some())
        })
        .map(|(key, _)| key.clone())
        .collect();
    let mut sites: BTreeMap<StoredVisibilitySource, Vec<VisibilityProposal>> = BTreeMap::new();
    for report in reports.iter_mut() {
        report.findings.retain(|finding| {
            !reconciled_keys.contains(&VisibilityConstraintKey::for_finding(finding))
        });
    }
    for (key, shared) in groups {
        for (mount, group) in shared.mounts {
            let rendered = group.render(callers, &mount.package_root, &mount.declaration);
            // Some scans defer an automatic edit until callers are merged and
            // therefore initially mark it None. An ineligible annotation or a
            // manual rendered outcome instead denotes an actual manual error.
            if inventory
                .sources
                .get(&key.source.path)
                .is_some_and(|mounts| mounts.len() > 1)
            {
                for candidate in group.candidates().filter(|candidate| {
                    !edits_visibility(candidate.finding)
                        && (candidate.constraint.annotation_edit_acceptance
                            == StoredExactBoundaryAcceptance::Ineligible
                            || rendered
                                .as_ref()
                                .is_some_and(|finding| !edits_visibility(finding)))
                }) {
                    if let Some(report_index) = group.reports.first() {
                        sites
                            .entry(key.source.clone())
                            .or_default()
                            .push(VisibilityProposal {
                                report_index: *report_index,
                                module_path:  module_segments(
                                    &mount.declaration.item_module_def_path,
                                ),
                                finding:      candidate.finding.clone(),
                            });
                    }
                }
            }
            if let Some(finding) = rendered {
                for report_index in &group.reports {
                    sites
                        .entry(key.source.clone())
                        .or_default()
                        .push(VisibilityProposal {
                            report_index: *report_index,
                            module_path:  module_segments(&mount.declaration.item_module_def_path),
                            finding:      finding.clone(),
                        });
                }
            }
        }
    }
    for (report_index, report) in reports.iter_mut().enumerate() {
        let mut retained = Vec::new();
        for finding in std::mem::take(&mut report.findings) {
            if !concerns_visibility(&finding) {
                retained.push(finding);
                continue;
            }
            let module = finding
                .item_def_path
                .as_deref()
                .and_then(|item| item.rsplit_once("::"))
                .map_or("", |(module, _)| module);
            let module_path = module_segments(module);
            sites
                .entry(VisibilityConstraintKey::for_finding(&finding).source)
                .or_default()
                .push(VisibilityProposal {
                    report_index,
                    module_path,
                    finding,
                });
        }
        report.findings = retained;
    }
    for proposals in sites.into_values() {
        for (report_index, finding) in resolve_visibility_proposals(proposals, &inventory, reports)
        {
            reports[report_index].findings.push(finding);
        }
    }
}

fn common_def_path_ancestor(left: &str, right: &str) -> String {
    left.split("::")
        .zip(right.split("::"))
        .take_while(|(left_segment, right_segment)| left_segment == right_segment)
        .map(|(segment, _)| segment)
        .collect::<Vec<_>>()
        .join("::")
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::CallerMap;
    use super::NoFacadeVisibilityRepair;
    use super::StoredCallerReconciliation;
    use super::StoredConstraintOutcome;
    use super::StoredExactBoundaryAcceptance;
    use super::StoredExactPathPolicy;
    use super::StoredFacadeConstraint;
    use super::StoredVisibilityConstraint;
    use super::StoredVisibilityDeclaration;
    use super::StoredVisibilityFacts;
    use super::StoredVisibilityReach;
    use super::StoredVisibilitySource;
    use super::StoredVisibilitySpelling;
    use super::VisibilityConstraintGroup;
    use super::VisibilityConstraintSet;
    use super::visibility;
    use crate::compiler::constants::FINDINGS_SCHEMA_VERSION;
    use crate::compiler::persistence::StoredFinding;
    use crate::compiler::persistence::StoredReport;
    use crate::compiler::persistence::caller_aware;
    use crate::compiler::persistence::schema::StoredModuleMountFact;
    use crate::compiler::persistence::schema::StoredPubUseFixFact;
    use crate::compiler::persistence::schema::UseSite;
    use crate::compiler::persistence::schema::UseSiteReference;
    use crate::config::DiagnosticCode;
    use crate::reporting::AllFeaturesCoverage;
    use crate::reporting::CompilerWarningFacts;
    use crate::reporting::ExactBoundarySpelling;
    use crate::reporting::FixSupport;
    use crate::reporting::Severity;

    struct TestMount {
        group:       VisibilityConstraintGroup,
        declaration: StoredVisibilityDeclaration,
    }

    impl Default for TestMount {
        fn default() -> Self {
            Self {
                group:       VisibilityConstraintGroup::default(),
                declaration: constraint(StoredFacadeConstraint::Absent, None).declaration,
            }
        }
    }

    impl TestMount {
        fn include_constraint(
            &mut self,
            report: usize,
            constraint: StoredVisibilityConstraint,
            finding: Option<StoredFinding>,
        ) {
            self.declaration = constraint.declaration;
            self.group.include(report, constraint.facts, finding);
        }

        fn render(&self, callers: &CallerMap, package: &str) -> Option<StoredFinding> {
            self.group.render(callers, package, &self.declaration)
        }
    }

    #[test]
    fn reach_join_is_associative_commutative_and_idempotent() {
        let reaches = [
            StoredVisibilityReach::Public,
            StoredVisibilityReach::Crate,
            restricted("crate::a"),
            restricted("crate::a::b"),
            restricted("crate::c"),
        ];

        for left in &reaches {
            assert_eq!(left.join(left), *left);
            for right in &reaches {
                assert_eq!(left.join(right), right.join(left));
                for third in &reaches {
                    assert_eq!(left.join(&right.join(third)), left.join(right).join(third));
                }
            }
        }
    }

    #[test]
    fn constraint_union_is_associative_commutative_and_idempotent() {
        let left = VisibilityConstraintSet::from_constraint(constraint(
            StoredFacadeConstraint::Absent,
            Some(restricted("crate::a::b")),
        ));
        let right = VisibilityConstraintSet::from_constraint(constraint(
            StoredFacadeConstraint::Resolved {
                required: restricted("crate::a"),
            },
            None,
        ));
        let third = VisibilityConstraintSet::from_constraint(constraint(
            StoredFacadeConstraint::Resolved {
                required: restricted("crate::c"),
            },
            None,
        ));

        assert_eq!(left.clone().join(left.clone()), left);
        assert_eq!(
            left.clone().join(right.clone()),
            right.clone().join(left.clone())
        );
        assert_eq!(
            left.clone().join(right.clone()).join(third.clone()),
            left.join(right.join(third))
        );
    }

    #[test]
    fn sibling_requirements_join_to_crate_reach() {
        let constraints = VisibilityConstraintSet::from_constraint(constraint(
            StoredFacadeConstraint::Resolved {
                required: restricted("crate::left"),
            },
            None,
        ))
        .join(VisibilityConstraintSet::from_constraint(constraint(
            StoredFacadeConstraint::Resolved {
                required: restricted("crate::right"),
            },
            None,
        )));

        assert_eq!(
            constraints.required_reach(),
            Some(StoredVisibilityReach::Crate)
        );
    }

    #[test]
    fn public_requirement_precedes_a_facade_blocker_in_both_orders() {
        let public = constraint(
            StoredFacadeConstraint::Absent,
            Some(StoredVisibilityReach::Public),
        );
        let blocker = constraint(StoredFacadeConstraint::Blocked, None);

        for constraints in [[public.clone(), blocker.clone()], [blocker, public]] {
            let mut group = TestMount::default();
            for constraint in constraints {
                let message = if constraint.facts.signature_requirement
                    == Some(StoredVisibilityReach::Public)
                {
                    "public requirement"
                } else {
                    "facade blocker"
                };
                group.include_constraint(0, constraint, Some(finding(message)));
            }

            let rendered = group.render(&CallerMap::new(), "/package");

            assert_eq!(
                rendered.as_ref().map(|finding| finding.message.as_str()),
                Some("public requirement")
            );
        }
    }

    #[test]
    fn sibling_resolved_facades_suppress_a_satisfied_crate_annotation() {
        let mut group = TestMount::default();
        for boundary in ["crate::left", "crate::right"] {
            let mut constraint = constraint(
                StoredFacadeConstraint::Resolved {
                    required: restricted(boundary),
                },
                None,
            );
            constraint.facts.declared_reach = StoredVisibilityReach::Crate;
            constraint.facts.spelling = StoredVisibilitySpelling::Crate;
            group.include_constraint(0, constraint, Some(finding(boundary)));
        }

        assert!(group.render(&CallerMap::new(), "/package").is_none());
    }

    #[test]
    fn accepted_constraint_never_renders_without_a_finding() {
        let mut constraint = constraint(
            StoredFacadeConstraint::Resolved {
                required: restricted("crate::a"),
            },
            None,
        );
        constraint.facts.outcome = StoredConstraintOutcome::Accepted;
        let mut group = TestMount::default();
        group.include_constraint(0, constraint, None);

        assert!(group.render(&CallerMap::new(), "/package").is_none());
    }

    #[test]
    fn facade_less_exact_caller_boundary_suppresses_the_finding() {
        let mut constraint = constraint(
            StoredFacadeConstraint::Impossible,
            Some(StoredVisibilityReach::Crate),
        );
        constraint.declaration.item_def_path = String::from("crate::a::b::c::item");
        constraint.declaration.item_module_def_path = String::from("crate::a::b::c");
        let mut group = TestMount::default();
        group.include_constraint(0, constraint, Some(finding("exact boundary")));
        let callers = caller_map("crate::a::b::c::item", "crate::a::caller");

        assert!(group.render(&callers, "/package").is_none());
    }

    #[test]
    fn facade_less_parent_boundary_is_accepted_for_parent_callers() {
        let constraint = constraint(
            StoredFacadeConstraint::Impossible,
            Some(StoredVisibilityReach::Crate),
        );
        let mut group = TestMount::default();
        group.include_constraint(0, constraint, Some(finding("parent boundary")));
        let callers = caller_map("crate::a::b::item", "crate::a::caller");

        assert!(group.render(&callers, "/package").is_none());
    }

    #[test]
    fn facade_less_crate_visibility_rewrites_to_the_cross_target_caller_boundary() {
        let mut constraint = constraint(
            StoredFacadeConstraint::Impossible,
            Some(StoredVisibilityReach::Crate),
        );
        constraint.diagnostic_code = DiagnosticCode::OverbroadPubCrate;
        constraint.facts.visibility_annotation = String::from("pub(crate)");
        constraint.facts.declared_reach = StoredVisibilityReach::Crate;
        constraint.facts.spelling = StoredVisibilitySpelling::Crate;
        constraint.declaration.item_def_path = String::from("crate::a::b::c::item");
        constraint.declaration.item_module_def_path = String::from("crate::a::b::c");
        let mut candidate = finding("crate visibility");
        candidate.diagnostic_code = DiagnosticCode::OverbroadPubCrate;
        candidate.fix_support = FixSupport::RestrictedAnnotation;
        let mut group = TestMount::default();
        group.include_constraint(0, constraint.clone(), Some(candidate.clone()));

        let callers = caller_map("crate::a::b::c::item", "crate::a::caller");
        let rendered = group.render(&callers, "/package");
        assert!(
            rendered.is_some(),
            "caller boundary must retain the finding"
        );
        let Some(rendered) = rendered else {
            return;
        };
        assert_eq!(
            rendered.suggestion.as_deref(),
            Some("consider using: `pub(in crate::a)`"),
        );
        assert_eq!(rendered.narrower_scope_def_path.as_deref(), Some("a"),);
        assert_eq!(rendered.fix_support, FixSupport::RestrictedAnnotation);

        let mut root_group = TestMount::default();
        root_group.include_constraint(0, constraint, Some(candidate));
        let root_callers = caller_map("crate::a::b::c::item", "crate::other");
        assert!(
            root_group.render(&root_callers, "/package").is_none(),
            "`pub(crate)` is already exact for crate-wide callers"
        );
    }

    /// The scan writes the structural headline from the one crate it compiled,
    /// where nothing reached the callers. When the cross-crate pass then
    /// resolves a boundary, that headline is stale: it denies the very
    /// visibility the finding's own `help:` line goes on to suggest.
    #[test]
    fn a_resolved_boundary_withdraws_the_headline_that_denied_one_exists() {
        let constraint = constraint(StoredFacadeConstraint::Impossible, None);
        let structural_headline = visibility::no_facade_headline(
            NoFacadeVisibilityRepair::StructuralMigrationForCallerLocations,
            String::from("replaced by the structural headline"),
        );
        let mut group = TestMount::default();
        group.include_constraint(0, constraint, Some(finding(&structural_headline)));

        let callers = caller_map("crate::a::b::item", "crate::other");
        let rendered = group.render(&callers, "/package");
        assert!(
            rendered.is_some(),
            "a cross-scope caller retains the finding"
        );
        let Some(rendered) = rendered else {
            return;
        };

        assert_eq!(
            rendered.suggestion.as_deref(),
            Some("consider using: `pub(crate)`"),
        );
        assert_eq!(
            rendered.message,
            "`pub(in crate::a)` is not the boundary this item's callers require",
        );
    }

    /// Only the structural headline is withdrawn. The other
    /// `forbidden_pub_in_crate` headlines describe why the annotation is
    /// forbidden, which resolving a boundary does not contradict.
    #[test]
    fn a_resolved_boundary_keeps_a_headline_that_never_denied_one_exists() {
        let constraint = constraint(StoredFacadeConstraint::Impossible, None);
        let mut group = TestMount::default();
        group.include_constraint(
            0,
            constraint,
            Some(finding("parent facade caps reach at `pub(crate)`")),
        );

        let callers = caller_map("crate::a::b::item", "crate::other");
        let rendered = group.render(&callers, "/package");
        assert_eq!(
            rendered.map(|finding| finding.message),
            Some(String::from("parent facade caps reach at `pub(crate)`")),
        );
    }

    #[test]
    fn facade_less_parent_boundary_upgrades_a_manual_candidate_to_pub_super() {
        let mut constraint = constraint(StoredFacadeConstraint::Impossible, None);
        constraint.diagnostic_code = DiagnosticCode::OverbroadPubCrate;
        constraint.facts.visibility_annotation = String::from("pub(crate)");
        constraint.facts.declared_reach = StoredVisibilityReach::Crate;
        constraint.facts.spelling = StoredVisibilitySpelling::Crate;
        let mut candidate = finding("crate visibility");
        candidate.diagnostic_code = DiagnosticCode::OverbroadPubCrate;
        let mut group = TestMount::default();
        group.include_constraint(0, constraint, Some(candidate));

        let callers = caller_map("crate::a::b::item", "crate::a::caller");
        let rendered = group.render(&callers, "/package");
        assert!(rendered.is_some(), "parent caller must retain the finding");
        let Some(rendered) = rendered else {
            return;
        };

        assert_eq!(
            rendered.suggestion.as_deref(),
            Some("consider using: `pub(super)`"),
        );
        assert_eq!(rendered.fix_support, FixSupport::RestrictedAnnotation);
        assert_eq!(
            rendered.visibility_annotation.as_deref(),
            Some("pub(crate)"),
        );
        assert_eq!(rendered.item_def_path.as_deref(), Some("crate::a::b::item"));
        assert_eq!(rendered.narrower_scope_def_path.as_deref(), Some("a"));
        assert_eq!(
            rendered.exact_boundary_spelling,
            ExactBoundarySpelling::Parent
        );
    }

    #[test]
    fn crate_signature_reach_accepts_a_matching_pub_crate_annotation() {
        let mut constraint = constraint(
            StoredFacadeConstraint::Absent,
            Some(StoredVisibilityReach::Crate),
        );
        constraint.diagnostic_code = DiagnosticCode::OverbroadPubCrate;
        constraint.facts.visibility_annotation = String::from("pub(crate)");
        constraint.facts.declared_reach = StoredVisibilityReach::Crate;
        constraint.facts.spelling = StoredVisibilitySpelling::Crate;
        let mut candidate = finding("crate visibility");
        candidate.diagnostic_code = DiagnosticCode::OverbroadPubCrate;
        let mut group = TestMount::default();
        group.include_constraint(0, constraint, Some(candidate));

        let callers = caller_map("crate::a::b::item", "crate::a::caller");
        assert!(
            group.render(&callers, "/package").is_none(),
            "the crate signature requirement must accept `pub(crate)`",
        );
    }

    #[test]
    fn bare_pub_rewrites_to_crate_reach_for_crate_wide_callers() {
        let mut constraint = constraint(StoredFacadeConstraint::Absent, None);
        constraint.diagnostic_code = DiagnosticCode::SuspiciousPub;
        constraint.facts.visibility_annotation = String::from("pub");
        constraint.facts.declared_reach = StoredVisibilityReach::Public;
        constraint.facts.spelling = StoredVisibilitySpelling::Public;
        let mut candidate = finding("public visibility");
        candidate.diagnostic_code = DiagnosticCode::SuspiciousPub;
        let mut group = TestMount::default();
        group.include_constraint(0, constraint, Some(candidate));
        let callers = caller_map("crate::a::b::item", "crate::other");

        let rendered = group.render(&callers, "/package");
        assert!(
            rendered.is_some(),
            "crate-wide callers must retain a narrowing finding"
        );
        let Some(rendered) = rendered else {
            return;
        };

        assert_eq!(
            rendered.suggestion.as_deref(),
            Some("consider using: `pub(crate)`"),
        );
        assert_eq!(rendered.fix_support, FixSupport::RestrictedAnnotation);
        assert_eq!(
            rendered.exact_boundary_spelling,
            ExactBoundarySpelling::Crate
        );
    }

    #[test]
    fn bare_pub_rewrites_to_an_exact_ancestor_for_sibling_callers() {
        let mut constraint = constraint(StoredFacadeConstraint::Absent, None);
        constraint.diagnostic_code = DiagnosticCode::SuspiciousPub;
        constraint.facts.visibility_annotation = String::from("pub");
        constraint.facts.declared_reach = StoredVisibilityReach::Public;
        constraint.facts.spelling = StoredVisibilitySpelling::Public;
        constraint.declaration.item_def_path = String::from("crate::a::b::c::item");
        constraint.declaration.item_module_def_path = String::from("crate::a::b::c");
        let mut candidate = finding("public visibility");
        candidate.diagnostic_code = DiagnosticCode::SuspiciousPub;
        let mut group = TestMount::default();
        group.include_constraint(0, constraint, Some(candidate));
        let callers = caller_map("crate::a::b::c::item", "crate::a::caller");

        let rendered = group.render(&callers, "/package");
        assert!(
            rendered.is_some(),
            "sibling callers must retain a narrowing finding"
        );
        let Some(rendered) = rendered else {
            return;
        };

        assert_eq!(
            rendered.suggestion.as_deref(),
            Some("consider using: `pub(in crate::a)`"),
        );
        assert_eq!(rendered.fix_support, FixSupport::RestrictedAnnotation);
    }

    fn caller_map(item_def_path: &str, caller_module_def_path: &str) -> CallerMap {
        let mut reports = vec![StoredReport {
            version:                    FINDINGS_SCHEMA_VERSION,
            analysis_fingerprint:       String::new(),
            scope_fingerprint:          String::new(),
            package_root:               String::from("/package"),
            crate_root_file:            String::from("/package/src/lib.rs"),
            config_fingerprint:         String::new(),
            source_files:               Vec::new(),
            findings:                   Vec::new(),
            visibility_constraints:     Vec::new(),
            pub_use_fix_facts:          Vec::new(),
            subtree_reexport_fix_facts: Vec::new(),
            module_mount_facts:         Vec::new(),
            all_features_coverage:      AllFeaturesCoverage::default(),
            compiler_warning_facts:     CompilerWarningFacts::None,
            use_sites:                  vec![UseSite {
                target_def_path:        item_def_path.to_string(),
                caller_module_def_path: caller_module_def_path.to_string(),
                reference:              UseSiteReference::Named,
            }],
        }];
        caller_aware::apply_caller_aware_suppression(&mut reports)
    }

    #[test]
    fn shared_source_visibility_keeps_a_later_mounts_crate_callers() {
        let mut reports = vec![
            mounted_visibility_report("/package", "crate::a::b", "crate::a::b"),
            mounted_visibility_report("/package", "crate::test_pointer", "crate"),
        ];
        let callers = caller_aware::apply_caller_aware_suppression(&mut reports);

        super::reconcile_visibility_constraints(&mut reports, &callers);

        assert!(
            reports.iter().all(|report| report.findings.is_empty()),
            "a mount needing crate visibility must prevent another mount's narrowing: {reports:#?}"
        );
    }

    #[test]
    fn shared_source_visibility_retains_current_when_exact_paths_disagree() {
        let mut reports = vec![
            mounted_visibility_report("/package", "crate::a::b::c", "crate::a::caller"),
            mounted_visibility_report("/package", "crate::x::y::z", "crate::x::caller"),
        ];
        let callers = caller_aware::apply_caller_aware_suppression(&mut reports);

        super::reconcile_visibility_constraints(&mut reports, &callers);

        assert!(
            reports.iter().all(|report| report.findings.is_empty()),
            "neither exact path is valid in both mounts: {reports:#?}"
        );
    }

    #[test]
    fn shared_source_visibility_includes_mounts_from_other_packages() {
        let mut reports = vec![
            mounted_visibility_report("/one", "crate::support::shared", "crate::support"),
            mounted_visibility_report("/two", "crate::support::shared", "crate"),
        ];
        let callers = caller_aware::apply_caller_aware_suppression(&mut reports);

        super::reconcile_visibility_constraints(&mut reports, &callers);

        assert!(
            reports.iter().all(|report| report.findings.is_empty()),
            "a source edit must satisfy callers in every package: {reports:#?}"
        );
    }

    #[test]
    fn shared_source_visibility_allows_a_common_parent_spelling() {
        let mut reports = vec![
            mounted_visibility_report("/package", "crate::a::b", "crate::a"),
            mounted_visibility_report("/package", "crate::x::y::z", "crate::x::y"),
        ];
        let callers = caller_aware::apply_caller_aware_suppression(&mut reports);

        super::reconcile_visibility_constraints(&mut reports, &callers);

        let findings: Vec<_> = reports.iter().flat_map(|report| &report.findings).collect();
        assert_eq!(findings.len(), 1, "one source gets one edit");
        assert_eq!(
            findings[0].exact_boundary_spelling,
            ExactBoundarySpelling::Parent
        );
        assert_eq!(findings[0].fix_support, FixSupport::RestrictedAnnotation);
    }

    #[test]
    fn shared_source_visibility_silent_public_mount_vetoes_narrowing() {
        let mut report =
            mounted_visibility_report("/package", "crate::internal::nested::hidden", "crate");
        let constraint = &mut report.visibility_constraints[0];
        constraint.diagnostic_code = DiagnosticCode::SuspiciousPub;
        constraint.facts.visibility_annotation = String::from("pub");
        constraint.facts.declared_reach = StoredVisibilityReach::Public;
        constraint.facts.spelling = StoredVisibilitySpelling::Public;
        constraint.facts.facade = StoredFacadeConstraint::Resolved {
            required: StoredVisibilityReach::Crate,
        };
        report.findings[0].diagnostic_code = DiagnosticCode::SuspiciousPub;
        report.module_mount_facts = ["api", "internal::nested::hidden"]
            .into_iter()
            .map(mount_fact)
            .collect();
        let mut reports = vec![report];
        let callers = caller_aware::apply_caller_aware_suppression(&mut reports);

        super::reconcile_visibility_constraints(&mut reports, &callers);

        assert_eq!(
            reports[0].findings.len(),
            0,
            "the public mount has no constraint or finding and vetoes the edit"
        );
    }

    #[test]
    fn shared_source_visibility_silent_used_mount_vetoes_unused_pub() {
        let mut report = mounted_visibility_report("/package", "crate::b", "crate::b");
        report.visibility_constraints.clear();
        let candidate = &mut report.findings[0];
        candidate.diagnostic_code = DiagnosticCode::UnusedPub;
        candidate.fix_support = FixSupport::UnusedPub;
        candidate.item_def_path = Some(String::from("crate::b::item"));
        candidate.visibility_annotation = None;
        candidate.narrower_scope_def_path = Some(String::from("crate::b"));
        report.module_mount_facts = ["a", "b"].into_iter().map(mount_fact).collect();
        report.use_sites = vec![UseSite {
            target_def_path:        String::from("crate::a::item"),
            caller_module_def_path: String::from("crate"),
            reference:              UseSiteReference::Named,
        }];
        let mut reports = vec![report];
        let callers = caller_aware::apply_caller_aware_suppression(&mut reports);

        super::reconcile_visibility_constraints(&mut reports, &callers);

        assert_eq!(
            reports[0].findings.len(),
            0,
            "unused in b cannot remove visibility used through a"
        );
    }

    #[test]
    fn shared_source_visibility_preserves_distinct_manual_diagnostics_once() {
        let mut reports: Vec<_> = ["crate::a", "crate::b", "crate::a"]
            .into_iter()
            .map(|module| {
                let mut report = mounted_visibility_report("/package", module, module);
                let constraint = &mut report.visibility_constraints[0];
                constraint.diagnostic_code = DiagnosticCode::ForbiddenPubInCrate;
                constraint.facts.visibility_annotation = String::from("pub(in self)");
                constraint.facts.spelling = StoredVisibilitySpelling::NonCanonical;
                constraint.facts.facade = StoredFacadeConstraint::Absent;
                constraint.facts.caller_reconciliation = StoredCallerReconciliation::Fixed;
                constraint.facts.exact_boundary_acceptance =
                    StoredExactBoundaryAcceptance::Ineligible;
                constraint.facts.annotation_edit_acceptance =
                    StoredExactBoundaryAcceptance::Ineligible;
                let candidate = &mut report.findings[0];
                candidate.diagnostic_code = DiagnosticCode::ForbiddenPubInCrate;
                candidate.message = String::from("manual spelling error");
                candidate.fix_support = FixSupport::None;
                report.module_mount_facts = ["a", "b"].into_iter().map(mount_fact).collect();
                report
            })
            .collect();
        let mut distinct = reports[1].findings[0].clone();
        distinct.message = String::from("another manual visibility error");
        reports[1].findings.push(distinct);
        let callers = caller_aware::apply_caller_aware_suppression(&mut reports);

        super::reconcile_visibility_constraints(&mut reports, &callers);

        let mut messages: Vec<_> = reports
            .iter()
            .flat_map(|report| &report.findings)
            .map(|finding| finding.message.as_str())
            .collect();
        messages.sort_unstable();
        assert_eq!(
            messages,
            ["another manual visibility error", "manual spelling error"]
        );
    }

    #[test]
    fn repeated_target_reports_preserve_single_mount_visibility_findings() {
        for (code, support) in [
            (
                DiagnosticCode::NarrowToPubCrate,
                FixSupport::NarrowToPubCrate,
            ),
            (
                DiagnosticCode::FieldVisibilityWiderThanType,
                FixSupport::FieldVisibility,
            ),
            (
                DiagnosticCode::InternalParentPubUseFacade,
                FixSupport::PubUse,
            ),
            (DiagnosticCode::UnusedPub, FixSupport::UnusedPub),
        ] {
            let mut reports: Vec<_> = (0..2)
                .map(|_| {
                    let mut report =
                        mounted_visibility_report("/package", "crate::helpers", "crate");
                    report.crate_root_file = String::from("/package/src/lib.rs");
                    report.visibility_constraints.clear();
                    let candidate = &mut report.findings[0];
                    candidate.diagnostic_code = code;
                    candidate.fix_support = support;
                    candidate.item_def_path = None;
                    report
                })
                .collect();
            super::reconcile_visibility_constraints(&mut reports, &CallerMap::new());
            assert_eq!(
                reports
                    .iter()
                    .map(|report| report.findings.len())
                    .sum::<usize>(),
                2,
                "recompiling one mount must preserve {code:?}/{support:?}"
            );
            assert!(
                reports
                    .iter()
                    .flat_map(|report| &report.findings)
                    .all(
                        |finding| finding.diagnostic_code == code && finding.fix_support == support
                    )
            );
        }
    }

    #[test]
    fn shared_edit_destination_vetoes_parent_facade_authorization() {
        let mut report = mounted_visibility_report("/package", "crate::actor", "crate");
        report.visibility_constraints.clear();
        let parent = "/package/src/actor/mod.rs";
        let child = "/package/src/shared.rs";
        let candidate = &mut report.findings[0];
        candidate.path = parent.to_string();
        candidate.diagnostic_code = DiagnosticCode::InternalParentPubUseFacade;
        candidate.fix_support = FixSupport::PubUse;
        candidate.item = Some(String::from("struct Item"));
        candidate.item_def_path = Some(String::from("crate::actor::shared::Item"));
        report.pub_use_fix_facts.push(StoredPubUseFixFact {
            child_path:      child.to_string(),
            child_line:      1,
            child_item_name: String::from("Item"),
            parent_path:     parent.to_string(),
            parent_line:     1,
            child_module:    String::from("shared"),
        });
        report.module_mount_facts = [
            (parent, "actor"),
            (child, "actor::shared"),
            (child, "other::nested::hidden"),
        ]
        .into_iter()
        .map(|(file, module)| {
            let mut fact = mount_fact(module);
            fact.file = file.to_string();
            fact
        })
        .collect();
        report.use_sites.push(UseSite {
            target_def_path:        String::from("crate::other::nested::hidden::Item"),
            caller_module_def_path: String::from("crate"),
            reference:              UseSiteReference::Named,
        });
        let mut reports = vec![report];
        let callers = caller_aware::apply_caller_aware_suppression(&mut reports);
        super::reconcile_visibility_constraints(&mut reports, &callers);
        assert_eq!(
            reports[0].findings.len(),
            0,
            "a parent facade must not authorize narrowing a shared child with a silent mount"
        );
    }

    fn mount_fact(module: &str) -> StoredModuleMountFact {
        StoredModuleMountFact {
            crate_root_file: String::from("/package/src/lib.rs"),
            file:            String::from("/package/src/lib.rs"),
            module_path:     module.split("::").map(str::to_string).collect(),
        }
    }

    fn mounted_visibility_report(package_root: &str, module: &str, caller: &str) -> StoredReport {
        let mut constraint = constraint(StoredFacadeConstraint::Impossible, None);
        constraint.diagnostic_code = DiagnosticCode::OverbroadPubCrate;
        constraint.facts.visibility_annotation = String::from("pub(crate)");
        constraint.facts.declared_reach = StoredVisibilityReach::Crate;
        constraint.facts.spelling = StoredVisibilitySpelling::Crate;
        constraint.declaration.item_def_path = format!("{module}::item");
        constraint.declaration.item_module_def_path = module.to_string();
        let mut candidate = finding("crate visibility");
        candidate.diagnostic_code = DiagnosticCode::OverbroadPubCrate;
        StoredReport {
            version:                    FINDINGS_SCHEMA_VERSION,
            analysis_fingerprint:       String::new(),
            scope_fingerprint:          String::new(),
            package_root:               package_root.to_string(),
            crate_root_file:            format!("{package_root}/{module}.rs"),
            config_fingerprint:         String::new(),
            source_files:               vec![constraint.source.path.clone()],
            findings:                   vec![candidate],
            use_sites:                  vec![UseSite {
                target_def_path:        constraint.declaration.item_def_path.clone(),
                caller_module_def_path: caller.to_string(),
                reference:              UseSiteReference::Named,
            }],
            visibility_constraints:     vec![constraint],
            pub_use_fix_facts:          Vec::new(),
            subtree_reexport_fix_facts: Vec::new(),
            module_mount_facts:         vec![mount_fact(
                module.strip_prefix("crate::").unwrap_or(module),
            )],
            all_features_coverage:      AllFeaturesCoverage::default(),
            compiler_warning_facts:     CompilerWarningFacts::None,
        }
    }

    fn restricted(boundary: &str) -> StoredVisibilityReach {
        StoredVisibilityReach::Restricted {
            boundary: boundary.to_string(),
        }
    }

    fn constraint(
        facade: StoredFacadeConstraint,
        signature_requirement: Option<StoredVisibilityReach>,
    ) -> StoredVisibilityConstraint {
        StoredVisibilityConstraint {
            diagnostic_code: DiagnosticCode::ForbiddenPubInCrate,
            source:          StoredVisibilitySource {
                path:   String::from("/package/src/lib.rs"),
                line:   1,
                column: 1,
            },
            declaration:     StoredVisibilityDeclaration {
                item_def_path:        String::from("crate::a::b::item"),
                item_module_def_path: String::from("crate::a::b"),
            },
            facts:           StoredVisibilityFacts {
                visibility_annotation: String::from("pub(in crate::a)"),
                declared_reach: restricted("crate::a"),
                interface_ceiling: None,
                spelling: StoredVisibilitySpelling::ExactPath,
                signature_requirement,
                facade,
                exact_boundary_acceptance: StoredExactBoundaryAcceptance::Eligible,
                annotation_edit_acceptance: StoredExactBoundaryAcceptance::Eligible,
                exact_path_policy: StoredExactPathPolicy::Allowed,
                caller_reconciliation: StoredCallerReconciliation::CallerAware,
                outcome: StoredConstraintOutcome::Finding,
            },
        }
    }

    /// A cache written by an older binary must be rejected outright, never
    /// loaded with a missing field filled in from a default. Defaulting
    /// `annotation_edit_acceptance` to `Ineligible` once made a stale cache
    /// report the pre-fix severity, which reads as an unfixed finding rather
    /// than an unrefreshed cache. Adding, removing, or renaming a field changes
    /// what an older cache can supply, so this list and
    /// `FINDINGS_SCHEMA_VERSION` move together.
    #[test]
    fn changing_the_stored_constraint_shape_requires_a_schema_version_bump() {
        let stored = serde_json::to_value(constraint(StoredFacadeConstraint::Absent, None))
            .expect("serialize stored constraint");
        let fields: Vec<&str> = stored
            .as_object()
            .expect("stored constraint serializes to a JSON object")
            .keys()
            .map(String::as_str)
            .collect();

        assert_eq!(
            fields,
            [
                "annotation_edit_acceptance",
                "caller_reconciliation",
                "declaration",
                "declared_reach",
                "diagnostic_code",
                "exact_boundary_acceptance",
                "exact_path_policy",
                "facade",
                "interface_ceiling",
                "outcome",
                "signature_requirement",
                "source",
                "spelling",
                "visibility_annotation",
            ],
            "stored constraint fields changed; bump FINDINGS_SCHEMA_VERSION (currently \
             {FINDINGS_SCHEMA_VERSION}) so a cache written by the previous binary is rejected \
             instead of loaded"
        );
    }

    fn finding(message: &str) -> StoredFinding {
        StoredFinding {
            severity:                Severity::Error,
            diagnostic_code:         DiagnosticCode::ForbiddenPubInCrate,
            path:                    String::from("/package/src/lib.rs"),
            line:                    1,
            column:                  1,
            highlight_len:           3,
            source_line:             String::from("pub(in crate::a) fn item() {}"),
            item:                    None,
            message:                 message.to_string(),
            suggestion:              Some(String::from("suggestion")),
            fix_support:             FixSupport::None,
            related:                 None,
            visibility_annotation:   None,
            item_def_path:           None,
            narrower_scope_def_path: None,
            exact_boundary_spelling: ExactBoundarySpelling::CratePath,
        }
    }
}
