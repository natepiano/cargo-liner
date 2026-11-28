//! Filter candidate paths by the synced tree, ignores and rule reach.

use super::catalog::Reach;
use super::failure::SitesFailure;
use super::source_tree::SourceTree;
use crate::finding_source::FindingSource;
use crate::tip_view;
use crate::tip_view::IgnoreEncoding;
use crate::tip_view::IgnorePaths;

pub(super) struct SiteScope<'a> {
    tree:    &'a SourceTree,
    ignored: IgnorePaths,
}

impl<'a> SiteScope<'a> {
    pub(super) fn read(
        tree: &'a SourceTree,
        source: &mut FindingSource<'_>,
    ) -> Result<Self, SitesFailure> {
        Ok(Self {
            tree,
            ignored: IgnorePaths::read(source, IgnoreEncoding::Strict, |path| tree.blob(path))?,
        })
    }

    pub(super) fn includes(&self, path: &str, reach: &Reach) -> bool {
        self.tree.blob(path).is_some() && !self.ignored.matches(path) && reach.includes(path)
    }

    pub(super) fn rust_paths(&self) -> impl Iterator<Item = &str> {
        self.tree
            .blobs()
            .map(|(path, _)| path.as_str())
            .filter(|path| tip_view::is_rust_path(path) && !self.ignored.matches(path))
    }
}
