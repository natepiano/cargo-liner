use std::fs;
use std::path::Path;
use std::path::PathBuf;

use pulldown_cmark::CodeBlockKind;
use pulldown_cmark::Event;
use pulldown_cmark::Parser;
use pulldown_cmark::Tag;
use walkdir::DirEntry;
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

/// Count tests for a single project root: `#[test]`-family functions
/// bucketed into unit (under `src/`) and integration (under `tests/`),
/// plus rustdoc doctests (code fences in doc comments under `src/`).
/// Reads only those two directories, never the whole tree, so it skips
/// `target/` and nested member crates (which are scanned as their own
/// roots). Doctests live only with the library source, so `tests/` is
/// scanned for attributes alone.
pub(crate) fn collect_test_counts_single(root: &Path) -> TestCounts {
    let src = count_src_dir(&root.join("src"));
    TestCounts {
        unit:        src.attributes,
        integration: count_attribute_dir(&root.join("tests")),
        doc:         src.doc.runnable,
        doc_ignored: src.doc.ignored,
    }
}

/// `#[test]`-family attribute count and doctest counts for one `src/`
/// directory, accumulated from a single read of each `.rs` file.
struct SrcCounts {
    attributes: usize,
    doc:        DocCounts,
}

fn count_src_dir(dir: &Path) -> SrcCounts {
    rust_files(dir)
        .filter_map(|path| fs::read_to_string(path).ok())
        .fold(
            SrcCounts {
                attributes: 0,
                doc:        DocCounts::default(),
            },
            |acc, source| SrcCounts {
                attributes: acc.attributes + count_test_attributes(&source),
                doc:        acc.doc.merged(count_doctests(&source)),
            },
        )
}

fn count_attribute_dir(dir: &Path) -> usize {
    rust_files(dir)
        .filter_map(|path| fs::read_to_string(path).ok())
        .map(|source| count_test_attributes(&source))
        .sum()
}

fn rust_files(dir: &Path) -> impl Iterator<Item = PathBuf> {
    WalkDir::new(dir)
        .into_iter()
        .flatten()
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "rs"))
        .map(DirEntry::into_path)
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

/// Runnable and ignored doctest counts.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
struct DocCounts {
    /// Code fences rustdoc would compile and run.
    runnable: usize,
    /// `ignore`-tagged fences rustdoc registers but skips.
    ignored:  usize,
}

impl DocCounts {
    const fn merged(self, other: Self) -> Self {
        Self {
            runnable: self.runnable + other.runnable,
            ignored:  self.ignored + other.ignored,
        }
    }
}

/// Count rustdoc doctests in one source file: code fences (and indented
/// code blocks) inside `///`, `//!`, `/** */`, or `/*! */` doc comments.
/// Runnable fences and `ignore`-tagged fences are tallied separately;
/// `text` and foreign-language fences count as neither. A heuristic — it
/// groups contiguous doc-comment lines into markdown documents and parses
/// each, so it never sees the item boundaries that rustdoc keys off.
fn count_doctests(source: &str) -> DocCounts {
    doc_comment_blocks(source)
        .iter()
        .map(|markdown| count_doctest_fences(markdown))
        .fold(DocCounts::default(), DocCounts::merged)
}

/// Split a source file into the markdown text of each doc comment: a run
/// of contiguous `///` / `//!` lines becomes one document, and each
/// `/** */` / `/*! */` block comment becomes its own. Keeping them
/// separate stops an unterminated fence in one comment from swallowing
/// the next.
fn doc_comment_blocks(source: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut line_run: Vec<String> = Vec::new();
    let mut block: Option<Vec<String>> = None;

    for raw in source.lines() {
        if let Some(acc) = block.as_mut() {
            let content = raw.trim_start();
            if let Some(end) = content.find("*/") {
                acc.push(strip_block_star(&content[..end]).to_string());
                blocks.push(acc.join("\n"));
                block = None;
            } else {
                acc.push(strip_block_star(content).to_string());
            }
            continue;
        }

        let trimmed = raw.trim_start();
        if let Some(content) = line_doc_content(trimmed) {
            line_run.push(content.to_string());
            continue;
        }
        if !line_run.is_empty() {
            blocks.push(line_run.join("\n"));
            line_run.clear();
        }
        if let Some(after) = block_doc_open(trimmed) {
            match after.find("*/") {
                Some(end) => blocks.push(strip_block_star(&after[..end]).to_string()),
                None => block = Some(vec![strip_block_star(after).to_string()]),
            }
        }
    }

    if !line_run.is_empty() {
        blocks.push(line_run.join("\n"));
    }
    if let Some(acc) = block {
        blocks.push(acc.join("\n"));
    }
    blocks
}

/// Markdown content of a `///` or `//!` line doc comment, with the one
/// leading space rustdoc strips removed. `None` for any other line —
/// including `////…`, which is a regular comment, not doc.
fn line_doc_content(trimmed: &str) -> Option<&str> {
    for prefix in ["///", "//!"] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            if prefix == "///" && rest.starts_with('/') {
                return None;
            }
            return Some(rest.strip_prefix(' ').unwrap_or(rest));
        }
    }
    None
}

/// Content after a `/**` or `/*!` block-doc opener on its first line.
/// `None` for any other line — including `/***…`, a regular block comment.
fn block_doc_open(trimmed: &str) -> Option<&str> {
    for prefix in ["/**", "/*!"] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            if prefix == "/**" && rest.starts_with('*') {
                return None;
            }
            return Some(rest);
        }
    }
    None
}

