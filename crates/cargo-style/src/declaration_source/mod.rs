//! Read committed declarations for finding proofs.

mod file_module;
mod normalized_source;
mod statement_location;

use std::cell::OnceCell;
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::ptr;
use std::rc::Rc;

pub(crate) use self::file_module::FileModule;
use self::file_module::FileModules;
use self::file_module::PlainLinks;
pub(crate) use self::file_module::SourceLocation;
pub(crate) use self::normalized_source::NormalizedSlice;
use self::normalized_source::NormalizedSource;
pub(crate) use self::statement_location::StatementLocation;
pub(crate) use self::statement_location::StatementRefusal;
use self::statement_location::StatementTokens;
use crate::declarations;
use crate::declarations::DeclarationAt;
use crate::declarations::DeclarationIndexAt;
use crate::declarations::DeclaredFile;
use crate::declarations::DeclaredItem;
use crate::declarations::ItemBody;
use crate::facts::StoredSyntax;
use crate::finding_source::FindingSource;
use crate::finding_source::FindingSourceFailure;
use crate::finding_source::SourceText;
use crate::finding_source::Utf8Source;
use crate::name_sites;
use crate::name_sites::BodyFacts;
use crate::name_sites::NameSiteReading;
use crate::name_sites::UnreadableBody;
use crate::source_lines::SiteLine;
use crate::source_lines::SourceLines;
use crate::tokenizer;
use crate::tokenizer::Token;

#[derive(Default)]
pub(crate) struct DeclarationSource {
    files:   HashMap<String, Rc<DeclarationFile>>,
    empty:   OnceCell<Rc<DeclarationFile>>,
    modules: HashMap<(String, String), FileModules>,
}

enum DeclarationText {
    Utf8(Rc<Utf8Source>),
    Local {
        lines:  SourceLines,
        syntax: StoredSyntax,
    },
}

impl DeclarationText {
    fn lines(&self) -> &SourceLines {
        match self {
            Self::Utf8(source) => &source.lines,
            Self::Local { lines, .. } => lines,
        }
    }

    fn syntax(&self) -> &StoredSyntax {
        match self {
            Self::Utf8(source) => source.parsed_syntax(),
            Self::Local { syntax, .. } => syntax,
        }
    }
}

pub(crate) struct DeclarationFile {
    source:       DeclarationText,
    words:        Vec<String>,
    word_lines:   Vec<usize>,
    links:        OnceCell<PlainLinks>,
    statements:   OnceCell<StatementTokens>,
    normalized:   NormalizedSource,
    item_indexes: OnceCell<HashMap<*const DeclaredItem, usize>>,
    bodies:       OnceCell<BodyFacts>,
}

impl DeclarationSource {
    pub(crate) fn file(
        &mut self,
        source: &mut FindingSource<'_>,
        location: SourceLocation<'_>,
    ) -> Result<Rc<DeclarationFile>, FindingSourceFailure> {
        let Some(blob) = source
            .tip(location.commit)?
            .blob(location.path)
            .map(str::to_owned)
        else {
            return Ok(Rc::clone(
                self.empty
                    .get_or_init(|| Rc::new(DeclarationFile::read(""))),
            ));
        };
        let file = match self.files.entry(blob) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let text =
                    source
                        .source(entry.key())
                        .map_err(|source| FindingSourceFailure::AtPath {
                            path:   location.path.to_owned(),
                            source: Box::new(source),
                        })?;
                let file = match text {
                    SourceText::Utf8(source) => {
                        DeclarationFile::from(DeclarationText::Utf8(Rc::clone(source)))
                    },
                    SourceText::NotUtf8(text) => DeclarationFile::read(text),
                };
                entry.insert(Rc::new(file))
            },
        };
        Ok(Rc::clone(file))
    }
}

