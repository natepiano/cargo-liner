use std::cmp::Reverse;
use std::collections::HashMap;
use std::fs;
use std::ops::Range;
use std::path::Path;

use anyhow::Result;
use rustc_middle::ty::TyCtxt;
use rustc_span::FileName;
use rustc_span::Span;
use rustc_span::def_id::LocalDefId;
use rustc_span::def_id::LocalModDefId;

use super::boundary;
use super::boundary::ModuleSourceMap;
use super::boundary::ParentBoundary;
use super::exports::ParentFacadeExports;
use crate::compiler::settings::DriverSettings;
use crate::compiler::source_cache::ExtractedPaths;
use crate::compiler::source_cache::PathOrigin;
use crate::compiler::source_cache::SourceCache;
use crate::compiler::source_cache::UseRename;
use crate::rust_syntax::PathAnchor;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::compiler) enum ParentFacadeUsage {
    Unused,
    UsedInsideSubtreeByRelativeImport,
    UsedInsideSubtreeByRelativePath,
    UsedInsideSubtreeByCrateImport,
    UsedInsideSubtreeByCratePath,
    UsedOutsideSubtree,
}

pub(in crate::compiler) type ParentFacadeUsageByName = HashMap<String, ParentFacadeUsage>;

pub(super) fn normalized_export_name(name: &str) -> &str { name.strip_prefix("r#").unwrap_or(name) }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParentFacadeReferenceUsage {
    None,
    Import(PathOrigin),
    DirectPath(PathOrigin),
}

struct LiteralModuleContext {
    root_path:     Vec<String>,
    inline_ranges: Vec<ModuleRange>,
}

impl LiteralModuleContext {
    fn path_at(&self, offset: usize) -> &[String] {
        self.inline_ranges
            .iter()
            .filter(|module_range| module_range.bytes.contains(&offset))
            .min_by_key(|module_range| {
                (
                    module_range
                        .bytes
                        .end
                        .saturating_sub(module_range.bytes.start),
                    Reverse(module_range.path.len()),
                )
            })
            .map_or(self.root_path.as_slice(), |module_range| {
                module_range.path.as_slice()
            })
    }
}

struct ModuleRange {
    path:  Vec<String>,
    bytes: Range<usize>,
}

pub(super) fn scan_facade_usage(
    source_cache: &SourceCache,
    settings: &DriverSettings,
    tcx: TyCtxt<'_>,
    module_sources: &ModuleSourceMap,
    parent_boundary: &ParentBoundary,
    exported_names: &ParentFacadeExports,
) -> Result<ParentFacadeUsageByName> {
    let mut usage_by_name = exported_names
        .explicit
        .iter()
        .map(|name| {
            (
                normalized_export_name(name).to_string(),
                ParentFacadeUsage::Unused,
            )
        })
        .collect::<ParentFacadeUsageByName>();
    for source_path in source_cache.source_files() {
        let Some(extracted) = source_cache.extracted_paths(source_path) else {
            continue;
        };
        for (current_module_path, module_suffix) in
            active_module_contexts(extracted, module_sources, tcx, source_path)
        {
            if source_path == parent_boundary.boundary_file
                && current_module_path == parent_boundary.module_path
            {
                continue;
            }
            let reference_usage_by_name = source_references_parent_exports(
                extracted,
                &current_module_path,
                &module_suffix,
                &parent_boundary.module_path,
                &exported_names.explicit,
            );
            let inside_subtree =
                module_path_is_descendant(&current_module_path, &parent_boundary.module_path);
            for (name, reference_usage) in reference_usage_by_name {
                let next_usage = match reference_usage {
                    ParentFacadeReferenceUsage::None => ParentFacadeUsage::Unused,
                    ParentFacadeReferenceUsage::Import(PathOrigin::Relative) => {
                        if inside_subtree {
                            ParentFacadeUsage::UsedInsideSubtreeByRelativeImport
                        } else {
                            ParentFacadeUsage::UsedOutsideSubtree
                        }
                    },
                    ParentFacadeReferenceUsage::Import(PathOrigin::Crate) => {
                        if inside_subtree {
                            ParentFacadeUsage::UsedInsideSubtreeByCrateImport
                        } else {
                            ParentFacadeUsage::UsedOutsideSubtree
                        }
                    },
                    ParentFacadeReferenceUsage::DirectPath(PathOrigin::Relative) => {
                        if inside_subtree {
                            ParentFacadeUsage::UsedInsideSubtreeByRelativePath
                        } else {
                            ParentFacadeUsage::UsedOutsideSubtree
                        }
                    },
                    ParentFacadeReferenceUsage::DirectPath(PathOrigin::Crate) => {
                        if inside_subtree {
                            ParentFacadeUsage::UsedInsideSubtreeByCratePath
                        } else {
                            ParentFacadeUsage::UsedOutsideSubtree
                        }
                    },
                };
                merge_usage_for_name(&mut usage_by_name, name, next_usage);
            }
        }
    }

    let literal_usage_by_name = workspace_source_parent_export_literal_usage(
        source_cache,
        settings,
        tcx,
        module_sources,
        &parent_boundary.module_path,
        &exported_names.explicit,
    )?;
    for (name, literal_usage) in literal_usage_by_name {
        merge_usage_for_name(&mut usage_by_name, name, literal_usage);
    }

    Ok(usage_by_name)
}

