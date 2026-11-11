use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::path::Path;

use anyhow::Context;
use anyhow::Result;
use walkdir::WalkDir;

use super::process;
use super::process::OccurrenceResolutionContext;
use super::scope;
use super::scope::ScopeCollectionContext;
use super::scope::ScopeSpan;
use super::visitor::InlinePathVisitor;
use crate::compiler::SOURCE_DIR_SRC;
use crate::fixes::imports::ConditionalAttributes;
use crate::fixes::imports::ImportSource;
use crate::fixes::imports::UseFix;
use crate::fixes::imports::ValidatedFixSet;
use crate::reporting::Finding;
use crate::rust_syntax::ModuleMap;
use crate::rust_syntax::ParsedSources;
use crate::selection::Selection;

pub(crate) struct InlinePathScan {
    pub findings: Vec<Finding>,
    pub fixes:    ValidatedFixSet,
}

pub(crate) fn scan_selection(
    selection: &Selection,
    parsed_sources: &ParsedSources,
) -> Result<InlinePathScan> {
    let mut all_findings = Vec::new();
    let mut all_fixes = Vec::new();
    for package_root in &selection.package_roots {
        let source_root = package_root.join(SOURCE_DIR_SRC);
        if !source_root.is_dir() {
            continue;
        }
        let module_map = parsed_sources.module_map(&source_root);
        for entry in WalkDir::new(&source_root)
            .into_iter()
            .filter_map(Result::ok)
        {
            let path = entry.path();
            if !entry.file_type().is_file()
                || path.extension().and_then(OsStr::to_str) != Some("rs")
            {
                continue;
            }
            let (findings, fixes) = scan_file(
                selection.analysis_root.as_path(),
                &source_root,
                path,
                &module_map,
                parsed_sources,
            )?;
            all_findings.extend(findings);
            all_fixes.extend(fixes);
        }
    }
    all_findings.sort_by(|a, b| (&a.path, a.line, a.column).cmp(&(&b.path, b.line, b.column)));
    all_findings.dedup_by(|a, b| a.path == b.path && a.line == b.line && a.column == b.column);
    Ok(InlinePathScan {
        findings: all_findings,
        fixes:    ValidatedFixSet::try_from(all_fixes)?,
    })
}

