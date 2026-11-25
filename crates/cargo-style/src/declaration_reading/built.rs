//! Read the item renames that findings name, and the endpoints of each built rename.

use crate::constants::ITEM_HEAD_KEYWORDS;
use crate::declaration_source::DeclarationFile;
use crate::declaration_source::DeclarationSource;
use crate::declaration_source::SourceLocation;
use crate::declarations::DeclarationAt;
use crate::declarations::DeclarationOwner;
use crate::declarations::ImportPosition;
use crate::finding_source::CommitResolution;
use crate::finding_source::FindingSource;
use crate::finding_source::FindingSourceFailure;
use crate::finding_source::TreeFile;
use crate::ledger_read::BuiltRow;
use crate::ledger_read::FindingEdit;
use crate::ledger_read::FindingFollow;
use crate::ledger_read::FindingKind;
use crate::source_sites;
use crate::tip_view;
use crate::tokenizer;
use crate::trait_identity::SitePosition;
use crate::word_syntax;

/// The path segment before a renamed name.
#[derive(Clone, PartialEq, Eq)]
pub(super) enum RenameScope {
    Unqualified,
    SelfQualified,
    TypeQualified(String),
    ModuleQualified(String),
}

/// The scope of a built rename, with `Self` read as its impl or trait's type.
#[derive(Clone, PartialEq, Eq)]
pub(super) enum BuiltScope {
    Unqualified,
    TypeQualified(String),
    ModuleQualified(String),
}

pub(super) struct ItemRename {
    pub(super) old:   String,
    pub(super) new:   String,
    pub(super) scope: RenameScope,
}

pub(super) enum ItemRenameReading {
    Renamed(ItemRename),
    NotRenamed,
}

/// A type finding's item head: its keyword, its name and the rest of the item.
pub(super) enum ItemHead<'a> {
    Head {
        keyword: &'a str,
        name:    &'a str,
        rest:    &'a str,
    },
    NoHead,
}

/// A built finding's rename and the sites where it reads its old and new spelling.
pub(super) struct BuiltRename {
    pub(super) old:             String,
    pub(super) new:             String,
    pub(super) scope:           BuiltScope,
    pub(super) path:            String,
    pub(super) commit:          String,
    pub(super) before_path:     String,
    pub(super) before_commit:   String,
    pub(super) proposal:        String,
    pub(super) before_position: SitePosition,
    pub(super) position:        SitePosition,
}

enum BuiltReading {
    Rename(Box<BuiltRename>),
    NoRename,
}

enum JudgedCommit {
    Resolved(String),
    Unresolved,
}

/// A built finding whose endpoints could not be read.
pub(super) struct BuiltFailure {
    pub(super) id:     String,
    pub(super) path:   String,
    pub(super) source: FindingSourceFailure,
}

/// The renames of all built findings, in ledger order.
pub(super) fn read_renames(
    rows: &[BuiltRow],
    source: &mut FindingSource<'_>,
    declarations: &mut DeclarationSource,
) -> Result<Vec<BuiltRename>, BuiltFailure> {
    let mut renames = Vec::new();
    for row in rows {
        let reading = read_built(row, source, declarations).map_err(|failure| BuiltFailure {
            id:     row.id.clone(),
            path:   row.path.clone(),
            source: failure,
        })?;
        if let BuiltReading::Rename(rename) = reading {
            renames.push(*rename);
        }
    }
    Ok(renames)
}

