use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::collections::BTreeSet;

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

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(in crate::compiler) struct StoredVisibilityConstraint {
    pub diagnostic_code:           DiagnosticCode,
    pub source:                    StoredVisibilitySource,
    pub declaration:               StoredVisibilityDeclaration,
    pub visibility_annotation:     String,
    pub declared_reach:            StoredVisibilityReach,
    pub spelling:                  StoredVisibilitySpelling,
    pub signature_requirement:     Option<StoredVisibilityReach>,
    pub facade:                    StoredFacadeConstraint,
    pub exact_boundary_acceptance: StoredExactBoundaryAcceptance,
    /// Project policy for an exact `pub(in crate::...)` replacement.
    #[serde(default)]
    pub exact_path_policy:         StoredExactPathPolicy,
    pub caller_reconciliation:     StoredCallerReconciliation,
    pub outcome:                   StoredConstraintOutcome,
}

impl StoredVisibilityConstraint {
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
    constraints: BTreeSet<StoredVisibilityConstraint>,
}

impl VisibilityConstraintSet {
    fn from_constraint(constraint: StoredVisibilityConstraint) -> Self {
        Self {
            constraints: BTreeSet::from([constraint]),
        }
    }

    fn join(mut self, additional: Self) -> Self {
        self.constraints.extend(additional.constraints);
        self
    }

    fn required_reach(&self) -> Option<StoredVisibilityReach> {
        self.constraints
            .iter()
            .filter_map(StoredVisibilityConstraint::required_reach)
            .reduce(|current, additional| current.join(&additional))
    }

    fn includes_facade_blocker(&self) -> bool {
        self.constraints
            .iter()
            .any(|constraint| matches!(constraint.facade, StoredFacadeConstraint::Blocked))
    }

