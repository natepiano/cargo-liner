use std::cmp::Reverse;
use std::collections::HashSet;
use std::path::Path;

use tokei::Config;
use tokei::Language;
use tokei::LanguageType;
use tokei::Languages;
use tokei::Report;
use tui_pane::PERF_LOG_TARGET;

use super::rust_breakdown;
use super::rust_breakdown::RustBreakdownCache;
use crate::channel::Sender;
use crate::project::AbsolutePath;
use crate::project::LangEntry;
use crate::project::LanguageStats;
use crate::scan::BackgroundMsg;
use crate::scan::SourceFileMeasurements;
use crate::scan::SourceMeasurements;
use crate::scan::background_workers;
use crate::scan::cargo_metadata::StreamingScanContext;
use crate::scan::disk_usage;
use crate::scan::disk_usage::DiskUsageTree;

#[derive(Default)]
pub(in crate::scan) struct LanguageMeasurements {
    reports:        SourceFileMeasurements<(LanguageType, Report)>,
    rust_breakdown: RustBreakdownCache,
}

pub fn spawn_initial_language_stats(
    scan_context: &StreamingScanContext,
    disk_entries: &[(String, AbsolutePath)],
) {
    for disk_usage_tree in disk_usage::group_disk_usage_trees(disk_entries) {
        spawn_language_stats_tree(scan_context, disk_usage_tree);
    }
}

fn spawn_language_stats_tree(scan_context: &StreamingScanContext, tree: DiskUsageTree) {
    let handle = scan_context.client.handle.clone();
    let sender = scan_context.sender.clone();
    let measurements = scan_context
        .source_measurements
        .languages(&tree.root_abs_path);

    handle.spawn(async move {
        let _ = tokio::task::spawn_blocking(move || {
            background_workers::measure_files(|| {
                if let Ok(mut measurements) = measurements.lock() {
                    emit_language_stats_for_tree(&tree, &sender, &mut measurements);
                }
            });
        })
        .await;
    });
}

fn emit_language_stats_for_tree(
    tree: &DiskUsageTree,
    sender: &Sender<BackgroundMsg>,
    measurements: &mut LanguageMeasurements,
) {
    let started = std::time::Instant::now();
    let config = Config {
        hidden: Some(false),
        ..tokei::Config::default()
    };
    let LanguageMeasurements {
        reports,
        rust_breakdown: rust_breakdown_cache,
    } = measurements;
    send_language_progress_plan(sender, tree.entries.len());
    let languages = collect_path_languages(&tree.root_abs_path, &config, reports);
    let seen = languages
        .values()
        .flat_map(|language| language.reports.iter().map(|report| report.name.clone()))
        .collect();
    rust_breakdown_cache.retain_seen(&seen);
    if tree.entries.len() == 1 {
        let entry = tree.entries[0].clone();
        send_language_stats_entry(
            sender,
            entry.clone(),
            build_language_stats(entry.as_path(), &languages, &config, rust_breakdown_cache),
        );
        tracing::trace!(
            target: PERF_LOG_TARGET,
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
                sender,
                entry.clone(),
                build_language_stats_for_root(
                    entry.as_path(),
                    &languages,
                    &config,
                    rust_breakdown_cache,
                ),
            );
        }
    }

    if let Some(entry) = root_entry {
        send_language_stats_entry(
            sender,
            entry,
            build_language_stats(
                tree.root_abs_path.as_path(),
                &languages,
                &config,
                rust_breakdown_cache,
            ),
        );
    }

    tracing::trace!(
        target: PERF_LOG_TARGET,
        elapsed_ms = tui_pane::perf_log_ms(started.elapsed().as_millis()),
        abs_path = %tree.root_abs_path.display(),
        rows = tree.entries.len(),
        "language_stats_tree"
    );
}

