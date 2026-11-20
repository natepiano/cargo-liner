//! Check which spelling the target declares under a qualified parent.

use std::collections::BTreeSet;
use std::ops::Range;
use std::path::Path;

use super::RenameApplied;
use super::constants::ITEM_KEYWORDS;
use super::constants::NAME_MACROS;
use super::constants::PARENT_KEYWORDS;
use super::constants::STANDARD_DERIVES;
use crate::declarations;
use crate::finding_source::TreeFile;
use crate::ledger_read::HeldRename;
use crate::tokenizer;
use crate::tokenizer::TokenKind;
use crate::word_syntax;
use crate::word_syntax::InlineModule;
use crate::word_syntax::MemberKind;

struct QualifiedRename<'a> {
    qualifier: Vec<&'a str>,
    parent:    &'a str,
    old:       &'a str,
    new:       &'a str,
}

enum QualifiedPair<'a> {
    Rename(QualifiedRename<'a>),
    Unsupported,
}

struct UncertainParent;

#[derive(Clone, Copy)]
enum ParentKind {
    Trait,
    Other,
}

impl<'a> QualifiedRename<'a> {
    fn read(finding: &'a HeldRename) -> QualifiedPair<'a> {
        let (Some((old_parent, old)), Some((new_parent, new))) = (
            finding.before.rsplit_once("::"),
            finding.after.rsplit_once("::"),
        ) else {
            return QualifiedPair::Unsupported;
        };
        if finding.kind != "rename"
            || old_parent != new_parent
            || old == new
            || !word_syntax::identifier(old)
            || !word_syntax::identifier(new)
        {
            return QualifiedPair::Unsupported;
        }
        let (qualifier, parent) = old_parent.rsplit_once("::").map_or_else(
            || (Vec::new(), old_parent),
            |(path, parent)| (path.split("::").collect(), parent),
        );
        QualifiedPair::Rename(Self {
            qualifier,
            parent,
            old,
            new,
        })
    }

    fn declares(&self, words: &[String], path: &str, name: &str) -> Result<bool, UncertainParent> {
        let starts = word_syntax::item_starts(words);
        let modules = word_syntax::inline_modules(words, &starts);
        let parents = starts
            .iter()
            .copied()
            .filter(|&index| {
                let word = word_syntax::word(words, index);
                PARENT_KEYWORDS.contains(&word)
                    && word_syntax::word(words, index + 1) == self.parent
                    && (word != "mod" || word_syntax::body_start(words, index) < words.len())
            })
            .collect::<Vec<_>>();
        let top_level = file_module(path) == self.parent;
        let attributes = word_syntax::decorated_items(words);
        if parents.len() + usize::from(top_level) > 1
            || parents.iter().any(|index| {
                attributes
                    .get(index)
                    .is_some_and(|attached| !standard_derives(attached))
            })
        {
            return Err(UncertainParent);
        }
        let home = parents
            .first()
            .map_or_else(Vec::new, |&index| word_syntax::enclosing(&modules, index));
        let home_path = home
            .iter()
            .map(|module| module.name.as_str())
            .collect::<Vec<_>>();
        if (!parents.is_empty() || top_level) && !home_path.ends_with(&self.qualifier) {
            return Err(UncertainParent);
        }
        let parent_kind = if parents.iter().any(|&index| words[index] == "trait") {
            ParentKind::Trait
        } else {
            ParentKind::Other
        };
        let mut bodies = self.impl_bodies(words, parent_kind, &starts, &modules, &home)?;
        if top_level {
            bodies.push(0..words.len());
        }
        for index in parents {
            let start = word_syntax::body_start(words, index);
            if start == words.len() {
                continue;
            }
            let end = word_syntax::closing(words, start);
            match words[index].as_str() {
                "struct" | "union" => {
                    if has_member(words, start, end, MemberKind::Field, name) {
                        return Ok(true);
                    }
                },
                "mod" | "trait" => bodies.push(start + 1..end),
                _ => {},
            }
        }
        // QualifiedRename::declares also scans enums outside word_syntax::item_starts.
        for index in 0..words.len().saturating_sub(1) {
            if words[index] == "enum" && words[index + 1] == self.parent {
                let start = word_syntax::body_start(words, index);
                if start < words.len()
                    && has_member(
                        words,
                        start,
                        word_syntax::closing(words, start),
                        MemberKind::Variant,
                        name,
                    )
                {
                    return Ok(true);
                }
            }
        }
        Ok(bodies
            .into_iter()
            .any(|body| direct_item(words, body, &starts, name)))
    }

    fn impl_bodies(
        &self,
        words: &[String],
        parent_kind: ParentKind,
        starts: &BTreeSet<usize>,
        modules: &[InlineModule],
        home: &[InlineModule],
    ) -> Result<Vec<Range<usize>>, UncertainParent> {
        let mut bodies = Vec::new();
        let mut qualifiers = BTreeSet::new();
        let mut depth = 0_i32;
        for (index, word) in words.iter().enumerate() {
            depth += i32::from(word == "{") - i32::from(word == "}");
            if word != "impl" || !starts.contains(&index) {
                continue;
            }
            let start = (index + 1..words.len())
                .find(|&at| matches!(words[at].as_str(), "{" | ";"))
                .unwrap_or(words.len());
            let header = &words[index + 1..start];
            if !header.iter().any(|word| word == self.parent) {
                continue;
            }
            let ((ParentKind::Other, ImplPath::Inherent(path))
            | (ParentKind::Trait, ImplPath::Trait(path))) = (parent_kind, impl_path(header)?)
            else {
                continue;
            };
            let Some((&name, qualifier)) = path.split_last() else {
                return Err(UncertainParent);
            };
            if name != self.parent {
                continue;
            }
            if !self.qualifier.ends_with(qualifier)
                || word_syntax::enclosing(modules, index) != home
                || usize::try_from(depth) != Ok(home.len())
            {
                return Err(UncertainParent);
            }
            qualifiers.insert(qualifier.to_vec());
            if word_syntax::word(words, start) == "{" {
                bodies.push(start + 1..word_syntax::closing(words, start));
            }
        }
        if qualifiers.len() > 1 {
            Err(UncertainParent)
        } else {
            Ok(bodies)
        }
    }
}

pub(super) fn check(finding: &HeldRename, file: TreeFile<'_>) -> RenameApplied {
    let unchecked = || RenameApplied::NoAppliedCheck {
        kind: finding.kind.clone(),
    };
    let (QualifiedPair::Rename(rename), TreeFile::Bytes(bytes)) =
        (QualifiedRename::read(finding), file)
    else {
        return unchecked();
    };
    let chars = String::from_utf8_lossy(bytes).chars().collect::<Vec<_>>();
    let tokens = tokenizer::read_with_comments(&chars).tokens;
    let names = tokens
        .iter()
        .filter(|token| token.kind == TokenKind::Identifier)
        .map(|token| token.value.strip_prefix("r#").unwrap_or(&token.value))
        .collect::<BTreeSet<_>>();
    let (old, new) = (names.contains(rename.old), names.contains(rename.new));
    if old == new
        || NAME_MACROS.iter().any(|name| names.contains(name))
        || tokens
            .windows(2)
            .any(|pair| pair[0].value == "[" && pair[1].value == "<")
    {
        return unchecked();
    }
    let held = if old { rename.old } else { rename.new };
    let words = tokens
        .into_iter()
        .map(|token| token.value)
        .collect::<Vec<_>>();
    if !rename
        .declares(&words, &finding.anchor.path, held)
        .unwrap_or(false)
    {
        return unchecked();
    }
    if old {
        RenameApplied::NotApplied {
            path: finding.anchor.path.clone(),
        }
    } else {
        RenameApplied::AppliedInCode
    }
}

enum ImplPath<'a> {
    Inherent(Vec<&'a str>),
    Trait(Vec<&'a str>),
}

fn impl_path(header: &[String]) -> Result<ImplPath<'_>, UncertainParent> {
    let mut depth = 0_i32;
    let mut begin = 0;
    let mut separator = None;
    let generic = word_syntax::word(header, 0) == "<";
    for (index, word) in header.iter().enumerate() {
        depth += i32::from(word == "<") - i32::from(word == ">");
        if depth < 0 {
            return Err(UncertainParent);
        }
        if depth == 0 {
            if generic && begin == 0 {
                begin = index + 1;
            }
            if word == "for"
                && word_syntax::word(header, index + 1) != "<"
                && separator.replace(index).is_some()
            {
                return Err(UncertainParent);
            }
        }
    }
    if depth != 0 {
        return Err(UncertainParent);
    }
    let path = declarations::type_path(&header[begin..separator.unwrap_or(header.len())]);
    Ok(if separator.is_some() {
        ImplPath::Trait(path)
    } else {
        ImplPath::Inherent(path)
    })
}

fn file_module(path: &str) -> &str {
    let file = Path::new(path);
    match file.file_name().and_then(|name| name.to_str()) {
        Some("lib.rs" | "main.rs") => "crate",
        Some("mod.rs") => file
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .unwrap_or_default(),
        _ => file
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or_default(),
    }
}

fn standard_derives(words: &[String]) -> bool {
    word_syntax::attribute_spans(words)
        .iter()
        .all(|&(start, end)| {
            end - start >= 6
                && words[start + 2] == "derive"
                && words[start + 3] == "("
                && words[start + 4..end - 2]
                    .iter()
                    .all(|word| word == "," || STANDARD_DERIVES.contains(&word.as_str()))
        })
}

fn has_member(words: &[String], start: usize, end: usize, kind: MemberKind, name: &str) -> bool {
    word_syntax::member_sites(words, start, end, kind)
        .iter()
        .any(|&index| words[index] == name)
}

fn direct_item(words: &[String], body: Range<usize>, starts: &BTreeSet<usize>, name: &str) -> bool {
    let mut index = body.start;
    while index < body.end {
        let word = words[index].as_str();
        if ITEM_KEYWORDS.contains(&word)
            && starts.contains(&index)
            && index + 1 < body.end
            && words[index + 1] == name
        {
            return true;
        }
        if matches!(word, "{" | "(" | "[") {
            index = word_syntax::closing(words, index);
        }
        index += 1;
    }
    false
}
