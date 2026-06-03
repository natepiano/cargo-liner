mod function_imports;
mod inline_calls;
mod references;
mod scan;
mod support;

pub(crate) use scan::PreferModuleImportScan;
pub(crate) use scan::scan_selection;
