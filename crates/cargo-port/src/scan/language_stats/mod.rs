use std::cmp::Reverse;
use std::path::Path;

use ignore::WalkBuilder;
use rayon::prelude::*;
use tokei::Config;
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

const LANGUAGE_PROGRESS_CHUNK_SIZE: usize = 32;
const TOKEI_IGNORE_FILE: &str = ".tokeignore";

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
    let config = Config {
        hidden: Some(false),
        ..tokei::Config::default()
    };
    if tree.entries.len() == 1 {
        let entry = tree.entries[0].clone();
        send_language_stats_entry(
            tx,
            entry.clone(),
            collect_path_language_stats(&entry, &config, Some(tx)),
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
                collect_path_language_stats(entry, &config, Some(tx)),
            );
        }
    }

    if let Some(entry) = root_entry {
        send_language_stats_entry(
            tx,
            entry,
            collect_path_language_stats(&tree.root_abs_path, &config, Some(tx)),
        );
    }
}

fn collect_path_language_stats(
    root: &AbsolutePath,
    config: &Config,
    progress_tx: Option<&Sender<BackgroundMsg>>,
) -> LanguageStats {
    let files = language_files(root, config);
    if let Some(tx) = progress_tx {
        send_language_progress_plan(tx, &files);
    }

    let parsed: Vec<_> = files
        .par_chunks(LANGUAGE_PROGRESS_CHUNK_SIZE)
        .flat_map_iter(|chunk| {
            let mut completed = Vec::with_capacity(chunk.len());
            let mut parsed = Vec::with_capacity(chunk.len());
            for (path, language) in chunk {
                let result = language.parse(path.as_path().to_path_buf(), config);
                completed.push(path.clone());
                parsed.push((*language, result));
            }
            if let Some(tx) = progress_tx {
                send_language_progress_batch(tx, completed);
            }
            parsed
        })
        .collect();

    let mut languages = Languages::new();
    for (language, result) in parsed {
        let entry = languages.entry(language).or_default();
        match result {
            Ok(report) => entry.add_report(report),
            Err((error, path)) => {
                entry.mark_inaccurate();
                tracing::debug!(
                    path = %path.display(),
                    error = %error,
                    "language_stats_file_read_failed"
                );
            },
        }
    }
    for language in languages.values_mut() {
        language.total();
    }
    build_language_stats(root.as_path(), &languages, config)
}

fn language_files(root: &AbsolutePath, config: &Config) -> Vec<(AbsolutePath, LanguageType)> {
    let mut walker = WalkBuilder::new(root.as_path());
    configure_language_walker(&mut walker, config);
    walker
        .build()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_type()
                .is_some_and(|file_type| file_type.is_file())
        })
        .filter_map(|entry| {
            let path = entry.into_path();
            let language = LanguageType::from_path(&path, config)?;
            Some((AbsolutePath::from(path), language))
        })
        .collect()
}

fn configure_language_walker(walker: &mut WalkBuilder, config: &Config) {
    let ignore = config.no_ignore.is_none_or(|enabled| !enabled);
    let ignore_dot = ignore && config.no_ignore_dot.is_none_or(|enabled| !enabled);
    let ignore_vcs = ignore && config.no_ignore_vcs.is_none_or(|enabled| !enabled);

    if ignore_dot {
        walker.add_custom_ignore_filename(TOKEI_IGNORE_FILE);
    }

    walker
        .git_exclude(ignore_vcs)
        .git_global(ignore_vcs)
        .git_ignore(ignore_vcs)
        .hidden(config.hidden.is_none_or(|enabled| !enabled))
        .ignore(ignore_dot)
        .parents(ignore && config.no_ignore_parent.is_none_or(|enabled| !enabled));
}

fn send_language_progress_plan(tx: &Sender<BackgroundMsg>, files: &[(AbsolutePath, LanguageType)]) {
    let entries: Vec<AbsolutePath> = files.iter().map(|(path, _)| path.clone()).collect();
    if entries.is_empty() {
        return;
    }
    let _ = tx.send(BackgroundMsg::LanguageStatsProgressPlan { entries });
}

fn send_language_progress_batch(tx: &Sender<BackgroundMsg>, entries: Vec<AbsolutePath>) {
    if entries.is_empty() {
        return;
    }
    let _ = tx.send(BackgroundMsg::LanguageStatsProgressBatch { entries });
}

fn send_language_stats_entry(tx: &Sender<BackgroundMsg>, path: AbsolutePath, stats: LanguageStats) {
    let _ = tx.send(BackgroundMsg::LanguageStatsBatch {
        entries: vec![(path, stats)],
    });
}

fn build_language_stats(root: &Path, languages: &Languages, config: &Config) -> LanguageStats {
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
                    rust_breakdown::child_entries(root, lang, config)
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
    collect_path_language_stats(&root, &config, None)
}
