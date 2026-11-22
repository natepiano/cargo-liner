//! Read nested syntax inside an item.

use super::item_reader::ItemReader;
use crate::constants::ITEMS;
use crate::tokenizer::TokenKind;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum NestedSyntax {
    Statements,
    Fields,
    Variants,
    Arms,
    Continuation,
    Parameters,
}

impl NestedSyntax {
    const fn statements(self) -> bool { matches!(self, Self::Statements | Self::Arms) }
    const fn fields(self) -> bool { matches!(self, Self::Fields | Self::Variants) }
}

impl ItemReader<'_> {
    fn starts_item(&self, cursor: usize, end: usize, parameter: ParameterPosition) -> bool {
        let word = self.word(cursor);
        let following = self.word(cursor + 1);
        match word {
            "impl" if parameter == ParameterPosition::Type => false,
            "const" if matches!(following, "{" | "move" | "|" | "||") => false,
            "union" => cursor + 1 < end && self.is_identifier(cursor + 1),
            "macro_rules" => following == "!",
            "fn" | "extern" => {
                let keyword = self.item_prefix(cursor, end);
                !(self.word(keyword) == "fn" && self.word(keyword + 1) == "(")
            },
            _ => ITEMS.contains(&word),
        }
    }

    fn statement_start(&self, start: usize, end: usize) -> bool {
        match self.word(start) {
            "as" | "await" | "dyn" | "enum" | "extern" | "fn" | "gen" | "impl" | "in" | "mod"
            | "mut" | "pub" | "ref" | "static" | "struct" | "trait" | "type" | "use" | "where"
            | "abstract" | "become" | "box" | "do" | "final" | "macro" | "override" | "priv"
            | "typeof" | "unsized" | "virtual" => false,
            "'" => start + 2 < end && self.is_identifier(start + 1) && self.word(start + 2) == ":",
            "<" => self.qualified_path(start, end),
            "{" | "(" | "[" | ";" | "&" | "&&" | "*" | "!" | "-" | "|" | "||" | "." | "::" => true,
            _ => matches!(
                self.tokens[start].kind,
                TokenKind::Literal | TokenKind::Identifier
            ),
        }
    }

    fn qualified_path(&self, start: usize, end: usize) -> bool {
        let mut depth = 0;
        for cursor in start..end {
            match self.word(cursor) {
                "<" => depth += 1,
                ">" => {
                    depth -= 1;
                    if depth == 0 {
                        return cursor > start + 1
                            && self.word(cursor + 1) == "::"
                            && cursor + 2 < end
                            && self.is_identifier(cursor + 2);
                    }
                },
                "::" | "&" | "'" | "," => {},
                _ if self.is_identifier(cursor) => {},
                _ => return false,
            }
        }
        false
    }

    fn binding_pattern(&self, cursor: usize, start: usize, end: usize) -> bool {
        let next =
            (cursor + 1..end).find(|&index| !self.is_identifier(index) && self.word(index) != "::");
        cursor > start
            && self.is_identifier(cursor - 1)
            && next.is_some_and(|index| index > cursor + 1 && self.word(index) == "{")
    }

    fn field_end(&self, cursor: usize, end: usize) -> Option<usize> {
        let field = self.item_prefix(cursor, end);
        if field != cursor && (self.word(cursor) != "pub" || !self.closed_prefix(cursor, field)) {
            return None;
        }
        if self.word(field) == "." && self.word(field + 1) == "." {
            return Some(field + 2);
        }
        (field < end && self.is_identifier(field) && !ITEMS.contains(&self.word(field)))
            .then(|| field + 1 + usize::from(self.word(field + 1) == ":"))
    }

    fn closes_named_generics(&self, mut cursor: usize) -> bool {
        let mut depth = 0;
        loop {
            match self.word(cursor) {
                ")" | "]" | "}" => {
                    let Some(previous) = self
                        .openers
                        .get(&cursor)
                        .and_then(|open| open.checked_sub(1))
                    else {
                        return false;
                    };
                    cursor = previous;
                    continue;
                },
                "(" | "[" | "{" | ";" => return false,
                ">" => depth += 1,
                "<" => depth -= 1,
                _ => {},
            }
            if depth == 0 {
                return cursor >= 1 && self.word(cursor - 1) == "::"
                    || cursor >= 2 && matches!(self.word(cursor - 2), "fn" | "struct");
            }
            let Some(previous) = cursor.checked_sub(1) else {
                return false;
            };
            cursor = previous;
        }
    }

    pub(super) fn nested(
        &mut self,
        start: usize,
        end: usize,
        owner: &[String],
        syntax: NestedSyntax,
    ) -> bool {
        let mut state = NestedPosition::new(start, syntax);
        while state.cursor < end {
            let cursor = state.cursor;
            let word = self.word(cursor);
            if word == "$" || (word == "@" && !self.binding_pattern(cursor, start, end)) {
                return false;
            }
            let statement = self.skip_attributes(cursor, end);
            if statement >= end {
                return true;
            }
            if statement != cursor {
                state.cursor = statement;
                state.segment = statement;
            }
            if !self.nested_step(&mut state, end, owner, syntax) {
                return false;
            }
        }
        true
    }

    fn nested_step(
        &mut self,
        state: &mut NestedPosition,
        end: usize,
        owner: &[String],
        syntax: NestedSyntax,
    ) -> bool {
        let cursor = state.cursor;
        let word = self.word(cursor);
        if state.boundary == Boundary::Field {
            let Some(next) = self.field_end(cursor, end) else {
                return false;
            };
            state.cursor = next;
            state.boundary = Boundary::Continuation;
            return true;
        }
        if state.boundary == Boundary::Item {
            let keyword = self.item_prefix(cursor, end);
            if keyword < end && self.starts_item(keyword, end, state.parameter) {
                if !self.closed_prefix(cursor, keyword) {
                    return false;
                }
                let item_word = self.word(keyword);
                let body = !(matches!(item_word, "type" | "const" | "static" | "use")
                    || item_word == "extern" && self.word(keyword + 1) == "crate");
                let Some(last) = self.item_end(keyword + 1, end, body) else {
                    return false;
                };
                if !self.record_nested_item(cursor, last, owner) {
                    return false;
                }
                state.cursor = last;
                state.segment = last;
                return true;
            }
        }
        if self.starts_item(cursor, end, state.parameter)
            || ((state.boundary == Boundary::Item) && !self.statement_start(cursor, end))
        {
            return false;
        }
        if matches!(word, "{" | "(" | "[") {
            return self.nested_group(state, end, owner, syntax);
        }
        state.advance(word, syntax);
        true
    }

    fn nested_group(
        &mut self,
        state: &mut NestedPosition,
        end: usize,
        owner: &[String],
        syntax: NestedSyntax,
    ) -> bool {
        let cursor = state.cursor;
        let Some(close) = self.close(cursor, end) else {
            return false;
        };
        let block = self.word(cursor) == "{";
        if block && state.impl_bound == ImplBound::Seen {
            return false;
        }
        let child = if block {
            self.block_syntax(state.segment, cursor)
        } else {
            NestedSyntax::Continuation
        };
        if !self.nested(cursor + 1, close, owner, child) {
            return false;
        }
        state.cursor = close + 1;
        state.boundary = if block && child.statements() {
            Boundary::Item
        } else {
            Boundary::Continuation
        };
        if block && state.cursor < end {
            match self.word(state.cursor) {
                "=>" if syntax == NestedSyntax::Arms => {
                    state.cursor += 1;
                    state.boundary = Boundary::Item;
                },
                "=" if child == NestedSyntax::Fields => {
                    state.cursor += 1;
                    state.boundary = Boundary::Item;
                },
                "," if syntax == NestedSyntax::Arms || syntax.fields() => {
                    state.boundary = Boundary::Continuation;
                },
                "?" | "." => state.boundary = Boundary::Continuation,
                ">" if self.closes_named_generics(state.cursor) => {
                    state.boundary = Boundary::Continuation;
                },
                "else" => state.boundary = Boundary::Item,
                _ => {},
            }
        }
        if state.boundary == Boundary::Item {
            state.segment = state.cursor;
        }
        true
    }

    fn block_syntax(&self, segment: usize, cursor: usize) -> NestedSyntax {
        let lead = self.word(segment);
        if segment < cursor && lead == "match" {
            NestedSyntax::Arms
        } else if cursor > segment
            && self.is_identifier(cursor - 1)
            && !matches!(
                self.word(cursor - 1),
                "else" | "unsafe" | "async" | "const" | "try" | "loop"
            )
            && !matches!(lead, "if" | "while" | "for")
        {
            NestedSyntax::Fields
        } else {
            NestedSyntax::Statements
        }
    }
}

