#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RebuildStatus {
    Needed,
    #[default]
    NotNeeded,
}

impl RebuildStatus {
    pub const fn needs_rebuild(self) -> bool { matches!(self, Self::Needed) }

    pub const fn merge_needed(&mut self, needs_rebuild: bool) {
        if needs_rebuild {
            *self = Self::Needed;
        }
    }
}

#[derive(Default)]
pub struct PollBackgroundStats {
    pub bg_msgs:                usize,
    pub disk_usage_msgs:        usize,
    pub git_info_msgs:          usize,
    pub lint_status_msgs:       usize,
    pub language_progress_msgs: usize,
    pub ci_msgs:                usize,
    pub example_msgs:           usize,
    pub tree_results:           usize,
    pub fit_results:            usize,
    pub disk_results:           usize,
    pub rebuild_status:         RebuildStatus,
}
