//! List settings typed in as text: [`list_display`] is how the row
//! reads, [`join_list`] is the text its editor opens on, and
//! [`parse_list`] reads the committed text back into entries.

use super::constants::EMPTY_LIST;
use super::constants::LIST_SEPARATOR;

/// Typed text as a list of entries: commas and whitespace both
/// separate, and an entry typed twice is kept once, where it first
/// appears.
#[must_use]
pub fn parse_list(text: &str) -> Vec<String> {
    let mut entries: Vec<String> = Vec::new();
    for entry in text
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|entry| !entry.is_empty())
    {
        if !entries.iter().any(|kept| kept == entry) {
            entries.push(entry.to_string());
        }
    }
    entries
}

/// The entries joined with [`LIST_SEPARATOR`]: the text a list row's
/// editor opens on. An empty list opens as empty text, leaving nothing
/// to type over.
#[must_use]
pub fn join_list(entries: &[String]) -> String { entries.join(LIST_SEPARATOR) }

/// A list setting as its row reads: the entries joined as by
/// [`join_list`], or a word in place of an empty list.
#[must_use]
pub fn list_display(entries: &[String]) -> String {
    if entries.is_empty() {
        return EMPTY_LIST.to_string();
    }
    join_list(entries)
}

#[cfg(test)]
mod tests {
    use super::EMPTY_LIST;
    use super::join_list;
    use super::list_display;
    use super::parse_list;

    fn entries(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_string()).collect()
    }

    #[test]
    fn typed_lists_split_on_commas_and_spaces_and_keep_each_entry_once() {
        assert_eq!(parse_list("port, handler"), ["port", "handler"]);
        assert_eq!(parse_list(" port  handler,,port "), ["port", "handler"]);
        assert_eq!(parse_list(" , "), [] as [String; 0]);
    }

    #[test]
    fn the_editor_opens_on_the_bare_entries_and_parses_back_to_them() {
        let list = entries(&["port", "handler"]);
        assert_eq!(join_list(&list), "port, handler");
        assert_eq!(parse_list(&join_list(&list)), list);
        assert_eq!(join_list(&[]), "");
    }

    #[test]
    fn an_empty_list_reads_as_a_word_in_its_row() {
        assert_eq!(
            list_display(&entries(&["port", "handler"])),
            "port, handler"
        );
        assert_eq!(list_display(&[]), EMPTY_LIST);
    }
}
