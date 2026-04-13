mod build;
mod driver;
mod exposure;
mod facade;
mod persistence;
mod source_cache;
mod visibility;

use std::env;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
pub(crate) use build::BuildOutputMode;
pub(crate) use build::SelectionResult;
pub(crate) use build::run_cargo_fix;
pub(crate) use build::run_selection;
pub(crate) use driver::driver_main;

use crate::config::VisibilityConfig;
use crate::constants::CONFIG_FINGERPRINT_ENV;
use crate::constants::CONFIG_JSON_ENV;
use crate::constants::CONFIG_ROOT_ENV;
use crate::constants::PACKAGE_ROOT_ENV;
use crate::constants::SCOPE_FINGERPRINT_ENV;

fn current_analysis_fingerprint() -> String {
    let version = env!("CARGO_PKG_VERSION");
    let git_hash = option_env!("MEND_GIT_HASH").unwrap_or("nogit");
    let build_id = option_env!("MEND_BUILD_ID").unwrap_or("nobuild");
    format!("{version}+{git_hash}+{build_id}")
}

#[derive(Debug, Clone)]
pub(super) struct DriverSettings {
    pub config_root:          PathBuf,
    pub config:               VisibilityConfig,
    pub config_fingerprint:   String,
    pub analysis_fingerprint: String,
    pub scope_fingerprint:    String,
    pub findings_dir:         PathBuf,
    pub package_root:         PathBuf,
}

impl DriverSettings {
    fn from_env() -> Result<Self> {
        let config_root = PathBuf::from(
            env::var_os(CONFIG_ROOT_ENV).context("missing MEND_CONFIG_ROOT for compiler driver")?,
        );
        let config = serde_json::from_str(
            &env::var(CONFIG_JSON_ENV).context("missing MEND_CONFIG_JSON for compiler driver")?,
        )
        .context("failed to parse MEND_CONFIG_JSON")?;
        let config_fingerprint =
            env::var(CONFIG_FINGERPRINT_ENV).context("missing MEND_CONFIG_FINGERPRINT")?;
        let findings_dir = PathBuf::from(
            env::var_os(crate::constants::FINDINGS_DIR_ENV)
                .context("missing MEND_FINDINGS_DIR for compiler driver")?,
        );
        let scope_fingerprint =
            env::var(SCOPE_FINGERPRINT_ENV).context("missing MEND_SCOPE_FINGERPRINT")?;
        let package_root = PathBuf::from(
            env::var_os(PACKAGE_ROOT_ENV)
                .context("missing CARGO_MANIFEST_DIR for compiler driver")?,
        );

        Ok(Self {
            config_root,
            config,
            config_fingerprint,
            analysis_fingerprint: current_analysis_fingerprint(),
            scope_fingerprint,
            findings_dir,
            package_root,
        })
    }
}

fn config_relative_path(file_path: &Path, config_root: &Path) -> Option<String> {
    file_path
        .strip_prefix(config_root)
        .ok()
        .map(normalize_relative_path)
        .or_else(|| {
            let canonical_file = fs::canonicalize(file_path).ok()?;
            let canonical_root = fs::canonicalize(config_root).ok()?;
            canonical_file
                .strip_prefix(canonical_root)
                .ok()
                .map(normalize_relative_path)
        })
}

fn config_relative_path_for_settings(
    file_path: &Path,
    settings: &DriverSettings,
) -> Option<String> {
    if file_path.is_relative() {
        let workspace_relative = normalize_relative_path(file_path);
        if settings.config_root.join(file_path).exists() {
            return Some(workspace_relative);
        }

        let package_relative = settings.package_root.join(file_path);
        return config_relative_path(&package_relative, &settings.config_root)
            .or(Some(workspace_relative));
    }

    config_relative_path(file_path, &settings.config_root)
}

