//! Check approved name mentions at aligned gaps between code tokens.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::ops::Range;

use super::patterns::BindingScope;
use super::patterns::PatternKind;
use super::scan::BindingScan;
use crate::rename_admission::BindingRename;
use crate::tokenizer::TokenKind;
use crate::word_syntax;

pub(super) struct CommentPlan<'a> {
    indexes: BTreeSet<usize>,
    edits:   Vec<CommentEdit<'a>>,
}

struct CommentEdit<'a> {
    index:       usize,
    span:        Range<usize>,
    replacement: &'a str,
}

pub(super) struct ProseMention;

impl<'a> CommentPlan<'a> {
    pub(super) fn read(
        scan: &BindingScan,
        scope: &BindingScope,
        rename: &BindingRename<'a>,
    ) -> Result<Self, ProseMention> {
        let docs = if scope.kind == PatternKind::FunctionParameter {
            function_docs(scan, scope.declaration)
        } else {
            None
        };
        let mut indexes = BTreeSet::new();
        let mut edits = Vec::new();
        for (index, comment) in scan.comments.iter().enumerate() {
            let gap = comment.token_position;
            let in_scope = scope
                .ranges
                .iter()
                .any(|range| range.contains(&gap) || gap == range.end && scan.word(gap - 1) != "}");
            if !in_scope && !docs.as_ref().is_some_and(|range| range.contains(&gap)) {
                continue;
            }
            let text = scan.chars[comment.start..comment.end]
                .iter()
                .collect::<String>();
            if !in_scope && !outer_doc(&text) {
                continue;
            }
            indexes.insert(index);
            let code = code_spans(&text);
            for span in identifier_mentions(&text, rename.old) {
                let link = text.get(span.start.saturating_sub(1)..span.start) == Some("[")
                    && text.as_bytes().get(span.end) == Some(&b']')
                    || text.get(span.start.saturating_sub(2)..span.start) == Some("](")
                        && text.as_bytes().get(span.end) == Some(&b')');
                if !link
                    && !code
                        .iter()
                        .any(|code| code.start <= span.start && span.end <= code.end)
                {
                    return Err(ProseMention);
                }
                edits.push(CommentEdit {
                    index,
                    span,
                    replacement: rename.new,
                });
            }
        }
        Ok(Self { indexes, edits })
    }

    pub(super) fn agrees(&self, bad: &BTreeSet<usize>) -> bool { self.indexes.is_disjoint(bad) }
}

pub(super) fn mismatches<'a>(
    before: &BindingScan,
    after: &BindingScan,
    locations: &[usize],
    plans: impl Iterator<Item = &'a CommentPlan<'a>>,
) -> BTreeSet<usize> {
    if before.comments.len() != after.comments.len() {
        return (0..before.comments.len()).collect();
    }
    let mut edits: BTreeMap<_, BTreeMap<_, _>> = BTreeMap::new();
    for plan in plans {
        for edit in &plan.edits {
            let key = (edit.span.start, edit.span.end);
            let old = edits
                .entry(edit.index)
                .or_default()
                .insert(key, edit.replacement);
            if old.is_some_and(|text| text != edit.replacement) {
                return (0..before.comments.len()).collect();
            }
        }
    }
    before
        .comments
        .iter()
        .zip(&after.comments)
        .enumerate()
        .filter_map(|(index, (old, new))| {
            let gap = if old.token_position == before.words.len() {
                Some(after.words.len())
            } else {
                locations.get(old.token_position).copied()
            };
            let mut expected = before.chars[old.start..old.end].iter().collect::<String>();
            if let Some(edits) = edits.get(&index) {
                for (&(start, end), &replacement) in edits.iter().rev() {
                    expected.replace_range(start..end, replacement);
                }
            }
            (gap != Some(new.token_position)
                || !expected
                    .chars()
                    .eq(after.chars[new.start..new.end].iter().copied()))
            .then_some(index)
        })
        .collect()
}

fn function_docs(scan: &BindingScan, declaration: usize) -> Option<Range<usize>> {
    let function = (0..declaration).find(|&index| {
        if scan.word(index) != "fn" {
            return false;
        }
        let paren = scan
            .level(index + 2, scan.words.len(), true)
            .into_iter()
            .find(|&at| matches!(scan.word(at), "(" | "{" | ";"));
        paren.is_some_and(|paren| {
            paren < declaration && scan.ends.get(&paren).is_some_and(|&end| declaration < end)
        })
    })?;
    let mut prefix = word_syntax::attribute_owners(&scan.words)
        .into_iter()
        .find_map(|(start, owner)| (owner == function).then_some(start))
        .unwrap_or(function);
    let openings: BTreeMap<_, _> = scan
        .ends
        .iter()
        .map(|(&open, &close)| (close, open))
        .collect();
    while let Some(previous) = prefix.checked_sub(1) {
        if matches!(
            scan.word(previous),
            "pub" | "async" | "unsafe" | "const" | "extern"
        ) {
            prefix = previous;
        } else if scan.word(previous) == ")"
            && let Some(&open) = openings.get(&previous)
            && open > 0
            && scan.word(open - 1) == "pub"
        {
            prefix = open - 1;
        } else if previous > 0
            && scan.tokens[previous].kind == TokenKind::Literal
            && scan.word(previous - 1) == "extern"
        {
            prefix = previous - 1;
        } else {
            break;
        }
    }
    Some(prefix..function + 1)
}

fn outer_doc(text: &str) -> bool {
    text.starts_with("///") && !text.starts_with("////")
        || text.starts_with("/**") && !text.starts_with("/***") && !text.starts_with("/**/")
}

fn code_spans(text: &str) -> Vec<Range<usize>> {
    let mut spans = Vec::new();
    let mut cursor = 0;
    while let Some(offset) = text[cursor..].find('\x60') {
        let start = cursor + offset;
        let run = text[start..]
            .bytes()
            .take_while(|&byte| byte == b'\x60')
            .count();
        let span = (1..=run).rev().find_map(|length| {
            let end = start + length;
            text[end..]
                .find(&text[start..end])
                .map(|closing| (end..end + closing, length))
        });
        if let Some((span, length)) = span {
            cursor = span.end + length;
            spans.push(span);
        } else {
            cursor = start + run;
        }
    }
    spans
}

fn identifier_mentions<'a>(
    text: &'a str,
    name: &'a str,
) -> impl Iterator<Item = Range<usize>> + 'a {
    text.match_indices(name).filter_map(|(start, matched)| {
        let end = start + matched.len();
        let bytes = text.as_bytes();
        let left = &bytes[..start];
        let right = &bytes[end..];
        let word = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'_';
        let join = |byte: u8| matches!(byte, b'-' | b'.' | b'/' | b'\\');
        let left_part = left
            .last()
            .is_some_and(|&byte| word(byte) || matches!(byte, b'-' | b'/' | b'\\'))
            || left.ends_with(b"::")
            || matches!(left, [.., word_byte, mark] if word(*word_byte) && join(*mark));
        let right_part = right.first().is_some_and(|&byte| word(byte))
            || right.starts_with(b"::")
            || matches!(right, [mark, word_byte, ..] if join(*mark) && word(*word_byte));
        (!left_part && !right_part).then_some(start..end)
    })
}
