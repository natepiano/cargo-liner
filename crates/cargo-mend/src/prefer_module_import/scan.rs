use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs;
use std::path::Path;

use anyhow::Context;
use anyhow::Result;
use syn::File;
use syn::Item;
use syn::spanned::Spanned;

use super::function_imports::ImportDetector;
use super::function_imports::ImportTarget;
use super::function_imports::RawCandidate;
use super::inline_calls;
use super::inline_calls::InlineCallCandidate;
use super::inline_calls::InlineCallDetector;
use super::references::BareReference;
use super::references::ReferenceCollector;
use super::shared;
use crate::config::DiagnosticCode;
use crate::constants::PATH_KEYWORD_SUPER;
use crate::constants::RUST_SOURCE_FILE_EXTENSION;
use crate::constants::SOURCE_DIR_SRC;
use crate::diagnostics::Finding;
use crate::diagnostics::Severity;
use crate::fix_support::FixSupport;
use crate::imports::ImportGroup;
use crate::imports::UseFix;
use crate::imports::ValidatedFixSet;
use crate::module_paths;
use crate::selection::Selection;

pub(crate) struct PreferModuleImportScan {
    pub findings: Vec<Finding>,
    pub fixes:    ValidatedFixSet,
}

pub(super) struct ScanFileContext<'a> {
    pub(super) analysis_root: &'a Path,
    pub(super) path:          &'a Path,
    pub(super) text:          &'a str,
    pub(super) offsets:       &'a [usize],
}

impl ScanFileContext<'_> {
    pub(super) fn display_path(&self) -> String {
        self.path
            .strip_prefix(self.analysis_root)
            .unwrap_or(self.path)
            .to_string_lossy()
            .replace('\\', "/")
    }
}

pub(super) struct ImportFindingInputs<'a> {
    module_to_functions: &'a BTreeMap<String, Vec<RawCandidate>>,
    func_to_module:      &'a BTreeMap<&'a str, (&'a str, ImportTarget)>,
    references:          &'a [BareReference],
}

pub(super) struct InlineCallFindingInputs<'a> {
    pub(super) candidates:            &'a [InlineCallCandidate],
    pub(super) will_import_modules:   &'a BTreeSet<Vec<String>>,
    pub(super) file_insertion_offset: usize,
}

pub(crate) fn scan_selection(selection: &Selection) -> Result<PreferModuleImportScan> {
    let mut all_findings = Vec::new();
    let mut all_fixes = Vec::new();
    for package_root in &selection.package_roots {
        let source_root = package_root.join(SOURCE_DIR_SRC);
        if !source_root.is_dir() {
            continue;
        }
        for entry in walkdir::WalkDir::new(&source_root)
            .into_iter()
            .filter_map(Result::ok)
        {
            let path = entry.path();
            if !entry.file_type().is_file()
                || path.extension().and_then(OsStr::to_str) != Some(RUST_SOURCE_FILE_EXTENSION)
            {
                continue;
            }
            let (findings, fixes) =
                scan_file(selection.analysis_root.as_path(), &source_root, path)?;
            all_findings.extend(findings);
            all_fixes.extend(fixes);
        }
    }
    all_findings.sort_by(|left, right| {
        (&left.path, left.line, left.column).cmp(&(&right.path, right.line, right.column))
    });
    all_findings.dedup_by(|left, right| {
        left.path == right.path && left.line == right.line && left.column == right.column
    });
    Ok(PreferModuleImportScan {
        findings: all_findings,
        fixes:    ValidatedFixSet::try_from(all_fixes)?,
    })
}

