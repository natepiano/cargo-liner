mod dispatch;
mod index;

pub(super) use dispatch::dispatch_finder_action;
pub(super) use dispatch::handle_finder_text_key;
pub(super) use dispatch::render_finder_pane_body;
#[cfg(test)]
use dispatch::search_finder;
pub(super) use index::FINDER_COLUMN_COUNT;
pub(super) use index::FinderItem;
#[cfg(test)]
use index::FinderKind;
pub(super) use index::build_finder_index;
#[cfg(test)]
use index::build_search_tokens;

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::project::AbsolutePath;
    use crate::project::Cargo;
    use crate::project::ExampleGroup;
    use crate::project::Package;
    use crate::project::ProjectType;
    use crate::project::RootItem;
    use crate::project::RustInfo;
    use crate::project::RustProject;
    use crate::project::VendoredPackage;
    use crate::project::Workspace;

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
    fn build_finder_index_includes_vendored_projects() {
        let ws = Workspace {
            path: test_path("~/rust/hana"),
            name: Some("hana".to_string()),
            rust: RustInfo {
                vendored: vec![VendoredPackage {
                    path: test_path("~/rust/hana/crates/clay-layout"),
                    name: Some("clay-layout".to_string()),
                    ..VendoredPackage::default()
                }],
                ..RustInfo::default()
            },
            ..Workspace::default()
        };
        let list_items = crate::tui::project_list::ProjectList::new(vec![RootItem::Rust(
            RustProject::Workspace(ws),
        )]);
        let (items, _widths) = build_finder_index(&list_items);
        assert!(items.iter().any(|item| {
            item.project_path == test_path("~/rust/hana/crates/clay-layout")
                && item.display_name == "clay-layout (vendored)"
                && item.branch.is_empty()
        }));
    }

    #[test]
    fn finder_single_word_does_not_match_across_unrelated_tokens() {
        let item = FinderItem {
            display_name:  "clay-layout (vendored)".to_string(),
            search_tokens: build_search_tokens(&[
                "clay-layout (vendored)",
                "clay-layout",
                "clay-layout",
                "~/rust/bevy_diegetic/clay-layout",
                "vendored",
                FinderKind::Project.label(),
            ]),
            kind:          FinderKind::Project,
            project_path:  test_path("~/rust/bevy_diegetic/clay-layout"),
            target_name:   None,
            parent_label:  "clay-layout".to_string(),
            branch:        String::new(),
            dir:           "~/rust/bevy_diegetic/clay-layout".to_string(),
        };

        let (results, total) = search_finder(&[item], "android", 50);
        assert!(results.is_empty());
        assert_eq!(total, 0);
    }

    #[test]
    fn finder_single_word_matches_directory_token() {
        let item = FinderItem {
            display_name:  "raylib_renderer".to_string(),
            search_tokens: build_search_tokens(&[
                "raylib_renderer",
                "clay-layout",
                "~/rust/bevy_diegetic/clay-layout",
                "",
                FinderKind::Example.label(),
            ]),
            kind:          FinderKind::Example,
            project_path:  test_path("~/rust/bevy_diegetic/clay-layout"),
            target_name:   Some("raylib_renderer".to_string()),
            parent_label:  "clay-layout".to_string(),
            branch:        String::new(),
            dir:           "~/rust/bevy_diegetic/clay-layout".to_string(),
        };

        let (results, total) = search_finder(&[item], "diegetic", 50);
        assert_eq!(results, vec![0]);
        assert_eq!(total, 1);
    }

    #[test]
    fn finder_multi_word_matches_across_tokens() {
        let item = FinderItem {
            display_name:  "build-easefunction-graphs".to_string(),
            search_tokens: build_search_tokens(&[
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
        };

        let (results, total) = search_finder(&[item], "tools graphs", 50);
        assert_eq!(results, vec![0]);
        assert_eq!(total, 1);
    }

    #[test]
    fn build_finder_index_tokenizes_display_name_and_dir_segments() {
        let pkg = Package {
            path: test_path("~/rust/bevy/tools/build-easefunction-graphs"),
            name: Some("build-easefunction-graphs".to_string()),
            rust: RustInfo {
                cargo: Cargo {
                    types: vec![ProjectType::Binary],
                    examples: vec![ExampleGroup {
                        category: String::new(),
                        names:    vec!["raylib_renderer".to_string()],
                    }],
                    ..Cargo::default()
                },
                ..RustInfo::default()
            },
            ..Package::default()
        };

        let (items, _widths) =
            build_finder_index(&crate::tui::project_list::ProjectList::new(vec![
                RootItem::Rust(RustProject::Package(pkg)),
            ]));
        assert!(items.iter().any(|item| {
            item.display_name == "build-easefunction-graphs"
                && item.search_tokens.iter().any(|token| token == "tools")
                && item.search_tokens.iter().any(|token| token == "graphs")
        }));
    }
}
