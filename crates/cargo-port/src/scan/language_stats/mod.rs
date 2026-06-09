use std::cmp::Reverse;
use std::path::Path;

use tokei::Config;
use tokei::Language;
use tokei::LanguageType;
use tokei::Languages;

use super::BackgroundMsg;
use super::cargo_metadata::StreamingScanContext;
use super::disk_usage;
use super::disk_usage::DiskUsageTree;
use crate::channel::Sender;
use crate::project::AbsolutePath;
use crate::project::LangEntry;
use crate::project::LanguageStats;

mod rust_breakdown;

use rust_breakdown::RustBreakdownCache;

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
        let _ = tokio::task::spawn_blocking(move || emit_language_stats_for_tree(&tree, &tx)).await;
    });
}

fn emit_language_stats_for_tree(tree: &DiskUsageTree, tx: &Sender<BackgroundMsg>) {
    let started = std::time::Instant::now();
    let config = Config {
        hidden: Some(false),
        ..tokei::Config::default()
    };
    let mut rust_cache = RustBreakdownCache::default();
    send_language_progress_plan(tx, tree.entries.len());
    let languages = collect_path_languages(&tree.root_abs_path, &config);
    if tree.entries.len() == 1 {
        let entry = tree.entries[0].clone();
        send_language_stats_entry(
            tx,
            entry.clone(),
            build_language_stats(entry.as_path(), &languages, &config, &mut rust_cache),
        );
        tracing::trace!(
            target: tui_pane::PERF_LOG_TARGET,
            elapsed_ms = tui_pane::perf_log_ms(started.elapsed().as_millis()),
            abs_path = %tree.root_abs_path.display(),
            rows = tree.entries.len(),
            "language_stats_tree"
        );
        return;
    }

    let mut root_entry = None;
    for entry in &tree.entries {
        if entry.as_path() == tree.root_abs_path.as_path() {
            root_entry = Some(entry.clone());
        } else {
            send_language_stats_entry(
                tx,
                entry.clone(),
                build_language_stats_for_root(
                    entry.as_path(),
                    &languages,
                    &config,
                    &mut rust_cache,
                ),
            );
        }
    }

    if let Some(entry) = root_entry {
        send_language_stats_entry(
            tx,
            entry,
            build_language_stats(
                tree.root_abs_path.as_path(),
                &languages,
                &config,
                &mut rust_cache,
            ),
        );
    }

    tracing::trace!(
        target: tui_pane::PERF_LOG_TARGET,
        elapsed_ms = tui_pane::perf_log_ms(started.elapsed().as_millis()),
        abs_path = %tree.root_abs_path.display(),
        rows = tree.entries.len(),
        "language_stats_tree"
    );
}

fn collect_path_language_stats(
    root: &AbsolutePath,
    config: &Config,
    rust_cache: &mut RustBreakdownCache,
) -> LanguageStats {
    let languages = collect_path_languages(root, config);
    build_language_stats(root.as_path(), &languages, config, rust_cache)
}

fn collect_path_languages(root: &AbsolutePath, config: &Config) -> Languages {
    let mut languages = Languages::new();
    languages.get_statistics(&[root.as_path()], &[], config);
    languages
}

fn send_language_progress_plan(tx: &Sender<BackgroundMsg>, units: usize) {
    if units == 0 {
        return;
    }
    let _ = tx.send(BackgroundMsg::LanguageStatsProgressPlan { units });
}

fn send_language_stats_entry(tx: &Sender<BackgroundMsg>, path: AbsolutePath, stats: LanguageStats) {
    let _ = tx.send(BackgroundMsg::LanguageStatsBatch {
        entries: vec![(path, stats)],
    });
}

fn build_language_stats_for_root(
    root: &Path,
    languages: &Languages,
    config: &Config,
    rust_cache: &mut RustBreakdownCache,
) -> LanguageStats {
    let mut filtered = Languages::new();
    for (language_type, language) in languages {
        let mut subset = Language::new();
        if language.inaccurate {
            subset.mark_inaccurate();
        }
        for report in &language.reports {
            if report.name.starts_with(root) {
                subset.add_report(report.clone());
            }
        }
        if !subset.reports.is_empty() {
            subset.total();
            filtered.insert(*language_type, subset);
        }
    }
    build_language_stats(root, &filtered, config, rust_cache)
}

fn build_language_stats(
    root: &Path,
    languages: &Languages,
    config: &Config,
    rust_cache: &mut RustBreakdownCache,
) -> LanguageStats {
    let mut entries: Vec<LangEntry> = languages
        .iter()
        .filter(|(_, lang)| lang.code > 0 || !lang.reports.is_empty())
        .map(|(lang_type, lang)| {
            // Merge embedded languages (e.g., doc comments classified as
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
                children: if *lang_type == LanguageType::Rust {
                    rust_breakdown::child_entries(root, lang, config, rust_cache)
                } else {
                    Vec::new()
                },
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
    let root = AbsolutePath::from(path);
    let mut rust_cache = RustBreakdownCache::default();
    collect_path_language_stats(&root, &config, &mut rust_cache)
}
