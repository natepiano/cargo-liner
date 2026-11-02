use std::fs;

use anyhow::Result;
use syn::Item;
use syn::Meta;
use syn::UseTree;

use super::in_body_use_finder;
use crate::fixes::imports;
use crate::fixes::imports::ValidatedFixSet;

fn assert_import_keeps_gate(template: &str) -> Result<()> {
    let text = format!(
        "{}\nfn sibling() {{ use crate::sibling::Other; }}\n",
        template.replace("IMPORT_BLOCK", "{ use absent_dependency::Value; 0 }")
    );
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("fixture.rs");
    fs::write(&path, &text)?;
    let syntax = syn::parse_file(&text)?;
    let (findings, fixes) = in_body_use_finder::scan(&syntax, &text, &path, "fixture.rs")?;
    assert_eq!(findings.len(), 2, "{text}");
    imports::apply_fixes(&ValidatedFixSet::try_from(fixes)?)?;
    let rewritten = fs::read_to_string(&path)?;
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
    for import in imports {
        let UseTree::Path(root) = &import.tree else {
            anyhow::bail!("expected a dependency import: {rewritten}");
        };
        if root.ident == "absent_dependency" {
            assert_eq!(import.attrs.len(), 1, "{rewritten}");
            let Meta::List(cfg) = &import.attrs[0].meta else {
                anyhow::bail!("expected a cfg attribute: {rewritten}");
            };
            assert!(cfg.path.is_ident("cfg"), "{rewritten}");
            assert_eq!(cfg.tokens.to_string(), "any ()", "{rewritten}");
        } else {
            assert!(import.attrs.is_empty(), "{rewritten}");
        }
    }
    assert_eq!(
        rewritten.matches("use absent_dependency::Value;").count(),
        1
    );
    Ok(())
}

macro_rules! gated_import_regressions {
    ($($name:ident => $source:expr),+ $(,)?) => {
        $(
            #[test]
            fn $name() -> Result<()> {
                assert_import_keeps_gate($source)
            }
        )+
    };
}

gated_import_regressions! {
    typed_function_parameter_keeps_gate =>
        "fn f(#[cfg(any())] _: [u8; IMPORT_BLOCK]) {}",
    typed_receiver_keeps_gate =>
        "impl Census { fn f(#[cfg(any())] self: Wrapper<IMPORT_BLOCK>) {} }",
    function_pointer_parameter_keeps_gate =>
        "type Callback = fn(#[cfg(any())] [u8; IMPORT_BLOCK]);",
    struct_literal_field_keeps_gate =>
        "fn f() { let _ = State { #[cfg(any())] value: IMPORT_BLOCK }; }",
    generic_type_parameter_keeps_gate =>
        "struct State<#[cfg(any())] T = [u8; IMPORT_BLOCK]>;",
    generic_const_parameter_keeps_gate =>
        "struct State<#[cfg(any())] const N: usize = IMPORT_BLOCK>;",
    where_predicate_keeps_gate =>
        "fn f() where #[cfg(any())] [u8; IMPORT_BLOCK]: Copy {}",
    variadic_parameter_keeps_gate =>
        "unsafe extern \"C\" fn f(#[cfg(any())] const IMPORT_BLOCK: ...) {}",
    typed_closure_parameter_keeps_gate =>
        "fn f() { let _ = |#[cfg(any())] _: [u8; IMPORT_BLOCK]| {}; }",
    const_pattern_keeps_gate =>
        "fn f() { let _ = |#[cfg(any())] const IMPORT_BLOCK| {}; }",
    binding_pattern_keeps_gate =>
        "fn f() { let _ = |#[cfg(any())] value @ const IMPORT_BLOCK| {}; }",
    parenthesized_pattern_keeps_gate =>
        "fn f() { let _ = |#[cfg(any())] (const IMPORT_BLOCK)| {}; }",
    reference_pattern_keeps_gate =>
        "fn f() { let _ = |#[cfg(any())] &const IMPORT_BLOCK| {}; }",
    slice_pattern_keeps_gate =>
        "fn f() { let _ = |#[cfg(any())] [const IMPORT_BLOCK]| {}; }",
    struct_pattern_keeps_gate =>
        "fn f() { let _ = |#[cfg(any())] State { value: const IMPORT_BLOCK }| {}; }",
    tuple_pattern_keeps_gate =>
        "fn f() { let _ = |#[cfg(any())] (const IMPORT_BLOCK,)| {}; }",
    tuple_struct_pattern_keeps_gate =>
        "fn f() { let _ = |#[cfg(any())] State(const IMPORT_BLOCK)| {}; }",
    path_pattern_keeps_gate =>
        "fn f() { let _ = |#[cfg(any())] State::<IMPORT_BLOCK>::VALUE| {}; }",
    struct_pattern_field_keeps_gate =>
        "fn f() { let State { #[cfg(any())] value: const IMPORT_BLOCK } = todo!(); }",
}
