use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use once_cell::sync::Lazy;
use regex::Regex;
use syn::Fields;
use syn::GenericParam;
use syn::ImplItem;
use syn::Item;
use syn::PatType;
use syn::PathArguments;
use syn::ReturnType;
use syn::TraitItem;
use syn::Type;
use walkdir::WalkDir;

use super::config::LoadedConfig;
use super::config::VisibilityConfig;
use super::diagnostics::Finding;
use super::diagnostics::Report;
use super::diagnostics::Severity;
use super::selection::Selection;

static RE_PUB_CRATE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\bpub\s*\(\s*crate\s*\)").unwrap());
static RE_PUB_IN_CRATE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\bpub\s*\(\s*in\s+crate::").unwrap());
static RE_PUB_MOD: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\s*pub\s+mod\s+([A-Za-z_][A-Za-z0-9_]*)\s*(?:;|\{)").unwrap());
static RE_PUBLIC_USE_CHILD_ITEM: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\s*pub(?:\s*\([^)]*\))?\s+use\s+(.+)$").unwrap());
static RE_PUB_FN: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\s*pub\s+(?:async\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap());
static RE_PUB_STRUCT: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\s*pub\s+struct\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap());
static RE_PUB_ENUM: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\s*pub\s+enum\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap());
static RE_PUB_TRAIT: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\s*pub\s+trait\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap());
static RE_PUB_TYPE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\s*pub\s+type\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap());
static RE_PUB_CONST: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\s*pub\s+const\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap());
static RE_PUB_STATIC: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\s*pub\s+static(?:\s+mut)?\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap());

pub(super) fn scan_selection(
    selection: &Selection,
    loaded_config: &LoadedConfig,
) -> Result<Report> {
    let mut findings = Vec::new();
    for package in &selection.packages {
        let crate_root = package
            .manifest_path
            .parent()
            .context("package manifest path had no parent directory")?;
        findings.extend(scan_crate(
            selection.analysis_root.as_path(),
            crate_root.as_std_path(),
            &loaded_config.root,
            &loaded_config.config,
        )?);
    }

    findings.sort_by(|a, b| {
        (a.severity, &a.path, a.line, &a.code).cmp(&(b.severity, &b.path, b.line, &b.code))
    });

    Ok(Report {
        root: selection.analysis_root.display().to_string(),
        findings,
    })
}

fn scan_crate(
    analysis_root: &Path,
    crate_root: &Path,
    config_root: &Path,
    config: &VisibilityConfig,
) -> Result<Vec<Finding>> {
    let src_root = crate_root.join("src");
    if !src_root.is_dir() {
        return Ok(Vec::new());
    }
    let root_module = if src_root.join("lib.rs").is_file() {
        src_root.join("lib.rs")
    } else {
        src_root.join("main.rs")
    };

    let mut source_files = Vec::new();
    let walker = WalkDir::new(&src_root).into_iter().filter_entry(|entry| {
        let name = entry.file_name().to_string_lossy();
        name != "target"
    });

    for entry in walker {
        let entry = entry?;
        if !entry.file_type().is_file()
            || entry.path().extension().and_then(|e| e.to_str()) != Some("rs")
        {
            continue;
        }
        let text = fs::read_to_string(entry.path())
            .with_context(|| format!("failed to read source file {}", entry.path().display()))?;
        source_files.push(SourceFile {
            path: entry.path().to_path_buf(),
            text,
        });
    }

    let mut findings = Vec::new();
    for source in &source_files {
        findings.extend(scan_file(
            analysis_root,
            crate_root,
            &src_root,
            &root_module,
            config_root,
            &source.path,
            &source.text,
            &source_files,
            config,
        )?);
    }

    Ok(findings)
}

struct SourceFile {
    path: PathBuf,
    text: String,
}

