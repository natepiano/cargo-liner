//! Read and render the facts of one Git blob.

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::fmt::Write;
use std::rc::Rc;

use gix::ObjectId;
#[cfg(test)]
use libc::LC_CTYPE;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Error;
use sha2::Digest;
use sha2::Sha256;

use crate::constants::CARRIAGE_RETURN_LINE_FEED;
use crate::constants::MODULE_SITES_FACT;
use crate::declarations;
use crate::declarations::DeclaredFile;
use crate::module_scan;
use crate::module_scan::ModuleScan;
use crate::module_scan::ModuleScanReading;
use crate::module_sites;
use crate::module_sites::ModuleSiteReading;
use crate::module_sites::ModuleSitesQuery;
use crate::name_sites;
use crate::name_sites::BodyFacts;
use crate::name_sites::BodyTokens;
use crate::name_sites::NameSiteReading;
use crate::request::FactKind;
use crate::request::NameWordClass;
use crate::tokenizer;
use crate::tokenizer::CommentSpan;
use crate::tokenizer::Token;
use crate::tokenizer::TokenKind;
use crate::unicode_tables::LINE_BOUNDARIES;

/// One output line for one requested blob fact.
#[derive(Serialize)]
struct FactLine<'a> {
    blob:    &'a str,
    fact:    FactKind,
    #[serde(flatten)]
    reading: Reading<FactValue<'a>>,
}

/// One output line for a module site query.
#[derive(Serialize)]
struct ModuleSitesLine<'a> {
    blob:    &'a str,
    path:    &'a str,
    fact:    &'a str,
    #[serde(flatten)]
    reading: Reading<Vec<ModuleSiteReading>>,
}

/// A fact value, or the reason that the blob has no value.
#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum Reading<Value> {
    Value(Value),
    Unreadable(Unreadable),
}

