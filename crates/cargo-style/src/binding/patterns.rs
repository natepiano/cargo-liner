//! Locate local declarations and the token ranges where each declaration is visible.

use std::collections::BTreeSet;
use std::ops::Range;

use super::scan;
use super::scan::BindingScan;
use crate::rename_admission::BindingRename;
use crate::rename_admission::RefusalCause;
use crate::word_syntax;

#[derive(Clone, Copy)]
pub(super) enum IdentifierMatch {
    WithoutRawPrefix,
    ExactPlainName,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum PatternKind {
    Let,
    ConditionLet,
    FunctionParameter,
    Closure,
    For,
    MatchArm,
}

#[derive(PartialEq, Eq)]
pub(super) struct PatternSite {
    pub(super) kind:    PatternKind,
    pub(super) pattern: Range<usize>,
    pub(super) ranges:  Vec<Range<usize>>,
}

pub(crate) struct BindingScope {
    pub(super) kind:        PatternKind,
    pub(super) declaration: usize,
    pub(super) ranges:      Vec<Range<usize>>,
}

impl PatternSite {
    fn new(kind: PatternKind, pattern: Range<usize>, body: Range<usize>) -> Self {
        Self {
            kind,
            ranges: vec![pattern.clone(), body],
            pattern,
        }
    }
}

impl BindingScan {
    pub(super) fn pattern_sites(&self) -> Vec<PatternSite> {
        let mut sites = Vec::new();
        let mut closure_ends = BTreeSet::new();
        for (index, word) in self.words.iter().enumerate() {
            if self
                .macros
                .iter()
                .any(|&(start, end)| (start..end).contains(&index))
            {
                continue;
            }
            let end = self.block_end(index);
            let lead = self.lead(index);
            match word.as_str() {
                "let" if matches!(lead, "{" | "}" | ";" | "]" | "if" | "while" | "&&") => {
                    sites.extend(self.let_site(index, end));
                },
                "fn" if word_syntax::identifier(self.word(index + 1)) => {
                    self.function_parameters(index, end, &mut sites);
                },
                "|" if !closure_ends.contains(&index)
                    && matches!(
                        lead,
                        "(" | "["
                            | "{"
                            | ","
                            | ";"
                            | "="
                            | "=>"
                            | "move"
                            | "return"
                            | "&&"
                            | "||"
                            | "!"
                            | ":"
                            | "async"
                            | "else"
                    ) =>
                {
                    self.closure_parameters(index, end, &mut closure_ends, &mut sites);
                },
                "for"
                    if matches!(
                        lead,
                        "{" | "}"
                            | ";"
                            | "="
                            | "=>"
                            | "("
                            | ","
                            | ":"
                            | "else"
                            | "]"
                            | "return"
                            | "break"
                    ) =>
                {
                    sites.extend(self.for_site(index, end));
                },
                "match" => self.match_arms(index, end, &mut sites),
                _ => {},
            }
        }
        sites
    }

    fn parameters(
        &self,
        start: usize,
        end: usize,
        kind: PatternKind,
        body: &Range<usize>,
        sites: &mut Vec<PatternSite>,
    ) {
        let mut begin = start;
        let separators = self
            .level(start, end, true)
            .into_iter()
            .filter(|&index| self.word(index) == ",")
            .chain(std::iter::once(end));
        for finish in separators {
            let colon = self.stop(begin, finish, &[":"]);
            if begin < colon && !(begin..colon).any(|i| self.word(i) == "self") {
                sites.push(PatternSite::new(kind, begin..colon, body.clone()));
            }
            begin = finish + 1;
        }
    }

    fn let_site(&self, index: usize, end: usize) -> Option<PatternSite> {
        let pattern_end = self.stop(index + 1, end, &["=", ";", ":", "else"]);
        if pattern_end == end {
            return None;
        }
        let finish = if self.word(pattern_end) == ":" {
            self.stop(pattern_end + 1, end, &["=", ";"])
        } else {
            pattern_end
        };
        let pattern = index + 1..pattern_end;
        if !matches!(self.lead(index), "if" | "while" | "&&") {
            let semi = self.stop(finish, end, &[";"]);
            return (semi < end)
                .then(|| PatternSite::new(PatternKind::Let, pattern, semi + 1..end));
        }
        let body = self.stop(finish + 1, end, &["&&", "{", "=>", ";"]);
        if body <= finish + 1
            || body >= end
            || !matches!(self.word(body), "&&" | "{")
            || self.level(finish + 1, body, false).into_iter().any(|i| {
                matches!(
                    self.word(i),
                    "if" | "match" | "loop" | "while" | "for" | "async" | "unsafe" | "|"
                )
            })
        {
            return None;
        }
        let block = self.stop(body, end, &["{", "=>", ";"]);
        if self.word(block) != "{" {
            return None;
        }
        self.ends
            .get(&block)
            .map(|&close| PatternSite::new(PatternKind::ConditionLet, pattern, body..close + 1))
    }