fn scan_file(
    root: &Path,
    crate_root: &Path,
    src_root: &Path,
    root_module: &Path,
    config_root: &Path,
    file: &Path,
    text: &str,
    source_files: &[SourceFile],
    config: &VisibilityConfig,
) -> Result<Vec<Finding>> {
    let rel_path = path_relative_to(file, root)?;
    let rel_path_string = rel_path.to_string_lossy().replace('\\', "/");

    let mut findings = Vec::new();
    let module_context = ModuleContext::for_file(crate_root, src_root, root_module, file);
    let same_file_public_api_dependencies = same_file_public_api_dependencies(text);
    let config_rel_path = path_relative_to(file, config_root)
        .ok()
        .map(|p| p.to_string_lossy().replace('\\', "/"));

    let mut depth = 0usize;
    let mut pending_cfg_test = false;
    let mut skip_until_depth: Option<usize> = None;
    for (idx, line) in text.lines().enumerate() {
        let line_no = idx + 1;
        let sanitized = sanitize_for_visibility_checks(line);
        let sanitized_trimmed = sanitized.trim_start();

        if let Some(skip_depth) = skip_until_depth {
            depth = update_brace_depth(depth, line);
            if depth < skip_depth {
                skip_until_depth = None;
            }
            continue;
        }

        if sanitized_trimmed.starts_with("#[cfg(test)]") {
            pending_cfg_test = true;
            depth = update_brace_depth(depth, line);
            continue;
        }

        if pending_cfg_test && sanitized_trimmed.starts_with("mod ") {
            let module_depth = depth;
            depth = update_brace_depth(depth, line);
            skip_until_depth = Some(module_depth + 1);
            pending_cfg_test = false;
            continue;
        }

        if pending_cfg_test && !sanitized_trimmed.is_empty() && !sanitized_trimmed.starts_with("#[")
        {
            pending_cfg_test = false;
        }

        if let Some(matched) = RE_PUB_CRATE.find(&sanitized) {
            let (column, highlight_len) =
                highlight_from_range(line, matched.start(), matched.end());
            findings.push(Finding {
                severity: Severity::Error,
                code: "forbidden_pub_crate".to_string(),
                path: rel_path_string.clone(),
                line: line_no,
                column,
                highlight_len,
                source_line: line.to_string(),
                item: None,
                message: "use of `pub(crate)` is forbidden by policy".to_string(),
            });
        }

        if let Some(matched) = RE_PUB_IN_CRATE.find(&sanitized) {
            let (column, highlight_len) =
                highlight_from_range(line, matched.start(), matched.end());
            findings.push(Finding {
                severity: Severity::Error,
                code: "forbidden_pub_in_crate".to_string(),
                path: rel_path_string.clone(),
                line: line_no,
                column,
                highlight_len,
                source_line: line.to_string(),
                item: None,
                message: "use of `pub(in crate::...)` is forbidden by policy".to_string(),
            });
        }

        if let Some(captures) = RE_PUB_MOD.captures(&sanitized) {
            let allowlisted = config_rel_path.as_ref().is_some_and(|config_rel| {
                config
                    .allow_pub_mod
                    .iter()
                    .any(|allowed| allowed == config_rel)
            });
            if !allowlisted {
                let module_name = captures.get(1).map(|m| m.as_str()).unwrap_or_default();
                let start = sanitized.find("pub mod").unwrap_or(0);
                let end = sanitized
                    .find(module_name)
                    .map(|index| index + module_name.len())
                    .unwrap_or_else(|| start + "pub mod".len());
                let (column, highlight_len) = highlight_from_range(line, start, end);
                findings.push(Finding {
                    severity: Severity::Error,
                    code: "review_pub_mod".to_string(),
                    path: rel_path_string.clone(),
                    line: line_no,
                    column,
                    highlight_len,
                    source_line: line.to_string(),
                    item: captures.get(1).map(|m| m.as_str().to_string()),
                    message: "`pub mod` requires explicit review or allowlisting".to_string(),
                });
            }
        }

        if depth == 0
            && sanitized_trimmed.starts_with("pub ")
            && !sanitized_trimmed.starts_with("pub mod")
            && !sanitized_trimmed.starts_with("pub use")
        {
            if let Some((kind, name)) = bare_pub_item(sanitized_trimmed) {
                let item_key = config_rel_path
                    .as_ref()
                    .map(|path| format!("{path}::{name}"));
                let allowlisted = item_key
                    .as_ref()
                    .is_some_and(|key| config.allow_pub_items.iter().any(|allowed| allowed == key));
                if !allowlisted {
                    if let Some(reason) = suspicious_pub_reason(
                        &module_context,
                        &name,
                        file,
                        source_files,
                        &same_file_public_api_dependencies,
                    )? {
                        let start = sanitized.find("pub").unwrap_or(0);
                        let end = sanitized
                            .find(&name)
                            .map(|index| index + name.len())
                            .unwrap_or_else(|| line.len());
                        let (column, highlight_len) = highlight_from_range(line, start, end);
                        findings.push(Finding {
                            severity: Severity::Warning,
                            code: "suspicious_bare_pub".to_string(),
                            path: rel_path_string.clone(),
                            line: line_no,
                            column,
                            highlight_len,
                            source_line: line.to_string(),
                            item: Some(format!("{kind} {name}")),
                            message: reason,
                        });
                    }
                }
            }
        }

        depth = update_brace_depth(depth, line);
    }

    Ok(findings)
}

