use std::path::Path;

use anyhow::Context;
use proc_macro2::LineColumn;
use syn::Attribute;
use syn::Block;
use syn::File;
use syn::Item;
use syn::ItemMod;
use syn::ItemUse;
use syn::Stmt;
use syn::UseTree;
use syn::spanned::Spanned;
use syn::visit::Visit;

use crate::config::DiagnosticCode;
use crate::fixes::constants::IMPORTS_AT_TOP_MESSAGE;
use crate::fixes::constants::IMPORTS_AT_TOP_SUGGESTION;
use crate::fixes::imports;
use crate::fixes::imports::ConditionalAttributes;
use crate::fixes::imports::GateSource;
use crate::fixes::imports::GateTracking;
use crate::fixes::imports::ImportGroup;
use crate::fixes::imports::ImportMove;
use crate::fixes::imports::ImportSource;
use crate::fixes::imports::ModuleImports;
use crate::fixes::imports::UseFix;
use crate::reporting::Finding;
use crate::reporting::FixSupport;
use crate::reporting::ItemVisibility;
use crate::reporting::Severity;

/// An active module scope required by every import move and insertion.
/// The file scan installs the root scope, and entering a module replaces it
/// until that module's traversal finishes; emission never has an absent scope.
struct ScopedUseFinder {
    /// Byte offset where new `use` lines insert at the top of the scope.
    /// Sits just after the last top-of-scope `use`, or at the first
    /// non-use item, or at the start of the scope when neither exists.
    insertion_offset: usize,
    /// Indent (spaces and/or tabs) to prepend to inserted `use` lines.
    indent:           String,
}

struct InBodyUseFinder<'a> {
    source:                 &'a ImportSource<'a>,
    text:                   &'a str,
    path:                   &'a Path,
    display_path:           &'a str,
    scope:                  ScopedUseFinder,
    imports:                ModuleImports,
    /// Source text of the `#[cfg]`/`#[cfg_attr]` attributes on every enclosing
    /// construct the visitor is currently inside, outermost first. A nested
    /// `use` that moves to the module scope carries these attributes with it so
    /// the import stays conditionally compiled. Emptied on entering an inline
    /// module (an inner `use` moves only to that module's top, which the
    /// module's own gate already wraps).
    conditional_attributes: ConditionalAttributes,
    result:                 anyhow::Result<()>,
    findings:               Vec<Finding>,
    fixes:                  Vec<UseFix>,
}

