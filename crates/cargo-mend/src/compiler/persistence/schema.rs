use serde::Deserialize;
use serde::Serialize;

use super::visibility_constraint::StoredVisibilityConstraint;
use crate::config::DiagnosticCode;
use crate::reporting::AllFeaturesCoverage;
use crate::reporting::CompilerWarningFacts;
use crate::reporting::ExactBoundarySpelling;
use crate::reporting::FixSupport;
use crate::reporting::ModuleForm;
use crate::reporting::ReexportVisibility;
use crate::reporting::Severity;

#[derive(Debug, Serialize, Deserialize)]
pub(in crate::compiler) struct StoredReport {
    pub version:                    u32,
    #[serde(default)]
    pub analysis_fingerprint:       String,
    #[serde(default)]
    pub scope_fingerprint:          String,
    pub package_root:               String,
    #[serde(default)]
    pub crate_root_file:            String,
    pub config_fingerprint:         String,
    /// Canonical source paths containing HIR items compiled for this target.
    #[serde(default)]
    pub source_files:               Vec<String>,
    pub findings:                   Vec<StoredFinding>,
    #[serde(default)]
    pub visibility_constraints:     Vec<StoredVisibilityConstraint>,
    #[serde(default)]
    pub pub_use_fix_facts:          Vec<StoredPubUseFixFact>,
    #[serde(default)]
    pub all_features_coverage:      AllFeaturesCoverage,
    #[serde(default, rename = "compiler_warnings")]
    pub compiler_warning_facts:     CompilerWarningFacts,
    #[serde(default)]
    pub use_sites:                  Vec<UseSite>,
    #[serde(default)]
    pub subtree_reexport_fix_facts: Vec<StoredSubtreeReexportFixFact>,
    #[serde(default)]
    pub module_mount_facts:         Vec<StoredModuleMountFact>,
}

/// How a caller reaches the target. A caller that writes a path to the item can
/// be served by a re-export of it; a caller that reaches it only through the
/// signature of some other item it named cannot, because that reach travels the
/// exposing item's path and a re-export of the target would have no user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(in crate::compiler) enum UseSiteReference {
    Named,
    ThroughSignature,
    PrivateImport,
    RestrictedImport,
    /// No caller at all: the declaration's own signature names the item, so
    /// rustc requires it to stay visible wherever that declaration is reachable
    /// (`private_interfaces`). It caps how far the annotation may narrow, but
    /// unlike the other kinds it never proves anyone uses the item, so it must
    /// not suppress a narrowing finding.
    DeclarationInterface,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(in crate::compiler) struct UseSite {
    /// Canonical def-path of the referenced item, e.g.
    /// `crate::tui::panes::cpu::cpu_required_pane_height`.
    pub target_def_path:        String,
    /// Canonical def-path of the module containing the call site, e.g.
    /// `crate::tui::render::tests`.
    pub caller_module_def_path: String,
    pub reference:              UseSiteReference,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(in crate::compiler) struct StoredFinding {
    pub severity:                Severity,
    pub diagnostic_code:         DiagnosticCode,
    pub path:                    String,
    pub line:                    usize,
    pub column:                  usize,
    pub highlight_len:           usize,
    pub source_line:             String,
    /// A rendering of the item this finding is about, not structured data:
    /// `"{kind_label} {name}"`. Build it with [`StoredFinding::render_item`]
    /// and read the bare name back with [`StoredFinding::item_name`] — those
    /// two are the only sanctioned way to cross this format, so a change to
    /// one side cannot silently break a consumer of the other.
    pub item:                    Option<String>,
    pub message:                 String,
    pub suggestion:              Option<String>,
    #[serde(default, rename = "fixability")]
    pub fix_support:             FixSupport,
    #[serde(default)]
    pub related:                 Option<String>,
    /// The complete source text of a visibility annotation. This is separate
    /// from `source_line`, which contains only the physical line shown in a
    /// diagnostic and therefore cannot represent a multiline annotation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visibility_annotation:   Option<String>,
    /// Canonical def-path of the item this finding is about. Set on
    /// narrowing-style findings so cross-compilation merge can look up the
    /// item's callers post-hoc and suppress findings that would break the
    /// build under the proposed narrower visibility.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_def_path:           Option<String>,
    /// Canonical def-path of the proposed narrower scope. For a finding
    /// suggesting `pub(super)`, this is the parent module's def-path. The
    /// finding is suppressed if any caller's module is not a descendant of
    /// this scope.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub narrower_scope_def_path: Option<String>,
    /// Selects the source syntax for an exact boundary rewrite.
    #[serde(default)]
    pub exact_boundary_spelling: ExactBoundarySpelling,
}

impl StoredFinding {
    /// Composes [`StoredFinding::item`] from the pieces every producer has on
    /// hand: the item's kind label (`"struct"`, `"fn"`, `"field"`, …) and its
    /// name.
    pub(in crate::compiler) fn render_item(kind_label: &str, name: &str) -> String {
        format!("{kind_label} {name}")
    }

    /// Recovers the bare item name from an [`StoredFinding::item`] rendering
    /// built by [`StoredFinding::render_item`]. `load::discard_fix_facts_for_suppressed_findings`
    /// joins findings to `StoredPubUseFixFact::child_item_name` through this,
    /// so it must stay the exact inverse of `render_item`.
    pub(in crate::compiler) fn item_name(item: &str) -> &str {
        item.rsplit_once(' ').map_or(item, |(_, name)| name)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub(in crate::compiler) struct StoredPubUseFixFact {
    pub child_path:      String,
    pub child_line:      usize,
    pub child_item_name: String,
    pub parent_path:     String,
    pub parent_line:     usize,
    pub child_module:    String,
}

/// Stored twin of `reporting::SubtreeReexportFixFact`, with absolute paths.
/// `use_path`, `use_line`, and `use_column` equal the `path`, `line`, and
/// `column` of the `pub_use_outside_subtree` finding the fact belongs to;
/// `load::discard_fix_facts_for_suppressed_findings` joins them on that.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(in crate::compiler) struct StoredSubtreeReexportFixFact {
    pub use_path:          String,
    pub use_line:          usize,
    pub use_column:        usize,
    pub exported_name:     String,
    pub written_path:      String,
    pub owner_module:      Vec<String>,
    pub owner_module_form: ModuleForm,
    pub source_module:     Vec<String>,
    pub target_scope:      Vec<String>,
    pub common_ancestor:   Vec<String>,
    pub crate_root_file:   String,
    pub ancestor_reexport: StoredAncestorReexport,
}

/// Stored twin of `reporting::AncestorReexport`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(in crate::compiler) enum StoredAncestorReexport {
    NotRequired,
    Insert(StoredAncestorReexportInsertion),
}

/// Stored twin of `reporting::AncestorReexportInsertion`, with an absolute
/// `file`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(in crate::compiler) struct StoredAncestorReexportInsertion {
    pub reexport_visibility: ReexportVisibility,
    pub relative_path:       String,
    pub cfg_attributes:      Vec<String>,
    pub file:                String,
    pub offset:              usize,
    pub indent:              String,
}

/// Stored twin of `reporting::ModuleMountFact`, with absolute paths.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(in crate::compiler) struct StoredModuleMountFact {
    pub crate_root_file: String,
    pub file:            String,
    pub module_path:     Vec<String>,
}
