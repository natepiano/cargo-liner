use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use toml::Value;

use crate::fixes::runner::function_length::constants::CLIPPY_DEFAULT_TOO_MANY_LINES_THRESHOLD;
use crate::fixes::runner::function_length::constants::DEFAULT_EDITION;

#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum FormatterConfiguration {
    Default,
    File(PathBuf),
}

/// Files in this group use the same parser edition and formatter configuration.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct FormattingGroup {
    pub(super) edition: String,
    configuration:      FormatterConfiguration,
}

impl FormattingGroup {
    pub(super) fn from_sources<'source>(
        sources: &'source BTreeMap<PathBuf, String>,
        workspace: &Path,
    ) -> Result<FormattingGroups<'source>> {
        let mut groups = FormattingGroups::new();
        for (path, text) in sources {
            let directory = path.parent().context("source has no parent directory")?;
            groups
                .entry(Self {
                    edition:       edition(directory, workspace)?,
                    configuration: formatter_configuration(directory)?,
                })
                .or_default()
                .push((path, text));
        }
        Ok(groups)
    }

    pub(super) fn requires_stdin(&self) -> Result<bool> {
        // Use stdin when the configuration contains ignore rules.
        // Rustfmt applies those rules to file input only.
        match &self.configuration {
            FormatterConfiguration::Default => Ok(false),
            FormatterConfiguration::File(path) => {
                let configuration: Value = toml::from_str(&fs::read_to_string(path)?)?;
                Ok(configuration.get("ignore").is_some())
            },
        }
    }

    pub(super) fn configuration_path(&self, directory: &Path) -> Result<PathBuf> {
        match &self.configuration {
            FormatterConfiguration::File(path) => Ok(path.clone()),
            FormatterConfiguration::Default => {
                let path = directory.join("rustfmt.toml");
                fs::write(&path, "")?;
                Ok(path)
            },
        }
    }
}

type FormattingGroups<'source> =
    BTreeMap<FormattingGroup, Vec<(&'source PathBuf, &'source String)>>;

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

pub(in crate::fixes::runner::function_length) fn threshold(workspace: &Path) -> Result<usize> {
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

pub(super) fn edition(directory: &Path, workspace: &Path) -> Result<String> {
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