fn scan_file(
    analysis_root: &Path,
    source_root: &Path,
    path: &Path,
    module_map: &ModuleMap,
    parsed_sources: &ParsedSources,
) -> Result<(Vec<Finding>, Vec<UseFix>)> {
    let Some(base_module_path) = module_map.scannable_module_path(source_root, path)? else {
        return Ok((Vec::new(), Vec::new()));
    };
    let parsed_source = parsed_sources
        .source(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let file = parsed_source
        .syntax()
        .with_context(|| format!("failed to parse {}", path.display()))?;
    let text = parsed_source.text();
    let indexed = ImportSource::new(text);
    let mut scopes = Vec::new();
    let mut scope_collection_context = ScopeCollectionContext {
        text,
        scopes: &mut scopes,
    };
    scope::collect_scopes(
        &file.items,
        ScopeSpan::new(0, text.len()),
        &base_module_path,
        &mut scope_collection_context,
        &indexed,
    )?;

    let mut visitor = InlinePathVisitor {
        occurrences:            Vec::new(),
        bare_type_names:        BTreeSet::new(),
        mod_depth:              0,
        generic_scopes:         Vec::new(),
        conditional_attributes: ConditionalAttributes::default(),
    };
    visitor.visit_file(file, &indexed);

    if visitor.occurrences.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }

    let display_path = path
        .strip_prefix(analysis_root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");

    let resolve_ctx = OccurrenceResolutionContext { scopes: &scopes };
    let resolved = visitor
        .occurrences
        .iter()
        .map(|occurrence| process::resolve_occurrence(occurrence, &resolve_ctx, &indexed))
        .collect::<Result<Vec<_>>>()?;
    let mut findings = Vec::new();
    let mut fixes = Vec::new();
    let mut inserted_use_paths: BTreeSet<(usize, String)> = BTreeSet::new();

    let ctx = resolve_ctx.analyze_collisions(
        path,
        &display_path,
        &visitor.occurrences,
        &resolved,
        &visitor.bare_type_names,
    );
    let import_attribute_plan = process::plan_import_attributes(&resolved);

    for resolved in &resolved {
        process::process_occurrence(
            resolved,
            &ctx,
            &import_attribute_plan,
            &mut inserted_use_paths,
            &mut findings,
            &mut fixes,
        );
    }

    Ok((findings, fixes))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::path::PathBuf;
    use std::process::Command;
    use std::process::Output;

    use anyhow::anyhow;
    use tempfile::TempDir;

    use crate::config::DiagnosticCode;
    use crate::fixes::imports;
    use crate::fixes::imports::ImportSource;
    use crate::reporting::Report;
    use crate::rust_syntax::ParsedSources;
    use crate::selection::Selection;
    use crate::selection::SelectionScope;

    const ABSOLUTE_SHADOWED_SOURCE: &str = "use other::std;\nfn f(_: ::std::time::Duration) {}\n";
    const ABSOLUTE_DURATION_PATH: &str = "::std::time::Duration";
    const ABSOLUTE_DURATION_IMPORT: &str = "use ::std::time::Duration;";

    #[test]
    fn absolute_paths_keep_external_identity_and_complete_highlight() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("lib.rs");
        std::fs::write(&path, ABSOLUTE_SHADOWED_SOURCE)?;
        let sources = ParsedSources::default();
        let module_map = sources.module_map(dir.path());
        let (findings, fixes) =
            super::scan_file(dir.path(), dir.path(), &path, &module_map, &sources)?;
        assert_eq!(findings.len(), 1);
        assert!(
            fixes
                .iter()
                .any(|fix| fix.replacement.trim() == ABSOLUTE_DURATION_IMPORT)
        );
        let rewrite = fixes
            .iter()
            .find(|fix| fix.start != fix.end)
            .ok_or_else(|| anyhow!("missing absolute path rewrite"))?;
        assert_eq!(
            &ABSOLUTE_SHADOWED_SOURCE[rewrite.start..rewrite.end],
            ABSOLUTE_DURATION_PATH
        );
        assert_eq!(findings[0].highlight_len, rewrite.end - rewrite.start);
        assert_eq!(findings[0].column, 9);
        Ok(())
    }

    #[test]
    fn completed_scan_rejects_ambiguous_and_already_bare_names() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("lib.rs");
        for text in [
            "fn f() { a::Widget; b::Widget; }",
            "fn f() { a::Widget; Widget; }",
        ] {
            std::fs::write(&path, text)?;
            let sources = ParsedSources::default();
            let module_map = sources.module_map(dir.path());
            let before = ImportSource::construction_count();
            let (findings, fixes) =
                super::scan_file(dir.path(), dir.path(), &path, &module_map, &sources)?;
            assert_eq!(ImportSource::construction_count() - before, 1);
            assert!(findings.is_empty());
            assert!(fixes.is_empty());
        }
        Ok(())
    }

    #[test]
    fn full_file_scan_shares_one_index_across_scopes_and_paths() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("lib.rs");
        let statements = "ext::Widget;".repeat(128);
        for gate in ["", "#[cfg(any(unix, windows))]\n"] {
            let text = format!(
                "mod nested {{ {gate}use ext::Existing; {gate}fn f() {{ {statements} }} \
                 mod deeper {{ {gate}fn g() {{ {statements} }} }} }}"
            );
            std::fs::write(&path, &text)?;
            let sources = ParsedSources::default();
            let module_map = sources.module_map(dir.path());
            let before = ImportSource::construction_count();
            let (findings, fixes) =
                super::scan_file(dir.path(), dir.path(), &path, &module_map, &sources)?;
            assert_eq!(ImportSource::construction_count() - before, 1);
            assert_eq!(findings.len(), 256);
            assert!(!fixes.is_empty());
            assert!(
                fixes
                    .iter()
                    .any(|fix| { fix.replacement.contains(&format!("{gate}use ext::Widget;")) })
            );
        }
        Ok(())
    }

    #[test]
    fn resolved_scan_keeps_unicode_diagnostic_line_and_edit_bytes() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("lib.rs");
        let line = "fn f() { /* é */ ext::Widget; }";
        let text = format!("// header\n{line}\n");
        std::fs::write(&path, &text)?;
        let sources = ParsedSources::default();
        let module_map = sources.module_map(dir.path());
        let before = ImportSource::construction_count();
        let (findings, fixes) =
            super::scan_file(dir.path(), dir.path(), &path, &module_map, &sources)?;
        assert_eq!(ImportSource::construction_count() - before, 1);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].source_line, line);
        assert_eq!(findings[0].line, 2);
        assert_eq!(findings[0].column, 18);
        let rewrites = fixes
            .iter()
            .filter(|fix| fix.start != fix.end)
            .collect::<Vec<_>>();
        assert_eq!(rewrites.len(), 1);
        assert_eq!(&text[rewrites[0].start..rewrites[0].end], "ext::Widget");
        assert_eq!(rewrites[0].replacement, "Widget");
        Ok(())
    }
    struct InlinePathBatch {
        root:    TempDir,
        modules: Vec<String>,
    }

    impl InlinePathBatch {
        fn new() -> anyhow::Result<Self> {
            let root = tempfile::tempdir()?;
            fs::create_dir(root.path().join("src"))?;
            Ok(Self {
                root,
                modules: Vec::new(),
            })
        }

        fn add_module(&mut self, name: &str, files: &[(&str, &str)]) -> anyhow::Result<PathBuf> {
            let root = self.root.path().join("src").join(name);
            for (file, source) in files {
                let destination = root.join(file);
                if let Some(parent) = destination.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(destination, source)?;
            }
            self.modules.push(name.to_string());
            let declarations = format!("mod {};", self.modules.join(";\nmod "));
            fs::write(self.root.path().join("src/lib.rs"), declarations)?;
            Ok(root)
        }

        fn scan(&self) -> anyhow::Result<super::InlinePathScan> {
            let root = self.root.path();
            let selection = Selection {
                manifest_path:    root.join("Cargo.toml"),
                manifest_dir:     root.to_path_buf(),
                workspace_root:   root.to_path_buf(),
                target_directory: root.join("target"),
                analysis_root:    root.to_path_buf(),
                scope:            SelectionScope::SinglePackage,
                package_roots:    vec![root.to_path_buf()],
                packages:         Vec::new(),
            };
            super::scan_selection(&selection, &ParsedSources::default())
        }

        fn compile(&self) -> std::io::Result<Output> {
            Command::new("rustc")
                .args(["--edition=2024", "--crate-type=lib", "--emit=metadata"])
                .arg(self.root.path().join("src/lib.rs"))
                .arg("-o")
                .arg(self.root.path().join("fixture.rmeta"))
                .output()
        }
    }

    fn member_report(report: &Report, member: &str) -> Report {
        Report {
            findings: report
                .findings
                .iter()
                .filter(|finding| Path::new(&finding.path).starts_with(member))
                .cloned()
                .collect(),
            ..Report::default()
        }
    }

    fn name_collision_skips_both(
        batch: &mut InlinePathBatch,
    ) -> anyhow::Result<impl FnOnce(&Report) -> anyhow::Result<()> + use<>> {
        let root = batch.add_module("name_collision_skips_both", &[("mod.rs", "")])?;
        fs::create_dir_all(root.join("parent/mod_a"))?;
        fs::create_dir_all(root.join("parent/mod_b"))?;
        fs::write(root.join("mod.rs"), "mod parent;\nfn main() {}\n")?;
        fs::write(
            root.join("parent.rs"),
            "mod mod_a;\nmod mod_b;\nmod consumer;\n",
        )?;
        fs::write(root.join("parent/mod_a.rs"), "pub struct Foo;\n")?;
        fs::write(root.join("parent/mod_b.rs"), "pub struct Foo;\n")?;
        fs::write(
            root.join("parent/consumer.rs"),
            r"fn example(_a: crate::name_collision_skips_both::parent::mod_a::Foo, _b: crate::name_collision_skips_both::parent::mod_b::Foo) {}
",
        )?;

        Ok(move |_: &Report| {
            let consumer = fs::read_to_string(root.join("parent/consumer.rs"))?;
            // Both Foo types have a name collision, so neither should be fixed
            assert!(
                consumer.contains("crate::name_collision_skips_both::parent::mod_a::Foo")
                    || consumer.contains("super::mod_a::Foo"),
                "collision should leave inline paths unchanged, got:\n{consumer}"
            );
            Ok(())
        })
    }

    fn skips_use_statements(
        batch: &mut InlinePathBatch,
    ) -> anyhow::Result<impl FnOnce(&Report) -> anyhow::Result<()> + use<>> {
        let root = batch.add_module("skips_use_statements", &[("mod.rs", "")])?;
        fs::create_dir_all(root.join("parent"))?;
        fs::write(root.join("mod.rs"), "mod parent;\nfn main() {}\n")?;
        fs::write(root.join("parent.rs"), "mod types;\nmod consumer;\n")?;
        fs::write(root.join("parent/types.rs"), "pub struct MyType;\n")?;
        fs::write(
            root.join("parent/consumer.rs"),
            "use crate::skips_use_statements::parent::types::MyType;\n\nfn example(_x: MyType) {}\n",
        )?;

        Ok(move |report: &Report| {
            let report = member_report(report, "src/skips_use_statements");
            assert!(
                !report
                    .findings
                    .iter()
                    .any(|f| f.diagnostic_code == DiagnosticCode::InlinePathQualifiedType),
                "use statements should not be flagged as inline path types"
            );
            Ok(())
        })
    }

    fn conflicting_private_import_keeps_qualified_type(
        batch: &mut InlinePathBatch,
    ) -> anyhow::Result<impl FnOnce(&Report) -> anyhow::Result<()> + use<>> {
        let root = batch.add_module(
            "conflicting_private_import_keeps_qualified_type",
            &[("mod.rs", "")],
        )?;
        let source = r#"use std::error::Error;
use std::io;

fn completed_capture() -> Result<(), io::Error> {
    Err(io::Error::other("capture failed"))
}

fn boxed_error(error: io::Error) -> Box<dyn Error> {
    Box::new(error)
}

fn main() {}
"#;
        fs::write(root.join("mod.rs"), source)?;

        Ok(move |report: &Report| {
            let report = member_report(
                report,
                "src/conflicting_private_import_keeps_qualified_type",
            );
            assert!(
                !report
                    .findings
                    .iter()
                    .any(|finding| finding.diagnostic_code
                        == DiagnosticCode::InlinePathQualifiedType),
                "conflicting private import should suppress the inline-type finding"
            );

            let main_rs = fs::read_to_string(root.join("mod.rs"))?;
            assert_eq!(
                main_rs,
                source.replacen(
                    "    pub trait CaptureError",
                    "    pub(crate) trait CaptureError",
                    1
                )
            );
            Ok(())
        })
    }

    fn grouped_private_import_keeps_qualified_type(
        batch: &mut InlinePathBatch,
    ) -> anyhow::Result<impl FnOnce(&Report) -> anyhow::Result<()> + use<>> {
        let root = batch.add_module(
            "grouped_private_import_keeps_qualified_type",
            &[("mod.rs", "")],
        )?;
        let source = r#"use std::{error::Error, io};

fn completed_capture() -> Result<(), io::Error> {
    Err(io::Error::other("capture failed"))
}

fn boxed_error(error: io::Error) -> Box<dyn Error> {
    Box::new(error)
}

fn main() {}
"#;
        fs::write(root.join("mod.rs"), source)?;

        Ok(move |_: &Report| {
            let main_rs = fs::read_to_string(root.join("mod.rs"))?;
            assert_eq!(main_rs, source);
            Ok(())
        })
    }

    fn bare_name_shadowing_skipped(
        batch: &mut InlinePathBatch,
    ) -> anyhow::Result<impl FnOnce(&Report) -> anyhow::Result<()> + use<>> {
        let root = batch.add_module("bare_name_shadowing_skipped", &[("mod.rs", "")])?;
        fs::create_dir_all(root.join("error"))?;
        fs::write(
            root.join("mod.rs"),
            "mod error;\nmod consumer;\nfn main() {}\n",
        )?;
        fs::write(
            root.join("error.rs"),
            "pub type Result<T> = core::result::Result<T, String>;\n",
        )?;
        // consumer uses both prelude Result<T, E> and
        // crate::bare_name_shadowing_skipped::error::Result<T> inline
        fs::write(
            root.join("consumer.rs"),
            r#"fn uses_prelude() -> Result<String, String> {
    Ok("hello".to_string())
}
fn uses_inline() -> crate::bare_name_shadowing_skipped::error::Result<String> {
    Ok("hello".to_string())
}
"#,
        )?;

        Ok(move |_: &Report| {
            let consumer = fs::read_to_string(root.join("consumer.rs"))?;
            // The inline path should NOT be replaced because doing so would
            // add `use crate::bare_name_shadowing_skipped::error::Result;` which shadows prelude
            // `Result<T, E>`
            assert!(
                consumer.contains("crate::bare_name_shadowing_skipped::error::Result<String>"),
                "inline path should be left alone to avoid shadowing prelude Result, got:\n{consumer}"
            );
            // Prelude usage should remain unchanged
            assert!(
                consumer.contains("Result<String, String>"),
                "prelude Result should be unchanged, got:\n{consumer}"
            );
            Ok(())
        })
    }

    fn multi_segment_path_with_pascal_first_segment_blocks_shadowing_import(
        batch: &mut InlinePathBatch,
    ) -> anyhow::Result<impl FnOnce(&Report) -> anyhow::Result<()> + use<>> {
        let root = batch.add_module(
            "multi_segment_path_with_pascal_first_segment_blocks_shadowing_import",
            &[("mod.rs", "")],
        )?;
        fs::write(
            root.join("mod.rs"),
            r#"use std::io;

pub fn open(path: &str) -> io::Result<String> {
    let _ = [Ok::<_, io::Error>(())].into_iter().filter_map(Result::ok);
    std::fs::read_to_string(path)
}

fn main() {
    let _ = open("x");
}
"#,
        )?;

        Ok(move |_: &Report| {
            let main_rs = fs::read_to_string(root.join("mod.rs"))?;
            assert!(
                !main_rs.contains("use io::Result;") && !main_rs.contains("use std::io::Result;"),
                "must not import a name that would shadow prelude `Result`, got:\n{main_rs}"
            );
            Ok(())
        })
    }

    fn does_not_import_prelude_names(
        batch: &mut InlinePathBatch,
    ) -> anyhow::Result<impl FnOnce(&Report) -> anyhow::Result<()> + use<>> {
        let root = batch.add_module("does_not_import_prelude_names", &[("mod.rs", "")])?;
        fs::write(
            root.join("mod.rs"),
            // `std::fmt::Result` is the type alias for `Result<(), fmt::Error>`.
            // Bringing it in as plain `Result` would shadow the prelude generic.
            r#"use std::fmt;

pub struct Marker;

impl fmt::Display for Marker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("marker")
    }
}

