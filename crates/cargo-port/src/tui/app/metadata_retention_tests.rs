use std::collections::BTreeMap;
use std::collections::HashMap;

use super::App;
use crate::project::AbsolutePath;
use crate::project::FileStamp;
use crate::project::ManifestFingerprint;
use crate::project::RootItem;
use crate::project::RustProject;
use crate::project::Workspace;
use crate::project::WorkspaceMetadata;
use crate::scan::BackgroundMsg;
use crate::tui::test_support;

#[test]
fn repeated_rescans_release_the_previous_startup_toast() {
    let mut app = test_support::make_app(&[]);
    for _ in 0..128 {
        app.initialize_startup_phase_tracker();
        app.prepare_rescan();
        app.framework
            .toasts
            .prune(std::time::Instant::now() + std::time::Duration::from_secs(1));
    }
    assert_eq!(
        app.framework
            .toasts
            .active_now()
            .iter()
            .filter(|toast| toast.title() == "Startup")
            .count(),
        0,
        "a reset must release the startup surface it owned"
    );
}

fn metadata(root: &AbsolutePath) -> WorkspaceMetadata {
    WorkspaceMetadata {
        declared_checkout_root:   root.clone(),
        target_directory:         root.join("target").into(),
        packages:                 HashMap::new(),
        fingerprint:              ManifestFingerprint {
            manifest:       FileStamp {
                content_hash: [0; 32],
            },
            lockfile:       None,
            rust_toolchain: None,
            configs:        BTreeMap::new(),
        },
        out_of_tree_target_bytes: None,
    }
}

fn replace_checkout(app: &mut App, root: &AbsolutePath) {
    app.handle_bg_msg(BackgroundMsg::ScanResult {
        projects:     vec![test_support::make_package("retention", root.as_path())],
        disk_entries: Vec::new(),
    });
}

#[test]
fn repeated_checkout_replacements_release_metadata() -> anyhow::Result<()> {
    let mut app = test_support::make_app(&[]);
    let store = app.scan.metadata_store_handle();
    let roots: Vec<_> = (0..16)
        .map(|index| AbsolutePath::from(format!("/tmp/test/metadata-retention/{index}")))
        .collect();
    for root in &roots {
        replace_checkout(&mut app, root);
        let mut guard = store.lock().map_err(|error| anyhow::anyhow!("{error}"))?;
        guard.next_generation(root);
        guard.upsert(metadata(root));
    }
    let guard = store.lock().map_err(|error| anyhow::anyhow!("{error}"))?;
    assert_eq!(
        roots
            .iter()
            .filter(|root| guard.get(root).is_some())
            .count(),
        1,
        "only the current checkout owns cached metadata"
    );
    assert_eq!(
        guard.dispatch_generations.len(),
        1,
        "retired checkout generations must not accumulate"
    );
    drop(guard);
    Ok(())
}

#[test]
fn removed_checkout_reply_cannot_restore_metadata() -> anyhow::Result<()> {
    let mut app = test_support::make_app(&[]);
    let retired = AbsolutePath::from("/tmp/test/metadata-retention/retired");
    let current = AbsolutePath::from("/tmp/test/metadata-retention/current");
    replace_checkout(&mut app, &retired);
    replace_checkout(&mut app, &current);
    let store = app.scan.metadata_store_handle();
    // An older scan can still dispatch after its checkout leaves the new tree.
    let generation = store
        .lock()
        .map_err(|error| anyhow::anyhow!("{error}"))?
        .next_generation(&retired);
    let snapshot = metadata(&retired);
    let fingerprint = snapshot.fingerprint.clone();
    app.handle_bg_msg(BackgroundMsg::CargoMetadata {
        workspace_root: retired.clone(),
        generation,
        fingerprint,
        result: Ok(snapshot),
    });
    let guard = store.lock().map_err(|error| anyhow::anyhow!("{error}"))?;
    assert!(
        guard.get(&retired).is_none(),
        "late replies cannot recreate an owner"
    );
    assert!(!guard.dispatch_generations.contains_key(&retired));
    drop(guard);
    Ok(())
}

#[test]
fn recreated_checkout_rejects_its_previous_lifetime_stamp() -> anyhow::Result<()> {
    let mut app = test_support::make_app(&[]);
    let root = AbsolutePath::from("/tmp/test/metadata-retention/recreated");
    let other = AbsolutePath::from("/tmp/test/metadata-retention/other");
    replace_checkout(&mut app, &root);
    let store = app.scan.metadata_store_handle();
    let old_generation = store
        .lock()
        .map_err(|error| anyhow::anyhow!("{error}"))?
        .next_generation(&root);
    replace_checkout(&mut app, &other);
    replace_checkout(&mut app, &root);
    let mut guard = store.lock().map_err(|error| anyhow::anyhow!("{error}"))?;
    let generation = guard.next_generation(&root);
    assert!(generation > old_generation);
    assert!(!guard.is_current_generation(&root, old_generation));
    assert!(guard.is_current_generation(&root, generation));
    assert!(
        guard.get(&root).is_none(),
        "recreated checkout has no metadata yet"
    );
    drop(guard);
    Ok(())
}

#[test]
fn rescans_keep_workspace_member_measurements_until_the_owner_leaves() {
    let root = AbsolutePath::from("/tmp/test/source-retention/workspace");
    let member = AbsolutePath::from("/tmp/test/source-retention/workspace/member");
    let workspace = RootItem::Rust(RustProject::Workspace(Workspace {
        path: root,
        groups: vec![crate::project::MemberGroup::Inline {
            members: vec![crate::project::Package {
                path: member.clone(),
                ..Default::default()
            }],
        }],
        ..Default::default()
    }));
    let mut app = test_support::make_app(std::slice::from_ref(&workspace));
    app.scan
        .source_measurements
        .observe_test_owner_for_test(&member);
    app.handle_bg_msg(BackgroundMsg::ScanResult {
        projects:     vec![workspace],
        disk_entries: Vec::new(),
    });
    assert!(
        app.scan
            .source_measurements
            .has_test_owner_for_test(&member),
        "workspace members still own their source counts after rescan"
    );
    app.handle_bg_msg(BackgroundMsg::ScanResult {
        projects:     Vec::new(),
        disk_entries: Vec::new(),
    });
    assert!(
        !app.scan
            .source_measurements
            .has_test_owner_for_test(&member)
    );
}

#[test]
fn rescans_keep_discovered_submodule_measurements_before_repo_info_arrives() {
    let root = AbsolutePath::from("/tmp/test/source-retention/parent");
    let submodule = AbsolutePath::from("/tmp/test/source-retention/parent/submodule");
    let project = test_support::make_package("parent", root.as_path());
    let mut app = test_support::make_app(std::slice::from_ref(&project));
    app.scan
        .source_measurements
        .observe_test_owner_for_test(&submodule);
    app.handle_bg_msg(BackgroundMsg::ScanResult {
        projects:     vec![project.clone()],
        disk_entries: vec![
            ("parent".to_string(), root.clone()),
            ("submodule".to_string(), submodule.clone()),
        ],
    });
    assert!(
        app.scan
            .source_measurements
            .has_test_owner_for_test(&submodule)
    );
    app.handle_bg_msg(BackgroundMsg::ScanResult {
        projects:     vec![project],
        disk_entries: vec![("parent".to_string(), root)],
    });
    assert!(
        !app.scan
            .source_measurements
            .has_test_owner_for_test(&submodule)
    );
}
