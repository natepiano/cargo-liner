use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::ops::Range;
use std::path::Path;

use anyhow::Context;
use anyhow::Result;
use syn::File;
use syn::Ident;
use syn::Item;
use syn::ItemMod;
use syn::ItemUse;
use syn::UseTree;
use syn::Visibility;
use syn::spanned::Spanned;
use syn::token::Brace;

use super::crate_files::OwnerCrate;
use super::local_use::GlobReach;
use super::local_use::LocalUse;
use crate::fixes::constants::CFG_TEST_ATTRIBUTE;
use crate::fixes::facade_redirect;
use crate::fixes::facade_redirect::IdentifierUse;
use crate::fixes::facade_redirect::LeafRetention;
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

/// The source region that holds an owner fact. Inline regions carry their
/// brace and items: an out-of-line module cannot enter inline editing.
#[derive(Clone, Copy)]
enum OwnerRegion<'s> {
    File,
    Inline {
        module: &'s ItemMod,
        brace:  &'s Brace,
        items:  &'s [Item],
    },
}

/// A fact placed in its use item and its actual source region.
struct Located<'s, 'f> {
    fact:     &'f OwnerFact<'f>,
    item_use: &'s ItemUse,
    region:   OwnerRegion<'s>,
}

/// Editing requires a first fact; an empty group cannot fall back to file
/// editing or reach an indexed first-element access.
struct NonEmptyOwnerRegion<'s, 'f> {
    first: Located<'s, 'f>,
    rest:  Vec<Located<'s, 'f>>,
}

impl<'s, 'f> NonEmptyOwnerRegion<'s, 'f> {
    fn iter(&self) -> impl Iterator<Item = &Located<'s, 'f>> {
        std::iter::once(&self.first).chain(&self.rest)
    }
}

/// The use item and its nonempty associated facts, joined while grouping by
/// that item's source start. Rewriting never chooses an item from an empty
/// group or accepts facts carrying their own, possibly different item.
struct ItemRewriteGroup<'s, 'f> {
    item_use: &'s ItemUse,
    first:    &'f OwnerFact<'f>,
    rest:     Vec<&'f OwnerFact<'f>>,
}

impl<'f> ItemRewriteGroup<'_, 'f> {
    fn facts(&self) -> impl Iterator<Item = &'f OwnerFact<'f>> + '_ {
        std::iter::once(self.first).chain(self.rest.iter().copied())
    }
}

/// Only required leaves reach rewriting; an unused leaf cannot accidentally
/// become an unconditional private import.
enum RequiredLocalUse {
    TestOnly,
    Always,
}

/// A removed leaf the owner module still needs, and how.
struct UsedLeaf<'a> {
    written_path: &'a str,
    local_use:    RequiredLocalUse,
}

/// What one edited `use` item becomes.
enum ItemRewrite {
    Delete {
        range: Range<usize>,
    },
    Replace {
        range: Range<usize>,
        first: String,
        rest:  Vec<String>,
    },
}

