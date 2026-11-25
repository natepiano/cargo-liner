//! Read current rule pins and resolve member reach from the checkout.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::HashSet;
use std::fs;
use std::io;
use std::io::ErrorKind;
use std::path::Path;
use std::path::PathBuf;

use toml::Table;
use toml::Value;

use super::UnavailableReason;
use super::frontmatter;
use super::frontmatter::Field;
use super::frontmatter::Fields;
use super::path_match;
use super::path_match::WorkspacePattern;

pub(super) struct Catalog {
    pub(super) checkout: String,
    pub(super) global:   String,
    rules:               BTreeMap<String, Rule>,
}

pub(super) struct Rule {
    pub(super) id:      String,
    pub(super) pin:     String,
    pub(super) reach:   Reach,
    pub(super) listing: Listing,
    refreshes_sites:    bool,
}

pub(super) enum Reach {
    Repository,
    Member(String),
}

pub(super) enum Listing {
    Regex(RegexSpec),
    /// Python lists no sites for a rule without a tier B or D candidates generator, such as a
    /// tier A, C or E rule.
    NoSites,
    Unavailable(UnavailableReason),
}

pub(super) struct RegexSpec {
    pub(super) origin:     PathBuf,
    pub(super) pattern:    String,
    pub(super) exclusion:  Exclusion,
    pub(super) globs:      Vec<String>,
    pub(super) paths:      Vec<String>,
    pub(super) exemptions: Vec<String>,
}

pub(super) enum Exclusion {
    None,
    Pattern(String),
}

#[derive(Clone, Copy)]
enum Origin {
    Global,
    Local,
}

