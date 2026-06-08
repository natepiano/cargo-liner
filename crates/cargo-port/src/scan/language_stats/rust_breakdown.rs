use std::fs;
use std::ops::Range;
use std::path::Component;
use std::path::Path;

use tokei::CodeStats;
use tokei::Config;
use tokei::Language;
use tokei::LanguageType;
use tokei::Report;

use crate::project::LangEntry;

const CODE_LABEL: &str = "code";
const UNIT_TESTS_LABEL: &str = "unit tests";
const INTEGRATION_TESTS_LABEL: &str = "integration tests";
const EXAMPLES_LABEL: &str = "examples";
const BENCHES_LABEL: &str = "benches";

pub(super) fn child_entries(root: &Path, language: &Language, config: &Config) -> Vec<LangEntry> {
    let mut buckets = RustBuckets::default();
    for report in &language.reports {
        let totals = LineTotals::from_report(report);
        match rust_file_bucket(root, report.name.as_path()) {
            RustBucket::Code => add_code_file_with_unit_split(&mut buckets, report, totals, config),
            bucket => buckets.add_file(bucket, totals),
        }
    }
    buckets.entries()
}

fn add_code_file_with_unit_split(
    buckets: &mut RustBuckets,
    report: &Report,
    totals: LineTotals,
    config: &Config,
) {
    let Some(unit) = unit_totals_for_file(report.name.as_path(), totals, config) else {
        buckets.add_file(RustBucket::Code, totals);
        return;
    };

    let code = totals.without(unit);
    buckets.add_file(RustBucket::UnitTests, unit);
    if !code.is_empty() {
        buckets.add_file(RustBucket::Code, code);
    }
}

fn unit_totals_for_file(path: &Path, totals: LineTotals, config: &Config) -> Option<LineTotals> {
    let source = fs::read_to_string(path).ok()?;
    let ranges = cfg_test_item_ranges(&source);
    if ranges.is_empty() {
        return None;
    }
    let unit = ranges
        .iter()
        .map(|range| {
            LineTotals::from_code_stats(
                &LanguageType::Rust.parse_from_str(&source[range.clone()], config),
            )
        })
        .fold(LineTotals::default(), |mut acc, totals| {
            acc.add(totals);
            acc
        })
        .capped_by(totals);
    (!unit.is_empty()).then_some(unit)
}

fn rust_file_bucket(root: &Path, path: &Path) -> RustBucket {
    let relative = path.strip_prefix(root).unwrap_or(path);
    let components = normal_components(relative);
    if components.iter().any(|part| *part == "examples") {
        RustBucket::Examples
    } else if components.iter().any(|part| *part == "benches") {
        RustBucket::Benches
    } else if is_src_unit_test_path(&components) {
        RustBucket::UnitTests
    } else if components.iter().any(|part| *part == "tests") {
        RustBucket::IntegrationTests
    } else {
        RustBucket::Code
    }
}

fn normal_components(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
            Component::Prefix(_)
            | Component::RootDir
            | Component::CurDir
            | Component::ParentDir => None,
        })
        .collect()
}

