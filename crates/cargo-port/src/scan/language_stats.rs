use std::cmp::Reverse;
use std::path::Path;

use tokei::Config;
use tokei::Languages;

use super::BackgroundMsg;
use super::cargo_metadata::StreamingScanContext;
use super::disk_usage::DiskUsageTree;
use super::disk_usage;
use crate::project::AbsolutePath;
use crate::project::LangEntry;
use crate::project::LanguageStats;

pub(super) fn spawn_initial_language_stats(
    scan_context: &StreamingScanContext,
    disk_entries: &[(String, AbsolutePath)],
) {
    for tree in disk_usage::group_disk_usage_trees(disk_entries) {
        spawn_language_stats_tree(scan_context, tree);
    }
}

fn spawn_language_stats_tree(scan_context: &StreamingScanContext, tree: DiskUsageTree) {
    let handle = scan_context.client.handle.clone();
    let tx = scan_context.tx.clone();

    handle.spawn(async move {
        let Ok(results) =
            tokio::task::spawn_blocking(move || collect_language_stats_for_tree(&tree)).await
        else {
            return;
        };
        if !results.is_empty() {
            let _ = tx.send(BackgroundMsg::LanguageStatsBatch { entries: results });
        }
    });
}

fn collect_language_stats_for_tree(tree: &DiskUsageTree) -> Vec<(AbsolutePath, LanguageStats)> {
    let config = Config {
        hidden: Some(false),
        ..tokei::Config::default()
    };
    let mut languages = tokei::Languages::new();
    languages.get_statistics(&[&tree.root_abs_path], &[], &config);

    // Build a single LanguageStats from all results — this covers the root.
    let stats = build_language_stats(&languages);

    // For each entry in the tree, run tokei on that specific subtree if it
    // differs from the root. For simple single-project trees, just reuse
    // the root stats.
    if tree.entries.len() == 1 {
        return vec![(tree.entries[0].clone(), stats)];
    }

    // Multi-entry tree: root gets the full stats, members get their own.
    let mut results = Vec::with_capacity(tree.entries.len());
    for entry in &tree.entries {
        if entry.as_path() == tree.root_abs_path.as_path() {
            results.push((entry.clone(), stats.clone()));
        } else {
            let mut member_langs = tokei::Languages::new();
            member_langs.get_statistics(&[entry.as_path()], &[], &config);
            results.push((entry.clone(), build_language_stats(&member_langs)));
        }
    }
    results
}

fn build_language_stats(languages: &Languages) -> LanguageStats {
    let mut entries: Vec<LangEntry> = languages
        .iter()
        .filter(|(_, lang)| lang.code > 0 || !lang.reports.is_empty())
        .map(|(lang_type, lang)| {
            // Merge children (e.g., doc comments classified as embedded
            // Markdown) back into the parent language's counts.
            let (child_code, child_comments, child_blanks) = lang
                .children
                .values()
                .flat_map(|reports| reports.iter())
                .fold((0, 0, 0), |(c, m, b), report| {
                    (
                        c + report.stats.code,
                        m + report.stats.comments,
                        b + report.stats.blanks,
                    )
                });
            LangEntry {
                language: lang_type.to_string(),
                files:    lang.reports.len(),
                code:     lang.code + child_code,
                comments: lang.comments + child_comments,
                blanks:   lang.blanks + child_blanks,
            }
        })
        .collect();
    entries.sort_by_key(|entry| Reverse(entry.code));
    LanguageStats { entries }
}

/// Collect language stats for a single project path (watcher discovery).
pub(crate) fn collect_language_stats_single(path: &Path) -> LanguageStats {
    let config = Config {
        hidden: Some(false),
        ..tokei::Config::default()
    };
    let mut languages = tokei::Languages::new();
    languages.get_statistics(&[path], &[], &config);
    build_language_stats(&languages)
}