impl InBodyUseFinder<'_> {
    fn attributes(&self, attributes: &[Attribute]) -> ConditionalAttributes {
        ConditionalAttributes::from_attributes(self.source, attributes)
    }

    fn try_emit_fix(&mut self, use_item: &ItemUse) -> anyhow::Result<()> {
        // `InBodyUseFinder::try_emit_fix` leaves `use_item` in place when a non-`cfg`
        // attribute on the `use` itself (`#[allow(...)]`, a doc comment, …) can change
        // what the import means.
        if use_item
            .attrs
            .iter()
            .any(|attribute| !imports::is_conditional(attribute))
        {
            return Ok(());
        }

        let Ok(bare_paths) = flatten_use_to_bare_paths(&use_item.tree)
            .map(ImportLeaf::into_named)
            .collect::<Result<Vec<_>, _>>()
        else {
            return Ok(());
        };
        if bare_paths.is_empty() {
            return Ok(());
        }

        // `conditional_attributes` combines every enclosing `#[cfg]` plus
        // any `#[cfg]` on the `use` itself. Non-empty means the import is
        // conditionally compiled and the attributes travel with it to the top.
        let mut conditional_attributes = self.conditional_attributes.clone();
        conditional_attributes.extend(self.attributes(&use_item.attrs));
        let insert = match self.imports.move_import(
            self.scope.insertion_offset,
            use_item,
            &conditional_attributes,
        ) {
            ImportMove::Blocked => return Ok(()),
            ImportMove::Delete => false,
            ImportMove::Insert => true,
        };
        self.emit_move(use_item, &bare_paths, &conditional_attributes, insert)
    }

    /// Push the finding and the insertion/deletion fixes for a movable `use`.
    /// Carried gates keep the import conditionally compiled. The shared owner
    /// chooses whether the move needs an insertion or only a deletion.
    fn emit_move(
        &mut self,
        use_item: &ItemUse,
        bare_paths: &[NamedImport],
        conditional_attributes: &ConditionalAttributes,
        insert: bool,
    ) -> anyhow::Result<()> {
        let use_span = use_item.span();
        // `use_span` includes the first attribute. Build the moved text from
        // `use_item.use_token` so `conditional_attributes` are emitted once.
        let attr_start = SourcePosition::new(self.source, use_span.start())?.offset;
        let use_kw = use_item.use_token.span().start();
        let use_location = SourcePosition::new(self.source, use_kw)?;
        let use_kw_start = use_location.offset;
        let use_end = SourcePosition::new(self.source, use_span.end())?.offset;
        let use_end_with_nl = if self.text.as_bytes().get(use_end) == Some(&b'\n') {
            use_end + 1
        } else {
            use_end
        };
        let line_start = line_start_offset(self.text, attr_start);
        let leading = &self.text[line_start..attr_start];
        let delete_start = if leading
            .chars()
            .all(|character| character == ' ' || character == '\t')
        {
            line_start
        } else {
            attr_start
        };

        let group = bare_paths.first().map(|binding| ImportGroup {
            bare_name: binding.name.clone(),
            full_path: binding.path.clone(),
        });

        self.findings.push(Finding {
            disposition:     Finding::disposition(
                DiagnosticCode::ImportsAtTop,
                Severity::Warning,
                FixSupport::ImportsAtTop,
            ),
            diagnostic_code: DiagnosticCode::ImportsAtTop,
            path:            self.display_path.to_string(),
            line:            use_kw.line,
            column:          use_kw.column + 1,
            highlight_len:   (use_end - use_kw_start).max(1),
            source_line:     use_location.source_line.to_string(),
            item:            None,
            message:         IMPORTS_AT_TOP_MESSAGE.to_string(),
            suggestion:      Some(IMPORTS_AT_TOP_SUGGESTION.to_string()),
            related:         None,
            item_visibility: ItemVisibility::default(),
        });

        if insert {
            self.emit_insertion(
                conditional_attributes,
                use_kw_start,
                use_end,
                group.as_ref(),
            );
        }

        self.fixes.push(UseFix {
            path:         self.path.to_path_buf(),
            start:        delete_start,
            end:          use_end_with_nl,
            replacement:  String::new(),
            import_group: group,
        });
        Ok(())
    }

    /// Insert the proposal chosen by the shared scope import owner.
    fn emit_insertion(
        &mut self,
        conditional_attributes: &ConditionalAttributes,
        use_kw_start: usize,
        use_end: usize,
        group: Option<&ImportGroup>,
    ) {
        let indent = self.scope.indent.clone();
        let insertion_offset = self.scope.insertion_offset;

        let use_text = &self.text[use_kw_start..use_end];
        let mut insertion = conditional_attributes.render(&indent);
        insertion.push_str(&indent);
        insertion.push_str(use_text);
        insertion.push('\n');
        self.fixes.push(UseFix {
            path:         self.path.to_path_buf(),
            start:        insertion_offset,
            end:          insertion_offset,
            replacement:  insertion,
            import_group: group.cloned(),
        });
    }
}

impl GateTracking for InBodyUseFinder<'_> {
    fn gate_source(&self) -> GateSource<'_, '_> {
        GateSource {
            indexed: self.source,
        }
    }

    fn conditional_attributes_mut(&mut self) -> &mut ConditionalAttributes {
        &mut self.conditional_attributes
    }
}

impl Visit<'_> for InBodyUseFinder<'_> {
    imports::gate_tracking_visit!();