fn read_built(
    row: &BuiltRow,
    source: &mut FindingSource<'_>,
    declarations: &mut DeclarationSource,
) -> Result<BuiltReading, FindingSourceFailure> {
    let ItemRenameReading::Renamed(rename) = item_rename(&row.edit) else {
        return Ok(BuiltReading::NoRename);
    };
    let JudgedCommit::Resolved(judged) = judged_commit(row.history.judging_commits(), source)?
    else {
        return Ok(BuiltReading::NoRename);
    };
    let history = &row.history;
    let (read, before_path, before_line, previous) = match &row.follow {
        FindingFollow::Followed {
            commit,
            from_line,
            from_path,
        } => (
            commit.clone(),
            from_path.clone(),
            // A line before the first line reads as line 0, as Python's position reading does.
            usize::try_from(*from_line).unwrap_or(0),
            first_named(history.previous_site_commit(), &judged),
        ),
        FindingFollow::NotFollowed {} => (
            judged.clone(),
            row.path.clone(),
            row.line,
            first_named(history.anchor_commit(), &judged),
        ),
    };
    let scope = match rename.scope {
        RenameScope::Unqualified => BuiltScope::Unqualified,
        RenameScope::TypeQualified(name) => BuiltScope::TypeQualified(name),
        RenameScope::ModuleQualified(module) => BuiltScope::ModuleQualified(module),
        RenameScope::SelfQualified => {
            let location = SourceLocation {
                commit: &read,
                path:   &row.path,
            };
            let file = declarations.file(source, location)?;
            let DeclarationAt::Named(item) = file.at(row.line) else {
                return Ok(BuiltReading::NoRename);
            };
            let DeclarationOwner::ImplOrTrait(owner) = item.owner() else {
                return Ok(BuiltReading::NoRename);
            };
            BuiltScope::TypeQualified(owner.clone())
        },
    };
    let before = SourceLocation {
        commit: &previous,
        path:   &before_path,
    };
    let before_position =
        site_position(source, declarations, before, before_line, &row.edit.before)?;
    let after = SourceLocation {
        commit: &read,
        path:   &row.path,
    };
    let position = site_position(source, declarations, after, row.line, &row.edit.after)?;
    Ok(BuiltReading::Rename(Box::new(BuiltRename {
        old: rename.old,
        new: rename.new,
        scope,
        path: row.path.clone(),
        commit: read.clone(),
        before_path: before_path.clone(),
        before_commit: previous.clone(),
        proposal: row.proposal.clone(),
        before_position,
        position,
    })))
}

/// The judging ticket's checkpoint, else its base, when git resolves it.
fn judged_commit<'a>(
    judging: impl Iterator<Item = &'a str>,
    source: &mut FindingSource<'_>,
) -> Result<JudgedCommit, FindingSourceFailure> {
    for commit in judging {
        if let CommitResolution::Resolved(resolved) = source.resolve_commit(commit)? {
            return Ok(JudgedCommit::Resolved(resolved.clone()));
        }
    }
    Ok(JudgedCommit::Unresolved)
}

fn first_named(recorded: &str, judged: &str) -> String {
    let commit = if recorded.is_empty() {
        judged
    } else {
        recorded
    };
    commit.to_owned()
}

/// The item rename that a finding's edit names.
///
/// A rename gives the last path segment of before and of after; a note after `;` in after is not
/// part of the name. The segment before the old name sets the scope. A type finding gives the item
/// name when only that name changes.
pub(super) fn item_rename(edit: &FindingEdit) -> ItemRenameReading {
    match &edit.kind {
        FindingKind::Rename => qualified_rename(&edit.before, &edit.after),
        FindingKind::Type => {
            let (
                ItemHead::Head {
                    keyword: old_keyword,
                    name: old,
                    rest: old_rest,
                },
                ItemHead::Head {
                    keyword: new_keyword,
                    name: new,
                    rest: new_rest,
                },
            ) = (
                item_head(source_sites::trim(&edit.before)),
                item_head(source_sites::trim(&edit.after)),
            )
            else {
                return ItemRenameReading::NotRenamed;
            };
            if old_keyword == new_keyword && old_rest == new_rest && old != new {
                ItemRenameReading::Renamed(ItemRename {
                    old:   old.to_owned(),
                    new:   new.to_owned(),
                    scope: RenameScope::Unqualified,
                })
            } else {
                ItemRenameReading::NotRenamed
            }
        },
        FindingKind::Restructure | FindingKind::Other(_) => ItemRenameReading::NotRenamed,
    }
}

fn qualified_rename(before: &str, after: &str) -> ItemRenameReading {
    let mut segments: Vec<_> = source_sites::trim(before).split("::").collect();
    let old = segments.pop().unwrap_or_default();
    let new = last_segment(after.split(';').next().unwrap_or_default());
    if old == new || !word_syntax::identifier(old) || !word_syntax::identifier(new) {
        return ItemRenameReading::NotRenamed;
    }
    let scope = match segments.last() {
        None => RenameScope::Unqualified,
        Some(qualifier) if !word_syntax::identifier(qualifier) => {
            return ItemRenameReading::NotRenamed;
        },
        Some(&"Self") => RenameScope::SelfQualified,
        Some(qualifier) => {
            let plain = qualifier.strip_prefix("r#").unwrap_or(qualifier);
            if plain.starts_with(|character: char| character.is_ascii_uppercase()) {
                RenameScope::TypeQualified((*qualifier).to_owned())
            } else {
                RenameScope::ModuleQualified(plain.to_owned())
            }
        },
    };
    ItemRenameReading::Renamed(ItemRename {
        old: old.to_owned(),
        new: new.to_owned(),
        scope,
    })
}

/// The last `::` segment of the trimmed text.
pub(super) fn last_segment(text: &str) -> &str {
    source_sites::trim(text)
        .rsplit("::")
        .next()
        .unwrap_or_default()
}

