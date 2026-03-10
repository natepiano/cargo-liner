#![allow(clippy::expect_used)]
#![allow(clippy::needless_raw_string_hashes)]
#![allow(clippy::struct_field_names)]
#![allow(clippy::too_many_lines)]

#[path = "diagnostics/allowances.rs"]
mod allowances;
mod common;
#[path = "diagnostics/import_fixes.rs"]
mod import_fixes;
#[path = "diagnostics/pub_use_fixes.rs"]
mod pub_use_fixes;
#[path = "diagnostics/rendering.rs"]
mod rendering;