    fn function_parameters(&self, index: usize, end: usize, sites: &mut Vec<PatternSite>) {
        let paren = self
            .level(index + 2, end, true)
            .into_iter()
            .find(|&i| matches!(self.word(i), "(" | "{" | ";"))
            .unwrap_or(end);
        if self.word(paren) != "(" {
            return;
        }
        if let Some(&close) = self.ends.get(&paren) {
            let body = self.stop(close + 1, end, &["{", ";"]);
            if self.word(body) == "{"
                && let Some(&finish) = self.ends.get(&body)
            {
                self.parameters(
                    paren + 1,
                    close,
                    PatternKind::FunctionParameter,
                    &(body..finish + 1),
                    sites,
                );
            }
        }
    }

    fn closure_parameters(
        &self,
        index: usize,
        end: usize,
        closure_ends: &mut BTreeSet<usize>,
        sites: &mut Vec<PatternSite>,
    ) {
        let close = self.stop(index + 1, end, &["|", ";", "=>"]);
        if close == end || self.word(close) != "|" {
            return;
        }
        closure_ends.insert(close);
        let mut body = close + 1;
        if self.word(body) == "->" {
            body = self.stop(body + 1, end, &["{", ";", "=>"]);
        }
        if body >= end {
            return;
        }
        let end = self
            .parents
            .get(&index)
            .map_or(end, |parent| end.min(self.ends[parent]));
        let finish = match self.ends.get(&body) {
            Some(close) if self.word(body) == "{" => close + 1,
            _ => self.stop(body, end, &[",", ";", "=>"]),
        };
        if self.word(finish) != "=>" {
            self.parameters(
                index + 1,
                close,
                PatternKind::Closure,
                &(body..finish),
                sites,
            );
        }
    }

    fn for_site(&self, index: usize, end: usize) -> Option<PatternSite> {
        if self.word(index + 1) == "<" {
            return None;
        }
        let finish = self.stop(index + 1, end, &["in", ";", "{"]);
        if finish == end || self.word(finish) != "in" {
            return None;
        }
        let body = self.stop(finish + 1, end, &["{", ";", "=>"]);
        if self.word(body) != "{" {
            return None;
        }
        self.ends
            .get(&body)
            .map(|&close| PatternSite::new(PatternKind::For, index + 1..finish, body..close + 1))
    }

    fn match_arms(&self, index: usize, end: usize, sites: &mut Vec<PatternSite>) {
        let body = self.stop(index + 1, end, &["{", ";"]);
        let Some(&finish) = self.ends.get(&body).filter(|_| self.word(body) == "{") else {
            return;
        };
        let mut begin = body + 1;
        while begin < finish {
            let arrow = self.stop(begin, finish, &["=>", ","]);
            if arrow == finish || self.word(arrow) != "=>" {
                return;
            }
            let guard = self.stop(begin, arrow, &["if"]);
            let arm = arrow + 1;
            let end = match self.ends.get(&arm) {
                Some(close) if self.word(arm) == "{" => close + 1,
                _ => self.stop(arm, finish, &[","]),
            };
            sites.push(PatternSite {
                kind:    PatternKind::MatchArm,
                pattern: begin..guard,
                ranges:  std::iter::once(begin..end).collect(),
            });
            begin = end + usize::from(self.word(end) == ",");
        }
    }

    pub(super) fn binding_word(
        &self,
        site: &PatternSite,
        index: usize,
        name: &str,
        mode: IdentifierMatch,
    ) -> bool {
        if !site.pattern.contains(&index) || !scan::local_name(name) {
            return false;
        }
        let word = self.word(index);
        let matches = match mode {
            IdentifierMatch::WithoutRawPrefix => scan::plain(word) == scan::plain(name),
            IdentifierMatch::ExactPlainName => !name.starts_with("r#") && word == name,
        };
        let following = self.word(index + 1);
        matches
            && !matches!(self.lead(index), "." | "::" | "'")
            && !matches!(following, "::" | "(" | "{" | "!")
            && (following != ":"
                || self.parents.get(&index) == self.parents.get(&site.pattern.start))
    }