pub(in crate::compiler) fn workspace_source_parent_export_literal_usage(
    source_cache: &SourceCache,
    settings: &DriverSettings,
    tcx: TyCtxt<'_>,
    module_sources: &ModuleSourceMap,
    module_path: &[String],
    exported_names: &[String],
) -> Result<ParentFacadeUsageByName> {
    let mut usage_by_name = exported_names
        .iter()
        .map(|name| {
            (
                normalized_export_name(name).to_string(),
                ParentFacadeUsage::Unused,
            )
        })
        .collect::<ParentFacadeUsageByName>();
    if module_path.is_empty() {
        return Ok(usage_by_name);
    }

    for file in source_cache.source_files() {
        if file.starts_with(&settings.findings_dir) {
            continue;
        }
        let root_modules = module_sources.root_modules_for_file(tcx, file);
        if root_modules.is_empty() {
            continue;
        }
        let source = source_cache.read_source(file)?;
        let mut matches = Vec::new();
        for name in exported_names {
            let name = normalized_export_name(name);
            for item_spelling in [name.to_string(), format!("r#{name}")] {
                matches.extend(source.match_indices(&item_spelling).filter_map(
                    |(item_offset, matched_item)| {
                        let path_offset =
                            literal_crate_path_start(source, item_offset, module_path)?;
                        let path_length = item_offset + matched_item.len() - path_offset;
                        literal_match_has_identifier_boundaries(source, path_offset, path_length)
                            .then_some((path_offset, name))
                    },
                ));
            }
        }
        if matches.is_empty() {
            continue;
        }
        let module_contexts = literal_module_contexts(module_sources, tcx, file, &root_modules);
        for (offset, name) in matches {
            for module_context in &module_contexts {
                let current_module_path = module_context.path_at(offset);
                let literal_usage = if module_path_is_descendant(current_module_path, module_path) {
                    ParentFacadeUsage::UsedInsideSubtreeByCratePath
                } else {
                    ParentFacadeUsage::UsedOutsideSubtree
                };
                merge_usage_for_name(&mut usage_by_name, name.to_string(), literal_usage);
            }
        }
    }

    Ok(usage_by_name)
}

