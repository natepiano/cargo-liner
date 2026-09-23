use std::fs;
use std::iter;
use std::path::Path;

use anyhow::Context;
use anyhow::Result;

use crate::fixes::constants::CFG_TEST_ATTRIBUTE;
use crate::fixes::facade_redirect::CallerBuilds;
use crate::fixes::imports::UseFix;
use crate::reporting::AncestorReexportInsertion;

/// The re-export added to the common ancestor module so callers outside the
/// target scope still find the item there: whole lines at the insertion
/// offset, each `#[cfg]` of the removed re-export first, then `#[cfg(test)]`
/// when only test builds compile those callers.
pub(super) fn insertion_fix(
    file: &Path,
    insertion: &AncestorReexportInsertion,
    builds: CallerBuilds,
) -> Result<UseFix> {
    let source =
        fs::read_to_string(file).with_context(|| format!("failed to read {}", file.display()))?;
    Ok(UseFix {
        path:         file.to_path_buf(),
        start:        insertion.offset,
        end:          insertion.offset,
        replacement:  insertion_text(&source, insertion, builds),
        import_group: None,
    })
}

fn insertion_text(
    source: &str,
    insertion: &AncestorReexportInsertion,
    builds: CallerBuilds,
) -> String {
    let offset = insertion.offset;
    let line_break = if offset > 0 && source.as_bytes().get(offset - 1) != Some(&b'\n') {
        "\n"
    } else {
        ""
    };
    let reexport = format!(
        "{} {};",
        insertion.reexport_visibility.use_keyword(),
        insertion.relative_path
    );
    let test_gate = (builds == CallerBuilds::TestOnly
        && !insertion
            .cfg_attributes
            .iter()
            .any(|attribute| attribute == CFG_TEST_ATTRIBUTE))
    .then_some(CFG_TEST_ATTRIBUTE);
    let mut text = line_break.to_string();
    for line in insertion
        .cfg_attributes
        .iter()
        .map(String::as_str)
        .chain(test_gate)
        .chain(iter::once(reexport.as_str()))
    {
        text.push_str(&insertion.indent);
        text.push_str(line);
        text.push('\n');
    }
    text
}

#[cfg(test)]
mod tests {
    use super::insertion_text;
    use crate::fixes::facade_redirect::CallerBuilds;
    use crate::reporting::AncestorReexportInsertion;
    use crate::reporting::ReexportVisibility;

    fn insertion(
        offset: usize,
        reexport_visibility: ReexportVisibility,
    ) -> AncestorReexportInsertion {
        AncestorReexportInsertion {
            reexport_visibility,
            relative_path: "staging::Widget".to_string(),
            cfg_attributes: vec!["#[cfg(test)]".to_string()],
            file: "src/tool/mod.rs".to_string(),
            offset,
            indent: "    ".to_string(),
        }
    }

    #[test]
    fn cfg_lines_precede_the_reexport_at_the_line_indent() {
        let source = "mod inner {\n    use a::b;\n}\n";
        let offset = source.find("}\n").unwrap_or_default();
        assert_eq!(
            insertion_text(
                source,
                &insertion(offset, ReexportVisibility::Crate),
                CallerBuilds::Every
            ),
            "    #[cfg(test)]\n    pub(crate) use staging::Widget;\n"
        );
    }

    #[test]
    fn an_offset_after_code_on_its_line_starts_a_new_line() {
        let source = "mod inner { use a::b; }";
        let offset = source.find('}').unwrap_or_default();
        assert_eq!(
            insertion_text(
                source,
                &insertion(offset, ReexportVisibility::Parent),
                CallerBuilds::Every
            ),
            "\n    #[cfg(test)]\n    pub(super) use staging::Widget;\n"
        );
    }

    #[test]
    fn test_only_callers_gate_the_reexport_after_its_cfg_lines() {
        let source = "mod inner {\n}\n";
        let offset = source.find("}\n").unwrap_or_default();
        let mut unix = insertion(offset, ReexportVisibility::Crate);
        unix.cfg_attributes = vec!["#[cfg(unix)]".to_string()];
        assert_eq!(
            insertion_text(source, &unix, CallerBuilds::TestOnly),
            "    #[cfg(unix)]\n    #[cfg(test)]\n    pub(crate) use staging::Widget;\n"
        );
        assert_eq!(
            insertion_text(
                source,
                &insertion(offset, ReexportVisibility::Crate),
                CallerBuilds::TestOnly
            ),
            "    #[cfg(test)]\n    pub(crate) use staging::Widget;\n"
        );
    }

    #[test]
    fn each_visibility_spells_its_keywords() {
        assert_eq!(ReexportVisibility::Public.use_keyword(), "pub use");
        assert_eq!(ReexportVisibility::Crate.use_keyword(), "pub(crate) use");
        assert_eq!(ReexportVisibility::Parent.use_keyword(), "pub(super) use");
        assert_eq!(ReexportVisibility::Private.use_keyword(), "use");
    }
}
