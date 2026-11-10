use anyhow::Context;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use tui_pane::ColorMode;
use tui_pane::PaneFocusState;
use unicode_width::UnicodeWidthStr;

use crate::project::AheadBehind;
use crate::project::Comparison;
use crate::project::TrackedRef;
use crate::tui::panes;
use crate::tui::panes::DescriptionState;
use crate::tui::panes::GitData;
use crate::tui::panes::PaneId;
use crate::tui::panes::pane_data::RemoteRow;
use crate::tui::panes::pane_data::RemoteRows;
use crate::tui::panes::pane_data::WorktreeInfo;
use crate::tui::test_support;

pub(super) fn assert_worktree_columns(
    buffer: &Buffer,
    area: Rect,
    header_y: u16,
    value_y: u16,
    expected: [&str; 4],
) -> anyhow::Result<()> {
    let header_row: String = (area.x + 1..area.right() - 1)
        .map(|x| buffer[(x, header_y)].symbol())
        .collect();
    let value_row: String = (area.x + 1..area.right() - 1)
        .map(|x| buffer[(x, value_y)].symbol())
        .collect();
    let headers: Vec<_> = header_row.split_whitespace().collect();
    let values: Vec<_> = value_row.split_whitespace().collect();
    let indices: Vec<usize> = headers
        .iter()
        .map(|header| {
            ["Name", "Branch", "Tracked", "Sync"]
                .iter()
                .position(|whole| whole == header)
                .context("whole header")
        })
        .collect::<anyhow::Result<_>>()?;
    assert_eq!(values.len(), headers.len(), "{value_row}");
    assert!(
        matches!(indices.as_slice(), [0, 1, 2, 3] | [0, 1, 3] | [1, 3] | [1]),
        "{header_row}"
    );
    for (token, index) in values.iter().zip(indices) {
        let original = expected[index];
        if *token != original {
            let prefix = token.strip_suffix('…').context("Cut column ellipsis")?;
            assert!(
                !prefix.is_empty() && original.starts_with(prefix),
                "{token}: {original}"
            );
        }
    }
    Ok(())
}

fn row_tokens(buffer: &Buffer, area: Rect, y: u16) -> anyhow::Result<Vec<(String, u16)>> {
    let mut tokens: Vec<(String, u16)> = Vec::new();
    let mut in_token = false;
    for x in area.x + 1..area.right() - 1 {
        let symbol = buffer[(x, y)].symbol();
        if symbol.trim().is_empty() {
            in_token = false;
        } else {
            if !in_token {
                tokens.push((String::new(), x));
                in_token = true;
            }
            tokens
                .last_mut()
                .context("Current token")?
                .0
                .push_str(symbol);
        }
    }
    Ok(tokens)
}