fn literal_crate_path_start(
    source: &str,
    item_start: usize,
    module_path: &[String],
) -> Option<usize> {
    let mut separator_offset = path_separator_before(source, item_start)?;
    for expected_segment in module_path.iter().rev() {
        let segment_end = skip_rust_trivia_backward(source, separator_offset);
        let segment_start = literal_identifier_start(source, segment_end);
        let source_segment = source.get(segment_start..segment_end)?.trim();
        if normalized_export_name(source_segment) != normalized_export_name(expected_segment) {
            return None;
        }
        separator_offset = path_separator_before(source, segment_start)?;
    }

    let crate_end = skip_rust_trivia_backward(source, separator_offset);
    let crate_start = literal_identifier_start(source, crate_end);
    (source.get(crate_start..crate_end)? == "crate").then_some(crate_start)
}

fn path_separator_before(source: &str, segment_start: usize) -> Option<usize> {
    let separator_end = skip_rust_trivia_backward(source, segment_start);
    source
        .get(..separator_end)?
        .strip_suffix("::")
        .map(str::len)
}

fn literal_identifier_start(source: &str, segment_end: usize) -> usize {
    let mut start = segment_end;
    while let Some((offset, character)) = source[..start].char_indices().next_back() {
        if !identifier_character(character) {
            break;
        }
        start = offset;
    }
    if start >= 2 && source.get(start - 2..start) == Some("r#") {
        start - 2
    } else {
        start
    }
}

#[derive(Clone, Copy)]
enum RustLexicalState {
    Code,
    NormalString,
    RawString { hashes: usize },
    CharLiteral,
    LineComment,
    BlockComment { depth: usize },
}

fn skip_rust_trivia_backward(source: &str, end: usize) -> usize {
    let bytes = source.as_bytes();
    let mut state = RustLexicalState::Code;
    let mut offset = 0;
    let mut trivia_start = None;
    while offset < end {
        state = match state {
            RustLexicalState::Code => {
                let character = source[offset..end].chars().next().unwrap_or_default();
                if character.is_whitespace() {
                    trivia_start.get_or_insert(offset);
                    offset += character.len_utf8();
                    RustLexicalState::Code
                } else if bytes.get(offset..offset + 2) == Some(b"//") {
                    trivia_start.get_or_insert(offset);
                    offset += 2;
                    RustLexicalState::LineComment
                } else if bytes.get(offset..offset + 2) == Some(b"/*") {
                    trivia_start.get_or_insert(offset);
                    offset += 2;
                    RustLexicalState::BlockComment { depth: 1 }
                } else if let Some((hashes, content_start)) = raw_string_opening(bytes, offset, end)
                {
                    trivia_start = None;
                    offset = content_start;
                    RustLexicalState::RawString { hashes }
                } else if bytes.get(offset) == Some(&b'"') {
                    trivia_start = None;
                    offset += 1;
                    RustLexicalState::NormalString
                } else if bytes.get(offset) == Some(&b'\'')
                    && starts_char_literal(source, offset, end)
                {
                    trivia_start = None;
                    offset += 1;
                    RustLexicalState::CharLiteral
                } else {
                    trivia_start = None;
                    offset += character.len_utf8();
                    RustLexicalState::Code
                }
            },
            RustLexicalState::NormalString => match bytes.get(offset) {
                Some(b'\\') => {
                    offset = (offset + 2).min(end);
                    RustLexicalState::NormalString
                },
                Some(b'"') => {
                    offset += 1;
                    RustLexicalState::Code
                },
                Some(_) => {
                    offset += 1;
                    RustLexicalState::NormalString
                },
                None => RustLexicalState::NormalString,
            },
            RustLexicalState::RawString { hashes } => {
                if raw_string_closes(bytes, offset, end, hashes) {
                    offset += hashes + 1;
                    RustLexicalState::Code
                } else {
                    offset += 1;
                    RustLexicalState::RawString { hashes }
                }
            },
            RustLexicalState::CharLiteral => match bytes.get(offset) {
                Some(b'\\') => {
                    offset = (offset + 2).min(end);
                    RustLexicalState::CharLiteral
                },
                Some(b'\'') => {
                    offset += 1;
                    RustLexicalState::Code
                },
                Some(_) => {
                    offset += 1;
                    RustLexicalState::CharLiteral
                },
                None => RustLexicalState::CharLiteral,
            },
            RustLexicalState::LineComment => {
                if bytes.get(offset) == Some(&b'\n') {
                    offset += 1;
                    RustLexicalState::Code
                } else {
                    offset += 1;
                    RustLexicalState::LineComment
                }
            },
            RustLexicalState::BlockComment { depth } => {
                if bytes.get(offset..offset + 2) == Some(b"/*") {
                    offset += 2;
                    RustLexicalState::BlockComment { depth: depth + 1 }
                } else if bytes.get(offset..offset + 2) == Some(b"*/") {
                    offset += 2;
                    if depth == 1 {
                        RustLexicalState::Code
                    } else {
                        RustLexicalState::BlockComment { depth: depth - 1 }
                    }
                } else {
                    offset += 1;
                    RustLexicalState::BlockComment { depth }
                }
            },
        };
    }
    trivia_start.unwrap_or(end)
}