    pub(super) fn scope_at(&self, declaration: usize) -> Result<BindingScope, RefusalCause> {
        let name = self.word(declaration);
        let site = self
            .sites
            .iter()
            .filter(|site| {
                self.binding_word(site, declaration, name, IdentifierMatch::WithoutRawPrefix)
            })
            .min_by_key(|site| site.pattern.len())
            .ok_or(RefusalCause::BindingScopeMissing)?;
        self.check_scrutinee(site)?;
        let shadowed = self
            .sites
            .iter()
            .filter(|other| {
                *other != site
                    && scan::contains(&site.ranges, other.pattern.start)
                    && other.pattern.clone().any(|i| {
                        self.binding_word(other, i, name, IdentifierMatch::WithoutRawPrefix)
                    })
            })
            .flat_map(|other| other.ranges.iter());
        let hidden_items = self
            .named_items
            .iter()
            .filter(|item| scan::contains(&site.ranges, item.start));
        let ranges = shadowed
            .chain(hidden_items)
            .fold(site.ranges.clone(), |ranges, hidden| {
                ranges
                    .into_iter()
                    .flat_map(|range| {
                        [
                            range.start..range.end.min(hidden.start),
                            range.start.max(hidden.end)..range.end,
                        ]
                    })
                    .filter(|range| !range.is_empty())
                    .collect()
            });
        Ok(BindingScope {
            kind: site.kind,
            declaration,
            ranges,
        })
    }

    fn check_scrutinee(&self, site: &PatternSite) -> Result<(), RefusalCause> {
        if matches!(site.kind, PatternKind::Let | PatternKind::ConditionLet) {
            let end = self.block_end(site.pattern.start);
            let scrutinee = self.stop(site.pattern.end, end, &["="]) + 1;
            let finish = if site.kind == PatternKind::ConditionLet {
                self.level(scrutinee, end, true)
                    .into_iter()
                    .find(|&i| matches!(self.word(i), "&&" | "{" | ";" | "=>"))
                    .unwrap_or(end)
            } else {
                let finish = self.stop(scrutinee, end, &["else", ";"]);
                if self.word(finish) == "else" {
                    finish
                } else {
                    scrutinee
                }
            };
            if (scrutinee..finish).any(|i| self.word(i) == "{") {
                return Err(RefusalCause::ConditionBlock);
            }
        }
        if site.kind == PatternKind::For {
            let iterator = site.pattern.end + 1;
            let body = &site.ranges[1];
            if body.start == iterator
                || (iterator..body.start).any(|i| {
                    matches!(
                        self.word(i),
                        "{" | "if" | "match" | "loop" | "while" | "for" | "async" | "unsafe"
                    )
                })
                || matches!(
                    self.word(body.end),
                    "else" | "{" | "." | "?" | "(" | "[" | "+" | "-" | "*" | "/" | "|" | "&"
                )
            {
                return Err(RefusalCause::IteratorBlock);
            }
        }
        Ok(())
    }

    pub(super) fn unread_binding(&self, scope: &BindingScope, rename: &BindingRename<'_>) -> bool {
        let parsed: BTreeSet<_> = self
            .sites
            .iter()
            .map(|site| site.pattern.start - 1)
            .collect();
        if self.words.iter().enumerate().any(|(i, word)| {
            word == "let" && scan::contains(&scope.ranges, i) && !parsed.contains(&i)
        }) {
            return true;
        }
        for &(bang, finish) in &self.macros {
            if !scope
                .ranges
                .iter()
                .any(|range| range.start < finish && bang < range.end)
            {
                continue;
            }
            if (bang..finish).any(|i| matches!(self.word(i), "let" | "fn" | "for" | "|")) {
                return true;
            }
            if self.lead(bang) == "matches" {
                let comma = self.stop(bang + 2, finish - 1, &[","]);
                let guard = self.stop(comma + 1, finish - 1, &["if"]);
                if (comma + 1..guard).any(|i| [rename.old, rename.new].contains(&self.word(i))) {
                    return true;
                }
            }
        }
        false
    }
}
