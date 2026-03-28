#![allow(clippy::expect_used)]
#![allow(clippy::needless_raw_string_hashes)]
#![allow(clippy::struct_field_names)]

#[path = "diagnostics/allowances.rs"]
mod allowances;
mod common;
#[path = "diagnostics/import_fixes.rs"]
mod import_fixes;
#[path = "diagnostics/inline_path_fixes.rs"]
mod inline_path_fixes;
#[path = "diagnostics/prefer_module_import.rs"]
mod prefer_module_import;
#[path = "diagnostics/pub_use_fixes.rs"]
mod pub_use_fixes;
#[path = "diagnostics/rendering.rs"]
mod rendering;
