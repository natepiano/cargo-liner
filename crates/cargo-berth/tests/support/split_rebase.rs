//! Real interactive split rebase steps shared by lifecycle and gate regressions.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use super::BERTH_EXECUTABLE;
use super::GIT;
use super::git_command;

/// Stop at the first commit and uncommit it while preserving update-ref todo entries.
pub(super) fn stop_for_split(holder: &Path, editor: &Path, trunk: &str) {
    write_first_rebase_action(editor, "edit");
    stop_with_editor(holder, editor, &["--update-refs", trunk]);
}

/// Stop a forced interactive rebase using the caller's editor and revision arguments.
pub(super) fn stop_with_editor(holder: &Path, editor: &Path, revisions: &[&str]) {
    let stopped = git_command(BERTH_EXECUTABLE)
        .args(["rebase", "--force-rebase", "-i"])
        .args(revisions)
        .env("GIT_SEQUENCE_EDITOR", editor)
        .current_dir(holder)
        .output()
        .expect("interactive rebase should stop for splitting");
    assert!(
        stopped.status.success(),
        "{}",
        String::from_utf8_lossy(&stopped.stderr)
    );
    let administrative_directory = GIT.stdout(holder, ["rev-parse", "--absolute-git-dir"]);
    assert!(
        Path::new(&administrative_directory)
            .join("rebase-merge/stopped-sha")
            .is_file()
    );
    GIT.run(holder, ["reset", "HEAD^"]);
}

/// Finish the split through Git's real branch reference transactions.
pub(super) fn continue_rebase(holder: &Path) {
    let continued = git_command(BERTH_EXECUTABLE)
        .args(["rebase", "--continue"])
        .env("GIT_EDITOR", "true")
        .current_dir(holder)
        .output()
        .expect("split rebase should continue");
    assert!(
        continued.status.success(),
        "{}",
        String::from_utf8_lossy(&continued.stderr)
    );
}

/// Edit the first picked commit while preserving update-ref commands in the todo.
pub(super) fn write_first_rebase_action(editor: &Path, action: &str) {
    fs::write(editor, format!(
        "#!/bin/sh\nsed '1s/^pick /{action} /' \"$1\" > \"$1.edited\"\nmv \"$1.edited\" \"$1\"\n"
    )).expect("sequence editor should write");
    fs::set_permissions(editor, fs::Permissions::from_mode(0o755))
        .expect("sequence editor should execute");
}
