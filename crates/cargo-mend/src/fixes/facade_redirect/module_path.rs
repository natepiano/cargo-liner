use std::iter;

use crate::rust_syntax::PathAnchor;

/// The crate-relative path `segments` names when written in `current_module_path`.
///
/// `None` means a `super` climbs above the crate root.
pub(in crate::fixes) fn absolute_use_path(
    current_module_path: &[String],
    segments: &[String],
) -> Option<Vec<String>> {
    match PathAnchor::first(segments)? {
        PathAnchor::Crate => Some(segments[1..].to_vec()),
        PathAnchor::SelfMod => Some(
            current_module_path
                .iter()
                .cloned()
                .chain(segments[1..].iter().cloned())
                .collect(),
        ),
        PathAnchor::Super => {
            let mut module = current_module_path.to_vec();
            let mut index = 0usize;
            while segments
                .get(index)
                .is_some_and(|seg| PathAnchor::from(seg.as_str()) == PathAnchor::Super)
            {
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
        PathAnchor::SelfType | PathAnchor::Name => Some(
            current_module_path
                .iter()
                .cloned()
                .chain(segments.iter().cloned())
                .collect(),
        ),
    }
}

/// How code in `current_module_path` spells the crate-relative `target_path`.
///
/// A target inside the current module's subtree is written relative
/// (`child::Item`), one level up through a shared ancestor as `super::Item`,
/// and anything further as `crate::…`. These are the spellings the
/// `shorten_local_crate_import` and `replace_deep_super_import` checks accept,
/// so a rewritten path never raises a follow-up finding.
pub(super) fn import_path(
    current_module_path: &[String],
    target_path: &[String],
    rename: Option<&str>,
) -> String {
    let common = common_prefix_len(current_module_path, target_path);
    let up_count = current_module_path.len() - common;
    let remainder = target_path[common..].iter().map(String::as_str);
    let segments: Vec<&str> = if up_count == 0 {
        if common == target_path.len() {
            vec!["self"]
        } else {
            remainder.collect()
        }
    } else if up_count == 1 && common > 0 {
        iter::once("super").chain(remainder).collect()
    } else {
        iter::once("crate")
            .chain(target_path.iter().map(String::as_str))
            .collect()
    };
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

#[cfg(test)]
mod tests {
    use super::absolute_use_path;
    use super::import_path;

    fn path(text: &str) -> Vec<String> {
        text.split("::")
            .filter(|segment| !segment.is_empty())
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn a_target_in_the_current_subtree_is_written_relative() {
        assert_eq!(
            import_path(&path("tool"), &path("tool::staging::f"), None),
            "staging::f"
        );
    }

    #[test]
    fn every_target_is_relative_from_the_crate_root() {
        assert_eq!(
            import_path(&[], &path("free_cam::Rig"), None),
            "free_cam::Rig"
        );
    }

    #[test]
    fn a_sibling_through_the_parent_is_written_with_super() {
        assert_eq!(
            import_path(
                &path("report::frontmatter"),
                &path("report::writer::Builder"),
                None
            ),
            "super::writer::Builder"
        );
    }

    #[test]
    fn two_levels_up_is_written_from_the_crate() {
        assert_eq!(
            import_path(
                &path("actor::nested::deeper"),
                &path("actor::child::Stats"),
                None
            ),
            "crate::actor::child::Stats"
        );
    }

    #[test]
    fn a_top_level_module_reaches_a_root_item_from_the_crate() {
        assert_eq!(
            import_path(&path("input"), &path("Rig"), None),
            "crate::Rig"
        );
    }

    #[test]
    fn the_current_module_itself_is_self() {
        assert_eq!(import_path(&path("tool"), &path("tool"), None), "self");
    }

    #[test]
    fn a_rename_is_kept() {
        assert_eq!(
            import_path(&path("tool"), &path("tool::staging::f"), Some("g")),
            "staging::f as g"
        );
    }

    #[test]
    fn super_segments_climb_from_the_current_module() {
        assert_eq!(
            absolute_use_path(&path("a::b"), &path("super::super::x")),
            Some(path("x"))
        );
        assert_eq!(
            absolute_use_path(&path("a"), &path("super::super::x")),
            None
        );
    }
}
