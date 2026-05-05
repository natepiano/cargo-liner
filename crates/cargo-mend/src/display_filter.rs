use std::path::Path;
use std::path::PathBuf;

use super::cli::CargoCheckCli;
use super::cli::TargetSelection;
use super::diagnostics::Report;
use super::selection::PackageMetadata;
use super::selection::TargetMetadata;

/// Filters mend findings based on the user's target-selection flags.
///
/// The analysis pass always compiles `--all-targets` so reachability and
/// visibility claims hold across the whole crate. The user's `--lib`,
/// `--bin`, `--example`, `--test`, `--bench`, and `--all-targets` flags
/// then narrow what gets *displayed*. With no target flags, everything is
/// reported.
pub(crate) enum DisplayFilter {
    AllowAll,
    AllowDirs {
        allowed:       Vec<PathBuf>,
        /// Other-target roots inside `src/`. Used to subtract files owned
        /// by other targets when the lib target is included (e.g. exclude
        /// `src/bin/foo.rs` from a `--lib` filter).
        excluded_dirs: Vec<PathBuf>,
        package_root:  PathBuf,
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

        let mut allowed = Vec::new();
        let mut excluded_dirs = Vec::new();
        let mut package_root = PathBuf::new();
        let mut lib_included = false;
        for package in packages {
            package_root.clone_from(&package.root);
            for target in &package.targets {
                let dir = target_directory(target);
                if cli_includes_target(cli, target) {
                    allowed.push(dir.clone());
                    if target.kind.iter().any(|k| k == "lib") {
                        lib_included = true;
                    }
                }
            }
            // When lib is included, subtract every other target directory
            // under the package root so `src/bin/foo.rs` etc. stay out of
            // the lib's allow set.
            if lib_included {
                for target in &package.targets {
                    if !target.kind.iter().any(|k| k == "lib") {
                        let dir = target_directory(target);
                        if dir.starts_with(&package.root) {
                            excluded_dirs.push(dir);
                        }
                    }
                }
            }
        }

        if allowed.is_empty() {
            // User asked for filtering but no targets matched — show
            // nothing rather than misreport.
            Self::AllowDirs {
                allowed:       Vec::new(),
                excluded_dirs: Vec::new(),
                package_root:  PathBuf::new(),
            }
        } else {
            Self::AllowDirs {
                allowed,
                excluded_dirs,
                package_root,
            }
        }
    }

    pub(crate) fn allows(&self, finding_path: &Path) -> bool {
        match self {
            Self::AllowAll => true,
            Self::AllowDirs {
                allowed,
                excluded_dirs,
                package_root,
            } => {
                let absolute = if finding_path.is_absolute() {
                    finding_path.to_path_buf()
                } else {
                    package_root.join(finding_path)
                };
                let in_allowed = allowed.iter().any(|dir| absolute.starts_with(dir));
                if !in_allowed {
                    return false;
                }
                // If a subtracted directory matches, the finding belongs
                // to a narrower target. Exclude it unless that target was
                // also explicitly included (its dir is in `allowed`).
                let in_excluded_dir = excluded_dirs.iter().any(|dir| absolute.starts_with(dir));
                if in_excluded_dir {
                    let lib_dir = package_root.join("src");
                    return allowed
                        .iter()
                        .any(|dir| absolute.starts_with(dir) && *dir != lib_dir);
                }
                true
            },
        }
    }

    pub(crate) fn apply(&self, report: &mut Report) {
        if matches!(self, Self::AllowAll) {
            return;
        }
        report
            .findings
            .retain(|finding| self.allows(Path::new(&finding.path)));
        report.refresh_summary();
    }
}

fn target_directory(target: &TargetMetadata) -> PathBuf {
    target
        .src_path
        .parent()
        .map_or_else(|| target.src_path.clone(), Path::to_path_buf)
}

