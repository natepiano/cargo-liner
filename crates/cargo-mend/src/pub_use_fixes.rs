use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use syn::Item;
use syn::ItemUse;
use syn::UseTree;
use syn::parse_file;
use syn::spanned::Spanned;
use syn::visit::Visit;

use super::diagnostics::Report;
use super::imports::UseFix;
use super::selection::Selection;

pub struct PubUseFixScan {
    pub fixes:         Vec<UseFix>,
    pub applied_count: usize,
}

struct Candidate {
    parent_mod:       PathBuf,
    parent_line:      usize,
    child_file:       PathBuf,
    child_line:       usize,
    exported_name:    String,
    parent_mod_path:  Vec<String>,
    target_item_path: Vec<String>,
}

enum CandidateScreening {
    Accept(Candidate),
    Skip,
}

pub fn scan_selection(selection: &Selection, report: &Report) -> Result<PubUseFixScan> {
    let mut fixes = Vec::new();
    let mut applied_pairs = 0usize;

    for candidate in collect_candidates(selection, report)? {
        let pair_fixes = fixes_for_candidate(selection, &candidate)?;
        if pair_fixes.is_empty() {
            continue;
        }
        applied_pairs += 1;
        fixes.extend(pair_fixes);
    }

    Ok(PubUseFixScan {
        fixes,
        applied_count: applied_pairs,
    })
}

fn collect_candidates(selection: &Selection, report: &Report) -> Result<Vec<Candidate>> {
    let mut candidates = Vec::new();
    for finding in &report.findings {
        if finding.code != "suspicious_pub" {
            continue;
        }
        if finding.fix_kind != Some(super::diagnostics::FixKind::ParentPubUse) {
            continue;
        }

        let child_rel = normalize_rel_path(&finding.path);
        let child_file = selection.analysis_root.join(&child_rel);
        let child_line = finding.line;
        let child_source = fs::read_to_string(&child_file)
            .with_context(|| format!("failed to read {}", child_file.display()))?;
        let child_item =
            item_name_from_child_pub(&child_source, child_line).with_context(|| {
                format!("failed to resolve child item from {child_rel}:{child_line}")
            })?;

        let parent_note = finding
            .related
            .as_deref()
            .and_then(|text| {
                text.strip_prefix(
                    "parent module also has an `unused import` warning for this `pub use` at ",
                )
            })
            .context("missing parent `unused import` pairing note for suspicious pub finding")?;
        let (parent_rel_path, parent_line) = split_rel_path_and_line(parent_note)?;
        let parent_mod = resolve_reported_path(selection, &parent_rel_path).with_context(|| {
            format!(
                "failed to resolve parent module path {}",
                parent_rel_path.display()
            )
        })?;
        let parent_source = fs::read_to_string(&parent_mod)
            .with_context(|| format!("failed to read {}", parent_mod.display()))?;
        let exported_name = item_name_from_parent_pub_use(&parent_source, parent_line)
            .with_context(|| {
                format!(
                    "failed to resolve exported item from {}:{}",
                    parent_rel_path.display(),
                    parent_line
                )
            })?;

        let src_root =
            find_src_root(&parent_mod).context("failed to determine src root for parent module")?;

        let child_module = child_file
            .file_stem()
            .and_then(|stem| stem.to_str())
            .filter(|stem| *stem != "mod")
            .context("child pub-use fix currently requires a non-mod child file")?
            .to_string();
        let parent_dir = parent_mod
            .parent()
            .context("parent mod.rs had no parent directory")?;
        let parent_mod_path = module_path_from_dir(&src_root, parent_dir)
            .context("failed to determine parent module path")?;
        let mut target_item_path = parent_mod_path.clone();
        target_item_path.push(child_module.clone());
        target_item_path.push(child_item.clone());

        let candidate = Candidate {
            parent_mod,
            parent_line,
            child_file,
            child_line,
            exported_name,
            parent_mod_path,
            target_item_path,
        };
        match screen_candidate(candidate, &child_item, &child_source)? {
            CandidateScreening::Accept(candidate) => candidates.push(candidate),
            CandidateScreening::Skip => {},
        }
    }

    Ok(candidates)
}

fn screen_candidate(
    candidate: Candidate,
    child_item: &str,
    child_source: &str,
) -> Result<CandidateScreening> {
    let export_match = if candidate.exported_name == child_item {
        CandidateExportMatch::Matches
    } else {
        CandidateExportMatch::Mismatch
    };
    let child_visibility = if line_contains_plain_pub(child_source, candidate.child_line)? {
        ChildVisibilityState::PlainPub
    } else {
        ChildVisibilityState::AlreadyNarrowed
    };

    Ok(match (export_match, child_visibility) {
        (CandidateExportMatch::Matches, ChildVisibilityState::PlainPub) => {
            CandidateScreening::Accept(candidate)
        },
        _ => CandidateScreening::Skip,
    })
}