#[derive(Serialize)]
#[serde(untagged)]
enum FactValue<'a> {
    Lines(usize),
    Names(Cow<'a, BTreeSet<String>>),
    Tokens(&'a [Token]),
    ModuleScan(&'a ModuleScan),
    Declarations(DeclarationFacts<'a>),
    NameSites(&'a [NameSiteReading]),
    RustCode(&'a str),
}

/// Why a blob has no facts.
#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum Unreadable {
    NotUtf8,
    MissingBlob,
    TokenSpans,
}

/// Declaration structure and the normalized offsets share one blob reading.
#[derive(Serialize)]
struct DeclarationFacts<'a> {
    #[serde(flatten)]
    file:        &'a DeclaredFile,
    body_tokens: &'a BodyTokens,
}

/// The stored, locale-independent facts of an object that Git supplied.
#[derive(Deserialize, Serialize)]
pub(crate) struct FactBundle {
    text:         String,
    tokens:       Vec<Token>,
    declarations: DeclaredFile,
    body:         BodyFacts,
    module_scan:  ModuleScanReading,
    lines:        usize,
    ascii_names:  BTreeSet<String>,
}

/// The blob facts, including facts that use text from `String::from_utf8_lossy`.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub(crate) enum FactReading {
    Read {
        bundle:        FactBundle,
        code_identity: CodeIdentity,
    },
    MalformedUtf8(FactBundle),
    MissingBlob,
}

pub(crate) enum BlobSource {
    Utf8 { text: String, syntax: BlobSyntax },
    NotUtf8(String),
    Missing,
}

/// Syntax for a source read. `Stored` uses the tokens and declarations from `FactCache`.
pub(crate) enum BlobSyntax {
    Stored(Box<StoredSyntax>),
    Unparsed,
}

pub(crate) struct StoredSyntax {
    pub(crate) tokens:       Vec<Token>,
    pub(crate) declarations: Rc<DeclaredFile>,
}

/// The Rust code identity of a UTF-8 blob.
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CodeIdentity {
    Digest(String),
    UnreadableTokens,
}

impl CodeIdentity {
    fn fact(&self) -> Reading<FactValue<'_>> {
        match self {
            Self::Digest(digest) => Reading::Value(FactValue::RustCode(digest)),
            Self::UnreadableTokens => Reading::Unreadable(Unreadable::TokenSpans),
        }
    }
}

impl FactReading {
    pub(crate) fn into_source(self) -> BlobSource {
        match self {
            Self::Read { bundle, .. } => BlobSource::Utf8 {
                text:   bundle.text,
                syntax: BlobSyntax::Stored(Box::new(StoredSyntax {
                    tokens:       bundle.tokens,
                    declarations: Rc::new(bundle.declarations),
                })),
            },
            Self::MalformedUtf8(bundle) => BlobSource::NotUtf8(bundle.text),
            Self::MissingBlob => BlobSource::Missing,
        }
    }

    /// Build each fact family from the same lexical reading.
    pub(crate) fn parse(data: &[u8]) -> Result<Self, Error> {
        let decoded = String::from_utf8_lossy(data);
        let text: Vec<char> = decoded.chars().collect();
        let reading = tokenizer::read_with_comments(&text);
        let bundle = |decoded, tokens: Vec<Token>| {
            let declarations = declarations::read_tokens(&tokens);
            FactBundle {
                text: decoded,
                body: name_sites::read_facts(&text, &declarations, &tokens),
                module_scan: module_scan::read_tokens(&tokens),
                lines: line_count(&text),
                tokens,
                declarations,
                ascii_names: ascii_names(data),
            }
        };
        Ok(match decoded {
            Cow::Borrowed(decoded) => Self::Read {
                code_identity: rust_code(&text, &reading.tokens, &reading.comments)?,
                bundle:        bundle(decoded.to_owned(), reading.tokens),
            },
            Cow::Owned(decoded) => Self::MalformedUtf8(bundle(decoded, reading.tokens)),
        })
    }

    /// Write the legacy JSON lines in the order supplied by the request.
    pub(crate) fn render(
        &self,
        blob: ObjectId,
        kinds: &[FactKind],
        queries: &[&ModuleSitesQuery],
        word_class: NameWordClass,
    ) -> Result<String, Error> {
        let mut lines = String::new();
        let blob = blob.to_string();
        for &fact in kinds {
            let reading = match self {
                Self::MalformedUtf8(_)
                    if fact == FactKind::Lines
                        || (fact == FactKind::Names
                            && matches!(word_class, NameWordClass::PosixAlnum)) =>
                {
                    Reading::Unreadable(Unreadable::NotUtf8)
                },
                Self::Read {
                    bundle,
                    code_identity,
                } => bundle.value(fact, word_class, code_identity.fact()),
                Self::MalformedUtf8(bundle) => {
                    bundle.value(fact, word_class, Reading::Unreadable(Unreadable::NotUtf8))
                },
                Self::MissingBlob => Reading::Unreadable(Unreadable::MissingBlob),
            };
            append_line(
                &mut lines,
                &FactLine {
                    blob: &blob,
                    fact,
                    reading,
                },
            )?;
        }
        if !queries.is_empty() {
            match self {
                Self::Read { bundle, .. } | Self::MalformedUtf8(bundle) => {
                    bundle.append_queries(&mut lines, &blob, queries)?;
                },
                Self::MissingBlob => {
                    for query in queries {
                        append_line(
                            &mut lines,
                            &ModuleSitesLine {
                                blob:    &blob,
                                path:    &query.path,
                                fact:    MODULE_SITES_FACT,
                                reading: Reading::Unreadable(Unreadable::MissingBlob),
                            },
                        )?;
                    }
                },
            }
        }
        Ok(lines)
    }
}

impl FactBundle {
    fn value<'a>(
        &'a self,
        fact: FactKind,
        word_class: NameWordClass,
        code_identity: Reading<FactValue<'a>>,
    ) -> Reading<FactValue<'a>> {
        Reading::Value(match fact {
            FactKind::Lines => FactValue::Lines(self.lines),
            FactKind::Names => match word_class {
                NameWordClass::PosixAlnum => FactValue::Names(Cow::Owned(names(&self.text))),
                NameWordClass::Ascii => FactValue::Names(Cow::Borrowed(&self.ascii_names)),
            },
            FactKind::Tokens => FactValue::Tokens(&self.tokens),
            FactKind::RustCode => return code_identity,
            FactKind::ModuleScan => match &self.module_scan {
                ModuleScanReading::Value(scan) => FactValue::ModuleScan(scan),
                ModuleScanReading::TokenSpansUnreadable => {
                    return Reading::Unreadable(Unreadable::TokenSpans);
                },
            },
            FactKind::Declarations => FactValue::Declarations(DeclarationFacts {
                file:        &self.declarations,
                body_tokens: &self.body.tokens,
            }),
            FactKind::NameSites => FactValue::NameSites(&self.body.sites),
        })
    }

    fn append_queries(
        &self,
        lines: &mut String,
        blob: &str,
        queries: &[&ModuleSitesQuery],
    ) -> Result<(), Error> {
        let text: Vec<char> = self.text.chars().collect();
        for query in queries {
            let sites = module_sites::read_tokens(&text, &self.tokens, &query.path, &query.moves);
            append_line(
                lines,
                &ModuleSitesLine {
                    blob,
                    path: &query.path,
                    fact: MODULE_SITES_FACT,
                    reading: Reading::Value(sites),
                },
            )?;
        }
        Ok(())
    }
}

fn append_line(lines: &mut String, value: &impl Serialize) -> Result<(), Error> {
    lines.push_str(&serde_json::to_string(value)?);
    lines.push('\n');
    Ok(())
}

/// Port `_rust_code`: retain each token and each inner doc at its token position.
fn rust_code(
    text: &[char],
    tokens: &[Token],
    comments: &[CommentSpan],
) -> Result<CodeIdentity, Error> {
    if tokens.iter().any(|token| token.kind == TokenKind::Unknown) {
        return Ok(CodeIdentity::UnreadableTokens);
    }
    let code: Vec<_> = tokens
        .iter()
        .map(|token| (&token.value, token.kind))
        .collect();
    let docs: Vec<_> = comments
        .iter()
        .filter(|comment| {
            text[comment.start..].starts_with(&['/', '/', '!'])
                || text[comment.start..].starts_with(&['/', '*', '!'])
        })
        .map(|comment| {
            (
                comment.token_position,
                text[comment.start..comment.end].iter().collect::<String>(),
            )
        })
        .collect();
    Ok(CodeIdentity::Digest(python_json_digest(&(code, docs))?))
}