    fn visit_item_mod(&mut self, node: &ItemMod) {
        let Some((brace, items)) = &node.content else {
            return;
        };
        let nested_default = format!("{}    ", self.scope.indent);
        let scope = SourcePosition::new(self.source, brace.span.open().end()).and_then(|opening| {
            compute_scope_for_items(
                items,
                self.text,
                self.source,
                &nested_default,
                opening.offset,
            )
        });
        let scope = match scope {
            Ok(scope) => scope,
            Err(error) => {
                self.result = Err(error);
                return;
            },
        };
        // `visit_item_mod` clears `self.conditional_attributes` because `node` opens its own scope:
        // an inner `use` moves only to this module's top, which the module's own gate
        // already wraps, so gates from enclosing constructs no longer apply. Clear the
        // stack for the body and restore it afterward.
        let outer_attributes = std::mem::take(&mut self.conditional_attributes);
        let parent_scope = std::mem::replace(&mut self.scope, scope);
        for item in items {
            self.visit_item(item);
        }
        self.scope = parent_scope;
        self.conditional_attributes = outer_attributes;
    }

    fn visit_block(&mut self, node: &Block) {
        for stmt in &node.stmts {
            if self.result.is_err() {
                return;
            }
            match stmt {
                Stmt::Item(Item::Use(use_item)) => {
                    if let Err(error) = self.try_emit_fix(use_item) {
                        self.result = Err(error);
                        return;
                    }
                },
                Stmt::Item(_) | Stmt::Local(_) | Stmt::Expr(_, _) | Stmt::Macro(_) => {
                    self.visit_stmt(stmt);
                },
            }
        }
    }
}

#[cfg(test)]
pub(super) fn scan(
    syntax: &File,
    text: &str,
    path: &Path,
    display_path: &str,
) -> anyhow::Result<(Vec<Finding>, Vec<UseFix>)> {
    scan_at_module(syntax, text, path, display_path, &[])
}

pub(super) fn scan_at_module(
    syntax: &File,
    text: &str,
    path: &Path,
    display_path: &str,
    module: &[String],
) -> anyhow::Result<(Vec<Finding>, Vec<UseFix>)> {
    let indexed = ImportSource::new(text);
    let root_scope = compute_scope_for_items(&syntax.items, text, &indexed, "", 0)?;
    let mut visitor = InBodyUseFinder {
        source: &indexed,
        text,
        path,
        display_path,
        scope: root_scope,
        imports: ModuleImports::from_syntax(syntax, &indexed, module)?,
        conditional_attributes: ConditionalAttributes::default(),
        result: Ok(()),
        findings: Vec::new(),
        fixes: Vec::new(),
    };
    for item in &syntax.items {
        visitor.visit_item(item);
    }
    visitor.result?;
    Ok((visitor.findings, visitor.fixes))
}

fn compute_scope_for_items(
    items: &[Item],
    text: &str,
    source: &ImportSource<'_>,
    default_indent: &str,
    empty_offset: usize,
) -> anyhow::Result<ScopedUseFinder> {
    let mut insertion = ScopeInsertion::EmptyModule(empty_offset);
    let mut detected_indent: Option<String> = None;

    for item in items {
        let item_start_lc = item.span().start();
        let item_start = SourcePosition::new(source, item_start_lc)?.offset;
        if let ScopeInsertion::EmptyModule(_) = insertion {
            insertion = ScopeInsertion::BeforeItem(item_start);
        }
        if detected_indent.is_none() {
            detected_indent = Some(indent_for_offset(text, item_start));
        }
        if let Item::Use(item_use) = item {
            let end = SourcePosition::new(source, item_use.span().end())?.offset;
            let end = if text.as_bytes().get(end) == Some(&b'\n') {
                end + 1
            } else {
                end
            };
            insertion = ScopeInsertion::AfterUse(end);
        }
    }

    let insertion_offset = insertion.offset();
    let indent = detected_indent.unwrap_or_else(|| default_indent.to_string());
    Ok(ScopedUseFinder {
        insertion_offset,
        indent,
    })
}

