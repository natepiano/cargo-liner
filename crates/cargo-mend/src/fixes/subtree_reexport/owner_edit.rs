use std::collections::BTreeMap;
use std::fs;
use std::ops::Range;
use std::path::Path;

use anyhow::Context;
use anyhow::Result;
use syn::Ident;
use syn::Item;
use syn::ItemMod;
use syn::ItemUse;
use syn::UseTree;
use syn::Visibility;
use syn::parse_file;
use syn::spanned::Spanned;

use super::crate_files::CrateFiles;
use super::local_use::GlobReach;
use super::local_use::LocalUse;
use crate::fixes::constants::CFG_TEST_ATTRIBUTE;
use crate::fixes::facade_redirect;
use crate::fixes::facade_redirect::SourceLines;
use crate::fixes::imports::UseFix;
use crate::fixes::visibility_annotation_site;
use crate::reporting::ModuleForm;
use crate::reporting::SubtreeReexportFixFact;
use crate::rust_syntax;

/// One fact of an owner region, by its index in the scan's fact list.
pub(super) struct OwnerFact<'a> {
    pub(super) index: usize,
    pub(super) fact:  &'a SubtreeReexportFixFact,
}

/// The owner edits of one region, or the facts whose `use` item or leaf the
/// source does not hold as the fact describes.
pub(super) enum RegionEdit {
    Edited(Vec<UseFix>),
    Unmatched(Vec<usize>),
}

/// What becomes of an inline owner module the edits leave with no items.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum EmptiedOwner {
    /// Delete the `mod` item, its docs and attributes included.
    Delete,
    /// Keep the empty `mod` item: a caller's `use` still names the module.
    Keep,
}

/// A fact placed in the owner file: the `use` item holding its leaf and the
/// inline module around that item, if any.
struct Located<'s, 'f> {
    fact:      &'f OwnerFact<'f>,
    item_use:  &'s ItemUse,
    enclosing: Option<&'s ItemMod>,
}

/// A removed leaf the owner module still needs, and how.
struct UsedLeaf<'a> {
    written_path: &'a str,
    local_use:    LocalUse,
}

/// What one edited `use` item becomes.
struct ItemRewrite {
    range:  Range<usize>,
    pieces: Vec<String>,
}

