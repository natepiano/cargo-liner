//! Check if a current item can introduce or already uses the wanted name.

use std::collections::BTreeSet;

use super::constants::STANDARD_EXPRESSION_MACROS;
use super::patterns::BindingScope;
use super::patterns::IdentifierMatch;
use super::scan;
use super::scan::BindingScan;
use crate::enclosing_items::EnclosingItem;
use crate::enclosing_items::OwnerBody;
use crate::enclosing_items::OwnerHeader;
use crate::rename_admission::OuterBinding;
use crate::rename_admission::PolicyInput;
use crate::rename_admission::PolicyLoad;
use crate::rename_admission::RefusalCause;
use crate::rename_admission::RefusalLocation;
use crate::rename_admission::RenameRefusal;
use crate::tokenizer::TokenKind;
use crate::word_syntax;

pub(super) enum Presence {
    Clear,
    Refused(PresenceRefusal),
}

pub(crate) enum PresenceRefusal {
    Blocked(OuterBinding),
    Unproved(RenameRefusal),
    MissingPolicyFlag,
}

impl BindingScan {
    pub(super) fn presence(
        &self,
        scope: &BindingScope,
        name: &str,
        policy: &PolicyInput,
    ) -> Presence {
        let target = scope.declaration;
        let items = match self.items() {
            Ok(items) => items,
            Err(error) => return self.refuse_at(RefusalCause::ContainingItemUnread(error), target),
        };
        let enclosing = owners(items, target);
        let Some(item) = enclosing.last() else {
            return self.refuse_at(RefusalCause::ContainingItemMissing, target);
        };
        if !unique_owners(&enclosing, items) {
            return self.refuse_at(RefusalCause::ItemPathNotUnique, target);
        }
        let mut region: BTreeSet<_> = item.tokens.clone().collect();
        for owner in &enclosing {
            if matches!(owner.kind.as_str(), "impl" | "trait") {
                let OwnerHeader::Proved { end, .. } = owner.header_tokens else {
                    return Presence::Refused(PresenceRefusal::Unproved(RenameRefusal {
                        reason:   RefusalCause::ItemHeaderUnread,
                        location: RefusalLocation::Line(self.tokens[target].line),
                    }));
                };
                region.extend(owner.tokens.start..end);
            }
        }
        let supplied = match policy.load() {
            PolicyLoad::Read(policy) => policy,
            PolicyLoad::Unread(error) => {
                return Presence::Refused(PresenceRefusal::Unproved(RenameRefusal {
                    reason:   RefusalCause::PolicyUnread(error.clone()),
                    location: RefusalLocation::Absent,
                }));
            },
            PolicyLoad::MissingFlag => {
                return Presence::Refused(PresenceRefusal::MissingPolicyFlag);
            },
        };
        if let Some((token, name)) =
            self.unresolved_attribute(&enclosing, target, &region, &supplied.inert_attributes)
        {
            let reason = RefusalCause::AttributeMacro {
                name: name.to_owned(),
                line: self.tokens[token].line,
            };
            return self.refuse_at(reason, token);
        }
        if let Some((token, name)) = self.unresolved_macro(&region, &supplied.std_macros) {
            let reason = RefusalCause::BangMacro {
                name: name.to_owned(),
                line: self.tokens[token].line,
            };
            return self.refuse_at(reason, token);
        }
        match self.name_presence(&region, target, scan::plain(name)) {
            Presence::Clear
                if enclosing
                    .iter()
                    .any(|owner| matches!(owner.body, OwnerBody::Unproved(_))) =>
            {
                self.refuse_at(RefusalCause::ItemBodyUnread, target)
            },
            result => result,
        }
    }

    fn name_presence(&self, region: &BTreeSet<usize>, target: usize, wanted: &str) -> Presence {
        let mut first = None;
        for &index in region {
            let token = &self.tokens[index];
            if token.kind != TokenKind::Identifier || scan::plain(&token.value) != wanted {
                continue;
            }
            first.get_or_insert(index);
            let candidate = self.sites.iter().any(|site| {
                scan::contains(&site.ranges, target)
                    && self.binding_word(site, index, wanted, IdentifierMatch::WithoutRawPrefix)
            });
            if candidate
                && self
                    .scope_at(index)
                    .is_ok_and(|scope| scan::contains(&scope.ranges, target))
            {
                return Presence::Refused(PresenceRefusal::Blocked(OuterBinding::new(
                    wanted,
                    token.line,
                    self.line_text(token.line),
                )));
            }
        }
        first.map_or(Presence::Clear, |index| {
            let reason = RefusalCause::WantedNameUsed {
                name: wanted.to_owned(),
                line: self.tokens[index].line,
            };
            self.refuse_at(reason, index)
        })
    }

