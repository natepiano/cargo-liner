use std::cmp::Reverse;

use crossterm::event::KeyCode;
use nucleo_matcher::Matcher;
use nucleo_matcher::Utf32Str;
use nucleo_matcher::pattern::Atom;
use nucleo_matcher::pattern::AtomKind;
use nucleo_matcher::pattern::CaseMatching;
use nucleo_matcher::pattern::Normalization;
use tui_pane::AppContext;
use tui_pane::FocusedPane;

use super::index::FinderItem;
use super::index::FinderKind;
use crate::ci;
use crate::tui::app::App;
use crate::tui::constants::MAX_FINDER_RESULTS;
use crate::tui::integration::AppPaneId;
use crate::tui::keymap::FinderAction;
use crate::tui::panes;
use crate::tui::panes::GitRow;
use crate::tui::panes::RunTargetKind;

/// "bench diegetic" and "diegetic bench" produce the same results.
pub(super) fn search_finder(
    index: &[FinderItem],
    query: &str,
    max_results: usize,
) -> (Vec<usize>, usize) {
    if query.is_empty() {
        return (Vec::new(), 0);
    }

    let words: Vec<&str> = query
        .split(|ch: char| ch.is_whitespace() || matches!(ch, '/' | '\\'))
        .filter(|word| !word.is_empty())
        .collect();
    if words.is_empty() {
        return (Vec::new(), 0);
    }

    let atoms: Vec<Atom> = words
        .iter()
        .map(|word| {
            Atom::new(
                word,
                CaseMatching::Smart,
                Normalization::Smart,
                AtomKind::Fuzzy,
                false,
            )
        })
        .collect();

    let mut matcher = Matcher::default();
    let mut scored: Vec<(usize, u16)> = index
        .iter()
        .enumerate()
        .filter_map(|(i, finder_item)| {
            let mut total_score: u16 = 0;
            for atom in &atoms {
                let score = finder_item
                    .search_tokens
                    .iter()
                    .filter_map(|token| {
                        let mut buf = Vec::new();
                        let utf32_str = Utf32Str::new(token, &mut buf);
                        atom.score(utf32_str, &mut matcher)
                    })
                    .max()?;
                total_score = total_score.saturating_add(score);
            }
            Some((i, total_score))
        })
        .collect();

    let total = scored.len();
    scored.sort_by_key(|entry| Reverse(entry.1));
    let indices = scored
        .into_iter()
        .take(max_results)
        .map(|(i, _)| i)
        .collect();
    (indices, total)
}

// `dispatch_finder_action` and `handle_finder_text_key` handle finder input.

pub(in crate::tui) fn dispatch_finder_action(action: FinderAction, app: &mut App) {
    match action {
        FinderAction::Activate => confirm_finder(app),
        FinderAction::Cancel => close_finder(app),
    }
}

pub(in crate::tui) fn handle_finder_text_key(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Up => {
            app.overlays.finder_pane.viewport.up();
        },
        KeyCode::Down => {
            app.overlays.finder_pane.viewport.down();
        },
        KeyCode::Home => {
            app.overlays.finder_pane.viewport.home();
        },
        KeyCode::End => {
            app.overlays.finder_pane.viewport.end();
        },
        KeyCode::PageUp => {
            app.overlays.finder_pane.viewport.page_up();
        },
        KeyCode::PageDown => {
            app.overlays.finder_pane.viewport.page_down();
        },
        KeyCode::Backspace => {
            if app.project_list.finder.query.is_empty() {
                close_finder(app);
            } else {
                app.project_list.finder.query.pop();
                refresh_finder_results(app);
            }
        },
        KeyCode::Char(c) => {
            app.project_list.finder.query.push(c);
            refresh_finder_results(app);
        },
        _ => {},
    }
}

fn close_finder(app: &mut App) {
    let focused_pane = app
        .overlays
        .take_finder_return()
        .unwrap_or(FocusedPane::App(AppPaneId::ProjectList));
    app.overlays.close_finder();
    app.project_list.finder.query.clear();
    app.project_list.finder.results.clear();
    app.overlays.finder_pane.viewport.home();
    app.set_focus(focused_pane);
}

