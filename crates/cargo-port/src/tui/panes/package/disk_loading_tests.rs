use std::path::Path;

use anyhow::Context;
use ratatui::layout::Rect;

use crate::project::AbsolutePath;
use crate::scan::BackgroundMsg;
use crate::scan::DirSizes;
use crate::tui::test_support;

#[test]
fn disk_rows_and_project_sum_change_from_loading_to_measured() -> anyhow::Result<()> {
    let scratch = tempfile::tempdir()?;
    let root = scratch.path().join("alpha");
    std::fs::create_dir_all(&root)?;
    let path = AbsolutePath::from(Path::new(&root));
    for width in [80, 95] {
        let message = BackgroundMsg::DiskUsageBatch {
            root_path: path.clone(),
            entries:   vec![(
                path.clone(),
                DirSizes {
                    total:                 4194,
                    in_project_target:     4096,
                    in_project_non_target: 98,
                    max_source_mtime:      None,
                },
            )],
        };
        let (loading, measured) =
            test_support::draw_disk_loading_transition_for_test(&path, message, width)?;
        let rows = test_support::rows(&loading);
        for label in ["Disk", "target/", "Other"] {
            let row = rows
                .iter()
                .find(|row| row.contains(label) && !row.contains("Projects"))
                .context(label)?;
            assert!(row.contains('…'), "{width}: {row}");
            assert!(!row.contains("0 B"), "{width}: {row}");
        }
        let target = test_support::position(&measured, "target/")?;
        let left = (0..target.0)
            .rev()
            .find(|x| measured[(*x, target.1)].symbol() == "│")
            .context("Package left border")?
            + 1;
        let right = (target.0..measured.area.right())
            .find(|x| measured[(*x, target.1)].symbol() == "│")
            .context("Package right border")?;
        let column = Rect::new(left, 0, right - left, measured.area.height);
        let disk = test_support::find_text_cell(&measured, column, "Disk")
            .context("Measured Disk label")?;
        let disk_value = test_support::find_text_cell(&measured, column, "4.1 KiB")
            .context("Measured Disk value")?;
        assert_eq!(disk_value.1, disk.1);
        for (label, value) in [
            ("Disk", "4.1 KiB"),
            ("target/", "4.0 KiB"),
            ("Other", "98 B"),
        ] {
            let label_position =
                test_support::find_text_cell(&measured, column, label).context(label)?;
            let value_position =
                test_support::find_text_cell(&measured, column, value).context(value)?;
            let label_end = label_position.0 + u16::try_from(label.len())?;
            let value_end = disk_value.0 + u16::try_from(value.len())?;
            let fits = label_end < disk_value.0 && value_end <= right;
            assert_eq!(
                value_position.1,
                label_position.1 + u16::from(!fits),
                "{width}: {label}: {:?}",
                test_support::rows(&measured)
            );
        }
        for (buffer, value) in [(&loading, "…"), (&measured, "4.1 KiB")] {
            let rows = test_support::rows(buffer);
            let row = rows
                .iter()
                .find(|row| row.contains('Σ'))
                .context("Sum row")?;
            let sum = row.split_once('Σ').context("Sum mark")?.1;
            let cell = sum.split('│').next().context("Sum value")?;
            assert_eq!(cell.trim_matches(' '), value, "{width}: {row}");
        }
    }
    Ok(())
}
