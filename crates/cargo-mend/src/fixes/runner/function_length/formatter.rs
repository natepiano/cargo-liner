use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;
use std::process::Stdio;
use std::sync::OnceLock;

use anyhow::Context;
use anyhow::Result;
use anyhow::ensure;
use toml::Value;

use super::constants::CLIPPY_DEFAULT_TOO_MANY_LINES_THRESHOLD;
use super::constants::DEFAULT_EDITION;
use super::constants::NIGHTLY_RUSTFMT_ARGUMENTS;
use super::constants::NIGHTLY_RUSTFMT_LOOKUP_ARGUMENTS;
use super::constants::RUSTFMT_ARGUMENTS;
use super::constants::RUSTFMT_COMMAND;
use super::constants::RUSTFMT_LOCAL_FILE_ARGUMENTS;
use super::constants::RUSTUP_COMMAND;
use super::constants::VERSION_ARGUMENT;

#[cfg(test)]
thread_local! {
    static FORMATTING_COMMANDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(super) fn formatting_commands() -> usize { FORMATTING_COMMANDS.get() }

/// The formatter available for function length protection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Formatter {
    Nightly,
    Path,
    Unavailable { cause: String },
}

#[derive(Debug)]
pub(super) enum FormatterProbe {
    Available,
    Unavailable(String),
}

enum NightlyExecutable {
    Direct(PathBuf),
    Rustup,
}

/// Files in this group use the same parser edition and formatter configuration.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct FormattingGroup {
    edition:       String,
    configuration: FormatterConfiguration,
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum FormatterConfiguration {
    Default,
    File(PathBuf),
}

impl Formatter {
    pub(super) fn detect() -> Self {
        static DETECTED: OnceLock<Formatter> = OnceLock::new();
        DETECTED
            .get_or_init(|| match nightly_probe() {
                FormatterProbe::Available => Self::Nightly,
                unavailable @ FormatterProbe::Unavailable(_) => {
                    Self::from_probes(unavailable, probe(RUSTFMT_COMMAND, &[]))
                },
            })
            .clone()
    }

    /// Select a formatter from probe results without consulting the host.
    pub(super) fn from_probes(nightly: FormatterProbe, path: FormatterProbe) -> Self {
        match (nightly, path) {
            (FormatterProbe::Available, _) => Self::Nightly,
            (FormatterProbe::Unavailable(_), FormatterProbe::Available) => Self::Path,
            (FormatterProbe::Unavailable(nightly), FormatterProbe::Unavailable(path)) => {
                Self::Unavailable {
                    cause: format!("{nightly}; {path}"),
                }
            },
        }
    }

    /// Format proposed sources without changing the source files.
    pub(super) fn formatted_batch(
        &self,
        sources: &BTreeMap<PathBuf, String>,
        workspace: &Path,
    ) -> Result<BTreeMap<PathBuf, String>> {
        let mut groups = BTreeMap::<FormattingGroup, Vec<(&PathBuf, &String)>>::new();
        for (path, text) in sources {
            let directory = path.parent().context("source has no parent directory")?;
            groups
                .entry(FormattingGroup {
                    edition:       edition(directory, workspace)?,
                    configuration: formatter_configuration(directory)?,
                })
                .or_default()
                .push((path, text));
        }
        let mut formatted = BTreeMap::new();
        for (group, members) in groups {
            // File input applies ignore rules that stdin does not apply.
            // Preserve stdin behavior when a configuration supplies those rules.
            let stdin_required = match &group.configuration {
                FormatterConfiguration::Default => false,
                FormatterConfiguration::File(path) => {
                    let configuration: Value = toml::from_str(&fs::read_to_string(path)?)?;
                    configuration.get("ignore").is_some()
                },
            };
            if !matches!(self, Self::Nightly) || stdin_required {
                for (path, text) in members {
                    formatted.insert(path.clone(), self.formatted(text, path, workspace)?);
                }
                continue;
            }
            let temporary =
                tempfile::tempdir().context("create formatter measurement directory")?;
            let configuration = match &group.configuration {
                FormatterConfiguration::File(path) => path.clone(),
                FormatterConfiguration::Default => {
                    let path = temporary.path().join("rustfmt.toml");
                    fs::write(&path, "")?;
                    path
                },
            };
            let mut command = nightly_command();
            command
                .args(["--emit", "files", "--quiet", "--edition"])
                .arg(&group.edition)
                .arg("--config-path")
                .arg(configuration)
                // Stdin formats generated sources. File input must do the same.
                .args(["--config", "skip_children=true,format_generated_files=true"])
                .current_dir(temporary.path());
            let mut staged = Vec::new();
            for (index, (path, text)) in members.into_iter().enumerate() {
                let staging = temporary.path().join(format!("measurement_{index}.rs"));
                fs::write(&staging, text)?;
                command.arg(&staging);
                staged.push((path, staging));
            }
            #[cfg(test)]
            FORMATTING_COMMANDS.update(|count| count + 1);
            let output = command
                .output()
                .context("start rustfmt for function measurements")?;
            ensure!(
                output.status.success(),
                "rustfmt could not measure the source group: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            for (path, staging) in staged {
                formatted.insert(path.clone(), fs::read_to_string(staging)?);
            }
        }
        Ok(formatted)
    }