fn scan_file(
    analysis_root: &Path,
    source_root: &Path,
    path: &Path,
) -> Result<(Vec<Finding>, Vec<UseFix>)> {
    let text =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let syntax =
        syn::parse_file(&text).with_context(|| format!("failed to parse {}", path.display()))?;
    let current_module_path = module_paths::file_module_path(source_root, path)
        .with_context(|| format!("failed to determine module path for {}", path.display()))?;
    let offsets = shared::line_offsets(&text);
    let file_context = ScanFileContext {
        analysis_root,
        path,
        text: &text,
        offsets: &offsets,
    };

    let declared_modules = collect_declared_modules(&syntax);

    let mut detector = ImportDetector {
        source_root,
        current_module_path: current_module_path.clone(),
        declared_modules: &declared_modules,
        candidates: Vec::new(),
    };
    syn::visit::Visit::visit_file(&mut detector, &syntax);

    let mut inline_detector = InlineCallDetector {
        source_root,
        current_module_path: &current_module_path,
        declared_modules: &declared_modules,
        candidates: Vec::new(),
        inline_mod_depth: 0,
    };
    syn::visit::Visit::visit_file(&mut inline_detector, &syntax);

    if detector.candidates.is_empty() && inline_detector.candidates.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }

    let mut module_to_functions: BTreeMap<String, Vec<RawCandidate>> = BTreeMap::new();
    for candidate in detector.candidates {
        module_to_functions
            .entry(candidate.module_path.clone())
            .or_default()
            .push(candidate);
    }

    let imported_names: BTreeSet<String> = module_to_functions
        .values()
        .flatten()
        .map(|candidate| candidate.function_name.clone())
        .collect();

    let mut collector = ReferenceCollector::new(&offsets, &imported_names);
    syn::visit::Visit::visit_file(&mut collector, &syntax);

    let mut func_to_module: BTreeMap<&str, (&str, ImportTarget)> = BTreeMap::new();
    for functions in module_to_functions.values() {
        for function in functions {
            func_to_module.insert(
                function.function_name.as_str(),
                (function.module_name.as_str(), function.import_target),
            );
        }
    }

    let (mut findings, mut fixes) = build_findings_and_fixes(
        &file_context,
        &ImportFindingInputs {
            module_to_functions: &module_to_functions,
            func_to_module:      &func_to_module,
            references:          &collector.references,
        },
    );

    if !inline_detector.candidates.is_empty() {
        let will_import_modules = build_will_import_modules(
            &syntax,
            source_root,
            &current_module_path,
            &module_to_functions,
        );
        let file_insertion_offset = file_level_insertion_offset(&syntax, &text, &offsets);
        let (inline_findings, inline_fixes) = inline_calls::build_inline_call_findings_and_fixes(
            &file_context,
            &InlineCallFindingInputs {
                candidates: &inline_detector.candidates,
                will_import_modules: &will_import_modules,
                file_insertion_offset,
            },
        );
        findings.extend(inline_findings);
        fixes.extend(inline_fixes);
    }

    Ok((findings, fixes))
}

fn collect_declared_modules(syntax: &File) -> BTreeSet<String> {
    syntax
        .items
        .iter()
        .filter_map(|item| {
            if let Item::Mod(item_mod) = item
                && item_mod.content.is_none()
            {
                Some(item_mod.ident.to_string())
            } else {
                None
            }
        })
        .collect()
}

fn build_will_import_modules(
    syntax: &File,
    source_root: &Path,
    current_module_path: &[String],
    module_to_functions: &BTreeMap<String, Vec<RawCandidate>>,
) -> BTreeSet<Vec<String>> {
    let mut will_import_modules: BTreeSet<Vec<String>> = BTreeSet::new();
    for item in &syntax.items {
        if let Item::Use(item_use) = item
            && let Some(flat) = shared::flatten_use_tree(&item_use.tree)
            && flat.rename.is_none()
            && let Some(absolute) = shared::resolve_to_absolute(&flat.segments, current_module_path)
            && !absolute.is_empty()
            && shared::leaf_is_module(source_root, &absolute)
        {
            will_import_modules.insert(absolute);
        }
    }

    for functions in module_to_functions.values() {
        for candidate in functions {
            will_import_modules.insert(candidate.absolute_module.clone());
        }
    }
    will_import_modules
}

fn file_level_insertion_offset(syntax: &File, text: &str, offsets: &[usize]) -> usize {
    let mut last_use_end: Option<usize> = None;
    let mut first_item_start: Option<usize> = None;
    for item in &syntax.items {
        let item_start = shared::offset(offsets, item.span().start());
        first_item_start.get_or_insert(item_start);
        if let Item::Use(item_use) = item {
            let end = shared::offset(offsets, item_use.span().end());
            let end = if text.as_bytes().get(end) == Some(&b'\n') {
                end + 1
            } else {
                end
            };
            last_use_end = Some(end);
        }
    }
    last_use_end.or(first_item_start).unwrap_or(0)
}