fn suspicious_pub_reason(
    module_context: &ModuleContext,
    item_name: &str,
    file: &Path,
    source_files: &[SourceFile],
    same_file_public_api_dependencies: &BTreeSet<String>,
) -> Result<Option<String>> {
    if module_context.is_root_or_boundary_file {
        return Ok(None);
    }

    if same_file_public_api_dependencies.contains(item_name) {
        return Ok(None);
    }

    let used_elsewhere = item_name_used_elsewhere(item_name, file, source_files)?;
    if !module_context.parent_module_is_public
        && !module_context.parent_publicly_reexports(item_name)?
        && !used_elsewhere
    {
        return Ok(Some(
            "bare `pub` item lives in a non-root child module whose parent module is private, is not publicly re-exported by that parent, and appears unused outside its defining file"
                .to_string(),
        ));
    }

    Ok(None)
}

fn same_file_public_api_dependencies(text: &str) -> BTreeSet<String> {
    let Ok(file) = syn::parse_file(text) else {
        return BTreeSet::new();
    };

    let public_item_names = collect_public_item_names(&file.items);
    let mut names = BTreeSet::new();
    for item in &file.items {
        collect_public_item_dependencies(item, &public_item_names, &mut names);
    }
    names
}

fn collect_public_item_names(items: &[Item]) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for item in items {
        match item {
            Item::Const(item) if visibility_is_public(&item.vis) => {
                names.insert(item.ident.to_string());
            },
            Item::Enum(item) if visibility_is_public(&item.vis) => {
                names.insert(item.ident.to_string());
            },
            Item::Fn(item) if visibility_is_public(&item.vis) => {
                names.insert(item.sig.ident.to_string());
            },
            Item::Static(item) if visibility_is_public(&item.vis) => {
                names.insert(item.ident.to_string());
            },
            Item::Struct(item) if visibility_is_public(&item.vis) => {
                names.insert(item.ident.to_string());
            },
            Item::Trait(item) if visibility_is_public(&item.vis) => {
                names.insert(item.ident.to_string());
            },
            Item::Type(item) if visibility_is_public(&item.vis) => {
                names.insert(item.ident.to_string());
            },
            _ => {},
        }
    }
    names
}

fn collect_public_item_dependencies(
    item: &Item,
    public_item_names: &BTreeSet<String>,
    names: &mut BTreeSet<String>,
) {
    match item {
        Item::Const(item) if visibility_is_public(&item.vis) => collect_type_names(&item.ty, names),
        Item::Enum(item) if visibility_is_public(&item.vis) => {
            for variant in &item.variants {
                collect_fields_type_names(&variant.fields, names);
            }
        },
        Item::Fn(item) if visibility_is_public(&item.vis) => {
            collect_signature_type_names(&item.sig, names);
        },
        Item::Static(item) if visibility_is_public(&item.vis) => {
            collect_type_names(&item.ty, names)
        },
        Item::Struct(item) if visibility_is_public(&item.vis) => {
            collect_public_struct_fields(&item.fields, names);
        },
        Item::Trait(item) if visibility_is_public(&item.vis) => {
            collect_trait_type_names(item, names);
        },
        Item::Type(item) if visibility_is_public(&item.vis) => {
            collect_type_names(&item.ty, names);
        },
        Item::Impl(item) => {
            if let Some((_, path, _)) = &item.trait_ {
                collect_path_segment_names(path, names);
            }
            collect_type_names(&item.self_ty, names);
            if impl_exposes_public_api(item, public_item_names) {
                for impl_item in &item.items {
                    match impl_item {
                        ImplItem::Const(item) => collect_type_names(&item.ty, names),
                        ImplItem::Fn(item) => collect_signature_type_names(&item.sig, names),
                        ImplItem::Type(item) => collect_type_names(&item.ty, names),
                        _ => {},
                    }
                }
            }
            for impl_item in &item.items {
                if let ImplItem::Fn(method) = impl_item
                    && visibility_is_public(&method.vis)
                {
                    collect_signature_type_names(&method.sig, names);
                }
            }
        },
        _ => {},
    }
}