struct WorkspaceMember {
    directory: String,
    package:   String,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum CatalogFailure {
    #[error("cannot read catalog path {path:?}")]
    Read {
        path:   PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("{path}: {reason}")]
    InvalidRule { path: PathBuf, reason: String },
    #[error("invalid workspace manifest {path:?}")]
    Manifest {
        path:   PathBuf,
        #[source]
        source: Box<toml::de::Error>,
    },
    #[error("invalid workspace path pattern")]
    Pattern(#[from] regex::Error),
    #[error("unknown rule {0}")]
    UnknownRule(String),
}

impl Catalog {
    pub(super) fn read(checkout: &Path) -> Result<Self, CatalogFailure> {
        let checkout = canonical(checkout)?;
        let global = global_root()?;
        let docs = checkout.join("docs");
        let mut rules = BTreeMap::new();
        load_directory(
            &global.join("rust"),
            &global,
            Origin::Global,
            &Reach::Repository,
            &mut rules,
        )?;
        load_directory(
            &docs.join("style"),
            &checkout,
            Origin::Local,
            &Reach::Repository,
            &mut rules,
        )?;

        let directories = children(&docs)?
            .into_iter()
            .map(|path| path.join("style"))
            .filter(|path| path.is_dir())
            .collect::<Vec<_>>();
        if !directories.is_empty() {
            let members = checkout_members(&checkout)?;
            for directory in directories {
                let paths = markdown(&directory)?;
                if paths.is_empty() {
                    continue;
                }
                let member = directory
                    .parent()
                    .and_then(Path::file_name)
                    .and_then(|name| name.to_str())
                    .unwrap_or_default();
                let reach = members
                    .iter()
                    .find(|item| {
                        item.package == member
                            || checkout
                                .join(&item.directory)
                                .canonicalize()
                                .is_ok_and(|path| {
                                    path.file_name().is_some_and(|name| name == member)
                                })
                    })
                    .ok_or_else(|| CatalogFailure::InvalidRule {
                        path:   paths[0].clone(),
                        reason: format!("member-local rule names no workspace member {member}"),
                    })?;
                add_rules(
                    paths,
                    &checkout,
                    Origin::Local,
                    &Reach::Member(reach.directory.clone()),
                    &mut rules,
                )?;
            }
        }
        Ok(Self {
            checkout: checkout.to_string_lossy().into_owned(),
            global: global.to_string_lossy().into_owned(),
            rules,
        })
    }

    pub(super) fn rule(&self, id: &str) -> Result<&Rule, CatalogFailure> {
        self.rules
            .get(id)
            .ok_or_else(|| CatalogFailure::UnknownRule(id.to_owned()))
    }

    pub(super) fn pins(&self) -> BTreeSet<(&str, &str)> {
        self.rules
            .values()
            .filter(|rule| rule.refreshes_sites)
            .map(|rule| (rule.id.as_str(), rule.pin.as_str()))
            .collect()
    }
}

impl Rule {
    pub(super) const fn is_regex(&self) -> bool { matches!(self.listing, Listing::Regex(_)) }

    fn read(
        path: &Path,
        root: &Path,
        origin: Origin,
        reach: &Reach,
    ) -> Result<Self, CatalogFailure> {
        let path = canonical(path)?;
        let fail = |reason: String| CatalogFailure::InvalidRule {
            path: path.clone(),
            reason,
        };
        let relative = path
            .strip_prefix(root)
            .map_err(|error| fail(error.to_string()))?;
        let bytes = fs::read(&path).map_err(|source| CatalogFailure::Read {
            path: path.clone(),
            source,
        })?;
        let text = std::str::from_utf8(&bytes).map_err(|error| fail(error.to_string()))?;
        let fields = frontmatter::parse(text).map_err(fail)?;
        let (listing, refreshes_sites) = classify(&fields, origin, &path).map_err(fail)?;
        let pin = gix::objs::compute_hash(gix::hash::Kind::Sha1, gix::objs::Kind::Blob, &bytes)
            .map_err(|error| fail(error.to_string()))?
            .to_string();
        let prefix = match origin {
            Origin::Global => "global",
            Origin::Local => "local",
        };
        Ok(Self {
            id: format!("{prefix}:{}", relative.display()),
            pin,
            reach: match reach {
                Reach::Repository => Reach::Repository,
                Reach::Member(path) => Reach::Member(path.clone()),
            },
            listing,
            refreshes_sites,
        })
    }
}

fn load_directory(
    directory: &Path,
    root: &Path,
    origin: Origin,
    reach: &Reach,
    rules: &mut BTreeMap<String, Rule>,
) -> Result<(), CatalogFailure> {
    add_rules(markdown(directory)?, root, origin, reach, rules)
}

fn add_rules(
    paths: Vec<PathBuf>,
    root: &Path,
    origin: Origin,
    reach: &Reach,
    rules: &mut BTreeMap<String, Rule>,
) -> Result<(), CatalogFailure> {
    for path in paths {
        let rule = Rule::read(&path, root, origin, reach)?;
        let _ = rules.insert(rule.id.clone(), rule);
    }
    Ok(())
}

fn global_root() -> Result<PathBuf, CatalogFailure> {
    let root = std::env::var_os("STYLE_HISTORY_NATE_STYLE_DIR").map_or_else(
        || dirs::home_dir().unwrap_or_default().join("rust/nate_style"),
        PathBuf::from,
    );
    match fs::canonicalize(&root) {
        Ok(root) => Ok(root),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            std::path::absolute(&root).map_err(|source| CatalogFailure::Read { path: root, source })
        },
        Err(source) => Err(CatalogFailure::Read { path: root, source }),
    }
}

fn canonical(path: &Path) -> Result<PathBuf, CatalogFailure> {
    path.canonicalize().map_err(|source| CatalogFailure::Read {
        path: path.to_owned(),
        source,
    })
}

fn children(directory: &Path) -> Result<Vec<PathBuf>, CatalogFailure> {
    let read = |source| CatalogFailure::Read {
        path: directory.to_owned(),
        source,
    };
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if matches!(error.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) => {
            return Ok(Vec::new());
        },
        Err(error) => return Err(read(error)),
    };
    let mut paths = entries
        .map(|entry| entry.map(|entry| entry.path()).map_err(read))
        .collect::<Result<Vec<_>, _>>()?;
    paths.sort_unstable();
    Ok(paths)
}

fn markdown(directory: &Path) -> Result<Vec<PathBuf>, CatalogFailure> {
    Ok(children(directory)?
        .into_iter()
        .filter(|path| {
            path.extension().is_some_and(|extension| extension == "md") && path.is_file()
        })
        .collect())
}

fn scalar<'a>(fields: &'a Fields, key: &str, default: &'a str) -> Result<&'a str, String> {
    match fields.get(key) {
        None => Ok(default),
        Some(Field::Scalar(text)) if !text.trim().is_empty() => Ok(text),
        _ => Err(format!("{key} must be a nonempty scalar")),
    }
}

fn values(fields: &Fields, key: &str) -> Result<Vec<String>, String> {
    match fields.get(key) {
        None => Ok(Vec::new()),
        Some(Field::Scalar(value)) => Ok(vec![value.clone()]),
        Some(Field::List(values)) => Ok(values.clone()),
        Some(Field::Candidates(_)) => Err(format!("{key} is a mapping, not scalar values")),
    }
}

enum Candidates {
    Absent,
    Declared {
        surface:   Surface,
        generator: Generator,
    },
}

enum Surface {
    Unclassified,
    Whole,
    Partial,
}

enum Generator {
    Other,
    Regex(RegexSpec),
}

