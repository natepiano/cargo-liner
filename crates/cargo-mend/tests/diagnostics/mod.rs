#![allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
#![allow(
    clippy::unwrap_used,
    reason = "tests should panic on unexpected values"
)]
#![allow(clippy::panic, reason = "tests should panic on unexpected values")]
#![allow(
    clippy::needless_raw_string_hashes,
    reason = "test fixtures use raw strings with varying hash counts for readability"
)]

mod allowances;
#[path = "../common/mod.rs"]
mod common;
mod field_visibility_wider_than_type;
mod import_fixes;
mod inline_path_fixes;
mod narrow_pub_crate;
mod prefer_module_import;
mod pub_use_fixes;
mod rendering;
