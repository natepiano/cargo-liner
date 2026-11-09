use std::fs;
use std::path::Path;

use anyhow::Context;
use cargo_metadata::PackageId;
use cargo_metadata::TargetKind;
use cargo_metadata::semver::Version;
use ratatui::layout::Rect;
use tui_pane::ColorMode;

use super::data;
use crate::project::AbsolutePath;
use crate::project::ManifestFingerprint;
use crate::project::Package;
use crate::project::PackageRecord;
use crate::project::PublishPolicy;
use crate::project::RootItem;
use crate::project::RustProject;
use crate::project::TargetRecord;
use crate::project::WorkspaceMetadata;
use crate::project::WorktreeGroup;
use crate::tui::panes::PaneId;
use crate::tui::test_support;

fn checkout_metadata(root: &Path) -> anyhow::Result<WorkspaceMetadata> {
    fs::create_dir_all(root.join("src"))?;
    fs::write(root.join("src/main.rs"), "fn main() {}")?;
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"proof_crate\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )?;
    let record = PackageRecord {
        name:          "proof_crate".to_string(),
        version:       Version::new(0, 1, 0),
        edition:       "2024".to_string(),
        description:   None,
        license:       None,
        homepage:      None,
        repository:    None,
        manifest_path: AbsolutePath::from(root.join("Cargo.toml")),
        targets:       vec![TargetRecord {
            name:              "proof_crate".to_string(),
            kinds:             vec![TargetKind::Bin],
            src_path:          AbsolutePath::from(root.join("src/main.rs")),
            required_features: Vec::new(),
        }],
        publish:       PublishPolicy::Never,
    };
    Ok(WorkspaceMetadata {
        declared_checkout_root:   AbsolutePath::from(root),
        target_directory:         AbsolutePath::from(root.join("target")),
        packages:                 std::iter::once((
            PackageId {
                repr: root.display().to_string(),
            },
            record,
        ))
        .collect(),
        fingerprint:              ManifestFingerprint::capture(root)?,
        out_of_tree_target_bytes: None,
    })
}

#[test]
fn same_name_worktree_sources_stay_distinct_after_the_width_cut() -> anyhow::Result<()> {
    for parents in [
        ["work-a", "work_b", "work_c"],
        ["work-a", "work_a", "work.a"],
    ] {
        let scratch = tempfile::tempdir()?;
        let roots: Vec<_> = parents
            .iter()
            .map(|parent| scratch.path().join(parent).join("proof_crate"))
            .collect();
        let metadata = roots
            .iter()
            .map(|root| checkout_metadata(root))
            .collect::<anyhow::Result<Vec<_>>>()?;
        let projects: Vec<_> = roots
            .iter()
            .map(|root| {
                RustProject::Package(Package {
                    path: AbsolutePath::from(root.as_path()),
                    name: Some("proof_crate".to_string()),
                    ..Package::default()
                })
            })
            .collect();
        let item = RootItem::Worktrees(WorktreeGroup::new(
            projects[0].clone(),
            projects[1..].to_vec(),
        ));
        let mut app = test_support::make_app(std::slice::from_ref(&item));
        app.framework.color_mode = ColorMode::Colored;
        for value in metadata {
            app.scan
                .metadata_store_handle()
                .lock()
                .map_err(|error| anyhow::anyhow!("{error}"))?
                .upsert(value);
        }
        app.ensure_detail_cached();
        let data =
            data::lookup_targets_data(&app, &AbsolutePath::from(roots[0].as_path()), Some(&item));
        assert_eq!(data.binaries.len(), roots.len());
        for root in &roots {
            let entry = data
                .binaries
                .iter()
                .find(|entry| entry.project_path.as_path() == root)
                .context("Target launch project")?;
            assert_eq!(entry.package_path.as_path(), root);
            assert_eq!(entry.src_path.as_path(), root.join("src/main.rs"));
        }
        for (width, pane_width) in [(80, 34), (95, 29)] {
            app.panes.targets.set_content(data.clone());
            let buffer =
                test_support::draw_pane_in_window(&mut app, PaneId::Targets, width, pane_width)?;
            let area = Rect::new(0, 0, pane_width, buffer.area.height);
            let source =
                test_support::find_text_cell(&buffer, area, "Source").context("Source header")?;
            let mut sources = Vec::new();
            for y in source.1 + 1..area.bottom() - 1 {
                let row: String = (area.x + 1..area.right() - 1)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect();
                if row.contains("binary") {
                    let kind = test_support::find_text_cell(
                        &buffer,
                        Rect::new(area.x, y, area.width, 1),
                        "binary",
                    )
                    .context("Binary kind")?;
                    let value: String = (source.0..kind.0)
                        .map(|x| buffer[(x, y)].symbol())
                        .collect();
                    assert!(!value.trim().is_empty(), "{width}: {row}");
                    sources.push(value.trim().to_string());
                }
            }
            assert_eq!(
                sources.len(),
                roots.len(),
                "{width}: {:?}",
                test_support::rows(&buffer)
            );
            sources.sort();
            sources.dedup();
            assert_eq!(sources.len(), roots.len(), "{width}: {sources:?}");
        }
    }
    Ok(())
}

