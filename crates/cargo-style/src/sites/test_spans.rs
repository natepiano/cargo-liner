//! Find test code by file path or by closed bodies after test attributes.

use std::ops::RangeInclusive;

use super::constants::CFG_TEST;
use super::item_scan::BodyClose;
use super::item_scan::FollowingBody;
use super::item_scan::ItemScanner;
use super::item_scan::Position;

pub(super) enum TestSpans {
    WholeFile,
    Inline(Vec<RangeInclusive<usize>>),
}

impl TestSpans {
    pub(super) fn inline(scanner: &ItemScanner, lines: &[&str]) -> Self {
        Self::Inline(
            lines
                .iter()
                .enumerate()
                .filter_map(|(line, text)| {
                    let column = text.find(CFG_TEST)? + CFG_TEST.len();
                    let start = Position { line, column };
                    let FollowingBody::Present(BodyClose::ClosedAt(close)) =
                        scanner.body_after(lines, start)
                    else {
                        return None;
                    };
                    Some(line + 1..=close.line + 1)
                })
                .collect(),
        )
    }

    pub(super) fn contains(&self, line: usize) -> bool {
        match self {
            Self::WholeFile => true,
            Self::Inline(spans) => spans.iter().any(|span| span.contains(&line)),
        }
    }
}

pub(super) fn is_test_file(path: &str) -> bool {
    path.ends_with("tests.rs") || path.starts_with("tests/") || path.contains("/tests/")
}
