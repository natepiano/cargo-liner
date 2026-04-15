#![allow(
    clippy::unwrap_used,
    reason = "tests should panic on unexpected values"
)]

use std::fs;
use std::path::Path;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use super::build;
use super::build::BuildOutputMode;
use super::build::DiagnosticBlockKind;
use super::facade;
use super::facade::ParentFacadeExports;
use super::facade::ParentFacadeVisibility;
use super::source_cache;
use super::visibility;
use super::visibility::CrateKind;
use super::visibility::ModuleLocation;
use super::visibility::ParentVisibility;
use crate::diagnostics::CompilerWarningFacts;

#[test]
fn allow_pub_crate_allows_library_crate_root_items() {
    assert!(visibility::allow_pub_crate_by_policy(
        CrateKind::Library,
        ModuleLocation::CrateRoot,
        ParentVisibility::Public
    ));
}

#[test]
fn allow_pub_crate_allows_top_level_private_library_modules() {
    assert!(visibility::allow_pub_crate_by_policy(
        CrateKind::Library,
        ModuleLocation::TopLevelPrivateModule,
        ParentVisibility::Private
    ));
}

#[test]
fn allow_pub_crate_rejects_nested_modules() {
    assert!(!visibility::allow_pub_crate_by_policy(
        CrateKind::Library,
        ModuleLocation::NestedModule,
        ParentVisibility::Private
    ));
}

#[test]
fn allow_pub_crate_rejects_binary_crate_root_items() {
    assert!(!visibility::allow_pub_crate_by_policy(
        CrateKind::Binary,
        ModuleLocation::CrateRoot,
        ParentVisibility::Public
    ));
}

#[test]
fn allow_pub_crate_allows_top_level_private_binary_modules() {
    assert!(visibility::allow_pub_crate_by_policy(
        CrateKind::Binary,
        ModuleLocation::TopLevelPrivateModule,
        ParentVisibility::Private
    ));
}

#[test]
fn allow_pub_crate_rejects_binary_nested_modules() {
    assert!(!visibility::allow_pub_crate_by_policy(
        CrateKind::Binary,
        ModuleLocation::NestedModule,
        ParentVisibility::Private
    ));
}

#[test]
fn forbidden_pub_crate_help_handles_crate_root_items() {
    assert_eq!(
        visibility::forbidden_pub_crate_help(ModuleLocation::CrateRoot),
        "consider using just `pub` or removing `pub(crate)` entirely"
    );
}

#[test]
fn forbidden_pub_crate_help_handles_top_level_private_modules() {
    assert_eq!(
        visibility::forbidden_pub_crate_help(ModuleLocation::TopLevelPrivateModule),
        "consider using just `pub` or removing `pub(crate)` entirely"
    );
}

#[test]
fn forbidden_pub_crate_help_handles_nested_private_modules() {
    assert_eq!(
        visibility::forbidden_pub_crate_help(ModuleLocation::NestedModule),
        "consider using `pub(super)` or removing `pub(crate)` entirely"
    );
}

#[test]
fn suspicious_pub_note_uses_public_api_wording_for_libraries() {
    assert_eq!(
        visibility::suspicious_pub_note(CrateKind::Library, "struct"),
        "struct is not reachable from the crate's public API"
    );
}

#[test]
fn suspicious_pub_note_uses_subtree_wording_for_binaries() {
    assert_eq!(
        visibility::suspicious_pub_note(CrateKind::Binary, "function"),
        "function is not used outside its parent module subtree"
    );
}

#[test]
fn analysis_source_root_ignores_build_scripts() {
    let package_root = Path::new("/tmp/example-crate");

    assert_eq!(
        source_cache::analysis_source_root_for(&package_root.join("src/lib.rs"), package_root),
        Some(package_root.join("src"))
    );
    assert_eq!(
        source_cache::analysis_source_root_for(&package_root.join("src/bin/demo.rs"), package_root),
        Some(package_root.join("src/bin"))
    );
    assert_eq!(
        source_cache::analysis_source_root_for(
            &package_root.join("examples/demo.rs"),
            package_root
        ),
        Some(package_root.join("examples"))
    );
    assert_eq!(
        source_cache::analysis_source_root_for(&package_root.join("build.rs"), package_root),
        None
    );
}

