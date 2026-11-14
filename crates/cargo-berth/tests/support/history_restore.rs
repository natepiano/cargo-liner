//! Restore a recorded history into an existing test fixture.
use std::path::Path;

use cargo_berth_test_support::DirectorySnapshot;
use cargo_berth_test_support::LedgerTemplate;
use cargo_berth_test_support::relocate_tree;

/// Restore a recorded history into an existing fixture with renewed owners.
pub(crate) fn restore(template: LedgerTemplate, root: &Path) {
    if !root.as_os_str().as_encoded_bytes().contains(&b'\\') {
        template.restore_into(root);
        return;
    }
    let source = template.instantiate();
    DirectorySnapshot::capture(source.path()).restore(root);
    relocate_tree(
        root,
        &[(
            source.path().to_string_lossy().into_owned(),
            root.to_string_lossy().into_owned(),
        )],
    );
}