fn normalize_relative_path(path: &Path) -> String { path.to_string_lossy().replace('\\', "/") }

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::path::PathBuf;
    use std::time::SystemTime;
    use std::time::UNIX_EPOCH;

    use super::DriverSettings;
    use super::build::BuildOutputMode;
    use super::build::DiagnosticBlockKind;
    use super::build::classify_diagnostic_block;
    use super::build::flush_diagnostic_block;
    use super::build::is_progress_line;
    use super::config_relative_path;
    use super::config_relative_path_for_settings;
    use super::current_analysis_fingerprint;
    use super::facade::ParentFacadeExports;
    use super::facade::ParentFacadeVisibility;
    use super::facade::exported_names_from_parent_boundary;
    use super::source_cache::analysis_source_root_for;
    use super::source_cache::module_path_from_source_file;
    use super::visibility::CrateKind;
    use super::visibility::ModuleLocation;
    use super::visibility::allow_pub_crate_by_policy;
    use super::visibility::forbidden_pub_crate_help;
    use super::visibility::suspicious_pub_note;
    use crate::config::VisibilityConfig;
    use crate::diagnostics::CompilerWarningFacts;

    #[test]
    fn allow_pub_crate_allows_library_crate_root_items() {
        assert!(allow_pub_crate_by_policy(
            CrateKind::Library,
            ModuleLocation::CrateRoot,
            true
        ));
    }

    #[test]
    fn allow_pub_crate_allows_top_level_private_library_modules() {
        assert!(allow_pub_crate_by_policy(
            CrateKind::Library,
            ModuleLocation::TopLevelPrivateModule,
            false
        ));
    }

    #[test]
    fn allow_pub_crate_rejects_nested_modules() {
        assert!(!allow_pub_crate_by_policy(
            CrateKind::Library,
            ModuleLocation::NestedModule,
            false
        ));
    }

    #[test]
    fn allow_pub_crate_rejects_binary_crate_root_items() {
        assert!(!allow_pub_crate_by_policy(
            CrateKind::Binary,
            ModuleLocation::CrateRoot,
            true
        ));
    }

    #[test]
    fn allow_pub_crate_allows_top_level_private_binary_modules() {
        assert!(allow_pub_crate_by_policy(
            CrateKind::Binary,
            ModuleLocation::TopLevelPrivateModule,
            false
        ));
    }

    #[test]
    fn allow_pub_crate_rejects_binary_nested_modules() {
        assert!(!allow_pub_crate_by_policy(
            CrateKind::Binary,
            ModuleLocation::NestedModule,
            false
        ));
    }

    #[test]
    fn forbidden_pub_crate_help_handles_crate_root_items() {
        assert_eq!(
            forbidden_pub_crate_help(ModuleLocation::CrateRoot),
            "consider using just `pub` or removing `pub(crate)` entirely"
        );
    }

    #[test]
    fn forbidden_pub_crate_help_handles_top_level_private_modules() {
        assert_eq!(
            forbidden_pub_crate_help(ModuleLocation::TopLevelPrivateModule),
            "consider using just `pub` or removing `pub(crate)` entirely"
        );
    }

    #[test]
    fn forbidden_pub_crate_help_handles_nested_private_modules() {
        assert_eq!(
            forbidden_pub_crate_help(ModuleLocation::NestedModule),
            "consider using `pub(super)` or removing `pub(crate)` entirely"
        );
    }

    #[test]
    fn suspicious_pub_note_uses_public_api_wording_for_libraries() {
        assert_eq!(
            suspicious_pub_note(CrateKind::Library, "struct"),
            "struct is not reachable from the crate's public API"
        );
    }

    #[test]
    fn suspicious_pub_note_uses_subtree_wording_for_binaries() {
        assert_eq!(
            suspicious_pub_note(CrateKind::Binary, "function"),
            "function is not used outside its parent module subtree"
        );
    }

    #[test]
    fn config_relative_path_handles_nested_workspace_paths() -> anyhow::Result<()> {
        let unique = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let workspace_root = std::env::temp_dir().join(format!("mend-config-root-test-{unique}"));
        let file_path = workspace_root.join("mcp/src/brp_tools/tools/mod.rs");
        let parent = file_path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("test path must have a parent directory"))?;
        fs::create_dir_all(parent)?;
        fs::write(&file_path, "pub mod world_query;\n")?;

        assert_eq!(
            config_relative_path(&file_path, &workspace_root).as_deref(),
            Some("mcp/src/brp_tools/tools/mod.rs")
        );

        Ok(())
    }

    #[test]
    fn config_relative_path_for_settings_handles_package_relative_workspace_paths() {
        let settings = DriverSettings {
            config_root:          PathBuf::from("/workspace/root"),
            config:               VisibilityConfig::default(),
            config_fingerprint:   "test".to_string(),
            scope_fingerprint:    "scope".to_string(),
            findings_dir:         PathBuf::from("/workspace/root/target/mend-findings"),
            package_root:         PathBuf::from("/workspace/root/mcp"),
            analysis_fingerprint: current_analysis_fingerprint(),
        };
        let file_path = PathBuf::from("src/brp_tools/tools/mod.rs");

        assert_eq!(
            config_relative_path_for_settings(&file_path, &settings).as_deref(),
            Some("mcp/src/brp_tools/tools/mod.rs")
        );
    }

    #[test]
    fn config_relative_path_for_settings_handles_workspace_relative_paths() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let config_root = temp.path().join("workspace");
        let package_root = config_root.join("mcp");
        std::fs::create_dir_all(package_root.join("src/brp_tools/tools"))?;
        std::fs::write(
            package_root.join("src/brp_tools/tools/mod.rs"),
            "pub mod child;\n",
        )?;
        let settings = DriverSettings {
            config_root,
            config: VisibilityConfig::default(),
            config_fingerprint: "test".to_string(),
            scope_fingerprint: "scope".to_string(),
            findings_dir: temp.path().join("workspace/target/mend-findings"),
            package_root,
            analysis_fingerprint: current_analysis_fingerprint(),
        };
        let file_path = PathBuf::from("mcp/src/brp_tools/tools/mod.rs");

        assert_eq!(
            config_relative_path_for_settings(&file_path, &settings).as_deref(),
            Some("mcp/src/brp_tools/tools/mod.rs")
        );

        Ok(())
    }

    #[test]
    fn analysis_source_root_ignores_build_scripts() {
        let package_root = Path::new("/tmp/example-crate");

        assert_eq!(
            analysis_source_root_for(&package_root.join("src/lib.rs"), package_root),
            Some(package_root.join("src"))
        );
        assert_eq!(
            analysis_source_root_for(&package_root.join("src/bin/demo.rs"), package_root),
            Some(package_root.join("src/bin"))
        );
        assert_eq!(
            analysis_source_root_for(&package_root.join("examples/demo.rs"), package_root),
            Some(package_root.join("examples"))
        );
        assert_eq!(
            analysis_source_root_for(&package_root.join("build.rs"), package_root),
            None
        );
    }

    #[test]
    fn grouped_parent_pub_use_is_fix_supported() {
        let source = "pub use report_writer::{ReportDefinition, ReportWriter};\n";
        let file = syn::parse_file(source).unwrap();
        let exports =
            exported_names_from_parent_boundary(&file, "report_writer", "ReportDefinition");
        assert_eq!(exports.explicit, vec!["ReportDefinition".to_string()]);
        assert!(exports.fix_supported);
    }

    #[test]
    fn multiline_grouped_parent_pub_use_is_fix_supported() {
        let source = "pub use child::{\n    Thing,\n    Other,\n};\n";
        let file = syn::parse_file(source).unwrap();
        let exports = exported_names_from_parent_boundary(&file, "child", "Thing");
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
            module_path_from_source_file(&src_dir, &main_rs),
            Some(Vec::new())
        );

        fs::remove_dir_all(&temp_dir)?;
        Ok(())
    }

    #[test]
    fn grouped_parent_pub_use_with_rename_is_manual_only() {
        let source = "pub use child::{Thing as RenamedThing, Other};\n";
        let file = syn::parse_file(source).unwrap();
        let exports = exported_names_from_parent_boundary(&file, "child", "Thing");
        assert_eq!(
            exports,
            ParentFacadeExports {
                explicit:      vec!["RenamedThing".to_string()],
                fix_supported: false,
                visibility:    Some(ParentFacadeVisibility::Public),
            }
        );

        let exports = exported_names_from_parent_boundary(&file, "child", "Other");
        assert_eq!(exports.explicit, vec!["Other".to_string()]);
        assert!(exports.fix_supported);
    }

    #[test]
    fn plain_building_progress_line_is_treated_as_progress() {
        let line = "    Building [                             ] 0/1: cli_json_clean_fixture      \r    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.16s\n";
        assert!(is_progress_line(line));
    }

    #[test]
    fn progress_line_with_embedded_warning_is_not_treated_as_progress() {
        let line = "    Building [                             ] 0/1: fixture...warning: unused import: `child::SpawnStats`\n";
        assert!(!is_progress_line(line));
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
            classify_diagnostic_block(&block),
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

        flush_diagnostic_block(
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
}
