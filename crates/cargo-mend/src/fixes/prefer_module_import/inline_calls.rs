use std::collections::BTreeMap;
use std::collections::BTreeSet;

use proc_macro2::LineColumn;
use syn::ExprPath;
use syn::File;
use syn::ItemMod;
use syn::ItemUse;
use syn::spanned::Spanned;
use syn::visit;
use syn::visit::Visit;

use super::function_imports::ImportTarget;
use super::scan::InlineCallFindingInputs;
use super::scan::ScanFileContext;
use super::support;
use super::support::CrateSources;
use crate::config::DiagnosticCode;
use crate::fixes::imports;
use crate::fixes::imports::ConditionalAttributes;
use crate::fixes::imports::GateSource;
use crate::fixes::imports::GateTracking;
use crate::fixes::imports::ImportGroup;
use crate::fixes::imports::ImportSource;
use crate::fixes::imports::UseFix;
use crate::reporting::Finding;
use crate::reporting::FixSupport;
use crate::reporting::ItemVisibility;
use crate::reporting::Severity;
use crate::rust_syntax::PathAnchor;

pub(super) struct InlineCallCandidate {
    pub(super) function_name:          String,
    pub(super) module_name:            String,
    pub(super) module_path:            String,
    pub(super) absolute_module:        Vec<String>,
    pub(super) prefix_start:           LineColumn,
    pub(super) leaf_start:             LineColumn,
    pub(super) full_span_start:        LineColumn,
    pub(super) full_span_end:          LineColumn,
    /// True when the target module is the file's own parent module.
    /// Rewrite the call to `super::function_name(...)` and add no `use`.
    pub(super) import_target:          ImportTarget,
    /// `#[cfg]` gates enclosing the call site. The synthesized
    /// `use module;` must repeat them or it fails to resolve in builds where
    /// the target module is configured out.
    pub(super) conditional_attributes: ConditionalAttributes,
}

pub(super) struct InlineCallDetector<'a> {
    pub(super) crate_sources:          CrateSources<'a>,
    pub(super) current_module_path:    &'a [String],
    pub(super) declared_modules:       &'a BTreeSet<String>,
    pub(super) candidates:             Vec<InlineCallCandidate>,
    pub(super) inline_mod_depth:       usize,
    pub(super) conditional_attributes: ConditionalAttributes,
}

impl GateTracking for GatedInlineCalls<'_, '_> {
    fn gate_source(&self) -> GateSource<'_, '_> {
        GateSource {
            indexed: self.source,
        }
    }

    fn conditional_attributes_mut(&mut self) -> &mut ConditionalAttributes {
        &mut self.visitor.conditional_attributes
    }
}

/// A traversal borrowing the file scan's checked index for all gate extraction.
struct GatedInlineCalls<'visit, 'source> {
    visitor: &'visit mut InlineCallDetector<'source>,
    source:  &'visit ImportSource<'source>,
}

impl<'source> InlineCallDetector<'source> {
    pub(super) fn visit_file(&mut self, node: &File, source: &ImportSource<'source>) {
        GatedInlineCalls {
            visitor: self,
            source,
        }
        .visit_file(node);
    }
}

impl Visit<'_> for GatedInlineCalls<'_, '_> {
    imports::gate_tracking_visit!();

    fn visit_item_use(&mut self, _: &ItemUse) {}

    fn visit_item_mod(&mut self, node: &ItemMod) {
        if node.content.is_some() {
            self.visitor.inline_mod_depth += 1;
            visit::visit_item_mod(self, node);
            self.visitor.inline_mod_depth -= 1;
        } else {
            visit::visit_item_mod(self, node);
        }
    }

    fn visit_expr_path(&mut self, node: &ExprPath) {
        if self.visitor.inline_mod_depth > 0 || node.qself.is_some() {
            return;
        }
        if let Some(candidate) = analyze_inline_call(
            self.visitor.crate_sources,
            self.visitor.current_module_path,
            self.visitor.declared_modules,
            node,
            self.visitor.conditional_attributes.clone(),
        ) {
            self.visitor.candidates.push(candidate);
        }
    }
}

enum InlineCallRewrite<'a> {
    Parent,
    Module {
        attributes: &'a ConditionalAttributes,
        prefix:     String,
    },
}

impl InlineCallRewrite<'_> {
    fn prefix(&self) -> &str {
        match self {
            Self::Parent => "super::",
            Self::Module { prefix, .. } => prefix,
        }
    }
}

