mod function_imports;
mod inline_calls;
mod references;
mod scan;
mod shared;

pub(crate) use scan::PreferModuleImportScan;
pub(crate) use scan::scan_selection;
