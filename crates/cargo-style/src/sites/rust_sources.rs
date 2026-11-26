//! Read and mask each Rust blob once for a sites request.

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::path::Path;
use std::rc::Rc;

use super::catalog::Reach;
use super::catalog::Rule;
use super::ledger::ListedCoordinate;
use super::mask;
use super::mask::MaskedSource;
use super::source_tree::SourceReadFailure;
use super::source_tree::SourceTree;
use crate::finding_source::FindingSource;
use crate::finding_source::SourceText;
use crate::finding_source::Utf8Source;
use crate::source_lines::SiteLine;
use crate::tip_view;
use crate::tip_view::RepositoryTip;

pub(super) struct RustSources {
    blobs: BTreeMap<String, RustBlob>,
}

pub(super) struct RustBlob {
    pub(super) masked: MaskedSource,
    pub(super) paths:  Vec<String>,
    original:          OriginalSource,
}

enum OriginalSource {
    Utf8(Rc<Utf8Source>),
    Unreadable,
}

impl RustBlob {
    pub(super) fn display_line(&self, line: usize) -> String {
        if let OriginalSource::Utf8(source) = &self.original
            && let SiteLine::Text(text) = source.lines.line(line)
        {
            return mask::display_line(text, 1);
        }
        String::new()
    }
}

impl RustSources {
    pub(super) fn read(
        repo: &Path,
        source: &mut FindingSource<'_>,
        commit: &str,
    ) -> Result<Self, SourceReadFailure> {
        let tree = SourceTree::read(repo, source, commit)?;
        let mut blobs: BTreeMap<_, RustBlob> = BTreeMap::new();
        for (path, entry) in tree.blobs().filter(|(path, _)| rust_path(path)) {
            match blobs.entry(entry.object.clone()) {
                Entry::Occupied(mut blob) => blob.get_mut().paths.push(path.clone()),
                Entry::Vacant(slot) => {
                    let (masked, original) = match source.source(&entry.object)? {
                        SourceText::Utf8(source) => {
                            let text = source.lines.text();
                            let masked = if text.contains('\r') {
                                mask::mask_source(&text.replace("\r\n", "\n").replace('\r', "\n"))
                            } else {
                                mask::mask_source(text)
                            };
                            (masked, OriginalSource::Utf8(Rc::clone(source)))
                        },
                        SourceText::NotUtf8(_) => {
                            (mask::mask_source(""), OriginalSource::Unreadable)
                        },
                    };
                    let _ = slot.insert(RustBlob {
                        masked,
                        paths: vec![path.clone()],
                        original,
                    });
                },
            }
        }
        Ok(Self { blobs })
    }

    pub(super) fn blobs(&self) -> impl Iterator<Item = &RustBlob> { self.blobs.values() }
}

fn rust_path(path: &str) -> bool {
    tip_view::is_rust_path(path)
        && !tip_view::path_parts(path).any(|part| part == "target" || part.starts_with('.'))
}

pub(super) struct RustCoordinate {
    pub(super) line: usize,
    pub(super) text: String,
}

pub(super) fn listed<'a>(
    tip: &RepositoryTip,
    rule: &Rule,
    candidates: impl IntoIterator<Item = (impl IntoIterator<Item = &'a String>, Vec<RustCoordinate>)>,
) -> Vec<ListedCoordinate> {
    let mut listed = Vec::new();
    for (paths, coordinates) in candidates {
        for path in paths.into_iter().filter(|path| in_reach(tip, rule, path)) {
            listed.extend(coordinates.iter().map(|site| ListedCoordinate {
                path: path.clone(),
                line: site.line,
                text: site.text.clone(),
            }));
        }
    }
    listed.sort_unstable();
    listed.dedup();
    listed
}

fn in_reach(tip: &RepositoryTip, rule: &Rule, path: &str) -> bool {
    tip.has_rust_path(path)
        && match &rule.reach {
            Reach::Repository => true,
            Reach::Member(directory) => tip_view::under(path, directory),
        }
}
