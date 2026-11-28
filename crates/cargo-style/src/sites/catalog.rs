//! Read current rule pins and resolve member reach from the checkout.

use std::collections::BTreeMap;
use std::fs;
use std::io::Error;
use std::io::ErrorKind;
use std::path::Path;
use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;

use super::frontmatter;
use super::frontmatter::Field;
use super::frontmatter::Fields;
use crate::rule_snapshot::CandidateSurface;
use crate::rule_snapshot::Family;
use crate::rule_snapshot::RuleFamily;
use crate::rule_snapshot::Surface;
use crate::site_ledger::InputScope;
use crate::tip_view;
use crate::tip_view::WorkspaceMember;
use crate::tip_view::WorkspaceReadFailure;

pub(crate) struct Catalog {
    pub(super) checkout: String,
    pub(super) global:   String,
    rules:               Vec<Rule>,
}

#[derive(Clone, Copy, Deserialize, PartialEq, Eq)]
pub(crate) struct RulePolicy {
    pub(crate) tier:        Tier,
    pub(crate) family:      RuleFamily,
    pub(crate) input_scope: RuleInputScope,
    #[serde(default)]
    pub(crate) surface:     CandidateSurface,
}

pub(crate) struct Rule {
    pub(crate) id:      String,
    pub(crate) pin:     String,
    pub(crate) reach:   Reach,
    pub(crate) policy:  RulePolicy,
    pub(super) listing: Listing,
}

#[derive(Clone, Copy, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum Tier {
    A,
    B,
    C,
    D,
    E,
    #[serde(rename = "unclassified")]
    Unclassified,
}

#[derive(Clone, Copy, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RuleInputScope {
    File,
    Crate,
    Manifest,
}

impl From<RuleInputScope> for InputScope {
    fn from(scope: RuleInputScope) -> Self {
        match scope {
            RuleInputScope::File => Self::File,
            RuleInputScope::Crate => Self::Crate,
            RuleInputScope::Manifest => Self::Manifest,
        }
    }
}

#[derive(Clone, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub(crate) enum Reach {
    Repository,
    Member(String),
}

impl Reach {
    pub(crate) fn includes(&self, path: &str) -> bool {
        match self {
            Self::Repository => true,
            Self::Member(directory) => tip_view::under(path, directory),
        }
    }
}

pub(super) enum Generator {
    Regex(RegexSpec),
    FieldAffixes,
    EnumVariantStutter,
    FieldTypeStutter,
    StructFields(StructFieldsSpec),
    SingleImplTraits,
    AllowsWithoutReason,
    TestAllowBoilerplate,
    ObserverGuards,
    Literals { exemptions: Vec<String> },
    TestLiterals,
    ModuleRootItems,
    SubmoduleNames,
    WorkspaceDeps,
    BevyKanaUsage,
    Unsupported { kind: String, origin: PathBuf },
}

pub(super) enum Listing {
    Candidates(Generator),
    Prefilter(String),
    NoSites,
}

impl Listing {
    pub(super) const fn enumerates_sites(&self) -> bool {
        !matches!(
            self,
            Self::NoSites | Self::Candidates(Generator::Unsupported { .. })
        )
    }
}

pub(super) struct RegexSpec {
    pub(super) origin:     PathBuf,
    pub(super) pattern:    String,
    pub(super) exclusion:  Exclusion,
    pub(super) globs:      Vec<String>,
    pub(super) paths:      Vec<String>,
    pub(super) exemptions: Vec<String>,
}