fn raw_string_opening(bytes: &[u8], offset: usize, end: usize) -> Option<(usize, usize)> {
    if bytes.get(offset) != Some(&b'r') {
        return None;
    }
    let mut quote_offset = offset + 1;
    while quote_offset < end && bytes.get(quote_offset) == Some(&b'#') {
        quote_offset += 1;
    }
    (bytes.get(quote_offset) == Some(&b'"'))
        .then_some((quote_offset.saturating_sub(offset + 1), quote_offset + 1))
}

fn raw_string_closes(bytes: &[u8], offset: usize, end: usize, hashes: usize) -> bool {
    bytes.get(offset) == Some(&b'"')
        && offset + hashes < end
        && bytes
            .get(offset + 1..offset + hashes + 1)
            .is_some_and(|closing_hashes| closing_hashes.iter().all(|byte| *byte == b'#'))
}

fn starts_char_literal(source: &str, quote_offset: usize, end: usize) -> bool {
    let content_start = quote_offset + 1;
    let line_end = source[content_start..end]
        .find('\n')
        .map_or(end, |offset| content_start + offset);
    let Some(first_character) = source[content_start..line_end].chars().next() else {
        return false;
    };
    if identifier_character(first_character) {
        let identifier_end = source[content_start..line_end]
            .char_indices()
            .take_while(|(_, character)| identifier_character(*character))
            .last()
            .map_or(content_start, |(offset, character)| {
                content_start + offset + character.len_utf8()
            });
        return source
            .get(identifier_end..line_end)
            .is_some_and(|tail| tail.starts_with('\''));
    }

    let mut characters = source[content_start..line_end].chars();
    while let Some(character) = characters.next() {
        if character == '\\' {
            characters.next();
        } else if character == '\'' {
            return true;
        }
    }
    false
}

fn literal_match_has_identifier_boundaries(source: &str, offset: usize, length: usize) -> bool {
    let Some(before) = source.get(..offset) else {
        return false;
    };
    let Some(after) = source.get(offset + length..) else {
        return false;
    };
    before
        .chars()
        .next_back()
        .is_none_or(|character| !identifier_character(character))
        && after
            .chars()
            .next()
            .is_none_or(|character| !identifier_character(character))
}

fn identifier_character(character: char) -> bool { character.is_alphanumeric() || character == '_' }