fn build_findings_and_fixes(
    file_context: &ScanFileContext<'_>,
    import_inputs: &ImportFindingInputs<'_>,
) -> (Vec<Finding>, Vec<UseFix>) {
    let display_path = file_context.display_path();
    let mut findings = Vec::new();
    let mut fixes = Vec::new();
    let mut rewritten_modules: BTreeSet<String> = BTreeSet::new();

    for functions in import_inputs.module_to_functions.values() {
        for function in functions {
            findings.push(build_function_finding(
                function,
                &display_path,
                file_context,
            ));
            fixes.push(build_function_use_fix(
                function,
                file_context,
                &mut rewritten_modules,
            ));
        }
    }

    fixes.extend(build_reference_fixes(file_context, import_inputs));

    (findings, fixes)
}

fn build_function_finding(
    function: &RawCandidate,
    display_path: &str,
    file_context: &ScanFileContext<'_>,
) -> Finding {
    let source_line = file_context
        .text
        .lines()
        .nth(function.span_start.line.saturating_sub(1))
        .unwrap_or_default()
        .to_string();

    let (message, suggestion) = if function.import_target == ImportTarget::ParentModule {
        (
            format!(
                "drop the import and call `super::{}` directly",
                function.function_name
            ),
            Some(format!(
                "remove this `use` and call `super::{}` at the use sites",
                function.function_name
            )),
        )
    } else {
        (
            format!(
                "import the module `{}` instead of the function `{}`",
                function.module_name, function.function_name
            ),
            Some(format!("consider using: `{}`", function.replacement_use)),
        )
    };

    Finding {
        severity: Severity::Warning,
        code: DiagnosticCode::PreferModuleImport,
        path: display_path.to_string(),
        line: function.span_start.line,
        column: function.span_start.column + 1,
        highlight_len: function.function_name.len().max(1),
        source_line,
        item: None,
        message,
        suggestion,
        fixability: FixSupport::PreferModuleImport,
        related: None,
    }
}

fn build_function_use_fix(
    function: &RawCandidate,
    file_context: &ScanFileContext<'_>,
    rewritten_modules: &mut BTreeSet<String>,
) -> UseFix {
    let byte_start = shared::offset(file_context.offsets, function.span_start);
    let byte_end = shared::offset(file_context.offsets, function.span_end);
    let byte_end_with_newline = if file_context.text.as_bytes().get(byte_end) == Some(&b'\n') {
        byte_end + 1
    } else {
        byte_end
    };
    let group = Some(ImportGroup {
        bare_name: function.module_name.clone(),
        full_path: function.absolute_module.join("::"),
    });

    if function.import_target == ImportTarget::ParentModule {
        UseFix {
            path:         file_context.path.to_path_buf(),
            start:        byte_start,
            end:          byte_end_with_newline,
            replacement:  String::new(),
            import_group: group,
        }
    } else if rewritten_modules.insert(function.module_path.clone()) {
        UseFix {
            path:         file_context.path.to_path_buf(),
            start:        byte_start,
            end:          byte_end,
            replacement:  function.replacement_use.clone(),
            import_group: group,
        }
    } else {
        UseFix {
            path:         file_context.path.to_path_buf(),
            start:        byte_start,
            end:          byte_end_with_newline,
            replacement:  String::new(),
            import_group: group,
        }
    }
}

fn build_reference_fixes(
    file_context: &ScanFileContext<'_>,
    import_inputs: &ImportFindingInputs<'_>,
) -> Vec<UseFix> {
    let mut fixes = Vec::new();
    for reference in import_inputs.references {
        if let Some(&(module_name, import_target)) =
            import_inputs.func_to_module.get(reference.name.as_str())
        {
            let group = import_inputs
                .module_to_functions
                .values()
                .flatten()
                .find(|function| function.module_name == module_name)
                .map(|function| ImportGroup {
                    bare_name: function.module_name.clone(),
                    full_path: function.absolute_module.join("::"),
                });
            let prefix = if import_target == ImportTarget::ParentModule {
                PATH_KEYWORD_SUPER
            } else {
                module_name
            };
            fixes.push(UseFix {
                path:         file_context.path.to_path_buf(),
                start:        reference.byte_start,
                end:          reference.byte_end,
                replacement:  format!("{prefix}::{}", reference.name),
                import_group: group,
            });
        }
    }
    fixes
}
