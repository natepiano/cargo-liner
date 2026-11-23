//! Check which names and format captures a binding can reach.

use std::collections::BTreeSet;

use super::EditRefusal;
use crate::binding::alignment;
use crate::binding::constants::STANDARD_EXPRESSION_MACROS;
use crate::binding::patterns::BindingScope;
use crate::binding::patterns::IdentifierMatch;
use crate::binding::patterns::PatternKind;
use crate::binding::scan;
use crate::binding::scan::BindingScan;
use crate::rename_admission::BindingRename;

#[derive(Clone, Copy)]
pub(super) enum CaptureEdit {
    Edited,
    Unchanged,
}

impl BindingScan {
    pub(super) fn check_file_names(&self, rename: &BindingRename<'_>) -> Result<(), EditRefusal> {
        if self
            .words
            .iter()
            .any(|word| scan::plain(word) == rename.new)
        {
            return Err(EditRefusal);
        }
        if self.has_glob() {
            return Err(EditRefusal);
        }
        let name = |index| scan::plain(self.word(index)) == rename.old;
        if self
            .attributes
            .iter()
            .any(|(&start, &end)| (start..end).any(name))
            || (0..self.words.len()).any(|index| {
                name(index) && (self.lead(index) == "::" || self.word(index + 1) == "::")
            })
        {
            return Err(EditRefusal);
        }
        let bindings = self
            .sites
            .iter()
            .flat_map(|site| {
                site.pattern.clone().filter(|&index| {
                    self.binding_word(site, index, rename.old, IdentifierMatch::ExactPlainName)
                })
            })
            .collect::<BTreeSet<_>>();
        let end = self.words.len();
        for (index, item) in self.words.iter().enumerate() {
            let word = item.as_str();
            let finish = if matches!(
                word,
                "fn" | "struct"
                    | "union"
                    | "enum"
                    | "trait"
                    | "type"
                    | "const"
                    | "static"
                    | "mod"
                    | "use"
                    | "impl"
            ) {
                let delimiters: &[&str] = if word == "use" { &[";"] } else { &["{", ";"] };
                let finish = self.stop(index + 1, end, delimiters);
                if matches!(word, "enum" | "trait")
                    && let Some(&close) = self.ends.get(&finish)
                {
                    close + 1
                } else {
                    finish
                }
            } else if matches!(word, ":" | "->" | "as") || word == "<" && self.lead(index) == "::" {
                self.level(index + 1, end, true)
                    .into_iter()
                    .find(|&position| {
                        matches!(
                            self.word(position),
                            "=" | ";" | "," | "{" | "}" | ")" | "=>" | "|" | ">" | ">>"
                        )
                    })
                    .unwrap_or(end)
            } else {
                continue;
            };
            if (index + 1..finish).any(|position| name(position) && !bindings.contains(&position)) {
                return Err(EditRefusal);
            }
        }
        Ok(())
    }

    pub(super) fn check_visibility(
        &self,
        scope: &BindingScope,
        rename: &BindingRename<'_>,
        capture_edit: CaptureEdit,
    ) -> Result<(), EditRefusal> {
        if self.unread_binding(scope, rename) {
            return Err(EditRefusal);
        }
        let macros = self.macros.iter().filter(|&&(bang, finish)| {
            scope
                .ranges
                .iter()
                .any(|range| range.start < finish && bang < range.end)
        });
        if matches!(capture_edit, CaptureEdit::Edited) && self.uncertain_format_macros()
            || self.uncertain_macro_origins() && macros.clone().next().is_some()
        {
            return Err(EditRefusal);
        }
        if scope.ranges.iter().cloned().flatten().any(|index| {
            self.word(index)
                .strip_prefix("r#")
                .is_some_and(|word| word == rename.old || word == rename.new)
        }) {
            return Err(EditRefusal);
        }
        if self.parameter_collision(scope, rename.new) {
            return Err(EditRefusal);
        }
        for &index in &self.formats {
            if !scan::contains(&scope.ranges, index) {
                continue;
            }
            let literal = self.word(index);
            if !literal.starts_with('r')
                && (literal.contains("\\u{")
                    || literal.as_bytes().windows(4).any(|bytes| {
                        bytes.starts_with(b"\\x") && bytes[2..].iter().all(u8::is_ascii_hexdigit)
                    }))
            {
                return Err(EditRefusal);
            }
            if has_capture(literal, rename.new) {
                return Err(EditRefusal);
            }
        }
        if macros.clone().any(|&(bang, _)| {
            let name = self.lead(bang);
            matches!(name, "concat" | "stringify") || !STANDARD_EXPRESSION_MACROS.contains(&name)
        }) {
            return Err(EditRefusal);
        }
        Ok(())
    }

    fn parameter_collision(&self, scope: &BindingScope, name: &str) -> bool {
        if !matches!(
            scope.kind,
            PatternKind::FunctionParameter | PatternKind::Closure
        ) {
            return false;
        }
        let body = self
            .sites
            .iter()
            .filter(|site| site.kind == scope.kind && site.pattern.contains(&scope.declaration))
            .filter_map(|site| site.ranges.last())
            .collect::<Vec<_>>();
        self.sites.iter().any(|site| {
            site.kind == scope.kind
                && site
                    .ranges
                    .last()
                    .is_some_and(|range| body.contains(&range))
                && site.pattern.clone().any(|index| {
                    self.binding_word(site, index, name, IdentifierMatch::ExactPlainName)
                })
        })
    }

    fn uncertain_format_macros(&self) -> bool {
        let format = |word| scan::format_argument_position(word).is_some();
        self.words
            .iter()
            .any(|word| scan::plain(word) == "macro_rules")
            || self.words.iter().enumerate().any(|(index, word)| {
                word == "use"
                    && (index + 1..self.stop(index + 1, self.words.len(), &[";"]))
                        .any(|position| format(self.word(position)))
            })
            || self.macros.iter().any(|&(bang, _)| {
                bang >= 2 && self.word(bang - 2) == "::" && format(self.lead(bang))
            })
    }
}

pub(super) fn has_capture(literal: &str, name: &str) -> bool {
    alignment::captures(literal)
        .into_iter()
        .any(|range| &literal[range] == name)
}