fn literal_module_contexts(
    module_sources: &ModuleSourceMap,
    tcx: TyCtxt<'_>,
    source_file: &Path,
    root_modules: &[LocalDefId],
) -> Vec<LiteralModuleContext> {
    root_modules
        .iter()
        .map(|root_module| {
            let root_path = boundary::module_path(tcx, *root_module);
            let mut inline_ranges = Vec::new();
            tcx.hir_for_each_module(|module| {
                let module_path = boundary::module_path(tcx, module.to_local_def_id());
                if module_path.len() <= root_path.len()
                    || !module_path.starts_with(&root_path)
                    || !module_sources.file_contains_module_path(tcx, source_file, &module_path)
                {
                    return;
                }
                let Some(bytes) = module_byte_range_in_file(tcx, module, source_file) else {
                    return;
                };
                inline_ranges.push(ModuleRange {
                    path: module_path,
                    bytes,
                });
            });
            LiteralModuleContext {
                root_path,
                inline_ranges,
            }
        })
        .collect()
}

fn module_byte_range_in_file(
    tcx: TyCtxt<'_>,
    module: LocalModDefId,
    source_file: &Path,
) -> Option<Range<usize>> {
    let (_, span, _) = tcx.hir_get_module(module);
    span_byte_range_in_file(tcx, span, source_file)
}

fn span_byte_range_in_file(
    tcx: TyCtxt<'_>,
    span: Span,
    source_file: &Path,
) -> Option<Range<usize>> {
    let source_map = tcx.sess.source_map();
    let start = source_map.lookup_byte_offset(span.lo());
    let end = source_map.lookup_byte_offset(span.hi());
    if start.sf.stable_id != end.sf.stable_id {
        return None;
    }
    let FileName::Real(real_file_name) = &start.sf.name else {
        return None;
    };
    let span_file = real_file_name.local_path()?;
    let canonical_span_file =
        fs::canonicalize(span_file).unwrap_or_else(|_| span_file.to_path_buf());
    let canonical_source_file =
        fs::canonicalize(source_file).unwrap_or_else(|_| source_file.to_path_buf());
    if canonical_span_file != canonical_source_file {
        return None;
    }
    let start_offset = start.sf.original_relative_byte_pos(span.lo()).0 as usize;
    let end_offset = end.sf.original_relative_byte_pos(span.hi()).0 as usize;
    Some(start_offset..end_offset)
}

fn source_references_parent_exports(
    extracted: &ExtractedPaths,
    current_module_path: &[String],
    module_suffix: &[String],
    module_path: &[String],
    exported_names: &[String],
) -> HashMap<String, ParentFacadeReferenceUsage> {
    let mut usage_by_name = HashMap::new();
    for extracted_path in &extracted.expr_paths {
        if extracted_path.module_suffix != module_suffix {
            continue;
        }
        let matching_name = matching_export_name_indexed(
            &extracted_path.segments,
            current_module_path,
            module_path,
            exported_names,
        )
        .or_else(|| {
            resolve_alias_expr_path(
                &extracted_path.segments,
                module_suffix,
                &extracted.use_renames,
            )
            .and_then(|resolved| {
                matching_export_name_indexed(
                    &resolved,
                    current_module_path,
                    module_path,
                    exported_names,
                )
            })
        });
        if let Some(name) = matching_name {
            merge_reference_usage_for_name(
                &mut usage_by_name,
                name,
                ParentFacadeReferenceUsage::DirectPath(extracted_path.origin),
            );
        }
    }

    for extracted_path in &extracted.use_paths {
        if extracted_path.module_suffix != module_suffix {
            continue;
        }
        if let Some(name) = matching_export_name_indexed(
            &extracted_path.segments,
            current_module_path,
            module_path,
            exported_names,
        ) {
            merge_reference_usage_for_name(
                &mut usage_by_name,
                name,
                ParentFacadeReferenceUsage::Import(extracted_path.origin),
            );
        }
    }

    usage_by_name
}

/// Resolves the first segment of an `expr_path` through module aliases.
///
/// Given `["test_utils", "assert_test_case"]` and a rename mapping
/// `test_utils → ["crate", "test_support"]`, returns
/// `["crate", "test_support", "assert_test_case"]`.
fn resolve_alias_expr_path(
    raw: &[String],
    module_suffix: &[String],
    renames: &[UseRename],
) -> Option<Vec<String>> {
    let first = raw.first()?;
    let rename = renames
        .iter()
        .find(|rename| rename.module_suffix == module_suffix && rename.alias == *first)?;
    let mut resolved = rename.original_path.clone();
    resolved.extend(raw[1..].iter().cloned());
    Some(resolved)
}