pub(super) fn build_inline_call_findings_and_fixes(
    file_context: &ScanFileContext<'_>,
    inline_inputs: &InlineCallFindingInputs<'_>,
) -> (Vec<Finding>, Vec<UseFix>) {
    let display_path = file_context.display_path();

    let mut findings = Vec::new();
    let mut fixes = Vec::new();
    let mut inserted_modules: BTreeSet<Vec<String>> = BTreeSet::new();
    // `insertions` stores indexes into `fixes` of the file-level `use` insertions.
    let mut insertions = Vec::new();
    let covering_gates = covering_gates(inline_inputs.candidates);

    for candidate in inline_inputs.candidates {
        // `covering_gates` supplies one gate for the `use module;` serving every call site in the
        // file, so it has to carry a gate that covers all of them. Call sites whose gates
        // disagree share no such import — those stay fully qualified, and
        // reporting a finding without an applicable fix would only invite the
        // rewrite that breaks the build.
        let rewrite = match candidate.import_target {
            ImportTarget::ParentModule => InlineCallRewrite::Parent,
            ImportTarget::OtherModule => match covering_gates.get(&candidate.absolute_module) {
                Some(attributes) => InlineCallRewrite::Module {
                    attributes,
                    prefix: format!("{}::", candidate.module_name),
                },
                None => continue,
            },
        };

        let prefix_start_byte = support::offset(file_context.offsets, candidate.prefix_start);
        let leaf_start_byte = support::offset(file_context.offsets, candidate.leaf_start);
        let full_start_byte = support::offset(file_context.offsets, candidate.full_span_start);
        let full_end_byte = support::offset(file_context.offsets, candidate.full_span_end);

        let source_line = file_context
            .text
            .lines()
            .nth(candidate.full_span_start.line.saturating_sub(1))
            .unwrap_or_default()
            .to_string();

        let full_path_text = file_context
            .text
            .get(full_start_byte..full_end_byte)
            .unwrap_or_default()
            .to_string();

        let (message, suggestion) = inline_call_text(candidate, &rewrite);

        findings.push(Finding {
            severity: Severity::Warning,
            diagnostic_code: DiagnosticCode::PreferModuleImport,
            path: display_path.clone(),
            line: candidate.full_span_start.line,
            column: candidate.full_span_start.column + 1,
            highlight_len: full_path_text.len().max(1),
            source_line,
            item: None,
            message,
            suggestion: Some(suggestion),
            fix_support: FixSupport::PreferModuleImport,
            related: None,
            item_visibility: ItemVisibility::default(),
        });

        let group = Some(ImportGroup {
            bare_name: candidate.module_name.clone(),
            full_path: candidate.absolute_module.join("::"),
        });

        let call_prefix = rewrite.prefix().to_string();
        fixes.push(UseFix {
            path:         file_context.path.to_path_buf(),
            start:        prefix_start_byte,
            end:          leaf_start_byte,
            replacement:  call_prefix,
            import_group: group.clone(),
        });

        let import_attributes = match rewrite {
            InlineCallRewrite::Parent => continue,
            InlineCallRewrite::Module { attributes, .. } => attributes,
        };
        if inline_inputs
            .will_import_modules
            .contains(&candidate.absolute_module)
        {
            continue;
        }
        if !inserted_modules.insert(candidate.absolute_module.clone()) {
            continue;
        }
        let offset = inline_inputs.file_insertion.offset;
        insertions.push(fixes.len());
        fixes.push(UseFix {
            path:         file_context.path.to_path_buf(),
            start:        offset,
            end:          offset,
            replacement:  format!(
                "{}use {};\n",
                import_attributes.render(""),
                candidate.module_path
            ),
            import_group: group,
        });
    }

    // `insertions` selects the smallest `UseFix::replacement`: insertions at one offset apply in
    // ascending replacement order, each landing before the last, so the smallest lands last and
    // its trailing blank line separates every inserted `use` from the first item.
    if inline_inputs.file_insertion.before_first_item
        && let Some(last) = insertions
            .into_iter()
            .min_by(|&left, &right| fixes[left].replacement.cmp(&fixes[right].replacement))
    {
        fixes[last].replacement.push('\n');
    }

    (findings, fixes)
}

fn inline_call_text(
    candidate: &InlineCallCandidate,
    rewrite: &InlineCallRewrite<'_>,
) -> (String, String) {
    match rewrite {
        InlineCallRewrite::Parent => (
            format!(
                "use `super::{}` instead of the fully-qualified path",
                candidate.function_name
            ),
            format!("rewrite the call as `super::{}`", candidate.function_name),
        ),
        InlineCallRewrite::Module { .. } => (
            format!(
                "import the module `{}` instead of using the fully-qualified path for `{}`",
                candidate.module_name, candidate.function_name
            ),
            format!(
                "add `use {};` and call `{}::{}`",
                candidate.module_path, candidate.module_name, candidate.function_name
            ),
        ),
    }
}

/// The gate each imported module's `use` must carry, keyed by absolute module
/// path. Modules whose call sites sit under gates that disagree are absent,
/// which tells the caller to leave those calls fully qualified.
fn covering_gates(
    candidates: &[InlineCallCandidate],
) -> BTreeMap<Vec<String>, ConditionalAttributes> {
    let mut grouped: BTreeMap<Vec<String>, BTreeSet<ConditionalAttributes>> = BTreeMap::new();
    for candidate in candidates {
        if candidate.import_target == ImportTarget::ParentModule {
            continue;
        }
        grouped
            .entry(candidate.absolute_module.clone())
            .or_default()
            .insert(candidate.conditional_attributes.clone());
    }
    grouped
        .into_iter()
        .filter_map(|(module, gates)| {
            ConditionalAttributes::covering(&gates).map(|covering| (module, covering))
        })
        .collect()
}