fn cli_includes_target(cli: &CargoCheckCli, target: &TargetMetadata) -> bool {
    let kind = target.kind.first().map_or("", String::as_str);
    match kind {
        "lib" => cli.target_selections.contains(&TargetSelection::Library),
        "bin" => {
            cli.target_selections.contains(&TargetSelection::Binaries)
                || cli.bin.iter().any(|name| name == &target.name)
        },
        "example" => {
            cli.target_selections.contains(&TargetSelection::Examples)
                || cli.example.iter().any(|name| name == &target.name)
        },
        "test" => {
            cli.target_selections.contains(&TargetSelection::Tests)
                || cli.test.iter().any(|name| name == &target.name)
        },
        "bench" => {
            cli.target_selections.contains(&TargetSelection::Benches)
                || cli.bench.iter().any(|name| name == &target.name)
        },
        _ => false,
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::cli::TargetSelection;
    use crate::selection::PackageMetadata;
    use crate::selection::TargetMetadata;

    fn target(kind: &str, name: &str, src_path: &str) -> TargetMetadata {
        TargetMetadata {
            kind:              vec![kind.to_string()],
            crate_types:       Vec::new(),
            name:              name.to_string(),
            src_path:          PathBuf::from(src_path),
            edition:           "2024".to_string(),
            required_features: Vec::new(),
            doc:               false,
            doctest:           false,
            test:              false,
        }
    }

    fn package_with_lib_and_example() -> PackageMetadata {
        PackageMetadata {
            id:            "pkg".to_string(),
            manifest_path: PathBuf::from("/proj/Cargo.toml"),
            root:          PathBuf::from("/proj"),
            targets:       vec![
                target("lib", "pkg", "/proj/src/lib.rs"),
                target("example", "demo", "/proj/examples/demo/main.rs"),
            ],
        }
    }

    #[test]
    fn no_target_flags_allows_everything() {
        let filter =
            DisplayFilter::from_cli(&CargoCheckCli::default(), &[package_with_lib_and_example()]);
        assert!(matches!(filter, DisplayFilter::AllowAll));
        assert!(filter.allows(Path::new("src/anywhere.rs")));
        assert!(filter.allows(Path::new("examples/demo/helper.rs")));
    }

    #[test]
    fn all_targets_flag_allows_everything() {
        let cli = CargoCheckCli {
            target_selections: BTreeSet::from([TargetSelection::All]),
            ..CargoCheckCli::default()
        };
        let filter = DisplayFilter::from_cli(&cli, &[package_with_lib_and_example()]);
        assert!(matches!(filter, DisplayFilter::AllowAll));
    }

    #[test]
    fn lib_flag_allows_lib_and_excludes_examples() {
        let cli = CargoCheckCli {
            target_selections: BTreeSet::from([TargetSelection::Library]),
            ..CargoCheckCli::default()
        };
        let filter = DisplayFilter::from_cli(&cli, &[package_with_lib_and_example()]);
        assert!(filter.allows(Path::new("/proj/src/helpers.rs")));
        assert!(!filter.allows(Path::new("/proj/examples/demo/helper.rs")));
    }

    #[test]
    fn lib_flag_excludes_src_bin_subdirectory() {
        let pkg = PackageMetadata {
            id:            "pkg".to_string(),
            manifest_path: PathBuf::from("/proj/Cargo.toml"),
            root:          PathBuf::from("/proj"),
            targets:       vec![
                target("lib", "pkg", "/proj/src/lib.rs"),
                target("bin", "tool", "/proj/src/bin/tool.rs"),
            ],
        };
        let cli = CargoCheckCli {
            target_selections: BTreeSet::from([TargetSelection::Library]),
            ..CargoCheckCli::default()
        };
        let filter = DisplayFilter::from_cli(&cli, &[pkg]);
        assert!(filter.allows(Path::new("/proj/src/lib_helpers.rs")));
        assert!(
            !filter.allows(Path::new("/proj/src/bin/tool.rs")),
            "src/bin/tool.rs belongs to the bin, not the lib",
        );
    }

    #[test]
    fn named_example_allows_only_that_example_directory() {
        let cli = CargoCheckCli {
            example: vec!["demo".to_string()],
            ..CargoCheckCli::default()
        };
        let filter = DisplayFilter::from_cli(&cli, &[package_with_lib_and_example()]);
        assert!(filter.allows(Path::new("/proj/examples/demo/helper.rs")));
        assert!(!filter.allows(Path::new("/proj/src/anywhere.rs")));
    }
}