enum CandidateExportMatch {
    Matches,
    Mismatch,
}

enum ChildVisibilityState {
    PlainPub,
    AlreadyNarrowed,
}

fn fixes_for_candidate(selection: &Selection, candidate: &Candidate) -> Result<Vec<UseFix>> {
    let mut fixes = Vec::new();
    let removal = build_parent_pub_use_removal(candidate)?;
    fixes.push(removal);

    let child_narrowing = build_child_pub_super_fix(candidate)?;
    fixes.push(child_narrowing);

    let parent_dir = candidate
        .parent_mod
        .parent()
        .context("candidate parent mod.rs had no parent directory")?;
    for file in rust_source_files(parent_dir)? {
        if file == candidate.child_file || file == candidate.parent_mod {
            continue;
        }
        fixes.extend(rewrite_in_subtree_imports(
            &selection.analysis_root,
            &candidate.parent_mod_path,
            &candidate.target_item_path,
            &candidate.exported_name,
            &file,
        )?);
    }

    dedup_fixes(&mut fixes);
    Ok(fixes)
}

fn build_parent_pub_use_removal(candidate: &Candidate) -> Result<UseFix> {
    let source = fs::read_to_string(&candidate.parent_mod)
        .with_context(|| format!("failed to read {}", candidate.parent_mod.display()))?;
    let line_span = line_span(&source, candidate.parent_line)
        .context("failed to compute parent pub use line span")?;
    Ok(UseFix {
        path:        candidate.parent_mod.clone(),
        start:       line_span.0,
        end:         line_span.1,
        replacement: String::new(),
    })
}

fn build_child_pub_super_fix(candidate: &Candidate) -> Result<UseFix> {
    let source = fs::read_to_string(&candidate.child_file)
        .with_context(|| format!("failed to read {}", candidate.child_file.display()))?;
    let line_span = line_span(&source, candidate.child_line)
        .context("failed to compute child visibility line span")?;
    let line_text = &source[line_span.0..line_span.1];
    let Some(relative_start) = line_text.find("pub ") else {
        anyhow::bail!(
            "child item line {} does not contain a plain `pub ` prefix",
            candidate.child_line
        );
    };
    Ok(UseFix {
        path:        candidate.child_file.clone(),
        start:       line_span.0 + relative_start,
        end:         line_span.0 + relative_start + 4,
        replacement: "pub(super) ".to_string(),
    })
}

fn line_contains_plain_pub(source: &str, line: usize) -> Result<bool> {
    let line_span = line_span(source, line).context("failed to compute child item line span")?;
    Ok(source[line_span.0..line_span.1].contains("pub "))
}

fn rewrite_in_subtree_imports(
    analysis_root: &Path,
    parent_mod_path: &[String],
    target_item_path: &[String],
    exported_name: &str,
    file: &Path,
) -> Result<Vec<UseFix>> {
    let source =
        fs::read_to_string(file).with_context(|| format!("failed to read {}", file.display()))?;
    let syntax =
        parse_file(&source).with_context(|| format!("failed to parse {}", file.display()))?;
    let src_root = find_src_root(file).with_context(|| {
        format!(
            "failed to determine src root for subtree file {} under {}",
            file.display(),
            analysis_root.display()
        )
    })?;
    let base_module_path = file_module_path(&src_root, file)
        .with_context(|| format!("failed to determine module path for {}", file.display()))?;
    let offsets = line_offsets(&source);
    let mut visitor = PubUseFixVisitor {
        file,
        source: &source,
        offsets: &offsets,
        current_module_path: base_module_path,
        parent_mod_path,
        target_item_path,
        exported_name,
        fixes: Vec::new(),
    };
    visitor.visit_file(&syntax);
    Ok(visitor.fixes)
}

struct PubUseFixVisitor<'a> {
    file:                &'a Path,
    source:              &'a str,
    offsets:             &'a [usize],
    current_module_path: Vec<String>,
    parent_mod_path:     &'a [String],
    target_item_path:    &'a [String],
    exported_name:       &'a str,
    fixes:               Vec<UseFix>,
}

impl Visit<'_> for PubUseFixVisitor<'_> {
    fn visit_item_mod(&mut self, node: &syn::ItemMod) {
        if let Some((_, items)) = &node.content {
            self.current_module_path.push(node.ident.to_string());
            for item in items {
                self.visit_item(item);
            }
            self.current_module_path.pop();
        }
    }

