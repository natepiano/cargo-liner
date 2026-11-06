use crate::tui::panes::PaneId;
use crate::tui::test_support;

#[test]
fn narrow_targets_keep_target_and_kind_before_source() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.panes.targets.set_content(test_support::target_data());
    let buffer = test_support::draw_pane(&mut app, PaneId::Targets, 23)?;
    test_support::position(&buffer, "Target")?;
    test_support::position(&buffer, "Kind")?;
    test_support::position(&buffer, "first")?;
    assert!(!test_support::rows(&buffer).join("\n").contains("Source"));
    Ok(())
}
