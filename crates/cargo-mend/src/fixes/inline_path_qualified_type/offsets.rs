use anyhow::Context;
use anyhow::Result;
use proc_macro2::LineColumn;

use crate::fixes::imports::ImportSource;

/// Validate an external position against the file's existing source owner.
/// Character columns are converted to bytes by the checked location constructor,
/// so multibyte characters do not shift replacement windows. Missing lines and
/// columns are rejected instead of becoming byte zero or the end of the source.
pub(super) fn offset(source: &ImportSource<'_>, position: LineColumn) -> Result<usize> {
    source
        .locate(position, position)
        .map(|location| location.start())
        .context("invalid scope source position")
}

#[cfg(test)]
mod tests {
    use proc_macro2::LineColumn;

    use crate::fixes::imports::ImportSource;

    #[test]
    fn checked_offsets_reject_missing_coordinates_and_preserve_unicode() -> anyhow::Result<()> {
        let source = ImportSource::new("// é\nuse α::Thing;\n");
        let at = |line, column| LineColumn { line, column };
        for position in [at(0, 0), at(4, 0), at(1, 5), at(2, 14)] {
            assert!(super::offset(&source, position).is_err());
        }
        assert_eq!(super::offset(&source, at(2, 7))?, "// é\nuse α::".len());
        assert_eq!(
            super::offset(&source, at(3, 0))?,
            "// é\nuse α::Thing;\n".len()
        );
        Ok(())
    }
}
