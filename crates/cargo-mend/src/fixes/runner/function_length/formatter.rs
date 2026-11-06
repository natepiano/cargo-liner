use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::process::Stdio;

use anyhow::Context;
use anyhow::Result;
use anyhow::ensure;
use toml::Value;

use super::constants::CLIPPY_DEFAULT_TOO_MANY_LINES_THRESHOLD;
use super::constants::DEFAULT_EDITION;
use super::constants::NIGHTLY_RUSTFMT_ARGUMENTS;
use super::constants::RUSTFMT_ARGUMENTS;
use super::constants::RUSTFMT_COMMAND;
use super::constants::RUSTFMT_LOCAL_FILE_ARGUMENTS;
use super::constants::RUSTUP_COMMAND;
use super::constants::VERSION_ARGUMENT;

/// The formatter available for function length protection.
#[derive(Debug, PartialEq, Eq)]
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

impl Formatter {
    pub(super) fn detect() -> Self {
        Self::from_probes(
            probe(RUSTUP_COMMAND, NIGHTLY_RUSTFMT_ARGUMENTS),
            probe(RUSTFMT_COMMAND, &[]),
        )
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

    pub(super) fn formatted(&self, text: &str, path: &Path, workspace: &Path) -> Result<String> {
        let directory = path.parent().context("source has no parent directory")?;
        let mut command = match self {
            Self::Nightly => {
                let mut command = Command::new(RUSTUP_COMMAND);
                command.args(NIGHTLY_RUSTFMT_ARGUMENTS);
                command
            },
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

fn probe(program: &str, arguments: &[&str]) -> FormatterProbe {
    match Command::new(program)
        .args(arguments)
        .arg(VERSION_ARGUMENT)
        .output()
    {
        Ok(output) if output.status.success() => FormatterProbe::Available,
        Ok(output) => FormatterProbe::Unavailable(format!(
            "{program} {}: {}",
            arguments.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )),
        Err(error) => FormatterProbe::Unavailable(format!("{program}: {error}")),
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
    use super::Formatter;
    use super::FormatterProbe;

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
