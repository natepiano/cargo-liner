use anyhow::Result;

use super::imports;
use super::imports_at_top;
use super::inline_path_qualified_type;
use super::prefer_module_import;
use crate::reporting::Finding;
use crate::rust_syntax::ParsedSources;
use crate::selection::Selection;

/// Run the real syntax scans after a fixture's compiler analysis.
pub(crate) fn collect_fixture_syntax_findings(selection: &Selection) -> Result<Vec<Finding>> {
    let sources = ParsedSources::for_selection(selection);
    let mut findings = imports::scan_selection(selection, &sources)?.findings;
    findings.extend(prefer_module_import::scan_selection(selection, &sources)?.findings);
    findings.extend(inline_path_qualified_type::scan_selection(selection, &sources)?.findings);
    findings.extend(imports_at_top::scan_selection(selection, &sources)?.findings);
    Ok(findings)
}