fn main() {}
"#,
        )?;

        Ok(move |_: &Report| {
            let main_rs = fs::read_to_string(root.join("mod.rs"))?;
            assert!(
                !main_rs.contains("use fmt::Result;") && !main_rs.contains("use std::fmt::Result;"),
                "must not import a name that shadows prelude `Result`, got:\n{main_rs}"
            );
            Ok(())
        })
    }

    fn generic_type_param_associated_items_are_not_flagged(
        batch: &mut InlinePathBatch,
    ) -> anyhow::Result<impl FnOnce(&Report) -> anyhow::Result<()> + use<>> {
        let root = batch.add_module(
            "generic_type_param_associated_items_are_not_flagged",
            &[("mod.rs", "")],
        )?;
        fs::write(
            root.join("mod.rs"),
            r#"pub trait Sink {
    type Ok;
    type Error;
    fn finish(self) -> Result<Self::Ok, Self::Error>;
}

pub fn run<S: Sink>(s: S) -> Result<S::Ok, S::Error> {
    s.finish()
}

pub trait Bucket {
    type Item;
}

pub fn first<B: Bucket>(_b: &B) -> Option<B::Item> {
    None
}

// PascalCase generic name — already covered by the PascalCase-first-segment
// filter, but exercise the generics-tracker path too.
pub fn next_idx<Idx: Iterator>(it: &mut Idx) -> Option<Idx::Item> {
    it.next()
}

// Non-idiomatic lowercase generic. `non_camel_case_types` warns, but this
// verifies generic tracking comes from syntax, not naming convention.
#[allow(
    non_camel_case_types,
    reason = "fixture intentionally covers lowercase generic parameters"
)]
pub fn lower<t: Bucket>(_b: &t) -> Option<t::Item> {
    None
}

