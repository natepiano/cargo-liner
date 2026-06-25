use std::path::Path;

use super::cache_size_index;
use super::history;
use super::paths;

/// Reclaim a project's lint cache directory. Best-effort: silent
/// on missing or locked paths. Called from the dismiss flow when
/// the project at `project_root` is gone from disk so a future
/// worktree/branch reusing this exact path starts clean.
pub(super) fn reclaim_project_cache(project_root: &Path) {
    reclaim_project_cache_under(paths::cache_root().as_path(), project_root);
}

/// Cache-root-explicit variant of [`reclaim_project_cache`].
/// Matches the `_under` pattern used by the rest of the module so
/// tests can target a tempdir.
pub(super) fn reclaim_project_cache_under(cache_root: &Path, project_root: &Path) {
    let project_dir = paths::project_dir_under(cache_root, project_root);
    let bytes = history::project_dir_bytes(&project_dir);
    if std::fs::remove_dir_all(&project_dir).is_ok() && bytes > 0 {
        cache_size_index::adjust(cache_root, -i64::try_from(bytes).unwrap_or(i64::MAX));
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::reclaim_project_cache_under;
    use crate::lint::history;
    use crate::lint::paths;
    use crate::lint::run::LintRun;
    use crate::lint::run::LintRunStatus;

    fn run(status: LintRunStatus) -> LintRun {
        LintRun {
            run_id: "run-1".to_string(),
            started_at: "2026-03-30T14:22:01-05:00".to_string(),
            finished_at: Some("2026-03-30T14:22:18-05:00".to_string()),
            duration_ms: Some(17_000),
            status,
            commands: Vec::new(),
            archive_bytes: 0,
        }
    }

    #[test]
    fn project_cache_removes_existing_directory() {
        let cache_dir = tempfile::tempdir().expect("tempdir");
        let project_dir = tempfile::tempdir().expect("tempdir");

        history::append_history_under(
            cache_dir.path(),
            project_dir.path(),
            &run(LintRunStatus::Passed),
            None,
        )
        .expect("append history");
        let project_cache = paths::project_dir_under(cache_dir.path(), project_dir.path());
        assert!(
            project_cache.as_path().is_dir(),
            "project cache directory must exist before reclamation",
        );

        reclaim_project_cache_under(cache_dir.path(), project_dir.path());

        assert!(
            !project_cache.as_path().exists(),
            "project cache directory must be removed after reclamation",
        );
        assert!(
            cache_dir.path().is_dir(),
            "cache root must survive — only the per-project subdir is reclaimed",
        );
    }

    #[test]
    fn project_cache_is_noop_when_directory_missing() {
        let cache_dir = tempfile::tempdir().expect("tempdir");
        let project_dir = tempfile::tempdir().expect("tempdir");

        // No history written — the per-project directory was never
        // created. Reclamation must not panic and must not disturb
        // the cache root.
        reclaim_project_cache_under(cache_dir.path(), project_dir.path());

        assert!(cache_dir.path().is_dir());
        let project_cache = paths::project_dir_under(cache_dir.path(), project_dir.path());
        assert!(!project_cache.as_path().exists());
    }
}