/// Strip the leading ` * ` decoration block doc comments conventionally
/// carry on each line, so a ` * ```rust` line reaches the parser as a
/// column-zero fence.
fn strip_block_star(line: &str) -> &str {
    let trimmed = line.trim_start();
    trimmed
        .strip_prefix('*')
        .map_or(trimmed, |rest| rest.strip_prefix(' ').unwrap_or(rest))
}

/// Tally the code blocks in one doc-comment markdown document by how
/// rustdoc would treat them.
fn count_doctest_fences(markdown: &str) -> DocCounts {
    let mut counts = DocCounts::default();
    for event in Parser::new(markdown) {
        if let Event::Start(Tag::CodeBlock(kind)) = event {
            match classify_code_block(&kind) {
                FenceKind::Runnable => counts.runnable += 1,
                FenceKind::Ignored => counts.ignored += 1,
                FenceKind::NotDoctest => {},
            }
        }
    }
    counts
}

/// How rustdoc treats a code block in a doc comment.
enum FenceKind {
    /// Compiled and run.
    Runnable,
    /// `ignore`-tagged: registered as a test but skipped.
    Ignored,
    /// `text` or a foreign-language tag — not a doctest at all.
    NotDoctest,
}

fn classify_code_block(kind: &CodeBlockKind) -> FenceKind {
    match kind {
        CodeBlockKind::Indented => FenceKind::Runnable,
        CodeBlockKind::Fenced(info) => classify_fence(info),
    }
}

/// Classify a fenced block from its info string. Empty info or only
/// rustdoc directives (`no_run`, `should_panic`, `compile_fail`,
/// `editionXXXX`, …) is runnable; an `ignore` token makes it ignored;
/// any other language tag (`text`, `bash`, …) makes it not a doctest.
fn classify_fence(info: &str) -> FenceKind {
    let mut ignored = false;
    for token in info.split([',', ' ', '\t']).filter(|t| !t.is_empty()) {
        if token == "ignore" || token.starts_with("ignore-") {
            ignored = true;
        } else if !is_rust_directive(token) {
            return FenceKind::NotDoctest;
        }
    }
    if ignored {
        FenceKind::Ignored
    } else {
        FenceKind::Runnable
    }
}

fn is_rust_directive(token: &str) -> bool {
    matches!(
        token,
        "rust"
            | "rs"
            | "should_panic"
            | "no_run"
            | "compile_fail"
            | "standalone_crate"
            | "test_harness"
    ) || token.starts_with("edition")
}

#[cfg(test)]
mod tests {
    use super::DocCounts;
    use super::count_doctests;
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

    #[test]
    fn plain_rust_fence_counts() {
        let source = "\
/// Example:
/// ```
/// let x = 1;
/// ```
pub fn f() {}
";
        assert_eq!(
            count_doctests(source),
            DocCounts {
                runnable: 1,
                ignored:  0,
            }
        );
    }

    #[test]
    fn rust_tagged_and_directive_fences_count() {
        let source = "\
/// ```rust
/// let x = 1;
/// ```
/// ```no_run
/// loop {}
/// ```
/// ```should_panic,edition2021
/// panic!()
/// ```
pub fn f() {}
";
        assert_eq!(
            count_doctests(source),
            DocCounts {
                runnable: 3,
                ignored:  0,
            }
        );
    }

    #[test]
    fn ignore_fence_counts_as_ignored_not_runnable() {
        let source = "\
/// ```ignore
/// not compiled
/// ```
/// ```rust,ignore
/// also skipped
/// ```
pub fn f() {}
";
        assert_eq!(
            count_doctests(source),
            DocCounts {
                runnable: 0,
                ignored:  2,
            }
        );
    }

    #[test]
    fn text_and_foreign_language_fences_count_as_neither() {
        let source = "\
/// ```text
/// plain output
/// ```
/// ```bash
/// echo hi
/// ```
pub fn f() {}
";
        assert_eq!(
            count_doctests(source),
            DocCounts {
                runnable: 0,
                ignored:  0,
            }
        );
    }

    #[test]
    fn runnable_and_ignored_fences_tally_separately() {
        let source = "\
/// ```
/// let x = 1;
/// ```
/// ```ignore
/// skipped
/// ```
pub fn f() {}
";
        assert_eq!(
            count_doctests(source),
            DocCounts {
                runnable: 1,
                ignored:  1,
            }
        );
    }

    #[test]
    fn inner_doc_comment_fence_counts() {
        let source = "\
//! Crate docs.
//! ```
//! let x = 1;
//! ```
";
        assert_eq!(
            count_doctests(source),
            DocCounts {
                runnable: 1,
                ignored:  0,
            }
        );
    }

    #[test]
    fn regular_comment_fence_does_not_count() {
        let source = "\
// ```
// let x = 1;
// ```
pub fn f() {}
";
        assert_eq!(count_doctests(source), DocCounts::default());
    }

    #[test]
    fn block_doc_comment_fence_counts() {
        let source = "\
/**
 * Example:
 * ```
 * let x = 1;
 * ```
 */
pub fn f() {}
";
        assert_eq!(
            count_doctests(source),
            DocCounts {
                runnable: 1,
                ignored:  0,
            }
        );
    }

    #[test]
    fn separate_doc_comments_count_independently() {
        let source = "\
/// ```
/// let a = 1;
/// ```
pub fn a() {}

/// ```
/// let b = 2;
/// ```
pub fn b() {}
";
        assert_eq!(
            count_doctests(source),
            DocCounts {
                runnable: 2,
                ignored:  0,
            }
        );
    }
}
