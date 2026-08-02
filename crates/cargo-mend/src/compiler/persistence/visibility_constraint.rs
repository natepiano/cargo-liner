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
    Resolved { required: StoredVisibilityReach },
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(in crate::compiler) enum StoredExactBoundaryAcceptance {
    Eligible,
    Ineligible,
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
    pub caller_reconciliation:     StoredCallerReconciliation,
    pub outcome:                   StoredConstraintOutcome,
}

impl StoredVisibilityConstraint {
    pub(in crate::compiler) fn required_reach(&self) -> Option<StoredVisibilityReach> {
        let facade_requirement = match &self.facade {
            StoredFacadeConstraint::Resolved { required } => Some(required.clone()),
            StoredFacadeConstraint::Absent | StoredFacadeConstraint::Blocked => None,
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
        self.constraints
            .iter()
            .any(|constraint| matches!(constraint.facade, StoredFacadeConstraint::Absent))
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

    fn uniform_declared_reach(&self) -> Option<StoredVisibilityReach> {
        let mut reaches = self
            .constraints
            .iter()
            .map(|constraint| &constraint.declared_reach);
        let first = reaches.next()?.clone();
        reaches.all(|reach| *reach == first).then_some(first)
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

    fn matches_finding(&self, package_root: &str, finding: &StoredFinding) -> bool {
        self.package_root == package_root
            && self.diagnostic_code == finding.diagnostic_code
            && self.source.path == finding.path
            && self.source.line == finding.line
            && self.source.column == finding.column
    }
}

#[derive(Clone)]
struct VisibilityFindingCandidate {
    constraint: StoredVisibilityConstraint,
    finding:    StoredFinding,
}

#[derive(Default)]
struct VisibilityConstraintGroup {
    constraints:  VisibilityConstraintSet,
    candidates:   Vec<VisibilityFindingCandidate>,
    report_index: Option<usize>,
}

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
        if required_reach == Some(StoredVisibilityReach::Public) {
            return self.public_candidate();
        }
        if self.constraints.includes_facade_blocker() {
            return self.blocker_candidate();
        }
        if self.constraints.all_facades_are_resolved() {
            return self.render_resolved_facades(required_reach.as_ref());
        }
        let mut finding = self.preferred_candidate()?.finding.clone();
        if !self.constraints.uses_caller_reconciliation() {
            return Some(finding);
        }
        let repair = self.no_facade_repair(callers, package_root, required_reach.as_ref());
        let repair_reach = required_reach.or_else(|| self.constraints.uniform_declared_reach());
        let boundary = repair_reach
            .as_ref()
            .map_or("crate", StoredVisibilityReach::boundary);
        finding.message = visibility::no_facade_headline(repair, finding.message);
        finding.suggestion = Some(visibility::no_facade_suggestion(repair, boundary));
        Some(finding)
    }

    fn public_candidate(&self) -> Option<StoredFinding> {
        self.candidates
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
            .map(|candidate| candidate.finding.clone())
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

    fn render_resolved_facades(
        &self,
        required_reach: Option<&StoredVisibilityReach>,
    ) -> Option<StoredFinding> {
        let required_reach = required_reach?;
        let declared_reach = self.constraints.uniform_declared_reach();
        if declared_reach.as_ref() == Some(required_reach)
            && self.constraints.all_exact_boundaries_are_eligible()
        {
            return None;
        }
        let mut finding = self.preferred_candidate()?.finding.clone();
        if declared_reach.as_ref() != Some(required_reach) {
            finding.suggestion = Some(format!("consider using: `{}`", required_reach.to_source()));
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

    fn no_facade_repair(
        &self,
        callers: &CallerMap,
        package_root: &str,
        required_reach: Option<&StoredVisibilityReach>,
    ) -> NoFacadeVisibilityRepair {
        let mut repair = NoFacadeVisibilityRepair::RemoveAnnotation;
        for constraint in &self.constraints.constraints {
            repair =
                repair.most_invasive(requirement_repair(&constraint.declaration, required_reach));
            let item_callers = caller_aware::callers_for_package(
                callers,
                package_root,
                &constraint.declaration.item_def_path,
            );
            repair = repair.most_invasive(caller_repair(&constraint.declaration, item_callers));
        }
        repair
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
        for constraint in &report.visibility_constraints {
            let key = VisibilityConstraintKey::new(&report.package_root, constraint);
            let finding = if constraint.outcome == StoredConstraintOutcome::Finding {
                report
                    .findings
                    .iter()
                    .find(|finding| key.matches_finding(&report.package_root, finding))
                    .cloned()
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
            !reconciled_keys
                .iter()
                .any(|key| key.matches_finding(&package_root, finding))
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
    use super::StoredFacadeConstraint;
    use super::StoredVisibilityConstraint;
    use super::StoredVisibilityDeclaration;
    use super::StoredVisibilityReach;
    use super::StoredVisibilitySource;
    use super::StoredVisibilitySpelling;
    use super::VisibilityConstraintGroup;
    use super::VisibilityConstraintSet;
    use crate::compiler::persistence::StoredFinding;
    use crate::config::DiagnosticCode;
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
        }
    }
}