    fn includes_absent_facade(&self) -> bool {
        self.constraints.iter().any(|constraint| {
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
                .iter()
                .all(|constraint| matches!(constraint.facade, StoredFacadeConstraint::Impossible))
    }

    fn all_facades_are_absent_or_impossible(&self) -> bool {
        !self.constraints.is_empty()
            && self.constraints.iter().all(|constraint| {
                matches!(
                    constraint.facade,
                    StoredFacadeConstraint::Absent | StoredFacadeConstraint::Impossible
                )
            })
    }

    fn all_facades_are_resolved(&self) -> bool {
        self.constraints
            .iter()
            .all(|constraint| matches!(constraint.facade, StoredFacadeConstraint::Resolved { .. }))
    }

    fn all_exact_boundaries_are_eligible(&self) -> bool {
        self.constraints.iter().all(|constraint| {
            constraint.exact_boundary_acceptance == StoredExactBoundaryAcceptance::Eligible
        })
    }

    fn all_exact_paths_are_allowed(&self) -> bool {
        self.constraints
            .iter()
            .all(|constraint| constraint.exact_path_policy == StoredExactPathPolicy::Allowed)
    }

    fn uniform_declared_reach(&self) -> Option<StoredVisibilityReach> {
        let mut reaches = self
            .constraints
            .iter()
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
            && self.constraints.iter().all(|constraint| {
                constraint.spelling == StoredVisibilitySpelling::ExactPath
                    && matches!(constraint.facade, StoredFacadeConstraint::Absent)
                    && constraint.signature_requirement.as_ref() == Some(&constraint.declared_reach)
            })
    }

    fn uses_caller_reconciliation(&self) -> bool {
        self.constraints.iter().any(|constraint| {
            constraint.caller_reconciliation == StoredCallerReconciliation::CallerAware
        }) || (self.includes_absent_facade() && self.required_reach().is_some())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct VisibilityConstraintKey {
    package_root:    String,
    diagnostic_code: DiagnosticCode,
    source:          StoredVisibilitySource,
}

impl VisibilityConstraintKey {
    fn new(package_root: &str, constraint: &StoredVisibilityConstraint) -> Self {
        Self {
            package_root:    package_root.to_string(),
            diagnostic_code: constraint.diagnostic_code,
            source:          constraint.source.clone(),
        }
    }

    /// The key a finding would carry if it were a constraint, so a finding can
    /// be matched against constraints by lookup instead of by scanning. A
    /// constraint and a finding correspond exactly when their package root,
    /// diagnostic code, and source position agree, which is precisely this key.
    fn for_finding(package_root: &str, finding: &StoredFinding) -> Self {
        Self {
            package_root:    package_root.to_string(),
            diagnostic_code: finding.diagnostic_code,
            source:          StoredVisibilitySource {
                path:   finding.path.clone(),
                line:   finding.line,
                column: finding.column,
            },
        }
    }
}

#[derive(Clone)]
struct VisibilityFindingCandidate {
    constraint: StoredVisibilityConstraint,
    finding:    StoredFinding,
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

#[derive(Default)]
struct VisibilityConstraintGroup {
    constraints:  VisibilityConstraintSet,
    candidates:   Vec<VisibilityFindingCandidate>,
    report_index: Option<usize>,
}

/// The written annotations `fixes::restricted_annotation` knows how to rewrite.
/// A finding whose annotation is not one of these is reported, never promoted to
/// fixable.
const SUPPORTED_ANNOTATIONS: [&str; 3] = ["pub", "pub(crate)", "pub(in crate)"];

impl VisibilityConstraintGroup {
    fn include(
        &mut self,
        report_index: usize,
        constraint: StoredVisibilityConstraint,
        finding: Option<StoredFinding>,
    ) {
        self.report_index = Some(
            self.report_index
                .map_or(report_index, |current| current.min(report_index)),
        );
        if let Some(finding) = finding {
            self.candidates.push(VisibilityFindingCandidate {
                constraint: constraint.clone(),
                finding,
            });
        }
        let current = std::mem::take(&mut self.constraints);
        self.constraints = current.join(VisibilityConstraintSet::from_constraint(constraint));
    }

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
            if self.constraints.constraints.iter().all(|constraint| {
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
        let mut finding = self.preferred_candidate()?.finding.clone();
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
            && self.constraints.constraints.iter().all(|constraint| {
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
            finding.suggestion = Some(visibility::no_facade_suggestion(repair, &boundary));
        }
        Some(finding)
    }

    fn declares_effective_required_boundary(
        &self,
        callers: &CallerMap,
        package_root: &str,
        required_reach: Option<&StoredVisibilityReach>,
    ) -> bool {
        if self.candidates.iter().any(|candidate| {
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
        let declaration = self
            .constraints
            .constraints
            .first()
            .map(|constraint| &constraint.declaration)?;
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
                .iter()
                .all(|constraint| constraint.spelling == StoredVisibilitySpelling::ExactPath)
            && self
                .constraints
                .uniform_declared_reach()
                .is_some_and(|declared| {
                    declared.boundary() == self.caller_boundary_path(callers, package_root)
                })
    }

    fn callers_require_structural_repair(&self, callers: &CallerMap, package_root: &str) -> bool {
        self.constraints.constraints.iter().any(|constraint| {
            let reaching_callers = caller_aware::callers_for_package(
                callers,
                package_root,
                &constraint.declaration.item_def_path,
            )
            .map(|item_callers| &item_callers.reaching);
            matches!(
                caller_repair(&constraint.declaration, reaching_callers),
                NoFacadeVisibilityRepair::StructuralMigrationForCallerLocations
            )
        })
    }

    fn exact_boundary_spelling(&self, boundary: &str) -> ExactBoundarySpelling {
        if boundary == "crate" {
            return ExactBoundarySpelling::Crate;
        }
        if self
            .constraints
            .constraints
            .first()
            .is_some_and(|constraint| {
                let parent =
                    visibility::parent_scope_def_path(&constraint.declaration.item_module_def_path);
                !parent.is_empty() && visibility::crate_rooted_def_path(parent) == boundary
            })
        {
            ExactBoundarySpelling::Parent
        } else {
            ExactBoundarySpelling::CratePath
        }
    }

    fn parent_boundary_path(&self) -> String {
        self.constraints.constraints.first().map_or_else(
            || String::from("crate"),
            |constraint| {
                visibility::crate_rooted_def_path(visibility::parent_scope_def_path(
                    &constraint.declaration.item_module_def_path,
                ))
            },
        )
    }

    fn supports_rewrite(&self, spelling: ExactBoundarySpelling) -> bool {
        self.constraints.all_exact_boundaries_are_eligible()
            && (spelling != ExactBoundarySpelling::CratePath
                || self.constraints.all_exact_paths_are_allowed())
    }

    fn apply_rewrite(
        &self,
        finding: &mut StoredFinding,
        boundary: &str,
        spelling: ExactBoundarySpelling,
    ) {
        finding.suggestion = Some(match spelling {
            ExactBoundarySpelling::CratePath => {
                format!("consider using: `pub(in {boundary})`")
            },
            ExactBoundarySpelling::Parent => String::from("consider using: `pub(super)`"),
            ExactBoundarySpelling::Crate => String::from("consider using: `pub(crate)`"),
            ExactBoundarySpelling::Private => String::from("consider removing the visibility"),
            ExactBoundarySpelling::Public => String::from("consider using: `pub`"),
        });
        finding.message = match finding.diagnostic_code {
            DiagnosticCode::OverbroadPubCrate => {
                String::from("`pub(crate)` is broader than required")
            },
            DiagnosticCode::SuspiciousPub => format!(
                "the narrowest proven visibility is {}",
                match spelling {
                    ExactBoundarySpelling::CratePath => format!("`pub(in {boundary})`"),
                    ExactBoundarySpelling::Parent => String::from("`pub(super)`"),
                    ExactBoundarySpelling::Crate => String::from("`pub(crate)`"),
                    ExactBoundarySpelling::Private => String::from("private"),
                    ExactBoundarySpelling::Public => String::from("`pub`"),
                }
            ),
            _ => finding.message.clone(),
        };
        // Only claim fixable when `fixes::restricted_annotation` will actually
        // rewrite this annotation. It edits a bare `pub`, `pub(crate)`, and
        // `pub(in crate)` and nothing else, so promoting a
        // `pub(in crate::a::b)` finding advertised a fix that never arrived:
        // every run reported it as fixable, applied nothing, and reported it
        // again. Widening the applier instead was tried and rolled back — a
        // boundary resolved from callers is not a compiling rewrite for an
        // already-restricted annotation.
        finding.fix_support = if self
            .constraints
            .constraints
            .first()
            .is_some_and(|constraint| {
                SUPPORTED_ANNOTATIONS.contains(&constraint.visibility_annotation.as_str())
            }) {
            FixSupport::RestrictedAnnotation
        } else {
            FixSupport::None
        };
        if let Some(constraint) = self.constraints.constraints.first() {
            finding.visibility_annotation = Some(constraint.visibility_annotation.clone());
            finding.item_def_path = Some(constraint.declaration.item_def_path.clone());
        }
        finding.narrower_scope_def_path = Some(match spelling {
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
        let caller_reach = if caller_boundary == "crate" {
            StoredVisibilityReach::Crate
        } else {
            StoredVisibilityReach::Restricted {
                boundary: caller_boundary,
            }
        };
        required_reach.map_or_else(
            || caller_reach.boundary().to_string(),
            |required| required.join(&caller_reach).boundary().to_string(),
        )
    }

    fn public_candidate(&self) -> Option<StoredFinding> {
        let mut finding = self
            .candidates
            .iter()
            .filter(|candidate| {
                candidate.constraint.required_reach() == Some(StoredVisibilityReach::Public)
            })
            .min_by(|left, right| candidate_order(left, right))
            .or_else(|| {
                self.candidates
                    .iter()
                    .min_by(|left, right| candidate_order(left, right))
            })
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
        self.candidates
            .iter()
            .filter(|candidate| {
                matches!(candidate.constraint.facade, StoredFacadeConstraint::Blocked)
            })
            .min_by(|left, right| candidate_order(left, right))
            .or_else(|| {
                self.candidates
                    .iter()
                    .min_by(|left, right| candidate_order(left, right))
            })
            .map(|candidate| candidate.finding.clone())
    }

    fn facade_cleanup_candidate(&self) -> Option<StoredFinding> {
        self.candidates
            .iter()
            .filter(|candidate| {
                matches!(
                    candidate.finding.fix_support,
                    FixSupport::PubUse | FixSupport::NeedsManualPubUseCleanup
                )
            })
            .min_by(|left, right| candidate_order(left, right))
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

    fn preferred_candidate(&self) -> Option<&VisibilityFindingCandidate> {
        self.candidates.iter().min_by(|left, right| {
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
        let demanded_beyond_signature = self.constraints.constraints.iter().any(|constraint| {
            // A boundary a narrower canonical annotation already reaches is a
            // spelling finding, not a facade demand, so it stays reported.
            if !matches!(
                requirement_repair(&constraint.declaration, required_reach),
                NoFacadeVisibilityRepair::StructuralMigrationForCallerLocations
            ) {
                return true;
            }
            let naming_callers = caller_aware::callers_for_package(
                callers,
                package_root,
                &constraint.declaration.item_def_path,
            )
            .map(|item_callers| &item_callers.naming);
            matches!(
                caller_repair(&constraint.declaration, naming_callers),
                NoFacadeVisibilityRepair::StructuralMigrationForCallerLocations
            )
        });
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
        let mut repair = NoFacadeVisibilityRepair::RemoveAnnotation;
        for constraint in &self.constraints.constraints {
            if !self.constraints.all_facades_are_impossible() {
                repair = repair
                    .most_invasive(requirement_repair(&constraint.declaration, required_reach));
            }
            let reaching_callers = caller_aware::callers_for_package(
                callers,
                package_root,
                &constraint.declaration.item_def_path,
            )
            .map(|item_callers| &item_callers.reaching);
            repair = repair.most_invasive(caller_repair(&constraint.declaration, reaching_callers));
        }
        repair
    }

    fn caller_boundary_path(&self, callers: &CallerMap, package_root: &str) -> String {
        let Some(declaration) = self
            .constraints
            .constraints
            .first()
            .map(|constraint| &constraint.declaration)
        else {
            return String::from("crate");
        };
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

pub(super) fn reconcile_visibility_constraints(reports: &mut [StoredReport], callers: &CallerMap) {
    let mut groups: BTreeMap<VisibilityConstraintKey, VisibilityConstraintGroup> = BTreeMap::new();
    for (report_index, report) in reports.iter().enumerate() {
        // Index the findings once per report rather than searching them for
        // every constraint: both counts grow with project size, so the search
        // costs the product of the two.
        let mut findings_by_key: BTreeMap<VisibilityConstraintKey, &StoredFinding> =
            BTreeMap::new();
        for finding in &report.findings {
            findings_by_key
                .entry(VisibilityConstraintKey::for_finding(
                    &report.package_root,
                    finding,
                ))
                .or_insert(finding);
        }
        for constraint in &report.visibility_constraints {
            let key = VisibilityConstraintKey::new(&report.package_root, constraint);
            let finding = if constraint.outcome == StoredConstraintOutcome::Finding {
                findings_by_key.get(&key).copied().cloned()
            } else {
                None
            };
            groups
                .entry(key)
                .or_default()
                .include(report_index, constraint.clone(), finding);
        }
    }

    let reconciled_keys = groups
        .iter()
        .filter(|(_, group)| !group.candidates.is_empty())
        .map(|(key, _)| key.clone())
        .collect::<BTreeSet<_>>();
    for report in reports.iter_mut() {
        let package_root = report.package_root.clone();
        report.findings.retain(|finding| {
            !reconciled_keys.contains(&VisibilityConstraintKey::for_finding(
                &package_root,
                finding,
            ))
        });
    }

    for (key, group) in groups {
        let Some(report_index) = group.report_index else {
            continue;
        };
        if let Some(finding) = group.render(callers, &key.package_root) {
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
mod tests {
    use super::CallerMap;
    use super::StoredCallerReconciliation;
    use super::StoredConstraintOutcome;
    use super::StoredExactBoundaryAcceptance;
    use super::StoredExactPathPolicy;
    use super::StoredFacadeConstraint;
    use super::StoredVisibilityConstraint;
    use super::StoredVisibilityDeclaration;
    use super::StoredVisibilityReach;
    use super::StoredVisibilitySource;
    use super::StoredVisibilitySpelling;
    use super::VisibilityConstraintGroup;
    use super::VisibilityConstraintSet;
    use crate::compiler::constants::FINDINGS_SCHEMA_VERSION;
    use crate::compiler::persistence::StoredFinding;
    use crate::compiler::persistence::StoredReport;
    use crate::compiler::persistence::caller_aware;
    use crate::compiler::persistence::schema::UseSite;
    use crate::compiler::persistence::schema::UseSiteReference;
    use crate::config::DiagnosticCode;
    use crate::reporting::AllFeaturesCoverage;
    use crate::reporting::CompilerWarningFacts;
    use crate::reporting::ExactBoundarySpelling;
    use crate::reporting::FixSupport;
    use crate::reporting::Severity;

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
            let mut group = VisibilityConstraintGroup::default();
            for constraint in constraints {
                let message =
                    if constraint.signature_requirement == Some(StoredVisibilityReach::Public) {
                        "public requirement"
                    } else {
                        "facade blocker"
                    };
                group.include(0, constraint, Some(finding(message)));
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
        let mut group = VisibilityConstraintGroup::default();
        for boundary in ["crate::left", "crate::right"] {
            let mut constraint = constraint(
                StoredFacadeConstraint::Resolved {
                    required: restricted(boundary),
                },
                None,
            );
            constraint.declared_reach = StoredVisibilityReach::Crate;
            constraint.spelling = StoredVisibilitySpelling::Crate;
            group.include(0, constraint, Some(finding(boundary)));
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
        constraint.outcome = StoredConstraintOutcome::Accepted;
        let mut group = VisibilityConstraintGroup::default();
        group.include(0, constraint, None);

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
        let mut group = VisibilityConstraintGroup::default();
        group.include(0, constraint, Some(finding("exact boundary")));
        let callers = caller_map("crate::a::b::c::item", "crate::a::caller");

        assert!(group.render(&callers, "/package").is_none());
    }

    #[test]
    fn facade_less_parent_boundary_is_accepted_for_parent_callers() {
        let constraint = constraint(
            StoredFacadeConstraint::Impossible,
            Some(StoredVisibilityReach::Crate),
        );
        let mut group = VisibilityConstraintGroup::default();
        group.include(0, constraint, Some(finding("parent boundary")));
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
        constraint.visibility_annotation = String::from("pub(crate)");
        constraint.declared_reach = StoredVisibilityReach::Crate;
        constraint.spelling = StoredVisibilitySpelling::Crate;
        constraint.declaration.item_def_path = String::from("crate::a::b::c::item");
        constraint.declaration.item_module_def_path = String::from("crate::a::b::c");
        let mut candidate = finding("crate visibility");
        candidate.diagnostic_code = DiagnosticCode::OverbroadPubCrate;
        candidate.fix_support = FixSupport::RestrictedAnnotation;
        let mut group = VisibilityConstraintGroup::default();
        group.include(0, constraint.clone(), Some(candidate.clone()));

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

        let mut root_group = VisibilityConstraintGroup::default();
        root_group.include(0, constraint, Some(candidate));
        let root_callers = caller_map("crate::a::b::c::item", "crate::other");
        assert!(
            root_group.render(&root_callers, "/package").is_none(),
            "`pub(crate)` is already exact for crate-wide callers"
        );
    }

    #[test]
    fn facade_less_parent_boundary_upgrades_a_manual_candidate_to_pub_super() {
        let mut constraint = constraint(StoredFacadeConstraint::Impossible, None);
        constraint.diagnostic_code = DiagnosticCode::OverbroadPubCrate;
        constraint.visibility_annotation = String::from("pub(crate)");
        constraint.declared_reach = StoredVisibilityReach::Crate;
        constraint.spelling = StoredVisibilitySpelling::Crate;
        let mut candidate = finding("crate visibility");
        candidate.diagnostic_code = DiagnosticCode::OverbroadPubCrate;
        let mut group = VisibilityConstraintGroup::default();
        group.include(0, constraint, Some(candidate));

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
        constraint.visibility_annotation = String::from("pub(crate)");
        constraint.declared_reach = StoredVisibilityReach::Crate;
        constraint.spelling = StoredVisibilitySpelling::Crate;
        let mut candidate = finding("crate visibility");
        candidate.diagnostic_code = DiagnosticCode::OverbroadPubCrate;
        let mut group = VisibilityConstraintGroup::default();
        group.include(0, constraint, Some(candidate));

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
        constraint.visibility_annotation = String::from("pub");
        constraint.declared_reach = StoredVisibilityReach::Public;
        constraint.spelling = StoredVisibilitySpelling::Public;
        let mut candidate = finding("public visibility");
        candidate.diagnostic_code = DiagnosticCode::SuspiciousPub;
        let mut group = VisibilityConstraintGroup::default();
        group.include(0, constraint, Some(candidate));
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
        constraint.visibility_annotation = String::from("pub");
        constraint.declared_reach = StoredVisibilityReach::Public;
        constraint.spelling = StoredVisibilitySpelling::Public;
        constraint.declaration.item_def_path = String::from("crate::a::b::c::item");
        constraint.declaration.item_module_def_path = String::from("crate::a::b::c");
        let mut candidate = finding("public visibility");
        candidate.diagnostic_code = DiagnosticCode::SuspiciousPub;
        let mut group = VisibilityConstraintGroup::default();
        group.include(0, constraint, Some(candidate));
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
            version:                FINDINGS_SCHEMA_VERSION,
            analysis_fingerprint:   String::new(),
            scope_fingerprint:      String::new(),
            package_root:           String::from("/package"),
            crate_root_file:        String::from("/package/src/lib.rs"),
            config_fingerprint:     String::new(),
            source_files:           Vec::new(),
            findings:               Vec::new(),
            visibility_constraints: Vec::new(),
            pub_use_fix_facts:      Vec::new(),
            all_features_coverage:  AllFeaturesCoverage::default(),
            compiler_warning_facts: CompilerWarningFacts::None,
            use_sites:              vec![UseSite {
                target_def_path:        item_def_path.to_string(),
                caller_module_def_path: caller_module_def_path.to_string(),
                reference:              UseSiteReference::Named,
            }],
        }];
        caller_aware::apply_caller_aware_suppression(&mut reports)
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
            source: StoredVisibilitySource {
                path:   String::from("/package/src/lib.rs"),
                line:   1,
                column: 1,
            },
            declaration: StoredVisibilityDeclaration {
                item_def_path:        String::from("crate::a::b::item"),
                item_module_def_path: String::from("crate::a::b"),
            },
            visibility_annotation: String::from("pub(in crate::a)"),
            declared_reach: restricted("crate::a"),
            spelling: StoredVisibilitySpelling::ExactPath,
            signature_requirement,
            facade,
            exact_boundary_acceptance: StoredExactBoundaryAcceptance::Eligible,
            exact_path_policy: StoredExactPathPolicy::Allowed,
            caller_reconciliation: StoredCallerReconciliation::CallerAware,
            outcome: StoredConstraintOutcome::Finding,
        }
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