fn candidates(fields: &Fields, path: &Path) -> Result<Candidates, String> {
    let block = match fields.get("candidates") {
        None => return Ok(Candidates::Absent),
        Some(Field::Candidates(block)) => block,
        _ => return Err("candidates must be a mapping".to_owned()),
    };
    let kind = scalar(block, "kind", "")?;
    if kind.is_empty() {
        return Err("candidates.kind is required".to_owned());
    }
    let surface = match scalar(block, "surface", "")? {
        "whole" => Surface::Whole,
        "partial" => Surface::Partial,
        _ if block.contains_key("surface") => {
            return Err("candidates.surface must be whole or partial".to_owned());
        },
        _ => Surface::Unclassified,
    };
    if matches!(surface, Surface::Partial) && scalar(block, "uncovered", "")?.is_empty() {
        return Err("partial candidates.surface requires uncovered".to_owned());
    }
    let _ = scalar(block, "uncovered", "")?;
    let pattern = scalar(block, "pattern", "")?.to_owned();
    let exclusion = match scalar(block, "exclude_pattern", "")? {
        "" => Exclusion::None,
        value => Exclusion::Pattern(value.to_owned()),
    };
    let _ = scalar(block, "field_type_pattern", "")?;
    let globs = values(block, "globs")?;
    let paths = values(block, "paths")?;
    let exemptions = values(block, "paths_exempt")?;
    let _ = values(block, "structs")?;
    let generator = if kind == "regex" {
        Generator::Regex(RegexSpec {
            origin: path.to_owned(),
            pattern,
            exclusion,
            globs,
            paths,
            exemptions,
        })
    } else {
        Generator::Other
    };
    Ok(Candidates::Declared { surface, generator })
}

/// Check the rule fields in the order of Python `_rule_from_text`, so the first refusal reason
/// is the same.
fn classify(fields: &Fields, origin: Origin, path: &Path) -> Result<(Listing, bool), String> {
    let candidates = candidates(fields, path)?;
    let classified = matches!(origin, Origin::Local)
        || ["tier", "family", "input_scope"]
            .iter()
            .any(|key| fields.contains_key(*key));
    let tier = if classified {
        let tier = read_classification(fields)?;
        if matches!(
            candidates,
            Candidates::Declared {
                surface: Surface::Unclassified,
                ..
            }
        ) {
            return Err("classified rule's candidates.surface must be whole or partial".to_owned());
        }
        tier
    } else {
        ""
    };
    let approval = scalar(fields, "approval", "")?;
    if !approval.is_empty() {
        if approval != "always" {
            return Err("approval must be always".to_owned());
        }
        if !matches!(tier, "B" | "C" | "D") {
            return Err("approval always requires a family".to_owned());
        }
    }
    let filtered = match fields.get("pre_filter") {
        Some(Field::Scalar(value)) => !value.is_empty(),
        None => false,
        _ => return Err("pre_filter must be a scalar".to_owned()),
    };
    if tier == "B" && matches!(candidates, Candidates::Absent) && !filtered {
        return Err("tier B requires candidates or pre_filter".to_owned());
    }
    let _ = scalar(fields, "mode", "")?;
    Ok(match (tier, candidates) {
        (
            "B" | "D",
            Candidates::Declared {
                generator: Generator::Regex(spec),
                ..
            },
        ) => (Listing::Regex(spec), true),
        (
            "B" | "D",
            Candidates::Declared {
                generator: Generator::Other,
                ..
            },
        ) => (
            Listing::Unavailable(UnavailableReason::UnsupportedGenerator),
            true,
        ),
        ("B", Candidates::Absent) if filtered => (
            Listing::Unavailable(UnavailableReason::PrefilterRequiresSharedMembership),
            true,
        ),
        _ => (Listing::NoSites, false),
    })
}

fn read_classification(fields: &Fields) -> Result<&str, String> {
    let tier = scalar(fields, "tier", "")?;
    if !matches!(tier, "A" | "B" | "C" | "D" | "E") {
        return Err("tier must be A, B, C, D or E".to_owned());
    }
    let mechanism = scalar(fields, "mechanism", "")?;
    if (tier == "A") != matches!(mechanism, "clippy" | "mend" | "rustfmt") {
        return Err(
            "tier A requires a clippy, mend or rustfmt mechanism, and vice versa".to_owned(),
        );
    }
    let family = scalar(fields, "family", "")?;
    if matches!(tier, "B" | "C" | "D") && family.is_empty() {
        return Err("family is required for tier B, C and D".to_owned());
    }
    if matches!(tier, "A" | "E") && !family.is_empty() {
        return Err("family is forbidden for tier A and E".to_owned());
    }
    if !matches!(family, "" | "names" | "values" | "types" | "modules") {
        return Err("family must be names, values, types or modules".to_owned());
    }
    if tier == "D" && family != "modules" {
        return Err("tier D requires the modules family".to_owned());
    }
    if !matches!(
        scalar(fields, "input_scope", "crate")?,
        "file" | "crate" | "manifest"
    ) {
        return Err("input_scope must be file, crate or manifest".to_owned());
    }
    Ok(tier)
}

