use std::fs;
use std::path::Path;

use anyhow::Context;
use anyhow::Result;

use super::imports::UseFix;
use crate::config::DiagnosticCode;
use crate::reporting::Report;

pub(crate) struct NarrowPubCrateScan {
    pub fixes: Vec<UseFix>,
}

pub(crate) fn scan_from_report(report: &Report) -> Result<NarrowPubCrateScan> {
    let root = Path::new(&report.root);
    let mut fixes = Vec::new();
    for finding in &report.findings {
        if finding.diagnostic_code != DiagnosticCode::NarrowToPubCrate {
            continue;
        }
        let absolute_path = root.join(&finding.path);
        let source = fs::read_to_string(&absolute_path)
            .with_context(|| format!("failed to read {}", absolute_path.display()))?;
        let Some(line_start) = line_byte_offset(&source, finding.line) else {
            continue;
        };
        let line_end = source[line_start..]
            .find('\n')
            .map_or(source.len(), |pos| line_start + pos);
        let line_text = &source[line_start..line_end];
        let Some(relative_start) = line_text.find("pub ") else {
            continue;
        };
        let start = line_start + relative_start;
        fixes.push(UseFix {
            path: absolute_path,
            start,
            end: start + "pub ".len(),
            replacement: "pub(crate) ".to_string(),
            import_group: None,
        });
    }
    Ok(NarrowPubCrateScan { fixes })
}

/// Return the byte offset of the start of 1-based `line` in `source`.
fn line_byte_offset(source: &str, line: usize) -> Option<usize> {
    if line == 0 {
        return None;
    }
    if line == 1 {
        return Some(0);
    }
    source
        .match_indices('\n')
        .nth(line - 2)
        .map(|(pos, _)| pos + 1)
}