    pub(super) fn formatted(&self, text: &str, path: &Path, workspace: &Path) -> Result<String> {
        let directory = path.parent().context("source has no parent directory")?;
        let mut command = match self {
            Self::Nightly => nightly_command(),
            Self::Path => Command::new(RUSTFMT_COMMAND),
            Self::Unavailable { cause } => anyhow::bail!("formatter unavailable: {cause}"),
        };
        command
            .args(RUSTFMT_ARGUMENTS)
            .arg(edition(directory, workspace)?)
            .args(RUSTFMT_LOCAL_FILE_ARGUMENTS)
            .current_dir(directory)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(test)]
        FORMATTING_COMMANDS.update(|count| count + 1);
        let mut child = command
            .spawn()
            .context("failed to start rustfmt for function measurement")?;
        child
            .stdin
            .take()
            .context("rustfmt stdin unavailable")?
            .write_all(text.as_bytes())?;
        let output = child.wait_with_output()?;
        ensure!(
            output.status.success(),
            "rustfmt could not measure {}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).context("rustfmt output is not UTF-8")
    }
}

fn formatter_configuration(directory: &Path) -> Result<FormatterConfiguration> {
    let directory = directory.canonicalize()?;
    let user_directories = [
        dirs::home_dir(),
        dirs::config_dir().map(|path| path.join("rustfmt")),
    ];
    for directory in directory
        .ancestors()
        .chain(user_directories.iter().flatten().map(PathBuf::as_path))
    {
        for name in [".rustfmt.toml", "rustfmt.toml"] {
            let path = directory.join(name);
            match fs::metadata(&path) {
                Ok(metadata) if metadata.is_file() => {
                    return Ok(FormatterConfiguration::File(path.canonicalize()?));
                },
                Ok(_) => {},
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                    ) => {},
                Err(error) => return Err(error.into()),
            }
        }
    }
    Ok(FormatterConfiguration::Default)
}

fn nightly_command() -> Command {
    static EXECUTABLE: OnceLock<NightlyExecutable> = OnceLock::new();
    match EXECUTABLE.get_or_init(|| {
        match Command::new(RUSTUP_COMMAND)
            .args(NIGHTLY_RUSTFMT_LOOKUP_ARGUMENTS)
            .output()
        {
            Ok(output) if output.status.success() => {
                let path = String::from_utf8_lossy(&output.stdout);
                if path.trim().is_empty() {
                    NightlyExecutable::Rustup
                } else {
                    NightlyExecutable::Direct(PathBuf::from(path.trim()))
                }
            },
            _ => NightlyExecutable::Rustup,
        }
    }) {
        NightlyExecutable::Direct(path) => Command::new(path),
        NightlyExecutable::Rustup => {
            let mut command = Command::new(RUSTUP_COMMAND);
            command.args(NIGHTLY_RUSTFMT_ARGUMENTS);
            command
        },
    }
}

fn nightly_probe() -> FormatterProbe {
    probe_result(
        nightly_command().arg(VERSION_ARGUMENT).output(),
        "nightly rustfmt",
        "nightly rustfmt",
    )
}

fn probe(program: &str, arguments: &[&str]) -> FormatterProbe {
    probe_result(
        Command::new(program)
            .args(arguments)
            .arg(VERSION_ARGUMENT)
            .output(),
        &format!("{program} {}", arguments.join(" ")),
        program,
    )
}

