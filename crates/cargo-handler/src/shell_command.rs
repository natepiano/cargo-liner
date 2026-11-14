//! The short form of a shell command for its tree row.
//!
//! A path argument draws as its file name. Quotes, spaces, redirections and
//! arguments that are not paths stay as the command wrote them.

/// `command` with each path argument cut to its file name.
pub(crate) fn short_command(command: &str) -> String {
    let mut out = String::with_capacity(command.len());
    let mut start = None;
    let mut quote = None;
    for (index, character) in command.char_indices() {
        match quote {
            Some(open) if character == open => quote = None,
            Some(_) => {},
            None if character.is_whitespace() => {
                if let Some(begin) = start.take() {
                    push_short_argument(&command[begin..index], &mut out);
                }
                out.push(character);
            },
            None => {
                if matches!(character, '"' | '\'') {
                    quote = Some(character);
                }
                start.get_or_insert(index);
            },
        }
    }
    if let Some(begin) = start {
        push_short_argument(&command[begin..], &mut out);
    }
    out
}

/// One argument, after its redirection or option name, with its path cut.
fn push_short_argument(argument: &str, out: &mut String) {
    let (prefix, value) = argument.split_at(prefix_length(argument));
    out.push_str(prefix);
    match whole_quote(value) {
        Some(quote) => {
            let inner = &value[1..value.len() - 1];
            out.push(quote);
            out.push_str(if is_explicit_path(inner) {
                file_name(inner)
            } else {
                inner
            });
            out.push(quote);
        },
        None if is_path(value) && !value.contains(['"', '\'']) => out.push_str(file_name(value)),
        None => out.push_str(value),
    }
}

/// Bytes of a redirection (`2>`, `>>`, `<`, `>&`) or an option name (`--log=`).
fn prefix_length(argument: &str) -> usize {
    let digits = argument.bytes().take_while(u8::is_ascii_digit).count();
    let redirection = argument[digits..]
        .bytes()
        .take_while(|byte| matches!(byte, b'<' | b'>' | b'&'))
        .count();
    if redirection > 0 {
        return digits + redirection;
    }
    if argument.starts_with('-')
        && let Some(equals) = argument.find('=')
        && !argument[..equals].contains(['/', '"', '\''])
    {
        return equals + 1;
    }
    0
}

/// The quote character that opens and closes the whole value.
fn whole_quote(value: &str) -> Option<char> {
    let first = value.chars().next()?;
    (matches!(first, '"' | '\'') && value.len() >= 2 && value.ends_with(first)).then_some(first)
}

/// An unquoted path names a folder and is not a URL.
fn is_path(value: &str) -> bool {
    value.contains('/') && !value.contains("://") && !file_name(value).is_empty()
}

/// A quoted path starts at a root, a home or a relative folder, so quoted
/// text such as a `sed` expression stays whole.
fn is_explicit_path(value: &str) -> bool {
    ["/", "./", "../", "~/"]
        .iter()
        .any(|start| value.starts_with(start))
        && is_path(value)
}

/// The last name of a path, with a final `/` kept for a folder.
fn file_name(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    let start = trimmed.rfind('/').map_or(0, |slash| slash + 1);
    let end = if trimmed.len() < path.len() {
        trimmed.len() + 1
    } else {
        trimmed.len()
    };
    &path[start..end]
}