    fn visit_item_use(&mut self, node: &ItemUse) {
        if let Some(replacement) = rewrite_use_tree(
            &self.current_module_path,
            &node.tree,
            self.parent_mod_path,
            self.target_item_path,
            self.exported_name,
        ) {
            let span = node.span();
            let start = offset(self.offsets, span.start());
            let end = offset(self.offsets, span.end());
            let original_item = &self.source[start..end];
            let rewritten =
                original_item.replacen(&replacement.original, &replacement.rewritten, 1);
            self.fixes.push(UseFix {
                path: self.file.to_path_buf(),
                start,
                end,
                replacement: rewritten,
            });
        }
    }
}

struct UseRewrite {
    original:  String,
    rewritten: String,
}

fn rewrite_use_tree(
    current_module_path: &[String],
    tree: &UseTree,
    parent_mod_path: &[String],
    target_item_path: &[String],
    exported_name: &str,
) -> Option<UseRewrite> {
    let import = flatten_use_tree(tree)?;
    let absolute = absolute_use_path(current_module_path, &import.segments)?;
    let expected: Vec<String> = parent_mod_path
        .iter()
        .cloned()
        .chain(std::iter::once(exported_name.to_string()))
        .collect();
    if absolute != expected {
        return None;
    }

    let rewritten = relative_path_from_module(
        current_module_path,
        target_item_path,
        import.rename.as_deref(),
    );
    (rewritten != import.original).then_some(UseRewrite {
        original: import.original,
        rewritten,
    })
}

struct FlattenedImport {
    segments: Vec<String>,
    original: String,
    rename:   Option<String>,
}

fn flatten_use_tree(tree: &UseTree) -> Option<FlattenedImport> {
    let mut segments = Vec::new();
    let mut rename = None;
    let mut cursor = tree;
    loop {
        match cursor {
            UseTree::Path(path) => {
                segments.push(path.ident.to_string());
                cursor = &path.tree;
            },
            UseTree::Name(name) => {
                segments.push(name.ident.to_string());
                break;
            },
            UseTree::Rename(rename_tree) => {
                segments.push(rename_tree.ident.to_string());
                rename = Some(rename_tree.rename.to_string());
                break;
            },
            _ => return None,
        }
    }
    Some(FlattenedImport {
        original: format_path(&segments, rename.as_deref()),
        segments,
        rename,
    })
}

fn absolute_use_path(current_module_path: &[String], segments: &[String]) -> Option<Vec<String>> {
    let first = segments.first()?.as_str();
    match first {
        "crate" => Some(segments[1..].to_vec()),
        "self" => Some(
            current_module_path
                .iter()
                .cloned()
                .chain(segments[1..].iter().cloned())
                .collect(),
        ),
        "super" => {
            let mut module = current_module_path.to_vec();
            let mut index = 0usize;
            while segments.get(index).is_some_and(|seg| seg == "super") {
                module.pop()?;
                index += 1;
            }
            Some(
                module
                    .into_iter()
                    .chain(segments[index..].iter().cloned())
                    .collect(),
            )
        },
        _ => Some(
            current_module_path
                .iter()
                .cloned()
                .chain(segments.iter().cloned())
                .collect(),
        ),
    }
}

fn relative_path_from_module(
    current_module_path: &[String],
    target_path: &[String],
    rename: Option<&str>,
) -> String {
    let common = common_prefix_len(current_module_path, target_path);
    let up_count = current_module_path.len().saturating_sub(common);
    let mut segments = Vec::new();
    for _ in 0..up_count {
        segments.push("super".to_string());
    }
    segments.extend(target_path[common..].iter().cloned());
    format_path(&segments, rename)
}

fn format_path(segments: &[String], rename: Option<&str>) -> String {
    let mut path = segments.join("::");
    if let Some(rename) = rename {
        path.push_str(" as ");
        path.push_str(rename);
    }
    path
}

fn common_prefix_len(left: &[String], right: &[String]) -> usize {
    left.iter()
        .zip(right.iter())
        .take_while(|(l, r)| l == r)
        .count()
}

fn file_module_path(src_root: &Path, path: &Path) -> Option<Vec<String>> {
    let relative = path.strip_prefix(src_root).ok()?;
    let stem = path.file_stem()?.to_str()?;
    let mut result: Vec<String> = relative
        .parent()
        .into_iter()
        .flat_map(|parent| parent.iter())
        .filter_map(|segment| segment.to_str().map(str::to_string))
        .collect();
    if stem != "lib" && stem != "main" && stem != "mod" {
        result.push(stem.to_string());
    }
    Some(result)
}

fn line_offsets(text: &str) -> Vec<usize> {
    let mut offsets = vec![0];
    for (idx, ch) in text.char_indices() {
        if ch == '\n' {
            offsets.push(idx + 1);
        }
    }
    offsets
}