pub(super) fn assert_shared_remote_columns(
    buffer: &Buffer,
    area: Rect,
    remote_header_y: u16,
    worktree_header_y: u16,
) -> anyhow::Result<()> {
    let remote_headers = row_tokens(buffer, area, remote_header_y)?;
    let worktree_headers = row_tokens(buffer, area, worktree_header_y)?;
    let remote_values = row_tokens(buffer, area, remote_header_y + 1)?;
    let worktree_values = row_tokens(buffer, area, worktree_header_y + 1)?;
    if worktree_headers.len() < 4 {
        let expected: &[&str] = match worktree_headers.len() {
            1 => &["Branch"],
            2 => &["Branch", "Sync"],
            3 => &["Name", "Branch", "Sync"],
            _ => anyhow::bail!(
                "unexpected worktree column count: {}",
                worktree_headers.len()
            ),
        };
        assert_eq!(
            worktree_headers
                .iter()
                .map(|(text, _)| text.as_str())
                .collect::<Vec<_>>(),
            expected
        );
        let remote_expected: &[&str] = match remote_headers.len() {
            1 => &["Remote"],
            2 => &["Remote", "Sync"],
            3 => &["Remote", "Branch", "Sync"],
            _ => anyhow::bail!("unexpected remote column count: {}", remote_headers.len()),
        };
        assert_eq!(
            remote_headers
                .iter()
                .map(|(text, _)| text.as_str())
                .collect::<Vec<_>>(),
            remote_expected
        );
        assert_eq!(remote_values.len(), remote_headers.len());
        assert_eq!(worktree_values.len(), worktree_headers.len());
        for label in ["Branch", "Sync"] {
            if let (Some(remote), Some(worktree)) = (
                remote_headers.iter().position(|(text, _)| text == label),
                worktree_headers.iter().position(|(text, _)| text == label),
            ) {
                assert_eq!(remote_headers[remote].1, worktree_headers[worktree].1);
                assert_eq!(remote_values[remote].1, worktree_values[worktree].1);
            }
        }
        return Ok(());
    }
    for tokens in [&worktree_headers, &worktree_values] {
        assert_eq!(tokens.len(), 4, "{tokens:?}");
    }
    let compact = remote_headers.len() == 4;
    let (name_index, name_end, gap) = if compact {
        assert_eq!(remote_headers.len(), 4, "{remote_headers:?}");
        assert_eq!(remote_values.len(), 4, "{remote_values:?}");
        (0, remote_headers[1].1, 1)
    } else {
        assert_eq!(remote_headers.len(), 5, "{remote_headers:?}");
        assert!(matches!(remote_values.len(), 5 | 6), "{remote_values:?}");
        let name_index = remote_values.len() - 5;
        let url = &remote_values[name_index + 1];
        assert!(url.0.width() >= 2, "{remote_values:?}");
        for (header, whole) in worktree_headers[1..]
            .iter()
            .zip(["Branch", "Tracked", "Sync"])
        {
            assert_eq!(header.0, whole, "{worktree_headers:?}");
        }
        (name_index, remote_headers[1].1, 2)
    };
    if matches!(area.width - 2, 15 | 22) {
        assert!(compact, "{remote_headers:?}");
    }
    let name = &remote_values[name_index].0;
    if name != "origin" {
        let prefix = name.strip_suffix('…').context("Remote name ellipsis")?;
        assert!(!prefix.is_empty() && "origin".starts_with(prefix), "{name}");
    }
    if name_end - remote_values[name_index].1 - gap >= 6 {
        assert_eq!(name, "origin");
    }
    for index in 0..3 {
        assert_eq!(
            remote_headers[remote_headers.len() - 3 + index].1,
            worktree_headers[index + 1].1
        );
        assert_eq!(
            remote_values[remote_values.len() - 3 + index].1,
            worktree_values[index + 1].1
        );
    }
    Ok(())
}

pub(super) fn assert_fitted_column(
    buffer: &Buffer,
    header_y: u16,
    value_y: u16,
    column: usize,
    whole: &str,
) -> anyhow::Result<()> {
    let row_text = |y| -> String {
        (buffer.area.x + 1..buffer.area.right() - 1)
            .map(|x| buffer[(x, y)].symbol())
            .collect()
    };
    let header_row = row_text(header_y);
    let value_row = row_text(value_y);
    let headers: Vec<_> = header_row.split_whitespace().collect();
    let values: Vec<_> = value_row.split_whitespace().collect();
    assert!(!headers.is_empty(), "{header_row}");
    assert!(
        headers.iter().all(|header| !header.contains('…')),
        "{header_row}"
    );
    assert_eq!(values.len(), headers.len(), "{value_row}");
    let area = Rect::new(buffer.area.x, header_y, buffer.area.width, 1);
    let start =
        test_support::find_text_cell(buffer, area, headers[column]).context("Column header")?;
    let end = if let Some(next) = headers.get(column + 1) {
        test_support::find_text_cell(buffer, area, next)
            .context("Next header")?
            .0
            - 1
    } else {
        buffer.area.right() - 2
    };
    let width = usize::from(end - start.0);
    let token = values[column];
    if whole.width() <= width {
        assert_eq!(token, whole, "{value_row}");
    } else {
        let prefix = token.strip_suffix('…').context("Cut value ellipsis")?;
        assert!(
            !prefix.is_empty() && whole.starts_with(prefix),
            "{value_row}"
        );
        assert_eq!(token.width(), width, "{value_row}");
    }
    Ok(())
}

