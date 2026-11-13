use anyhow::Result;

use super::FixScans;
use super::MendRunner;
use crate::fixes::imports;
use crate::fixes::inline_path_qualified_type;
use crate::fixes::prefer_module_import;
use crate::rust_syntax::ParsedSources;
use crate::selection::Selection;

/// Use the runner's production plan to combine both syntax scans.
pub(crate) fn apply_fixture_module_imports(selection: &Selection) -> Result<()> {
    let sources = ParsedSources::for_selection(selection);
    let module_imports = prefer_module_import::scan_selection(selection, &sources)?;
    let imports = imports::scan_selection(selection, &sources)?;
    apply_fixture_plan(
        selection,
        &sources,
        FixScans {
            imports: Some(&imports),
            module_imports: Some(&module_imports),
            ..FixScans::default()
        },
    )
}

/// Use the same production plan for type paths and import shortening.
pub(crate) fn apply_fixture_inline_paths(selection: &Selection) -> Result<()> {
    let sources = ParsedSources::for_selection(selection);
    let inline_types = inline_path_qualified_type::scan_selection(selection, &sources)?;
    let imports = imports::scan_selection(selection, &sources)?;
    apply_fixture_plan(
        selection,
        &sources,
        FixScans {
            imports: Some(&imports),
            inline_types: Some(&inline_types),
            ..FixScans::default()
        },
    )
}

fn apply_fixture_plan(
    selection: &Selection,
    sources: &ParsedSources,
    scans: FixScans<'_>,
) -> Result<()> {
    let plan = MendRunner::combined_function_plan(
        scans,
        sources,
        &selection.package_roots,
        &selection.workspace_root,
    )
    .map_err(|error| anyhow::anyhow!("{error}"))?;
    imports::apply_fixes(&plan.fixes)?;
    Ok(())
}
