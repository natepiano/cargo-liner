//! Locate the binding on the nearest unique matching source line.

use std::collections::BTreeSet;

use super::EditRefusal;
use crate::binding::patterns::BindingScope;
use crate::binding::patterns::IdentifierMatch;
use crate::binding::scan::BindingScan;
use crate::rename_admission::BindingRename;
use crate::source_sites;

impl BindingScan {
    pub(super) fn locate(&self, rename: &BindingRename<'_>) -> Result<BindingScope, EditRefusal> {
        let text = source_sites::trim(rename.text);
        let nearest = self
            .source
            .lines()
            .enumerate()
            .filter(|(_, line)| source_sites::trim(line) == text)
            .map(|(index, _)| ((index + 1).abs_diff(rename.line), index + 1));
        let line = nearest_line(nearest)?;
        let candidates = self
            .sites
            .iter()
            .flat_map(|site| {
                site.pattern.clone().filter(|&index| {
                    self.tokens[index].line == line
                        && self.binding_word(
                            site,
                            index,
                            rename.old,
                            IdentifierMatch::ExactPlainName,
                        )
                })
            })
            .collect::<BTreeSet<_>>();
        if candidates.len() != 1 {
            return Err(EditRefusal);
        }
        self.scope_at(candidates.into_iter().next().ok_or(EditRefusal)?)
            .map_err(|_| EditRefusal)
    }
}

fn nearest_line(lines: impl Iterator<Item = (usize, usize)>) -> Result<usize, EditRefusal> {
    let mut nearest = None;
    let mut tied = false;
    for (distance, line) in lines {
        match nearest {
            Some((best, _)) if distance > best => {},
            Some((best, _)) if distance == best => tied = true,
            _ => {
                nearest = Some((distance, line));
                tied = false;
            },
        }
    }
    if tied {
        Err(EditRefusal)
    } else {
        nearest.map(|(_, line)| line).ok_or(EditRefusal)
    }
}