#[test]
fn worktrees_keep_whole_headers_and_share_visible_sync_columns_with_remotes() -> anyhow::Result<()>
{
    for (width, pane_width) in [(80, 17), (95, 24)] {
        let mut app = test_support::project_app();
        app.framework.color_mode = ColorMode::Colored;
        let comparison = Comparison::Counted(AheadBehind {
            ahead:  0,
            behind: 0,
        });
        let name = "proof_crate_with_a_very_long_checkout_name";
        app.panes.git.set_content(GitData {
            description: DescriptionState::KnownEmpty,
            worktrees: vec![WorktreeInfo {
                name:         name.to_string(),
                path:         "/tmp/proof_crate".to_string(),
                branch:       Some("a".to_string()),
                tracked:      Some("b".to_string()),
                ahead_behind: comparison,
            }],
            remotes: RemoteRows::Listed(vec![RemoteRow {
                name:            "origin".to_string(),
                icon:            "",
                display_url:     "github.com/nate/proof_crate".to_string(),
                branch:          "c".to_string(),
                tracked_ref:     TrackedRef::Named("d".to_string()),
                status:          comparison,
                full_url:        None,
                push_annotation: None,
            }]),
            ..GitData::default()
        });
        let buffer = test_support::draw_pane_in_window(&mut app, PaneId::Git, width, pane_width)?;
        let area = Rect::new(0, 0, pane_width, buffer.area.height);
        let worktrees = test_support::find_text_cell(&buffer, area, "Worktrees")
            .context("Worktrees section")?;
        let remotes =
            test_support::find_text_cell(&buffer, area, "Remotes").context("Remotes section")?;
        let worktree_area = Rect::new(
            0,
            worktrees.1 + 1,
            pane_width,
            area.bottom() - worktrees.1 - 2,
        );
        let remote_area = Rect::new(0, remotes.1 + 1, pane_width, worktrees.1 - remotes.1 - 1);
        let worktree_y = test_support::find_text_cell(&buffer, worktree_area, "☑️")
            .context("Worktree Sync")?
            .1;
        assert_worktree_columns(
            &buffer,
            area,
            worktrees.1 + 1,
            worktree_y,
            [name, "a", "b", "☑️"],
        )?;
        assert_shared_remote_columns(&buffer, area, remotes.1 + 1, worktrees.1 + 1)?;
        let remote_y = test_support::find_text_cell(&buffer, remote_area, "☑️")
            .context("Remote Sync")?
            .1;
        for (label, worktree_value, remote_value) in [
            ("Branch", "a", "c"),
            ("Tracked", "b", "d"),
            ("Sync", "☑️", "☑️"),
        ] {
            let remote_header = Rect::new(0, remotes.1 + 1, pane_width, 1);
            if test_support::find_text_cell(&buffer, remote_header, label).is_none() {
                continue;
            }
            let worktree = test_support::find_text_cell(
                &buffer,
                Rect::new(0, worktree_y, pane_width, 1),
                worktree_value,
            )
            .context("Whole worktree value")?;
            let remote = test_support::find_text_cell(
                &buffer,
                Rect::new(0, remote_y, pane_width, 1),
                remote_value,
            )
            .context("Whole remote value")?;
            assert_eq!(worktree.0, remote.0, "{width}: {worktree_value}");
        }
    }
    Ok(())
}