fn matching_export_name_indexed<'name>(
    raw: &[String],
    current_module_path: &[String],
    module_path: &[String],
    exported_names: &'name [String],
) -> Option<&'name str> {
    resolve_module_relative_paths(raw, current_module_path)
        .into_iter()
        .find_map(|segments| {
            (segments.len() == module_path.len() + 1
                && segments[..module_path.len()].iter().zip(module_path).all(
                    |(source_segment, compiler_segment)| {
                        normalized_export_name(source_segment)
                            == normalized_export_name(compiler_segment)
                    },
                ))
            .then(|| {
                exported_names
                    .iter()
                    .find(|name| {
                        normalized_export_name(name)
                            == normalized_export_name(&segments[module_path.len()])
                    })
                    .map(|name| normalized_export_name(name))
            })
            .flatten()
        })
}

fn merge_reference_usage_for_name(
    usage_by_name: &mut HashMap<String, ParentFacadeReferenceUsage>,
    name: &str,
    next: ParentFacadeReferenceUsage,
) {
    let current = usage_by_name
        .get(name)
        .copied()
        .unwrap_or(ParentFacadeReferenceUsage::None);
    usage_by_name.insert(name.to_string(), merge_reference_usage(current, next));
}

pub(super) fn resolve_module_relative_paths(
    raw: &[String],
    current_module_path: &[String],
) -> Vec<Vec<String>> {
    if raw.is_empty() {
        return Vec::new();
    }

    let Some(path_anchor) = PathAnchor::first(raw) else {
        return Vec::new();
    };
    match path_anchor {
        PathAnchor::Crate => return vec![raw[1..].to_vec()],
        PathAnchor::SelfMod => {
            let mut resolved = current_module_path.to_vec();
            resolved.extend(raw[1..].iter().cloned());
            return vec![resolved];
        },
        PathAnchor::Super => {
            let mut index = 0usize;
            let mut resolved = current_module_path.to_vec();
            while raw
                .get(index)
                .is_some_and(|segment| PathAnchor::from(segment.as_str()) == PathAnchor::Super)
            {
                if resolved.pop().is_none() {
                    return Vec::new();
                }
                index += 1;
            }
            if raw
                .get(index)
                .is_some_and(|segment| PathAnchor::from(segment.as_str()) == PathAnchor::SelfMod)
            {
                index += 1;
            }
            resolved.extend(raw[index..].iter().cloned());
            return vec![resolved];
        },
        PathAnchor::SelfType | PathAnchor::Name => {},
    }

    (0..=current_module_path.len())
        .map(|prefix_len| {
            let mut resolved = current_module_path[..prefix_len].to_vec();
            resolved.extend(raw.iter().cloned());
            resolved
        })
        .collect()
}

const fn merge_reference_usage(
    current: ParentFacadeReferenceUsage,
    next: ParentFacadeReferenceUsage,
) -> ParentFacadeReferenceUsage {
    match (current, next) {
        (ParentFacadeReferenceUsage::DirectPath(PathOrigin::Relative), _)
        | (_, ParentFacadeReferenceUsage::DirectPath(PathOrigin::Relative)) => {
            ParentFacadeReferenceUsage::DirectPath(PathOrigin::Relative)
        },
        (ParentFacadeReferenceUsage::Import(PathOrigin::Relative), _)
        | (_, ParentFacadeReferenceUsage::Import(PathOrigin::Relative)) => {
            ParentFacadeReferenceUsage::Import(PathOrigin::Relative)
        },
        (ParentFacadeReferenceUsage::DirectPath(PathOrigin::Crate), _)
        | (_, ParentFacadeReferenceUsage::DirectPath(PathOrigin::Crate)) => {
            ParentFacadeReferenceUsage::DirectPath(PathOrigin::Crate)
        },
        (ParentFacadeReferenceUsage::Import(PathOrigin::Crate), _)
        | (_, ParentFacadeReferenceUsage::Import(PathOrigin::Crate)) => {
            ParentFacadeReferenceUsage::Import(PathOrigin::Crate)
        },
        _ => ParentFacadeReferenceUsage::None,
    }
}