fn indent_for_offset(text: &str, offset: usize) -> String {
    let line_start = line_start_offset(text, offset);
    let leading = &text[line_start..offset];
    if leading
        .chars()
        .all(|character| character == ' ' || character == '\t')
    {
        leading.to_string()
    } else {
        String::new()
    }
}

fn line_start_offset(text: &str, offset: usize) -> usize {
    text[..offset]
        .rfind('\n')
        .map_or(0, |position| position + 1)
}

/// A named import that can participate in move and duplicate planning.
#[derive(Debug, PartialEq)]
struct NamedImport {
    name: String,
    path: String,
}

/// Globs may shadow arbitrary names, so only the named phase can be moved.
#[derive(Debug, PartialEq)]
enum ImportLeaf {
    Named(NamedImport),
    Glob { path: String },
}

impl ImportLeaf {
    fn into_named(self) -> Result<NamedImport, String> {
        match self {
            Self::Named(binding) => Ok(binding),
            Self::Glob { path } => Err(path),
        }
    }
}

/// An empty module inserts at its real opening position, supplied by the caller.
/// It never starts at a placeholder zero that needs repair after scope collection.
enum ScopeInsertion {
    AfterUse(usize),
    BeforeItem(usize),
    EmptyModule(usize),
}

impl ScopeInsertion {
    const fn offset(self) -> usize {
        match self {
            Self::AfterUse(offset) | Self::BeforeItem(offset) | Self::EmptyModule(offset) => offset,
        }
    }
}

fn flatten_use_to_bare_paths(tree: &UseTree) -> impl Iterator<Item = ImportLeaf> {
    imports::collect_use_bindings(tree)
        .into_iter()
        .map(|binding| {
            binding.name().map_or_else(
                || ImportLeaf::Glob {
                    path: binding.path().to_string(),
                },
                |name| {
                    ImportLeaf::Named(NamedImport {
                        name: name.to_string(),
                        path: binding.path().to_string(),
                    })
                },
            )
        })
}

/// A source-bound character position checked before move or insertion planning.
/// Invalid lines and columns are rejected instead of becoming byte zero; diagnostics
/// carry the original source line rather than substituting empty text for a missing line.
/// Character columns are converted to byte offsets against that same source.
struct SourcePosition<'a> {
    offset:      usize,
    source_line: &'a str,
}