#[test]
fn grouped_parent_pub_use_is_fix_supported() {
    let source = "pub use report_writer::{ReportDefinition, ReportWriter};\n";
    let file = syn::parse_file(source).unwrap();
    let exports =
        facade::exported_names_from_parent_boundary(&file, "report_writer", "ReportDefinition");
    assert_eq!(exports.explicit, vec!["ReportDefinition".to_string()]);
    assert!(exports.fix_supported);
}

#[test]
fn multiline_grouped_parent_pub_use_is_fix_supported() {
    let source = "pub use child::{\n    Thing,\n    Other,\n};\n";
    let file = syn::parse_file(source).unwrap();
    let exports = facade::exported_names_from_parent_boundary(&file, "child", "Thing");
    assert_eq!(exports.explicit, vec!["Thing".to_string()]);
    assert!(exports.fix_supported);
}

#[test]
fn module_path_from_source_file_treats_main_rs_as_crate_root() -> anyhow::Result<()> {
    let unique = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let temp_dir = std::env::temp_dir().join(format!("mend-main-root-test-{unique}"));
    let src_dir = temp_dir.join("src");
    fs::create_dir_all(&src_dir)?;
    let main_rs = src_dir.join("main.rs");
    fs::write(&main_rs, "fn main() {}\n")?;

    assert_eq!(
        source_cache::module_path_from_source_file(&src_dir, &main_rs),
        Some(Vec::new())
    );

    fs::remove_dir_all(&temp_dir)?;
    Ok(())
}

#[test]
fn grouped_parent_pub_use_with_rename_is_manual_only() {
    let source = "pub use child::{Thing as RenamedThing, Other};\n";
    let file = syn::parse_file(source).unwrap();
    let exports = facade::exported_names_from_parent_boundary(&file, "child", "Thing");
    assert_eq!(
        exports,
        ParentFacadeExports {
            explicit:      vec!["RenamedThing".to_string()],
            fix_supported: false,
            visibility:    Some(ParentFacadeVisibility::Public),
        }
    );

    let exports = facade::exported_names_from_parent_boundary(&file, "child", "Other");
    assert_eq!(exports.explicit, vec!["Other".to_string()]);
    assert!(exports.fix_supported);
}

#[test]
fn plain_building_progress_line_is_treated_as_progress() {
    let line = "    Building [                             ] 0/1: cli_json_clean_fixture      \r    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.16s\n";
    assert!(build::is_progress_line(line));
}

#[test]
fn progress_line_with_embedded_warning_is_not_treated_as_progress() {
    let line = "    Building [                             ] 0/1: fixture...warning: unused import: `child::SpawnStats`\n";
    assert!(!build::is_progress_line(line));
}

#[test]
fn classify_suppresses_unused_import_when_warning_follows_progress_prefix() {
    let block = vec![
        "    Building [                             ] 0/1: fixture...warning: unused import: `child::SpawnStats`\n"
            .to_string(),
        " --> src/actor/mod.rs:2:9\n".to_string(),
        "  |\n".to_string(),
        "2 | pub use child::SpawnStats;\n".to_string(),
        "  |         ^^^^^^^^^^^^^^^^^\n".to_string(),
        "\n".to_string(),
    ];

    assert!(matches!(
        build::classify_diagnostic_block(&block),
        DiagnosticBlockKind::SuppressedUnusedImport
    ));
}

#[test]
fn quiet_builds_do_not_accumulate_compiler_warning_summary_counts() {
    let mut block = vec![
        "warning: `fixture` (lib) generated 3 warnings (1 duplicate) (run `cargo fix --lib -p fixture` to apply 1 suggestion)\n"
            .to_string(),
        "\n".to_string(),
    ];
    let mut printed_suppression_notice = false;
    let mut compiler_warnings = CompilerWarningFacts::None;
    let mut compiler_warning_count = 0;
    let mut compiler_fixable_count = 0;

    build::flush_diagnostic_block(
        &mut block,
        &mut printed_suppression_notice,
        &mut compiler_warnings,
        &mut compiler_warning_count,
        &mut compiler_fixable_count,
        BuildOutputMode::Quiet,
    );

    assert_eq!(compiler_warning_count, 0);
    assert_eq!(compiler_fixable_count, 0);
}