/// Hash compact JSON with the same ASCII escapes as Python.
pub(crate) fn python_json_digest(value: &impl Serialize) -> Result<String, Error> {
    python_serialized_json_digest(&serde_json::to_string(value)?)
}

pub(crate) fn python_serialized_json_digest(serialized: &str) -> Result<String, Error> {
    source_digest(&python_ascii_json(serialized)?)
}

pub(crate) fn python_ascii_json(serialized: &str) -> Result<String, Error> {
    let mut ascii = String::with_capacity(serialized.len());
    for character in serialized.chars() {
        if character <= '~' {
            ascii.push(character);
        } else {
            // Python ensure_ascii escapes all non-ASCII code units, including DEL.
            for unit in character.encode_utf16(&mut [0; 2]) {
                write!(ascii, "\\u{unit:04x}")
                    .map_err(std::io::Error::other)
                    .map_err(Error::io)?;
            }
        }
    }
    Ok(ascii)
}

pub(crate) fn source_digest(text: &str) -> Result<String, Error> {
    let mut hexadecimal = String::new();
    for byte in Sha256::digest(text.as_bytes()) {
        write!(hexadecimal, "{byte:02x}")
            .map_err(std::io::Error::other)
            .map_err(Error::io)?;
    }
    Ok(hexadecimal)
}

/// Python `len(text.replace("\r\n", "\n").replace("\r", "\n").splitlines())`.
fn line_count(text: &[char]) -> usize {
    let boundaries = text
        .iter()
        .filter(|&&character| tokenizer::in_table(LINE_BOUNDARIES, character))
        .count();
    let carriage_return_pairs = text
        .windows(CARRIAGE_RETURN_LINE_FEED.len())
        .filter(|pair| *pair == CARRIAGE_RETURN_LINE_FEED)
        .count();
    let unended_last_line = text
        .last()
        .is_some_and(|&character| !tokenizer::in_table(LINE_BOUNDARIES, character));
    boundaries - carriage_return_pairs + usize::from(unended_last_line)
}

#[allow(
    unsafe_code,
    reason = "The host POSIX word class requires the C iswalnum function."
)]
unsafe extern "C" {
    fn iswalnum(character: u32) -> libc::c_int;
}

/// Each longest run of the host POSIX alphanumeric class and underscore.
#[allow(
    unsafe_code,
    reason = "The C iswalnum function preserves the host POSIX word class."
)]
fn names(text: &str) -> BTreeSet<String> {
    text.split(|character| {
        // SAFETY: Each Rust character is a valid code point for the host wide-character classifier.
        character != '_' && unsafe { iswalnum(u32::from(character)) } == 0
    })
    .filter(|word| !word.is_empty())
    .map(str::to_owned)
    .collect()
}

/// ASCII words remain readable when other bytes are not UTF-8.
fn ascii_names(data: &[u8]) -> BTreeSet<String> {
    data.split(|byte| !byte.is_ascii_alphanumeric() && *byte != b'_')
        .filter(|word| !word.is_empty())
        .map(|word| String::from_utf8_lossy(word).into_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::Error;

    use super::FactReading;
    use super::LC_CTYPE;
    use super::line_count;
    use super::names;

    fn characters(text: &str) -> Vec<char> { text.chars().collect() }

    #[test]
    fn valid_utf8_cache_rows_reject_a_not_utf8_code_identity() -> Result<(), Error> {
        let mut stored = serde_json::to_value(FactReading::parse(b"fn example() {}\n")?)?;
        stored["read"]["code_identity"] = serde_json::json!({"unreadable": "not_utf8"});
        assert!(serde_json::from_value::<FactReading>(stored).is_err());
        Ok(())
    }

    #[test]
    fn line_count_follows_python_splitlines() {
        for (text, expected) in [
            ("", 0),
            ("a", 1),
            ("a\n", 1),
            ("a\r\nb", 2),
            ("\r\r\n", 2),
            ("a\u{1c}b\u{2028}", 2),
            ("a\tb", 1),
        ] {
            assert_eq!(line_count(&characters(text)), expected, "{text:?}");
        }
    }

    #[test]
    #[allow(
        unsafe_code,
        reason = "The test selects a UTF-8 locale before it checks the POSIX word class."
    )]
    fn names_are_word_runs_in_code_comments_and_literals() {
        let selected = [c"C.UTF-8", c"en_US.UTF-8"].into_iter().any(|locale| {
            // SAFETY: No other unit test changes or reads the C locale.
            !unsafe { libc::setlocale(LC_CTYPE, locale.as_ptr()) }.is_null()
        });
        assert!(selected, "the test needs an available UTF-8 locale");
        let found = names("let naïve_1 = \"x::y\"; // ü");
        let expected = ["let", "naïve_1", "x", "y", "ü"].map(str::to_owned);
        assert_eq!(found, expected.into_iter().collect());
    }
}