#[test]
fn compact_worktree_tables_keep_both_blank_margins() -> anyhow::Result<()> {
    for (width, pane_width) in [(80, 17), (95, 24)] {
        let mut app = test_support::project_app();
        app.framework.color_mode = ColorMode::Colored;
        app.panes.git.set_content(GitData {
            description: DescriptionState::KnownEmpty,
            worktrees: ["first_checkout", "second_checkout"]
                .map(|name| WorktreeInfo {
                    name:         name.to_string(),
                    path:         format!("/tmp/{name}"),
                    branch:       Some("a".to_string()),
                    tracked:      Some("b".to_string()),
                    ahead_behind: Comparison::Counted(AheadBehind {
                        ahead:  0,
                        behind: 0,
                    }),
                })
                .to_vec(),
            ..GitData::default()
        });
        let buffer = test_support::draw_pane_in_window(&mut app, PaneId::Git, width, pane_width)?;
        let title = test_support::position(&buffer, "Worktrees")?;
        for y in title.1 + 1..=title.1 + 3 {
            assert_eq!(buffer[(0, y)].symbol(), "│");
            assert_eq!(
                buffer[(1, y)].symbol(),
                " ",
                "{width}: {:?}",
                test_support::rows(&buffer)
            );
            assert_eq!(buffer[(pane_width - 2, y)].symbol(), " ");
            assert_eq!(buffer[(pane_width - 1, y)].symbol(), "│");
        }
    }
    Ok(())
}

#[test]
fn compact_worktree_tables_keep_wide_branch_text_and_whole_sync() -> anyhow::Result<()> {
    let mut app = test_support::project_app();
    app.framework.color_mode = ColorMode::Colored;
    app.panes.git.set_content(GitData {
        description: DescriptionState::KnownEmpty,
        worktrees: vec![WorktreeInfo {
            name:         "proof_crate".to_string(),
            path:         "/tmp/proof_crate".to_string(),
            branch:       Some("界界".to_string()),
            tracked:      None,
            ahead_behind: Comparison::NotCompared,
        }],
        ..GitData::default()
    });
    let buffer = test_support::draw_pane_in_window(&mut app, PaneId::Git, 80, 17)?;
    let title = test_support::position(&buffer, "Worktrees")?;
    let text = |y| -> anyhow::Result<String> {
        let mut row = String::new();
        let mut x = 1;
        while x < 16 {
            let symbol = buffer[(x, y)].symbol();
            row.push_str(symbol);
            x += u16::try_from(symbol.width().max(1))?;
        }
        Ok(row)
    };
    let header = text(title.1 + 1)?;
    let row = text(title.1 + 2)?;
    let values: Vec<_> = row.split_whitespace().collect();
    assert_eq!(
        header.split_whitespace().collect::<Vec<_>>(),
        ["Branch", "Sync"],
        "{header}"
    );
    assert_eq!(values.len(), 2, "{row}");
    assert_eq!(values[0], "界界", "{row}");
    assert_eq!(values[1], "──", "{row}");
    Ok(())
}

