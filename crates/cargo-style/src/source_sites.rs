//! Compare the text, enclosing owners, and declarations of two source sites.

use std::cell::OnceCell;
use std::rc::Rc;

use crate::declarations::DeclarationAt;
use crate::declarations::DeclaredFile;
use crate::enclosing_items;
use crate::enclosing_items::EnclosingItem;
use crate::enclosing_items::OwnerBody;
use crate::enclosing_items::UnreadableItems;
use crate::finding_source::Utf8Source;
use crate::source_lines;
use crate::source_lines::SiteLine;
use crate::source_lines::SourceLines;
use crate::tokenizer;
use crate::unicode_tables::SPACE;

pub(crate) struct SourceSites {
    source:       Rc<Utf8Source>,
    items:        Vec<EnclosingItem>,
    word_lines:   Vec<usize>,
    declarations: Rc<DeclaredFile>,
    body_lines:   OnceCell<Vec<bool>>,
}

impl SourceSites {
    pub(crate) fn read(source: Rc<Utf8Source>) -> Result<Self, UnreadableItems> {
        let syntax = source.parsed_syntax();
        let items = enclosing_items::read(&source.lines, &syntax.tokens)?;
        let word_lines = syntax.tokens.iter().map(|token| token.line).collect();
        let declarations = Rc::clone(&syntax.declarations);
        Ok(Self {
            source,
            items,
            word_lines,
            declarations,
            body_lines: OnceCell::new(),
        })
    }

    pub(crate) fn line(&self, number: usize) -> SiteLine<'_> { self.lines().line(number) }

    pub(crate) fn check_owner(&self, line: usize) -> Result<(), UnreadableItems> {
        if self
            .items
            .iter()
            .any(|item| matches!(&item.body, OwnerBody::Unproved(lines) if lines.contains(&line)))
        {
            Err(UnreadableItems)
        } else {
            Ok(())
        }
    }

    fn lines(&self) -> &SourceLines { &self.source.lines }

    fn owner(&self, line: usize) -> DeclarationAt<'_> {
        self.declarations.at(&self.word_lines, line)
    }

    pub(crate) fn locates_subject(&self, line: usize, subject: &str) -> bool {
        let lines = match self.owner(line) {
            DeclarationAt::Named(owner) => owner.lines(),
            DeclarationAt::Unnamed => {
                let Some(item) = self
                    .enclosing(line)
                    .min_by_key(|item| item.lines.end() - item.lines.start())
                else {
                    return false;
                };
                item.lines.clone()
            },
        };
        let subject = tokenizer::read_with_comments(&subject.chars().collect::<Vec<_>>()).tokens;
        if subject.is_empty() {
            return false;
        }
        let source =
            tokenizer::read_with_comments(&self.lines().slice(lines).chars().collect::<Vec<_>>())
                .tokens;
        source.windows(subject.len()).any(|window| {
            window
                .iter()
                .zip(&subject)
                .all(|(left, right)| left.value == right.value && left.kind == right.kind)
        })
    }

    pub(crate) fn same_owner(
        &self,
        line: usize,
        other: &Self,
        mapped: usize,
    ) -> Result<bool, UnreadableItems> {
        self.check_owner(line)?;
        other.check_owner(mapped)?;
        let old = self.enclosing(line).collect::<Vec<_>>();
        let new = other.enclosing(mapped).collect::<Vec<_>>();
        if old.len() != new.len()
            || !old.iter().zip(new).all(|(left, right)| {
                left.identity == right.identity
                    && left.header == right.header
                    && self.unique(left)
                    && other.unique(right)
            })
        {
            return Ok(false);
        }
        let SiteLine::Text(text) = self.line(line) else {
            return Ok(false);
        };
        let text = trim(text);
        let same = match (self.owner(line), other.owner(mapped)) {
            (DeclarationAt::Named(left), DeclarationAt::Named(right)) => {
                left.same_declaration(right)
                    && left.same_readable_membership(right)
                    && self.declarations.unique(left)
                    && other.declarations.unique(right)
                    && unique_line(
                        source_lines::split_lines(self.lines().slice(left.lines())),
                        text,
                    )
                    && unique_line(
                        source_lines::split_lines(other.lines().slice(right.lines())),
                        text,
                    )
            },
            (DeclarationAt::Unnamed, DeclarationAt::Unnamed) => {
                unique_line(self.lines().lines(), text) && unique_line(other.lines().lines(), text)
            },
            _ => false,
        };
        Ok(same)
    }

    pub(crate) fn same_item(&self, line: usize, other: &Self, mapped: usize) -> bool {
        let same = if let (DeclarationAt::Named(left), DeclarationAt::Named(right)) =
            (self.owner(line), other.owner(mapped))
        {
            left.has_proved_body()
                && self.lines().slice(left.lines()) == other.lines().slice(right.lines())
        } else {
            let mut before = self.enclosing(line).filter(|item| item.kind == "impl");
            let mut after = other.enclosing(mapped).filter(|item| item.kind == "impl");
            let (Some(left), Some(right)) = (before.next(), after.next()) else {
                return false;
            };
            before.next().is_none()
                && after.next().is_none()
                && left.identity == right.identity
                && self.unique(left)
                && other.unique(right)
                && self.lines().slice(left.lines.clone())
                    == other.lines().slice(right.lines.clone())
        };
        same && self.skeleton().eq(other.skeleton())
    }

    fn skeleton(&self) -> impl Iterator<Item = &str> {
        let covered = self.body_lines.get_or_init(|| {
            let mut covered = vec![false; self.lines().count()];
            for item in self
                .declarations
                .items
                .iter()
                .filter(|item| item.has_proved_body())
            {
                let lines = item.lines();
                let end = (*lines.end()).min(covered.len());
                if let Some(range) = covered.get_mut(lines.start() - 1..end) {
                    range.fill(true);
                }
            }
            covered
        });
        self.lines()
            .lines()
            .zip(covered)
            .filter_map(|(line, &covered)| (!covered).then_some(line))
    }

    fn enclosing(&self, line: usize) -> impl Iterator<Item = &EnclosingItem> {
        self.items
            .iter()
            .filter(move |item| item.lines.contains(&line))
    }

    fn unique(&self, item: &EnclosingItem) -> bool {
        self.items
            .iter()
            .filter(|candidate| candidate.identity == item.identity)
            .take(2)
            .count()
            == 1
    }
}

pub(crate) fn trim(text: &str) -> &str {
    text.trim_matches(|character| tokenizer::in_table(SPACE, character))
}

fn unique_line<'a>(lines: impl Iterator<Item = &'a str>, text: &str) -> bool {
    lines.filter(|line| trim(line) == text).take(2).count() == 1
}
