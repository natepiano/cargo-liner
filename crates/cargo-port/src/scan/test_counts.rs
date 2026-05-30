use std::fs;
use std::path::Path;

use walkdir::WalkDir;

use super::BackgroundMsg;
use super::cargo_metadata::StreamingScanContext;
use super::disk_usage;
use super::disk_usage::DiskUsageTree;
use crate::project::AbsolutePath;
use crate::project::TestCounts;

/// `#[test]`-family attribute paths counted as test functions. Each entry
/// is the path as written between `#[` and the following `]` or `(`, so
/// `#[tokio::test]` matches `tokio::test` and `#[test_case(...)]` matches
/// `test_case`. `#[cfg(test)]` is deliberately absent — its attribute
/// path is `cfg`, not `test`, so it never matches.
const TEST_ATTRIBUTES: [&str; 6] = [
    "test",
    "tokio::test",
    "async_std::test",
    "rstest",
    "test_case",
    "googletest::test",
];

/// Spawn the initial bulk test-count scan, batched per disk-usage tree to
/// mirror the language-stats fan-out.
pub(super) fn spawn_initial_test_counts(
    scan_context: &StreamingScanContext,
    disk_entries: &[(String, AbsolutePath)],
) {
    for tree in disk_usage::group_disk_usage_trees(disk_entries) {
        spawn_test_counts_tree(scan_context, tree);
    }
}

fn spawn_test_counts_tree(scan_context: &StreamingScanContext, tree: DiskUsageTree) {
    let handle = scan_context.client.handle.clone();
    let tx = scan_context.tx.clone();

    handle.spawn(async move {
        let Ok(results) =
            tokio::task::spawn_blocking(move || collect_test_counts_for_tree(&tree)).await
        else {
            return;
        };
        if !results.is_empty() {
            let _ = tx.send(BackgroundMsg::TestCountsBatch { entries: results });
        }
    });
}

fn collect_test_counts_for_tree(tree: &DiskUsageTree) -> Vec<(AbsolutePath, TestCounts)> {
    tree.entries
        .iter()
        .map(|entry| (entry.clone(), collect_test_counts_single(entry.as_path())))
        .collect()
}

/// Count `#[test]`-family functions for a single project root, bucketed
/// into unit (under `src/`) and integration (under `tests/`). Reads only
/// those two directories, never the whole tree, so it skips `target/` and
/// nested member crates (which are scanned as their own roots).
pub(crate) fn collect_test_counts_single(root: &Path) -> TestCounts {
    TestCounts {
        unit:        count_dir(&root.join("src")),
        integration: count_dir(&root.join("tests")),
    }
}

fn count_dir(dir: &Path) -> usize {
    WalkDir::new(dir)
        .into_iter()
        .flatten()
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "rs"))
        .map(|entry| {
            fs::read_to_string(entry.path()).map_or(0, |source| count_test_attributes(&source))
        })
        .sum()
}

fn count_test_attributes(source: &str) -> usize {
    source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .map(count_in_line)
        .sum()
}

fn count_in_line(line: &str) -> usize {
    let mut count = 0;
    let mut rest = line;
    while let Some(pos) = rest.find("#[") {
        rest = &rest[pos + 2..];
        if TEST_ATTRIBUTES.contains(&attr_path(rest)) {
            count += 1;
        }
    }
    count
}

/// Read the attribute path right after `#[` — the run of identifier
/// characters and `::` separators, ignoring leading whitespace. Stops at
/// the first delimiter (`(`, `]`, comma, etc.).
fn attr_path(after_open: &str) -> &str {
    let trimmed = after_open.trim_start();
    let end = trimmed
        .find(|c: char| !(c.is_alphanumeric() || c == '_' || c == ':'))
        .unwrap_or(trimmed.len());
    &trimmed[..end]
}

#[cfg(test)]
mod tests {
    use super::count_test_attributes;

    #[test]
    fn counts_plain_and_qualified_test_attributes() {
        let source = "\
#[test]
fn a() {}
#[tokio::test]
async fn b() {}
#[test_case(1, 2)]
fn c() {}
";
        assert_eq!(count_test_attributes(source), 3);
    }

    #[test]
    fn cfg_test_and_derive_do_not_count() {
        let source = "\
#[cfg(test)]
mod inner {
    #[derive(Debug)]
    struct S;
}
";
        assert_eq!(count_test_attributes(source), 0);
    }

    #[test]
    fn commented_out_attribute_does_not_count() {
        assert_eq!(count_test_attributes("// #[test]\n#[test]\nfn a() {}"), 1);
    }

    #[test]
    fn rstest_and_async_std_count() {
        assert_eq!(
            count_test_attributes("#[rstest]\nfn a() {}\n#[async_std::test]\nasync fn b() {}"),
            2
        );
    }
}