fn collect_path_languages(
    root: &AbsolutePath,
    config: &Config,
    measurements: &mut SourceFileMeasurements<(LanguageType, Report)>,
) -> Languages {
    let mut languages = Languages::new();
    let mut seen = HashSet::new();
    // Match Tokei's default discovery policy. Walking remains fresh so edits
    // to ignore files and newly created or removed files take effect.
    let mut walker = ignore::WalkBuilder::new(root.as_path());
    walker
        .add_custom_ignore_filename(".tokeignore")
        .hidden(true);
    for entry in walker.build().flatten() {
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        let path = entry.into_path();
        let Some(language_type) = LanguageType::from_path(&path, config) else {
            continue;
        };
        seen.insert(path.clone());
        match measurements.measure(&path, || {
            language_type
                .parse(path.clone(), config)
                .map(|report| (language_type, report))
                .map_err(|(error, _)| error)
        }) {
            Ok((language_type, report)) => languages
                .entry(language_type)
                .or_default()
                .add_report(report),
            Err(_) => languages
                .entry(language_type)
                .or_default()
                .mark_inaccurate(),
        }
    }
    measurements.retain_seen(&seen);
    for language in languages.values_mut() {
        language.total();
    }
    tracing::trace!(target: PERF_LOG_TARGET, root = %root.display(), reads = measurements.reads, reuses = measurements.reuses, retained_files = measurements.files_len(), live_files = seen.len(), "language_source_measurements");
    languages
}

fn send_language_progress_plan(sender: &Sender<BackgroundMsg>, units: usize) {
    if units == 0 {
        return;
    }
    let _ = sender.send(BackgroundMsg::LanguageStatsProgressPlan { units });
}

fn send_language_stats_entry(
    sender: &Sender<BackgroundMsg>,
    path: AbsolutePath,
    stats: LanguageStats,
) {
    let _ = sender.send(BackgroundMsg::LanguageStatsBatch {
        entries: vec![(path, stats)],
    });
}

fn build_language_stats_for_root(
    root: &Path,
    languages: &Languages,
    config: &Config,
    rust_breakdown_cache: &mut RustBreakdownCache,
) -> LanguageStats {
    let mut filtered_languages = Languages::new();
    for (language_type, language) in languages {
        let mut filtered_language = Language::new();
        if language.inaccurate {
            filtered_language.mark_inaccurate();
        }
        for report in &language.reports {
            if report.name.starts_with(root) {
                filtered_language.add_report(report.clone());
            }
        }
        if !filtered_language.reports.is_empty() {
            filtered_language.total();
            filtered_languages.insert(*language_type, filtered_language);
        }
    }
    build_language_stats(root, &filtered_languages, config, rust_breakdown_cache)
}

