//! Run the regex generator against committed file contents.

use std::collections::HashSet;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::io::ErrorKind;
use std::io::Seek as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;
use std::process::Stdio;
use std::string::FromUtf8Error;

use super::catalog::Exclusion;
use super::catalog::RegexSpec;
use super::catalog::Rule;
use super::constants::CLAUDE_PYTHON;
use super::constants::MATCH_EXCLUSIONS;
use super::constants::PACKAGE_PYTHONS;
use super::constants::PER_USER_PROFILES;
use super::constants::PROFILE_PYTHON;
use super::constants::PYTHON_COMMAND;
use super::constants::PYTHON_VERSION_CHECK;
use super::constants::SYSTEM_PYTHON;
use super::frontmatter;
use super::ledger::ListedCoordinate;
use super::mask;
use super::path_match;
use super::source_tree::SourceReadFailure;
use super::source_tree::SourceTree;
use crate::finding_source::FindingSource;
use crate::finding_source::FindingSourceFailure;
use crate::finding_source::SourceText;
use crate::source_lines;

pub(super) struct RegexTool {
    executable:        PathBuf,
    tree_ignore_paths: HashSet<PathBuf>,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum RegexFailure {
    #[error("cannot prepare or run the regex generator")]
    Process(#[from] io::Error),
    #[error("cannot read the rg configuration or output as UTF-8")]
    Encoding(#[from] FromUtf8Error),
    #[error("cannot encode or read the regex exclusion request")]
    ExclusionReply(#[from] serde_json::Error),
    #[error(transparent)]
    Source(#[from] FindingSourceFailure),
    #[error("candidates regex for {origin} failed (rg exit {status}): {diagnostic}")]
    Search {
        origin:     PathBuf,
        status:     String,
        diagnostic: String,
    },
    #[error("candidates kind=regex in {0} has no pattern")]
    EmptyPattern(PathBuf),
    #[error("path must be relative to the repository: {0}")]
    SearchPath(String),
    #[error("no python3 3.12 or newer is available for the exclusion pattern; set CLAUDE_PYTHON")]
    PythonUnavailable,
    #[error("{rule}: {diagnostic}")]
    Exclusion {
        rule:       String,
        diagnostic: String,
    },
    #[error(transparent)]
    SourceTree(#[from] SourceReadFailure),
}

impl RegexTool {
    pub(super) fn read() -> Result<Self, RegexFailure> {
        let executable = on_path(OsStr::new("rg"))
            .ok_or_else(|| io::Error::new(ErrorKind::NotFound, "rg is not on PATH"))?
            .canonicalize()?;
        let mut tree_ignore_paths = HashSet::new();
        if let Some(path) = std::env::var_os("RIPGREP_CONFIG_PATH").filter(|path| !path.is_empty())
        {
            let config = match fs::read(path) {
                Ok(bytes) => String::from_utf8(bytes)?,
                Err(error) if error.kind() == ErrorKind::NotFound => String::new(),
                Err(error) => return Err(error.into()),
            };
            let mut arguments = source_lines::split_lines(&config)
                .map(frontmatter::without_line_ending)
                .filter(|line| !line.is_empty() && !line.starts_with('#'))
                .peekable();
            while let Some(argument) = arguments.next() {
                let ignore = if argument == "--ignore-file" {
                    arguments.peek().copied()
                } else {
                    argument.strip_prefix("--ignore-file=")
                };
                if let Some(ignore) = ignore {
                    let ignore = Path::new(ignore);
                    if ignore.is_absolute() {
                        // A missing external input invalidates the observation.
                        let _ = fs::read(ignore)?;
                    } else {
                        // Python compares the `pathlib` form, which drops each `.` component.
                        let _ = tree_ignore_paths.insert(
                            ignore
                                .components()
                                .filter(|part| !matches!(part, Component::CurDir))
                                .collect(),
                        );
                    }
                }
            }
        }
        Ok(Self {
            executable,
            tree_ignore_paths,
        })
    }

    fn is_ignore(&self, path: &str) -> bool {
        let file = Path::new(path);
        file.file_name()
            .and_then(OsStr::to_str)
            .is_some_and(|name| matches!(name, ".ignore" | ".gitignore" | ".rgignore"))
            || self.tree_ignore_paths.contains(file)
    }

    fn search(
        &self,
        root: &Path,
        spec: &RegexSpec,
        operation: RgOperation,
    ) -> Result<Output, RegexFailure> {
        let mut command = Command::new(&self.executable);
        command.current_dir(root).stdin(Stdio::null());
        match operation {
            RgOperation::SelectFiles => command.args(["--files", "--null"]),
            RgOperation::MatchLines => {
                command.args(["--line-number", "--no-heading", "--color", "never"])
            },
        };
        let _ = command.args(["--glob", "!target/**"]);
        if spec.globs.is_empty() {
            let _ = command.args(["--type", "rust"]);
        }
        for glob in &spec.globs {
            let _ = command.arg(format!("--glob={glob}"));
        }
        if matches!(operation, RgOperation::MatchLines) {
            let _ = command.args(["-e", &spec.pattern]);
        }
        let _ = command.arg("--");
        if spec.paths.is_empty() {
            let _ = command.arg(".");
        }
        for path in &spec.paths {
            let _ = relative(path)?;
            let _ = command.arg(path);
        }

        let output = command.output()?;
        if !matches!(output.status.code(), Some(0 | 1)) {
            return Err(RegexFailure::Search {
                origin:     spec.origin.clone(),
                status:     output
                    .status
                    .code()
                    .map_or_else(|| "signal".to_owned(), |code| code.to_string()),
                diagnostic: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            });
        }
        Ok(output)
    }
}

#[derive(Clone, Copy)]
enum RgOperation {
    SelectFiles,
    MatchLines,
}

pub(super) fn enumerate(
    repo: &Path,
    source: &mut FindingSource<'_>,
    tree: &SourceTree,
    rule: &Rule,
    spec: &RegexSpec,
    tool: &RegexTool,
) -> Result<Vec<ListedCoordinate>, RegexFailure> {
    if spec.pattern.is_empty() {
        return Err(RegexFailure::EmptyPattern(spec.origin.clone()));
    }
    let temporary = tempfile::Builder::new().prefix("style-regex-").tempdir()?;
    let root = temporary.path();
    let blobs = tree.blobs();
    for (path, entry) in blobs.clone() {
        let target = root.join(relative(path)?);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        if tool.is_ignore(path) {
            write_blob(repo, source, &entry.object, &target)?;
        } else {
            let _ = fs::File::create(target)?;
        }
    }
    let selected = String::from_utf8(tool.search(root, spec, RgOperation::SelectFiles)?.stdout)?;
    let selected = selected
        .split_terminator('\0')
        .map(without_dot)
        .collect::<HashSet<_>>();
    for (path, entry) in blobs {
        if selected.contains(path.as_str()) && !tool.is_ignore(path) {
            write_blob(repo, source, &entry.object, &root.join(path))?;
        }
    }
    let matches = String::from_utf8(tool.search(root, spec, RgOperation::MatchLines)?.stdout)?;
    listed_coordinates(&matches, rule, spec)
}

fn without_dot(path: &str) -> &str { path.strip_prefix("./").unwrap_or(path) }

fn relative(path: &str) -> Result<PathBuf, RegexFailure> {
    let mut result = PathBuf::new();
    for part in Path::new(path).components() {
        match part {
            Component::Normal(part) => result.push(part),
            Component::CurDir => {},
            _ => return Err(RegexFailure::SearchPath(path.to_owned())),
        }
    }
    Ok(result)
}

fn write_blob(
    repo: &Path,
    source: &mut FindingSource<'_>,
    blob: &str,
    path: &Path,
) -> Result<(), RegexFailure> {
    match source.source(blob)? {
        SourceText::Utf8(text) => fs::write(path, text.lines.text())?,
        SourceText::NotUtf8(_) => {
            // The shared text reader replaces invalid bytes. Ripgrep needs the exact bytes.
            let output = Command::new("git")
                .arg("-C")
                .arg(repo)
                .args(["cat-file", "blob", blob])
                .output()?;
            if !output.status.success() {
                return Err(SourceReadFailure::MissingObject(blob.to_owned()).into());
            }
            fs::write(path, output.stdout)?;
        },
    }
    Ok(())
}

fn listed_coordinates(
    matches: &str,
    rule: &Rule,
    spec: &RegexSpec,
) -> Result<Vec<ListedCoordinate>, RegexFailure> {
    let matches = source_lines::split_lines(matches)
        .map(frontmatter::without_line_ending)
        .filter_map(|row| {
            let mut fields = row.splitn(3, ':');
            let path = without_dot(fields.next()?);
            let line = fields.next()?.parse::<usize>().ok()?;
            let text = fields.next()?;
            Some((path, line, text))
        })
        .filter(|(path, _, _)| !path_match::is_exempt(path, &spec.exemptions))
        .collect::<Vec<_>>();
    let excluded = match &spec.exclusion {
        Exclusion::None => vec![false; matches.len()],
        Exclusion::Pattern(pattern) => python_excludes(&rule.id, pattern, &matches)?,
    };

    let mut sites = Vec::new();
    for ((raw_path, line, text), excluded) in matches.into_iter().zip(excluded) {
        if excluded {
            continue;
        }
        let path = relative(raw_path)?.to_string_lossy().into_owned();
        sites.push(ListedCoordinate {
            path,
            line,
            text: mask::display_line(text, 1),
        });
    }
    Ok(sites)
}

fn python_excludes(
    rule: &str,
    pattern: &str,
    matches: &[(&str, usize, &str)],
) -> Result<Vec<bool>, RegexFailure> {
    let lines = matches.iter().map(|(_, _, text)| *text).collect::<Vec<_>>();
    python_matches(rule, pattern, &lines)
}

pub(super) fn python_matches(
    rule: &str,
    pattern: &str,
    lines: &[&str],
) -> Result<Vec<bool>, RegexFailure> {
    // Use Python re for rule patterns. Search all supplied strings in one process.
    let mut input = tempfile::tempfile()?;
    serde_json::to_writer(&mut input, &(pattern, lines))?;
    input.rewind()?;
    let output = Command::new(exclusion_python()?)
        .args(["-I", "-S", "-c", MATCH_EXCLUSIONS])
        .stdin(input)
        .output()?;
    if !output.status.success() {
        return Err(RegexFailure::Exclusion {
            rule:       rule.to_owned(),
            diagnostic: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    let result: Vec<bool> = serde_json::from_slice(&output.stdout)?;
    if result.len() != lines.len() {
        return Err(RegexFailure::Exclusion {
            rule:       rule.to_owned(),
            diagnostic: "exclusion reply has the wrong number of rows".to_owned(),
        });
    }
    Ok(result)
}

/// Select Python as `scripts/lib/py` does: try `CLAUDE_PYTHON`, then each known `python3`, and
/// take the first one that is 3.12 or newer.
fn exclusion_python() -> Result<PathBuf, RegexFailure> {
    let configured = std::env::var_os(CLAUDE_PYTHON)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    let profile = Path::new(PER_USER_PROFILES)
        .join(std::env::var_os("USER").unwrap_or_default())
        .join(PROFILE_PYTHON);
    configured
        .into_iter()
        .chain([
            PathBuf::from(PYTHON_COMMAND),
            PathBuf::from(SYSTEM_PYTHON),
            profile,
        ])
        .chain(PACKAGE_PYTHONS.map(PathBuf::from))
        .filter_map(|candidate| executable(&candidate))
        .find(|python| {
            Command::new(python)
                .args(["-I", "-c", PYTHON_VERSION_CHECK])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|status| status.success())
        })
        .ok_or(RegexFailure::PythonUnavailable)
}

/// Look up a bare command name on `PATH`, as the shell does. Use any other path as it is.
fn executable(candidate: &Path) -> Option<PathBuf> {
    match candidate.file_name() {
        Some(name) if name == candidate.as_os_str() => on_path(name),
        _ => is_executable(candidate).then(|| candidate.to_owned()),
    }
}

fn on_path(name: &OsStr) -> Option<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .map(|directory| directory.join(name))
        .find(|file| is_executable(file))
}

fn is_executable(file: &Path) -> bool {
    file.metadata()
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}
