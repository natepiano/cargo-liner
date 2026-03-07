mod cli;
mod config;
mod diagnostics;
mod render;
mod scan;
mod selection;

use std::io::IsTerminal;
use std::process::ExitCode;

use anyhow::Result;

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("vischeck: {err:#}");
            ExitCode::from(2)
        },
    }
}

fn run() -> Result<ExitCode> {
    let cli = cli::parse();
    let selection = selection::resolve_cargo_selection(cli.manifest_path.as_deref())?;
    let config = config::load_config(
        selection.manifest_dir.as_path(),
        selection.workspace_root.as_path(),
        cli.config.as_deref(),
    )?;
    let report = scan::scan_selection(&selection, &config)?;

    if cli.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!(
            "{}",
            render::render_human_report(&report, std::io::stdout().is_terminal())
        );
    }

    if report.has_errors() {
        return Ok(ExitCode::from(1));
    }

    if cli.fail_on_warn && report.has_warnings() {
        return Ok(ExitCode::from(2));
    }

    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::fs;

    use anyhow::Result;
    use tempfile::tempdir;

    use super::config;
    use super::diagnostics::DIAGNOSTICS;
    use super::render;
    use super::scan;
    use super::selection;

    #[test]
    fn every_diagnostic_has_a_unique_readme_anchor() {
        let readme = include_str!("../README.md");
        let mut seen_codes = BTreeSet::new();
        let mut seen_anchors = BTreeSet::new();

        for spec in DIAGNOSTICS {
            assert!(
                seen_codes.insert(spec.code),
                "duplicate diagnostic code: {}",
                spec.code
            );
            assert!(
                seen_anchors.insert(spec.help_anchor),
                "duplicate README anchor: {}",
                spec.help_anchor
            );
            let anchor = format!(r#"<a id="{}"></a>"#, spec.help_anchor);
            assert!(
                readme.contains(&anchor),
                "README is missing anchor for {}: {}",
                spec.code,
                spec.help_anchor
            );
        }
    }

    #[test]
    fn fixture_renders_every_current_diagnostic() -> Result<()> {
        let temp = tempdir()?;
        fs::create_dir_all(temp.path().join("src/private_parent"))?;

        fs::write(
            temp.path().join("Cargo.toml"),
            r#"[package]
name = "fixture"
version = "0.1.0"
edition = "2024"
"#,
        )?;
        fs::write(
            temp.path().join("src/main.rs"),
            r#"pub(crate) fn crate_only() {}
pub(in crate::private_parent) fn subtree_only() {}
pub mod review_mod;
mod private_parent;

fn main() {}
"#,
        )?;
        fs::write(temp.path().join("src/review_mod.rs"), "\n")?;
        fs::write(
            temp.path().join("src/private_parent.rs"),
            "mod child;\npub use child::PublicContainer;\n",
        )?;
        fs::write(
            temp.path().join("src/private_parent/child.rs"),
            r#"pub enum LegitDependency {
    Unit,
}

pub struct PublicContainer {
    pub dependency: LegitDependency,
}

pub struct Suspicious;
"#,
        )?;

        let manifest_path = temp.path().join("Cargo.toml");
        let selection = selection::resolve_cargo_selection(Some(&manifest_path))?;
        let loaded_config = config::load_config(
            selection.manifest_dir.as_path(),
            selection.workspace_root.as_path(),
            None,
        )?;
        let report = scan::scan_selection(&selection, &loaded_config)?;

        let rendered = render::render_human_report(&report, false);
        let codes: BTreeSet<_> = report
            .findings
            .iter()
            .map(|finding| finding.code.as_str())
            .collect();
        let expected_codes: BTreeSet<_> = DIAGNOSTICS.iter().map(|spec| spec.code).collect();

        assert_eq!(
            codes, expected_codes,
            "fixture should trigger every diagnostic exactly once"
        );
        assert_eq!(
            report.findings.len(),
            DIAGNOSTICS.len(),
            "fixture should trigger one finding per diagnostic"
        );

        for spec in DIAGNOSTICS {
            assert!(
                rendered.contains(spec.headline),
                "rendered output is missing headline for {}",
                spec.code
            );
            let help_url = format!(
                "https://github.com/natepiano/cargo-vischeck#{}",
                spec.help_anchor
            );
            assert!(
                rendered.contains(&help_url),
                "rendered output is missing help URL for {}",
                spec.code
            );
        }

        assert!(rendered.contains("help: consider using: `pub(super)`"));
        Ok(())
    }
}
