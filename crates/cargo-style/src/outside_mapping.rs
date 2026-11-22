//! Map the source coordinate of an outside rename.

use std::collections::HashMap;
use std::ops::Range;

use serde::Serialize;

use crate::constants::AUTOJUNK_MIN_LINES;
use crate::constants::AUTOJUNK_POPULARITY_DIVISOR;
use crate::declaration_source::DeclarationFile;
use crate::declarations::DeclarationAt;
use crate::same_declaration::Declaration;
use crate::source_lines;
use crate::source_sites;

#[derive(Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum MappedSite {
    MappedDeclaration {
        #[serde(flatten)]
        site: MappedDeclaration,
    },
    MappedNoDeclaration {
        path: String,
        line: usize,
    },
    UnmappedSite {
        reason: MappingFailure,
    },
}

#[derive(Clone, Serialize)]
pub(crate) struct MappedDeclaration {
    path:                   String,
    pub(crate) line:        usize,
    pub(crate) declaration: Declaration,
}

#[derive(Clone, Copy, Serialize)]
pub(crate) enum MappingFailure {
    #[serde(rename = "source site deleted")]
    Deleted,
    #[serde(rename = "source mapping unreadable")]
    Unreadable,
    #[serde(rename = "several declarations own mapped site")]
    Ambiguous,
    #[serde(rename = "mapped site outside destination")]
    Outside,
}

pub(crate) enum OutsidePatch {
    Hunks(Vec<LineHunk>),
    Unreadable,
}

pub(crate) struct LineHunk {
    old: Range<usize>,
    new: Range<usize>,
}

impl OutsidePatch {
    pub(crate) fn from_myers(diff: &str) -> Self {
        let hunks: Vec<_> = diff.split('\n').filter_map(LineHunk::from_header).collect();
        if hunks.is_empty() && !source_sites::trim(diff).is_empty() {
            Self::Unreadable
        } else {
            Self::Hunks(hunks)
        }
    }

    pub(crate) fn from_normalized_lines(before: &DeclarationFile, after: &DeclarationFile) -> Self {
        let normalize = |file: &DeclarationFile| {
            let mut text = file.normalized_text().to_owned();
            text.push('\n');
            text
        };
        let before = normalize(before);
        let after = normalize(after);
        let before: Vec<_> = source_lines::split_lines(&before).collect();
        let after: Vec<_> = source_lines::split_lines(&after).collect();
        Self::Hunks(matching_hunks(&before, &after))
    }

    pub(crate) fn map(&self, path: &str, line: usize, destination: &DeclarationFile) -> MappedSite {
        let target = match self.coordinate(line, destination.line_count()) {
            Ok(target) => target,
            Err(reason) => return MappedSite::UnmappedSite { reason },
        };
        let DeclarationAt::Named(item) = destination.at(target) else {
            return MappedSite::MappedNoDeclaration {
                path: path.to_owned(),
                line: target,
            };
        };
        if destination.items().iter().any(|other| {
            other != item
                && (target == *other.lines().start()
                    || target == other.keyword_line()
                    || target == *other.lines().end())
        }) {
            return MappedSite::UnmappedSite {
                reason: MappingFailure::Ambiguous,
            };
        }
        MappedSite::MappedDeclaration {
            site: MappedDeclaration {
                path:        path.to_owned(),
                line:        target,
                declaration: item.into(),
            },
        }
    }

    fn coordinate(&self, line: usize, line_count: usize) -> Result<usize, MappingFailure> {
        if line == 0 {
            return Err(MappingFailure::Outside);
        }
        let Self::Hunks(hunks) = self else {
            return Err(MappingFailure::Unreadable);
        };
        let mut offset = 0_i128;
        for hunk in hunks {
            if hunk.old.contains(&line) {
                if hunk.new.is_empty() {
                    return Err(MappingFailure::Deleted);
                }
                let mapped = hunk.new.start + (line - hunk.old.start).min(hunk.new.len() - 1);
                return in_source(mapped, line_count);
            }
            let past = if hunk.old.is_empty() {
                hunk.old.start < line
            } else {
                hunk.old.end <= line
            };
            if !past {
                break;
            }
            offset += hunk.new.len() as i128 - hunk.old.len() as i128;
        }
        let mapped = usize::try_from(line as i128 + offset).map_err(|_| MappingFailure::Outside)?;
        in_source(mapped, line_count)
    }
}