impl DeclarationFile {
    fn read(text: &str) -> Self {
        let tokens = Self::read_tokens(text);
        let declarations = Rc::new(declarations::read_tokens(&tokens));
        Self::from(DeclarationText::Local {
            lines:  SourceLines::from(text.to_owned()),
            syntax: StoredSyntax {
                tokens,
                declarations,
            },
        })
    }

    pub(crate) fn at(&self, line: usize) -> DeclarationAt<'_> {
        self.declared().at(&self.word_lines, line)
    }

    pub(crate) fn index_at(&self, line: usize) -> DeclarationIndexAt {
        self.declared().index_at(&self.word_lines, line)
    }

    pub(crate) fn items(&self) -> &[DeclaredItem] { &self.declared().items }

    pub(crate) fn words(&self) -> &[String] { &self.words }

    pub(crate) fn word_line(&self, index: usize) -> usize { self.word_lines[index] }

    pub(crate) fn lines(&self) -> &SourceLines { self.source.lines() }

    pub(crate) fn line_count(&self) -> usize { self.lines().count() }

    pub(crate) fn line_text(&self, line: usize) -> &str {
        match self.lines().line(line) {
            SiteLine::Text(text) => text,
            SiteLine::Unavailable => "",
        }
    }

    pub(crate) fn tokens(&self) -> &[Token] { &self.source.syntax().tokens }

    pub(crate) fn body_facts(&self) -> &BodyFacts {
        self.bodies.get_or_init(|| {
            let characters = self.lines().text().chars().collect::<Vec<_>>();
            name_sites::read_facts(&characters, self.declared(), self.tokens())
        })
    }

    pub(crate) fn name_sites(&self, item: &DeclaredItem) -> &NameSiteReading {
        self.item_index(item).map_or(
            &NameSiteReading::Unreadable(UnreadableBody::Unreadable),
            |index| &self.body_facts().sites[index],
        )
    }

    pub(crate) fn body(&self, item: &DeclaredItem) -> &ItemBody {
        self.item_index(item).map_or(&ItemBody::NoBody, |index| {
            &self.declared().spans[index].body
        })
    }

    fn item_index(&self, item: &DeclaredItem) -> Option<usize> {
        self.item_indexes
            .get_or_init(|| {
                // Python selects the first equal declaration.
                let mut first_indexes = HashMap::new();
                self.items()
                    .iter()
                    .enumerate()
                    .map(|(index, item)| {
                        (
                            ptr::from_ref(item),
                            *first_indexes.entry(item).or_insert(index),
                        )
                    })
                    .collect()
            })
            .get(&ptr::from_ref(item))
            .copied()
    }

    pub(crate) fn lexical_parent(&self, item: &DeclaredItem) -> DeclarationAt<'_> {
        let spans = &self.declared().spans;
        let Some(index) = self.item_index(item) else {
            return DeclarationAt::Unnamed;
        };
        let keyword = spans[index].keyword_word();
        self.items().iter().zip(spans)
            .filter(|(_, span)| matches!(&span.body, ItemBody::Braced(body) if body.open < keyword && keyword < body.close))
            .max_by_key(|(_, span)| span.keyword_word())
            .map_or(DeclarationAt::Unnamed, |(parent, _)| DeclarationAt::Named(parent))
    }

    fn declared(&self) -> &DeclaredFile { &self.source.syntax().declarations }

    fn read_tokens(text: &str) -> Vec<Token> {
        tokenizer::read_with_comments(&text.chars().collect::<Vec<_>>()).tokens
    }
}

impl From<DeclarationText> for DeclarationFile {
    fn from(source: DeclarationText) -> Self {
        let tokens = &source.syntax().tokens;
        Self {
            words: tokens.iter().map(|token| token.value.clone()).collect(),
            word_lines: tokens.iter().map(|token| token.line).collect(),
            source,
            links: OnceCell::new(),
            statements: OnceCell::new(),
            normalized: NormalizedSource::default(),
            item_indexes: OnceCell::new(),
            bodies: OnceCell::new(),
        }
    }
}