fn assert_long_checkout_sources(parents: &[&str]) -> anyhow::Result<()> {
    let scratch = tempfile::tempdir()?;
    let roots: Vec<_> = parents
        .iter()
        .map(|parent| scratch.path().join(parent).join("proof_crate"))
        .collect();
    let metadata = roots
        .iter()
        .map(|root| checkout_metadata(root))
        .collect::<anyhow::Result<Vec<_>>>()?;
    let projects: Vec<_> = roots
        .iter()
        .map(|root| {
            RustProject::Package(Package {
                path: AbsolutePath::from(root.as_path()),
                name: Some("proof_crate".to_string()),
                ..Package::default()
            })
        })
        .collect();
    let item = RootItem::Worktrees(WorktreeGroup::new(
        projects[0].clone(),
        projects[1..].to_vec(),
    ));
    let mut app = test_support::make_app(std::slice::from_ref(&item));
    app.framework.color_mode = ColorMode::Colored;
    for value in metadata {
        app.scan
            .metadata_store_handle()
            .lock()
            .map_err(|error| anyhow::anyhow!("{error}"))?
            .upsert(value);
    }
    app.ensure_detail_cached();
    let data =
        data::lookup_targets_data(&app, &AbsolutePath::from(roots[0].as_path()), Some(&item));
    assert_eq!(data.binaries.len(), roots.len());
    for root in &roots {
        let entry = data
            .binaries
            .iter()
            .find(|entry| entry.project_path.as_path() == root)
            .context("Target launch project")?;
        assert_eq!(entry.package_path.as_path(), root);
        assert_eq!(entry.src_path.as_path(), root.join("src/main.rs"));
    }
    let mut failures = Vec::new();
    for (width, pane_width) in [(80, 34), (95, 29)] {
        app.panes.targets.set_content(data.clone());
        let buffer =
            test_support::draw_pane_in_window(&mut app, PaneId::Targets, width, pane_width)?;
        let area = buffer.area;
        let source =
            test_support::find_text_cell(&buffer, area, "Source").context("Source header")?;
        let mut sources = Vec::new();
        for y in source.1 + 1..area.bottom() - 1 {
            let row_area = Rect::new(0, y, pane_width, 1);
            if let Some(kind) = test_support::find_text_cell(&buffer, row_area, "binary") {
                let target: String = (1..source.0).map(|x| buffer[(x, y)].symbol()).collect();
                let label: String = (source.0..kind.0)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect();
                if target.trim() != "proof_crate" {
                    failures.push(format!("{width}: Target reads {target:?}"));
                }
                sources.push(label.trim().to_string());
            }
        }
        assert_eq!(
            sources.len(),
            roots.len(),
            "{width}: {:?}",
            test_support::rows(&buffer)
        );
        let drawn = sources.clone();
        sources.sort();
        sources.dedup();
        if sources.len() != roots.len() || sources.iter().any(String::is_empty) {
            failures.push(format!("{width}: Sources read {drawn:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    Ok(())
}

#[test]
fn long_checkout_sources_and_target_names_remain_distinct_and_whole() -> anyhow::Result<()> {
    assert_long_checkout_sources(&[
        "claude-build-followups-sdlc-merge-gate",
        "claude-build-followups-verify-run-place",
        "long-names",
    ])
}

#[test]
fn checkout_sources_keep_differing_ends_after_the_width_cut() -> anyhow::Result<()> {
    assert_long_checkout_sources(&["verylongcheckoutA", "verylongcheckoutB"])
}

#[test]
fn checkout_sources_keep_differing_delimiters_after_the_width_cut() -> anyhow::Result<()> {
    assert_long_checkout_sources(&["long-shared-work-a", "long-shared-work_a"])
}
