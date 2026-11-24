//! Compare the bodies of a source declaration and a candidate.

use crate::declaration_source::DeclarationFile;
use crate::declaration_source::NormalizedSlice;
use crate::declarations::DeclaredItem;
use crate::declarations::ItemBody;
use crate::name_sites::BodyTokenState;
use crate::name_sites::BodyTokens;
use crate::name_sites::NameSiteReading;
use crate::source_lines;
use crate::source_sites;

pub(super) enum ReadableBody {
    Lines(BodyLines),
    Unreadable,
}

pub(super) struct BodyLines {
    source:           Vec<char>,
    pub(super) count: usize,
}

pub(super) fn read_body(file: &DeclarationFile, item: &DeclaredItem) -> ReadableBody {
    let ItemBody::Braced(body) = file.body(item) else {
        return ReadableBody::Unreadable;
    };
    let facts = file.body_facts();
    let tokens = match &facts.tokens {
        BodyTokens::State(BodyTokenState::Unreadable) => return ReadableBody::Unreadable,
        BodyTokens::State(BodyTokenState::Original) => file.tokens(),
        BodyTokens::Tokens(tokens) => tokens,
    };
    let (Some(open), Some(close)) = (tokens.get(body.open), tokens.get(body.close)) else {
        return ReadableBody::Unreadable;
    };
    let start = open.start + open.value.chars().count();
    let NormalizedSlice::Text(body_text) = file.normalized_slice(start..close.start) else {
        return ReadableBody::Unreadable;
    };
    let lines: Vec<_> = source_lines::split_lines(body_text)
        .map(|line| line.strip_suffix('\n').unwrap_or(line))
        .filter(|line| !source_sites::trim(line).is_empty())
        .collect();
    ReadableBody::Lines(BodyLines {
        source: lines.join("\n").chars().collect(),
        count:  lines.len(),
    })
}

pub(super) fn preserved_lines(
    sites: &NameSiteReading,
    old: &BodyLines,
    new: &BodyLines,
    name: &str,
) -> usize {
    let NameSiteReading::Sites { sites } = sites else {
        return 0;
    };
    let mut replaced = String::new();
    let mut cursor = 0;
    for &[start, end] in sites {
        let Some(unchanged) = old.source.get(cursor..start) else {
            return 0;
        };
        replaced.extend(unchanged);
        replaced.push_str(name);
        cursor = end;
    }
    let Some(tail) = old.source.get(cursor..) else {
        return 0;
    };
    replaced.extend(tail);
    let source: String = new.source.iter().collect();
    let lines: Vec<_> = source.split('\n').collect();
    let mut counts = vec![0; lines.len() + 1];
    for line in replaced.split('\n') {
        let mut previous = 0;
        for (index, other) in lines.iter().enumerate() {
            let saved = counts[index + 1];
            counts[index + 1] = if line == *other {
                previous + 1
            } else {
                saved.max(counts[index])
            };
            previous = saved;
        }
    }
    counts[lines.len()]
}