fn checkout_members(root: &Path) -> Result<Vec<WorkspaceMember>, CatalogFailure> {
    let mut directories = vec![root.to_owned()];
    let mut manifests = BTreeMap::new();
    while let Some(directory) = directories.pop() {
        for path in children(&directory)? {
            if path
                .symlink_metadata()
                .is_ok_and(|metadata| metadata.is_dir())
            {
                directories.push(path);
            } else if path.file_name().is_some_and(|name| name == "Cargo.toml") && path.is_file() {
                let relative = directory.strip_prefix(root).unwrap_or(&directory);
                let name = if relative.as_os_str().is_empty() {
                    ".".to_owned()
                } else {
                    relative.to_string_lossy().into_owned()
                };
                let text = fs::read_to_string(&path).map_err(|source| CatalogFailure::Read {
                    path: path.clone(),
                    source,
                })?;
                let _ = manifests.insert(name, text);
            }
        }
    }
    let workspace = manifest(root, ".", manifests.get(".").map_or("", String::as_str))?;
    let workspace = table(&workspace, "workspace");
    let members = workspace_patterns(workspace, "members")?;
    let exclude = workspace_patterns(workspace, "exclude")?;
    let inherited = workspace.and_then(|workspace| table(workspace, "dependencies"));
    let mut pending = manifests
        .keys()
        .filter(|name| **name == "." || matches_any(name, &members))
        .cloned()
        .collect::<Vec<_>>();
    let mut visited = HashSet::new();
    let mut found = Vec::new();
    while let Some(directory) = pending.pop() {
        if !visited.insert(directory.clone())
            || Path::new(&directory)
                .ancestors()
                .any(|part| matches_any(part.to_str().unwrap_or_default(), &exclude))
        {
            continue;
        }
        let current = manifest(root, &directory, &manifests[&directory])?;
        let Some(package) = table(&current, "package")
            .and_then(|package| package.get("name"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        let targets = table(&current, "target")
            .into_iter()
            .flat_map(Table::values)
            .filter_map(Value::as_table);
        for owner in std::iter::once(&current).chain(targets) {
            for path in dependencies(owner, inherited, &directory) {
                if manifests.contains_key(&path) {
                    pending.push(path);
                }
            }
        }
        found.push(WorkspaceMember {
            directory,
            package: package.to_owned(),
        });
    }
    found.sort_by(|left, right| {
        path_match::path_parts(&right.directory)
            .count()
            .cmp(&path_match::path_parts(&left.directory).count())
            .then_with(|| left.directory.cmp(&right.directory))
    });
    Ok(found)
}

fn table<'a>(owner: &'a Table, key: &str) -> Option<&'a Table> {
    owner.get(key).and_then(Value::as_table)
}

fn workspace_patterns(
    workspace: Option<&Table>,
    field: &str,
) -> Result<Vec<WorkspacePattern>, CatalogFailure> {
    workspace
        .and_then(|workspace| workspace.get(field))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|pattern| !pattern.starts_with('/'))
        .map(WorkspacePattern::new)
        .collect::<Result<_, _>>()
        .map_err(CatalogFailure::from)
}

fn matches_any(path: &str, patterns: &[WorkspacePattern]) -> bool {
    patterns.iter().any(|pattern| pattern.matches(path))
}

fn manifest(root: &Path, directory: &str, text: &str) -> Result<Table, CatalogFailure> {
    text.parse().map_err(|source| CatalogFailure::Manifest {
        path:   root.join(directory).join("Cargo.toml"),
        source: Box::new(source),
    })
}

fn dependencies<'a>(
    owner: &'a Table,
    inherited: Option<&'a Table>,
    directory: &'a str,
) -> impl Iterator<Item = String> + 'a {
    ["dependencies", "dev-dependencies", "build-dependencies"]
        .into_iter()
        .filter_map(|kind| table(owner, kind))
        .flat_map(Table::iter)
        .filter_map(move |(name, value)| {
            let dependency = value.as_table()?;
            let (dependency, base) =
                if dependency.get("workspace").and_then(Value::as_bool) == Some(true) {
                    (table(inherited?, name)?, ".")
                } else {
                    (dependency, directory)
                };
            let path = dependency.get("path")?.as_str()?;
            if path.starts_with('/') {
                return None;
            }
            let mut parts = Vec::new();
            for part in path_match::path_parts(base).chain(path_match::path_parts(path)) {
                if part == ".." {
                    let _ = parts.pop()?;
                } else {
                    parts.push(part);
                }
            }
            Some(if parts.is_empty() {
                ".".to_owned()
            } else {
                parts.join("/")
            })
        })
}