const fn merge_parent_facade_usage(
    current: ParentFacadeUsage,
    next: ParentFacadeUsage,
) -> ParentFacadeUsage {
    match (current, next) {
        (ParentFacadeUsage::UsedOutsideSubtree, _) | (_, ParentFacadeUsage::UsedOutsideSubtree) => {
            ParentFacadeUsage::UsedOutsideSubtree
        },
        (ParentFacadeUsage::UsedInsideSubtreeByRelativePath, _)
        | (_, ParentFacadeUsage::UsedInsideSubtreeByRelativePath) => {
            ParentFacadeUsage::UsedInsideSubtreeByRelativePath
        },
        (ParentFacadeUsage::UsedInsideSubtreeByRelativeImport, _)
        | (_, ParentFacadeUsage::UsedInsideSubtreeByRelativeImport) => {
            ParentFacadeUsage::UsedInsideSubtreeByRelativeImport
        },
        (ParentFacadeUsage::UsedInsideSubtreeByCratePath, _)
        | (_, ParentFacadeUsage::UsedInsideSubtreeByCratePath) => {
            ParentFacadeUsage::UsedInsideSubtreeByCratePath
        },
        (ParentFacadeUsage::UsedInsideSubtreeByCrateImport, _)
        | (_, ParentFacadeUsage::UsedInsideSubtreeByCrateImport) => {
            ParentFacadeUsage::UsedInsideSubtreeByCrateImport
        },
        _ => ParentFacadeUsage::Unused,
    }
}

fn merge_usage_for_name(
    usage_by_name: &mut ParentFacadeUsageByName,
    name: String,
    next: ParentFacadeUsage,
) {
    let current = usage_by_name
        .get(&name)
        .copied()
        .unwrap_or(ParentFacadeUsage::Unused);
    usage_by_name.insert(name, merge_parent_facade_usage(current, next));
}

pub(in crate::compiler) fn path_exists_outside_child_module(
    source_cache: &SourceCache,
    source_root: &Path,
    tcx: TyCtxt<'_>,
    module_sources: &ModuleSourceMap,
    child_module_path: &[String],
    item_name: &str,
) -> bool {
    for source_file in source_cache.source_files_under(source_root) {
        let Some(extracted) = source_cache.extracted_paths(source_file) else {
            continue;
        };
        for (current_module_path, module_suffix) in
            active_module_contexts(extracted, module_sources, tcx, source_file)
        {
            if module_path_is_descendant(&current_module_path, child_module_path) {
                continue;
            }
            if extracted_paths_mention_child_item(
                extracted,
                &current_module_path,
                &module_suffix,
                child_module_path,
                item_name,
            ) {
                return true;
            }
        }
    }

    false
}

pub(in crate::compiler) fn path_exists_outside_module(
    source_cache: &SourceCache,
    source_root: &Path,
    tcx: TyCtxt<'_>,
    module_sources: &ModuleSourceMap,
    module_path: &[String],
    item_names: &[String],
) -> bool {
    for source_file in source_cache.source_files_under(source_root) {
        let Some(extracted) = source_cache.extracted_paths(source_file) else {
            continue;
        };
        for (current_module_path, module_suffix) in
            active_module_contexts(extracted, module_sources, tcx, source_file)
        {
            if module_path_is_descendant(&current_module_path, module_path) {
                continue;
            }
            if source_references_parent_exports(
                extracted,
                &current_module_path,
                &module_suffix,
                module_path,
                item_names,
            )
            .values()
            .any(|usage| !matches!(usage, ParentFacadeReferenceUsage::None))
            {
                return true;
            }
        }
    }
    false
}

