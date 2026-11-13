use std::cell::OnceCell;
use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;

use super::CARGO_TARGET_KIND_BENCH;
use super::CARGO_TARGET_KIND_BIN;
use super::CARGO_TARGET_KIND_EXAMPLE;
use super::CARGO_TARGET_KIND_LIB;
use super::CARGO_TARGET_KIND_TEST;
use super::PackageMetadata;
use super::TargetMetadata;
use crate::config::CargoCheckCli;
use crate::config::TargetSelection;
use crate::reporting::Report;
use crate::rust_syntax::ModuleMap;
use crate::rust_syntax::ParsedSources;
use crate::rust_syntax::SourcePathKey;

/// Filters mend findings based on the user's target-selection flags.
///
/// The analysis pass always compiles `--all-targets` so reachability and
/// visibility claims hold across the whole crate. The user's `--lib`,
/// `--bin`, `--example`, `--test`, `--bench`, and `--all-targets` flags
/// then narrow what gets *displayed*. With no target flags, everything is
/// reported.
pub(crate) enum DisplayFilter {
    AllowAll,
    SelectedSources {
        sources:       OnceCell<BTreeSet<SourcePathKey>>,
        crate_roots:   BTreeSet<SourcePathKey>,
        package_roots: Vec<PathBuf>,
    },
}

impl DisplayFilter {
    pub(crate) fn from_cli(cli: &CargoCheckCli, packages: &[PackageMetadata]) -> Self {
        let asks_for_all = cli.target_selections.contains(&TargetSelection::All);
        let any_target_flag = !cli.target_selections.is_empty()
            || !cli.bin.is_empty()
            || !cli.example.is_empty()
            || !cli.test.is_empty()
            || !cli.bench.is_empty();
        if asks_for_all || !any_target_flag {
            return Self::AllowAll;
        }

        let parsed = ParsedSources::default();
        let mut crate_roots = BTreeSet::new();
        for package in packages {
            for target in &package.targets {
                if cli_includes_target(cli, target) == TargetInclusion::Omitted {
                    continue;
                }
                crate_roots.insert(parsed.path_key(&target.src_path));
            }
        }
        Self::SelectedSources {
            sources: OnceCell::new(),
            crate_roots,
            package_roots: packages
                .iter()
                .map(|package| package.root.clone())
                .collect(),
        }
    }

    pub(crate) fn allows(&self, finding_path: &Path) -> FindingDisplay {
        match self {
            Self::AllowAll => FindingDisplay::Included,
            Self::SelectedSources {
                sources,
                package_roots,
                crate_roots,
            } => {
                let parsed = ParsedSources::default();
                let sources = sources.get_or_init(|| {
                    // Cargo lists target roots, but only source text identifies their modules and
                    // includes.
                    let mut sources = BTreeSet::new();
                    for root in crate_roots {
                        let modules = ModuleMap::for_crate_root(root.as_path(), &parsed);
                        sources.extend(
                            modules
                                .declared_files()
                                .map(|(file, _)| parsed.path_key(file)),
                        );
                        if let Ok(included) =
                            ModuleMap::reachable_sources([root.as_path().to_path_buf()], &parsed)
                        {
                            sources.extend(included.iter().map(|file| parsed.path_key(file)));
                        }
                    }
                    sources
                });
                let included = if finding_path.is_absolute() {
                    sources.contains(&parsed.path_key(finding_path))
                } else {
                    package_roots
                        .iter()
                        .any(|root| sources.contains(&parsed.path_key(&root.join(finding_path))))
                };
                if included {
                    FindingDisplay::Included
                } else {
                    FindingDisplay::Excluded
                }
            },
        }
    }

    pub(crate) fn apply(&self, report: &mut Report) {
        let Self::SelectedSources { crate_roots, .. } = self else {
            return;
        };
        let parsed = ParsedSources::default();
        let root = Path::new(&report.root);
        // Compiler records identify active sources, including generated sources and nonliteral
        // includes.
        let compiled_sources: BTreeSet<_> = report
            .facts
            .module_mount_facts
            .iter()
            .filter(|mount| {
                crate_roots.contains(&parsed.path_key(&root.join(&mount.crate_root_file)))
            })
            .map(|mount| parsed.path_key(&root.join(&mount.file)))
            .collect();
        let has_compiler_mounts = report.facts.module_mount_facts.iter().next().is_some();
        report.findings.retain(|finding| {
            let file = root.join(&finding.path);
            if has_compiler_mounts {
                compiled_sources.contains(&parsed.path_key(&file))
            } else {
                self.allows(&file) == FindingDisplay::Included
            }
        });
        report.refresh_summary();
    }
}

/// Whether target selection includes a finding in the displayed report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FindingDisplay {
    Included,
    Excluded,
}