pub(super) struct StructFieldsSpec {
    pub(super) origin:             PathBuf,
    pub(super) structs:            Vec<String>,
    pub(super) field_type_pattern: String,
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

#[derive(Debug, thiserror::Error)]
pub(crate) enum CatalogFailure {
    #[error("cannot read catalog path {path:?}")]
    Read {
        path:   PathBuf,
        #[source]
        source: Error,
    },
    #[error("{path}: {reason}")]
    InvalidRule { path: PathBuf, reason: String },
    #[error("cannot read the workspace at {root:?}")]
    Workspace {
        root:   PathBuf,
        source: WorkspaceReadFailure,
    },
    #[error("unknown rule {0}")]
    UnknownRule(String),
}

impl Catalog {
    pub(crate) fn read(checkout: &Path) -> Result<Self, CatalogFailure> {
        let checkout = canonical(checkout)?;
        let global = global_root()?;
        let docs = checkout.join("docs");
        let mut rules = Vec::new();
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

    pub(crate) fn rule(&self, id: &str) -> Result<&Rule, CatalogFailure> {
        self.rules
            .iter()
            .rev()
            .find(|rule| rule.id == id)
            .ok_or_else(|| CatalogFailure::UnknownRule(id.to_owned()))
    }

    pub(crate) fn discoveries(&self, id: &str) -> impl Iterator<Item = &Rule> {
        self.rules.iter().filter(move |rule| rule.id == id)
    }

    pub(crate) fn classified_rules(&self) -> impl Iterator<Item = &Rule> {
        self.rules
            .iter()
            .filter(|rule| rule.policy.tier != Tier::Unclassified)
    }

    pub(super) fn listing_pins(&self) -> impl Iterator<Item = (&str, &str)> {
        self.rules
            .iter()
            .filter(|rule| !matches!(rule.listing, Listing::NoSites))
            .map(|rule| (rule.id.as_str(), rule.pin.as_str()))
    }

    pub(super) fn has_regex_generator(&self) -> bool {
        self.generator_rules()
            .any(|(_, generator)| matches!(generator, Generator::Regex(_)))
    }

    pub(super) fn generator_rules(&self) -> impl Iterator<Item = (&Rule, &Generator)> {
        self.rules.iter().filter_map(|rule| match &rule.listing {
            Listing::Candidates(generator) => Some((rule, generator)),
            Listing::Prefilter(_) | Listing::NoSites => None,
        })
    }

    pub(super) fn prefilter_rules(&self) -> impl Iterator<Item = (&Rule, &str)> {
        self.rules.iter().filter_map(|rule| match &rule.listing {
            Listing::Prefilter(pattern) => Some((rule, pattern.as_str())),
            Listing::Candidates(_) | Listing::NoSites => None,
        })
    }
}

impl Rule {
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
        let (policy, listing) = classify(&fields, origin, &path).map_err(fail)?;
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
            reach: reach.clone(),
            policy,
            listing,
        })
    }
}

fn load_directory(
    directory: &Path,
    root: &Path,
    origin: Origin,
    reach: &Reach,
    rules: &mut Vec<Rule>,
) -> Result<(), CatalogFailure> {
    add_rules(markdown(directory)?, root, origin, reach, rules)
}