fn extracted_paths_mention_child_item(
    extracted: &ExtractedPaths,
    current_module_path: &[String],
    module_suffix: &[String],
    child_module_path: &[String],
    item_name: &str,
) -> bool {
    extracted.use_paths.iter().any(|extracted_path| {
        extracted_path.module_suffix == module_suffix
            && resolved_path_mentions_child_item(
                &extracted_path.segments,
                current_module_path,
                child_module_path,
                item_name,
            )
    }) || extracted.expr_paths.iter().any(|extracted_path| {
        extracted_path.module_suffix == module_suffix
            && (resolved_path_mentions_child_item(
                &extracted_path.segments,
                current_module_path,
                child_module_path,
                item_name,
            ) || resolve_alias_expr_path(
                &extracted_path.segments,
                module_suffix,
                &extracted.use_renames,
            )
            .is_some_and(|resolved| {
                resolved_path_mentions_child_item(
                    &resolved,
                    current_module_path,
                    child_module_path,
                    item_name,
                )
            }))
    })
}

fn active_module_contexts(
    extracted: &ExtractedPaths,
    module_sources: &ModuleSourceMap,
    tcx: TyCtxt<'_>,
    source_file: &Path,
) -> Vec<(Vec<String>, Vec<String>)> {
    let mut contexts = Vec::new();
    for root_module in module_sources.root_modules_for_file(tcx, source_file) {
        let root_path = boundary::module_path(tcx, root_module);
        for module_suffix in lexical_module_suffixes(extracted) {
            let mut current_module_path = root_path.clone();
            current_module_path.extend(module_suffix.iter().cloned());
            if module_sources.file_contains_module_path(tcx, source_file, &current_module_path)
                && !contexts
                    .iter()
                    .any(|(path, _)| path == &current_module_path)
            {
                contexts.push((current_module_path, module_suffix.to_vec()));
            }
        }
    }
    contexts
}

fn lexical_module_suffixes(extracted: &ExtractedPaths) -> Vec<&[String]> {
    let mut suffixes = Vec::new();
    for extracted_path in extracted.use_paths.iter().chain(&extracted.expr_paths) {
        let module_suffix = extracted_path.module_suffix.as_slice();
        if !suffixes.contains(&module_suffix) {
            suffixes.push(module_suffix);
        }
    }
    suffixes
}

fn resolved_path_mentions_child_item(
    path: &[String],
    current_module_path: &[String],
    child_module_path: &[String],
    item_name: &str,
) -> bool {
    resolve_module_relative_paths(path, current_module_path)
        .into_iter()
        .any(|resolved| path_mentions_child_item(&resolved, child_module_path, item_name))
}

fn path_mentions_child_item(
    path: &[String],
    child_module_path: &[String],
    item_name: &str,
) -> bool {
    path.len() > child_module_path.len()
        && path[..child_module_path.len()] == *child_module_path
        && (path[child_module_path.len()] == item_name || path[child_module_path.len()] == "*")
}

fn module_path_is_descendant(candidate: &[String], parent: &[String]) -> bool {
    candidate == parent || (candidate.len() > parent.len() && candidate[..parent.len()] == *parent)
}

#[cfg(test)]
mod tests {
    use super::matching_export_name_indexed;

    #[test]
    fn parsed_path_matches_raw_module_segments() {
        let source_segments = ["crate", "a", "r#type", "r#inner", "Thing"].map(String::from);
        let compiler_segments = ["a", "type", "inner"].map(String::from);
        let exported_names = [String::from("Thing")];

        assert_eq!(
            matching_export_name_indexed(
                &source_segments,
                &[],
                &compiler_segments,
                &exported_names,
            ),
            Some("Thing")
        );
    }
}