/// Match `(keyword)\s+(identifier)(.*)` against the whole text, as Python's item-head pattern does.
pub(super) fn item_head(text: &str) -> ItemHead<'_> {
    for keyword in ITEM_HEAD_KEYWORDS {
        let Some(after) = text.strip_prefix(keyword) else {
            continue;
        };
        let name_start = after.trim_start_matches(tokenizer::python_space);
        if name_start.len() == after.len() {
            continue;
        }
        let length = identifier_length(name_start);
        if length == 0 {
            continue;
        }
        let (name, rest) = name_start.split_at(length);
        return ItemHead::Head {
            keyword,
            name,
            rest,
        };
    }
    ItemHead::NoHead
}

/// The length of the longest identifier at the start of `text`.
fn identifier_length(text: &str) -> usize {
    let word = |character: char| character.is_ascii_alphanumeric() || character == '_';
    let plain_length = |body: &str| {
        if body.starts_with(|character: char| character.is_ascii_alphabetic() || character == '_') {
            body.find(|character: char| !word(character))
                .unwrap_or(body.len())
        } else {
            0
        }
    };
    if let Some(raw) = text.strip_prefix("r#") {
        let length = plain_length(raw);
        if length > 0 {
            return length + 2;
        }
    }
    plain_length(text)
}

/// The module a file declares: `crate` for `src/lib.rs` and `src/main.rs`, the directory for
/// `mod.rs` and other `lib.rs` and `main.rs` files, else the file stem.
pub(super) fn declaring_module(path: &str) -> String {
    let parts: Vec<_> = tip_view::path_parts(path).collect();
    let name = parts.last().copied().unwrap_or_default();
    let parent = parts.iter().rev().nth(1).copied().unwrap_or_default();
    let stem = match name.rfind('.') {
        Some(dot) if dot > 0 && dot + 1 < name.len() => &name[..dot],
        _ => name,
    };
    if parent == "src" && matches!(stem, "lib" | "main") {
        return "crate".to_owned();
    }
    if matches!(stem, "mod" | "lib" | "main") {
        parent.to_owned()
    } else {
        stem.to_owned()
    }
}

/// The import scope of a qualifier spelled at `line`, else of the declared item's header there.
pub(super) fn site_position(
    source: &mut FindingSource<'_>,
    declarations: &mut DeclarationSource,
    location: SourceLocation<'_>,
    line: usize,
    spelling: &str,
) -> Result<SitePosition, FindingSourceFailure> {
    if !matches!(
        source.raw_file(location.commit, location.path)?,
        TreeFile::Bytes(_)
    ) {
        return Ok(SitePosition::Unproved);
    }
    let file = declarations.file(source, location)?;
    let spelled: Vec<char> = spelling
        .split(';')
        .next()
        .unwrap_or_default()
        .chars()
        .collect();
    let wanted: Vec<_> = tokenizer::read_with_comments(&spelled)
        .tokens
        .into_iter()
        .map(|token| token.value)
        .collect();
    if wanted.is_empty() {
        return Ok(SitePosition::Unproved);
    }
    let words = file.words();
    let mut blocks = Vec::new();
    let mut scopes: Vec<(Vec<usize>, usize)> = Vec::new();
    for (index, word) in words.iter().enumerate() {
        if file.word_line(index) == line && words[index..].starts_with(&wanted) {
            match scopes.iter_mut().find(|(scope, _)| *scope == blocks) {
                Some(entry) => entry.1 = index,
                None => scopes.push((blocks.clone(), index)),
            }
        }
        if word == "{" {
            blocks.push(index);
        } else if word == "}" {
            blocks.pop();
        }
    }
    Ok(match scopes.as_slice() {
        [] => header_position(&file, line),
        [(scope, index)] if !scope.is_empty() => {
            SitePosition::Proved(ImportPosition::BlockScoped(*index))
        },
        [_] => SitePosition::Proved(ImportPosition::FileLevel),
        _ => SitePosition::Unproved,
    })
}

/// The header scope of the one item that owns `line`; several item boundaries prove none.
fn header_position(file: &DeclarationFile, line: usize) -> SitePosition {
    let DeclarationAt::Named(item) = file.at(line) else {
        return SitePosition::Unproved;
    };
    let mut boundaries = file.items().iter().filter(|other| {
        *other.lines().start() == line
            || other.keyword_line() == line
            || *other.lines().end() == line
    });
    let first = boundaries.next();
    if boundaries.next().is_some()
        || first.is_some_and(|other| other.import_position() != item.import_position())
    {
        return SitePosition::Unproved;
    }
    SitePosition::Proved(item.import_position().clone())
}