// `syn::Generics::params` on an impl block remains in scope while visiting
// `ImplItemFn` bodies; the method's own signature can add generics.
pub struct Wrap<T>(pub T);
impl<T: Bucket> Wrap<T> {
    pub fn nested<R: Sink>(_r: R) -> Option<(T::Item, R::Ok, R::Error)> {
        None
    }
}

fn main() {}
"#,
        )?;

        Ok(move |report: &Report| {
            let report = member_report(
                report,
                "src/generic_type_param_associated_items_are_not_flagged",
            );
            let inline_findings: Vec<_> = report
                .findings
                .iter()
                .filter(|f| f.diagnostic_code == DiagnosticCode::InlinePathQualifiedType)
                .collect();
            assert!(
                inline_findings.is_empty(),
                "associated items on generic params must not be flagged, got: \
                 {inline_findings:?}"
            );
            Ok(())
        })
    }

    fn generic_type_param_in_fn_body_is_not_flagged(
        batch: &mut InlinePathBatch,
    ) -> anyhow::Result<impl FnOnce(&Report) -> anyhow::Result<()> + use<>> {
        let root = batch.add_module(
            "generic_type_param_in_fn_body_is_not_flagged",
            &[("mod.rs", "")],
        )?;
        fs::write(
            root.join("mod.rs"),
            r"pub trait Bucket {
    type Item;
}

// Closure parameter type uses the fn's generic — used to be flagged because
// the generic was popped before the body was visited.
pub fn drive<A: Bucket>(_items: &[A::Item]) -> usize {
    let _count = |_x: &A::Item, acc: &mut usize| {
        *acc += 1;
    };
    0
}

// Same situation inside an impl method.
pub struct Wrap<T>(pub T);
impl<T: Bucket> Wrap<T> {
    pub fn run(&self, _items: &[T::Item]) -> usize {
        let _take = |_x: &T::Item| {};
        0
    }
}

// Trait method body referencing the impl's generic.
pub trait Driver<T: Bucket> {
    fn drive(&self, items: &[T::Item]) -> usize {
        let _take = |_x: &T::Item| {};
        items.len()
    }
}

fn main() {}
",
        )?;

        Ok(move |report: &Report| {
            let report = member_report(report, "src/generic_type_param_in_fn_body_is_not_flagged");
            let inline_findings: Vec<_> = report
                .findings
                .iter()
                .filter(|f| f.diagnostic_code == DiagnosticCode::InlinePathQualifiedType)
                .collect();
            assert!(
                inline_findings.is_empty(),
                "generic-param associated items in fn bodies must not be flagged, got: \
                 {inline_findings:?}"
            );
            Ok(())
        })
    }

    fn enum_variant_two_segment_path_is_not_flagged(
        batch: &mut InlinePathBatch,
    ) -> anyhow::Result<impl FnOnce(&Report) -> anyhow::Result<()> + use<>> {
        let root = batch.add_module(
            "enum_variant_two_segment_path_is_not_flagged",
            &[("mod.rs", "")],
        )?;
        fs::write(
            root.join("mod.rs"),
            r"pub struct Package;

pub enum RustProject {
    Package(Package),
}

pub fn unwrap(item: RustProject) -> Option<Package> {
    let RustProject::Package(pkg) = item else {
        return None;
    };
    Some(pkg)
}

fn main() {}
",
        )?;

        Ok(move |report: &Report| {
            let report = member_report(report, "src/enum_variant_two_segment_path_is_not_flagged");
            assert!(
                !report
                    .findings
                    .iter()
                    .any(|f| f.diagnostic_code == DiagnosticCode::InlinePathQualifiedType),
                "`Type::Variant` patterns must not be flagged, got findings: {:?}",
                report
                    .findings
                    .iter()
                    .filter(|f| f.diagnostic_code == DiagnosticCode::InlinePathQualifiedType)
                    .collect::<Vec<_>>()
            );
            Ok(())
        })
    }

    #[test]
    fn preserves_colliding_and_non_type_paths_in_independent_modules() -> anyhow::Result<()> {
        let mut batch = InlinePathBatch::new()?;
        let name_collision_skips_both = name_collision_skips_both(&mut batch)?;
        let skips_use_statements = skips_use_statements(&mut batch)?;
        let conflicting_private_import_keeps_qualified_type =
            conflicting_private_import_keeps_qualified_type(&mut batch)?;
        let grouped_private_import_keeps_qualified_type =
            grouped_private_import_keeps_qualified_type(&mut batch)?;
        let bare_name_shadowing_skipped = bare_name_shadowing_skipped(&mut batch)?;
        let multi_segment_path_with_pascal_first_segment_blocks_shadowing_import =
            multi_segment_path_with_pascal_first_segment_blocks_shadowing_import(&mut batch)?;
        let does_not_import_prelude_names = does_not_import_prelude_names(&mut batch)?;
        let generic_type_param_associated_items_are_not_flagged =
            generic_type_param_associated_items_are_not_flagged(&mut batch)?;
        let generic_type_param_in_fn_body_is_not_flagged =
            generic_type_param_in_fn_body_is_not_flagged(&mut batch)?;
        let enum_variant_two_segment_path_is_not_flagged =
            enum_variant_two_segment_path_is_not_flagged(&mut batch)?;
        let scanned = batch.scan()?;
        let report = Report {
            findings: scanned.findings,
            ..Report::default()
        };
        imports::apply_fixes(&scanned.fixes)?;
        let output = batch.compile()?;
        assert!(
            output.status.success(),
            "batch fix failed: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        name_collision_skips_both(&report)?;
        skips_use_statements(&report)?;
        conflicting_private_import_keeps_qualified_type(&report)?;
        grouped_private_import_keeps_qualified_type(&report)?;
        bare_name_shadowing_skipped(&report)?;
        multi_segment_path_with_pascal_first_segment_blocks_shadowing_import(&report)?;
        does_not_import_prelude_names(&report)?;
        generic_type_param_associated_items_are_not_flagged(&report)?;
        generic_type_param_in_fn_body_is_not_flagged(&report)?;
        enum_variant_two_segment_path_is_not_flagged(&report)?;
        Ok(())
    }
}