fn impl_exposes_public_api(item: &syn::ItemImpl, public_item_names: &BTreeSet<String>) -> bool {
    if item.trait_.is_none() {
        return false;
    }

    let Some(self_ty_name) = type_terminal_ident(&item.self_ty) else {
        return false;
    };

    public_item_names.contains(&self_ty_name)
}

fn type_terminal_ident(ty: &Type) -> Option<String> {
    match ty {
        Type::Group(ty) => type_terminal_ident(&ty.elem),
        Type::Paren(ty) => type_terminal_ident(&ty.elem),
        Type::Path(ty) => ty
            .path
            .segments
            .last()
            .map(|segment| segment.ident.to_string()),
        Type::Reference(ty) => type_terminal_ident(&ty.elem),
        _ => None,
    }
}

fn collect_public_struct_fields(fields: &Fields, names: &mut BTreeSet<String>) {
    match fields {
        Fields::Named(fields) => {
            for field in &fields.named {
                if visibility_is_public(&field.vis) {
                    collect_type_names(&field.ty, names);
                }
            }
        },
        Fields::Unnamed(fields) => {
            for field in &fields.unnamed {
                if visibility_is_public(&field.vis) {
                    collect_type_names(&field.ty, names);
                }
            }
        },
        Fields::Unit => {},
    }
}

fn collect_fields_type_names(fields: &Fields, names: &mut BTreeSet<String>) {
    match fields {
        Fields::Named(fields) => {
            for field in &fields.named {
                collect_type_names(&field.ty, names);
            }
        },
        Fields::Unnamed(fields) => {
            for field in &fields.unnamed {
                collect_type_names(&field.ty, names);
            }
        },
        Fields::Unit => {},
    }
}

fn collect_signature_type_names(signature: &syn::Signature, names: &mut BTreeSet<String>) {
    for generic in &signature.generics.params {
        match generic {
            GenericParam::Type(generic) => {
                for bound in &generic.bounds {
                    if let syn::TypeParamBound::Trait(bound) = bound {
                        collect_path_segment_names(&bound.path, names);
                    }
                }
            },
            GenericParam::Const(generic) => collect_type_names(&generic.ty, names),
            GenericParam::Lifetime(_) => {},
        }
    }

    for input in &signature.inputs {
        match input {
            syn::FnArg::Receiver(_) => {},
            syn::FnArg::Typed(PatType { ty, .. }) => collect_type_names(ty, names),
        }
    }

    if let ReturnType::Type(_, ty) = &signature.output {
        collect_type_names(ty, names);
    }
}

fn collect_trait_type_names(item: &syn::ItemTrait, names: &mut BTreeSet<String>) {
    for bound in &item.supertraits {
        if let syn::TypeParamBound::Trait(bound) = bound {
            collect_path_segment_names(&bound.path, names);
        }
    }

    for trait_item in &item.items {
        match trait_item {
            TraitItem::Const(item) => collect_type_names(&item.ty, names),
            TraitItem::Fn(item) => collect_signature_type_names(&item.sig, names),
            TraitItem::Type(item) => {
                for bound in &item.bounds {
                    if let syn::TypeParamBound::Trait(bound) = bound {
                        collect_path_segment_names(&bound.path, names);
                    }
                }
            },
            _ => {},
        }
    }
}