#[derive(PartialEq, Eq)]
enum Boundary {
    Item,
    Field,
    Continuation,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ParameterPosition {
    OutsideType,
    Type,
}

#[derive(PartialEq, Eq)]
enum ImplBound {
    Absent,
    Seen,
}

struct NestedPosition {
    cursor:     usize,
    segment:    usize,
    parameter:  ParameterPosition,
    impl_bound: ImplBound,
    boundary:   Boundary,
}

impl NestedPosition {
    const fn new(start: usize, syntax: NestedSyntax) -> Self {
        Self {
            cursor:     start,
            segment:    start,
            parameter:  ParameterPosition::OutsideType,
            impl_bound: ImplBound::Absent,
            boundary:   if syntax.statements() {
                Boundary::Item
            } else if syntax.fields() {
                Boundary::Field
            } else {
                Boundary::Continuation
            },
        }
    }

    fn advance(&mut self, word: &str, syntax: NestedSyntax) {
        if syntax == NestedSyntax::Parameters {
            match word {
                ":" => self.parameter = ParameterPosition::Type,
                "," | "=" => self.parameter = ParameterPosition::OutsideType,
                "impl" if self.parameter == ParameterPosition::Type => {
                    self.impl_bound = ImplBound::Seen;
                },
                _ => {},
            }
        }
        self.cursor += 1;
        self.boundary = match word {
            ";" if syntax == NestedSyntax::Statements => Boundary::Item,
            "," | "=>" if syntax == NestedSyntax::Arms => Boundary::Item,
            "," if syntax.fields() => Boundary::Field,
            _ => Boundary::Continuation,
        };
        if matches!(word, ";" | ",") || (word == "=>" && syntax == NestedSyntax::Arms) {
            self.segment = self.cursor;
        }
    }
}