fn refresh_finder_results(app: &mut App) {
    let (results, total) = {
        let finder = &app.project_list.finder;
        search_finder(&finder.index, &finder.query, MAX_FINDER_RESULTS)
    };
    let finder = &mut app.project_list.finder;
    finder.results = results;
    finder.total = total;
    app.overlays.finder_pane.viewport.home();
}

fn confirm_finder(app: &mut App) {
    let Some(&idx) = app
        .project_list
        .finder
        .results
        .get(app.overlays.finder_pane.viewport.pos())
    else {
        return;
    };
    let finder_item = app.project_list.finder.index[idx].clone();

    let focused_pane = app
        .overlays
        .take_finder_return()
        .unwrap_or(FocusedPane::App(AppPaneId::ProjectList));
    app.overlays.close_finder();
    app.project_list.finder.query.clear();
    app.project_list.finder.results.clear();
    app.overlays.finder_pane.viewport.home();
    app.set_focus(focused_pane);

    // `project_list.select_project_in_tree` selects `item.project_path` before any target or
    // pull-request navigation.
    let include_non_rust = app.config.include_non_rust().includes_non_rust();
    app.project_list
        .select_project_in_tree(finder_item.project_path.as_path(), include_non_rust);
    app.record_selection_choice();

    match finder_item.kind {
        FinderKind::Project => {
            // `FinderKind::Project` needs no further action after `select_project_in_tree`.
        },
        FinderKind::Binary | FinderKind::Example | FinderKind::Bench => {
            navigate_to_target(app, &finder_item);
        },
        FinderKind::PullRequest => {
            navigate_to_pull_request(app, &finder_item);
        },
    }
}

/// Rebuild the detail panes for the just-selected project. `select_project_in_tree`
/// only moves the tree cursor; the panes are otherwise repopulated lazily on the
/// next render — too late for the finder, which reads pane content synchronously
/// here to land the cursor on a specific row.
fn repopulate_selected_detail_panes(app: &mut App) {
    app.sync_selected_project();
    app.ensure_detail_cached();
}

/// After selecting the parent project, focus the targets column and scroll
/// to the matching target entry.
fn navigate_to_target(app: &mut App, item: &FinderItem) {
    repopulate_selected_detail_panes(app);
    let Some(targets_data) = app.panes.targets.available_content(&app.project_list) else {
        return;
    };
    if !targets_data.has_targets() {
        return;
    }
    app.set_focus(FocusedPane::App(AppPaneId::Targets));

    // Match `entry.name` and `entry.run_target_kind` against `target_name` and `target_kind`, then
    // call `app.panes.targets.viewport.set_pos(i)`.
    let target_entries = panes::build_target_list_from_data(&targets_data);
    let target_kind = match item.kind {
        FinderKind::Binary => RunTargetKind::Binary,
        FinderKind::Example => RunTargetKind::Example,
        FinderKind::Bench => RunTargetKind::Bench,
        FinderKind::Project | FinderKind::PullRequest => return,
    };
    let target_name = item.target_name.as_deref().unwrap_or("");
    for (i, target_entry) in target_entries.iter().enumerate() {
        if target_entry.name == target_name
            && std::mem::discriminant(&target_entry.run_target_kind)
                == std::mem::discriminant(&target_kind)
        {
            app.panes.targets.viewport.set_pos(i);
            return;
        }
    }
}