fn build_language_stats(
    root: &Path,
    languages: &Languages,
    config: &Config,
    rust_breakdown_cache: &mut RustBreakdownCache,
) -> LanguageStats {
    let mut entries: Vec<LangEntry> = languages
        .iter()
        .filter(|(_, lang)| lang.code > 0 || !lang.reports.is_empty())
        .map(|(lang_type, lang)| {
            // Sum `report.stats` from `lang.children` into `LangEntry` counts
            // so embedded Markdown doc comments count toward their parent language.
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
                    rust_breakdown::child_entries(root, lang, config, rust_breakdown_cache)
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
pub(crate) fn language_measurement_job(
    root: AbsolutePath,
    measurements: &SourceMeasurements,
) -> impl FnOnce() -> LanguageStats + use<> {
    let cache = measurements.languages(&root);
    move || {
        background_workers::measure_files(|| {
            let mut cache = cache
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let config = Config {
                hidden: Some(false),
                ..Config::default()
            };
            let languages = collect_path_languages(&root, &config, &mut cache.reports);
            let seen = languages
                .values()
                .flat_map(|language| language.reports.iter().map(|report| report.name.clone()))
                .collect();
            cache.rust_breakdown.retain_seen(&seen);
            build_language_stats(
                root.as_path(),
                &languages,
                &config,
                &mut cache.rust_breakdown,
            )
        })
    }
}

#[cfg(test)]
pub(crate) fn collect_language_stats_single(path: &Path) -> LanguageStats {
    language_measurement_job(
        AbsolutePath::from(path),
        &crate::scan::SourceMeasurements::default(),
    )()
}

#[cfg(test)]
mod measurement_tests {
    use super::*;

    #[test]
    fn enrichment_reuses_live_files_and_drops_removed_temporary_paths() -> std::io::Result<()> {
        let directory = tempfile::tempdir()?;
        let root = AbsolutePath::from(directory.path());
        std::fs::write(directory.path().join("lib.rs"), "fn stable() {}")?;
        let measurements = std::sync::Arc::new(crate::scan::SourceMeasurements::default());
        for index in 0..128 {
            if index > 0 {
                std::fs::remove_file(directory.path().join(format!("event_{}.rs", index - 1)))?;
            }
            std::fs::write(
                directory.path().join(format!("event_{index}.rs")),
                "fn transient() {}",
            )?;
            let stats = language_measurement_job(root.clone(), &measurements)();
            assert_eq!(
                stats.entries.iter().map(|entry| entry.files).sum::<usize>(),
                2
            );
            let cache = measurements.languages(&root);
            let cache = cache
                .lock()
                .map_err(|error| std::io::Error::other(error.to_string()))?;
            assert_eq!(
                cache.reports.files_len(),
                2,
                "the cache must contain only the two live source paths"
            );
            assert_eq!(cache.reports.reads, index + 2);
        }
        let (sender, receiver) = crate::channel::unbounded();
        let late_job =
            crate::enrichment::language_enrichment_job(root.clone(), sender, &measurements);
        measurements.retain_projects(|_| false);
        late_job();
        let message = receiver
            .recv_timeout(std::time::Duration::from_secs(2))
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        let BackgroundMsg::LanguageStatsBatch { entries } = message else {
            return Err(std::io::Error::other(
                "enrichment returned a different message",
            ));
        };
        assert_eq!(
            entries[0]
                .1
                .entries
                .iter()
                .map(|entry| entry.files)
                .sum::<usize>(),
            2
        );
        assert!(
            !measurements.has_language_owner_for_test(&root),
            "late enrichment must not restore a retired owner"
        );
        Ok(())
    }

    #[test]
    fn repeated_language_scans_reuse_unchanged_files_and_apply_ignore_changes()
    -> std::io::Result<()> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("lib.rs");
        std::fs::write(&source, "fn main() {}\n")?;
        std::fs::write(directory.path().join("ignored.rs"), "fn ignored() {}\n")?;
        std::fs::write(directory.path().join(".tokeignore"), "ignored.rs\n")?;
        let root = AbsolutePath::from(directory.path());
        let config = Config {
            hidden: Some(false),
            ..Config::default()
        };
        let mut cache = SourceFileMeasurements::default();
        let measured = collect_path_languages(&root, &config, &mut cache);
        let mut reference = Languages::new();
        reference.get_statistics(&[directory.path()], &[], &config);
        assert_eq!(measured.len(), reference.len());
        for (kind, language) in &reference {
            assert_eq!(measured[kind].code, language.code);
            assert_eq!(measured[kind].comments, language.comments);
            assert_eq!(measured[kind].blanks, language.blanks);
            assert_eq!(measured[kind].reports.len(), language.reports.len());
        }
        for _ in 0..16 {
            collect_path_languages(&root, &config, &mut cache);
        }
        assert_eq!(cache.reads, 1);
        assert_eq!(cache.reuses, 16);
        std::fs::write(&source, "fn main() {}\nfn added() {}\n")?;
        assert_eq!(
            collect_path_languages(&root, &config, &mut cache)[&LanguageType::Rust].code,
            2
        );
        assert_eq!(cache.reads, 2);
        std::fs::write(directory.path().join(".tokeignore"), "")?;
        assert_eq!(
            collect_path_languages(&root, &config, &mut cache)[&LanguageType::Rust]
                .reports
                .len(),
            2
        );
        assert_eq!(cache.reads, 3);
        Ok(())
    }
}
