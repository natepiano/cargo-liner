//! Read committed declarations for finding proofs.

mod file_module;
mod statement_location;

use std::cell::OnceCell;
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::rc::Rc;

pub(crate) use self::file_module::FileModule;
use self::file_module::FileModules;
use self::file_module::PlainLinks;
pub(crate) use self::file_module::SourceLocation;
pub(crate) use self::statement_location::StatementLocation;
pub(crate) use self::statement_location::StatementRefusal;
use self::statement_location::StatementTokens;
use crate::declarations;
use crate::declarations::DeclarationAt;
use crate::declarations::DeclaredFile;
use crate::declarations::DeclaredItem;
use crate::facts::StoredSyntax;
use crate::finding_source::FindingSource;
use crate::finding_source::FindingSourceFailure;
use crate::finding_source::SourceText;
use crate::finding_source::Utf8Source;
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
    source:     DeclarationText,
    words:      Vec<String>,
    word_lines: Vec<usize>,
    links:      OnceCell<PlainLinks>,
    statements: OnceCell<StatementTokens>,
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
                let file = match source.source(entry.key())? {
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

    pub(crate) fn items(&self) -> &[DeclaredItem] { &self.declared().items }

    pub(crate) fn words(&self) -> &[String] { &self.words }

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
        }
    }
}
