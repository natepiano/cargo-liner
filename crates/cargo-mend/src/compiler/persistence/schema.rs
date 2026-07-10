use serde::Deserialize;
use serde::Serialize;

use crate::config::DiagnosticCode;
use crate::reporting::CompilerWarningFacts;
use crate::reporting::FixSupport;
use crate::reporting::Severity;

#[derive(Debug, Serialize, Deserialize)]
pub struct StoredReport {
    pub version:                u32,
    #[serde(default)]
    pub analysis_fingerprint:   String,
    #[serde(default)]
    pub scope_fingerprint:      String,
    pub package_root:           String,
    #[serde(default)]
    pub crate_root_file:        String,
    pub config_fingerprint:     String,
    /// Canonical source paths containing HIR items compiled for this target.
    #[serde(default)]
    pub source_files:           Vec<String>,
    pub findings:               Vec<StoredFinding>,
    #[serde(default)]
    pub pub_use_fix_facts:      Vec<StoredPubUseFixFact>,
    #[serde(default, rename = "compiler_warnings")]
    pub compiler_warning_facts: CompilerWarningFacts,
    #[serde(default)]
    pub use_sites:              Vec<UseSite>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UseSite {
    /// Canonical def-path of the referenced item, e.g.
    /// `crate::tui::panes::cpu::cpu_required_pane_height`.
    pub target_def_path:        String,
    /// Canonical def-path of the module containing the call site, e.g.
    /// `crate::tui::render::tests`.
    pub caller_module_def_path: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StoredFinding {
    pub severity:                Severity,
    pub diagnostic_code:         DiagnosticCode,
    pub path:                    String,
    pub line:                    usize,
    pub column:                  usize,
    pub highlight_len:           usize,
    pub source_line:             String,
    pub item:                    Option<String>,
    pub message:                 String,
    pub suggestion:              Option<String>,
    #[serde(default, rename = "fixability")]
    pub fix_support:             FixSupport,
    #[serde(default)]
    pub related:                 Option<String>,
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
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StoredPubUseFixFact {
    pub child_path:      String,
    pub child_line:      usize,
    pub child_item_name: String,
    pub parent_path:     String,
    pub parent_line:     usize,
    pub child_module:    String,
}