/// Removes every fact's re-export from the owner module in `file` (all facts
/// share one owner module), keeping a private `use` of a name the owner
/// module still uses. An inline owner module left with no items is deleted
/// unless `emptied` says to keep it.
pub(super) fn edit_owner_region(
    file: &Path,
    facts: &[OwnerFact<'_>],
    crate_files: &CrateFiles,
    emptied: EmptiedOwner,
) -> Result<RegionEdit> {
    let source =
        fs::read_to_string(file).with_context(|| format!("failed to read {}", file.display()))?;
    edit_owner_source(file, &source, facts, crate_files, emptied)
}

fn edit_owner_source(
    file: &Path,
    source: &str,
    facts: &[OwnerFact<'_>],
    crate_files: &CrateFiles,
    emptied: EmptiedOwner,
) -> Result<RegionEdit> {
    let syntax =
        parse_file(source).with_context(|| format!("failed to parse {}", file.display()))?;
    let lines = SourceLines::new(source);
    let mut located = Vec::new();
    let mut unmatched = Vec::new();
    for fact in facts {
        match locate(source, &lines, &syntax.items, fact) {
            Some(found) => located.push(found),
            None => unmatched.push(fact.index),
        }
    }
    if !unmatched.is_empty() {
        return Ok(RegionEdit::Unmatched(unmatched));
    }

    // Two copies of one inline module under different `#[cfg]`s are two
    // regions of the same module path.
    let mut regions: BTreeMap<Option<usize>, Vec<Located<'_, '_>>> = BTreeMap::new();
    for found in located {
        let key = found
            .enclosing
            .map(|item_mod| lines.offset(item_mod.span().start()));
        regions.entry(key).or_default().push(found);
    }
    let mut fixes = Vec::new();
    for region in regions.into_values() {
        fixes.extend(edit_region(
            file,
            source,
            &lines,
            &syntax.items,
            &region,
            crate_files,
            emptied,
        )?);
    }
    Ok(RegionEdit::Edited(fixes))
}

fn edit_region(
    file: &Path,
    source: &str,
    lines: &SourceLines<'_>,
    file_items: &[Item],
    region: &[Located<'_, '_>],
    crate_files: &CrateFiles,
    emptied: EmptiedOwner,
) -> Result<Vec<UseFix>> {
    let enclosing = region.first().and_then(|found| found.enclosing);
    let (region_range, region_items) = match enclosing {
        Some(item_mod) => {
            let Some((brace, items)) = &item_mod.content else {
                return Ok(Vec::new());
            };
            let start = lines.offset(brace.span.open().end());
            let end = lines.offset(brace.span.close().start());
            (start..end, items.as_slice())
        },
        None => (0..source.len(), file_items),
    };

    // Group the facts by the `use` item holding them, keyed by its start.
    let mut items: BTreeMap<usize, Vec<&Located<'_, '_>>> = BTreeMap::new();
    for found in region {
        let (start, _) = facade_redirect::item_use_byte_range(lines, found.item_use);
        items.entry(start).or_default().push(found);
    }

    // Blank the region's edited items and its nested inline modules, and
    // everything outside the region, before looking for local uses.
    let mut excluded = vec![0..region_range.start, region_range.end..source.len()];
    for group in items.values() {
        let (start, end) = facade_redirect::item_use_byte_range(lines, group[0].item_use);
        excluded.push(start..end);
    }
    for item in region_items {
        if let Item::Mod(item_mod) = item
            && item_mod.content.is_some()
        {
            excluded
                .push(lines.offset(item_mod.span().start())..lines.offset(item_mod.span().end()));
        }
    }

    let owner = &region[0].fact.fact.owner_module;
    let mut rewrites = Vec::new();
    for group in items.values() {
        let mut used_leaves = Vec::new();
        for found in group {
            let name = &found.fact.fact.exported_name;
            let local_use =
                if facade_redirect::word_occurs_outside(source, excluded.iter().cloned(), name) {
                    LocalUse::Always
                } else {
                    let reach = GlobReach { owner, name };
                    let inline = reach.items_reach(source, lines, region_items, owner);
                    if inline == LocalUse::Always {
                        inline
                    } else {
                        inline.max(reach.files_reach(crate_files, file)?)
                    }
                };
            if local_use != LocalUse::Unused {
                used_leaves.push(UsedLeaf {
                    written_path: found.fact.fact.written_path.as_str(),
                    local_use,
                });
            }
        }
        rewrites.push(rewrite_item(source, lines, group, &used_leaves));
    }

    let delete_module = emptied == EmptiedOwner::Delete
        && region_items.len() == rewrites.len()
        && rewrites.iter().all(|rewrite| rewrite.pieces.is_empty());
    if let Some(item_mod) = enclosing.filter(|_| delete_module) {
        let range = lines.offset(item_mod.span().start())..lines.offset(item_mod.span().end());
        return Ok(vec![facade_redirect::line_deletion(file, source, range)]);
    }
    Ok(rewrites
        .into_iter()
        .map(|rewrite| {
            if rewrite.pieces.is_empty() {
                facade_redirect::line_deletion(file, source, rewrite.range)
            } else {
                let separator = format!("\n{}", lines.indent_before(rewrite.range.start));
                UseFix {
                    path:         file.to_path_buf(),
                    start:        rewrite.range.start,
                    end:          rewrite.range.end,
                    replacement:  rewrite.pieces.join(&separator),
                    import_group: None,
                }
            }
        })
        .collect())
}

/// The `use` item without the facts' leaves, followed by a private `use` of
/// each removed path in `used_leaves`, both under the item's attributes. A
/// leaf only tests use gains `#[cfg(test)]` unless the item already has it.
fn rewrite_item(
    source: &str,
    lines: &SourceLines<'_>,
    group: &[&Located<'_, '_>],
    used_leaves: &[UsedLeaf<'_>],
) -> ItemRewrite {
    let item_use = group[0].item_use;
    let (start, end) = facade_redirect::item_use_byte_range(lines, item_use);
    let removed = group
        .iter()
        .map(|found| written_segments(&found.fact.fact.written_path))
        .collect::<Vec<_>>();
    let kept = facade_redirect::remove_use_leaves(Vec::new(), &item_use.tree, &|prefix, ident| {
        removed.contains(&leaf_segments(prefix, ident))
    });
    let tree_start = lines.offset(item_use.tree.span().start());
    let attributes_end = match &item_use.vis {
        Visibility::Inherited => lines.offset(item_use.use_token.span.start()),
        visibility => lines.offset(visibility.span().start()),
    };
    let attributes = &source[start..attributes_end];
    let mut pieces = Vec::new();
    if let Some(kept) = kept {
        pieces.push(format!(
            "{}{};",
            &source[start..tree_start],
            facade_redirect::render_use_tree(&kept)
        ));
    }
    let test_gate = if rust_syntax::is_cfg_test(&item_use.attrs) {
        String::new()
    } else {
        format!("{CFG_TEST_ATTRIBUTE}\n{}", lines.indent_before(start))
    };
    for leaf in used_leaves {
        let gate = if leaf.local_use == LocalUse::TestOnly {
            test_gate.as_str()
        } else {
            ""
        };
        pieces.push(format!("{attributes}{gate}use {};", leaf.written_path));
    }
    ItemRewrite {
        range: start..end,
        pieces,
    }
}

/// Finds the `use` item whose byte range holds the fact's position, and checks
/// it holds the fact's leaf and sits where the fact's module form says.
fn locate<'s, 'f>(
    source: &str,
    lines: &SourceLines<'_>,
    items: &'s [Item],
    fact: &'f OwnerFact<'f>,
) -> Option<Located<'s, 'f>> {
    let line_start = visibility_annotation_site::line_byte_offset(source, fact.fact.use_line)?;
    let line_end = source[line_start..]
        .find('\n')
        .map_or(source.len(), |index| line_start + index);
    let position = line_start
        + visibility_annotation_site::byte_offset_of_display_column(
            &source[line_start..line_end],
            fact.fact.use_column,
        )?;
    let (item_use, enclosing) = find_item_use(lines, items, None, position)?;
    let form_matches = match fact.fact.owner_module_form {
        ModuleForm::File => enclosing.is_none(),
        ModuleForm::Inline => enclosing.is_some(),
    };
    let leaf = written_segments(&fact.fact.written_path);
    (form_matches && has_name_leaf(Vec::new(), &item_use.tree, &leaf)).then_some(Located {
        fact,
        item_use,
        enclosing,
    })
}

fn find_item_use<'s>(
    lines: &SourceLines<'_>,
    items: &'s [Item],
    enclosing: Option<&'s ItemMod>,
    position: usize,
) -> Option<(&'s ItemUse, Option<&'s ItemMod>)> {
    items.iter().find_map(|item| match item {
        Item::Use(item_use) => {
            let (start, end) = facade_redirect::item_use_byte_range(lines, item_use);
            (start <= position && position < end).then_some((item_use, enclosing))
        },
        Item::Mod(item_mod) => {
            let (_, content) = item_mod.content.as_ref()?;
            find_item_use(lines, content, Some(item_mod), position)
        },
        _ => None,
    })
}

fn has_name_leaf(prefix: Vec<String>, tree: &UseTree, leaf: &[String]) -> bool {
    match tree {
        UseTree::Path(path) => {
            let mut next = prefix;
            next.push(path.ident.to_string());
            has_name_leaf(next, &path.tree, leaf)
        },
        UseTree::Name(name) => leaf_segments(&prefix, &name.ident) == leaf,
        UseTree::Group(group) => group
            .items
            .iter()
            .any(|item| has_name_leaf(prefix.clone(), item, leaf)),
        UseTree::Rename(_) | UseTree::Glob(_) => false,
    }
}

/// A `use` leaf's written segments without a leading `self`.
fn leaf_segments(prefix: &[String], ident: &Ident) -> Vec<String> {
    let mut segments = prefix.to_vec();
    segments.push(ident.to_string());
    rust_syntax::trim_leading_self(&segments).to_vec()
}

/// The fact's written path as segments without a leading `self`.
fn written_segments(written_path: &str) -> Vec<String> {
    let segments = written_path
        .split("::")
        .map(str::to_string)
        .collect::<Vec<_>>();
    rust_syntax::trim_leading_self(&segments).to_vec()
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::cmp::Reverse;
    use std::path::Path;

    use super::EmptiedOwner;
    use super::OwnerFact;
    use super::RegionEdit;
    use super::edit_owner_source;
    use crate::fixes::subtree_reexport::crate_files::CrateFiles;
    use crate::reporting::AncestorReexport;
    use crate::reporting::ModuleForm;
    use crate::reporting::SubtreeReexportFixFact;

    fn fact(
        line: usize,
        column: usize,
        name: &str,
        owner: &str,
        form: ModuleForm,
    ) -> SubtreeReexportFixFact {
        SubtreeReexportFixFact {
            use_path:          "src/owner.rs".to_string(),
            use_line:          line,
            use_column:        column,
            exported_name:     name.to_string(),
            written_path:      format!("crate::source::{name}"),
            owner_module:      vec![owner.to_string()],
            owner_module_form: form,
            source_module:     vec!["source".to_string()],
            target_scope:      Vec::new(),
            common_ancestor:   Vec::new(),
            crate_root_file:   "src/lib.rs".to_string(),
            ancestor_reexport: AncestorReexport::NotRequired,
        }
    }

    fn file_fact(line: usize, name: &str) -> SubtreeReexportFixFact {
        fact(line, 1, name, "owner", ModuleForm::File)
    }

    fn edit(source: &str, facts: &[SubtreeReexportFixFact], emptied: EmptiedOwner) -> RegionEdit {
        let owner_facts = facts
            .iter()
            .enumerate()
            .map(|(index, fact)| OwnerFact { index, fact })
            .collect::<Vec<_>>();
        edit_owner_source(
            Path::new("owner.rs"),
            source,
            &owner_facts,
            &CrateFiles::default(),
            emptied,
        )
        .expect("edit owner")
    }

    fn edited(source: &str, facts: &[SubtreeReexportFixFact]) -> String {
        edited_with(source, facts, EmptiedOwner::Delete)
    }

    fn edited_with(
        source: &str,
        facts: &[SubtreeReexportFixFact],
        emptied: EmptiedOwner,
    ) -> String {
        let RegionEdit::Edited(mut fixes) = edit(source, facts, emptied) else {
            panic!("every fact should be located");
        };
        fixes.sort_by_key(|fix| Reverse(fix.start));
        let mut text = source.to_string();
        for fix in fixes {
            text.replace_range(fix.start..fix.end, &fix.replacement);
        }
        text
    }

    #[test]
    fn a_group_keeps_its_other_leaves() {
        assert_eq!(
            edited(
                "pub use crate::source::{Widget, Other};\n",
                &[file_fact(1, "Widget")]
            ),
            "pub use crate::source::Other;\n"
        );
    }

    #[test]
    fn two_facts_on_one_item_are_edited_together() {
        assert_eq!(
            edited(
                "use x::Y;\npub use crate::source::{Widget, Other};\nfn f() {}\n",
                &[file_fact(2, "Widget"), file_fact(2, "Other")]
            ),
            "use x::Y;\nfn f() {}\n"
        );
    }

    #[test]
    fn a_locally_used_name_keeps_a_private_use_under_its_attributes() {
        assert_eq!(
            edited(
                "#[cfg(test)]\npub(crate) use crate::source::{Widget, Other};\nfn f() -> Widget { todo!() }\n",
                &[file_fact(2, "Widget")]
            ),
            "#[cfg(test)]\npub(crate) use crate::source::Other;\n#[cfg(test)]\nuse crate::source::Widget;\nfn f() -> Widget { todo!() }\n"
        );
    }

    #[test]
    fn an_unused_cfg_reexport_goes_with_its_attributes() {
        assert_eq!(
            edited(
                "#[cfg(test)]\npub use crate::source::Widget;\nfn f() {}\n",
                &[file_fact(2, "Widget")]
            ),
            "fn f() {}\n"
        );
    }

    #[test]
    fn an_emptied_inline_owner_is_deleted_with_its_docs() {
        assert_eq!(
            edited(
                "/// Test access.\n#[cfg(test)]\nmod panel {\n    pub use crate::source::Widget;\n}\nfn g() {}\n",
                &[fact(4, 5, "Widget", "panel", ModuleForm::Inline)]
            ),
            "fn g() {}\n"
        );
    }

    #[test]
    fn an_emptied_inline_owner_a_caller_still_names_is_kept_empty() {
        assert_eq!(
            edited_with(
                "mod panel {\n    pub use crate::source::Widget;\n}\nfn g() {}\n",
                &[fact(2, 5, "Widget", "panel", ModuleForm::Inline)],
                EmptiedOwner::Keep,
            ),
            "mod panel {\n}\nfn g() {}\n"
        );
    }

    #[test]
    fn an_inline_owner_using_the_name_keeps_it_privately() {
        assert_eq!(
            edited(
                "mod panel {\n    pub use crate::source::Widget;\n    fn f() -> Widget { todo!() }\n}\n",
                &[fact(2, 5, "Widget", "panel", ModuleForm::Inline)]
            ),
            "mod panel {\n    use crate::source::Widget;\n    fn f() -> Widget { todo!() }\n}\n"
        );
    }

    #[test]
    fn a_nested_glob_of_the_owner_keeps_a_private_use() {
        assert_eq!(
            edited(
                "pub use crate::source::Widget;\nmod tests {\n    use super::*;\n    fn f() { Widget::new(); }\n}\n",
                &[file_fact(1, "Widget")]
            ),
            "use crate::source::Widget;\nmod tests {\n    use super::*;\n    fn f() { Widget::new(); }\n}\n"
        );
    }

    #[test]
    fn a_cfg_test_glob_of_the_owner_keeps_a_cfg_test_private_use() {
        assert_eq!(
            edited(
                "pub use crate::source::Widget;\n#[cfg(test)]\nmod tests {\n    use super::*;\n    fn f() { Widget::new(); }\n}\n",
                &[file_fact(1, "Widget")]
            ),
            "#[cfg(test)]\nuse crate::source::Widget;\n#[cfg(test)]\nmod tests {\n    use super::*;\n    fn f() { Widget::new(); }\n}\n"
        );
    }

    #[test]
    fn an_inline_owner_writes_the_cfg_test_gate_at_its_indent() {
        assert_eq!(
            edited(
                "mod panel {\n    pub use crate::source::Widget;\n    #[cfg(test)]\n    mod tests {\n        use super::*;\n        fn f() { Widget::new(); }\n    }\n}\n",
                &[fact(2, 5, "Widget", "panel", ModuleForm::Inline)]
            ),
            "mod panel {\n    #[cfg(test)]\n    use crate::source::Widget;\n    #[cfg(test)]\n    mod tests {\n        use super::*;\n        fn f() { Widget::new(); }\n    }\n}\n"
        );
    }

    #[test]
    fn a_cfg_test_reexport_keeps_one_cfg_test_gate() {
        assert_eq!(
            edited(
                "#[cfg(test)]\npub use crate::source::Widget;\n#[cfg(test)]\nmod tests {\n    use super::*;\n    fn f() { Widget::new(); }\n}\n",
                &[file_fact(2, "Widget")]
            ),
            "#[cfg(test)]\nuse crate::source::Widget;\n#[cfg(test)]\nmod tests {\n    use super::*;\n    fn f() { Widget::new(); }\n}\n"
        );
    }

    #[test]
    fn a_doc_comment_mention_keeps_no_private_use() {
        assert_eq!(
            edited(
                "//! Builds a [`Widget`].\npub use crate::source::Widget;\nfn g() {}\n",
                &[file_fact(2, "Widget")]
            ),
            "//! Builds a [`Widget`].\nfn g() {}\n"
        );
    }

    #[test]
    fn a_nested_path_caller_does_not_keep_a_private_use() {
        assert_eq!(
            edited(
                "pub use crate::source::Widget;\nmod tests {\n    fn f() { super::Widget::new(); }\n}\n",
                &[file_fact(1, "Widget")]
            ),
            "mod tests {\n    fn f() { super::Widget::new(); }\n}\n"
        );
    }

    #[test]
    fn a_fact_off_its_use_item_is_unmatched() {
        assert!(matches!(
            edit(
                "fn f() {}\npub use crate::source::Widget;\n",
                &[file_fact(1, "Widget")],
                EmptiedOwner::Delete,
            ),
            RegionEdit::Unmatched(indexes) if indexes == [0]
        ));
    }
}