impl<'a> SourcePosition<'a> {
    fn new(source: &ImportSource<'a>, position: LineColumn) -> anyhow::Result<Self> {
        let location = source
            .locate(position, position)
            .with_context(|| format!("invalid import source position: {position:?}"))?;
        Ok(Self {
            offset:      location.start(),
            source_line: location.source_line(),
        })
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use proc_macro2::LineColumn;
    use syn::File;
    use syn::Item;
    use syn::UseTree;
    use syn::parse_str;

    use super::ImportLeaf;
    use super::NamedImport;
    use super::flatten_use_to_bare_paths;

    fn named(name: &str, path: &str) -> ImportLeaf {
        ImportLeaf::Named(NamedImport {
            name: name.to_string(),
            path: path.to_string(),
        })
    }

    fn parse_tree(source: &str) -> UseTree {
        let file: File = parse_str(source).expect("parse");
        let item = file
            .items
            .into_iter()
            .next()
            .expect("fixture should produce one item");
        let Item::Use(item_use) = item else {
            panic!("expected use item")
        };
        item_use.tree
    }

    #[test]
    fn checked_positions_reject_invalid_lines_and_columns_and_keep_source_text() {
        let text = "// é\nfn f() { use crate::a::Thing; }\n";
        let source = crate::fixes::imports::ImportSource::new(text);
        let at = |line, column| LineColumn { line, column };
        for position in [at(0, 0), at(4, 0), at(2, 99)] {
            assert!(super::SourcePosition::new(&source, position).is_err());
        }
        let position = super::SourcePosition::new(&source, at(2, 9)).expect("use keyword");
        assert_eq!(position.source_line, "fn f() { use crate::a::Thing; }");
        assert_eq!(&text[position.offset..position.offset + 3], "use");
        let end = super::SourcePosition::new(&source, at(3, 0)).expect("final empty line");
        assert_eq!(end.offset, text.len());
        assert_eq!(end.source_line, "");
    }

    #[test]
    fn finder_rejects_mismatched_source_before_returning_edits() {
        let syntax = syn::parse_file("fn f() { use crate::a::Thing; }").expect("fixture");
        assert!(super::scan(&syntax, "", std::path::Path::new("lib.rs"), "lib.rs").is_err());
    }

    #[test]
    fn moved_import_keeps_unicode_diagnostic_and_edit_bytes() -> anyhow::Result<()> {
        let line = "fn f() { /* é */ use crate::a::Thing; }";
        let text = format!("// é\n{line}\n");
        let syntax = syn::parse_file(&text)?;
        let (findings, fixes) =
            super::scan(&syntax, &text, std::path::Path::new("lib.rs"), "lib.rs")?;
        assert_eq!(findings[0].source_line, line);
        assert_eq!(findings[0].line, 2);
        assert_eq!(fixes[0].replacement, "use crate::a::Thing;\n");
        assert_eq!(&text[fixes[1].start..fixes[1].end], "use crate::a::Thing;");
        Ok(())
    }

    #[test]
    fn moved_imports_keep_cfg_only_dependency_gates() -> anyhow::Result<()> {
        let text = r#"#[cfg(target_os = "macos")]
extern crate libproc;

#[cfg(target_os = "macos")]
fn process_endpoints() {
    use libproc::proc_pid::pidinfo;
    pidinfo();
}

struct Census;

impl Census {
    #[cfg(target_os = "macos")]
    fn process_details() {
        use libproc::bsd_info::BSDInfo;
        let _: BSDInfo;
    }
}
"#;
        let syntax = syn::parse_file(text)?;
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("fixture.rs");
        std::fs::write(&path, text)?;
        let (findings, fixes) = super::scan(&syntax, text, &path, "fixture.rs")?;
        assert_eq!(findings.len(), 2);
        let fixes = crate::fixes::imports::ValidatedFixSet::try_from(fixes)?;
        crate::fixes::imports::apply_fixes(&fixes)?;
        let rewritten = std::fs::read_to_string(&path)?;
        let rewritten_syntax = syn::parse_file(&rewritten)?;
        let imports = rewritten_syntax
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Use(import) => Some(import),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(imports.len(), 2, "{rewritten}");
        let indexed = crate::fixes::imports::ImportSource::new(&rewritten);
        for import in imports {
            assert_eq!(import.attrs.len(), 1, "{rewritten}");
            let gates = crate::fixes::imports::ConditionalAttributes::from_attributes(
                &indexed,
                &import.attrs,
            );
            assert_eq!(
                gates.render(""),
                "#[cfg(target_os = \"macos\")]\n",
                "{rewritten}"
            );
        }
        assert!(!rewritten.contains("    use libproc::"), "{rewritten}");
        Ok(())
    }

    #[test]
    fn moved_import_combines_enclosing_and_own_gates() -> anyhow::Result<()> {
        let text = r#"#[cfg(feature = "processes")]
impl Census {
    #[cfg(target_os = "macos")]
    fn details() {
        #[cfg_attr(unix, cfg(feature = "details"))]
        let _ = {
            #[cfg(debug_assertions)]
            use libproc::bsd_info::BSDInfo;
        };
    }
}
"#;
        let syntax = syn::parse_file(text)?;
        let (findings, fixes) = super::scan(
            &syntax,
            text,
            std::path::Path::new("fixture.rs"),
            "fixture.rs",
        )?;
        assert_eq!(findings.len(), 1);
        assert_eq!(fixes.len(), 2);
        assert_eq!(
            fixes[0].replacement,
            concat!(
                "#[cfg(feature = \"processes\")]\n",
                "#[cfg(target_os = \"macos\")]\n",
                "#[cfg_attr(unix, cfg(feature = \"details\"))]\n",
                "#[cfg(debug_assertions)]\n",
                "use libproc::bsd_info::BSDInfo;\n",
            )
        );
        Ok(())
    }

    #[test]
    fn moved_import_keeps_only_gates_below_its_module() -> anyhow::Result<()> {
        let text = r#"#[cfg(feature = "processes")]
mod platform {
    trait Census {
        #[cfg(target_os = "macos")]
        fn details() {
            use libproc::bsd_info::BSDInfo;
        }
    }
}
fn other() { use crate::other::Other; }
"#;
        let syntax = syn::parse_file(text)?;
        let (findings, fixes) = super::scan(
            &syntax,
            text,
            std::path::Path::new("fixture.rs"),
            "fixture.rs",
        )?;
        assert_eq!(findings.len(), 2);
        assert_eq!(fixes.len(), 4);
        assert_eq!(
            fixes[0].replacement,
            "    #[cfg(target_os = \"macos\")]\n    use libproc::bsd_info::BSDInfo;\n"
        );
        assert_eq!(&text[fixes[0].start..][..5], "trait");
        assert_eq!(fixes[2].replacement, "use crate::other::Other;\n");
        assert_eq!(fixes[2].start, 0);
        Ok(())
    }

    #[test]
    fn full_file_scan_uses_one_index_for_gated_and_ungated_nodes() -> anyhow::Result<()> {
        for gate in ["", "#[cfg(any(unix, windows))]\n"] {
            let statements = "value();".repeat(128);
            let text =
                format!("mod nested {{ {gate}fn f() {{ use crate::a::Thing; {statements} }} }}");
            let syntax = syn::parse_file(&text)?;
            let before = crate::fixes::imports::ImportSource::construction_count();
            let (findings, fixes) = super::scan(
                &syntax,
                &text,
                std::path::Path::new("fixture.rs"),
                "fixture.rs",
            )?;
            assert_eq!(
                crate::fixes::imports::ImportSource::construction_count() - before,
                1
            );
            assert_eq!(findings.len(), 1);
            assert!(fixes.iter().any(|fix| {
                fix.replacement
                    .contains(&format!("{gate}use crate::a::Thing;"))
            }));
        }
        Ok(())
    }

    #[test]
    fn scope_insertion_uses_the_actual_empty_module_position() {
        let source = crate::fixes::imports::ImportSource::new("// head\nmod a {}");
        let scope = super::compute_scope_for_items(&[], "// head\nmod a {}", &source, "    ", 15)
            .expect("valid empty scope");
        assert_eq!(scope.insertion_offset, 15);
        assert_eq!(scope.indent, "    ");
        assert_eq!(super::ScopeInsertion::AfterUse(7).offset(), 7);
        assert_eq!(super::ScopeInsertion::BeforeItem(3).offset(), 3);
    }

    #[test]
    fn scoped_moves_restore_the_parent_and_leave_globs_in_place() -> anyhow::Result<()> {
        let source = concat!(
            "mod a {\n    fn f() { use crate::a::A; }\n}\n",
            "fn g() { use crate::b::B; }\n",
            "fn h() { use crate::c::*; }\n",
        );
        let syntax = syn::parse_file(source)?;
        let (findings, fixes) = super::scan(
            &syntax,
            source,
            std::path::Path::new("fixture.rs"),
            "fixture.rs",
        )
        .expect("valid source positions");
        assert_eq!(findings.len(), 2);
        assert_eq!(fixes.len(), 4);
        assert!(fixes[0].replacement.starts_with("    use crate::a::A;"));
        assert!(fixes[2].replacement.starts_with("use crate::b::B;"));
        assert_eq!(fixes[2].start, 0);
        Ok(())
    }

    #[test]
    fn flatten_simple_name() {
        let tree = parse_tree("use crate::foo::Bar;");
        assert_eq!(
            flatten_use_to_bare_paths(&tree).collect::<Vec<_>>(),
            vec![named("Bar", "crate::foo::Bar")]
        );
    }

    #[test]
    fn flatten_rename_uses_rename_as_bare() {
        let tree = parse_tree("use crate::foo::Bar as Renamed;");
        assert_eq!(
            flatten_use_to_bare_paths(&tree).collect::<Vec<_>>(),
            vec![named("Renamed", "crate::foo::Bar")]
        );
    }

    #[test]
    fn flatten_group_expands_into_one_entry_per_leaf() {
        let tree = parse_tree("use crate::foo::{Bar, Baz};");
        let result = flatten_use_to_bare_paths(&tree).collect::<Vec<_>>();
        assert!(
            result.contains(&named("Bar", "crate::foo::Bar")),
            "{result:?}"
        );
        assert!(
            result.contains(&named("Baz", "crate::foo::Baz")),
            "{result:?}"
        );
    }

    #[test]
    fn flatten_glob_names_its_phase() {
        let tree = parse_tree("use crate::foo::*;");
        assert_eq!(
            flatten_use_to_bare_paths(&tree).collect::<Vec<_>>(),
            vec![ImportLeaf::Glob {
                path: "crate::foo".to_string(),
            }]
        );
    }
    #[test]
    fn moving_imports_preserves_gate_collisions_and_duplicate_deletion() -> anyhow::Result<()> {
        for (existing, nested, findings, fixes) in [
            ("use crate::a::Thing;", "use crate::a::Thing;", 1, 1),
            (
                "use crate::a::Thing;",
                "#[cfg(test)] use crate::a::Thing;",
                0,
                0,
            ),
            (
                "#[cfg(test)] use crate::a::Thing;",
                "use crate::a::Thing;",
                0,
                0,
            ),
            ("use crate::b::Thing;", "use crate::a::Thing;", 0, 0),
            ("struct Thing;", "use crate::a::Thing;", 0, 0),
            ("", "#[cfg(test)] use crate::a::Thing;", 1, 2),
            ("", "use crate::a::Thing;", 1, 2),
        ] {
            let source = format!("{existing}\nfn f() {{ {nested} }}\n");
            let syntax = syn::parse_file(&source).expect("parse import fixture");
            let (actual_findings, actual_fixes) = super::scan(
                &syntax,
                &source,
                std::path::Path::new("fixture.rs"),
                "fixture.rs",
            )?;
            assert_eq!(
                (actual_findings.len(), actual_fixes.len()),
                (findings, fixes),
                "{source}"
            );
            if existing.is_empty() {
                let insertion = &actual_fixes[0].replacement;
                assert!(insertion.contains("use crate::a::Thing;"), "{insertion}");
                assert_eq!(
                    insertion.contains("#[cfg(test)]"),
                    nested.starts_with("#[cfg(test)]")
                );
            }
        }
        Ok(())
    }

    #[test]
    fn a_moved_gated_import_blocks_a_later_import_of_the_same_name() {
        let source =
            "fn f() { #[cfg(test)] use crate::a::Thing; }\nfn g() { use crate::a::Thing; }\n";
        let syntax = syn::parse_file(source).expect("parse import fixture");
        let (findings, fixes) = super::scan(
            &syntax,
            source,
            std::path::Path::new("fixture.rs"),
            "fixture.rs",
        )
        .expect("valid source positions");
        assert_eq!(findings.len(), 1);
        assert_eq!(fixes.len(), 2);
        assert!(
            fixes[0]
                .replacement
                .starts_with("#[cfg(test)]\nuse crate::a::Thing;")
        );
    }
}
