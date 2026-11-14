use std::collections::BTreeMap;
use std::collections::HashMap;
use std::path::Path;
use std::time::SystemTime;

use super::DirSizes;
use super::DiskModificationTime;
use super::DiskUsageTree;
use crate::project::AbsolutePath;

#[derive(Clone, Copy)]
pub(super) enum ContributionChange {
    Added,
    Removed,
}

#[derive(Debug, Default)]
struct DiskEntryTotals {
    total:        u64,
    target:       u64,
    non_target:   u64,
    source_times: BTreeMap<SystemTime, usize>,
}

impl DiskEntryTotals {
    fn apply(&mut self, contribution: DirSizes, change: ContributionChange) {
        match change {
            ContributionChange::Added => {
                self.total += contribution.total;
                self.target += contribution.in_project_target;
                self.non_target += contribution.in_project_non_target;
                if let Some(time) = contribution.max_source_mtime {
                    *self.source_times.entry(time).or_default() += 1;
                }
            },
            ContributionChange::Removed => {
                self.total -= contribution.total;
                self.target -= contribution.in_project_target;
                self.non_target -= contribution.in_project_non_target;
                if let Some(time) = contribution.max_source_mtime
                    && let Some(count) = self.source_times.get_mut(&time)
                {
                    *count -= 1;
                    if *count == 0 {
                        self.source_times.remove(&time);
                    }
                }
            },
        }
    }

    fn sizes(&self) -> DirSizes {
        DirSizes {
            total:                 self.total,
            in_project_target:     self.target,
            in_project_non_target: self.non_target,
            max_source_mtime:      self.source_times.last_key_value().map(|(time, _)| *time),
        }
    }
}

/// `DiskTreeTotals` updates entry totals from changed file contributions.
#[derive(Debug)]
pub(super) struct DiskTreeTotals {
    root:    AbsolutePath,
    entries: HashMap<AbsolutePath, DiskEntryTotals>,
}

impl DiskTreeTotals {
    pub(super) fn new(tree: &DiskUsageTree) -> Self {
        Self {
            root:    tree.root_abs_path.clone(),
            entries: tree
                .entries
                .iter()
                .map(|path| (path.clone(), DiskEntryTotals::default()))
                .collect(),
        }
    }

    pub(super) fn change_file(
        &mut self,
        path: &Path,
        bytes: u64,
        modified: DiskModificationTime,
        change: ContributionChange,
    ) {
        let mut contribution = DirSizes::default();
        let modified = match modified {
            DiskModificationTime::Available(time) => Some(time),
            DiskModificationTime::Unavailable => None,
        };
        contribution.add_file(bytes, path, modified);
        let mut current = path.parent();
        while let Some(directory) = current {
            if let Some(totals) = self.entries.get_mut(directory) {
                totals.apply(contribution, change);
            }
            if directory == self.root.as_path() {
                break;
            }
            current = directory.parent();
        }
    }

    pub(super) fn sizes(&self, tree: &DiskUsageTree) -> Vec<(AbsolutePath, DirSizes)> {
        tree.entries
            .iter()
            .map(|path| {
                (
                    path.clone(),
                    self.entries
                        .get(path)
                        .map_or_else(DirSizes::default, DiskEntryTotals::sizes),
                )
            })
            .collect()
    }
}