    fn unresolved_attribute<'a>(
        &'a self,
        enclosing: &[&EnclosingItem],
        target: usize,
        region: &BTreeSet<usize>,
        inert: &[String],
    ) -> Option<(usize, &'a str)> {
        let owners = word_syntax::attribute_owners(&self.words);
        for (&start, &finish) in &self.attributes {
            let inner = self.word(start + 1) == "!";
            let inherited = inner
                && self.parents.get(&start).is_none_or(|&parent| {
                    self.word(parent) == "{" && parent < target && target < self.ends[&parent]
                });
            let attached = owners.get(&start).is_some_and(|&owner| {
                enclosing.iter().any(|item| {
                    matches!(item.header_tokens, OwnerHeader::Proved { end, .. }
                    if item.tokens.start <= start && start <= owner && owner < end)
                })
            });
            if !inherited && !attached && !region.contains(&start) {
                continue;
            }
            let at = start + if inner { 3 } else { 2 };
            let name = if at < finish {
                scan::plain(self.word(at))
            } else {
                "<unread>"
            };
            if self.word(at + 1) == "::" || !inert.iter().any(|allowed| allowed == name) {
                return Some((start, name));
            }
        }
        None
    }

    fn unresolved_macro<'a>(
        &'a self,
        region: &BTreeSet<usize>,
        standard: &[String],
    ) -> Option<(usize, &'a str)> {
        let uncertain = self.uncertain_macro_origins();
        self.macros.iter().find_map(|&(bang, finish)| {
            if !region.contains(&bang) {
                return None;
            }
            let name = scan::plain(self.lead(bang));
            let mut start = bang.saturating_sub(1);
            while start >= 2 && self.word(start - 1) == "::" {
                start -= 2;
            }
            let words: Vec<_> = self.words[start..finish]
                .iter()
                .map(|word| scan::plain(word))
                .collect();
            (!standard.iter().any(|allowed| allowed == name)
                || macro_refusal(&words, name, uncertain))
            .then_some((bang, name))
        })
    }

    pub(super) fn uncertain_macro_origins(&self) -> bool {
        if self
            .words
            .iter()
            .any(|word| matches!(scan::plain(word), "macro_rules" | "macro_use" | "macro"))
            || self.has_glob()
        {
            return true;
        }
        let called: BTreeSet<_> = self
            .words
            .windows(2)
            .filter(|pair| scan::plain(&pair[1]) == "!")
            .map(|pair| scan::plain(&pair[0]))
            .collect();
        self.words.iter().enumerate().any(|(i, word)| {
            word == "use"
                && self.words[i..]
                    .iter()
                    .take_while(|word| *word != ";")
                    .any(|word| called.contains(scan::plain(word)))
        })
    }

    fn refuse_at(&self, reason: RefusalCause, token: usize) -> Presence {
        let line = self.tokens[token].line;
        Presence::Refused(PresenceRefusal::Unproved(RenameRefusal {
            reason,
            location: RefusalLocation::SourceLine {
                line,
                text: self.line_text(line).to_owned(),
            },
        }))
    }
}

fn macro_refusal(words: &[&str], name: &str, uncertain: bool) -> bool {
    for (index, &word) in words.iter().enumerate() {
        if matches!(word, "include" | "macro" | "macro_rules") {
            return true;
        }
        let following = words.get(index + 1).copied().unwrap_or("");
        if word != name || following == "::" {
            continue;
        }
        let qualified = index > 0 && words[index - 1] == "::";
        if following != "!" {
            if qualified
                || words[..index]
                    .iter()
                    .rev()
                    .take_while(|&&word| word != ";")
                    .any(|&word| word == "use")
            {
                return true;
            }
            continue;
        }
        if !matches!(words.get(index + 2), Some(&("(" | "[" | "{"))) {
            continue;
        }
        let mut start = index;
        while start >= 2 && words[start - 1] == "::" && word_syntax::identifier(words[start - 2]) {
            start -= 2;
        }
        if start > 0 && words[start - 1] == "::" {
            start -= 1;
        }
        if uncertain
            || !STANDARD_EXPRESSION_MACROS.contains(&word)
            || !matches!(
                &words[start..index],
                [] | ["std" | "core", "::"] | ["::", "std" | "core", "::"]
            )
        {
            return true;
        }
    }
    false
}

pub(super) fn owners(items: &[EnclosingItem], index: usize) -> Vec<&EnclosingItem> {
    let mut owners: Vec<_> = items
        .iter()
        .filter(|item| item.tokens.contains(&index))
        .collect();
    owners.sort_by_key(|item| item.tokens.start);
    owners
}

pub(super) fn unique_owners(owners: &[&EnclosingItem], items: &[EnclosingItem]) -> bool {
    owners.iter().all(|owner| {
        items
            .iter()
            .filter(|item| item.identity == owner.identity)
            .take(2)
            .count()
            == 1
    })
}