#[test]
fn long_branches_keep_visible_values_whole_or_marked_in_both_tables() -> anyhow::Result<()> {
    for (window, pane_width) in [(80, 17), (95, 24), (160, 80)] {
        let tracked = if window == 80 { "ma" } else { "main" };
        let mut app = test_support::project_app();
        app.framework.color_mode = ColorMode::Colored;
        app.panes.git.set_content(GitData {
            description: DescriptionState::KnownEmpty,
            worktrees: vec![WorktreeInfo {
                name:         "proof_crate".to_string(),
                path:         "/tmp/proof_crate".to_string(),
                branch:       Some("verify_a_very_long_branch".to_string()),
                tracked:      Some(tracked.to_string()),
                ahead_behind: Comparison::NotCompared,
            }],
            remotes: RemoteRows::Listed(vec![RemoteRow {
                name:            "origin".to_string(),
                icon:            "",
                display_url:     "github.com/nate/proof_crate".to_string(),
                branch:          "verify_a_very_long_branch".to_string(),
                tracked_ref:     TrackedRef::Named(tracked.to_string()),
                status:          Comparison::NotCompared,
                full_url:        None,
                push_annotation: None,
            }]),
            ..GitData::default()
        });
        app.panes.git.focus.pane_focus_state = PaneFocusState::Active;
        let fields = panes::git_fields_from_data(app.panes.git.content().context("Git content")?);
        for (offset, label) in [(0, "Remotes"), (1, "Worktrees")] {
            app.panes.git.viewport.set_pos(fields.len() + offset);
            let buffer =
                test_support::draw_pane_in_window(&mut app, PaneId::Git, window, pane_width)?;
            test_support::position(&buffer, label)?;
            let worktree = test_support::position(&buffer, "Worktrees")?;
            let remote = test_support::position(&buffer, "Remotes")?;
            let area = Rect::new(0, 0, pane_width, buffer.area.height);
            assert_shared_remote_columns(&buffer, area, remote.1 + 1, worktree.1 + 1)?;
            for section in [remote, worktree] {
                let row = Rect::new(0, section.1 + 2, pane_width, 1);
                let header = Rect::new(0, section.1 + 1, pane_width, 1);
                let header_text: String = (1..pane_width - 1)
                    .map(|x| buffer[(x, header.y)].symbol())
                    .collect();
                let headers: Vec<_> = header_text.split_whitespace().collect();
                if headers.contains(&"Tracked") {
                    test_support::find_text_cell(&buffer, row, tracked)
                        .context("Whole short Tracked value")?;
                } else {
                    assert!(pane_width < 29, "{header_text}");
                    assert!(
                        !headers.iter().any(|label| label.contains('…')),
                        "{header_text}"
                    );
                }
                let text: String = (1..pane_width - 1)
                    .map(|x| buffer[(x, row.y)].symbol())
                    .collect();
                let values: Vec<_> = text.split_whitespace().collect();
                if let Some(branch) = headers.iter().position(|label| *label == "Branch") {
                    assert!(
                        values[branch] == "verify_a_very_long_branch"
                            || values[branch].ends_with('…'),
                        "{text}"
                    );
                }
                assert_eq!(values.last(), Some(&"──"));
                assert_eq!(buffer[(1, row.y)].symbol(), " ");
                assert_eq!(buffer[(pane_width - 2, row.y)].symbol(), " ");
            }
            let title_row = &test_support::rows(&buffer)
                [usize::from(if offset == 0 { remote.1 } else { worktree.1 })];
            let count = if window == 80 { "(1)" } else { "(1 of 1)" };
            assert!(
                title_row.contains(&format!("{label} {count}")),
                "{title_row}"
            );
        }
    }
    Ok(())
}

#[test]
fn worktree_tables_drop_columns_before_cutting_headers() -> anyhow::Result<()> {
    for (pane_width, expected) in [
        (23, vec!["Name", "Branch", "Sync"]),
        (24, vec!["Name", "Branch", "Sync"]),
        (27, vec!["Name", "Branch", "Sync"]),
        (28, vec!["Name", "Branch", "Tracked", "Sync"]),
        (29, vec!["Name", "Branch", "Tracked", "Sync"]),
    ] {
        let mut app = test_support::project_app();
        app.framework.color_mode = ColorMode::Colored;
        app.panes.git.set_content(GitData {
            description: DescriptionState::KnownEmpty,
            worktrees: ["master", "verify-run-place", "sdlc-merge-gate"]
                .into_iter()
                .map(|branch| WorktreeInfo {
                    name:         "proof_crate".to_string(),
                    path:         format!("/tmp/{branch}/proof_crate"),
                    branch:       Some(branch.to_string()),
                    tracked:      (branch != "master").then(|| "master".to_string()),
                    ahead_behind: if branch == "master" {
                        Comparison::NotCompared
                    } else {
                        Comparison::Counted(AheadBehind {
                            ahead:  0,
                            behind: 0,
                        })
                    },
                })
                .collect(),
            ..GitData::default()
        });
        let buffer = test_support::draw_pane_in_window(&mut app, PaneId::Git, 95, pane_width)?;
        let title = test_support::position(&buffer, "Worktrees")?;
        let row = Rect::new(0, title.1 + 1, pane_width, 1);
        test_support::find_text_cell(&buffer, row, "Branch").context("whole Branch header")?;
        let header: String = (1..pane_width - 1)
            .map(|x| buffer[(x, row.y)].symbol())
            .collect();
        assert_eq!(
            header.split_whitespace().collect::<Vec<_>>(),
            expected,
            "{header}"
        );
    }
    Ok(())
}
