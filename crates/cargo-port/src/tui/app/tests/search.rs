use super::*;

#[test]
fn search_finds_workspace_members_and_vendored_crates() {
    let member_path = "~/ws/crates/member";
    let vendored_path = "~/ws/vendor/helper";
    let workspace = RootItem::Rust(RustProject::Workspace(make_workspace_raw_with_vendored(
        Some("ws"),
        "~/ws",
        vec![inline_group(vec![make_member(Some("member"), member_path)])],
        vec![make_member(Some("helper"), vendored_path)],
        None,
    )));
    let mut app = make_app(&[make_workspace_project(Some("ws"), "~/ws")]);
    apply_items(&mut app, &[workspace]);

    app.update_search("member");
    assert!(
        app.filtered
            .iter()
            .any(|hit| hit.abs_path == test_path(member_path) && hit.name == "member")
    );

    app.update_search("helper");
    assert!(
        app.filtered
            .iter()
            .any(|hit| hit.abs_path == test_path(vendored_path) && hit.name == "helper (vendored)")
    );
}

#[test]
fn search_finds_linked_worktree_children_and_confirms_selection() {
    let member_path = "~/ws_feat/crates/member_feat";
    let vendored_path = "~/ws_feat/vendor/helper_feat";
    let item = make_workspace_worktrees_item(
        make_workspace_raw(Some("ws"), "~/ws", Vec::new(), None),
        vec![make_workspace_raw_with_vendored(
            Some("ws"),
            "~/ws_feat",
            vec![inline_group(vec![make_package_with_vendored(
                Some("member_feat"),
                member_path,
                vec![make_member(Some("helper_feat"), vendored_path)],
            )])],
            Vec::new(),
            Some("ws_feat"),
        )],
    );
    let mut app = make_app(&[make_workspace_project(Some("ws"), "~/ws")]);
    apply_items(&mut app, &[item]);

    app.update_search("helper_feat");
    assert!(app.filtered.iter().any(
        |hit| hit.abs_path == test_path(vendored_path) && hit.name == "helper_feat (vendored)"
    ));

    app.update_search("member_feat");
    app.confirm_search();

    assert_eq!(
        app.selected_project_path().map(Path::to_path_buf),
        Some(test_path(member_path).to_path_buf())
    );
}
