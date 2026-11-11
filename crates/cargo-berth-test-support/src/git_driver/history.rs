//! Git histories shared by recorded builders and live board operations.

use std::fs;
use std::path::Path;

use tempfile::tempdir;

use super::GitDriver;

impl GitDriver {
    /// Commit the library source on the selected base and return its tip.
    ///
    /// # Panics
    /// Panics when a Git command or a fixture file fails.
    #[must_use]
    pub fn commit_library_target_from_base(
        self,
        repository_root: &Path,
        phase_start_head: &str,
        source: &str,
        message: &str,
    ) -> String {
        self.run(
            repository_root,
            [
                "-c",
                "core.hooksPath=/dev/null",
                "reset",
                "--hard",
                phase_start_head,
            ],
        );
        fs::write(repository_root.join("src/lib.rs"), source).expect("target source should write");
        self.run(repository_root, ["add", "src/lib.rs"]);
        self.run(
            repository_root,
            [
                "-c",
                "core.hooksPath=/dev/null",
                "commit",
                "--quiet",
                "-m",
                message,
            ],
        );
        self.stdout(repository_root, ["rev-parse", "HEAD"])
    }

    /// Commit a competing scope with a private index and return its tip.
    ///
    /// # Panics
    /// Panics when a Git command or a fixture file fails.
    #[must_use]
    pub fn commit_competitor_scope(
        self,
        repository_root: &Path,
        scope_path: &str,
        phase_start_head: &str,
        reservation_index: usize,
    ) -> String {
        let scratch = tempdir().expect("competitor scratch directory should exist");
        let index_path = scratch.path().join("index");
        let source_path = scratch.path().join("competitor.rs");
        fs::write(&source_path, "pub fn competitor() {}\n")
            .expect("competitor source should write");

        self.git_with_index(
            repository_root,
            &index_path,
            &["read-tree", phase_start_head],
        );
        let blob = self.git_with_index(
            repository_root,
            &index_path,
            &[
                "hash-object",
                "-w",
                "--path",
                scope_path,
                source_path
                    .to_str()
                    .expect("competitor source path should be UTF-8"),
            ],
        );
        self.git_with_index(
            repository_root,
            &index_path,
            &[
                "update-index",
                "--add",
                "--cacheinfo",
                &format!("100644,{blob},{scope_path}"),
            ],
        );
        let tree = self.git_with_index(repository_root, &index_path, &["write-tree"]);
        let protected_tip = self.git_with_index(
            repository_root,
            &index_path,
            &[
                "commit-tree",
                &tree,
                "-p",
                phase_start_head,
                "-m",
                "competitor work inside its own scope",
            ],
        );
        // The reference keeps the competitor commit reachable for the reservation.
        self.run(
            repository_root,
            [
                "update-ref",
                &format!("refs/heads/competitor-{reservation_index}"),
                &protected_tip,
            ],
        );
        protected_tip
    }

    /// Commit outside the protected scopes and return the new trunk tip.
    ///
    /// # Panics
    /// Panics when a Git command or a fixture file fails.
    #[must_use]
    pub fn advance_trunk(self, repository_root: &Path, marker: &str) -> String {
        let advancing_source = format!("src/advanced-{marker}.rs");
        let message = format!("{marker} advance past the proof");
        fs::write(
            repository_root.join(&advancing_source),
            "pub fn advanced() {}\n",
        )
        .expect("advancing source should write");
        self.run(repository_root, ["add", &advancing_source]);
        self.run(
            repository_root,
            [
                "-c",
                "core.hooksPath=/dev/null",
                "commit",
                "--quiet",
                "-m",
                &message,
            ],
        );
        self.stdout(repository_root, ["rev-parse", "HEAD"])
    }

    fn git_with_index(
        self,
        repository_root: &Path,
        index_path: &Path,
        arguments: &[&str],
    ) -> String {
        let output = self.output_with_environment(
            repository_root,
            arguments,
            "GIT_INDEX_FILE",
            index_path.to_str().expect("index path should be UTF-8"),
        );
        assert!(
            output.status.success(),
            "git {arguments:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout)
            .expect("git output should be UTF-8")
            .trim()
            .to_owned()
    }
}