fn in_source(line: usize, line_count: usize) -> Result<usize, MappingFailure> {
    if (1..=line_count).contains(&line) {
        Ok(line)
    } else {
        Err(MappingFailure::Outside)
    }
}

impl LineHunk {
    fn from_header(line: &str) -> Option<Self> {
        let header = source_lines::patch_header(line)?;
        Some(Self {
            old: header.old.start..header.old.start.saturating_add(header.old.count),
            new: header.new.start..header.new.start.saturating_add(header.new.count),
        })
    }

    fn from_gap(old: Range<usize>, new: Range<usize>) -> Self {
        let old_start = old.start + usize::from(!old.is_empty());
        let new_start = new.start + usize::from(!new.is_empty());
        Self {
            old: old_start..old_start + old.len(),
            new: new_start..new_start + new.len(),
        }
    }
}

#[derive(Clone, Copy, Default)]
struct EqualRun {
    old:    usize,
    new:    usize,
    length: usize,
}

/// Match the equal runs from Python's `SequenceMatcher` with the default `autojunk` setting.
fn matching_hunks(before: &[&str], after: &[&str]) -> Vec<LineHunk> {
    let mut positions: HashMap<&str, Vec<usize>> = HashMap::new();
    for (index, &line) in after.iter().enumerate() {
        positions.entry(line).or_default().push(index);
    }
    if after.len() >= AUTOJUNK_MIN_LINES {
        let limit = after.len() / AUTOJUNK_POPULARITY_DIVISOR + 1;
        positions.retain(|_, indices| indices.len() <= limit);
    }
    let mut pending = vec![(0..before.len(), 0..after.len())];
    let mut equal = Vec::new();
    while let Some((old, new)) = pending.pop() {
        let run = longest_run(before, after, &positions, old.clone(), new.clone());
        if run.length == 0 {
            continue;
        }
        equal.push(run);
        if old.start < run.old && new.start < run.new {
            pending.push((old.start..run.old, new.start..run.new));
        }
        if run.old + run.length < old.end && run.new + run.length < new.end {
            pending.push((run.old + run.length..old.end, run.new + run.length..new.end));
        }
    }
    equal.sort_unstable_by_key(|run| (run.old, run.new));
    equal.push(EqualRun {
        old:    before.len(),
        new:    after.len(),
        length: 0,
    });
    let mut hunks = Vec::new();
    let mut old_end = 0;
    let mut new_end = 0;
    for run in equal {
        if old_end < run.old || new_end < run.new {
            hunks.push(LineHunk::from_gap(old_end..run.old, new_end..run.new));
        }
        old_end = run.old + run.length;
        new_end = run.new + run.length;
    }
    hunks
}

fn longest_run(
    before: &[&str],
    after: &[&str],
    positions: &HashMap<&str, Vec<usize>>,
    old: Range<usize>,
    new: Range<usize>,
) -> EqualRun {
    let mut best = EqualRun {
        old:    old.start,
        new:    new.start,
        length: 0,
    };
    let mut previous = HashMap::new();
    let mut current = HashMap::new();
    for i in old.clone() {
        if let Some(indices) = positions.get(before[i]) {
            let first = indices.partition_point(|&j| j < new.start);
            for &j in &indices[first..] {
                if j >= new.end {
                    break;
                }
                let length = previous.get(&j).copied().unwrap_or(0) + 1;
                current.insert(j + 1, length);
                if length > best.length {
                    best = EqualRun {
                        old: i + 1 - length,
                        new: j + 1 - length,
                        length,
                    };
                }
            }
        }
        std::mem::swap(&mut previous, &mut current);
        current.clear();
    }
    // Popular lines cannot seed a match. They can extend an equal run.
    while best.old > old.start
        && best.new > new.start
        && before[best.old - 1] == after[best.new - 1]
    {
        best.old -= 1;
        best.new -= 1;
        best.length += 1;
    }
    while best.old + best.length < old.end
        && best.new + best.length < new.end
        && before[best.old + best.length] == after[best.new + best.length]
    {
        best.length += 1;
    }
    best
}
