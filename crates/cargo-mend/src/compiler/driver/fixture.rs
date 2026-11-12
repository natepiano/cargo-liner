use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Context;
use anyhow::Result;
use anyhow::bail;

use super::AnalysisCallbacks;
use super::IntoExitCode;
use crate::compiler;
use crate::compiler::persistence;
use crate::compiler::persistence::AnalysisEvidence;
use crate::compiler::settings;
use crate::compiler::settings::DriverSettings;
use crate::config;
use crate::config::CargoCheckCli;
use crate::config::DiagnosticStatus;
use crate::config::LoadedConfig;
use crate::fixes;
use crate::reporting;
use crate::selection;
use crate::selection::PackageMetadata;
use crate::selection::Selection;
use crate::selection::TargetMetadata;
use crate::selection::TargetSupport;
use crate::support;

/// Compile both target modes that a normal all-targets check analyzes.
enum FixtureTargetMode {
    Main,
    Test,
}

impl FixtureTargetMode {
    const fn suffix(&self) -> &'static str {
        match self {
            Self::Main => "main",
            Self::Test => "test",
        }
    }
}

/// A real compiler invocation for one fixture target and its selected configuration.
struct FixtureCompilation<'a> {
    package:   &'a PackageMetadata,
    target:    &'a TargetMetadata,
    mode:      FixtureTargetMode,
    selection: &'a Selection,
    config:    &'a LoadedConfig,
}

impl FixtureCompilation<'_> {
    fn run(&self, output: &Path) -> Result<PathBuf> {
        let settings = DriverSettings {
            config_root:          self.config.root.clone(),
            visibility_config:    self.config.visibility_config.clone(),
            config_fingerprint:   self.config.fingerprint.clone(),
            analysis_fingerprint: settings::current_analysis_fingerprint(),
            scope_fingerprint:    compiler::fixture_scope_fingerprint(
                &selection::build_cargo_check_plan(self.selection, &CargoCheckCli::default()),
            ),
            analyzing_dir:        self.selection.target_directory.join("mend-analyzing"),
            package_root:         self.package.root.clone(),
        };
        let mut arguments = vec![
            "rustc".to_string(),
            self.target.src_path.display().to_string(),
            "--crate-name".to_string(),
            self.target.name.replace('-', "_"),
            format!("--edition={}", self.target.edition),
            "--crate-type".to_string(),
            self.target.crate_types.join(","),
            "--sysroot".to_string(),
            env!("MEND_BUILD_SYSROOT").to_string(),
            "--emit=metadata".to_string(),
            "-o".to_string(),
            output.display().to_string(),
        ];
        if matches!(self.mode, FixtureTargetMode::Test) {
            arguments.push("--test".to_string());
        }
        let mut callbacks = AnalysisCallbacks::new(settings);
        let status = rustc_driver::catch_with_exit_code(|| {
            rustc_driver::run_compiler(&arguments, &mut callbacks);
        })
        .into_exit_code();
        if let Some(error) = callbacks.error {
            return Err(error);
        }
        if status != ExitCode::SUCCESS {
            bail!("fixture target {} failed to compile", self.target.name);
        }
        Ok(persistence::report_path_for_metadata(output))
    }
}

/// Read the canonical fixture manifest and its Cargo target metadata.
pub(crate) fn read_fixture_selection(manifest: &Path) -> Result<Selection> {
    let manifest = manifest
        .canonicalize()
        .context("resolve fixture manifest path")?;
    let metadata = support::cargo_command()
        .args([
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--manifest-path",
        ])
        .arg(&manifest)
        .output()
        .context("read fixture Cargo metadata")?;
    if !metadata.status.success() {
        bail!(
            "fixture Cargo metadata failed: {}",
            String::from_utf8_lossy(&metadata.stderr)
        );
    }
    selection::fixture_selection(&manifest, &serde_json::from_slice(&metadata.stdout)?)
}

/// Analyze the fixture with production compiler callbacks and the JSON renderer.
pub(crate) fn analyze_fixture_json(manifest: &Path) -> Result<String> {
    let selection = read_fixture_selection(manifest)?;
    let config = config::load_config(
        &selection.manifest_dir,
        &selection.workspace_root,
        None,
        &config::load_global_config(),
    )?;
    let outputs = selection.target_directory.join("fixture-analysis");
    fs::create_dir_all(&outputs).context("create fixture compiler output directory")?;
    let mut reports = Vec::new();
    for (package_index, package) in selection.packages.iter().enumerate() {
        for (target_index, target) in package.targets.iter().enumerate() {
            let modes = match target.test {
                TargetSupport::Enabled => vec![FixtureTargetMode::Main, FixtureTargetMode::Test],
                TargetSupport::Disabled => vec![FixtureTargetMode::Main],
            };
            for mode in modes {
                let output = outputs.join(format!(
                    "{package_index}-{target_index}-{}.rmeta",
                    mode.suffix()
                ));
                reports.push(
                    FixtureCompilation {
                        package,
                        target,
                        mode,
                        selection: &selection,
                        config: &config,
                    }
                    .run(&output)?,
                );
            }
        }
    }
    let loaded = persistence::load_report(&reports, &selection, &config.fingerprint);
    if loaded.analysis_evidence == AnalysisEvidence::Absent {
        bail!("fixture compiler analysis produced no report");
    }
    let mut report = loaded.report;
    report
        .findings
        .extend(fixes::collect_fixture_syntax_findings(&selection)?);
    report.findings.retain(|finding| {
        config
            .diagnostics_config
            .is_enabled(finding.diagnostic_code)
            == DiagnosticStatus::Enabled
    });
    reporting::sort_and_dedup_findings(&mut report.findings);
    report.refresh_summary();
    reporting::render_report(&report, &selection)
}