fn offset(line_offsets: &[usize], position: proc_macro2::LineColumn) -> usize {
    line_offsets
        .get(position.line.saturating_sub(1))
        .copied()
        .unwrap_or(0)
        + position.column
}

fn line_span(source: &str, line: usize) -> Option<(usize, usize)> {
    let offsets = line_offsets(source);
    let start = *offsets.get(line.saturating_sub(1))?;
    let end = offsets.get(line).copied().unwrap_or(source.len());
    Some((start, end))
}

fn item_name_from_parent_pub_use(source: &str, line: usize) -> Result<String> {
    let file = parse_file(source).context("failed to parse parent module file")?;
    for item in file.items {
        let Item::Use(item_use) = item else {
            continue;
        };
        if !matches!(item_use.vis, syn::Visibility::Public(_)) {
            continue;
        }
        let use_line = item_use.span().start().line;
        if use_line != line {
            continue;
        }
        let import = flatten_use_tree(&item_use.tree)
            .context("parent pub use fix currently supports only simple pub use items")?;
        if import.segments.len() < 2 {
            anyhow::bail!("parent pub use fix requires a child-module path");
        }
        if import.rename.is_some() {
            anyhow::bail!("parent pub use fix does not support renamed pub uses yet");
        }
        let Some(last_segment) = import.segments.last() else {
            anyhow::bail!("flattened import unexpectedly had no tail segment");
        };
        return Ok(last_segment.clone());
    }
    anyhow::bail!("matching pub use item not found on line {line}")
}

fn item_name_from_child_pub(source: &str, line: usize) -> Result<String> {
    let file = parse_file(source).context("failed to parse child module file")?;
    for item in file.items {
        match item {
            Item::Const(item) if span_contains_line(item.span(), line) => {
                return Ok(item.ident.to_string());
            },
            Item::Enum(item) if span_contains_line(item.span(), line) => {
                return Ok(item.ident.to_string());
            },
            Item::Fn(item) if span_contains_line(item.span(), line) => {
                return Ok(item.sig.ident.to_string());
            },
            Item::Static(item) if span_contains_line(item.span(), line) => {
                return Ok(item.ident.to_string());
            },
            Item::Struct(item) if span_contains_line(item.span(), line) => {
                return Ok(item.ident.to_string());
            },
            Item::Trait(item) if span_contains_line(item.span(), line) => {
                return Ok(item.ident.to_string());
            },
            Item::Type(item) if span_contains_line(item.span(), line) => {
                return Ok(item.ident.to_string());
            },
            Item::Union(item) if span_contains_line(item.span(), line) => {
                return Ok(item.ident.to_string());
            },
            _ => {},
        }
    }
    anyhow::bail!("matching public child item not found on line {line}")
}

fn span_contains_line(span: proc_macro2::Span, line: usize) -> bool {
    let start = span.start().line;
    let end = span.end().line;
    (start..=end).contains(&line)
}

fn split_rel_path_and_line(text: &str) -> Result<(PathBuf, usize)> {
    let Some((path, line)) = text.rsplit_once(':') else {
        anyhow::bail!("expected path:line, got `{text}`");
    };
    Ok((PathBuf::from(path), line.parse()?))
}

fn normalize_rel_path(path: impl AsRef<Path>) -> String {
    path.as_ref().to_string_lossy().replace('\\', "/")
}

fn resolve_reported_path(selection: &Selection, rel_path: &Path) -> Result<PathBuf> {
    let direct = selection.analysis_root.join(rel_path);
    if direct.is_file() {
        return Ok(direct);
    }

    let src_prefixed = selection.analysis_root.join("src").join(rel_path);
    if src_prefixed.is_file() {
        return Ok(src_prefixed);
    }

    anyhow::bail!(
        "reported path {} did not resolve under {}",
        rel_path.display(),
        selection.analysis_root.display()
    );
}

fn module_path_from_dir(src_root: &Path, module_dir: &Path) -> Option<Vec<String>> {
    let relative = module_dir.strip_prefix(src_root).ok()?;
    let components = relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    (!components.is_empty()).then_some(components)
}

fn rust_source_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    collect_rust_source_files(dir, &mut files)?;
    Ok(files)
}

fn find_src_root(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .find(|ancestor| ancestor.file_name().and_then(|name| name.to_str()) == Some("src"))
        .map(Path::to_path_buf)
}

fn collect_rust_source_files(dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir)
        .with_context(|| format!("failed to read source directory {}", dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_rust_source_files(&path, files)?;
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            files.push(path);
        }
    }
    Ok(())
}

fn dedup_fixes(fixes: &mut Vec<UseFix>) {
    let mut seen = BTreeSet::new();
    fixes.retain(|fix| {
        seen.insert((
            fix.path.clone(),
            fix.start,
            fix.end,
            fix.replacement.clone(),
        ))
    });
}
