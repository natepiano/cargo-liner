use super::AbsolutePath;
use super::App;
use super::Itertools;
use super::LintRun;
use super::lint;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LintsProjectKind {
    Rust,
    #[default]
    NonRust,
}

impl LintsProjectKind {
    const fn from_is_rust(is_rust: bool) -> Self {
        if is_rust { Self::Rust } else { Self::NonRust }
    }

    pub const fn is_rust(self) -> bool { matches!(self, Self::Rust) }
}

#[derive(Clone, Default)]
pub struct LintsData {
    pub runs:         Vec<LintRun>,
    /// Archive-directory size in bytes for each run, aligned by index with
    /// `runs`.
    /// Per-run archive size aligned with `runs`. `None` means the run has
    /// no archive entry yet; `Some(0)` means the archive exists and is
    /// empty. The renderer renders `None` as "—" and `Some(_)` as a byte
    /// count, distinguishing missing data from known-empty.
    pub sizes:        Vec<Option<u64>>,
    /// The checkout(s) the runs belong to: one path for a single project,
    /// one per visible checkout when a worktree-group parent row aggregates
    /// every checkout's history. `owner_of` indexes into this, so the
    /// per-run cost is a plain index — the paths are stored once, not
    /// cloned per run.
    pub owner_paths:  Vec<AbsolutePath>,
    /// Per-run index into `owner_paths`, aligned with `runs`. Identifies
    /// which checkout each run came from so `open_lint_run_output` resolves
    /// its logs against the right cache directory.
    pub owner_of:     Vec<usize>,
    pub project_kind: LintsProjectKind,
}

impl LintsData {
    pub const fn has_runs(&self) -> bool { !self.runs.is_empty() }

    /// The checkout path owning the run at `index`, used to resolve its
    /// archived log files. Falls back to the first owner when the index
    /// map is short (single-project rows share one owner).
    pub fn owner_path_for_run(&self, index: usize) -> Option<&AbsolutePath> {
        let owner = self.owner_of.get(index).copied().unwrap_or(0);
        self.owner_paths.get(owner)
    }
}

pub fn build_lints_data(app: &App) -> LintsData {
    let is_rust = app
        .project_list
        .selected_project_path()
        .is_some_and(|path| app.project_list.is_rust_at_path(path));

    // Worktree-group parent row: merge every visible checkout's history so
    // the list isn't limited to the primary's path. Each checkout's runs
    // are read through the same `lint_at_path` a single project uses.
    if let Some(paths) = app.project_list.selected_worktree_group_checkout_paths() {
        return aggregate_group_lints(app, paths, is_rust);
    }

    let selected_path = app.project_list.selected_project_path();
    let lint_runs = selected_path.and_then(|path| {
        app.lint_at_path(path)
            .or_else(|| app.project_list.vendored_owner_lint(path))
    });
    let (runs, sizes) = lint_runs.map_or_else(
        || (Vec::new(), Vec::new()),
        |lr| {
            let sizes: Vec<Option<u64>> = lr
                .runs()
                .iter()
                .map(|run| lr.archive_bytes(&run.run_id))
                .collect();
            (lr.runs().to_vec(), sizes)
        },
    );
    let owner_paths = selected_path.map(AbsolutePath::from).into_iter().collect();
    let owner_of = vec![0; runs.len()];
    LintsData {
        runs,
        sizes,
        owner_paths,
        owner_of,
        project_kind: LintsProjectKind::from_is_rust(is_rust),
    }
}

/// Merge the lint histories of `paths` (a worktree group's visible
/// checkouts) into one list, newest-first, tagging each run with the
/// index of the checkout it came from so its logs stay resolvable.
fn aggregate_group_lints(app: &App, paths: Vec<AbsolutePath>, is_rust: bool) -> LintsData {
    // For each checkout, for each of its runs, emit (run, archive size,
    // checkout index) — `flat_map` flattens the two levels into one stream.
    let mut merged: Vec<(LintRun, Option<u64>, usize)> = paths
        .iter()
        .enumerate()
        .flat_map(|(owner, path)| {
            app.lint_at_path(path.as_path())
                .into_iter()
                .flat_map(move |lr| {
                    lr.runs()
                        .iter()
                        .map(move |run| (run.clone(), lr.archive_bytes(&run.run_id), owner))
                })
        })
        .collect();
    // Newest-first by actual instant (RFC3339 offsets can differ across a
    // DST boundary, so compare parsed timestamps, not the raw strings).
    merged.sort_by(|a, b| {
        lint::parse_timestamp(&b.0.started_at).cmp(&lint::parse_timestamp(&a.0.started_at))
    });

    // Fan the merged stream into the three index-aligned vectors in one pass.
    let (runs, sizes, owner_of): (Vec<LintRun>, Vec<Option<u64>>, Vec<usize>) =
        merged.into_iter().multiunzip();
    LintsData {
        runs,
        sizes,
        owner_paths: paths,
        owner_of,
        project_kind: LintsProjectKind::from_is_rust(is_rust),
    }
}