fn collect_type_names(ty: &Type, names: &mut BTreeSet<String>) {
    match ty {
        Type::Array(ty) => collect_type_names(&ty.elem, names),
        Type::BareFn(ty) => {
            for input in &ty.inputs {
                collect_type_names(&input.ty, names);
            }
            if let ReturnType::Type(_, ty) = &ty.output {
                collect_type_names(ty, names);
            }
        },
        Type::Group(ty) => collect_type_names(&ty.elem, names),
        Type::ImplTrait(ty) => {
            for bound in &ty.bounds {
                if let syn::TypeParamBound::Trait(bound) = bound {
                    collect_path_segment_names(&bound.path, names);
                }
            }
        },
        Type::Macro(_) => {},
        Type::Paren(ty) => collect_type_names(&ty.elem, names),
        Type::Path(ty) => collect_type_path_names(&ty.path, names),
        Type::Ptr(ty) => collect_type_names(&ty.elem, names),
        Type::Reference(ty) => collect_type_names(&ty.elem, names),
        Type::Slice(ty) => collect_type_names(&ty.elem, names),
        Type::TraitObject(ty) => {
            for bound in &ty.bounds {
                if let syn::TypeParamBound::Trait(bound) = bound {
                    collect_path_segment_names(&bound.path, names);
                }
            }
        },
        Type::Tuple(ty) => {
            for elem in &ty.elems {
                collect_type_names(elem, names);
            }
        },
        _ => {},
    }
}

fn collect_type_path_names(path: &syn::Path, names: &mut BTreeSet<String>) {
    collect_path_segment_names(path, names);

    for segment in &path.segments {
        if let PathArguments::AngleBracketed(arguments) = &segment.arguments {
            for arg in &arguments.args {
                match arg {
                    syn::GenericArgument::Type(ty) => collect_type_names(ty, names),
                    syn::GenericArgument::AssocType(binding) => {
                        collect_type_names(&binding.ty, names);
                    },
                    syn::GenericArgument::Constraint(constraint) => {
                        for bound in &constraint.bounds {
                            if let syn::TypeParamBound::Trait(bound) = bound {
                                collect_path_segment_names(&bound.path, names);
                            }
                        }
                    },
                    _ => {},
                }
            }
        }
    }
}

fn collect_path_segment_names(path: &syn::Path, names: &mut BTreeSet<String>) {
    for segment in &path.segments {
        names.insert(segment.ident.to_string());
    }
}

fn visibility_is_public(visibility: &syn::Visibility) -> bool {
    matches!(visibility, syn::Visibility::Public(_))
}

#[cfg(test)]
mod tests {
    use super::same_file_public_api_dependencies;

    #[test]
    fn tracks_associated_types_used_by_public_trait_impls() {
        let deps = same_file_public_api_dependencies(
            r#"
pub trait ToolLike {
    type Output;
}

pub struct PublicTool;

pub struct AssociatedOutput;

impl ToolLike for PublicTool {
    type Output = AssociatedOutput;
}
"#,
        );

        assert!(deps.contains("AssociatedOutput"));
    }
}

fn item_name_used_elsewhere(
    item_name: &str,
    file: &Path,
    source_files: &[SourceFile],
) -> Result<bool> {
    let pattern = Regex::new(&format!(r"\b{}\b", regex::escape(item_name)))?;
    Ok(source_files
        .iter()
        .filter(|source| source.path != file)
        .any(|source| pattern.is_match(&source.text)))
}

fn sanitize_for_visibility_checks(line: &str) -> String {
    let mut result = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;

    while let Some(ch) = chars.next() {
        if !in_string && ch == '/' && chars.peek() == Some(&'/') {
            break;
        }

        if in_string {
            if escaped {
                escaped = false;
                result.push(' ');
                continue;
            }

            match ch {
                '\\' => {
                    escaped = true;
                    result.push(' ');
                },
                '"' => {
                    in_string = false;
                    result.push(' ');
                },
                _ => result.push(' '),
            }
            continue;
        }

        if ch == '"' {
            in_string = true;
            result.push(' ');
            continue;
        }

        result.push(ch);
    }

    result
}

fn highlight_from_range(line: &str, start: usize, end: usize) -> (usize, usize) {
    let safe_start = start.min(line.len());
    let safe_end = end.max(safe_start).min(line.len());
    let column = line[..safe_start].chars().count() + 1;
    let highlight_len = line[safe_start..safe_end].chars().count().max(1);
    (column, highlight_len)
}

