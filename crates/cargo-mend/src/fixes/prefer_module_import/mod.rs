mod descendant_imports;
mod function_imports;
mod inline_calls;
mod preserved_imports;
mod references;
mod scan;
mod support;

pub(crate) use scan::PreferModuleImportScan;
pub(crate) use scan::scan_selection;