fn navigate_to_pull_request(app: &mut App, item: &FinderItem) {
    let Some(pull_request_target) = item.pr_target.as_ref() else {
        return;
    };
    repopulate_selected_detail_panes(app);
    let selected_owner_repo = app
        .project_list
        .selected_project_path()
        .and_then(|path| app.project_list.fetch_url_for(path))
        .and_then(|url| ci::parse_owner_repo(&url));
    if selected_owner_repo.as_ref() != Some(&pull_request_target.owner_repo) {
        return;
    }
    app.set_focus(FocusedPane::App(AppPaneId::Git));
    let Some(git_data) = app.panes.git.content() else {
        return;
    };
    let mut index = 0;
    while let Some(git_row) = panes::git_row_at(git_data, index) {
        if let GitRow::PullRequest(pull_request) = git_row
            && pull_request.number == pull_request_target.number
        {
            app.panes.git.viewport.set_pos(index);
            return;
        }
        index += 1;
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::search_finder;
    use crate::project::AbsolutePath;
    use crate::tui::finder::index;
    use crate::tui::finder::index::FinderItem;
    use crate::tui::finder::index::FinderKind;

    fn test_path(path: &str) -> AbsolutePath {
        let pb = if path == "~" {
            dirs::home_dir().unwrap_or_else(|| PathBuf::from(path))
        } else if let Some(rest) = path.strip_prefix("~/") {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("/tmp"))
                .join(rest)
        } else {
            PathBuf::from(path)
        };
        AbsolutePath::from(pb)
    }

    #[test]
    fn single_word_does_not_match_across_unrelated_tokens() {
        let finder_item = FinderItem {
            display_name:  "clay-layout (vendored)".to_string(),
            search_tokens: index::build_search_tokens(&[
                "clay-layout (vendored)",
                "clay-layout",
                "clay-layout",
                "~/rust/fake_widgets/clay-layout",
                "vendored",
                FinderKind::Project.label(),
            ]),
            kind:          FinderKind::Project,
            project_path:  test_path("~/rust/fake_widgets/clay-layout"),
            target_name:   None,
            parent_label:  "clay-layout".to_string(),
            branch:        String::new(),
            dir:           "~/rust/fake_widgets/clay-layout".to_string(),
            pr_target:     None,
        };

        let (results, total) = search_finder(&[finder_item], "android", 50);
        assert_eq!(results, [] as [usize; 0]);
        assert_eq!(total, 0);
    }

    #[test]
    fn single_word_matches_directory_token() {
        let finder_item = FinderItem {
            display_name:  "raylib_renderer".to_string(),
            search_tokens: index::build_search_tokens(&[
                "raylib_renderer",
                "clay-layout",
                "~/rust/fake_widgets/clay-layout",
                "",
                FinderKind::Example.label(),
            ]),
            kind:          FinderKind::Example,
            project_path:  test_path("~/rust/fake_widgets/clay-layout"),
            target_name:   Some("raylib_renderer".to_string()),
            parent_label:  "clay-layout".to_string(),
            branch:        String::new(),
            dir:           "~/rust/fake_widgets/clay-layout".to_string(),
            pr_target:     None,
        };

        let (results, total) = search_finder(&[finder_item], "widgets", 50);
        assert_eq!(results, vec![0]);
        assert_eq!(total, 1);
    }

    #[test]
    fn multi_word_matches_across_tokens() {
        let finder_item = FinderItem {
            display_name:  "build-easefunction-graphs".to_string(),
            search_tokens: index::build_search_tokens(&[
                "build-easefunction-graphs",
                "build-easefunction-graphs",
                "~/rust/bevy/tools/build-easefunction-graphs",
                "fix/position-before-size-v0.19",
                FinderKind::Binary.label(),
            ]),
            kind:          FinderKind::Binary,
            project_path:  test_path("~/rust/bevy/tools/build-easefunction-graphs"),
            target_name:   Some("build-easefunction-graphs".to_string()),
            parent_label:  "build-easefunction-graphs".to_string(),
            branch:        "fix/position-before-size-v0.19".to_string(),
            dir:           "~/rust/bevy/tools/build-easefunction-graphs".to_string(),
            pr_target:     None,
        };

        let (results, total) = search_finder(&[finder_item], "tools graphs", 50);
        assert_eq!(results, vec![0]);
        assert_eq!(total, 1);
    }

    #[test]
    fn treats_slash_as_word_separator() {
        let finder_item = FinderItem {
            display_name:  "bevy".to_string(),
            search_tokens: index::build_search_tokens(&[
                "bevy",
                "bevy",
                "~/rust/bevy",
                "main",
                FinderKind::Project.label(),
            ]),
            kind:          FinderKind::Project,
            project_path:  test_path("~/rust/bevy"),
            target_name:   None,
            parent_label:  "bevy".to_string(),
            branch:        "main".to_string(),
            dir:           "~/rust/bevy".to_string(),
            pr_target:     None,
        };
        let index = [finder_item];
        let (with_slash, total_slash) = search_finder(&index, "rust/", 50);
        assert_eq!(with_slash, vec![0]);
        assert_eq!(total_slash, 1);
        let (with_path, total_path) = search_finder(&index, "rust/bevy", 50);
        assert_eq!(with_path, vec![0]);
        assert_eq!(total_path, 1);
    }
}