fn analyze_inline_call(
    crate_sources: CrateSources<'_>,
    current_module_path: &[String],
    declared_modules: &BTreeSet<String>,
    node: &ExprPath,
    conditional_attributes: ConditionalAttributes,
) -> Option<InlineCallCandidate> {
    let path = &node.path;
    let segments: Vec<String> = path
        .segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect();
    if segments.len() < 3 {
        return None;
    }

    let path_anchor = PathAnchor::first(&segments)?;
    if !path_anchor.is_crate_relative() {
        return None;
    }

    let leaf = segments.last()?;
    if !support::is_snake_case_function_name(leaf) {
        return None;
    }

    let absolute_segments = support::resolve_to_absolute(&segments, current_module_path)?;
    if absolute_segments.is_empty() {
        return None;
    }
    if support::leaf_is_module(crate_sources, &absolute_segments) {
        return None;
    }

    let absolute_module = absolute_segments[..absolute_segments.len() - 1].to_vec();
    if absolute_module.is_empty() || !support::leaf_is_module(crate_sources, &absolute_module) {
        return None;
    }

    let module_name = segments[segments.len() - 2].clone();
    if PathAnchor::from(module_name.as_str()).is_crate_relative() {
        return None;
    }
    if !support::is_snake_case_module_name(&module_name) {
        return None;
    }
    if declared_modules.contains(&module_name) {
        return None;
    }

    let module_segments = &segments[..segments.len() - 1];
    let shortened = support::shorten_module_path(current_module_path, module_segments);
    let import_target = match shortened.as_slice() {
        [segment] if PathAnchor::from(segment.as_str()) == PathAnchor::Super => {
            ImportTarget::ParentModule
        },
        _ => ImportTarget::OtherModule,
    };
    let module_path = shortened.join("::");

    let first_seg = path.segments.first()?;
    let leaf_seg = path.segments.last()?;

    Some(InlineCallCandidate {
        function_name: leaf.clone(),
        module_name,
        module_path,
        absolute_module,
        prefix_start: first_seg.ident.span().start(),
        leaf_start: leaf_seg.ident.span().start(),
        full_span_start: path.span().start(),
        full_span_end: path.span().end(),
        import_target,
        conditional_attributes,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::Path;

    use proc_macro2::LineColumn;

    use super::InlineCallCandidate;
    use super::build_inline_call_findings_and_fixes;
    use crate::fixes::imports::ConditionalAttributes;
    use crate::fixes::prefer_module_import::function_imports::ImportTarget;
    use crate::fixes::prefer_module_import::scan::FileInsertion;
    use crate::fixes::prefer_module_import::scan::InlineCallFindingInputs;
    use crate::fixes::prefer_module_import::scan::ScanFileContext;

    #[test]
    fn parent_calls_insert_no_import_and_module_calls_reuse_planned_imports() {
        let text = "crate::outer::run(); crate::util::run();";
        let candidates = [
            ("outer", ImportTarget::ParentModule, 0, 14, 17),
            ("util", ImportTarget::OtherModule, 21, 34, 37),
        ]
        .map(|(name, target, start, leaf, end)| InlineCallCandidate {
            function_name:          "run".to_string(),
            module_name:            name.to_string(),
            module_path:            format!("crate::{name}"),
            absolute_module:        vec![name.to_string()],
            prefix_start:           LineColumn {
                line:   1,
                column: start,
            },
            leaf_start:             LineColumn {
                line:   1,
                column: leaf,
            },
            full_span_start:        LineColumn {
                line:   1,
                column: start,
            },
            full_span_end:          LineColumn {
                line:   1,
                column: end,
            },
            import_target:          target,
            conditional_attributes: ConditionalAttributes::default(),
        });
        let context = ScanFileContext {
            analysis_root: Path::new("."),
            path: Path::new("src/lib.rs"),
            text,
            offsets: &[0],
        };
        for (planned, expected) in [
            (
                BTreeSet::new(),
                vec!["super::", "util::", "use crate::util;\n\n"],
            ),
            (
                BTreeSet::from([vec!["util".to_string()]]),
                vec!["super::", "util::"],
            ),
        ] {
            let (findings, fixes) = build_inline_call_findings_and_fixes(
                &context,
                &InlineCallFindingInputs {
                    candidates:          &candidates,
                    will_import_modules: &planned,
                    file_insertion:      FileInsertion {
                        offset:            0,
                        before_first_item: true,
                    },
                },
            );
            assert_eq!(findings.len(), 2);
            assert_eq!(
                fixes
                    .iter()
                    .map(|fix| fix.replacement.as_str())
                    .collect::<Vec<_>>(),
                expected
            );
        }
    }
}