fn probe_result(
    output: std::io::Result<Output>,
    failure_label: &str,
    error_label: &str,
) -> FormatterProbe {
    match output {
        Ok(output) if output.status.success() => FormatterProbe::Available,
        Ok(output) => FormatterProbe::Unavailable(format!(
            "{failure_label}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )),
        Err(error) => FormatterProbe::Unavailable(format!("{error_label}: {error}")),
    }
}

pub(super) fn threshold(workspace: &Path) -> Result<usize> {
    for name in ["clippy.toml", ".clippy.toml"] {
        let path = workspace.join(name);
        if path.is_file() {
            let text = std::fs::read_to_string(&path)?;
            let config: Value = toml::from_str(&text)?;
            return config.get("too-many-lines-threshold").map_or(
                Ok(CLIPPY_DEFAULT_TOO_MANY_LINES_THRESHOLD),
                |value| {
                    usize::try_from(
                        value
                            .as_integer()
                            .context("invalid too-many-lines-threshold")?,
                    )
                    .context("invalid too-many-lines-threshold")
                },
            );
        }
    }
    Ok(CLIPPY_DEFAULT_TOO_MANY_LINES_THRESHOLD)
}

fn edition(directory: &Path, workspace: &Path) -> Result<String> {
    for ancestor in directory.ancestors() {
        let manifest = ancestor.join("Cargo.toml");
        if manifest.is_file() {
            let config: Value = toml::from_str(&std::fs::read_to_string(manifest)?)?;
            if let Some(package) = config.get("package") {
                let Some(edition) = package.get("edition") else {
                    return Ok(DEFAULT_EDITION.to_string());
                };
                if let Some(value) = edition.as_str() {
                    return Ok(value.to_string());
                }
                break;
            }
        }
        if ancestor == workspace {
            break;
        }
    }
    let manifest = workspace.join("Cargo.toml");
    if manifest.is_file() {
        let config: Value = toml::from_str(&std::fs::read_to_string(manifest)?)?;
        if let Some(value) = config
            .get("workspace")
            .and_then(|value| value.get("package"))
            .and_then(|value| value.get("edition"))
            .and_then(toml::Value::as_str)
        {
            return Ok(value.to_string());
        }
    }
    Ok(DEFAULT_EDITION.to_string())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;

    use super::Formatter;
    use super::FormatterProbe;

    #[test]
    fn batch_measurements_keep_configuration_edition_and_source_bytes() -> anyhow::Result<()> {
        let temporary = tempfile::tempdir()?;
        let mut sources = BTreeMap::new();
        let mut expected = BTreeMap::new();
        for (name, edition, configuration, indentation) in [
            (
                "spaces",
                "2024",
                "hard_tabs = false\nformat_generated_files = false\n",
                "    ",
            ),
            (
                "tabs",
                "2015",
                "hard_tabs = true\nformat_generated_files = false\n",
                "\t",
            ),
        ] {
            let root = temporary.path().join(name);
            fs::create_dir(&root)?;
            fs::write(root.join("rustfmt.toml"), configuration)?;
            fs::write(
                root.join("Cargo.toml"),
                format!("[package]\nname = {name:?}\nversion = \"0.1.0\"\nedition = {edition:?}\n"),
            )?;
            for (filename, source, result) in [
                (
                    "ordinary.rs",
                    if edition == "2024" {
                        "async fn alpha() { let value = 1; }\n"
                    } else {
                        "fn async() { let value = 1; }\n"
                    },
                    format!(
                        "{}() {{\n{indentation}let value = 1;\n}}\n",
                        if edition == "2024" {
                            "async fn alpha"
                        } else {
                            "fn async"
                        }
                    ),
                ),
                (
                    "generated.rs",
                    "// @generated\nfn beta() { let value = 2; }\n",
                    format!("// @generated\nfn beta() {{\n{indentation}let value = 2;\n}}\n"),
                ),
                (
                    "skipped.rs",
                    "#![rustfmt::skip]\nfn gamma( ){let value=3;}\n",
                    "#![rustfmt::skip]\nfn gamma( ){let value=3;}\n".to_owned(),
                ),
            ] {
                let path = root.join(filename);
                fs::write(&path, "source bytes must stay unchanged\n")?;
                sources.insert(path.clone(), source.to_owned());
                expected.insert(path, result);
            }
        }
        let formatted = Formatter::Nightly.formatted_batch(&sources, temporary.path())?;
        assert_eq!(formatted, expected);
        for path in sources.keys() {
            assert_eq!(
                fs::read_to_string(path)?,
                "source bytes must stay unchanged\n"
            );
        }
        Ok(())
    }

    #[test]
    fn formatter_choice_prefers_nightly_with_workspace_options() {
        assert_eq!(
            Formatter::from_probes(FormatterProbe::Available, FormatterProbe::Available),
            Formatter::Nightly
        );
    }

    #[test]
    fn formatter_choice_uses_path_without_rustup_or_nightly() {
        for cause in ["rustup not found", "nightly rustfmt not installed"] {
            assert_eq!(
                Formatter::from_probes(
                    FormatterProbe::Unavailable(cause.to_string()),
                    FormatterProbe::Available,
                ),
                Formatter::Path
            );
        }
    }

    #[test]
    fn formatter_choice_preserves_both_missing_formatter_causes() {
        let choice = Formatter::from_probes(
            FormatterProbe::Unavailable("rustup not found".to_string()),
            FormatterProbe::Unavailable("rustfmt not found".to_string()),
        );
        assert!(matches!(choice, Formatter::Unavailable { cause }
            if cause.contains("rustup not found") && cause.contains("rustfmt not found")));
    }
}
