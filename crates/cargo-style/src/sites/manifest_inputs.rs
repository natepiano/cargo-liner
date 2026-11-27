//! Read committed manifests and resolve explicit workspace members.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::btree_map::Entry;
use std::path::Path;
use std::rc::Rc;

use regex::Regex;
use toml::Table;
use toml::Value;

use super::constants::PYTHON_INPUT_ROOT;
use super::failure::SitesFailure;
use super::frontmatter;
use super::manifest_syntax;
use super::path_match;
use super::path_match::WorkspaceGlob;
use super::source_tree::SiteText;
use super::source_tree::SourceTree;
use crate::finding_source::FindingSource;
use crate::source_lines::SourceLines;
use crate::tip_view;

pub(super) enum Manifest {
    Empty,
    Parsed {
        source: Rc<SourceLines>,
        table:  Table,
    },
}

impl Manifest {
    pub(super) fn table(&self, key: &str) -> Option<&Table> {
        match self {
            Self::Parsed { table, .. } => table.get(key).and_then(Value::as_table),
            Self::Empty => None,
        }
    }

    pub(super) fn dependency_line(&self, name: &str, space: &str) -> Result<usize, SitesFailure> {
        let Self::Parsed { source, .. } = self else {
            return Ok(1);
        };
        let name = regex::escape(name);
        let assignment = Regex::new(&format!(r#"^[{space}]*"?{name}"?[{space}]*="#))
            .map_err(SitesFailure::DependencyPattern)?;
        Ok(source
            .lines()
            .map(frontmatter::without_line_ending)
            .position(|line| assignment.is_match(line))
            .map_or(1, |index| index + 1))
    }

    fn has_package(&self) -> bool {
        matches!(self, Self::Parsed { table, .. } if table.contains_key("package"))
    }
}

pub(super) struct ManifestBlobs<'tree> {
    tree:   &'tree SourceTree,
    parsed: BTreeMap<&'tree str, Manifest>,
}

impl<'tree> ManifestBlobs<'tree> {
    pub(super) const fn new(tree: &'tree SourceTree) -> Self {
        Self {
            tree,
            parsed: BTreeMap::new(),
        }
    }

    pub(super) fn validate_root(
        &mut self,
        source: &mut FindingSource<'_>,
    ) -> Result<(), SitesFailure> {
        let path = "Cargo.toml";
        if self.tree.blob(path).is_none() {
            return Ok(());
        }
        let valid = match self.tree.read_text(source, path)? {
            SiteText::Utf8(_) => matches!(self.read(source, path)?, Manifest::Parsed { .. }),
            SiteText::NotUtf8(text) => manifest_syntax::manifest_table(&text).is_some(),
        };
        if valid {
            Ok(())
        } else {
            Err(SitesFailure::RootManifest)
        }
    }

    pub(super) fn read(
        &mut self,
        source: &mut FindingSource<'_>,
        path: &str,
    ) -> Result<&Manifest, SitesFailure> {
        let Some(blob) = self.tree.blob(path) else {
            return Ok(&Manifest::Empty);
        };
        match self.parsed.entry(blob) {
            Entry::Occupied(entry) => Ok(entry.into_mut()),
            Entry::Vacant(entry) => {
                let SiteText::Utf8(text) = self.tree.read_text(source, path)? else {
                    return Err(SitesFailure::ManifestEncoding(path.to_owned()));
                };
                let manifest =
                    manifest_syntax::manifest_table(text.text()).map_or(Manifest::Empty, |table| {
                        Manifest::Parsed {
                            source: text,
                            table,
                        }
                    });
                Ok(entry.insert(manifest))
            },
        }
    }
}

pub(super) struct WorkspacePaths<'tree> {
    paths: BTreeSet<&'tree str>,
}

impl<'tree> From<&'tree SourceTree> for WorkspacePaths<'tree> {
    fn from(tree: &'tree SourceTree) -> Self {
        let mut paths = tree
            .blobs()
            .flat_map(|(path, _)| {
                std::iter::successors(Some(path.as_str()), |&path| {
                    path.rsplit_once('/').map(|(parent, _)| parent)
                })
            })
            .collect::<BTreeSet<_>>();
        if !paths.is_empty() {
            let _ = paths.insert(".");
        }
        Self { paths }
    }
}

impl WorkspacePaths<'_> {
    pub(super) fn contains(&self, path: &str) -> bool { self.paths.contains(path) }

    pub(super) fn members(&self, root: &Manifest) -> Result<Vec<String>, SitesFailure> {
        let Some(workspace) = root.table("workspace").filter(|table| !table.is_empty()) else {
            return Ok(Vec::new());
        };
        let mut manifests = Vec::new();
        if root.has_package() {
            manifests.push("Cargo.toml".to_owned());
        }
        let members = workspace
            .get("members")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str);
        for member in members {
            if member.contains('*') {
                let pattern = WorkspaceGlob::new(member).map_err(SitesFailure::WorkspacePattern)?;
                manifests.extend(
                    self.paths
                        .iter()
                        .filter(|path| pattern.matches(path))
                        .map(|path| manifest_path(path))
                        .filter(|path| self.contains(path)),
                );
            } else {
                let path = explicit_manifest_path(member)?;
                if self.contains(&path) {
                    manifests.push(path);
                }
            }
        }
        Ok(manifests)
    }
}

fn explicit_manifest_path(member: &str) -> Result<String, SitesFailure> {
    let manifest = manifest_path(member);
    let anchor = path_match::posix_anchor(member);
    if anchor.is_empty() {
        return Ok(manifest);
    }
    let root = Path::new(PYTHON_INPUT_ROOT);
    let path = Path::new(anchor).join(manifest);
    if anchor == path_match::posix_anchor(PYTHON_INPUT_ROOT)
        && let Ok(relative) = path.strip_prefix(root)
    {
        return Ok(relative.to_string_lossy().into_owned());
    }
    Err(SitesFailure::WorkspaceMemberOutsideRoot {
        path,
        root: root.to_path_buf(),
    })
}

fn manifest_path(member: &str) -> String {
    tip_view::path_parts(member)
        .chain(std::iter::once("Cargo.toml"))
        .collect::<Vec<_>>()
        .join("/")
}