fn add_rules(
    paths: Vec<PathBuf>,
    root: &Path,
    origin: Origin,
    reach: &Reach,
    rules: &mut Vec<Rule>,
) -> Result<(), CatalogFailure> {
    for path in paths {
        rules.push(Rule::read(&path, root, origin, reach)?);
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
    let field_type_pattern = scalar(block, "field_type_pattern", "")?.to_owned();
    let globs = values(block, "globs")?;
    let paths = values(block, "paths")?;
    let exemptions = values(block, "paths_exempt")?;
    let structs = values(block, "structs")?;
    let generator = match kind {
        "regex" => Generator::Regex(RegexSpec {
            origin: path.to_owned(),
            pattern,
            exclusion,
            globs,
            paths,
            exemptions,
        }),
        "field_affixes" => Generator::FieldAffixes,
        "enum_variant_stutter" => Generator::EnumVariantStutter,
        "field_type_stutter" => Generator::FieldTypeStutter,
        "struct_fields" => Generator::StructFields(StructFieldsSpec {
            origin: path.to_owned(),
            structs,
            field_type_pattern,
        }),
        "single_impl_traits" => Generator::SingleImplTraits,
        "allows_without_reason" => Generator::AllowsWithoutReason,
        "test_allow_boilerplate" => Generator::TestAllowBoilerplate,
        "observer_guards" => Generator::ObserverGuards,
        "literals" => Generator::Literals { exemptions },
        "test_literals" => Generator::TestLiterals,
        "module_root_items" => Generator::ModuleRootItems,
        "submodule_names" => Generator::SubmoduleNames,
        "workspace_deps" => Generator::WorkspaceDeps,
        "bevy_kana_usage" => Generator::BevyKanaUsage,
        _ => Generator::Unsupported {
            kind:   kind.to_owned(),
            origin: path.to_owned(),
        },
    };
    Ok(Candidates::Declared { surface, generator })
}

/// Check the rule fields in the order of Python `_rule_from_text`, so the first refusal reason
/// is the same.
fn classify(fields: &Fields, origin: Origin, path: &Path) -> Result<(RulePolicy, Listing), String> {
    let candidates = candidates(fields, path)?;
    let classified = matches!(origin, Origin::Local)
        || ["tier", "family", "input_scope"]
            .iter()
            .any(|key| fields.contains_key(*key));
    let mut policy = if classified {
        let classification = read_classification(fields)?;
        if matches!(
            candidates,
            Candidates::Declared {
                surface: Surface::Unclassified,
                ..
            }
        ) {
            return Err("classified rule's candidates.surface must be whole or partial".to_owned());
        }
        classification
    } else {
        RulePolicy {
            tier:        Tier::Unclassified,
            family:      RuleFamily::Unspecified,
            input_scope: RuleInputScope::Crate,
            surface:     CandidateSurface::NoCandidates,
        }
    };
    let approval = scalar(fields, "approval", "")?;
    if !approval.is_empty() {
        if approval != "always" {
            return Err("approval must be always".to_owned());
        }
        if !matches!(policy.tier, Tier::B | Tier::C | Tier::D) {
            return Err("approval always requires a family".to_owned());
        }
    }
    let prefilter = match fields.get("pre_filter") {
        Some(Field::Scalar(value)) => value.as_str(),
        None => "",
        _ => return Err("pre_filter must be a scalar".to_owned()),
    };
    if policy.tier == Tier::B && matches!(candidates, Candidates::Absent) && prefilter.is_empty() {
        return Err("tier B requires candidates or pre_filter".to_owned());
    }
    let _ = scalar(fields, "mode", "")?;
    policy.surface = match &candidates {
        Candidates::Declared { surface, .. } => CandidateSurface::Declared(*surface),
        Candidates::Absent => CandidateSurface::NoCandidates,
    };
    let listing = match (policy.tier, candidates) {
        (Tier::B | Tier::D, Candidates::Declared { generator, .. }) => {
            Listing::Candidates(generator)
        },
        (Tier::B, Candidates::Absent) => Listing::Prefilter(prefilter.to_owned()),
        _ => Listing::NoSites,
    };
    Ok((policy, listing))
}

fn read_classification(fields: &Fields) -> Result<RulePolicy, String> {
    let tier = match scalar(fields, "tier", "")? {
        "A" => Tier::A,
        "B" => Tier::B,
        "C" => Tier::C,
        "D" => Tier::D,
        "E" => Tier::E,
        _ => return Err("tier must be A, B, C, D or E".to_owned()),
    };
    let mechanism = scalar(fields, "mechanism", "")?;
    if (tier == Tier::A) != matches!(mechanism, "clippy" | "mend" | "rustfmt") {
        return Err(
            "tier A requires a clippy, mend or rustfmt mechanism, and vice versa".to_owned(),
        );
    }
    let family = scalar(fields, "family", "")?;
    if matches!(tier, Tier::B | Tier::C | Tier::D) && family.is_empty() {
        return Err("family is required for tier B, C and D".to_owned());
    }
    if matches!(tier, Tier::A | Tier::E) && !family.is_empty() {
        return Err("family is forbidden for tier A and E".to_owned());
    }
    let family = match family {
        "" => RuleFamily::Unspecified,
        "names" => RuleFamily::Known(Family::Names),
        "values" => RuleFamily::Known(Family::Values),
        "types" => RuleFamily::Known(Family::Types),
        "modules" => RuleFamily::Known(Family::Modules),
        _ => return Err("family must be names, values, types or modules".to_owned()),
    };
    if tier == Tier::D && family != RuleFamily::Known(Family::Modules) {
        return Err("tier D requires the modules family".to_owned());
    }
    let scope = match scalar(fields, "input_scope", "crate")? {
        "file" => RuleInputScope::File,
        "crate" => RuleInputScope::Crate,
        "manifest" => RuleInputScope::Manifest,
        _ => return Err("input_scope must be file, crate or manifest".to_owned()),
    };
    Ok(RulePolicy {
        tier,
        family,
        input_scope: scope,
        surface: CandidateSurface::NoCandidates,
    })
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
    tip_view::workspace_members(&manifests).map_err(|source| CatalogFailure::Workspace {
        root: root.to_owned(),
        source,
    })
}