/// Whether the command line selects a particular target for display.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TargetInclusion {
    Included,
    Omitted,
}

fn cli_includes_target(cli: &CargoCheckCli, target: &TargetMetadata) -> TargetInclusion {
    let kind = target.kind.first().map_or("", String::as_str);
    if match kind {
        CARGO_TARGET_KIND_LIB => cli.target_selections.contains(&TargetSelection::Library),
        CARGO_TARGET_KIND_BIN => {
            cli.target_selections.contains(&TargetSelection::Binaries)
                || cli.bin.iter().any(|name| name == &target.name)
        },
        CARGO_TARGET_KIND_EXAMPLE => {
            cli.target_selections.contains(&TargetSelection::Examples)
                || cli.example.iter().any(|name| name == &target.name)
        },
        CARGO_TARGET_KIND_TEST => {
            cli.target_selections.contains(&TargetSelection::Tests)
                || cli.test.iter().any(|name| name == &target.name)
        },
        CARGO_TARGET_KIND_BENCH => {
            cli.target_selections.contains(&TargetSelection::Benches)
                || cli.bench.iter().any(|name| name == &target.name)
        },
        _ => false,
    } {
        TargetInclusion::Included
    } else {
        TargetInclusion::Omitted
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::fs;
    use std::io;

    use tempfile::TempDir;

    use super::*;
    use crate::config::TargetSelection;
    use crate::reporting::ReportFacts;
    use crate::selection::CARGO_TARGET_KIND_BIN;
    use crate::selection::CARGO_TARGET_KIND_EXAMPLE;
    use crate::selection::CARGO_TARGET_KIND_LIB;
    use crate::selection::PackageMetadata;
    use crate::selection::TargetMetadata;
    use crate::selection::TargetSupport;

    fn target(kind: &str, name: &str, src_path: &Path) -> TargetMetadata {
        TargetMetadata {
            kind:              vec![kind.to_string()],
            crate_types:       Vec::new(),
            name:              name.to_string(),
            src_path:          src_path.to_path_buf(),
            edition:           "2024".to_string(),
            required_features: Vec::new(),
            doc:               TargetSupport::Disabled,
            doctest:           TargetSupport::Disabled,
            test:              TargetSupport::Disabled,
        }
    }

    fn package_with_lib_and_example() -> io::Result<(TempDir, PackageMetadata)> {
        let temp = tempfile::tempdir()?;
        for directory in ["src/bin", "examples/demo"] {
            fs::create_dir_all(temp.path().join(directory))?;
        }
        for (file, source) in [
            (
                "src/lib.rs",
                "mod helpers; mod helper; mod lib_helpers; mod anywhere;",
            ),
            ("src/helpers.rs", ""),
            ("src/helper.rs", ""),
            ("src/lib_helpers.rs", ""),
            ("src/anywhere.rs", ""),
            ("src/bin/tool.rs", "fn main() {}"),
            ("examples/demo/main.rs", "mod helper; fn main() {}"),
            ("examples/demo/helper.rs", ""),
        ] {
            fs::write(temp.path().join(file), source)?;
        }
        let root = temp.path();
        let package = PackageMetadata {
            id:            "pkg".to_string(),
            manifest_path: root.join("Cargo.toml"),
            root:          root.to_path_buf(),
            targets:       vec![
                target(CARGO_TARGET_KIND_LIB, "pkg", &root.join("src/lib.rs")),
                target(
                    CARGO_TARGET_KIND_EXAMPLE,
                    "demo",
                    &root.join("examples/demo/main.rs"),
                ),
            ],
        };
        Ok((temp, package))
    }

    #[test]
    fn compiler_mounts_filter_without_reading_sources() -> io::Result<()> {
        let (temp, package) = package_with_lib_and_example()?;
        let cli = CargoCheckCli {
            target_selections: BTreeSet::from([TargetSelection::Library]),
            ..CargoCheckCli::default()
        };
        let reads_before = ParsedSources::source_read_attempts();
        let filter = DisplayFilter::from_cli(&cli, &[package]);
        let mut report = Report {
            root: temp.path().to_string_lossy().into_owned(),
            facts: ReportFacts {
                module_mount_facts: vec![crate::reporting::ModuleMountFact {
                    crate_root_file: String::from("src/lib.rs"),
                    file:            String::from("generated.rs"),
                    module_path:     vec![String::from("generated")],
                }]
                .into(),
                ..crate::reporting::ReportFacts::default()
            },
            ..Report::default()
        };
        filter.apply(&mut report);
        assert_eq!(
            ParsedSources::source_read_attempts() - reads_before,
            0,
            "compiler mounts must select sources without reading source text"
        );
        Ok(())
    }

    #[test]
    fn no_target_flags_allows_everything() -> io::Result<()> {
        let fixture = package_with_lib_and_example()?;
        let filter = DisplayFilter::from_cli(&CargoCheckCli::default(), &[fixture.1]);
        assert!(matches!(filter, DisplayFilter::AllowAll));
        assert_eq!(
            filter.allows(Path::new("src/anywhere.rs")),
            FindingDisplay::Included
        );
        assert_eq!(
            filter.allows(Path::new("examples/demo/helper.rs")),
            FindingDisplay::Included
        );
        Ok(())
    }

    #[test]
    fn all_targets_flag_allows_everything() -> io::Result<()> {
        let cli = CargoCheckCli {
            target_selections: BTreeSet::from([TargetSelection::All]),
            ..CargoCheckCli::default()
        };
        let fixture = package_with_lib_and_example()?;
        let filter = DisplayFilter::from_cli(&cli, &[fixture.1]);
        assert!(matches!(filter, DisplayFilter::AllowAll));
        Ok(())
    }

    #[test]
    fn lib_flag_allows_lib_and_excludes_examples() -> io::Result<()> {
        let cli = CargoCheckCli {
            target_selections: BTreeSet::from([TargetSelection::Library]),
            ..CargoCheckCli::default()
        };
        let (temp, package) = package_with_lib_and_example()?;
        let filter = DisplayFilter::from_cli(&cli, &[package]);
        assert_eq!(
            filter.allows(&temp.path().join("src/helpers.rs")),
            FindingDisplay::Included
        );
        assert_eq!(
            filter.allows(&temp.path().join("examples/demo/helper.rs")),
            FindingDisplay::Excluded
        );
        Ok(())
    }

    #[test]
    fn lib_flag_excludes_src_bin_subdirectory() -> io::Result<()> {
        let (temp, mut pkg) = package_with_lib_and_example()?;
        pkg.targets = vec![
            target(
                CARGO_TARGET_KIND_LIB,
                "pkg",
                &temp.path().join("src/lib.rs"),
            ),
            target(
                CARGO_TARGET_KIND_BIN,
                "tool",
                &temp.path().join("src/bin/tool.rs"),
            ),
        ];
        let cli = CargoCheckCli {
            target_selections: BTreeSet::from([TargetSelection::Library]),
            ..CargoCheckCli::default()
        };
        let filter = DisplayFilter::from_cli(&cli, &[pkg]);
        assert_eq!(
            filter.allows(&temp.path().join("src/lib_helpers.rs")),
            FindingDisplay::Included
        );
        assert_eq!(
            filter.allows(&temp.path().join("src/bin/tool.rs")),
            FindingDisplay::Excluded,
            "src/bin/tool.rs belongs to the bin, not the lib",
        );
        Ok(())
    }

    #[test]
    fn named_example_allows_only_that_example_directory() -> io::Result<()> {
        let cli = CargoCheckCli {
            example: vec!["demo".to_string()],
            ..CargoCheckCli::default()
        };
        let (temp, package) = package_with_lib_and_example()?;
        let filter = DisplayFilter::from_cli(&cli, &[package]);
        assert_eq!(
            filter.allows(&temp.path().join("examples/demo/helper.rs")),
            FindingDisplay::Included
        );
        assert_eq!(
            filter.allows(&temp.path().join("src/anywhere.rs")),
            FindingDisplay::Excluded
        );
        Ok(())
    }
    #[test]
    fn library_role_uses_every_kind_and_preserves_explicit_binary_selection() -> io::Result<()> {
        let (temp, mut package) = package_with_lib_and_example()?;
        let mut library = target(
            CARGO_TARGET_KIND_BIN,
            "pkg",
            &temp.path().join("src/lib.rs"),
        );
        library.kind.push(CARGO_TARGET_KIND_LIB.to_string());
        package.targets = vec![
            library,
            target(
                CARGO_TARGET_KIND_BIN,
                "tool",
                &temp.path().join("src/bin/tool.rs"),
            ),
        ];
        let mut cli = CargoCheckCli {
            target_selections: BTreeSet::from([TargetSelection::Binaries]),
            ..CargoCheckCli::default()
        };
        let filter = DisplayFilter::from_cli(&cli, &[package.clone()]);
        assert_eq!(
            filter.allows(Path::new("src/helper.rs")),
            FindingDisplay::Included
        );
        assert_eq!(
            filter.allows(Path::new("src/bin/tool.rs")),
            FindingDisplay::Included
        );
        cli.target_selections = BTreeSet::from([TargetSelection::Library]);
        let filter = DisplayFilter::from_cli(&cli, &[package]);
        assert_eq!(
            filter.allows(Path::new("src/helper.rs")),
            FindingDisplay::Excluded
        );
        assert_eq!(
            filter.allows(Path::new("src/bin/tool.rs")),
            FindingDisplay::Excluded
        );
        Ok(())
    }
}