/// Removes every fact's re-export from the owner module in `file` (all facts
/// share one owner module), keeping a private `use` of a name the owner
/// module still uses. An inline owner module left with no items is deleted
/// unless `emptied` says to keep it.
pub(super) fn edit_owner_region(
    file: &Path,
    facts: &[OwnerFact<'_>],
    owner_crate: OwnerCrate<'_>,
    emptied: EmptiedOwner,
) -> Result<RegionEdit> {
    let source = owner_crate
        .sources
        .source(file)
        .with_context(|| format!("failed to read {}", file.display()))?;
    let syntax = source
        .syntax()
        .with_context(|| format!("failed to parse {}", file.display()))?;
    edit_owner_source(file, source.text(), syntax, facts, owner_crate, emptied)
}

fn edit_owner_source(
    file: &Path,
    source: &str,
    syntax: &File,
    facts: &[OwnerFact<'_>],
    owner_crate: OwnerCrate<'_>,
    emptied: EmptiedOwner,
) -> Result<RegionEdit> {
    let lines = SourceLines::new(source);
    let mut located = Vec::new();
    let mut unmatched = Vec::new();
    for fact in facts {
        match locate(source, &lines, &syntax.items, fact)? {
            Some(found) => located.push(found),
            None => unmatched.push(fact.index),
        }
    }
    if !unmatched.is_empty() {
        return Ok(RegionEdit::Unmatched(unmatched));
    }

    // `regions` keeps two copies of one inline module under different `#[cfg]`s
    // separate: they are two regions of the same module path.
    let mut regions: BTreeMap<Option<usize>, NonEmptyOwnerRegion<'_, '_>> = BTreeMap::new();
    for found in located {
        let key = match found.region {
            OwnerRegion::File => None,
            OwnerRegion::Inline { module, .. } => {
                Some(SourceLines::offset(lines.position(module.span().start())?))
            },
        };
        match regions.entry(key) {
            Entry::Vacant(entry) => {
                entry.insert(NonEmptyOwnerRegion {
                    first: found,
                    rest:  Vec::new(),
                });
            },
            Entry::Occupied(mut entry) => entry.get_mut().rest.push(found),
        }
    }
    let mut fixes = Vec::new();
    for region in regions.into_values() {
        fixes.extend(edit_region(
            file,
            source,
            &lines,
            &syntax.items,
            &region,
            owner_crate,
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
    region: &NonEmptyOwnerRegion<'_, '_>,
    owner_crate: OwnerCrate<'_>,
    emptied: EmptiedOwner,
) -> Result<Vec<UseFix>> {
    let (region_range, region_items) = match region.first.region {
        OwnerRegion::Inline { brace, items, .. } => {
            let (start, end) =
                lines.span_range(brace.span.open().end(), brace.span.close().start())?;
            (start..end, items)
        },
        OwnerRegion::File => (0..source.len(), file_items),
    };

    // `items` groups facts with the use item holding them, keyed by its start.
    let mut items: BTreeMap<usize, ItemRewriteGroup<'_, '_>> = BTreeMap::new();
    for found in region.iter() {
        let (start, _) = facade_redirect::item_use_byte_range(lines, found.item_use)?;
        match items.entry(start) {
            Entry::Vacant(entry) => {
                entry.insert(ItemRewriteGroup {
                    item_use: found.item_use,
                    first:    found.fact,
                    rest:     Vec::new(),
                });
            },
            Entry::Occupied(mut entry) => entry.get_mut().rest.push(found.fact),
        }
    }

    // `excluded` identifies the region's edited items, its nested inline modules,
    // and everything outside the region for `facade_redirect::word_occurs_outside`
    // to blank before looking for local uses.
    let mut excluded = vec![0..region_range.start, region_range.end..source.len()];
    for group in items.values() {
        let (start, end) = facade_redirect::item_use_byte_range(lines, group.item_use)?;
        excluded.push(start..end);
    }
    for item in region_items {
        if let Item::Mod(item_mod) = item
            && item_mod.content.is_some()
        {
            let (start, end) = lines.span_range(item_mod.span().start(), item_mod.span().end())?;
            excluded.push(start..end);
        }
    }

    let owner = &region.first.fact.fact.owner_module;
    let mut rewrites = Vec::new();
    for group in items.values() {
        let mut used_leaves = Vec::new();
        for found in group.facts() {
            let name = &found.fact.exported_name;
            let local_use = match facade_redirect::word_occurs_outside(
                source,
                excluded.iter().cloned(),
                name,
            ) {
                IdentifierUse::Present | IdentifierUse::Unparseable => LocalUse::Always,
                IdentifierUse::Absent => {
                    let reach = GlobReach { owner, name };
                    let inline = reach.items_reach(source, lines, region_items, owner)?;
                    if inline == LocalUse::Always {
                        inline
                    } else {
                        inline.max(reach.files_reach(owner_crate, file)?)
                    }
                },
            };
            let local_use = match local_use {
                LocalUse::Unused => continue,
                LocalUse::TestOnly => RequiredLocalUse::TestOnly,
                LocalUse::Always => RequiredLocalUse::Always,
            };
            used_leaves.push(UsedLeaf {
                written_path: found.fact.written_path.as_str(),
                local_use,
            });
        }
        rewrites.push(rewrite_item(source, lines, group, &used_leaves)?);
    }

    let delete_module = emptied == EmptiedOwner::Delete
        && region_items.len() == rewrites.len()
        && rewrites
            .iter()
            .all(|rewrite| matches!(rewrite, ItemRewrite::Delete { .. }));
    if let OwnerRegion::Inline {
        module: item_mod, ..
    } = region.first.region
        && delete_module
    {
        let (start, end) = lines.span_range(item_mod.span().start(), item_mod.span().end())?;
        let range = start..end;
        return Ok(vec![facade_redirect::line_deletion(file, source, range)]);
    }
    Ok(rewrites
        .into_iter()
        .map(|rewrite| fix_for_rewrite(file, source, lines, rewrite))
        .collect())
}

fn fix_for_rewrite(
    file: &Path,
    source: &str,
    lines: &SourceLines<'_>,
    rewrite: ItemRewrite,
) -> UseFix {
    match rewrite {
        ItemRewrite::Delete { range } => facade_redirect::line_deletion(file, source, range),
        ItemRewrite::Replace { range, first, rest } => {
            let separator = format!("\n{}", lines.indent_before(range.start));
            let mut replacement = first;
            for piece in rest {
                replacement.push_str(&separator);
                replacement.push_str(&piece);
            }
            UseFix {
                path: file.to_path_buf(),
                start: range.start,
                end: range.end,
                replacement,
                import_group: None,
            }
        },
    }
}

/// The `use` item without the facts' leaves, followed by a private `use` of
/// each removed path in `used_leaves`, both under the item's attributes. A
/// leaf only tests use gains `#[cfg(test)]` unless the item already has it.
fn rewrite_item(
    source: &str,
    lines: &SourceLines<'_>,
    group: &ItemRewriteGroup<'_, '_>,
    used_leaves: &[UsedLeaf<'_>],
) -> Result<ItemRewrite> {
    let item_use = group.item_use;
    let (start, end) = facade_redirect::item_use_byte_range(lines, item_use)?;
    let removed = group
        .facts()
        .map(|found| written_segments(&found.fact.written_path))
        .collect::<Vec<_>>();
    let kept = facade_redirect::remove_use_leaves(Vec::new(), &item_use.tree, &|prefix, ident| {
        if removed.contains(&leaf_segments(prefix, ident)) {
            LeafRetention::Remove
        } else {
            LeafRetention::Keep
        }
    });
    let tree_start = SourceLines::offset(lines.position(item_use.tree.span().start())?);
    let attributes_position = match &item_use.vis {
        Visibility::Inherited => item_use.use_token.span.start(),
        Visibility::Public(visibility) => visibility.span().start(),
        Visibility::Restricted(visibility) => visibility.span().start(),
    };
    let attributes_end = SourceLines::offset(lines.position(attributes_position)?);
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
        let gate = match leaf.local_use {
            RequiredLocalUse::TestOnly => test_gate.as_str(),
            RequiredLocalUse::Always => "",
        };
        pieces.push(format!("{attributes}{gate}use {};", leaf.written_path));
    }
    let mut pieces = pieces.into_iter();
    Ok(pieces
        .next()
        .map_or(ItemRewrite::Delete { range: start..end }, |first| {
            ItemRewrite::Replace {
                range: start..end,
                first,
                rest: pieces.collect(),
            }
        }))
}

/// Finds the `use` item whose byte range holds the fact's position, and checks
/// it holds the fact's leaf and sits where the fact's module form says.
fn locate<'s, 'f>(
    source: &str,
    lines: &SourceLines<'_>,
    items: &'s [Item],
    fact: &'f OwnerFact<'f>,
) -> Result<Option<Located<'s, 'f>>> {
    let Some(line_start) = visibility_annotation_site::line_byte_offset(source, fact.fact.use_line)
    else {
        return Ok(None);
    };
    let line_end = source[line_start..]
        .find('\n')
        .map_or(source.len(), |index| line_start + index);
    let Some(column) = visibility_annotation_site::byte_offset_of_display_column(
        &source[line_start..line_end],
        fact.fact.use_column,
    ) else {
        return Ok(None);
    };
    let position = line_start + column;
    let Some((item_use, region)) = find_item_use(lines, items, OwnerRegion::File, position)? else {
        return Ok(None);
    };
    let form_matches = matches!(
        (fact.fact.owner_module_form, region),
        (ModuleForm::File, OwnerRegion::File) | (ModuleForm::Inline, OwnerRegion::Inline { .. })
    );
    let leaf = written_segments(&fact.fact.written_path);
    Ok(
        (form_matches && has_name_leaf(Vec::new(), &item_use.tree, &leaf)).then_some(Located {
            fact,
            item_use,
            region,
        }),
    )
}

fn find_item_use<'s>(
    lines: &SourceLines<'_>,
    items: &'s [Item],
    region: OwnerRegion<'s>,
    position: usize,
) -> Result<Option<(&'s ItemUse, OwnerRegion<'s>)>> {
    for item in items {
        if let Item::Use(item_use) = item {
            let (start, end) = facade_redirect::item_use_byte_range(lines, item_use)?;
            if start <= position && position < end {
                return Ok(Some((item_use, region)));
            }
        }
        if let Item::Mod(item_mod) = item
            && let Some((brace, content)) = &item_mod.content
            && let Some(found) = find_item_use(
                lines,
                content,
                OwnerRegion::Inline {
                    module: item_mod,
                    brace,
                    items: content,
                },
                position,
            )?
        {
            return Ok(Some(found));
        }
    }
    Ok(None)
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
    use crate::fixes::subtree_reexport::crate_files::OwnerCrate;
    use crate::reporting::AncestorReexport;
    use crate::reporting::ModuleForm;
    use crate::reporting::SubtreeReexportFixFact;
    use crate::rust_syntax::ParsedSources;

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
        let syntax = syn::parse_file(source).expect("parse owner");
        edit_owner_source(
            Path::new("owner.rs"),
            source,
            &syntax,
            &owner_facts,
            OwnerCrate {
                files:   &CrateFiles::default(),
                sources: &ParsedSources::default(),
            },
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
    fn inline_regions_require_content_and_match_the_fact_form() {
        let source = "mod external;\npub use crate::source::Widget;\n";
        assert!(matches!(
            edit(source, &[fact(2, 1, "Widget", "external", ModuleForm::Inline)],
                EmptiedOwner::Delete),
            RegionEdit::Unmatched(indexes) if indexes == [0]
        ));
        assert_eq!(edited(source, &[file_fact(2, "Widget")]), "mod external;\n");
    }

    #[test]
    fn separate_inline_definitions_keep_their_own_nonempty_regions() {
        let source = concat!(
            "#[cfg(unix)]\nmod owner { pub use crate::source::Widget; }\n",
            "#[cfg(windows)]\nmod owner { pub use crate::source::Widget; }\n",
            "fn keep() {}\n",
        );
        assert_eq!(
            edited(
                source,
                &[
                    fact(2, 13, "Widget", "owner", ModuleForm::Inline),
                    fact(4, 13, "Widget", "owner", ModuleForm::Inline),
                ]
            ),
            "fn keep() {}\n"
        );
    }

    #[test]
    fn different_use_items_rewrite_only_their_own_nonempty_fact_groups() {
        let source = concat!(
            "pub use crate::source::{Widget, Keep};\n",
            "pub use crate::source::{Gadget, Stay};\n",
            "fn f(_: Widget) {}\n",
        );
        assert_eq!(
            edited(source, &[file_fact(1, "Widget"), file_fact(2, "Gadget")]),
            concat!(
                "pub use crate::source::Keep;\nuse crate::source::Widget;\n",
                "pub use crate::source::Stay;\nfn f(_: Widget) {}\n",
            )
        );
    }

    #[test]
    fn inline_owner_edit_keeps_multibyte_prefix_and_surrounding_code() {
        let source = concat!(
            "mod panel { /* é */ pub use crate::source::Widget;",
            " fn f(_: Widget) {} }\n",
        );
        let start = source.find("pub use").expect("re-export");
        let column = source[..start].chars().count() + 1;
        assert_eq!(
            edited(
                source,
                &[fact(1, column, "Widget", "panel", ModuleForm::Inline)]
            ),
            concat!(
                "mod panel { /* é */ use crate::source::Widget;",
                " fn f(_: Widget) {} }\n",
            )
        );
    }

    #[test]
    fn replacing_multiple_locally_used_leaves_preserves_piece_order_and_indent() {
        assert_eq!(
            edited(
                concat!(
                    "mod panel {\n",
                    "    pub use crate::source::{Widget, Other};\n",
                    "    fn f(_: Widget, _: Other) {}\n",
                    "}\n",
                ),
                &[
                    fact(2, 5, "Widget", "panel", ModuleForm::Inline),
                    fact(2, 5, "Other", "panel", ModuleForm::Inline)
                ]
            ),
            concat!(
                "mod panel {\n",
                "    use crate::source::Widget;\n",
                "    use crate::source::Other;\n",
                "    fn f(_: Widget, _: Other) {}\n",
                "}\n",
            )
        );
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