fn bare_pub_item(trimmed: &str) -> Option<(&'static str, String)> {
    let candidates = [
        ("fn", &*RE_PUB_FN),
        ("struct", &*RE_PUB_STRUCT),
        ("enum", &*RE_PUB_ENUM),
        ("trait", &*RE_PUB_TRAIT),
        ("type", &*RE_PUB_TYPE),
        ("const", &*RE_PUB_CONST),
        ("static", &*RE_PUB_STATIC),
    ];

    for (kind, re) in candidates {
        if let Some(captures) = re.captures(trimmed) {
            return captures.get(1).map(|m| (kind, m.as_str().to_string()));
        }
    }

    None
}

fn update_brace_depth(mut depth: usize, line: &str) -> usize {
    for ch in line.chars() {
        match ch {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            _ => {},
        }
    }
    depth
}

fn path_relative_to<'a>(path: &'a Path, root: &Path) -> Result<&'a Path> {
    path.strip_prefix(root).with_context(|| {
        format!(
            "failed to make {} relative to {}",
            path.display(),
            root.display()
        )
    })
}

#[derive(Debug, Clone)]
struct ModuleContext {
    parent_file:              Option<PathBuf>,
    child_module_name:        Option<String>,
    parent_module_is_public:  bool,
    is_root_or_boundary_file: bool,
}

impl ModuleContext {
    fn for_file(crate_root: &Path, src_root: &Path, root_module: &Path, file: &Path) -> Self {
        let rel_to_src = file.strip_prefix(src_root).unwrap();
        let is_root_file = file == root_module;
        let is_mod_rs = file.file_name().and_then(|n| n.to_str()) == Some("mod.rs");
        let is_top_level_file = rel_to_src.components().count() == 1;

        if is_root_file || is_mod_rs || is_top_level_file {
            return Self {
                parent_file:              None,
                child_module_name:        None,
                parent_module_is_public:  false,
                is_root_or_boundary_file: true,
            };
        }

        let child_module_name = file
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap()
            .to_string();
        let parent_dir = file.parent().unwrap();
        let parent_file = if parent_dir == src_root {
            root_module.to_path_buf()
        } else {
            let mod_rs = parent_dir.join("mod.rs");
            if mod_rs.is_file() {
                mod_rs
            } else {
                let parent_module_name = parent_dir.file_name().and_then(|s| s.to_str()).unwrap();
                crate_root
                    .join("src")
                    .join(format!("{parent_module_name}.rs"))
            }
        };

        let parent_text = fs::read_to_string(&parent_file).unwrap_or_default();
        let parent_module_is_public =
            parent_declares_public_module(&parent_text, &child_module_name);

        Self {
            parent_file: Some(parent_file),
            child_module_name: Some(child_module_name),
            parent_module_is_public,
            is_root_or_boundary_file: false,
        }
    }

    fn parent_publicly_reexports(&self, item_name: &str) -> Result<bool> {
        let (Some(parent_file), Some(child_module_name)) =
            (&self.parent_file, &self.child_module_name)
        else {
            return Ok(false);
        };

        let text = fs::read_to_string(parent_file)
            .with_context(|| format!("failed to read parent module {}", parent_file.display()))?;

        for line in text.lines() {
            let Some(captures) = RE_PUBLIC_USE_CHILD_ITEM.captures(line) else {
                continue;
            };

            let body = captures.get(1).map(|m| m.as_str()).unwrap_or_default();
            if !body.contains(child_module_name) {
                continue;
            }

            if body.contains(&format!("{child_module_name}::*")) {
                return Ok(true);
            }

            if body.contains(&format!("{child_module_name}::{item_name}")) {
                return Ok(true);
            }

            if body.contains(&format!("{child_module_name}::{{")) && body.contains(item_name) {
                return Ok(true);
            }
        }

        Ok(false)
    }
}

fn parent_declares_public_module(parent_text: &str, child_module_name: &str) -> bool {
    let exact = format!("pub mod {child_module_name}");
    parent_text.lines().any(|line| line.contains(&exact))
}