fn is_src_unit_test_path(components: &[String]) -> bool {
    let has_src_tests_dir = components.iter().enumerate().any(|(index, part)| {
        part == "tests"
            && components[..index]
                .iter()
                .any(|candidate| candidate == "src")
    });
    let is_tests_file = components.last().is_some_and(|file| file == "tests.rs")
        && components.iter().any(|part| part == "src");
    has_src_tests_dir || is_tests_file
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RustBucket {
    Code,
    UnitTests,
    IntegrationTests,
    Examples,
    Benches,
}

impl RustBucket {
    const ORDERED: [Self; 5] = [
        Self::Code,
        Self::UnitTests,
        Self::IntegrationTests,
        Self::Examples,
        Self::Benches,
    ];

    const fn label(self) -> &'static str {
        match self {
            Self::Code => CODE_LABEL,
            Self::UnitTests => UNIT_TESTS_LABEL,
            Self::IntegrationTests => INTEGRATION_TESTS_LABEL,
            Self::Examples => EXAMPLES_LABEL,
            Self::Benches => BENCHES_LABEL,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct LineTotals {
    code:     usize,
    comments: usize,
    blanks:   usize,
}

impl LineTotals {
    fn from_report(report: &Report) -> Self { Self::from_code_stats(&report.stats) }

    fn from_code_stats(stats: &CodeStats) -> Self {
        let stats = stats.summarise();
        Self {
            code:     stats.code,
            comments: stats.comments,
            blanks:   stats.blanks,
        }
    }

    const fn is_empty(self) -> bool { self.code == 0 && self.comments == 0 && self.blanks == 0 }

    const fn add(&mut self, other: Self) {
        self.code += other.code;
        self.comments += other.comments;
        self.blanks += other.blanks;
    }

    const fn capped_by(self, max: Self) -> Self {
        Self {
            code:     if self.code > max.code {
                max.code
            } else {
                self.code
            },
            comments: if self.comments > max.comments {
                max.comments
            } else {
                self.comments
            },
            blanks:   if self.blanks > max.blanks {
                max.blanks
            } else {
                self.blanks
            },
        }
    }

    const fn without(self, other: Self) -> Self {
        Self {
            code:     self.code.saturating_sub(other.code),
            comments: self.comments.saturating_sub(other.comments),
            blanks:   self.blanks.saturating_sub(other.blanks),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct BucketTotals {
    files: usize,
    lines: LineTotals,
}

impl BucketTotals {
    const fn is_empty(self) -> bool { self.files == 0 && self.lines.is_empty() }

    const fn add_file(&mut self, totals: LineTotals) {
        if totals.is_empty() {
            return;
        }
        self.files += 1;
        self.lines.add(totals);
    }

    fn entry(self, label: &'static str) -> Option<LangEntry> {
        (!self.is_empty()).then(|| LangEntry {
            language: label.to_string(),
            files:    self.files,
            code:     self.lines.code,
            comments: self.lines.comments,
            blanks:   self.lines.blanks,
            children: Vec::new(),
        })
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct RustBuckets {
    code:        BucketTotals,
    unit:        BucketTotals,
    integration: BucketTotals,
    examples:    BucketTotals,
    benches:     BucketTotals,
}

impl RustBuckets {
    const fn add_file(&mut self, bucket: RustBucket, totals: LineTotals) {
        self.bucket_mut(bucket).add_file(totals);
    }

    const fn bucket_mut(&mut self, bucket: RustBucket) -> &mut BucketTotals {
        match bucket {
            RustBucket::Code => &mut self.code,
            RustBucket::UnitTests => &mut self.unit,
            RustBucket::IntegrationTests => &mut self.integration,
            RustBucket::Examples => &mut self.examples,
            RustBucket::Benches => &mut self.benches,
        }
    }

    const fn bucket(self, bucket: RustBucket) -> BucketTotals {
        match bucket {
            RustBucket::Code => self.code,
            RustBucket::UnitTests => self.unit,
            RustBucket::IntegrationTests => self.integration,
            RustBucket::Examples => self.examples,
            RustBucket::Benches => self.benches,
        }
    }

    fn entries(self) -> Vec<LangEntry> {
        RustBucket::ORDERED
            .into_iter()
            .filter_map(|bucket| self.bucket(bucket).entry(bucket.label()))
            .collect()
    }
}

fn cfg_test_item_ranges(source: &str) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut offset = 0;
    for line in source.split_inclusive('\n') {
        if is_cfg_test_attr(line.trim_start()) {
            let search_start = offset + line.len();
            if let Some(end) = cfg_test_item_end(source, search_start) {
                ranges.push(offset..end);
            }
        }
        offset += line.len();
    }
    merge_ranges(ranges)
}

fn is_cfg_test_attr(trimmed: &str) -> bool {
    let Some(after_open) = trimmed.strip_prefix("#[") else {
        return false;
    };
    let Some(end) = after_open.find(']') else {
        return false;
    };
    let compact = after_open[..end]
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>();
    let Some(inner) = compact
        .strip_prefix("cfg(")
        .and_then(|value| value.strip_suffix(')'))
    else {
        return false;
    };
    inner
        .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
        .any(|token| token == "test")
}

fn cfg_test_item_end(source: &str, search_start: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut index = search_start;
    while index < bytes.len() {
        if starts_with(bytes, index, b"//") {
            index = skip_line_comment(bytes, index);
            continue;
        }
        if starts_with(bytes, index, b"/*") {
            index = skip_block_comment(bytes, index);
            continue;
        }
        if let Some(hash_count) = raw_string_hash_count(bytes, index) {
            index = skip_raw_string(bytes, index, hash_count);
            continue;
        }
        match bytes[index] {
            b'"' => index = skip_quoted_string(bytes, index),
            b'\'' => {
                index = char_literal_end(bytes, index).unwrap_or(index + 1);
            },
            b'{' => return matching_brace_end(source, index),
            b';' => return Some(index + 1),
            _ => index += 1,
        }
    }
    None
}

fn merge_ranges(mut ranges: Vec<Range<usize>>) -> Vec<Range<usize>> {
    ranges.sort_by_key(|range| range.start);
    let mut merged: Vec<Range<usize>> = Vec::new();
    for range in ranges {
        if let Some(last) = merged.last_mut()
            && range.start <= last.end
        {
            last.end = last.end.max(range.end);
            continue;
        }
        merged.push(range);
    }
    merged
}

fn matching_brace_end(source: &str, open: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut index = open;
    let mut depth = 0usize;
    while index < bytes.len() {
        if starts_with(bytes, index, b"//") {
            index = skip_line_comment(bytes, index);
            continue;
        }
        if starts_with(bytes, index, b"/*") {
            index = skip_block_comment(bytes, index);
            continue;
        }
        if let Some(hash_count) = raw_string_hash_count(bytes, index) {
            index = skip_raw_string(bytes, index, hash_count);
            continue;
        }
        match bytes[index] {
            b'"' => index = skip_quoted_string(bytes, index),
            b'\'' => {
                index = char_literal_end(bytes, index).unwrap_or(index + 1);
            },
            b'{' => {
                depth += 1;
                index += 1;
            },
            b'}' => {
                depth = depth.saturating_sub(1);
                index += 1;
                if depth == 0 {
                    return Some(index);
                }
            },
            _ => index += 1,
        }
    }
    None
}

fn starts_with(bytes: &[u8], index: usize, pattern: &[u8]) -> bool {
    bytes
        .get(index..index + pattern.len())
        .is_some_and(|candidate| candidate == pattern)
}

fn skip_line_comment(bytes: &[u8], mut index: usize) -> usize {
    while index < bytes.len() && bytes[index] != b'\n' {
        index += 1;
    }
    index
}

fn skip_block_comment(bytes: &[u8], mut index: usize) -> usize {
    let mut depth = 1usize;
    index += 2;
    while index < bytes.len() {
        if starts_with(bytes, index, b"/*") {
            depth += 1;
            index += 2;
        } else if starts_with(bytes, index, b"*/") {
            depth = depth.saturating_sub(1);
            index += 2;
            if depth == 0 {
                return index;
            }
        } else {
            index += 1;
        }
    }
    index
}

fn raw_string_hash_count(bytes: &[u8], index: usize) -> Option<usize> {
    if bytes.get(index) != Some(&b'r') {
        return None;
    }
    let mut cursor = index + 1;
    while bytes.get(cursor) == Some(&b'#') {
        cursor += 1;
    }
    (bytes.get(cursor) == Some(&b'"')).then_some(cursor - index - 1)
}

fn skip_raw_string(bytes: &[u8], index: usize, hash_count: usize) -> usize {
    let mut cursor = index + hash_count + 2;
    while cursor < bytes.len() {
        if bytes[cursor] == b'"' {
            let hash_start = cursor + 1;
            let hash_end = hash_start + hash_count;
            if bytes
                .get(hash_start..hash_end)
                .is_some_and(|hashes| hashes.iter().all(|byte| *byte == b'#'))
            {
                return hash_end;
            }
        }
        cursor += 1;
    }
    cursor
}

fn skip_quoted_string(bytes: &[u8], mut index: usize) -> usize {
    index += 1;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            b'"' => return index + 1,
            _ => index += 1,
        }
    }
    index
}

fn char_literal_end(bytes: &[u8], index: usize) -> Option<usize> {
    let mut cursor = index + 1;
    while cursor < bytes.len() && cursor <= index + 6 && bytes[cursor] != b'\n' {
        match bytes[cursor] {
            b'\\' => cursor += 2,
            b'\'' => return Some(cursor + 1),
            _ => cursor += 1,
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::fs;
    use std::io;
    use std::path::Path;

    use super::BENCHES_LABEL;
    use super::CODE_LABEL;
    use super::EXAMPLES_LABEL;
    use super::INTEGRATION_TESTS_LABEL;
    use super::UNIT_TESTS_LABEL;
    use crate::project::LangEntry;
    use crate::project::LanguageStats;
    use crate::scan::language_stats;

    type TestResult = Result<(), Box<dyn Error>>;

    fn write_file(root: &Path, relative: &str, contents: &str) -> io::Result<()> {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, contents)
    }

    fn rust_entry(stats: &LanguageStats) -> io::Result<&LangEntry> {
        stats
            .entries
            .iter()
            .find(|entry| entry.language == "Rust")
            .ok_or_else(|| io::Error::other("missing Rust entry"))
    }

    fn child_entry<'a>(entry: &'a LangEntry, label: &str) -> io::Result<&'a LangEntry> {
        entry
            .children
            .iter()
            .find(|child| child.language == label)
            .ok_or_else(|| io::Error::other(format!("missing `{label}` child")))
    }

    #[test]
    fn rust_breakdown_uses_requested_child_labels() -> TestResult {
        let dir = tempfile::tempdir()?;
        write_file(dir.path(), "src/lib.rs", "pub fn code() {}\n")?;
        write_file(dir.path(), "src/tests.rs", "#[test]\nfn unit_file() {}\n")?;
        write_file(
            dir.path(),
            "tests/integration.rs",
            "#[test]\nfn integration() {}\n",
        )?;
        write_file(dir.path(), "examples/demo.rs", "fn main() {}\n")?;
        write_file(dir.path(), "benches/bench.rs", "fn main() {}\n")?;

        let stats = language_stats::collect_language_stats_single(dir.path());
        let rust = rust_entry(&stats)?;
        let labels = rust
            .children
            .iter()
            .map(|entry| entry.language.as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            labels,
            vec![
                CODE_LABEL,
                UNIT_TESTS_LABEL,
                INTEGRATION_TESTS_LABEL,
                EXAMPLES_LABEL,
                BENCHES_LABEL,
            ]
        );
        assert_rust_children_add_to_parent(rust);
        Ok(())
    }

    #[test]
    fn inline_cfg_test_module_moves_loc_to_unit_tests() -> TestResult {
        let dir = tempfile::tempdir()?;
        write_file(
            dir.path(),
            "src/lib.rs",
            "\
pub fn production() -> usize {
    1
}

#[cfg(test)]
mod tests {
    #[test]
    fn unit() {
        let text = \"{still text}\";
        assert_eq!(text.len(), 12);
    }
}
",
        )?;

        let stats = language_stats::collect_language_stats_single(dir.path());
        let rust = rust_entry(&stats)?;
        let code = child_entry(rust, CODE_LABEL)?;
        let unit = child_entry(rust, UNIT_TESTS_LABEL)?;

        assert!(code.code > 0);
        assert!(unit.code > 0);
        assert_rust_children_add_to_parent(rust);
        Ok(())
    }

    fn assert_rust_children_add_to_parent(rust: &LangEntry) {
        let code: usize = rust.children.iter().map(|entry| entry.code).sum();
        let comments: usize = rust.children.iter().map(|entry| entry.comments).sum();
        let blanks: usize = rust.children.iter().map(|entry| entry.blanks).sum();
        assert_eq!(code, rust.code);
        assert_eq!(comments, rust.comments);
        assert_eq!(blanks, rust.blanks);
    }
}
